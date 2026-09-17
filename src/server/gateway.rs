use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use crate::config::Config;
use crate::utils::ratelimit::{classify_tier, RateLimiter};
use crate::utils::security::is_known_vpn_or_datacenter;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, warn};

#[derive(Clone)]
pub struct GatewayState {
    pub config: Arc<Config>,
    pub client: reqwest::Client,
    pub rate_limiter: Arc<RateLimiter>,
}

pub fn build_gateway_router(config: Arc<Config>) -> Router {
    let state = GatewayState {
        config,
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default(),
        rate_limiter: Arc::new(RateLimiter::new()),
    };

    Router::new()
        .fallback(any(proxy_handler))
        .with_state(state)
}

pub async fn proxy_handler(
    State(state): State<GatewayState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
) -> Response {
    let client_ip = addr.ip();

    // Anti-VPN check at gateway level
    if state.config.security.block_vpn && is_known_vpn_or_datacenter(&client_ip) {
        let client_ref = crate::utils::crypto::privacy_fingerprint(
            &state.config.server.secret_key,
            "log-client-ip",
            &client_ip.to_string(),
        );
        warn!("Gateway rejected VPN/Datacenter client: {}", client_ref);
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
        let client_ref = crate::utils::crypto::privacy_fingerprint(
            &state.config.server.secret_key,
            "log-client-ip",
            &client_ip.to_string(),
        );
        warn!(
            "Gateway: Rate limit exceeded for client {} on tier {:?} ({}) - Retry after {}s",
            client_ref, tier, path, retry_after
        );
        let body = serde_json::json!({
            "error": "Too many requests. Please slow down.",
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
    let host = req
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let is_bancho = (method == Method::POST) && (path == "/" || path == "/c");
    let is_roseflower = host.starts_with("roseflower.")
        || path == "/multi"
        || path.starts_with("/multi/")
        || path == "/api/multi"
        || path.starts_with("/api/multi/");

    let target_port = if is_bancho {
        state.config.server.bancho_port
    } else if is_roseflower {
        state.config.server.roseflower_port
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
        if !matches!(
            name.as_str(),
            "host"
                | "connection"
                | "forwarded"
                | "x-forwarded-for"
                | "x-real-ip"
                | "cf-connecting-ip"
                | "x-forwarded-host"
                | "x-forwarded-proto"
        ) {
            forward_headers.insert(name.clone(), val.clone());
        }
    }

    // Add X-Forwarded-For
    forward_headers.insert(
        "x-forwarded-for",
        HeaderValue::from_str(&client_ip.to_string()).unwrap(),
    );

    // Forward original host as X-Forwarded-Host
    if !host.is_empty() {
        if let Ok(hv) = HeaderValue::from_str(&host) {
            forward_headers.insert("x-forwarded-host", hv);
        }
    }

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
