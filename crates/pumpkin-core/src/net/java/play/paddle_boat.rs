#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub fn handle_paddle_boat(&self, player: &Arc<Player>, packet: &SPaddleBoat) {
        player.apply_client_paddles(packet.left_paddle, packet.right_paddle);
    }
}
