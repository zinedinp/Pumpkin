//! Zip mods for Pumpkin and Melon: manifest, rustc of `src/` or prebuilt
//! libs per OS/Arch, C ABI intake. Does not rebuild Pumpkin.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod activate;
pub mod cabi;
pub mod error;
pub mod manifest;
pub mod pack;
pub mod prepare;
pub mod target;

pub use activate::{activate, register_data};
pub use cabi::{
    HostVTable, INIT_ABI_MISMATCH, INIT_NULL_HOST, INIT_OK, INIT_REGISTER_FAILED, MAX_ABI_STRING,
    MAX_LOG, PUMPKIN_MOD_ABI, REG_DUPLICATE, REG_EMPTY_ID, REG_NULL, REG_OK, REG_TOO_LONG,
    RegisteredBlock, load_native, registered_blocks,
};
pub use error::ModError;
pub use manifest::{CodeKind, Manifest, ModsState, Side};
pub use pack::{PackStep, compile_crate, pack_dir};
pub use prepare::{
    PrepareEvent, PrepareOpts, PrepareReport, PreparedMod, TimedStep, extracted_mods, list_mods,
    prepare, save_state,
};
pub use target::{
    CompilePlan, TARGETS, cargo_target_var, compile_plan, host_triple, lib_file_name,
    resolve_target,
};
