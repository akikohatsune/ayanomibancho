#![allow(dead_code)]

// Bancho Client & Server Packet IDs
// how many time for this?

// Server -> Client Packets (CHO_*)
pub const CHO_USER_ID: u16 = 5;
pub const CHO_SEND_MESSAGE: u16 = 7;
pub const CHO_PONG: u16 = 8;
pub const CHO_HANDLE_USER_QUIT: u16 = 12;
pub const CHO_USER_STATS: u16 = 11;
pub const CHO_SPECTATE_FRAMES: u16 = 15;
pub const CHO_SPECTATOR_CANT_SPECTATE: u16 = 22;
pub const CHO_NOTIFICATION: u16 = 24;
pub const CHO_MATCH_UPDATE: u16 = 26;
pub const CHO_MATCH_NEW: u16 = 27;
pub const CHO_MATCH_DISBAND: u16 = 28;
pub const CHO_MATCH_JOIN_SUCCESS: u16 = 36;
pub const CHO_MATCH_JOIN_FAIL: u16 = 37;
pub const CHO_SPECTATOR_JOINED: u16 = 42;
pub const CHO_SPECTATOR_LEFT: u16 = 43;
pub const CHO_FELLOW_SPECTATOR_JOINED: u16 = 44;
pub const CHO_FELLOW_SPECTATOR_LEFT: u16 = 45;
pub const CHO_MATCH_START: u16 = 46;
pub const CHO_MATCH_SCORE_UPDATE: u16 = 48;
pub const CHO_MATCH_TRANSFER_HOST: u16 = 50;
pub const CHO_MATCH_ALL_PLAYERS_LOADED: u16 = 53;
pub const CHO_MATCH_PLAYER_FAILED: u16 = 57;
pub const CHO_MATCH_COMPLETE: u16 = 58;
pub const CHO_MATCH_SKIP: u16 = 61;
pub const CHO_CHANNEL_JOIN_SUCCESS: u16 = 64;
pub const CHO_CHANNEL_INFO: u16 = 65;
pub const CHO_BANCHO_PRIVILEGES: u16 = 71;
pub const CHO_PROTOCOL_VERSION: u16 = 75;
pub const CHO_MATCH_PLAYER_SKIPPED: u16 = 81;
pub const CHO_USER_PRESENCE: u16 = 83;
pub const CHO_RESTART: u16 = 86;
pub const CHO_MATCH_INVITE: u16 = 88;
pub const CHO_CHANNEL_INFO_END: u16 = 89;
pub const CHO_CHANNEL_PART: u16 = 90;

// Client -> Server Packets (OSU_*)
pub const OSU_CHANGE_ACTION: u16 = 0;
pub const OSU_SEND_PUBLIC_MESSAGE: u16 = 1;
pub const OSU_LOGOUT: u16 = 2;
pub const OSU_REQUEST_STATUS_UPDATE: u16 = 3;
pub const OSU_PONG: u16 = 4;
pub const OSU_START_SPECTATING: u16 = 16;
pub const OSU_STOP_SPECTATING: u16 = 17;
pub const OSU_SPECTATE_FRAMES: u16 = 18;
pub const OSU_CANT_SPECTATE: u16 = 21;
pub const OSU_SEND_PRIVATE_MESSAGE: u16 = 25;
pub const OSU_LOBBY_PART: u16 = 29;
pub const OSU_LOBBY_JOIN: u16 = 30;
pub const OSU_MATCH_CREATE: u16 = 31;
pub const OSU_MATCH_JOIN: u16 = 32;
pub const OSU_MATCH_PART: u16 = 33;
pub const OSU_MATCH_CHANGE_SLOT: u16 = 38;
pub const OSU_MATCH_READY: u16 = 39;
pub const OSU_MATCH_LOCK: u16 = 40;
pub const OSU_MATCH_CHANGE_SETTINGS: u16 = 41;
pub const OSU_MATCH_START: u16 = 44;
pub const OSU_MATCH_SCORE_UPDATE: u16 = 47;
pub const OSU_MATCH_LOAD_COMPLETE: u16 = 49;
pub const OSU_MATCH_CHANGE_HOST: u16 = 50;
pub const OSU_MATCH_CHANGE_MODS: u16 = 51;
pub const OSU_MATCH_CHANGE_TEAM: u16 = 52;
pub const OSU_MATCH_NO_BEATMAP: u16 = 54;
pub const OSU_MATCH_NOT_READY: u16 = 55;
pub const OSU_MATCH_FAILED: u16 = 56;
pub const OSU_MATCH_HAS_BEATMAP: u16 = 59;
pub const OSU_MATCH_SKIP_REQUEST: u16 = 60;
pub const OSU_MATCH_COMPLETE: u16 = 62;
pub const OSU_CHANNEL_JOIN: u16 = 63;
pub const OSU_CHANNEL_PART: u16 = 78;
pub const OSU_MATCH_INVITE: u16 = 70;
pub const OSU_RECEIVE_UPDATES: u16 = 79;
pub const OSU_USER_STATS_REQUEST: u16 = 85;

// Bancho Privileges bitmask
pub const PRIV_PLAYER: u32 = 1;
pub const PRIV_MODERATOR: u32 = 2;
pub const PRIV_SUPPORTER: u32 = 4;
pub const PRIV_OWNER: u32 = 16;

// osu! Action Status
pub const ACTION_IDLE: u8 = 0;
pub const ACTION_AFK: u8 = 1;
pub const ACTION_PLAYING: u8 = 2;
pub const ACTION_EDITING: u8 = 3;
pub const ACTION_MODDING: u8 = 4;
pub const ACTION_MULTIPLAYER: u8 = 5;
pub const ACTION_WATCHING: u8 = 6;
pub const ACTION_TESTING: u8 = 8;
pub const ACTION_SUBMITTING: u8 = 9;
pub const ACTION_PAUSED: u8 = 10;
pub const ACTION_LOBBY: u8 = 11;
pub const ACTION_DIRECT: u8 = 13;

// Multiplayer Slot Status
pub const SLOT_OPEN: u8 = 1;
pub const SLOT_LOCKED: u8 = 2;
pub const SLOT_NOT_READY: u8 = 4;
pub const SLOT_READY: u8 = 8;
pub const SLOT_NO_MAP: u8 = 16;
pub const SLOT_PLAYING: u8 = 32;
pub const SLOT_COMPLETE: u8 = 64;
pub const SLOT_QUIT: u8 = 128;
pub const SLOT_HAS_PLAYER: u8 = SLOT_NOT_READY | SLOT_READY | SLOT_NO_MAP | SLOT_PLAYING | SLOT_COMPLETE;

// Multiplayer Slot Teams
pub const TEAM_NEUTRAL: u8 = 0;
pub const TEAM_BLUE: u8 = 1;
pub const TEAM_RED: u8 = 2;

// Multiplayer Scoring Types
pub const SCORE_TYPE_SCORE: u8 = 0;
pub const SCORE_TYPE_ACCURACY: u8 = 1;
pub const SCORE_TYPE_COMBO: u8 = 2;
pub const SCORE_TYPE_SCORE_V2: u8 = 3;

// Multiplayer Team Types
pub const TEAM_TYPE_HEAD_TO_HEAD: u8 = 0;
pub const TEAM_TYPE_TAG_COOP: u8 = 1;
pub const TEAM_TYPE_TEAM_VS: u8 = 2;
pub const TEAM_TYPE_TAG_TEAM_VS: u8 = 3;
