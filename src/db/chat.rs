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
pub struct DbChatMessage {
    pub id: i64,
    pub sender_id: i64,
    pub sender_name: String,
    pub target: String,
    pub message: String,
    pub is_private: i64,
    pub sent_at: String,
}

/// Initializes the dedicated SQLite database for Chat messages
pub async fn init_chat_db(db_path: &str) -> Result<DbPool, sqlx::Error> {
    if let Some(parent) = Path::new(db_path).parent() {
        if !parent.exists() {
            let _ = fs::create_dir_all(parent);
        }
    }

    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_millis(5000));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    sqlx::query("PRAGMA temp_store = MEMORY; PRAGMA cache_size = -16000;")
        .execute(&pool)
        .await?;

    // Create table & performance indexes
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS chat_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            sender_id INTEGER NOT NULL,
            sender_name TEXT NOT NULL,
            target TEXT NOT NULL,
            message TEXT NOT NULL,
            is_private INTEGER NOT NULL DEFAULT 0,
            sent_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_chat_target_time ON chat_messages(target, sent_at DESC);
        CREATE INDEX IF NOT EXISTS idx_chat_sender ON chat_messages(sender_id);
        "#,
    )
    .execute(&pool)
    .await?;

    info!("Dedicated Chat Database initialized & protected at {}", db_path);
    Ok(pool)
}

/// Saves a chat message to the dedicated chat database
pub async fn save_chat_message(
    pool: &DbPool,
    sender_id: i32,
    sender_name: &str,
    target: &str,
    message: &str,
    is_private: bool,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(
        r#"
        INSERT INTO chat_messages (sender_id, sender_name, target, message, is_private)
        VALUES (?, ?, ?, ?, ?)
        "#,
    )
    .bind(sender_id as i64)
    .bind(sender_name)
    .bind(target)
    .bind(message)
    .bind(if is_private { 1i64 } else { 0i64 })
    .execute(pool)
    .await?;

    Ok(row.last_insert_rowid())
}

/// Retrieves the most recent messages for a specific channel (e.g. #osu, #multiplayer)
/// Returned in chronological order (oldest first) so clients display them seamlessly.
pub async fn get_channel_history(
    pool: &DbPool,
    target: &str,
    limit: i64,
) -> Result<Vec<DbChatMessage>, sqlx::Error> {
    sqlx::query_as::<_, DbChatMessage>(
        r#"
        SELECT id, sender_id, sender_name, target, message, is_private, sent_at
        FROM (
            SELECT id, sender_id, sender_name, target, message, is_private, sent_at
            FROM chat_messages
            WHERE target = ?
            ORDER BY id DESC
            LIMIT ?
        )
        ORDER BY id ASC
        "#,
    )
    .bind(target)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Retrieves bidirectional direct messages between two users in chronological order.
pub async fn get_direct_messages(
    pool: &DbPool,
    user1_name: &str,
    user2_name: &str,
    limit: i64,
) -> Result<Vec<DbChatMessage>, sqlx::Error> {
    sqlx::query_as::<_, DbChatMessage>(
        r#"
        SELECT id, sender_id, sender_name, target, message, is_private, sent_at
        FROM (
            SELECT id, sender_id, sender_name, target, message, is_private, sent_at
            FROM chat_messages
            WHERE is_private = 1
              AND ((LOWER(sender_name) = LOWER(?) AND LOWER(target) = LOWER(?))
                OR (LOWER(sender_name) = LOWER(?) AND LOWER(target) = LOWER(?)))
            ORDER BY id DESC
            LIMIT ?
        )
        ORDER BY id ASC
        "#,
    )
    .bind(user1_name)
    .bind(user2_name)
    .bind(user2_name)
    .bind(user1_name)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Retrieves the most recent public messages across all channels for dashboard viewing
pub async fn get_recent_chats(
    pool: &DbPool,
    limit: i64,
) -> Result<Vec<DbChatMessage>, sqlx::Error> {
    sqlx::query_as::<_, DbChatMessage>(
        r#"
        SELECT id, sender_id, sender_name, target, message, is_private, sent_at
        FROM chat_messages
        WHERE is_private = 0
        ORDER BY id DESC
        LIMIT ?
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_chat_persistence() {
        let test_dir = "data/test_chat_db";
        let db_path = format!("{}/test_chat.db", test_dir);
        let _ = fs::remove_dir_all(test_dir);

        let pool = init_chat_db(&db_path).await.unwrap();

        // Save public messages
        let id1 = save_chat_message(&pool, 1, "AyanomiBot", "#osu", "Welcome to #osu!", false)
            .await
            .unwrap();
        assert!(id1 > 0);

        let id2 = save_chat_message(&pool, 2, "Player1", "#osu", "Hello everyone!", false)
            .await
            .unwrap();
        assert!(id2 > id1);

        // Save private message
        let id3 = save_chat_message(&pool, 2, "Player1", "Player2", "Hey private message!", true)
            .await
            .unwrap();
        assert!(id3 > id2);

        let id4 = save_chat_message(&pool, 3, "Player2", "Player1", "Reply to your PM!", true)
            .await
            .unwrap();
        assert!(id4 > id3);

        // Query #osu channel history (should be chronological: oldest first)
        let history = get_channel_history(&pool, "#osu", 10).await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].message, "Welcome to #osu!"); // oldest first
        assert_eq!(history[1].message, "Hello everyone!");

        // Query direct messages between Player1 and Player2
        let dm_history = get_direct_messages(&pool, "Player1", "Player2", 10).await.unwrap();
        assert_eq!(dm_history.len(), 2);
        assert_eq!(dm_history[0].message, "Hey private message!");
        assert_eq!(dm_history[1].message, "Reply to your PM!");

        // Query recent public chats
        let recent = get_recent_chats(&pool, 10).await.unwrap();
        assert_eq!(recent.len(), 2); // private messages excluded

        drop(pool);
        let _ = fs::remove_dir_all(test_dir);
    }
}
