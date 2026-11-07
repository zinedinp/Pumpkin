//! C ABI intake: `dlopen` a cdylib, check `pumpkin_mod_abi`, call `pumpkin_mod_init`.
//! The library stays mapped until process exit (`dlclose` is unsound with `'static`).
//!
//! No Rust types cross the line. The vtable is `#[repr(C)]`, `'static`, and
//! sized so a newer host can grow it without breaking older mods.

use crate::error::ModError;
use libloading::{Library, Symbol};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tracing::{info, warn};

/// Bump when export names or the meaning of existing vtable slots change.
pub const PUMPKIN_MOD_ABI: u32 = 1;

/// `pumpkin_mod_init` return: success.
pub const INIT_OK: i32 = 0;
/// `pumpkin_mod_init` return: `host` was null.
pub const INIT_NULL_HOST: i32 = 1;
/// `pumpkin_mod_init` return: `host.abi` did not match the mod.
pub const INIT_ABI_MISMATCH: i32 = 2;
/// `pumpkin_mod_init` return: a host call (e.g. `register_block`) failed.
pub const INIT_REGISTER_FAILED: i32 = 3;

/// `register_block` return: success.
pub const REG_OK: i32 = 0;
/// `register_block` return: a pointer was null while `len > 0`.
pub const REG_NULL: i32 = 1;
/// `register_block` return: id was empty.
pub const REG_EMPTY_ID: i32 = 2;
/// `register_block` return: id or name longer than [`MAX_ABI_STRING`].
pub const REG_TOO_LONG: i32 = 3;
/// `register_block` return: this id was already registered.
pub const REG_DUPLICATE: i32 = 4;

/// Max bytes for ids and display names crossing the ABI.
pub const MAX_ABI_STRING: usize = 256;
/// Max bytes accepted by [`host_log`]. Longer buffers are truncated.
pub const MAX_LOG: usize = 64 * 1024;

/// Host functions compiled into Pumpkin/Melon. No Rust types cross the line.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostVTable {
    pub abi: u32,
    /// `size_of::<HostVTable>()` of the host. A mod must not call a slot
    /// whose offset is `>= size`.
    pub size: u32,
    pub log: unsafe extern "C" fn(*const u8, usize),
    pub register_block: unsafe extern "C" fn(*const u8, usize, *const u8, usize) -> i32,
}

/// A block the mod registered during `pumpkin_mod_init`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredBlock {
    pub id: String,
    pub name: String,
}

const HOST: HostVTable = HostVTable {
    abi: PUMPKIN_MOD_ABI,
    size: size_of::<HostVTable>() as u32,
    log: host_log,
    register_block: host_register_block,
};

type InitFn = unsafe extern "C" fn(*const HostVTable) -> i32;

/// A loaded cdylib kept alive for the process.
pub struct LoadedLib {
    pub path: PathBuf,
    _library: Library,
}

static LOADED: OnceLock<Mutex<Vec<LoadedLib>>> = OnceLock::new();
static BLOCKS: OnceLock<Mutex<Vec<RegisteredBlock>>> = OnceLock::new();

fn loaded() -> &'static Mutex<Vec<LoadedLib>> {
    LOADED.get_or_init(|| Mutex::new(Vec::new()))
}

fn blocks() -> &'static Mutex<Vec<RegisteredBlock>> {
    BLOCKS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Blocks registered by native mods since process start.
#[must_use]
pub fn registered_blocks() -> Vec<RegisteredBlock> {
    blocks()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// Borrow host memory for `len` bytes. `len == 0` is always empty, even if
/// `ptr` is null. `len > max` is an error (callers truncate logs themselves).
unsafe fn host_slice<'a>(ptr: *const u8, len: usize, max: usize) -> Result<&'a [u8], SliceError> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        return Err(SliceError::Null);
    }
    if len > max {
        return Err(SliceError::TooLong);
    }
    // Reject wrapping so `from_raw_parts` cannot form a slice past the
    // address space.
    if ptr.wrapping_add(len) < ptr {
        return Err(SliceError::TooLong);
    }
    // SAFETY: caller (the mod) owns `ptr[0..len]` for this call; `len` is
    // capped and the range does not wrap.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

enum SliceError {
    Null,
    TooLong,
}

unsafe extern "C" fn host_log(ptr: *const u8, len: usize) {
    let len = len.min(MAX_LOG);
    // SAFETY: the mod passes a buffer it owns for this call, or null+0.
    let bytes = match unsafe { host_slice(ptr, len, MAX_LOG) } {
        Ok(bytes) => bytes,
        Err(_) => {
            warn!(target: "pumpkin_mod", "log: invalid pointer/len");
            return;
        }
    };
    if bytes.is_empty() {
        return;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => info!(target: "pumpkin_mod", "{text}"),
        Err(_) => info!(target: "pumpkin_mod", "{}", String::from_utf8_lossy(bytes)),
    }
}

unsafe extern "C" fn host_register_block(
    id_ptr: *const u8,
    id_len: usize,
    name_ptr: *const u8,
    name_len: usize,
) -> i32 {
    // SAFETY: the mod owns both buffers for this call.
    let id = match unsafe { host_slice(id_ptr, id_len, MAX_ABI_STRING) } {
        Ok(bytes) => bytes,
        Err(SliceError::Null) => return REG_NULL,
        Err(SliceError::TooLong) => return REG_TOO_LONG,
    };
    let name = match unsafe { host_slice(name_ptr, name_len, MAX_ABI_STRING) } {
        Ok(bytes) => bytes,
        Err(SliceError::Null) => return REG_NULL,
        Err(SliceError::TooLong) => return REG_TOO_LONG,
    };
    if id.is_empty() {
        return REG_EMPTY_ID;
    }
    let id = String::from_utf8_lossy(id).into_owned();
    let name = if name.is_empty() {
        id.clone()
    } else {
        String::from_utf8_lossy(name).into_owned()
    };
    let mut guard = blocks()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.iter().any(|b| b.id == id) {
        return REG_DUPLICATE;
    }
    info!(target: "pumpkin_mod", "registered block {id} ({name})");
    guard.push(RegisteredBlock { id, name });
    REG_OK
}

fn init_reason(code: i32) -> &'static str {
    match code {
        INIT_OK => "ok",
        INIT_NULL_HOST => "null host vtable",
        INIT_ABI_MISMATCH => "host abi mismatch",
        INIT_REGISTER_FAILED => "register_block failed",
        _ => "unknown init error",
    }
}

/// Load `path`, check ABI, call `pumpkin_mod_init`.
pub fn load_native(path: &Path) -> Result<(), ModError> {
    if !path.is_file() {
        return Err(ModError::NativeNotFound {
            path: path.to_path_buf(),
        });
    }

    // Integrated server and client share the process: a lib that already
    // ran its init must not run it again (`register_block` would report a
    // duplicate).
    if loaded()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|lib| lib.path == path)
    {
        return Ok(());
    }

    // SAFETY: path is a player-installed cdylib; symbols are the C ABI contract.
    let library = unsafe { Library::new(path) }.map_err(|e| ModError::Native {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    let abi = read_abi(&library, path)?;
    if abi != PUMPKIN_MOD_ABI {
        return Err(ModError::AbiMismatch {
            path: path.to_path_buf(),
            found: abi,
            expected: PUMPKIN_MOD_ABI,
        });
    }

    let init = read_init(&library, path)?;
    // SAFETY: `init` is the exported C entry; HOST is process-static.
    let rc = unsafe { init(&HOST) };
    if rc != INIT_OK {
        return Err(ModError::InitFailed {
            path: path.to_path_buf(),
            code: rc,
            reason: init_reason(rc),
        });
    }

    loaded()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(LoadedLib {
            path: path.to_path_buf(),
            _library: library,
        });
    info!(
        target: "pumpkin_mod",
        "loaded native mod {}",
        path.display()
    );
    Ok(())
}

fn read_abi(library: &Library, path: &Path) -> Result<u32, ModError> {
    // SAFETY: `pumpkin_mod_abi` is an exported `u32` from the C ABI contract.
    let symbol: Symbol<*const u32> =
        unsafe { library.get(b"pumpkin_mod_abi\0") }.map_err(|_| ModError::MissingSymbol {
            path: path.to_path_buf(),
            symbol: "pumpkin_mod_abi",
        })?;
    let ptr = *symbol;
    if ptr.is_null() {
        return Err(ModError::MissingSymbol {
            path: path.to_path_buf(),
            symbol: "pumpkin_mod_abi",
        });
    }
    // SAFETY: `ptr` is the address of the exported `static u32`.
    Ok(unsafe { ptr.read() })
}

fn read_init(library: &Library, path: &Path) -> Result<InitFn, ModError> {
    // SAFETY: `pumpkin_mod_init` is the exported C entry from the same contract.
    let symbol: Symbol<InitFn> =
        unsafe { library.get(b"pumpkin_mod_init\0") }.map_err(|_| ModError::MissingSymbol {
            path: path.to_path_buf(),
            symbol: "pumpkin_mod_init",
        })?;
    Ok(*symbol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::{host_triple, lib_file_name};
    use std::process::Command;

    const HOST_VTABLE_SRC: &str = r#"
        #[repr(C)]
        pub struct HostVTable {
            pub abi: u32,
            pub size: u32,
            pub log: unsafe extern "C" fn(*const u8, usize),
            pub register_block: unsafe extern "C" fn(*const u8, usize, *const u8, usize) -> i32,
        }
    "#;

    struct Cdylib {
        _dir: tempfile::TempDir,
        path: PathBuf,
    }

    fn compile(src: &str) -> Cdylib {
        let dir = tempfile::tempdir().unwrap();
        let rs = dir.path().join("lib.rs");
        std::fs::write(&rs, src).unwrap();
        let path = dir.path().join(lib_file_name("t", host_triple()));
        let output = Command::new("rustc")
            .args([
                "--edition",
                "2024",
                "--crate-type",
                "cdylib",
                "-C",
                "opt-level=1",
                "-o",
            ])
            .arg(&path)
            .arg(&rs)
            .output()
            .expect("rustc");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Cdylib { _dir: dir, path }
    }

    #[test]
    fn missing_file_is_native_not_found() {
        let err = load_native(Path::new("/no/such/mod.so")).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("not found"), "{text}");
    }

    #[test]
    fn missing_abi_symbol_names_the_export() {
        let lib = compile(
            r#"
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn pumpkin_mod_init(_host: *const u8) -> i32 { 0 }
            "#,
        );
        let err = load_native(&lib.path).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("pumpkin_mod_abi"), "{text}");
        assert!(text.contains("missing export"), "{text}");
    }

    #[test]
    fn abi_mismatch_reports_both_versions() {
        let lib = compile(
            r#"
            #[unsafe(no_mangle)]
            pub static pumpkin_mod_abi: u32 = 99;
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn pumpkin_mod_init(_host: *const u8) -> i32 { 0 }
            "#,
        );
        let err = load_native(&lib.path).unwrap_err();
        match err {
            ModError::AbiMismatch {
                found: 99,
                expected: PUMPKIN_MOD_ABI,
                ..
            } => {}
            other => panic!("{other}"),
        }
        let text = err.to_string();
        assert!(text.contains("99"), "{text}");
        assert!(text.contains(&PUMPKIN_MOD_ABI.to_string()), "{text}");
    }

    #[test]
    fn init_null_host_code_is_named() {
        let lib = compile(&format!(
            r#"
            {HOST_VTABLE_SRC}
            #[unsafe(no_mangle)]
            pub static pumpkin_mod_abi: u32 = 1;
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn pumpkin_mod_init(host: *const HostVTable) -> i32 {{
                if host.is_null() {{ return 1; }}
                1
            }}
            "#
        ));
        let err = load_native(&lib.path).unwrap_err();
        match err {
            ModError::InitFailed {
                code: INIT_NULL_HOST,
                reason,
                ..
            } => assert_eq!(reason, "null host vtable"),
            other => panic!("{other}"),
        }
    }

    #[test]
    fn load_registers_stealframe_and_logs() {
        let lib = compile(&format!(
            r#"
            {HOST_VTABLE_SRC}
            #[unsafe(no_mangle)]
            pub static pumpkin_mod_abi: u32 = 1;
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn pumpkin_mod_init(host: *const HostVTable) -> i32 {{
                if host.is_null() {{ return 1; }}
                let host = unsafe {{ &*host }};
                if host.abi != pumpkin_mod_abi {{ return 2; }}
                let id = b"hello:stealframe";
                let name = b"Stealframe";
                let rc = unsafe {{ (host.register_block)(id.as_ptr(), id.len(), name.as_ptr(), name.len()) }};
                if rc != 0 {{ return 3; }}
                let msg = b"hello: stealframe ready";
                unsafe {{ (host.log)(msg.as_ptr(), msg.len()) }};
                0
            }}
            "#
        ));
        load_native(&lib.path).unwrap();
        let blocks = registered_blocks();
        assert!(
            blocks
                .iter()
                .any(|b| b.id == "hello:stealframe" && b.name == "Stealframe"),
            "{blocks:?}"
        );
    }

    #[test]
    fn duplicate_block_is_a_precise_register_error() {
        let lib = compile(&format!(
            r#"
            {HOST_VTABLE_SRC}
            #[unsafe(no_mangle)]
            pub static pumpkin_mod_abi: u32 = 1;
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn pumpkin_mod_init(host: *const HostVTable) -> i32 {{
                let host = unsafe {{ &*host }};
                let id = b"dup:block";
                let a = unsafe {{ (host.register_block)(id.as_ptr(), id.len(), std::ptr::null(), 0) }};
                let b = unsafe {{ (host.register_block)(id.as_ptr(), id.len(), std::ptr::null(), 0) }};
                if a != 0 {{ return 3; }}
                if b != 4 {{ return 3; }}
                0
            }}
            "#
        ));
        load_native(&lib.path).unwrap();
    }

    #[test]
    fn loading_the_same_lib_twice_does_not_run_init_again() {
        let lib = compile(&format!(
            r#"
            {HOST_VTABLE_SRC}
            #[unsafe(no_mangle)]
            pub static pumpkin_mod_abi: u32 = 1;
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn pumpkin_mod_init(host: *const HostVTable) -> i32 {{
                let host = unsafe {{ &*host }};
                let id = b"twice:block";
                let rc = unsafe {{ (host.register_block)(id.as_ptr(), id.len(), std::ptr::null(), 0) }};
                if rc != 0 {{ return 3; }}
                0
            }}
            "#
        ));
        load_native(&lib.path).unwrap();
        // Integrated server, then client: the second call must be a no-op.
        load_native(&lib.path).unwrap();
    }
}
