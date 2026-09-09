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
pub struct DbBadge {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub icon_url: String,
    pub tag: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DbBadgeWithStats {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub icon_url: String,
    pub tag: String,
    pub holders_count: i64,
    pub created_at: String,
}

/// Strips leading `[...] ` tag prefix from a username.
/// e.g. `[AM] Ayanomi` -> `Ayanomi`
pub fn clean_username(name: &str) -> &str {
    let trimmed = name.trim();
    if trimmed.starts_with('[') {
        if let Some(idx) = trimmed.find(']') {
            return trimmed[idx + 1..].trim_start();
        }
    }
    trimmed
}

/// Returns the formatted display name for a user.
/// If user has a badge with a non-empty tag, prefixes with `[TAG] base_name`.
/// e.g. `[AM] Ayanomi`
pub async fn get_user_display_name(
    pool: &DbPool,
    user_id: i32,
    base_username: &str,
) -> String {
    let clean = clean_username(base_username);
    if let Ok(badges) = get_user_badges(pool, user_id).await {
        for b in badges {
            let tag = b.tag.trim();
            if !tag.is_empty() {
                return format!("[{}] {}", tag, clean);
            }
        }
    }
    clean.to_string()
}

/// Initializes the dedicated SQLite database for Badges
pub async fn init_badges_db(db_path: &str) -> Result<DbPool, sqlx::Error> {
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

    sqlx::query("PRAGMA temp_store = MEMORY; PRAGMA cache_size = -16000;")
        .execute(&pool)
        .await?;

    // Create tables & indexes
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS badges (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            icon_url TEXT NOT NULL,
            tag TEXT NOT NULL DEFAULT '',
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS user_badges (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            badge_id INTEGER NOT NULL REFERENCES badges(id) ON DELETE CASCADE,
            awarded_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(user_id, badge_id)
        );

        CREATE INDEX IF NOT EXISTS idx_user_badges_user ON user_badges(user_id);
        CREATE INDEX IF NOT EXISTS idx_user_badges_badge ON user_badges(badge_id);
        "#,
    )
    .execute(&pool)
    .await?;

    // Auto-migration: ensure tag column exists if table was created previously without it
    let _ = sqlx::query("ALTER TABLE badges ADD COLUMN tag TEXT NOT NULL DEFAULT '';")
        .execute(&pool)
        .await;

    // Seed default starter badges if table is empty
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM badges")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);

    if count == 0 {
        let defaults = [
            ("Server Admin", "Administrator and operator of AyanomiBancho", "👑", "AM"),
            ("Tournament Champion", "Multiplayer tournament winner", "🏆", "CHAMP"),
            ("Early Pioneer", "Pioneer player who joined early in server history", "🚀", "OG"),
            ("Supporter", "Generous supporter of the server", "💖", "SUP"),
            ("Pro Clicker", "Deadeye rhythm clicker with remarkable skills", "🎯", "PRO"),
        ];

        for (name, desc, icon, tag) in defaults {
            let _ = sqlx::query(
                "INSERT INTO badges (name, description, icon_url, tag) VALUES (?, ?, ?, ?)"
            )
            .bind(name)
            .bind(desc)
            .bind(icon)
            .bind(tag)
            .execute(&pool)
            .await;
        }
        info!("Seeded default starter badges into {}", db_path);
    }

    info!("Dedicated Badges Database initialized & protected at {}", db_path);
    Ok(pool)
}

/// Creates a new badge definition
pub async fn create_badge(
    pool: &DbPool,
    name: &str,
    description: &str,
    icon_url: &str,
    tag: &str,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(
        "INSERT INTO badges (name, description, icon_url, tag) VALUES (?, ?, ?, ?)"
    )
    .bind(name)
    .bind(description)
    .bind(icon_url)
    .bind(tag)
    .execute(pool)
    .await?;

    Ok(row.last_insert_rowid())
}

/// Awards a badge to a specific user (idempotent)
pub async fn award_badge(
    pool: &DbPool,
    user_id: i32,
    badge_id: i64,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query(
        "INSERT OR IGNORE INTO user_badges (user_id, badge_id) VALUES (?, ?)"
    )
    .bind(user_id as i64)
    .bind(badge_id)
    .execute(pool)
    .await?;

    Ok(res.rows_affected() > 0)
}

/// Revokes a badge from a user
pub async fn revoke_badge(
    pool: &DbPool,
    user_id: i32,
    badge_id: i64,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query(
        "DELETE FROM user_badges WHERE user_id = ? AND badge_id = ?"
    )
    .bind(user_id as i64)
    .bind(badge_id)
    .execute(pool)
    .await?;

    Ok(res.rows_affected() > 0)
}

/// Retrieves all badges awarded to a specific user
pub async fn get_user_badges(
    pool: &DbPool,
    user_id: i32,
) -> Result<Vec<DbBadge>, sqlx::Error> {
    sqlx::query_as::<_, DbBadge>(
        r#"
        SELECT b.id, b.name, b.description, b.icon_url, b.tag, ub.awarded_at as created_at
        FROM user_badges ub
        JOIN badges b ON b.id = ub.badge_id
        WHERE ub.user_id = ?
        ORDER BY ub.id ASC
        "#,
    )
    .bind(user_id as i64)
    .fetch_all(pool)
    .await
}

/// Lists all badges along with how many users currently hold them
pub async fn list_all_badges(
    pool: &DbPool,
) -> Result<Vec<DbBadgeWithStats>, sqlx::Error> {
    sqlx::query_as::<_, DbBadgeWithStats>(
        r#"
        SELECT b.id, b.name, b.description, b.icon_url, b.tag, b.created_at,
               COUNT(ub.id) as holders_count
        FROM badges b
        LEFT JOIN user_badges ub ON ub.badge_id = b.id
        GROUP BY b.id
        ORDER BY b.id ASC
        "#,
    )
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_badges_lifecycle() {
        let test_dir = std::env::temp_dir().join("test_badges_db");
        let _ = fs::create_dir_all(&test_dir);
        let db_path = test_dir.join("test_badges.db").to_str().unwrap().replace('\\', "/");
        let _ = fs::remove_file(test_dir.join("test_badges.db"));

        let pool = init_badges_db(&db_path).await.unwrap();

        // Check seeded default badges
        let badges = list_all_badges(&pool).await.unwrap();
        assert_eq!(badges.len(), 5);
        assert_eq!(badges[0].name, "Server Admin");
        assert_eq!(badges[0].tag, "AM");

        // Award badge #1 to user 100
        let awarded = award_badge(&pool, 100, badges[0].id).await.unwrap();
        assert!(awarded);

        // Awarding again must be idempotent (no duplicate error)
        let awarded_again = award_badge(&pool, 100, badges[0].id).await.unwrap();
        assert!(!awarded_again);

        // Check user badges
        let user_b = get_user_badges(&pool, 100).await.unwrap();
        assert_eq!(user_b.len(), 1);
        assert_eq!(user_b[0].name, "Server Admin");
        assert_eq!(user_b[0].tag, "AM");

        // Check display name
        let disp = get_user_display_name(&pool, 100, "Ayanomi").await;
        assert_eq!(disp, "[AM] Ayanomi");

        // Clean username check
        assert_eq!(clean_username("[AM] Ayanomi"), "Ayanomi");
        assert_eq!(clean_username("Ayanomi"), "Ayanomi");

        // Revoke badge
        let revoked = revoke_badge(&pool, 100, badges[0].id).await.unwrap();
        assert!(revoked);

        let user_b_after = get_user_badges(&pool, 100).await.unwrap();
        assert_eq!(user_b_after.len(), 0);

        // Display name after revoke
        let disp_after = get_user_display_name(&pool, 100, "Ayanomi").await;
        assert_eq!(disp_after, "Ayanomi");

        // Create custom badge
        let new_id = create_badge(&pool, "Speed Demon", "Double Time master", "⚡", "FAST").await.unwrap();
        assert!(new_id > 0);

        let badges_updated = list_all_badges(&pool).await.unwrap();
        assert_eq!(badges_updated.len(), 6);

        drop(pool);
        let _ = fs::remove_dir_all(test_dir);
    }
}

