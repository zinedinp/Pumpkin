//! Serves the configuration file to the GUI editor and writes it back.
use pumpkin_config::{LoadConfiguration, PumpkinConfig};
use pumpkin_gui_api::ConfigFile;

/// The file's path, resolved the same way `main` resolves it at startup.
fn path_of(file: ConfigFile) -> std::path::PathBuf {
    let dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    match file {
        ConfigFile::Pumpkin => dir.join(PumpkinConfig::get_path()),
    }
}

/// Reads the file as it is on disk, rather than re-serializing
pub fn read(file: ConfigFile) -> Result<String, String> {
    let path = path_of(file);
    std::fs::read_to_string(&path).map_err(|err| format!("Could not read {}: {err}", path.display()))
}

/// Parses `toml` and only then replaces the file, so a rejected edit leaves the old file intact.
///
/// Deliberately does not call `validate()`: it asserts, and a panic here would take the running
/// server down over a bad value in a file that is not even loaded yet.
pub fn write(file: ConfigFile, toml: &str) -> Result<(), String> {
    let parsed: toml::Value = toml::from_str(toml).map_err(|err| err.to_string())?;

    let config: PumpkinConfig = parsed
        .clone()
        .try_into()
        .map_err(|err: toml::de::Error| err.to_string())?;

    let round_trip = toml::Value::try_from(&config).map_err(|err| err.to_string())?;
    let mut unknown = Vec::new();
    collect_unknown(&parsed, &round_trip, &mut String::new(), &mut unknown);
    if !unknown.is_empty() {
        return Err(format!("unknown setting(s): {}", unknown.join(", ")));
    }

    let path = path_of(file);
    std::fs::write(&path, toml).map_err(|err| format!("Could not write {}: {err}", path.display()))
}

/// Records every key in `submitted` that the parsed config does not have, as a dotted path.
fn collect_unknown(
    submitted: &toml::Value,
    known: &toml::Value,
    prefix: &mut String,
    out: &mut Vec<String>,
) {
    let (Some(submitted), Some(known)) = (submitted.as_table(), known.as_table()) else {
        return;
    };

    for (key, value) in submitted {
        let restore = prefix.len();
        if !prefix.is_empty() {
            prefix.push('.');
        }
        prefix.push_str(key);

        match known.get(key) {
            Some(known) => collect_unknown(value, known, prefix, out),
            None => out.push(prefix.clone()),
        }

        prefix.truncate(restore);
    }
}
