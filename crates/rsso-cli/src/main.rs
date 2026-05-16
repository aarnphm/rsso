use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rsso_riot::routing::account_by_riot_id_url;

#[derive(Debug, Parser)]
#[command(name = "rsso-cli")]
#[command(about = "Local tools for rsso")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Riot {
        #[command(subcommand)]
        command: RiotCommand,
    },
    Discord {
        #[command(subcommand)]
        command: DiscordCommand,
    },
}

#[derive(Debug, Subcommand)]
enum RiotCommand {
    ProbeAccount {
        riot_id: String,
        #[arg(long, env = "RIOT_API_KEY")]
        api_key: String,
        #[arg(long, default_value = "AMERICAS")]
        regional: String,
    },
}

#[derive(Debug, Subcommand)]
enum DiscordCommand {
    CommandsJson,
    RegisterCommands {
        #[arg(long, env = "DISCORD_APP_ID")]
        app_id: String,
        #[arg(long, env = "DISCORD_GUILD_ID")]
        guild_id: String,
        #[arg(long, env = "DISCORD_BOT_TOKEN")]
        bot_token: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Riot { command } => handle_riot(command).await,
        Command::Discord { command } => handle_discord(command).await,
    }
}

async fn handle_riot(command: RiotCommand) -> Result<()> {
    match command {
        RiotCommand::ProbeAccount {
            riot_id,
            api_key,
            regional,
        } => {
            let parsed = rsso_riot::parse_riot_id(&riot_id)?;
            let url = account_by_riot_id_url(&regional, &parsed.game_name, &parsed.tag_line)?;
            let client = reqwest::Client::new();
            let response = client
                .get(url)
                .header("X-Riot-Token", api_key)
                .send()
                .await
                .context("riot account request failed")?;
            let status = response.status();
            let body = response.text().await.context("riot response body failed")?;
            println!("status={status}");
            println!("{body}");
            Ok(())
        }
    }
}

async fn handle_discord(command: DiscordCommand) -> Result<()> {
    match command {
        DiscordCommand::CommandsJson => {
            println!("{}", serde_json::to_string_pretty(&command_manifest())?);
            Ok(())
        }
        DiscordCommand::RegisterCommands {
            app_id,
            guild_id,
            bot_token,
        } => {
            let url = format!(
                "https://discord.com/api/v10/applications/{app_id}/guilds/{guild_id}/commands"
            );
            let response = reqwest::Client::new()
                .put(url)
                .bearer_auth(bot_token)
                .json(&command_manifest())
                .send()
                .await
                .context("discord command registration request failed")?;
            let status = response.status();
            let body = response
                .text()
                .await
                .context("discord response body failed")?;
            println!("status={status}");
            println!("{body}");
            Ok(())
        }
    }
}

fn command_manifest() -> serde_json::Value {
    serde_json::json!([
        {
            "name": "register-summoners",
            "description": "Register your Riot ID as a trusted in-house claim",
            "options": [
                {"name": "riot_id", "description": "Riot ID, for example GameName#TAG", "type": 3, "required": true}
            ]
        },
        {
            "name": "game",
            "description": "Create a new in-house game",
            "options": [
                {
                    "name": "mode",
                    "description": "Game mode",
                    "type": 3,
                    "required": true,
                    "choices": [
                        {"name": "Rift", "value": "rift"},
                        {"name": "ARAM", "value": "aram"},
                        {"name": "ARAM: Mayhem", "value": "aram_mayhem"},
                        {"name": "Other", "value": "other"}
                    ]
                },
                {"name": "user_1", "description": "Player 1", "type": 6, "required": true},
                {"name": "user_2", "description": "Player 2", "type": 6, "required": true},
                {"name": "user_3", "description": "Player 3", "type": 6, "required": false},
                {"name": "user_4", "description": "Player 4", "type": 6, "required": false},
                {"name": "user_5", "description": "Player 5", "type": 6, "required": false},
                {"name": "user_6", "description": "Player 6", "type": 6, "required": false},
                {"name": "user_7", "description": "Player 7", "type": 6, "required": false},
                {"name": "user_8", "description": "Player 8", "type": 6, "required": false},
                {"name": "user_9", "description": "Player 9", "type": 6, "required": false},
                {"name": "user_10", "description": "Player 10", "type": 6, "required": false}
            ]
        },
        {
            "name": "add",
            "description": "Add a player to a game",
            "options": [
                {"name": "game_id", "description": "Local game id", "type": 3, "required": true},
                {"name": "user", "description": "Discord user", "type": 6, "required": true}
            ]
        },
        {
            "name": "randomize",
            "description": "Randomize teams for a game",
            "options": [
                {"name": "game_id", "description": "Local game id", "type": 3, "required": true}
            ]
        },
        {
            "name": "result",
            "description": "Report the winning side",
            "options": [
                {"name": "game_id", "description": "Local game id", "type": 3, "required": true},
                {
                    "name": "winner",
                    "description": "Winning side",
                    "type": 3,
                    "required": true,
                    "choices": [
                        {"name": "Blue", "value": "blue"},
                        {"name": "Red", "value": "red"}
                    ]
                }
            ]
        },
        {
            "name": "finish",
            "description": "Finalize a game from a Riot match id",
            "options": [
                {"name": "riot_match_id", "description": "Riot match id, for example NA1_4901234567", "type": 3, "required": true},
                {"name": "game_id", "description": "Local game id", "type": 3, "required": false},
                {
                    "name": "winner",
                    "description": "Winning side",
                    "type": 3,
                    "required": false,
                    "choices": [
                        {"name": "Blue", "value": "blue"},
                        {"name": "Red", "value": "red"}
                    ]
                }
            ]
        },
        {
            "name": "end",
            "description": "Finalize a reported game",
            "options": [
                {"name": "game_id", "description": "Local game id", "type": 3, "required": true}
            ]
        },
        {
            "name": "stats",
            "description": "Show in-house stats",
            "options": [
                {"name": "user", "description": "Discord user", "type": 6, "required": false},
                {"name": "mode", "description": "Mode filter", "type": 3, "required": false, "choices": mode_choices()}
            ]
        },
        {
            "name": "leaderboards",
            "description": "Show in-house leaderboard",
            "options": [
                {"name": "mode", "description": "Mode filter", "type": 3, "required": false, "choices": mode_choices()}
            ]
        },
        {
            "name": "analysis",
            "description": "Show analysis stub",
            "options": [
                {"name": "mode", "description": "Mode filter", "type": 3, "required": false, "choices": mode_choices()}
            ]
        }
    ])
}

fn mode_choices() -> serde_json::Value {
    serde_json::json!([
        {"name": "Rift", "value": "rift"},
        {"name": "ARAM", "value": "aram"},
        {"name": "ARAM: Mayhem", "value": "aram_mayhem"},
        {"name": "Other", "value": "other"}
    ])
}
