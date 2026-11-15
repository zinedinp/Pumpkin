//! Custom movement states over `melon:movement`.
//!
//! The server tells a client which states it allows, grants a started one
//! its grace, and shows it to the players that see the mover. The states
//! come from [`pumpkin_data::mod_movement`].

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use pumpkin_data::mod_movement::{self, MovementSpec};
use pumpkin_protocol::java::client::play::CCustomPayload;
use pumpkin_protocol::java::melon_movement::{CHANNEL, MovementPayload};
use tracing::debug;

use super::Player;
use crate::entity::EntityBase;
use crate::plugin::player::player_custom_movement::PlayerCustomMovementEvent;
use crate::server::Server;

/// What one player has registered and started.
#[derive(Default)]
pub struct CustomMovement(Mutex<State>);

#[derive(Default)]
struct State {
    /// `minecraft:register`ed plugin channels.
    channels: HashSet<String>,
    /// Per state: the server tick from which it may start again.
    ready_at: HashMap<String, i32>,
    /// The extra speed of the last start and the tick it runs out.
    allowance: Option<(f64, i32)>,
}

impl CustomMovement {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[must_use]
    pub fn has_channel(&self, channel: &str) -> bool {
        self.lock().channels.contains(channel)
    }

    /// Extra blocks per tick the move checks grant at `tick`.
    #[must_use]
    pub fn extra_speed(&self, tick: i32) -> f64 {
        self.lock()
            .allowance
            .filter(|(_, until)| tick < *until)
            .map_or(0.0, |(speed, _)| speed)
    }
}

impl Player {
    /// `minecraft:register`: remembers the channel; on `melon:movement`
    /// sends the allowed states.
    pub fn register_channel(&self, channel: &str) {
        self.custom_movement
            .lock()
            .channels
            .insert(channel.to_owned());
        if channel == CHANNEL {
            self.send_allowed_movements();
        }
    }

    pub fn unregister_channel(&self, channel: &str) {
        self.custom_movement.lock().channels.remove(channel);
    }

    /// `Allowed`, if the client listens on the channel.
    pub fn send_allowed_movements(&self) {
        if !self.custom_movement.has_channel(CHANNEL) {
            return;
        }
        let ids = mod_movement::movements()
            .into_iter()
            .map(|m| m.id)
            .collect();
        self.send_movement_payload(&MovementPayload::Allowed(ids));
    }

    fn send_movement_payload(&self, payload: &MovementPayload) {
        let data = payload.encode();
        self.try_send_client_packet(&CCustomPayload::new(CHANNEL, &data));
    }

    /// A `melon:movement` payload from this player's client.
    pub fn handle_movement_payload(self: &Arc<Self>, server: &Arc<Server>, data: &[u8]) {
        match MovementPayload::decode(data) {
            Ok(MovementPayload::Start { id }) => self.start_custom_movement(server, id),
            Ok(other) => debug!("{}: unexpected {other:?}", self.gameprofile.name),
            Err(e) => debug!("{}: {e}", self.gameprofile.name),
        }
    }

    /// `Start`: an allowed state off cooldown gets its grace and is shown.
    // TODO(mods): an unknown, cooling-down or cancelled start is dropped
    // silently; the client keeps moving and only the move checks answer.
    // A `Denied` reply would let it stop the state.
    fn start_custom_movement(self: &Arc<Self>, server: &Arc<Server>, id: String) {
        let Some(spec) = mod_movement::movement(&id) else {
            debug!("{} started unknown movement {id}", self.gameprofile.name);
            return;
        };
        let tick = server.tick_count.load(std::sync::atomic::Ordering::Relaxed);
        if self
            .custom_movement
            .lock()
            .ready_at
            .get(&id)
            .is_some_and(|ready| tick < *ready)
        {
            return;
        }
        let mut event = PlayerCustomMovementEvent::new(self.clone(), id);
        server.plugin_manager.fire_blocking(server, &mut event);
        if event.cancelled {
            return;
        }
        self.grant(&spec, tick);
        if spec.broadcast {
            self.show_movement(&spec.id);
        }
    }

    fn grant(&self, spec: &MovementSpec, tick: i32) {
        let grace = i32::try_from(spec.grace_ticks).unwrap_or(i32::MAX);
        let cooldown = i32::try_from(spec.cooldown_ticks).unwrap_or(i32::MAX);
        let mut state = self.custom_movement.lock();
        state
            .ready_at
            .insert(spec.id.clone(), tick.saturating_add(cooldown));
        state.allowance = Some((spec.extra_speed, tick.saturating_add(grace)));
        drop(state);
        // Vanilla's impulse grace: skips "moved wrongly" and keeps the
        // fall from this push harmless.
        self.living_entity.apply_post_impulse_grace_time(grace);
    }

    /// `Started` to every player that sees this one and listens.
    fn show_movement(&self, id: &str) {
        let entity = self.get_entity();
        let world = entity.world.load_full();
        let Some(tracked) = world.entity_tracker.get_tracked_entity(entity.entity_id) else {
            return;
        };
        let payload = MovementPayload::Started {
            entity_id: entity.entity_id,
            id: id.to_owned(),
        };
        for watcher in tracked.watchers() {
            if watcher.custom_movement.has_channel(CHANNEL) {
                watcher.send_movement_payload(&payload);
            }
        }
    }
}

/// After the allowed states changed: tells every listening player.
pub fn broadcast_allowed(server: &Server) {
    for world in server.worlds.load().iter() {
        for player in world.players.load().iter() {
            player.send_allowed_movements();
        }
    }
}
