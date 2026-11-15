//! `melon:swap`: break the clicked block and place the held one in its spot
//! as one change, so what it holds up (a torch, sand) stays.
//!
//! - The client registers the channel; a server that takes swaps registers
//!   it back (`minecraft:register`). Only then does the client send any.
//! - A swap (client → server) is a `ServerboundUseItemOnPacket` body: the
//!   hand, the clicked block and face, the cursor and the sequence.

// TODO(upstream): Melon-specific, like `melon_movement`.

/// The plugin channel.
pub const CHANNEL: &str = "melon:swap";
