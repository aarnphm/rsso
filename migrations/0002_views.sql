CREATE VIEW IF NOT EXISTS player_record_view AS
SELECT
    p.guild_id,
    p.discord_user_id,
    p.riot_game_name,
    p.riot_tag_line,
    p.rating,
    p.wins,
    p.losses,
    CASE
        WHEN (p.wins + p.losses) = 0 THEN 0.0
        ELSE CAST(p.wins AS REAL) / CAST(p.wins + p.losses AS REAL)
    END AS win_rate
FROM players p;

CREATE VIEW IF NOT EXISTS mode_leaderboard_view AS
SELECT
    g.guild_id,
    g.mode,
    gp.discord_user_id,
    COUNT(*) AS games,
    SUM(CASE WHEN g.winning_side = gp.team THEN 1 ELSE 0 END) AS wins,
    SUM(CASE WHEN g.winning_side != gp.team THEN 1 ELSE 0 END) AS losses,
    CASE
        WHEN COUNT(*) = 0 THEN 0.0
        ELSE CAST(SUM(CASE WHEN g.winning_side = gp.team THEN 1 ELSE 0 END) AS REAL) / CAST(COUNT(*) AS REAL)
    END AS win_rate
FROM games g
JOIN game_players gp ON gp.game_id = g.game_id
WHERE g.status = 'finalized'
GROUP BY g.guild_id, g.mode, gp.discord_user_id;

CREATE VIEW IF NOT EXISTS leaderboard_view AS
SELECT
    p.guild_id,
    p.discord_user_id,
    p.riot_game_name,
    p.riot_tag_line,
    p.rating,
    p.wins,
    p.losses,
    CASE
        WHEN (p.wins + p.losses) = 0 THEN 0.0
        ELSE CAST(p.wins AS REAL) / CAST(p.wins + p.losses AS REAL)
    END AS win_rate
FROM players p
ORDER BY p.guild_id, p.rating DESC, p.wins DESC;

CREATE VIEW IF NOT EXISTS champion_stats_view AS
SELECT
    m.guild_id,
    m.mode,
    mp.discord_user_id,
    mp.champion_id,
    mp.champion_name,
    COUNT(*) AS games,
    SUM(CASE WHEN mp.win = 1 THEN 1 ELSE 0 END) AS wins,
    AVG(mp.kills) AS avg_kills,
    AVG(mp.deaths) AS avg_deaths,
    AVG(mp.assists) AS avg_assists,
    AVG(mp.total_damage) AS avg_total_damage
FROM match_participants mp
JOIN matches m ON m.riot_match_id = mp.riot_match_id
GROUP BY m.guild_id, m.mode, mp.discord_user_id, mp.champion_id, mp.champion_name;
