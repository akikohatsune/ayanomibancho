use crate::db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DbMatchHistory {
    pub id: i64,
    pub name: String,
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
pub struct DbMatchScore {
    pub id: i64,
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
pub struct MatchWithScores {
    pub match_info: DbMatchHistory,
    pub scores: Vec<DbMatchScore>,
}

#[derive(Debug, Clone)]
pub struct NewMatchScore {
    pub user_id: i32,
    pub username: String,
    pub slot_id: u8,
    pub team: u8,
    pub score: i32,
    pub max_combo: u16,
    pub accuracy: f32,
    pub c300: u16,
    pub c100: u16,
    pub c50: u16,
    pub c_miss: u16,
    pub c_geki: u16,
    pub c_katu: u16,
    pub passed: bool,
    pub won: bool,
}

pub async fn save_match_result(
    pool: &DbPool,
    name: &str,
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

    let row = sqlx::query(
        r#"
        INSERT INTO match_history (
            name, beatmap_id, beatmap_name, beatmap_md5,
            mode, scoring_type, team_type, mods, played_at,
            duration_seconds, winner_id, winner_name
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        RETURNING id
        "#,
    )
    .bind(name)
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
    .fetch_one(pool)
    .await?;

    let match_id: i64 = sqlx::Row::get(&row, 0);

    for sc in scores {
        sqlx::query(
            r#"
            INSERT INTO match_scores (
                match_id, user_id, username, slot_id, team,
                score, max_combo, accuracy,
                c300, c100, c50, c_miss, c_geki, c_katu,
                passed, won
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(match_id)
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
        .execute(pool)
        .await?;
    }

    Ok(match_id)
}

pub async fn get_recent_matches(pool: &DbPool, limit: i64) -> Result<Vec<DbMatchHistory>, sqlx::Error> {
    sqlx::query_as::<_, DbMatchHistory>(
        r#"
        SELECT id, name, beatmap_id, beatmap_name, beatmap_md5,
               mode, scoring_type, team_type, mods, played_at,
               duration_seconds, winner_id, winner_name
        FROM match_history
        ORDER BY played_at DESC
        LIMIT ?
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn get_match_detail(pool: &DbPool, match_id: i64) -> Result<Option<MatchWithScores>, sqlx::Error> {
    let match_opt = sqlx::query_as::<_, DbMatchHistory>(
        r#"
        SELECT id, name, beatmap_id, beatmap_name, beatmap_md5,
               mode, scoring_type, team_type, mods, played_at,
               duration_seconds, winner_id, winner_name
        FROM match_history
        WHERE id = ?
        "#,
    )
    .bind(match_id)
    .fetch_optional(pool)
    .await?;

    match match_opt {
        Some(match_info) => {
            let scores = sqlx::query_as::<_, DbMatchScore>(
                r#"
                SELECT id, match_id, user_id, username, slot_id, team,
                       score, max_combo, accuracy,
                       c300, c100, c50, c_miss, c_geki, c_katu,
                       passed, won
                FROM match_scores
                WHERE match_id = ?
                ORDER BY score DESC
                "#,
            )
            .bind(match_id)
            .fetch_all(pool)
            .await?;

            Ok(Some(MatchWithScores { match_info, scores }))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use std::fs;

    #[tokio::test]
    async fn test_match_history_persistence() {
        let test_dir = "data/test_matches_db";
        let db_path = format!("{}/test.db", test_dir);
        let _ = fs::remove_dir_all(test_dir);
        let pool = init_db(&db_path).await.unwrap();

        let user = crate::db::users::create_user(&pool, "AyanomiPlayer", "hash", "test@ayanomi.local", 235).await.unwrap();

        let scores = vec![NewMatchScore {
            user_id: user.id,
            username: user.username.clone(),
            slot_id: 0,
            team: 0,
            score: 1500000,
            max_combo: 450,
            accuracy: 98.75,
            c300: 300,
            c100: 5,
            c50: 1,
            c_miss: 0,
            c_geki: 50,
            c_katu: 10,
            passed: true,
            won: true,
        }];

        let match_id = save_match_result(
            &pool,
            "Tournament Match #1",
            12345,
            "Kira Kira Days",
            "abcdef1234567890",
            0,
            0,
            0,
            0,
            120,
            user.id,
            &user.username,
            &scores,
        )
        .await
        .unwrap();

        assert!(match_id > 0);

        let recent = get_recent_matches(&pool, 10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].name, "Tournament Match #1");
        assert_eq!(recent[0].winner_name, "AyanomiPlayer");

        let detail = get_match_detail(&pool, match_id).await.unwrap().unwrap();
        assert_eq!(detail.match_info.beatmap_id, 12345);
        assert_eq!(detail.scores.len(), 1);
        assert_eq!(detail.scores[0].score, 1500000);
        assert_eq!(detail.scores[0].won, 1);

        drop(pool);
        let _ = fs::remove_dir_all(test_dir);
    }
}
