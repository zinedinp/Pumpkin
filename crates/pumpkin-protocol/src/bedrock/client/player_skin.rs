// Last verified for v2169

use pumpkin_macros::packet;
use uuid::Uuid;

use super::player_list::Skin;
use crate::serial::PacketWrite;

#[derive(PacketWrite)]
#[packet(93)]
pub struct CPlayerSkin {
    pub uuid: Uuid,
    pub skin: Skin,
    pub new_skin_name: String,
    pub old_skin_name: String,
}
