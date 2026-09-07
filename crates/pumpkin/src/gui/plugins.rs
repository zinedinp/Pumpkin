//! Builds the plugin list the GUI renders.
//!
//! Sent on connect and after a plugin is loaded, unloaded or reloaded — not with every snapshot,
//! since plugin state changes are rare.

use std::sync::Arc;

use pumpkin_gui_api::{PluginKind, PluginPermission, PluginRow, PluginState};

use crate::plugin::{InactivePlugin, PluginMetadata, permissions::get_permission_description};
use crate::server::Server;

/// Everything a `ServerMessage::Plugins` carries: the rows plus the global hot-reload flag.
pub async fn snapshot(server: &Arc<Server>) -> (Vec<PluginRow>, bool) {
    (
        collect(server).await,
        server.plugin_manager.is_hot_reload_enabled(),
    )
}

/// Collects every plugin the manager knows about, running or not.
///
/// Four sources, because a plugin stops being "loaded" in four different ways and each keeps its
/// own record: the manager's list, the inactive map, the failed-state map, and the files no loader
/// accepted. A plugin missing from the list is exactly when someone goes looking for it.
pub async fn collect(server: &Arc<Server>) -> Vec<PluginRow> {
    let manager = &server.plugin_manager;
    let loading = manager.get_loading_plugins().await;
    let failed = manager.get_failed_plugins().await;

    let mut rows: Vec<PluginRow> = manager
        .plugin_entries()
        .into_iter()
        .map(|entry| PluginRow {
            state: state_of(&entry.metadata.name, entry.active, &loading, &failed),
            kind: kind_of(&entry.path),
            path: entry.path.to_string_lossy().into_owned(),
            active: entry.active,
            can_unload: entry.can_unload,
            ..from_metadata(entry.metadata)
        })
        .collect();

    for plugin in manager.inactive_plugins().await {
        push_unless_known(&mut rows, inactive_row(plugin));
    }

    for (name, error) in failed {
        push_unless_known(
            &mut rows,
            PluginRow {
                state: PluginState::Failed(error),
                name,
                ..PluginRow::default()
            },
        );
    }

    for path in manager.unloaded_files().await {
        // Only files that claim to be plugins. Everything else in the directory (a README, a
        // config, a jar)
        let Some(kind) = plugin_kind(&path) else {
            continue;
        };
        push_unless_known(
            &mut rows,
            PluginRow {
                state: PluginState::Failed("No plugin loader accepted this file.".to_owned()),
                path: path.to_string_lossy().into_owned(),
                kind,
                ..named_after_file(&path)
            },
        );
    }

    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

/// Keeps the first, richer record of a plugin when a later source names it again.
fn push_unless_known(rows: &mut Vec<PluginRow>, row: PluginRow) {
    if !rows.iter().any(|known| known.name == row.name) {
        rows.push(row);
    }
}

fn inactive_row(plugin: InactivePlugin) -> PluginRow {
    // A plugin the loader rejected never named itself, so the file has to stand in for it.
    let described = plugin
        .metadata
        .map_or_else(|| named_after_file(&plugin.path), from_metadata);

    PluginRow {
        // An empty reason means it was unloaded on request, not that it broke.
        state: if plugin.reason.is_empty() {
            PluginState::Unloaded
        } else {
            PluginState::Failed(plugin.reason)
        },
        kind: kind_of(&plugin.path),
        path: plugin.path.to_string_lossy().into_owned(),
        active: false,
        can_unload: false,
        ..described
    }
}

/// The stand-in row for a file that never told us what it is.
fn named_after_file(path: &std::path::Path) -> PluginRow {
    PluginRow {
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        ..PluginRow::default()
    }
}

/// The half of a row that is the plugin's own description of itself.
fn from_metadata(metadata: PluginMetadata) -> PluginRow {
    PluginRow {
        name: metadata.name,
        version: metadata.version,
        authors: metadata.authors,
        description: metadata.description,
        dependencies: metadata.dependencies,
        permissions: metadata
            .permissions
            .into_iter()
            .map(describe_permission)
            .collect(),
        ..PluginRow::default()
    }
}

/// Pairs a permission with its explanation, so the GUI can say what granting it means.
fn describe_permission(name: String) -> PluginPermission {
    let description = get_permission_description(&name)
        .unwrap_or_default()
        .to_owned();
    PluginPermission { name, description }
}

fn state_of(
    name: &str,
    active: bool,
    loading: &[String],
    failed: &[(String, String)],
) -> PluginState {
    if let Some((_, error)) = failed.iter().find(|(failed, _)| failed == name) {
        return PluginState::Failed(error.clone());
    }
    if loading.iter().any(|plugin| plugin == name) {
        return PluginState::Loading;
    }
    // Still listed but not running: a loader that cannot unload keeps the entry and clears the
    // flag instead of dropping it.
    if active {
        PluginState::Loaded
    } else {
        PluginState::Unloaded
    }
}

/// A loaded plugin's file is loadable by definition, so anything that is not wasm is native.
fn kind_of(path: &std::path::Path) -> PluginKind {
    if path.extension().is_some_and(|ext| ext == "wasm") {
        PluginKind::Wasm
    } else {
        PluginKind::Native
    }
}

/// The same question for a file that never loaded, where the extension is all there is to go on.
fn plugin_kind(path: &std::path::Path) -> Option<PluginKind> {
    match path.extension()?.to_str()? {
        "wasm" => Some(PluginKind::Wasm),
        "so" | "dll" | "dylib" => Some(PluginKind::Native),
        _ => None,
    }
}
