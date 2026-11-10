#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub fn handle_confirm_teleport(&self, player: &Player, confirm_teleport: &SConfirmTeleport) {
        // Vanilla `ServerGamePacketListenerImpl.handleAcceptTeleportPacket`
        // (and Paper) ignore a confirm whose id is not the one we are
        // waiting for. A later teleport replaces `awaiting_teleport`; the
        // stale confirm for the previous id must not kick.
        let mut awaiting_teleport = player
            .awaiting_teleport
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some((id, position)) = awaiting_teleport.as_ref() else {
            return;
        };
        if id != &confirm_teleport.teleport_id {
            return;
        }
        player.get_entity().set_pos(*position);
        *awaiting_teleport = None;
    }
}
