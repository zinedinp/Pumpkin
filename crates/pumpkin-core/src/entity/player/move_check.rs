//! Server checks on client moves (`ServerGamePacketListenerImpl.handlePlayerPositionChange`).
//!
//! The checks are a list on the [`Server`](crate::server::Server): vanilla's
//! two by default, and plugins add their own (an anticheat) or drop one.
//! A rejected move snaps the client back. Custom movement states
//! ([`super::custom_movement`]) widen them for a few ticks.
//!
//! The vehicle a player steers runs the same checks
//! (`handleMoveVehicle`), with [`Mover::Vehicle`] in the attempt.

use std::sync::{Arc, Mutex, PoisonError};

use pumpkin_data::game_rules::{GameRule, GameRuleValue};
use pumpkin_util::math::vector3::Vector3;
use tracing::warn;

use super::Player;
use crate::entity::EntityBase;

/// What the client moved.
#[derive(Clone, Copy)]
pub enum Mover<'a> {
    /// The player itself (`ServerboundMovePlayerPacket`).
    Player,
    /// The vehicle it steers (`ServerboundMoveVehiclePacket`).
    Vehicle(&'a dyn EntityBase),
}

impl std::fmt::Debug for Mover<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Player => f.write_str("Player"),
            Self::Vehicle(v) => write!(f, "Vehicle({})", v.get_entity().entity_id),
        }
    }
}

/// One client move as the checks see it.
#[derive(Clone, Copy, Debug)]
pub struct MoveAttempt<'a> {
    pub mover: Mover<'a>,
    /// Where the server has the mover.
    pub start: Vector3<f64>,
    /// Where the client says it is.
    pub target: Vector3<f64>,
    /// Where the mover was when this tick began (`firstGoodX`).
    pub first_good: Vector3<f64>,
    /// Move packets since then, at least 1 (`deltaPackets`). Vehicles
    /// don't count them: always 1.
    pub packets: i32,
    /// `isFallFlying`.
    pub fall_flying: bool,
    /// Extra blocks per tick a custom movement state grants right now.
    pub extra_speed: f64,
    /// The tick rate runs normally (not frozen or stepping).
    pub runs_normally: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveVerdict {
    Accept,
    Reject,
}

pub trait MoveCheck: Send + Sync {
    /// For logs and for removing it again.
    fn name(&self) -> &'static str;

    fn check(&self, player: &Player, attempt: &MoveAttempt<'_>) -> MoveVerdict;
}

/// Vanilla's checks, in vanilla's order.
#[must_use]
pub fn vanilla_checks() -> Vec<Arc<dyn MoveCheck>> {
    vec![Arc::new(MovedTooQuickly), Arc::new(MovedWrongly)]
}

/// Runs `checks` until one rejects.
pub fn run(
    checks: &[Arc<dyn MoveCheck>],
    player: &Player,
    attempt: &MoveAttempt<'_>,
) -> MoveVerdict {
    for check in checks {
        if check.check(player, attempt) == MoveVerdict::Reject {
            return MoveVerdict::Reject;
        }
    }
    MoveVerdict::Accept
}

fn game_rule(player: &Player, rule: &GameRule) -> bool {
    matches!(
        player.world().get_game_rule(rule),
        GameRuleValue::Bool(true)
    )
}

/// "moved too quickly": farther from the tick's first good spot than the
/// velocity explains, by 10 blocks per packet (about 17 gliding). A
/// vehicle gets 10 blocks a tick, whatever the game rules say.
pub struct MovedTooQuickly;

impl MoveCheck for MovedTooQuickly {
    fn name(&self) -> &'static str {
        "moved_too_quickly"
    }

    fn check(&self, player: &Player, attempt: &MoveAttempt<'_>) -> MoveVerdict {
        // TODO(parity): vanilla also skips the owner of an integrated server
        // and a player changing dimension; Pumpkin knows neither yet.
        let (velocity, allowance) = match attempt.mover {
            Mover::Player => {
                // `shouldCheckPlayerMovement`.
                if !attempt.runs_normally
                    || !game_rule(player, &GameRule::PlayerMovementCheck)
                    || (attempt.fall_flying && !game_rule(player, &GameRule::ElytraMovementCheck))
                {
                    return MoveVerdict::Accept;
                }
                let per_packet = if attempt.fall_flying { 300.0 } else { 100.0 };
                (
                    player.get_entity().velocity.load(),
                    per_packet * f64::from(attempt.packets),
                )
            }
            Mover::Vehicle(vehicle) => (vehicle.get_entity().velocity.load(), 100.0),
        };
        let expected = velocity.length() + attempt.extra_speed;
        let moved = (attempt.target - attempt.first_good).length_squared();
        if moved - expected * expected > allowance {
            let d = attempt.target - attempt.first_good;
            warn!(
                "{} moved too quickly! {},{},{}",
                mover_name(player, attempt.mover),
                d.x,
                d.y,
                d.z
            );
            return MoveVerdict::Reject;
        }
        MoveVerdict::Accept
    }
}

/// Who the logs blame: the player, or `"<vehicle> (vehicle of <player>)"`.
fn mover_name(player: &Player, mover: Mover<'_>) -> String {
    match mover {
        Mover::Player => player.gameprofile.name.clone(),
        Mover::Vehicle(vehicle) => format!(
            "{} (vehicle of {})",
            vehicle.get_entity().entity_type.resource_name,
            player.gameprofile.name
        ),
    }
}

/// "moved wrongly": the blocks would have stopped the move more than a
/// quarter block short of where the client ended up.
pub struct MovedWrongly;

impl MoveCheck for MovedWrongly {
    fn name(&self) -> &'static str {
        "moved_wrongly"
    }

    fn check(&self, player: &Player, attempt: &MoveAttempt<'_>) -> MoveVerdict {
        match attempt.mover {
            Mover::Player => Self::check_player(player, attempt),
            Mover::Vehicle(vehicle) => Self::check_vehicle(player, vehicle, attempt),
        }
    }
}

impl MovedWrongly {
    fn check_player(player: &Player, attempt: &MoveAttempt<'_>) -> MoveVerdict {
        let entity = player.get_entity();
        if entity.no_physics.load(std::sync::atomic::Ordering::Relaxed)
            || player.is_sleeping()
            || player.is_creative()
            || player.is_spectator()
            || player.living_entity.is_in_post_impulse_grace_time()
        {
            return MoveVerdict::Accept;
        }
        let delta = attempt.target - attempt.start;
        // TODO(parity): `collide` knows blocks only: no step-up, no entity
        // colliders (boats, shulkers) and no world border, unlike
        // `Player.move`. Too lenient there; differing block shapes could
        // still snap a legit move back.
        let moved = entity.collide(delta, player).movement;
        let mut miss = attempt.target - (attempt.start + moved);
        // Vanilla's `yDist > -0.5 || yDist < 0.5` always holds: only the
        // horizontal miss counts.
        miss.y = 0.0;
        if miss.length_squared() <= 0.0625 {
            return MoveVerdict::Accept;
        }
        // TODO(parity): vanilla also rejects a passing move that enters a
        // block the old box didn't touch (`isEntityCollidingWithAnythingNew`).
        // A player already stuck in blocks may move out of them.
        if !player
            .world()
            .no_collision(player, entity.bounding_box.load())
        {
            return MoveVerdict::Accept;
        }
        warn!("{} moved wrongly!", player.gameprofile.name);
        MoveVerdict::Reject
    }

    /// The vehicle's half: no exemptions, and it steps up as it would
    /// (`vehicle.move(MoverType.PLAYER, …)`); a ridden horse a full block.
    fn check_vehicle(
        player: &Player,
        vehicle: &dyn EntityBase,
        attempt: &MoveAttempt<'_>,
    ) -> MoveVerdict {
        let entity = vehicle.get_entity();
        let delta = attempt.target - attempt.start;
        // TODO(parity): no entity colliders and no world border, as for
        // the player.
        let moved = entity.adjust_movement_for_collisions(delta, vehicle);
        let mut miss = attempt.target - (attempt.start + moved);
        miss.y = 0.0;
        if miss.length_squared() <= 0.0625 {
            return MoveVerdict::Accept;
        }
        // TODO(parity): `isEntityCollidingWithAnythingNew`, as for the player.
        if !player
            .world()
            .no_collision(vehicle, entity.bounding_box.load())
        {
            return MoveVerdict::Accept;
        }
        warn!("{} moved wrongly!", mover_name(player, attempt.mover));
        MoveVerdict::Reject
    }
}

/// The per-tick bookkeeping vanilla keeps on the connection.
#[derive(Default)]
pub struct MoveCheckState(Mutex<Counters>);

#[derive(Default)]
struct Counters {
    first_good: Vector3<f64>,
    received: i32,
    known: i32,
    /// `lastVehicle`: the vehicle the player steered when the tick began.
    vehicle: Option<VehicleGood>,
    /// `vehiclePositionLastResetAt`, in server ticks.
    vehicle_reset_at: i32,
}

/// `vehicleFirstGoodX`. Vanilla's `vehicleLastGoodX` is where the
/// server has the vehicle, which is where the checks start anyway.
#[derive(Clone, Copy)]
struct VehicleGood {
    id: i32,
    first: Vector3<f64>,
}

impl MoveCheckState {
    /// `resetPosition` and `knownMovePacketCount` at the start of a tick,
    /// with the vehicle the player steers now (`lastVehicle`).
    pub fn start_tick(&self, pos: Vector3<f64>, vehicle: Option<(i32, Vector3<f64>)>) {
        let mut c = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        c.first_good = pos;
        c.known = c.received;
        c.vehicle = vehicle.map(|(id, first)| VehicleGood { id, first });
    }

    /// One more move packet: the first good spot and the packets since.
    pub fn count_packet(&self) -> (Vector3<f64>, i32) {
        let mut c = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        c.received = c.received.wrapping_add(1);
        let mut packets = c.received.wrapping_sub(c.known);
        if packets > 5 {
            packets = 1;
        }
        (c.first_good, packets.max(1))
    }

    /// Where vehicle `id` was when this tick began, if the player steered
    /// it then; moves of any other vehicle are ignored.
    pub fn vehicle_first_good(&self, id: i32) -> Option<Vector3<f64>> {
        let c = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        c.vehicle.filter(|v| v.id == id).map(|v| v.first)
    }

    /// Whether a vehicle the player doesn't steer is put back on its
    /// client: once a second, or when the client is off by a block.
    pub fn vehicle_reset_due(&self, tick: i32, far: bool) -> bool {
        let mut c = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let due = tick.wrapping_sub(c.vehicle_reset_at) > 20 || far;
        if due {
            c.vehicle_reset_at = tick;
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_of_packets_counts_as_one() {
        let state = MoveCheckState::default();
        state.start_tick(Vector3::new(1.0, 2.0, 3.0), None);
        assert_eq!(state.count_packet(), (Vector3::new(1.0, 2.0, 3.0), 1));
        assert_eq!(state.count_packet().1, 2);
        for _ in 0..4 {
            state.count_packet();
        }
        assert_eq!(state.count_packet().1, 1, "more than 5 resets to 1");
        state.start_tick(Vector3::new(0.0, 0.0, 0.0), None);
        assert_eq!(state.count_packet().1, 1);
    }

    #[test]
    fn only_the_vehicle_steered_at_the_tick_start_counts() {
        let state = MoveCheckState::default();
        let at = Vector3::new(4.0, 64.0, 4.0);
        state.start_tick(at, Some((7, at)));
        assert!(state.vehicle_first_good(8).is_none());
        assert_eq!(state.vehicle_first_good(7), Some(at));
        state.start_tick(at, None);
        assert!(
            state.vehicle_first_good(7).is_none(),
            "a new mount waits a tick"
        );
    }

    #[test]
    fn a_vehicle_reset_waits_a_second_unless_far_off() {
        let state = MoveCheckState::default();
        assert!(state.vehicle_reset_due(100, false));
        assert!(!state.vehicle_reset_due(110, false));
        assert!(state.vehicle_reset_due(111, true));
        assert!(state.vehicle_reset_due(132, false));
    }
}
