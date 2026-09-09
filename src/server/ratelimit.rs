use crate::state::AppState;
use crate::utils::ratelimit::classify_tier;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use tracing::warn;

/// Extracts client IP from headers (reverse proxy) or socket ConnectInfo
pub fn extract_client_ip(req: &Request) -> IpAddr {
    // 1. Check X-Forwarded-For header
    if let Some(forwarded) = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first_ip_str) = forwarded.split(',').next() {
            if let Ok(ip) = IpAddr::from_str(first_ip_str.trim()) {
                return ip;
            }
        }
    }

    // 2. Check X-Real-IP header
    if let Some(real_ip) = req.headers().get("x-real-ip").and_then(|v| v.to_str().ok()) {
        if let Ok(ip) = IpAddr::from_str(real_ip.trim()) {
            return ip;
        }
    }

    // 3. Check ConnectInfo from underlying socket
    if let Some(connect_info) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
        return connect_info.0.ip();
    }

    // Fallback to loopback
    IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
}

/// Axum middleware that applies multi-tier token bucket rate limiting to incoming requests
pub async fn ratelimit_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let client_ip = extract_client_ip(&req);
    let method = req.method().as_str();
    let path = req.uri().path();
    let tier = classify_tier(method, path);

    tracing::info!("RateLimit: IP={}, tier={:?}, path={}", client_ip, tier, path);

    match state.rate_limiter.check(&client_ip, tier, &state.config.ratelimit) {
        Ok(()) => next.run(req).await,
        Err(retry_after) => {
            warn!(
                "Anti-Raid: Rate limit exceeded for IP {} on tier {:?} ({}) - Retry after {}s",
                client_ip, tier, path, retry_after
            );

            let body = serde_json::json!({
                "error": "Too many requests. Please slow down (Anti-Raid Protection active).",
                "tier": format!("{:?}", tier),
                "retry_after_seconds": retry_after
            });

            (
                StatusCode::TOO_MANY_REQUESTS,
                [
                    (header::RETRY_AFTER, retry_after.to_string()),
                    (header::CONTENT_TYPE, "application/json".to_string()),
                ],
                axum::Json(body),
            )
                .into_response()
        }
    }
}

/// Determines if an IP address originates from localhost (loopback) or local private LAN
pub fn is_local_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            // 127.0.0.0/8 (Loopback)
            octets[0] == 127
            // 10.0.0.0/8 (Private)
            || octets[0] == 10
            // 172.16.0.0/12 (Private)
            || (octets[0] == 172 && (octets[1] >= 16 && octets[1] <= 31))
            // 192.168.0.0/16 (Private)
            || (octets[0] == 192 && octets[1] == 168)
            // 169.254.0.0/16 (Link-local)
            || (octets[0] == 169 && octets[1] == 254)
        }
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback() || {
                let segments = ipv6.segments();
                // fc00::/7 (Unique local)
                (segments[0] & 0xfe00) == 0xfc00
                // fe80::/10 (Link-local)
                || (segments[0] & 0xffc0) == 0xfe80
            }
        }
    }
}

/// Middleware that strictly blocks non-local access to the Admin Panel and sensitive Admin APIs
pub async fn admin_local_guard_middleware(
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    let is_admin_route = path == "/admin"
        || path.starts_with("/api/backgrounds/upload")
        || path.starts_with("/api/backgrounds/delete")
        || path.starts_with("/api/badges/create")
        || path.starts_with("/api/badges/award")
        || path.starts_with("/api/badges/revoke");

    if is_admin_route {
        let client_ip = extract_client_ip(&req);
        if !is_local_ip(&client_ip) {
            warn!(
                "Security: Blocked unauthorized non-local access to admin route: IP={}, path={}",
                client_ip,
                path
            );

            let forbidden_html = r###"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="icon" type="image/png" href="/static/favicon.png">
    <link rel="shortcut icon" href="/favicon.ico">
    <title>403 Forbidden - Local Access Only</title>
    <style>
        body {
            background: #11141a;
            color: #f1f5f9;
            font-family: system-ui, -apple-system, sans-serif;
            display: flex;
            align-items: center;
            justify-content: center;
            min-height: 100vh;
            margin: 0;
        }
        .card {
            background: #181b22;
            border: 1px solid #ef4444;
            border-radius: 8px;
            padding: 2.5rem;
            max-width: 480px;
            text-align: center;
            box-shadow: 0 10px 30px rgba(0,0,0,0.5);
        }
        h1 { color: #ef4444; font-size: 1.6rem; margin: 0 0 1rem 0; }
        p { color: #94a3b8; line-height: 1.6; font-size: 0.95rem; margin-bottom: 1.5rem; }
        a { color: #e0558e; text-decoration: none; font-weight: 700; }
    </style>
</head>
<body>
    <div class="card">
        <h1>403 Forbidden</h1>
        <p>
            The Administrator Control Panel is strictly restricted to local connections (localhost / LAN). External access from the internet is prohibited.
        </p>
        <a href="/">← Return to Homepage</a>
    </div>
</body>
</html>"###;

            return (
                StatusCode::FORBIDDEN,
                [
                    (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                axum::response::Html(forbidden_html),
            )
                .into_response();
        }
    }

    next.run(req).await
}
