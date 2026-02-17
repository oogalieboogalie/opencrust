use poise::serenity_prelude as serenity;

/// Shared data accessible in all poise commands.
#[derive(Debug, Clone)]
pub struct CommandData {
    /// The channel identifier used in OpenCrust.
    pub channel_id: String,
}

/// Type alias for the poise error type.
pub type CommandError = Box<dyn std::error::Error + Send + Sync>;

/// Type alias for the poise context.
pub type CommandContext<'a> = poise::Context<'a, CommandData, CommandError>;

/// `/ping` — Simple health check slash command.
#[poise::command(slash_command)]
pub async fn ping(ctx: CommandContext<'_>) -> Result<(), CommandError> {
    ctx.say("🏓 Pong! OpenCrust Discord channel is active.")
        .await?;
    Ok(())
}

/// `/status` — Report the bot's current status.
#[poise::command(slash_command)]
pub async fn status(ctx: CommandContext<'_>) -> Result<(), CommandError> {
    let uptime_info = format!(
        "**OpenCrust Discord Bot**\n\
         Channel ID: `{}`\n\
         Status: ✅ Connected\n\
         Latency: Checking...",
        ctx.data().channel_id
    );
    ctx.say(uptime_info).await?;
    Ok(())
}

/// Build the poise framework with all registered commands.
///
/// Returns the list of commands that can be used with poise.
pub fn all_commands() -> Vec<poise::Command<CommandData, CommandError>> {
    vec![ping(), status()]
}

/// Register slash commands with Discord for the given guild IDs.
///
/// If `guild_ids` is empty, commands are registered globally (takes up to 1 hour).
/// If `guild_ids` is provided, commands are registered per-guild (instant).
pub async fn register_commands(
    ctx: &serenity::Context,
    guild_ids: &[u64],
    commands: &[poise::Command<CommandData, CommandError>],
) -> Result<(), CommandError> {
    if guild_ids.is_empty() {
        tracing::info!("registering {} slash commands globally", commands.len());
        poise::builtins::register_globally(ctx, commands).await?;
    } else {
        for &guild_id in guild_ids {
            let guild = serenity::GuildId::new(guild_id);
            tracing::info!(
                "registering {} slash commands for guild {}",
                commands.len(),
                guild_id
            );
            poise::builtins::register_in_guild(ctx, commands, guild).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_commands_includes_expected_commands() {
        let commands = all_commands();
        let command_names: Vec<_> = commands.iter().map(|c| c.name.as_str()).collect();
        assert!(command_names.contains(&"ping"));
        assert!(command_names.contains(&"status"));
    }

    #[test]
    fn command_data_is_clone() {
        let data = CommandData {
            channel_id: "discord".to_string(),
        };
        let cloned = data.clone();
        assert_eq!(cloned.channel_id, "discord");
    }
}
