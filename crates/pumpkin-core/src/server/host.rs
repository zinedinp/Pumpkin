//! What the host of an embedded server controls while it runs.
//!
//! The host is a client's integrated server (vanilla `IntegratedServer`): the
//! pause, the owner's rights and the rules for guests once the world is
//! opened to others. A standalone server is never embedded, and nothing here
//! changes it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use pumpkin_util::PermissionLvl;

/// The host's controls. Off until [`Host::embed`].
pub struct Host {
    embedded: AtomicBool,
    /// The host's game is paused (vanilla `Minecraft.isPaused`).
    paused: AtomicBool,
    /// Open to others: on LAN.
    published: AtomicBool,
    /// Guests are verified with Mojang (Java) or Xbox (Bedrock); the config
    /// decides on a standalone server.
    online_mode: AtomicBool,
    /// The world's "Allow Commands".
    allow_commands: AtomicBool,
    /// Guests get the game master's commands (`guestCommandAccess`).
    guest_command_access: AtomicBool,
    /// Guests play the world's game mode (`forceGameMode`, on by default).
    force_game_mode: AtomicBool,
    hardcore: AtomicBool,
    /// The owner's name: vanilla `isSingleplayerOwner` compares names.
    owner: Mutex<Option<String>>,
}

impl Host {
    #[must_use]
    pub const fn new(hardcore: bool) -> Self {
        Self {
            embedded: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            published: AtomicBool::new(false),
            online_mode: AtomicBool::new(true),
            allow_commands: AtomicBool::new(false),
            guest_command_access: AtomicBool::new(false),
            force_game_mode: AtomicBool::new(true),
            hardcore: AtomicBool::new(hardcore),
            owner: Mutex::new(None),
        }
    }

    /// Makes this an embedded server owned by the player `owner`.
    pub fn embed(&self, owner: &str, allow_commands: bool) {
        *self
            .owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(owner.to_owned());
        self.allow_commands.store(allow_commands, Ordering::Relaxed);
        self.embedded.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_embedded(&self) -> bool {
        self.embedded.load(Ordering::Acquire)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// Whether this tick pauses: an embedded server pauses with its host's
    /// game and while no one is on it (`IntegratedServer.tickServer`).
    #[must_use]
    pub fn pauses(&self, has_players: bool) -> bool {
        self.is_embedded() && (self.paused.load(Ordering::Relaxed) || !has_players)
    }

    pub fn set_published(&self, published: bool) {
        self.published.store(published, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_published(&self) -> bool {
        self.published.load(Ordering::Relaxed)
    }

    pub fn set_online_mode(&self, online: bool) {
        self.online_mode.store(online, Ordering::Relaxed);
    }

    /// Whether guests are verified: the host's switch when embedded, else `configured`.
    #[must_use]
    pub fn online_mode(&self, configured: bool) -> bool {
        if self.is_embedded() {
            self.online_mode.load(Ordering::Relaxed)
        } else {
            configured
        }
    }

    pub fn set_allow_commands(&self, allow: bool) {
        self.allow_commands.store(allow, Ordering::Relaxed);
    }

    #[must_use]
    pub fn allow_commands(&self) -> bool {
        self.allow_commands.load(Ordering::Relaxed)
    }

    pub fn set_guest_command_access(&self, access: bool) {
        self.guest_command_access.store(access, Ordering::Relaxed);
    }

    #[must_use]
    pub fn guest_command_access(&self) -> bool {
        self.guest_command_access.load(Ordering::Relaxed)
    }

    pub fn set_force_game_mode(&self, force: bool) {
        self.force_game_mode.store(force, Ordering::Relaxed);
    }

    #[must_use]
    pub fn force_game_mode(&self) -> bool {
        self.force_game_mode.load(Ordering::Relaxed)
    }

    pub fn set_hardcore(&self, hardcore: bool) {
        self.hardcore.store(hardcore, Ordering::Relaxed);
    }

    /// `isSingleplayerOwner`.
    #[must_use]
    pub fn is_owner(&self, name: &str) -> bool {
        self.owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_deref()
            .is_some_and(|owner| owner.eq_ignore_ascii_case(name))
    }

    /// `IntegratedServer.getProfilePermissions`: the owner is an operator
    /// while commands are allowed; guests get `guestCommandAccess`'s level.
    /// Otherwise, and on a standalone server, the operator list's level.
    #[must_use]
    pub fn permission_level(
        &self,
        name: &str,
        listed: Option<PermissionLvl>,
        default: PermissionLvl,
    ) -> PermissionLvl {
        let listed = listed.unwrap_or(default);
        if !self.is_embedded() {
            return listed;
        }
        let allow = self.allow_commands();
        if self.is_owner(name) {
            return if allow { PermissionLvl::Four } else { listed };
        }
        match (allow, self.guest_command_access()) {
            (true, true) => PermissionLvl::Two,
            (true, false) => PermissionLvl::Zero,
            (false, _) => listed,
        }
    }

    /// `getForcedGameType`: guests play the world's game mode while the world
    /// is open, outside hardcore.
    #[must_use]
    pub fn forces_game_mode(&self) -> bool {
        self.is_embedded()
            && self.force_game_mode()
            && self.is_published()
            && !self.hardcore.load(Ordering::Relaxed)
    }
}

/// What a tick does to the pause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseStep {
    Running,
    /// Pausing now: save first.
    Pausing,
    Paused,
    /// Running again: the clients' time is synced.
    Resuming,
}

impl PauseStep {
    #[must_use]
    pub const fn between(was_paused: bool, paused: bool) -> Self {
        match (was_paused, paused) {
            (false, false) => Self::Running,
            (false, true) => Self::Pausing,
            (true, true) => Self::Paused,
            (true, false) => Self::Resuming,
        }
    }

    #[must_use]
    pub const fn is_paused(self) -> bool {
        matches!(self, Self::Pausing | Self::Paused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn embedded() -> Host {
        let host = Host::new(false);
        host.embed("Steve", true);
        host
    }

    #[test]
    fn a_standalone_server_never_pauses_and_keeps_its_config() {
        let host = Host::new(false);
        host.set_paused(true);
        host.set_online_mode(false);
        assert!(!host.pauses(false));
        assert!(host.online_mode(true));
        assert_eq!(
            host.permission_level("Steve", Some(PermissionLvl::Three), PermissionLvl::Zero),
            PermissionLvl::Three
        );
    }

    #[test]
    fn an_embedded_server_pauses_with_its_host_or_when_empty() {
        let host = embedded();
        assert!(!host.pauses(true));
        assert!(host.pauses(false));
        host.set_paused(true);
        assert!(host.pauses(true));
    }

    #[test]
    fn an_embedded_server_takes_the_hosts_online_mode() {
        let host = embedded();
        assert!(host.online_mode(false));
        host.set_online_mode(false);
        assert!(!host.online_mode(true));
    }

    #[test]
    fn the_steps_of_the_pause() {
        assert_eq!(PauseStep::between(false, true), PauseStep::Pausing);
        assert_eq!(PauseStep::between(true, false), PauseStep::Resuming);
        assert!(PauseStep::between(true, true).is_paused());
        assert!(!PauseStep::between(true, false).is_paused());
    }

    #[test]
    fn the_owner_is_an_operator_while_commands_are_allowed() {
        let host = embedded();
        let level = |name| host.permission_level(name, None, PermissionLvl::Zero);
        assert_eq!(level("steve"), PermissionLvl::Four);
        assert_eq!(level("Alex"), PermissionLvl::Zero);
        host.set_guest_command_access(true);
        assert_eq!(level("Alex"), PermissionLvl::Two);
        host.set_allow_commands(false);
        assert_eq!(level("Steve"), PermissionLvl::Zero);
        assert_eq!(level("Alex"), PermissionLvl::Zero);
    }

    #[test]
    fn the_game_mode_is_forced_only_while_published_outside_hardcore() {
        let host = embedded();
        assert!(!host.forces_game_mode());
        host.set_published(true);
        assert!(host.forces_game_mode());
        host.set_hardcore(true);
        assert!(!host.forces_game_mode());
    }
}
