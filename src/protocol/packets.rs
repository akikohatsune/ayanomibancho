use super::constants::*;
use super::reader::PacketReader;
use super::writer::PacketWriter;
use std::io;

#[derive(Debug, Clone)]
pub struct UserPresence {
    pub user_id: i32,
    pub username: String,
    pub utc_offset: u8,
    pub country_code: u8,
    pub bancho_privileges: u8,
    pub game_mode: u8,
    pub longitude: f32,
    pub latitude: f32,
    pub rank: i32,
}

#[derive(Debug, Clone, Default)]
pub struct UserStats {
    pub user_id: i32,
    pub action: u8,
    pub info_text: String,
    pub map_md5: String,
    pub mods: u32,
    pub mode: u8,
    pub map_id: i32,
    pub ranked_score: i64,
    pub accuracy: f32,
    pub play_count: i32,
    pub total_score: i64,
    pub rank: i32,
    pub pp: i16,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub sender: String,
    pub content: String,
    pub target: String,
    pub sender_id: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchSlot {
    pub status: u8,
    pub team: u8,
    pub user_id: i32,
    pub mods: u32,
}

impl Default for MatchSlot {
    fn default() -> Self {
        Self {
            status: SLOT_OPEN,
            team: TEAM_NEUTRAL,
            user_id: -1,
            mods: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchScoreFrame {
    pub time: i32,
    pub slot_id: u8,
    pub total_300: u16,
    pub total_100: u16,
    pub total_50: u16,
    pub total_geki: u16,
    pub total_katu: u16,
    pub total_miss: u16,
    pub total_score: i32,
    pub max_combo: u16,
    pub current_combo: u16,
    pub perfect: bool,
    pub hp: u8,
    pub tag: u8,
    pub score_v2: bool,
    pub combo_portion: f64,
    pub bonus_portion: f64,
}

impl Default for MatchScoreFrame {
    fn default() -> Self {
        Self {
            time: 0,
            slot_id: 0,
            total_300: 0,
            total_100: 0,
            total_50: 0,
            total_geki: 0,
            total_katu: 0,
            total_miss: 0,
            total_score: 0,
            max_combo: 0,
            current_combo: 0,
            perfect: false,
            hp: 200,
            tag: 0,
            score_v2: false,
            combo_portion: 0.0,
            bonus_portion: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Match {
    pub id: u16,
    pub in_progress: bool,
    pub match_type: u8,
    pub active_mods: u32,
    pub name: String,
    pub password: Option<String>,
    pub beatmap_name: String,
    pub beatmap_id: i32,
    pub beatmap_md5: String,
    pub slots: [MatchSlot; 16],
    pub host_id: i32,
    pub play_mode: u8,
    pub scoring_type: u8,
    pub team_type: u8,
    pub freemod: bool,
    pub seed: u32,
}

impl Default for Match {
    fn default() -> Self {
        Self {
            id: 0,
            in_progress: false,
            match_type: 0,
            active_mods: 0,
            name: String::new(),
            password: None,
            beatmap_name: String::new(),
            beatmap_id: 0,
            beatmap_md5: String::new(),
            slots: [MatchSlot::default(); 16],
            host_id: 0,
            play_mode: 0,
            scoring_type: 0,
            team_type: 0,
            freemod: false,
            seed: 0,
        }
    }
}

pub fn build_user_id(id: i32) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_i32(id);
    PacketWriter::build_packet(CHO_USER_ID, &writer.into_bytes())
}

pub fn build_protocol_version(version: i32) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_i32(version);
    PacketWriter::build_packet(CHO_PROTOCOL_VERSION, &writer.into_bytes())
}

pub fn build_bancho_privileges(privs: u32) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_u32(privs);
    PacketWriter::build_packet(CHO_BANCHO_PRIVILEGES, &writer.into_bytes())
}

pub fn build_notification(msg: &str) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_osu_string(msg);
    PacketWriter::build_packet(CHO_NOTIFICATION, &writer.into_bytes())
}

pub fn build_pong() -> Vec<u8> {
    PacketWriter::build_packet(CHO_PONG, &[])
}

pub fn build_channel_info_end() -> Vec<u8> {
    PacketWriter::build_packet(CHO_CHANNEL_INFO_END, &[])
}

pub fn build_channel_info(name: &str, topic: &str, user_count: i16) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_osu_string(name);
    writer.write_osu_string(topic);
    writer.write_i16(user_count);
    PacketWriter::build_packet(CHO_CHANNEL_INFO, &writer.into_bytes())
}

pub fn build_channel_join_success(channel: &str) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_osu_string(channel);
    PacketWriter::build_packet(CHO_CHANNEL_JOIN_SUCCESS, &writer.into_bytes())
}

pub fn build_channel_part(channel: &str) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_osu_string(channel);
    PacketWriter::build_packet(CHO_CHANNEL_PART, &writer.into_bytes())
}

pub fn build_send_message(msg: &ChatMessage) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_osu_string(&msg.sender);
    writer.write_osu_string(&msg.content);
    writer.write_osu_string(&msg.target);
    writer.write_i32(msg.sender_id);
    PacketWriter::build_packet(CHO_SEND_MESSAGE, &writer.into_bytes())
}

pub fn build_user_presence(presence: &UserPresence) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_i32(presence.user_id);
    writer.write_osu_string(&presence.username);
    writer.write_u8(presence.utc_offset);
    writer.write_u8(presence.country_code);
    writer.write_u8(presence.bancho_privileges);
    writer.write_f32(presence.longitude);
    writer.write_f32(presence.latitude);
    writer.write_i32(presence.rank);
    PacketWriter::build_packet(CHO_USER_PRESENCE, &writer.into_bytes())
}

pub fn build_user_stats(stats: &UserStats) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_i32(stats.user_id);
    writer.write_u8(stats.action);
    writer.write_osu_string(&stats.info_text);
    writer.write_osu_string(&stats.map_md5);
    writer.write_u32(stats.mods);
    writer.write_u8(stats.mode);
    writer.write_i32(stats.map_id);
    writer.write_i64(stats.ranked_score);
    // accuracy as float 0.0 - 1.0 in osu stats
    writer.write_f32(stats.accuracy / 100.0);
    writer.write_i32(stats.play_count);
    writer.write_i64(stats.total_score);
    writer.write_i32(stats.rank);
    writer.write_i16(stats.pp);
    PacketWriter::build_packet(CHO_USER_STATS, &writer.into_bytes())
}

pub fn build_user_quit(user_id: i32, quit_state: u8) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_i32(user_id);
    writer.write_u8(quit_state);
    PacketWriter::build_packet(CHO_HANDLE_USER_QUIT, &writer.into_bytes())
}

pub fn build_restart(retry_ms: i32) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_i32(retry_ms);
    PacketWriter::build_packet(CHO_RESTART, &writer.into_bytes())
}

// Spectator Packets
pub fn build_spectator_joined(user_id: i32) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_i32(user_id);
    PacketWriter::build_packet(CHO_SPECTATOR_JOINED, &writer.into_bytes())
}

pub fn build_spectator_left(user_id: i32) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_i32(user_id);
    PacketWriter::build_packet(CHO_SPECTATOR_LEFT, &writer.into_bytes())
}

pub fn build_spectator_frames(raw_data: &[u8]) -> Vec<u8> {
    PacketWriter::build_packet(CHO_SPECTATE_FRAMES, raw_data)
}

pub fn build_fellow_spectator_joined(user_id: i32) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_i32(user_id);
    PacketWriter::build_packet(CHO_FELLOW_SPECTATOR_JOINED, &writer.into_bytes())
}

pub fn build_fellow_spectator_left(user_id: i32) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_i32(user_id);
    PacketWriter::build_packet(CHO_FELLOW_SPECTATOR_LEFT, &writer.into_bytes())
}

pub fn build_spectator_cant_spectate(user_id: i32) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_i32(user_id);
    PacketWriter::build_packet(CHO_SPECTATOR_CANT_SPECTATE, &writer.into_bytes())
}

// Multiplayer Packets
pub fn build_match_packet(packet_id: u16, m: &Match, send_password: bool) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_u16(m.id);
    writer.write_bool(m.in_progress);
    writer.write_u8(m.match_type);
    writer.write_u32(m.active_mods);
    writer.write_osu_string(&m.name);
    if send_password {
        writer.write_osu_string(m.password.as_deref().unwrap_or(""));
    } else {
        writer.write_osu_string(if m.password.is_some() { " " } else { "" });
    }
    writer.write_osu_string(&m.beatmap_name);
    writer.write_i32(m.beatmap_id);
    writer.write_osu_string(&m.beatmap_md5);

    // 16 slot statuses
    for slot in &m.slots {
        writer.write_u8(slot.status);
    }
    // 16 slot teams
    for slot in &m.slots {
        writer.write_u8(slot.team);
    }
    // user IDs for occupied slots
    for slot in &m.slots {
        if (slot.status & SLOT_HAS_PLAYER) > 0 {
            writer.write_i32(slot.user_id);
        }
    }
    writer.write_i32(m.host_id);
    writer.write_u8(m.play_mode);
    writer.write_u8(m.scoring_type);
    writer.write_u8(m.team_type);
    writer.write_bool(m.freemod);
    if m.freemod {
        for slot in &m.slots {
            writer.write_u32(slot.mods);
        }
    }
    writer.write_u32(m.seed);

    PacketWriter::build_packet(packet_id, &writer.into_bytes())
}

pub fn build_match_new(m: &Match) -> Vec<u8> {
    build_match_packet(CHO_MATCH_NEW, m, false)
}

pub fn build_match_update(m: &Match) -> Vec<u8> {
    build_match_packet(CHO_MATCH_UPDATE, m, false)
}

pub fn build_match_join_success(m: &Match) -> Vec<u8> {
    build_match_packet(CHO_MATCH_JOIN_SUCCESS, m, true)
}

pub fn build_match_join_fail() -> Vec<u8> {
    PacketWriter::build_packet(CHO_MATCH_JOIN_FAIL, &[])
}

pub fn build_match_disband(match_id: i32) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_i32(match_id);
    PacketWriter::build_packet(CHO_MATCH_DISBAND, &writer.into_bytes())
}

pub fn build_match_start(m: &Match) -> Vec<u8> {
    build_match_packet(CHO_MATCH_START, m, false)
}

pub fn build_match_score_update(frame: &MatchScoreFrame) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_i32(frame.time);
    writer.write_u8(frame.slot_id);
    writer.write_u16(frame.total_300);
    writer.write_u16(frame.total_100);
    writer.write_u16(frame.total_50);
    writer.write_u16(frame.total_geki);
    writer.write_u16(frame.total_katu);
    writer.write_u16(frame.total_miss);
    writer.write_i32(frame.total_score);
    writer.write_u16(frame.max_combo);
    writer.write_u16(frame.current_combo);
    writer.write_bool(frame.perfect);
    writer.write_u8(frame.hp);
    writer.write_u8(frame.tag);
    writer.write_bool(frame.score_v2);
    if frame.score_v2 {
        writer.write_f64(frame.combo_portion);
        writer.write_f64(frame.bonus_portion);
    }
    PacketWriter::build_packet(CHO_MATCH_SCORE_UPDATE, &writer.into_bytes())
}

pub fn build_match_transfer_host() -> Vec<u8> {
    PacketWriter::build_packet(CHO_MATCH_TRANSFER_HOST, &[])
}

pub fn build_match_all_players_loaded() -> Vec<u8> {
    PacketWriter::build_packet(CHO_MATCH_ALL_PLAYERS_LOADED, &[])
}

pub fn build_match_player_failed(slot_id: u32) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_u32(slot_id);
    PacketWriter::build_packet(CHO_MATCH_PLAYER_FAILED, &writer.into_bytes())
}

pub fn build_match_complete() -> Vec<u8> {
    PacketWriter::build_packet(CHO_MATCH_COMPLETE, &[])
}

pub fn build_match_player_skipped(slot_id: u32) -> Vec<u8> {
    let mut writer = PacketWriter::new();
    writer.write_u32(slot_id);
    PacketWriter::build_packet(CHO_MATCH_PLAYER_SKIPPED, &writer.into_bytes())
}

pub fn parse_match(reader: &mut PacketReader) -> io::Result<Match> {
    let id = reader.read_u16()?;
    let in_progress = reader.read_bool()?;
    let match_type = reader.read_u8()?;
    let active_mods = reader.read_u32()?;
    let name = reader.read_osu_string()?;
    let password_str = reader.read_osu_string()?;
    let password = if password_str.is_empty() { None } else { Some(password_str) };
    let beatmap_name = reader.read_osu_string()?;
    let beatmap_id = reader.read_i32()?;
    let beatmap_md5 = reader.read_osu_string()?;

    let mut slots = [MatchSlot::default(); 16];
    for i in 0..16 {
        slots[i].status = reader.read_u8()?;
    }
    for i in 0..16 {
        slots[i].team = reader.read_u8()?;
    }
    for i in 0..16 {
        if (slots[i].status & SLOT_HAS_PLAYER) > 0 {
            slots[i].user_id = reader.read_i32()?;
        } else {
            slots[i].user_id = -1;
        }
    }

    let host_id = reader.read_i32()?;
    let play_mode = reader.read_u8()?;
    let scoring_type = reader.read_u8()?;
    let team_type = reader.read_u8()?;
    let freemod = reader.read_bool()?;

    if freemod {
        for i in 0..16 {
            slots[i].mods = reader.read_u32()?;
        }
    }

    let seed = if reader.remaining() >= 4 {
        reader.read_u32()?
    } else {
        0
    };

    Ok(Match {
        id,
        in_progress,
        match_type,
        active_mods,
        name,
        password,
        beatmap_name,
        beatmap_id,
        beatmap_md5,
        slots,
        host_id,
        play_mode,
        scoring_type,
        team_type,
        freemod,
        seed,
    })
}

pub fn parse_score_frame(reader: &mut PacketReader) -> io::Result<MatchScoreFrame> {
    let time = reader.read_i32()?;
    let slot_id = reader.read_u8()?;
    let total_300 = reader.read_u16()?;
    let total_100 = reader.read_u16()?;
    let total_50 = reader.read_u16()?;
    let total_geki = reader.read_u16()?;
    let total_katu = reader.read_u16()?;
    let total_miss = reader.read_u16()?;
    let total_score = reader.read_i32()?;
    let max_combo = reader.read_u16()?;
    let current_combo = reader.read_u16()?;
    let perfect = reader.read_bool()?;
    let hp = reader.read_u8()?;
    let tag = reader.read_u8()?;
    let score_v2 = reader.read_bool()?;
    let (combo_portion, bonus_portion) = if score_v2 && reader.remaining() >= 16 {
        (reader.read_f64()?, reader.read_f64()?)
    } else {
        (0.0, 0.0)
    };

    Ok(MatchScoreFrame {
        time,
        slot_id,
        total_300,
        total_100,
        total_50,
        total_geki,
        total_katu,
        total_miss,
        total_score,
        max_combo,
        current_combo,
        perfect,
        hp,
        tag,
        score_v2,
        combo_portion,
        bonus_portion,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::reader::PacketReader;

    #[test]
    fn test_user_id_packet() {
        let pkt = build_user_id(1337);
        let mut reader = PacketReader::new(&pkt);
        let (id, len) = reader.read_packet_header().unwrap().unwrap();
        assert_eq!(id, CHO_USER_ID);
        assert_eq!(len, 4);
        assert_eq!(reader.read_i32().unwrap(), 1337);
    }

    #[test]
    fn test_match_roundtrip() {
        let mut m = Match::default();
        m.id = 1;
        m.name = "Ayanomi Room".to_string();
        m.beatmap_name = "Kira Kira Days".to_string();
        m.beatmap_id = 12345;
        m.beatmap_md5 = "abcdef1234567890".to_string();
        m.host_id = 1000;
        m.slots[0].status = SLOT_READY;
        m.slots[0].user_id = 1000;

        let pkt = build_match_join_success(&m);
        let mut reader = PacketReader::new(&pkt);
        let (pkt_id, _) = reader.read_packet_header().unwrap().unwrap();
        assert_eq!(pkt_id, CHO_MATCH_JOIN_SUCCESS);

        let parsed = parse_match(&mut reader).unwrap();
        assert_eq!(parsed.id, 1);
        assert_eq!(parsed.name, "Ayanomi Room");
        assert_eq!(parsed.host_id, 1000);
        assert_eq!(parsed.slots[0].user_id, 1000);
        assert_eq!(parsed.slots[0].status, SLOT_READY);
    }

    #[test]
    fn test_score_frame_roundtrip() {
        let frame = MatchScoreFrame {
            time: 15200,
            slot_id: 3,
            total_300: 250,
            total_100: 5,
            total_50: 1,
            total_geki: 50,
            total_katu: 10,
            total_miss: 0,
            total_score: 1540000,
            max_combo: 450,
            current_combo: 450,
            perfect: true,
            hp: 200,
            tag: 0,
            score_v2: false,
            combo_portion: 0.0,
            bonus_portion: 0.0,
        };

        let pkt = build_match_score_update(&frame);
        let mut reader = PacketReader::new(&pkt);
        let (pkt_id, _) = reader.read_packet_header().unwrap().unwrap();
        assert_eq!(pkt_id, CHO_MATCH_SCORE_UPDATE);

        let parsed = parse_score_frame(&mut reader).unwrap();
        assert_eq!(parsed.slot_id, 3);
        assert_eq!(parsed.total_score, 1540000);
        assert_eq!(parsed.max_combo, 450);
        assert_eq!(parsed.perfect, true);
    }
}
