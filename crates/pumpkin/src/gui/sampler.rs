//! Turns live server state into [`Snapshot`]s for the window.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use pumpkin_config::gui::GuiConfig;
use pumpkin_gui::{GuiSide, PlayerRow, ServerMeta, Snapshot, SystemSampler, WorldRow};

use crate::server::Server;
use crate::{SHOULD_STOP, STOP_INTERRUPT};

/// Folders walked for one dimension. `root` is shared; region/entities/poi are not.
struct WorldScan {
    dimension: String,
    root: std::path::PathBuf,
    region: std::path::PathBuf,
    entities: std::path::PathBuf,
    poi: std::path::PathBuf,
}

impl WorldScan {
    fn dimension_size(&self) -> u64 {
        pumpkin_gui::directory_size(&self.region)
            + pumpkin_gui::directory_size(&self.entities)
            + pumpkin_gui::directory_size(&self.poi)
    }
}

/// Disk figures, shared between the slow scanner and the fast sampler.
#[derive(Default, Clone)]
struct DiskUsage {
    worlds_size: Option<u64>,
    per_world: Vec<(String, u64)>,
    free: u64,
    total: u64,
}

pub fn spawn(server: &Arc<Server>, side: &GuiSide, config: &GuiConfig) {
    let disk = Arc::new(ArcSwap::from_pointee(DiskUsage::default()));

    spawn_disk_scanner(server, &disk, config.disk_scan_secs);
    spawn_fast_sampler(server, side, &disk, config.refresh_ms);
}

fn spawn_fast_sampler(
    server: &Arc<Server>,
    side: &GuiSide,
    disk: &Arc<ArcSwap<DiskUsage>>,
    refresh_ms: u64,
) {
    let server = server.clone();
    let snapshot = side.snapshot.clone();
    let disk = disk.clone();
    // The OS reports CPU use as a delta between samples, so anything under Sysinfo's minimum
    // interval would report noise.
    let interval = Duration::from_millis(refresh_ms.max(200));

    server.clone().spawn_task(async move {
        let mut system = SystemSampler::new();
        let started = Instant::now();
        let meta = Arc::new(server_meta(&server));

        let mut last_net = (crate::metrics::bytes_in(), crate::metrics::bytes_out());
        let mut last_at = Instant::now();
        // Reused when `active_chunks` is being written and a read would block the tick.
        let mut last_worlds: Vec<WorldRow> = Vec::new();

        while !SHOULD_STOP.load(Ordering::Relaxed) {
            let stats = system.sample();
            let disk = (*disk.load()).clone();

            let now = Instant::now();
            let elapsed = now.duration_since(last_at).as_secs_f64().max(0.001);
            let net = (crate::metrics::bytes_in(), crate::metrics::bytes_out());
            let net_in = ((net.0.saturating_sub(last_net.0)) as f64 / elapsed) as u64;
            let net_out = ((net.1.saturating_sub(last_net.1)) as f64 / elapsed) as u64;
            last_net = net;
            last_at = now;

            let worlds = collect_worlds(&server, &last_worlds, &disk.per_world);
            last_worlds.clone_from(&worlds);

            snapshot.store(Arc::new(Snapshot {
                server_ready: true,
                cpu_total: stats.cpu_total,
                cpu_per_core: stats.cpu_per_core,
                cpu_temp_c: stats.cpu_temp_c,
                mem_process_rss: stats.mem_process_rss,
                mem_system_used: stats.mem_system_used,
                mem_system_total: stats.mem_system_total,
                // `get_tps()` is the theoretical rate the measured tick time would allow, which
                // reads as thousands of TPS on an idle server.
                tps: server.get_tps().min(f64::from(server.basic_config.tps)),
                mspt: server.get_mspt(),
                tick_times_nanos: tick_times(&server),
                players: collect_players(&server),
                worlds,
                worlds_size_bytes: disk.worlds_size,
                disk_free: disk.free,
                disk_total: disk.total,
                net_in_bps: net_in,
                net_out_bps: net_out,
                uptime_secs: started.elapsed().as_secs(),
                meta: meta.clone(),
            }));

            tokio::select! {
                () = tokio::time::sleep(interval) => {},
                () = STOP_INTERRUPT.cancelled() => break,
            }
        }
    });
}

fn spawn_disk_scanner(server: &Arc<Server>, disk: &Arc<ArcSwap<DiskUsage>>, scan_secs: u64) {
    let server = server.clone();
    let disk = disk.clone();
    let interval = Duration::from_secs(scan_secs.max(5));

    server.clone().spawn_task(async move {
        while !SHOULD_STOP.load(Ordering::Relaxed) {
            let scans: Vec<WorldScan> = server
                .worlds
                .load()
                .iter()
                .map(|world| {
                    let folder = &world.level.level_folder;
                    WorldScan {
                        dimension: world.dimension.minecraft_name.to_owned(),
                        root: folder.root_folder.clone(),
                        region: folder.region_folder.clone(),
                        entities: folder.entities_folder.clone(),
                        poi: folder.poi_folder.clone(),
                    }
                })
                .collect();

            // Walking a world directory is slow and blocking; keep it off the async runtime.
            let scanned = tokio::task::spawn_blocking(move || {
                let mut system = SystemSampler::new();
                let free = scans
                    .first()
                    .and_then(|scan| system.disk_space_for(&scan.root))
                    .unwrap_or_default();

                // Keyed by dimension: every world shares the same root folder, so a name lookup
                // would assign the overworld total to the Nether and the End as well.
                let per_world: Vec<(String, u64)> = scans
                    .iter()
                    .map(|scan| (scan.dimension.clone(), scan.dimension_size()))
                    .collect();

                // Unique roots so playerdata and level.dat are not counted once per dimension.
                let mut roots: Vec<std::path::PathBuf> =
                    scans.iter().map(|s| s.root.clone()).collect();
                roots.sort();
                roots.dedup();
                let worlds_size = roots
                    .iter()
                    .map(|root| pumpkin_gui::directory_size(root))
                    .sum();

                DiskUsage {
                    worlds_size: Some(worlds_size),
                    per_world,
                    free: free.free,
                    total: free.total,
                }
            })
            .await;

            if let Ok(scanned) = scanned {
                disk.store(Arc::new(scanned));
            }

            tokio::select! {
                () = tokio::time::sleep(interval) => {},
                () = STOP_INTERRUPT.cancelled() => break,
            }
        }
    });
}

/// The server's tick ring buffer, rotated so index 0 is the oldest sample.
fn tick_times(server: &Server) -> Vec<i64> {
    let times = server.get_tick_times_nanos_copy();
    let count = server.tick_count.load(Ordering::Relaxed);

    if count <= 0 {
        return Vec::new();
    }

    let filled = (count as usize).min(times.len());
    let next = (count as usize) % times.len();

    if filled < times.len() {
        // Still filling for the first time: entries beyond `filled` are zeroes, not old samples.
        return times[..filled].to_vec();
    }

    times[next..]
        .iter()
        .chain(&times[..next])
        .copied()
        .collect()
}

fn collect_players(server: &Server) -> Vec<PlayerRow> {
    let mut rows = Vec::new();

    // `for_each_player` avoids the Vec that `get_all_players` allocates on every sample.
    server.for_each_player(|player| {
        rows.push(PlayerRow {
            name: player.gameprofile.name.clone(),
            uuid: player.gameprofile.id.to_string(),
            ping_ms: i32::try_from(player.ping.load(Ordering::Relaxed)).unwrap_or(i32::MAX),
            dimension: player.world().dimension.minecraft_name.to_owned(),
            gamemode: format!("{:?}", player.gamemode.load()).to_lowercase(),
            online_secs: player.joined_at.elapsed().as_secs(),
        });
    });

    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

fn collect_worlds(
    server: &Server,
    previous: &[WorldRow],
    sizes: &[(String, u64)],
) -> Vec<WorldRow> {
    server
        .worlds
        .load()
        .iter()
        .map(|world| {
            let name = world.get_world_name().to_owned();
            let loaded_chunks = world.active_chunks.try_read().map_or_else(
                |_| {
                    previous
                        .iter()
                        .find(|row| row.name == name)
                        .map_or(0, |row| row.loaded_chunks)
                },
                |chunks| chunks.len(),
            );

            let (time_of_day, weather) = world_time_and_weather(world);
            let dimension = world.dimension.minecraft_name.to_owned();
            let size_bytes = sizes
                .iter()
                .find(|(dim, _)| dim == &dimension)
                .map(|(_, size)| *size)
                .or_else(|| {
                    previous
                        .iter()
                        .find(|row| row.dimension == dimension)
                        .and_then(|row| row.size_bytes)
                });

            WorldRow {
                name,
                dimension,
                loaded_chunks,
                entities: world.entities.load().len(),
                size_bytes,
                time_of_day,
                weather,
            }
        })
        .collect()
}

fn world_time_and_weather(world: &crate::world::World) -> (i64, String) {
    let time_of_day = world.level_time.lock().map_or(0, |time| time.time_of_day);

    let weather = world.weather.lock().map_or_else(
        |_| "unknown".to_owned(),
        |weather| {
            if weather.thundering {
                "thunder".to_owned()
            } else if weather.raining {
                "rain".to_owned()
            } else {
                "clear".to_owned()
            }
        },
    );

    (time_of_day, weather)
}

fn server_meta(server: &Server) -> ServerMeta {
    let networking = &server.advanced_config.networking;

    ServerMeta {
        pumpkin_version: env!("CARGO_PKG_VERSION").to_owned(),
        commit: env!("GIT_HASH").to_owned(),
        java_version: pumpkin_data::packet::CURRENT_MC_VERSION
            .protocol_version()
            .to_string(),
        bedrock_version: pumpkin_world::CURRENT_BEDROCK_MC_VERSION.to_owned(),
        java_address: if networking.java.enabled {
            networking.java.address.to_string()
        } else {
            String::new()
        },
        bedrock_address: if networking.bedrock.enabled {
            networking.bedrock.nethernet.address.to_string()
        } else {
            String::new()
        },
        cpu_cores: std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
        tick_budget_ms: 1000.0 / f64::from(server.basic_config.tps).max(1.0),
    }
}
