//! `melon:swap` ([`pumpkin_protocol::java::melon_swap`]): break and place in
//! one step, so the neighbours only ever see the new block.

#[allow(clippy::wildcard_imports)]
use super::*;
use pumpkin_protocol::ServerPacket;
use pumpkin_protocol::java::client::play::CCustomPayload;
use pumpkin_protocol::java::melon_swap::CHANNEL;
use pumpkin_util::version::JavaMinecraftVersion;

impl JavaClient {
    /// The client asked for `melon:swap`: registers it back, so it may send.
    pub fn accept_swap_channel(&self, player: &Player) {
        player.try_send_client_packet(&CCustomPayload::new(
            "minecraft:register",
            CHANNEL.as_bytes(),
        ));
    }

    /// Breaks the clicked block without updating its neighbours, then places
    /// the held block into the gap. If the placement fails, the neighbours
    /// get the updates a plain break would have sent.
    pub fn handle_swap_block(
        &self,
        player: &Arc<Player>,
        server: &Arc<Server>,
        mut data: &[u8],
        version: &JavaMinecraftVersion,
    ) -> Result<(), BlockPlacingError> {
        let Ok(use_item_on) = SUseItemOn::read(&mut data, version) else {
            return Ok(());
        };
        let pos = use_item_on.position;
        let world = player.world();
        let quiet = BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE;
        if !player.has_client_loaded()
            || !Self::can_swap(player, &use_item_on)
            || !player.break_block_with(&pos, quiet)
        {
            self.update_sequence(use_item_on.sequence.0);
            self.sync_block_state_to_client(&world, pos);
            return Ok(());
        }
        let (gap_block, gap) = world.get_block_and_state_id(&pos);
        let placed = self.handle_use_item_on(player, &use_item_on, server);
        if world.get_block_state_id(&pos) == gap {
            world.update_neighbors_at(&pos, gap_block, None);
            server
                .block_registry
                .update_neighbors(&world, &pos, BlockFlags::NOTIFY_LISTENERS);
        }
        placed
    }

    /// In reach, a block item in the hand, and the block breaks at once:
    /// always in creative (unless the tool can't), else by instant mining.
    fn can_swap(player: &Arc<Player>, use_item_on: &SUseItemOn) -> bool {
        let pos = use_item_on.position;
        let Ok(hand) = Hand::from_packet_id(use_item_on.hand.0) else {
            return false;
        };
        let held = player.inventory().get_stack_in_hand(hand);
        if Block::from_item_id(held.item.id).is_none()
            || !player.can_interact_with_block_at(&pos, 1.0)
        {
            return false;
        }
        let world = player.world();
        let (block, state) = world.get_block_and_state(&pos);
        let gamemode = player.gamemode.load();
        if state.is_air()
            || gamemode == GameMode::Spectator
            || player.cannot_mine_block(world.server.upgrade().as_deref(), block, gamemode)
        {
            return false;
        }
        gamemode == GameMode::Creative || block::calc_block_breaking(player, state, block) >= 1.0
    }
}
