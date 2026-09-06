//! Turns live server state into [`Snapshot`]s for the window.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use pumpkin_util::permission::PermissionLvl;
use uuid::Uuid;

use arc_swap::ArcSwap;
use pumpkin_config::gui::GuiConfig;
use pumpkin_gui_api::{
    LogRing, PlayerRow, ServerMessage, ServerMeta, Snapshot, SystemSampler, WorldRow,
};

use super::ipc::Broadcaster;
use crate::net::ClientPlatform;
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
        pumpkin_gui_api::directory_size(&self.region)
            + pumpkin_gui_api::directory_size(&self.entities)
            + pumpkin_gui_api::directory_size(&self.poi)
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

pub fn spawn(server: &Arc<Server>, ring: &Arc<LogRing>, tx: &Broadcaster, config: &GuiConfig) {
    let disk = Arc::new(ArcSwap::from_pointee(DiskUsage::default()));

    spawn_disk_scanner(server, &disk, config.disk_scan_secs);
    spawn_fast_sampler(server, ring, tx, &disk, config.refresh_ms);
}

fn spawn_fast_sampler(
    server: &Arc<Server>,
    ring: &Arc<LogRing>,
    tx: &Broadcaster,
    disk: &Arc<ArcSwap<DiskUsage>>,
    refresh_ms: u64,
) {
    let server = server.clone();
    let ring = ring.clone();
    let tx = tx.clone();
    let disk = disk.clone();
    // The OS reports CPU use as a delta between samples, so anything under Sysinfo's minimum
    // interval would report noise.
    let interval = Duration::from_millis(refresh_ms.max(200));

    server.clone().spawn_task(async move {
        let mut system = SystemSampler::new();
        let started = Instant::now();

        let mut last_net = (crate::metrics::bytes_in(), crate::metrics::bytes_out());
        let mut last_at = Instant::now();
        // Reused when `active_chunks` is being written and a read would block the tick.
        let mut last_worlds: Vec<WorldRow> = Vec::new();
        let mut log_cursor = 0u64;

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

            let players = collect_players(&server);
            let worlds = collect_worlds(&server, &last_worlds, &disk.per_world, &players);
            last_worlds.clone_from(&worlds);

            let _ = tx.send(ServerMessage::Snapshot(Snapshot {
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
                players,
                worlds,
                worlds_size_bytes: disk.worlds_size,
                disk_free: disk.free,
                disk_total: disk.total,
                net_in_bps: net_in,
                net_out_bps: net_out,
                uptime_secs: started.elapsed().as_secs(),
            }));

            let mut new_lines = Vec::new();
            log_cursor = ring.drain_since(log_cursor, &mut new_lines);
            if !new_lines.is_empty() {
                let _ = tx.send(ServerMessage::LogLines(new_lines));
            }

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
                    .map(|root| pumpkin_gui_api::directory_size(root))
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

fn blank_player(uuid: Uuid, name: String) -> PlayerRow {
    PlayerRow {
        name,
        uuid: uuid.to_string(),
        edition: String::new(),
        ping_ms: -1,
        dimension: String::new(),
        gamemode: String::new(),
        online_secs: 0,
        online: false,
        operator: false,
        banned: false,
        whitelisted: false,
    }
}

fn collect_players(server: &Server) -> Vec<PlayerRow> {
    let mut by_uuid: HashMap<Uuid, PlayerRow> = HashMap::new();

    // `for_each_player` avoids the Vec that `get_all_players` allocates on every sample.
    server.for_each_player(|player| {
        let uuid = player.gameprofile.id;
        let row = by_uuid
            .entry(uuid)
            .or_insert_with(|| blank_player(uuid, player.gameprofile.name.clone()));
        row.name.clone_from(&player.gameprofile.name);
        row.edition = match player.client.as_ref() {
            ClientPlatform::Java(_) => "java".to_owned(),
            ClientPlatform::Bedrock(_) => "bedrock".to_owned(),
        };
        row.online = true;
        row.ping_ms = i32::try_from(player.ping.load(Ordering::Relaxed)).unwrap_or(i32::MAX);
        player
            .world()
            .dimension
            .minecraft_name
            .clone_into(&mut row.dimension);
        row.gamemode = format!("{:?}", player.gamemode.load()).to_lowercase();
        row.online_secs = player.joined_at.elapsed().as_secs();
        row.operator |= player.permission_lvl.load() != PermissionLvl::Zero;
    });

    if let Ok(ops) = server.data.operator_config.read() {
        for entry in &ops.ops {
            let row = by_uuid
                .entry(entry.uuid)
                .or_insert_with(|| blank_player(entry.uuid, entry.name.clone()));
            if row.name.is_empty() {
                row.name.clone_from(&entry.name);
            }
            row.operator = true;
        }
    }

    if let Ok(bans) = server.data.banned_player_list.read() {
        for entry in &bans.banned_players {
            let row = by_uuid
                .entry(entry.uuid)
                .or_insert_with(|| blank_player(entry.uuid, entry.name.clone()));
            if row.name.is_empty() {
                row.name.clone_from(&entry.name);
            }
            row.banned = true;
        }
    }

    if let Ok(whitelist) = server.data.whitelist_config.read() {
        for entry in &whitelist.whitelist {
            let row = by_uuid
                .entry(entry.uuid)
                .or_insert_with(|| blank_player(entry.uuid, entry.name.clone()));
            if row.name.is_empty() {
                row.name.clone_from(&entry.name);
            }
            row.whitelisted = true;
        }
    }

    if let Ok(cache) = server.data.user_cache.read() {
        for (uuid, name, edition) in cache.iter_profiles() {
            let row = by_uuid
                .entry(uuid)
                .or_insert_with(|| blank_player(uuid, name.to_owned()));
            if row.name.is_empty() {
                name.clone_into(&mut row.name);
            }
            if row.edition.is_empty()
                && let Some(edition) = edition
            {
                edition.clone_into(&mut row.edition);
            }
        }
    }

    let mut rows: Vec<PlayerRow> = by_uuid.into_values().collect();
    // Java + Bedrock on the same gamertag must stay in a total order
    rows.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.edition.cmp(&right.edition))
            .then_with(|| left.uuid.cmp(&right.uuid))
    });
    rows
}

fn collect_worlds(
    server: &Server,
    previous: &[WorldRow],
    sizes: &[(String, u64)],
    players: &[PlayerRow],
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

            let player_count = players
                .iter()
                .filter(|player| player.online && player.dimension == dimension)
                .count();

            WorldRow {
                name,
                dimension,
                loaded_chunks,
                entities: world.entities.load().len(),
                players: player_count,
                size_bytes,
                time_of_day,
                weather,
            }
        })
        .collect()
}

fn world_time_and_weather(world: &crate::world::World) -> (i64, String) {
    let time_of_day = world.level_time.lock().map_or(0, |time| time.time_of_day);

    // Weather rides the overworld's day/night timeline, so `timelines` (not `has_skylight` -
    // the end also has a skylight for lighting purposes) is what actually distinguishes it: only
    // dimensions sharing that timeline (overworld, overworld_caves) have weather in vanilla.
    let weather = if world.dimension.timelines == Some("#minecraft:in_overworld") {
        world.weather.lock().map_or_else(
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
        )
    } else {
        "none".to_owned()
    };

    (time_of_day, weather)
}

pub(super) fn server_meta(server: &Server) -> ServerMeta {
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
