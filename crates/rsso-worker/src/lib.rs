#[cfg(target_arch = "wasm32")]
mod worker_entry {
    use async_trait::async_trait;
    use rsso_cron::{
        FinishedMatch, FinishedMatchProbe, FinishedParticipant, LiveGame, LiveGameProbe,
        LiveGameStatus,
    };
    use rsso_discord::interaction::InteractionType;
    use rsso_discord::response::{
        deferred_response, deferred_update_response, message_response, pong_response,
    };
    use rsso_discord::{
        parse_command, verify::verify_discord_request, DiscordCommand, Interaction, WinnerCommand,
    };
    use rsso_domain::{parse_riot_match_id, GameId, Rng, TeamSide};
    use rsso_handlers::{
        handle_command_with_resolver, CommandContext, CommandResponse, HandlerError,
        ResolvedRiotAccount, ResolvedRiotMatch, ResolvedRiotParticipant, RiotAccountResolver,
        RiotMatchResolver, StatsCard, StatsOverviewCard, TeamCard,
    };
    use rsso_riot::routing::{
        account_by_riot_id_url, active_game_url, match_detail_url,
        match_regional_route_for_platform,
    };
    use rsso_storage::{
        d1::D1Storage, GameRow, MatchLinkRow, PlayerStatsRow, RosterPlayer, Storage,
    };
    use serde::Serialize;
    use worker::{
        event, Context, Date, Env, Fetch, Headers, Method, Request, RequestInit, Response, Result,
        ScheduleContext,
    };

    #[event(fetch)]
    pub async fn fetch(mut req: Request, env: Env, ctx: Context) -> Result<Response> {
        console_error_panic_hook::set_once();

        match (req.method(), req.path().as_str()) {
            (Method::Get, "/healthz") => Response::ok("ok"),
            (Method::Get | Method::Head, "/riot.txt") => riot_txt_response(),
            (Method::Post, "/interactions" | "/discord/interactions") => {
                handle_interaction(&mut req, env, ctx).await
            }
            _ => Response::error("not found", 404),
        }
    }

    fn riot_txt_response() -> Result<Response> {
        let Some(value) = option_env!("RIOT_TXT")
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Response::error("riot.txt is not configured", 404);
        };
        let mut response = Response::ok(format!("{value}\n"))?;
        response
            .headers_mut()
            .set("Content-Type", "text/plain; charset=utf-8")?;
        response.headers_mut().set("Cache-Control", "no-store")?;
        Ok(response)
    }

    #[event(scheduled)]
    pub async fn scheduled(_event: worker::ScheduledEvent, env: Env, _ctx: ScheduleContext) {
        console_error_panic_hook::set_once();
        let Ok(db) = env.d1("DB") else {
            worker::console_error!("missing DB binding");
            return;
        };
        let storage = D1Storage::new(db);
        let resolver = WorkerRiotAccountResolver::from_env(&env);
        match rsso_cron::run_poll(&storage, &resolver, now()).await {
            Ok(summary) => worker::console_log!(
                "cron inspected={} marked_live={} bumped_not_found={} finalized={} missing_puuid={} probe_errors={}",
                summary.inspected,
                summary.marked_live,
                summary.bumped_not_found,
                summary.finalized,
                summary.missing_puuid,
                summary.probe_errors
            ),
            Err(error) => worker::console_error!("cron failed: {error}"),
        }
    }

    async fn handle_interaction(req: &mut Request, env: Env, ctx: Context) -> Result<Response> {
        let signature = req.headers().get("X-Signature-Ed25519")?;
        let timestamp = req.headers().get("X-Signature-Timestamp")?;
        let body = req.bytes().await?;
        let public_key = env.secret("DISCORD_PUBLIC_KEY")?.to_string();

        if let Err(error) = verify_discord_request(
            &public_key,
            signature.as_deref(),
            timestamp.as_deref(),
            &body,
        ) {
            return Response::error(format!("unauthorized: {error}"), 401);
        }

        let interaction = serde_json::from_slice::<Interaction>(&body)
            .map_err(|error| worker::Error::RustError(format!("invalid interaction: {error}")))?;

        if interaction.kind == InteractionType::Ping {
            return Response::from_json(&pong_response());
        }

        if interaction.kind == InteractionType::MessageComponent {
            return handle_component_interaction(interaction, env, ctx).await;
        }

        let Some(data) = interaction.data.as_ref() else {
            return Response::from_json(&message_response("Missing command data.", true));
        };
        let command = match parse_command(data) {
            Ok(command) => command,
            Err(error) => {
                return Response::from_json(&message_response(
                    format!("Command error: {error}"),
                    true,
                ))
            }
        };
        let Some(guild_id) = interaction.guild_id.clone() else {
            return Response::from_json(&message_response("Guild commands only for now.", true));
        };
        let Some(channel_id) = interaction.channel_id.clone() else {
            return Response::from_json(&message_response("Missing channel id.", true));
        };
        let Some(actor_id) = interaction.actor_user_id().map(str::to_owned) else {
            return Response::from_json(&message_response("Missing actor id.", true));
        };

        let context = CommandContext {
            guild_id,
            channel_id,
            actor_id,
            now: now(),
        };
        let ephemeral = command_initial_ephemeral(&command);
        let application_id = interaction.application_id;
        let token = interaction.token;
        let background_env = env.clone();
        ctx.wait_until(async move {
            let message = match background_env.d1("DB") {
                Ok(db) => {
                    let storage = D1Storage::new(db);
                    let resolver = WorkerRiotAccountResolver::from_env(&background_env);
                    let mut rng = WorkerRng::new();
                    match handle_command_with_resolver(
                        &storage, &resolver, context, command, &mut rng,
                    )
                    .await
                    {
                        Ok(response) => discord_message_from_response(&response),
                        Err(error) => DiscordWebhookMessage::plain(format!("{error}")),
                    }
                }
                Err(error) => DiscordWebhookMessage::plain(format!("D1 binding error: {error}")),
            };
            if let Err(error) = edit_original_response(&application_id, &token, &message).await {
                worker::console_error!("discord followup failed: {error}");
            }
        });
        Response::from_json(&deferred_response(ephemeral))
    }

    async fn handle_component_interaction(
        interaction: Interaction,
        env: Env,
        ctx: Context,
    ) -> Result<Response> {
        let Some(data) = interaction.data.as_ref() else {
            return Response::from_json(&message_response("Missing component data.", true));
        };
        let Some(custom_id) = data.custom_id.as_deref() else {
            return Response::from_json(&message_response("Missing component id.", true));
        };
        let Some((game_id, winner)) = parse_winner_button_id(custom_id) else {
            return Response::from_json(&message_response("Unknown button.", true));
        };
        let Some(guild_id) = interaction.guild_id.clone() else {
            return Response::from_json(&message_response("Guild buttons only for now.", true));
        };
        let Some(channel_id) = interaction.channel_id.clone() else {
            return Response::from_json(&message_response("Missing channel id.", true));
        };
        let Some(actor_id) = interaction.actor_user_id().map(str::to_owned) else {
            return Response::from_json(&message_response("Missing actor id.", true));
        };

        let context = CommandContext {
            guild_id,
            channel_id,
            actor_id,
            now: now(),
        };
        let application_id = interaction.application_id;
        let token = interaction.token;
        let background_env = env.clone();
        ctx.wait_until(async move {
            let summary_game_id = game_id.clone();
            let actor_id = context.actor_id.clone();
            let command = DiscordCommand::Winner(WinnerCommand { game_id, winner });
            let message = match background_env.d1("DB") {
                Ok(db) => {
                    let storage = D1Storage::new(db);
                    let resolver = WorkerRiotAccountResolver::from_env(&background_env);
                    let mut rng = WorkerRng::new();
                    match handle_command_with_resolver(
                        &storage, &resolver, context, command, &mut rng,
                    )
                    .await
                    {
                        Ok(response) => match result_message_for_game(
                            &storage,
                            &summary_game_id,
                            winner,
                            &actor_id,
                            response.content,
                        )
                        .await
                        {
                            Ok(message) => message,
                            Err(error) => DiscordWebhookMessage::plain(format!(
                                "Marked game {} won by {}.\nCould not load result card: {error}",
                                summary_game_id,
                                winner.as_str()
                            ))
                            .clear_rich_state(),
                        },
                        Err(error) => match result_message_for_game(
                            &storage,
                            &summary_game_id,
                            winner,
                            &actor_id,
                            format!("{error}"),
                        )
                        .await
                        {
                            Ok(message) => message,
                            Err(_) => {
                                DiscordWebhookMessage::plain(format!("{error}")).clear_rich_state()
                            }
                        },
                    }
                }
                Err(error) => DiscordWebhookMessage::plain(format!("D1 binding error: {error}")),
            };
            if let Err(error) = edit_original_response(&application_id, &token, &message).await {
                worker::console_error!("discord button update failed: {error}");
            }
        });
        Response::from_json(&deferred_update_response())
    }

    async fn result_message_for_game<S: Storage + ?Sized>(
        storage: &S,
        game_id: &GameId,
        fallback_winner: TeamSide,
        actor_id: &str,
        content: String,
    ) -> std::result::Result<DiscordWebhookMessage, String> {
        let game = storage
            .game_by_id(game_id)
            .await
            .map_err(|error| error.to_string())?;
        let mode = game
            .mode()
            .map_err(|error| format!("invalid stored mode: {error}"))?;
        let recorded_winner = game
            .winning_side
            .as_deref()
            .and_then(|side| side.parse().ok());
        let roster = storage
            .roster(game_id)
            .await
            .map_err(|error| error.to_string())?;
        let links = storage
            .matches_for_game(game_id)
            .await
            .map_err(|error| error.to_string())?;
        let mut red = Vec::new();
        let mut blue = Vec::new();
        for player in roster {
            let stats = storage
                .stats_for_player(&game.guild_id, &player.discord_user_id, Some(mode))
                .await
                .map_err(|error| error.to_string())?;
            let line = player_result_line(&player, stats.as_ref());
            match player.team_side() {
                Some(TeamSide::Red) => red.push(line),
                Some(TeamSide::Blue) => blue.push(line),
                None => {}
            }
        }

        Ok(DiscordWebhookMessage {
            content,
            embeds: Some(vec![result_embed(
                &game,
                recorded_winner,
                fallback_winner,
                actor_id,
                &team_lines(red),
                &team_lines(blue),
                &match_lines(&game, &links),
            )]),
            components: Some(Vec::new()),
        })
    }

    fn result_embed(
        game: &GameRow,
        recorded_winner: Option<TeamSide>,
        fallback_winner: TeamSide,
        actor_id: &str,
        red: &str,
        blue: &str,
        matches: &str,
    ) -> DiscordEmbed {
        let display_winner = recorded_winner.unwrap_or(fallback_winner);
        let (title, description, color) = if recorded_winner.is_some() {
            (
                format!("{} wins {}", team_title(display_winner), game.game_id),
                "Result recorded and ratings updated.".to_owned(),
                team_color(display_winner),
            )
        } else {
            (
                format!("Game {}", game.game_id),
                "No winner is recorded yet.".to_owned(),
                0xfee75c,
            )
        };
        DiscordEmbed {
            title,
            description,
            color,
            fields: vec![
                DiscordEmbedField {
                    name: "Current match".to_owned(),
                    value: matches.to_owned(),
                    inline: false,
                },
                DiscordEmbedField {
                    name: team_field_name(TeamSide::Red, recorded_winner),
                    value: red.to_owned(),
                    inline: true,
                },
                DiscordEmbedField {
                    name: team_field_name(TeamSide::Blue, recorded_winner),
                    value: blue.to_owned(),
                    inline: true,
                },
            ],
            footer: DiscordEmbedFooter {
                text: format!("Winner button used by {actor_id}"),
            },
        }
    }

    fn team_field_name(team: TeamSide, winner: Option<TeamSide>) -> String {
        if winner == Some(team) {
            format!("{} team (winner)", team_title(team))
        } else {
            format!("{} team", team_title(team))
        }
    }

    fn team_title(team: TeamSide) -> &'static str {
        match team {
            TeamSide::Red => "Red",
            TeamSide::Blue => "Blue",
        }
    }

    fn team_color(team: TeamSide) -> u32 {
        match team {
            TeamSide::Red => 0xed4245,
            TeamSide::Blue => 0x5865f2,
        }
    }

    fn team_lines(lines: Vec<String>) -> String {
        if lines.is_empty() {
            return "none".to_owned();
        }
        let mut value = lines.join("\n");
        truncate_embed_value(&mut value);
        value
    }

    fn player_result_line(player: &RosterPlayer, stats: Option<&PlayerStatsRow>) -> String {
        let Some(stats) = stats else {
            return format!(
                "<@{}> - {} rating - no record yet",
                player.discord_user_id, player.rating
            );
        };
        let avg = match (
            stats.avg_kills,
            stats.avg_deaths,
            stats.avg_assists,
            stats.avg_total_damage,
        ) {
            (Some(kills), Some(deaths), Some(assists), Some(damage)) => {
                format!(
                    " - {:.1}/{:.1}/{:.1} - {:.0} avg dmg",
                    kills, deaths, assists, damage
                )
            }
            (Some(kills), Some(deaths), Some(assists), None) => {
                format!(" - {:.1}/{:.1}/{:.1}", kills, deaths, assists)
            }
            _ => String::new(),
        };
        format!(
            "<@{}> - {} rating - {}W/{}L - {:.1}% WR{}",
            stats.discord_user_id,
            stats.rating,
            stats.wins,
            stats.losses,
            stats.win_rate * 100.0,
            avg
        )
    }

    fn match_lines(game: &GameRow, links: &[MatchLinkRow]) -> String {
        let status = game
            .status()
            .map_or_else(|_| game.status.clone(), |status| status.as_str().to_owned());
        let mut lines = vec![
            format!("Local game: `{}`", game.game_id),
            format!("Mode: `{}`", game.mode),
            format!("Status: `{status}`"),
            format!(
                "Riot match: {}",
                game.riot_match_id
                    .as_ref()
                    .map_or("pending".to_owned(), |match_id| format!("`{match_id}`"))
            ),
        ];
        if links.is_empty() {
            lines.push("Match-V5 stats: pending".to_owned());
        } else {
            for link in links.iter().take(3) {
                lines.push(format!(
                    "`{}` - {} - {} participant row(s)",
                    link.riot_match_id, link.data_source, link.participant_count
                ));
            }
            if links.len() > 3 {
                lines.push(format!("{} more linked match(es)", links.len() - 3));
            }
        }
        let mut value = lines.join("\n");
        truncate_embed_value(&mut value);
        value
    }

    fn command_initial_ephemeral(command: &DiscordCommand) -> bool {
        !matches!(
            command,
            DiscordCommand::Create(_)
                | DiscordCommand::Add(_)
                | DiscordCommand::Next
                | DiscordCommand::Winner(_)
                | DiscordCommand::Game(_)
                | DiscordCommand::Result { .. }
                | DiscordCommand::Results(_)
                | DiscordCommand::Finish(_)
                | DiscordCommand::LinkMatch(_)
                | DiscordCommand::End { .. }
                | DiscordCommand::Leaderboards { .. }
        )
    }

    fn parse_winner_button_id(custom_id: &str) -> Option<(GameId, TeamSide)> {
        let mut parts = custom_id.split(':');
        if parts.next()? != "rsso" || parts.next()? != "winner" {
            return None;
        }
        let game_id = GameId::new(parts.next()?);
        let winner = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some((game_id, winner))
    }

    fn now() -> i64 {
        Date::now().as_millis() as i64
    }

    #[derive(Debug, Clone)]
    struct WorkerRng {
        fallback: u64,
    }

    impl WorkerRng {
        fn new() -> Self {
            Self {
                fallback: Date::now().as_millis(),
            }
        }
    }

    impl Rng for WorkerRng {
        fn next_u32(&mut self) -> u32 {
            let mut bytes = [0_u8; 4];
            if getrandom::getrandom(&mut bytes).is_ok() {
                return u32::from_le_bytes(bytes);
            }
            self.fallback ^= self.fallback << 13;
            self.fallback ^= self.fallback >> 7;
            self.fallback ^= self.fallback << 17;
            (self.fallback & u64::from(u32::MAX)) as u32
        }
    }

    #[derive(Debug, Serialize)]
    struct DiscordWebhookMessage {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        embeds: Option<Vec<DiscordEmbed>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        components: Option<Vec<DiscordActionRow>>,
    }

    impl DiscordWebhookMessage {
        fn plain(content: impl Into<String>) -> Self {
            Self {
                content: content.into(),
                embeds: None,
                components: None,
            }
        }

        fn clear_rich_state(mut self) -> Self {
            self.embeds = Some(Vec::new());
            self.components = Some(Vec::new());
            self
        }
    }

    #[derive(Debug, Serialize)]
    struct DiscordEmbed {
        title: String,
        description: String,
        color: u32,
        fields: Vec<DiscordEmbedField>,
        footer: DiscordEmbedFooter,
    }

    #[derive(Debug, Serialize)]
    struct DiscordEmbedField {
        name: String,
        value: String,
        inline: bool,
    }

    #[derive(Debug, Serialize)]
    struct DiscordEmbedFooter {
        text: String,
    }

    #[derive(Debug, Serialize)]
    struct DiscordActionRow {
        #[serde(rename = "type")]
        kind: u8,
        components: Vec<DiscordButton>,
    }

    #[derive(Debug, Serialize)]
    struct DiscordButton {
        #[serde(rename = "type")]
        kind: u8,
        style: u8,
        label: String,
        custom_id: String,
    }

    fn discord_message_from_response(response: &CommandResponse) -> DiscordWebhookMessage {
        if let Some(stats_overview_card) = response.stats_overview_card.as_ref() {
            return DiscordWebhookMessage {
                content: "Stats for everyone".to_owned(),
                embeds: Some(vec![stats_overview_embed(stats_overview_card)]),
                components: None,
            };
        }

        if let Some(stats_card) = response.stats_card.as_ref() {
            return DiscordWebhookMessage {
                content: format!("Stats for <@{}>", stats_card.discord_user_id.as_str()),
                embeds: Some(vec![stats_embed(stats_card)]),
                components: None,
            };
        }

        if let Some(team_card) = response.team_card.as_ref() {
            return DiscordWebhookMessage {
                content: response.content.clone(),
                embeds: Some(vec![team_embed(team_card)]),
                components: Some(vec![winner_button_row(&team_card.game_id)]),
            };
        }

        DiscordWebhookMessage::plain(response.content.clone())
    }

    fn stats_overview_embed(stats_overview_card: &StatsOverviewCard) -> DiscordEmbed {
        DiscordEmbed {
            title: "In-house stats".to_owned(),
            description: format!("Mode: {}", stats_overview_card.mode_label),
            color: 0x5865f2,
            fields: stats_overview_fields(stats_overview_card),
            footer: DiscordEmbedFooter {
                text: format!("Showing {} player(s).", stats_overview_card.rows.len()),
            },
        }
    }

    fn stats_overview_fields(stats_overview_card: &StatsOverviewCard) -> Vec<DiscordEmbedField> {
        const OVERVIEW_FIELD_BUDGET: usize = 5_000;

        if stats_overview_card.rows.is_empty() {
            return vec![DiscordEmbedField {
                name: "Players".to_owned(),
                value: "none yet".to_owned(),
                inline: false,
            }];
        }

        let mut fields = Vec::new();
        let mut lines = Vec::new();
        let mut current_len = 0_usize;
        let mut used_len = 0_usize;
        for (index, row) in stats_overview_card.rows.iter().enumerate() {
            let mut line = stats_overview_line(row);
            truncate_embed_value(&mut line);
            let separator_len = usize::from(!lines.is_empty());
            if used_len + separator_len + line.len() > OVERVIEW_FIELD_BUDGET {
                let remaining = stats_overview_card.rows.len() - index;
                let overflow =
                    format!("{remaining} more player(s) omitted by Discord embed limits.");
                if current_len + separator_len + overflow.len() <= 1024 {
                    lines.push(overflow);
                }
                break;
            }
            if current_len + separator_len + line.len() > 1024 && !lines.is_empty() {
                fields.push(DiscordEmbedField {
                    name: stats_overview_field_name(fields.len()),
                    value: lines.join("\n"),
                    inline: false,
                });
                lines.clear();
                current_len = 0;
                if fields.len() == 24 {
                    break;
                }
            }
            current_len += usize::from(!lines.is_empty()) + line.len();
            used_len += separator_len + line.len();
            lines.push(line);
        }
        if !lines.is_empty() && fields.len() < 25 {
            fields.push(DiscordEmbedField {
                name: stats_overview_field_name(fields.len()),
                value: lines.join("\n"),
                inline: false,
            });
        }
        fields
    }

    fn stats_overview_field_name(index: usize) -> String {
        if index == 0 {
            "Players".to_owned()
        } else {
            format!("Players {}", index + 1)
        }
    }

    fn stats_overview_line(row: &rsso_handlers::StatsOverviewRow) -> String {
        let mut line = format!(
            "{}. <@{}> - `{}` - {}W/{}L - {} WR - {} rating",
            row.rank,
            row.discord_user_id.as_str(),
            row.riot_id,
            row.wins,
            row.losses,
            row.win_rate,
            row.rating
        );
        if let Some(kda) = row.kda.as_ref() {
            line.push_str(&format!(" - {kda}"));
        }
        if let Some(damage) = row.average_damage.as_ref() {
            line.push_str(&format!(" - {damage}"));
        }
        line
    }

    fn stats_embed(stats_card: &StatsCard) -> DiscordEmbed {
        DiscordEmbed {
            title: format!("Stats for {}", stats_card.riot_id),
            description: format!(
                "<@{}> - {}",
                stats_card.discord_user_id.as_str(),
                stats_card.mode_label
            ),
            color: stats_color(stats_card),
            fields: vec![
                DiscordEmbedField {
                    name: "Player".to_owned(),
                    value: format!(
                        "<@{}>\n`{}`\nMode: `{}`",
                        stats_card.discord_user_id.as_str(),
                        stats_card.riot_id,
                        stats_card.mode_label
                    ),
                    inline: true,
                },
                DiscordEmbedField {
                    name: "Record".to_owned(),
                    value: format!(
                        "{}W / {}L\n{} games\n{} WR\n{} rating",
                        stats_card.wins,
                        stats_card.losses,
                        stats_card.games_total,
                        stats_card.win_rate,
                        stats_card.rating
                    ),
                    inline: true,
                },
                DiscordEmbedField {
                    name: "Averages".to_owned(),
                    value: stats_averages(stats_card),
                    inline: true,
                },
                DiscordEmbedField {
                    name: "Most won with".to_owned(),
                    value: embed_lines(&stats_card.most_won_with, "none yet"),
                    inline: false,
                },
                DiscordEmbedField {
                    name: "Most lost with".to_owned(),
                    value: embed_lines(&stats_card.most_lost_with, "none yet"),
                    inline: false,
                },
            ],
            footer: DiscordEmbedFooter {
                text: "Stats update after /winner or hydrated Match-V5 rows.".to_owned(),
            },
        }
    }

    fn stats_color(stats_card: &StatsCard) -> u32 {
        if stats_card.games_total == 0 {
            0xfee75c
        } else if stats_card.wins >= stats_card.losses {
            0x57f287
        } else {
            0xed4245
        }
    }

    fn stats_averages(stats_card: &StatsCard) -> String {
        let mut lines = Vec::new();
        if let Some(kda) = stats_card.kda.as_ref() {
            lines.push(kda.clone());
        }
        if let Some(damage) = stats_card.average_damage.as_ref() {
            lines.push(damage.clone());
        }
        embed_lines(&lines, "No Match-V5 averages yet.")
    }

    fn embed_lines(lines: &[String], empty: &str) -> String {
        if lines.is_empty() {
            return empty.to_owned();
        }
        let mut value = lines.join("\n");
        truncate_embed_value(&mut value);
        value
    }

    fn truncate_embed_value(value: &mut String) {
        const DISCORD_EMBED_FIELD_LIMIT: usize = 1024;
        const ELLIPSIS_LEN: usize = 3;
        if value.len() <= DISCORD_EMBED_FIELD_LIMIT {
            return;
        }
        let mut boundary = DISCORD_EMBED_FIELD_LIMIT - ELLIPSIS_LEN;
        while !value.is_char_boundary(boundary) {
            boundary -= 1;
        }
        value.truncate(boundary);
        value.push_str("...");
    }

    fn team_embed(team_card: &TeamCard) -> DiscordEmbed {
        DiscordEmbed {
            title: format!("In-house {}", team_card.game_id),
            description: format!("Mode: {}", team_card.mode.as_str()),
            color: 0x5865f2,
            fields: vec![
                DiscordEmbedField {
                    name: "🔴 Red team".to_owned(),
                    value: mentions_or_empty(&team_card.red),
                    inline: true,
                },
                DiscordEmbedField {
                    name: "🔵 Blue team".to_owned(),
                    value: mentions_or_empty(&team_card.blue),
                    inline: true,
                },
            ],
            footer: DiscordEmbedFooter {
                text: "Use the buttons when the game ends.".to_owned(),
            },
        }
    }

    fn mentions_or_empty(users: &[rsso_domain::DiscordUserId]) -> String {
        if users.is_empty() {
            return "none".to_owned();
        }
        users
            .iter()
            .map(|user| format!("<@{}>", user.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn winner_button_row(game_id: &GameId) -> DiscordActionRow {
        DiscordActionRow {
            kind: 1,
            components: vec![
                DiscordButton {
                    kind: 2,
                    style: 4,
                    label: "Red wins".to_owned(),
                    custom_id: format!("rsso:winner:{game_id}:red"),
                },
                DiscordButton {
                    kind: 2,
                    style: 1,
                    label: "Blue wins".to_owned(),
                    custom_id: format!("rsso:winner:{game_id}:blue"),
                },
            ],
        }
    }

    async fn edit_original_response(
        application_id: &str,
        token: &str,
        message: &DiscordWebhookMessage,
    ) -> Result<()> {
        let url = format!(
            "https://discord.com/api/v10/webhooks/{application_id}/{token}/messages/@original"
        );
        let headers = Headers::new();
        headers.set("Content-Type", "application/json")?;
        let body = serde_json::to_string(message)
            .map_err(|err| worker::Error::RustError(format!("discord message json: {err}")))?;
        let mut init = RequestInit::new();
        init.with_method(Method::Patch)
            .with_headers(headers)
            .with_body(Some(wasm_bindgen::JsValue::from_str(&body)));
        let request = Request::new_with_init(&url, &init)?;
        let response = Fetch::Request(request).send().await?;
        if (200..300).contains(&response.status_code()) {
            Ok(())
        } else {
            Err(worker::Error::RustError(format!(
                "discord returned HTTP {}",
                response.status_code()
            )))
        }
    }

    #[derive(Debug, Clone)]
    struct WorkerRiotAccountResolver {
        api_key: Option<String>,
        regional_route: String,
        platform_route: String,
    }

    impl WorkerRiotAccountResolver {
        fn from_env(env: &Env) -> Self {
            Self {
                api_key: env
                    .secret("RIOT_API_KEY")
                    .ok()
                    .map(|secret| secret.to_string()),
                regional_route: env
                    .var("RIOT_REGIONAL_DEFAULT")
                    .map(|value| value.to_string())
                    .unwrap_or_else(|_| "AMERICAS".to_owned()),
                platform_route: env
                    .var("RIOT_PLATFORM_DEFAULT")
                    .map(|value| value.to_string())
                    .unwrap_or_else(|_| "NA1".to_owned()),
            }
        }
    }

    #[async_trait(?Send)]
    impl RiotAccountResolver for WorkerRiotAccountResolver {
        async fn resolve_riot_id(
            &self,
            riot_id: &str,
        ) -> std::result::Result<Option<ResolvedRiotAccount>, HandlerError> {
            let Some(api_key) = self.api_key.as_deref() else {
                return Ok(None);
            };
            let parsed = rsso_riot::parse_riot_id(riot_id)
                .map_err(|err| HandlerError::UserFacing(format!("Invalid Riot ID: {err}")))?;
            let url =
                account_by_riot_id_url(&self.regional_route, &parsed.game_name, &parsed.tag_line)
                    .map_err(|err| HandlerError::UserFacing(format!("Riot route error: {err}")))?;
            let headers = Headers::new();
            headers
                .set("X-Riot-Token", api_key)
                .map_err(|err| HandlerError::UserFacing(format!("Riot header error: {err}")))?;
            let mut init = RequestInit::new();
            init.with_method(Method::Get).with_headers(headers);
            let request = Request::new_with_init(&url, &init)
                .map_err(|err| HandlerError::UserFacing(format!("Riot request error: {err}")))?;
            let mut response = Fetch::Request(request)
                .send()
                .await
                .map_err(|err| HandlerError::UserFacing(format!("Riot request failed: {err}")))?;
            match response.status_code() {
                200 => {
                    let account =
                        response
                            .json::<rsso_riot::AccountDto>()
                            .await
                            .map_err(|err| {
                                HandlerError::UserFacing(format!(
                                    "Riot account parse failed: {err}"
                                ))
                            })?;
                    Ok(Some(ResolvedRiotAccount {
                        puuid: account.puuid,
                        game_name: account.game_name,
                        tag_line: account.tag_line,
                    }))
                }
                404 => Ok(None),
                status => Err(HandlerError::UserFacing(format!(
                    "Riot account lookup failed with HTTP {status}"
                ))),
            }
        }
    }

    #[async_trait(?Send)]
    impl RiotMatchResolver for WorkerRiotAccountResolver {
        async fn resolve_match(
            &self,
            riot_match_id: &str,
        ) -> std::result::Result<Option<ResolvedRiotMatch>, HandlerError> {
            let Some(api_key) = self.api_key.as_deref() else {
                return Ok(None);
            };
            let regional_route = parse_riot_match_id(riot_match_id)
                .ok()
                .and_then(|match_id| {
                    match_regional_route_for_platform(&match_id.platform)
                        .ok()
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| self.regional_route.clone());
            let url = match_detail_url(&regional_route, riot_match_id)
                .map_err(|err| HandlerError::UserFacing(format!("Riot route error: {err}")))?;
            let headers = Headers::new();
            headers
                .set("X-Riot-Token", api_key)
                .map_err(|err| HandlerError::UserFacing(format!("Riot header error: {err}")))?;
            let mut init = RequestInit::new();
            init.with_method(Method::Get).with_headers(headers);
            let request = Request::new_with_init(&url, &init)
                .map_err(|err| HandlerError::UserFacing(format!("Riot request error: {err}")))?;
            let mut response = Fetch::Request(request)
                .send()
                .await
                .map_err(|err| HandlerError::UserFacing(format!("Riot request failed: {err}")))?;
            match response.status_code() {
                200 => {
                    let match_dto =
                        response
                            .json::<rsso_riot::MatchDto>()
                            .await
                            .map_err(|err| {
                                HandlerError::UserFacing(format!("Riot match parse failed: {err}"))
                            })?;
                    let payload_json = serde_json::to_string(&match_dto).ok();
                    let participants = match_dto
                        .info
                        .participants
                        .iter()
                        .map(|participant| ResolvedRiotParticipant {
                            puuid: participant.puuid.clone(),
                            team_id: participant.team_id,
                            champion_id: participant.champion_id,
                            champion_name: participant.champion_name.clone(),
                            win: participant.win,
                            kills: participant.kills,
                            deaths: participant.deaths,
                            assists: participant.assists,
                            total_damage: participant.total_damage_dealt_to_champions,
                            gold_earned: participant.gold_earned,
                            total_minions: Some(
                                participant.total_minions_killed.unwrap_or(0)
                                    + participant.neutral_minions_killed.unwrap_or(0),
                            ),
                            vision_score: participant.vision_score,
                            raw_json: serde_json::to_string(participant).ok(),
                        })
                        .collect();
                    Ok(Some(ResolvedRiotMatch {
                        riot_match_id: match_dto.metadata.match_id,
                        queue_id: match_dto.info.queue_id,
                        map_id: match_dto.info.map_id,
                        game_mode: match_dto.info.game_mode,
                        game_type: match_dto.info.game_type,
                        payload_json,
                        participants,
                    }))
                }
                403 | 404 => Ok(None),
                status => Err(HandlerError::UserFacing(format!(
                    "Riot match lookup failed with HTTP {status}"
                ))),
            }
        }
    }

    #[async_trait(?Send)]
    impl LiveGameProbe for WorkerRiotAccountResolver {
        async fn active_game_by_puuid(
            &self,
            puuid: &str,
        ) -> std::result::Result<LiveGameStatus, String> {
            let Some(api_key) = self.api_key.as_deref() else {
                return Err("RIOT_API_KEY is not configured".to_owned());
            };
            let url = active_game_url(&self.platform_route, puuid)
                .map_err(|err| format!("Riot route error: {err}"))?;
            let headers = Headers::new();
            headers
                .set("X-Riot-Token", api_key)
                .map_err(|err| format!("Riot header error: {err}"))?;
            let mut init = RequestInit::new();
            init.with_method(Method::Get).with_headers(headers);
            let request = Request::new_with_init(&url, &init)
                .map_err(|err| format!("Riot request error: {err}"))?;
            let mut response = Fetch::Request(request)
                .send()
                .await
                .map_err(|err| format!("Riot request failed: {err}"))?;
            match response.status_code() {
                200 => {
                    let game = response
                        .json::<rsso_riot::CurrentGameInfoDto>()
                        .await
                        .map_err(|err| format!("Riot spectator parse failed: {err}"))?;
                    let platform_id = game
                        .platform_id
                        .clone()
                        .unwrap_or_else(|| self.platform_route.to_ascii_uppercase());
                    let participant_puuids = game
                        .participants
                        .iter()
                        .filter_map(|participant| participant.puuid.clone())
                        .collect();
                    Ok(LiveGameStatus::Live(LiveGame {
                        riot_match_id: Some(format!("{platform_id}_{}", game.game_id)),
                        queue_id: game.game_queue_config_id,
                        map_id: game.map_id,
                        game_mode: game.game_mode,
                        game_type: game.game_type,
                        participant_puuids,
                    }))
                }
                404 => Ok(LiveGameStatus::NotFound),
                status => Err(format!("Riot spectator lookup failed with HTTP {status}")),
            }
        }
    }

    #[async_trait(?Send)]
    impl FinishedMatchProbe for WorkerRiotAccountResolver {
        async fn finished_match(
            &self,
            riot_match_id: &str,
        ) -> std::result::Result<Option<FinishedMatch>, String> {
            let resolved = self
                .resolve_match(riot_match_id)
                .await
                .map_err(|error| error.to_string())?;
            Ok(resolved.map(|match_detail| FinishedMatch {
                riot_match_id: match_detail.riot_match_id,
                queue_id: match_detail.queue_id,
                map_id: match_detail.map_id,
                game_mode: match_detail.game_mode,
                game_type: match_detail.game_type,
                payload_json: match_detail.payload_json,
                participants: match_detail
                    .participants
                    .into_iter()
                    .map(|participant| FinishedParticipant {
                        puuid: participant.puuid,
                        team_id: participant.team_id,
                        champion_id: participant.champion_id,
                        champion_name: participant.champion_name,
                        win: participant.win,
                        kills: participant.kills,
                        deaths: participant.deaths,
                        assists: participant.assists,
                        total_damage: participant.total_damage,
                        gold_earned: participant.gold_earned,
                        total_minions: participant.total_minions,
                        vision_score: participant.vision_score,
                        raw_json: participant.raw_json,
                    })
                    .collect(),
            }))
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn host_build_placeholder() -> &'static str {
    "rsso-worker is meant to run on wasm32-unknown-unknown"
}
