use async_trait::async_trait;
use rsso_discord::{DiscordCommand, FinishCommand, GameCommand, HydrateCommand, ResultsCommand};
use rsso_domain::{
    split_even_teams, DiscordUserId, GameId, GameModeKind, GameStatus, QueueId, Rng,
    TeamAssignment, TeamSide,
};
use rsso_riot::parse_riot_id;
use rsso_storage::{
    GameRow, MatchRecord, NewGame, NewPlayer, ParticipantRecord, RosterPlayer, Storage,
    StorageError,
};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct CommandContext {
    pub guild_id: String,
    pub channel_id: String,
    pub actor_id: String,
    pub now: i64,
}

#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("{0}")]
    UserFacing(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRiotAccount {
    pub puuid: String,
    pub game_name: String,
    pub tag_line: String,
}

#[async_trait(?Send)]
pub trait RiotAccountResolver {
    async fn resolve_riot_id(
        &self,
        riot_id: &str,
    ) -> Result<Option<ResolvedRiotAccount>, HandlerError>;
}

#[derive(Debug, Clone)]
pub struct ResolvedRiotMatch {
    pub riot_match_id: String,
    pub queue_id: Option<u16>,
    pub map_id: Option<u16>,
    pub game_mode: Option<String>,
    pub game_type: Option<String>,
    pub payload_json: Option<String>,
    pub participants: Vec<ResolvedRiotParticipant>,
}

#[derive(Debug, Clone)]
pub struct ResolvedRiotParticipant {
    pub puuid: String,
    pub team_id: Option<u16>,
    pub champion_id: Option<u16>,
    pub champion_name: Option<String>,
    pub win: Option<bool>,
    pub kills: Option<u16>,
    pub deaths: Option<u16>,
    pub assists: Option<u16>,
    pub total_damage: Option<u32>,
    pub gold_earned: Option<u32>,
    pub total_minions: Option<u32>,
    pub vision_score: Option<u32>,
    pub raw_json: Option<String>,
}

#[async_trait(?Send)]
pub trait RiotMatchResolver {
    async fn resolve_match(
        &self,
        riot_match_id: &str,
    ) -> Result<Option<ResolvedRiotMatch>, HandlerError>;
}

#[derive(Debug, Clone, Copy)]
pub struct NoopRiotAccountResolver;

#[async_trait(?Send)]
impl RiotAccountResolver for NoopRiotAccountResolver {
    async fn resolve_riot_id(
        &self,
        _riot_id: &str,
    ) -> Result<Option<ResolvedRiotAccount>, HandlerError> {
        Ok(None)
    }
}

#[async_trait(?Send)]
impl RiotMatchResolver for NoopRiotAccountResolver {
    async fn resolve_match(
        &self,
        _riot_match_id: &str,
    ) -> Result<Option<ResolvedRiotMatch>, HandlerError> {
        Ok(None)
    }
}

pub async fn handle_command<S: Storage + ?Sized>(
    storage: &S,
    context: CommandContext,
    command: DiscordCommand,
    rng: &mut impl Rng,
) -> Result<String, HandlerError> {
    handle_command_with_resolver(storage, &NoopRiotAccountResolver, context, command, rng).await
}

pub async fn handle_command_with_resolver<
    S: Storage + ?Sized,
    R: RiotAccountResolver + RiotMatchResolver + ?Sized,
>(
    storage: &S,
    resolver: &R,
    context: CommandContext,
    command: DiscordCommand,
    rng: &mut impl Rng,
) -> Result<String, HandlerError> {
    match command {
        DiscordCommand::RegisterSummoners { riot_id } => {
            handle_register(storage, resolver, context, &riot_id).await
        }
        DiscordCommand::Game(command) => handle_game(storage, context, command, rng).await,
        DiscordCommand::Add { game_id, user } => handle_add(storage, context, game_id, user).await,
        DiscordCommand::Randomize { game_id } => {
            handle_randomize(storage, context, game_id, rng).await
        }
        DiscordCommand::Result { game_id, winner } => {
            handle_result(storage, context, game_id, winner).await
        }
        DiscordCommand::Results(command) => {
            handle_results(storage, resolver, context, command).await
        }
        DiscordCommand::Finish(command) => handle_finish(storage, resolver, context, command).await,
        DiscordCommand::Hydrate(command) => {
            handle_hydrate(storage, resolver, context, command).await
        }
        DiscordCommand::End { game_id } => handle_end(storage, context, game_id).await,
        DiscordCommand::Status { game_id } => handle_status(storage, context, game_id).await,
        DiscordCommand::Stats { user, mode } => {
            let user = user.unwrap_or_else(|| DiscordUserId::new(context.actor_id.clone()));
            let stats = storage
                .stats_for_player(&context.guild_id, user.as_str(), mode)
                .await?;
            Ok(match stats {
                Some(stats) => format_stats_line(
                    &format!("{}#{}", stats.riot_game_name, stats.riot_tag_line),
                    stats.rating,
                    stats.wins,
                    stats.losses,
                    stats.win_rate,
                    StatsAverages {
                        kills: stats.avg_kills,
                        deaths: stats.avg_deaths,
                        assists: stats.avg_assists,
                        damage: stats.avg_total_damage,
                    },
                ),
                None => "No stats found for that player yet.".to_owned(),
            })
        }
        DiscordCommand::Leaderboards { mode } => {
            let rows = storage.leaderboard(&context.guild_id, mode, 10).await?;
            if rows.is_empty() {
                return Ok("No leaderboard rows yet.".to_owned());
            }
            let lines = rows
                .iter()
                .enumerate()
                .map(|(idx, row)| {
                    let summary = format_stats_line(
                        &format!("{}#{}", row.riot_game_name, row.riot_tag_line),
                        row.rating,
                        row.wins,
                        row.losses,
                        row.win_rate,
                        StatsAverages {
                            kills: row.avg_kills,
                            deaths: row.avg_deaths,
                            assists: row.avg_assists,
                            damage: row.avg_total_damage,
                        },
                    );
                    format!("{}. {}", idx + 1, summary)
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(lines)
        }
        DiscordCommand::Analysis { mode } => {
            let suffix = mode.map_or(String::new(), |mode| format!(" for {}", mode.as_str()));
            Ok(format!(
                "Analysis{suffix} is wired as a v1 stub. Need more rows first."
            ))
        }
    }
}

async fn handle_register<S: Storage + ?Sized>(
    storage: &S,
    resolver: &(impl RiotAccountResolver + ?Sized),
    context: CommandContext,
    riot_id: &str,
) -> Result<String, HandlerError> {
    let parsed = parse_riot_id(riot_id)
        .map_err(|err| HandlerError::UserFacing(format!("Invalid Riot ID: {err}")))?;
    let resolved = resolver.resolve_riot_id(riot_id).await?;
    let riot_puuid = resolved.as_ref().map(|account| account.puuid.clone());
    let riot_game_name = resolved.as_ref().map_or_else(
        || parsed.game_name.clone(),
        |account| account.game_name.clone(),
    );
    let riot_tag_line = resolved.as_ref().map_or_else(
        || parsed.tag_line.clone(),
        |account| account.tag_line.clone(),
    );
    storage
        .upsert_player(NewPlayer {
            guild_id: context.guild_id,
            discord_user_id: context.actor_id,
            riot_puuid,
            riot_game_name: riot_game_name.clone(),
            riot_tag_line: riot_tag_line.clone(),
            now: context.now,
        })
        .await?;
    let suffix = if resolved.is_some() {
        " with Riot API PUUID."
    } else {
        " as a trusted claim."
    };
    Ok(format!(
        "Registered {riot_game_name}#{riot_tag_line}{suffix}"
    ))
}

async fn handle_game<S: Storage + ?Sized>(
    storage: &S,
    context: CommandContext,
    command: GameCommand,
    rng: &mut impl Rng,
) -> Result<String, HandlerError> {
    if command
        .users
        .iter()
        .any(|user| user.as_str() == context.actor_id)
    {
        // Fine, the creator can play. This branch exists to make the intent obvious.
    }
    let CommandContext {
        guild_id,
        channel_id,
        actor_id,
        now,
    } = context;
    let game_id = GameId::new(format!("g_{}", nanoid::nanoid!(10)));
    let user_ids = command
        .users
        .iter()
        .map(|user| user.as_str().to_owned())
        .collect::<Vec<_>>();
    storage
        .create_game(
            NewGame {
                game_id: game_id.as_str().to_owned(),
                guild_id,
                channel_id,
                creator_discord_id: actor_id,
                mode: command.mode,
                now,
            },
            &user_ids,
        )
        .await?;
    if user_ids.len() % 2 == 0 {
        let assignments = split_even_teams(&command.users, rng)
            .map_err(|err| HandlerError::UserFacing(format!("Cannot randomize: {err}")))?;
        storage.assign_teams(&game_id, &assignments, now).await?;
        return Ok(format!(
            "Created {} in {} with {} players.\n{}",
            game_id,
            command.mode.as_str(),
            user_ids.len(),
            format_assignments(&assignments)
        ));
    }
    Ok(format!(
        "Created {} in {} with {} players. Add one more player before randomizing. Riot match id is pending until Spectator sees the live game.",
        game_id,
        command.mode.as_str(),
        user_ids.len()
    ))
}

async fn handle_add<S: Storage + ?Sized>(
    storage: &S,
    context: CommandContext,
    game_id: GameId,
    user: DiscordUserId,
) -> Result<String, HandlerError> {
    let game = storage.game_by_id(&game_id).await?;
    if game.guild_id != context.guild_id {
        return Err(HandlerError::UserFacing(
            "That game belongs to a different guild.".to_owned(),
        ));
    }
    let status = game
        .status()
        .map_err(|err| HandlerError::UserFacing(format!("Invalid stored status: {err}")))?;
    if !matches!(status, GameStatus::Lobby | GameStatus::Randomized) {
        return Err(HandlerError::UserFacing(
            "Cannot add players after the lobby is locked.".to_owned(),
        ));
    }
    storage
        .add_player(&game_id, &context.guild_id, user.as_str(), context.now)
        .await?;
    Ok(format!("Added <@{}> to {}.", user.as_str(), game_id))
}

async fn handle_randomize<S: Storage + ?Sized>(
    storage: &S,
    context: CommandContext,
    game_id: GameId,
    rng: &mut impl Rng,
) -> Result<String, HandlerError> {
    let game = storage.game_by_id(&game_id).await?;
    if game.guild_id != context.guild_id {
        return Err(HandlerError::UserFacing(
            "That game belongs to a different guild.".to_owned(),
        ));
    }
    let status = game
        .status()
        .map_err(|err| HandlerError::UserFacing(format!("Invalid stored status: {err}")))?;
    if !matches!(status, GameStatus::Lobby | GameStatus::Randomized) {
        return Err(HandlerError::UserFacing(
            "Cannot randomize after the lobby is locked.".to_owned(),
        ));
    }
    let roster = storage.roster(&game_id).await?;
    let users = roster
        .iter()
        .map(|player| DiscordUserId::new(player.discord_user_id.clone()))
        .collect::<Vec<_>>();
    let assignments = split_even_teams(&users, rng)
        .map_err(|err| HandlerError::UserFacing(format!("Cannot randomize: {err}")))?;
    storage
        .assign_teams(&game_id, &assignments, context.now)
        .await?;
    Ok(format!(
        "{} randomized.\n{}",
        game_id,
        format_assignments(&assignments)
    ))
}

fn format_assignments(assignments: &[TeamAssignment]) -> String {
    let blue = assignments
        .iter()
        .filter(|assignment| assignment.team == TeamSide::Blue)
        .map(|assignment| format!("<@{}>", assignment.discord_user_id.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    let red = assignments
        .iter()
        .filter(|assignment| assignment.team == TeamSide::Red)
        .map(|assignment| format!("<@{}>", assignment.discord_user_id.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    format!("Blue: {blue}\nRed: {red}")
}

async fn handle_result<S: Storage + ?Sized>(
    storage: &S,
    context: CommandContext,
    game_id: GameId,
    winner: TeamSide,
) -> Result<String, HandlerError> {
    storage
        .record_vote(&game_id, &context.actor_id, winner, context.now)
        .await?;
    storage.mark_reported(&game_id, winner, context.now).await?;
    Ok(format!(
        "{} reported as {} win. Use `/end {}` to finalize.",
        game_id,
        winner.as_str(),
        game_id
    ))
}

async fn handle_results<S: Storage + ?Sized>(
    storage: &S,
    resolver: &(impl RiotMatchResolver + ?Sized),
    context: CommandContext,
    command: ResultsCommand,
) -> Result<String, HandlerError> {
    if command.winner.is_none() && command.riot_match_id.is_none() {
        return Err(HandlerError::UserFacing(
            "`/results` needs `winner`, `riot_match_id`, or both.".to_owned(),
        ));
    }
    let (game_id, game) = load_game_for_guild(
        storage,
        &context.guild_id,
        command.game_id.clone(),
        "No open game to report.",
    )
    .await?;
    let status = game
        .status()
        .map_err(|err| HandlerError::UserFacing(format!("Invalid stored status: {err}")))?;
    if status == GameStatus::Lobby {
        return Err(HandlerError::UserFacing(
            "Randomize teams before reporting results.".to_owned(),
        ));
    }
    if matches!(status, GameStatus::Finalized | GameStatus::Cancelled) {
        return Err(HandlerError::UserFacing(
            "That game is already closed.".to_owned(),
        ));
    }

    let mode = game
        .mode()
        .map_err(|err| HandlerError::UserFacing(format!("Invalid stored mode: {err}")))?;
    let roster = storage.roster(&game_id).await?;
    let mut linked_match_id = None;
    let mut derived_winner = None;

    if let Some(riot_match_id) = command.riot_match_id.as_deref() {
        let resolved_match = resolver.resolve_match(riot_match_id).await?;
        if let Some(match_detail) = resolved_match {
            validate_match_mode(mode, &match_detail)?;
            derived_winner = derive_winner(&match_detail);
            let match_record = build_match_record(&game, &roster, match_detail)?;
            linked_match_id = Some(match_record.riot_match_id.clone());
            storage
                .record_match(&game_id, match_record, context.now)
                .await?;
        } else {
            linked_match_id = Some(riot_match_id.to_owned());
            storage
                .record_match(
                    &game_id,
                    manual_match_record(&context.guild_id, mode, riot_match_id),
                    context.now,
                )
                .await?;
        }
    }

    let Some(winner) = command.winner.or(derived_winner) else {
        let match_id = linked_match_id.ok_or_else(|| {
            HandlerError::UserFacing("`/results` needs a winner to report.".to_owned())
        })?;
        return Ok(format!(
            "{} linked to {}. Riot did not return a winner yet, so the game is not reported. Run `/results game_id:{} winner:Blue` or `/results game_id:{} winner:Red` once the winner is known.",
            game_id, match_id, game_id, game_id
        ));
    };

    storage
        .record_vote(&game_id, &context.actor_id, winner, context.now)
        .await?;

    if let Some(match_id) = linked_match_id {
        storage
            .finalize_game(&game_id, winner, Some(&match_id), context.now)
            .await?;
        return Ok(format!(
            "{} finalized from {} as {} win.",
            game_id,
            match_id,
            winner.as_str()
        ));
    }

    storage.mark_reported(&game_id, winner, context.now).await?;
    Ok(format!(
        "{} reported as {} win. Use `/end {}` to finalize.",
        game_id,
        winner.as_str(),
        game_id
    ))
}

async fn handle_finish<S: Storage + ?Sized>(
    storage: &S,
    resolver: &(impl RiotMatchResolver + ?Sized),
    context: CommandContext,
    command: FinishCommand,
) -> Result<String, HandlerError> {
    let (game_id, game) = load_game_for_guild(
        storage,
        &context.guild_id,
        command.game_id.clone(),
        "No open game to finish.",
    )
    .await?;
    let mode = game
        .mode()
        .map_err(|err| HandlerError::UserFacing(format!("Invalid stored mode: {err}")))?;
    let roster = storage.roster(&game_id).await?;
    let resolved_match = resolver.resolve_match(&command.riot_match_id).await?;
    let stored_winner: Option<TeamSide> = game
        .winning_side
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(|err| HandlerError::UserFacing(format!("{err}")))?;
    let winner = match (command.winner, resolved_match.as_ref(), stored_winner) {
        (Some(winner), _, _) => winner,
        (None, Some(match_detail), _) => derive_winner(match_detail).ok_or_else(|| {
            HandlerError::UserFacing(
                "Could not derive winner from Riot match data; pass `winner` explicitly."
                    .to_owned(),
            )
        })?,
        (None, None, Some(winner)) => winner,
        (None, None, None) => {
            storage
                .record_match(
                    &game_id,
                    manual_match_record(&context.guild_id, mode, &command.riot_match_id),
                    context.now,
                )
                .await?;
            return Err(HandlerError::UserFacing(
                format!(
                    "{} linked to {}. Riot did not return match data, so `/finish` needs `winner:Blue` or `winner:Red`.",
                    game_id, command.riot_match_id
                ),
            ));
        }
    };
    if let Some(match_detail) = resolved_match {
        validate_match_mode(mode, &match_detail)?;
        let match_record = build_match_record(&game, &roster, match_detail)?;
        storage
            .record_match(&game_id, match_record, context.now)
            .await?;
    } else {
        storage
            .record_match(
                &game_id,
                manual_match_record(&context.guild_id, mode, &command.riot_match_id),
                context.now,
            )
            .await?;
    }
    storage
        .finalize_game(&game_id, winner, Some(&command.riot_match_id), context.now)
        .await?;
    Ok(format!(
        "{} finalized from {} as {} win.",
        game_id,
        command.riot_match_id,
        winner.as_str()
    ))
}

async fn handle_hydrate<S: Storage + ?Sized>(
    storage: &S,
    resolver: &(impl RiotMatchResolver + ?Sized),
    context: CommandContext,
    command: HydrateCommand,
) -> Result<String, HandlerError> {
    let (game_id, game, riot_match_id) = load_hydrate_target(storage, &context, command).await?;
    let status = game
        .status()
        .map_err(|err| HandlerError::UserFacing(format!("Invalid stored status: {err}")))?;
    if status == GameStatus::Lobby {
        return Err(HandlerError::UserFacing(
            "Randomize teams before hydrating match stats.".to_owned(),
        ));
    }
    if status == GameStatus::Cancelled {
        return Err(HandlerError::UserFacing(
            "Cannot hydrate a cancelled game.".to_owned(),
        ));
    }

    let Some(match_detail) = resolver.resolve_match(&riot_match_id).await? else {
        return Ok(format!(
            "{} is linked to {}, but Riot still does not return Match-V5 data for it.",
            game_id, riot_match_id
        ));
    };

    let mode = game
        .mode()
        .map_err(|err| HandlerError::UserFacing(format!("Invalid stored mode: {err}")))?;
    validate_match_mode(mode, &match_detail)?;
    let stored_winner: Option<TeamSide> = game
        .winning_side
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(|err| HandlerError::UserFacing(format!("{err}")))?;
    let derived_winner = derive_winner(&match_detail);
    if let (Some(stored), Some(derived)) = (stored_winner, derived_winner) {
        if stored != derived {
            return Err(HandlerError::UserFacing(format!(
                "Riot winner {} conflicts with stored winner {} for {}.",
                derived.as_str(),
                stored.as_str(),
                game_id
            )));
        }
    }

    let roster = storage.roster(&game_id).await?;
    let match_record = build_match_record(&game, &roster, match_detail)?;
    let participant_count = match_record.participants.len();
    storage
        .record_match(&game_id, match_record, context.now)
        .await?;
    Ok(format!(
        "{} hydrated from {} with {} participant row(s).",
        game_id, riot_match_id, participant_count
    ))
}

async fn handle_end<S: Storage + ?Sized>(
    storage: &S,
    context: CommandContext,
    game_id: GameId,
) -> Result<String, HandlerError> {
    let game = storage.game_by_id(&game_id).await?;
    if game.guild_id != context.guild_id {
        return Err(HandlerError::UserFacing(
            "That game belongs to a different guild.".to_owned(),
        ));
    }
    if game.creator_discord_id != context.actor_id {
        return Err(HandlerError::UserFacing(
            "Only the game creator can end this game for now.".to_owned(),
        ));
    }
    let winner = game
        .winning_side
        .as_deref()
        .ok_or_else(|| HandlerError::UserFacing("Report a winner before ending.".to_owned()))?
        .parse()
        .map_err(|err| HandlerError::UserFacing(format!("{err}")))?;
    storage
        .finalize_game(&game_id, winner, game.riot_match_id.as_deref(), context.now)
        .await?;
    Ok(format!("{} finalized as {} win.", game_id, winner.as_str()))
}

async fn load_hydrate_target<S: Storage + ?Sized>(
    storage: &S,
    context: &CommandContext,
    command: HydrateCommand,
) -> Result<(GameId, GameRow, String), HandlerError> {
    match (command.game_id, command.riot_match_id) {
        (Some(game_id), Some(riot_match_id)) => {
            let game = storage.game_by_id(&game_id).await?;
            ensure_game_in_guild(&game, &context.guild_id)?;
            Ok((game_id, game, riot_match_id))
        }
        (Some(game_id), None) => {
            let game = storage.game_by_id(&game_id).await?;
            ensure_game_in_guild(&game, &context.guild_id)?;
            let riot_match_id = game.riot_match_id.clone().ok_or_else(|| {
                HandlerError::UserFacing(format!("{game_id} has no Riot match id yet."))
            })?;
            Ok((game_id, game, riot_match_id))
        }
        (None, Some(riot_match_id)) => {
            let game = storage
                .game_by_riot_match_id(&context.guild_id, &riot_match_id)
                .await?
                .ok_or_else(|| {
                    HandlerError::UserFacing(format!(
                        "No local game is linked to {riot_match_id}; pass `game_id` too."
                    ))
                })?;
            let game_id = GameId::new(game.game_id.clone());
            Ok((game_id, game, riot_match_id))
        }
        (None, None) => {
            let game = storage
                .latest_game_with_match_for_guild(&context.guild_id)
                .await?
                .ok_or_else(|| {
                    HandlerError::UserFacing(
                        "No linked game found to hydrate; pass `game_id` or `riot_match_id`."
                            .to_owned(),
                    )
                })?;
            let game_id = GameId::new(game.game_id.clone());
            let riot_match_id = game.riot_match_id.clone().ok_or_else(|| {
                HandlerError::UserFacing(format!("{game_id} has no Riot match id yet."))
            })?;
            Ok((game_id, game, riot_match_id))
        }
    }
}

async fn handle_status<S: Storage + ?Sized>(
    storage: &S,
    context: CommandContext,
    game_id: Option<GameId>,
) -> Result<String, HandlerError> {
    let (game_id, game) =
        load_game_for_guild(storage, &context.guild_id, game_id, "No open game found.").await?;
    let riot_link = game.riot_match_id.as_deref().map_or_else(
        || "Riot match id: pending until Spectator sees the live game".to_owned(),
        |match_id| format!("Riot match id: {match_id}"),
    );
    Ok(format!(
        "{}: mode {}, status {}, {}",
        game_id, game.mode, game.status, riot_link
    ))
}

async fn load_game_for_guild<S: Storage + ?Sized>(
    storage: &S,
    guild_id: &str,
    game_id: Option<GameId>,
    no_open_message: &'static str,
) -> Result<(GameId, GameRow), HandlerError> {
    let (game_id, game) = match game_id {
        Some(game_id) => {
            let game = storage.game_by_id(&game_id).await?;
            (game_id, game)
        }
        None => {
            let game = storage
                .open_game_for_guild(guild_id)
                .await?
                .ok_or_else(|| HandlerError::UserFacing(no_open_message.to_owned()))?;
            (GameId::new(game.game_id.clone()), game)
        }
    };
    if game.guild_id != guild_id {
        return Err(wrong_guild_error());
    }
    Ok((game_id, game))
}

fn ensure_game_in_guild(game: &GameRow, guild_id: &str) -> Result<(), HandlerError> {
    if game.guild_id == guild_id {
        Ok(())
    } else {
        Err(wrong_guild_error())
    }
}

fn wrong_guild_error() -> HandlerError {
    HandlerError::UserFacing("That game belongs to a different guild.".to_owned())
}

fn derive_winner(match_detail: &ResolvedRiotMatch) -> Option<TeamSide> {
    match_detail.participants.iter().find_map(|participant| {
        let won = participant.win?;
        if !won {
            return None;
        }
        side_from_team_id(participant.team_id)
    })
}

fn validate_match_mode(
    expected: GameModeKind,
    match_detail: &ResolvedRiotMatch,
) -> Result<(), HandlerError> {
    let queue_id = match_detail.queue_id.map(QueueId);
    if expected.accepts_queue(queue_id) {
        Ok(())
    } else {
        Err(HandlerError::UserFacing(format!(
            "Riot match queue {:?} does not match expected mode {}.",
            match_detail.queue_id,
            expected.as_str()
        )))
    }
}

fn build_match_record(
    game: &GameRow,
    roster: &[RosterPlayer],
    match_detail: ResolvedRiotMatch,
) -> Result<MatchRecord, HandlerError> {
    let mode = game
        .mode()
        .map_err(|err| HandlerError::UserFacing(format!("Invalid stored mode: {err}")))?;
    let roster_by_puuid = roster
        .iter()
        .filter_map(|player| {
            player
                .riot_puuid
                .as_ref()
                .map(|puuid| (puuid.as_str(), player))
        })
        .collect::<HashMap<_, _>>();
    let match_puuids = match_detail
        .participants
        .iter()
        .map(|participant| participant.puuid.as_str())
        .collect::<HashSet<_>>();
    let missing_roster_puuids = roster_by_puuid
        .keys()
        .filter(|puuid| !match_puuids.contains(**puuid))
        .count();
    if missing_roster_puuids > 0 {
        return Err(HandlerError::UserFacing(format!(
            "Riot match is missing {missing_roster_puuids} registered roster player(s)."
        )));
    }
    let participants = match_detail
        .participants
        .into_iter()
        .map(|participant| {
            let roster_player = roster_by_puuid.get(participant.puuid.as_str());
            ParticipantRecord {
                puuid: participant.puuid,
                discord_user_id: roster_player.map(|player| player.discord_user_id.clone()),
                team: side_from_team_id(participant.team_id)
                    .or_else(|| roster_player.and_then(|player| player.team_side())),
                champion_id: participant.champion_id.map(i64::from),
                champion_name: participant.champion_name,
                win: participant.win,
                kills: participant.kills.map(i64::from),
                deaths: participant.deaths.map(i64::from),
                assists: participant.assists.map(i64::from),
                total_damage: participant.total_damage.map(i64::from),
                gold_earned: participant.gold_earned.map(i64::from),
                total_minions: participant.total_minions.map(i64::from),
                vision_score: participant.vision_score.map(i64::from),
                raw_json: participant.raw_json,
            }
        })
        .collect();
    Ok(MatchRecord {
        riot_match_id: match_detail.riot_match_id,
        guild_id: game.guild_id.clone(),
        mode,
        queue_id: match_detail.queue_id.map(i64::from),
        map_id: match_detail.map_id.map(i64::from),
        riot_game_mode: match_detail.game_mode,
        riot_game_type: match_detail.game_type,
        data_source: "match_v5".to_owned(),
        payload_json: match_detail.payload_json,
        participants,
    })
}

fn manual_match_record(guild_id: &str, mode: GameModeKind, riot_match_id: &str) -> MatchRecord {
    MatchRecord {
        riot_match_id: riot_match_id.to_owned(),
        guild_id: guild_id.to_owned(),
        mode,
        queue_id: None,
        map_id: None,
        riot_game_mode: None,
        riot_game_type: None,
        data_source: "manual".to_owned(),
        payload_json: None,
        participants: Vec::new(),
    }
}

fn side_from_team_id(team_id: Option<u16>) -> Option<TeamSide> {
    match team_id {
        Some(100) => Some(TeamSide::Blue),
        Some(200) => Some(TeamSide::Red),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
struct StatsAverages {
    kills: Option<f64>,
    deaths: Option<f64>,
    assists: Option<f64>,
    damage: Option<f64>,
}

fn format_stats_line(
    label: &str,
    rating: i32,
    wins: i32,
    losses: i32,
    win_rate: f64,
    averages: StatsAverages,
) -> String {
    let mut line = format!(
        "{label}: {wins}W-{losses}L, {:.1}% WR, rating {rating}",
        win_rate * 100.0
    );
    if let (Some(kills), Some(deaths), Some(assists)) =
        (averages.kills, averages.deaths, averages.assists)
    {
        line.push_str(&format!(", {:.1}/{:.1}/{:.1} KDA", kills, deaths, assists));
    }
    if let Some(damage) = averages.damage {
        line.push_str(&format!(", {:.0} avg dmg", damage));
    }
    line
}
