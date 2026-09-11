use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub gameplay: GameplayConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    pub database: DatabaseConfig,
    pub mirrors: MirrorConfig,
    #[serde(default, alias = "menu_backgrounds")]
    pub backgrounds: BackgroundsConfig,
    #[serde(default)]
    pub ratelimit: RateLimitConfig,
    #[serde(default)]
    pub turnstile: TurnstileConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_bancho_port")]
    pub bancho_port: u16,
    #[serde(default = "default_web_port")]
    pub web_port: u16,
    pub domain: String,
    pub name: String,
    pub welcome_message: String,
    #[serde(default = "default_secret_key")]
    pub secret_key: String,
}

fn default_secret_key() -> String {
    "ayanomi_secret".to_string()
}

fn default_bancho_port() -> u16 {
    5001
}
fn default_web_port() -> u16 {
    5002
}

#[derive(Debug, Clone, Deserialize)]
pub struct GameplayConfig {
    pub auto_register: bool,
    pub default_country: u8,
    pub bot_name: String,
    pub bot_id: i32,
    #[serde(default = "default_chat_history_limit")]
    pub chat_history_limit: i64,
}

fn default_chat_history_limit() -> i64 {
    25
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SecurityConfig {
    #[serde(default = "default_true")]
    pub anti_multiaccount: bool,
    #[serde(default = "default_one")]
    pub max_accounts_per_hwid: u32,
    #[serde(default)]
    pub block_vpn: bool,
}

fn default_true() -> bool {
    true
}
fn default_one() -> u32 {
    1
}
fn default_backup_interval() -> u64 {
    60
}
fn default_max_backups() -> usize {
    5
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub path: String,
    #[serde(default = "default_chat_db_path")]
    pub chat_path: String,
    #[serde(default = "default_badges_db_path")]
    pub badges_path: String,
    #[serde(default = "default_multi_db_path")]
    pub multi_path: String,
    #[serde(default = "default_true")]
    pub auto_backup: bool,
    #[serde(default = "default_backup_interval")]
    pub backup_interval_minutes: u64,
    #[serde(default = "default_max_backups")]
    pub max_backups_kept: usize,
}

fn default_chat_db_path() -> String {
    "data/ayanomi_chat.db".to_string()
}

fn default_badges_db_path() -> String {
    "data/ayanomi_badges.db".to_string()
}

fn default_multi_db_path() -> String {
    "data/ayanomi_multi.db".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct MirrorConfig {
    pub direct_search_api: String,
    pub download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BackgroundsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_backgrounds_dir")]
    pub directory: String,
    #[serde(default = "default_artist_name")]
    pub artist_name: String,
}

fn default_backgrounds_dir() -> String {
    "data/backgrounds".to_string()
}

fn default_artist_name() -> String {
    "AyanomiBancho".to_string()
}

impl Default for BackgroundsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            directory: default_backgrounds_dir(),
            artist_name: default_artist_name(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_general_rpm")]
    pub general_rpm: u32,
    #[serde(default = "default_bancho_rpm")]
    pub bancho_rpm: u32,
    #[serde(default = "default_direct_rpm")]
    pub direct_rpm: u32,
    #[serde(default = "default_sensitive_rpm")]
    pub sensitive_rpm: u32,
    #[serde(default = "default_true")]
    pub whitelist_private: bool,
}

fn default_general_rpm() -> u32 {
    120
}
fn default_bancho_rpm() -> u32 {
    180
}
fn default_direct_rpm() -> u32 {
    40
}
fn default_sensitive_rpm() -> u32 {
    10
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            general_rpm: default_general_rpm(),
            bancho_rpm: default_bancho_rpm(),
            direct_rpm: default_direct_rpm(),
            sensitive_rpm: default_sensitive_rpm(),
            whitelist_private: true,
        }
    }
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        #[allow(unused_mut)]
        let mut config: Config = toml::from_str(&content)?;

        // On non-Windows platforms (e.g. ARMv7, ARMv8 Linux, Docker, Raspberry Pi),
        // gracefully convert any hardcoded Windows drive paths (C:/...) to relative Unix paths
        #[cfg(not(windows))]
        {
            if config.database.path.contains(':') {
                config.database.path = "data/ayanomi.db".to_string();
            }
            if config.database.chat_path.contains(':') {
                config.database.chat_path = "data/ayanomi_chat.db".to_string();
            }
            if config.database.badges_path.contains(':') {
                config.database.badges_path = "data/ayanomi_badges.db".to_string();
            }
            if config.database.multi_path.contains(':') {
                config.database.multi_path = "data/ayanomi_multi.db".to_string();
            }
        }

        Ok(config)
    }

    pub fn default_config() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 5000,
                bancho_port: 5001,
                web_port: 5002,
                domain: "127.0.0.1:5000".to_string(),
                name: "AyanomiBancho".to_string(),
                welcome_message: "Welcome to AyanomiBancho!".to_string(),
                secret_key: "ayanomi_secret".to_string(),
            },
            gameplay: GameplayConfig {
                auto_register: true,
                default_country: 233,
                bot_name: "Miku".to_string(),
                bot_id: 3,
                chat_history_limit: 25,
            },
            security: SecurityConfig {
                anti_multiaccount: true,
                max_accounts_per_hwid: 1,
                block_vpn: false,
            },
            database: DatabaseConfig {
                path: "data/ayanomi.db".to_string(),
                chat_path: "data/ayanomi_chat.db".to_string(),
                badges_path: "data/ayanomi_badges.db".to_string(),
                multi_path: "data/ayanomi_multi.db".to_string(),
                auto_backup: true,
                backup_interval_minutes: 60,
                max_backups_kept: 5,
            },
            mirrors: MirrorConfig {
                direct_search_api: "https://catboy.best/api/search".to_string(),
                download_url: "https://catboy.best/d/{}".to_string(),
            },
            backgrounds: BackgroundsConfig::default(),
            ratelimit: RateLimitConfig::default(),
            turnstile: TurnstileConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TurnstileConfig {
    #[serde(default = "default_turnstile_enabled")]
    pub enabled: bool,
    #[serde(default = "default_turnstile_site_key")]
    pub site_key: String,
    #[serde(default)]
    pub secret_key: String,
    #[serde(default = "default_turnstile_action")]
    pub expected_action: Option<String>,
    #[serde(default)]
    pub expected_hostnames: Vec<String>,
}

fn default_turnstile_enabled() -> bool {
    true
}

fn default_turnstile_site_key() -> String {
    "0x4AAAAAAEtvk0dbaHJLPHIb".to_string()
}

fn default_turnstile_action() -> Option<String> {
    Some("login".to_string())
}
