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

pub fn render_navbar(active: &str, server_name: &str, user: Option<&User>) -> String {
    let is_home = if active == "home" { "class='active'" } else { "" };
    let is_lb = if active == "leaderboard" { "class='active'" } else { "" };
    let is_multi = if active == "multi" { "class='active'" } else { "" };
    let is_rule = if active == "rule" { "class='active'" } else { "" };
    let is_staff = if active == "staff" { "class='active'" } else { "" };
    let is_connect = if active == "connect" { "class='active'" } else { "" };
    let is_login = if active == "login" { "class='active'" } else { "" };

    let nav_actions = match user {
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

    crate::server::templates::render_template(
        "navbar",
        &[
            ("SERVER_NAME", server_name),
            ("IS_HOME", is_home),
            ("IS_LB", is_lb),
            ("IS_MULTI", is_multi),
            ("IS_RULE", is_rule),
            ("IS_STAFF", is_staff),
            ("IS_CONNECT", is_connect),
            ("IS_LOGIN", is_login),
            ("NAV_ACTIONS", &nav_actions),
        ],
    )
}

fn render_footer(server_name: &str) -> String {
    crate::server::templates::render_template(
        "footer",
        &[("SERVER_NAME", server_name)],
    )
}

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
    let footer = render_footer(&state.config.server.name);

    let online_count_str = online_count.to_string();
    let total_users_str = format_number(total_users);
    let total_scores_str = format_number(total_scores);

    let html = crate::server::templates::render_page(
        "index",
        "Home",
        &state.config.server.name,
        &navbar,
        &footer,
        "",
        "",
        &[
            ("SERVER_NAME", &state.config.server.name),
            ("ONLINE_COUNT", &online_count_str),
            ("TOTAL_USERS", &total_users_str),
            ("TOTAL_SCORES", &total_scores_str),
            ("PLAYERS_HTML", &players_html),
            ("RANKERS_HTML", &rankers_html),
            ("MATCHES_HTML", &matches_html),
        ],
    );

    Html(html)
}

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
    let footer = render_footer(&state.config.server.name);

    let tab_std = tab_btn(0, "Standard");
    let tab_taiko = tab_btn(1, "Taiko");
    let tab_ctb = tab_btn(2, "Catch");
    let tab_mania = tab_btn(3, "Mania");
    let tab_rx_std = tab_btn(4, "RX Std");
    let tab_rx_taiko = tab_btn(5, "RX Taiko");
    let tab_rx_ctb = tab_btn(6, "RX Catch");

    let html = crate::server::templates::render_page(
        "leaderboard",
        "Global Leaderboard",
        &state.config.server.name,
        &navbar,
        &footer,
        "",
        "",
        &[
            ("MODE_NAME", mode_name),
            ("TAB_STD", &tab_std),
            ("TAB_TAIKO", &tab_taiko),
            ("TAB_CTB", &tab_ctb),
            ("TAB_MANIA", &tab_mania),
            ("TAB_RX_STD", &tab_rx_std),
            ("TAB_RX_TAIKO", &tab_rx_taiko),
            ("TAB_RX_CTB", &tab_rx_ctb),
            ("TABLE_ROWS", &table_rows),
        ],
    );

    Html(html)
}

pub async fn profile_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i32>,
) -> Html<String> {
    let current_user = get_authenticated_user(&state, &headers).await;
    let user = match get_user_by_id(&state.db, user_id).await {
        Ok(Some(u)) => u,
        _ => {
            let not_found_msg = format!("Player with ID #{} does not exist on this server.", user_id);
            let not_found_html = crate::server::templates::render_page(
                "404",
                "Player Not Found",
                &state.config.server.name,
                &render_navbar("", &state.config.server.name, current_user.as_ref()),
                &render_footer(&state.config.server.name),
                "",
                "",
                &[("MESSAGE", &not_found_msg)],
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

    // profile_js now served via /static/js/profile.js


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
    let footer = render_footer(&state.config.server.name);

        let user_id_str = user.id.to_string();
    let avatar_ver_str = avatar_ver.to_string();
    let banner_ver_str = banner_ver.to_string();
    let rank_std_str = rank_std.to_string();
    let pp_std_str = format_number(stats_std.pp as i64);
    let acc_std_str = format!("{:.2}%", stats_std.accuracy);
    let plays_std_str = format_number(stats_std.play_count as i64);
    let pp_taiko_str = format_number(stats_taiko.pp as i64);
    let acc_taiko_str = format!("{:.2}%", stats_taiko.accuracy);
    let plays_taiko_str = format_number(stats_taiko.play_count as i64);
    let pp_ctb_str = format_number(stats_ctb.pp as i64);
    let acc_ctb_str = format!("{:.2}%", stats_ctb.accuracy);
    let plays_ctb_str = format_number(stats_ctb.play_count as i64);
    let pp_mania_str = format_number(stats_mania.pp as i64);
    let acc_mania_str = format!("{:.2}%", stats_mania.accuracy);
    let plays_mania_str = format_number(stats_mania.play_count as i64);

    let extra_js = format!(
        r###"<script src="https://cdn.jsdelivr.net/npm/marked/marked.min.js"></script>
        <script>const INITIAL_RAW_BIO = {raw_bio_json};</script>
        <script src="/static/js/profile.js"></script>"###,
        raw_bio_json = raw_bio_json
    );

    let html = crate::server::templates::render_page(
        "profile",
        &clean_name,
        &state.config.server.name,
        &navbar,
        &footer,
        "",
        &extra_js,
        &[
            ("USER_ID", &user_id_str),
            ("BANNER_VER", &banner_ver_str),
            ("AVATAR_VER", &avatar_ver_str),
            ("USERNAME", &html_escape(clean_name)),
            ("COVER_ACTIONS_TOP", &cover_actions_top),
            ("AVATAR_OVERLAY", &avatar_overlay),
            ("PREFIX_TAG", &prefix_tag),
            ("COUNTRY_BADGE_HTML", &country_badge_html),
            ("JOIN_DATE", &join_date),
            ("RANK_STD", &rank_std_str),
            ("BIO_EDIT_BTN", &bio_edit_btn),
            ("BIO_EDIT_SECTION", &bio_edit_section),
            ("BADGES_HTML", &badges_html),
            ("PP_STD", &pp_std_str),
            ("ACC_STD", &acc_std_str),
            ("PLAYS_STD", &plays_std_str),
            ("PP_TAIKO", &pp_taiko_str),
            ("ACC_TAIKO", &acc_taiko_str),
            ("PLAYS_TAIKO", &plays_taiko_str),
            ("PP_CTB", &pp_ctb_str),
            ("ACC_CTB", &acc_ctb_str),
            ("PLAYS_CTB", &plays_ctb_str),
            ("PP_MANIA", &pp_mania_str),
            ("ACC_MANIA", &acc_mania_str),
            ("PLAYS_MANIA", &plays_mania_str),
            ("SCORES_HTML", &scores_html),
            ("COUNTRY_MODAL_HTML", &country_modal_html),
        ],
    );

    Html(html)
}

pub async fn login_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Html<String> {
    let current_user = get_authenticated_user(&state, &headers).await;
    let navbar = render_navbar("login", &state.config.server.name, current_user.as_ref());
    let footer = render_footer(&state.config.server.name);

    match current_user {
        Some(user) => {
            let country = bancho_id_to_country(user.country);
            let user_id_str = user.id.to_string();
            let html = crate::server::templates::render_page(
                "account",
                "Account",
                &state.config.server.name,
                &navbar,
                &footer,
                "",
                "",
                &[
                    ("USER_ID", &user_id_str),
                    ("USERNAME", &html_escape(&user.username)),
                    ("COUNTRY_CODE", country.code),
                ],
            );
            Html(html)
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

            let extra_js = r#"<script src="https://challenges.cloudflare.com/turnstile/v0/api.js" async defer></script><script src="/static/js/login.js"></script>"#;

            let html = crate::server::templates::render_page(
                "login",
                "Sign In",
                &state.config.server.name,
                &navbar,
                &footer,
                "",
                extra_js,
                &[
                    ("SERVER_NAME", &state.config.server.name),
                    ("TURNSTILE_WIDGET", &turnstile_widget),
                ],
            );
            Html(html)
        }
    }
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
        let login_html = crate::server::templates::render_page(
            "admin_login",
            "Admin Gate",
            &state.config.server.name,
            &render_navbar("admin", &state.config.server.name, None),
            &render_footer(&state.config.server.name),
            "",
            "",
            &[("SERVER_NAME", &state.config.server.name)],
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

    let ram_used_str = format!("{:.1}", ram_used);
    let ram_total_str = format!("{:.1}", ram_total);
    let ram_pct_str = format!("{:.1}", ram_pct);
    let db_size_str = format_number(db_size as i64);
    let chat_db_size_str = format_number(chat_db_size as i64);
    let badges_db_size_str = format_number(badges_db_size as i64);
    let uptime_str = format!("{}m", uptime_sec / 60);
    let ratelimit_blocked_str = format_number(ratelimit_blocked as i64);
    let ratelimit_ips_str = format_number(ratelimit_ips as i64);

    let extra_js = r#"<script src="/static/js/admin.js"></script>"#;

    let html = crate::server::templates::render_page(
        "admin",
        "Admin Dashboard",
        &state.config.server.name,
        &render_navbar("admin", &state.config.server.name, None),
        &render_footer(&state.config.server.name),
        "",
        extra_js,
        &[
            ("SERVER_NAME", &state.config.server.name),
            ("RAM_USED", &ram_used_str),
            ("RAM_TOTAL", &ram_total_str),
            ("RAM_PCT", &ram_pct_str),
            ("DB_SIZE", &db_size_str),
            ("CHAT_DB_SIZE", &chat_db_size_str),
            ("BADGES_DB_SIZE", &badges_db_size_str),
            ("UPTIME_STR", &uptime_str),
            ("RATELIMIT_BLOCKED", &ratelimit_blocked_str),
            ("RATELIMIT_IPS", &ratelimit_ips_str),
            ("BADGES_HTML", &badges_html),
            ("BADGE_SELECT_OPTIONS", &badge_select_options),
            ("CHAT_HTML", &chat_html),
            ("BACKGROUNDS_HTML", &backgrounds_html),
        ],
    );

    Html(html)
}

pub async fn connect_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Html<String> {
    let current_user = get_authenticated_user(&state, &headers).await;
    let navbar = render_navbar("connect", &state.config.server.name, current_user.as_ref());
    let footer = render_footer(&state.config.server.name);

    let extra_js = r#"<script src="/static/js/connect.js"></script>"#;

    let html = crate::server::templates::render_page(
        "connect",
        "How to Connect",
        &state.config.server.name,
        &navbar,
        &footer,
        "",
        extra_js,
        &[
            ("SERVER_NAME", &state.config.server.name),
            ("SERVER_DOMAIN", &state.config.server.domain),
        ],
    );

    Html(html)
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
    let footer = render_footer(&state.config.server.name);

    let html = crate::server::templates::render_page(
        "rules",
        "Server Rules & Community Guidelines",
        &state.config.server.name,
        &navbar,
        &footer,
        "",
        "",
        &[("SERVER_NAME", &state.config.server.name)],
    );

    Html(html)
}

pub async fn staff_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Html<String> {
    let current_user = get_authenticated_user(&state, &headers).await;
    let navbar = render_navbar("staff", &state.config.server.name, current_user.as_ref());
    let footer = render_footer(&state.config.server.name);

    let html = crate::server::templates::render_page(
        "staff",
        "Staff & Credits",
        &state.config.server.name,
        &navbar,
        &footer,
        "",
        "",
        &[("SERVER_NAME", &state.config.server.name)],
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
    let footer = render_footer(&state.config.server.name);
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

    let total_rooms_str = total_rooms.to_string();
    let total_players_str = total_players.to_string();
    let total_matches_str = total_matches.to_string();

    let html = crate::server::templates::render_page(
        "multi",
        "Live Multiplayer Rooms",
        &state.config.server.name,
        &navbar,
        &footer,
        "",
        "",
        &[
            ("SERVER_NAME", &state.config.server.name),
            ("TOTAL_ROOMS", &total_rooms_str),
            ("TOTAL_PLAYERS", &total_players_str),
            ("TOTAL_MATCHES", &total_matches_str),
            ("ROOMS_HTML", &rooms_html),
        ],
    );

    Html(html)
}
