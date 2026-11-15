use pumpkin_macros::{Event, cancellable};
use std::sync::Arc;

use crate::entity::player::Player;

use super::PlayerEvent;

/// A player began a custom movement state (`melon:movement`) the server
/// allows. Cancel it to give no grace and tell no one; the client still
/// moved, so the move checks judge it as an ordinary move.
#[cancellable]
#[derive(Event, Clone)]
pub struct PlayerCustomMovementEvent {
    pub player: Arc<Player>,
    /// The state's id, e.g. `example:dodge`.
    pub id: String,
}

impl PlayerCustomMovementEvent {
    #[must_use]
    pub const fn new(player: Arc<Player>, id: String) -> Self {
        Self {
            player,
            id,
            cancelled: false,
        }
    }
}

impl PlayerEvent for PlayerCustomMovementEvent {
    fn get_player(&self) -> &Arc<Player> {
        &self.player
    }
}
