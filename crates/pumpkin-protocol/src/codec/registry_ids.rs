//! Network ids of registry entries (`ByteBufCodecs.holderRegistry`).
//!
//! A holder id indexes the registry as the *server* synced it in the
//! configuration phase (`ClientboundRegistryDataPacket`), datapack entries
//! included. Pumpkin's generated tables only know the vanilla order, which
//! is right for a Pumpkin server but not for a client on another server.
//!
//! A client installs the synced registries on each thread that decodes or
//! encodes play packets ([`install`]); codecs resolve ids through
//! [`name_of`] / [`id_of`], falling back to the vanilla tables where a
//! registry wasn't synced (or on the server, where nothing is installed).
//! An id neither knows is an error, as in vanilla.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// Entry names per registry, in network id order.
#[derive(Clone, Debug, Default)]
pub struct SyncedRegistries {
    entries: HashMap<String, Vec<String>>,
}

impl SyncedRegistries {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// One `ClientboundRegistryDataPacket`: `registry` (`minecraft:wolf_variant`)
    /// and its entry names in packet order.
    pub fn insert(&mut self, registry: impl Into<String>, entries: Vec<String>) {
        let entries = entries.into_iter().map(full_name).collect();
        self.entries.insert(full_name(registry.into()), entries);
    }

    #[must_use]
    pub fn entries(&self, registry: &str) -> Option<&[String]> {
        self.entries.get(registry).map(Vec::as_slice)
    }
}

thread_local! {
    static INSTALLED: RefCell<Option<Arc<SyncedRegistries>>> = const { RefCell::new(None) };
}

/// Resolves ids on this thread through `registries` (`None` back to the
/// vanilla tables).
pub fn install(registries: Option<Arc<SyncedRegistries>>) {
    INSTALLED.with(|installed| *installed.borrow_mut() = registries);
}

/// Runs `f` with `registries` installed, then restores what was before.
pub fn with<R>(registries: Arc<SyncedRegistries>, f: impl FnOnce() -> R) -> R {
    let previous = INSTALLED.with(|installed| installed.replace(Some(registries)));
    let result = f();
    INSTALLED.with(|installed| *installed.borrow_mut() = previous);
    result
}

fn synced<R>(registry: &str, f: impl FnOnce(&[String]) -> R) -> Option<R> {
    INSTALLED.with(|installed| {
        let installed = installed.borrow();
        installed.as_ref()?.entries(registry).map(f)
    })
}

/// `minecraft:` for a name without a namespace.
fn full_name(name: String) -> String {
    if name.contains(':') {
        name
    } else {
        format!("minecraft:{name}")
    }
}

/// The entry `id` names in `registry`: the synced one if installed, else
/// `vanilla` (the generated table's name for that index).
pub fn name_of(
    registry: &str,
    id: i32,
    vanilla: impl FnOnce(usize) -> Option<String>,
) -> Option<String> {
    let index = usize::try_from(id).ok()?;
    synced(registry, |entries| entries.get(index).cloned())
        .unwrap_or_else(|| vanilla(index).map(full_name))
}

/// The network id of `name` in `registry`: the synced one if installed,
/// else `vanilla` (the generated table's id).
pub fn id_of(
    registry: &str,
    name: &str,
    vanilla: impl FnOnce(&str) -> Option<usize>,
) -> Option<i32> {
    let full = full_name(name.to_owned());
    let index = synced(registry, |entries| entries.iter().position(|e| *e == full))
        .unwrap_or_else(|| vanilla(name))?;
    i32::try_from(index).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vanilla(index: usize) -> Option<String> {
        ["pale", "ashen"].get(index).map(|s| (*s).to_owned())
    }

    #[test]
    fn without_synced_registries_the_vanilla_table_answers() {
        install(None);
        assert_eq!(
            name_of("minecraft:wolf_variant", 1, vanilla).as_deref(),
            Some("minecraft:ashen")
        );
        assert_eq!(name_of("minecraft:wolf_variant", 5, vanilla), None);
        assert_eq!(name_of("minecraft:wolf_variant", -1, vanilla), None);
    }

    #[test]
    fn synced_registries_win_with_their_own_order_and_entries() {
        let mut synced = SyncedRegistries::new();
        synced.insert(
            "minecraft:wolf_variant",
            vec!["ashen".into(), "server:custom".into()],
        );
        with(Arc::new(synced), || {
            assert_eq!(
                name_of("minecraft:wolf_variant", 1, vanilla).as_deref(),
                Some("server:custom")
            );
            assert_eq!(
                id_of("minecraft:wolf_variant", "minecraft:ashen", |_| None),
                Some(0)
            );
            assert_eq!(
                id_of("minecraft:wolf_variant", "server:custom", |_| None),
                Some(1)
            );
            // Not synced: back to the vanilla table.
            assert_eq!(
                name_of("minecraft:cat_variant", 0, vanilla).as_deref(),
                Some("minecraft:pale")
            );
        });
        assert_eq!(
            name_of("minecraft:wolf_variant", 1, vanilla).as_deref(),
            Some("minecraft:ashen")
        );
    }
}
