# rsso

Discord-native League of Legends in-house tracking.

The first version is API-key-first: Discord users claim a Riot ID, the bot tracks one active game per guild, teams are randomized with a Fisher-Yates shuffle, and stats are stored by local game/mode. RSO and Tournament API support are later upgrades that should fill the same schema instead of replacing it.

There is intentionally no `pyproject.toml` or Python helper package here. The repo uses a Cargo workspace under `crates/`, `wrangler` for Cloudflare Workers, and D1 migrations under `migrations/`.

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
- `/game mode user_1 ... user_10`
- `/add game_id user`
- `/randomize game_id`
- `/result game_id winner`
- `/finish riot_match_id game_id? winner?`
- `/end game_id`
- `/stats user? mode?`
- `/leaderboards mode?`
- `/analysis mode?`

## Riot API Path

`/register-summoners` accepts a Riot ID shaped as `GameName#TAG` and resolves it through Account-V1 when `RIOT_API_KEY` is configured. If the key is absent or the account is not found, it stores the claim without a PUUID so the in-house can still run in trust mode.

The scheduled worker runs every 3 minutes. For randomized or in-game rows, it probes Spectator-V5 by the first roster PUUID, validates that every known roster PUUID appears in the returned live game, validates the queue against the requested local mode, then marks the game `ingame` with the live Riot match id. Two consecutive Spectator 404s after `ingame` trigger a Match-V5 fetch; if Riot returns a valid roster and winner, the worker stores participant stats and finalizes automatically. If Match-V5 cannot see the match yet, the game stays on the manual `/end` or `/finish` path.

`/finish riot_match_id game_id? winner?` fetches Match-V5 when the API key can see the match. It validates the match roster, derives the winner when Riot data has enough team info, writes `matches` plus `match_participants`, and finalizes rating/W-L state. Manual `winner` remains supported because Riot custom-match history is gated behind RSO for many flows.

ARAM and ARAM: Mayhem have first-class mode rows. Public ARAM queues validate against `450` and `2400`; custom games with queue `0` are accepted because in-house lobbies can report as custom queue data.

## Cloudflare Setup

1. Create the D1 database and replace `database_id = "REPLACE_ME"` in `wrangler.toml`.
2. Set Worker secrets:

```sh
npx wrangler secret put DISCORD_PUBLIC_KEY
npx wrangler secret put DISCORD_BOT_TOKEN
npx wrangler secret put RIOT_API_KEY
```

3. Apply migrations:

```sh
npm run d1:local
npm run d1:remote
```

4. Generate or register Discord commands:

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
npm run d1:local
```
