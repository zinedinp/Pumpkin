//! `pumpkin --version`

/// Every `pumpkin --version` line starts with this.
pub const VERSION_PREFIX: &str = "pumpkin ";
/// The capability token a `gui`-feature build appends as the last item in the
/// parenthesised group, e.g. `pumpkin 0.1.0-dev (commit 38fd0c2, release, gui)`.
pub const GUI_VERSION_MARKER: &str = "gui";

/// Builds the `pumpkin --version` line the functions below parse.
#[must_use]
pub fn format_version_line(version: &str, commit: &str, profile: &str, gui: bool) -> String {
    let marker = if gui {
        format!(", {GUI_VERSION_MARKER}")
    } else {
        String::new()
    };
    format!("{VERSION_PREFIX}{version} (commit {commit}, {profile}{marker})")
}

/// True if `line` looks like a Pumpkin version line at all, regardless of capability.
#[must_use]
pub fn is_pumpkin_version_line(line: &str) -> bool {
    line.trim().starts_with(VERSION_PREFIX)
}

/// True if `line` is a Pumpkin version line from a build with the `gui` feature.
#[must_use]
pub fn is_gui_capable_version_line(line: &str) -> bool {
    let line = line.trim();
    if !line.starts_with(VERSION_PREFIX) {
        return false;
    }
    let Some(open) = line.find('(') else {
        return false;
    };
    let Some(close) = line.rfind(')') else {
        return false;
    };
    if close < open {
        return false;
    }
    line[open + 1..close]
        .split(',')
        .any(|token| token.trim() == GUI_VERSION_MARKER)
}
