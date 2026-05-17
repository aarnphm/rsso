# rsso

Discord-native League of Legends in-house tracking.

The first version is API-key-first: Discord users claim a Riot ID, the bot tracks one active game per guild, teams are randomized with a Fisher-Yates shuffle, and stats are stored by local game/mode. RSO and Tournament API support are later upgrades that should fill the same schema instead of replacing it.

The repo uses a Cargo workspace under `crates/`, `wrangler` for Cloudflare Workers, and D1 migrations under `migrations/`.

## Shape

- `crates/rsso-domain`: pure IDs, game modes, state transitions, ELO, team shuffling.
- `crates/rsso-discord`: Discord interaction DTOs, slash-command parsing, Ed25519 verification. Native builds use `serenity` for endpoint verification; wasm builds use a small direct verifier because Serenity's async stack is not Worker-shaped.
- `crates/rsso-riot`: Riot URL builders and DTOs for Account-V1, Spectator-V5, and Match-V5.
- `crates/rsso-storage`: the D1 repository and row models.
- `crates/rsso-handlers`: command behavior over storage, Riot lookup, and randomness.
- `crates/rsso-cron`: the 3-minute active-game poller.
- `crates/rsso-worker`: Cloudflare Worker entrypoints for Discord interactions and scheduled events.
- `crates/rsso-cli`: local helpers for Discord command JSON and guild command registration.

## Commands

- `/register-summoners riot_id`
- `/create users mode?`
- `/add user_1 ... user_10 game_id?`
- `/next`
- `/winner game_id side`
- `/stats name? user? mode?`
- `/leaderboard mode?`

The Discord command manifest intentionally registers only those seven commands. The code still has parser and handler paths for `game`, `randomize`, `result`, `results`, `finish`, `hydrate`, `link-match`, `end`, `status`, legacy `leaderboards`, and `analysis`, because those are useful later once RSO-backed Riot match ingestion becomes a real product flow instead of slash-command clutter.

## Riot API Path

The local-first flow is `/create`, optionally `/add`, `/winner`, `/next`, `/register-summoners` when Riot IDs are known, then `/stats` or `/leaderboard`. `/create` accepts one `users` text field containing Discord mentions, assigns a `g_...` local game id, creates pending Discord-only player rows for anyone without a Riot ID yet, randomizes even teams immediately, posts a public team card, and defaults to `ARAM: Mayhem` unless `mode` is passed. `/add` attaches extra players to the current open game and posts a fresh team card when the resulting roster is even. `/next` copies the latest completed roster and advances to the next balanced side split, so repeated in-house games can rotate through the roster without retyping every player. The team card includes Red/Blue winner buttons, which call the same finalization path as `/winner game_id side`. `/winner` finalizes W/L and ratings without waiting on Riot Match-V5, which matters because many custom-match backfill paths need RSO access. `/stats` with no `user` or `name` returns a private everyone card; `/leaderboard` posts the public top rows; `/stats user:` or `/stats name:` returns a single-player rich card with Discord mentions, Riot ID when known, mode scope, record, rating, Match-V5 averages, and teammate pairings.

`/register-summoners` accepts a Riot ID shaped as `GameName#TAG` and resolves it through Account-V1 when `RIOT_API_KEY` is configured. If Riot lookup is unavailable, the command still stores a trusted claim so the local in-house flow can run. If the player already joined games as pending, registration updates that same Discord row and preserves their local rating and record.

The scheduled worker runs every 3 minutes. For randomized or in-game rows, it probes Spectator-V5 by the first roster PUUID, validates that every known roster PUUID appears in the returned live game, validates the queue against the requested local mode, then marks the game `ingame` with the live Riot match id. Two consecutive Spectator 404s after `ingame` trigger a Match-V5 fetch; if Riot returns a valid roster and winner, the worker stores participant stats and finalizes automatically. If Match-V5 cannot see the match yet, the game stays on the manual `/end` or `/finish` path.

`/results game_id? winner? riot_match_id? region?` reports a winner. If `riot_match_id` is provided and a winner is known from Riot or from the command, it finalizes rating/W-L state immediately. If `riot_match_id` is absent, it only marks the game as reported and leaves `/end` as the manual finalization gate. When a Riot match id is provided, it tries Match-V5, stores the local game to Riot match link, and writes participant stats when Riot returns match data. If Riot cannot see the match yet, it still stores the supplied match id as a manual link so it can be resolved later. `riot_match_id` accepts either `NA1_5561312307` or the numeric game id `5561312307`; numeric ids default to `region:NA`.

`/finish riot_match_id game_id? winner? region?` fetches Match-V5 when the API key can see the match. It validates the match roster, derives the winner when Riot data has enough team info, writes `matches` plus `match_participants`, and finalizes rating/W-L state. Manual `winner` remains supported because Riot custom-match history is gated behind RSO for many flows. If Match-V5 returns 403, `/finish` can still close the game when `winner` is passed or the local game was already reported.

`/hydrate game_id? riot_match_id? region?` retries Match-V5 for linked matches and backfills `match_participants` when Riot data becomes visible later. With no options, it targets the latest linked game in the guild and hydrates every linked match that is still missing Riot stats. With `game_id` and `riot_match_id`, it also attaches that Riot id to the local session if Riot still cannot return data yet. Numeric `riot_match_id` inputs use the same region normalization as `/finish`.

`/link-match riot_match_id game_id? region?` attaches another Riot match id to a local game session without changing ratings or W/L. If `game_id` is omitted, it uses the current open game. This makes `games` the in-house session row and `matches` the one-to-many Riot match history under that session.

ARAM and ARAM: Mayhem have first-class mode rows. Public ARAM queues validate against `450` and `2400`; custom games with queue `0` are accepted because in-house lobbies can report as custom queue data.

For the full Discord workflow, see [docs/commands.md](docs/commands.md).

## Cloudflare Setup

1. Create the D1 database and replace `database_id = "REPLACE_ME"` in `wrangler.toml`.
2. Set Worker secrets:

```sh
npx wrangler secret put DISCORD_PUBLIC_KEY
npx wrangler secret put DISCORD_BOT_TOKEN
npx wrangler secret put RIOT_API_KEY
```

3. Place the Riot domain-verification file at repo root as `riot.txt`. The file is ignored by git and injected into the Worker during `wrangler deploy`; the deployed URL is `https://rsso.aarnphm.workers.dev/riot.txt`.

4. Apply migrations:

```sh
npm run d1:local
npm run d1:remote
```

5. Generate or register Discord commands:

```sh
cargo run -p rsso-cli -- discord commands-json
cargo run -p rsso-cli -- discord register-commands --app-id "$DISCORD_APP_ID" --guild-id "$DISCORD_GUILD_ID" --bot-token "$DISCORD_BOT_TOKEN"
```

Wrangler invokes `worker-build` through Cargo from `crates/rsso-worker`, which is the current `workers-rs` custom-build path. There is no npm `worker-build` package.

## Local Checks

```sh
cargo fmt
cargo clippy --all --benches --tests --examples --all-features
cargo test
cargo check -p rsso-worker --target wasm32-unknown-unknown
npx wrangler deploy --dry-run
curl -fsS https://rsso.aarnphm.workers.dev/riot.txt >/dev/null
npm run d1:local
```
