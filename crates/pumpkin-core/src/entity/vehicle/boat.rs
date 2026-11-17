use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam::atomic::AtomicCell;

use crate::entity::player::Player;
use crate::entity::vehicle::minecart::container::{self, MinecartInventory};
use crate::entity::{Entity, EntityBase, living::LivingEntity};
use crate::server::Server;
use crate::world::game_event::GameEventContext;

use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::CustomNameImpl;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::translation;
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::java::client::play::Metadata;
use pumpkin_util::GameMode;

use pumpkin_data::fluid::Fluid;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::text::TextComponent;

use crate::entity::vehicle::vehicle::VehicleEntity;

const FRONT_SEAT_OFFSET: f32 = 0.2;
const BACK_SEAT_OFFSET: f32 = -0.6;
const ANIMAL_SEAT_SHIFT: f32 = 0.2;
const CHEST_BOAT_CONTAINER_SIZE: usize = 27;

#[must_use]
pub const fn is_chest_boat(entity_type: &EntityType) -> bool {
    entity_type.id == EntityType::ACACIA_CHEST_BOAT.id
        || entity_type.id == EntityType::BAMBOO_CHEST_RAFT.id
        || entity_type.id == EntityType::BIRCH_CHEST_BOAT.id
        || entity_type.id == EntityType::CHERRY_CHEST_BOAT.id
        || entity_type.id == EntityType::DARK_OAK_CHEST_BOAT.id
        || entity_type.id == EntityType::JUNGLE_CHEST_BOAT.id
        || entity_type.id == EntityType::MANGROVE_CHEST_BOAT.id
        || entity_type.id == EntityType::OAK_CHEST_BOAT.id
        || entity_type.id == EntityType::PALE_OAK_CHEST_BOAT.id
        || entity_type.id == EntityType::POPLAR_CHEST_BOAT.id
        || entity_type.id == EntityType::SPRUCE_CHEST_BOAT.id
}

pub struct BoatEntity {
    pub vehicle: VehicleEntity,
    /// `outOfControlTicks`: how long the boat has been under water.
    ticks_underwater: AtomicCell<f32>,
    left_paddle_moving: AtomicBool,
    right_paddle_moving: AtomicBool,
    inventory: Option<Arc<MinecartInventory>>,
}

impl BoatEntity {
    #[must_use]
    pub fn new(mut entity: Entity) -> Self {
        entity.blocks_building = true;
        let inventory = is_chest_boat(entity.entity_type)
            .then(|| Arc::new(MinecartInventory::new(CHEST_BOAT_CONTAINER_SIZE)));
        Self {
            vehicle: VehicleEntity::new(entity),
            ticks_underwater: AtomicCell::new(0.0),
            left_paddle_moving: AtomicBool::new(false),
            right_paddle_moving: AtomicBool::new(false),
            inventory,
        }
    }

    const fn max_passengers(&self) -> usize {
        if self.inventory.is_some() { 1 } else { 2 }
    }

    pub fn open_custom_inventory_screen(&self, player: &Arc<Player>) -> bool {
        let Some(inventory) = &self.inventory else {
            return false;
        };
        let entity_type = self.vehicle.entity.entity_type;
        let bedrock_key = if entity_type.id == EntityType::BAMBOO_CHEST_RAFT.id {
            translation::bedrock::ENTITY_CHEST_RAFT_NAME
        } else {
            translation::bedrock::ENTITY_CHEST_BOAT_NAME
        };
        let title = TextComponent::translate_cross(
            format!("entity.minecraft.{}", entity_type.resource_name),
            bedrock_key,
            [],
        );
        let opened = container::open(&self.vehicle.entity, player, inventory, title, false);
        if opened {
            let world = self.vehicle.entity.world.load();
            world.emit_game_event_with_context(
                GameEvent::ContainerOpen.name(),
                self.vehicle.entity.pos.load(),
                &GameEventContext {
                    source_entity: Some(player.as_ref()),
                },
            );
        }
        opened
    }

    /// `setPaddleState`. Like vanilla's entity data, only a change goes
    /// out: the driver's client reports the paddles every tick.
    pub fn set_paddles(&self, left: bool, right: bool) {
        let was_left = self.left_paddle_moving.swap(left, Ordering::Relaxed);
        let was_right = self.right_paddle_moving.swap(right, Ordering::Relaxed);
        if (was_left, was_right) == (left, right) {
            return;
        }

        self.vehicle.entity.send_meta_data(
            &[
                Metadata::new(pumpkin_data::tracked_data::boat::ID_PADDLE_LEFT, left),
                Metadata::new(pumpkin_data::tracked_data::boat::ID_PADDLE_RIGHT, right),
            ],
            None,
        );
    }

    fn send_wobble_metadata(&self) {
        self.vehicle.send_wobble_metadata();
    }

    /// `AbstractBoat.isUnderwater`: water above the top of the boat.
    fn is_under_water(&self) -> bool {
        let entity = &self.vehicle.entity;
        let world = entity.world.load();
        let bounds = entity.bounding_box.load();
        let top = bounds.max.y + 0.001;
        let floor = |v: f64| v.floor() as i32;
        let ceil = |v: f64| v.ceil() as i32;
        for x in floor(bounds.min.x)..ceil(bounds.max.x) {
            for y in floor(bounds.max.y)..ceil(top) {
                for z in floor(bounds.min.z)..ceil(bounds.max.z) {
                    let pos = BlockPos::new(x, y, z);
                    let (fluid, state) = world.get_fluid_and_fluid_state(&pos);
                    if fluid.matches_type(&Fluid::WATER)
                        && top
                            < f64::from(y) + f64::from(world.get_fluid_height(&pos, fluid, &state))
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `outOfControlTicks`, and `ejectPassengers` once it reaches 60.
    fn tick_under_water(&self) {
        if !self.is_under_water() {
            self.ticks_underwater.store(0.0);
            return;
        }
        let ticks = self.ticks_underwater.load() + 1.0;
        self.ticks_underwater.store(ticks);
        if ticks >= 60.0 {
            let entity = &self.vehicle.entity;
            let riders: Vec<i32> = entity
                .passengers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .map(|p| p.get_entity().entity_id)
                .collect();
            for rider in riders {
                entity.remove_passenger(rider);
            }
        }
    }

    /// The paddles rest unless a player rows (`tick`'s authoritative half).
    fn rest_paddles_without_player(&self) {
        let player_rows = self
            .vehicle
            .entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .first()
            .is_some_and(|p| p.get_player().is_some());
        if !player_rows {
            self.set_paddles(false, false);
        }
    }
}

impl EntityBase for BoatEntity {
    fn passenger_attachment_point(
        &self,
        passenger: &dyn EntityBase,
        index: usize,
    ) -> Option<Vector3<f64>> {
        let entity = self.get_entity();
        let seats = entity.entity_type.boat?;
        let mut offset = seats.single_passenger_x_offset;
        let passenger_count = entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        if passenger_count > 1 {
            offset = if index == 0 {
                FRONT_SEAT_OFFSET
            } else {
                BACK_SEAT_OFFSET
            };
            if passenger
                .get_mob()
                .and_then(|mob| mob.as_animal())
                .is_some()
            {
                offset += ANIMAL_SEAT_SHIFT;
            }
        }
        Some(crate::entity::attachment_point(
            [0.0, seats.ride_height, f64::from(offset)],
            1.0,
            entity.yaw.load(),
        ))
    }

    fn get_controlling_passenger(&self) -> Option<Arc<dyn EntityBase>> {
        self.get_entity()
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .first()
            .filter(|passenger| passenger.get_living_entity().is_some())
            .cloned()
    }

    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        if let Some(inventory) = &self.inventory {
            inventory.write_nbt(nbt);
        }
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        if let Some(inventory) = &self.inventory {
            inventory.read_nbt(nbt);
        }
    }

    fn get_entity(&self) -> &Entity {
        &self.vehicle.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn has_container_inventory(&self) -> bool {
        self.inventory.is_some()
    }

    fn get_container_inventory(&self) -> Option<Arc<dyn Inventory>> {
        let container = self.inventory.as_ref()?;
        if container.has_loot_table() {
            let world = self.vehicle.entity.world.load();
            container.unpack_loot(&crate::world::loot::LootContextParameters {
                world: Some(&world),
                position: Some(self.vehicle.entity.pos.load()),
                ..Default::default()
            });
        }
        Some(container.clone() as Arc<dyn Inventory>)
    }

    fn tick(&self, _caller: &dyn EntityBase, _server: &Server) {
        self.tick_under_water();
        self.vehicle.tick();
        self.rest_paddles_without_player();
        // TODO(parity): the server doesn't float or move a boat nobody
        // steers yet (`floatBoat`, `move`), nor bubble columns.
    }

    fn init_data_tracker(&self) {
        self.send_wobble_metadata();
    }

    fn can_hit(&self) -> bool {
        self.vehicle.entity.is_alive()
    }

    fn is_collidable(&self, _entity: Option<Box<dyn EntityBase>>) -> bool {
        true
    }

    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        amount: f32,
        _damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        _cause: Option<&dyn EntityBase>,
    ) -> bool {
        let creative = source
            .and_then(EntityBase::get_player)
            .is_some_and(|player| player.gamemode.load() == GameMode::Creative);
        let will_break = self.vehicle.entity.is_alive()
            && (creative || self.vehicle.get_damage() + amount * 10.0 > 40.0);

        let damaged = self.vehicle.damage_with_context(amount, source);

        if will_break && !creative && self.vehicle.entity.is_removed() {
            let world = self.vehicle.entity.world.load();
            if world.level_info.load().game_rules.entity_drops {
                let position = self.vehicle.entity.block_pos.load();
                if let Some(container) = &self.inventory
                    && container.claim_drops()
                {
                    container.unpack_loot(&crate::world::loot::LootContextParameters {
                        world: Some(&world),
                        position: Some(self.vehicle.entity.pos.load()),
                        ..Default::default()
                    });
                    let inventory: Arc<dyn Inventory> = container.clone();
                    world.scatter_inventory(&position, &inventory);
                }
                if let Some(item) =
                    Item::from_registry_key(self.vehicle.entity.entity_type.resource_name)
                {
                    let mut stack = ItemStack::new(1, item);
                    if let Some(custom_name) = self.vehicle.entity.custom_name.load().as_ref() {
                        stack.set_data_component(CustomNameImpl {
                            name: custom_name.clone(),
                        });
                    }
                    world.drop_stack(&position, stack);
                }
            }
        }

        damaged
    }

    fn interact(&self, player: &Arc<Player>, _item_stack: &mut ItemStack) -> bool {
        let can_add_passenger = self.ticks_underwater.load() < 60.0
            && !player.get_entity().has_vehicle()
            && self
                .vehicle
                .entity
                .passengers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
                < self.max_passengers();

        if self.inventory.is_some() && (!can_add_passenger || player.get_entity().is_sneaking()) {
            let _ = self.open_custom_inventory_screen(player);
            return true;
        }

        if player.get_entity().is_sneaking() || !can_add_passenger {
            return false;
        }

        let world = self.vehicle.entity.world.load();
        let Some(vehicle) = world.get_entity_by_id(self.vehicle.entity.entity_id) else {
            return false;
        };

        let Some(passenger) = world.get_player_by_id(player.entity_id()) else {
            return false;
        };

        self.vehicle
            .entity
            .add_passenger(vehicle, passenger as Arc<dyn EntityBase>);

        true
    }

    fn set_paddle_state(&self, left: bool, right: bool) {
        self.set_paddles(left, right);
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn is_pushable(&self) -> bool {
        true
    }
}
