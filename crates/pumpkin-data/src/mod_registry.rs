//! Runtime Block/Item overlay for zip-mods, like NeoForge `DeferredRegister`
//! plus a matching `BlockItem`. Vanilla generated tables stay frozen; extra
//! ids sit in a reserved range after [`BlockId::COUNT`] / [`BlockStateId::COUNT`].

// TODO(mods): open spots, most important first.
// - Pump chunks store raw state ids. Ids come from registration order, so
//   adding, removing or reordering a mod shifts every mod block in existing
//   worlds. Store a name palette (or a per-world id->name table) and remap
//   on load. Anvil already saves names.
// - A native mod can only say "id + name". Hardness, blast resistance and
//   tool requirement are fixed in `pumpkin_mod::activate` (5.0 / 6.0 / true);
//   they need a grown `HostVTable` (ABI 2) or a JSON sidecar per block.
// - Full opaque cubes only: no custom shapes, properties/multi-state, light,
//   sounds (sound_type 27 = iron), map colour, loot table (drops nothing),
//   tags (mineable/needs_tool), recipes, block entities, standalone items.
// - Server-only mods shift ids on the client: Melon prepares with
//   `Side::Both`, the server with `Side::Server`. Ids must come from one
//   side-independent list (e.g. every enabled mod's `data/` blocks).
// - `block_state_flags` in Melon and `Block::from_*` fall back through a
//   `Mutex<Vec>` linear scan; switch to an index `Vec` read via `RwLock`
//   or `OnceLock` once the set is frozen after `activate`.
// - Caps (`BLOCK_CAP`, `STATE_CAP` 256, `ITEM_BASE` 17000) are fixed; `Full`
//   is only an error log. Bedrock network ids for mod states are the raw id.
// - Codegen (`tools/pumpkin-codegen/src/{block,item}.rs`) hand-writes the
//   overlay `else` branch in each lookup. Only blocks and items have it;
//   fluids, sounds, entity types and tags have no mod hook yet.
use crate::block_properties::NoteblockInstrument;
use crate::block_state::PistonBehavior;
use crate::data_component::DataComponent;
use crate::data_component_impl::{
    DataComponentImpl, ItemModelImpl, ItemNameImpl, MaxStackSizeImpl,
};
use crate::item::Item;
use crate::{Block, BlockId, BlockState, BlockStateId};
use serde::Deserialize;
use std::borrow::Cow;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// Extra block ids: `[BlockId::COUNT, BlockId::COUNT + BLOCK_CAP)`.
pub const BLOCK_CAP: u16 = 256;
/// Extra block-state ids: `[BlockStateId::COUNT, BlockStateId::COUNT + STATE_CAP)`.
pub const STATE_CAP: u16 = 256;
/// First numeric item id used for zip-mod block items (vanilla tops out ~1656).
const ITEM_BASE: u16 = 17_000;

struct Entry {
    id: &'static str,
    block: &'static Block,
    item: &'static Item,
    state: &'static BlockState,
}

fn table() -> &'static Mutex<Vec<Entry>> {
    static TABLE: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(Vec::new()))
}

fn lock() -> std::sync::MutexGuard<'static, Vec<Entry>> {
    table()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A zip-mod block already registered under `id`.
#[must_use]
pub fn block_from_name(name: &str) -> Option<&'static Block> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    lock()
        .iter()
        .find(|e| e.id == name || e.block.name == name)
        .map(|e| e.block)
}

/// Item overlay for [`Item::from_registry_key`].
#[must_use]
pub fn item_from_name(name: &str) -> Option<&'static Item> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    lock()
        .iter()
        .find(|e| e.id == name || e.item.registry_key == name)
        .map(|e| e.item)
}

/// Item overlay for [`Item::from_id`].
#[must_use]
pub fn item_from_id(id: u16) -> Option<&'static Item> {
    lock().iter().find(|e| e.item.id == id).map(|e| e.item)
}

/// Block overlay for [`Block::from_id`].
#[must_use]
pub fn block_from_id(id: u16) -> Option<&'static Block> {
    lock()
        .iter()
        .find(|e| e.block.id.as_u16() == id)
        .map(|e| e.block)
}

/// State overlay for [`BlockState::from_id`].
#[must_use]
pub fn state_from_id(id: u16) -> Option<&'static BlockState> {
    lock()
        .iter()
        .find(|e| e.state.id.as_u16() == id)
        .map(|e| e.state)
}

/// [`BlockId`] for an extra state id.
#[must_use]
pub fn block_id_from_state(state_id: u16) -> Option<BlockId> {
    lock()
        .iter()
        .find(|e| e.state.id.as_u16() == state_id)
        .map(|e| e.block.id)
}

/// Block overlay for [`Block::from_item_id`].
#[must_use]
pub fn block_from_item_id(item_id: u16) -> Option<&'static Block> {
    lock()
        .iter()
        .find(|e| e.item.id == item_id)
        .map(|e| e.block)
}

/// Every extra block-state id, in registration order.
#[must_use]
pub fn extra_state_ids() -> Vec<u16> {
    lock().iter().map(|e| e.state.id.as_u16()).collect()
}

#[derive(Deserialize)]
struct BlockFile {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default = "default_hardness")]
    hardness: f32,
    #[serde(default = "default_blast")]
    blast_resistance: f32,
    #[serde(default)]
    requires_tool: bool,
}

fn default_hardness() -> f32 {
    5.0
}

fn default_blast() -> f32 {
    6.0
}

/// Read `data/<ns>/blocks/*.json` from an extracted zip-mod folder.
pub fn load_extracted_dir(dir: &Path) {
    let data = dir.join("data");
    let Ok(namespaces) = std::fs::read_dir(&data) else {
        return;
    };
    let mut files = Vec::new();
    for ns in namespaces.flatten() {
        let blocks = ns.path().join("blocks");
        let Ok(entries) = std::fs::read_dir(&blocks) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(file) = serde_json::from_str::<BlockFile>(&text) else {
            continue;
        };
        let display = file
            .name
            .as_deref()
            .unwrap_or(file.id.rsplit(':').next().unwrap_or(file.id.as_str()));
        let _ = register_simple_block(
            &file.id,
            display,
            file.hardness,
            file.blast_resistance,
            file.requires_tool,
        );
    }
}

/// Register a full-cube block and its item, NeoForge `registerSimpleBlock` +
/// `registerSimpleBlockItem`. Same `id` a second time is a no-op.
pub fn register_simple_block(
    id: &str,
    display_name: &str,
    hardness: f32,
    blast_resistance: f32,
    requires_tool: bool,
) -> Result<&'static Block, RegisterError> {
    let _ = display_name;
    if id.is_empty() {
        return Err(RegisterError::EmptyId);
    }
    let mut guard = lock();
    if let Some(existing) = guard.iter().find(|e| e.id == id) {
        return Ok(existing.block);
    }
    if guard.len() >= BLOCK_CAP as usize {
        return Err(RegisterError::Full);
    }
    let index = u16::try_from(guard.len()).map_err(|_| RegisterError::Full)?;
    let block_id = BlockId::COUNT
        .checked_add(index)
        .and_then(BlockId::new)
        .ok_or(RegisterError::Full)?;
    let state_id = BlockStateId::COUNT
        .checked_add(index)
        .and_then(BlockStateId::new)
        .ok_or(RegisterError::Full)?;
    let item_id = ITEM_BASE.saturating_add(index);

    let registry_key: &'static str = Box::leak(id.to_owned().into_boxed_str());
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    let path_key = path.replace('/', ".");
    let item_name: &'static str =
        Box::leak(format!("block.{namespace}.{path_key}").into_boxed_str());
    let model_id: &'static str = registry_key;

    let max = Box::leak(Box::new(MaxStackSizeImpl { size: 64 }));
    let iname = Box::leak(Box::new(ItemNameImpl {
        name: Cow::Borrowed(item_name),
    }));
    let imodel = Box::leak(Box::new(ItemModelImpl {
        id: Cow::Borrowed(model_id),
    }));
    let components: &'static [(DataComponent, &'static dyn DataComponentImpl)] =
        Box::leak(Box::new([
            (DataComponent::MaxStackSize, max as _),
            (DataComponent::ItemName, iname as _),
            (DataComponent::ItemModel, imodel as _),
        ]));
    let item = Box::leak(Box::new(Item {
        id: item_id,
        registry_key,
        components,
    }));

    // Iron-block flags (full opaque cube). Bit 2 is `TOOL_REQUIRED`.
    let mut state_flags: u16 = 3524;
    if !requires_tool {
        state_flags &= !4;
    }
    let state = Box::leak(Box::new(BlockState {
        id: state_id,
        state_flags,
        side_flags: 255,
        instrument: NoteblockInstrument::IronXylophone,
        luminance: 0,
        piston_behavior: PistonBehavior::Normal,
        hardness,
        collision_shapes: &[0],
        outline_shapes: &[0],
        opacity: 15,
        sound_type: 27,
        block_entity_type: u16::MAX,
    }));
    let states: &'static [BlockState] = std::slice::from_ref(state);

    let block = Box::leak(Box::new(Block {
        id: block_id,
        name: registry_key,
        hardness,
        blast_resistance,
        map_color: 6,
        slipperiness: 0.6,
        velocity_multiplier: 1.0,
        jump_velocity_multiplier: 1.0,
        bounce_restitution: 0.0,
        fall_distance_reduction: 0.0,
        item_id,
        default_state: state,
        states,
        flammable: None,
        experience: None,
        loot_table: None,
    }));
    guard.push(Entry {
        id: registry_key,
        block,
        item,
        state,
    });
    Ok(block)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterError {
    EmptyId,
    Full,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stealframe_registers_as_placeable_block_item() {
        let block =
            register_simple_block("hello:stealframe", "Stealframe", 5.0, 6.0, true).unwrap();
        assert_eq!(block.name, "hello:stealframe");
        assert_eq!(
            Item::from_registry_key("hello:stealframe").map(|i| i.registry_key),
            Some("hello:stealframe")
        );
        let item = item_from_name("hello:stealframe").unwrap();
        assert_eq!(
            Block::from_item_id(item.id).map(|b| b.name),
            Some("hello:stealframe")
        );
        assert_eq!(
            Block::from_registry_key("hello:stealframe").map(|b| b.name),
            Some("hello:stealframe")
        );
        assert!(BlockId::new(block.id.as_u16()).is_some());
        assert!(BlockStateId::new(block.default_state.id.as_u16()).is_some());
        assert_eq!(Block::from_id(block.id).name, "hello:stealframe");
        assert_eq!(
            BlockState::from_id(block.default_state.id).id,
            block.default_state.id
        );
        assert!(!block.default_state.is_air());
        assert!(block.default_state.is_solid_render());
    }

    #[test]
    fn load_extracted_dir_reads_blocks_json() {
        let dir = std::env::temp_dir().join(format!(
            "pumpkin-mod-block-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let blocks = dir.join("data/hello/blocks");
        std::fs::create_dir_all(&blocks).unwrap();
        std::fs::write(
            blocks.join("stealframe.json"),
            r#"{"id":"hello:stealframe","name":"Stealframe","hardness":5.0,"blast_resistance":6.0,"requires_tool":true}"#,
        )
        .unwrap();
        load_extracted_dir(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let item = Item::from_registry_key("hello:stealframe").unwrap();
        assert_eq!(
            Block::from_item_id(item.id).map(|b| b.name),
            Some("hello:stealframe")
        );
    }
}
