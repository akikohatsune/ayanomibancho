use crate::db::users::{get_user_by_id, is_session_revoked, User};
use crate::state::AppState;
use crate::utils::crypto::{privacy_fingerprint, session_user_id, verify_session};
use axum::http::{header, HeaderMap};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse {
    pub success: bool,
    pub message: String,
}

pub fn session_cookie_value(headers: &HeaderMap) -> Option<&str> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    for cookie in cookie_header.split(';') {
        let mut parts = cookie.trim().splitn(2, '=');
        if let (Some(name), Some(val)) = (parts.next(), parts.next()) {
            if name == "ayanomi_session" {
                return Some(val);
            }
        }
    }
    None
}

pub async fn get_authenticated_user(state: &AppState, headers: &HeaderMap) -> Option<User> {
    let token = session_cookie_value(headers)?;
    let token_hash = privacy_fingerprint(
        &state.config.server.secret_key,
        "revoked-session",
        token,
    );
    if is_session_revoked(&state.db, &token_hash).await.unwrap_or(true) {
        return None;
    }

    let user_id = session_user_id(token)?;
    let user = get_user_by_id(&state.db, user_id).await.ok()??;
    verify_session(token, &user.password_hash, &state.config.server.secret_key)?;
    Some(user)
}
