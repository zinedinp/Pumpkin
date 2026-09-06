use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use pumpkin_util::text::color::NamedColor;

use crate::command::argument_builder::{ArgumentBuilder, command};
use crate::command::context::command_context::CommandContext;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::restart_server;

const DESCRIPTION: &str = "Save and stop the server, then start it again.";

const PERMISSION: &str = "pumpkin:command.restart";

struct RestartCommandExecutor;

impl CommandExecutor for RestartCommandExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        context.source.send_feedback(
            TextComponent::text("Restarting the server").color_named(NamedColor::Red),
            true,
        );
        restart_server();
        Ok(1)
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    // Same level as `stop`: this ends everyone's session just as abruptly.
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Four),
    ));

    dispatcher.register(
        command("restart", DESCRIPTION)
            .requires(PERMISSION)
            .executes(RestartCommandExecutor),
    );
}
