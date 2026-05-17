#[cfg(target_arch = "wasm32")]
mod worker_entry {
    use async_trait::async_trait;
    use rsso_cron::{
        FinishedMatch, FinishedMatchProbe, FinishedParticipant, LiveGame, LiveGameProbe,
        LiveGameStatus,
    };
    use rsso_discord::interaction::InteractionType;
    use rsso_discord::response::{deferred_response, message_response, pong_response};
    use rsso_discord::{parse_command, verify::verify_discord_request, Interaction};
    use rsso_domain::{parse_riot_match_id, Rng};
    use rsso_handlers::{
        handle_command_with_resolver, CommandContext, HandlerError, ResolvedRiotAccount,
        ResolvedRiotMatch, ResolvedRiotParticipant, RiotAccountResolver, RiotMatchResolver,
    };
    use rsso_riot::routing::{
        account_by_riot_id_url, active_game_url, match_detail_url,
        match_regional_route_for_platform,
    };
    use rsso_storage::d1::D1Storage;
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
        let application_id = interaction.application_id;
        let token = interaction.token;
        let background_env = env.clone();
        ctx.wait_until(async move {
            let content = match background_env.d1("DB") {
                Ok(db) => {
                    let storage = D1Storage::new(db);
                    let resolver = WorkerRiotAccountResolver::from_env(&background_env);
                    let mut rng = WorkerRng::new();
                    match handle_command_with_resolver(
                        &storage, &resolver, context, command, &mut rng,
                    )
                    .await
                    {
                        Ok(content) => content,
                        Err(error) => format!("{error}"),
                    }
                }
                Err(error) => format!("D1 binding error: {error}"),
            };
            if let Err(error) = edit_original_response(&application_id, &token, &content).await {
                worker::console_error!("discord followup failed: {error}");
            }
        });
        Response::from_json(&deferred_response(true))
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

    async fn edit_original_response(
        application_id: &str,
        token: &str,
        content: &str,
    ) -> Result<()> {
        let url = format!(
            "https://discord.com/api/v10/webhooks/{application_id}/{token}/messages/@original"
        );
        let headers = Headers::new();
        headers.set("Content-Type", "application/json")?;
        let body = serde_json::json!({ "content": content });
        let mut init = RequestInit::new();
        init.with_method(Method::Patch)
            .with_headers(headers)
            .with_body(Some(wasm_bindgen::JsValue::from_str(&body.to_string())));
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
