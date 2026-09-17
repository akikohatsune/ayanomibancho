use ayanomibancho::config::Config;
use ayanomibancho::server::build_gateway_router;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,ayanomi_gateway=debug")),
        )
        .init();

    info!("Starting AyanomiBancho Gateway...");

    let config = Config::load("config.toml")?;
    let app = build_gateway_router(Arc::new(config.clone()));

    let bind_addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = TcpListener::bind(&bind_addr).await?;

    info!("Ayanomi Gateway listening on http://{}", bind_addr);
    info!("Routing Bancho POST traffic -> Port {}", config.server.bancho_port);
    info!("Routing Roseflower traffic -> Port {} (roseflower.<domain> or /multi)", config.server.roseflower_port);
    info!("Routing Web & Frontend traffic -> Port {}", config.server.web_port);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
