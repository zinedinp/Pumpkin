//! Builds the plugin list the GUI renders.
//!
//! Sent on connect and after a plugin is loaded, unloaded or reloaded — not with every snapshot,
//! since plugin state changes are rare.

use std::sync::Arc;

use pumpkin_gui_api::{PluginKind, PluginRow, PluginState};

use crate::server::Server;

/// Collects every plugin the manager knows about, loaded or not.
pub async fn collect(server: &Arc<Server>) -> Vec<PluginRow> {
    let manager = &server.plugin_manager;
    let loading = manager.get_loading_plugins().await;
    let failed = manager.get_failed_plugins().await;

    let mut rows: Vec<PluginRow> = manager
        .plugin_entries()
        .into_iter()
        .map(|entry| {
            let name = entry.metadata.name;
            let state = state_of(&name, &loading, &failed);
            PluginRow {
                version: entry.metadata.version,
                authors: entry.metadata.authors,
                description: entry.metadata.description,
                dependencies: entry.metadata.dependencies,
                permissions: entry.metadata.permissions,
                kind: kind_of(&entry.path),
                path: entry.path.to_string_lossy().into_owned(),
                active: entry.active,
                can_unload: entry.can_unload,
                state,
                name,
            }
        })
        .collect();

    // A plugin that failed to load never made it into the manager's list, so it would otherwise be
    // invisible - which is exactly when a user goes looking for it.
    for (name, error) in failed {
        if !rows.iter().any(|row| row.name == name) {
            rows.push(PluginRow {
                state: PluginState::Failed(error),
                name,
                ..PluginRow::default()
            });
        }
    }

    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

fn state_of(name: &str, loading: &[String], failed: &[(String, String)]) -> PluginState {
    if let Some((_, error)) = failed.iter().find(|(failed, _)| failed == name) {
        return PluginState::Failed(error.clone());
    }
    if loading.iter().any(|plugin| plugin == name) {
        return PluginState::Loading;
    }
    PluginState::Loaded
}

fn kind_of(path: &std::path::Path) -> PluginKind {
    if path.extension().is_some_and(|ext| ext == "wasm") {
        PluginKind::Wasm
    } else {
        PluginKind::Native
    }
}
