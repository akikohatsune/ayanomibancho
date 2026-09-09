#![allow(dead_code)]

use crate::db::scores::get_user_recent_scores;
use crate::db::users::{get_or_create_stats, get_user_by_username, get_user_rank};
use crate::db::DbPool;
use crate::protocol::packets::{build_send_message, build_user_stats, ChatMessage};
use rand::Rng;
use std::sync::Arc;
use tokio::sync::RwLock;

pub fn format_mods(mods: u32) -> String {
    if mods == 0 {
        return "None".to_string();
    }
    let mut s = String::new();
    if (mods & 1) != 0 { s.push_str("NF"); }
    if (mods & 2) != 0 { s.push_str("EZ"); }
    if (mods & 8) != 0 { s.push_str("HD"); }
    if (mods & 16) != 0 { s.push_str("HR"); }
    if (mods & 32) != 0 { s.push_str("SD"); }
    if (mods & 64) != 0 { s.push_str("DT"); }
    if (mods & 128) != 0 { s.push_str("RX"); }
    if (mods & 256) != 0 { s.push_str("HT"); }
    if (mods & 512) != 0 { s.push_str("NC"); }
    if (mods & 1024) != 0 { s.push_str("FL"); }
    if (mods & 2048) != 0 { s.push_str("Auto"); }
    if (mods & 4096) != 0 { s.push_str("SO"); }
    if (mods & 8192) != 0 { s.push_str("AP"); }
    if (mods & 16384) != 0 { s.push_str("PF"); }
    if s.is_empty() { "None".to_string() } else { format!("+{}", s) }
}

fn format_action(action: u8, info_text: &str) -> String {
    match action {
        0 => "Idle".to_string(),
        1 => "AFK".to_string(),
        2 => if info_text.is_empty() { "Playing".to_string() } else { format!("Playing: {}", info_text) },
        3 => if info_text.is_empty() { "Editing a beatmap".to_string() } else { format!("Editing: {}", info_text) },
        4 => if info_text.is_empty() { "Modding a beatmap".to_string() } else { format!("Modding: {}", info_text) },
        5 => "in Multiplayer match".to_string(),
        6 => if info_text.is_empty() { "Watching".to_string() } else { format!("Watching: {}", info_text) },
        8 => "Testing a beatmap".to_string(),
        9 => "Submitting score".to_string(),
        10 => "Paused".to_string(),
        11 => "in Multiplayer Lobby".to_string(),
        13 => "in osu!Direct".to_string(),
        _ => "Online".to_string(),
    }
}

pub async fn handle_bot_command(
    command_text: &str,
    sender_username: &str,
    sender_id: i32,
    target_channel: &str,
    bot_name: &str,
    bot_id: i32,
    state: &Arc<RwLock<super::state::BanchoState>>,
    db: &DbPool,
) -> Option<Vec<u8>> {
    let trimmed = command_text.trim();
    if !trimmed.starts_with('!') {
        return None;
    }

    let parts: Vec<&str> = trimmed[1..].split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let cmd = parts[0].to_lowercase();
    let reply_target = if target_channel.starts_with('#') {
        target_channel
    } else {
        sender_username
    };

    let reply_text = match cmd.as_str() {
        "help" => {
            "Available commands: !help, !relax [on/off] (!rx), !roll [n], !stats [user] [rx], !recent [user] (!r), !where [user], !uptime, !ping. Server: AyanomiBancho".to_string()
        }
        "relax" | "rx" => {
            let (new_state, p_mode) = {
                let mut st = state.write().await;
                if let Some(session) = st.get_session_by_user_id_mut(sender_id) {
                    let desired = match parts.get(1).map(|s| s.to_lowercase()).as_deref() {
                        Some("on") | Some("1") | Some("true") => true,
                        Some("off") | Some("0") | Some("false") => false,
                        _ => !session.is_relax,
                    };
                    session.is_relax = desired;
                    (desired, session.mode)
                } else {
                    return Some(build_send_message(&ChatMessage {
                        sender: bot_name.to_string(),
                        content: "Không tìm thấy phiên làm việc của bạn.".to_string(),
                        target: reply_target.to_string(),
                        sender_id: bot_id,
                    }));
                }
            };

            let stats_mode = if new_state && p_mode <= 2 {
                p_mode + 4
            } else {
                p_mode
            };

            let db_stats = get_or_create_stats(db, sender_id, stats_mode)
                .await
                .unwrap_or_default();
            let rank = get_user_rank(db, sender_id, stats_mode).await.unwrap_or(1);

            let pkt = {
                let st = state.read().await;
                if let Some(session) = st.get_session_by_user_id(sender_id) {
                    let stats_pkt = session.to_stats(&db_stats, rank);
                    build_user_stats(&stats_pkt)
                } else {
                    Vec::new()
                }
            };

            if !pkt.is_empty() {
                let mut st = state.write().await;
                st.broadcast(&pkt);
            }

            let mode_name = match p_mode {
                0 => "osu! Standard",
                1 => "Taiko",
                2 => "Catch The Beat",
                3 => "osu!mania",
                _ => "osu!",
            };

            if new_state {
                format!(
                    "Relax mode: BẬT [ON] cho {}. Bảng điểm và xếp hạng bây giờ sẽ tính riêng cho Relax (RX)! PP: {} | Rank: #{}",
                    mode_name, db_stats.pp, rank
                )
            } else {
                format!(
                    "Relax mode: TẮT [OFF] cho {}. Trở về chế độ Standard/Vanilla bình thường! PP: {} | Rank: #{}",
                    mode_name, db_stats.pp, rank
                )
            }
        }
        "roll" => {
            let max_val: u32 = if parts.len() > 1 {
                parts[1].parse().unwrap_or(100).max(1)
            } else {
                100
            };
            let mut rng = rand::thread_rng();
            let rolled = rng.gen_range(1..=max_val);
            format!("{} rolls {} point(s) (1-{})!", sender_username, rolled, max_val)
        }
        "ping" => "Pong! Latency: ~1ms (local Bancho)".to_string(),
        "uptime" => {
            let elapsed = {
                let st = state.read().await;
                st.start_time.elapsed().as_secs()
            };
            let hours = elapsed / 3600;
            let minutes = (elapsed % 3600) / 60;
            let seconds = elapsed % 60;
            format!("Server Uptime: {}h {}m {}s", hours, minutes, seconds)
        }
        "stats" => {
            let (target_id, target_name, is_relax) = if parts.len() > 1 {
                let raw_name = parts[1];
                let name = crate::db::badges::clean_username(raw_name);
                let rx_arg = parts.get(2).map(|s| s.eq_ignore_ascii_case("rx") || s.eq_ignore_ascii_case("relax")).unwrap_or(false);
                if let Ok(Some(u)) = get_user_by_username(db, name).await {
                    let rx = rx_arg || {
                        let st = state.read().await;
                        st.sessions.values().find(|s| s.user_id == u.id).map(|s| s.is_relax).unwrap_or(false)
                    };
                    (u.id, u.username, rx)
                } else {
                    return Some(build_send_message(&ChatMessage {
                        sender: bot_name.to_string(),
                        content: format!("User '{}' not found.", raw_name),
                        target: reply_target.to_string(),
                        sender_id: bot_id,
                    }));
                }
            } else {
                let rx = {
                    let st = state.read().await;
                    st.sessions.values().find(|s| s.user_id == sender_id).map(|s| s.is_relax).unwrap_or(false)
                };
                (sender_id, sender_username.to_string(), rx)
            };

            let stats_mode = if is_relax { 4 } else { 0 };
            let rank = get_user_rank(db, target_id, stats_mode).await.unwrap_or(1);
            if let Ok(stats) = get_or_create_stats(db, target_id, stats_mode).await {
                let mode_label = if is_relax { " (Relax)" } else { "" };
                format!(
                    "Stats for {}{} (#{rank}): PP: {} | Accuracy: {:.2}% | Ranked Score: {} | Plays: {}",
                    target_name, mode_label, stats.pp, stats.accuracy, stats.ranked_score, stats.play_count
                )
            } else {
                "Failed to retrieve user statistics.".to_string()
            }
        }
        "recent" | "r" => {
            let (target_id, target_name) = if parts.len() > 1 {
                let raw_name = parts[1];
                let name = crate::db::badges::clean_username(raw_name);
                if let Ok(Some(u)) = get_user_by_username(db, name).await {
                    (u.id, u.username)
                } else {
                    return Some(build_send_message(&ChatMessage {
                        sender: bot_name.to_string(),
                        content: format!("User '{}' not found.", raw_name),
                        target: reply_target.to_string(),
                        sender_id: bot_id,
                    }));
                }
            } else {
                (sender_id, sender_username.to_string())
            };

            let recent = get_user_recent_scores(db, target_id, 1).await.unwrap_or_default();
            if let Some(sc) = recent.first() {
                let mods_str = format_mods(sc.mods);
                format!(
                    "Recent play for {}: Map [{}] | Score: {} | Combo: {}x | 300: {}, 100: {}, 50: {}, Miss: {} | Mods: {}",
                    target_name, sc.map_md5, sc.score, sc.max_combo, sc.c300, sc.c100, sc.c50, sc.c_miss, mods_str
                )
            } else {
                format!("No recent plays found for {}.", target_name)
            }
        }
        "where" => {
            let target_name = if parts.len() > 1 {
                parts[1]
            } else {
                sender_username
            };
            let clean_target = crate::db::badges::clean_username(target_name);

            let user_info = {
                let st = state.read().await;
                st.sessions
                    .values()
                    .find(|s| s.username.eq_ignore_ascii_case(target_name) || crate::db::badges::clean_username(&s.username).eq_ignore_ascii_case(clean_target))
                    .map(|s| (s.username.clone(), s.action, s.info_text.clone()))
            };

            if let Some((uname, action, info_text)) = user_info {
                let action_str = format_action(action, &info_text);
                format!("{} is currently {}.", uname, action_str)
            } else {
                format!("{} is currently offline.", target_name)
            }
        }
        _ => return None,
    };

    let chat_msg = ChatMessage {
        sender: bot_name.to_string(),
        content: reply_text,
        target: reply_target.to_string(),
        sender_id: bot_id,
    };

    Some(build_send_message(&chat_msg))
}
