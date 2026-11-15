//! Custom movement states of mods (a dodge, a wall run): what the server
//! allows over `melon:movement` and how far its checks bend for them.
//!
//! From `data/<ns>/movements/*.json` (via
//! [`load_extracted_dir`](crate::mod_registry::load_extracted_dir)) or a
//! plugin at runtime. The client side of a state is the mod's own code.

use std::path::Path;
use std::sync::{Mutex, PoisonError};

use serde::Deserialize;

/// One allowed state. Only `id` is needed.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MovementSpec {
    /// `namespace:name`, as the client's state reports it.
    pub id: String,
    /// Ticks after a start that skip "moved wrongly"
    /// (`isInPostImpulseGraceTime`) and keep fall damage off.
    pub grace_ticks: u32,
    /// Extra blocks per tick "moved too quickly" accepts in the grace.
    pub extra_speed: f64,
    /// Ticks before the same player may start it again.
    pub cooldown_ticks: u32,
    /// Tell the players that see this one, for its look.
    pub broadcast: bool,
}

impl Default for MovementSpec {
    fn default() -> Self {
        Self {
            id: String::new(),
            grace_ticks: 10,
            extra_speed: 1.0,
            cooldown_ticks: 0,
            broadcast: true,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum MovementError {
    /// Not `namespace:name`, or longer than the protocol allows.
    BadId,
}

static MOVEMENTS: Mutex<Vec<MovementSpec>> = Mutex::new(Vec::new());

fn lock() -> std::sync::MutexGuard<'static, Vec<MovementSpec>> {
    MOVEMENTS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Adds `spec`, or replaces the one with its id.
pub fn register_movement(spec: MovementSpec) -> Result<(), MovementError> {
    let valid = spec.id.len() <= 256
        && spec
            .id
            .split_once(':')
            .is_some_and(|(ns, name)| !ns.is_empty() && !name.is_empty());
    if !valid {
        return Err(MovementError::BadId);
    }
    let mut movements = lock();
    match movements.iter_mut().find(|m| m.id == spec.id) {
        Some(existing) => *existing = spec,
        None => movements.push(spec),
    }
    Ok(())
}

/// Removes the state `id`; whether it was there.
pub fn unregister_movement(id: &str) -> bool {
    let mut movements = lock();
    let before = movements.len();
    movements.retain(|m| m.id != id);
    movements.len() != before
}

#[must_use]
pub fn movement(id: &str) -> Option<MovementSpec> {
    lock().iter().find(|m| m.id == id).cloned()
}

/// Every allowed state, in registration order.
#[must_use]
pub fn movements() -> Vec<MovementSpec> {
    lock().clone()
}

/// Registers one `data/<ns>/movements/*.json`.
pub(crate) fn load_file(path: &Path) {
    let parsed = std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|text| serde_json::from_str::<MovementSpec>(&text).map_err(|e| e.to_string()))
        .and_then(|spec| register_movement(spec).map_err(|e| format!("{e:?}")));
    if let Err(e) = parsed {
        // pumpkin-data has no logger; a skipped mod movement must still show.
        #[allow(clippy::print_stderr)]
        {
            eprintln!("mod movement {} unreadable: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_needs_a_namespaced_id_and_replaces_its_namesake() {
        assert_eq!(
            register_movement(MovementSpec {
                id: "dodge".into(),
                ..MovementSpec::default()
            }),
            Err(MovementError::BadId)
        );
        let spec: MovementSpec =
            serde_json::from_str(r#"{"id":"test_mm:dodge","cooldown_ticks":20}"#).unwrap();
        assert_eq!((spec.grace_ticks, spec.cooldown_ticks), (10, 20));
        register_movement(spec.clone()).unwrap();
        register_movement(MovementSpec {
            broadcast: false,
            ..spec
        })
        .unwrap();
        let found = movement("test_mm:dodge").unwrap();
        assert!(!found.broadcast);
        assert_eq!(
            movements()
                .iter()
                .filter(|m| m.id == "test_mm:dodge")
                .count(),
            1
        );
        assert!(unregister_movement("test_mm:dodge"));
        assert!(movement("test_mm:dodge").is_none());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(serde_json::from_str::<MovementSpec>(r#"{"id":"a:b","speed":3}"#).is_err());
    }
}
