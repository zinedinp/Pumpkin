use serde::{Deserialize, Serialize};

/// How the zip ships code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CodeKind {
    #[default]
    None,
    Wasm,
    Source,
    Prebuilt,
}

/// `side` in `melon-mod.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Client,
    Server,
    #[default]
    Both,
}

impl Side {
    #[must_use]
    pub const fn on_server(self) -> bool {
        matches!(self, Self::Server | Self::Both)
    }

    #[must_use]
    pub const fn on_client(self) -> bool {
        matches!(self, Self::Client | Self::Both)
    }
}

/// Root of `melon-mod.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub side: Side,
    #[serde(default = "default_api")]
    pub api: u32,
    #[serde(default)]
    pub code: CodeKind,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub name: Option<String>,
}

const fn default_api() -> u32 {
    1
}

impl Manifest {
    /// Parse a `melon-mod.toml` body.
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

/// Which zips in `mods/` the player turned off.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModsState {
    #[serde(default)]
    pub disabled: Vec<String>,
}

impl ModsState {
    #[must_use]
    pub fn is_enabled(&self, id: &str) -> bool {
        !self.disabled.iter().any(|d| d == id)
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        self.disabled.retain(|d| d != id);
        if !enabled {
            self.disabled.push(id.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_minimal_manifest() {
        let m = Manifest::parse(
            r#"
id = "hello"
version = "0.1.0"
"#,
        )
        .unwrap();
        assert_eq!(m.id, "hello");
        assert_eq!(m.api, 1);
        assert_eq!(m.code, CodeKind::None);
        assert_eq!(m.side, Side::Both);
    }
}
