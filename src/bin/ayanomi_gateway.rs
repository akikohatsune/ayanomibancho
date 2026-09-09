use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use ayanomibancho::config::Config;
use ayanomibancho::utils::security::is_known_vpn_or_datacenter;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use ayanomibancho::utils::ratelimit::{classify_tier, RateLimiter};

#[derive(Clone)]
struct GatewayState {
    config: Arc<Config>,
    client: reqwest::Client,
    rate_limiter: Arc<RateLimiter>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,ayanomi_gateway=debug")),
        )
        .init();

    info!("Starting AyanomiBancho - Fault-Isolated Gateway...");

    let config = Config::load("config.toml").unwrap_or_else(|_| Config::default_config());
    let state = GatewayState {
        config: Arc::new(config.clone()),
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?,
        rate_limiter: Arc::new(RateLimiter::new()),
    };

    let app = Router::new()
        .fallback(any(proxy_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let bind_addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = TcpListener::bind(&bind_addr).await?;

    info!("Ayanomi Gateway listening on http://{}", bind_addr);
    info!("Routing Bancho POST traffic -> Port {}", config.server.bancho_port);
    info!("Routing Web & API traffic    -> Port {}", config.server.web_port);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

async fn proxy_handler(
    State(state): State<GatewayState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
) -> Response {
    let client_ip = addr.ip();

    // Anti-VPN check at gateway level
    if state.config.security.block_vpn && is_known_vpn_or_datacenter(&client_ip) {
        warn!("Gateway rejected VPN/Datacenter IP: {}", client_ip);
        return (
            StatusCode::FORBIDDEN,
            "Access denied: VPN or Datacenter Proxy is not permitted.",
        )
            .into_response();
    }

    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path();
    let query = uri.query().unwrap_or("");

    // Anti-Raid / Rate Limit check at gateway level
    let tier = classify_tier(method.as_str(), path);
    if let Err(retry_after) = state.rate_limiter.check(&client_ip, tier, &state.config.ratelimit) {
        warn!(
            "Gateway Anti-Raid: Rate limit exceeded for IP {} on tier {:?} ({}) - Retry after {}s",
            client_ip, tier, path, retry_after
        );
        let body = serde_json::json!({
            "error": "Too many requests. Please slow down (Anti-Raid Protection active).",
            "tier": format!("{:?}", tier),
            "retry_after_seconds": retry_after
        });
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [
                (axum::http::header::RETRY_AFTER, retry_after.to_string()),
                (axum::http::header::CONTENT_TYPE, "application/json".to_string()),
            ],
            axum::Json(body),
        )
            .into_response();
    }

    // Determine target service port
    let is_bancho = (method == Method::POST) && (path == "/" || path == "/c");
    let target_port = if is_bancho {
        state.config.server.bancho_port
    } else {
        state.config.server.web_port
    };

    let target_url = if query.is_empty() {
        format!("http://127.0.0.1:{}{}", target_port, path)
    } else {
        format!("http://127.0.0.1:{}{}?{}", target_port, path, query)
    };

    // Forward headers
    let mut forward_headers = reqwest::header::HeaderMap::new();
    for (name, val) in req.headers() {
        if name != "host" && name != "connection" {
            forward_headers.insert(name.clone(), val.clone());
        }
    }

    // Add X-Forwarded-For
    forward_headers.insert(
        "x-forwarded-for",
        HeaderValue::from_str(&client_ip.to_string()).unwrap(),
    );

    // Read body
    let body_bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            error!("Failed to read request body: {}", e);
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    // Send proxied request
    match state
        .client
        .request(method, &target_url)
        .headers(forward_headers)
        .body(body_bytes)
        .send()
        .await
    {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
            let resp_headers = resp.headers().clone();
            let bytes = resp.bytes().await.unwrap_or_default();

            let mut response = (status, bytes).into_response();
            let headers_mut = response.headers_mut();
            for (k, v) in resp_headers {
                if let Some(name) = k {
                    headers_mut.insert(name, v);
                }
            }
            response
        }
        Err(e) => {
            warn!(
                "Service on port {} is unreachable: {}. Returning isolated error.",
                target_port, e
            );
            (
                StatusCode::BAD_GATEWAY,
                "Service temporarily unavailable (Fault Isolated)",
            )
                .into_response()
        }
    }
}
