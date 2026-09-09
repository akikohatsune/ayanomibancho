use crate::bancho::bot::handle_bot_command;
use crate::bancho::state::BanchoState;
use crate::config::Config;
use crate::db::matches::{save_match_result, NewMatchScore};
use crate::db::users::{get_or_create_stats, get_user_rank};
use crate::db::DbPool;
use crate::protocol::constants::*;
use crate::protocol::packets::*;
use crate::protocol::reader::PacketReader;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

pub fn get_slot_status_text(status: u8) -> &'static str {
    if (status & SLOT_PLAYING) > 0 {
        "Playing"
    } else if (status & SLOT_READY) > 0 {
        "Ready"
    } else if (status & SLOT_NOT_READY) > 0 {
        "Not Ready"
    } else if (status & SLOT_NO_MAP) > 0 {
        "No Map"
    } else if (status & SLOT_COMPLETE) > 0 {
        "Complete"
    } else if status == SLOT_LOCKED {
        "Locked"
    } else {
        "Open"
    }
}

pub fn sync_match_to_multi_db(multi_db: Arc<DbPool>, st: &BanchoState, match_id: u16) {
    if let Some(m) = st.matches.get(&match_id) {
        let host_name = st
            .get_session_by_user_id(m.host_id)
            .map(|s| s.username.clone())
            .unwrap_or_else(|| format!("User {}", m.host_id));

        let mut slots = Vec::with_capacity(16);
        for (i, slot) in m.slots.iter().enumerate() {
            let username = if slot.user_id > 0 {
                st.get_session_by_user_id(slot.user_id)
                    .map(|s| s.username.clone())
                    .unwrap_or_else(|| format!("Player {}", slot.user_id))
            } else {
                String::new()
            };

            slots.push(crate::db::multi::LiveSlotInfo {
                slot_id: i,
                user_id: slot.user_id,
                username,
                status: slot.status,
                status_text: get_slot_status_text(slot.status).to_string(),
                team: slot.team,
                mods: slot.mods,
            });
        }

        let player_count = m
            .slots
            .iter()
            .filter(|s| (s.status & SLOT_HAS_PLAYER) > 0 && s.user_id > 0)
            .count() as i64;
        let slots_json = serde_json::to_string(&slots).unwrap_or_else(|_| "[]".to_string());
        let now = chrono::Utc::now().timestamp();

        let db_room = crate::db::multi::DbLiveRoom {
            match_id: m.id as i64,
            name: m.name.clone(),
            host_id: m.host_id as i64,
            host_name,
            beatmap_id: m.beatmap_id as i64,
            beatmap_name: m.beatmap_name.clone(),
            beatmap_md5: m.beatmap_md5.clone(),
            mode: m.play_mode as i64,
            scoring_type: m.scoring_type as i64,
            team_type: m.team_type as i64,
            mods: m.active_mods as i64,
            in_progress: if m.in_progress { 1 } else { 0 },
            player_count,
            slots_json,
            created_at: now,
            updated_at: now,
        };

        tokio::spawn(async move {
            let _ = crate::db::multi::upsert_room(&multi_db, &db_room).await;
        });
    }
}

pub fn cleanup_disbanded_matches(multi_db: Arc<DbPool>, st: &mut BanchoState) {
    let disbanded = st.take_disbanded_matches();
    for mid in disbanded {
        let pool = multi_db.clone();
        tokio::spawn(async move {
            let _ = crate::db::multi::delete_room_data(&pool, mid).await;
        });
    }
}

pub async fn handle_client_packets(
    token: &str,
    data: &[u8],
    state: Arc<RwLock<BanchoState>>,
    db: Arc<DbPool>,
    chat_db: Arc<DbPool>,
    multi_db: Arc<DbPool>,
    config: Arc<Config>,
) {
    let mut reader = PacketReader::new(data);

    while !reader.is_empty() {
        let (packet_id, length) = match reader.read_packet_header() {
            Ok(Some(h)) => h,
            _ => break,
        };

        let payload = match reader.read_bytes(length) {
            Ok(p) => p,
            Err(_) => break,
        };

        let mut payload_reader = PacketReader::new(&payload);

        let (user_id, username) = {
            let st = state.read().await;
            match st.get_session(token) {
                Some(s) => (s.user_id, s.username.clone()),
                None => break,
            }
        };

        match packet_id {
            OSU_PONG => {
                let mut st = state.write().await;
                if let Some(session) = st.get_session_mut(token) {
                    session.last_ping = std::time::Instant::now();
                }
            }

            OSU_CHANGE_ACTION => {
                if let (Ok(action), Ok(info_text), Ok(map_md5), Ok(mods), Ok(mode), Ok(map_id)) = (
                    payload_reader.read_u8(),
                    payload_reader.read_osu_string(),
                    payload_reader.read_osu_string(),
                    payload_reader.read_u32(),
                    payload_reader.read_u8(),
                    payload_reader.read_i32(),
                ) {
                    let mut st = state.write().await;
                    let (stats_pkt, user_id) = {
                        if let Some(session) = st.get_session_mut(token) {
                            session.action = action;
                            session.info_text = info_text;
                            session.map_md5 = map_md5;
                            session.mods = mods;
                            session.mode = mode;
                            session.map_id = map_id;
                            session.last_ping = std::time::Instant::now();
                            (session.clone(), session.user_id)
                        } else {
                            continue;
                        }
                    };

                    drop(st);

                    let eff_mode = if (stats_pkt.is_relax || (stats_pkt.mods & 128) != 0) && mode <= 2 {
                        mode + 4
                    } else {
                        mode
                    };
                    let db_stats = get_or_create_stats(&db, user_id, eff_mode).await.unwrap_or_default();
                    let rank = get_user_rank(&db, user_id, eff_mode).await.unwrap_or(1);
                    let user_stats = stats_pkt.to_stats(&db_stats, rank);
                    let pkt = build_user_stats(&user_stats);

                    let mut st = state.write().await;
                    st.broadcast(&pkt);
                }
            }

            OSU_SEND_PUBLIC_MESSAGE => {
                let _sender = payload_reader.read_osu_string().unwrap_or_default();
                let raw_content = payload_reader.read_osu_string().unwrap_or_default();
                let target = payload_reader.read_osu_string().unwrap_or_default();
                let _sender_id = payload_reader.read_i32().unwrap_or(0);

                let (sender_username, sender_id, content) = {
                    let st = state.read().await;
                    if let Some(session) = st.get_session(token) {
                        let filtered = st.filter.filter(&raw_content);
                        (session.username.clone(), session.user_id, filtered)
                    } else {
                        continue;
                    }
                };

                info!("[Chat] {} -> {}: {}", sender_username, target, content);

                // Asynchronously log public chat to dedicated chat database
                let chat_db_clone = chat_db.clone();
                let s_id = sender_id;
                let s_name = sender_username.clone();
                let t_name = target.clone();
                let c_msg = content.clone();
                tokio::spawn(async move {
                    let _ = crate::db::chat::save_chat_message(
                        &chat_db_clone,
                        s_id,
                        &s_name,
                        &t_name,
                        &c_msg,
                        false,
                    )
                    .await;
                });

                // Check for bot command first
                if let Some(bot_reply) = handle_bot_command(
                    &content,
                    &sender_username,
                    sender_id,
                    &target,
                    &config.gameplay.bot_name,
                    config.gameplay.bot_id,
                    &state,
                    &db,
                )
                .await
                {
                    let mut st = state.write().await;
                    if target.starts_with('#') {
                        st.broadcast_to_channel(&target, &bot_reply);
                    } else if let Some(session) = st.get_session_mut(token) {
                        session.enqueue_packet(&bot_reply);
                    }
                } else {
                    let chat_msg = ChatMessage {
                        sender: sender_username,
                        content,
                        target: target.clone(),
                        sender_id,
                    };
                    let msg_pkt = build_send_message(&chat_msg);
                    let mut st = state.write().await;
                    st.broadcast_to_channel_except(&target, &msg_pkt, sender_id);
                }
            }

            OSU_SEND_PRIVATE_MESSAGE => {
                let _sender = payload_reader.read_osu_string().unwrap_or_default();
                let raw_content = payload_reader.read_osu_string().unwrap_or_default();
                let target_user = payload_reader.read_osu_string().unwrap_or_default();
                let _sender_id = payload_reader.read_i32().unwrap_or(0);

                let (sender_username, sender_id, content) = {
                    let st = state.read().await;
                    if let Some(session) = st.get_session(token) {
                        let filtered = st.filter.filter(&raw_content);
                        (session.username.clone(), session.user_id, filtered)
                    } else {
                        continue;
                    }
                };

                // Asynchronously log private chat to dedicated chat database
                let chat_db_clone = chat_db.clone();
                let s_id = sender_id;
                let s_name = sender_username.clone();
                let t_name = target_user.clone();
                let c_msg = content.clone();
                tokio::spawn(async move {
                    let _ = crate::db::chat::save_chat_message(
                        &chat_db_clone,
                        s_id,
                        &s_name,
                        &t_name,
                        &c_msg,
                        true,
                    )
                    .await;
                });

                // Check if target is bot
                if target_user.eq_ignore_ascii_case(&config.gameplay.bot_name) {
                    if let Some(bot_reply) = handle_bot_command(
                        &content,
                        &sender_username,
                        sender_id,
                        &sender_username,
                        &config.gameplay.bot_name,
                        config.gameplay.bot_id,
                        &state,
                        &db,
                    )
                    .await
                    {
                        let mut st = state.write().await;
                        if let Some(session) = st.get_session_mut(token) {
                            session.enqueue_packet(&bot_reply);
                        }
                    }
                } else {
                    let target_token = {
                        let st = state.read().await;
                        let clean_target = crate::db::badges::clean_username(&target_user);
                        st.sessions
                            .values()
                            .find(|s| {
                                s.username.eq_ignore_ascii_case(&target_user)
                                    || crate::db::badges::clean_username(&s.username).eq_ignore_ascii_case(clean_target)
                            })
                            .map(|s| s.token.clone())
                    };

                    if let Some(ref t_token) = target_token {
                        let chat_msg = ChatMessage {
                            sender: sender_username,
                            content,
                            target: target_user,
                            sender_id,
                        };
                        let msg_pkt = build_send_message(&chat_msg);

                        let mut st = state.write().await;
                        if let Some(session) = st.get_session_mut(t_token) {
                            session.enqueue_packet(&msg_pkt);
                        }
                    }
                }
            }

            OSU_CHANNEL_JOIN => {
                if let Ok(ch_name) = payload_reader.read_osu_string() {
                    let mut st = state.write().await;
                    let user_id = if let Some(session) = st.get_session_mut(token) {
                        session.channels.insert(ch_name.clone());
                        session.enqueue_packet(&build_channel_join_success(&ch_name));
                        session.user_id
                    } else {
                        continue;
                    };

                    if let Some(ch) = st.channels.get_mut(&ch_name) {
                        ch.add_member(user_id);
                    }
                }
            }

            OSU_CHANNEL_PART => {
                if let Ok(ch_name) = payload_reader.read_osu_string() {
                    let mut st = state.write().await;
                    let user_id = if let Some(session) = st.get_session_mut(token) {
                        session.channels.remove(&ch_name);
                        session.enqueue_packet(&build_channel_part(&ch_name));
                        session.user_id
                    } else {
                        continue;
                    };

                    if let Some(ch) = st.channels.get_mut(&ch_name) {
                        ch.remove_member(user_id);
                    }
                }
            }

            OSU_REQUEST_STATUS_UPDATE => {
                let st = state.read().await;
                if let Some(session) = st.get_session(token) {
                    let user_id = session.user_id;
                    let mode = session.mode;
                    let is_relax = session.is_relax;
                    let session_clone = session.clone();
                    drop(st);

                    let eff_mode = if is_relax && mode <= 2 {
                        mode + 4
                    } else {
                        mode
                    };
                    let db_stats = get_or_create_stats(&db, user_id, eff_mode).await.unwrap_or_default();
                    let rank = get_user_rank(&db, user_id, eff_mode).await.unwrap_or(1);
                    let pkt = build_user_stats(&session_clone.to_stats(&db_stats, rank));

                    let mut st = state.write().await;
                    if let Some(s) = st.get_session_mut(token) {
                        s.enqueue_packet(&pkt);
                    }
                }
            }

            OSU_USER_STATS_REQUEST => {
                let count = payload_reader.read_i16().unwrap_or(0);
                if count > 0 && count <= 256 {
                    let mut requested_ids = Vec::with_capacity(count as usize);
                    for _ in 0..count {
                        if let Ok(uid) = payload_reader.read_i32() {
                            requested_ids.push(uid);
                        }
                    }

                    let bot_id = config.gameplay.bot_id;
                    for uid in requested_ids {
                        if uid == bot_id {
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
                            let pkt = build_user_stats(&bot_stats);
                            let mut st = state.write().await;
                            if let Some(session) = st.get_session_mut(token) {
                                session.enqueue_packet(&pkt);
                            }
                        } else {
                            let session_info = {
                                let st = state.read().await;
                                st.sessions.values().find(|s| s.user_id == uid).cloned()
                            };

                            let stats = if let Some(target_session) = session_info {
                                let eff_mode = if target_session.is_relax && target_session.mode <= 2 {
                                    target_session.mode + 4
                                } else {
                                    target_session.mode
                                };
                                let db_stats = get_or_create_stats(&db, uid, eff_mode).await.unwrap_or_default();
                                let rank = get_user_rank(&db, uid, eff_mode).await.unwrap_or(1);
                                target_session.to_stats(&db_stats, rank)
                            } else {
                                let db_stats = get_or_create_stats(&db, uid, 0).await.unwrap_or_default();
                                let rank = get_user_rank(&db, uid, 0).await.unwrap_or(1);
                                UserStats {
                                    user_id: uid,
                                    action: 0,
                                    info_text: "".to_string(),
                                    map_md5: "".to_string(),
                                    mods: 0,
                                    mode: 0,
                                    map_id: 0,
                                    ranked_score: db_stats.ranked_score,
                                    accuracy: db_stats.accuracy as f32,
                                    play_count: db_stats.play_count as i32,
                                    total_score: db_stats.total_score,
                                    rank: rank as i32,
                                    pp: db_stats.pp as i16,
                                }
                            };

                            let pkt = build_user_stats(&stats);
                            let mut st = state.write().await;
                            if let Some(session) = st.get_session_mut(token) {
                                session.enqueue_packet(&pkt);
                            }
                        }
                    }
                }
            }

            OSU_RECEIVE_UPDATES => {
                let _filter = payload_reader.read_i32().unwrap_or(0);
            }

            // --- Spectator Mode ---
            OSU_START_SPECTATING => {
                if let Ok(target_id) = payload_reader.read_i32() {
                    let mut st = state.write().await;
                    if let Some(old_host) = st.stop_spectating(user_id) {
                        let left_pkt = build_spectator_left(user_id);
                        st.send_to_user(old_host, &left_pkt);
                        let fellow_left = build_fellow_spectator_left(user_id);
                        st.broadcast_to_spectators(old_host, &fellow_left);
                    }

                    if st.user_id_to_token.contains_key(&target_id) && target_id != user_id {
                        let fellow_join = build_fellow_spectator_joined(user_id);
                        st.broadcast_to_spectators(target_id, &fellow_join);

                        if let Some(existing_specs) = st.spectators.get(&target_id).cloned() {
                            for spec_id in existing_specs {
                                let fellow_existing = build_fellow_spectator_joined(spec_id);
                                st.send_to_user(user_id, &fellow_existing);
                            }
                        }

                        st.spectators.entry(target_id).or_default().push(user_id);
                        st.spectating_target.insert(user_id, target_id);

                        let join_pkt = build_spectator_joined(user_id);
                        st.send_to_user(target_id, &join_pkt);

                        if let Some(session) = st.get_session_mut(token) {
                            session.channels.insert("#spectator".to_string());
                            session.enqueue_packet(&build_channel_join_success("#spectator"));
                        }
                        if let Some(ch) = st.channels.get_mut("#spectator") {
                            ch.add_member(user_id);
                        }
                    }
                }
            }

            OSU_STOP_SPECTATING => {
                let mut st = state.write().await;
                if let Some(host_id) = st.stop_spectating(user_id) {
                    let left_pkt = build_spectator_left(user_id);
                    st.send_to_user(host_id, &left_pkt);
                    let fellow_left = build_fellow_spectator_left(user_id);
                    st.broadcast_to_spectators(host_id, &fellow_left);
                }
            }

            OSU_SPECTATE_FRAMES => {
                let mut st = state.write().await;
                let frames_pkt = build_spectator_frames(&payload);
                st.broadcast_to_spectators(user_id, &frames_pkt);
            }

            OSU_CANT_SPECTATE => {
                let mut st = state.write().await;
                if let Some(&host_id) = st.spectating_target.get(&user_id) {
                    let cant_pkt = build_spectator_cant_spectate(user_id);
                    st.send_to_user(host_id, &cant_pkt);
                    st.broadcast_to_spectators(host_id, &cant_pkt);
                }
            }

            // --- Multiplayer Lobby & Match ---
            OSU_LOBBY_JOIN => {
                let mut st = state.write().await;
                st.lobby_subscribers.insert(user_id);
                let match_packets: Vec<Vec<u8>> = st.matches.values().map(build_match_new).collect();
                if let Some(session) = st.get_session_mut(token) {
                    for pkt in match_packets {
                        session.enqueue_packet(&pkt);
                    }
                }
            }

            OSU_LOBBY_PART => {
                let mut st = state.write().await;
                st.lobby_subscribers.remove(&user_id);
            }

            OSU_MATCH_CREATE => {
                if let Ok(mut new_match) = parse_match(&mut payload_reader) {
                    let mut st = state.write().await;
                    if let Some((old_id, updated_match, is_disbanded)) = st.remove_user_from_match(user_id) {
                        if is_disbanded {
                            st.broadcast_to_lobby(&build_match_disband(old_id as i32));
                        } else {
                            let update_pkt = build_match_update(&updated_match);
                            st.broadcast_to_match(old_id, &update_pkt, None);
                            st.broadcast_to_lobby(&update_pkt);
                        }
                    }

                    let match_id = st.next_match_id;
                    st.next_match_id = st.next_match_id.wrapping_add(1);
                    if st.next_match_id == 0 { st.next_match_id = 1; }

                    new_match.id = match_id;
                    new_match.host_id = user_id;
                    new_match.in_progress = false;
                    new_match.slots = [MatchSlot::default(); 16];
                    new_match.slots[0].status = SLOT_NOT_READY;
                    new_match.slots[0].user_id = user_id;

                    let join_success_pkt = build_match_join_success(&new_match);
                    let match_new_pkt = build_match_new(&new_match);

                    st.matches.insert(match_id, new_match.clone());
                    st.user_to_match.insert(user_id, match_id);

                    if let Some(session) = st.get_session_mut(token) {
                        session.enqueue_packet(&join_success_pkt);
                        session.channels.insert("#multiplayer".to_string());
                        session.enqueue_packet(&build_channel_join_success("#multiplayer"));
                    }
                    if let Some(ch) = st.channels.get_mut("#multiplayer") {
                        ch.add_member(user_id);
                    }

                    st.broadcast_to_lobby(&match_new_pkt);
                    info!("User '{}' created match #{} '{}'", username, match_id, new_match.name);
                    sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                    cleanup_disbanded_matches(multi_db.clone(), &mut st);
                }
            }

            OSU_MATCH_JOIN => {
                if let (Ok(match_id_i32), Ok(password)) = (payload_reader.read_i32(), payload_reader.read_osu_string()) {
                    let match_id = match_id_i32 as u16;
                    let mut st = state.write().await;

                    let can_join = match st.matches.get(&match_id) {
                        Some(m) => {
                            if let Some(ref m_pass) = m.password {
                                if !m_pass.is_empty() && m_pass != &password {
                                    false
                                } else {
                                    m.slots.iter().any(|s| s.status == SLOT_OPEN)
                                }
                            } else {
                                m.slots.iter().any(|s| s.status == SLOT_OPEN)
                            }
                        }
                        None => false,
                    };

                    if !can_join {
                        if let Some(session) = st.get_session_mut(token) {
                            session.enqueue_packet(&build_match_join_fail());
                        }
                        continue;
                    }

                    if let Some((old_id, updated_match, is_disbanded)) = st.remove_user_from_match(user_id) {
                        if is_disbanded {
                            st.broadcast_to_lobby(&build_match_disband(old_id as i32));
                        } else {
                            let update_pkt = build_match_update(&updated_match);
                            st.broadcast_to_match(old_id, &update_pkt, None);
                            st.broadcast_to_lobby(&update_pkt);
                        }
                    }

                    let join_info = if let Some(m) = st.matches.get_mut(&match_id) {
                        if let Some(slot) = m.slots.iter_mut().find(|s| s.status == SLOT_OPEN) {
                            slot.status = SLOT_NOT_READY;
                            slot.user_id = user_id;
                            slot.team = TEAM_NEUTRAL;
                            slot.mods = 0;
                        }

                        let join_pkt = build_match_join_success(m);
                        let update_pkt = build_match_update(m);
                        let m_name = m.name.clone();
                        Some((join_pkt, update_pkt, m_name))
                    } else {
                        None
                    };

                    if let Some((join_pkt, update_pkt, m_name)) = join_info {
                        st.user_to_match.insert(user_id, match_id);

                        if let Some(session) = st.get_session_mut(token) {
                            session.enqueue_packet(&join_pkt);
                            session.channels.insert("#multiplayer".to_string());
                            session.enqueue_packet(&build_channel_join_success("#multiplayer"));
                        }
                        if let Some(ch) = st.channels.get_mut("#multiplayer") {
                            ch.add_member(user_id);
                        }

                        st.broadcast_to_match(match_id, &update_pkt, Some(user_id));
                        st.broadcast_to_lobby(&update_pkt);
                        info!("User '{}' joined match #{} '{}'", username, match_id, m_name);
                        sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                        cleanup_disbanded_matches(multi_db.clone(), &mut st);
                    }
                }
            }

            OSU_MATCH_PART => {
                let mut st = state.write().await;
                if let Some((match_id, updated_match, is_disbanded)) = st.remove_user_from_match(user_id) {
                    if is_disbanded {
                        st.broadcast_to_lobby(&build_match_disband(match_id as i32));
                        info!("Match #{} disbanded.", match_id);
                        cleanup_disbanded_matches(multi_db.clone(), &mut st);
                    } else {
                        let update_pkt = build_match_update(&updated_match);
                        st.broadcast_to_match(match_id, &update_pkt, None);
                        st.broadcast_to_lobby(&update_pkt);
                        sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                    }
                }
            }

            OSU_MATCH_CHANGE_SLOT => {
                if let Ok(slot_id_i32) = payload_reader.read_i32() {
                    let new_slot = slot_id_i32 as usize;
                    let mut st = state.write().await;
                    let update_pkt = if let Some(&match_id) = st.user_to_match.get(&user_id) {
                        if let Some(m) = st.matches.get_mut(&match_id) {
                            if new_slot < 16 && m.slots[new_slot].status == SLOT_OPEN {
                                if let Some(old_slot) = m.slots.iter_mut().find(|s| s.user_id == user_id) {
                                    let old_user_id = old_slot.user_id;
                                    let old_status = old_slot.status;
                                    let old_team = old_slot.team;
                                    let old_mods = old_slot.mods;
                                    *old_slot = MatchSlot::default();

                                    m.slots[new_slot].user_id = old_user_id;
                                    m.slots[new_slot].status = old_status;
                                    m.slots[new_slot].team = old_team;
                                    m.slots[new_slot].mods = old_mods;

                                    Some((match_id, build_match_update(m)))
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some((match_id, pkt)) = update_pkt {
                        st.broadcast_to_match(match_id, &pkt, None);
                        st.broadcast_to_lobby(&pkt);
                        sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                    }
                }
            }

            OSU_MATCH_READY => {
                let mut st = state.write().await;
                let update_pkt = if let Some(&match_id) = st.user_to_match.get(&user_id) {
                    if let Some(m) = st.matches.get_mut(&match_id) {
                        if let Some(slot) = m.slots.iter_mut().find(|s| s.user_id == user_id) {
                            slot.status = SLOT_READY;
                            Some((match_id, build_match_update(m)))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some((match_id, pkt)) = update_pkt {
                    st.broadcast_to_match(match_id, &pkt, None);
                    st.broadcast_to_lobby(&pkt);
                    sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                }
            }

            OSU_MATCH_NOT_READY => {
                let mut st = state.write().await;
                let update_pkt = if let Some(&match_id) = st.user_to_match.get(&user_id) {
                    if let Some(m) = st.matches.get_mut(&match_id) {
                        if let Some(slot) = m.slots.iter_mut().find(|s| s.user_id == user_id) {
                            slot.status = SLOT_NOT_READY;
                            Some((match_id, build_match_update(m)))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some((match_id, pkt)) = update_pkt {
                    st.broadcast_to_match(match_id, &pkt, None);
                    st.broadcast_to_lobby(&pkt);
                    sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                }
            }

            OSU_MATCH_LOCK => {
                if let Ok(slot_id_i32) = payload_reader.read_i32() {
                    let slot_id = slot_id_i32 as usize;
                    let mut st = state.write().await;
                    let update_pkt = if let Some(&match_id) = st.user_to_match.get(&user_id) {
                        if let Some(m) = st.matches.get_mut(&match_id) {
                            if m.host_id == user_id && slot_id < 16 {
                                let cur = m.slots[slot_id].status;
                                if cur == SLOT_OPEN {
                                    m.slots[slot_id].status = SLOT_LOCKED;
                                } else if cur == SLOT_LOCKED {
                                    m.slots[slot_id].status = SLOT_OPEN;
                                }
                                Some((match_id, build_match_update(m)))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some((match_id, pkt)) = update_pkt {
                        st.broadcast_to_match(match_id, &pkt, None);
                        st.broadcast_to_lobby(&pkt);
                        sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                    }
                }
            }

            OSU_MATCH_CHANGE_SETTINGS => {
                if let Ok(updated) = parse_match(&mut payload_reader) {
                    let mut st = state.write().await;
                    let update_pkt = if let Some(&match_id) = st.user_to_match.get(&user_id) {
                        if let Some(m) = st.matches.get_mut(&match_id) {
                            if m.host_id == user_id {
                                m.name = updated.name;
                                m.beatmap_name = updated.beatmap_name;
                                m.beatmap_id = updated.beatmap_id;
                                m.beatmap_md5 = updated.beatmap_md5;
                                m.active_mods = updated.active_mods;
                                m.play_mode = updated.play_mode;
                                m.scoring_type = updated.scoring_type;
                                m.team_type = updated.team_type;
                                m.freemod = updated.freemod;

                                for slot in m.slots.iter_mut() {
                                    if slot.user_id > 0 && slot.user_id != user_id && slot.status == SLOT_READY {
                                        slot.status = SLOT_NOT_READY;
                                    }
                                }

                                Some((match_id, build_match_update(m)))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some((match_id, pkt)) = update_pkt {
                        st.broadcast_to_match(match_id, &pkt, None);
                        st.broadcast_to_lobby(&pkt);
                        sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                    }
                }
            }

            OSU_MATCH_CHANGE_MODS => {
                if let Ok(mods) = payload_reader.read_u32() {
                    let mut st = state.write().await;
                    let update_pkt = if let Some(&match_id) = st.user_to_match.get(&user_id) {
                        if let Some(m) = st.matches.get_mut(&match_id) {
                            if m.freemod {
                                if let Some(slot) = m.slots.iter_mut().find(|s| s.user_id == user_id) {
                                    slot.mods = mods;
                                    Some((match_id, build_match_update(m)))
                                } else {
                                    None
                                }
                            } else if m.host_id == user_id {
                                m.active_mods = mods;
                                Some((match_id, build_match_update(m)))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some((match_id, pkt)) = update_pkt {
                        st.broadcast_to_match(match_id, &pkt, None);
                        sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                    }
                }
            }

            OSU_MATCH_CHANGE_TEAM => {
                let mut st = state.write().await;
                let update_pkt = if let Some(&match_id) = st.user_to_match.get(&user_id) {
                    if let Some(m) = st.matches.get_mut(&match_id) {
                        if let Some(slot) = m.slots.iter_mut().find(|s| s.user_id == user_id) {
                            slot.team = if slot.team == TEAM_BLUE { TEAM_RED } else { TEAM_BLUE };
                            Some((match_id, build_match_update(m)))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some((match_id, pkt)) = update_pkt {
                    st.broadcast_to_match(match_id, &pkt, None);
                    sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                }
            }

            OSU_MATCH_CHANGE_HOST => {
                if let Ok(slot_id_i32) = payload_reader.read_i32() {
                    let slot_id = slot_id_i32 as usize;
                    let mut st = state.write().await;
                    let host_info = if let Some(&match_id) = st.user_to_match.get(&user_id) {
                        if let Some(m) = st.matches.get_mut(&match_id) {
                            if m.host_id == user_id && slot_id < 16 && (m.slots[slot_id].status & SLOT_HAS_PLAYER) > 0 {
                                let new_host = m.slots[slot_id].user_id;
                                m.host_id = new_host;
                                Some((match_id, new_host, build_match_update(m)))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some((match_id, new_host, pkt)) = host_info {
                        st.send_to_user(new_host, &build_match_transfer_host());
                        st.broadcast_to_match(match_id, &pkt, None);
                        st.broadcast_to_lobby(&pkt);
                        sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                    }
                }
            }

            OSU_MATCH_NO_BEATMAP => {
                let mut st = state.write().await;
                let update_pkt = if let Some(&match_id) = st.user_to_match.get(&user_id) {
                    if let Some(m) = st.matches.get_mut(&match_id) {
                        if let Some(slot) = m.slots.iter_mut().find(|s| s.user_id == user_id) {
                            slot.status = SLOT_NO_MAP;
                            Some((match_id, build_match_update(m)))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some((match_id, pkt)) = update_pkt {
                    st.broadcast_to_match(match_id, &pkt, None);
                    sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                }
            }

            OSU_MATCH_HAS_BEATMAP => {
                let mut st = state.write().await;
                let update_pkt = if let Some(&match_id) = st.user_to_match.get(&user_id) {
                    if let Some(m) = st.matches.get_mut(&match_id) {
                        if let Some(slot) = m.slots.iter_mut().find(|s| s.user_id == user_id) {
                            slot.status = SLOT_NOT_READY;
                            Some((match_id, build_match_update(m)))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some((match_id, pkt)) = update_pkt {
                    st.broadcast_to_match(match_id, &pkt, None);
                    sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                }
            }

            OSU_MATCH_START => {
                let mut st = state.write().await;
                let start_info = if let Some(&match_id) = st.user_to_match.get(&user_id) {
                    if let Some(m) = st.matches.get_mut(&match_id) {
                        if m.host_id == user_id {
                            m.in_progress = true;
                            for slot in m.slots.iter_mut() {
                                if (slot.status & (SLOT_READY | SLOT_NOT_READY)) > 0 {
                                    slot.status = SLOT_PLAYING;
                                }
                            }
                            Some((match_id, build_match_start(m), build_match_update(m), m.name.clone()))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some((match_id, start_pkt, update_pkt, m_name)) = start_info {
                    st.match_start_times.insert(match_id, std::time::Instant::now());
                    st.match_last_scores.insert(match_id, HashMap::new());
                    st.broadcast_to_match(match_id, &start_pkt, None);
                    st.broadcast_to_lobby(&update_pkt);
                    info!("Match #{} '{}' started gameplay", match_id, m_name);
                    sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                }
            }

            OSU_MATCH_LOAD_COMPLETE => {
                let mut st = state.write().await;
                if let Some(&match_id) = st.user_to_match.get(&user_id) {
                    let all_loaded_pkt = build_match_all_players_loaded();
                    st.broadcast_to_match(match_id, &all_loaded_pkt, None);
                }
            }

            OSU_MATCH_SCORE_UPDATE => {
                if let Ok(mut frame) = parse_score_frame(&mut payload_reader) {
                    let mut st = state.write().await;
                    if let Some(&match_id) = st.user_to_match.get(&user_id) {
                        let score_pkt_opt = if let Some(m) = st.matches.get(&match_id) {
                            if let Some(slot_idx) = m.slots.iter().position(|s| s.user_id == user_id) {
                                frame.slot_id = slot_idx as u8;
                                Some(build_match_score_update(&frame))
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        if let Some(score_pkt) = score_pkt_opt {
                            st.match_last_scores
                                .entry(match_id)
                                .or_default()
                                .insert(user_id, frame);
                            st.broadcast_to_match(match_id, &score_pkt, Some(user_id));
                        }
                    }
                }
            }

            OSU_MATCH_FAILED => {
                let mut st = state.write().await;
                if let Some(&match_id) = st.user_to_match.get(&user_id) {
                    let fail_pkt_opt = if let Some(m) = st.matches.get(&match_id) {
                        m.slots.iter().position(|s| s.user_id == user_id).map(|idx| build_match_player_failed(idx as u32))
                    } else {
                        None
                    };
                    if let Some(fail_pkt) = fail_pkt_opt {
                        st.broadcast_to_match(match_id, &fail_pkt, None);
                    }
                }
            }

            OSU_MATCH_SKIP_REQUEST => {
                let mut st = state.write().await;
                if let Some(&match_id) = st.user_to_match.get(&user_id) {
                    let skip_pkt_opt = if let Some(m) = st.matches.get(&match_id) {
                        m.slots.iter().position(|s| s.user_id == user_id).map(|idx| build_match_player_skipped(idx as u32))
                    } else {
                        None
                    };
                    if let Some(skip_pkt) = skip_pkt_opt {
                        st.broadcast_to_match(match_id, &skip_pkt, None);
                    }
                }
            }

            OSU_MATCH_COMPLETE => {
                let (match_id_opt, should_finish) = {
                    let mut st = state.write().await;
                    if let Some(&m_id) = st.user_to_match.get(&user_id) {
                        if let Some(m) = st.matches.get_mut(&m_id) {
                            if let Some(slot) = m.slots.iter_mut().find(|s| s.user_id == user_id) {
                                slot.status = SLOT_COMPLETE;
                            }
                            let still_playing = m.slots.iter().any(|s| s.status == SLOT_PLAYING);
                            (Some(m_id), !still_playing)
                        } else {
                            (None, false)
                        }
                    } else {
                        (None, false)
                    }
                };

                if let (Some(match_id), true) = (match_id_opt, should_finish) {
                    let mut st = state.write().await;
                    let duration = st
                        .match_start_times
                        .remove(&match_id)
                        .map(|t| t.elapsed().as_secs() as i64)
                        .unwrap_or(0);
                    let last_scores = st.match_last_scores.remove(&match_id).unwrap_or_default();

                    let complete_pkt = build_match_complete();
                    st.broadcast_to_match(match_id, &complete_pkt, None);

                    let mut user_names = HashMap::new();
                    for s in st.sessions.values() {
                        user_names.insert(s.user_id, s.username.clone());
                    }

                    if let Some(m) = st.matches.get_mut(&match_id) {
                        m.in_progress = false;

                        let mut match_scores = Vec::new();
                        let mut winner_id = -1;
                        let mut winner_name = String::new();
                        let mut highest_score = -1;

                        for (i, slot) in m.slots.iter().enumerate() {
                            if (slot.status & SLOT_HAS_PLAYER) > 0 && slot.user_id > 0 {
                                let u_name = user_names
                                    .get(&slot.user_id)
                                    .cloned()
                                    .unwrap_or_else(|| format!("Player {}", slot.user_id));

                                let (score, max_combo, accuracy, c300, c100, c50, c_miss, c_geki, c_katu, passed) =
                                    if let Some(frame) = last_scores.get(&slot.user_id) {
                                        let total_hits = (frame.total_300 + frame.total_100 + frame.total_50 + frame.total_miss) as f32;
                                        let acc = if total_hits > 0.0 {
                                            ((frame.total_300 as f32 * 300.0 + frame.total_100 as f32 * 100.0 + frame.total_50 as f32 * 50.0)
                                                / (total_hits * 300.0))
                                                * 100.0
                                        } else {
                                            0.0
                                        };
                                        (
                                            frame.total_score,
                                            frame.max_combo,
                                            acc,
                                            frame.total_300,
                                            frame.total_100,
                                            frame.total_50,
                                            frame.total_miss,
                                            frame.total_geki,
                                            frame.total_katu,
                                            true,
                                        )
                                    } else {
                                        (0, 0, 0.0, 0, 0, 0, 0, 0, 0, false)
                                    };

                                if score > highest_score {
                                    highest_score = score;
                                    winner_id = slot.user_id;
                                    winner_name = u_name.clone();
                                }

                                match_scores.push(NewMatchScore {
                                    user_id: slot.user_id,
                                    username: u_name,
                                    slot_id: i as u8,
                                    team: slot.team,
                                    score,
                                    max_combo,
                                    accuracy,
                                    c300,
                                    c100,
                                    c50,
                                    c_miss,
                                    c_geki,
                                    c_katu,
                                    passed,
                                    won: false,
                                });
                            }
                        }

                        let match_name = m.name.clone();
                        let beatmap_id = m.beatmap_id;
                        let beatmap_name = m.beatmap_name.clone();
                        let beatmap_md5 = m.beatmap_md5.clone();
                        let mode = m.play_mode;
                        let scoring_type = m.scoring_type;
                        let team_type = m.team_type;
                        let mods = m.active_mods;

                        if team_type == TEAM_TYPE_TEAM_VS {
                            let mut blue_score: i64 = 0;
                            let mut red_score: i64 = 0;
                            for sc in &match_scores {
                                if sc.team == TEAM_BLUE {
                                    blue_score += sc.score as i64;
                                } else if sc.team == TEAM_RED {
                                    red_score += sc.score as i64;
                                }
                            }
                            let winning_team = if blue_score > red_score {
                                TEAM_BLUE
                            } else if red_score > blue_score {
                                TEAM_RED
                            } else {
                                TEAM_NEUTRAL
                            };
                            for sc in match_scores.iter_mut() {
                                if sc.team == winning_team && winning_team != TEAM_NEUTRAL {
                                    sc.won = true;
                                }
                            }
                            let team_str = if winning_team == TEAM_BLUE {
                                "Blue Team"
                            } else if winning_team == TEAM_RED {
                                "Red Team"
                            } else {
                                "Draw"
                            };
                            winner_name = team_str.to_string();
                        } else {
                            for sc in match_scores.iter_mut() {
                                if sc.user_id == winner_id {
                                    sc.won = true;
                                }
                            }
                        }

                        for slot in m.slots.iter_mut() {
                            if (slot.status & SLOT_HAS_PLAYER) > 0 {
                                slot.status = SLOT_NOT_READY;
                            }
                        }

                        let update_pkt = build_match_update(m);

                        st.broadcast_to_match(match_id, &update_pkt, None);
                        st.broadcast_to_lobby(&update_pkt);

                        if team_type == TEAM_TYPE_TEAM_VS {
                            let announce_msg = format!("Team Vs match concluded! Winner: {}", winner_name);
                            let chat_pkt = build_send_message(&ChatMessage {
                                sender: config.gameplay.bot_name.clone(),
                                content: announce_msg.clone(),
                                target: "#multiplayer".to_string(),
                                sender_id: config.gameplay.bot_id,
                            });
                            st.broadcast_to_channel("#multiplayer", &chat_pkt);
                            let chat_db_clone = chat_db.clone();
                            let bot_name = config.gameplay.bot_name.clone();
                            let bot_id = config.gameplay.bot_id;
                            tokio::spawn(async move {
                                let _ = crate::db::chat::save_chat_message(
                                    &chat_db_clone,
                                    bot_id,
                                    &bot_name,
                                    "#multiplayer",
                                    &announce_msg,
                                    false,
                                )
                                .await;
                            });
                        } else if !winner_name.is_empty() {
                            let announce_msg = format!("Match concluded! Winner: {} ({} points)", winner_name, highest_score);
                            let chat_pkt = build_send_message(&ChatMessage {
                                sender: config.gameplay.bot_name.clone(),
                                content: announce_msg.clone(),
                                target: "#multiplayer".to_string(),
                                sender_id: config.gameplay.bot_id,
                            });
                            st.broadcast_to_channel("#multiplayer", &chat_pkt);
                            let chat_db_clone = chat_db.clone();
                            let bot_name = config.gameplay.bot_name.clone();
                            let bot_id = config.gameplay.bot_id;
                            tokio::spawn(async move {
                                let _ = crate::db::chat::save_chat_message(
                                    &chat_db_clone,
                                    bot_id,
                                    &bot_name,
                                    "#multiplayer",
                                    &announce_msg,
                                    false,
                                )
                                .await;
                            });
                        }

                        let db_clone = db.clone();
                        let m_name_for_save = match_name.clone();
                        let b_name_for_save = beatmap_name.clone();
                        let b_md5_for_save = beatmap_md5.clone();
                        let w_name_for_save = winner_name.clone();
                        let scores_for_save = match_scores.clone();
                        tokio::spawn(async move {
                            if let Err(e) = save_match_result(
                                &db_clone,
                                &m_name_for_save,
                                beatmap_id,
                                &b_name_for_save,
                                &b_md5_for_save,
                                mode,
                                scoring_type,
                                team_type,
                                mods,
                                duration,
                                winner_id,
                                &w_name_for_save,
                                &scores_for_save,
                            )
                            .await
                            {
                                tracing::error!("Failed to save multiplayer match result to database: {}", e);
                            } else {
                                tracing::info!("Saved multiplayer match result for '{}' to database.", m_name_for_save);
                            }
                        });

                        let multi_db_clone = multi_db.clone();
                        let scores_clone = match_scores;
                        let b_name = beatmap_name;
                        let b_md5 = beatmap_md5;
                        let w_name = winner_name;
                        tokio::spawn(async move {
                            let _ = crate::db::multi::record_multi_game(
                                &multi_db_clone,
                                match_id,
                                beatmap_id,
                                &b_name,
                                &b_md5,
                                mode,
                                scoring_type,
                                team_type,
                                mods,
                                duration,
                                winner_id,
                                &w_name,
                                &scores_clone,
                            )
                            .await;
                        });

                        sync_match_to_multi_db(multi_db.clone(), &st, match_id);
                    }
                }
            }

            OSU_LOGOUT => {
                let mut st = state.write().await;
                if let Some(session) = st.remove_session(token) {
                    info!("Player {} logged out.", session.username);
                    let quit_pkt = build_user_quit(session.user_id, 0);
                    st.broadcast(&quit_pkt);
                    cleanup_disbanded_matches(multi_db.clone(), &mut st);
                }
                break;
            }

            other => {
                debug!("Unhandled client packet id: {}", other);
            }
        }
    }
}
