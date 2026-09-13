use crate::{
    bedrock::server::login::ClientData,
    codec::{var_long::VarLong, var_uint::VarUInt},
    serial::PacketWrite,
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use pumpkin_macros::packet;
use std::io::{Error, Write};
use uuid::Uuid;

use super::common::BuildPlatform;

const WIDE_SKIN_RESOURCE_PATCH: &[u8] = br#"{"geometry":{"default":"geometry.humanoid.custom"}}"#;
const SLIM_SKIN_RESOURCE_PATCH: &[u8] =
    br#"{"geometry":{"default":"geometry.humanoid.customSlim"}}"#;
const DEFAULT_SKIN_GEOMETRY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/bedrock/player_geometry.json"
));

#[packet(63)]
pub struct CPlayerList {
    pub action: u8,
    pub entries: Vec<PlayerListEntry>,
}

impl PacketWrite for CPlayerList {
    fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        VarUInt(self.entries.len() as u32).write(writer)?;
        for entry in &self.entries {
            match self.action {
                Self::ACTION_ADD => {
                    VarUInt(1).write(writer)?;
                    Self::ACTION_ADD.write(writer)?;
                    entry.write(writer)?;
                }
                Self::ACTION_REMOVE => {
                    VarUInt(0).write(writer)?;
                    Self::ACTION_REMOVE.write(writer)?;
                    entry.uuid.write(writer)?;
                }
                _ => return Err(Error::other("Invalid PlayerList action")),
            }
        }
        Ok(())
    }
}

impl CPlayerList {
    pub const ACTION_ADD: u8 = 0;
    pub const ACTION_REMOVE: u8 = 1;
}

pub struct PlayerListEntry {
    pub uuid: Uuid,
    pub entity_unique_id: VarLong,
    pub username: String,
    pub xuid: String,
    pub platform_chat_id: String,
    pub build_platform: BuildPlatform,
    pub skin: Skin,
    pub is_teacher: bool,
    pub is_host: bool,
    pub is_sub_client: bool,
    pub player_color: [u8; 4], // ARGB
}

impl PacketWrite for PlayerListEntry {
    fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.uuid.write(writer)?;
        self.entity_unique_id.write(writer)?;
        self.username.write(writer)?;
        self.xuid.write(writer)?;
        self.platform_chat_id.write(writer)?;
        self.build_platform.write(writer)?;
        self.skin.write(writer)?;
        self.is_teacher.write(writer)?;
        self.is_host.write(writer)?;
        self.is_sub_client.write(writer)?;
        u32::from_be_bytes(self.player_color).write(writer)
    }
}

#[derive(Clone)]
pub struct Skin {
    pub skin_id: String,
    pub play_fab_id: String,
    pub resource_patch: Vec<u8>,
    pub image_width: u32,
    pub image_height: u32,
    pub skin_data: Vec<u8>,
    pub animations: Vec<SkinAnimation>,
    pub cape_width: u32,
    pub cape_height: u32,
    pub cape_data: Vec<u8>,
    pub geometry_data: Vec<u8>,
    pub animation_data: Vec<u8>,
    pub geometry_data_engine_version: Vec<u8>,
    pub cape_id: String,
    pub full_id: String,
    pub arm_size: String,
    pub skin_color: String,
    pub persona_pieces: Vec<PersonaPiece>,
    pub piece_tint_colors: Vec<PieceTintColor>,
    pub is_premium: bool,
    pub is_persona: bool,
    pub persona_cape_on_classic: bool,
    pub is_primary_user: bool,
    pub override_appearance: bool,
    pub is_trusted: bool,
    pub profile_hash: String,
}

impl Skin {
    #[must_use]
    pub fn steve() -> Self {
        Self {
            skin_id: "Standard_Custom".to_string(),
            play_fab_id: String::new(),
            resource_patch: WIDE_SKIN_RESOURCE_PATCH.to_vec(),
            image_width: 64,
            image_height: 64,
            // 64 * 64 * 4 = 16384 bytes of raw RGBA data
            // Fill with 255 so the skin is visible (solid white) instead of invisible (transparent)
            skin_data: vec![255; 16384],
            animations: Vec::new(),
            cape_width: 0,
            cape_height: 0,
            cape_data: Vec::new(),
            geometry_data: DEFAULT_SKIN_GEOMETRY.to_vec(),
            animation_data: Vec::new(),
            geometry_data_engine_version: b"1.26.40".to_vec(),
            cape_id: String::new(),
            full_id: "Standard_Custom".to_string(),
            arm_size: "wide".to_string(),
            skin_color: "#0".to_string(),
            persona_pieces: Vec::new(),
            piece_tint_colors: Vec::new(),
            is_premium: true,
            is_persona: false,
            persona_cape_on_classic: false,
            is_primary_user: false,
            override_appearance: true,
            is_trusted: true,
            profile_hash: String::new(),
        }
    }

    /// Selects the standard wide or slim player geometry while preserving the
    /// rest of the serialized skin.
    pub fn set_slim(&mut self, slim: bool) {
        self.arm_size = if slim { "slim" } else { "wide" }.to_string();
        self.resource_patch = if slim {
            SLIM_SKIN_RESOURCE_PATCH
        } else {
            WIDE_SKIN_RESOURCE_PATCH
        }
        .to_vec();
    }

    /// Builds the `PlayerList` skin from a Bedrock login JWT. Missing or truncated
    /// image data returns `None` so the caller can fall back to Steve.
    #[must_use]
    pub fn from_client_data(data: &ClientData) -> Option<Self> {
        let skin_data = decode_b64(&data.skin_data);
        let (image_width, image_height) =
            rgba_dimensions(&skin_data, data.skin_image_width, data.skin_image_height)?;

        let mut skin = Self::steve();
        skin.set_slim(data.arm_size.eq_ignore_ascii_case("slim"));

        if let Some(patch) = decode_text_field(&data.skin_resource_patch) {
            skin.resource_patch = patch;
        }
        if let Some(geometry) = decode_text_field(&data.skin_geometry) {
            skin.geometry_data = geometry;
        }
        if !data.skin_geometry_version.is_empty() {
            skin.geometry_data_engine_version = data.skin_geometry_version.as_bytes().to_vec();
        }

        let cape_data = decode_b64(&data.cape_data);
        let (cape_width, cape_height) = if cape_data.is_empty() {
            (0, 0)
        } else {
            rgba_dimensions(&cape_data, data.cape_image_width, data.cape_image_height).unwrap_or((
                data.cape_image_width.max(0) as u32,
                data.cape_image_height.max(0) as u32,
            ))
        };

        skin.skin_id.clone_from(&data.skin_id);
        skin.play_fab_id.clone_from(&data.play_fab_id);
        skin.image_width = image_width;
        skin.image_height = image_height;
        skin.skin_data = skin_data;
        skin.animations = data
            .animated_image_data
            .iter()
            .filter_map(SkinAnimation::from_login)
            .collect();
        skin.cape_width = cape_width;
        skin.cape_height = cape_height;
        skin.cape_data = cape_data;
        skin.animation_data = data.skin_animation_data.as_bytes().to_vec();
        skin.cape_id.clone_from(&data.cape_id);
        skin.full_id = if data.cape_id.is_empty() {
            data.skin_id.clone()
        } else {
            format!("{}{}", data.skin_id, data.cape_id)
        };
        if !data.skin_colour.is_empty() {
            skin.skin_color.clone_from(&data.skin_colour);
        }
        skin.persona_pieces = data
            .persona_pieces
            .iter()
            .map(PersonaPiece::from_login)
            .collect();
        skin.piece_tint_colors = data
            .piece_tint_colours
            .iter()
            .map(PieceTintColor::from_login)
            .collect();
        skin.is_premium = data.premium_skin;
        skin.is_persona = data.persona_skin;
        skin.persona_cape_on_classic = data.cape_on_classic_skin;
        // Viewers drop untrusted skins. Login already proved this is the player's own data.
        skin.is_trusted = true;
        skin.override_appearance = true;
        Some(skin)
    }
}

fn decode_b64(value: &str) -> Vec<u8> {
    BASE64_STANDARD
        .decode(value.trim().as_bytes())
        .unwrap_or_default()
}

fn decode_text_field(value: &str) -> Option<Vec<u8>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(
        BASE64_STANDARD
            .decode(trimmed.as_bytes())
            .unwrap_or_else(|_| trimmed.as_bytes().to_vec()),
    )
}

const fn rgba_dimensions(data: &[u8], width: i32, height: i32) -> Option<(u32, u32)> {
    if data.len() < 64 * 32 * 4 || !data.len().is_multiple_of(4) {
        return None;
    }
    let pixels = data.len() / 4;
    if width > 0 && height > 0 && width as usize * height as usize == pixels {
        return Some((width as u32, height as u32));
    }
    match pixels {
        8192 => Some((64, 32)),
        16384 => Some((64, 64)),
        65536 => Some((128, 128)),
        _ => None,
    }
}

fn persona_piece_type(name: &str) -> i32 {
    let name = name
        .strip_prefix("persona_")
        .unwrap_or(name)
        .replace('-', "_");
    match name.to_ascii_lowercase().as_str() {
        "skeleton" => 1,
        "body" => 2,
        "skin" => 3,
        "bottom" => 4,
        "feet" => 5,
        "dress" => 6,
        "top" => 7,
        "high_pants" | "highpants" => 8,
        "hands" | "hand" => 9,
        "outerwear" => 10,
        "facial_hair" | "facialhair" => 11,
        "mouth" => 12,
        "eyes" => 13,
        "hair" => 14,
        "hood" => 15,
        "back" => 16,
        "face_accessory" | "faceaccessory" => 17,
        "head" => 18,
        "legs" => 19,
        "left_leg" | "leftleg" => 20,
        "right_leg" | "rightleg" => 21,
        "arms" => 22,
        "left_arm" | "leftarm" => 23,
        "right_arm" | "rightarm" => 24,
        "capes" | "cape" => 25,
        "classic_skin" | "classicskin" => 26,
        "emote" => 27,
        "coco" => 28,
        "unsupported" => 29,
        _ => 0,
    }
}

impl PacketWrite for Skin {
    fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.skin_id.write(writer)?;
        self.play_fab_id.write(writer)?;
        VarUInt(self.resource_patch.len() as u32).write(writer)?;
        writer.write_all(&self.resource_patch)?;
        self.image_width.write(writer)?;
        self.image_height.write(writer)?;
        VarUInt(self.skin_data.len() as u32).write(writer)?;
        writer.write_all(&self.skin_data)?;
        VarUInt(self.animations.len() as u32).write(writer)?;
        for anim in &self.animations {
            anim.write(writer)?;
        }
        self.cape_width.write(writer)?;
        self.cape_height.write(writer)?;
        VarUInt(self.cape_data.len() as u32).write(writer)?;
        writer.write_all(&self.cape_data)?;
        VarUInt(self.geometry_data.len() as u32).write(writer)?;
        writer.write_all(&self.geometry_data)?;
        VarUInt(self.geometry_data_engine_version.len() as u32).write(writer)?;
        writer.write_all(&self.geometry_data_engine_version)?;
        VarUInt(self.animation_data.len() as u32).write(writer)?;
        writer.write_all(&self.animation_data)?;
        self.cape_id.write(writer)?;
        self.full_id.write(writer)?;
        u8::from(!self.arm_size.eq_ignore_ascii_case("slim")).write(writer)?;
        parse_color(&self.skin_color).write(writer)?;
        VarUInt(self.persona_pieces.len() as u32).write(writer)?;
        for piece in &self.persona_pieces {
            piece.write(writer)?;
        }
        VarUInt(self.piece_tint_colors.len() as u32).write(writer)?;
        for color in &self.piece_tint_colors {
            color.write(writer)?;
        }
        self.is_premium.write(writer)?;
        self.is_persona.write(writer)?;
        self.persona_cape_on_classic.write(writer)?;
        self.is_primary_user.write(writer)?;
        self.override_appearance.write(writer)?;
        self.is_trusted.to_string().write(writer)?;
        self.profile_hash.write(writer)
    }
}

#[derive(Clone)]
pub struct SkinAnimation {
    pub image_width: u32,
    pub image_height: u32,
    pub image_data: Vec<u8>,
    pub animation_type: u32,
    pub frames: f32,
    pub expression_type: u32,
}

impl SkinAnimation {
    fn from_login(anim: &crate::bedrock::server::login::SkinAnimation) -> Option<Self> {
        let image_data = decode_b64(&anim.image);
        let (image_width, image_height) =
            rgba_dimensions(&image_data, anim.image_width, anim.image_height)?;
        Some(Self {
            image_width,
            image_height,
            image_data,
            animation_type: anim.animation_type.max(0) as u32,
            frames: anim.frames as f32,
            expression_type: anim.animation_expression.max(0) as u32,
        })
    }
}

impl PacketWrite for SkinAnimation {
    fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.image_width.write(writer)?;
        self.image_height.write(writer)?;
        VarUInt(self.image_data.len() as u32).write(writer)?;
        writer.write_all(&self.image_data)?;
        VarUInt(self.animation_type).write(writer)?;
        self.frames.write(writer)?;
        VarUInt(self.expression_type).write(writer)
    }
}

#[derive(Clone, PacketWrite)]
pub struct PersonaPiece {
    pub piece_id: String,
    pub piece_type: i32,
    pub pack_id: Uuid,
    pub is_default: bool,
    pub product_id: String,
}

impl PersonaPiece {
    fn from_login(piece: &crate::bedrock::server::login::PersonaPiece) -> Self {
        Self {
            piece_id: piece.piece_id.clone(),
            piece_type: persona_piece_type(&piece.piece_type),
            pack_id: Uuid::parse_str(&piece.pack_id).unwrap_or(Uuid::nil()),
            is_default: piece.is_default,
            product_id: piece.product_id.clone(),
        }
    }
}

#[derive(Clone)]
pub struct PieceTintColor {
    pub piece_type: String,
    pub colors: [i32; 4],
}

impl PieceTintColor {
    fn from_login(tint: &crate::bedrock::server::login::PersonaPieceTintColour) -> Self {
        Self {
            piece_type: tint.piece_type.clone(),
            colors: tint.colours.each_ref().map(|color| parse_color(color)),
        }
    }
}

impl PacketWrite for PieceTintColor {
    fn write<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        let piece_type = if self.piece_type == "persona_hand" {
            "hands"
        } else {
            self.piece_type
                .strip_prefix("persona_")
                .unwrap_or(&self.piece_type)
        };
        piece_type.write(writer)?;
        for color in self.colors {
            color.write(writer)?;
        }
        Ok(())
    }
}

fn parse_color(color: &str) -> i32 {
    let value = color.trim_start_matches('#');
    u32::from_str_radix(value, 16).unwrap_or_default() as i32
}

#[cfg(test)]
mod tests {
    use super::{
        BASE64_STANDARD, DEFAULT_SKIN_GEOMETRY, SLIM_SKIN_RESOURCE_PATCH, Skin,
        WIDE_SKIN_RESOURCE_PATCH, persona_piece_type,
    };
    use base64::Engine;

    #[test]
    fn fallback_skin_contains_the_geometry_it_references() {
        let skin = Skin::steve();

        assert_eq!(skin.resource_patch, WIDE_SKIN_RESOURCE_PATCH);
        assert_eq!(skin.geometry_data, DEFAULT_SKIN_GEOMETRY);
        assert!(
            String::from_utf8_lossy(&skin.geometry_data)
                .contains(r#""identifier":"geometry.humanoid.custom""#)
        );
        assert!(skin.override_appearance);
        assert!(skin.is_trusted);
    }

    #[test]
    fn slim_fallback_skin_references_the_slim_geometry() {
        let mut skin = Skin::steve();
        skin.set_slim(true);

        assert_eq!(skin.arm_size, "slim");
        assert_eq!(skin.resource_patch, SLIM_SKIN_RESOURCE_PATCH);
        assert!(
            String::from_utf8_lossy(&skin.geometry_data)
                .contains(r#""identifier":"geometry.humanoid.customSlim""#)
        );
    }

    #[test]
    fn client_data_skin_keeps_login_pixels_and_slim_geometry() {
        let pixels = vec![1u8; 64 * 64 * 4];
        let json = serde_json::json!({
            "ClientRandomId": 0,
            "DeviceOS": 1,
            "DeviceId": "dev",
            "GameVersion": "1.26.45",
            "LanguageCode": "en_US",
            "CurrentInputMode": 1,
            "DefaultInputMode": 1,
            "UIProfile": 0,
            "ServerAddress": "127.0.0.1",
            "MaxViewDistance": 12,
            "SkinId": "custom-slim",
            "SkinData": BASE64_STANDARD.encode(&pixels),
            "SkinImageWidth": 64,
            "SkinImageHeight": 64,
            "ArmSize": "slim",
            "CapeId": "cape-1",
            "PlayFabId": "pf",
            "PersonaSkin": false,
            "PremiumSkin": true,
            "SkinGeometryData": BASE64_STANDARD.encode(br#"{"minecraft:geometry":[]}"#),
        });
        let data: crate::bedrock::server::login::ClientData =
            serde_json::from_value(json).expect("client data");
        let skin = Skin::from_client_data(&data).expect("skin");

        assert_eq!(skin.skin_id, "custom-slim");
        assert_eq!(skin.full_id, "custom-slimcape-1");
        assert_eq!(skin.play_fab_id, "pf");
        assert_eq!(skin.arm_size, "slim");
        assert_eq!(skin.resource_patch, SLIM_SKIN_RESOURCE_PATCH);
        assert_eq!(skin.skin_data, pixels);
        assert_eq!(skin.geometry_data, br#"{"minecraft:geometry":[]}"#);
        assert!(skin.is_trusted);
        assert!(skin.override_appearance);
        assert!(skin.is_premium);
    }

    #[test]
    fn empty_login_skin_falls_back() {
        let json = serde_json::json!({
            "ClientRandomId": 0,
            "DeviceOS": 1,
            "DeviceId": "dev",
            "GameVersion": "1.26.45",
            "LanguageCode": "en_US",
            "CurrentInputMode": 1,
            "DefaultInputMode": 1,
            "UIProfile": 0,
            "ServerAddress": "127.0.0.1",
            "MaxViewDistance": 12,
        });
        let data: crate::bedrock::server::login::ClientData =
            serde_json::from_value(json).expect("client data");
        assert!(Skin::from_client_data(&data).is_none());
    }

    #[test]
    fn persona_piece_names_map_to_protocol_values() {
        assert_eq!(persona_piece_type("persona_hair"), 14);
        assert_eq!(persona_piece_type("persona_hands"), 9);
        assert_eq!(persona_piece_type("classic_skin"), 26);
        assert_eq!(persona_piece_type("nope"), 0);
    }
}
