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
            .push(LogLevel::Info, "demo".to_owned(), &format!("> {line}"));
    }

    fn completions(&self, line: &str, _cursor: usize) -> Vec<String> {
        ["tps", "list", "help", "stop"]
            .into_iter()
            .filter(|candidate| candidate.starts_with(line))
            .map(str::to_owned)
            .collect()
    }

    fn request_stop(&self) {
        self.logs
            .push(LogLevel::Warn, "demo".to_owned(), "stop requested");
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

    // Shows off what `to_pretty_console()`-style ANSI (colour, bold) and an OSC 8 hyperlink look
    // like once parsed, plus a bare URL that gets auto-linkified with no escape codes at all.
    side.logs.push(
        LogLevel::Info,
        "demo".to_owned(),
        "\u{1b}[1m\u{1b}[32mWelcome\u{1b}[0m to \u{1b}]8;;https://pumpkinmc.org\u{1b}\\Pumpkin\u{1b}]8;;\u{1b}\\! See https://example.com/docs for details.",
    );

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
            worlds: vec![
                WorldRow {
                    name: "world".to_owned(),
                    dimension: "minecraft:overworld".to_owned(),
                    loaded_chunks: 1024,
                    entities: 137,
                    players: 2,
                    size_bytes: Some(512 * 1024 * 1024),
                    time_of_day: elapsed as i64 * 2000,
                    weather: "clear".to_owned(),
                },
                WorldRow {
                    name: "world".to_owned(),
                    dimension: "minecraft:the_nether".to_owned(),
                    loaded_chunks: 86,
                    entities: 12,
                    players: 1,
                    size_bytes: Some(48 * 1024 * 1024),
                    time_of_day: 0,
                    weather: "none".to_owned(),
                },
                WorldRow {
                    name: "world".to_owned(),
                    dimension: "minecraft:the_end".to_owned(),
                    loaded_chunks: 24,
                    entities: 4,
                    players: 1,
                    size_bytes: Some(16 * 1024 * 1024),
                    time_of_day: 0,
                    weather: "none".to_owned(),
                },
            ],
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
            &format!("sampled at {elapsed:.1}s"),
        );

        std::thread::sleep(Duration::from_millis(500));
    }
}

fn demo_players(elapsed: f64) -> Vec<PlayerRow> {
    [
        ("Alex", "minecraft:overworld", true, true, false, true),
        ("Steve", "minecraft:the_nether", true, false, false, false),
        ("Herobrine", "minecraft:the_end", true, false, true, false),
        ("Notch", "minecraft:overworld", true, false, false, true),
        ("Dinnerbone", "", false, true, false, false),
        ("jeb_", "", false, false, false, true),
        ("Grumm", "", false, false, true, false),
    ]
    .into_iter()
    .enumerate()
    .map(
        |(index, (name, dimension, online, operator, banned, whitelisted))| PlayerRow {
            name: name.to_owned(),
            uuid: format!("00000000-0000-0000-0000-00000000000{index}"),
            edition: if index % 2 == 0 {
                "java".to_owned()
            } else {
                "bedrock".to_owned()
            },
            ping_ms: if online {
                20 + i32::try_from(index).unwrap_or(0) * 15
            } else {
                -1
            },
            dimension: dimension.to_owned(),
            gamemode: if online {
                "survival".to_owned()
            } else {
                String::new()
            },
            online_secs: if online { elapsed as u64 } else { 0 },
            online,
            operator,
            banned,
            whitelisted,
        },
    )
    .collect()
}
