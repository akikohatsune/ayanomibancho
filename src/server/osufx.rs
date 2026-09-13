//! osu!fx Client Compatibility & Support Module
//! 
//! This module isolates all logic specifically required by the custom osu! client "osu!fx"
//! (Cuttingedge modded client with custom shaders, PP display, and multi-server config).
//! Standard osu! stable client connections are handled separately.

use axum::extract::Query;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

/// Query parameters passed specifically by osu!fx during connection handshake
#[derive(Debug, Deserialize, Default)]
pub struct OsuFxConnectQuery {
    /// Client build version (e.g. b20161205.1cuttingedge)
    pub v: Option<String>,
    /// Username
    pub u: Option<String>,
    /// Password MD5 hash or session info
    pub h: Option<String>,
    /// osu!fx installed .NET frameworks and modules (e.g. dotnet30|dotnet35|dotnet4)
    pub fx: Option<String>,
    /// osu!fx custom channel / client info
    pub ch: Option<String>,
    /// Retry attempt counter
    pub retry: Option<String>,
    /// Failed bancho endpoint URL reported by osu!fx if a previous connection attempt failed
    pub fail: Option<String>,
    /// Additional osu!fx token/parameter
    pub x: Option<String>,
}

/// Check if an incoming request is coming from an osu!fx client
pub fn is_osufx_request(query: &OsuFxConnectQuery, headers: &HeaderMap) -> bool {
    // 1. Direct fx query parameter presence
    if query.fx.is_some() || query.ch.is_some() || query.fail.is_some() {
        return true;
    }

    // 2. Client version containing cuttingedge with fx metadata
    if let Some(ref v) = query.v {
        if v.contains("cuttingedge") {
            return true;
        }
    }

    // 3. Header check
    if let Some(ua) = headers.get("user-agent").and_then(|h| h.to_str().ok()) {
        if ua.contains("osufx") || ua.contains("cuttingedge") {
            return true;
        }
    }

    false
}

/// Dedicated handshake handler for osu!fx (`/web/bancho_connect.php`)
pub async fn osufx_bancho_connect(
    Query(query): Query<OsuFxConnectQuery>,
    _headers: HeaderMap,
) -> Response {
    let _ = query;

    let mut response = (StatusCode::OK, "vn\n").into_response();
    let h = response.headers_mut();
    h.insert("content-type", HeaderValue::from_static("text/html; charset=UTF-8"));
    h.insert("x-client-flavor", HeaderValue::from_static("osu!fx"));
    response
}

/// Handler for `/web/osu-checktweets.php` required by osu!fx right after handshake
pub async fn osufx_checktweets() -> Response {
    // Return empty 200 OK so osu!fx doesn't throw a NotFound network error
    let mut response = (StatusCode::OK, "").into_response();
    let h = response.headers_mut();
    h.insert("content-type", HeaderValue::from_static("text/html; charset=UTF-8"));
    response
}

/// Handler for `/web/osu-error.php` error reporting endpoint from client
pub async fn osufx_error_report(_body: String) -> Response {
    (StatusCode::OK, "").into_response()
}

/// Bancho ping handler for `GET /` and `GET /c` specifically for osu!fx health checks
pub async fn osufx_bancho_ping() -> Response {
    let mut response = (StatusCode::OK, "AyanomiBancho online\n").into_response();
    let h = response.headers_mut();
    h.insert("cho-protocol", HeaderValue::from_static("19"));
    h.insert("content-type", HeaderValue::from_static("text/plain; charset=UTF-8"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_osufx_detection() {
        // osu!fx query with fx modules
        let fx_query = OsuFxConnectQuery {
            v: Some("b20161205.1cuttingedge".to_string()),
            u: Some("Player1".to_string()),
            fx: Some("dotnet30|dotnet35|dotnet4".to_string()),
            ..Default::default()
        };
        assert!(is_osufx_request(&fx_query, &HeaderMap::new()));

        // osu! stable query without fx
        let stable_query = OsuFxConnectQuery {
            v: Some("b20201110".to_string()),
            u: Some("Player2".to_string()),
            ..Default::default()
        };
        assert!(!is_osufx_request(&stable_query, &HeaderMap::new()));
    }

    #[tokio::test]
    async fn test_osufx_handlers() {
        let resp = osufx_checktweets().await;
        assert_eq!(resp.status(), StatusCode::OK);

        let ping_resp = osufx_bancho_ping().await;
        assert_eq!(ping_resp.status(), StatusCode::OK);
        assert_eq!(
            ping_resp.headers().get("cho-protocol").unwrap().to_str().unwrap(),
            "19"
        );
    }
}
