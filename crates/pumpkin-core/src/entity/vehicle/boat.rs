use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam::atomic::AtomicCell;

use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase, living::LivingEntity};
use crate::server::Server;

use pumpkin_data::damage::DamageType;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_protocol::java::client::play::Metadata;

use pumpkin_data::fluid::Fluid;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use crate::entity::vehicle::vehicle::VehicleEntity;

const FRONT_SEAT_OFFSET: f32 = 0.2;
const BACK_SEAT_OFFSET: f32 = -0.6;
const ANIMAL_SEAT_SHIFT: f32 = 0.2;

pub struct BoatEntity {
    pub vehicle: VehicleEntity,
    /// `outOfControlTicks`: how long the boat has been under water.
    ticks_underwater: AtomicCell<f32>,
    left_paddle_moving: AtomicBool,
    right_paddle_moving: AtomicBool,
}

impl BoatEntity {
    pub const fn new(mut entity: Entity) -> Self {
        entity.blocks_building = true;
        Self {
            vehicle: VehicleEntity::new(entity),
            ticks_underwater: AtomicCell::new(0.0),
            left_paddle_moving: AtomicBool::new(false),
            right_paddle_moving: AtomicBool::new(false),
        }
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

    fn get_entity(&self) -> &Entity {
        &self.vehicle.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
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
        self.vehicle.damage_with_context(amount, source)
    }

    fn interact(&self, player: &Arc<Player>, _item_stack: &mut ItemStack) -> bool {
        if player.get_entity().is_sneaking() {
            return false;
        }

        if self.ticks_underwater.load() >= 60.0 {
            return false;
        }

        if self
            .vehicle
            .entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
            >= 2
        {
            return false;
        }

        if player.get_entity().has_vehicle() {
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
