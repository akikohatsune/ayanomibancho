use crate::db::users::DbUserStats;
use crate::protocol::packets::{UserPresence, UserStats};
use std::collections::HashSet;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct Session {
    pub token: String,
    pub user_id: i32,
    pub username: String,
    pub utc_offset: u8,
    pub country_code: u8,
    pub privileges: u32,
    pub action: u8,
    pub info_text: String,
    pub map_md5: String,
    pub mods: u32,
    pub mode: u8,
    pub map_id: i32,
    pub last_ping: Instant,
    pub packet_queue: Vec<u8>,
    pub channels: HashSet<String>,
    pub is_relax: bool,
}

impl Session {
    pub fn new(
        token: String,
        user_id: i32,
        username: String,
        utc_offset: u8,
        country_code: u8,
        privileges: u32,
    ) -> Self {
        Self {
            token,
            user_id,
            username,
            utc_offset,
            country_code,
            privileges,
            action: 0,
            info_text: String::new(),
            map_md5: String::new(),
            mods: 0,
            mode: 0,
            map_id: 0,
            last_ping: Instant::now(),
            packet_queue: Vec::new(),
            channels: HashSet::new(),
            is_relax: false,
        }
    }

    pub fn enqueue_packet(&mut self, packet: &[u8]) {
        self.packet_queue.extend_from_slice(packet);
    }

    pub fn dequeue_all_packets(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.packet_queue)
    }

    pub fn to_presence(&self, rank: i32) -> UserPresence {
        UserPresence {
            user_id: self.user_id,
            username: self.username.clone(),
            utc_offset: self.utc_offset,
            country_code: self.country_code,
            bancho_privileges: self.privileges as u8,
            game_mode: self.mode,
            longitude: 0.0,
            latitude: 0.0,
            rank,
        }
    }

    pub fn to_stats(&self, db_stats: &DbUserStats, rank: i32) -> UserStats {
        UserStats {
            user_id: self.user_id,
            action: self.action,
            info_text: self.info_text.clone(),
            map_md5: self.map_md5.clone(),
            mods: self.mods,
            mode: self.mode,
            map_id: self.map_id,
            ranked_score: db_stats.ranked_score,
            accuracy: db_stats.accuracy,
            play_count: db_stats.play_count,
            total_score: db_stats.total_score,
            rank,
            pp: db_stats.pp,
        }
    }
}
