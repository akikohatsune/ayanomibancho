#![allow(dead_code)]

use crate::db::matches::get_recent_matches;
use crate::db::scores::{count_scores, get_user_recent_scores};
use crate::db::users::{
    count_users, create_user, get_leaderboard, get_or_create_stats, get_user_by_id,
    get_user_by_username, get_user_rank, User,
};
use crate::state::AppState;
use crate::utils::country::{bancho_id_to_country, iso_to_bancho_id};
use crate::utils::crypto::{
    hash_password, md5_hex, sign_session, verify_password, verify_session,
};
use crate::utils::telemetry::{get_file_size_kb, get_memory_metrics, probe_mirror_health, ServerHealthReport};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Json, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub cf_turnstile_response: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    pub email: Option<String>,
    pub country: Option<String>,
    #[serde(default)]
    pub cf_turnstile_response: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub bio: Option<String>,
    pub country: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ApiResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct LeaderboardQuery {
    pub m: Option<u8>,
}

#[derive(Debug, Deserialize)]
pub struct AdminQuery {
    pub key: Option<String>,
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

pub fn format_number(num: i64) -> String {
    let s = num.to_string();
    let mut result = String::new();
    let len = s.len();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result
}

pub fn calculate_grade(acc: f32, misses: i32) -> (&'static str, &'static str, &'static str) {
    if misses == 0 && acc >= 99.99 {
        ("SS", "#00f0ff", "rgba(0, 240, 255, 0.15)")
    } else if misses == 0 && acc >= 95.0 {
        ("S", "#fbbf24", "rgba(251, 191, 36, 0.15)")
    } else if acc >= 90.0 && misses <= 2 {
        ("A", "#00e676", "rgba(0, 230, 118, 0.15)")
    } else if acc >= 80.0 {
        ("B", "#38bdf8", "rgba(56, 189, 248, 0.15)")
    } else if acc >= 70.0 {
        ("C", "#c084fc", "rgba(192, 132, 252, 0.15)")
    } else {
        ("D", "#ff4060", "rgba(255, 64, 96, 0.15)")
    }
}

pub async fn get_server_status(State(state): State<AppState>) -> Json<ServerHealthReport> {
    let (ram_used, ram_total, ram_pct) = get_memory_metrics();
    let db_size = get_file_size_kb(&state.config.database.path);
    let chat_db_size = get_file_size_kb(&state.config.database.chat_path);
    let badges_db_size = get_file_size_kb(&state.config.database.badges_path);
    let uptime = { state.bancho.read().await.start_time.elapsed().as_secs() };
    let active_sessions = { state.bancho.read().await.online_count() };
    let total_users = count_users(&state.db).await.unwrap_or(0);
    let total_scores = count_scores(&state.db).await.unwrap_or(0);

    let bancho_url = format!("http://127.0.0.1:{}/health/bancho", state.config.server.bancho_port);
    let bancho_healthy = state
        .http_client
        .get(&bancho_url)
        .timeout(Duration::from_millis(500))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(true);

    let mirror_status = probe_mirror_health(&state.http_client, &state.config.mirrors.download_url).await;

    let overall = if bancho_healthy {
        "healthy".to_string()
    } else {
        "degraded".to_string()
    };

    Json(ServerHealthReport {
        server_name: state.config.server.name.clone(),
        overall_status: overall,
        gateway_healthy: true,
        bancho_healthy,
        web_healthy: true,
        mirror_status,
        ram_used_mb: ram_used,
        ram_total_mb: ram_total,
        ram_usage_percent: ram_pct,
        uptime_seconds: uptime,
        db_size_kb: db_size,
        chat_db_size_kb: chat_db_size,
        badges_db_size_kb: badges_db_size,
        active_sessions,
        total_registered_users: total_users,
        total_scores_recorded: total_scores,
        ratelimit_active: state.config.ratelimit.enabled,
        ratelimit_blocked_count: state.rate_limiter.total_blocked_count(),
        ratelimit_tracked_ips: state.rate_limiter.tracked_ips_count(),
    })
}

pub async fn register_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RegisterRequest>,
) -> Response {
    let username = payload.username.trim();

    // Turnstile bot verification
    if state.config.turnstile.enabled {
        let secret = std::env::var("TURNSTILE_SECRET")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| state.config.turnstile.secret_key.clone());

        if !secret.trim().is_empty() {
            let token = payload.cf_turnstile_response.as_deref().unwrap_or("");
            if token.is_empty() {
                return (
                    StatusCode::FORBIDDEN,
                    Json(ApiResponse {
                        success: false,
                        message: "Bot verification required. Please complete the Turnstile challenge.".to_string(),
                    }),
                )
                    .into_response();
            }

            let client_ip = headers
                .get("cf-connecting-ip")
                .or_else(|| headers.get("x-forwarded-for"))
                .or_else(|| headers.get("x-real-ip"))
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(',').next())
                .map(|s| s.trim());

            if let Err(err) = crate::utils::turnstile::verify_turnstile_token(
                &secret,
                token,
                client_ip,
                state.config.turnstile.expected_action.as_deref(),
                &state.config.turnstile.expected_hostnames,
            )
            .await
            {
                return (
                    StatusCode::FORBIDDEN,
                    Json(ApiResponse {
                        success: false,
                        message: format!("Security check failed: {}", err),
                    }),
                )
                    .into_response();
            }
        }
    }

    if username.is_empty() || username.len() < 2 || username.len() > 20 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                success: false,
                message: "Username must be between 2 and 20 characters.".to_string(),
            }),
        )
            .into_response();
    }

    if payload.password.len() < 4 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                success: false,
                message: "Password must be at least 4 characters long.".to_string(),
            }),
        )
            .into_response();
    }

    if let Ok(Some(_)) = get_user_by_username(&state.db, username).await {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse {
                success: false,
                message: format!("Username '{}' is already taken. Please choose another one.", username),
            }),
        )
            .into_response();
    }

    let country_id = if let Some(ref c_str) = payload.country {
        if let Ok(id) = c_str.parse::<u8>() {
            id
        } else {
            iso_to_bancho_id(c_str)
        }
    } else {
        state.config.gameplay.default_country
    };

    let md5_pass = md5_hex(&payload.password);
    let pwd_hash = match hash_password(&md5_pass) {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    message: format!("Password encryption error: {}", e),
                }),
            )
                .into_response();
        }
    };

    let email = payload.email.unwrap_or_else(|| format!("{}@ayanomi.local", username));

    match create_user(&state.db, username, &pwd_hash, &email, country_id).await {
        Ok(user) => (
            StatusCode::CREATED,
            Json(ApiResponse {
                success: true,
                message: format!("Welcome! Account '{}' (ID: {}) has been created successfully.", user.username, user.id),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                message: format!("Database error: {}", e),
            }),
        )
            .into_response(),
    }
}

pub async fn update_profile_api(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UpdateProfileRequest>,
) -> Response {
    let user = match get_authenticated_user(&state, &headers).await {
        Some(u) => u,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse {
                    success: false,
                    message: "Please log in to update your profile.".to_string(),
                }),
            )
                .into_response();
        }
    };

    if let Some(ref bio_text) = payload.bio {
        if bio_text.len() > 2000 {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    success: false,
                    message: "Bio cannot exceed 2,000 characters.".to_string(),
                }),
            )
                .into_response();
        }
        if let Err(e) = crate::db::users::update_user_bio(&state.db, user.id, bio_text).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    message: format!("Failed to update bio: {}", e),
                }),
            )
                .into_response();
        }
    }

    if let Some(ref c_str) = payload.country {
        let cid = if let Ok(id) = c_str.parse::<u8>() {
            id
        } else {
            iso_to_bancho_id(c_str)
        };
        let _ = crate::db::users::update_user_country(&state.db, user.id, cid).await;
    }

    (
        StatusCode::OK,
        Json(ApiResponse {
            success: true,
            message: "Profile updated successfully!".to_string(),
        }),
    )
        .into_response()
}

fn common_css() -> &'static str {
    r###"
        :root {
            --bg-base: #11141a;
            --bg-surface: #181b22;
            --bg-surface-hover: #1f232c;
            --card-border: #282d38;
            --card-border-subtle: #1e222b;
            
            --primary: #e0558e;
            --primary-hover: #c9447a;
            
            --accent: #705df2;
            --accent-hover: #5d4be0;
            
            --cyan: #0ea5e9;
            --emerald: #10b981;
            --amber: #f59e0b;
            --rose: #ef4444;
            
            --text-main: #f1f5f9;
            --text-muted: #94a3b8;
            --text-sub: #64748b;
        }

        * {
            box-sizing: border-box;
            margin: 0;
            padding: 0;
            font-family: 'Plus Jakarta Sans', system-ui, -apple-system, sans-serif;
        }

        body {
            background-color: var(--bg-base);
            color: var(--text-main);
            min-height: 100vh;
            display: flex;
            flex-direction: column;
            overflow-x: hidden;
            -webkit-font-smoothing: antialiased;
        }

        a { color: inherit; text-decoration: none; }

        /* Navbar */
        nav {
            position: sticky;
            top: 0;
            z-index: 1000;
            background: var(--bg-surface);
            border-bottom: 1px solid var(--card-border);
        }

        .nav-container {
            max-width: 1200px;
            margin: 0 auto;
            display: flex;
            align-items: center;
            justify-content: space-between;
            padding: 0.85rem 1.5rem;
        }

        .nav-brand {
            display: flex;
            align-items: center;
            font-size: 1.35rem;
            font-weight: 800;
            letter-spacing: -0.5px;
            color: var(--primary);
        }

        .nav-links {
            display: flex;
            align-items: center;
            gap: 1.6rem;
            list-style: none;
            font-size: 0.92rem;
            font-weight: 600;
        }

        .nav-links a {
            color: var(--text-muted);
            padding: 0.4rem 0;
            transition: color 0.15s ease;
        }

        .nav-links a:hover, .nav-links a.active {
            color: var(--text-main);
        }

        .nav-links a.active {
            border-bottom: 2px solid var(--primary);
        }

        .nav-actions {
            display: flex;
            align-items: center;
            gap: 0.8rem;
        }

        /* Buttons */
        .btn {
            display: inline-flex;
            align-items: center;
            justify-content: center;
            padding: 0.55rem 1.2rem;
            border-radius: 6px;
            font-weight: 700;
            font-size: 0.9rem;
            border: 1px solid transparent;
            cursor: pointer;
            transition: background-color 0.15s ease, border-color 0.15s ease;
            text-decoration: none;
            user-select: none;
        }

        .btn-primary {
            background: var(--primary);
            color: #fff;
        }

        .btn-primary:hover {
            background: var(--primary-hover);
        }

        .btn-outline {
            background: transparent;
            border: 1px solid var(--card-border);
            color: var(--text-main);
        }

        .btn-outline:hover {
            background: var(--bg-surface-hover);
            border-color: #3f4657;
        }

        .btn-accent {
            background: var(--accent);
            color: #fff;
        }

        .btn-accent:hover {
            background: var(--accent-hover);
        }

        .btn-danger {
            background: transparent;
            color: var(--rose);
            border: 1px solid var(--rose);
            padding: 0.35rem 0.75rem;
            border-radius: 4px;
            font-size: 0.82rem;
            font-weight: 600;
        }

        .btn-danger:hover {
            background: var(--rose);
            color: #fff;
        }

        /* Container */
        .main-container {
            max-width: 1200px;
            width: 100%;
            margin: 2rem auto;
            padding: 0 1.5rem;
            flex: 1;
            display: flex;
            flex-direction: column;
            gap: 2rem;
        }

        /* Flat Cards */
        .glass-card {
            background: var(--bg-surface);
            border: 1px solid var(--card-border);
            border-radius: 8px;
            padding: 1.6rem;
        }

        .card-header-bar {
            display: flex;
            align-items: center;
            justify-content: space-between;
            margin-bottom: 1.2rem;
            flex-wrap: wrap;
            gap: 1rem;
        }

        .card-heading {
            font-size: 1.25rem;
            font-weight: 800;
            letter-spacing: -0.3px;
        }

        /* Hero Banner */
        .hero-banner {
            background: var(--bg-surface);
            border: 1px solid var(--card-border);
            border-radius: 8px;
            padding: 3rem 2rem;
            text-align: center;
            display: flex;
            flex-direction: column;
            align-items: center;
            gap: 1.2rem;
        }

        .hero-tag {
            display: inline-flex;
            align-items: center;
            background: var(--bg-surface-hover);
            border: 1px solid var(--card-border);
            color: var(--primary);
            padding: 4px 14px;
            border-radius: 4px;
            font-size: 0.85rem;
            font-weight: 700;
            letter-spacing: 0.5px;
        }

        .hero-title {
            font-size: 2.8rem;
            font-weight: 900;
            letter-spacing: -0.8px;
            line-height: 1.2;
            max-width: 820px;
        }

        .hero-subtitle {
            max-width: 660px;
            color: var(--text-muted);
            font-size: 1.05rem;
            line-height: 1.6;
        }

        .hero-btns {
            display: flex;
            gap: 1rem;
            flex-wrap: wrap;
            justify-content: center;
            margin-top: 0.5rem;
        }

        /* Stat Grid Chips */
        .hero-stats-strip {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(210px, 1fr));
            gap: 1rem;
        }

        .stat-box {
            background: var(--bg-surface);
            border: 1px solid var(--card-border);
            border-radius: 8px;
            padding: 1.2rem;
            text-align: center;
        }

        .stat-box .stat-val {
            font-size: 2rem;
            font-weight: 800;
            margin-top: 0.2rem;
        }

        .stat-box .stat-lbl {
            font-size: 0.8rem;
            text-transform: uppercase;
            letter-spacing: 0.5px;
            color: var(--text-muted);
            font-weight: 700;
        }

        /* Online Players Modern Grid */
        .player-pill-card {
            background: var(--bg-surface-hover);
            border: 1px solid var(--card-border);
            border-radius: 6px;
            padding: 0.8rem 1rem;
            display: flex;
            align-items: center;
            gap: 0.8rem;
        }

        .player-avatar-wrapper {
            position: relative;
            width: 40px;
            height: 40px;
            flex-shrink: 0;
        }

        .player-avatar-img {
            width: 100%;
            height: 100%;
            border-radius: 50%;
            object-fit: cover;
            border: 1px solid var(--card-border);
        }

        .online-dot-badge {
            position: absolute;
            bottom: 0;
            right: 0;
            width: 10px;
            height: 10px;
            border-radius: 50%;
            background: var(--emerald);
            border: 2px solid var(--bg-surface);
        }

        /* Country & Badge Tags */
        .country-tag {
            display: inline-block;
            font-family: 'JetBrains Mono', monospace;
            font-size: 0.78rem;
            font-weight: 700;
            color: var(--text-muted);
            background: var(--bg-base);
            border: 1px solid var(--card-border);
            padding: 1px 5px;
            border-radius: 4px;
        }

        .badge-tag {
            display: inline-block;
            font-size: 0.78rem;
            font-weight: 700;
            color: var(--accent);
            background: var(--bg-base);
            border: 1px solid var(--card-border);
            padding: 1px 6px;
            border-radius: 4px;
            margin-left: 4px;
        }

        /* Grade Pill (SS, S, A, B, C, D) */
        .grade-badge {
            display: inline-flex;
            align-items: center;
            justify-content: center;
            width: 36px;
            height: 22px;
            border-radius: 4px;
            font-weight: 800;
            font-size: 0.82rem;
        }

        /* Tables */
        .modern-table {
            width: 100%;
            border-collapse: collapse;
            font-size: 0.92rem;
        }

        .modern-table th {
            padding: 0.75rem 1rem;
            color: var(--text-sub);
            font-size: 0.8rem;
            font-weight: 700;
            text-transform: uppercase;
            border-bottom: 2px solid var(--card-border);
            text-align: left;
        }

        .modern-table td {
            padding: 0.8rem 1rem;
            border-bottom: 1px solid var(--card-border-subtle);
            vertical-align: middle;
        }

        .modern-table tbody tr:hover {
            background: var(--bg-surface-hover);
        }

        /* Rank Medals */
        .rank-box {
            font-size: 1rem;
            font-weight: 800;
            display: inline-flex;
            align-items: center;
            justify-content: center;
            min-width: 32px;
        }

        .rank-gold { color: #f59e0b; font-weight: 800; }
        .rank-silver { color: #cbd5e1; font-weight: 800; }
        .rank-bronze { color: #d97706; font-weight: 800; }

        .pp-highlight {
            font-weight: 800;
            color: var(--primary);
            font-size: 1rem;
        }

        .pp-highlight span {
            font-size: 0.78rem;
            color: var(--text-muted);
            font-weight: 600;
            margin-left: 2px;
        }

        /* Step cards */
        .step-box {
            display: flex;
            gap: 1rem;
            padding: 1.2rem;
            background: var(--bg-surface-hover);
            border: 1px solid var(--card-border);
            border-radius: 6px;
            align-items: flex-start;
        }

        .step-num-bubble {
            width: 36px;
            height: 36px;
            border-radius: 6px;
            background: var(--bg-base);
            border: 1px solid var(--card-border);
            color: var(--primary);
            display: flex;
            align-items: center;
            justify-content: center;
            font-size: 1.05rem;
            font-weight: 800;
            flex-shrink: 0;
        }

        .terminal-box {
            background: #0d0f14;
            border: 1px solid var(--card-border);
            border-radius: 6px;
            padding: 0.8rem 1rem;
            font-family: 'JetBrains Mono', monospace;
            font-size: 0.9rem;
            color: var(--cyan);
            margin-top: 0.5rem;
            display: flex;
            align-items: center;
            justify-content: space-between;
        }

        /* Form Controls */
        .form-label {
            font-size: 0.86rem;
            font-weight: 600;
            color: var(--text-muted);
            margin-bottom: 0.4rem;
            display: block;
        }

        .input-glass {
            width: 100%;
            background: #0d0f14;
            border: 1px solid var(--card-border);
            border-radius: 6px;
            padding: 0.75rem 1rem;
            color: #fff;
            font-size: 0.92rem;
        }

        .input-glass:focus {
            outline: none;
            border-color: var(--primary);
        }

        /* Footer */
        footer {
            border-top: 1px solid var(--card-border);
            background: var(--bg-surface);
            padding: 2.2rem 1.5rem;
            margin-top: 4rem;
            color: var(--text-sub);
            font-size: 0.88rem;
        }

        .footer-content {
            max-width: 1200px;
            margin: 0 auto;
            display: flex;
            flex-direction: column;
            align-items: center;
            gap: 1rem;
            text-align: center;
        }

        .footer-nav {
            display: flex;
            gap: 1.8rem;
            flex-wrap: wrap;
            justify-content: center;
            font-weight: 600;
        }

        .footer-nav a:hover {
            color: var(--primary);
        }
    "###
}

fn admin_js() -> &'static str {
    r###"
        async function handleAward(e) {
            e.preventDefault();
            const msgEl = document.getElementById('awardMsg');
            const userId = parseInt(document.getElementById('awardUserId').value);
            const badgeId = parseInt(document.getElementById('awardBadgeId').value);
            msgEl.textContent = "Processing...";
            msgEl.style.color = "#94a3b8";

            try {
                const res = await fetch('/api/badges/award', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ user_id: userId, badge_id: badgeId })
                });
                const data = await res.json();
                if (res.ok) {
                    msgEl.textContent = "[Success] Badge awarded successfully! Refresh to see changes.";
                    msgEl.style.color = "#10b981";
                } else {
                    msgEl.textContent = "[Error] " + (data.error || "Failed to award badge");
                    msgEl.style.color = "#ef4444";
                }
            } catch(err) {
                msgEl.textContent = "[Error] Server connection error";
                msgEl.style.color = "#ef4444";
            }
        }

        async function handleCreateBadge(e) {
            e.preventDefault();
            const msgEl = document.getElementById('createBadgeMsg');
            const icon = document.getElementById('newBadgeIcon').value;
            const name = document.getElementById('newBadgeName').value;
            const description = document.getElementById('newBadgeDesc').value;
            msgEl.textContent = "Creating...";
            msgEl.style.color = "#94a3b8";

            try {
                const res = await fetch('/api/badges/create', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ icon, name, description })
                });
                const data = await res.json();
                if (res.ok) {
                    msgEl.textContent = "[Success] Badge created successfully! Refresh to update list.";
                    msgEl.style.color = "#10b981";
                } else {
                    msgEl.textContent = "[Error] " + (data.error || "Failed to create badge");
                    msgEl.style.color = "#ef4444";
                }
            } catch(err) {
                msgEl.textContent = "[Error] Server connection error";
                msgEl.style.color = "#ef4444";
            }
        }
    "###
}

fn login_js() -> &'static str {
    r###"
        async function handleLogin(e) {
            e.preventDefault();
            const msgEl = document.getElementById('loginMsg');
            const username = document.getElementById('loginUser').value.trim();
            const password = document.getElementById('loginPass').value;
            
            // Extract Cloudflare Turnstile token if present
            const turnstileInput = document.querySelector('[name="cf-turnstile-response"]');
            const cf_turnstile_response = turnstileInput ? turnstileInput.value : undefined;

            msgEl.textContent = "Signing in...";
            msgEl.style.color = "#94a3b8";

            try {
                const res = await fetch('/api/login', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ username, password, cf_turnstile_response })
                });
                const data = await res.json();
                if (res.ok && data.success) {
                    msgEl.textContent = "[Success] " + data.message;
                    msgEl.style.color = "#10b981";
                    setTimeout(() => {
                        window.location.href = '/login';
                    }, 500);
                } else {
                    msgEl.textContent = "[Error] " + (data.message || "Login failed.");
                    msgEl.style.color = "#ef4444";
                    // Turnstile token lifecycle: tokens are single-use, reset for retry
                    if (window.turnstile) {
                        try { window.turnstile.reset(); } catch(_) {}
                    }
                }
            } catch(err) {
                msgEl.textContent = "[Error] Server connection error.";
                msgEl.style.color = "#ef4444";
                if (window.turnstile) {
                    try { window.turnstile.reset(); } catch(_) {}
                }
            }
        }
    "###
}


fn profile_js() -> &'static str {
    r###"
        let currentBio = typeof INITIAL_RAW_BIO !== 'undefined' ? INITIAL_RAW_BIO : "";

        function showToast(msg, type = 'success') {
            let toast = document.getElementById('profileToast');
            if (!toast) {
                toast = document.createElement('div');
                toast.id = 'profileToast';
                toast.className = 'profile-toast';
                document.body.appendChild(toast);
            }
            toast.textContent = msg;
            toast.className = 'profile-toast show ' + (type === 'error' ? 'toast-error' : 'toast-success');
            clearTimeout(window.__toastTimer);
            window.__toastTimer = setTimeout(() => {
                toast.className = 'profile-toast';
            }, 3500);
        }

        function parseBioMarkdown(raw) {
            if (!raw || !raw.trim()) {
                return '<p style="color: var(--text-muted); font-style: italic;">No bio written yet. Click \'Edit\' to share something about yourself!</p>';
            }
            if (window.marked && typeof window.marked.parse === 'function') {
                try {
                    return window.marked.parse(raw, { breaks: true, gfm: true });
                } catch(e) {
                    console.warn('marked parse error:', e);
                }
            }
            // Fallback safe markdown parser
            let s = raw.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
            s = s.replace(/```([\s\S]*?)```/g, '<pre><code>$1</code></pre>');
            s = s.replace(/`([^`]+)`/g, '<code>$1</code>');
            s = s.replace(/^### (.*$)/gim, '<h3>$1</h3>');
            s = s.replace(/^## (.*$)/gim, '<h2>$1</h2>');
            s = s.replace(/^# (.*$)/gim, '<h1>$1</h1>');
            s = s.replace(/^\> (.*$)/gim, '<blockquote>$1</blockquote>');
            s = s.replace(/\*\*\*(.*?)\*\*\*/g, '<b><i>$1</i></b>');
            s = s.replace(/\*\*(.*?)\*\*/g, '<b>$1</b>');
            s = s.replace(/\*(.*?)\*/g, '<i>$1</i>');
            s = s.replace(/~~(.*?)~~/g, '<del>$1</del>');
            s = s.replace(/!\[(.*?)\]\((.*?)\)/g, '<img alt="$1" src="$2" style="max-width: 100%; border-radius: 6px; margin: 0.5rem 0;" />');
            s = s.replace(/\[(.*?)\]\((.*?)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer" style="color: #f472b6; text-decoration: underline;">$1</a>');
            s = s.replace(/^\s*-\s+(.*$)/gim, '<li>$1</li>');
            s = s.replace(/\n\n+/g, '</p><p>');
            s = s.replace(/\n/g, '<br>');
            return '<p>' + s + '</p>';
        }

        document.addEventListener('DOMContentLoaded', () => {
            const bioContentEl = document.getElementById('bioContent');
            if (bioContentEl) {
                bioContentEl.innerHTML = parseBioMarkdown(currentBio);
            }
            const bioEditorInput = document.getElementById('bioEditorInput');
            if (bioEditorInput) {
                bioEditorInput.addEventListener('input', updateBioCounter);
            }
        });

        function toggleBioEdit(show) {
            const viewMode = document.getElementById('bioViewMode');
            const editMode = document.getElementById('bioEditMode');
            const editBtn = document.getElementById('btnBioEdit');
            if (show) {
                viewMode.style.display = 'none';
                editMode.style.display = 'block';
                if (editBtn) editBtn.style.display = 'none';
                const textarea = document.getElementById('bioEditorInput');
                textarea.value = currentBio;
                updateBioCounter();
                setEditorTab('write');
                textarea.focus();
            } else {
                viewMode.style.display = 'block';
                editMode.style.display = 'none';
                if (editBtn) editBtn.style.display = 'flex';
            }
        }

        function setEditorTab(tab) {
            const tabWrite = document.getElementById('tabWrite');
            const tabPreview = document.getElementById('tabPreview');
            const editorWrite = document.getElementById('editorWriteArea');
            const editorPreview = document.getElementById('editorPreviewArea');
            if (tab === 'write') {
                tabWrite.classList.add('active');
                tabPreview.classList.remove('active');
                editorWrite.style.display = 'block';
                editorPreview.style.display = 'none';
            } else {
                tabWrite.classList.remove('active');
                tabPreview.classList.add('active');
                editorWrite.style.display = 'none';
                editorPreview.style.display = 'block';
                const val = document.getElementById('bioEditorInput').value;
                editorPreview.innerHTML = parseBioMarkdown(val);
            }
        }

        function insertMarkdown(prefix, suffix, defaultText = 'text') {
            const el = document.getElementById('bioEditorInput');
            const start = el.selectionStart;
            const end = el.selectionEnd;
            const text = el.value;
            const selected = text.substring(start, end) || defaultText;
            const replacement = prefix + selected + suffix;
            el.value = text.substring(0, start) + replacement + text.substring(end);
            el.focus();
            el.setSelectionRange(start + prefix.length, start + prefix.length + selected.length);
            updateBioCounter();
        }

        function updateBioCounter() {
            const el = document.getElementById('bioEditorInput');
            const cnt = document.getElementById('bioCharCount');
            if (el && cnt) cnt.textContent = el.value.length;
        }

        async function saveBioEdit() {
            const newBio = document.getElementById('bioEditorInput').value;
            const btn = document.getElementById('btnSaveBio');
            const origHtml = btn.innerHTML;
            btn.disabled = true;
            btn.textContent = "Saving...";

            try {
                const res = await fetch('/api/profile/update', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ bio: newBio })
                });
                const data = await res.json();
                if (res.ok && data.success) {
                    currentBio = newBio;
                    document.getElementById('bioContent').innerHTML = parseBioMarkdown(currentBio);
                    toggleBioEdit(false);
                    showToast("Bio updated successfully!", "success");
                } else {
                    showToast(data.message || "Failed to save bio.", "error");
                }
            } catch(e) {
                showToast("Connection error while saving bio.", "error");
            } finally {
                btn.disabled = false;
                btn.innerHTML = origHtml;
            }
        }

        async function handleAvatarUpload(e) {
            const file = e.target.files[0];
            if (!file) return;
            if (file.size > 5 * 1024 * 1024) {
                showToast("Avatar file exceeds 5MB limit.", "error");
                return;
            }
            showToast("Uploading avatar...", "info");
            const fd = new FormData();
            fd.append('avatar', file);

            try {
                const res = await fetch('/api/profile/avatar', {
                    method: 'POST',
                    body: fd
                });
                const data = await res.json();
                if (res.ok && data.success) {
                    const v = Date.now();
                    document.querySelectorAll('img[src*="/a/"]').forEach(img => {
                        img.src = img.src.split('?')[0] + '?v=' + v;
                    });
                    showToast("Avatar updated successfully!", "success");
                } else {
                    showToast(data.message || "Failed to upload avatar.", "error");
                }
            } catch(err) {
                showToast("Connection error uploading avatar.", "error");
            } finally {
                e.target.value = '';
            }
        }

        async function handleAvatarReset(e) {
            if (e) e.preventDefault();
            if (!confirm("Reset avatar to default (Marisa)?")) return;
            showToast("Resetting avatar...", "info");
            try {
                const res = await fetch('/api/profile/avatar/reset', { method: 'POST' });
                const data = await res.json();
                if (res.ok && data.success) {
                    const v = Date.now();
                    document.querySelectorAll('img[src*="/a/"]').forEach(img => {
                        img.src = img.src.split('?')[0] + '?v=' + v;
                    });
                    showToast("Avatar reset to default!", "success");
                } else {
                    showToast(data.message || "Failed to reset avatar.", "error");
                }
            } catch(err) {
                showToast("Connection error resetting avatar.", "error");
            }
        }

        async function handleBannerUpload(e) {
            const file = e.target.files[0];
            if (!file) return;
            if (file.size > 10 * 1024 * 1024) {
                showToast("Banner image exceeds 10MB limit.", "error");
                return;
            }
            showToast("Uploading banner...", "info");
            const fd = new FormData();
            fd.append('banner', file);

            try {
                const res = await fetch('/api/profile/banner', {
                    method: 'POST',
                    body: fd
                });
                const data = await res.json();
                if (res.ok && data.success) {
                    const v = Date.now();
                    const cover = document.getElementById('profileCover');
                    if (cover) {
                        const uid = cover.dataset.userId;
                        cover.style.backgroundImage = `linear-gradient(180deg, rgba(15, 23, 42, 0.25) 0%, rgba(15, 23, 42, 0.75) 55%, rgba(15, 23, 42, 0.96) 100%), url('/b/${uid}?v=${v}')`;
                    }
                    showToast("Banner updated successfully!", "success");
                } else {
                    showToast(data.message || "Failed to upload banner.", "error");
                }
            } catch(err) {
                showToast("Connection error uploading banner.", "error");
            } finally {
                e.target.value = '';
            }
        }

        async function handleBannerReset(e) {
            if (e) e.preventDefault();
            if (!confirm("Reset profile cover banner to default?")) return;
            showToast("Resetting banner...", "info");
            try {
                const res = await fetch('/api/profile/banner/reset', { method: 'POST' });
                const data = await res.json();
                if (res.ok && data.success) {
                    const v = Date.now();
                    const cover = document.getElementById('profileCover');
                    if (cover) {
                        const uid = cover.dataset.userId;
                        cover.style.backgroundImage = `linear-gradient(180deg, rgba(15, 23, 42, 0.25) 0%, rgba(15, 23, 42, 0.75) 55%, rgba(15, 23, 42, 0.96) 100%), url('/b/${uid}?v=${v}')`;
                    }
                    showToast("Banner reset to default!", "success");
                } else {
                    showToast(data.message || "Failed to reset banner.", "error");
                }
            } catch(err) {
                showToast("Connection error resetting banner.", "error");
            }
        }

        function openCountryModal() {
            const modal = document.getElementById('countryModal');
            if (modal) modal.style.display = 'flex';
        }

        function closeCountryModal() {
            const modal = document.getElementById('countryModal');
            if (modal) modal.style.display = 'none';
        }

        async function saveCountryChange() {
            const select = document.getElementById('countryModalSelect');
            if (!select) return;
            const cid = select.value;
            const opt = select.options[select.selectedIndex];
            const flag = opt ? opt.getAttribute('data-flag') || '' : '';
            const code = opt ? opt.getAttribute('data-code') || '' : '';
            const name = opt ? opt.getAttribute('data-name') || '' : '';

            closeCountryModal();
            showToast("Updating country...", "info");

            try {
                const res = await fetch('/api/profile/update', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ country: cid })
                });
                const data = await res.json();
                if (res.ok && data.success) {
                    const el = document.getElementById('countryDisplayTxt');
                    if (el) el.textContent = `${flag} ${name} (${code})`;
                    showToast("Country updated successfully!", "success");
                } else {
                    showToast(data.message || "Failed to update country.", "error");
                }
            } catch(err) {
                showToast("Connection error updating country.", "error");
            }
        }
    "###
}


pub async fn get_authenticated_user(state: &AppState, headers: &HeaderMap) -> Option<User> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    for cookie in cookie_header.split(';') {
        let mut parts = cookie.trim().splitn(2, '=');
        if let (Some(name), Some(val)) = (parts.next(), parts.next()) {
            if name == "ayanomi_session" {
                let user_id = val.split('.').next()?.parse::<i32>().ok()?;
                if let Ok(Some(user)) = get_user_by_id(&state.db, user_id).await {
                    if verify_session(val, &user.password_hash, &state.config.server.admin_key).is_some() {
                        return Some(user);
                    }
                }
            }
        }
    }
    None
}

fn render_navbar(active: &str, server_name: &str, user: Option<&User>) -> String {
    let is_home = if active == "home" { "class='active'" } else { "" };
    let is_lb = if active == "leaderboard" { "class='active'" } else { "" };
    let is_multi = if active == "multi" { "class='active'" } else { "" };
    let is_rule = if active == "rule" { "class='active'" } else { "" };
    let is_staff = if active == "staff" { "class='active'" } else { "" };
    let is_connect = if active == "connect" { "class='active'" } else { "" };
    let is_login = if active == "login" { "class='active'" } else { "" };

    let actions = match user {
        Some(u) => {
            format!(
                r###"<div class="nav-actions" style="display: flex; align-items: center; gap: 0.75rem;">
                    <a href="/login" style="display: flex; align-items: center; gap: 0.5rem; text-decoration: none; color: var(--text-main); font-weight: 700; padding: 4px 10px; border-radius: 6px; background: var(--bg-surface-hover); border: 1px solid var(--card-border);">
                        <img src="/a/{id}" style="width: 26px; height: 26px; border-radius: 50%; object-fit: cover; border: 1px solid var(--card-border);">
                        <span>{name}</span>
                    </a>
                    <a href="/logout" onclick="handleLogout(event)" class="btn btn-outline" style="padding: 0.4rem 0.8rem; font-size: 0.85rem; color: var(--rose); border-color: var(--rose);">Sign Out</a>
                </div>"###,
                id = u.id,
                name = html_escape(&u.username)
            )
        }
        None => {
            r###"<div class="nav-actions">
                <a href="/login" class="btn btn-primary">Sign In</a>
            </div>"###.to_string()
        }
    };

    format!(
        r###"<nav>
            <div class="nav-container">
                <a href="/" class="nav-brand" style="display: flex; align-items: center;">
                    <img src="/static/logo.png" alt="{server_name}" style="height: 38px; width: auto; max-width: 220px; object-fit: contain; filter: drop-shadow(0 2px 6px rgba(0,0,0,0.4));">
                </a>
                <ul class="nav-links">
                    <li><a href="/" {is_home}>Home</a></li>
                    <li><a href="/leaderboard" {is_lb}>Leaderboard</a></li>
                    <li><a href="/multi" {is_multi}>Multiplayer</a></li>
                    <li><a href="/rule" {is_rule}>Rules</a></li>
                    <li><a href="/staff" {is_staff}>Staff & Credits</a></li>
                    <li><a href="/connect" {is_connect}>Connect</a></li>
                    <li><a href="/login" {is_login}>Account</a></li>
                </ul>
                {actions}
            </div>
        </nav>"###,
        server_name = server_name,
        is_home = is_home,
        is_lb = is_lb,
        is_multi = is_multi,
        is_rule = is_rule,
        is_staff = is_staff,
        is_connect = is_connect,
        is_login = is_login,
        actions = actions
    )
}

fn render_footer() -> String {
    r###"<footer>
        <div class="footer-content">
            <div style="display: flex; align-items: center; justify-content: center; margin-bottom: 0.6rem;">
                <img src="/static/logo.png" alt="AyanomiBancho" style="height: 32px; width: auto; object-fit: contain; opacity: 0.85;">
            </div>
            <div style="font-weight: 500;">
                © 2026 <b style="color: var(--text-main);">AyanomiBancho</b> • Very low-cost osu! Private Server • RustLang in ~50hr
            </div>
            <div class="footer-nav">
                <a href="/">Home</a>
                <a href="/leaderboard">Leaderboard</a>
                <a href="/multi">Multiplayer</a>
                <a href="/rule">Rules</a>
                <a href="/staff">Staff & Credits</a>
                <a href="/connect">How to Connect</a>
                <a href="/admin" style="color: var(--text-sub); font-size: 0.85rem;">Admin Panel</a>
            </div>
        </div>
    </footer>
    <script>
        async function handleLogout(e) {
            if (e) e.preventDefault();
            try {
                await fetch('/api/logout', { method: 'POST' });
            } catch (_) {}
            document.cookie = "ayanomi_session=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=0";
            window.location.href = "/login";
        }
    </script>"###.to_string()
}

// -------------------------------------------------------------------------------------------------
// 1. Player-Facing Homepage (GET /)
// -------------------------------------------------------------------------------------------------

pub async fn index_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Html<String> {
    let current_user = get_authenticated_user(&state, &headers).await;
    let online_count = { state.bancho.read().await.online_count() };
    let total_users = count_users(&state.db).await.unwrap_or(0);
    let total_scores = count_scores(&state.db).await.unwrap_or(0);

    // Online players
    let raw_sessions: Vec<(i32, String, String, &'static str)> = {
        let st = state.bancho.read().await;
        st.sessions
            .values()
            .map(|s| {
                let status = if s.info_text.is_empty() {
                    "In Menus".to_string()
                } else {
                    s.info_text.clone()
                };
                let country = bancho_id_to_country(s.country_code);
                (s.user_id, s.username.clone(), status, country.code)
            })
            .collect()
    };

    let mut players_html = String::new();
    if raw_sessions.is_empty() {
        players_html.push_str("<p style='color: var(--text-muted); font-style: italic; padding: 1rem;'>No players currently online. Start osu! and join now!</p>");
    } else {
        players_html.push_str(r#"<div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(270px, 1fr)); gap: 1rem;">"#);
        for (user_id, name, status, country_code) in raw_sessions {
            let clean_name = crate::db::badges::clean_username(&name);
            let badges = crate::db::badges::get_user_badges(&state.badges_db, user_id).await.unwrap_or_default();
            let user_badge_tag = badges.iter().find_map(|b| {
                let t = b.tag.trim();
                if !t.is_empty() { Some((t.to_string(), b.name.clone())) } else { None }
            });

            let prefix_tag = if let Some((ref tag, ref bname)) = user_badge_tag {
                format!(r#"<span class="country-tag" style="color: #f472b6; background: rgba(244, 114, 182, 0.18); border: none; font-weight: 700;" title="{}">[{}]</span>"#, html_escape(bname), html_escape(tag))
            } else {
                format!(r#"<span class="country-tag">[{}]</span>"#, country_code)
            };

            players_html.push_str(&format!(
                r###"<div class="player-pill-card">
                    <a href="/u/{user_id}" class="player-avatar-wrapper">
                        <img src="/a/{user_id}" class="player-avatar-img" alt="{clean_name}">
                        <span class="online-dot-badge"></span>
                    </a>
                    <div style="flex: 1; overflow: hidden;">
                        <div style="font-weight: 700; display: flex; align-items: center; gap: 0.4rem; flex-wrap: wrap;">
                            {prefix_tag}
                            <a href="/u/{user_id}" style="color: var(--text-main);">{clean_name}</a>
                        </div>
                        <div style="font-size: 0.8rem; color: var(--text-muted); margin-top: 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">{status}</div>
                    </div>
                </div>"###,
                user_id = user_id, prefix_tag = prefix_tag, clean_name = html_escape(clean_name), status = html_escape(&status)
            ));
        }
        players_html.push_str("</div>");
    }

    // Top 5 rankers
    let top_rankers = get_leaderboard(&state.db, 0, 5).await.unwrap_or_default();
    let mut rankers_html = String::new();
    if top_rankers.is_empty() {
        rankers_html.push_str("<p style='color: var(--text-muted); font-style: italic; padding: 1rem;'>No ranked players yet. Submit your first score to climb the leaderboard!</p>");
    } else {
        rankers_html.push_str(r###"<div style="overflow-x: auto;"><table class="modern-table">
            <thead>
                <tr>
                    <th style="width: 80px;">Rank</th>
                    <th>Player</th>
                    <th style="text-align: right;">Performance Points</th>
                    <th style="text-align: right;">Accuracy</th>
                    <th style="text-align: right;">Ranked Score</th>
                </tr>
            </thead>
            <tbody>"###);
        for u in top_rankers {
            let rank_badge = match u.rank {
                1 => "<span class='rank-box rank-gold'>#1</span>",
                2 => "<span class='rank-box rank-silver'>#2</span>",
                3 => "<span class='rank-box rank-bronze'>#3</span>",
                _ => "",
            };
            let rank_display = if !rank_badge.is_empty() {
                rank_badge.to_string()
            } else {
                format!("<span class='rank-box' style='color: var(--text-muted);'>#{}</span>", u.rank)
            };
            let country = bancho_id_to_country(u.country);
            let badges = crate::db::badges::get_user_badges(&state.badges_db, u.user_id).await.unwrap_or_default();
            let user_badge_tag = badges.iter().find_map(|b| {
                let t = b.tag.trim();
                if !t.is_empty() { Some((t.to_string(), b.name.clone())) } else { None }
            });

            let prefix_tag = if let Some((ref tag, ref bname)) = user_badge_tag {
                format!(r#"<span class="country-tag" style="color: #f472b6; background: rgba(244, 114, 182, 0.18); border: none; font-weight: 700;" title="{}">[{}]</span>"#, html_escape(bname), html_escape(tag))
            } else {
                format!(r#"<span class="country-tag">[{}]</span>"#, country.code)
            };
            let clean_name = crate::db::badges::clean_username(&u.username);
            rankers_html.push_str(&format!(
                r###"<tr>
                    <td>{rank_display}</td>
                    <td>
                        <div style="display: flex; align-items: center; gap: 0.8rem;">
                            <a href="/u/{id}"><img src="/a/{id}" style="width: 36px; height: 36px; border-radius: 50%; object-fit: cover; border: 1px solid var(--card-border);" alt="{username}"></a>
                            <div style="display: flex; align-items: center; gap: 0.4rem; flex-wrap: wrap;">
                                {prefix_tag}
                                <a href="/u/{id}" style="font-weight: 700; color: var(--text-main); font-size: 0.95rem;">{username}</a>
                            </div>
                        </div>
                    </td>
                    <td style="text-align: right;"><div class="pp-highlight">{pp}<span>pp</span></div></td>
                    <td style="text-align: right; font-weight: 600; color: var(--text-muted);">{acc:.2}%</td>
                    <td style="text-align: right; font-weight: 600; font-family: 'JetBrains Mono', monospace; color: #cbd5e1;">{score}</td>
                </tr>"###,
                rank_display = rank_display,
                id = u.user_id,
                username = html_escape(clean_name),
                prefix_tag = prefix_tag,
                pp = format_number(u.pp as i64),
                acc = u.accuracy,
                score = format_number(u.ranked_score)
            ));
        }
        rankers_html.push_str("</tbody></table></div>");
    }

    // Recent matches
    let recent_matches = get_recent_matches(&state.db, 5).await.unwrap_or_default();
    let mut matches_html = String::new();
    if recent_matches.is_empty() {
        matches_html.push_str("<p style='color: var(--text-muted); font-style: italic; padding: 1rem;'>No multiplayer matches have completed yet.</p>");
    } else {
        matches_html.push_str(r###"<div style="overflow-x: auto;"><table class="modern-table">
            <thead>
                <tr>
                    <th>Match</th>
                    <th>Beatmap</th>
                    <th>Winner</th>
                    <th style="text-align: right;">Duration</th>
                </tr>
            </thead>
            <tbody>"###);
        for m in recent_matches {
            let duration_min = m.duration_seconds / 60;
            let duration_sec = m.duration_seconds % 60;
            let winner_display = if m.winner_name.is_empty() {
                "-".to_string()
            } else {
                m.winner_name
            };
            matches_html.push_str(&format!(
                r###"<tr>
                    <td style="font-weight: 700; color: var(--text-main);">#{id} {name}</td>
                    <td style="color: var(--primary); font-weight: 600;">{bname}</td>
                    <td style="color: var(--emerald); font-weight: 700;">{winner}</td>
                    <td style="text-align: right; color: var(--text-muted); font-family: monospace;">{min:02}:{sec:02}</td>
                </tr>"###,
                id = m.id,
                name = html_escape(&m.name),
                bname = html_escape(&m.beatmap_name),
                winner = html_escape(&winner_display),
                min = duration_min,
                sec = duration_sec
            ));
        }
        matches_html.push_str("</tbody></table></div>");
    }

    let navbar = render_navbar("home", &state.config.server.name, current_user.as_ref());
    let footer = render_footer();

    let html = format!(
        r###"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="icon" type="image/png" href="/static/favicon.png">
    <link rel="shortcut icon" href="/favicon.ico">
    <title>{name} - osu! Private Server</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@400;500;600;700;800;900&family=JetBrains+Mono:wght@500;700&display=swap" rel="stylesheet">
    <style>{css}</style>
</head>
<body>
    {navbar}

    <main class="main-container">
        <!-- Hero Section -->
        <section class="hero-banner">
            <div style="text-align: center; margin-bottom: 1.4rem;">
                <img src="/static/logo.png" alt="{name}" style="max-width: 480px; width: 85%; height: auto; filter: drop-shadow(0 4px 20px rgba(255, 107, 0, 0.25));">
            </div>
            <h1 class="hero-title" style="max-width: 900px;">
                <span style="color: var(--primary);">{name}</span> — Zero delay. <span style="white-space: nowrap;">Real-time</span> ranks. Just play.
            </h1>
            <div class="hero-btns">
                <a href="/connect" class="btn btn-primary" style="padding: 0.75rem 1.6rem; font-size: 1rem;">How to Connect</a>
                <a href="/leaderboard" class="btn btn-outline" style="padding: 0.75rem 1.6rem; font-size: 1rem;">View Leaderboards</a>
            </div>
        </section>

        <!-- Stats Counter Strip -->
        <div class="hero-stats-strip">
            <div class="stat-box">
                <div class="stat-lbl">Online Players</div>
                <div class="stat-val" style="color: var(--emerald);">{online_count}</div>
            </div>
            <div class="stat-box">
                <div class="stat-lbl">Registered Members</div>
                <div class="stat-val" style="color: var(--primary);">{total_users}</div>
            </div>
            <div class="stat-box">
                <div class="stat-lbl">Total Plays</div>
                <div class="stat-val" style="color: var(--cyan);">{total_scores}</div>
            </div>
            <div class="stat-box">
                <div class="stat-lbl">Server Status</div>
                <div class="stat-val" style="color: var(--emerald); font-size: 1.4rem;">Online</div>
            </div>
        </div>

        <!-- Online Players -->
        <div class="glass-card">
            <div class="card-header-bar">
                <div class="card-heading">Online Players</div>
                <span style="color: var(--emerald); font-weight: 600; font-size: 0.88rem;">{online_count} players online</span>
            </div>
            {players_html}
        </div>

        <!-- Leaderboard Preview -->
        <div class="glass-card">
            <div class="card-header-bar">
                <div class="card-heading">Top 5 osu! Standard Rankings</div>
                <a href="/leaderboard" class="btn btn-outline" style="padding: 0.45rem 0.9rem; font-size: 0.85rem;">Full Leaderboard &rarr;</a>
            </div>
            {rankers_html}
        </div>

        <!-- Recent Matches -->
        <div class="glass-card">
            <div class="card-header-bar">
                <div class="card-heading">Recent Multiplayer Matches</div>
            </div>
            {matches_html}
        </div>

        <!-- Connect Callout Banner -->
        <div class="glass-card" style="display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 1.5rem; padding: 2rem;">
            <div>
                <div class="card-heading" style="margin-bottom: 0.4rem;">Ready to Join the Game?</div>
                <p style="color: var(--text-muted); font-size: 0.95rem; margin: 0;">Connect your osu! client in less than 2 minutes with -devserver. No registration hassle!</p>
            </div>
            <a href="/connect" class="btn btn-primary" style="padding: 0.75rem 1.6rem; font-size: 1rem;">View Connection Guide &rarr;</a>
        </div>
    </main>

    {footer}
</body>
</html>"###,
        name = state.config.server.name,
        online_count = online_count,
        total_users = format_number(total_users),
        total_scores = format_number(total_scores),
        players_html = players_html,
        rankers_html = rankers_html,
        matches_html = matches_html,
        navbar = navbar,
        footer = footer,
        css = common_css()
    );

    Html(html)
}

// -------------------------------------------------------------------------------------------------
// 2. Leaderboard Page (GET /leaderboard)
// -------------------------------------------------------------------------------------------------

pub async fn leaderboard_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<LeaderboardQuery>,
) -> Html<String> {
    let current_user = get_authenticated_user(&state, &headers).await;
    let mode = params.m.unwrap_or(0).min(6);
    let mode_name = match mode {
        0 => "osu! Standard",
        1 => "osu!taiko",
        2 => "osu!catch",
        3 => "osu!mania",
        4 => "osu! Standard (Relax)",
        5 => "osu!taiko (Relax)",
        6 => "osu!catch (Relax)",
        _ => "osu!",
    };

    let users = get_leaderboard(&state.db, mode, 50).await.unwrap_or_default();
    let mut table_rows = String::new();

    if users.is_empty() {
        table_rows.push_str("<tr><td colspan='6' style='text-align: center; color: var(--text-muted); padding: 3rem;'>No scores recorded for this game mode yet.</td></tr>");
    } else {
        for u in users {
            let rank_badge = match u.rank {
                1 => "<span class='rank-box rank-gold'>#1</span>",
                2 => "<span class='rank-box rank-silver'>#2</span>",
                3 => "<span class='rank-box rank-bronze'>#3</span>",
                _ => "",
            };
            let rank_display = if !rank_badge.is_empty() {
                rank_badge.to_string()
            } else {
                format!("<span class='rank-box' style='color: var(--text-muted);'>#{}</span>", u.rank)
            };
            let country = bancho_id_to_country(u.country);
            let badges = crate::db::badges::get_user_badges(&state.badges_db, u.user_id).await.unwrap_or_default();
            let user_badge_tag = badges.iter().find_map(|b| {
                let t = b.tag.trim();
                if !t.is_empty() { Some((t.to_string(), b.name.clone())) } else { None }
            });

            let prefix_tag = if let Some((ref tag, ref bname)) = user_badge_tag {
                format!(r#"<span class="country-tag" style="color: #f472b6; background: rgba(244, 114, 182, 0.18); border: none; font-weight: 700;" title="{}">[{}]</span>"#, html_escape(bname), html_escape(tag))
            } else {
                format!(r#"<span class="country-tag">[{}]</span>"#, country.code)
            };
            let clean_name = crate::db::badges::clean_username(&u.username);

            table_rows.push_str(&format!(
                r###"<tr>
                    <td>{rank_display}</td>
                    <td>
                        <div style="display: flex; align-items: center; gap: 0.8rem;">
                            <a href="/u/{id}"><img src="/a/{id}" style="width: 36px; height: 36px; border-radius: 50%; object-fit: cover; border: 1px solid var(--card-border);" alt="{username}"></a>
                            <div style="display: flex; align-items: center; gap: 0.4rem; flex-wrap: wrap;">
                                {prefix_tag}
                                <a href="/u/{id}" style="font-weight: 700; color: var(--text-main); font-size: 0.95rem;">{username}</a>
                            </div>
                        </div>
                    </td>
                    <td style="text-align: right;"><div class="pp-highlight">{pp}<span>pp</span></div></td>
                    <td style="text-align: right; font-weight: 600; color: var(--text-muted);">{acc:.2}%</td>
                    <td style="text-align: right; font-weight: 600; font-family: 'JetBrains Mono', monospace; color: #cbd5e1;">{score}</td>
                    <td style="text-align: right; color: var(--text-muted); font-weight: 600;">{plays}</td>
                </tr>"###,
                rank_display = rank_display,
                id = u.user_id,
                username = html_escape(clean_name),
                prefix_tag = prefix_tag,
                pp = format_number(u.pp as i64),
                acc = u.accuracy,
                score = format_number(u.ranked_score),
                plays = format_number(u.play_count as i64)
            ));
        }
    }

    let tab_btn = |m: u8, name: &str| -> String {
        let active_cls = if m == mode { "btn-primary" } else { "btn-outline" };
        format!(r#"<a href="/leaderboard?m={}" class="btn {}" style="padding: 0.5rem 1.1rem;">{}</a>"#, m, active_cls, name)
    };

    let navbar = render_navbar("leaderboard", &state.config.server.name, current_user.as_ref());
    let footer = render_footer();

    let html = format!(
        r###"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="icon" type="image/png" href="/static/favicon.png">
    <link rel="shortcut icon" href="/favicon.ico">
    <title>Leaderboard - {name}</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@400;500;600;700;800;900&family=JetBrains+Mono:wght@500;700&display=swap" rel="stylesheet">
    <style>{css}</style>
</head>
<body>
    {navbar}

    <main class="main-container">
        <div style="display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 1.2rem;">
            <div>
                <h1 style="font-size: 2rem; font-weight: 800; letter-spacing: -0.5px;">Global Leaderboard</h1>
                <p style="color: var(--text-muted); font-size: 0.95rem; margin-top: 0.3rem;">Game Mode: <b style="color: var(--primary);">{mode_name}</b></p>
            </div>
            <div style="display: flex; gap: 0.6rem; flex-wrap: wrap;">
                {tab_std}
                {tab_taiko}
                {tab_ctb}
                {tab_mania}
                <span style="border-left: 1px solid var(--card-border); margin: 0 0.3rem;"></span>
                {tab_rx_std}
                {tab_rx_taiko}
                {tab_rx_ctb}
            </div>
        </div>

        <div class="glass-card" style="padding: 1.2rem;">
            <div style="overflow-x: auto;">
                <table class="modern-table">
                    <thead>
                        <tr>
                            <th style="width: 80px;">Rank</th>
                            <th>Player</th>
                            <th style="text-align: right;">PP</th>
                            <th style="text-align: right;">Accuracy</th>
                            <th style="text-align: right;">Ranked Score</th>
                            <th style="text-align: right;">Play Count</th>
                        </tr>
                    </thead>
                    <tbody>
                        {table_rows}
                    </tbody>
                </table>
            </div>
        </div>
    </main>

    {footer}
</body>
</html>"###,
        name = state.config.server.name,
        mode_name = mode_name,
        tab_std = tab_btn(0, "Standard"),
        tab_taiko = tab_btn(1, "Taiko"),
        tab_ctb = tab_btn(2, "Catch"),
        tab_mania = tab_btn(3, "Mania"),
        tab_rx_std = tab_btn(4, "RX Std"),
        tab_rx_taiko = tab_btn(5, "RX Taiko"),
        tab_rx_ctb = tab_btn(6, "RX Catch"),
        table_rows = table_rows,
        navbar = navbar,
        footer = footer,
        css = common_css()
    );

    Html(html)
}

// -------------------------------------------------------------------------------------------------
// 3. Player Profile Page (GET /u/{id})
// -------------------------------------------------------------------------------------------------

pub async fn profile_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i32>,
) -> Html<String> {
    let current_user = get_authenticated_user(&state, &headers).await;
    let user = match get_user_by_id(&state.db, user_id).await {
        Ok(Some(u)) => u,
        _ => {
            let not_found_html = format!(
                r###"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8"><link rel="icon" type="image/png" href="/static/favicon.png"><link rel="shortcut icon" href="/favicon.ico"><title>Player Not Found</title><style>{css}</style></head>
                <body>{nav}<main class="main-container" style="text-align: center; padding: 6rem 1rem;">
                <h1 style="font-size: 2.8rem; font-weight: 900; color: var(--rose);">404</h1>
                <p style="color: var(--text-muted); margin-top: 0.8rem; font-size: 1.1rem;">Player with ID #{id} does not exist on this server.</p>
                <a href="/leaderboard" class="btn btn-primary" style="margin-top: 1.8rem;">View Leaderboard</a>
                </main>{footer}</body></html>"###,
                css = common_css(),
                nav = render_navbar("", &state.config.server.name, current_user.as_ref()),
                footer = render_footer(),
                id = user_id
            );
            return Html(not_found_html);
        }
    };

    let is_owner = current_user.as_ref().map(|u| u.id == user.id).unwrap_or(false);
    let country = bancho_id_to_country(user.country);
    let rank_std = get_user_rank(&state.db, user_id, 0).await.unwrap_or(1);
    let stats_std = get_or_create_stats(&state.db, user_id, 0).await.unwrap_or_default();
    let stats_taiko = get_or_create_stats(&state.db, user_id, 1).await.unwrap_or_default();
    let stats_ctb = get_or_create_stats(&state.db, user_id, 2).await.unwrap_or_default();
    let stats_mania = get_or_create_stats(&state.db, user_id, 3).await.unwrap_or_default();

    let raw_bio_json = serde_json::to_string(&user.bio).unwrap_or_else(|_| "\"\"".to_string());

    let mut country_modal_options = String::new();
    if is_owner {
        for c in crate::utils::country::COUNTRIES {
            let selected = if c.bancho_id == user.country { "selected" } else { "" };
            country_modal_options.push_str(&format!(
                r#"<option value="{}" data-flag="{}" data-code="{}" data-name="{}" {}>{} {} ({})</option>"#,
                c.bancho_id, c.flag, c.code, c.name, selected, c.flag, c.name, c.code
            ));
        }
    }

    let avatar_ver = Utc::now().timestamp_millis();
    let banner_ver = Utc::now().timestamp_millis();

    let (cover_actions_top, avatar_overlay, country_badge_html, bio_edit_btn, bio_edit_section, country_modal_html) = if is_owner {
        let cover_act = format!(
            r###"<div class="cover-actions-top">
                <label class="btn-cover-action" for="bannerFileInput" title="Upload cover banner (PNG, JPG, WebP up to 10MB)">
                    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z"/><circle cx="12" cy="13" r="4"/></svg>
                    <span>Change Banner</span>
                </label>
                <button type="button" onclick="handleBannerReset(event)" class="btn-cover-action btn-cover-reset" title="Reset banner to default">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/></svg>
                    <span>Reset</span>
                </button>
                <input type="file" id="bannerFileInput" accept=".png,.jpg,.jpeg,.webp" style="display: none;" onchange="handleBannerUpload(event)">
            </div>"###
        );

        let av_ov = format!(
            r###"<label for="avatarFileInput" class="avatar-hover-overlay" title="Click to change avatar">
                <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z"/><circle cx="12" cy="13" r="4"/></svg>
                <span style="font-size: 0.72rem; font-weight: 700; margin-top: 2px;">Change</span>
            </label>
            <input type="file" id="avatarFileInput" accept=".png,.jpg,.jpeg,.webp" style="display: none;" onchange="handleAvatarUpload(event)">"###
        );

        let c_badge = format!(
            r###"<button type="button" onclick="openCountryModal()" class="country-edit-badge" title="Click to change country">
                <span id="countryDisplayTxt">{} {} ({})</span>
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 20h9"/><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>
            </button>
            <button type="button" onclick="handleAvatarReset(event)" class="btn-subtle-reset" title="Reset Avatar to default Marisa">Reset Avatar</button>"###,
            country.flag, country.name, country.code
        );

        let b_btn = format!(
            r###"<button id="btnBioEdit" type="button" onclick="toggleBioEdit(true)" class="btn btn-outline" style="font-size: 0.85rem; padding: 0.35rem 0.85rem; color: #f472b6; border-color: rgba(244,114,182,0.4); display: flex; align-items: center; gap: 0.4rem;">
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 20h9"/><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>
                <span>Edit</span>
            </button>"###
        );

        let b_sec = format!(
            r###"<div id="bioEditMode" style="display: none; margin-top: 0.5rem;">
                <div style="display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid var(--card-border); padding-bottom: 0.6rem; margin-bottom: 0.8rem; flex-wrap: wrap; gap: 0.5rem;">
                    <div style="display: flex; gap: 0.4rem;">
                        <button id="tabWrite" type="button" class="editor-tab-btn active" onclick="setEditorTab('write')">Write</button>
                        <button id="tabPreview" type="button" class="editor-tab-btn" onclick="setEditorTab('preview')">Preview</button>
                    </div>
                    <div style="display: flex; gap: 0.3rem; align-items: center; flex-wrap: wrap;">
                        <button type="button" class="editor-fmt-btn" onclick="insertMarkdown('**', '**', 'bold text')" title="Bold"><b>B</b></button>
                        <button type="button" class="editor-fmt-btn" onclick="insertMarkdown('*', '*', 'italic text')" title="Italic"><i>I</i></button>
                        <button type="button" class="editor-fmt-btn" onclick="insertMarkdown('### ', '', 'Heading')" title="Heading"><b>H</b></button>
                        <button type="button" class="editor-fmt-btn" onclick="insertMarkdown('> ', '', 'Quote')" title="Quote"><b>&ldquo;</b></button>
                        <button type="button" class="editor-fmt-btn" onclick="insertMarkdown('```\n', '\n```', 'code here')" title="Code Block"><code>&lt;&gt;</code></button>
                        <button type="button" class="editor-fmt-btn" onclick="insertMarkdown('[', '](https://example.com)', 'Link text')" title="Link"><b>Link</b></button>
                        <button type="button" class="editor-fmt-btn" onclick="insertMarkdown('![alt text](', ')', 'https://example.com/image.png')" title="Image"><b>Image</b></button>
                        <span style="font-size: 0.82rem; color: var(--text-muted); margin-left: 0.8rem;"><span id="bioCharCount">0</span>/2000</span>
                    </div>
                </div>

                <div id="editorWriteArea">
                    <textarea id="bioEditorInput" class="input-glass" rows="9" maxlength="2000" placeholder="Write something about yourself in Markdown... (Headings, bold, italic, quotes, links, images, code)" style="width: 100%; font-family: 'JetBrains Mono', monospace; font-size: 0.92rem; line-height: 1.6; resize: vertical; margin-bottom: 0.8rem; box-sizing: border-box;"></textarea>
                </div>

                <div id="editorPreviewArea" style="display: none; min-height: 180px; padding: 1.2rem; background: var(--bg-surface-hover); border: 1px solid var(--card-border); border-radius: 6px; margin-bottom: 0.8rem;"></div>

                <div style="display: flex; align-items: center; gap: 0.6rem;">
                    <button type="button" id="btnSaveBio" onclick="saveBioEdit()" class="btn btn-primary" style="padding: 0.5rem 1.4rem; font-size: 0.9rem; display: inline-flex; align-items: center; gap: 0.4rem;">
                        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="20 6 9 17 4 12"/></svg>
                        <span>Save Changes</span>
                    </button>
                    <button type="button" onclick="toggleBioEdit(false)" class="btn btn-outline" style="padding: 0.5rem 1.2rem; font-size: 0.9rem;">Cancel</button>
                </div>
            </div>"###
        );

        let modal = format!(
            r###"<div id="countryModal" class="modal-overlay" onclick="if(event.target===this)closeCountryModal()">
                <div class="modal-card">
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.2rem;">
                        <h3 style="font-size: 1.2rem; font-weight: 800; margin: 0;">Change Country Flag</h3>
                        <button type="button" onclick="closeCountryModal()" style="background: transparent; border: none; color: var(--text-muted); font-size: 1.4rem; cursor: pointer; line-height: 1;">&times;</button>
                    </div>
                    <p style="color: var(--text-muted); font-size: 0.88rem; margin-bottom: 1rem;">Select your country flag to display on your profile and global rankings:</p>
                    <select id="countryModalSelect" class="input-glass" style="width: 100%; margin-bottom: 1.4rem;">
                        {}
                    </select>
                    <div style="display: flex; justify-content: flex-end; gap: 0.6rem;">
                        <button type="button" onclick="closeCountryModal()" class="btn btn-outline" style="padding: 0.5rem 1.2rem; font-size: 0.9rem;">Cancel</button>
                        <button type="button" onclick="saveCountryChange()" class="btn btn-primary" style="padding: 0.5rem 1.4rem; font-size: 0.9rem;">Save Country</button>
                    </div>
                </div>
            </div>"###,
            country_modal_options
        );

        (cover_act, av_ov, c_badge, b_btn, b_sec, modal)
    } else {
        let c_badge = format!(
            r#"<span class="country-tag" style="font-size: 0.9rem; padding: 3px 8px;">{} {} ({})</span>"#,
            country.flag, country.name, country.code
        );
        (String::new(), String::new(), c_badge, String::new(), String::new(), String::new())
    };

    let profile_script = format!(
        r###"<script src="https://cdn.jsdelivr.net/npm/marked/marked.min.js"></script>
        <script>
        const INITIAL_RAW_BIO = {raw_bio_json};
        {js}
        </script>"###,
        raw_bio_json = raw_bio_json,
        js = profile_js()
    );

    // Badges collection
    let badges = crate::db::badges::get_user_badges(&state.badges_db, user_id).await.unwrap_or_default();
    let mut badges_html = String::new();
    if badges.is_empty() {
        badges_html.push_str("<p style='color: var(--text-muted); font-style: italic;'>This player has not earned any badges yet. Participate in tournaments or server events to unlock!</p>");
    } else {
        badges_html.push_str(r#"<div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 0.9rem;">"#);
        for b in &badges {
            let tag_display = if !b.tag.trim().is_empty() {
                format!("[{}] {}", b.tag.trim(), b.name)
            } else {
                format!("[{}]", b.name)
            };
            badges_html.push_str(&format!(
                r###"<div style="background: var(--bg-surface-hover); border: 1px solid var(--card-border); border-radius: 6px; padding: 0.9rem;">
                    <div style="font-weight: 700; font-size: 0.92rem; color: var(--primary);">{}</div>
                    <div style="font-size: 0.82rem; color: var(--text-muted); margin-top: 3px;">{}</div>
                </div>"###,
                html_escape(&tag_display), html_escape(&b.description)
            ));
        }
        badges_html.push_str("</div>");
    }

    // Recent plays
    let recent_scores = get_user_recent_scores(&state.db, user_id, 10).await.unwrap_or_default();
    let mut scores_html = String::new();
    if recent_scores.is_empty() {
        scores_html.push_str("<p style='color: var(--text-muted); font-style: italic; padding: 1rem;'>No beatmaps played recently.</p>");
    } else {
        scores_html.push_str(r###"<div style="overflow-x: auto;"><table class="modern-table">
            <thead>
                <tr>
                    <th style="width: 70px;">Rank</th>
                    <th>Beatmap</th>
                    <th style="text-align: right;">Score</th>
                    <th style="text-align: right;">Max Combo</th>
                    <th style="text-align: right;">300 / 100 / 50</th>
                    <th style="text-align: right;">Miss</th>
                </tr>
            </thead>
            <tbody>"###);
        for s in recent_scores {
            let acc = if s.c300 + s.c100 + s.c50 + s.c_miss > 0 {
                let total_hits = (s.c300 + s.c100 + s.c50 + s.c_miss) as f32;
                ((s.c300 as f32 * 300.0 + s.c100 as f32 * 100.0 + s.c50 as f32 * 50.0) / (total_hits * 300.0)) * 100.0
            } else {
                100.0
            };
            let (grade_text, grade_color, grade_bg) = calculate_grade(acc, s.c_miss);

            let meta = crate::db::beatmaps::resolve_beatmap_meta(&state.db, &s.map_md5).await;
            let display_name = meta.display_name();

            let mods_str = crate::bancho::bot::format_mods(s.mods as u32);
            let mods_badge = if !mods_str.is_empty() && mods_str != "None" {
                format!(
                    r#"<span style="margin-left: 7px; font-size: 0.76rem; font-weight: 800; color: #f59e0b; background: rgba(245, 158, 11, 0.15); padding: 2px 6px; border-radius: 4px; vertical-align: middle;">+{}</span>"#,
                    html_escape(&mods_str)
                )
            } else {
                String::new()
            };

            let beatmap_cell = if meta.beatmap_id > 0 {
                format!(
                    r#"<a href="https://osu.ppy.sh/b/{bid}" target="_blank" rel="noopener noreferrer" style="color: var(--text-main); font-weight: 600; text-decoration: none; transition: color 0.2s;" onmouseover="this.style.color='#f472b6'" onmouseout="this.style.color='var(--text-main)'" title="View beatmap #{bid} on osu!web">{name}</a>{mods}"#,
                    bid = meta.beatmap_id,
                    name = html_escape(&display_name),
                    mods = mods_badge
                )
            } else {
                format!(
                    r#"<span style="color: var(--text-main); font-weight: 600;">{}</span>{}"#,
                    html_escape(&display_name),
                    mods_badge
                )
            };

            scores_html.push_str(&format!(
                r###"<tr>
                    <td><span class="grade-badge" style="color: {gcolor}; background: {gbg}; border: 1px solid {gcolor};">{gtext}</span></td>
                    <td style="font-size: 0.92rem;">{bm_cell}</td>
                    <td style="text-align: right; font-weight: 700; font-family: 'JetBrains Mono', monospace;">{score}</td>
                    <td style="text-align: right; color: var(--emerald); font-weight: 700;">{combo}x</td>
                    <td style="text-align: right; color: var(--text-muted); font-size: 0.88rem;">{c300} / {c100} / {c50}</td>
                    <td style="text-align: right; color: var(--rose); font-weight: 700;">{miss}</td>
                </tr>"###,
                gcolor = grade_color,
                gbg = grade_bg,
                gtext = grade_text,
                bm_cell = beatmap_cell,
                score = format_number(s.score),
                combo = s.max_combo,
                c300 = s.c300,
                c100 = s.c100,
                c50 = s.c50,
                miss = s.c_miss
            ));
        }
        scores_html.push_str("</tbody></table></div>");
    }

    let join_date = DateTime::<Utc>::from_timestamp(user.created_at, 0)
        .map(|d| d.format("%d/%m/%Y").to_string())
        .unwrap_or_else(|| "N/A".to_string());

    let user_badge_tag = badges.iter().find_map(|b| {
        let t = b.tag.trim();
        if !t.is_empty() { Some((t.to_string(), b.name.clone())) } else { None }
    });

    let prefix_tag = if let Some((ref tag, ref bname)) = user_badge_tag {
        format!(
            r#"<span style="font-size: 1.15rem; padding: 2px 9px; color: #f472b6; background: rgba(244, 114, 182, 0.18); border: none; border-radius: 6px; font-weight: 800; display: inline-flex; align-items: center; vertical-align: middle;" title="{}">[{}]</span>"#,
            html_escape(bname),
            html_escape(tag)
        )
    } else {
        format!(
            r#"<span class="country-tag" style="font-size: 0.9rem; padding: 3px 8px;">[{}]</span>"#,
            country.code
        )
    };
    let clean_name = crate::db::badges::clean_username(&user.username);

    let navbar = render_navbar("", &state.config.server.name, current_user.as_ref());
    let footer = render_footer();

    let html = format!(
        r###"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="icon" type="image/png" href="/static/favicon.png">
    <link rel="shortcut icon" href="/favicon.ico">
    <title>{username} - Player Profile</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@400;500;600;700;800;900&family=JetBrains+Mono:wght@500;700&display=swap" rel="stylesheet">
    <style>
        {css}
        .osu-profile-cover {{
            min-height: 440px;
            border-radius: 14px;
            border: 1px solid var(--card-border);
            position: relative;
            background-size: cover;
            background-position: center;
            display: flex;
            flex-direction: column;
            justify-content: flex-end;
            padding: 2.2rem 2.5rem;
            box-shadow: 0 12px 36px rgba(0,0,0,0.55);
            margin-bottom: 1.8rem;
        }}
        .cover-actions-top {{
            position: absolute;
            top: 1.4rem;
            right: 1.4rem;
            display: flex;
            gap: 0.6rem;
            z-index: 5;
        }}
        .btn-cover-action {{
            background: rgba(15, 23, 42, 0.75);
            backdrop-filter: blur(8px);
            border: 1px solid rgba(255,255,255,0.2);
            color: #f8fafc;
            padding: 0.5rem 1rem;
            border-radius: 20px;
            font-size: 0.85rem;
            font-weight: 600;
            display: inline-flex;
            align-items: center;
            gap: 0.4rem;
            cursor: pointer;
            transition: all 0.2s;
        }}
        .btn-cover-action:hover {{
            background: rgba(244, 114, 182, 0.85);
            border-color: #f472b6;
            color: #fff;
            transform: translateY(-1px);
        }}
        .btn-cover-reset:hover {{
            background: rgba(239, 68, 68, 0.85);
            border-color: #ef4444;
        }}
        .avatar-container {{
            position: relative;
            width: 120px;
            height: 120px;
            border-radius: 50%;
            flex-shrink: 0;
        }}
        .osu-avatar {{
            width: 120px;
            height: 120px;
            border-radius: 50%;
            object-fit: cover;
            border: 4px solid rgba(255,255,255,0.3);
            box-shadow: 0 6px 24px rgba(0,0,0,0.65);
            display: block;
        }}
        .avatar-hover-overlay {{
            position: absolute;
            inset: 0;
            border-radius: 50%;
            background: rgba(15, 23, 42, 0.75);
            backdrop-filter: blur(4px);
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            color: #fff;
            font-size: 0.72rem;
            font-weight: 700;
            opacity: 0;
            cursor: pointer;
            transition: opacity 0.2s;
        }}
        .avatar-container:hover .avatar-hover-overlay {{
            opacity: 1;
        }}
        .country-edit-badge {{
            background: rgba(255,255,255,0.08);
            border: 1px solid rgba(255,255,255,0.15);
            color: var(--text-main);
            padding: 0.25rem 0.65rem;
            border-radius: 6px;
            font-size: 0.86rem;
            font-weight: 600;
            display: inline-flex;
            align-items: center;
            gap: 0.4rem;
            cursor: pointer;
            transition: all 0.2s;
        }}
        .country-edit-badge:hover {{
            background: rgba(244, 114, 182, 0.2);
            border-color: #f472b6;
            color: #f472b6;
        }}
        .btn-subtle-reset {{
            background: transparent;
            border: 1px solid rgba(239,68,68,0.3);
            color: #fca5a5;
            padding: 0.2rem 0.6rem;
            border-radius: 5px;
            font-size: 0.78rem;
            font-weight: 600;
            cursor: pointer;
            transition: all 0.2s;
        }}
        .btn-subtle-reset:hover {{
            background: rgba(239,68,68,0.15);
            border-color: #ef4444;
            color: #fff;
        }}
        .bio-markdown {{
            color: var(--text-main);
            font-size: 0.95rem;
            line-height: 1.7;
            word-break: break-word;
        }}
        .bio-markdown h1, .bio-markdown h2, .bio-markdown h3 {{
            color: #fff;
            font-weight: 800;
            margin: 1rem 0 0.5rem;
        }}
        .bio-markdown h1 {{ font-size: 1.5rem; border-bottom: 1px solid rgba(255,255,255,0.1); padding-bottom: 0.3rem; }}
        .bio-markdown h2 {{ font-size: 1.3rem; }}
        .bio-markdown h3 {{ font-size: 1.15rem; }}
        .bio-markdown blockquote {{
            border-left: 3px solid #f472b6;
            background: rgba(244, 114, 182, 0.08);
            padding: 0.6rem 1rem;
            border-radius: 0 6px 6px 0;
            margin: 0.8rem 0;
            font-style: italic;
            color: #cbd5e1;
        }}
        .bio-markdown pre {{
            background: #0b0f19;
            border: 1px solid rgba(255,255,255,0.08);
            border-radius: 6px;
            padding: 0.9rem 1.1rem;
            overflow-x: auto;
            margin: 0.8rem 0;
        }}
        .bio-markdown code {{
            font-family: 'JetBrains Mono', monospace;
            font-size: 0.88rem;
        }}
        .bio-markdown p code {{
            background: rgba(255,255,255,0.08);
            color: #f472b6;
            padding: 2px 5px;
            border-radius: 4px;
        }}
        .bio-markdown a {{
            color: #f472b6;
            text-decoration: underline;
            transition: color 0.2s;
        }}
        .bio-markdown a:hover {{
            color: #fb7185;
        }}
        .bio-markdown img {{
            max-width: 100%;
            height: auto;
            border-radius: 8px;
            margin: 0.5rem 0;
            box-shadow: 0 4px 14px rgba(0,0,0,0.3);
        }}
        .bio-markdown ul, .bio-markdown ol {{
            padding-left: 1.4rem;
            margin: 0.5rem 0;
        }}
        .editor-tab-btn {{
            background: transparent;
            border: none;
            color: var(--text-muted);
            font-size: 0.88rem;
            font-weight: 700;
            padding: 0.4rem 0.9rem;
            cursor: pointer;
            border-bottom: 2px solid transparent;
            transition: all 0.2s;
        }}
        .editor-tab-btn.active {{
            color: #f472b6;
            border-bottom: 2px solid #f472b6;
        }}
        .editor-fmt-btn {{
            background: rgba(255,255,255,0.05);
            border: 1px solid rgba(255,255,255,0.1);
            color: var(--text-main);
            padding: 0.25rem 0.55rem;
            border-radius: 4px;
            font-size: 0.82rem;
            font-weight: 700;
            cursor: pointer;
            transition: all 0.2s;
        }}
        .editor-fmt-btn:hover {{
            background: rgba(244, 114, 182, 0.2);
            border-color: #f472b6;
            color: #f472b6;
        }}
        .profile-toast {{
            position: fixed;
            bottom: 24px;
            right: 24px;
            padding: 0.75rem 1.4rem;
            background: rgba(15, 23, 42, 0.95);
            border: 1px solid #10b981;
            color: #fff;
            font-size: 0.92rem;
            font-weight: 600;
            border-radius: 8px;
            box-shadow: 0 8px 24px rgba(0,0,0,0.6);
            transform: translateY(100px);
            opacity: 0;
            transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1);
            z-index: 9999;
            pointer-events: none;
        }}
        .profile-toast.show {{
            transform: translateY(0);
            opacity: 1;
        }}
        .profile-toast.toast-error {{
            border-color: #ef4444;
        }}
        .modal-overlay {{
            position: fixed;
            inset: 0;
            background: rgba(0, 0, 0, 0.75);
            backdrop-filter: blur(4px);
            display: none;
            align-items: center;
            justify-content: center;
            z-index: 9998;
        }}
        .modal-card {{
            background: var(--bg-surface);
            border: 1px solid var(--card-border);
            border-radius: 12px;
            padding: 1.8rem;
            max-width: 440px;
            width: 90%;
            box-shadow: 0 16px 40px rgba(0,0,0,0.7);
        }}
        .mode-card {{
            background: var(--bg-surface-hover);
            border: 1px solid var(--card-border);
            border-radius: 6px;
            padding: 1.2rem;
        }}
    </style>
</head>
<body>
    {navbar}

    <main class="main-container">
        <!-- osu! Profile Header Cover -->
        <div id="profileCover" class="osu-profile-cover" data-user-id="{user_id}" style="background-image: linear-gradient(180deg, rgba(15, 23, 42, 0.05) 0%, rgba(15, 23, 42, 0.2) 45%, rgba(15, 23, 42, 0.78) 80%, rgba(15, 23, 42, 0.98) 100%), url('/b/{user_id}?v={banner_ver}');">
            {cover_actions_top}
            <div style="display: flex; align-items: flex-end; gap: 1.6rem; flex-wrap: wrap; z-index: 2;">
                <div class="avatar-container">
                    <img id="mainProfileAvatar" src="/a/{user_id}?v={avatar_ver}" class="osu-avatar" alt="{username}">
                    {avatar_overlay}
                </div>
                <div style="flex: 1; padding-bottom: 0.3rem;">
                    <h1 style="font-size: 2.2rem; font-weight: 800; letter-spacing: -0.5px; display: flex; align-items: center; gap: 0.6rem; margin: 0 0 0.6rem 0;">
                        {prefix_tag}
                        <span>{username}</span>
                    </h1>
                    <div style="display: flex; gap: 1rem; align-items: center; flex-wrap: wrap; color: var(--text-muted); font-size: 0.92rem; font-weight: 500;">
                        {country_badge_html}
                        <div>Joined: <b style="color: var(--text-main);">{join_date}</b></div>
                        <div>Standard Rank: <b style="color: #f59e0b;">#{rank_std}</b></div>
                    </div>
                </div>
            </div>
        </div>

        <!-- About Me ("me!") Section with Markdown -->
        <div class="glass-card" style="padding: 1.6rem 2rem;">
            <div class="card-header-bar" style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1rem;">
                <div class="card-heading" style="margin: 0; font-size: 1.25rem;">me!</div>
                {bio_edit_btn}
            </div>
            
            <div id="bioViewMode">
                <div id="bioContent" class="bio-markdown"></div>
            </div>

            {bio_edit_section}
        </div>

        <!-- Badges Showcase -->
        <div class="glass-card">
            <div class="card-heading" style="margin-bottom: 1.2rem;">Badges Showcase</div>
            {badges_html}
        </div>

        <!-- 4 Modes Stats -->
        <div class="glass-card">
            <div class="card-heading" style="margin-bottom: 1.2rem;">Game Modes Statistics</div>
            <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(230px, 1fr)); gap: 1.2rem;">
                <div class="mode-card">
                    <div style="font-weight: 800; color: var(--primary); font-size: 1.05rem; margin-bottom: 0.4rem;">osu! Standard</div>
                    <div style="font-size: 1.6rem; font-weight: 800; color: #fff;">{pp_std} <span style="font-size: 0.85rem; color: var(--text-muted);">pp</span></div>
                    <div style="font-size: 0.86rem; color: var(--text-muted); margin-top: 0.4rem;">Accuracy: <b style="color: var(--text-main);">{acc_std:.2}%</b></div>
                    <div style="font-size: 0.86rem; color: var(--text-muted);">Plays: <b style="color: var(--text-main);">{plays_std}</b></div>
                </div>
                <div class="mode-card">
                    <div style="font-weight: 800; color: var(--accent); font-size: 1.05rem; margin-bottom: 0.4rem;">osu!taiko</div>
                    <div style="font-size: 1.6rem; font-weight: 800; color: #fff;">{pp_taiko} <span style="font-size: 0.85rem; color: var(--text-muted);">pp</span></div>
                    <div style="font-size: 0.86rem; color: var(--text-muted); margin-top: 0.4rem;">Accuracy: <b style="color: var(--text-main);">{acc_taiko:.2}%</b></div>
                    <div style="font-size: 0.86rem; color: var(--text-muted);">Plays: <b style="color: var(--text-main);">{plays_taiko}</b></div>
                </div>
                <div class="mode-card">
                    <div style="font-weight: 800; color: var(--emerald); font-size: 1.05rem; margin-bottom: 0.4rem;">osu!catch</div>
                    <div style="font-size: 1.6rem; font-weight: 800; color: #fff;">{pp_ctb} <span style="font-size: 0.85rem; color: var(--text-muted);">pp</span></div>
                    <div style="font-size: 0.86rem; color: var(--text-muted); margin-top: 0.4rem;">Accuracy: <b style="color: var(--text-main);">{acc_ctb:.2}%</b></div>
                    <div style="font-size: 0.86rem; color: var(--text-muted);">Plays: <b style="color: var(--text-main);">{plays_ctb}</b></div>
                </div>
                <div class="mode-card">
                    <div style="font-weight: 800; color: var(--amber); font-size: 1.05rem; margin-bottom: 0.4rem;">osu!mania</div>
                    <div style="font-size: 1.6rem; font-weight: 800; color: #fff;">{pp_mania} <span style="font-size: 0.85rem; color: var(--text-muted);">pp</span></div>
                    <div style="font-size: 0.86rem; color: var(--text-muted); margin-top: 0.4rem;">Accuracy: <b style="color: var(--text-main);">{acc_mania:.2}%</b></div>
                    <div style="font-size: 0.86rem; color: var(--text-muted);">Plays: <b style="color: var(--text-main);">{plays_mania}</b></div>
                </div>
            </div>
        </div>

        <!-- Recent Scores -->
        <div class="glass-card">
            <div class="card-heading" style="margin-bottom: 1.2rem;">Recent Plays</div>
            {scores_html}
        </div>
    </main>

    {country_modal_html}
    <div id="profileToast" class="profile-toast"></div>
    {footer}
    {profile_script}
</body>
</html>"###,
        username = html_escape(clean_name),
        prefix_tag = prefix_tag,
        user_id = user.id,
        join_date = join_date,
        rank_std = rank_std,
        badges_html = badges_html,
        scores_html = scores_html,
        pp_std = format_number(stats_std.pp as i64),
        acc_std = stats_std.accuracy,
        plays_std = format_number(stats_std.play_count as i64),
        pp_taiko = format_number(stats_taiko.pp as i64),
        acc_taiko = stats_taiko.accuracy,
        plays_taiko = format_number(stats_taiko.play_count as i64),
        pp_ctb = format_number(stats_ctb.pp as i64),
        acc_ctb = stats_ctb.accuracy,
        plays_ctb = format_number(stats_ctb.play_count as i64),
        pp_mania = format_number(stats_mania.pp as i64),
        acc_mania = stats_mania.accuracy,
        plays_mania = format_number(stats_mania.play_count as i64),
        navbar = navbar,
        footer = footer,
        profile_script = profile_script,
        avatar_ver = avatar_ver,
        banner_ver = banner_ver,
        cover_actions_top = cover_actions_top,
        avatar_overlay = avatar_overlay,
        country_badge_html = country_badge_html,
        bio_edit_btn = bio_edit_btn,
        bio_edit_section = bio_edit_section,
        country_modal_html = country_modal_html,
        css = common_css()
    );

    Html(html)
}


// -------------------------------------------------------------------------------------------------
// 4. Login & Account Control Panel (GET /login, GET /logout, POST /api/login, POST /api/logout)
// -------------------------------------------------------------------------------------------------

pub async fn login_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Html<String> {
    let current_user = get_authenticated_user(&state, &headers).await;
    let navbar = render_navbar("login", &state.config.server.name, current_user.as_ref());
    let footer = render_footer();

    let content = match current_user {
        Some(user) => {
            let country = bancho_id_to_country(user.country);
            format!(
                r###"<div class="glass-card" style="max-width: 440px; margin: 4rem auto; padding: 2.4rem; text-align: center;">
                    <div style="margin-bottom: 1rem;">
                        <img src="/a/{user_id}" style="width: 88px; height: 88px; border-radius: 50%; object-fit: cover; border: 2px solid var(--card-border);" alt="{username}">
                    </div>
                    <div style="display: flex; align-items: center; justify-content: center; gap: 0.5rem; flex-wrap: wrap;">
                        <span class="country-tag">[{country_code}]</span>
                        <h1 style="font-size: 1.8rem; font-weight: 800;">{username}</h1>
                    </div>
                    <p style="color: var(--text-muted); font-size: 0.92rem; margin: 0.4rem 0 1.6rem;">
                        Account: <b style="color: var(--text-main);">#{user_id}</b> • Status: <b style="color: var(--emerald);">Signed In</b>
                    </p>
                    <div style="display: flex; flex-direction: column; gap: 0.8rem;">
                        <a href="/u/{user_id}" class="btn btn-primary" style="padding: 0.75rem; font-size: 0.95rem;">
                            View Profile
                        </a>
                        <a href="/logout" onclick="handleLogout(event)" class="btn btn-outline" style="padding: 0.75rem; font-size: 0.95rem; color: var(--rose); border-color: var(--rose);">
                            Sign Out
                        </a>
                    </div>
                </div>"###,
                user_id = user.id,
                username = html_escape(&user.username),
                country_code = country.code
            )
        }
        None => {
            let turnstile_widget = if state.config.turnstile.enabled {
                let site_key = &state.config.turnstile.site_key;
                let action = state.config.turnstile.expected_action.as_deref().unwrap_or("login");
                format!(
                    r###"<div class="cf-turnstile" data-sitekey="{site_key}" data-action="{action}" data-theme="dark" style="margin-bottom: 1.2rem; display: flex; justify-content: center;"></div>"###
                )
            } else {
                String::new()
            };

            format!(
                r###"<div class="glass-card" style="max-width: 420px; margin: 4rem auto; padding: 2.4rem; text-align: center;">
                    <div style="margin-bottom: 1.2rem;">
                        <img src="/static/logo.png" alt="AyanomiBancho" style="max-width: 280px; width: 85%; height: auto; filter: drop-shadow(0 4px 14px rgba(255,107,0,0.2));">
                    </div>
                    <h1 style="font-size: 1.6rem; font-weight: 800; letter-spacing: -0.4px;">Sign In to Server</h1>
                    <p style="color: var(--text-muted); font-size: 0.9rem; margin: 0.5rem 0 1.6rem; line-height: 1.5;">
                        Enter your credentials below to sign in. If you do not have an account yet, entering new credentials will automatically create and activate your account!
                    </p>
                    <form id="loginForm" onsubmit="handleLogin(event)">
                        <div style="text-align: left; margin-bottom: 1rem;">
                            <label class="form-label">Username</label>
                            <input class="input-glass" id="loginUser" type="text" required placeholder="Enter username..." autofocus>
                        </div>
                        <div style="text-align: left; margin-bottom: 1.4rem;">
                            <label class="form-label">Password</label>
                            <input class="input-glass" id="loginPass" type="password" required placeholder="Enter password...">
                        </div>
                        {turnstile_widget}
                        <button type="submit" class="btn btn-primary" style="width: 100%; padding: 0.75rem; font-size: 0.95rem;">Sign In / Create Account</button>
                    </form>
                    <div id="loginMsg" style="margin-top: 1rem; font-size: 0.9rem; font-weight: 600;"></div>
                </div>"###
            )
        }
    };

    let turnstile_script = if state.config.turnstile.enabled {
        r#"<script src="https://challenges.cloudflare.com/turnstile/v0/api.js" async defer></script>"#
    } else {
        ""
    };

    let html = format!(
        r###"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="icon" type="image/png" href="/static/favicon.png">
    <link rel="shortcut icon" href="/favicon.ico">
    <title>Sign In / Account - {name}</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@500;600;700;800;900&family=JetBrains+Mono:wght@500;700&display=swap" rel="stylesheet">
    {turnstile_script}
    <style>
        {css}
    </style>
</head>
<body>
    {navbar}
    <main class="main-container">
        {content}
    </main>
    {footer}
    <script>
{js}
    </script>
</body>
</html>"###,
        name = state.config.server.name,
        navbar = navbar,
        content = content,
        footer = footer,
        css = common_css(),
        js = login_js()
    );

    Html(html)
}

pub async fn logout_handler() -> Response {
    let mut response = axum::response::Redirect::to("/login").into_response();
    if let Ok(cookie_val) = "ayanomi_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT".parse() {
        response.headers_mut().insert(header::SET_COOKIE, cookie_val);
    }
    response
}

pub async fn api_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<LoginRequest>,
) -> Response {
    let username = payload.username.trim();
    let password = payload.password.trim();

    // Turnstile bot verification
    if state.config.turnstile.enabled {
        let secret = std::env::var("TURNSTILE_SECRET")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| state.config.turnstile.secret_key.clone());

        if !secret.trim().is_empty() {
            let token = payload.cf_turnstile_response.as_deref().unwrap_or("");
            if token.is_empty() {
                return (
                    StatusCode::FORBIDDEN,
                    Json(ApiResponse {
                        success: false,
                        message: "Bot verification required. Please complete the Turnstile challenge.".to_string(),
                    }),
                )
                    .into_response();
            }

            let client_ip = headers
                .get("cf-connecting-ip")
                .or_else(|| headers.get("x-forwarded-for"))
                .or_else(|| headers.get("x-real-ip"))
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(',').next())
                .map(|s| s.trim());

            if let Err(err) = crate::utils::turnstile::verify_turnstile_token(
                &secret,
                token,
                client_ip,
                state.config.turnstile.expected_action.as_deref(),
                &state.config.turnstile.expected_hostnames,
            )
            .await
            {
                return (
                    StatusCode::FORBIDDEN,
                    Json(ApiResponse {
                        success: false,
                        message: format!("Security check failed: {}", err),
                    }),
                )
                    .into_response();
            }
        }
    }


    if username.is_empty() || password.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                success: false,
                message: "Please enter both username and password.".to_string(),
            }),
        )
            .into_response();
    }

    let user_opt = match get_user_by_username(&state.db, username).await {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    message: format!("Database error: {}", e),
                }),
            )
                .into_response();
        }
    };

    let user = match user_opt {
        Some(u) => {
            let pass_md5 = md5_hex(password);
            let valid = verify_password(&pass_md5, &u.password_hash) || verify_password(password, &u.password_hash);
            if !valid {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(ApiResponse {
                        success: false,
                        message: "Incorrect password. Please try again.".to_string(),
                    }),
                )
                    .into_response();
            }
            u
        }
        None => {
            if state.config.gameplay.auto_register {
                let pass_md5 = md5_hex(password);
                let pwd_hash = match hash_password(&pass_md5) {
                    Ok(h) => h,
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiResponse {
                                success: false,
                                message: format!("Password encryption error: {}", e),
                            }),
                        )
                            .into_response();
                    }
                };
                let email = format!("{}@ayanomi.local", username);
                match create_user(&state.db, username, &pwd_hash, &email, state.config.gameplay.default_country).await {
                    Ok(new_u) => new_u,
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiResponse {
                                success: false,
                                message: format!("Account creation error: {}", e),
                            }),
                        )
                            .into_response();
                    }
                }
            } else {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ApiResponse {
                        success: false,
                        message: "Account does not exist on this server.".to_string(),
                    }),
                )
                    .into_response();
            }
        }
    };

    let token = sign_session(user.id, &user.password_hash, &state.config.server.admin_key);
    let cookie_str = format!("ayanomi_session={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000", token);

    let mut response = (
        StatusCode::OK,
        Json(ApiResponse {
            success: true,
            message: format!("Signed in successfully! Welcome, {}", user.username),
        }),
    )
        .into_response();

    if let Ok(cookie_val) = cookie_str.parse() {
        response.headers_mut().insert(header::SET_COOKIE, cookie_val);
    }

    response
}

pub async fn api_logout() -> Response {
    let mut response = (
        StatusCode::OK,
        Json(ApiResponse {
            success: true,
            message: "Signed out successfully.".to_string(),
        }),
    )
        .into_response();

    if let Ok(cookie_val) = "ayanomi_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT".parse() {
        response.headers_mut().insert(header::SET_COOKIE, cookie_val);
    }

    response
}

// -------------------------------------------------------------------------------------------------
// 5. Admin Control Panel (GET /admin)
// -------------------------------------------------------------------------------------------------

pub async fn admin_page(
    State(state): State<AppState>,
    Query(params): Query<AdminQuery>,
) -> Html<String> {
    let key = params.key.as_deref().unwrap_or("");
    let is_authorized = !key.is_empty() && key == state.config.server.admin_key;

    // Login screen if unauthorized
    if !is_authorized {
        let login_html = format!(
            r###"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="icon" type="image/png" href="/static/favicon.png">
    <link rel="shortcut icon" href="/favicon.ico">
    <title>Admin Gate - {name}</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@500;700;800&display=swap" rel="stylesheet">
    <style>
        {css}
        .admin-login-card {{
            max-width: 420px;
            margin: 6rem auto;
            background: var(--bg-surface);
            border: 1px solid var(--card-border);
            border-radius: 8px;
            padding: 2.4rem;
            text-align: center;
        }}
    </style>
</head>
<body>
    <div class="admin-login-card">
        <h1 style="font-size: 1.5rem; font-weight: 800; letter-spacing: -0.4px;">Administrator Access</h1>
        <p style="color: var(--text-muted); font-size: 0.9rem; margin: 0.6rem 0 1.6rem; line-height: 1.5;">
            This page is restricted. Please enter your secret <b>Admin Key</b> to unlock the control panel.
        </p>
        <form method="get" action="/admin">
            <div style="text-align: left; margin-bottom: 1.2rem;">
                <label class="form-label">Admin Key</label>
                <input class="input-glass" name="key" type="password" required placeholder="Enter secret admin_key..." autofocus>
            </div>
            <button type="submit" class="btn btn-primary" style="width: 100%; padding: 0.75rem; font-size: 0.95rem;">Unlock Dashboard</button>
        </form>
        <div style="margin-top: 1.6rem; font-size: 0.88rem;">
            <a href="/" style="color: var(--primary); font-weight: 600;">← Back to Player Website</a>
        </div>
    </div>
</body>
</html>"###,
            name = state.config.server.name,
            css = common_css()
        );
        return Html(login_html);
    }

    // Authorized Dashboard
    let (ram_used, ram_total, ram_pct) = get_memory_metrics();
    let db_size = get_file_size_kb(&state.config.database.path);
    let chat_db_size = get_file_size_kb(&state.config.database.chat_path);
    let badges_db_size = get_file_size_kb(&state.config.database.badges_path);
    let uptime_sec = { state.bancho.read().await.start_time.elapsed().as_secs() };
    let ratelimit_blocked = state.rate_limiter.total_blocked_count();
    let ratelimit_ips = state.rate_limiter.tracked_ips_count();

    // Badges
    let all_badges = crate::db::badges::list_all_badges(&state.badges_db).await.unwrap_or_default();
    let mut badges_html = String::new();
    let mut badge_select_options = String::new();
    for b in &all_badges {
        badge_select_options.push_str(&format!(r#"<option value="{}">[{}] {}</option>"#, b.id, html_escape(&b.name), html_escape(&b.description)));
        badges_html.push_str(&format!(
            r###"<div style="background: var(--bg-surface-hover); border: 1px solid var(--card-border); border-radius: 6px; padding: 0.9rem;">
                <div style="font-weight: 700; font-size: 0.95rem; color: var(--primary);">[{}]</div>
                <div style="font-size: 0.82rem; color: var(--text-muted); margin-top: 2px;">{}</div>
                <div style="font-size: 0.78rem; color: var(--accent); margin-top: 4px; font-weight: 600;">{} holders</div>
            </div>"###,
            html_escape(&b.name), html_escape(&b.description), b.holders_count
        ));
    }

    // Chat history (logs)
    let recent_chats = crate::db::chat::get_recent_chats(&state.chat_db, 30).await.unwrap_or_default();
    let mut chat_html = String::new();
    if recent_chats.is_empty() {
        chat_html.push_str("<p style='color: var(--text-muted); font-style: italic; padding: 1rem;'>No messages recorded in the chat database yet.</p>");
    } else {
        chat_html.push_str(r###"<div style="overflow-x: auto;"><table class="modern-table">
            <thead>
                <tr>
                    <th style="width: 170px;">Timestamp</th>
                    <th style="width: 150px;">Sender</th>
                    <th style="width: 150px;">Channel / Target</th>
                    <th>Message</th>
                </tr>
            </thead>
            <tbody>"###);
        for c in recent_chats {
            let channel_badge = if c.is_private != 0 {
                format!(r#"<span class="badge-tag" style="color: var(--rose);">[Private] {}</span>"#, c.target)
            } else {
                format!(r#"<span class="badge-tag" style="color: var(--accent);">[Channel] {}</span>"#, c.target)
            };
            chat_html.push_str(&format!(
                r###"<tr>
                    <td style="color: var(--text-muted); font-size: 0.82rem; font-family: monospace;">{}</td>
                    <td style="font-weight: 700; color: var(--primary);">{name}</td>
                    <td>{ch}</td>
                    <td style="word-break: break-word;">{msg}</td>
                </tr>"###,
                c.sent_at,
                name = html_escape(&c.sender_name),
                ch = channel_badge,
                msg = html_escape(&c.message)
            ));
        }
        chat_html.push_str("</tbody></table></div>");
    }

    // Backgrounds
    let bg_list = crate::server::backgrounds::scan_backgrounds(&state.config.backgrounds.directory);
    let mut backgrounds_html = String::new();
    if bg_list.is_empty() {
        backgrounds_html.push_str("<p style='color: var(--text-muted); font-style: italic; padding: 1rem;'>No background images found in directory.</p>");
    } else {
        backgrounds_html.push_str(r#"<div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 1rem; margin-top: 1rem;">"#);
        for (fname, size) in &bg_list {
            let size_kb = size / 1024;
            backgrounds_html.push_str(&format!(
                r###"<div style="background: var(--bg-surface-hover); border: 1px solid var(--card-border); border-radius: 6px; overflow: hidden;">
                    <a href="/backgrounds/{fname}" target="_blank">
                        <img src="/backgrounds/{fname}" style="width: 100%; height: 120px; object-fit: cover; display: block;" alt="{fname}">
                    </a>
                    <div style="padding: 0.75rem; display: flex; justify-content: space-between; align-items: center; gap: 0.5rem;">
                        <div style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 0.85rem; font-weight: 700;">{fname} ({size_kb}KB)</div>
                        <form action="/api/backgrounds/delete/{fname}" method="post" onsubmit="return confirm('Are you sure you want to delete this background?');">
                            <button type="submit" class="btn-danger">Delete</button>
                        </form>
                    </div>
                </div>"###,
                fname = fname, size_kb = size_kb
            ));
        }
        backgrounds_html.push_str("</div>");
    }

    let html = format!(
        r###"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="icon" type="image/png" href="/static/favicon.png">
    <link rel="shortcut icon" href="/favicon.ico">
    <title>Admin Control Panel - {name}</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@500;600;700;800;900&family=JetBrains+Mono:wght@500;700&display=swap" rel="stylesheet">
    <style>
        {css}
        .admin-nav {{
            background: var(--bg-surface);
            border-bottom: 1px solid var(--card-border);
            padding: 1rem 2rem;
            display: flex;
            align-items: center;
            justify-content: space-between;
        }}
    </style>
</head>
<body>
    <div class="admin-nav">
        <div>
            <div style="font-size: 1.25rem; font-weight: 800; letter-spacing: -0.3px;">{name} - Administration Dashboard</div>
            <div style="font-size: 0.8rem; color: var(--emerald); font-weight: 600;">Administrator privileges active</div>
        </div>
        <a href="/" class="btn btn-outline">View Player Website</a>
    </div>

    <main class="main-container">
        <!-- Telemetry & Database Status -->
        <div class="glass-card">
            <div class="card-heading" style="margin-bottom: 1.2rem;">System Resources &amp; Database Telemetry</div>
            <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 1rem;">
                <div style="background: var(--bg-surface-hover); border: 1px solid var(--card-border); border-radius: 6px; padding: 1.1rem;">
                    <div style="font-size: 0.78rem; color: var(--text-muted); font-weight: 700;">SERVER MEMORY (RAM)</div>
                    <div style="font-size: 1.4rem; font-weight: 800; color: var(--primary); margin-top: 4px;">{ram_used} MB <span style="font-size: 0.82rem; color: var(--text-muted);">/ {ram_total} MB ({ram_pct:.1}%)</span></div>
                </div>
                <div style="background: var(--bg-surface-hover); border: 1px solid var(--card-border); border-radius: 6px; padding: 1.1rem;">
                    <div style="font-size: 0.78rem; color: var(--text-muted); font-weight: 700;">CORE DB (ayanomi.db)</div>
                    <div style="font-size: 1.4rem; font-weight: 800; color: #a5b4fc; margin-top: 4px;">{db_size} KB <span style="font-size: 0.82rem; color: var(--text-muted);">(WAL)</span></div>
                </div>
                <div style="background: var(--bg-surface-hover); border: 1px solid var(--card-border); border-radius: 6px; padding: 1.1rem;">
                    <div style="font-size: 0.78rem; color: var(--text-muted); font-weight: 700;">CHAT DB (ayanomi_chat.db)</div>
                    <div style="font-size: 1.4rem; font-weight: 800; color: var(--emerald); margin-top: 4px;">{chat_db_size} KB <span style="font-size: 0.82rem; color: var(--text-muted);">(Isolated)</span></div>
                </div>
                <div style="background: var(--bg-surface-hover); border: 1px solid var(--card-border); border-radius: 6px; padding: 1.1rem;">
                    <div style="font-size: 0.78rem; color: var(--text-muted); font-weight: 700;">BADGES DB (ayanomi_badges.db)</div>
                    <div style="font-size: 1.4rem; font-weight: 800; color: var(--amber); margin-top: 4px;">{badges_db_size} KB <span style="font-size: 0.82rem; color: var(--text-muted);">(Isolated)</span></div>
                </div>
                <div style="background: var(--bg-surface-hover); border: 1px solid var(--card-border); border-radius: 6px; padding: 1.1rem;">
                    <div style="font-size: 0.78rem; color: var(--text-muted); font-weight: 700;">UPTIME</div>
                    <div style="font-size: 1.4rem; font-weight: 800; margin-top: 4px;">{uptime_str}</div>
                </div>
                <div style="background: var(--bg-surface-hover); border: 1px solid var(--card-border); border-radius: 6px; padding: 1.1rem;">
                    <div style="font-size: 0.78rem; color: var(--text-muted); font-weight: 700;">ANTI-RAID MITIGATIONS</div>
                    <div style="font-size: 1.4rem; font-weight: 800; color: var(--emerald); margin-top: 4px;">{ratelimit_blocked} blocked <span style="font-size: 0.82rem; color: var(--text-muted);">({ratelimit_ips} IPs)</span></div>
                </div>
            </div>
        </div>

        <!-- Badges Management -->
        <div class="glass-card">
            <div class="card-heading" style="margin-bottom: 1.2rem;">Badges Management &amp; Distribution</div>
            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 1rem; margin-bottom: 1.4rem;">
                {badges_html}
            </div>

            <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: 1.6rem; padding-top: 1.4rem; border-top: 1px solid var(--card-border);">
                <!-- Award Form -->
                <div>
                    <h3 style="font-size: 1.05rem; font-weight: 800; color: var(--primary); margin-bottom: 0.8rem;">Award Badge to Player</h3>
                    <form onsubmit="handleAward(event)" style="display: flex; flex-direction: column; gap: 0.8rem;">
                        <div>
                            <label class="form-label">User ID (Recipient)</label>
                            <input class="input-glass" id="awardUserId" type="number" required placeholder="e.g. 1">
                        </div>
                        <div>
                            <label class="form-label">Select Badge</label>
                            <select class="input-glass" id="awardBadgeId">
                                {badge_select_options}
                            </select>
                        </div>
                        <button type="submit" class="btn btn-primary" style="margin-top: 0.3rem;">Award Badge Now</button>
                    </form>
                    <div id="awardMsg" style="margin-top: 0.6rem; font-size: 0.9rem; font-weight: 600;"></div>
                </div>

                <!-- Create Form -->
                <div>
                    <h3 style="font-size: 1.05rem; font-weight: 800; color: var(--accent); margin-bottom: 0.8rem;">Create New Badge</h3>
                    <form onsubmit="handleCreateBadge(event)" style="display: flex; flex-direction: column; gap: 0.8rem;">
                        <div style="display: flex; gap: 0.8rem;">
                            <div style="width: 100px;">
                                <label class="form-label">Tag (Code)</label>
                                <input class="input-glass" id="newBadgeIcon" type="text" required placeholder="Tag" value="TAG" style="text-align: center;">
                            </div>
                            <div style="flex: 1;">
                                <label class="form-label">Badge Name</label>
                                <input class="input-glass" id="newBadgeName" type="text" required placeholder="Speed Demon">
                            </div>
                        </div>
                        <div>
                            <label class="form-label">Description</label>
                            <input class="input-glass" id="newBadgeDesc" type="text" required placeholder="Complete high-speed DT beatmap">
                        </div>
                        <button type="submit" class="btn btn-accent" style="margin-top: 0.3rem;">Create Badge</button>
                    </form>
                    <div id="createBadgeMsg" style="margin-top: 0.6rem; font-size: 0.9rem; font-weight: 600;"></div>
                </div>
            </div>
        </div>

        <!-- Chat Logs -->
        <div class="glass-card">
            <div class="card-heading" style="margin-bottom: 0.4rem;">Global Server Chat History (Dedicated Chat DB)</div>
            <p style="color: var(--text-muted); font-size: 0.88rem; margin-bottom: 1rem;">
                Messages are logged independently in <code>ayanomi_chat.db</code>. Only administrators have access to this log.
            </p>
            {chat_html}
        </div>

        <!-- Backgrounds -->
        <div class="glass-card">
            <div class="card-heading" style="margin-bottom: 0.4rem;">osu! Menu Backgrounds Management (Seasonal Backgrounds)</div>
            <p style="color: var(--text-muted); font-size: 0.88rem;">
                Upload new background images for the osu! client to display on the main menu.
            </p>
            {backgrounds_html}
            <div style="margin-top: 1.2rem; padding-top: 1.2rem; border-top: 1px solid var(--card-border);">
                <form action="/api/backgrounds/upload" method="post" enctype="multipart/form-data" style="display: flex; gap: 0.8rem; align-items: center; flex-wrap: wrap;">
                    <input type="file" name="file" accept=".jpg,.jpeg,.png,.webp" required class="input-glass" style="max-width: 320px; padding: 0.5rem 0.8rem;">
                    <button type="submit" class="btn btn-primary">Upload Background</button>
                </form>
            </div>
        </div>
    </main>

    <script>
{js}
    </script>
</body>
</html>"###,
        name = state.config.server.name,
        ram_used = ram_used,
        ram_total = ram_total,
        ram_pct = ram_pct,
        db_size = db_size,
        chat_db_size = chat_db_size,
        badges_db_size = badges_db_size,
        uptime_str = format!("{}m", uptime_sec / 60),
        ratelimit_blocked = ratelimit_blocked,
        ratelimit_ips = ratelimit_ips,
        badges_html = badges_html,
        badge_select_options = badge_select_options,
        chat_html = chat_html,
        backgrounds_html = backgrounds_html,
        css = common_css(),
        js = admin_js()
    );

    Html(html)
}

// -------------------------------------------------------------------------------------------------
// 6. How to Connect Page (GET /connect)
// -------------------------------------------------------------------------------------------------

pub async fn connect_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Html<String> {
    let current_user = get_authenticated_user(&state, &headers).await;
    let navbar = render_navbar("connect", &state.config.server.name, current_user.as_ref());
    let footer = render_footer();

    let domain = &state.config.server.domain;
    let name = &state.config.server.name;

    let html = format!(
        r###"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="icon" type="image/png" href="/static/favicon.png">
    <link rel="shortcut icon" href="/favicon.ico">
    <title>How to Connect - {name}</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@400;500;600;700;800;900&family=JetBrains+Mono:wght@500;700&display=swap" rel="stylesheet">
    <style>
        {css}
        .code-block {{
            background: #0a0c10;
            border: 1px solid var(--card-border);
            border-radius: 6px;
            padding: 0.9rem 1.1rem;
            font-family: 'JetBrains Mono', monospace;
            font-size: 0.9rem;
            color: var(--cyan);
            display: flex;
            align-items: center;
            justify-content: space-between;
            gap: 1rem;
            word-break: break-all;
            margin-top: 0.6rem;
        }}
        .copy-btn {{
            background: var(--bg-surface-hover);
            border: 1px solid var(--card-border);
            color: var(--text-main);
            padding: 0.35rem 0.75rem;
            border-radius: 4px;
            font-size: 0.8rem;
            font-weight: 700;
            cursor: pointer;
            flex-shrink: 0;
            transition: all 0.15s ease;
        }}
        .copy-btn:hover {{
            border-color: var(--primary);
            color: var(--primary);
        }}
        .badge-method {{
            display: inline-flex;
            align-items: center;
            padding: 3px 8px;
            border-radius: 4px;
            font-size: 0.75rem;
            font-weight: 800;
            letter-spacing: 0.5px;
            text-transform: uppercase;
        }}
        .method-recommended {{
            background: rgba(16, 185, 129, 0.15);
            color: var(--emerald);
            border: 1px solid var(--emerald);
        }}
        .method-alt {{
            background: rgba(14, 165, 233, 0.15);
            color: var(--cyan);
            border: 1px solid var(--cyan);
        }}
        .info-grid {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
            gap: 1rem;
        }}
        .info-card {{
            background: var(--bg-surface-hover);
            border: 1px solid var(--card-border);
            border-radius: 6px;
            padding: 1rem 1.2rem;
        }}
        .info-card-label {{
            font-size: 0.78rem;
            color: var(--text-muted);
            text-transform: uppercase;
            font-weight: 700;
            letter-spacing: 0.5px;
        }}
        .info-card-val {{
            font-size: 1.05rem;
            font-weight: 800;
            color: var(--text-main);
            margin-top: 0.2rem;
            font-family: 'JetBrains Mono', monospace;
        }}
        .faq-item {{
            background: var(--bg-surface-hover);
            border: 1px solid var(--card-border);
            border-radius: 6px;
            padding: 1.2rem;
        }}
        .faq-q {{
            font-weight: 800;
            font-size: 0.98rem;
            color: var(--text-main);
            margin-bottom: 0.4rem;
        }}
        .faq-a {{
            font-size: 0.9rem;
            color: var(--text-muted);
            line-height: 1.6;
        }}
    </style>
</head>
<body>
    {navbar}

    <main class="main-container">
        <!-- Hero Header -->
        <section class="hero-banner" style="padding: 2.5rem 1.5rem;">
            <div style="text-align: center; margin-bottom: 1.2rem;">
                <img src="/static/logo.png" alt="{name}" style="max-width: 400px; width: 80%; height: auto; filter: drop-shadow(0 4px 18px rgba(255, 107, 0, 0.22));">
            </div>
            <div class="hero-tag">CONNECTION GUIDE</div>
            <h1 class="hero-title" style="font-size: 2.4rem;">
                Connect to <span style="color: var(--primary);">{name}</span>
            </h1>
            <p class="hero-subtitle">
                Follow this simple guide to connect your osu! client to {name}. No client modification or file replacement needed!
            </p>
        </section>

        <!-- Server Information Overview -->
        <div class="glass-card">
            <div class="card-heading" style="margin-bottom: 1rem;">Server Information</div>
            <div class="info-grid">
                <div class="info-card">
                    <div class="info-card-label">Server Domain</div>
                    <div class="info-card-val">{domain}</div>
                </div>
                <div class="info-card">
                    <div class="info-card-label">Bancho Gateway</div>
                    <div class="info-card-val">c.{domain}</div>
                </div>
                <div class="info-card">
                    <div class="info-card-label">Avatar Server</div>
                    <div class="info-card-val">a.{domain}</div>
                </div>
                <div class="info-card">
                    <div class="info-card-label">Server Status</div>
                    <div class="info-card-val" style="color: var(--emerald); font-family: inherit;">
                        <span style="display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: var(--emerald); margin-right: 6px;"></span>
                        Online
                    </div>
                </div>
            </div>
        </div>

        <!-- Method 1: Desktop Shortcut (Recommended) -->
        <div class="glass-card" style="border: 1px solid var(--emerald);">
            <div class="card-header-bar">
                <div style="display: flex; align-items: center; gap: 0.75rem; flex-wrap: wrap;">
                    <div class="card-heading">Method 1: Desktop Shortcut</div>
                    <span class="badge-method method-recommended">Recommended</span>
                </div>
            </div>

            <div style="display: flex; flex-direction: column; gap: 1.2rem; margin-top: 0.5rem;">
                <div class="step-box">
                    <div class="step-num-bubble">1</div>
                    <div style="flex: 1;">
                        <div style="font-weight: 700; font-size: 1rem;">Locate or create your osu! shortcut</div>
                        <p style="color: var(--text-muted); font-size: 0.9rem; margin-top: 4px; line-height: 1.5;">
                            Navigate to your osu! installation directory (usually <code>%LOCALAPPDATA%\osu!</code>). Right click <b>osu!.exe</b> and select <b>Send to &rarr; Desktop (create shortcut)</b>.
                        </p>
                    </div>
                </div>

                <div class="step-box">
                    <div class="step-num-bubble">2</div>
                    <div style="flex: 1;">
                        <div style="font-weight: 700; font-size: 1rem;">Add the <code>-devserver</code> flag to Properties</div>
                        <p style="color: var(--text-muted); font-size: 0.9rem; margin-top: 4px; line-height: 1.5;">
                            Right click the new shortcut, select <b>Properties</b>, and go to the <b>Shortcut</b> tab. In the <b>Target</b> field, add a space followed by <code>-devserver {domain}</code> at the very end:
                        </p>
                        <div class="code-block">
                            <span id="targetExample">"C:\Users\YourUser\AppData\Local\osu!\osu!.exe" -devserver {domain}</span>
                            <button type="button" class="copy-btn" onclick="copyText('targetExample', this)">Copy</button>
                        </div>
                    </div>
                </div>

                <div class="step-box">
                    <div class="step-num-bubble">3</div>
                    <div style="flex: 1;">
                        <div style="font-weight: 700; font-size: 1rem;">Launch and Log In!</div>
                        <p style="color: var(--text-muted); font-size: 0.9rem; margin-top: 4px; line-height: 1.5;">
                            Double click the shortcut to launch osu!. When the in-game login screen appears, simply enter your username and password. If your account is not yet registered, the server will <b>automatically create and authenticate your account</b> right away!
                        </p>
                    </div>
                </div>
            </div>
        </div>

        <!-- Method 2: Command Line / PowerShell Launch -->
        <div class="glass-card">
            <div class="card-header-bar">
                <div style="display: flex; align-items: center; gap: 0.75rem; flex-wrap: wrap;">
                    <div class="card-heading">Method 2: One-Line Terminal Command</div>
                    <span class="badge-method method-alt">Quick Launch</span>
                </div>
            </div>

            <p style="color: var(--text-muted); font-size: 0.92rem; margin-bottom: 0.8rem; line-height: 1.5;">
                Press <kbd style="background: var(--bg-surface-hover); border: 1px solid var(--card-border); padding: 2px 6px; border-radius: 4px; font-family: monospace;">Win + R</kbd>, type <code>powershell</code>, and run this command:
            </p>

            <div class="code-block">
                <span id="psCommand">Start-Process "$env:LOCALAPPDATA\osu!\osu!.exe" -ArgumentList "-devserver {domain}"</span>
                <button type="button" class="copy-btn" onclick="copyText('psCommand', this)">Copy Command</button>
            </div>
        </div>

        <!-- SSL / HTTPS Certificate Guide -->
        <div class="glass-card">
            <div class="card-header-bar">
                <div class="card-heading">SSL Certificate Setup (If Required)</div>
                <a href="/certificate" class="btn btn-outline" style="font-size: 0.85rem; padding: 0.4rem 0.9rem;">Download CA Certificate (.crt)</a>
            </div>

            <p style="color: var(--text-muted); font-size: 0.92rem; line-height: 1.6;">
                If your osu! client warns about SSL or fails to connect over HTTPS, install our Root Certificate into Windows:
            </p>

            <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 1rem; margin-top: 1rem;">
                <div style="background: var(--bg-surface-hover); border: 1px solid var(--card-border); border-radius: 6px; padding: 1.2rem;">
                    <div style="font-weight: 700; font-size: 0.95rem; margin-bottom: 0.4rem;">Option A: Manual Installation</div>
                    <ol style="color: var(--text-muted); font-size: 0.88rem; line-height: 1.6; padding-left: 1.2rem;">
                        <li>Click the <b>Download CA Certificate</b> button above.</li>
                        <li>Double-click the downloaded <code>ayanomi_ca.crt</code> file.</li>
                        <li>Click <b>Install Certificate...</b> &rarr; choose <b>Current User</b>.</li>
                        <li>Select <b>Place all certificates in the following store</b>.</li>
                        <li>Browse and select <b>Trusted Root Certification Authorities</b>.</li>
                        <li>Click <b>Next</b> &rarr; <b>Finish</b>.</li>
                    </ol>
                </div>

                <div style="background: var(--bg-surface-hover); border: 1px solid var(--card-border); border-radius: 6px; padding: 1.2rem;">
                    <div style="font-weight: 700; font-size: 0.95rem; margin-bottom: 0.4rem;">Option B: PowerShell One-Liner</div>
                    <p style="color: var(--text-muted); font-size: 0.88rem; margin-bottom: 0.6rem;">Run PowerShell as Administrator after downloading:</p>
                    <div class="code-block" style="font-size: 0.82rem;">
                        <span id="certCommand">Import-Certificate -FilePath "$HOME\Downloads\ayanomi_ca.crt" -CertStoreLocation Cert:\CurrentUser\Root</span>
                        <button type="button" class="copy-btn" onclick="copyText('certCommand', this)">Copy</button>
                    </div>
                </div>
            </div>
        </div>

        <!-- How to Return to Official Servers -->
        <div class="glass-card">
            <div class="card-heading" style="margin-bottom: 0.6rem;">How to Switch Back to Official osu!</div>
            <p style="color: var(--text-muted); font-size: 0.92rem; line-height: 1.6;">
                Because {name} connects via the non-invasive <code>-devserver</code> flag, no client files are modified. Whenever you want to play on the official Bancho servers, simply launch <code>osu!.exe</code> normally without the <code>-devserver</code> argument!
            </p>
        </div>

        <!-- FAQ & Troubleshooting -->
        <div class="glass-card">
            <div class="card-heading" style="margin-bottom: 1.2rem;">Frequently Asked Questions</div>
            <div style="display: flex; flex-direction: column; gap: 1rem;">
                <div class="faq-item">
                    <div class="faq-q">Can I download beatmaps in-game with osu!Direct?</div>
                    <div class="faq-a">Yes! osu!Direct is fully enabled and unlimited for all players on {name}. You can search and download beatmaps directly from the in-game menus.</div>
                </div>
                <div class="faq-item">
                    <div class="faq-q">How does Relax (RX) ranking work?</div>
                    <div class="faq-a">Scores achieved with the Relax mod (RX) are automatically calculated and recorded in the Relax leaderboards for Standard, Taiko, and Catch modes. Check out the <a href="/leaderboard?m=4" style="color: var(--primary); font-weight: 600;">Relax Leaderboard</a>!</div>
                </div>
                <div class="faq-item">
                    <div class="faq-q">Game says "Attempting to connect..." repeatedly?</div>
                    <div class="faq-a">Make sure the server is online and you're launching osu! with <code>-devserver {domain}</code>. Also ensure your Windows Firewall allows connections on ports 80, 443, and 5000-5002.</div>
                </div>
                <div class="faq-item">
                    <div class="faq-q">How do I change my avatar and country flag?</div>
                    <div class="faq-a">Log in to your account on this website, then navigate to your <a href="/login" style="color: var(--primary); font-weight: 600;">Player Profile</a> page to customize your avatar, bio, and country flag!</div>
                </div>
            </div>
        </div>
    </main>

    {footer}

    <script>
{js}
    </script>
</body>
</html>"###,
        name = name,
        domain = domain,
        navbar = navbar,
        footer = footer,
        css = common_css(),
        js = connect_js()
    );

    Html(html)
}

fn connect_js() -> &'static str {
    r###"
        function copyText(id, btn) {
            const el = document.getElementById(id);
            if (!el) return;
            const text = el.innerText;
            navigator.clipboard.writeText(text).then(() => {
                const orig = btn.innerText;
                btn.innerText = "Copied!";
                btn.style.color = "#10b981";
                setTimeout(() => {
                    btn.innerText = orig;
                    btn.style.color = "";
                }, 2000);
            }).catch(() => {
                const input = document.createElement('textarea');
                input.value = text;
                document.body.appendChild(input);
                input.select();
                document.execCommand('copy');
                document.body.removeChild(input);
                btn.innerText = "Copied!";
                setTimeout(() => { btn.innerText = "Copy"; }, 2000);
            });
        }
    "###
}

pub async fn download_ca_cert() -> Response {
    if let Ok(bytes) = std::fs::read("data/certs/ca.crt") {
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/x-x509-ca-cert"),
                (header::CONTENT_DISPOSITION, "attachment; filename=\"ayanomi_ca.crt\""),
            ],
            bytes,
        )
            .into_response()
    } else {
        (StatusCode::NOT_FOUND, "Certificate file not found").into_response()
    }
}

// -------------------------------------------------------------------------------------------------
// 7. Server Rules Page (GET /rule & GET /rules)
// -------------------------------------------------------------------------------------------------

pub async fn rule_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Html<String> {
    let current_user = get_authenticated_user(&state, &headers).await;
    let navbar = render_navbar("rule", &state.config.server.name, current_user.as_ref());
    let footer = render_footer();
    let name = &state.config.server.name;

    let html = format!(
        r###"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="icon" type="image/png" href="/static/favicon.png">
    <link rel="shortcut icon" href="/favicon.ico">
    <title>Server Rules - {name}</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@400;500;600;700;800;900&family=JetBrains+Mono:wght@500;700&display=swap" rel="stylesheet">
    <style>
        {css}
        .rule-card {{
            background: var(--bg-surface);
            border: 1px solid var(--card-border);
            border-radius: 8px;
            padding: 1.5rem;
            display: flex;
            gap: 1.2rem;
            align-items: flex-start;
            transition: all 0.2s ease;
        }}
        .rule-card:hover {{
            border-color: rgba(167, 139, 250, 0.4);
            transform: translateY(-2px);
        }}
        .rule-icon {{
            width: 48px;
            height: 48px;
            display: flex;
            align-items: center;
            justify-content: center;
            border-radius: 8px;
            background: var(--bg-surface-hover);
            border: 1px solid var(--card-border);
            flex-shrink: 0;
            color: var(--primary);
        }}
        .rule-num {{
            font-family: 'JetBrains Mono', monospace;
            font-size: 0.8rem;
            font-weight: 700;
            color: var(--primary);
            text-transform: uppercase;
            letter-spacing: 0.5px;
            margin-bottom: 0.2rem;
        }}
        .rule-title {{
            font-size: 1.15rem;
            font-weight: 800;
            color: var(--text-main);
            margin-bottom: 0.5rem;
        }}
        .rule-desc {{
            color: var(--text-muted);
            font-size: 0.92rem;
            line-height: 1.6;
        }}
        .rule-desc b {{
            color: var(--text-main);
        }}
        .rule-notice {{
            background: rgba(239, 68, 68, 0.1);
            border: 1px solid rgba(239, 68, 68, 0.3);
            border-radius: 8px;
            padding: 1.2rem 1.5rem;
            margin-top: 1.5rem;
            display: flex;
            align-items: center;
            gap: 1rem;
        }}
    </style>
</head>
<body>
    {navbar}

    <main class="main-container">
        <!-- Header Hero -->
        <div style="text-align: center; max-width: 800px; margin: 1rem auto 2rem auto;">
            <div class="hero-tag" style="margin-bottom: 0.8rem;">COMMUNITY GUIDELINES & FAIR PLAY</div>
            <h1 style="font-size: 2.4rem; font-weight: 900; letter-spacing: -0.5px;">Server Rules & Guidelines</h1>
            <p style="color: var(--text-muted); font-size: 1.05rem; line-height: 1.6; margin-top: 0.5rem;">
                To cultivate a respectful, healthy, and competitive osu! environment, all players participating on <b>{name}</b> must strictly adhere to the following rules.
            </p>
        </div>

        <!-- Rules List -->
        <div style="display: flex; flex-direction: column; gap: 1rem;">
            <!-- Rule 1 -->
            <div class="rule-card">
                <div class="rule-icon">
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"></path><circle cx="9" cy="7" r="4"></circle><path d="M23 21v-2a4 4 0 0 0-3-3.87"></path><path d="M16 3.13a4 4 0 0 1 0 7.75"></path></svg>
                </div>
                <div style="flex: 1;">
                    <div class="rule-num">Rule 01</div>
                    <div class="rule-title">Respect & Community Conduct</div>
                    <div class="rule-desc">
                        Treat everyone with dignity. Harassment, hate speech, racism, regional slurs, abuse, and toxic behavior are strictly forbidden across public channels (<code>#osu</code>, <code>#announce</code>) and direct messages. Spamming, external server advertising, phishing links, and malicious exploits will result in immediate silence or restriction.
                    </div>
                </div>
            </div>

            <!-- Rule 2 -->
            <div class="rule-card">
                <div class="rule-icon">
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path></svg>
                </div>
                <div style="flex: 1;">
                    <div class="rule-num">Rule 02</div>
                    <div class="rule-title">Fair Play & Zero Tolerance for Cheating</div>
                    <div class="rule-desc">
                        <b>All forms of gameplay manipulation or automated cheating are strictly forbidden.</b> This includes timewarp, unauthorized Relax hacks (only the official server <code>!rx</code> mode is permitted), replay submitters, auto-aim, cursor dance macros, keyboard macro scripts, and memory manipulation tools. Infringing accounts will face a <b>permanent account ban and hardware ID (HWID) blacklist</b> without warning.
                    </div>
                </div>
            </div>

            <!-- Rule 3 -->
            <div class="rule-card">
                <div class="rule-icon">
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"></path><circle cx="12" cy="7" r="4"></circle></svg>
                </div>
                <div style="flex: 1;">
                    <div class="rule-num">Rule 03</div>
                    <div class="rule-title">One Account Per Player</div>
                    <div class="rule-desc">
                        Each individual is entitled to <b>strictly one account</b>. Creating secondary accounts (multi-accounting), account sharing, boosting, PP farming services, and buying or selling accounts are prohibited. Automated client hardware checks will reject duplicate registrations on the same machine.
                    </div>
                </div>
            </div>

            <!-- Rule 4 -->
            <div class="rule-card">
                <div class="rule-icon">
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"></rect><circle cx="8.5" cy="8.5" r="1.5"></circle><polyline points="21 15 16 10 5 21"></polyline></svg>
                </div>
                <div style="flex: 1;">
                    <div class="rule-num">Rule 04</div>
                    <div class="rule-title">Profile & Media Standards</div>
                    <div class="rule-desc">
                        Avatars, user bios, and display names must remain appropriate for all audiences. Content containing sexually explicit media (18+ NSFW), severe gore, hate symbols, or illegal material is strictly forbidden. Impersonating staff, developers, or official bots will result in immediate suspension.
                    </div>
                </div>
            </div>

            <!-- Rule 5 -->
            <div class="rule-card">
                <div class="rule-icon">
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"></polygon></svg>
                </div>
                <div style="flex: 1;">
                    <div class="rule-num">Rule 05</div>
                    <div class="rule-title">Responsible Bug Disclosure & Staff Authority</div>
                    <div class="rule-desc">
                        If you discover a glitch, exploit, or security vulnerability, you must report it promptly to administration through the <a href="/staff" style="color: var(--primary); font-weight: 700;">Staff & Credits</a> page rather than exploiting it for personal gain. Administration decisions regarding server stability and integrity are final.
                    </div>
                </div>
            </div>
        </div>

        <!-- Warning Notice -->
        <div class="rule-notice">
            <div style="flex-shrink: 0; display: flex; align-items: center; justify-content: center;">
                <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#f87171" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"></path><line x1="12" y1="9" x2="12" y2="13"></line><line x1="12" y1="17" x2="12.01" y2="17"></line></svg>
            </div>
            <div style="font-size: 0.92rem; color: #fca5a5; line-height: 1.6;">
                <b>Important Notice:</b> By connecting to and playing on {name}, you acknowledge and agree to comply with all server rules and guidelines. Have fun and enjoy climbing the leaderboards!
            </div>
        </div>
    </main>

    {footer}
</body>
</html>"###,
        name = name,
        navbar = navbar,
        footer = footer,
        css = common_css()
    );

    Html(html)
}

// -------------------------------------------------------------------------------------------------
// 8. Staff & Credits Page (GET /staff)
// -------------------------------------------------------------------------------------------------

pub async fn staff_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Html<String> {
    let current_user = get_authenticated_user(&state, &headers).await;
    let navbar = render_navbar("staff", &state.config.server.name, current_user.as_ref());
    let footer = render_footer();
    let name = &state.config.server.name;

    let html = format!(
        r###"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="icon" type="image/png" href="/static/favicon.png">
    <link rel="shortcut icon" href="/favicon.ico">
    <title>Staff & Credits - {name}</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@400;500;600;700;800;900&family=JetBrains+Mono:wght@500;700&display=swap" rel="stylesheet">
    <style>
        {css}
        .staff-card {{
            background: var(--bg-surface);
            border: 1px solid var(--card-border);
            border-radius: 8px;
            padding: 1.5rem;
            display: flex;
            gap: 1.2rem;
            align-items: center;
            transition: all 0.2s ease;
        }}
        .staff-card:hover {{
            border-color: rgba(167, 139, 250, 0.4);
            transform: translateY(-2px);
        }}
        .staff-avatar {{
            width: 72px;
            height: 72px;
            border-radius: 50%;
            object-fit: cover;
            border: 2px solid var(--card-border);
            flex-shrink: 0;
        }}
        .tech-card {{
            background: var(--bg-surface);
            border: 1px solid var(--card-border);
            border-radius: 12px;
            overflow: hidden;
            display: flex;
            flex-direction: column;
            align-items: center;
            text-align: center;
        }}
        .tech-banner {{
            width: 100%;
            padding: 2.6rem 1.5rem;
            background: rgba(255, 255, 255, 0.015);
            border-bottom: 1px solid var(--card-border);
            display: flex;
            align-items: center;
            justify-content: center;
            box-sizing: border-box;
        }}
        .tech-banner-img {{
            max-height: 240px;
            max-width: 92%;
            width: auto;
            height: auto;
            object-fit: contain;
        }}
        .tech-banner-svg {{
            height: 144px;
            width: auto;
            max-width: 92%;
            display: block;
        }}
        .tech-info {{
            padding: 1.6rem 1.8rem 1.8rem 1.8rem;
            width: 100%;
            box-sizing: border-box;
            display: flex;
            flex-direction: column;
            align-items: center;
            text-align: center;
        }}
        .tech-name {{
            font-weight: 800;
            font-size: 1.3rem;
            color: var(--text-main);
            margin-bottom: 0.5rem;
            display: flex;
            align-items: center;
            justify-content: center;
            gap: 0.5rem;
        }}
        .tech-desc {{
            color: var(--text-muted);
            font-size: 0.94rem;
            line-height: 1.65;
            max-width: 780px;
            margin: 0 auto;
            text-align: center;
        }}
        .oss-link {{
            display: inline-flex;
            align-items: center;
            gap: 0.5rem;
            padding: 0.5rem 0.9rem;
            background: var(--bg-surface-hover);
            border: 1px solid var(--card-border);
            border-radius: 6px;
            color: var(--text-main);
            text-decoration: none;
            font-size: 0.86rem;
            font-weight: 600;
            font-family: 'JetBrains Mono', monospace;
            width: fit-content;
            transition: all 0.2s ease;
        }}
        .oss-link:hover {{
            border-color: var(--primary);
            color: var(--primary);
            background: rgba(167, 139, 250, 0.08);
            transform: translateX(4px);
            text-decoration: none;
        }}
    </style>
</head>
<body>
    {navbar}

    <main class="main-container">
        <!-- Header Hero -->
        <div style="text-align: center; max-width: 800px; margin: 1rem auto 2rem auto;">
            <div class="hero-tag" style="margin-bottom: 0.8rem;">TEAM & OPEN SOURCE ACKNOWLEDGEMENTS</div>
            <h1 style="font-size: 2.4rem; font-weight: 900; letter-spacing: -0.5px;">Staff & Open Source Credits</h1>
            <p style="color: var(--text-muted); font-size: 1.05rem; line-height: 1.6; margin-top: 0.5rem;">
                Introducing the <b>{name}</b> administration team and acknowledging the open source software, communities, and technologies powering the platform.
            </p>
        </div>

        <!-- Section 1: Server Staff -->
        <div class="glass-card">
            <div class="card-heading" style="margin-bottom: 1.2rem;">Server Administration</div>
            <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: 1.2rem;">
                <!-- Ayanomi -->
                <div class="staff-card">
                    <img src="/a/1" class="staff-avatar" alt="Ayanomi">
                    <div style="flex: 1; overflow: hidden;">
                        <div style="display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap;">
                            <span class="country-tag" style="color: #f472b6; background: rgba(244, 114, 182, 0.18); border: none; font-weight: 800;" title="Server Admin">[AM]</span>
                            <a href="/u/1" style="font-weight: 800; font-size: 1.15rem; color: var(--text-main); text-decoration: none;">Ayanomi</a>
                        </div>
                        <div style="font-size: 0.85rem; font-weight: 700; color: var(--primary); margin-top: 2px;">Founder & Lead Server Developer</div>
                        <div style="font-size: 0.85rem; color: var(--text-muted); margin-top: 4px; line-height: 1.5;">
                            Founder, system architect, and primary developer of the AyanomiBancho server.
                        </div>
                    </div>
                </div>
            </div>
        </div>

        <!-- Section 2: Technology Stack & Open Source Credits -->
        <div class="glass-card">
            <div class="card-heading" style="margin-bottom: 0.6rem;">Technology Stack</div>
            <p style="color: var(--text-muted); font-size: 0.92rem; line-height: 1.6; margin-bottom: 1.4rem;">
                The <b>{name}</b> platform is engineered and powered by the following technologies:
            </p>

            <div style="display: flex; flex-direction: column; gap: 1.4rem;">
                <!-- Rust -->
                <div class="tech-card">
                    <div class="tech-banner">
                        <img src="/static/rust_logo.png" alt="Rust" class="tech-banner-img">
                    </div>
                    <div class="tech-info">
                        <div class="tech-name">Rust (Rustlang)</div>
                        <div class="tech-desc">
                            Modern systems programming language delivering maximum performance, absolute memory safety, and ultra-low latency across the entire AyanomiBancho infrastructure.
                        </div>
                    </div>
                </div>

                <!-- SQL -->
                <div class="tech-card">
                    <div class="tech-banner">
                        <svg viewBox="0 0 128 128" class="tech-banner-svg" xmlns="http://www.w3.org/2000/svg">
                            <defs>
                                <linearGradient id="sqlite-grad" x1="-15.615" x2="-6.741" y1="-9.108" y2="-9.108" gradientTransform="rotate(90 -90.486 64.634) scale(9.2712)" gradientUnits="userSpaceOnUse">
                                    <stop stop-color="#95d7f4" offset="0"/>
                                    <stop stop-color="#0f7fcc" offset=".92"/>
                                    <stop stop-color="#0f7fcc" offset="1"/>
                                </linearGradient>
                            </defs>
                            <path d="M69.5 99.176c-.059-.73-.094-1.2-.094-1.2S67.2 83.087 64.57 78.642c-.414-.707.043-3.594 1.207-7.88.68 1.169 3.54 6.192 4.118 7.81.648 1.824.78 2.347.78 2.347s-1.57-8.082-4.144-12.797a162.286 162.286 0 012.004-6.265c.973 1.71 3.313 5.859 3.828 7.3.102.293.192.543.27.774.023-.137.05-.274.074-.414-.59-2.504-1.75-6.86-3.336-10.082 3.52-18.328 15.531-42.824 27.84-53.754H16.9c-5.387 0-9.789 4.406-9.789 9.789v88.57c0 5.383 4.406 9.789 9.79 9.789h52.897a118.657 118.657 0 01-.297-14.652" fill="#0b7fcc"/>
                            <path d="M65.777 70.762c.68 1.168 3.54 6.188 4.117 7.809.649 1.824.781 2.347.781 2.347s-1.57-8.082-4.144-12.797a164.535 164.535 0 012.004-6.27c.887 1.567 2.922 5.169 3.652 6.872l.082-.961c-.648-2.496-1.633-5.766-2.898-8.328 3.242-16.871 13.68-38.97 24.926-50.898H16.899a6.94 6.94 0 00-6.934 6.933v82.11c17.527-6.731 38.664-12.88 56.855-12.614-.672-2.605-1.441-4.96-2.25-6.324-.414-.707.043-3.597 1.207-7.879" fill="url(#sqlite-grad)"/>
                            <path d="M115.95 2.781c-5.5-4.906-12.164-2.933-18.734 2.899a44.347 44.347 0 00-2.914 2.859c-11.25 11.926-21.684 34.023-24.926 50.895 1.262 2.563 2.25 5.832 2.894 8.328.168.64.32 1.242.442 1.754.285 1.207.437 1.996.437 1.996s-.101-.383-.515-1.582c-.078-.23-.168-.484-.27-.773-.043-.125-.105-.274-.172-.434-.734-1.703-2.765-5.305-3.656-6.867-.762 2.25-1.437 4.36-2.004 6.265 2.578 4.715 4.149 12.797 4.149 12.797s-.137-.523-.782-2.347c-.578-1.621-3.441-6.64-4.117-7.809-1.164 4.281-1.625 7.172-1.207 7.88.809 1.362 1.574 3.722 2.25 6.323 1.524 5.867 2.586 13.012 2.586 13.012s.031.469.094 1.2a118.653 118.653 0 00.297 14.651c.504 6.11 1.453 11.363 2.664 14.172l.828-.449c-1.781-5.535-2.504-12.793-2.188-21.156.48-12.793 3.422-28.215 8.856-44.289 9.191-24.27 21.938-43.738 33.602-53.035-10.633 9.602-25.023 40.684-29.332 52.195-4.82 12.891-8.238 24.984-10.301 36.574 3.55-10.863 15.047-15.53 15.047-15.53s5.637-6.958 12.227-16.888c-3.95.903-10.43 2.442-12.598 3.352-3.2 1.344-4.067 1.8-4.067 1.8s10.371-6.312 19.27-9.171c12.234-19.27 25.562-46.648 12.141-58.621" fill="#003956"/>
                        </svg>
                    </div>
                    <div class="tech-info">
                        <div class="tech-name">SQL (SQLite & SQLx)</div>
                        <div class="tech-desc">
                            High-performance SQL database engine with independent database isolation and Write-Ahead Logging (WAL) mode for instantaneous queries and complete data integrity.
                        </div>
                    </div>
                </div>

                <!-- Tokio -->
                <div class="tech-card">
                    <div class="tech-banner">
                        <svg viewBox="0 0 120 108" class="tech-banner-svg" xmlns="http://www.w3.org/2000/svg">
                            <g fill="#38bdf8">
                                <polygon transform="translate(37, 67) rotate(-300) translate(-37, -67)" points="35 74 35 76 39 76 39 74 39 60 39 58 35 58 35 60"></polygon>
                                <polygon transform="translate(83, 67) rotate(-60) translate(-83, -67)" points="81 74 81 76 85 76 85 74 85 60 85 58 81 58 81 60"></polygon>
                                <path d="M80,54 C80,43 71,34 60,34 C49,34 40,43 40,54 C40,65 49,74 60,74 C71,74 80,65 80,54 Z M44,54 C44,45 51,38 60,38 C69,38 76,45 76,54 C76,63 69,70 60,70 C51,70 44,63 44,54 Z"></path>
                                <circle cx="24" cy="75" r="4"></circle>
                                <circle cx="60" cy="96" r="4"></circle>
                                <circle cx="60" cy="12" r="4"></circle>
                                <circle cx="96" cy="33" r="4"></circle>
                                <circle cx="24" cy="33" r="4"></circle>
                                <circle cx="96" cy="75" r="4"></circle>
                                <circle cx="60" cy="54" r="4"></circle>
                                <polygon points="2 52 0 52 0 56 2 56 40 56 42 56 42 52 40 52"></polygon>
                                <polygon points="80 52 78 52 78 56 80 56 118 56 120 56 120 52 118 52"></polygon>
                                <polygon points="73 70 72 69 68 71 69 72 89 105 90 107 93 105 92 103"></polygon>
                                <polygon points="33 2 32 0 28 2 29 4 48 37 49 39 53 37 52 35"></polygon>
                                <polygon points="91 4 92 2 88 0 87 2 68 35 67 37 71 39 72 37"></polygon>
                                <polygon points="51 73 52 71 48 69 47 71 28 103 27 105 31 107 32 105"></polygon>
                                <polygon points="58 87 58 89 62 89 62 87 62 73 62 71 58 71 58 73"></polygon>
                                <polygon points="58 35 58 37 62 37 62 35 62 21 62 19 58 19 58 21"></polygon>
                                <polygon transform="translate(37, 41) rotate(-60) translate(-37, -41)" points="35 48 35 50 39 50 39 48 39 34 39 32 35 32 35 34"></polygon>
                                <polygon transform="translate(83, 41) rotate(-300) translate(-83, -41)" points="81 48 81 50 85 50 85 48 85 34 85 32 81 32 81 34"></polygon>
                            </g>
                        </svg>
                    </div>
                    <div class="tech-info">
                        <div class="tech-name">Tokio</div>
                        <div class="tech-desc">
                            High-throughput, asynchronous multi-threaded runtime for Rust, handling tens of thousands of concurrent connections with exceptional reliability and speed.
                        </div>
                    </div>
                </div>

                <!-- Axum -->
                <div class="tech-card">
                    <div class="tech-banner">
                        <svg viewBox="0 0 128 128" class="tech-banner-svg" xmlns="http://www.w3.org/2000/svg">
                            <defs>
                                <linearGradient id="axum-grad" x1="0%" y1="0%" x2="100%" y2="100%">
                                    <stop offset="0%" stop-color="#818cf8"/>
                                    <stop offset="100%" stop-color="#c084fc"/>
                                </linearGradient>
                            </defs>
                            <rect x="6" y="6" width="116" height="116" rx="26" fill="rgba(129, 140, 248, 0.08)" stroke="url(#axum-grad)" stroke-width="3"/>
                            <path d="M64 24 L98 44 L98 84 L64 104 L30 84 L30 44 Z" fill="none" stroke="url(#axum-grad)" stroke-width="4.5" stroke-linejoin="round"/>
                            <path d="M64 24 L64 104" stroke="url(#axum-grad)" stroke-width="3" opacity="0.45"/>
                            <path d="M30 44 L64 64 L98 44" fill="none" stroke="url(#axum-grad)" stroke-width="3.5" stroke-linejoin="round"/>
                            <circle cx="64" cy="64" r="8" fill="#a855f7"/>
                            <circle cx="64" cy="64" r="4" fill="#ffffff"/>
                            <circle cx="64" cy="24" r="5" fill="#38bdf8"/>
                            <circle cx="30" cy="84" r="5" fill="#38bdf8"/>
                            <circle cx="98" cy="84" r="5" fill="#38bdf8"/>
                        </svg>
                    </div>
                    <div class="tech-info">
                        <div class="tech-name">Axum</div>
                        <div class="tech-desc">
                            Modern, high-performance web framework built specifically for the Tokio and Rust ecosystem, serving APIs and web pages with sub-millisecond latency.
                        </div>
                    </div>
                </div>
            </div>
        </div>

        <!-- Section 3: Special Thanks -->
        <div class="glass-card">
            <div class="card-heading" style="margin-bottom: 0.8rem;">Special Acknowledgements</div>
            <div style="color: var(--text-muted); font-size: 0.92rem; line-height: 1.7;">
                <p style="margin-bottom: 0.8rem;">
                    <b>Rust Open Source Community</b>: Sincere appreciation to the authors and maintainers of the outstanding open-source libraries powering this project on <b>GitHub</b>:
                </p>
                <div style="display: flex; flex-direction: column; gap: 0.5rem; margin-bottom: 1.2rem; align-items: flex-start;">
                    <a href="https://github.com/tokio-rs/tokio" target="_blank" rel="noopener noreferrer" class="oss-link" title="Asynchronous runtime for Rust">tokio-rs/tokio</a>
                    <a href="https://github.com/tokio-rs/axum" target="_blank" rel="noopener noreferrer" class="oss-link" title="Modular web application framework">tokio-rs/axum</a>
                    <a href="https://github.com/launchbadge/sqlx" target="_blank" rel="noopener noreferrer" class="oss-link" title="Async, pure Rust SQL crate">launchbadge/sqlx</a>
                    <a href="https://github.com/pure-peace/simple-rijndael" target="_blank" rel="noopener noreferrer" class="oss-link" title="Rijndael AES encryption implementation">pure-peace/simple-rijndael</a>
                    <a href="https://github.com/Keats/rust-bcrypt" target="_blank" rel="noopener noreferrer" class="oss-link" title="Bcrypt password hashing">Keats/rust-bcrypt</a>
                    <a href="https://github.com/seanmonstar/reqwest" target="_blank" rel="noopener noreferrer" class="oss-link" title="Fast HTTP Client for Rust">seanmonstar/reqwest</a>
                    <a href="https://github.com/chronotope/chrono" target="_blank" rel="noopener noreferrer" class="oss-link" title="Date and time library for Rust">chronotope/chrono</a>
                    <a href="https://github.com/rwf2/multer" target="_blank" rel="noopener noreferrer" class="oss-link" title="Multipart form data parser">rwf2/multer</a>
                </div>
                <p>
                    <b>{name} Player Community</b>: A heartfelt thank you to all players who support, provide feedback, and help build a passionate gaming home together!
                </p>
            </div>
        </div>
    </main>

    {footer}
</body>
</html>"###,
        name = name,
        navbar = navbar,
        footer = footer,
        css = common_css()
    );

    Html(html)
}

pub static RUST_LOGO_BYTES: &[u8] = include_bytes!("rust_logo.png");

pub async fn rust_logo_handler() -> impl axum::response::IntoResponse {
    (
        [
            ("content-type", "image/png"),
            ("cache-control", "public, max-age=86400"),
        ],
        RUST_LOGO_BYTES,
    )
}

pub static SERVER_LOGO_BYTES: &[u8] = include_bytes!("server_logo.png");

pub async fn server_logo_handler() -> impl axum::response::IntoResponse {
    (
        [
            ("content-type", "image/png"),
            ("cache-control", "public, max-age=86400"),
        ],
        SERVER_LOGO_BYTES,
    )
}

pub static MENU_OSU_BYTES: &[u8] = include_bytes!("menu-osu.png");

pub async fn menu_osu_handler() -> impl axum::response::IntoResponse {
    (
        [
            ("content-type", "image/png"),
            ("cache-control", "public, max-age=86400"),
        ],
        MENU_OSU_BYTES,
    )
}

// -------------------------------------------------------------------------------------------------
// 9. Multiplayer Live Tracking Page (GET /multi)
// -------------------------------------------------------------------------------------------------

fn get_multi_mode_name(mode: i64) -> &'static str {
    match mode {
        0 => "osu! Standard",
        1 => "Taiko",
        2 => "Catch",
        3 => "osu!mania",
        _ => "osu!",
    }
}

fn get_multi_scoring_name(st: i64) -> &'static str {
    match st {
        0 => "Score",
        1 => "Accuracy",
        2 => "Combo",
        3 => "Score V2",
        _ => "Score",
    }
}

fn get_multi_team_name(tt: i64) -> &'static str {
    match tt {
        0 => "Head to Head",
        1 => "Tag Co-op",
        2 => "Team VS",
        3 => "Tag Team VS",
        _ => "Head to Head",
    }
}

pub async fn multi_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Html<String> {
    let current_user = get_authenticated_user(&state, &headers).await;
    let navbar = render_navbar("multi", &state.config.server.name, current_user.as_ref());
    let footer = render_footer();
    let name = &state.config.server.name;

    // Sync any live in-memory bancho rooms to multi_db before rendering
    {
        let st = state.bancho.read().await;
        for &match_id in st.matches.keys() {
            crate::bancho::handler::sync_match_to_multi_db(state.multi_db.clone(), &st, match_id);
        }
    }

    let rooms = crate::db::multi::get_all_live_rooms(&state.multi_db).await.unwrap_or_default();

    let total_rooms = rooms.len();
    let total_players: i64 = rooms.iter().map(|r| r.room.player_count).sum();
    let total_matches: usize = rooms.iter().map(|r| r.games.len()).sum();

    let mut rooms_html = String::new();
    if rooms.is_empty() {
        rooms_html.push_str(r###"
        <div style="text-align: center; padding: 4rem 2rem; background: var(--bg-surface); border: 1px solid var(--card-border); border-radius: 12px; margin-top: 1rem;">
            <div style="width: 64px; height: 64px; margin: 0 auto 1.2rem auto; border-radius: 50%; background: var(--bg-surface-hover); display: flex; align-items: center; justify-content: center; border: 1px solid var(--card-border); color: var(--primary);">
                <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 11h4M8 9v4M15 12h.01M18 10h.01M17.32 5H6.68a4 4 0 0 0-3.978 3.59c-.006.052-.01.101-.017.152C2.604 9.416 2 14.456 2 16a3 3 0 0 0 3 3c1 0 1.5-.5 2-1l1.414-1.414A2 2 0 0 1 9.828 16h4.344a2 2 0 0 1 1.414.586L17 18c.5.5 1 1 2 1a3 3 0 0 0 3-3c0-1.545-.604-6.584-.685-7.258-.007-.05-.011-.1-.017-.151A4 4 0 0 0 17.32 5z"></path></svg>
            </div>
            <h3 style="font-size: 1.4rem; font-weight: 800; color: var(--text-main); margin-bottom: 0.5rem;">No Active Multiplayer Rooms</h3>
            <p style="color: var(--text-muted); font-size: 0.95rem; max-width: 520px; margin: 0 auto 1.5rem auto; line-height: 1.6;">
                There are currently no active lobbies on the server. Connect to <b>"###);
        rooms_html.push_str(name);
        rooms_html.push_str(r###"</b> via your osu! client, create or join a room in the multiplayer lobby, and watch it live here!
            </p>
            <a href="/connect" class="btn btn-primary" style="display: inline-flex; align-items: center; gap: 0.5rem;">
                <span>View Connection Guide</span>
            </a>
        </div>
        "###);
    } else {
        for item in &rooms {
            let r = &item.room;
            let status_badge = if r.in_progress == 1 {
                r###"<span class="multi-badge playing"><span class="pulse-dot red"></span> IN MATCH</span>"###
            } else {
                r###"<span class="multi-badge waiting"><span class="pulse-dot green"></span> WAITING IN LOBBY</span>"###
            };

            let map_link = if r.beatmap_id > 0 {
                format!(
                    r###"<a href="https://osu.ppy.sh/b/{id}" target="_blank" rel="noopener noreferrer" style="color: var(--text-main); text-decoration: none; font-weight: 700;">{name}</a>"###,
                    id = r.beatmap_id,
                    name = html_escape(&r.beatmap_name)
                )
            } else {
                format!(r###"<span style="font-weight: 700; color: var(--text-main);">{name}</span>"###, name = html_escape(&r.beatmap_name))
            };

            let mut slots_html = String::new();
            for s in &item.slots {
                if s.user_id > 0 {
                    let team_badge = if s.team == 1 {
                        r###"<span class="team-badge blue">Blue</span>"###
                    } else if s.team == 2 {
                        r###"<span class="team-badge red">Red</span>"###
                    } else {
                        ""
                    };

                    let is_host = s.user_id as i64 == r.host_id;
                    let role_badge = if is_host {
                        r###"<span class="slot-status-badge host">Host</span>"###
                    } else {
                        match s.status_text.as_str() {
                            "Ready" => r###"<span class="slot-status-badge ready">Ready</span>"###,
                            "Playing" => r###"<span class="slot-status-badge playing">Playing</span>"###,
                            "No Map" => r###"<span class="slot-status-badge nomap">No Map</span>"###,
                            "Complete" => r###"<span class="slot-status-badge complete">Finished</span>"###,
                            _ => r###"<span class="slot-status-badge notready">Not Ready</span>"###,
                        }
                    };

                    slots_html.push_str(&format!(
                        r###"
                        <div class="slot-box occupied">
                            <div class="slot-num">Slot {num}</div>
                            <img src="/a/{uid}" class="slot-avatar" alt="Avatar" onerror="this.src='/a/1'">
                            <div class="slot-info">
                                <a href="/u/{uid}" class="slot-username">{uname}</a>
                                <div style="display: flex; gap: 0.3rem; align-items: center; margin-top: 2px;">
                                    {team_badge}
                                    {role_badge}
                                </div>
                            </div>
                        </div>"###,
                        num = s.slot_id + 1,
                        uid = s.user_id,
                        uname = html_escape(&s.username),
                        team_badge = team_badge,
                        role_badge = role_badge
                    ));
                } else if s.status_text == "Locked" {
                    slots_html.push_str(&format!(
                        r###"
                        <div class="slot-box locked">
                            <div class="slot-num">Slot {num}</div>
                            <div class="slot-empty-text">Locked</div>
                        </div>"###,
                        num = s.slot_id + 1
                    ));
                } else {
                    slots_html.push_str(&format!(
                        r###"
                        <div class="slot-box open">
                            <div class="slot-num">Slot {num}</div>
                            <div class="slot-empty-text">Open</div>
                        </div>"###,
                        num = s.slot_id + 1
                    ));
                }
            }

            let mut games_html = String::new();
            if item.games.is_empty() {
                games_html.push_str(r###"
                <div style="font-size: 0.88rem; color: var(--text-muted); padding: 0.8rem 1rem; background: var(--bg-surface-hover); border-radius: 6px; margin-top: 1rem; border: 1px solid var(--card-border);">
                    No completed match rounds yet in this session. Match scoreboards will be recorded here when rounds conclude.
                </div>
                "###);
            } else {
                for (idx, g) in item.games.iter().enumerate() {
                    let duration_str = format!("{}m {:02}s", g.game.duration_seconds / 60, g.game.duration_seconds % 60);
                    let mut score_rows = String::new();

                    for (rank, sc) in g.scores.iter().enumerate() {
                        let rank_badge = if sc.won == 1 {
                            r###"<span style="color: #fbbf24; font-weight: 800;">Winner</span>"###
                        } else {
                            &format!("#{}", rank + 1)
                        };

                        let team_label = if sc.team == 1 {
                            r###"<span style="color: #60a5fa; font-size: 0.8rem; font-weight: 700;">Blue</span>"###
                        } else if sc.team == 2 {
                            r###"<span style="color: #f87171; font-size: 0.8rem; font-weight: 700;">Red</span>"###
                        } else {
                            ""
                        };

                        let grade_info = calculate_grade(sc.accuracy as f32, sc.c_miss as i32);

                        score_rows.push_str(&format!(
                            r###"
                            <tr>
                                <td style="padding: 0.6rem 0.8rem; font-weight: 700; font-family: 'JetBrains Mono', monospace;">{rank_badge}</td>
                                <td style="padding: 0.6rem 0.8rem;">
                                    <div style="display: flex; align-items: center; gap: 0.6rem;">
                                        <img src="/a/{uid}" style="width: 24px; height: 24px; border-radius: 50%; object-fit: cover;">
                                        <a href="/u/{uid}" style="color: var(--text-main); font-weight: 700; text-decoration: none;">{uname}</a>
                                        {team_label}
                                    </div>
                                </td>
                                <td style="padding: 0.6rem 0.8rem; font-weight: 800; font-family: 'JetBrains Mono', monospace; color: var(--primary);">{score}</td>
                                <td style="padding: 0.6rem 0.8rem; font-family: 'JetBrains Mono', monospace;">{combo}x</td>
                                <td style="padding: 0.6rem 0.8rem; font-family: 'JetBrains Mono', monospace;">{acc:.2}%</td>
                                <td style="padding: 0.6rem 0.8rem; font-size: 0.85rem; color: var(--text-muted); font-family: 'JetBrains Mono', monospace;">{hits}</td>
                                <td style="padding: 0.6rem 0.8rem;">
                                    <span style="display: inline-block; padding: 2px 6px; border-radius: 4px; font-weight: 800; font-size: 0.78rem; background: {grade_bg}; color: {grade_color};">{grade}</span>
                                </td>
                            </tr>"###,
                            rank_badge = rank_badge,
                            uid = sc.user_id,
                            uname = html_escape(&sc.username),
                            team_label = team_label,
                            score = format_number(sc.score),
                            combo = sc.max_combo,
                            acc = sc.accuracy,
                            hits = format!("{}/{}/{}/{}", sc.c300, sc.c100, sc.c50, sc.c_miss),
                            grade_bg = grade_info.2,
                            grade_color = grade_info.1,
                            grade = grade_info.0,
                        ));
                    }

                    games_html.push_str(&format!(
                        r###"
                        <div class="game-result-card">
                            <div class="game-result-header">
                                <div style="display: flex; align-items: center; gap: 0.6rem; flex-wrap: wrap;">
                                    <span class="game-round-tag">Round #{num}</span>
                                    <span style="font-weight: 700; color: var(--text-main); font-size: 0.95rem;">{map_name}</span>
                                </div>
                                <div style="display: flex; align-items: center; gap: 1rem; font-size: 0.85rem; color: var(--text-muted);">
                                    <span>Duration: <b>{duration}</b></span>
                                    <span>Winner: <b style="color: #fbbf24;">{winner}</b></span>
                                </div>
                            </div>
                            <div style="overflow-x: auto;">
                                <table class="score-table">
                                    <thead>
                                        <tr>
                                            <th>Rank</th>
                                            <th>Player</th>
                                            <th>Score</th>
                                            <th>Max Combo</th>
                                            <th>Accuracy</th>
                                            <th>300/100/50/Miss</th>
                                            <th>Grade</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {score_rows}
                                    </tbody>
                                </table>
                            </div>
                        </div>"###,
                        num = item.games.len() - idx,
                        map_name = html_escape(&g.game.beatmap_name),
                        duration = duration_str,
                        winner = html_escape(&g.game.winner_name),
                        score_rows = score_rows
                    ));
                }
            }

            rooms_html.push_str(&format!(
                r###"
                <div class="room-container">
                    <div class="room-header">
                        <div class="room-title-group">
                            {status_badge}
                            <h2 class="room-title">{room_name}</h2>
                            <span class="match-id-pill">Room #{mid}</span>
                        </div>
                        <div class="room-host">
                            <img src="/a/{host_id}" class="host-avatar" alt="Host Avatar" onerror="this.src='/a/1'">
                            <div>
                                <div style="font-size: 0.72rem; text-transform: uppercase; color: var(--text-muted); font-weight: 700;">Room Host</div>
                                <a href="/u/{host_id}" class="host-name">{host_name}</a>
                            </div>
                        </div>
                    </div>

                    <div class="room-chips">
                        <span class="chip">Mode: <b>{mode}</b></span>
                        <span class="chip">Condition: <b>{scoring}</b></span>
                        <span class="chip">Format: <b>{team_type}</b></span>
                        <span class="chip">Players: <b>{p_count}/16</b></span>
                    </div>

                    <div class="beatmap-banner">
                        <div style="display: flex; align-items: center; gap: 0.8rem; min-width: 0;">
                            <div class="beatmap-icon">
                                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"></circle><path d="M10 15l5-3-5-3v6z"></path></svg>
                            </div>
                            <div style="min-width: 0;">
                                <div style="font-size: 0.75rem; text-transform: uppercase; color: var(--text-muted); font-weight: 700;">Current Beatmap</div>
                                <div style="font-size: 1.05rem; font-weight: 700; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">{map_link}</div>
                            </div>
                        </div>
                    </div>

                    <div style="margin-top: 1.2rem;">
                        <div style="font-size: 0.85rem; font-weight: 800; text-transform: uppercase; color: var(--text-muted); letter-spacing: 0.5px; margin-bottom: 0.6rem;">
                            Player Slots ({p_count}/16)
                        </div>
                        <div class="slots-grid">
                            {slots_html}
                        </div>
                    </div>

                    <div style="margin-top: 1.5rem;">
                        <div style="font-size: 0.85rem; font-weight: 800; text-transform: uppercase; color: var(--text-muted); letter-spacing: 0.5px; margin-bottom: 0.6rem;">
                            Room Match History ({games_count} games)
                        </div>
                        {games_html}
                    </div>
                </div>"###,
                status_badge = status_badge,
                room_name = html_escape(&r.name),
                mid = r.match_id,
                host_id = r.host_id,
                host_name = html_escape(&r.host_name),
                mode = get_multi_mode_name(r.mode),
                scoring = get_multi_scoring_name(r.scoring_type),
                team_type = get_multi_team_name(r.team_type),
                p_count = r.player_count,
                map_link = map_link,
                slots_html = slots_html,
                games_count = item.games.len(),
                games_html = games_html
            ));
        }
    }

    let html = format!(
        r###"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="icon" type="image/png" href="/static/favicon.png">
    <link rel="shortcut icon" href="/favicon.ico">
    <title>Multiplayer Tracker - {name}</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@400;500;600;700;800;900&family=JetBrains+Mono:wght@500;700;800&display=swap" rel="stylesheet">
    <style>
        {css}
        .multi-badge {{
            display: inline-flex;
            align-items: center;
            gap: 0.4rem;
            padding: 4px 10px;
            border-radius: 6px;
            font-size: 0.75rem;
            font-weight: 800;
            letter-spacing: 0.5px;
            text-transform: uppercase;
        }}
        .multi-badge.playing {{
            background: rgba(239, 68, 68, 0.15);
            color: #f87171;
            border: 1px solid rgba(239, 68, 68, 0.35);
        }}
        .multi-badge.waiting {{
            background: rgba(34, 197, 94, 0.15);
            color: #4ade80;
            border: 1px solid rgba(34, 197, 94, 0.35);
        }}
        .pulse-dot {{
            width: 8px;
            height: 8px;
            border-radius: 50%;
            display: inline-block;
        }}
        .pulse-dot.red {{
            background: #ef4444;
            box-shadow: 0 0 8px #ef4444;
            animation: pulse 1.5s infinite;
        }}
        .pulse-dot.green {{
            background: #22c55e;
            box-shadow: 0 0 8px #22c55e;
            animation: pulse 2s infinite;
        }}
        @keyframes pulse {{
            0% {{ opacity: 1; transform: scale(1); }}
            50% {{ opacity: 0.4; transform: scale(0.85); }}
            100% {{ opacity: 1; transform: scale(1); }}
        }}
        .room-container {{
            background: var(--bg-surface);
            border: 1px solid var(--card-border);
            border-radius: 12px;
            padding: 1.5rem;
            margin-bottom: 2rem;
            box-shadow: 0 4px 20px rgba(0, 0, 0, 0.25);
        }}
        .room-header {{
            display: flex;
            justify-content: space-between;
            align-items: center;
            gap: 1rem;
            flex-wrap: wrap;
            padding-bottom: 1rem;
            border-bottom: 1px solid var(--card-border);
        }}
        .room-title-group {{
            display: flex;
            align-items: center;
            gap: 0.8rem;
            flex-wrap: wrap;
        }}
        .room-title {{
            font-size: 1.35rem;
            font-weight: 800;
            color: var(--text-main);
            margin: 0;
        }}
        .match-id-pill {{
            font-family: 'JetBrains Mono', monospace;
            font-size: 0.8rem;
            font-weight: 700;
            color: var(--text-muted);
            background: var(--bg-surface-hover);
            padding: 2px 8px;
            border-radius: 4px;
            border: 1px solid var(--card-border);
        }}
        .room-host {{
            display: flex;
            align-items: center;
            gap: 0.6rem;
            background: var(--bg-surface-hover);
            padding: 6px 12px;
            border-radius: 8px;
            border: 1px solid var(--card-border);
        }}
        .host-avatar {{
            width: 32px;
            height: 32px;
            border-radius: 50%;
            object-fit: cover;
            border: 1px solid var(--card-border);
        }}
        .host-name {{
            font-weight: 700;
            font-size: 0.95rem;
            color: var(--primary);
            text-decoration: none;
        }}
        .room-chips {{
            display: flex;
            gap: 0.5rem;
            flex-wrap: wrap;
            margin-top: 1rem;
        }}
        .chip {{
            background: var(--bg-surface-hover);
            border: 1px solid var(--card-border);
            padding: 4px 10px;
            border-radius: 6px;
            font-size: 0.82rem;
            color: var(--text-muted);
        }}
        .chip b {{
            color: var(--text-main);
        }}
        .beatmap-banner {{
            background: var(--bg-surface-hover);
            border: 1px solid var(--card-border);
            border-radius: 8px;
            padding: 0.8rem 1.2rem;
            margin-top: 1rem;
            display: flex;
            justify-content: space-between;
            align-items: center;
        }}
        .beatmap-icon {{
            width: 36px;
            height: 36px;
            border-radius: 8px;
            background: rgba(167, 139, 250, 0.15);
            color: var(--primary);
            display: flex;
            align-items: center;
            justify-content: center;
            flex-shrink: 0;
        }}
        .slots-grid {{
            display: grid;
            grid-template-columns: repeat(auto-fill, minmax(230px, 1fr));
            gap: 0.75rem;
        }}
        .slot-box {{
            padding: 0.75rem;
            border-radius: 8px;
            border: 1px solid var(--card-border);
            display: flex;
            align-items: center;
            gap: 0.7rem;
            position: relative;
            background: var(--bg-surface-hover);
        }}
        .slot-box.open {{
            border-style: dashed;
            opacity: 0.6;
            justify-content: center;
        }}
        .slot-box.locked {{
            border-style: dashed;
            opacity: 0.35;
            justify-content: center;
        }}
        .slot-num {{
            position: absolute;
            top: 4px;
            right: 6px;
            font-size: 0.68rem;
            font-family: 'JetBrains Mono', monospace;
            color: var(--text-sub);
            font-weight: 700;
        }}
        .slot-avatar {{
            width: 34px;
            height: 34px;
            border-radius: 50%;
            object-fit: cover;
            border: 1px solid var(--card-border);
            flex-shrink: 0;
        }}
        .slot-info {{
            min-width: 0;
            flex: 1;
        }}
        .slot-username {{
            font-weight: 700;
            font-size: 0.9rem;
            color: var(--text-main);
            text-decoration: none;
            display: block;
            white-space: nowrap;
            overflow: hidden;
            text-overflow: ellipsis;
        }}
        .slot-empty-text {{
            font-size: 0.8rem;
            font-weight: 600;
            color: var(--text-sub);
        }}
        .team-badge {{
            font-size: 0.68rem;
            font-weight: 800;
            padding: 1px 5px;
            border-radius: 4px;
            text-transform: uppercase;
        }}
        .team-badge.blue {{
            background: rgba(59, 130, 246, 0.2);
            color: #60a5fa;
            border: 1px solid rgba(59, 130, 246, 0.4);
        }}
        .team-badge.red {{
            background: rgba(239, 68, 68, 0.2);
            color: #f87171;
            border: 1px solid rgba(239, 68, 68, 0.4);
        }}
        .slot-status-badge {{
            font-size: 0.68rem;
            font-weight: 700;
            padding: 1px 5px;
            border-radius: 4px;
        }}
        .slot-status-badge.host {{
            background: rgba(251, 191, 36, 0.2);
            color: #fbbf24;
        }}
        .slot-status-badge.ready {{
            background: rgba(34, 197, 94, 0.2);
            color: #4ade80;
        }}
        .slot-status-badge.notready {{
            background: rgba(148, 163, 184, 0.15);
            color: var(--text-muted);
        }}
        .slot-status-badge.playing {{
            background: rgba(167, 139, 250, 0.2);
            color: #c084fc;
        }}
        .slot-status-badge.nomap {{
            background: rgba(239, 68, 68, 0.2);
            color: #f87171;
        }}
        .slot-status-badge.complete {{
            background: rgba(14, 165, 233, 0.2);
            color: #38bdf8;
        }}
        .game-result-card {{
            background: var(--bg-surface-hover);
            border: 1px solid var(--card-border);
            border-radius: 8px;
            margin-top: 0.8rem;
            overflow: hidden;
        }}
        .game-result-header {{
            padding: 0.8rem 1rem;
            background: rgba(0, 0, 0, 0.2);
            display: flex;
            justify-content: space-between;
            align-items: center;
            flex-wrap: wrap;
            gap: 0.8rem;
            border-bottom: 1px solid var(--card-border);
        }}
        .game-round-tag {{
            background: var(--primary);
            color: #0b0c10;
            font-family: 'JetBrains Mono', monospace;
            font-size: 0.75rem;
            font-weight: 800;
            padding: 2px 7px;
            border-radius: 4px;
        }}
        .score-table {{
            width: 100%;
            border-collapse: collapse;
            font-size: 0.88rem;
        }}
        .score-table th {{
            text-align: left;
            padding: 0.6rem 0.8rem;
            font-size: 0.75rem;
            text-transform: uppercase;
            letter-spacing: 0.5px;
            color: var(--text-muted);
            border-bottom: 1px solid var(--card-border);
            background: rgba(0, 0, 0, 0.1);
        }}
        .score-table td {{
            border-bottom: 1px solid rgba(255, 255, 255, 0.04);
        }}
        .score-table tr:last-child td {{
            border-bottom: none;
        }}
    </style>
</head>
<body>
    {navbar}

    <main class="main-container">
        <!-- Header Hero -->
        <div style="text-align: center; max-width: 800px; margin: 1rem auto 2rem auto;">
            <div class="hero-tag" style="margin-bottom: 0.8rem;">REAL-TIME MULTIPLAYER TRACKER</div>
            <h1 style="font-size: 2.4rem; font-weight: 900; letter-spacing: -0.5px;">Live Multiplayer Rooms</h1>
            <p style="color: var(--text-muted); font-size: 1.05rem; line-height: 1.6; margin-top: 0.5rem;">
                Live monitoring of osu! lobbies, player slot readiness, beatmaps, and round match results on <b>{name}</b>.
            </p>

            <div style="display: flex; justify-content: center; gap: 0.8rem; flex-wrap: wrap; margin-top: 1.2rem;">
                <div class="chip">Active Rooms: <b style="color: var(--primary);">{total_rooms}</b></div>
                <div class="chip">Players in Multi: <b>{total_players}</b></div>
                <div class="chip">Completed Rounds: <b>{total_matches}</b></div>
                <div class="chip" style="display: flex; align-items: center; gap: 0.4rem;">
                    <span class="pulse-dot green"></span> Live Sync Active
                </div>
            </div>
        </div>

        <!-- Rooms Feed -->
        {rooms_html}
    </main>

    {footer}

    <script>
        // Automatic soft-refresh every 6 seconds to keep live multi state fresh
        setTimeout(function() {{
            window.location.reload();
        }}, 6000);
    </script>
</body>
</html>"###,
        name = name,
        navbar = navbar,
        footer = footer,
        total_rooms = total_rooms,
        total_players = total_players,
        total_matches = total_matches,
        rooms_html = rooms_html,
        css = common_css()
    );

    Html(html)
}

