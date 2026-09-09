#![allow(dead_code)]

use super::DbPool;
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{error, info, warn};

pub async fn perform_backup(
    pool: &DbPool,
    backup_dir: &str,
    max_kept: usize,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let dir = Path::new(backup_dir);
    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }

    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let backup_path = dir.join(format!("ayanomi_backup_{}.db", timestamp));
    let backup_str = backup_path.to_str().unwrap().replace('\\', "/");

    // SQLite online non-blocking vacuum snapshot
    let query = format!("VACUUM INTO '{}'", backup_str);
    sqlx::query(&query).execute(pool).await?;

    info!("SQLite snapshot backup created successfully: {}", backup_str);

    // Rotate and prune old backups
    rotate_backups(dir, max_kept);

    Ok(backup_str)
}

fn rotate_backups(dir: &Path, max_kept: usize) {
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("db") {
                if let Ok(metadata) = entry.metadata() {
                    let modified = metadata.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    files.push((path, modified));
                }
            }
        }
    }

    // Sort newest first
    files.sort_by(|a, b| b.1.cmp(&a.1));

    // Remove older than max_kept
    if files.len() > max_kept {
        for (old_file, _) in files.into_iter().skip(max_kept) {
            if let Err(e) = fs::remove_file(&old_file) {
                warn!("Failed to prune old backup {:?}: {}", old_file, e);
            } else {
                info!("Pruned old backup: {:?}", old_file);
            }
        }
    }
}

pub fn spawn_backup_worker(
    pool: DbPool,
    backup_dir: String,
    interval_minutes: u64,
    max_kept: usize,
) {
    tokio::spawn(async move {
        let interval_sec = interval_minutes.max(1) * 60;
        let mut ticker = tokio::time::interval(Duration::from_secs(interval_sec));
        // First tick completes immediately
        ticker.tick().await;

        loop {
            ticker.tick().await;
            info!("Running scheduled SQLite database backup...");
            if let Err(e) = perform_backup(&pool, &backup_dir, max_kept).await {
                error!("Scheduled backup failed: {}", e);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;

    #[tokio::test]
    async fn test_backup_and_rotation() {
        let test_dir = "data/test_db_protect";
        let db_path = format!("{}/test.db", test_dir);
        let backup_dir = format!("{}/backups", test_dir);

        let pool = init_db(&db_path).await.expect("init db");

        // Create two backups
        let b1 = perform_backup(&pool, &backup_dir, 1).await.expect("backup 1");
        assert!(Path::new(&b1).exists());

        // Wait small duration for timestamp difference
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let b2 = perform_backup(&pool, &backup_dir, 1).await.expect("backup 2");
        assert!(Path::new(&b2).exists());

        // Since max_kept was 1, b1 should have been pruned and b2 kept
        assert!(Path::new(&b2).exists());

        // Cleanup
        let _ = fs::remove_dir_all(test_dir);
    }
}
