use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Pool, Row, Sqlite};
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use tracing::{error, info, warn};

pub mod backup;
pub mod badges;
pub mod beatmaps;
pub mod chat;
pub mod matches;
pub mod multi;
pub mod scores;
pub mod users;

pub type DbPool = Pool<Sqlite>;

pub async fn init_db(db_path: &str) -> Result<DbPool, sqlx::Error> {
    // Ensure parent directory exists with secure permissions
    if let Some(parent) = Path::new(db_path).parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).expect("Failed to create database directory");
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }

    // SQLite Hardening & Protection Options:
    // 1. WAL Mode (Write-Ahead Logging) for concurrent reads & writes
    // 2. 5000ms busy timeout to prevent "database is locked" errors
    // 3. Foreign key constraints enabled
    // 4. In-memory temporary store and 64MB cache size
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

    // Apply additional performance and security PRAGMAs
    sqlx::query("PRAGMA temp_store = MEMORY; PRAGMA cache_size = -64000;")
        .execute(&pool)
        .await?;

    // Check database integrity on startup
    match sqlx::query("PRAGMA integrity_check(100);")
        .fetch_one(&pool)
        .await
    {
        Ok(row) => {
            let status: String = row.get(0);
            if status == "ok" {
                info!("SQLite integrity check: PASSED (Database is healthy)");
            } else {
                warn!("SQLite integrity check WARNING: {}", status);
            }
        }
        Err(e) => {
            error!("Failed to run SQLite integrity check: {}", e);
        }
    }

    // Run migrations / create tables
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL COLLATE NOCASE,
            password_hash TEXT NOT NULL,
            email TEXT NOT NULL DEFAULT '',
            privileges INTEGER NOT NULL DEFAULT 1,
            country INTEGER NOT NULL DEFAULT 235,
            bio TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS stats (
            user_id INTEGER NOT NULL,
            mode INTEGER NOT NULL,
            ranked_score INTEGER NOT NULL DEFAULT 0,
            accuracy REAL NOT NULL DEFAULT 0.0,
            play_count INTEGER NOT NULL DEFAULT 0,
            total_score INTEGER NOT NULL DEFAULT 0,
            pp INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (user_id, mode),
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS scores (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            map_md5 TEXT NOT NULL,
            user_id INTEGER NOT NULL,
            score INTEGER NOT NULL,
            max_combo INTEGER NOT NULL,
            c300 INTEGER NOT NULL,
            c100 INTEGER NOT NULL,
            c50 INTEGER NOT NULL,
            c_geki INTEGER NOT NULL DEFAULT 0,
            c_katu INTEGER NOT NULL DEFAULT 0,
            c_miss INTEGER NOT NULL,
            perfect INTEGER NOT NULL DEFAULT 0,
            mods INTEGER NOT NULL DEFAULT 0,
            mode INTEGER NOT NULL DEFAULT 0,
            submitted_at INTEGER NOT NULL,
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS user_hardware (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            adapters_hash TEXT NOT NULL,
            uninstall_id TEXT NOT NULL,
            disk_signature TEXT NOT NULL,
            last_ip TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS beatmaps (
            map_md5 TEXT PRIMARY KEY,
            beatmap_id INTEGER NOT NULL DEFAULT 0,
            beatmapset_id INTEGER NOT NULL DEFAULT 0,
            artist TEXT NOT NULL DEFAULT '',
            title TEXT NOT NULL DEFAULT '',
            version TEXT NOT NULL DEFAULT '',
            creator TEXT NOT NULL DEFAULT '',
            updated_at INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS match_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            beatmap_id INTEGER NOT NULL,
            beatmap_name TEXT NOT NULL,
            beatmap_md5 TEXT NOT NULL,
            mode INTEGER NOT NULL,
            scoring_type INTEGER NOT NULL,
            team_type INTEGER NOT NULL,
            mods INTEGER NOT NULL,
            played_at INTEGER NOT NULL,
            duration_seconds INTEGER NOT NULL DEFAULT 0,
            winner_id INTEGER NOT NULL DEFAULT -1,
            winner_name TEXT NOT NULL DEFAULT ''
        );

        CREATE TABLE IF NOT EXISTS match_scores (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            match_id INTEGER NOT NULL,
            user_id INTEGER NOT NULL,
            username TEXT NOT NULL,
            slot_id INTEGER NOT NULL,
            team INTEGER NOT NULL DEFAULT 0,
            score INTEGER NOT NULL,
            max_combo INTEGER NOT NULL,
            accuracy REAL NOT NULL,
            c300 INTEGER NOT NULL,
            c100 INTEGER NOT NULL,
            c50 INTEGER NOT NULL,
            c_miss INTEGER NOT NULL,
            c_geki INTEGER NOT NULL DEFAULT 0,
            c_katu INTEGER NOT NULL DEFAULT 0,
            passed INTEGER NOT NULL DEFAULT 1,
            won INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (match_id) REFERENCES match_history(id) ON DELETE CASCADE,
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_scores_map_mode ON scores(map_md5, mode, score DESC);
        CREATE INDEX IF NOT EXISTS idx_scores_user ON scores(user_id);
        CREATE INDEX IF NOT EXISTS idx_hardware_mac ON user_hardware(adapters_hash);
        CREATE INDEX IF NOT EXISTS idx_hardware_disk ON user_hardware(disk_signature);
        CREATE INDEX IF NOT EXISTS idx_match_history_played ON match_history(played_at DESC);
        CREATE INDEX IF NOT EXISTS idx_match_scores_match ON match_scores(match_id);
        CREATE INDEX IF NOT EXISTS idx_match_scores_user ON match_scores(user_id);
        "#,
    )
    .execute(&pool)
    .await?;

    // Auto-migration: ensure bio column exists on existing databases
    let _ = sqlx::query("ALTER TABLE users ADD COLUMN bio TEXT NOT NULL DEFAULT '';")
        .execute(&pool)
        .await;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if Path::new(db_path).exists() {
            let _ = fs::set_permissions(db_path, fs::Permissions::from_mode(0o600));
        }
    }

    info!("SQLite database initialized & protected at {}", db_path);
    Ok(pool)
}
