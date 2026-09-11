#![allow(dead_code)]

use crate::db::scores::{get_personal_best, get_top_scores_for_map, save_score};
use crate::db::users::{create_user, get_user_by_email, get_user_by_id, get_user_by_username};
use crate::state::AppState;
use crate::utils::crypto::{hash_password, md5_hex};
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{error, info, warn};
use base64::Engine;
use simple_rijndael::impls::RijndaelCbc;
use simple_rijndael::paddings::{Pkcs7Padding, ZeroPadding};

#[derive(Debug, Deserialize)]
pub struct BanchoConnectQuery {
    pub v: Option<String>,
    pub u: Option<String>,
}

pub async fn web_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "web"
    }))
}

pub async fn bancho_connect() -> Response {
    (StatusCode::OK, "vn\n").into_response()
}

pub async fn check_updates() -> Response {
    (StatusCode::OK, "[]").into_response()
}

pub async fn lastfm() -> Response {
    (StatusCode::OK, "-3\n").into_response()
}

pub async fn get_friends() -> Response {
    (StatusCode::OK, "").into_response()
}

pub async fn osu_markasread() -> Response {
    (StatusCode::OK, "ok").into_response()
}

pub async fn osu_getbeatmapinfo() -> Response {
    (StatusCode::OK, "").into_response()
}

pub static FAVICON_ICO_BYTES: &[u8] = include_bytes!("favicon.ico");
pub static FAVICON_PNG_BYTES: &[u8] = include_bytes!("favicon.png");

pub async fn favicon() -> impl IntoResponse {
    (
        [
            ("content-type", "image/x-icon"),
            ("cache-control", "public, max-age=86400"),
        ],
        FAVICON_ICO_BYTES,
    )
}

pub async fn favicon_png() -> impl IntoResponse {
    (
        [
            ("content-type", "image/png"),
            ("cache-control", "public, max-age=86400"),
        ],
        FAVICON_PNG_BYTES,
    )
}

#[derive(Debug, Deserialize)]
pub struct GetScoresQuery {
    pub c: Option<String>, // beatmap md5
    pub f: Option<String>, // filename
    pub m: Option<u8>,     // mode (0-3)
    pub i: Option<i32>,    // beatmapset id
    pub mods: Option<u32>, // mods
    pub v: Option<i32>,    // view type (1=global, etc.)
    pub us: Option<String>, // username
    pub ha: Option<String>, // password hash
}

pub async fn get_scores(
    State(state): State<AppState>,
    Query(params): Query<GetScoresQuery>,
) -> Response {
    let map_md5 = match params.c {
        Some(ref hash) if !hash.is_empty() => hash.clone(),
        _ => return (StatusCode::OK, "-1\n").into_response(),
    };

    let mode = params.m.unwrap_or(0);
    let session_is_relax = if let Some(ref username) = params.us {
        let clean = crate::db::badges::clean_username(username);
        let st = state.bancho.read().await;
        st.sessions
            .values()
            .find(|s| s.username.eq_ignore_ascii_case(username) || crate::db::badges::clean_username(&s.username).eq_ignore_ascii_case(clean))
            .map(|s| s.is_relax)
            .unwrap_or(false)
    } else {
        false
    };
    let is_relax = (params.mods.unwrap_or(0) & 128) != 0 || session_is_relax;
    info!(
        "Fetching leaderboard for map MD5: {} (mode: {}, relax: {}, session_rx: {})",
        map_md5, mode, is_relax, session_is_relax
    );

    let scores = get_top_scores_for_map(&state.db, &map_md5, mode, is_relax, 50)
        .await
        .unwrap_or_default();

    let pb_score = if let Some(ref username) = params.us {
        let clean = crate::db::badges::clean_username(username);
        if let Ok(Some(user)) = get_user_by_username(&state.db, clean).await {
            get_personal_best(&state.db, &map_md5, user.id, mode, is_relax)
                .await
                .unwrap_or(None)
        } else {
            None
        }
    } else {
        None
    };

    let mut lines = Vec::new();
    let bid = 1;
    let bsid = params.i.unwrap_or(1);

    // Line 0: {status}|{osz_exists}|{bid}|{bsid}|{scores_len}|{rating}|{check_status}
    // status 2 = Ranked (so osu! client treats map as ranked, submits scores, and displays rankings)
    lines.push(format!("2|false|{}|{}|{}|0|", bid, bsid, scores.len()));
    // Line 1: offset
    lines.push("0".to_string());
    // Line 2: song title / artist
    let song_title = params.f.as_deref().unwrap_or("Beatmap");
    lines.push(song_title.to_string());
    if let Some(ref filename) = params.f {
        crate::db::beatmaps::save_raw_beatmap_name(&state.db, &map_md5, filename, bid as i64).await;
    }
    // Line 3: rating
    lines.push("10.0".to_string());

    // Line 4: personal best
    if let Some(pb) = pb_score {
        let pb_name = crate::db::badges::get_user_display_name(&state.badges_db, pb.user_id, &pb.username).await;
        let pb_rank = 1;
        lines.push(format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            pb.id,
            pb_name,
            pb.score,
            pb.max_combo,
            pb.c50,
            pb.c100,
            pb.c300,
            pb.c_miss,
            pb.c_katu,
            pb.c_geki,
            pb.perfect,
            pb.mods,
            pb.user_id,
            pb_rank,
            pb.submitted_at,
            0
        ));
    } else {
        lines.push("".to_string());
    }

    // Lines 5+: top scores
    for (idx, sc) in scores.iter().enumerate() {
        let sc_name = crate::db::badges::get_user_display_name(&state.badges_db, sc.user_id, &sc.username).await;
        lines.push(format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            sc.id,
            sc_name,
            sc.score,
            sc.max_combo,
            sc.c50,
            sc.c100,
            sc.c300,
            sc.c_miss,
            sc.c_katu,
            sc.c_geki,
            sc.perfect,
            sc.mods,
            sc.user_id,
            idx + 1,
            sc.submitted_at,
            0
        ));
    }

    let mut response_body = lines.join("\n");
    if scores.is_empty() {
        response_body.push_str("\n\n");
    }
    (StatusCode::OK, response_body).into_response()
}

fn url_decode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            result.push(' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                result.push(byte as char);
                i += 3;
            } else {
                result.push('%');
                i += 1;
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

fn parse_post_params(content_type: &str, body: &[u8]) -> HashMap<String, String> {
    parse_post_params_raw(content_type, body)
        .into_iter()
        .map(|(k, v)| (k, String::from_utf8_lossy(&v).into_owned()))
        .collect()
}

/// Parse multipart form data preserving raw bytes per field (important for binary 'score' field).
fn parse_post_params_raw(content_type: &str, body: &[u8]) -> HashMap<String, Vec<u8>> {
    let mut map = HashMap::new();
    if content_type.contains("multipart/form-data") {
        if let Some(idx) = content_type.find("boundary=") {
            let raw_boundary = &content_type[idx + 9..];
            let boundary = raw_boundary.split(';').next().unwrap_or("").trim_matches('"').trim();
            let delimiter = format!("--{}", boundary);
            let delim_bytes = delimiter.as_bytes();
            let mut start = 0usize;
            while let Some(pos) = find_bytes(&body[start..], delim_bytes) {
                let part_start = start + pos + delim_bytes.len();
                let part_start = if body.get(part_start..part_start + 2) == Some(b"\r\n") {
                    part_start + 2
                } else {
                    part_start
                };
                let next_pos = find_bytes(&body[part_start..], delim_bytes);
                let part_end = match next_pos {
                    Some(p) => part_start + p,
                    None => body.len(),
                };
                let part = &body[part_start..part_end];
                if let Some(sep) = find_bytes(part, b"\r\n\r\n") {
                    let headers = &part[..sep];
                    let mut content = &part[sep + 4..];
                    if content.ends_with(b"\r\n") {
                        content = &content[..content.len() - 2];
                    }
                    let headers_str = String::from_utf8_lossy(headers);
                    if let Some(name_idx) = headers_str.find("name=\"") {
                        let after = &headers_str[name_idx + 6..];
                        if let Some(end_quote) = after.find('"') {
                            let name = after[..end_quote].to_string();
                            let is_file = headers_str.contains("filename=");
                            if name == "score" {
                                if is_file || content.starts_with(b"\x5d\x00\x00\x20\x00") {
                                    info!("Multipart: detected replay file in 'score' field ({} bytes)", content.len());
                                    map.insert("score_replay".to_string(), content.to_vec());
                                } else {
                                    info!("Multipart: detected score data in 'score' field ({} bytes)", content.len());
                                    map.insert("score".to_string(), content.to_vec());
                                }
                            } else {
                                map.insert(name, content.to_vec());
                            }
                        }
                    }
                }
                start = part_end;
                if next_pos.is_none() { break; }
            }
        }
    } else {
        let body_str = String::from_utf8_lossy(body);
        for pair in body_str.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                let decoded_k = url_decode(k);
                let decoded_v = url_decode(v);
                map.insert(decoded_k, decoded_v.into_bytes());
            }
        }
    }
    map
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() { return None; }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn decode_b64_flexible(s: &str) -> Option<Vec<u8>> {
    let raw = s.trim();
    if raw.is_empty() {
        return None;
    }

    // 1. Try raw directly with Standard engine
    if let Ok(bytes) = base64::prelude::BASE64_STANDARD.decode(raw) {
        return Some(bytes);
    }

    // 2. Try raw with URL-safe engine
    if let Ok(bytes) = base64::prelude::BASE64_URL_SAFE.decode(raw) {
        return Some(bytes);
    }

    // 3. Try with padding added
    let mut padded = raw.to_string();
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    if let Ok(bytes) = base64::prelude::BASE64_STANDARD.decode(&padded) {
        return Some(bytes);
    }
    if let Ok(bytes) = base64::prelude::BASE64_URL_SAFE.decode(&padded) {
        return Some(bytes);
    }

    // 4. In case it was percent-encoded (%2B -> +, %2F -> /)
    if raw.contains('%') {
        let unquoted = raw.replace("%2B", "+").replace("%2b", "+")
                          .replace("%2F", "/").replace("%2f", "/")
                          .replace("%3D", "=").replace("%3d", "=");
        let mut unquoted_padded = unquoted.trim().to_string();
        while unquoted_padded.len() % 4 != 0 {
            unquoted_padded.push('=');
        }
        if let Ok(bytes) = base64::prelude::BASE64_STANDARD.decode(&unquoted_padded) {
            return Some(bytes);
        }
    }

    None
}

pub fn decrypt_osu_score(raw_score: &str, iv_b64: &str, osuver: &str) -> Option<String> {
    let trimmed = raw_score.trim();
    if trimmed.is_empty() {
        return None;
    }

    // If it's already plaintext (e.g. test requests)
    if trimmed.contains(':') && trimmed.split(':').count() >= 10 {
        return Some(trimmed.to_string());
    }

    let iv_bytes = match decode_b64_flexible(iv_b64) {
        Some(b) if b.len() == 32 => b,
        Some(b) => {
            warn!("decrypt_osu_score: IV decoded length is {}, expected 32", b.len());
            return None;
        }
        None => {
            warn!("decrypt_osu_score: failed to decode IV from base64 (raw len={})", iv_b64.len());
            return None;
        }
    };

    let cipher_bytes = match decode_b64_flexible(trimmed) {
        Some(b) if !b.is_empty() && b.len() % 32 == 0 => b,
        Some(b) => {
            warn!("decrypt_osu_score: ciphertext len {} is not a multiple of 32", b.len());
            return None;
        }
        None => {
            warn!("decrypt_osu_score: failed to decode ciphertext from base64 (raw len={})", trimmed.len());
            return None;
        }
    };

    let ver_str = osuver.trim();
    let digits: String = ver_str.chars().filter(|c| c.is_ascii_digit()).collect();

    let mut candidate_keys: Vec<Vec<u8>> = Vec::new();

    // 1. 8-digit date key: "osu!-scoreburgr---------{digits[..8]}"
    if digits.len() >= 8 {
        let k = format!("osu!-scoreburgr---------{}", &digits[..8]);
        if k.len() == 32 {
            candidate_keys.push(k.into_bytes());
        }
    }

    // 2. Direct format with osuver (e.g. if osuver is 8 chars, or truncate/pad to 32)
    if !ver_str.is_empty() {
        let mut k = format!("osu!-scoreburgr---------{}", ver_str);
        if k.len() > 32 {
            k.truncate(32);
        } else {
            while k.len() < 32 {
                k.push('-');
            }
        }
        let b = k.into_bytes();
        if !candidate_keys.contains(&b) {
            candidate_keys.push(b);
        }
    }

    // 3. Fallback key
    let fallback = b"h89f2-890h2h89b34g-h80g134n90133".to_vec();
    if !candidate_keys.contains(&fallback) {
        candidate_keys.push(fallback);
    }

    for key in &candidate_keys {
        let key_str = String::from_utf8_lossy(key);
        // Try ZeroPadding
        if let Ok(cipher) = RijndaelCbc::<ZeroPadding>::new(key, 32) {
            if let Ok(decrypted) = cipher.decrypt(&iv_bytes, cipher_bytes.clone()) {
                let s = String::from_utf8_lossy(&decrypted);
                let cleaned = s.trim_matches(|c: char| c.is_control() || c == '\0' || (c as u32) < 32);
                if cleaned.contains(':') && cleaned.split(':').count() >= 10 {
                    info!("Successfully decrypted score using key '{}' (ZeroPadding)", key_str);
                    return Some(cleaned.to_string());
                }
            }
        }

        // Try Pkcs7Padding
        if let Ok(cipher) = RijndaelCbc::<Pkcs7Padding>::new(key, 32) {
            if let Ok(decrypted) = cipher.decrypt(&iv_bytes, cipher_bytes.clone()) {
                let s = String::from_utf8_lossy(&decrypted);
                let cleaned = s.trim_matches(|c: char| c.is_control() || c == '\0' || (c as u32) < 32);
                if cleaned.contains(':') && cleaned.split(':').count() >= 10 {
                    info!("Successfully decrypted score using key '{}' (Pkcs7Padding)", key_str);
                    return Some(cleaned.to_string());
                }
            }
        }
    }

    warn!(
        "Failed to decrypt score! Tried {} keys. IV len={}, Cipher len={}, osuver={:?}",
        candidate_keys.len(),
        iv_bytes.len(),
        cipher_bytes.len(),
        osuver
    );
    None
}

/// Decrypt score using raw cipher bytes (not base64-encoded).
/// osu! sends the 'score' multipart field as raw binary (Rijndael-256 CBC encrypted),
/// and 'iv' as base64.
pub fn decrypt_osu_score_bytes(cipher_bytes_raw: &[u8], iv_b64: &str, osuver: &str) -> Option<String> {
    if cipher_bytes_raw.is_empty() {
        warn!("decrypt_osu_score_bytes: empty cipher bytes");
        return None;
    }

    let iv_bytes = match decode_b64_flexible(iv_b64) {
        Some(b) if b.len() == 32 => b,
        Some(b) => {
            warn!("decrypt_osu_score_bytes: IV decoded length is {}, expected 32", b.len());
            return None;
        }
        None => {
            warn!("decrypt_osu_score_bytes: failed to decode IV from base64 (raw len={})", iv_b64.len());
            return None;
        }
    };

    // osu! pads cipher to multiple of 32
    let cipher_bytes: Vec<u8> = if cipher_bytes_raw.len() % 32 == 0 {
        cipher_bytes_raw.to_vec()
    } else {
        // pad to next multiple of 32
        let padded_len = ((cipher_bytes_raw.len() / 32) + 1) * 32;
        let mut padded = cipher_bytes_raw.to_vec();
        padded.resize(padded_len, 0);
        warn!("decrypt_osu_score_bytes: ciphertext len {} not multiple of 32, padded to {}", cipher_bytes_raw.len(), padded_len);
        padded
    };

    let ver_str = osuver.trim();
    let digits: String = ver_str.chars().filter(|c| c.is_ascii_digit()).collect();

    let mut candidate_keys: Vec<Vec<u8>> = Vec::new();

    // 1. Primary key: "osu!-scoreburgr---------{8_digit_date}"
    if digits.len() >= 8 {
        let k = format!("osu!-scoreburgr---------{}", &digits[..8]);
        if k.len() == 32 {
            candidate_keys.push(k.into_bytes());
        }
    }

    // 2. With full osuver string truncated/padded to 32
    if !ver_str.is_empty() {
        let mut k = format!("osu!-scoreburgr---------{}", ver_str);
        if k.len() > 32 { k.truncate(32); } else { while k.len() < 32 { k.push('-'); } }
        let b = k.into_bytes();
        if !candidate_keys.contains(&b) { candidate_keys.push(b); }
    }

    // 3. Fallback
    let fallback = b"h89f2-890h2h89b34g-h80g134n90133".to_vec();
    if !candidate_keys.contains(&fallback) { candidate_keys.push(fallback); }

    for key in &candidate_keys {
        let key_str = String::from_utf8_lossy(key);
        for padding_label in &["ZeroPadding", "Pkcs7Padding"] {
            let result = if *padding_label == "ZeroPadding" {
                RijndaelCbc::<ZeroPadding>::new(key, 32)
                    .map_err(|e| format!("new() err: {:?}", e))
                    .and_then(|c| c.decrypt(&iv_bytes, cipher_bytes.clone()).map_err(|e| format!("decrypt() err: {:?}", e)))
            } else {
                RijndaelCbc::<Pkcs7Padding>::new(key, 32)
                    .map_err(|e| format!("new() err: {:?}", e))
                    .and_then(|c| c.decrypt(&iv_bytes, cipher_bytes.clone()).map_err(|e| format!("decrypt() err: {:?}", e)))
            };
            match result {
                Ok(decrypted) => {
                    let s = String::from_utf8_lossy(&decrypted);
                    let cleaned: String = s.trim_end_matches(|c: char| c == '\0' || c.is_control()).to_string();
                    let colon_count = cleaned.chars().filter(|&c| c == ':').count();
                    // Log first 80 printable chars of decrypted to verify
                    let preview: String = cleaned.chars().take(80).collect();
                    info!("key='{}' ({}): decrypted {} bytes, {} colons, preview='{}'",
                        key_str, padding_label, decrypted.len(), colon_count, preview);
                    if colon_count >= 10 {
                        info!("decrypt_osu_score_bytes: OK with key='{}' ({})", key_str, padding_label);
                        return Some(cleaned);
                    }
                }
                Err(e) => {
                    warn!("key='{}' ({}): {}", key_str, padding_label, e);
                }
            }
        }
    }

    warn!(
        "decrypt_osu_score_bytes: All keys failed. IV len={}, Cipher len={}, osuver={:?}",
        iv_bytes.len(), cipher_bytes.len(), osuver
    );
    None
}

pub async fn submit_score(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Use raw parser so binary 'score' field is not corrupted by UTF-8 conversion
    let form_raw = parse_post_params_raw(content_type, &body);
    let form: HashMap<String, String> = form_raw.iter()
        .map(|(k, v)| (k.clone(), String::from_utf8_lossy(v).into_owned()))
        .collect();

    info!(
        "Score submission received with {} keys: {:?}",
        form_raw.len(),
        form_raw.keys().collect::<Vec<_>>()
    );

    // In osu! score submission:
    //   'score'        = base64-encoded encrypted score data (~150-250 bytes)
    //   'score_replay' = binary replay data (LZMA stream, ~30KB)
    //   's'            = base64-encoded encrypted client hashes
    //   'iv'           = base64-encoded IV (44 bytes = 32 bytes decoded)
    //   'osuver'       = client version string (e.g. "b20260711.1" or "20260711")
    let score_b64 = form.get("score")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| form.get("s").map(|s| s.trim().to_string()))
        .unwrap_or_default();

    let iv_b64 = form.get("iv").map(|s| s.as_str()).unwrap_or("");
    let osuver = form.get("osuver").map(|s| s.as_str()).unwrap_or("");
    let x_param = form.get("x").map(|s| s.as_str()).unwrap_or("");
    let ft_param = form.get("ft").map(|s| s.as_str()).unwrap_or("0");

    info!(
        "Score submission params: iv_b64='{}', osuver='{}', x='{}', ft='{}', score_b64_len={}",
        iv_b64,
        osuver,
        x_param,
        ft_param,
        score_b64.len()
    );

    let score_str = if score_b64.contains(':') && score_b64.split(':').count() >= 10 {
        info!("Score data is already plaintext");
        score_b64.clone()
    } else {
        let raw_score_bytes: Vec<u8> = match decode_b64_flexible(&score_b64) {
            Some(b) => b,
            None => {
                warn!("Could not base64-decode 'score' field (len={}): '{}'", score_b64.len(), &score_b64[..score_b64.len().min(80)]);
                vec![]
            }
        };

        if raw_score_bytes.is_empty() {
            warn!("Score ciphertext ('score' field) is empty or failed to decode, skipping");
            return (StatusCode::OK, "beatmapId:0|beatmapSetId:0|beatmapPlaycount:1|beatmapPasscount:1\n").into_response();
        }

        match decrypt_osu_score_bytes(&raw_score_bytes, iv_b64, osuver) {
            Some(s) => s,
            None => {
                warn!("Could not decrypt score (cipher len={})", raw_score_bytes.len());
                return (StatusCode::OK, "error").into_response();
            }
        }
    };

    let parts: Vec<&str> = score_str.split(':').collect();
    info!("Score parts count: {}", parts.len());

    if parts.len() >= 16 {
        let map_md5 = parts[0].trim();
        let username_clean = parts[1].trim_matches(|c: char| c.is_whitespace() || c.is_control() || c == '\0');
        let username = username_clean.to_string();
        let c300: i32 = parts[3].trim().parse().unwrap_or(0);
        let c100: i32 = parts[4].trim().parse().unwrap_or(0);
        let c50: i32 = parts[5].trim().parse().unwrap_or(0);
        let c_geki: i32 = parts[6].trim().parse().unwrap_or(0);
        let c_katu: i32 = parts[7].trim().parse().unwrap_or(0);
        let c_miss: i32 = parts[8].trim().parse().unwrap_or(0);
        let score: i64 = parts[9].trim().parse().unwrap_or(0);
        let max_combo: i32 = parts[10].trim().parse().unwrap_or(0);
        let perfect: i32 = if parts[11].trim().eq_ignore_ascii_case("true") || parts[11].trim() == "1" { 1 } else { 0 };
        let mut mods: u32 = parts[13].trim().parse().unwrap_or(0);
        let passed = parts[14].trim().eq_ignore_ascii_case("true") || parts[14].trim() == "1";
        let mode: u8 = parts[15].trim().parse().unwrap_or(0);

        info!(
            "Parsed score: user='{}', map='{}', score={}, combo={}, passed={}, mode={}, mods={}",
            username, map_md5, score, max_combo, passed, mode, mods
        );

        let clean_user = crate::db::badges::clean_username(&username);

        // Check if user session has Relax enabled
        let session_is_relax = {
            let st = state.bancho.read().await;
            st.sessions
                .values()
                .find(|s| s.username.eq_ignore_ascii_case(&username) || crate::db::badges::clean_username(&s.username).eq_ignore_ascii_case(clean_user))
                .map(|s| s.is_relax)
                .unwrap_or(false)
        };

        if session_is_relax {
            mods |= 128; // Ensure Relax bit is set
        }

        let is_relax = (mods & 128) != 0;
        let is_passed = passed && x_param != "1" && (ft_param == "0" || ft_param.is_empty());

        // Robust user lookup: DB query -> Bancho session fallback
        let user = match get_user_by_username(&state.db, clean_user).await {
            Ok(Some(u)) => {
                info!("Found user '{}' (ID: {}) in database by username", u.username, u.id);
                Some(u)
            }
            Ok(None) => {
                warn!("User '{}' (clean: '{}') not found in database by username. Searching active Bancho sessions...", username, clean_user);
                let session_user_id = {
                    let st = state.bancho.read().await;
                    st.sessions
                        .values()
                        .find(|s| s.username.eq_ignore_ascii_case(&username) || crate::db::badges::clean_username(&s.username).eq_ignore_ascii_case(clean_user))
                        .map(|s| s.user_id)
                };
                if let Some(uid) = session_user_id {
                    match get_user_by_id(&state.db, uid).await {
                        Ok(Some(u)) => {
                            info!("Resolved user '{}' (ID: {}) from active Bancho session", u.username, u.id);
                            Some(u)
                        }
                        _ => None,
                    }
                } else {
                    let single_uid = {
                        let st = state.bancho.read().await;
                        if st.sessions.len() == 1 {
                            st.sessions.values().next().map(|s| s.user_id)
                        } else {
                            None
                        }
                    };
                    if let Some(uid) = single_uid {
                        warn!("Using single active session user ID {}", uid);
                        get_user_by_id(&state.db, uid).await.ok().flatten()
                    } else {
                        None
                    }
                }
            }
            Err(e) => {
                error!("Database error querying user '{}': {}", username, e);
                None
            }
        };

        if let Some(user) = user {
            info!(
                "Recording score for {}: Map: {}, Score: {}, Combo: {}, Relax: {}, Passed: {}",
                user.username, map_md5, score, max_combo, is_relax, is_passed
            );

            if is_passed {
                match save_score(
                    &state.db, map_md5, user.id, score, max_combo, c300, c100, c50, c_geki, c_katu,
                    c_miss, perfect, mods, mode,
                ).await {
                    Ok(res) => {
                        let score_id = res.score_id;
                        info!(
                            "Score #{} successfully saved for user '{}' (Acc: {:.2}%, Score PP: {:.1}, Profile PP: {} -> {})!",
                            score_id, user.username, res.score_acc, res.score_pp, res.total_pp_before, res.total_pp_after
                        );

                        // Non-blocking notification to Bancho service to broadcast CHO_USER_STATS
                        let client = state.http_client.clone();
                        let bancho_port = state.config.server.bancho_port;
                        let user_id = user.id;

                        tokio::spawn(async move {
                            let url = format!("http://127.0.0.1:{}/internal/stats_update", bancho_port);
                            let _ = client
                                .post(&url)
                                .json(&serde_json::json!({
                                    "user_id": user_id,
                                    "mode": mode,
                                    "is_relax": is_relax
                                }))
                                .send()
                                .await;
                        });

                        let score_pp_int = res.score_pp.round() as i32;

                        // Return full submission charts response required by osu! client to animate ranking screen
                        let charts = format!(
                            "beatmapId:1|beatmapSetId:1|beatmapPlaycount:1|beatmapPasscount:1|approvedDate:0\n\
                            chartId:beatmap|chartUrl:http://127.0.0.1:5000/b/1|chartName:Beatmap Ranking|rankBefore:1|rankAfter:1|maxComboBefore:0|maxComboAfter:{}|accuracyBefore:0|accuracyAfter:{:.2}|rankedScoreBefore:0|rankedScoreAfter:{}|ppBefore:{}|ppAfter:{}|onlineScoreId:{}\n\
                            chartId:overall|chartUrl:http://127.0.0.1:5000/u/{}|chartName:Overall Ranking|rankBefore:1|rankAfter:1|rankedScoreBefore:0|rankedScoreAfter:{}|totalScoreBefore:0|totalScoreAfter:{}|maxComboBefore:0|maxComboAfter:{}|accuracyBefore:{:.2}|accuracyAfter:{:.2}|ppBefore:{}|ppAfter:{}|achievements-new:|onlineScoreId:{}\n",
                            max_combo, res.score_acc, score, score_pp_int, score_pp_int, score_id,
                            user.id, score, score, max_combo, res.acc_before, res.acc_after, res.total_pp_before, res.total_pp_after, score_id
                        );
                        return (StatusCode::OK, charts).into_response();
                    }
                    Err(e) => {
                        error!("Failed to save score to database for user '{}': {}", user.username, e);
                    }
                }
            } else {
                info!("Play was quit or failed (passed={}, x={}, ft={}), not recording to leaderboard.", passed, x_param, ft_param);
            }
        } else {
            warn!("Cannot record score: player could not be resolved from username '{}'", username);
        }
    } else {
        warn!(
            "Score submission data did not match expected format (len={}): score_str='{}', raw_len={}",
            parts.len(),
            score_str,
            score_b64.len()
        );
    }

    let response_text = "beatmapId:0|beatmapSetId:0|beatmapPlaycount:1|beatmapPasscount:1\n";
    (StatusCode::OK, response_text).into_response()
}

pub async fn get_matches_api(
    State(state): State<AppState>,
) -> Response {
    match crate::db::matches::get_recent_matches(&state.db, 50).await {
        Ok(matches) => Json(matches).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn get_match_detail_api(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Response {
    match crate::db::matches::get_match_detail(&state.db, id).await {
        Ok(Some(detail)) => Json(detail).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Match not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct ChatHistoryQuery {
    pub target: Option<String>,
    pub limit: Option<i64>,
}

pub async fn get_chat_history_api(
    State(state): State<AppState>,
    Query(params): Query<ChatHistoryQuery>,
) -> Response {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let result = if let Some(ref target) = params.target {
        crate::db::chat::get_channel_history(&state.chat_db, target, limit).await
    } else {
        crate::db::chat::get_recent_chats(&state.chat_db, limit).await
    };

    match result {
        Ok(messages) => Json(messages).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn list_badges_api(
    State(state): State<AppState>,
) -> Response {
    match crate::db::badges::list_all_badges(&state.badges_db).await {
        Ok(badges) => Json(badges).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn get_user_badges_api(
    State(state): State<AppState>,
    axum::extract::Path(user_id): axum::extract::Path<i32>,
) -> Response {
    match crate::db::badges::get_user_badges(&state.badges_db, user_id).await {
        Ok(badges) => Json(badges).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateBadgePayload {
    pub name: String,
    pub description: String,
    pub icon: String,
    pub tag: Option<String>,
}

pub async fn create_badge_api(
    State(state): State<AppState>,
    Json(payload): Json<CreateBadgePayload>,
) -> Response {
    if payload.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Badge name cannot be empty" })),
        )
            .into_response();
    }
    let tag = payload.tag.as_deref().unwrap_or("").trim();
    match crate::db::badges::create_badge(&state.badges_db, &payload.name, &payload.description, &payload.icon, tag).await {
        Ok(id) => Json(serde_json::json!({ "id": id, "name": payload.name, "description": payload.description, "icon_url": payload.icon, "tag": tag })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct AwardBadgePayload {
    pub user_id: i32,
    pub badge_id: i64,
}

pub async fn award_badge_api(
    State(state): State<AppState>,
    Json(payload): Json<AwardBadgePayload>,
) -> Response {
    match crate::db::badges::award_badge(&state.badges_db, payload.user_id, payload.badge_id).await {
        Ok(awarded) => Json(serde_json::json!({ "success": true, "awarded": awarded })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct RevokeBadgePayload {
    pub user_id: i32,
    pub badge_id: i64,
}

pub async fn revoke_badge_api(
    State(state): State<AppState>,
    Json(payload): Json<RevokeBadgePayload>,
) -> Response {
    match crate::db::badges::revoke_badge(&state.badges_db, payload.user_id, payload.badge_id).await {
        Ok(revoked) => Json(serde_json::json!({ "success": true, "revoked": revoked })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

fn parse_form_pairs(input: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();

    // Check for multipart/form-data
    if input.contains("Content-Disposition:") || input.contains("form-data;") {
        for part in input.split("Content-Disposition:") {
            if let Some(name_start) = part.find("name=\"") {
                let rest = &part[name_start + 6..];
                if let Some(name_end) = rest.find('"') {
                    let field_name = &rest[..name_end];
                    if let Some(val_start) = rest.find("\r\n\r\n") {
                        let val_part = &rest[val_start + 4..];
                        let val = val_part.split("\r\n").next().unwrap_or("").trim();
                        pairs.push((field_name.to_string(), val.to_string()));
                    } else if let Some(val_start) = rest.find("\n\n") {
                        let val_part = &rest[val_start + 2..];
                        let val = val_part.split('\n').next().unwrap_or("").trim();
                        pairs.push((field_name.to_string(), val.to_string()));
                    }
                }
            }
        }
        if !pairs.is_empty() {
            return pairs;
        }
    }

    // Standard x-www-form-urlencoded
    for part in input.split('&') {
        if part.is_empty() {
            continue;
        }
        let mut split = part.splitn(2, '=');
        let key = split.next().unwrap_or("");
        let val = split.next().unwrap_or("");
        pairs.push((url_decode(key), url_decode(val)));
    }
    pairs
}

/// Endpoint: POST /users and POST /users/
/// Handles osu! client in-game account registration and live username/email validation.
pub async fn osu_register_user(
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let body_str = String::from_utf8_lossy(&body);
    let pairs = parse_form_pairs(&body_str);

    let mut check: Option<String> = None;
    let mut username: Option<String> = None;
    let mut email: Option<String> = None;
    let mut password: Option<String> = None;

    for (k, v) in pairs {
        match k.as_str() {
            "check" => check = Some(v),
            "user[username]" | "username" => username = Some(v),
            "user[user_email]" | "user[email]" | "email" => email = Some(v),
            "user[password]" | "password" => password = Some(v),
            _ => {}
        }
    }

    let u_name = username.unwrap_or_default().trim().to_string();
    let u_email = email.unwrap_or_default().trim().to_string();
    let u_pass = password.unwrap_or_default().trim().to_string();

    info!(
        "osu_register_user: name='{}', email='{}', pass_len={}, check={:?}",
        u_name, u_email, u_pass.len(), check
    );

    // If check=1/2, or if email/password is empty, client is performing live validation
    let is_live_check = check.as_deref() == Some("1")
        || check.as_deref() == Some("2")
        || u_pass.is_empty()
        || u_email.is_empty();

    let mut errors: HashMap<String, Vec<String>> = HashMap::new();

    let mut add_err = |field1: &str, field2: &str, msg: &str| {
        errors.entry(field1.to_string()).or_default().push(msg.to_string());
        errors.entry(field2.to_string()).or_default().push(msg.to_string());
    };

    // 1. Username validation
    if u_name.is_empty() {
        if !is_live_check {
            add_err("username", "user[username]", "Please enter a username.");
        }
    } else if u_name.len() < 3 {
        add_err("username", "user[username]", "Username must be at least 3 characters long.");
    } else if u_name.len() > 20 {
        add_err("username", "user[username]", "Username must not exceed 20 characters.");
    } else if u_name.contains(' ') && u_name.contains('_') {
        add_err("username", "user[username]", "Username cannot contain both spaces and underscores.");
    } else if !u_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '[' || c == ']' || c == '-' || c == ' ') {
        add_err("username", "user[username]", "Username contains invalid characters.");
    } else {
        match get_user_by_username(&state.db, &u_name).await {
            Ok(Some(_)) => {
                add_err("username", "user[username]", "Username is already taken.");
            }
            Err(e) => {
                warn!("Database error checking username '{}': {}", u_name, e);
            }
            _ => {}
        }
    }

    // 2. Email validation
    if !u_email.is_empty() {
        if !u_email.contains('@') || !u_email.contains('.') || u_email.len() < 5 {
            add_err("user_email", "user[user_email]", "Please enter a valid email address.");
        } else {
            match get_user_by_email(&state.db, &u_email).await {
                Ok(Some(_)) => {
                    add_err("user_email", "user[user_email]", "Email address is already in use.");
                }
                Err(e) => {
                    warn!("Database error checking email '{}': {}", u_email, e);
                }
                _ => {}
            }
        }
    } else if !is_live_check {
        add_err("user_email", "user[user_email]", "Please enter an email address.");
    }

    // 3. Password validation
    if !u_pass.is_empty() {
        if u_pass.len() < 4 {
            add_err("password", "user[password]", "Password must be at least 4 characters long.");
        }
    } else if !is_live_check {
        add_err("password", "user[password]", "Please enter a password.");
    }

    // Live validation response
    if is_live_check {
        let json_body = serde_json::json!({
            "errors": errors
        });
        return (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            json_body.to_string(),
        ).into_response();
    }

    // Final registration submit validation
    if !errors.is_empty() {
        let json_body = serde_json::json!({
            "errors": errors
        });
        return (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            json_body.to_string(),
        ).into_response();
    }

    // Hash password and save user
    let md5_pass = md5_hex(&u_pass);
    let pwd_hash = match hash_password(&md5_pass) {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Password hashing error: {}", e),
            ).into_response();
        }
    };

    let user_email = if u_email.is_empty() {
        format!("{}@ayanomi.local", u_name)
    } else {
        u_email
    };

    match create_user(
        &state.db,
        &u_name,
        &pwd_hash,
        &user_email,
        state.config.gameplay.default_country,
    ).await {
        Ok(user) => {
            info!("osu! in-game registration successful for user '{}' (ID: {})", user.username, user.id);
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "text/plain")],
                "ok",
            ).into_response()
        }
        Err(e) => {
            warn!("Failed to create user during in-game registration: {}", e);
            let mut errs = HashMap::new();
            errs.insert("username".to_string(), vec![format!("Registration failed: {}", e)]);
            let json_body = serde_json::json!({
                "errors": errs
            });
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                json_body.to_string(),
            ).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decrypt_osu_score() {
        let plaintext = "c51aee56bb5195244252d190baa54b49:PurePeace:815772625adad639735a044e8aba9360:44:18:6:4:6:20:38645:26:False:F:64:False:0:210615064431:20210520";
        let key = b"osu!-scoreburgr---------20210520";
        let iv = [
            240, 124, 26, 154, 27, 186, 98, 170, 95, 190, 213, 103, 13, 128, 39, 59,
            217, 84, 68, 144, 173, 93, 117, 132, 33, 213, 96, 154, 228, 231, 197, 162,
        ];
        let cipher = RijndaelCbc::<ZeroPadding>::new(key, 32).unwrap();
        let encrypted = cipher.encrypt(&iv, plaintext.as_bytes().to_vec()).unwrap();
        let enc_b64 = base64::prelude::BASE64_STANDARD.encode(&encrypted);
        let iv_b64 = base64::prelude::BASE64_STANDARD.encode(&iv);

        let decrypted = decrypt_osu_score(&enc_b64, &iv_b64, "b20210520.1");
        assert!(decrypted.is_some());
        let res = decrypted.unwrap();
        assert!(res.starts_with("c51aee56bb5195244252d190baa54b49:PurePeace"));
    }
}
