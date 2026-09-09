use ayanomibancho::config::Config;
use ayanomibancho::db::backup::spawn_backup_worker;
use ayanomibancho::db::badges::init_badges_db;
use ayanomibancho::db::chat::init_chat_db;
use ayanomibancho::db::init_db;
use ayanomibancho::protocol::packets::build_user_quit;
use ayanomibancho::server::{build_bancho_router, build_web_router};
use ayanomibancho::state::AppState;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use tokio::net::TcpListener;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,ayanomibancho=debug")),
        )
        .init();

    info!("===========================================================");
    info!("                      AyanomiBancho                        ");
    info!("===========================================================");

    let config = Config::load("config.toml").unwrap_or_else(|e| {
        warn!("Failed to load config.toml ({}). Using default config.", e);
        Config::default_config()
    });

    let db_pool = init_db(&config.database.path).await?;
    let chat_pool = init_chat_db(&config.database.chat_path).await?;
    let badges_pool = init_badges_db(&config.database.badges_path).await?;
    let multi_pool = ayanomibancho::db::multi::init_multi_db(&config.database.multi_path).await?;

    // Spawn automated SQLite backup worker if enabled
    if config.database.auto_backup {
        let parent = Path::new(&config.database.path)
            .parent()
            .unwrap_or(Path::new("data"))
            .to_str()
            .unwrap();
        let backup_dir = format!("{}/backups", parent);
        spawn_backup_worker(
            db_pool.clone(),
            backup_dir,
            config.database.backup_interval_minutes,
            config.database.max_backups_kept,
        );
        info!(
            "SQLite Auto-Backup active: every {} mins (keeping last {} backups)",
            config.database.backup_interval_minutes, config.database.max_backups_kept
        );
    }

    let app_state = AppState::new(db_pool, chat_pool, badges_pool, multi_pool, config.clone());

    // 1. Spawn session timeout pruner for Bancho
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

    // 2. Spawn Rate Limiter idle cleanup worker (every 5 mins)
    let rate_limiter_clone = app_state.rate_limiter.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            interval.tick().await;
            rate_limiter_clone.prune_stale(Duration::from_secs(600));
        }
    });

    // 3. Spawn Bancho Service on Port 5001 (Isolated)
    let bancho_state = app_state.clone();
    let bancho_addr = format!("{}:{}", config.server.host, config.server.bancho_port);
    tokio::spawn(async move {
        match TcpListener::bind(&bancho_addr).await {
            Ok(listener) => {
                info!("[Bancho Service] Online on http://{}", bancho_addr);
                let app = build_bancho_router(bancho_state);
                if let Err(e) = axum::serve(listener, app).await {
                    error!("[Bancho Service] Error: {}", e);
                }
            }
            Err(e) => error!("[Bancho Service] Failed to bind: {}", e),
        }
    });

    // 3. Spawn Web Service on Port 5002 (Isolated)
    let web_state = app_state.clone();
    let web_addr = format!("{}:{}", config.server.host, config.server.web_port);
    tokio::spawn(async move {
        match TcpListener::bind(&web_addr).await {
            Ok(listener) => {
                info!("[Web Service] Online on http://{}", web_addr);
                let app = build_web_router(web_state);
                if let Err(e) = axum::serve(listener, app).await {
                    error!("[Web Service] Error: {}", e);
                }
            }
            Err(e) => error!("[Web Service] Failed to bind: {}", e),
        }
    });

    // 4. Run Gateway on Port 5000 (Main Entrypoint)
    tokio::time::sleep(Duration::from_millis(100)).await;

    let gateway_addr = format!("{}:{}", config.server.host, config.server.port);
    let gateway_listener = TcpListener::bind(&gateway_addr).await?;

    info!("-----------------------------------------------------------");
    info!("Gateway listening on:      http://{}", gateway_addr);
    info!("Server Status & Dashboard: http://{}", config.server.domain);
    info!("osu! Client connect:       osu!.exe -devserver {}", config.server.domain);
    info!("-----------------------------------------------------------");

    let unified_router = ayanomibancho::server::build_unified_router(app_state);
    axum::serve(
        gateway_listener,
        unified_router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
