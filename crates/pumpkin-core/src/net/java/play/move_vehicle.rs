#[allow(clippy::wildcard_imports)]
use super::*;

use crate::entity::player::client_vehicle_move::VehicleMove;

impl JavaClient {
    pub fn handle_move_vehicle(&self, player: &Arc<Player>, server: &Arc<Server>, packet: &SMoveVehicle) {
        // A movement packet was received this tick — tracked for SClientTickEnd zeroing.
        self.received_movement_this_tick
            .store(true, Ordering::Relaxed);
        player.apply_client_vehicle_move(
            server,
            VehicleMove {
                position: Vector3::new(packet.x, packet.y, packet.z),
                yaw: packet.yaw,
                pitch: packet.pitch,
                on_ground: packet.on_ground,
            },
        );
    }
}
