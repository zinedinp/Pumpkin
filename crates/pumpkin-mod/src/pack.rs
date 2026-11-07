use crate::error::ModError;
use crate::target::{
    CompilePlan, cargo_target_var, compile_plan, host_triple, is_wasm, lib_file_name,
};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;
use tracing::{info, warn};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

/// One rustc/cargo step while packing.
#[derive(Debug, Clone)]
pub struct PackStep {
    pub triple: String,
    pub millis: u128,
    pub output: PathBuf,
    /// Set when this triple was not built (Apple SDK missing, …).
    pub note: Option<String>,
}

/// Pack `dir` (a crate folder with `melon-mod.toml`) into `out` zip.
/// `triples` are rustc targets; empty means content only (plus existing wasm/native).
pub fn pack_dir(dir: &Path, out: &Path, triples: &[&str]) -> Result<Vec<PackStep>, ModError> {
    let manifest_path = dir.join("melon-mod.toml");
    if !manifest_path.is_file() {
        return Err(ModError::MissingManifest {
            id: dir.display().to_string(),
        });
    }
    let manifest = crate::manifest::Manifest::parse(&std::fs::read_to_string(&manifest_path)?)?;
    let mut steps = Vec::new();
    let crate_dir = dir.join("src");
    if crate_dir.join("Cargo.toml").is_file() {
        for triple in triples {
            match compile_plan(triple) {
                CompilePlan::Skip { dest, reason } => {
                    warn!(target: "pumpkin_mod", "skipping {dest}: {reason}");
                    steps.push(PackStep {
                        triple: dest.to_owned(),
                        millis: 0,
                        output: PathBuf::new(),
                        note: Some(reason.to_owned()),
                    });
                }
                CompilePlan::Wasm { compile } => {
                    let started = Instant::now();
                    let artifact = compile_crate(&crate_dir, &manifest.id, compile)?;
                    let millis = started.elapsed().as_millis();
                    std::fs::create_dir_all(dir.join("wasm"))?;
                    let dest = dir.join("wasm").join("server.wasm");
                    std::fs::copy(&artifact, &dest)?;
                    info!(
                        target: "pumpkin_mod",
                        "packed {} for {compile} in {millis}ms",
                        manifest.id
                    );
                    steps.push(PackStep {
                        triple: compile.to_owned(),
                        millis,
                        output: dest,
                        note: None,
                    });
                }
                CompilePlan::Native { compile, dest } => {
                    let started = Instant::now();
                    let artifact = compile_crate(&crate_dir, &manifest.id, compile)?;
                    let millis = started.elapsed().as_millis();
                    let dest_dir = dir.join("native").join(dest);
                    std::fs::create_dir_all(&dest_dir)?;
                    let dest_file = dest_dir.join(lib_file_name(&manifest.id, dest));
                    std::fs::copy(&artifact, &dest_file)?;
                    let note = (compile != dest).then(|| format!("linked as {compile}"));
                    info!(
                        target: "pumpkin_mod",
                        "packed {} for {dest} in {millis}ms",
                        manifest.id
                    );
                    steps.push(PackStep {
                        triple: dest.to_owned(),
                        millis,
                        output: dest_file,
                        note,
                    });
                }
            }
        }
    }
    write_zip(dir, out)?;
    Ok(steps)
}

/// Compile the crate at `crate_dir` for `triple`. Returns the artifact path.
pub fn compile_crate(crate_dir: &Path, id: &str, triple: &str) -> Result<PathBuf, ModError> {
    let target_dir = crate_dir.join("target");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(crate_dir)
        .args(["build", "--release", "--target", triple])
        .env("CARGO_TARGET_DIR", &target_dir);
    apply_cross_linker(&mut cmd, triple);
    let output = cmd.output().map_err(|e| ModError::Compile {
        id: id.to_owned(),
        message: format!("could not run cargo: {e}"),
    })?;
    if !output.status.success() {
        return Err(ModError::Compile {
            id: id.to_owned(),
            message: compile_fail_message(triple, &String::from_utf8_lossy(&output.stderr)),
        });
    }
    let artifact = if is_wasm(triple) {
        target_dir
            .join(triple)
            .join("release")
            .join(format!("{}.wasm", id.replace('-', "_")))
    } else {
        target_dir
            .join(triple)
            .join("release")
            .join(lib_file_name(id, triple))
    };
    if !artifact.is_file() {
        return Err(ModError::Compile {
            id: id.to_owned(),
            message: format!("cargo succeeded but {} is missing", artifact.display()),
        });
    }
    Ok(artifact)
}

fn apply_cross_linker(cmd: &mut Command, triple: &str) {
    if triple == host_triple() || is_wasm(triple) {
        return;
    }
    if let Some(gcc) = linux_cross_gcc(triple).filter(|name| tool_exists(name)) {
        cmd.env(cargo_target_var(triple, "LINKER"), gcc);
        return;
    }
    if !triple.contains("linux") {
        return;
    }
    let Some(lld) = rust_lld_path() else {
        return;
    };
    cmd.env(cargo_target_var(triple, "LINKER"), lld);
    cmd.env(
        cargo_target_var(triple, "RUSTFLAGS"),
        "-C linker-flavor=ld.lld",
    );
}

fn linux_cross_gcc(triple: &str) -> Option<&'static str> {
    match triple {
        "aarch64-unknown-linux-gnu" => Some("aarch64-linux-gnu-gcc"),
        "x86_64-unknown-linux-gnu" => Some("x86_64-linux-gnu-gcc"),
        _ => None,
    }
}

fn tool_exists(name: &str) -> bool {
    Command::new(name)
        .arg("-dumpversion")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn rust_lld_path() -> Option<PathBuf> {
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sysroot = String::from_utf8(output.stdout).ok()?;
    let path = PathBuf::from(sysroot.trim())
        .join("lib/rustlib")
        .join(host_triple())
        .join("bin")
        .join("rust-lld");
    path.is_file().then_some(path)
}

fn compile_fail_message(triple: &str, stderr: &str) -> String {
    let mut message = stderr.to_owned();
    let needs_sysroot = stderr.contains("fix-cortex-a53")
        || stderr.contains("unable to find library -lc")
        || stderr.contains("cannot open crtbegin");
    if triple.contains("linux") && needs_sysroot {
        message.push_str(
            "\nnote: std crates need a matching cross-gcc (e.g. gcc-aarch64-linux-gnu). \
             no_std cdylibs link with rust-lld without a sysroot.",
        );
    }
    message
}

fn write_zip(dir: &Path, out: &Path) -> Result<(), ModError> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(out)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    add_tree(&mut zip, dir, dir, options)?;
    zip.finish()?;
    Ok(())
}

fn add_tree(
    zip: &mut ZipWriter<File>,
    root: &Path,
    dir: &Path,
    options: SimpleFileOptions,
) -> Result<(), ModError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if name == "target" || name == ".git" || name == ".cache" {
            continue;
        }
        if path.is_dir() {
            add_tree(zip, root, &path, options)?;
            continue;
        }
        let rel = path.strip_prefix(root).map_err(|_| {
            ModError::message(format!("{} is outside {}", path.display(), root.display()))
        })?;
        let name = rel.to_string_lossy().replace('\\', "/");
        zip.start_file(&name, options)?;
        let mut buf = Vec::new();
        File::open(&path)?.read_to_end(&mut buf)?;
        zip.write_all(&buf)?;
    }
    Ok(())
}
