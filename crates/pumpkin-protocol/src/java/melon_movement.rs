//! `melon:movement`: custom movement states (a mod's dodge, a wall run)
//! between Melon clients and a server that allows them.
//!
//! - `Allowed` (server → client): the states the server accepts; a client
//!   runs no others. Sent once the client registers the channel, and again
//!   when the list changes.
//! - `Start` (client → server): the player began a state.
//! - `Started` (server → client): another player began one, for its look.
//!
//! One tag byte, then `VarInt`s and `VarInt`-prefixed UTF-8 strings.

/// The plugin channel.
pub const CHANNEL: &str = "melon:movement";

/// Longest state id.
pub const MAX_ID: usize = 256;
/// Most states in one `Allowed`.
pub const MAX_STATES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MovementPayload {
    Allowed(Vec<String>),
    Start { id: String },
    Started { entity_id: i32, id: String },
}

const ALLOWED: u8 = 0;
const START: u8 = 1;
const STARTED: u8 = 2;

#[derive(Debug, PartialEq, Eq)]
pub struct DecodeError(&'static str);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "melon:movement: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

impl MovementPayload {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::Allowed(ids) => {
                out.push(ALLOWED);
                write_var_int(&mut out, ids.len() as i32);
                for id in ids {
                    write_str(&mut out, id);
                }
            }
            Self::Start { id } => {
                out.push(START);
                write_str(&mut out, id);
            }
            Self::Started { entity_id, id } => {
                out.push(STARTED);
                write_var_int(&mut out, *entity_id);
                write_str(&mut out, id);
            }
        }
        out
    }

    pub fn decode(mut data: &[u8]) -> Result<Self, DecodeError> {
        let r = &mut data;
        let payload = match take(r, 1)?[0] {
            ALLOWED => {
                let count = usize::try_from(read_var_int(r)?)
                    .ok()
                    .filter(|n| *n <= MAX_STATES)
                    .ok_or(DecodeError("too many states"))?;
                Self::Allowed((0..count).map(|_| read_str(r)).collect::<Result<_, _>>()?)
            }
            START => Self::Start { id: read_str(r)? },
            STARTED => Self::Started {
                entity_id: read_var_int(r)?,
                id: read_str(r)?,
            },
            _ => return Err(DecodeError("unknown tag")),
        };
        if r.is_empty() {
            Ok(payload)
        } else {
            Err(DecodeError("trailing bytes"))
        }
    }
}

const fn take<'a>(r: &mut &'a [u8], n: usize) -> Result<&'a [u8], DecodeError> {
    if r.len() < n {
        return Err(DecodeError("truncated"));
    }
    let (head, rest) = r.split_at(n);
    *r = rest;
    Ok(head)
}

fn write_var_int(out: &mut Vec<u8>, value: i32) {
    let mut v = value as u32;
    loop {
        if v & !0x7f == 0 {
            out.push(v as u8);
            return;
        }
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
}

fn read_var_int(r: &mut &[u8]) -> Result<i32, DecodeError> {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = take(r, 1)?[0];
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value as i32);
        }
    }
    Err(DecodeError("VarInt too long"))
}

fn write_str(out: &mut Vec<u8>, s: &str) {
    write_var_int(out, s.len() as i32);
    out.extend_from_slice(s.as_bytes());
}

fn read_str(r: &mut &[u8]) -> Result<String, DecodeError> {
    let len = usize::try_from(read_var_int(r)?)
        .ok()
        .filter(|n| *n <= MAX_ID)
        .ok_or(DecodeError("id too long"))?;
    let bytes = take(r, len)?;
    String::from_utf8(bytes.to_vec()).map_err(|_| DecodeError("id not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_payload_round_trips() {
        for payload in [
            MovementPayload::Allowed(vec!["example:dodge".into(), "x:wall_run".into()]),
            MovementPayload::Allowed(Vec::new()),
            MovementPayload::Start {
                id: "example:dodge".into(),
            },
            MovementPayload::Started {
                entity_id: -7,
                id: "example:dodge".into(),
            },
        ] {
            assert_eq!(MovementPayload::decode(&payload.encode()), Ok(payload));
        }
    }

    #[test]
    fn broken_payloads_are_errors() {
        assert!(MovementPayload::decode(&[]).is_err());
        assert!(MovementPayload::decode(&[9]).is_err());
        assert!(MovementPayload::decode(&[START, 5, b'a']).is_err());
        let mut long = MovementPayload::Start { id: "a".into() }.encode();
        long.push(0);
        assert!(MovementPayload::decode(&long).is_err());
        let mut huge = vec![ALLOWED];
        write_var_int(&mut huge, 100_000);
        assert!(MovementPayload::decode(&huge).is_err());
    }
}
