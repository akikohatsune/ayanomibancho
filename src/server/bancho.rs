#![allow(dead_code)]

use crate::bancho::handler::handle_client_packets;
use crate::bancho::session::Session;
use crate::db::users::{
    check_multiaccount, create_user, get_or_create_stats, get_user_by_username, get_user_rank,
    record_user_hardware, update_user_password,
};
use crate::protocol::packets::*;
use crate::state::AppState;
use crate::utils::crypto::{hash_password, md5_hex, verify_password};
use crate::utils::security::{is_known_vpn_or_datacenter, is_private_or_loopback_ip, parse_client_hashes};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Debug, Deserialize, Serialize)]
pub struct InternalStatsUpdate {
    pub user_id: i32,
    pub mode: u8,
    pub is_relax: Option<bool>,
}

pub async fn internal_stats_update(
    State(state): State<AppState>,
    Json(payload): Json<InternalStatsUpdate>,
) -> Response {
    let info = {
        let st = state.bancho.read().await;
        if let Some(session) = st.get_session_by_user_id(payload.user_id) {
            let is_rx = payload.is_relax.unwrap_or(session.is_relax);
            let eff_mode = if is_rx && payload.mode <= 2 {
                payload.mode + 4
            } else {
                payload.mode
            };
            Some((eff_mode, is_rx))
        } else {
            None
        }
    };

    if let Some((eff_mode, is_rx)) = info {
        let db_stats = get_or_create_stats(&state.db, payload.user_id, eff_mode)
            .await
            .unwrap_or_default();
        let rank = get_user_rank(&state.db, payload.user_id, eff_mode)
            .await
            .unwrap_or(1);

        let mut st = state.bancho.write().await;
        if let Some(session) = st.get_session_by_user_id(payload.user_id) {
            let stats_pkt = session.to_stats(&db_stats, rank);
            let packet = build_user_stats(&stats_pkt);
            st.broadcast(&packet);
            info!(
                "Broadcasted updated stats for user ID {} (mode: {}, eff_mode: {}, rx: {})",
                payload.user_id, payload.mode, eff_mode, is_rx
            );
        }
    }

    StatusCode::OK.into_response()
}

pub async fn bancho_health(State(state): State<AppState>) -> Json<serde_json::Value> {
    let online = { state.bancho.read().await.online_count() };
    let uptime = { state.bancho.read().await.start_time.elapsed().as_secs() };
    Json(serde_json::json!({
        "status": "healthy",
        "service": "bancho",
        "online_players": online,
        "uptime_seconds": uptime,
    }))
}

pub async fn bancho_post_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let osu_token = headers
        .get("osu-token")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

    let client_ip = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_else(|| "127.0.0.1".parse().unwrap());

    match osu_token.filter(|s| !s.is_empty()) {
        None => handle_login(state, client_ip, body).await,
        Some(token) => handle_packet_poll(state, token, body).await,
    }
}

fn bancho_fail_response(fail_pkt: Vec<u8>) -> Response {
    let mut response = (StatusCode::OK, fail_pkt).into_response();
    let h = response.headers_mut();
    h.insert("cho-token", HeaderValue::from_static("None"));
    h.insert("cho-protocol", HeaderValue::from_static("19"));
    h.insert("content-type", HeaderValue::from_static("application/octet-stream"));
    response
}

async fn handle_login(state: AppState, client_ip: std::net::IpAddr, body: Bytes) -> Response {
    let body_str = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid UTF-8 in login payload").into_response();
        }
    };

    let lines: Vec<&str> = body_str.lines().collect();
    if lines.len() < 3 {
        warn!("Login failed: invalid payload format (less than 3 lines)");
        return bancho_fail_response(build_user_id(-1));
    }

    let username = lines[0].trim();
    let password_md5 = lines[1].trim();
    let client_info = lines[2].trim();

    info!("Login attempt for user: '{}' from IP: {}", username, client_ip);

    // Anti-VPN check
    if state.config.security.block_vpn && is_known_vpn_or_datacenter(&client_ip) {
        warn!("Login blocked: Client IP {} detected as VPN/Datacenter Proxy", client_ip);
        let mut fail_pkts = build_user_id(-3); // Banned / Access denied
        fail_pkts.extend_from_slice(&build_notification("VPN or Datacenter Proxy is not allowed on this server."));
        return bancho_fail_response(fail_pkts);
    }

    // Parse client_info: osu_version|utc_offset|display_city|client_hashes|pm_private
    let client_parts: Vec<&str> = client_info.split('|').collect();
    let utc_offset: u8 = if client_parts.len() > 1 {
        client_parts[1].parse::<i8>().unwrap_or(0).wrapping_add(24) as u8
    } else {
        24
    };

    let raw_hashes = client_parts.get(3).unwrap_or(&"");
    let hardware = parse_client_hashes(raw_hashes);

    let clean_login_username = crate::db::badges::clean_username(username);

    // Verify or auto-register user
    let user = match get_user_by_username(&state.db, clean_login_username).await {
        Ok(Some(u)) => {
            let pass_valid = verify_password(password_md5, &u.password_hash)
                || verify_password(&md5_hex(password_md5), &u.password_hash)
                || (u.password_hash.len() == 32 && u.password_hash.eq_ignore_ascii_case(password_md5));

            if !pass_valid {
                warn!("Invalid password for user '{}'", username);
                return bancho_fail_response(build_user_id(-1));
            }

            // Auto-upgrade legacy 32-char plain MD5 hash to bcrypt
            if u.password_hash.len() == 32 && u.password_hash.eq_ignore_ascii_case(password_md5) {
                if let Ok(upgraded) = hash_password(password_md5) {
                    let _ = update_user_password(&state.db, u.id, &upgraded).await;
                }
            }

            // Anti-multiaccount check on existing user (exempt private/localhost IP)
            if state.config.security.anti_multiaccount && !is_private_or_loopback_ip(&client_ip) {
                match check_multiaccount(&state.db, Some(u.id), &hardware, state.config.security.max_accounts_per_hwid).await {
                    Ok(true) => {
                        warn!("Multiaccount violation detected for user '{}'", username);
                        return bancho_fail_response(build_user_id(-4)); // Standard osu! Multiaccount error
                    }
                    Err(e) => warn!("Hardware check error: {}", e),
                    _ => {}
                }
            }

            u
        }
        Ok(None) => {
            if state.config.gameplay.auto_register {
                // Anti-multiaccount check before registration (exempt private/localhost IP)
                if state.config.security.anti_multiaccount && !is_private_or_loopback_ip(&client_ip) {
                    match check_multiaccount(&state.db, None, &hardware, state.config.security.max_accounts_per_hwid).await {
                        Ok(true) => {
                            warn!("Registration blocked: Multiaccount limit exceeded on HWID: {:?}", hardware);
                            return bancho_fail_response(build_user_id(-4));
                        }
                        Err(e) => warn!("Hardware check error: {}", e),
                        _ => {}
                    }
                }

                info!("Auto-registering new user: '{}'", username);
                let hashed = hash_password(password_md5).unwrap_or_else(|_| password_md5.to_string());
                match create_user(
                    &state.db,
                    username,
                    &hashed,
                    &format!("{}@ayanomi.local", username),
                    state.config.gameplay.default_country,
                )
                .await
                {
                    Ok(new_u) => new_u,
                    Err(e) => {
                        error!("Failed to auto-register user: {}", e);
                        return bancho_fail_response(build_user_id(-5));
                    }
                }
            } else {
                warn!("User '{}' not found and auto-register disabled", username);
                return bancho_fail_response(build_user_id(-1));
            }
        }
        Err(e) => {
            error!("Database error during login: {}", e);
            return bancho_fail_response(build_user_id(-5));
        }
    };

    // Record hardware HWID
    let _ = record_user_hardware(&state.db, user.id, &hardware, &client_ip.to_string()).await;

    let session_token = Uuid::new_v4().to_string();

    // Evict any previous session for this user
    {
        let mut st = state.bancho.write().await;
        if let Some(old_token) = st.user_id_to_token.get(&user.id).cloned() {
            st.remove_session(&old_token);
        }
    }

    let effective_privileges = (user.privileges as u32) | crate::protocol::constants::PRIV_SUPPORTER;

    let display_name = crate::db::badges::get_user_display_name(&state.badges_db, user.id, &user.username).await;

    let mut session = Session::new(
        session_token.clone(),
        user.id,
        display_name.clone(),
        utc_offset,
        user.country,
        effective_privileges,
    );

    let db_stats = get_or_create_stats(&state.db, user.id, 0).await.unwrap_or_default();
    let rank = get_user_rank(&state.db, user.id, 0).await.unwrap_or(1);

    // Assemble initial login reply packets
    let mut initial_packets = Vec::new();
    initial_packets.extend_from_slice(&build_protocol_version(19));
    initial_packets.extend_from_slice(&build_user_id(user.id));
    initial_packets.extend_from_slice(&build_bancho_privileges(effective_privileges));
    initial_packets.extend_from_slice(&build_notification(&state.config.server.welcome_message));

    // Channels information
    {
        let st = state.bancho.read().await;
        for ch in st.channels.values() {
            initial_packets.extend_from_slice(&build_channel_info(
                &ch.name,
                &ch.topic,
                ch.user_count(),
            ));
        }
    }
    initial_packets.extend_from_slice(&build_channel_info_end());

    // Auto-join #osu and #announce
    session.channels.insert("#osu".to_string());
    session.channels.insert("#announce".to_string());
    initial_packets.extend_from_slice(&build_channel_join_success("#osu"));
    initial_packets.extend_from_slice(&build_channel_join_success("#announce"));

    // Replay recent message history for auto-joined channels
    let history_limit = state.config.gameplay.chat_history_limit;
    if history_limit > 0 {
        if let Ok(osu_history) = crate::db::chat::get_channel_history(&state.chat_db, "#osu", history_limit).await {
            for msg in osu_history {
                let chat_msg = ChatMessage {
                    sender: msg.sender_name,
                    content: msg.message,
                    target: "#osu".to_string(),
                    sender_id: msg.sender_id as i32,
                };
                initial_packets.extend_from_slice(&build_send_message(&chat_msg));
            }
        }
        if let Ok(ann_history) = crate::db::chat::get_channel_history(&state.chat_db, "#announce", history_limit).await {
            for msg in ann_history {
                let chat_msg = ChatMessage {
                    sender: msg.sender_name,
                    content: msg.message,
                    target: "#announce".to_string(),
                    sender_id: msg.sender_id as i32,
                };
                initial_packets.extend_from_slice(&build_send_message(&chat_msg));
            }
        }
    }

    // BanchoBot presence, stats, and welcome PM
    let bot_name = state.config.gameplay.bot_name.clone();
    let bot_id = state.config.gameplay.bot_id;
    let bot_presence = UserPresence {
        user_id: bot_id,
        username: bot_name.clone(),
        utc_offset: 24,
        country_code: state.config.gameplay.default_country,
        bancho_privileges: (crate::protocol::constants::PRIV_PLAYER | crate::protocol::constants::PRIV_MODERATOR) as u8,
        game_mode: 0,
        longitude: 0.0,
        latitude: 0.0,
        rank: 0,
    };
    let bot_stats = UserStats {
        user_id: bot_id,
        action: 0,
        info_text: "Serving AyanomiBancho!".to_string(),
        map_md5: "".to_string(),
        mods: 0,
        mode: 0,
        map_id: 0,
        ranked_score: 0,
        accuracy: 0.0,
        play_count: 0,
        total_score: 0,
        rank: 0,
        pp: 0,
    };
    initial_packets.extend_from_slice(&build_user_presence(&bot_presence));
    initial_packets.extend_from_slice(&build_user_stats(&bot_stats));

    let welcome_pm = ChatMessage {
        sender: bot_name,
        content: format!("Chào mừng {} đến với AyanomiBancho! Gõ !help để xem các lệnh bot.", display_name),
        target: display_name.clone(),
        sender_id: bot_id,
    };
    initial_packets.extend_from_slice(&build_send_message(&welcome_pm));

    // User's own presence & stats
    let self_presence = session.to_presence(rank);
    let self_stats = session.to_stats(&db_stats, rank);
    initial_packets.extend_from_slice(&build_user_presence(&self_presence));
    initial_packets.extend_from_slice(&build_user_stats(&self_stats));

    // Send existing online users' presence & stats to new player
    let self_presence_pkt = build_user_presence(&self_presence);
    let self_stats_pkt = build_user_stats(&self_stats);

    {
        let mut st = state.bancho.write().await;

        if let Some(ch) = st.channels.get_mut("#osu") {
            ch.add_member(user.id);
        }
        if let Some(ch) = st.channels.get_mut("#announce") {
            ch.add_member(user.id);
        }

        for other in st.sessions.values() {
            let other_presence = other.to_presence(1);
            let other_stats = UserStats {
                user_id: other.user_id,
                action: other.action,
                info_text: other.info_text.clone(),
                map_md5: other.map_md5.clone(),
                mods: other.mods,
                mode: other.mode,
                map_id: other.map_id,
                ranked_score: 0,
                accuracy: 0.0,
                play_count: 0,
                total_score: 0,
                rank: 1,
                pp: 0,
            };
            initial_packets.extend_from_slice(&build_user_presence(&other_presence));
            initial_packets.extend_from_slice(&build_user_stats(&other_stats));
        }

        st.broadcast(&self_presence_pkt);
        st.broadcast(&self_stats_pkt);

        st.add_session(session);
    }

    info!("User '{}' (display: '{}', ID: {}) successfully connected to Bancho", user.username, display_name, user.id);

    let mut response = (StatusCode::OK, initial_packets).into_response();
    let h = response.headers_mut();
    h.insert(
        "cho-token",
        HeaderValue::from_str(&session_token).unwrap_or(HeaderValue::from_static("")),
    );
    h.insert("cho-protocol", HeaderValue::from_static("19"));
    h.insert("content-type", HeaderValue::from_static("application/octet-stream"));
    response
}

async fn handle_packet_poll(state: AppState, token: String, body: Bytes) -> Response {
    let session_exists = {
        let mut st = state.bancho.write().await;
        if let Some(session) = st.get_session_mut(&token) {
            session.last_ping = std::time::Instant::now();
            true
        } else {
            false
        }
    };

    if !session_exists {
        let restart_pkt = build_restart(0);
        let mut response = (StatusCode::OK, restart_pkt).into_response();
        let h = response.headers_mut();
        h.insert("cho-token", HeaderValue::from_static("None"));
        h.insert("cho-protocol", HeaderValue::from_static("19"));
        h.insert("content-type", HeaderValue::from_static("application/octet-stream"));
        return response;
    }

    if !body.is_empty() {
        handle_client_packets(
            &token,
            &body,
            state.bancho.clone(),
            state.db.clone(),
            state.chat_db.clone(),
            state.multi_db.clone(),
            state.config.clone(),
        )
        .await;
    }

    let packets = {
        let mut st = state.bancho.write().await;
        if let Some(session) = st.get_session_mut(&token) {
            session.dequeue_all_packets()
        } else {
            Vec::new()
        }
    };

    let mut response = (StatusCode::OK, packets).into_response();
    let h = response.headers_mut();
    h.insert("cho-protocol", HeaderValue::from_static("19"));
    h.insert("content-type", HeaderValue::from_static("application/octet-stream"));
    response
}
