# Discord Command Runbook

This bot uses Discord guild slash commands backed by a Cloudflare Worker and D1. Commands are registered through `rsso-cli`, then handled by the deployed Worker at the Discord interaction endpoint.

## Server Workflow

1. Register each player:

```text
/register-summoners riot_id:Cyracen#NA1
```

`riot_id` must use Riot ID format, `GameName#TAG`. With `RIOT_API_KEY` configured, the bot stores the Riot PUUID. If the key is absent or Riot returns 404, the claim is stored without a PUUID so the in-house can still run.

2. Create a game:

```text
/game mode:ARAM user_1:@a user_2:@b user_3:@c user_4:@d
```

The bot creates one local `game_id`, for example `g_abc123`, and stores the local roster. Even player counts are randomized immediately. Odd player counts stay in `lobby` until another player is added.

3. Add a missing player when needed:

```text
/add game_id:g_abc123 user:@e
/randomize game_id:g_abc123
```

`/add` only works before the game is locked by live-game tracking. `/randomize` can be rerun while the game is still in `lobby` or `randomized`.

4. Check the local game to Riot match link:

```text
/status
/status game_id:g_abc123
```

`/status` without `game_id` reads the current open game for the guild. The Riot match id is pending until the 3-minute scheduled worker sees the game through Spectator-V5 or someone links an id manually.

5. Report the result:

```text
/results game_id:g_abc123 winner:Blue
/results game_id:g_abc123 riot_match_id:NA1_4901234567
/results game_id:g_abc123 riot_match_id:5561312307 region:NA
/results game_id:g_abc123 winner:Blue riot_match_id:NA1_4901234567
```

`/results` can be used after teams are randomized, while the game is `randomized`, `ingame`, `ambiguous`, or already `reported`.

When `riot_match_id` is present, the bot tries Match-V5. If Riot returns the match, the bot validates the mode, validates that registered roster PUUIDs appear in the match, stores `matches` and `match_participants`, and derives the winner when possible. If Riot does not return the match yet, the bot still links the supplied Riot match id to the local game as manual data.

`riot_match_id` accepts either a full Riot match id such as `NA1_5561312307` or a numeric Riot game id such as `5561312307`. Numeric game ids default to `region:NA`, which becomes `NA1_5561312307`. Pass `region:EUW`, `region:KR`, or another supported region when the match is outside NA.

Supported `region` values are `NA`, `BR`, `LAN`, `LAS`, `EUW`, `EUNE`, `KR`, `JP`, `OCE`, `TR`, `RU`, `PH`, `SG`, `TH`, `TW`, and `VN`.

When `winner` and `riot_match_id` are both present, the game finalizes immediately. When `winner` is present without `riot_match_id`, the game moves to `reported`; ratings and W/L counters stay unchanged until `/end`.

6. Finalize a reported game:

```text
/end game_id:g_abc123
```

`/end` finalizes the reported winner, closes the local game, and updates ratings plus W/L counters. For now, only the creator of the local game can run `/end`.

7. Finalize directly from Riot match id:

```text
/finish riot_match_id:NA1_4901234567
/finish riot_match_id:5561312307 region:NA
/finish riot_match_id:NA1_4901234567 game_id:g_abc123 winner:Blue
```

`/finish` is the direct finalization path. Use it when the Riot match id is known and the game should close immediately. If `game_id` is omitted, the bot uses the current open game for the guild. If Match-V5 cannot derive a winner or returns 403, pass `winner`; if the game was already reported, `/finish` can reuse the stored winner. If no winner is available, `/finish` still links the Riot match id and tells you to rerun it with `winner`.

8. Read stats:

```text
/stats
/stats user:@a
/stats user:@a mode:ARAM
/leaderboards
/leaderboards mode:ARAM: Mayhem
/analysis
```

Stats and leaderboards count finalized games. Champion and damage averages depend on Riot match data in `match_participants`.

## Match Linking Rules

- `/game` creates the local `game_id`.
- The scheduled Worker runs every 3 minutes and links `game_id` to `riot_match_id` when Spectator-V5 sees a registered PUUID in an active game.
- `/results riot_match_id:... winner:...` links a Riot match id after the fact and finalizes ratings.
- `/finish riot_match_id:...` links the Riot match id and finalizes the game in one command.
- `/status` shows the current link state.

## Operator Commands

Print the command manifest:

```sh
cargo run -p rsso-cli -- discord commands-json
```

Register guild commands:

```sh
cargo run -p rsso-cli -- discord register-commands \
  --app-id "$DISCORD_APP_ID" \
  --guild-id "$DISCORD_GUILD_ID" \
  --bot-token "$DISCORD_BOT_TOKEN"
```

The current development guild can also be registered explicitly:

```sh
DISCORD_GUILD_ID=1444542180361240689 cargo run -p rsso-cli -- discord register-commands
```

Deploy the Worker:

```sh
npx wrangler deploy
```

Apply D1 migrations:

```sh
npx wrangler d1 migrations apply DB --local
npx wrangler d1 migrations apply DB --remote
```

Probe Riot account lookup from the CLI:

```sh
cargo run -p rsso-cli -- riot probe-account "GameName#TAG" --api-key "$RIOT_API_KEY"
```

## Expected States

- `lobby`: local roster exists, teams are not stable yet.
- `randomized`: teams are assigned and the lobby can be played.
- `ingame`: Spectator-V5 found the active Riot game.
- `reported`: a winner is known, ratings are not finalized yet.
- `ambiguous`: Riot data conflicted with local expectations and needs manual handling.
- `finalized`: ratings and W/L counters are closed.
- `cancelled`: the game was closed without a result.
