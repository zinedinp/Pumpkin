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
//   they need a grown `HostVTable` (ABI 2). `data/<ns>/blocks/*.json` can
//   say all of [`BlockSpec`].
// - Cube shapes only (full, or none to walk through): no custom shapes,
//   properties/multi-state, sounds (sound_type 27 = iron), map colour, loot
//   table (drops nothing), tags (mineable/needs_tool), recipes, block
//   entities, standalone items.
// - Server-only mods shift ids on the client: Melon prepares with
//   `Side::Both`, the server with `Side::Server`. Ids must come from one
//   side-independent list (e.g. every enabled mod's `data/` blocks).
// - Name lookups (`block_from_name`, `item_from_*`) still scan a
//   `Mutex<Vec>`; the id lookups read lock-free slots (`state_at`,
//   `block_at`), which `BlockState::from_id` and `Block::from_id` use.
// - Caps (`BLOCK_CAP`, `STATE_CAP` 256, `ITEM_BASE` 17000) are fixed; `Full`
//   is only an error log. Bedrock network ids for mod states are the raw id.
// - Codegen (`tools/pumpkin-codegen/src/{block,item}.rs`) hand-writes the
//   overlay `else` branch in each lookup. Only blocks and items have it;
//   fluids, sounds, entity types and tags have no mod hook yet.
use crate::block_properties::NoteblockInstrument;
use crate::block_state::{
    CAN_OCCLUDE, IS_FULL_CUBE, IS_SOLID, IS_SOLID_BLOCK, IS_SOLID_RENDER, PistonBehavior,
    TOOL_REQUIRED,
};
use crate::data_component::DataComponent;
use crate::data_component_impl::{
    DataComponentImpl, ItemModelImpl, ItemNameImpl, MaxStackSizeImpl,
};
use crate::item::Item;
use crate::{Block, BlockId, BlockState, BlockStateId};
use serde::Deserialize;
use std::borrow::Cow;
use std::cell::UnsafeCell;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// Extra block ids: `[BlockId::COUNT, BlockId::COUNT + BLOCK_CAP)`.
pub const BLOCK_CAP: u16 = 256;
/// Extra block-state ids: `[BlockStateId::COUNT, BlockStateId::COUNT + STATE_CAP)`.
pub const STATE_CAP: u16 = 256;
/// First numeric item id used for zip-mod block items (vanilla tops out ~1656).
const ITEM_BASE: u16 = 17_000;

/// How a client draws a mod block, beyond what [`BlockState`] says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RenderLayer {
    /// From the textures: opaque texels solid, clear ones cutout, partly
    /// clear ones translucent.
    #[default]
    Auto,
    Solid,
    Cutout,
    Translucent,
}

/// What a client needs to draw a mod block that [`BlockState`] has no
/// field for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderHints {
    pub layer: RenderLayer,
    /// Corner shading on its faces (a model's `ambientocclusion`).
    pub ambient_occlusion: bool,
}

/// Everything a zip-mod block can say about itself. [`Default`] is a full
/// opaque cube like an iron block.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct BlockSpec {
    pub hardness: f32,
    pub blast_resistance: f32,
    pub requires_tool: bool,
    /// Entities collide with its full cube; `false` walks through it.
    pub collides: bool,
    pub render: RenderSpec,
}

/// The `render` part of a [`BlockSpec`].
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct RenderSpec {
    /// A full, opaque cube (`isSolidRender`): neighbours' faces against it
    /// are hidden and it darkens the corners next to it.
    pub solid: bool,
    /// Hides what is behind it from the client's culling (`canOcclude`).
    /// Defaults to `solid`.
    pub occludes: Option<bool>,
    /// Light it gives off, 0-15.
    pub luminance: u8,
    /// Light it takes away passing through, 0-15. Defaults to 15 for a
    /// solid block, else 0.
    pub light_opacity: Option<u8>,
    pub layer: RenderLayer,
    pub ambient_occlusion: bool,
}

impl Default for BlockSpec {
    fn default() -> Self {
        Self {
            hardness: 5.0,
            blast_resistance: 6.0,
            requires_tool: false,
            collides: true,
            render: RenderSpec::default(),
        }
    }
}

impl Default for RenderSpec {
    fn default() -> Self {
        Self {
            solid: true,
            occludes: None,
            luminance: 0,
            light_opacity: None,
            layer: RenderLayer::Auto,
            ambient_occlusion: true,
        }
    }
}

impl BlockSpec {
    /// [`BlockState::state_flags`] for this block.
    #[must_use]
    pub fn state_flags(&self) -> u16 {
        let render = &self.render;
        let mut flags = 0;
        if self.requires_tool {
            flags |= TOOL_REQUIRED;
        }
        if self.collides {
            flags |= IS_SOLID | IS_FULL_CUBE;
            if render.solid {
                flags |= IS_SOLID_BLOCK;
            }
        }
        if render.solid {
            flags |= IS_SOLID_RENDER;
        }
        if render.occludes.unwrap_or(render.solid) {
            flags |= CAN_OCCLUDE;
        }
        flags
    }

    /// [`BlockState::opacity`].
    #[must_use]
    pub fn light_opacity(&self) -> u8 {
        self.render
            .light_opacity
            .unwrap_or(if self.render.solid { 15 } else { 0 })
            .min(15)
    }

    #[must_use]
    pub fn hints(&self) -> RenderHints {
        RenderHints {
            layer: self.render.layer,
            ambient_occlusion: self.render.ambient_occlusion,
        }
    }
}

/// One slot per extra id, written once while registering (under the
/// table's lock) and read without one, from `const fn`s too:
/// [`BlockState::from_id`] and [`Block::from_id`] stay `const` for
/// vanilla ids and find mod ids here at run time.
struct Slots<T: 'static, const N: usize>(UnsafeCell<[Option<&'static T>; N]>);

// SAFETY: a slot goes from `None` to `Some` once, before its id is handed
// out (registration returns it), and never changes again. Readers of a
// registered id therefore only see the finished pointer.
unsafe impl<T: Sync, const N: usize> Sync for Slots<T, N> {}

impl<T: 'static, const N: usize> Slots<T, N> {
    const fn new() -> Self {
        Self(UnsafeCell::new([None; N]))
    }

    const fn get(&self, index: usize) -> Option<&'static T> {
        if index >= N {
            return None;
        }
        // SAFETY: see `impl Sync`.
        unsafe { (*self.0.get())[index] }
    }

    /// # Safety
    /// Only while holding the table's lock, once per slot.
    unsafe fn set(&self, index: usize, value: &'static T) {
        unsafe { (*self.0.get())[index] = Some(value) };
    }
}

const STATES: usize = STATE_CAP as usize;
const BLOCKS: usize = BLOCK_CAP as usize;
static STATE_SLOTS: Slots<BlockState, STATES> = Slots::new();
static BLOCK_SLOTS: Slots<Block, BLOCKS> = Slots::new();
/// The block of each extra state, by state slot.
static STATE_BLOCK_SLOTS: Slots<Block, STATES> = Slots::new();
static HINT_SLOTS: Slots<RenderHints, STATES> = Slots::new();

/// The registered state with extra id `id`, if any. `const` so that
/// [`BlockState::from_id`] can fall back to it.
#[must_use]
pub const fn state_at(id: u16) -> Option<&'static BlockState> {
    match id.checked_sub(BlockStateId::COUNT) {
        Some(index) => STATE_SLOTS.get(index as usize),
        None => None,
    }
}

/// The registered block with extra id `id`, if any.
#[must_use]
pub const fn block_at(id: u16) -> Option<&'static Block> {
    match id.checked_sub(BlockId::COUNT) {
        Some(index) => BLOCK_SLOTS.get(index as usize),
        None => None,
    }
}

/// The block of the registered extra state `state_id`, if any.
#[must_use]
pub const fn block_of_state_at(state_id: u16) -> Option<&'static Block> {
    match state_id.checked_sub(BlockStateId::COUNT) {
        Some(index) => STATE_BLOCK_SLOTS.get(index as usize),
        None => None,
    }
}

/// How a client draws the registered extra state `state_id`.
#[must_use]
pub fn render_hints(state_id: u16) -> Option<RenderHints> {
    let index = state_id.checked_sub(BlockStateId::COUNT)?;
    HINT_SLOTS.get(index as usize).copied()
}

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
    block_at(id)
}

/// State overlay for [`BlockState::from_id`].
#[must_use]
pub fn state_from_id(id: u16) -> Option<&'static BlockState> {
    state_at(id)
}

/// [`BlockId`] for an extra state id.
#[must_use]
pub fn block_id_from_state(state_id: u16) -> Option<BlockId> {
    block_of_state_at(state_id).map(|block| block.id)
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

/// `data/<ns>/blocks/*.json`: an id, a name, and the rest of a
/// [`BlockSpec`] (every field optional).
#[derive(Deserialize)]
struct BlockFile {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(flatten)]
    spec: BlockSpec,
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
        let file = match serde_json::from_str::<BlockFile>(&text) {
            Ok(file) => file,
            Err(e) => {
                eprintln!("mod block {} unreadable: {e}", path.display());
                continue;
            }
        };
        let _ = register_block(&file.id, &file.spec);
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
    register_block(
        id,
        &BlockSpec {
            hardness,
            blast_resistance,
            requires_tool,
            ..BlockSpec::default()
        },
    )
}

/// Register a block as `spec` says, and its item. Same `id` a second time
/// is a no-op that returns the first one.
pub fn register_block(id: &str, spec: &BlockSpec) -> Result<&'static Block, RegisterError> {
    if id.is_empty() {
        return Err(RegisterError::EmptyId);
    }
    let mut guard = lock();
    if let Some(existing) = guard.iter().find(|e| e.id == id) {
        return Ok(existing.block);
    }
    if guard.len() >= BLOCK_CAP.min(STATE_CAP) as usize {
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
        crafting_remainder: None,
    }));

    // Shape 0 is the full cube.
    const FULL_CUBE: &[u16] = &[0];
    let state = Box::leak(Box::new(BlockState {
        id: state_id,
        state_flags: spec.state_flags(),
        side_flags: if spec.collides { 255 } else { 0 },
        instrument: NoteblockInstrument::IronXylophone,
        luminance: spec.render.luminance.min(15),
        piston_behavior: PistonBehavior::Normal,
        hardness: spec.hardness,
        collision_shapes: if spec.collides { FULL_CUBE } else { &[] },
        outline_shapes: FULL_CUBE,
        opacity: spec.light_opacity(),
        sound_type: 27,
        block_entity_type: u16::MAX,
    }));
    let states: &'static [BlockState] = std::slice::from_ref(state);

    let block = Box::leak(Box::new(Block {
        id: block_id,
        name: registry_key,
        hardness: spec.hardness,
        blast_resistance: spec.blast_resistance,
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
    let hints: &'static RenderHints = Box::leak(Box::new(spec.hints()));
    let slot = usize::from(index);
    // SAFETY: under the table's lock, and `index` is new: each slot is
    // set once, before `register_block` hands out its ids.
    unsafe {
        STATE_SLOTS.set(slot, state);
        BLOCK_SLOTS.set(slot, block);
        STATE_BLOCK_SLOTS.set(slot, block);
        HINT_SLOTS.set(slot, hints);
    }
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

    #[test]
    fn the_default_spec_is_an_iron_block() {
        let spec = BlockSpec::default();
        let iron = Block::IRON_BLOCK.default_state;
        assert_eq!(spec.state_flags(), iron.state_flags & !TOOL_REQUIRED);
        assert_eq!(spec.light_opacity(), iron.opacity);
    }

    #[test]
    fn render_stats_reach_the_block_state() {
        let spec: BlockSpec = serde_json::from_str(
            r#"{"collides": false, "render": {"solid": false, "luminance": 12, "layer": "cutout", "ambient_occlusion": false}}"#,
        )
        .unwrap();
        let block = register_block("probe:lamp_grass", &spec).unwrap();
        let state = BlockState::from_id(block.default_state.id);
        assert_eq!(state.id, block.default_state.id);
        assert!(!state.is_solid_render() && !state.can_occlude() && !state.is_air());
        assert_eq!((state.luminance, state.opacity), (12, 0));
        assert_eq!(state.get_block_collision_shapes().count(), 0);
        assert_eq!(Block::from_state_id(state.id).name, "probe:lamp_grass");
        assert_eq!(
            render_hints(state.id.as_u16()),
            Some(RenderHints {
                layer: RenderLayer::Cutout,
                ambient_occlusion: false,
            })
        );
    }

    #[test]
    fn an_unregistered_extra_id_reads_as_air() {
        let id = BlockStateId::new(BlockStateId::COUNT + STATE_CAP - 1).unwrap();
        assert!(BlockState::from_id(id).is_air());
        assert_eq!(Block::from_state_id(id).id, BlockId::AIR);
        let block = BlockId::new(BlockId::COUNT + BLOCK_CAP - 1).unwrap();
        assert_eq!(Block::from_id(block).id, BlockId::AIR);
    }
}
