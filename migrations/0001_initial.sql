PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS players (
    guild_id          TEXT    NOT NULL,
    discord_user_id   TEXT    NOT NULL,
    riot_puuid        TEXT,
    riot_game_name    TEXT    NOT NULL,
    riot_tag_line     TEXT    NOT NULL,
    claim_status      TEXT    NOT NULL CHECK(claim_status IN ('trusted','rso')),
    rating            INTEGER NOT NULL DEFAULT 1500,
    wins              INTEGER NOT NULL DEFAULT 0,
    losses            INTEGER NOT NULL DEFAULT 0,
    consented_at      INTEGER NOT NULL,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    PRIMARY KEY (guild_id, discord_user_id)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_players_riot_id
ON players(guild_id, riot_game_name, riot_tag_line);

CREATE UNIQUE INDEX IF NOT EXISTS idx_players_puuid
ON players(guild_id, riot_puuid)
WHERE riot_puuid IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_players_guild_rating
ON players(guild_id, rating DESC);

CREATE TABLE IF NOT EXISTS games (
    game_id            TEXT    PRIMARY KEY,
    guild_id           TEXT    NOT NULL,
    channel_id         TEXT    NOT NULL,
    creator_discord_id TEXT    NOT NULL,
    status             TEXT    NOT NULL CHECK(status IN ('lobby','randomized','ingame','reported','finalized','cancelled','ambiguous')),
    mode               TEXT    NOT NULL CHECK(mode IN ('rift','aram','aram_mayhem','other')),
    winning_side       TEXT             CHECK(winning_side IN ('blue','red')),
    version            INTEGER NOT NULL DEFAULT 0,
    is_open            INTEGER GENERATED ALWAYS AS
        (CASE WHEN status IN ('lobby','randomized','ingame','reported','ambiguous') THEN 1 ELSE NULL END) STORED,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    randomized_at      INTEGER,
    started_at         INTEGER,
    ended_at           INTEGER,
    riot_match_id      TEXT,
    queue_id           INTEGER,
    map_id             INTEGER,
    riot_game_mode     TEXT,
    riot_game_type     TEXT,
    consecutive_404    INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (guild_id, creator_discord_id) REFERENCES players(guild_id, discord_user_id)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS uniq_one_open_per_guild
ON games(guild_id)
WHERE is_open IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_games_guild_status
ON games(guild_id, status, created_at DESC);

CREATE TABLE IF NOT EXISTS game_players (
    game_id         TEXT    NOT NULL,
    guild_id        TEXT    NOT NULL,
    discord_user_id TEXT    NOT NULL,
    riot_puuid      TEXT,
    team            TEXT             CHECK(team IN ('blue','red')),
    slot            INTEGER          CHECK(slot BETWEEN 0 AND 9),
    pre_rating      INTEGER,
    post_rating     INTEGER,
    result_vote     TEXT             CHECK(result_vote IN ('blue','red')),
    voted_at        INTEGER,
    joined_at       INTEGER NOT NULL,
    PRIMARY KEY (game_id, discord_user_id),
    FOREIGN KEY (game_id) REFERENCES games(game_id) ON DELETE CASCADE,
    FOREIGN KEY (guild_id, discord_user_id) REFERENCES players(guild_id, discord_user_id)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_game_players_game_team
ON game_players(game_id, team, slot);

CREATE INDEX IF NOT EXISTS idx_game_players_discord
ON game_players(guild_id, discord_user_id);

CREATE TABLE IF NOT EXISTS matches (
    riot_match_id      TEXT    PRIMARY KEY,
    game_id            TEXT    NOT NULL,
    guild_id           TEXT    NOT NULL,
    mode               TEXT    NOT NULL,
    queue_id           INTEGER,
    map_id             INTEGER,
    riot_game_mode     TEXT,
    riot_game_type     TEXT,
    data_source        TEXT    NOT NULL CHECK(data_source IN ('manual','match_v5','rso','tournament')),
    payload_json       TEXT,
    finalized_at       INTEGER NOT NULL,
    FOREIGN KEY (game_id) REFERENCES games(game_id) ON DELETE CASCADE
) STRICT;

CREATE TABLE IF NOT EXISTS match_participants (
    riot_match_id       TEXT    NOT NULL,
    puuid               TEXT    NOT NULL,
    discord_user_id     TEXT,
    team                TEXT             CHECK(team IN ('blue','red')),
    champion_id         INTEGER,
    champion_name       TEXT,
    win                 INTEGER,
    kills               INTEGER,
    deaths              INTEGER,
    assists             INTEGER,
    total_damage        INTEGER,
    gold_earned         INTEGER,
    total_minions       INTEGER,
    vision_score        INTEGER,
    raw_json            TEXT,
    PRIMARY KEY (riot_match_id, puuid),
    FOREIGN KEY (riot_match_id) REFERENCES matches(riot_match_id) ON DELETE CASCADE
) STRICT;

CREATE TABLE IF NOT EXISTS match_candidates (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    game_id             TEXT    NOT NULL,
    riot_match_id       TEXT    NOT NULL,
    status              TEXT    NOT NULL CHECK(status IN ('accepted','rejected','ambiguous')),
    reason              TEXT,
    discovered_at       INTEGER NOT NULL,
    payload_json        TEXT,
    FOREIGN KEY (game_id) REFERENCES games(game_id) ON DELETE CASCADE
) STRICT;

CREATE INDEX IF NOT EXISTS idx_match_candidates_game
ON match_candidates(game_id, discovered_at DESC);

CREATE TABLE IF NOT EXISTS events (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id     TEXT    NOT NULL,
    game_id      TEXT,
    actor_id     TEXT,
    kind         TEXT    NOT NULL,
    payload_json TEXT    NOT NULL DEFAULT '{}',
    created_at   INTEGER NOT NULL,
    FOREIGN KEY (game_id) REFERENCES games(game_id) ON DELETE SET NULL
) STRICT;

CREATE INDEX IF NOT EXISTS idx_events_game
ON events(game_id, created_at);

CREATE INDEX IF NOT EXISTS idx_events_guild_kind
ON events(guild_id, kind, created_at DESC);
