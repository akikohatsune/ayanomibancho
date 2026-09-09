#![allow(dead_code)]

use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use tracing::{info, warn};

#[derive(Debug, Deserialize)]
pub struct CheesegullChild {
    #[serde(rename = "BeatmapID")]
    pub beatmap_id: Option<i32>,
    #[serde(rename = "ParentSetID")]
    pub parent_set_id: Option<i32>,
    #[serde(rename = "DiffName")]
    pub diff_name: Option<String>,
    #[serde(rename = "Mode")]
    pub mode: Option<i32>,
    #[serde(rename = "BPM")]
    pub bpm: Option<f64>,
    #[serde(rename = "AR")]
    pub ar: Option<f64>,
    #[serde(rename = "OD")]
    pub od: Option<f64>,
    #[serde(rename = "CS")]
    pub cs: Option<f64>,
    #[serde(rename = "HP")]
    pub hp: Option<f64>,
    #[serde(rename = "TotalLength")]
    pub total_length: Option<i32>,
    #[serde(rename = "DifficultyRating")]
    pub difficulty_rating: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct CheesegullSet {
    #[serde(rename = "SetID")]
    pub set_id: i32,
    #[serde(rename = "Artist")]
    pub artist: Option<String>,
    #[serde(rename = "Title")]
    pub title: Option<String>,
    #[serde(rename = "Creator")]
    pub creator: Option<String>,
    #[serde(rename = "RankedStatus")]
    pub ranked_status: Option<i32>,
    #[serde(rename = "LastUpdate")]
    pub last_update: Option<String>,
    #[serde(rename = "HasVideo")]
    pub has_video: Option<bool>,
    #[serde(rename = "ChildrenBeatmaps")]
    pub children_beatmaps: Option<Vec<CheesegullChild>>,
}

#[derive(Debug, Deserialize)]
pub struct CheesegullBeatmapSingle {
    #[serde(rename = "BeatmapID")]
    pub beatmap_id: Option<i32>,
    #[serde(rename = "ParentSetID")]
    pub parent_set_id: Option<i32>,
}

/// Helper to get the base API endpoint for Cheesegull-compatible mirrors
fn get_api_base(configured: &str) -> &str {
    let s = configured.trim_end_matches('/');
    if let Some(stripped) = s.strip_suffix("/search") {
        stripped
    } else {
        s
    }
}

/// Formats a Cheesegull Beatmap Set into osu!Direct legacy pipe-delimited search format
fn format_direct_set(set: &CheesegullSet) -> String {
    let set_id = set.set_id;
    let artist = set.artist.as_deref().unwrap_or("Unknown").replace('|', "-");
    let title = set.title.as_deref().unwrap_or("Unknown").replace('|', "-");
    let creator = set.creator.as_deref().unwrap_or("Unknown").replace('|', "-");
    let status = match set.ranked_status.unwrap_or(0) {
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        _ => 0,
    };
    let update = set
        .last_update
        .as_deref()
        .unwrap_or("2026-01-01 00:00:00")
        .replace('T', " ")
        .trim_end_matches('Z')
        .to_string();
    let has_video_int = if set.has_video.unwrap_or(false) { 1 } else { 0 };
    let no_video_size = if set.has_video.unwrap_or(false) { "7331" } else { "" };

    let mut diff_strs = Vec::new();
    if let Some(children) = &set.children_beatmaps {
        for child in children {
            let diff_name = child
                .diff_name
                .as_deref()
                .unwrap_or("Normal")
                .replace('@', "")
                .replace('|', "-")
                .replace(',', " ");
            let mode = child.mode.unwrap_or(0);
            diff_strs.push(format!("{diff_name}@{mode}"));
        }
    }
    let diffs = diff_strs.join(",");

    format!("{set_id}.osz|{artist}|{title}|{creator}|{status}|10.00|{update}|{set_id}|{set_id}|{has_video_int}|0|1337|{no_video_size}|{diffs}|")
}

/// Formats a Cheesegull Beatmap Set into osu!Direct single set info format (np)
fn format_direct_np(set: &CheesegullSet) -> String {
    let set_id = set.set_id;
    let artist = set.artist.as_deref().unwrap_or("Unknown").replace('|', "-");
    let title = set.title.as_deref().unwrap_or("Unknown").replace('|', "-");
    let creator = set.creator.as_deref().unwrap_or("Unknown").replace('|', "-");
    let status = match set.ranked_status.unwrap_or(0) {
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        _ => 0,
    };
    let update = set
        .last_update
        .as_deref()
        .unwrap_or("2026-01-01 00:00:00")
        .replace('T', " ")
        .trim_end_matches('Z')
        .to_string();
    let has_video_int = if set.has_video.unwrap_or(false) { 1 } else { 0 };
    let no_video_size = if set.has_video.unwrap_or(false) { "7331" } else { "" };

    format!("{set_id}.osz|{artist}|{title}|{creator}|{status}|10.00|{update}|{set_id}|{set_id}|{has_video_int}|0|1337|{no_video_size}")
}

/// Redirects `/d/:id` requests to the configured beatmap mirror (Catboy, Sayobot, etc.)
pub async fn download_beatmap(
    State(state): State<AppState>,
    Path(set_id): Path<String>,
) -> Response {
    let mirror_template = &state.config.mirrors.download_url;
    let target_url = mirror_template.replace("{}", &set_id);

    info!("Redirecting beatmap download for set {} -> {}", set_id, target_url);
    Redirect::temporary(&target_url).into_response()
}

/// Handles `/web/osu-search.php` for osu!Direct in-game beatmap searching
pub async fn search_beatmaps(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    // If request contains 'b' or 's', it's a single set lookup
    if params.contains_key("b") || params.contains_key("s") {
        return search_beatmap_set(State(state), Query(params)).await;
    }

    let search_endpoint = &state.config.mirrors.direct_search_api;
    let mut url = match reqwest::Url::parse(search_endpoint) {
        Ok(u) => u,
        Err(e) => {
            warn!("Failed to parse mirror search URL {}: {}", search_endpoint, e);
            reqwest::Url::parse("https://catboy.best/api/search").unwrap()
        }
    };

    let p: i32 = params.get("p").and_then(|v| v.parse().ok()).unwrap_or(0);
    let offset = (p.max(0) * 100).to_string();

    {
        let mut query_pairs = url.query_pairs_mut();
        query_pairs.append_pair("amount", "100");
        query_pairs.append_pair("offset", &offset);

        // Map query text
        if let Some(q) = params.get("q") {
            let trimmed = q.trim();
            let lower = trimmed.to_lowercase();
            if !lower.is_empty()
                && lower != "top rated"
                && lower != "most played"
                && lower != "newest"
                && lower != "newest maps"
            {
                query_pairs.append_pair("q", trimmed);
            }
        }

        // Map mode (-1: all, 0: osu, 1: taiko, 2: catch, 3: mania)
        if let Some(m_str) = params.get("m") {
            if let Ok(m) = m_str.parse::<i32>() {
                if (0..=3).contains(&m) {
                    query_pairs.append_pair("mode", &m.to_string());
                }
            }
        }

        // Map ranked status
        if let Some(r_str) = params.get("r") {
            if let Ok(r) = r_str.parse::<i32>() {
                match r {
                    0 | 7 => { query_pairs.append_pair("status", "1"); }
                    2 => { query_pairs.append_pair("status", "0"); }
                    3 => { query_pairs.append_pair("status", "3"); }
                    5 => { query_pairs.append_pair("status", "-2"); }
                    8 => { query_pairs.append_pair("status", "4"); }
                    4 => { /* All / Any - do not filter status */ }
                    _ => { query_pairs.append_pair("status", "1"); }
                }
            }
        } else {
            query_pairs.append_pair("status", "1");
        }
    }

    info!("Querying mirror search: {}", url);

    match state
        .http_client
        .get(url)
        .header(header::USER_AGENT, "osu! / AyanomiBancho")
        .timeout(Duration::from_secs(8))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<Vec<CheesegullSet>>().await {
                Ok(sets) => {
                    let mut out = String::new();
                    let count_header = if sets.len() >= 100 {
                        "101\r\n".to_string()
                    } else {
                        format!("{}\r\n", sets.len())
                    };
                    out.push_str(&count_header);
                    for set in &sets {
                        out.push_str(&format_direct_set(set));
                        out.push_str("\r\n");
                    }
                    (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
                        out,
                    )
                        .into_response()
                }
                Err(e) => {
                    warn!("Failed to deserialize Cheesegull JSON: {}", e);
                    (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
                        "0\r\n",
                    )
                        .into_response()
                }
            }
        }
        Ok(resp) => {
            warn!("External mirror search returned HTTP {}", resp.status());
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
                "0\r\n",
            )
                .into_response()
        }
        Err(e) => {
            warn!("External mirror search timed out or failed: {}", e);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
                "0\r\n",
            )
                .into_response()
        }
    }
}

/// Handles `/web/osu-search-set.php` for single beatmap / beatmap set lookup (chat /np, map links)
pub async fn search_beatmap_set(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let api_base = get_api_base(&state.config.mirrors.direct_search_api);

    let target_set: Option<CheesegullSet> = if let Some(b_id) = params.get("b") {
        let beatmap_url = format!("{}/b/{}", api_base, b_id.trim());
        match state
            .http_client
            .get(&beatmap_url)
            .header(header::USER_AGENT, "osu! / AyanomiBancho")
            .timeout(Duration::from_secs(6))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<CheesegullBeatmapSingle>().await {
                    Ok(bm) => {
                        if let Some(parent_id) = bm.parent_set_id {
                            let set_url = format!("{}/s/{}", api_base, parent_id);
                            fetch_set(&state.http_client, &set_url).await
                        } else {
                            None
                        }
                    }
                    Err(e) => {
                        warn!("Failed to deserialize single beatmap {}: {}", b_id, e);
                        None
                    }
                }
            }
            _ => None,
        }
    } else if let Some(s_id) = params.get("s") {
        let set_url = format!("{}/s/{}", api_base, s_id.trim());
        fetch_set(&state.http_client, &set_url).await
    } else {
        None
    };

    if let Some(set) = target_set {
        let formatted = format!("{}\r\n", format_direct_np(&set));
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
            formatted,
        )
            .into_response()
    } else {
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
            "",
        )
            .into_response()
    }
}

async fn fetch_set(client: &reqwest::Client, url: &str) -> Option<CheesegullSet> {
    match client
        .get(url)
        .header(header::USER_AGENT, "osu! / AyanomiBancho")
        .timeout(Duration::from_secs(6))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => resp.json::<CheesegullSet>().await.ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_catboy_reqwest() {
        let client = reqwest::Client::builder()
            .user_agent("osu! / AyanomiBancho")
            .build()
            .unwrap();
        let resp = client.get("https://catboy.best/api/search?amount=2").send().await.unwrap();
        assert!(resp.status().is_success());
        let sets: Vec<CheesegullSet> = resp.json().await.unwrap();
        assert_eq!(sets.len(), 2);
    }
}
