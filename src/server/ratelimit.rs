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

/// Helper to check if a request is authorized for admin access:
/// Authenticated session cookie belongs to a user who holds the `AM` badge.
pub async fn is_admin_authorized(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> bool {
    if let Some(user) = crate::server::frontend::get_authenticated_user(state, headers).await {
        if crate::db::badges::user_has_badge_tag(&state.badges_db, user.id, "AM").await {
            return true;
        }
    }

    false
}

/// Middleware that strictly guards access to the Admin Panel and sensitive Admin APIs.
/// Only allows access to authenticated users possessing the `[AM]` badge.
pub async fn admin_local_guard_middleware(
    State(state): State<AppState>,
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
        let is_authorized = is_admin_authorized(&state, req.headers()).await;

        if !is_authorized {
            // For the /admin page itself, allow unauthorized requests through to admin_page
            // so it can render the login/gate screen with proper guidance
            if path == "/admin" {
                return next.run(req).await;
            }

            let client_ip = extract_client_ip(&req);
            warn!(
                "Security: Blocked unauthorized access to admin route: IP={}, path={}",
                client_ip,
                path
            );

            let forbidden_json = serde_json::json!({
                "error": "Forbidden: Requires an account with [AM] badge"
            });

            return (
                StatusCode::FORBIDDEN,
                [
                    (header::CONTENT_TYPE, "application/json"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                axum::Json(forbidden_json),
            )
                .into_response();
        }
    }

    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn session_headers(user: &crate::db::users::User, secret: &str) -> axum::http::HeaderMap {
        let token = crate::utils::crypto::sign_session(user.id, &user.password_hash, secret);
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            format!("ayanomi_session={token}").parse().unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn test_admin_authorization_requires_current_user_am_badge() {
        let config = Config::default_config();

        let tmp = std::env::temp_dir().join(format!("test_admin_auth_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&tmp);
        let main_db = crate::db::init_db(tmp.join("main.db").to_str().unwrap()).await.unwrap();
        let chat_db = crate::db::chat::init_chat_db(tmp.join("chat.db").to_str().unwrap()).await.unwrap();
        let badges_db = crate::db::badges::init_badges_db(tmp.join("badges.db").to_str().unwrap()).await.unwrap();
        let multi_db = crate::db::multi::init_multi_db(tmp.join("multi.db").to_str().unwrap()).await.unwrap();

        let state = AppState::new(main_db, chat_db, badges_db, multi_db, config);

        let empty_headers = axum::http::HeaderMap::new();

        // Unauthorized when not logged in
        assert!(!is_admin_authorized(&state, &empty_headers).await);

        let admin = crate::db::users::create_user(
            &state.db,
            "badge_admin",
            "admin-password-hash",
            "admin@example.com",
            233,
        )
        .await
        .unwrap();
        let regular_user = crate::db::users::create_user(
            &state.db,
            "regular_user",
            "user-password-hash",
            "user@example.com",
            233,
        )
        .await
        .unwrap();

        let admin_headers = session_headers(&admin, &state.config.server.secret_key);
        let regular_headers = session_headers(&regular_user, &state.config.server.secret_key);

        // A valid login without the AM badge is not enough.
        assert!(!is_admin_authorized(&state, &admin_headers).await);

        let am_badge = crate::db::badges::list_all_badges(&state.badges_db)
            .await
            .unwrap()
            .into_iter()
            .find(|badge| badge.tag.eq_ignore_ascii_case("AM"))
            .unwrap();
        crate::db::badges::award_badge(&state.badges_db, admin.id, am_badge.id)
            .await
            .unwrap();

        // Only the logged-in AM holder is authorized. Another user stays blocked
        // even though an AM holder now exists in the database.
        assert!(is_admin_authorized(&state, &admin_headers).await);
        assert!(!is_admin_authorized(&state, &regular_headers).await);

        state.db.close().await;
        state.chat_db.close().await;
        state.badges_db.close().await;
        state.multi_db.close().await;
        drop(state);
        let _ = std::fs::remove_dir_all(tmp);
    }
}
