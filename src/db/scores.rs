#![allow(dead_code)]

use super::DbPool;
use chrono::Utc;
use sqlx::Row;

#[derive(Debug, Clone)]
pub struct ScoreRecord {
    pub id: i64,
    pub map_md5: String,
    pub user_id: i32,
    pub username: String,
    pub score: i64,
    pub max_combo: i32,
    pub c300: i32,
    pub c100: i32,
    pub c50: i32,
    pub c_geki: i32,
    pub c_katu: i32,
    pub c_miss: i32,
    pub perfect: i32,
    pub mods: u32,
    pub mode: u8,
    pub submitted_at: i64,
}

pub async fn save_score(
    pool: &DbPool,
    map_md5: &str,
    user_id: i32,
    score: i64,
    max_combo: i32,
    c300: i32,
    c100: i32,
    c50: i32,
    c_geki: i32,
    c_katu: i32,
    c_miss: i32,
    perfect: i32,
    mods: u32,
    mode: u8,
) -> Result<i64, sqlx::Error> {
    let now = Utc::now().timestamp();
    let result = sqlx::query(
        r#"
        INSERT INTO scores (
            map_md5, user_id, score, max_combo, 
            c300, c100, c50, c_geki, c_katu, c_miss, 
            perfect, mods, mode, submitted_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(map_md5)
    .bind(user_id)
    .bind(score)
    .bind(max_combo)
    .bind(c300)
    .bind(c100)
    .bind(c50)
    .bind(c_geki)
    .bind(c_katu)
    .bind(c_miss)
    .bind(perfect)
    .bind(mods as i64)
    .bind(mode as i32)
    .bind(now)
    .execute(pool)
    .await?;

    let score_id = result.last_insert_rowid();

    // If Relax (mods & 128 != 0), effective stats mode is mode + 4 (for std, taiko, ctb)
    let is_relax = (mods & 128) != 0;
    let stats_mode = if is_relax && mode <= 2 {
        mode + 4
    } else {
        mode
    };

    // Ensure stats row exists for stats_mode
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO stats (user_id, mode, ranked_score, accuracy, play_count, total_score, pp) VALUES (?, ?, 0, 0.0, 0, 0, 0)"
    )
    .bind(user_id)
    .bind(stats_mode as i32)
    .execute(pool)
    .await;

    // Update user stats (total score, play count, ranked score)
    let ranked_query = if is_relax {
        r#"
        UPDATE stats 
        SET total_score = total_score + ?,
            play_count = play_count + 1,
            ranked_score = (
                SELECT COALESCE(MAX(s.score), 0) 
                FROM scores s 
                WHERE s.user_id = stats.user_id AND s.mode = ? AND (s.mods & 128) != 0
            )
        WHERE user_id = ? AND mode = ?
        "#
    } else {
        r#"
        UPDATE stats 
        SET total_score = total_score + ?,
            play_count = play_count + 1,
            ranked_score = (
                SELECT COALESCE(MAX(s.score), 0) 
                FROM scores s 
                WHERE s.user_id = stats.user_id AND s.mode = ? AND (s.mods & 128) = 0
            )
        WHERE user_id = ? AND mode = ?
        "#
    };

    sqlx::query(ranked_query)
        .bind(score)
        .bind(mode as i32)
        .bind(user_id)
        .bind(stats_mode as i32)
        .execute(pool)
        .await?;

    Ok(score_id)
}

pub async fn get_top_scores_for_map(
    pool: &DbPool,
    map_md5: &str,
    mode: u8,
    is_relax: bool,
    limit: i64,
) -> Result<Vec<ScoreRecord>, sqlx::Error> {
    let query_str = if is_relax {
        r#"
        SELECT s.id, s.map_md5, s.user_id, u.username, s.score, s.max_combo,
               s.c300, s.c100, s.c50, s.c_geki, s.c_katu, s.c_miss,
               s.perfect, s.mods, s.mode, s.submitted_at
        FROM scores s
        JOIN users u ON u.id = s.user_id
        WHERE s.map_md5 = ? AND s.mode = ? AND (s.mods & 128) != 0
        ORDER BY s.score DESC, s.submitted_at ASC
        LIMIT ?
        "#
    } else {
        r#"
        SELECT s.id, s.map_md5, s.user_id, u.username, s.score, s.max_combo,
               s.c300, s.c100, s.c50, s.c_geki, s.c_katu, s.c_miss,
               s.perfect, s.mods, s.mode, s.submitted_at
        FROM scores s
        JOIN users u ON u.id = s.user_id
        WHERE s.map_md5 = ? AND s.mode = ? AND (s.mods & 128) = 0
        ORDER BY s.score DESC, s.submitted_at ASC
        LIMIT ?
        "#
    };

    let rows = sqlx::query(query_str)
        .bind(map_md5)
        .bind(mode as i32)
        .bind(limit)
        .fetch_all(pool)
        .await?;

    let scores = rows
        .into_iter()
        .map(|r| ScoreRecord {
            id: r.get("id"),
            map_md5: r.get("map_md5"),
            user_id: r.get("user_id"),
            username: r.get("username"),
            score: r.get("score"),
            max_combo: r.get("max_combo"),
            c300: r.get("c300"),
            c100: r.get("c100"),
            c50: r.get("c50"),
            c_geki: r.get("c_geki"),
            c_katu: r.get("c_katu"),
            c_miss: r.get("c_miss"),
            perfect: r.get("perfect"),
            mods: r.get::<i64, _>("mods") as u32,
            mode: r.get::<i32, _>("mode") as u8,
            submitted_at: r.get("submitted_at"),
        })
        .collect();

    Ok(scores)
}

pub async fn get_personal_best(
    pool: &DbPool,
    map_md5: &str,
    user_id: i32,
    mode: u8,
    is_relax: bool,
) -> Result<Option<ScoreRecord>, sqlx::Error> {
    let query_str = if is_relax {
        r#"
        SELECT s.id, s.map_md5, s.user_id, u.username, s.score, s.max_combo,
               s.c300, s.c100, s.c50, s.c_geki, s.c_katu, s.c_miss,
               s.perfect, s.mods, s.mode, s.submitted_at
        FROM scores s
        JOIN users u ON u.id = s.user_id
        WHERE s.map_md5 = ? AND s.user_id = ? AND s.mode = ? AND (s.mods & 128) != 0
        ORDER BY s.score DESC
        LIMIT 1
        "#
    } else {
        r#"
        SELECT s.id, s.map_md5, s.user_id, u.username, s.score, s.max_combo,
               s.c300, s.c100, s.c50, s.c_geki, s.c_katu, s.c_miss,
               s.perfect, s.mods, s.mode, s.submitted_at
        FROM scores s
        JOIN users u ON u.id = s.user_id
        WHERE s.map_md5 = ? AND s.user_id = ? AND s.mode = ? AND (s.mods & 128) = 0
        ORDER BY s.score DESC
        LIMIT 1
        "#
    };

    let row = sqlx::query(query_str)
        .bind(map_md5)
        .bind(user_id)
        .bind(mode as i32)
        .fetch_optional(pool)
        .await?;

    if let Some(r) = row {
        Ok(Some(ScoreRecord {
            id: r.get("id"),
            map_md5: r.get("map_md5"),
            user_id: r.get("user_id"),
            username: r.get("username"),
            score: r.get("score"),
            max_combo: r.get("max_combo"),
            c300: r.get("c300"),
            c100: r.get("c100"),
            c50: r.get("c50"),
            c_geki: r.get("c_geki"),
            c_katu: r.get("c_katu"),
            c_miss: r.get("c_miss"),
            perfect: r.get("perfect"),
            mods: r.get::<i64, _>("mods") as u32,
            mode: r.get::<i32, _>("mode") as u8,
            submitted_at: r.get("submitted_at"),
        }))
    } else {
        Ok(None)
    }
}

pub async fn count_scores(pool: &DbPool) -> Result<i64, sqlx::Error> {
    let row = sqlx::query("SELECT COUNT(*) as cnt FROM scores")
        .fetch_one(pool)
        .await?;
    Ok(row.get("cnt"))
}

pub async fn get_user_recent_scores(
    pool: &DbPool,
    user_id: i32,
    limit: i64,
) -> Result<Vec<ScoreRecord>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT s.id, s.map_md5, s.user_id, u.username, s.score, s.max_combo,
               s.c300, s.c100, s.c50, s.c_geki, s.c_katu, s.c_miss,
               s.perfect, s.mods, s.mode, s.submitted_at
        FROM scores s
        JOIN users u ON u.id = s.user_id
        WHERE s.user_id = ?
        ORDER BY s.id DESC
        LIMIT ?
        "#,
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let scores = rows
        .into_iter()
        .map(|r| ScoreRecord {
            id: r.get("id"),
            map_md5: r.get("map_md5"),
            user_id: r.get("user_id"),
            username: r.get("username"),
            score: r.get("score"),
            max_combo: r.get("max_combo"),
            c300: r.get("c300"),
            c100: r.get("c100"),
            c50: r.get("c50"),
            c_geki: r.get("c_geki"),
            c_katu: r.get("c_katu"),
            c_miss: r.get("c_miss"),
            perfect: r.get("perfect"),
            mods: r.get::<i64, _>("mods") as u32,
            mode: r.get::<i32, _>("mode") as u8,
            submitted_at: r.get("submitted_at"),
        })
        .collect();

    Ok(scores)
}
