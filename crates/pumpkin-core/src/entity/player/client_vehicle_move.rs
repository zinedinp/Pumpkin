//! The vehicle a player steers moves on its client, as vanilla's
//! `handleMoveVehicle` and `handlePaddleBoat` take it: the client reports
//! where it is, the server checks the move ([`super::move_check`]) and
//! snaps the client back with `ClientboundMoveVehiclePacket` when it
//! refuses one.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use pumpkin_data::translation;
use pumpkin_protocol::java::client::play::CMoveVehicle;
use pumpkin_util::math::{vector3::Vector3, wrap_degrees};
use pumpkin_util::text::TextComponent;

use crate::entity::EntityBase;
use crate::net::DisconnectReason;
use crate::server::Server;
use crate::world::chunker;

use super::Player;
use super::move_check::{self, MoveAttempt, MoveVerdict, Mover};

/// One `ServerboundMoveVehiclePacket`.
#[derive(Clone, Copy, Debug)]
pub struct VehicleMove {
    pub position: Vector3<f64>,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
}

impl Player {
    /// Whether this player is `vehicle`'s controlling passenger.
    pub fn controls(&self, vehicle: &dyn EntityBase) -> bool {
        vehicle
            .get_controlling_passenger()
            .is_some_and(|p| p.get_entity().entity_id == self.entity_id())
    }

    /// `getControlledVehicle`: what this player rides and steers.
    pub fn controlled_vehicle(&self) -> Option<Arc<dyn EntityBase>> {
        self.get_entity()
            .get_vehicle()
            .filter(|vehicle| self.controls(vehicle.as_ref()))
    }

    /// The root vehicle when this player steers it: the one whose moves
    /// the client sends.
    pub(crate) fn steered_root_vehicle(&self) -> Option<Arc<dyn EntityBase>> {
        self.get_entity()
            .get_root_vehicle()
            .filter(|vehicle| self.controls(vehicle.as_ref()))
    }

    /// `ClientboundMoveVehiclePacket.fromEntity`: put the client's vehicle
    /// back where the server has it.
    pub fn reset_client_vehicle(&self, vehicle: &dyn EntityBase) {
        let entity = vehicle.get_entity();
        let pos = entity.pos.load();
        self.try_send_client_packet(&CMoveVehicle::new(
            pos.x,
            pos.y,
            pos.z,
            entity.yaw.load(),
            entity.pitch.load(),
        ));
    }

    /// `handleMoveVehicle`.
    pub fn apply_client_vehicle_move(self: &Arc<Self>, server: &Arc<Server>, requested: VehicleMove) {
        let pos = requested.position;
        if !(pos.x.is_finite() && pos.y.is_finite() && pos.z.is_finite())
            || !requested.yaw.is_finite()
            || !requested.pitch.is_finite()
        {
            self.client.try_kick(
                DisconnectReason::Kicked,
                &TextComponent::translate_cross(
                    translation::java::MULTIPLAYER_DISCONNECT_INVALID_VEHICLE_MOVEMENT,
                    translation::java::MULTIPLAYER_DISCONNECT_INVALID_VEHICLE_MOVEMENT,
                    [],
                ),
            );
            return;
        }
        let awaiting_teleport = self
            .awaiting_teleport
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some();
        if awaiting_teleport || !self.has_client_loaded() {
            return;
        }
        let Some(vehicle) = self.get_entity().get_root_vehicle() else {
            return;
        };
        let id = vehicle.get_entity().entity_id;
        if !self.controls(vehicle.as_ref()) {
            // The client thinks it steers what the server says it doesn't.
            let tick = server.tick_count.load(Ordering::Relaxed);
            let far = vehicle.get_entity().pos.load().squared_distance_to_vec(&pos) > 1.0;
            if self.move_check.vehicle_reset_due(tick, far) {
                tracing::warn!(
                    "{} was expected to be controlling vehicle {} but was not. Resetting vehicle position.",
                    self.gameprofile.name,
                    vehicle.get_entity().entity_type.resource_name
                );
                self.reset_client_vehicle(vehicle.as_ref());
            }
            return;
        }
        // A vehicle mounted this tick moves from the next one on.
        let Some(first_good) = self.move_check.vehicle_first_good(id) else {
            return;
        };
        let entity = vehicle.get_entity();
        let start = entity.pos.load();
        let target = Vector3::new(
            Self::clamp_horizontal(pos.x),
            Self::clamp_vertical(pos.y),
            Self::clamp_horizontal(pos.z),
        );
        let (yaw, pitch) = (wrap_degrees(requested.yaw), wrap_degrees(requested.pitch));
        let attempt = MoveAttempt {
            mover: Mover::Vehicle(vehicle.as_ref()),
            start,
            target,
            first_good,
            packets: 1,
            fall_flying: false,
            extra_speed: 0.0,
            runs_normally: server.tick_rate_manager.runs_normally(),
        };
        // Vanilla's `absSnapTo` keeps the client's rotation either way.
        entity.set_rotation(yaw, pitch);
        if move_check::run(&server.move_checks.load(), self, &attempt) == MoveVerdict::Reject {
            self.reset_client_vehicle(vehicle.as_ref());
            return;
        }

        // TODO(parity): vanilla kicks a player whose vehicle floats in the
        // air too long (`clientVehicleIsFloating`); Pumpkin has no flying
        // kick for players on foot either.
        entity.set_pos(target);
        let moved = target - start;
        entity
            .on_ground
            .store(requested.on_ground, Ordering::Relaxed);
        entity.emit_client_movement(vehicle.as_ref(), moved);
        // TODO(parity): `doCheckFallDamage` also runs for a boat, which
        // breaks when it lands from high up.
        if let Some(living) = vehicle.get_living_entity() {
            living.fall(vehicle.as_ref(), moved.y, requested.on_ground);
        }
        crate::entity::position_riders(vehicle.as_ref());
        self.ride_in_lava_advancement(vehicle.as_ref(), target);
        self.count_ride_distance(moved);
        chunker::update_position(self);
    }

    /// `ServerboundPaddleBoatPacket`: only from the boat's driver.
    pub fn apply_client_paddles(&self, left: bool, right: bool) {
        if let Some(vehicle) = self.controlled_vehicle() {
            vehicle.set_paddle_state(left, right);
        }
    }

    /// `RideEntityInLavaTrigger`: how far the vehicle went on lava.
    fn ride_in_lava_advancement(&self, vehicle: &dyn EntityBase, pos: Vector3<f64>) {
        let world = self.world();
        let block = pos.to_block_pos();
        let below = block.down();
        let lava = &pumpkin_data::Block::LAVA;
        if world.get_block(&block) != lava && world.get_block(&below) != lava {
            self.entered_lava_pos.store(None);
            return;
        }
        let start = self.entered_lava_pos.load().unwrap_or(pos);
        if self.entered_lava_pos.load().is_none() {
            self.entered_lava_pos.store(Some(pos));
        }
        let distance_horizontal = (pos.x - start.x).hypot(pos.z - start.z);
        self.trigger_advancement(
            super::advancement::trigger::AdvancementTrigger::RideEntityInLava {
                distance_horizontal,
                dimension: world.dimension.minecraft_name.to_string(),
                vehicle_type: Some(format!(
                    "minecraft:{}",
                    vehicle.get_entity().entity_type.resource_name
                )),
            },
        );
    }

    /// `checkMovementStatistics` for a ride: `boat_one_cm` and the like.
    fn count_ride_distance(&self, moved: Vector3<f64>) {
        let cm = (moved.length() * 100.0).round() as i32;
        if cm > 0 {
            self.increment_custom_stat(self.get_movement_statistic(), cm);
        }
    }
}
