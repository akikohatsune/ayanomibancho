use ayanomibancho::config::Config;
use ayanomibancho::db::{badges::init_badges_db, chat::init_chat_db, init_db};
use ayanomibancho::server::build_web_router;
use ayanomibancho::state::AppState;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,ayanomi_web=debug")),
        )
        .init();

    info!("Starting AyanomiBancho - Dedicated Web & API Service...");

    let config = Config::load("config.toml").unwrap_or_else(|_| Config::default_config());
    let db_pool = init_db(&config.database.path).await?;
    let chat_pool = init_chat_db(&config.database.chat_path).await?;
    let badges_pool = init_badges_db(&config.database.badges_path).await?;
    let multi_pool = ayanomibancho::db::multi::init_multi_db(&config.database.multi_path).await?;
    let app_state = AppState::new(db_pool, chat_pool, badges_pool, multi_pool, config.clone());

    let app = build_web_router(app_state);
    let bind_addr = format!("{}:{}", config.server.host, config.server.web_port);
    let listener = TcpListener::bind(&bind_addr).await?;

    info!("Web & API Service listening on http://{}", bind_addr);
    axum::serve(listener, app).await?;

    Ok(())
}
