use crate::db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::FromRow;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DbFriend {
    pub user_id: i32,
    pub friend_id: i32,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendUserInfo {
    pub user_id: i32,
    pub username: String,
    pub country: u8,
    pub country_code: String,
    pub country_name: String,
    pub rank_std: i32,
    pub pp_std: i64,
    pub is_mutual: bool,
    pub added_at: i64,
}

/// Initializes the dedicated SQLite database for Friends
pub async fn init_friends_db(db_path: &str) -> Result<DbPool, sqlx::Error> {
    if let Some(parent) = Path::new(db_path).parent() {
        if !parent.exists() {
            let _ = fs::create_dir_all(parent);
        }
    }

    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_millis(5000))
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS friends (
            user_id INTEGER NOT NULL,
            friend_id INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (user_id, friend_id)
        );
        CREATE INDEX IF NOT EXISTS idx_friends_user ON friends(user_id);
        CREATE INDEX IF NOT EXISTS idx_friends_friend ON friends(friend_id);
        "#,
    )
    .execute(&pool)
    .await?;

    crate::db::protect_sqlite_path(db_path);

    info!("Dedicated Friends Database initialized & protected at {}", db_path);
    Ok(pool)
}

/// Adds a friend for `user_id`. Returns true if newly inserted.
pub async fn add_friend(
    pool: &DbPool,
    user_id: i32,
    friend_id: i32,
) -> Result<bool, sqlx::Error> {
    if user_id == friend_id {
        return Ok(false);
    }
    let now = chrono::Utc::now().timestamp();
    let res = sqlx::query(
        "INSERT OR IGNORE INTO friends (user_id, friend_id, created_at) VALUES (?, ?, ?);",
    )
    .bind(user_id)
    .bind(friend_id)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(res.rows_affected() > 0)
}

/// Removes a friend for `user_id`. Returns true if deleted.
pub async fn remove_friend(
    pool: &DbPool,
    user_id: i32,
    friend_id: i32,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM friends WHERE user_id = ? AND friend_id = ?;")
        .bind(user_id)
        .bind(friend_id)
        .execute(pool)
        .await?;

    Ok(res.rows_affected() > 0)
}

/// Checks if `user_id` has added `friend_id`.
pub async fn is_friend(
    pool: &DbPool,
    user_id: i32,
    friend_id: i32,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query("SELECT 1 FROM friends WHERE user_id = ? AND friend_id = ? LIMIT 1;")
        .bind(user_id)
        .bind(friend_id)
        .fetch_optional(pool)
        .await?;

    Ok(row.is_some())
}

/// Checks if `user_id` and `other_id` are mutual friends.
pub async fn is_mutual_friend(
    pool: &DbPool,
    user_id: i32,
    other_id: i32,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT 1 FROM friends f1
        INNER JOIN friends f2 ON f1.user_id = f2.friend_id AND f1.friend_id = f2.user_id
        WHERE f1.user_id = ? AND f1.friend_id = ?
        LIMIT 1;
        "#,
    )
    .bind(user_id)
    .bind(other_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.is_some())
}

/// Retrieves the raw list of friend IDs for `user_id`.
pub async fn get_friend_ids(
    pool: &DbPool,
    user_id: i32,
) -> Result<Vec<i32>, sqlx::Error> {
    let rows = sqlx::query_scalar::<_, i32>(
        "SELECT friend_id FROM friends WHERE user_id = ? ORDER BY created_at DESC;",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Retrieves full friends list with user statistics.
pub async fn get_friends_list(
    friends_pool: &DbPool,
    users_pool: &DbPool,
    user_id: i32,
) -> Result<Vec<FriendUserInfo>, sqlx::Error> {
    let records = sqlx::query_as::<_, DbFriend>(
        "SELECT user_id, friend_id, created_at FROM friends WHERE user_id = ? ORDER BY created_at DESC;",
    )
    .bind(user_id)
    .fetch_all(friends_pool)
    .await?;

    let mut result = Vec::new();

    for r in records {
        if let Ok(Some(u)) = crate::db::users::get_user_by_id(users_pool, r.friend_id).await {
            let country_info = crate::utils::country::bancho_id_to_country(u.country);
            let rank = crate::db::users::get_user_rank(users_pool, u.id, 0)
                .await
                .unwrap_or(1);
            let stats = crate::db::users::get_or_create_stats(users_pool, u.id, 0)
                .await
                .unwrap_or_default();
            let mutual = is_mutual_friend(friends_pool, user_id, u.id)
                .await
                .unwrap_or(false);

            result.push(FriendUserInfo {
                user_id: u.id,
                username: u.username,
                country: u.country,
                country_code: country_info.code.to_string(),
                country_name: country_info.name.to_string(),
                rank_std: rank,
                pp_std: stats.pp as i64,
                is_mutual: mutual,
                added_at: r.created_at,
            });
        }
    }

    Ok(result)
}
