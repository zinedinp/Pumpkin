use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use pumpkin_util::text::color::NamedColor;

use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::core::string::StringArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};

const DESCRIPTION: &str = "Manage loaded worlds.";

const PERMISSION: &str = "pumpkin:command.world";

struct ListExecutor;

impl CommandExecutor for ListExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let worlds = context.server().worlds.load();
        let names: Vec<String> = worlds
            .iter()
            .map(|world| {
                format!(
                    "{} ({})",
                    world.get_world_name(),
                    world.dimension.minecraft_name
                )
            })
            .collect();

        context.source.send_feedback(
            TextComponent::text(format!(
                "There are {} world(s) loaded: {}",
                names.len(),
                names.join(", ")
            )),
            false,
        );
        Ok(1)
    }
}

struct UnloadExecutor;

impl CommandExecutor for UnloadExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let name = StringArgumentType::get(context, "world")?.to_string();

        let server = context.server().clone();
        let unloading = server.clone();
        let source = context.source.clone();

        // `unload_world` is async and already refuses the primary world and worlds with players
        // still in them, so its error is the one worth reporting.
        server.spawn_task(async move {
            match unloading.unload_world(&name).await {
                Ok(()) => source.send_feedback(
                    TextComponent::text(format!("Unloaded world '{name}'")),
                    true,
                ),
                Err(err) => source.send_error(TextComponent::text(err).color_named(NamedColor::Red)),
            }
        });

        Ok(1)
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Four),
    ));

    dispatcher.register(
        command("world", DESCRIPTION)
            .requires(PERMISSION)
            .then(literal("list").executes(ListExecutor))
            .then(
                literal("unload").then(
                    // Greedy, not `SingleWord`: dimension names are colon-qualified
                    // (`minecraft:the_end`) and level names may contain spaces.
                    argument("world", StringArgumentType::GreedyPhrase).executes(UnloadExecutor),
                ),
            ),
    );
}
