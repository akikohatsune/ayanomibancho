#![allow(dead_code)]

use super::channels::{default_channels, Channel};
use super::session::Session;
use crate::protocol::constants::*;
use crate::protocol::packets::{
    build_fellow_spectator_left, build_match_disband, build_match_update, build_spectator_left,
    Match, MatchScoreFrame, MatchSlot,
};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

pub struct BanchoState {
    pub sessions: HashMap<String, Session>,
    pub user_id_to_token: HashMap<i32, String>,
    pub channels: HashMap<String, Channel>,
    pub start_time: Instant,

    // Multiplayer State
    pub matches: HashMap<u16, Match>,
    pub next_match_id: u16,
    pub user_to_match: HashMap<i32, u16>,
    pub lobby_subscribers: HashSet<i32>,
    pub match_start_times: HashMap<u16, Instant>,
    pub match_last_scores: HashMap<u16, HashMap<i32, MatchScoreFrame>>,
    pub match_loaded_users: HashMap<u16, HashSet<i32>>,
    pub pending_disbanded_matches: Vec<u16>,

    // Spectator State
    pub spectators: HashMap<i32, Vec<i32>>,       // host_id -> [spectator_id]
    pub spectating_target: HashMap<i32, i32>,     // spectator_id -> host_id

    // Chat Filter & Private Message Security
    pub filter: super::filter::ChatFilter,
    pub pm_warned_pairs: HashSet<(i32, i32)>,
}

impl BanchoState {
    pub fn new() -> Self {
        let mut channels = HashMap::new();
        for ch in default_channels() {
            channels.insert(ch.name.clone(), ch);
        }

        Self {
            sessions: HashMap::new(),
            user_id_to_token: HashMap::new(),
            channels,
            start_time: Instant::now(),
            matches: HashMap::new(),
            next_match_id: 1,
            user_to_match: HashMap::new(),
            lobby_subscribers: HashSet::new(),
            match_start_times: HashMap::new(),
            match_last_scores: HashMap::new(),
            match_loaded_users: HashMap::new(),
            pending_disbanded_matches: Vec::new(),
            spectators: HashMap::new(),
            spectating_target: HashMap::new(),
            filter: super::filter::ChatFilter::load_or_create("data/filters.txt"),
            pm_warned_pairs: HashSet::new(),
        }
    }

    pub fn add_session(&mut self, session: Session) {
        self.user_id_to_token.insert(session.user_id, session.token.clone());
        self.sessions.insert(session.token.clone(), session);
    }

    pub fn get_session(&self, token: &str) -> Option<&Session> {
        self.sessions.get(token)
    }

    pub fn get_session_mut(&mut self, token: &str) -> Option<&mut Session> {
        self.sessions.get_mut(token)
    }

    pub fn get_session_by_user_id(&self, user_id: i32) -> Option<&Session> {
        let token = self.user_id_to_token.get(&user_id)?;
        self.sessions.get(token)
    }

    pub fn get_session_by_user_id_mut(&mut self, user_id: i32) -> Option<&mut Session> {
        let token = self.user_id_to_token.get(&user_id)?;
        self.sessions.get_mut(token)
    }

    pub fn remove_session(&mut self, token: &str) -> Option<Session> {
        if let Some(session) = self.sessions.remove(token) {
            self.user_id_to_token.remove(&session.user_id);
            let user_id = session.user_id;

            // Remove from lobby
            self.lobby_subscribers.remove(&user_id);

            // Remove from match if any
            if let Some((match_id, updated_match, is_disbanded)) = self.remove_user_from_match(user_id) {
                if is_disbanded {
                    let disband_pkt = build_match_disband(match_id as i32);
                    self.broadcast_to_lobby(&disband_pkt);
                } else {
                    let update_pkt = build_match_update(&updated_match);
                    self.broadcast_to_match(match_id, &update_pkt, None);
                    self.broadcast_to_lobby(&update_pkt);
                }
            }

            // Spectator cleanup (if user was spectating someone)
            if let Some(host_id) = self.stop_spectating(user_id) {
                let spec_left_pkt = build_spectator_left(user_id);
                self.send_to_user(host_id, &spec_left_pkt);
                let fellow_left_pkt = build_fellow_spectator_left(user_id);
                self.broadcast_to_spectators(host_id, &fellow_left_pkt);
            }

            // Spectator cleanup (if others were spectating this user)
            if let Some(specs) = self.spectators.remove(&user_id) {
                let spec_left_pkt = build_spectator_left(user_id);
                for spec_id in specs {
                    self.spectating_target.remove(&spec_id);
                    self.send_to_user(spec_id, &spec_left_pkt);
                }
            }

            // Remove user from all channels
            for ch in self.channels.values_mut() {
                ch.remove_member(user_id);
            }
            Some(session)
        } else {
            None
        }
    }

    pub fn remove_user_from_match(&mut self, user_id: i32) -> Option<(u16, Match, bool)> {
        let match_id = self.user_to_match.remove(&user_id)?;
        let m = self.matches.get_mut(&match_id)?;

        let mut vacated = false;
        for slot in m.slots.iter_mut() {
            if slot.user_id == user_id {
                *slot = MatchSlot::default();
                vacated = true;
                break;
            }
        }

        if !vacated {
            return None;
        }

        let remaining_players: Vec<i32> = m
            .slots
            .iter()
            .filter(|s| (s.status & SLOT_HAS_PLAYER) > 0 && s.user_id > 0)
            .map(|s| s.user_id)
            .collect();

        if remaining_players.is_empty() {
            let match_clone = self.matches.remove(&match_id).unwrap();
            self.match_start_times.remove(&match_id);
            self.match_last_scores.remove(&match_id);
            self.match_loaded_users.remove(&match_id);
            self.pending_disbanded_matches.push(match_id);
            Some((match_id, match_clone, true))
        } else {
            if m.host_id == user_id {
                m.host_id = remaining_players[0];
            }
            let match_clone = m.clone();
            Some((match_id, match_clone, false))
        }
    }

    pub fn take_disbanded_matches(&mut self) -> Vec<u16> {
        std::mem::take(&mut self.pending_disbanded_matches)
    }

    pub fn stop_spectating(&mut self, spectator_id: i32) -> Option<i32> {
        let host_id = self.spectating_target.remove(&spectator_id)?;
        if let Some(specs) = self.spectators.get_mut(&host_id) {
            specs.retain(|&x| x != spectator_id);
            if specs.is_empty() {
                self.spectators.remove(&host_id);
            }
        }
        Some(host_id)
    }

    pub fn broadcast_to_lobby(&mut self, packet: &[u8]) {
        for user_id in &self.lobby_subscribers {
            if let Some(token) = self.user_id_to_token.get(user_id) {
                if let Some(session) = self.sessions.get_mut(token) {
                    session.enqueue_packet(packet);
                }
            }
        }
    }

    pub fn broadcast_to_match(&mut self, match_id: u16, packet: &[u8], except_user_id: Option<i32>) {
        if let Some(m) = self.matches.get(&match_id) {
            for slot in &m.slots {
                let user_id = slot.user_id;
                if (slot.status & SLOT_HAS_PLAYER) > 0
                    && user_id > 0
                    && Some(user_id) != except_user_id
                {
                    if let Some(token) = self.user_id_to_token.get(&user_id) {
                        if let Some(session) = self.sessions.get_mut(token) {
                            session.enqueue_packet(packet);
                        }
                    }
                }
            }
        }
    }

    pub fn broadcast_to_spectators(&mut self, host_id: i32, packet: &[u8]) {
        if let Some(specs) = self.spectators.get(&host_id) {
            for spec_id in specs {
                if let Some(token) = self.user_id_to_token.get(spec_id) {
                    if let Some(session) = self.sessions.get_mut(token) {
                        session.enqueue_packet(packet);
                    }
                }
            }
        }
    }

    pub fn broadcast(&mut self, packet: &[u8]) {
        for session in self.sessions.values_mut() {
            session.enqueue_packet(packet);
        }
    }

    pub fn broadcast_except(&mut self, packet: &[u8], except_token: &str) {
        for (token, session) in self.sessions.iter_mut() {
            if token != except_token {
                session.enqueue_packet(packet);
            }
        }
    }

    pub fn broadcast_to_channel(&mut self, channel_name: &str, packet: &[u8]) {
        if let Some(ch) = self.channels.get(channel_name) {
            for member_id in &ch.members {
                if let Some(token) = self.user_id_to_token.get(member_id) {
                    if let Some(session) = self.sessions.get_mut(token) {
                        session.enqueue_packet(packet);
                    }
                }
            }
        }
    }

    pub fn broadcast_to_channel_except(&mut self, channel_name: &str, packet: &[u8], except_user_id: i32) {
        if let Some(ch) = self.channels.get(channel_name) {
            for member_id in &ch.members {
                if *member_id != except_user_id {
                    if let Some(token) = self.user_id_to_token.get(member_id) {
                        if let Some(session) = self.sessions.get_mut(token) {
                            session.enqueue_packet(packet);
                        }
                    }
                }
            }
        }
    }

    pub fn send_to_user(&mut self, user_id: i32, packet: &[u8]) -> bool {
        if let Some(token) = self.user_id_to_token.get(&user_id) {
            if let Some(session) = self.sessions.get_mut(token) {
                session.enqueue_packet(packet);
                return true;
            }
        }
        false
    }

    pub fn online_count(&self) -> usize {
        self.sessions.len()
    }
}
