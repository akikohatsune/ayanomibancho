use ayanomibancho::config::Config;
use ayanomibancho::db::badges::init_badges_db;
use ayanomibancho::db::chat::init_chat_db;
use ayanomibancho::db::init_db;
use ayanomibancho::protocol::packets::build_user_quit;
use ayanomibancho::server::build_bancho_router;
use ayanomibancho::state::AppState;
use std::time::Duration;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,ayanomi_bancho=debug")),
        )
        .init();

    info!("Starting AyanomiBancho - Dedicated Bancho Service...");

    let config = Config::load("config.toml").unwrap_or_else(|_| Config::default_config());
    let db_pool = init_db(&config.database.path).await?;
    let chat_pool = init_chat_db(&config.database.chat_path).await?;
    let badges_pool = init_badges_db(&config.database.badges_path).await?;
    let multi_pool = ayanomibancho::db::multi::init_multi_db(&config.database.multi_path).await?;
    let app_state = AppState::new(db_pool, chat_pool, badges_pool, multi_pool, config.clone());

    // Spawn session timeout cleanup
    let bancho_state_clone = app_state.bancho.clone();
    let multi_db_clone = app_state.multi_db.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            let mut st = bancho_state_clone.write().await;
            let now = std::time::Instant::now();
            let mut timed_out_tokens = Vec::new();

            for (token, session) in st.sessions.iter() {
                if now.duration_since(session.last_ping) > Duration::from_secs(120) {
                    timed_out_tokens.push((token.clone(), session.user_id, session.username.clone()));
                }
            }

            for (token, user_id, username) in timed_out_tokens {
                info!("Session timed out for user '{}' (ID: {})", username, user_id);
                st.remove_session(&token);
                let quit_pkt = build_user_quit(user_id, 2);
                st.broadcast(&quit_pkt);
            }

            let disbanded = st.take_disbanded_matches();
            for mid in disbanded {
                let pool = multi_db_clone.clone();
                tokio::spawn(async move {
                    let _ = ayanomibancho::db::multi::delete_room_data(&pool, mid).await;
                });
            }
        }
    });

    let app = build_bancho_router(app_state);
    let bind_addr = format!("{}:{}", config.server.host, config.server.bancho_port);
    let listener = TcpListener::bind(&bind_addr).await?;

    info!("Bancho Service listening on http://{}", bind_addr);
    axum::serve(listener, app).await?;

    Ok(())
}
