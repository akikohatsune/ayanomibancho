#![allow(dead_code)]

use super::DbPool;
use crate::db::beatmaps::get_beatmap_by_md5;
use crate::utils::score_calc::{
    calculate_accuracy, calculate_score_pp, calculate_weighted_accuracy, calculate_weighted_pp,
};
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
    pub pp: f64,
    pub accuracy: f32,
}

#[derive(Debug, Clone, Default)]
pub struct ScoreSaveResult {
    pub score_id: i64,
    pub score_pp: f64,
    pub score_acc: f32,
    pub total_pp_before: i16,
    pub total_pp_after: i16,
    pub acc_before: f32,
    pub acc_after: f32,
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
) -> Result<ScoreSaveResult, sqlx::Error> {
    let now = Utc::now().timestamp();

    // 1. Calculate accuracy for this score
    let score_acc = calculate_accuracy(mode, c300, c100, c50, c_geki, c_katu, c_miss);

    // 2. Fetch beatmap meta to get stars & max_combo if cached
    let meta = get_beatmap_by_md5(pool, map_md5).await;
    let map_stars = meta.as_ref().map(|m| m.stars).filter(|&s| s > 0.0);
    let map_max_combo = meta.as_ref().map(|m| m.max_combo).filter(|&c| c > 0);

    // 3. Calculate PP for this score
    let score_pp = calculate_score_pp(
        mode,
        mods,
        max_combo,
        c300,
        c100,
        c50,
        c_geki,
        c_katu,
        c_miss,
        score,
        map_stars,
        map_max_combo,
    );

    // 4. Save to scores table with pp and accuracy
    let result = sqlx::query(
        r#"
        INSERT INTO scores (
            map_md5, user_id, score, max_combo, 
            c300, c100, c50, c_geki, c_katu, c_miss, 
            perfect, mods, mode, submitted_at, pp, accuracy
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
    .bind(score_pp)
    .bind(score_acc as f64)
    .execute(pool)
    .await?;

    let score_id = result.last_insert_rowid();

    // 5. Determine effective stats mode (for Relax, mode + 4 for std, taiko, ctb)
    let is_relax = (mods & 128) != 0;
    let stats_mode = if is_relax && mode <= 2 {
        mode + 4
    } else {
        mode
    };

    // Ensure stats row exists for stats_mode
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO stats (user_id, mode, ranked_score, accuracy, play_count, total_score, pp) VALUES (?, ?, 0, 0.0, 0, 0, 0)",
    )
    .bind(user_id)
    .bind(stats_mode as i32)
    .execute(pool)
    .await;

    // Get previous stats for chart animations
    let before_row = sqlx::query("SELECT accuracy, pp FROM stats WHERE user_id = ? AND mode = ?")
        .bind(user_id)
        .bind(stats_mode as i32)
        .fetch_optional(pool)
        .await?;

    let (acc_before, pp_before) = match before_row {
        Some(r) => (
            r.get::<f64, _>("accuracy") as f32,
            r.get::<i32, _>("pp") as i16,
        ),
        None => (0.0, 0),
    };

    // 6. Query user's best scores (top 100 per unique beatmap)
    let top_scores_query = if is_relax {
        r#"
        WITH ranked_scores AS (
            SELECT pp, accuracy,
                   ROW_NUMBER() OVER (PARTITION BY map_md5 ORDER BY pp DESC, score DESC) as rn
            FROM scores
            WHERE user_id = ? AND mode = ? AND (mods & 128) != 0
        )
        SELECT pp, accuracy FROM ranked_scores WHERE rn = 1 ORDER BY pp DESC LIMIT 100
        "#
    } else {
        r#"
        WITH ranked_scores AS (
            SELECT pp, accuracy,
                   ROW_NUMBER() OVER (PARTITION BY map_md5 ORDER BY pp DESC, score DESC) as rn
            FROM scores
            WHERE user_id = ? AND mode = ? AND (mods & 128) = 0
        )
        SELECT pp, accuracy FROM ranked_scores WHERE rn = 1 ORDER BY pp DESC LIMIT 100
        "#
    };

    let rows = sqlx::query(top_scores_query)
        .bind(user_id)
        .bind(mode as i32)
        .fetch_all(pool)
        .await?;

    let pps: Vec<f64> = rows.iter().map(|r| r.get::<f64, _>("pp")).collect();
    let accs: Vec<f64> = rows
        .iter()
        .map(|r| r.get::<f64, _>("accuracy"))
        .collect();

    let new_total_pp = calculate_weighted_pp(&pps).round().clamp(0.0, 32767.0) as i16;
    let new_acc = calculate_weighted_accuracy(&accs) as f32;

    // 7. Update user stats (total score, play count, ranked score, accuracy, and pp)
    let ranked_query = if is_relax {
        r#"
        UPDATE stats 
        SET total_score = total_score + ?,
            play_count = play_count + 1,
            accuracy = ?,
            pp = ?,
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
            accuracy = ?,
            pp = ?,
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
        .bind(new_acc as f64)
        .bind(new_total_pp as i32)
        .bind(mode as i32)
        .bind(user_id)
        .bind(stats_mode as i32)
        .execute(pool)
        .await?;

    Ok(ScoreSaveResult {
        score_id,
        score_pp,
        score_acc,
        total_pp_before: pp_before,
        total_pp_after: new_total_pp,
        acc_before,
        acc_after: new_acc,
    })
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
               s.perfect, s.mods, s.mode, s.submitted_at, s.pp, s.accuracy
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
               s.perfect, s.mods, s.mode, s.submitted_at, s.pp, s.accuracy
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
            pp: r.get::<f64, _>("pp"),
            accuracy: r.get::<f64, _>("accuracy") as f32,
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
               s.perfect, s.mods, s.mode, s.submitted_at, s.pp, s.accuracy
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
               s.perfect, s.mods, s.mode, s.submitted_at, s.pp, s.accuracy
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
            pp: r.get::<f64, _>("pp"),
            accuracy: r.get::<f64, _>("accuracy") as f32,
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
               s.perfect, s.mods, s.mode, s.submitted_at, s.pp, s.accuracy
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
            pp: r.get::<f64, _>("pp"),
            accuracy: r.get::<f64, _>("accuracy") as f32,
        })
        .collect();

    Ok(scores)
}

pub async fn recalculate_user_stats(
    pool: &DbPool,
    user_id: i32,
    mode: u8,
    is_relax: bool,
) -> Result<(), sqlx::Error> {
    let stats_mode = if is_relax && mode <= 2 {
        mode + 4
    } else {
        mode
    };

    let top_scores_query = if is_relax {
        r#"
        WITH ranked_scores AS (
            SELECT pp, accuracy,
                   ROW_NUMBER() OVER (PARTITION BY map_md5 ORDER BY pp DESC, score DESC) as rn
            FROM scores
            WHERE user_id = ? AND mode = ? AND (mods & 128) != 0
        )
        SELECT pp, accuracy FROM ranked_scores WHERE rn = 1 ORDER BY pp DESC LIMIT 100
        "#
    } else {
        r#"
        WITH ranked_scores AS (
            SELECT pp, accuracy,
                   ROW_NUMBER() OVER (PARTITION BY map_md5 ORDER BY pp DESC, score DESC) as rn
            FROM scores
            WHERE user_id = ? AND mode = ? AND (mods & 128) = 0
        )
        SELECT pp, accuracy FROM ranked_scores WHERE rn = 1 ORDER BY pp DESC LIMIT 100
        "#
    };

    let rows = sqlx::query(top_scores_query)
        .bind(user_id)
        .bind(mode as i32)
        .fetch_all(pool)
        .await?;

    let pps: Vec<f64> = rows.iter().map(|r| r.get::<f64, _>("pp")).collect();
    let accs: Vec<f64> = rows
        .iter()
        .map(|r| r.get::<f64, _>("accuracy"))
        .collect();

    let new_total_pp = calculate_weighted_pp(&pps).round().clamp(0.0, 32767.0) as i16;
    let new_acc = calculate_weighted_accuracy(&accs) as f32;

    let ranked_query = if is_relax {
        r#"
        UPDATE stats 
        SET accuracy = ?,
            pp = ?,
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
        SET accuracy = ?,
            pp = ?,
            ranked_score = (
                SELECT COALESCE(MAX(s.score), 0) 
                FROM scores s 
                WHERE s.user_id = stats.user_id AND s.mode = ? AND (s.mods & 128) = 0
            )
        WHERE user_id = ? AND mode = ?
        "#
    };

    sqlx::query(ranked_query)
        .bind(new_acc as f64)
        .bind(new_total_pp as i32)
        .bind(mode as i32)
        .bind(user_id)
        .bind(stats_mode as i32)
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn recalculate_all_scores_and_stats(pool: &DbPool) -> Result<(), sqlx::Error> {
    // 1. Recalculate scores that have pp == 0.0 or accuracy == 0.0
    let rows = sqlx::query(
        r#"
        SELECT s.id, s.map_md5, s.user_id, s.score, s.max_combo,
               s.c300, s.c100, s.c50, s.c_geki, s.c_katu, s.c_miss,
               s.mods, s.mode, b.stars, b.max_combo as b_max_combo
        FROM scores s
        LEFT JOIN beatmaps b ON b.map_md5 = s.map_md5
        WHERE (s.pp = 0.0 OR s.accuracy = 0.0) AND (s.c300 > 0 OR s.c100 > 0 OR s.c50 > 0)
        "#,
    )
    .fetch_all(pool)
    .await?;

    for r in rows {
        let sid: i64 = r.get("id");
        let sc_mode: i32 = r.get("mode");
        let sc_mods: i64 = r.get("mods");
        let max_combo: i32 = r.get("max_combo");
        let c300: i32 = r.get("c300");
        let c100: i32 = r.get("c100");
        let c50: i32 = r.get("c50");
        let c_geki: i32 = r.get("c_geki");
        let c_katu: i32 = r.get("c_katu");
        let c_miss: i32 = r.get("c_miss");
        let score: i64 = r.get("score");
        let stars: Option<f64> = r.try_get("stars").ok();
        let b_max_combo: Option<i32> = r.try_get("b_max_combo").ok();

        let acc = calculate_accuracy(sc_mode as u8, c300, c100, c50, c_geki, c_katu, c_miss);
        let pp = calculate_score_pp(
            sc_mode as u8,
            sc_mods as u32,
            max_combo,
            c300,
            c100,
            c50,
            c_geki,
            c_katu,
            c_miss,
            score,
            stars.filter(|&s| s > 0.0),
            b_max_combo.filter(|&c| c > 0),
        );

        let _ = sqlx::query("UPDATE scores SET pp = ?, accuracy = ? WHERE id = ?")
            .bind(pp)
            .bind(acc as f64)
            .bind(sid)
            .execute(pool)
            .await;
    }

    // 2. Recalculate all user stats rows
    let stat_rows = sqlx::query("SELECT user_id, mode FROM stats")
        .fetch_all(pool)
        .await?;

    for sr in stat_rows {
        let uid: i32 = sr.get("user_id");
        let smode: i32 = sr.get("mode");
        let is_rx = smode >= 4;
        let orig_mode = if is_rx { (smode - 4) as u8 } else { smode as u8 };
        let _ = recalculate_user_stats(pool, uid, orig_mode, is_rx).await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;

    #[tokio::test]
    async fn test_save_score_and_stats_calculation() {
        let pool = init_db(":memory:").await.expect("Failed to init in-memory DB");

        // Create test user
        let user_id: i64 = sqlx::query(
            "INSERT INTO users (username, password_hash, created_at) VALUES ('tester', 'hash', 1000)"
        )
        .execute(&pool)
        .await
        .unwrap()
        .last_insert_rowid();

        // 1. Submit first score
        let res1 = save_score(
            &pool, "map1_md5", user_id as i32, 1000000, 500, 500, 0, 0, 0, 0, 0, 1, 0, 0
        ).await.unwrap();

        assert_eq!(res1.score_id, 1);
        assert!((res1.score_acc - 100.0).abs() < 0.001);
        assert!(res1.score_pp > 0.0);
        assert_eq!(res1.total_pp_before, 0);
        assert!(res1.total_pp_after > 0);
        assert!((res1.acc_after - 100.0).abs() < 0.001);

        // Check stats in DB
        let stat_row = sqlx::query("SELECT pp, accuracy, play_count, total_score FROM stats WHERE user_id = ? AND mode = 0")
            .bind(user_id as i32)
            .fetch_one(&pool)
            .await
            .unwrap();

        let db_pp: i32 = stat_row.get("pp");
        let db_acc: f64 = stat_row.get("accuracy");
        let db_plays: i32 = stat_row.get("play_count");

        assert_eq!(db_pp, res1.total_pp_after as i32);
        assert!((db_acc - 100.0).abs() < 0.001);
        assert_eq!(db_plays, 1);

        // 2. Submit second score on different map
        let res2 = save_score(
            &pool, "map2_md5", user_id as i32, 800000, 400, 380, 20, 0, 0, 0, 0, 0, 0, 0
        ).await.unwrap();

        assert_eq!(res2.score_id, 2);
        assert!(res2.score_acc < 100.0 && res2.score_acc > 90.0);
        assert_eq!(res2.total_pp_before, res1.total_pp_after);
        assert!(res2.total_pp_after > res1.total_pp_after);
        // Weighted accuracy should be between 90 and 100
        assert!(res2.acc_after > 90.0 && res2.acc_after < 100.0);
    }
}
