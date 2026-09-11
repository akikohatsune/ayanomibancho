use crate::db::matches::NewMatchScore;
use crate::db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::FromRow;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSlotInfo {
    pub slot_id: usize,
    pub user_id: i32,
    pub username: String,
    pub status: u8,
    pub status_text: String,
    pub team: u8,
    pub mods: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DbLiveRoom {
    pub match_id: i64,
    pub name: String,
    pub host_id: i64,
    pub host_name: String,
    pub beatmap_id: i64,
    pub beatmap_name: String,
    pub beatmap_md5: String,
    pub mode: i64,
    pub scoring_type: i64,
    pub team_type: i64,
    pub mods: i64,
    pub in_progress: i64,
    pub player_count: i64,
    pub slots_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DbMultiGame {
    pub id: i64,
    pub match_id: i64,
    pub beatmap_id: i64,
    pub beatmap_name: String,
    pub beatmap_md5: String,
    pub mode: i64,
    pub scoring_type: i64,
    pub team_type: i64,
    pub mods: i64,
    pub played_at: i64,
    pub duration_seconds: i64,
    pub winner_id: i64,
    pub winner_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DbMultiScore {
    pub id: i64,
    pub game_id: i64,
    pub match_id: i64,
    pub user_id: i64,
    pub username: String,
    pub slot_id: i64,
    pub team: i64,
    pub score: i64,
    pub max_combo: i64,
    pub accuracy: f64,
    pub c300: i64,
    pub c100: i64,
    pub c50: i64,
    pub c_miss: i64,
    pub c_geki: i64,
    pub c_katu: i64,
    pub passed: i64,
    pub won: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiGameWithScores {
    pub game: DbMultiGame,
    pub scores: Vec<DbMultiScore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveRoomDetails {
    pub room: DbLiveRoom,
    pub slots: Vec<LiveSlotInfo>,
    pub games: Vec<MultiGameWithScores>,
}

pub async fn init_multi_db(db_path: &str) -> Result<DbPool, sqlx::Error> {
    if let Some(parent) = Path::new(db_path).parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).expect("Failed to create multi database directory");
        }
    }

    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_millis(5000))
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(10)
        .connect_with(options)
        .await?;

    sqlx::query("PRAGMA temp_store = MEMORY; PRAGMA cache_size = -32000;")
        .execute(&pool)
        .await?;

    // Create multi tracking schema
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS multi_rooms (
            match_id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            host_id INTEGER NOT NULL,
            host_name TEXT NOT NULL,
            beatmap_id INTEGER NOT NULL,
            beatmap_name TEXT NOT NULL,
            beatmap_md5 TEXT NOT NULL,
            mode INTEGER NOT NULL,
            scoring_type INTEGER NOT NULL,
            team_type INTEGER NOT NULL,
            mods INTEGER NOT NULL,
            in_progress INTEGER NOT NULL DEFAULT 0,
            player_count INTEGER NOT NULL DEFAULT 1,
            slots_json TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS multi_games (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            match_id INTEGER NOT NULL,
            beatmap_id INTEGER NOT NULL,
            beatmap_name TEXT NOT NULL,
            beatmap_md5 TEXT NOT NULL,
            mode INTEGER NOT NULL,
            scoring_type INTEGER NOT NULL,
            team_type INTEGER NOT NULL,
            mods INTEGER NOT NULL,
            played_at INTEGER NOT NULL,
            duration_seconds INTEGER NOT NULL,
            winner_id INTEGER NOT NULL,
            winner_name TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS multi_scores (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id INTEGER NOT NULL,
            match_id INTEGER NOT NULL,
            user_id INTEGER NOT NULL,
            username TEXT NOT NULL,
            slot_id INTEGER NOT NULL,
            team INTEGER NOT NULL,
            score INTEGER NOT NULL,
            max_combo INTEGER NOT NULL,
            accuracy REAL NOT NULL,
            c300 INTEGER NOT NULL,
            c100 INTEGER NOT NULL,
            c50 INTEGER NOT NULL,
            c_miss INTEGER NOT NULL,
            c_geki INTEGER NOT NULL,
            c_katu INTEGER NOT NULL,
            passed INTEGER NOT NULL,
            won INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_multi_games_match ON multi_games(match_id);
        CREATE INDEX IF NOT EXISTS idx_multi_scores_game ON multi_scores(game_id);
        CREATE INDEX IF NOT EXISTS idx_multi_scores_match ON multi_scores(match_id);
        CREATE INDEX IF NOT EXISTS idx_multi_games_match_id_desc ON multi_games(match_id, id DESC);
        CREATE INDEX IF NOT EXISTS idx_multi_scores_game_score_desc ON multi_scores(game_id, score DESC);
        "#,
    )
    .execute(&pool)
    .await?;

    // On startup, prune any old ghost rooms left over from past process runs
    sqlx::query("DELETE FROM multi_scores; DELETE FROM multi_games; DELETE FROM multi_rooms;")
        .execute(&pool)
        .await?;

    info!("Dedicated Multi Database initialized & cleared at {}", db_path);
    Ok(pool)
}

/// Upsert active live room state
pub async fn upsert_room(pool: &DbPool, room: &DbLiveRoom) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO multi_rooms (
            match_id, name, host_id, host_name,
            beatmap_id, beatmap_name, beatmap_md5,
            mode, scoring_type, team_type, mods,
            in_progress, player_count, slots_json,
            created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(match_id) DO UPDATE SET
            name = excluded.name,
            host_id = excluded.host_id,
            host_name = excluded.host_name,
            beatmap_id = excluded.beatmap_id,
            beatmap_name = excluded.beatmap_name,
            beatmap_md5 = excluded.beatmap_md5,
            mode = excluded.mode,
            scoring_type = excluded.scoring_type,
            team_type = excluded.team_type,
            mods = excluded.mods,
            in_progress = excluded.in_progress,
            player_count = excluded.player_count,
            slots_json = excluded.slots_json,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(room.match_id)
    .bind(&room.name)
    .bind(room.host_id)
    .bind(&room.host_name)
    .bind(room.beatmap_id)
    .bind(&room.beatmap_name)
    .bind(&room.beatmap_md5)
    .bind(room.mode)
    .bind(room.scoring_type)
    .bind(room.team_type)
    .bind(room.mods)
    .bind(room.in_progress)
    .bind(room.player_count)
    .bind(&room.slots_json)
    .bind(room.created_at)
    .bind(room.updated_at)
    .execute(pool)
    .await?;

    Ok(())
}

/// Records a completed match round and player scores for a specific live room
pub async fn record_multi_game(
    pool: &DbPool,
    match_id: u16,
    beatmap_id: i32,
    beatmap_name: &str,
    beatmap_md5: &str,
    mode: u8,
    scoring_type: u8,
    team_type: u8,
    mods: u32,
    duration_seconds: i64,
    winner_id: i32,
    winner_name: &str,
    scores: &[NewMatchScore],
) -> Result<i64, sqlx::Error> {
    let now = chrono::Utc::now().timestamp();
    let mut transaction = pool.begin().await?;

    let row = sqlx::query(
        r#"
        INSERT INTO multi_games (
            match_id, beatmap_id, beatmap_name, beatmap_md5,
            mode, scoring_type, team_type, mods,
            played_at, duration_seconds, winner_id, winner_name
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        RETURNING id
        "#,
    )
    .bind(match_id as i64)
    .bind(beatmap_id as i64)
    .bind(beatmap_name)
    .bind(beatmap_md5)
    .bind(mode as i64)
    .bind(scoring_type as i64)
    .bind(team_type as i64)
    .bind(mods as i64)
    .bind(now)
    .bind(duration_seconds)
    .bind(winner_id as i64)
    .bind(winner_name)
    .fetch_one(&mut *transaction)
    .await?;

    let game_id: i64 = sqlx::Row::get(&row, 0);

    for sc in scores {
        sqlx::query(
            r#"
            INSERT INTO multi_scores (
                game_id, match_id, user_id, username, slot_id, team,
                score, max_combo, accuracy,
                c300, c100, c50, c_miss, c_geki, c_katu,
                passed, won
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(game_id)
        .bind(match_id as i64)
        .bind(sc.user_id as i64)
        .bind(&sc.username)
        .bind(sc.slot_id as i64)
        .bind(sc.team as i64)
        .bind(sc.score as i64)
        .bind(sc.max_combo as i64)
        .bind(sc.accuracy as f64)
        .bind(sc.c300 as i64)
        .bind(sc.c100 as i64)
        .bind(sc.c50 as i64)
        .bind(sc.c_miss as i64)
        .bind(sc.c_geki as i64)
        .bind(sc.c_katu as i64)
        .bind(if sc.passed { 1i64 } else { 0i64 })
        .bind(if sc.won { 1i64 } else { 0i64 })
        .execute(&mut *transaction)
        .await?;
    }

    transaction.commit().await?;
    Ok(game_id)
}

/// Deletes all data belonging to a room when it is closed or disbanded
pub async fn delete_room_data(pool: &DbPool, match_id: u16) -> Result<(), sqlx::Error> {
    let mid = match_id as i64;
    let mut transaction = pool.begin().await?;

    sqlx::query("DELETE FROM multi_scores WHERE match_id = ?;")
        .bind(mid)
        .execute(&mut *transaction)
        .await?;

    sqlx::query("DELETE FROM multi_games WHERE match_id = ?;")
        .bind(mid)
        .execute(&mut *transaction)
        .await?;

    sqlx::query("DELETE FROM multi_rooms WHERE match_id = ?;")
        .bind(mid)
        .execute(&mut *transaction)
        .await?;

    transaction.commit().await?;
    info!("Cleaned up all multi tracking data for disbanded room #{}", match_id);
    Ok(())
}

/// Fetches all active rooms with their slot breakdowns and recent game history
pub async fn get_all_live_rooms(pool: &DbPool) -> Result<Vec<LiveRoomDetails>, sqlx::Error> {
    let rooms = sqlx::query_as::<_, DbLiveRoom>(
        r#"
        SELECT match_id, name, host_id, host_name,
               beatmap_id, beatmap_name, beatmap_md5,
               mode, scoring_type, team_type, mods,
               in_progress, player_count, slots_json,
               created_at, updated_at
        FROM multi_rooms
        ORDER BY updated_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    if rooms.is_empty() {
        return Ok(Vec::new());
    }

    // The multi database only contains data for active rooms. Fetch each table once
    // and assemble the hierarchy in memory instead of issuing one query per room/game.
    let games = sqlx::query_as::<_, DbMultiGame>(
        r#"
        SELECT id, match_id, beatmap_id, beatmap_name, beatmap_md5,
               mode, scoring_type, team_type, mods,
               played_at, duration_seconds, winner_id, winner_name
        FROM multi_games
        ORDER BY match_id, id DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let scores = sqlx::query_as::<_, DbMultiScore>(
        r#"
        SELECT id, game_id, match_id, user_id, username, slot_id, team,
               score, max_combo, accuracy,
               c300, c100, c50, c_miss, c_geki, c_katu,
               passed, won
        FROM multi_scores
        ORDER BY game_id, score DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut scores_by_game: HashMap<i64, Vec<DbMultiScore>> = HashMap::new();
    for score in scores {
        scores_by_game.entry(score.game_id).or_default().push(score);
    }

    let mut games_by_match: HashMap<i64, Vec<MultiGameWithScores>> = HashMap::new();
    for game in games {
        let game_scores = scores_by_game.remove(&game.id).unwrap_or_default();
        games_by_match
            .entry(game.match_id)
            .or_default()
            .push(MultiGameWithScores {
                game,
                scores: game_scores,
            });
    }

    Ok(rooms
        .into_iter()
        .map(|room| {
            let slots = serde_json::from_str(&room.slots_json).unwrap_or_default();
            let games = games_by_match.remove(&room.match_id).unwrap_or_default();
            LiveRoomDetails { room, slots, games }
        })
        .collect())
}
