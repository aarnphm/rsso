CREATE INDEX IF NOT EXISTS idx_matches_game_finalized
ON matches(game_id, finalized_at DESC);

CREATE INDEX IF NOT EXISTS idx_matches_guild_finalized
ON matches(guild_id, finalized_at DESC);
