use async_trait::async_trait;
use rsso_discord::{
    AddCommand, CreateCommand, DiscordCommand, FinishCommand, GameCommand, HydrateCommand,
    LinkMatchCommand, ResultsCommand, StatsCommand, WinnerCommand,
};
use rsso_domain::{
    split_even_teams, DiscordUserId, GameId, GameModeKind, GameStatus, QueueId, Rng,
    TeamAssignment, TeamSide,
};
use rsso_riot::parse_riot_id;
use rsso_storage::{
    is_pending_riot_id, GameRow, LeaderboardRow, MatchLinkRow, MatchRecord, NewGame, NewPlayer,
    ParticipantRecord, PlayerStatsRow, RosterPlayer, Storage, StorageError, TeammateStatsRow,
};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

const STATS_OVERVIEW_LIMIT: u8 = 100;
const LEADERBOARD_LIMIT: u8 = 10;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAudience {
    Ephemeral,
    Public,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResponse {
    pub content: String,
    pub audience: CommandAudience,
    pub team_card: Option<TeamCard>,
    pub stats_card: Option<StatsCard>,
    pub stats_overview_card: Option<StatsOverviewCard>,
}

impl CommandResponse {
    pub fn ephemeral(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            audience: CommandAudience::Ephemeral,
            team_card: None,
            stats_card: None,
            stats_overview_card: None,
        }
    }

    pub fn public(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            audience: CommandAudience::Public,
            team_card: None,
            stats_card: None,
            stats_overview_card: None,
        }
    }

    pub fn with_team_card(mut self, team_card: TeamCard) -> Self {
        self.team_card = Some(team_card);
        self
    }

    pub fn with_stats_card(mut self, stats_card: StatsCard) -> Self {
        self.stats_card = Some(stats_card);
        self
    }

    pub fn with_stats_overview_card(mut self, stats_overview_card: StatsOverviewCard) -> Self {
        self.stats_overview_card = Some(stats_overview_card);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamCard {
    pub game_id: GameId,
    pub mode: GameModeKind,
    pub red: Vec<DiscordUserId>,
    pub blue: Vec<DiscordUserId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsCard {
    pub discord_user_id: DiscordUserId,
    pub riot_id: String,
    pub mode_label: String,
    pub rating: i32,
    pub wins: i32,
    pub losses: i32,
    pub games_total: i32,
    pub win_rate: String,
    pub kda: Option<String>,
    pub average_damage: Option<String>,
    pub most_won_with: Vec<String>,
    pub most_lost_with: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsOverviewCard {
    pub mode_label: String,
    pub rows: Vec<StatsOverviewRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsOverviewRow {
    pub rank: usize,
    pub discord_user_id: DiscordUserId,
    pub riot_id: String,
    pub rating: i32,
    pub wins: i32,
    pub losses: i32,
    pub games_total: i32,
    pub win_rate: String,
    pub kda: Option<String>,
    pub average_damage: Option<String>,
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
    handle_command_with_resolver(storage, &NoopRiotAccountResolver, context, command, rng)
        .await
        .map(|response| response.content)
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
) -> Result<CommandResponse, HandlerError> {
    match command {
        DiscordCommand::RegisterSummoners { riot_id } => {
            handle_register(storage, resolver, context, &riot_id)
                .await
                .map(CommandResponse::ephemeral)
        }
        DiscordCommand::Create(command) => handle_create(storage, context, command, rng).await,
        DiscordCommand::Game(command) => handle_game(storage, context, command, rng)
            .await
            .map(CommandResponse::public),
        DiscordCommand::Add(command) => handle_add(storage, context, command, rng).await,
        DiscordCommand::Next => handle_next(storage, context).await,
        DiscordCommand::Randomize { game_id } => handle_randomize(storage, context, game_id, rng)
            .await
            .map(CommandResponse::public),
        DiscordCommand::Winner(command) => handle_winner(storage, context, command).await,
        DiscordCommand::Result { game_id, winner } => {
            handle_result(storage, context, game_id, winner)
                .await
                .map(CommandResponse::public)
        }
        DiscordCommand::Results(command) => handle_results(storage, resolver, context, command)
            .await
            .map(CommandResponse::public),
        DiscordCommand::Finish(command) => handle_finish(storage, resolver, context, command)
            .await
            .map(CommandResponse::public),
        DiscordCommand::Hydrate(command) => handle_hydrate(storage, resolver, context, command)
            .await
            .map(CommandResponse::ephemeral),
        DiscordCommand::LinkMatch(command) => {
            handle_link_match(storage, resolver, context, command)
                .await
                .map(CommandResponse::public)
        }
        DiscordCommand::End { game_id } => handle_end(storage, context, game_id)
            .await
            .map(CommandResponse::public),
        DiscordCommand::Status { game_id } => handle_status(storage, context, game_id)
            .await
            .map(CommandResponse::ephemeral),
        DiscordCommand::Stats(command) => handle_stats(storage, context, command).await,
        DiscordCommand::Leaderboards { mode } => {
            let rows = storage
                .leaderboard(&context.guild_id, mode, LEADERBOARD_LIMIT)
                .await?;
            if rows.is_empty() {
                return Ok(CommandResponse::public("No leaderboard rows yet."));
            }
            let overview = stats_overview_card(&rows, mode);
            Ok(CommandResponse::public(format_stats_overview(&overview))
                .with_stats_overview_card(overview))
        }
        DiscordCommand::Analysis { mode } => {
            let suffix = mode.map_or(String::new(), |mode| format!(" for {}", mode.as_str()));
            Ok(CommandResponse::ephemeral(format!(
                "Analysis{suffix} is wired as a v1 stub. Need more rows first."
            )))
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

async fn handle_create<S: Storage + ?Sized>(
    storage: &S,
    context: CommandContext,
    command: CreateCommand,
    rng: &mut impl Rng,
) -> Result<CommandResponse, HandlerError> {
    ensure_unique_users(&command.users)?;

    let assignments = split_even_teams(&command.users, rng)
        .map_err(|err| HandlerError::UserFacing(format!("Cannot randomize: {err}")))?;
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
                guild_id: context.guild_id,
                channel_id: context.channel_id,
                creator_discord_id: context.actor_id,
                mode: command.mode,
                now: context.now,
            },
            &user_ids,
        )
        .await
        .map_err(|error| match error {
            StorageError::ActiveGameExists => HandlerError::UserFacing(
                "An open game already exists. Use `/status`, then close it with `/winner` or `/finish` before creating another.".to_owned(),
            ),
            other => HandlerError::Storage(other),
        })?;
    storage
        .assign_teams(&game_id, &assignments, context.now)
        .await?;
    Ok(CommandResponse::public(format!(
        "Created game `{}` ({})",
        game_id,
        command.mode.as_str()
    ))
    .with_team_card(team_card(&game_id, command.mode, &assignments)))
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
    command: AddCommand,
    rng: &mut impl Rng,
) -> Result<CommandResponse, HandlerError> {
    let (game_id, game) = load_game_for_guild(
        storage,
        &context.guild_id,
        command.game_id.clone(),
        "No open game to add players to.",
    )
    .await?;
    let status = game
        .status()
        .map_err(|err| HandlerError::UserFacing(format!("Invalid stored status: {err}")))?;
    if !matches!(status, GameStatus::Lobby | GameStatus::Randomized) {
        return Err(HandlerError::UserFacing(
            "Cannot add players after the lobby is locked.".to_owned(),
        ));
    }

    ensure_unique_users(&command.users)?;

    let roster = storage.roster(&game_id).await?;
    let roster_ids = roster
        .iter()
        .map(|player| player.discord_user_id.as_str())
        .collect::<HashSet<_>>();
    for user in &command.users {
        if roster_ids.contains(user.as_str()) {
            return Err(HandlerError::UserFacing(format!(
                "<@{}> is already in {}.",
                user.as_str(),
                game_id
            )));
        }
    }

    for user in &command.users {
        storage
            .add_player(&game_id, &context.guild_id, user.as_str(), context.now)
            .await?;
    }

    let mentions = command
        .users
        .iter()
        .map(|user| format!("<@{}>", user.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    let updated_roster = storage.roster(&game_id).await?;
    let player_count = updated_roster.len();
    if player_count % 2 != 0 {
        return Ok(CommandResponse::public(format!(
            "Added {mentions} to {game_id}. Roster has {player_count} players; add one more before teams are assigned."
        )));
    }

    let users = updated_roster
        .iter()
        .map(|player| DiscordUserId::new(player.discord_user_id.clone()))
        .collect::<Vec<_>>();
    let assignments = split_even_teams(&users, rng)
        .map_err(|err| HandlerError::UserFacing(format!("Cannot randomize: {err}")))?;
    storage
        .assign_teams(&game_id, &assignments, context.now)
        .await?;
    let prefix = if status == GameStatus::Randomized {
        "Teams were re-randomized."
    } else {
        "Teams were randomized."
    };
    let mode = game
        .mode()
        .map_err(|err| HandlerError::UserFacing(format!("Invalid stored mode: {err}")))?;
    Ok(
        CommandResponse::public(format!("Added {mentions} to `{game_id}`. {prefix}"))
            .with_team_card(team_card(&game_id, mode, &assignments)),
    )
}

async fn handle_next<S: Storage + ?Sized>(
    storage: &S,
    context: CommandContext,
) -> Result<CommandResponse, HandlerError> {
    if storage
        .open_game_for_guild(&context.guild_id)
        .await?
        .is_some()
    {
        return Err(HandlerError::UserFacing(
            "Close the current open game with `/winner` before starting `/next`.".to_owned(),
        ));
    }

    let previous = storage
        .latest_game_for_guild(&context.guild_id)
        .await?
        .ok_or_else(|| {
            HandlerError::UserFacing("No previous game found for `/next`.".to_owned())
        })?;
    let previous_id = GameId::new(previous.game_id.clone());
    let mode = previous
        .mode()
        .map_err(|err| HandlerError::UserFacing(format!("Invalid stored mode: {err}")))?;
    let roster = storage.roster(&previous_id).await?;
    let assignments = next_rotation_assignments(&roster)?;
    let game_id = GameId::new(format!("g_{}", nanoid::nanoid!(10)));
    let user_ids = roster
        .iter()
        .map(|player| player.discord_user_id.clone())
        .collect::<Vec<_>>();

    storage
        .create_game(
            NewGame {
                game_id: game_id.as_str().to_owned(),
                guild_id: context.guild_id,
                channel_id: context.channel_id,
                creator_discord_id: context.actor_id,
                mode,
                now: context.now,
            },
            &user_ids,
        )
        .await
        .map_err(|error| match error {
            StorageError::ActiveGameExists => HandlerError::UserFacing(
                "An open game already exists. Close it with `/winner` first.".to_owned(),
            ),
            other => HandlerError::Storage(other),
        })?;
    storage
        .assign_teams(&game_id, &assignments, context.now)
        .await?;

    Ok(CommandResponse::public(format!(
        "Created next game `{}` from `{}` ({})",
        game_id,
        previous_id,
        mode.as_str()
    ))
    .with_team_card(team_card(&game_id, mode, &assignments)))
}

fn ensure_unique_users(users: &[DiscordUserId]) -> Result<(), HandlerError> {
    let mut seen = HashSet::new();
    for user in users {
        if !seen.insert(user.as_str()) {
            return Err(HandlerError::UserFacing(format!(
                "<@{}> was provided more than once.",
                user.as_str()
            )));
        }
    }
    Ok(())
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
    format!("Red team: {red}\nBlue team: {blue}")
}

fn team_card(game_id: &GameId, mode: GameModeKind, assignments: &[TeamAssignment]) -> TeamCard {
    TeamCard {
        game_id: game_id.clone(),
        mode,
        red: team_members(assignments, TeamSide::Red),
        blue: team_members(assignments, TeamSide::Blue),
    }
}

fn team_members(assignments: &[TeamAssignment], side: TeamSide) -> Vec<DiscordUserId> {
    assignments
        .iter()
        .filter(|assignment| assignment.team == side)
        .map(|assignment| assignment.discord_user_id.clone())
        .collect()
}

fn next_rotation_assignments(roster: &[RosterPlayer]) -> Result<Vec<TeamAssignment>, HandlerError> {
    if roster.len() < 2 {
        return Err(HandlerError::UserFacing(
            "`/next` needs at least two players in the previous game.".to_owned(),
        ));
    }
    if roster.len() % 2 != 0 {
        return Err(HandlerError::UserFacing(
            "`/next` needs an even previous roster. Use `/create` for the new roster.".to_owned(),
        ));
    }

    let mut players = roster
        .iter()
        .map(|player| DiscordUserId::new(player.discord_user_id.clone()))
        .collect::<Vec<_>>();
    players.sort_by(|left, right| left.as_str().cmp(right.as_str()));

    let team_size = players.len() / 2;
    let combinations = index_combinations(players.len(), team_size);
    let current_blue = roster
        .iter()
        .filter(|player| player.team_side() == Some(TeamSide::Blue))
        .map(|player| player.discord_user_id.as_str())
        .collect::<HashSet<_>>();
    let current = players
        .iter()
        .enumerate()
        .filter_map(|(index, player)| current_blue.contains(player.as_str()).then_some(index))
        .collect::<Vec<_>>();
    let next = combinations
        .iter()
        .position(|candidate| candidate == &current)
        .map_or_else(
            || combinations.first().cloned(),
            |index| combinations.get((index + 1) % combinations.len()).cloned(),
        )
        .ok_or_else(|| HandlerError::UserFacing("Could not build the next rotation.".to_owned()))?;
    let blue_indices = next.into_iter().collect::<HashSet<_>>();

    let mut blue_slot = 0_u8;
    let mut red_slot = 0_u8;
    let assignments = players
        .into_iter()
        .enumerate()
        .map(|(index, discord_user_id)| {
            let (team, slot) = if blue_indices.contains(&index) {
                let slot = blue_slot;
                blue_slot = blue_slot.saturating_add(1);
                (TeamSide::Blue, slot)
            } else {
                let slot = red_slot;
                red_slot = red_slot.saturating_add(1);
                (TeamSide::Red, slot)
            };
            TeamAssignment {
                discord_user_id,
                team,
                slot,
            }
        })
        .collect();
    Ok(assignments)
}

fn index_combinations(size: usize, selected: usize) -> Vec<Vec<usize>> {
    if selected == 0 || selected > size {
        return Vec::new();
    }

    let mut combinations = Vec::new();
    let mut current = Vec::with_capacity(selected);
    push_index_combinations(0, size, selected, &mut current, &mut combinations);
    combinations
}

fn push_index_combinations(
    start: usize,
    size: usize,
    selected: usize,
    current: &mut Vec<usize>,
    combinations: &mut Vec<Vec<usize>>,
) {
    if current.len() == selected {
        combinations.push(current.clone());
        return;
    }
    let remaining = selected - current.len();
    if size.saturating_sub(start) < remaining {
        return;
    }
    let last_start = size - remaining;
    for index in start..=last_start {
        current.push(index);
        push_index_combinations(index + 1, size, selected, current, combinations);
        current.pop();
    }
}

async fn handle_winner<S: Storage + ?Sized>(
    storage: &S,
    context: CommandContext,
    command: WinnerCommand,
) -> Result<CommandResponse, HandlerError> {
    let game = storage.game_by_id(&command.game_id).await?;
    ensure_game_in_guild(&game, &context.guild_id)?;
    let status = game
        .status()
        .map_err(|err| HandlerError::UserFacing(format!("Invalid stored status: {err}")))?;
    match status {
        GameStatus::Lobby => Err(HandlerError::UserFacing(
            "Randomize teams before marking a winner.".to_owned(),
        )),
        GameStatus::Finalized => Err(HandlerError::UserFacing(
            "That game is already finalized.".to_owned(),
        )),
        GameStatus::Cancelled => Err(HandlerError::UserFacing(
            "That game is cancelled.".to_owned(),
        )),
        GameStatus::Randomized
        | GameStatus::Ingame
        | GameStatus::Reported
        | GameStatus::Ambiguous => {
            storage
                .record_vote(
                    &command.game_id,
                    &context.actor_id,
                    command.winner,
                    context.now,
                )
                .await?;
            storage
                .finalize_game(
                    &command.game_id,
                    command.winner,
                    game.riot_match_id.as_deref(),
                    context.now,
                )
                .await?;
            Ok(CommandResponse::public(format!(
                "Marked game {} won by {}.",
                command.game_id,
                command.winner.as_str()
            )))
        }
    }
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
    let target = load_hydrate_target(storage, &context, command).await?;
    ensure_match_linkable(&target.game)?;

    if target.riot_match_ids.is_empty() {
        return Ok(format!(
            "{} has no linked matches missing Riot stats.",
            target.game_id
        ));
    }

    let roster = storage.roster(&target.game_id).await?;
    let mut outcomes = Vec::new();
    for riot_match_id in &target.riot_match_ids {
        outcomes.push(
            sync_riot_match(
                storage,
                resolver,
                MatchSyncRequest {
                    game_id: &target.game_id,
                    game: &target.game,
                    roster: &roster,
                    riot_match_id,
                    link_when_unavailable: target.link_when_unavailable,
                    now: context.now,
                },
            )
            .await?,
        );
    }

    Ok(format_match_sync_outcomes(&target.game_id, &outcomes))
}

async fn handle_link_match<S: Storage + ?Sized>(
    storage: &S,
    resolver: &(impl RiotMatchResolver + ?Sized),
    context: CommandContext,
    command: LinkMatchCommand,
) -> Result<String, HandlerError> {
    let (game_id, game) = load_game_for_guild(
        storage,
        &context.guild_id,
        command.game_id.clone(),
        "No open game to link. Pass `game_id` to link a closed session.",
    )
    .await?;
    ensure_match_linkable(&game)?;
    ensure_match_not_linked_elsewhere(storage, &context.guild_id, &game_id, &command.riot_match_id)
        .await?;
    let roster = storage.roster(&game_id).await?;
    let outcome = sync_riot_match(
        storage,
        resolver,
        MatchSyncRequest {
            game_id: &game_id,
            game: &game,
            roster: &roster,
            riot_match_id: &command.riot_match_id,
            link_when_unavailable: true,
            now: context.now,
        },
    )
    .await?;
    Ok(format_match_sync_outcomes(&game_id, &[outcome]))
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

#[derive(Debug)]
struct HydrateTarget {
    game_id: GameId,
    game: GameRow,
    riot_match_ids: Vec<String>,
    link_when_unavailable: bool,
}

async fn load_hydrate_target<S: Storage + ?Sized>(
    storage: &S,
    context: &CommandContext,
    command: HydrateCommand,
) -> Result<HydrateTarget, HandlerError> {
    match (command.game_id, command.riot_match_id) {
        (Some(game_id), Some(riot_match_id)) => {
            let game = storage.game_by_id(&game_id).await?;
            ensure_game_in_guild(&game, &context.guild_id)?;
            ensure_match_not_linked_elsewhere(storage, &context.guild_id, &game_id, &riot_match_id)
                .await?;
            Ok(HydrateTarget {
                game_id,
                game,
                riot_match_ids: vec![riot_match_id],
                link_when_unavailable: true,
            })
        }
        (Some(game_id), None) => {
            let game = storage.game_by_id(&game_id).await?;
            ensure_game_in_guild(&game, &context.guild_id)?;
            let riot_match_ids = missing_match_ids(storage, &game_id, &game).await?;
            Ok(HydrateTarget {
                game_id,
                game,
                riot_match_ids,
                link_when_unavailable: false,
            })
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
            Ok(HydrateTarget {
                game_id,
                game,
                riot_match_ids: vec![riot_match_id],
                link_when_unavailable: false,
            })
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
            let riot_match_ids = missing_match_ids(storage, &game_id, &game).await?;
            Ok(HydrateTarget {
                game_id,
                game,
                riot_match_ids,
                link_when_unavailable: false,
            })
        }
    }
}

async fn missing_match_ids<S: Storage + ?Sized>(
    storage: &S,
    game_id: &GameId,
    game: &GameRow,
) -> Result<Vec<String>, HandlerError> {
    let links = storage.matches_for_game(game_id).await?;
    let mut missing = links
        .iter()
        .filter(|link| link.needs_hydration())
        .map(|link| link.riot_match_id.clone())
        .collect::<Vec<_>>();
    if let Some(riot_match_id) = game.riot_match_id.as_ref() {
        if !links
            .iter()
            .any(|link| link.riot_match_id == *riot_match_id)
        {
            missing.push(riot_match_id.clone());
        }
    }
    if links.is_empty() && missing.is_empty() {
        return Err(HandlerError::UserFacing(format!(
            "{game_id} has no Riot match id yet."
        )));
    }
    Ok(missing)
}

fn ensure_match_linkable(game: &GameRow) -> Result<(), HandlerError> {
    let status = game
        .status()
        .map_err(|err| HandlerError::UserFacing(format!("Invalid stored status: {err}")))?;
    match status {
        GameStatus::Lobby => Err(HandlerError::UserFacing(
            "Randomize teams before linking Riot matches.".to_owned(),
        )),
        GameStatus::Cancelled => Err(HandlerError::UserFacing(
            "Cannot link Riot matches to a cancelled game.".to_owned(),
        )),
        GameStatus::Randomized
        | GameStatus::Ingame
        | GameStatus::Reported
        | GameStatus::Finalized
        | GameStatus::Ambiguous => Ok(()),
    }
}

async fn ensure_match_not_linked_elsewhere<S: Storage + ?Sized>(
    storage: &S,
    guild_id: &str,
    game_id: &GameId,
    riot_match_id: &str,
) -> Result<(), HandlerError> {
    let Some(existing) = storage
        .game_by_riot_match_id(guild_id, riot_match_id)
        .await?
    else {
        return Ok(());
    };
    if existing.game_id == game_id.as_str() {
        Ok(())
    } else {
        Err(HandlerError::UserFacing(format!(
            "{riot_match_id} is already linked to {}.",
            existing.game_id
        )))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MatchSyncOutcome {
    Hydrated {
        riot_match_id: String,
        participant_count: usize,
    },
    LinkedManual {
        riot_match_id: String,
    },
    Unavailable {
        riot_match_id: String,
    },
}

#[derive(Debug, Clone, Copy)]
struct MatchSyncRequest<'a> {
    game_id: &'a GameId,
    game: &'a GameRow,
    roster: &'a [RosterPlayer],
    riot_match_id: &'a str,
    link_when_unavailable: bool,
    now: i64,
}

async fn sync_riot_match<S: Storage + ?Sized>(
    storage: &S,
    resolver: &(impl RiotMatchResolver + ?Sized),
    request: MatchSyncRequest<'_>,
) -> Result<MatchSyncOutcome, HandlerError> {
    let mode = request
        .game
        .mode()
        .map_err(|err| HandlerError::UserFacing(format!("Invalid stored mode: {err}")))?;
    let Some(match_detail) = resolver.resolve_match(request.riot_match_id).await? else {
        if request.link_when_unavailable {
            storage
                .record_match(
                    request.game_id,
                    manual_match_record(&request.game.guild_id, mode, request.riot_match_id),
                    request.now,
                )
                .await?;
            return Ok(MatchSyncOutcome::LinkedManual {
                riot_match_id: request.riot_match_id.to_owned(),
            });
        }
        return Ok(MatchSyncOutcome::Unavailable {
            riot_match_id: request.riot_match_id.to_owned(),
        });
    };

    validate_match_mode(mode, &match_detail)?;
    let stored_winner: Option<TeamSide> = request
        .game
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
                request.game_id
            )));
        }
    }

    let match_record = build_match_record(request.game, request.roster, match_detail)?;
    let riot_match_id = match_record.riot_match_id.clone();
    let participant_count = match_record.participants.len();
    storage
        .record_match(request.game_id, match_record, request.now)
        .await?;
    Ok(MatchSyncOutcome::Hydrated {
        riot_match_id,
        participant_count,
    })
}

fn format_match_sync_outcomes(game_id: &GameId, outcomes: &[MatchSyncOutcome]) -> String {
    if let [outcome] = outcomes {
        return match outcome {
            MatchSyncOutcome::Hydrated {
                riot_match_id,
                participant_count,
            } => format!(
                "{game_id} hydrated from {riot_match_id} with {participant_count} participant row(s)."
            ),
            MatchSyncOutcome::LinkedManual { riot_match_id } => format!(
                "{game_id} linked to {riot_match_id}. Riot still does not return Match-V5 data; run `/hydrate game_id:{game_id}` later."
            ),
            MatchSyncOutcome::Unavailable { riot_match_id } => format!(
                "{game_id} is linked to {riot_match_id}, but Riot still does not return Match-V5 data for it."
            ),
        };
    }

    let mut hydrated_ids = Vec::new();
    let mut participant_rows = 0_usize;
    let mut linked_ids = Vec::new();
    let mut unavailable_ids = Vec::new();
    for outcome in outcomes {
        match outcome {
            MatchSyncOutcome::Hydrated {
                riot_match_id,
                participant_count,
            } => {
                hydrated_ids.push(riot_match_id.clone());
                participant_rows += *participant_count;
            }
            MatchSyncOutcome::LinkedManual { riot_match_id } => {
                linked_ids.push(riot_match_id.clone());
            }
            MatchSyncOutcome::Unavailable { riot_match_id } => {
                unavailable_ids.push(riot_match_id.clone());
            }
        }
    }

    let mut parts = Vec::new();
    if !hydrated_ids.is_empty() {
        parts.push(format!(
            "hydrated {} match(es) ({}) with {participant_rows} participant row(s)",
            hydrated_ids.len(),
            format_limited_ids(&hydrated_ids)
        ));
    }
    if !linked_ids.is_empty() {
        parts.push(format!(
            "linked {} match(es) still unavailable from Riot ({})",
            linked_ids.len(),
            format_limited_ids(&linked_ids)
        ));
    }
    if !unavailable_ids.is_empty() {
        parts.push(format!(
            "{} linked match(es) still unavailable from Riot ({})",
            unavailable_ids.len(),
            format_limited_ids(&unavailable_ids)
        ));
    }
    format!("{game_id}: {}.", parts.join("; "))
}

fn format_limited_ids(ids: &[String]) -> String {
    let shown = ids
        .iter()
        .take(3)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    if ids.len() > 3 {
        format!("{shown}, +{} more", ids.len() - 3)
    } else {
        shown
    }
}

fn format_status_match_links(links: &[MatchLinkRow], fallback: Option<&str>) -> String {
    if links.is_empty() {
        return fallback.map_or_else(
            || "Riot matches: pending until Spectator sees the live game".to_owned(),
            |match_id| format!("Riot matches: 1 linked ({match_id}, missing stats)"),
        );
    }

    let ids = links
        .iter()
        .take(3)
        .map(|link| link.riot_match_id.clone())
        .collect::<Vec<_>>();
    let suffix = if links.len() > 3 {
        format!(", +{} more", links.len() - 3)
    } else {
        String::new()
    };
    let missing = links.iter().filter(|link| link.needs_hydration()).count();
    let hydrate_status = if missing > 0 {
        format!(", {missing} missing stats")
    } else {
        ", stats hydrated".to_owned()
    };
    format!(
        "Riot matches: {} linked ({}{}{})",
        links.len(),
        ids.join(", "),
        suffix,
        hydrate_status
    )
}

async fn handle_status<S: Storage + ?Sized>(
    storage: &S,
    context: CommandContext,
    game_id: Option<GameId>,
) -> Result<String, HandlerError> {
    let (game_id, game) =
        load_game_for_guild(storage, &context.guild_id, game_id, "No open game found.").await?;
    let match_links = storage.matches_for_game(&game_id).await?;
    let riot_link = format_status_match_links(&match_links, game.riot_match_id.as_deref());
    Ok(format!(
        "{}: mode {}, status {}, {}",
        game_id, game.mode, game.status, riot_link
    ))
}

async fn handle_stats<S: Storage + ?Sized>(
    storage: &S,
    context: CommandContext,
    command: StatsCommand,
) -> Result<CommandResponse, HandlerError> {
    if command.user.is_none() && command.name.is_none() {
        let rows = storage
            .leaderboard(&context.guild_id, command.mode, STATS_OVERVIEW_LIMIT)
            .await?;
        if rows.is_empty() {
            return Ok(CommandResponse::ephemeral("No stats found yet."));
        }
        let overview = stats_overview_card(&rows, command.mode);
        return Ok(CommandResponse::ephemeral(format_stats_overview(&overview))
            .with_stats_overview_card(overview));
    }

    let discord_user_id = match (command.user, command.name.as_deref()) {
        (Some(user), None) => user,
        (None, Some(name)) => storage
            .get_player_by_riot_name(&context.guild_id, name)
            .await?
            .map(|player| DiscordUserId::new(player.discord_user_id))
            .ok_or_else(|| {
                HandlerError::UserFacing(format!("No registered player found for `{name}`."))
            })?,
        (None, None) => {
            return Err(HandlerError::UserFacing(
                "Use `/stats`, `/stats user:...`, or `/stats name:...`.".to_owned(),
            ));
        }
        (Some(_), Some(_)) => {
            return Err(HandlerError::UserFacing(
                "Use either `name` or `user`, not both.".to_owned(),
            ));
        }
    };

    let Some(stats) = storage
        .stats_for_player(&context.guild_id, discord_user_id.as_str(), command.mode)
        .await?
    else {
        return Ok(CommandResponse::ephemeral(
            "No stats found for that player yet.",
        ));
    };
    let teammates = storage
        .teammate_stats(&context.guild_id, discord_user_id.as_str(), command.mode)
        .await?;
    Ok(
        CommandResponse::ephemeral(format_detailed_stats(&stats, &teammates))
            .with_stats_card(stats_card(&stats, &teammates, command.mode)),
    )
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

fn format_detailed_stats(stats: &PlayerStatsRow, teammates: &[TeammateStatsRow]) -> String {
    let games_total = stats.wins + stats.losses;
    let win_rate = stats.win_rate * 100.0;
    let most_won = teammates
        .iter()
        .filter(|row| row.wins > 0)
        .max_by_key(|row| (row.wins, row.games));
    let most_lost = teammates
        .iter()
        .filter(|row| row.losses > 0)
        .max_by_key(|row| (row.losses, row.games));

    format!(
        "{}\nW: {} L: {} Games total: {} WR: {:.1}% Rating: {}\nMost won with players: {}\nMost lost with players: {}",
        display_riot_id(&stats.discord_user_id, &stats.riot_game_name, &stats.riot_tag_line),
        stats.wins,
        stats.losses,
        games_total,
        win_rate,
        stats.rating,
        format_teammate_row(most_won),
        format_teammate_row(most_lost),
    )
}

fn stats_overview_card(rows: &[LeaderboardRow], mode: Option<GameModeKind>) -> StatsOverviewCard {
    StatsOverviewCard {
        mode_label: mode_label(mode),
        rows: rows
            .iter()
            .enumerate()
            .map(|(index, row)| stats_overview_row(index + 1, row))
            .collect(),
    }
}

fn stats_overview_row(rank: usize, row: &LeaderboardRow) -> StatsOverviewRow {
    StatsOverviewRow {
        rank,
        discord_user_id: DiscordUserId::new(row.discord_user_id.clone()),
        riot_id: display_riot_id(
            &row.discord_user_id,
            &row.riot_game_name,
            &row.riot_tag_line,
        ),
        rating: row.rating,
        wins: row.wins,
        losses: row.losses,
        games_total: row.wins + row.losses,
        win_rate: format!("{:.1}%", row.win_rate * 100.0),
        kda: format_kda_values(row.avg_kills, row.avg_deaths, row.avg_assists),
        average_damage: row
            .avg_total_damage
            .map(|damage| format!("{damage:.0} avg dmg")),
    }
}

fn format_stats_overview(card: &StatsOverviewCard) -> String {
    let rows = card
        .rows
        .iter()
        .map(|row| {
            format!(
                "{}. {} {}W/{}L {} WR rating {}",
                row.rank, row.riot_id, row.wins, row.losses, row.win_rate, row.rating
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("Stats for everyone ({})\n{rows}", card.mode_label)
}

fn stats_card(
    stats: &PlayerStatsRow,
    teammates: &[TeammateStatsRow],
    mode: Option<GameModeKind>,
) -> StatsCard {
    StatsCard {
        discord_user_id: DiscordUserId::new(stats.discord_user_id.clone()),
        riot_id: display_riot_id(
            &stats.discord_user_id,
            &stats.riot_game_name,
            &stats.riot_tag_line,
        ),
        mode_label: mode_label(mode),
        rating: stats.rating,
        wins: stats.wins,
        losses: stats.losses,
        games_total: stats.wins + stats.losses,
        win_rate: format!("{:.1}%", stats.win_rate * 100.0),
        kda: format_kda_values(stats.avg_kills, stats.avg_deaths, stats.avg_assists),
        average_damage: stats
            .avg_total_damage
            .map(|damage| format!("{damage:.0} avg dmg")),
        most_won_with: top_teammates_by_wins(teammates)
            .into_iter()
            .map(format_teammate_card_row)
            .collect(),
        most_lost_with: top_teammates_by_losses(teammates)
            .into_iter()
            .map(format_teammate_card_row)
            .collect(),
    }
}

fn mode_label(mode: Option<GameModeKind>) -> String {
    mode.map_or_else(|| "all modes".to_owned(), |mode| mode.as_str().to_owned())
}

fn format_kda_values(
    kills: Option<f64>,
    deaths: Option<f64>,
    assists: Option<f64>,
) -> Option<String> {
    match (kills, deaths, assists) {
        (Some(kills), Some(deaths), Some(assists)) => {
            Some(format!("{kills:.1}/{deaths:.1}/{assists:.1} KDA"))
        }
        _ => None,
    }
}

fn display_riot_id(discord_user_id: &str, riot_game_name: &str, riot_tag_line: &str) -> String {
    if is_pending_riot_id(discord_user_id, riot_game_name, riot_tag_line) {
        "unregistered".to_owned()
    } else {
        format!("{riot_game_name}#{riot_tag_line}")
    }
}

fn top_teammates_by_wins(teammates: &[TeammateStatsRow]) -> Vec<&TeammateStatsRow> {
    let mut rows = teammates
        .iter()
        .filter(|row| row.wins > 0)
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .wins
            .cmp(&left.wins)
            .then_with(|| right.games.cmp(&left.games))
            .then_with(|| left.losses.cmp(&right.losses))
            .then_with(|| left.riot_game_name.cmp(&right.riot_game_name))
    });
    rows.truncate(3);
    rows
}

fn top_teammates_by_losses(teammates: &[TeammateStatsRow]) -> Vec<&TeammateStatsRow> {
    let mut rows = teammates
        .iter()
        .filter(|row| row.losses > 0)
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .losses
            .cmp(&left.losses)
            .then_with(|| right.games.cmp(&left.games))
            .then_with(|| left.wins.cmp(&right.wins))
            .then_with(|| left.riot_game_name.cmp(&right.riot_game_name))
    });
    rows.truncate(3);
    rows
}

fn format_teammate_card_row(row: &TeammateStatsRow) -> String {
    format!(
        "<@{}> - {} - {}W/{}L, {} games",
        row.discord_user_id,
        display_riot_id(
            &row.discord_user_id,
            &row.riot_game_name,
            &row.riot_tag_line
        ),
        row.wins,
        row.losses,
        row.games
    )
}

fn format_teammate_row(row: Option<&TeammateStatsRow>) -> String {
    row.map_or_else(
        || "none yet".to_owned(),
        |row| {
            format!(
                "{} ({}W/{}L, {} games)",
                display_riot_id(
                    &row.discord_user_id,
                    &row.riot_game_name,
                    &row.riot_tag_line
                ),
                row.wins,
                row.losses,
                row.games
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::{
        format_detailed_stats, format_match_sync_outcomes, format_status_match_links,
        next_rotation_assignments, stats_card, stats_overview_card, MatchSyncOutcome,
    };
    use rsso_domain::{GameId, GameModeKind, TeamSide};
    use rsso_storage::{
        pending_riot_game_name, LeaderboardRow, MatchLinkRow, PlayerStatsRow, RosterPlayer,
        TeammateStatsRow, PENDING_RIOT_TAG_LINE,
    };

    #[test]
    fn formats_batch_hydrate_outcomes() {
        let game_id = GameId::new("g_session");
        let message = format_match_sync_outcomes(
            &game_id,
            &[
                MatchSyncOutcome::Hydrated {
                    riot_match_id: "NA1_1".to_owned(),
                    participant_count: 10,
                },
                MatchSyncOutcome::Unavailable {
                    riot_match_id: "NA1_2".to_owned(),
                },
            ],
        );
        assert!(message.contains("hydrated 1 match(es)"));
        assert!(message.contains("1 linked match(es) still unavailable"));
    }

    #[test]
    fn status_reports_multiple_matches_and_missing_stats() {
        let message = format_status_match_links(
            &[
                MatchLinkRow {
                    riot_match_id: "NA1_2".to_owned(),
                    data_source: "manual".to_owned(),
                    queue_id: None,
                    map_id: None,
                    riot_game_mode: None,
                    riot_game_type: None,
                    finalized_at: 20,
                    participant_count: 0,
                },
                MatchLinkRow {
                    riot_match_id: "NA1_1".to_owned(),
                    data_source: "match_v5".to_owned(),
                    queue_id: Some(450),
                    map_id: Some(12),
                    riot_game_mode: Some("ARAM".to_owned()),
                    riot_game_type: Some("CUSTOM_GAME".to_owned()),
                    finalized_at: 10,
                    participant_count: 10,
                },
            ],
            None,
        );
        assert!(message.contains("2 linked"));
        assert!(message.contains("1 missing stats"));
    }

    #[test]
    fn formats_simple_stats_with_teammates() {
        let stats = PlayerStatsRow {
            guild_id: "g".to_owned(),
            discord_user_id: "1".to_owned(),
            riot_game_name: "Cyracen".to_owned(),
            riot_tag_line: "NA1".to_owned(),
            rating: 1520,
            wins: 12,
            losses: 8,
            win_rate: 0.6,
            avg_kills: None,
            avg_deaths: None,
            avg_assists: None,
            avg_total_damage: None,
        };
        let teammates = vec![
            TeammateStatsRow {
                discord_user_id: "2".to_owned(),
                riot_game_name: "Vu".to_owned(),
                riot_tag_line: "NA1".to_owned(),
                games: 5,
                wins: 4,
                losses: 1,
            },
            TeammateStatsRow {
                discord_user_id: "3".to_owned(),
                riot_game_name: "Chongly".to_owned(),
                riot_tag_line: "NA1".to_owned(),
                games: 6,
                wins: 1,
                losses: 5,
            },
        ];
        let message = format_detailed_stats(&stats, &teammates);
        assert!(message.contains("W: 12 L: 8 Games total: 20 WR: 60.0%"));
        assert!(message.contains("Most won with players: Vu#NA1"));
        assert!(message.contains("Most lost with players: Chongly#NA1"));
    }

    #[test]
    fn builds_stats_card_with_mentions_and_averages() {
        let stats = PlayerStatsRow {
            guild_id: "g".to_owned(),
            discord_user_id: "1".to_owned(),
            riot_game_name: "Cyracen".to_owned(),
            riot_tag_line: "NA1".to_owned(),
            rating: 1520,
            wins: 12,
            losses: 8,
            win_rate: 0.6,
            avg_kills: Some(8.2),
            avg_deaths: Some(4.4),
            avg_assists: Some(19.1),
            avg_total_damage: Some(28_450.0),
        };
        let teammates = vec![
            TeammateStatsRow {
                discord_user_id: "2".to_owned(),
                riot_game_name: "Vu".to_owned(),
                riot_tag_line: "NA1".to_owned(),
                games: 5,
                wins: 4,
                losses: 1,
            },
            TeammateStatsRow {
                discord_user_id: "3".to_owned(),
                riot_game_name: "Chongly".to_owned(),
                riot_tag_line: "NA1".to_owned(),
                games: 6,
                wins: 1,
                losses: 5,
            },
        ];

        let card = stats_card(&stats, &teammates, Some(GameModeKind::AramMayhem));

        assert_eq!(card.riot_id, "Cyracen#NA1");
        assert_eq!(card.mode_label, "aram_mayhem");
        assert_eq!(card.win_rate, "60.0%");
        assert_eq!(card.kda.as_deref(), Some("8.2/4.4/19.1 KDA"));
        assert_eq!(card.average_damage.as_deref(), Some("28450 avg dmg"));
        assert_eq!(
            card.most_won_with,
            vec![
                "<@2> - Vu#NA1 - 4W/1L, 5 games".to_owned(),
                "<@3> - Chongly#NA1 - 1W/5L, 6 games".to_owned(),
            ]
        );
        assert_eq!(
            card.most_lost_with,
            vec![
                "<@3> - Chongly#NA1 - 1W/5L, 6 games".to_owned(),
                "<@2> - Vu#NA1 - 4W/1L, 5 games".to_owned(),
            ]
        );
    }

    #[test]
    fn builds_everyone_stats_card_and_hides_pending_riot_id() {
        let rows = vec![
            LeaderboardRow {
                discord_user_id: "1".to_owned(),
                riot_game_name: "Cyracen".to_owned(),
                riot_tag_line: "NA1".to_owned(),
                rating: 1520,
                wins: 12,
                losses: 8,
                win_rate: 0.6,
                avg_kills: Some(8.2),
                avg_deaths: Some(4.4),
                avg_assists: Some(19.1),
                avg_total_damage: Some(28_450.0),
            },
            LeaderboardRow {
                discord_user_id: "2".to_owned(),
                riot_game_name: pending_riot_game_name("2"),
                riot_tag_line: PENDING_RIOT_TAG_LINE.to_owned(),
                rating: 1500,
                wins: 0,
                losses: 0,
                win_rate: 0.0,
                avg_kills: None,
                avg_deaths: None,
                avg_assists: None,
                avg_total_damage: None,
            },
        ];

        let card = stats_overview_card(&rows, None);

        assert_eq!(card.mode_label, "all modes");
        assert_eq!(card.rows[0].riot_id, "Cyracen#NA1");
        assert_eq!(card.rows[0].games_total, 20);
        assert_eq!(card.rows[0].kda.as_deref(), Some("8.2/4.4/19.1 KDA"));
        assert_eq!(card.rows[1].riot_id, "unregistered");
        assert_eq!(card.rows[1].games_total, 0);
    }

    #[test]
    fn next_rotation_advances_balanced_split() {
        let roster = vec![
            roster_player("1", "blue"),
            roster_player("2", "blue"),
            roster_player("3", "red"),
            roster_player("4", "red"),
        ];
        let assignments = next_rotation_assignments(&roster).expect("next rotation");
        let blue = assignments
            .iter()
            .filter(|assignment| assignment.team == TeamSide::Blue)
            .map(|assignment| assignment.discord_user_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(blue, vec!["1", "3"]);
    }

    fn roster_player(discord_user_id: &str, team: &str) -> RosterPlayer {
        RosterPlayer {
            discord_user_id: discord_user_id.to_owned(),
            riot_puuid: None,
            team: Some(team.to_owned()),
            rating: 1500,
        }
    }
}
