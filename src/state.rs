use crate::bancho::state::BanchoState;
use crate::config::Config;
use crate::db::DbPool;
use crate::utils::ratelimit::RateLimiter;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<DbPool>,
    pub chat_db: Arc<DbPool>,
    pub badges_db: Arc<DbPool>,
    pub multi_db: Arc<DbPool>,
    pub bancho: Arc<RwLock<BanchoState>>,
    pub config: Arc<Config>,
    pub http_client: reqwest::Client,
    pub rate_limiter: Arc<RateLimiter>,
}

impl AppState {
    pub fn new(db: DbPool, chat_db: DbPool, badges_db: DbPool, multi_db: DbPool, config: Config) -> Self {
        Self {
            db: Arc::new(db),
            chat_db: Arc::new(chat_db),
            badges_db: Arc::new(badges_db),
            multi_db: Arc::new(multi_db),
            bancho: Arc::new(RwLock::new(BanchoState::new())),
            config: Arc::new(config),
            http_client: reqwest::Client::builder()
                .user_agent("osu! / AyanomiBancho")
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            rate_limiter: Arc::new(RateLimiter::new()),
        }
    }
}
