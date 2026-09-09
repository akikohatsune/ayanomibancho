#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};
use sysinfo::{MemoryRefreshKind, RefreshKind, System};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerHealthReport {
    pub server_name: String,
    pub overall_status: String, // "healthy", "degraded", "offline"
    pub gateway_healthy: bool,
    pub bancho_healthy: bool,
    pub web_healthy: bool,
    pub mirror_status: String, // "reachable", "slow", "unreachable"
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub ram_usage_percent: f32,
    pub uptime_seconds: u64,
    pub db_size_kb: u64,
    pub chat_db_size_kb: u64,
    pub badges_db_size_kb: u64,
    pub active_sessions: usize,
    pub total_registered_users: i64,
    pub total_scores_recorded: i64,
    pub ratelimit_active: bool,
    pub ratelimit_blocked_count: u64,
    pub ratelimit_tracked_ips: usize,
}

pub fn get_memory_metrics() -> (u64, u64, f32) {
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    sys.refresh_memory();

    let total = sys.total_memory() / (1024 * 1024);
    let used = sys.used_memory() / (1024 * 1024);
    let percent = if total > 0 {
        (used as f32 / total as f32) * 100.0
    } else {
        0.0
    };

    (used, total, percent)
}

pub fn get_file_size_kb<P: AsRef<Path>>(path: P) -> u64 {
    fs::metadata(path)
        .map(|m| m.len() / 1024)
        .unwrap_or(0)
}

/// Quick check if external beatmap mirror is reachable with aggressive 2s timeout
pub async fn probe_mirror_health(client: &reqwest::Client, mirror_url: &str) -> String {
    let check_url = format!("{}/d/1", mirror_url.trim_end_matches('/'));
    let start = Instant::now();

    match client
        .head(&check_url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
    {
        Ok(_) => {
            if start.elapsed() > Duration::from_millis(1500) {
                "slow".to_string()
            } else {
                "reachable".to_string()
            }
        }
        Err(_) => "unreachable".to_string(),
    }
}
