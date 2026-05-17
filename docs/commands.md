# Discord Command Runbook

This bot uses Discord guild slash commands backed by a Cloudflare Worker and D1. The registered command surface is intentionally small: `/register-summoners`, `/create`, `/add`, `/winner`, and `/stats`.

## Active Workflow

1. Register each player:

```text
/register-summoners riot_id:Cyracen#NA1
```

`riot_id` must use Riot ID format, `GameName#TAG`. With `RIOT_API_KEY` configured, the bot stores the Riot PUUID. If Riot lookup is unavailable, the command still stores a trusted claim so the local in-house flow can run.

2. Create and randomize one game:

```text
/create user_1:@vu user_2:@chongly user_3:@cyracen user_4:@tanguan
/create user_1:@vu user_2:@chongly mode:ARAM
```

`/create` requires an even roster between 2 and 10 players. It creates a 4-digit local `game_id`, defaults to `ARAM: Mayhem`, and returns Discord mentions for each team:

```text
Created with gameId 1283 (aram_mayhem)
Red team: @vu @cyracen
Blue team: @tanguan @chongly
```

3. Add late players if needed:

```text
/add user_1:@late
/add user_1:@late user_2:@also_late
```

`/add` defaults to the current open game. When the resulting roster is even, it randomizes teams again. When the roster is odd, it keeps the game open and asks for one more player.

4. Mark the winner:

```text
/winner game_id:1283 winner:Red
```

`/winner` finalizes the game immediately, updates W/L counters, updates ratings, and closes the singleton open game.

5. Read stats:

```text
/stats
/stats name:Cyracen
/stats name:Cyracen#NA1
/stats user:@cyracen
/stats user:@cyracen mode:ARAM
```

`/stats name:` resolves against registered Riot game names. The response includes W/L, total games, win rate, rating, and teammate rows for most wins and losses together.

## Deferred Commands

The code still has parser and handler support for these commands, but `rsso-cli` no longer registers them in Discord:

- `/game mode user_1 ... user_10`
- `/randomize game_id`
- `/result game_id winner`
- `/results game_id? winner? riot_match_id? region?`
- `/finish riot_match_id game_id? winner? region?`
- `/hydrate game_id? riot_match_id? region?`
- `/link-match riot_match_id game_id? region?`
- `/end game_id`
- `/status game_id?`
- `/leaderboards mode?`
- `/analysis mode?`

`/result` was the old two-step manual path: report a winner, then `/end` finalized later. `/results` was the Riot-aware path: report a winner and optionally attach Match-V5 data. `/winner` replaces the local need for both by finalizing immediately. The Riot-aware paths can come back later under cleaner names once RSO-backed ingestion is worth exposing.

## Operator Commands

Print the registered command manifest:

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
curl -fsS https://rsso.aarnphm.workers.dev/riot.txt >/dev/null
```

`wrangler.toml` reads the ignored repo-root `riot.txt` during the custom Worker build and embeds it into the deployed `/riot.txt` route for Riot domain verification. Keep the file local; it should not be committed.

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
