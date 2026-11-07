//! Puts prepared mods into the process: block/item registry, then native
//! libs. One path for Pumpkin and Melon, so both sides assign the same ids.
//!
//! Order is part of the protocol: the ids come from registration order, so
//! every `data/<ns>/blocks/*.json` of all mods (sorted) goes first, then the
//! blocks native mods register in `pumpkin_mod_init`. Calling this again
//! (integrated server plus client in one process) changes nothing.

use crate::cabi::{load_native, registered_blocks};
use crate::prepare::PreparedMod;
use pumpkin_data::mod_registry::{load_extracted_dir, register_simple_block};
use std::path::{Path, PathBuf};
use tracing::error;

/// Registers the JSON blocks of already extracted mod folders.
/// Repeating a folder is a no-op.
pub fn register_data(extracted: impl IntoIterator<Item = impl AsRef<Path>>) {
    for dir in extracted {
        load_extracted_dir(dir.as_ref());
    }
}

/// JSON blocks of all `mods`, then their native libs, then the blocks the
/// libs registered. Failures are logged; one broken mod does not stop the rest.
pub fn activate(mods: &[PreparedMod]) {
    let dirs: Vec<PathBuf> = mods.iter().map(|m| m.extracted.clone()).collect();
    register_data(&dirs);
    for prepared in mods {
        if let Some(lib) = &prepared.native_lib
            && let Err(e) = load_native(lib)
        {
            error!(target: "pumpkin_mod", "native mod {}: {e}", prepared.manifest.id);
        }
    }
    for block in registered_blocks() {
        if let Err(e) = register_simple_block(&block.id, &block.name, 5.0, 6.0, true) {
            error!(target: "pumpkin_mod", "mod block {}: {e:?}", block.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_blocks_register_once_for_repeated_activation() {
        let dir = tempfile::tempdir().unwrap();
        let blocks = dir.path().join("data/act/blocks");
        std::fs::create_dir_all(&blocks).unwrap();
        std::fs::write(blocks.join("b.json"), r#"{"id":"act:b"}"#).unwrap();
        register_data([dir.path()]);
        let first = pumpkin_data::Block::from_registry_key("act:b").unwrap().id;
        register_data([dir.path()]);
        assert_eq!(
            pumpkin_data::Block::from_registry_key("act:b").unwrap().id,
            first
        );
    }
}
