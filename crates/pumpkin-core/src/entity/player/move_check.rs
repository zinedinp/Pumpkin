//! Server checks on client moves (`ServerGamePacketListenerImpl.handlePlayerPositionChange`).
//!
//! The checks are a list on the [`Server`](crate::server::Server): vanilla's
//! two by default, and plugins add their own (an anticheat) or drop one.
//! A rejected move snaps the client back. Custom movement states
//! ([`super::custom_movement`]) widen them for a few ticks.

use std::sync::{Arc, Mutex, PoisonError};

use pumpkin_data::game_rules::{GameRule, GameRuleValue};
use pumpkin_util::math::vector3::Vector3;
use tracing::warn;

use super::Player;
use crate::entity::EntityBase;

/// One client move as the checks see it.
#[derive(Clone, Copy, Debug)]
pub struct MoveAttempt {
    /// Where the server has the player.
    pub start: Vector3<f64>,
    /// Where the client says it is.
    pub target: Vector3<f64>,
    /// Where the player stood when this tick began (`firstGoodX`).
    pub first_good: Vector3<f64>,
    /// Move packets since then, at least 1 (`deltaPackets`).
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

    fn check(&self, player: &Player, attempt: &MoveAttempt) -> MoveVerdict;
}

/// Vanilla's checks, in vanilla's order.
#[must_use]
pub fn vanilla_checks() -> Vec<Arc<dyn MoveCheck>> {
    vec![Arc::new(MovedTooQuickly), Arc::new(MovedWrongly)]
}

/// Runs `checks` until one rejects.
pub fn run(checks: &[Arc<dyn MoveCheck>], player: &Player, attempt: &MoveAttempt) -> MoveVerdict {
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
/// velocity explains, by 10 blocks per packet (about 17 gliding).
pub struct MovedTooQuickly;

impl MoveCheck for MovedTooQuickly {
    fn name(&self) -> &'static str {
        "moved_too_quickly"
    }

    fn check(&self, player: &Player, attempt: &MoveAttempt) -> MoveVerdict {
        // `shouldCheckPlayerMovement`.
        // TODO(parity): vanilla also skips the owner of an integrated server
        // and a player changing dimension; Pumpkin knows neither yet.
        if !attempt.runs_normally
            || !game_rule(player, &GameRule::PlayerMovementCheck)
            || (attempt.fall_flying && !game_rule(player, &GameRule::ElytraMovementCheck))
        {
            return MoveVerdict::Accept;
        }
        let expected = player.get_entity().velocity.load().length() + attempt.extra_speed;
        let moved = (attempt.target - attempt.first_good).length_squared();
        let per_packet = if attempt.fall_flying { 300.0 } else { 100.0 };
        if moved - expected * expected > per_packet * f64::from(attempt.packets) {
            let d = attempt.target - attempt.first_good;
            warn!(
                "{} moved too quickly! {},{},{}",
                player.gameprofile.name, d.x, d.y, d.z
            );
            return MoveVerdict::Reject;
        }
        MoveVerdict::Accept
    }
}

/// "moved wrongly": the blocks would have stopped the move more than a
/// quarter block short of where the client ended up.
pub struct MovedWrongly;

impl MoveCheck for MovedWrongly {
    fn name(&self) -> &'static str {
        "moved_wrongly"
    }

    fn check(&self, player: &Player, attempt: &MoveAttempt) -> MoveVerdict {
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
        let moved = entity.collide(delta, player).movement;
        let mut miss = attempt.target - (attempt.start + moved);
        // Vanilla's `yDist > -0.5 || yDist < 0.5` always holds: only the
        // horizontal miss counts.
        miss.y = 0.0;
        if miss.length_squared() <= 0.0625 {
            return MoveVerdict::Accept;
        }
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
}

/// The per-tick bookkeeping vanilla keeps on the connection.
#[derive(Default)]
pub struct MoveCheckState(Mutex<Counters>);

#[derive(Default)]
struct Counters {
    first_good: Vector3<f64>,
    received: i32,
    known: i32,
}

impl MoveCheckState {
    /// `resetPosition` and `knownMovePacketCount` at the start of a tick.
    pub fn start_tick(&self, pos: Vector3<f64>) {
        let mut c = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        c.first_good = pos;
        c.known = c.received;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_of_packets_counts_as_one() {
        let state = MoveCheckState::default();
        state.start_tick(Vector3::new(1.0, 2.0, 3.0));
        assert_eq!(state.count_packet(), (Vector3::new(1.0, 2.0, 3.0), 1));
        assert_eq!(state.count_packet().1, 2);
        for _ in 0..4 {
            state.count_packet();
        }
        assert_eq!(state.count_packet().1, 1, "more than 5 resets to 1");
        state.start_tick(Vector3::new(0.0, 0.0, 0.0));
        assert_eq!(state.count_packet().1, 1);
    }
}
