# Discord Command Runbook

This bot uses Discord guild slash commands backed by a Cloudflare Worker and D1. The registered command surface is intentionally small: `/register-summoners`, `/create`, `/add`, `/next`, `/winner`, `/stats`, and `/leaderboard`.

## Active Workflow

1. Register each player when their Riot ID is known:

```text
/register-summoners riot_id:Cyracen#NA1
```

`riot_id` must use Riot ID format, `GameName#TAG`. With `RIOT_API_KEY` configured, the bot stores the Riot PUUID. If Riot lookup is unavailable, the command still stores a trusted claim so the local in-house flow can run. Players can join `/create` and `/add` before this step; the bot keeps a pending Discord-only row and `/register-summoners` fills in the Riot identity later.

2. Create and randomize one game:

```text
/create users:"@vu @chongly @cyracen @tanguan"
/create users:"@vu @chongly" mode:ARAM
```

`users:` is a text field with Discord mentions. `/create` requires an even roster between 2 and 10 Discord users. It creates a `g_...` local `game_id`, creates pending player rows for anyone who has not registered a Riot ID yet, defaults to `ARAM: Mayhem`, and posts a public team card with winner buttons:

```text
Created game g_SRLV8AbxYO (aram_mayhem)
Embed:
  Red team: @vu @cyracen
  Blue team: @tanguan @chongly
Buttons:
  Red wins
  Blue wins
```

3. Add late players if needed:

```text
/add user_1:@late
/add user_1:@late user_2:@also_late
```

`/add` defaults to the current open game and accepts multiple users. When the resulting roster is even, it randomizes teams again and posts a fresh public team card with winner buttons. When the roster is odd, it keeps the game open and asks for one more player.

4. Start the next rotation:

```text
/next
```

`/next` creates a new open game from the latest completed roster, keeps the same mode, and advances to the next balanced team split. It refuses to run while another game is open.

5. Mark the winner:

```text
/winner game_id:g_SRLV8AbxYO side:Red
```

`/winner` finalizes the game immediately, updates W/L counters, updates ratings, and closes the singleton open game. The Red/Blue winner buttons on the team card call the same finalization path, then replace the original team card with a result card showing current match state, linked Riot match status, and post-game team records.

6. Read stats:

```text
/stats
/stats mode:ARAM
/stats name:Cyracen
/stats name:Cyracen#NA1
/stats user:@cyracen
/stats user:@cyracen mode:ARAM
```

With no `user` or `name`, `/stats` shows everyone in the guild. `mode:` scopes that overview to one mode. `user:` or `name:` shows a single-player stat card. `name:` resolves against registered Riot game names. The responses are ephemeral stat cards with Discord mentions, Riot IDs when known, mode scope, W/L, total games, win rate, rating, Match-V5 averages when available, and teammate rows for the strongest win/loss pairings.

7. Show the public leaderboard:

```text
/leaderboard
/leaderboard mode:ARAM
```

`/leaderboard` posts the top in-house rows publicly with rating, W/L, win rate, and Match-V5 averages when available. Use `/stats` for the larger private everyone view.

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
- `/leaderboards mode?` (legacy parser alias for `/leaderboard`)
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
