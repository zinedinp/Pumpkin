use serde::{Deserialize, Serialize};

/// Zip-mod folder: content, WASM, and native cdylibs. Separate from `plugins`.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct ModsConfig {
    /// Scan `dir` for `.zip` mods at start.
    pub enabled: bool,
    /// Folder of `.zip` files, relative to the working directory.
    pub dir: String,
    /// Run rustc on `src/` in a zip when no prebuilt lib for this OS/Arch exists.
    pub compile_source: bool,
}

impl Default for ModsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dir: "./mods".to_owned(),
            compile_source: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_enables_the_mods_folder() {
        let config = ModsConfig::default();
        assert!(config.enabled);
        assert_eq!(config.dir, "./mods");
        assert!(config.compile_source);
    }
}
