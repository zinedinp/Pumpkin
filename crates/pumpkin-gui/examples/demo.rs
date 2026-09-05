#![allow(clippy::print_stderr)]

//! Runs the window against fake server data.
//!
//! Lets the GUI be developed without starting a Minecraft server: CPU, memory and disk are real
//! (they come from the same [`SystemSampler`] the server uses), everything server-shaped is
//! synthetic.
//!
//! `cargo run -p pumpkin-gui --example demo`

use std::sync::Arc;
use std::time::{Duration, Instant};

use pumpkin_gui::{
    GuiCommands, GuiSide, LogLevel, PlayerRow, ServerMeta, Snapshot, SystemSampler,
    ThemePreference, WorldRow,
};

struct DemoCommands {
    logs: Arc<pumpkin_gui::LogRing>,
}

impl GuiCommands for DemoCommands {
    fn submit(&self, line: String) {
        self.logs
            .push(LogLevel::Info, "demo".to_owned(), format!("> {line}"));
    }

    fn completions(&self, line: &str, _cursor: usize) -> Vec<String> {
        ["tps", "list", "help", "stop"]
            .into_iter()
            .filter(|candidate| candidate.starts_with(line))
            .map(str::to_owned)
            .collect()
    }

    fn request_stop(&self) {
        self.logs.push(
            LogLevel::Warn,
            "demo".to_owned(),
            "stop requested".to_owned(),
        );
    }
}

fn main() {
    // Lets a headless capture pick the theme without a display or a mouse.
    let theme = std::env::var("PUMPKIN_GUI_THEME").map_or(ThemePreference::System, |value| {
        ThemePreference::from_config(&value)
    });
    let side = GuiSide::new(theme, 5000);

    let commands: Arc<dyn GuiCommands> = Arc::new(DemoCommands {
        logs: side.logs.clone(),
    });
    let _ = side.commands.set(commands);

    let sampler_side = side.clone();
    let sampler = std::thread::Builder::new()
        .name("demo-sampler".to_owned())
        .spawn(move || sample_loop(&sampler_side));

    if sampler.is_err() {
        return;
    }

    match pumpkin_gui::run(side) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("demo: {err}");
            std::process::exit(1);
        }
    }
}

fn sample_loop(side: &GuiSide) {
    let mut sampler = SystemSampler::new();
    let started = Instant::now();
    let meta = Arc::new(ServerMeta {
        pumpkin_version: "0.1.0-demo".to_owned(),
        commit: "deadbeef".to_owned(),
        java_version: "26.2".to_owned(),
        bedrock_version: "1.26.45".to_owned(),
        java_address: "0.0.0.0:25565".to_owned(),
        bedrock_address: "0.0.0.0:19132".to_owned(),
        cpu_cores: std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
        tick_budget_ms: 50.0,
    });

    // Real disk figures for the current directory, so `disk_space_for` is exercised too. Scanned
    // once here rather than per tick, mirroring the server's slow sampler.
    let disk = std::env::current_dir()
        .ok()
        .and_then(|cwd| sampler.disk_space_for(&cwd))
        .unwrap_or_default();

    loop {
        let stats = sampler.sample();
        let elapsed = started.elapsed().as_secs_f64();

        // A slow sine keeps the tick graph visibly moving without pretending to be a real server.
        let mspt = 25.0 + 20.0 * (elapsed / 4.0).sin();

        side.snapshot.store(Arc::new(Snapshot {
            server_ready: true,
            cpu_total: stats.cpu_total,
            cpu_per_core: stats.cpu_per_core,
            cpu_temp_c: stats.cpu_temp_c,
            mem_process_rss: stats.mem_process_rss,
            mem_system_used: stats.mem_system_used,
            mem_system_total: stats.mem_system_total,
            tps: (1000.0 / mspt).min(20.0),
            mspt,
            tick_times_nanos: (0..100)
                .map(|i| ((mspt + f64::from(i % 7)) * 1_000_000.0) as i64)
                .collect(),
            players: demo_players(elapsed),
            worlds: vec![WorldRow {
                name: "world".to_owned(),
                dimension: "minecraft:overworld".to_owned(),
                loaded_chunks: 1024,
                entities: 137,
                size_bytes: Some(512 * 1024 * 1024),
                time_of_day: (elapsed as i64 * 20) % 24_000,
                weather: "clear".to_owned(),
            }],
            worlds_size_bytes: Some(512 * 1024 * 1024),
            disk_free: disk.free,
            disk_total: disk.total,
            net_in_bps: 12_000,
            net_out_bps: 48_000,
            uptime_secs: started.elapsed().as_secs(),
            meta: meta.clone(),
        }));

        side.logs.push(
            LogLevel::Info,
            "demo".to_owned(),
            format!("sampled at {elapsed:.1}s"),
        );

        std::thread::sleep(Duration::from_millis(500));
    }
}

fn demo_players(elapsed: f64) -> Vec<PlayerRow> {
    [
        ("Alex", "minecraft:overworld"),
        ("Steve", "minecraft:the_nether"),
        ("Herobrine", "minecraft:the_end"),
        ("Notch", "minecraft:overworld"),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (name, dimension))| PlayerRow {
        name: name.to_owned(),
        uuid: format!("00000000-0000-0000-0000-00000000000{index}"),
        ping_ms: 20 + i32::try_from(index).unwrap_or(0) * 15,
        dimension: dimension.to_owned(),
        gamemode: "survival".to_owned(),
        online_secs: elapsed as u64,
    })
    .collect()
}
