use crate::error::ModError;
use crate::manifest::{CodeKind, Manifest, ModsState, Side};
use crate::pack::compile_crate;
use crate::target::{host_triple, is_wasm, lib_file_name};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::{info, warn};
use zip::ZipArchive;

/// One timed step for the loading screen / server log.
#[derive(Debug, Clone)]
pub struct TimedStep {
    pub id: String,
    pub step: String,
    pub millis: u128,
    pub cached: bool,
}

/// A zip after extract and compile.
#[derive(Debug, Clone)]
pub struct PreparedMod {
    pub manifest: Manifest,
    pub zip_path: PathBuf,
    pub extracted: PathBuf,
    pub native_lib: Option<PathBuf>,
    pub server_wasm: Option<PathBuf>,
    pub client_wasm: Option<PathBuf>,
    pub load_wasm: Option<PathBuf>,
    pub steps: Vec<TimedStep>,
}

/// Whole `mods/` folder.
#[derive(Debug, Clone, Default)]
pub struct PrepareReport {
    pub mods: Vec<PreparedMod>,
    pub steps: Vec<TimedStep>,
    pub total_millis: u128,
}

/// Live line for the launcher overlay.
#[derive(Debug, Clone)]
pub struct PrepareEvent {
    pub id: String,
    pub message: String,
}

/// Where to look and where to write cache.
#[derive(Debug, Clone)]
pub struct PrepareOpts {
    pub mods_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub plugins_dir: Option<PathBuf>,
    pub compile_source: bool,
    pub side: Side,
}

impl PrepareOpts {
    #[must_use]
    pub fn for_game_dir(game_dir: &Path) -> Self {
        Self {
            mods_dir: game_dir.join("mods"),
            cache_dir: game_dir.join("mods").join(".cache"),
            plugins_dir: Some(game_dir.join("plugins")),
            compile_source: true,
            side: Side::Both,
        }
    }
}

/// Scan `mods/*.zip`, extract, compile source or pick prebuilt, copy wasm into `plugins/`.
pub fn prepare(
    opts: &PrepareOpts,
    mut on_event: impl FnMut(PrepareEvent),
) -> Result<PrepareReport, ModError> {
    let started = Instant::now();
    std::fs::create_dir_all(&opts.mods_dir)?;
    std::fs::create_dir_all(&opts.cache_dir)?;
    let state = load_state(opts.mods_dir.parent().unwrap_or(&opts.mods_dir))?;
    let mut report = PrepareReport::default();
    let mut zips: Vec<PathBuf> = std::fs::read_dir(&opts.mods_dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")))
        .collect();
    zips.sort();
    for zip_path in zips {
        match prepare_one(opts, &state, &zip_path, &mut on_event) {
            Ok(Some(prepared)) => {
                report.steps.extend(prepared.steps.iter().cloned());
                report.mods.push(prepared);
            }
            Ok(None) => {}
            Err(err) => {
                warn!(target: "pumpkin_mod", "{}: {err}", zip_path.display());
                on_event(PrepareEvent {
                    id: zip_path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    message: err.to_string(),
                });
            }
        }
    }
    report.total_millis = started.elapsed().as_millis();
    info!(
        target: "pumpkin_mod",
        "mods ready in {}ms ({} zip{})",
        report.total_millis,
        report.mods.len(),
        if report.mods.len() == 1 { "" } else { "s" }
    );
    Ok(report)
}

fn load_state(game_dir: &Path) -> Result<ModsState, ModError> {
    let path = game_dir.join("mods.json");
    if !path.is_file() {
        return Ok(ModsState::default());
    }
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

/// Write which mod ids are disabled.
pub fn save_state(game_dir: &Path, state: &ModsState) -> Result<(), ModError> {
    let path = game_dir.join("mods.json");
    let json = serde_json::to_string_pretty(state).map_err(|e| ModError::message(e.to_string()))?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Extracted zip-mod folders under `mods/.cache` that are still enabled.
#[must_use]
pub fn extracted_mods(cache_dir: &Path) -> Vec<PathBuf> {
    let game_dir = cache_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or(cache_dir);
    let state = load_state(game_dir).unwrap_or_else(|_| ModsState::default());
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("melon-mod.toml").is_file())
        .collect();
    dirs.sort();
    dirs.into_iter()
        .filter(|dir| {
            let Ok(text) = std::fs::read_to_string(dir.join("melon-mod.toml")) else {
                return false;
            };
            Manifest::parse(&text).is_ok_and(|m| state.is_enabled(&m.id))
        })
        .collect()
}

/// List zips and their manifests without compiling.
pub fn list_mods(mods_dir: &Path) -> Vec<(PathBuf, Result<Manifest, String>)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(mods_dir) else {
        return out;
    };
    let mut zips: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")))
        .collect();
    zips.sort();
    for path in zips {
        let manifest = read_manifest(&path).map_err(|e| e.to_string());
        out.push((path, manifest));
    }
    out
}

fn read_manifest(zip_path: &Path) -> Result<Manifest, ModError> {
    let file = File::open(zip_path)?;
    let mut archive = ZipArchive::new(file)?;
    let mut entry = archive
        .by_name("melon-mod.toml")
        .map_err(|_| ModError::MissingManifest {
            id: zip_path.display().to_string(),
        })?;
    let mut text = String::new();
    entry.read_to_string(&mut text)?;
    Ok(Manifest::parse(&text)?)
}

#[allow(clippy::too_many_lines)]
fn prepare_one(
    opts: &PrepareOpts,
    state: &ModsState,
    zip_path: &Path,
    on_event: &mut impl FnMut(PrepareEvent),
) -> Result<Option<PreparedMod>, ModError> {
    let manifest = read_manifest(zip_path)?;
    if !state.is_enabled(&manifest.id) {
        on_event(PrepareEvent {
            id: manifest.id,
            message: "disabled".into(),
        });
        return Ok(None);
    }
    let want_server = opts.side.on_server() && manifest.side.on_server();
    let want_client = opts.side.on_client() && manifest.side.on_client();
    if !want_server && !want_client {
        return Ok(None);
    }

    on_event(PrepareEvent {
        id: manifest.id.clone(),
        message: "extract".into(),
    });
    let hash = sha256_file(zip_path)?;
    let extracted = opts
        .cache_dir
        .join(format!("{}-{}", manifest.id, manifest.version));
    let stamp = extracted.join(".zipsha");
    let mut steps = Vec::new();
    let extract_started = Instant::now();
    let cached_extract = stamp.is_file()
        && std::fs::read_to_string(&stamp).is_ok_and(|s| s.trim() == hash)
        && extracted.join("melon-mod.toml").is_file();
    if !cached_extract {
        if extracted.exists() {
            std::fs::remove_dir_all(&extracted)?;
        }
        extract_zip(zip_path, &extracted)?;
        std::fs::write(&stamp, &hash)?;
    }
    steps.push(TimedStep {
        id: manifest.id.clone(),
        step: "extract".into(),
        millis: extract_started.elapsed().as_millis(),
        cached: cached_extract,
    });

    let host = host_triple();
    let mut native_lib = prebuilt_lib(&extracted, &manifest.id, host);
    let mut server_wasm = existing_wasm(&extracted, "server.wasm");
    let client_wasm = existing_wasm(&extracted, "client.wasm");
    let load_wasm = existing_wasm(&extracted, "load.wasm");

    if opts.compile_source && extracted.join("src").join("Cargo.toml").is_file() {
        let crate_dir = extracted.join("src");
        if native_lib.is_none() && !is_wasm(host) {
            on_event(PrepareEvent {
                id: manifest.id.clone(),
                message: format!("rustc {host}"),
            });
            let (path, millis, cached) =
                compile_or_cache(&crate_dir, &manifest.id, host, &opts.cache_dir)?;
            native_lib = Some(path);
            steps.push(TimedStep {
                id: manifest.id.clone(),
                step: format!("rustc {host}"),
                millis,
                cached,
            });
        }
        if server_wasm.is_none() {
            on_event(PrepareEvent {
                id: manifest.id.clone(),
                message: "rustc wasm32-wasip2".into(),
            });
            match compile_or_cache(&crate_dir, &manifest.id, "wasm32-wasip2", &opts.cache_dir) {
                Ok((path, millis, cached)) => {
                    server_wasm = Some(path);
                    steps.push(TimedStep {
                        id: manifest.id.clone(),
                        step: "rustc wasm32-wasip2".into(),
                        millis,
                        cached,
                    });
                }
                Err(err) => {
                    warn!(
                        target: "pumpkin_mod",
                        "{}: wasm compile skipped: {err}",
                        manifest.id
                    );
                    on_event(PrepareEvent {
                        id: manifest.id.clone(),
                        message: format!("wasm skipped: {err}"),
                    });
                }
            }
        }
    }

    if let (Some(plugins), Some(wasm)) = (&opts.plugins_dir, &server_wasm)
        && want_server
    {
        std::fs::create_dir_all(plugins)?;
        let dest = plugins.join(format!("{}.wasm", manifest.id));
        std::fs::copy(wasm, dest)?;
    }

    if matches!(manifest.code, CodeKind::Prebuilt) && native_lib.is_none() && want_server {
        return Err(ModError::MissingPrebuilt {
            triple: host.to_owned(),
            path: zip_path.to_path_buf(),
        });
    }

    for step in &steps {
        info!(
            target: "pumpkin_mod",
            "{} {}: {}ms{}",
            step.id,
            step.step,
            step.millis,
            if step.cached { " (cache)" } else { "" }
        );
        on_event(PrepareEvent {
            id: step.id.clone(),
            message: format!(
                "{} {}ms{}",
                step.step,
                step.millis,
                if step.cached { " (cache)" } else { "" }
            ),
        });
    }

    Ok(Some(PreparedMod {
        manifest,
        zip_path: zip_path.to_path_buf(),
        extracted,
        native_lib,
        server_wasm,
        client_wasm,
        load_wasm,
        steps,
    }))
}

fn compile_or_cache(
    crate_dir: &Path,
    id: &str,
    triple: &str,
    cache_dir: &Path,
) -> Result<(PathBuf, u128, bool), ModError> {
    let dest_dir = cache_dir.join(id).join(triple);
    std::fs::create_dir_all(&dest_dir)?;
    let dest = if is_wasm(triple) {
        dest_dir.join("server.wasm")
    } else {
        dest_dir.join(lib_file_name(id, triple))
    };
    let src_hash = hash_tree(crate_dir)?;
    let stamp = dest_dir.join(".srchash");
    if dest.is_file()
        && stamp.is_file()
        && std::fs::read_to_string(&stamp).is_ok_and(|s| s == src_hash)
    {
        return Ok((dest, 0, true));
    }
    let started = Instant::now();
    let artifact = compile_crate(crate_dir, id, triple)?;
    std::fs::copy(&artifact, &dest)?;
    std::fs::write(stamp, src_hash)?;
    Ok((dest, started.elapsed().as_millis(), false))
}

fn prebuilt_lib(extracted: &Path, id: &str, triple: &str) -> Option<PathBuf> {
    let path = extracted
        .join("native")
        .join(triple)
        .join(lib_file_name(id, triple));
    path.is_file().then_some(path)
}

fn existing_wasm(extracted: &Path, name: &str) -> Option<PathBuf> {
    let path = extracted.join("wasm").join(name);
    path.is_file().then_some(path)
}

fn sha256_file(path: &Path) -> Result<String, ModError> {
    let mut hasher = Sha256::new();
    let mut file = File::open(path)?;
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn hash_tree(dir: &Path) -> Result<String, ModError> {
    let mut hasher = Sha256::new();
    hash_walk(&mut hasher, dir)?;
    Ok(hex::encode(hasher.finalize()))
}

fn hash_walk(hasher: &mut Sha256, dir: &Path) -> Result<(), ModError> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        let path = entry.path();
        hasher.update(name.to_string_lossy().as_bytes());
        if path.is_dir() {
            hash_walk(hasher, &path)?;
        } else {
            hasher.update(std::fs::read(path)?);
        }
    }
    Ok(())
}

fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), ModError> {
    std::fs::create_dir_all(dest)?;
    let file = File::open(zip_path)?;
    let mut archive = ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        if rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            continue;
        }
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut dest_file = File::create(&out)?;
        std::io::copy(&mut entry, &mut dest_file)?;
        dest_file.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::pack_dir;
    use std::io::Write;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    #[test]
    fn content_zip_extracts_without_rustc() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path();
        let mods = game.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        let zip_path = mods.join("hello.zip");
        let file = File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file("melon-mod.toml", options).unwrap();
        zip.write_all(b"id = \"hello\"\nversion = \"0.1.0\"\n")
            .unwrap();
        zip.start_file("data/hello/note.txt", options).unwrap();
        zip.write_all(b"hi").unwrap();
        zip.finish().unwrap();

        let report = prepare(&PrepareOpts::for_game_dir(game), |_| {}).unwrap();
        assert_eq!(report.mods.len(), 1);
        assert_eq!(report.mods[0].manifest.id, "hello");
        assert!(
            report.mods[0]
                .extracted
                .join("data/hello/note.txt")
                .is_file()
        );
        assert!(report.total_millis < 60_000);
    }

    #[test]
    fn pack_dir_writes_a_zip_with_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("hello");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("melon-mod.toml"),
            "id = \"hello\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("assets/hello")).unwrap();
        std::fs::write(root.join("assets/hello/a.txt"), "x").unwrap();
        let out = dir.path().join("hello.zip");
        pack_dir(&root, &out, &[]).unwrap();
        assert!(out.is_file());
        let m = read_manifest(&out).unwrap();
        assert_eq!(m.id, "hello");
    }

    #[test]
    fn stealframe_assets_extract() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path();
        let mods = game.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        let zip_path = mods.join("hello.zip");
        let file = File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file("melon-mod.toml", options).unwrap();
        zip.write_all(b"id = \"hello\"\nversion = \"0.1.0\"\n")
            .unwrap();
        zip.start_file("assets/hello/blockstates/stealframe.json", options)
            .unwrap();
        zip.write_all(br#"{"variants":{"":{"model":"hello:block/stealframe"}}}"#)
            .unwrap();
        zip.start_file("assets/hello/models/block/stealframe.json", options)
            .unwrap();
        zip.write_all(
            br#"{"parent":"minecraft:block/cube_all","textures":{"all":"hello:block/stealframe"}}"#,
        )
        .unwrap();
        zip.start_file("assets/hello/lang/en_us.json", options)
            .unwrap();
        zip.write_all(br#"{"block.hello.stealframe":"Stealframe"}"#)
            .unwrap();
        zip.start_file("data/hello/blocks/stealframe.json", options)
            .unwrap();
        zip.write_all(br#"{"id":"hello:stealframe","name":"Stealframe"}"#)
            .unwrap();
        zip.finish().unwrap();

        let report = prepare(&PrepareOpts::for_game_dir(game), |_| {}).unwrap();
        let extracted = &report.mods[0].extracted;
        assert!(
            extracted
                .join("assets/hello/blockstates/stealframe.json")
                .is_file()
        );
        assert!(
            extracted
                .join("data/hello/blocks/stealframe.json")
                .is_file()
        );
        let lang = std::fs::read_to_string(extracted.join("assets/hello/lang/en_us.json")).unwrap();
        assert!(lang.contains("stealframe"), "{lang}");
    }

    #[test]
    fn extracted_mods_lists_enabled_cache() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path();
        let cache = game.join("mods").join(".cache");
        let hello = cache.join("hello-0.1.0");
        std::fs::create_dir_all(&hello).unwrap();
        std::fs::write(
            hello.join("melon-mod.toml"),
            "id = \"hello\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let listed = extracted_mods(&cache);
        assert_eq!(listed, vec![hello]);
    }
}
