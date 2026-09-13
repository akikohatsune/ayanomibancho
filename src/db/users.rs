#![allow(dead_code)]

use super::DbPool;
use crate::utils::security::ClientHardware;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderboardUser {
    pub rank: i32,
    pub user_id: i32,
    pub username: String,
    pub country: u8,
    pub pp: i16,
    pub ranked_score: i64,
    pub total_score: i64,
    pub accuracy: f32,
    pub play_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: i32,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    #[serde(skip_serializing)]
    pub email: String,
    pub privileges: i32,
    pub country: u8,
    pub bio: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct DbUserStats {
    pub user_id: i32,
    pub mode: u8,
    pub ranked_score: i64,
    pub accuracy: f32,
    pub play_count: i32,
    pub total_score: i64,
    pub pp: i16,
}

pub async fn get_user_by_username(pool: &DbPool, username: &str) -> Result<Option<User>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, username, password_hash, email, privileges, country, bio, created_at FROM users WHERE username = ? COLLATE NOCASE",
    )
    .bind(username)
    .fetch_optional(pool)
    .await?;

    if let Some(r) = row {
        Ok(Some(User {
            id: r.get("id"),
            username: r.get("username"),
            password_hash: r.get("password_hash"),
            email: r.get("email"),
            privileges: r.get("privileges"),
            country: r.get::<i32, _>("country") as u8,
            bio: r.get("bio"),
            created_at: r.get("created_at"),
        }))
    } else {
        Ok(None)
    }
}

pub async fn get_user_by_id(pool: &DbPool, id: i32) -> Result<Option<User>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, username, password_hash, email, privileges, country, bio, created_at FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    if let Some(r) = row {
        Ok(Some(User {
            id: r.get("id"),
            username: r.get("username"),
            password_hash: r.get("password_hash"),
            email: r.get("email"),
            privileges: r.get("privileges"),
            country: r.get::<i32, _>("country") as u8,
            bio: r.get("bio"),
            created_at: r.get("created_at"),
        }))
    } else {
        Ok(None)
    }
}

pub async fn get_user_by_email(pool: &DbPool, email: &str) -> Result<Option<User>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, username, password_hash, email, privileges, country, bio, created_at FROM users WHERE email = ? COLLATE NOCASE",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;

    if let Some(r) = row {
        Ok(Some(User {
            id: r.get("id"),
            username: r.get("username"),
            password_hash: r.get("password_hash"),
            email: r.get("email"),
            privileges: r.get("privileges"),
            country: r.get::<i32, _>("country") as u8,
            bio: r.get("bio"),
            created_at: r.get("created_at"),
        }))
    } else {
        Ok(None)
    }
}

pub async fn update_user_password(pool: &DbPool, user_id: i32, new_hash: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(new_hash)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn revoke_session(
    pool: &DbPool,
    token_hash: &str,
    expires_at: i64,
) -> Result<(), sqlx::Error> {
    let now = Utc::now().timestamp();
    sqlx::query("DELETE FROM revoked_sessions WHERE expires_at <= ?")
        .bind(now)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT OR REPLACE INTO revoked_sessions (token_hash, expires_at) VALUES (?, ?)",
    )
    .bind(token_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn is_session_revoked(pool: &DbPool, token_hash: &str) -> Result<bool, sqlx::Error> {
    let now = Utc::now().timestamp();
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM revoked_sessions WHERE token_hash = ? AND expires_at > ? LIMIT 1",
    )
    .bind(token_hash)
    .bind(now)
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

pub async fn update_user_bio(pool: &DbPool, user_id: i32, bio: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET bio = ? WHERE id = ?")
        .bind(bio)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_user_country(pool: &DbPool, user_id: i32, country: u8) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET country = ? WHERE id = ?")
        .bind(country as i32)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn create_user(
    pool: &DbPool,
    username: &str,
    password_hash: &str,
    email: &str,
    country: u8,
) -> Result<User, sqlx::Error> {
    let now = Utc::now().timestamp();
    let result = sqlx::query(
        "INSERT INTO users (username, password_hash, email, privileges, country, bio, created_at) VALUES (?, ?, ?, 1, ?, '', ?)",
    )
    .bind(username)
    .bind(password_hash)
    .bind(email)
    .bind(country as i32)
    .bind(now)
    .execute(pool)
    .await?;

    let user_id = result.last_insert_rowid() as i32;

    // Create default stats for all 4 game modes (0=std, 1=taiko, 2=ctb, 3=mania)
    for mode in 0..4 {
        sqlx::query(
            "INSERT OR IGNORE INTO stats (user_id, mode, ranked_score, accuracy, play_count, total_score, pp) VALUES (?, ?, 0, 0.0, 0, 0, 0)",
        )
        .bind(user_id)
        .bind(mode)
        .execute(pool)
        .await?;
    }

    Ok(User {
        id: user_id,
        username: username.to_string(),
        password_hash: password_hash.to_string(),
        email: email.to_string(),
        privileges: 1,
        country,
        bio: "".to_string(),
        created_at: now,
    })
}

pub async fn check_multiaccount(
    pool: &DbPool,
    target_user_id: Option<i32>,
    hw: &ClientHardware,
    max_allowed: u32,
) -> Result<bool, sqlx::Error> {
    // If HWID is blank/empty, don't flag as multiaccount
    if hw.adapters_hash.is_empty() && hw.disk_signature.is_empty() {
        return Ok(false);
    }

    let rows = sqlx::query(
        r#"
        SELECT DISTINCT user_id FROM user_hardware
        WHERE (adapters_hash = ? AND adapters_hash != '')
           OR (disk_signature = ? AND disk_signature != '')
        "#,
    )
    .bind(&hw.adapters_hash)
    .bind(&hw.disk_signature)
    .fetch_all(pool)
    .await?;

    let mut distinct_users: Vec<i32> = rows.into_iter().map(|r| r.get("user_id")).collect();

    // If checking an existing user, they are allowed to login on their own HWID
    if let Some(uid) = target_user_id {
        distinct_users.retain(|&id| id != uid);
    }

    Ok(distinct_users.len() >= max_allowed as usize)
}

pub async fn record_user_hardware(
    pool: &DbPool,
    user_id: i32,
    hw: &ClientHardware,
    ip: &str,
) -> Result<(), sqlx::Error> {
    let now = Utc::now().timestamp();
    sqlx::query(
        r#"
        INSERT INTO user_hardware (user_id, adapters_hash, uninstall_id, disk_signature, last_ip, protection_version, created_at)
        VALUES (?, ?, ?, ?, ?, 1, ?)
        "#,
    )
    .bind(user_id)
    .bind(&hw.adapters_hash)
    .bind(&hw.uninstall_id)
    .bind(&hw.disk_signature)
    .bind(ip)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(())
}

fn looks_like_privacy_fingerprint(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

pub async fn migrate_legacy_hardware_identifiers(
    pool: &DbPool,
    secret: &str,
) -> Result<u64, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, adapters_hash, uninstall_id, disk_signature, last_ip FROM user_hardware WHERE protection_version = 0",
    )
    .fetch_all(pool)
    .await?;

    let mut migrated = 0u64;
    for row in rows {
        let id: i64 = row.get("id");
        let protect = |namespace: &str, value: String| {
            if value.is_empty() || looks_like_privacy_fingerprint(&value) {
                value
            } else {
                crate::utils::crypto::privacy_fingerprint(secret, namespace, &value)
            }
        };
        let adapters = protect("hardware-adapter", row.get("adapters_hash"));
        let uninstall = protect("hardware-uninstall", row.get("uninstall_id"));
        let disk = protect("hardware-disk", row.get("disk_signature"));
        let ip = protect("client-ip", row.get("last_ip"));

        sqlx::query(
            "UPDATE user_hardware SET adapters_hash = ?, uninstall_id = ?, disk_signature = ?, last_ip = ?, protection_version = 1 WHERE id = ?",
        )
        .bind(adapters)
        .bind(uninstall)
        .bind(disk)
        .bind(ip)
        .bind(id)
        .execute(pool)
        .await?;
        migrated += 1;
    }

    Ok(migrated)
}

pub async fn migrate_legacy_md5_passwords(pool: &DbPool) -> Result<u64, sqlx::Error> {
    let rows = sqlx::query("SELECT id, password_hash FROM users WHERE length(password_hash) = 32")
        .fetch_all(pool)
        .await?;

    let mut migrated = 0u64;
    for row in rows {
        let id: i32 = row.get("id");
        let hash: String = row.get("password_hash");
        if hash.len() == 32 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(upgraded) = crate::utils::crypto::hash_password(&hash) {
                sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
                    .bind(upgraded)
                    .bind(id)
                    .execute(pool)
                    .await?;
                migrated += 1;
            }
        }
    }

    Ok(migrated)
}

pub async fn get_or_create_stats(
    pool: &DbPool,
    user_id: i32,
    mode: u8,
) -> Result<DbUserStats, sqlx::Error> {
    let row = sqlx::query(
        "SELECT user_id, mode, ranked_score, accuracy, play_count, total_score, pp FROM stats WHERE user_id = ? AND mode = ?",
    )
    .bind(user_id)
    .bind(mode as i32)
    .fetch_optional(pool)
    .await?;

    if let Some(r) = row {
        Ok(DbUserStats {
            user_id: r.get("user_id"),
            mode: r.get::<i32, _>("mode") as u8,
            ranked_score: r.get("ranked_score"),
            accuracy: r.get::<f64, _>("accuracy") as f32,
            play_count: r.get("play_count"),
            total_score: r.get("total_score"),
            pp: r.get::<i32, _>("pp") as i16,
        })
    } else {
        sqlx::query(
            "INSERT OR IGNORE INTO stats (user_id, mode, ranked_score, accuracy, play_count, total_score, pp) VALUES (?, ?, 0, 0.0, 0, 0, 0)",
        )
        .bind(user_id)
        .bind(mode as i32)
        .execute(pool)
        .await?;

        Ok(DbUserStats {
            user_id,
            mode,
            ranked_score: 0,
            accuracy: 0.0,
            play_count: 0,
            total_score: 0,
            pp: 0,
        })
    }
}

pub async fn get_user_rank(pool: &DbPool, user_id: i32, mode: u8) -> Result<i32, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT COUNT(*) + 1 AS rank FROM stats 
        WHERE mode = ? AND (
            pp > (SELECT pp FROM stats WHERE user_id = ? AND mode = ?)
            OR (
                pp = (SELECT pp FROM stats WHERE user_id = ? AND mode = ?)
                AND ranked_score > (SELECT ranked_score FROM stats WHERE user_id = ? AND mode = ?)
            )
        )
        "#,
    )
    .bind(mode as i32)
    .bind(user_id)
    .bind(mode as i32)
    .bind(user_id)
    .bind(mode as i32)
    .bind(user_id)
    .bind(mode as i32)
    .fetch_one(pool)
    .await?;

    Ok(row.get::<i64, _>("rank") as i32)
}

pub async fn count_users(pool: &DbPool) -> Result<i64, sqlx::Error> {
    let row = sqlx::query("SELECT COUNT(*) as cnt FROM users")
        .fetch_one(pool)
        .await?;
    Ok(row.get("cnt"))
}

pub async fn get_leaderboard(
    pool: &DbPool,
    mode: u8,
    limit: i64,
) -> Result<Vec<LeaderboardUser>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT u.id, u.username, u.country, s.pp, s.ranked_score, s.total_score, s.accuracy, s.play_count
        FROM stats s
        JOIN users u ON u.id = s.user_id
        WHERE s.mode = ? AND (s.play_count > 0 OR s.total_score > 0)
        ORDER BY s.pp DESC, s.ranked_score DESC, s.play_count DESC
        LIMIT ?
        "#,
    )
    .bind(mode as i32)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let mut result = Vec::new();
    for (idx, r) in rows.into_iter().enumerate() {
        result.push(LeaderboardUser {
            rank: (idx + 1) as i32,
            user_id: r.get("id"),
            username: r.get("username"),
            country: r.get::<i32, _>("country") as u8,
            pp: r.get::<i32, _>("pp") as i16,
            ranked_score: r.get("ranked_score"),
            total_score: r.get("total_score"),
            accuracy: r.get::<f64, _>("accuracy") as f32,
            play_count: r.get("play_count"),
        });
    }

    Ok(result)
}

#[cfg(test)]
mod privacy_tests {
    use super::*;

    #[test]
    fn user_serialization_omits_credentials_and_email() {
        let user = User {
            id: 7,
            username: "public-name".to_string(),
            password_hash: "private-password-hash".to_string(),
            email: "private@example.test".to_string(),
            privileges: 1,
            country: 1,
            bio: String::new(),
            created_at: 0,
        };
        let json = serde_json::to_value(user).unwrap();
        assert_eq!(json["username"], "public-name");
        assert!(json.get("password_hash").is_none());
        assert!(json.get("email").is_none());
    }

    #[tokio::test]
    async fn legacy_hardware_identifiers_are_migrated_once() {
        let pool = crate::db::init_db(":memory:").await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, email, created_at) VALUES (1, 'privacy-test', 'hash', '', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO user_hardware (user_id, adapters_hash, uninstall_id, disk_signature, last_ip, protection_version, created_at) VALUES (1, 'raw-adapter', 'raw-uninstall', 'raw-disk', '203.0.113.7', 0, 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let secret = "test-migration-secret-at-least-32-bytes";
        assert_eq!(migrate_legacy_hardware_identifiers(&pool, secret).await.unwrap(), 1);
        assert_eq!(migrate_legacy_hardware_identifiers(&pool, secret).await.unwrap(), 0);

        let row = sqlx::query(
            "SELECT adapters_hash, uninstall_id, disk_signature, last_ip, protection_version FROM user_hardware WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_ne!(row.get::<String, _>("adapters_hash"), "raw-adapter");
        assert_ne!(row.get::<String, _>("last_ip"), "203.0.113.7");
        assert_eq!(row.get::<i64, _>("protection_version"), 1);
    }

    #[tokio::test]
    async fn revoked_sessions_are_detected_until_expiry() {
        let pool = crate::db::init_db(":memory:").await.unwrap();
        let future = Utc::now().timestamp() + 60;
        revoke_session(&pool, "token-fingerprint", future).await.unwrap();
        assert!(is_session_revoked(&pool, "token-fingerprint").await.unwrap());
        assert!(!is_session_revoked(&pool, "different-token").await.unwrap());
    }

    #[tokio::test]
    async fn legacy_md5_passwords_are_migrated_to_bcrypt() {
        let pool = crate::db::init_db(":memory:").await.unwrap();
        let legacy_md5 = "098f6bcd4621d373cade4e832627b4f6"; // md5("test")
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, email, created_at) VALUES (1, 'legacy-user', ?, '', 0)",
        )
        .bind(legacy_md5)
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(migrate_legacy_md5_passwords(&pool).await.unwrap(), 1);
        assert_eq!(migrate_legacy_md5_passwords(&pool).await.unwrap(), 0);

        let row = sqlx::query("SELECT password_hash FROM users WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        let upgraded_hash: String = row.get("password_hash");
        assert_ne!(upgraded_hash, legacy_md5);
        assert!(crate::utils::crypto::verify_password(legacy_md5, &upgraded_hash));
    }
}
