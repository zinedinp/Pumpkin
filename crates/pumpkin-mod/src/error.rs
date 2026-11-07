use std::path::PathBuf;
use thiserror::Error;

/// Failures while reading a zip, compiling source, or taking a native lib.
#[derive(Debug, Error)]
pub enum ModError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("{0}")]
    Message(String),
    #[error("mod '{id}' has no melon-mod.toml")]
    MissingManifest { id: String },
    #[error("no native library for {triple} in {path:?}")]
    MissingPrebuilt { triple: String, path: PathBuf },
    #[error("{path:?}: abi {found} does not match host {expected} (export `pumpkin_mod_abi: u32 = {expected}`)")]
    AbiMismatch {
        path: PathBuf,
        found: u32,
        expected: u32,
    },
    #[error("{path:?}: missing export `{symbol}`")]
    MissingSymbol { path: PathBuf, symbol: &'static str },
    #[error("{path:?}: native library not found")]
    NativeNotFound { path: PathBuf },
    #[error("{path:?}: dlopen failed: {message}")]
    Native { path: PathBuf, message: String },
    #[error("{path:?}: pumpkin_mod_init returned {code} ({reason})")]
    InitFailed {
        path: PathBuf,
        code: i32,
        reason: &'static str,
    },
    #[error("rustc/cargo failed for {id}: {message}")]
    Compile { id: String, message: String },
}

impl ModError {
    pub fn message(text: impl Into<String>) -> Self {
        Self::Message(text.into())
    }
}
