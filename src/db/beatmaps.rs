use crate::db::DbPool;
use serde::Deserialize;
use sqlx::Row;
use std::time::Duration;
use tracing::{info, warn};

#[derive(Debug, Clone, Default)]
pub struct BeatmapMeta {
    pub map_md5: String,
    pub beatmap_id: i64,
    pub beatmapset_id: i64,
    pub artist: String,
    pub title: String,
    pub version: String,
    pub creator: String,
}

impl BeatmapMeta {
    /// Formatted display name, e.g. "nekodex - new beginnings [tutorial]"
    pub fn display_name(&self) -> String {
        if !self.title.is_empty() {
            let artist = if self.artist.is_empty() { "Unknown" } else { &self.artist };
            if !self.version.is_empty() {
                format!("{} - {} [{}]", artist, self.title, self.version)
            } else {
                format!("{} - {}", artist, self.title)
            }
        } else {
            let short = if self.map_md5.len() >= 8 { &self.map_md5[..8] } else { &self.map_md5 };
            format!("Beatmap ({})", short)
        }
    }
}

/// Retrieves cached beatmap metadata from beatmaps table
pub async fn get_beatmap_by_md5(pool: &DbPool, md5: &str) -> Option<BeatmapMeta> {
    let row = sqlx::query(
        r#"
        SELECT map_md5, beatmap_id, beatmapset_id, artist, title, version, creator
        FROM beatmaps
        WHERE map_md5 = ?
        LIMIT 1
        "#,
    )
    .bind(md5)
    .fetch_optional(pool)
    .await
    .ok()??;

    Some(BeatmapMeta {
        map_md5: row.get("map_md5"),
        beatmap_id: row.get("beatmap_id"),
        beatmapset_id: row.get("beatmapset_id"),
        artist: row.get("artist"),
        title: row.get("title"),
        version: row.get("version"),
        creator: row.get("creator"),
    })
}

/// Saves or updates beatmap metadata in beatmaps table
pub async fn save_beatmap(pool: &DbPool, meta: &BeatmapMeta) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        r#"
        INSERT INTO beatmaps (map_md5, beatmap_id, beatmapset_id, artist, title, version, creator, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(map_md5) DO UPDATE SET
            beatmap_id = CASE WHEN excluded.beatmap_id > 0 THEN excluded.beatmap_id ELSE beatmaps.beatmap_id END,
            beatmapset_id = CASE WHEN excluded.beatmapset_id > 0 THEN excluded.beatmapset_id ELSE beatmaps.beatmapset_id END,
            artist = CASE WHEN excluded.artist != '' THEN excluded.artist ELSE beatmaps.artist END,
            title = CASE WHEN excluded.title != '' THEN excluded.title ELSE beatmaps.title END,
            version = CASE WHEN excluded.version != '' THEN excluded.version ELSE beatmaps.version END,
            creator = CASE WHEN excluded.creator != '' THEN excluded.creator ELSE beatmaps.creator END,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(&meta.map_md5)
    .bind(meta.beatmap_id)
    .bind(meta.beatmapset_id)
    .bind(&meta.artist)
    .bind(&meta.title)
    .bind(&meta.version)
    .bind(&meta.creator)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(())
}

/// Saves raw beatmap filename/name string (e.g. from osu! f param or multiplayer room)
pub async fn save_raw_beatmap_name(pool: &DbPool, md5: &str, raw_name: &str, beatmap_id: i64) {
    if md5.trim().is_empty() || raw_name.trim().is_empty() {
        return;
    }

    let clean = raw_name.trim().trim_end_matches(".osu");
    let (artist, title, version) = parse_osu_filename(clean);

    let meta = BeatmapMeta {
        map_md5: md5.to_string(),
        beatmap_id,
        beatmapset_id: 0,
        artist,
        title,
        version,
        creator: String::new(),
    };

    let _ = save_beatmap(pool, &meta).await;
}

fn parse_osu_filename(s: &str) -> (String, String, String) {
    let mut version = String::new();
    let mut rest = s;

    if let (Some(start), Some(end)) = (s.rfind('['), s.rfind(']')) {
        if start < end {
            version = s[start + 1..end].trim().to_string();
            rest = s[..start].trim();
        }
    }

    if let (Some(c_start), Some(c_end)) = (rest.rfind('('), rest.rfind(')')) {
        if c_start < c_end && c_end == rest.len() - 1 {
            rest = rest[..c_start].trim();
        }
    }

    if let Some(dash_idx) = rest.find(" - ") {
        let artist = rest[..dash_idx].trim().to_string();
        let title = rest[dash_idx + 3..].trim().to_string();
        (artist, title, version)
    } else {
        (String::new(), rest.to_string(), version)
    }
}

#[derive(Debug, Deserialize)]
struct CatboyBeatmap {
    id: Option<i64>,
    beatmapset_id: Option<i64>,
    version: Option<String>,
    set: Option<CatboySet>,
}

#[derive(Debug, Deserialize)]
struct CatboySet {
    id: Option<i64>,
    title: Option<String>,
    title_unicode: Option<String>,
    artist: Option<String>,
    artist_unicode: Option<String>,
    creator: Option<String>,
}

/// Resolves beatmap metadata from SQLite cache or online mirror (Catboy API)
pub async fn resolve_beatmap_meta(pool: &DbPool, md5: &str) -> BeatmapMeta {
    let clean_md5 = md5.trim();
    if clean_md5.is_empty() {
        return BeatmapMeta::default();
    }

    // Step 1: Check database cache
    if let Some(cached) = get_beatmap_by_md5(pool, clean_md5).await {
        if !cached.title.is_empty() {
            return cached;
        }
    }

    // Step 2: Check match_history table
    if let Ok(Some(row)) = sqlx::query(
        "SELECT beatmap_name, beatmap_id FROM match_history WHERE beatmap_md5 = ? AND beatmap_name != '' LIMIT 1"
    )
    .bind(clean_md5)
    .fetch_optional(pool)
    .await
    {
        let name: String = row.get("beatmap_name");
        let bid: i64 = row.get("beatmap_id");
        if !name.is_empty() {
            save_raw_beatmap_name(pool, clean_md5, &name, bid).await;
            if let Some(cached) = get_beatmap_by_md5(pool, clean_md5).await {
                return cached;
            }
        }
    }

    // Step 3: Fetch from Catboy mirror API
    let url = format!("https://catboy.best/api/v2/md5/{}", clean_md5);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build();

    if let Ok(client) = client {
        match client.get(&url).header("User-Agent", "AyanomiBancho").send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(data) = resp.json::<CatboyBeatmap>().await {
                    let set = data.set;
                    let title = set.as_ref()
                        .and_then(|s| s.title.clone().or_else(|| s.title_unicode.clone()))
                        .unwrap_or_default();
                    let artist = set.as_ref()
                        .and_then(|s| s.artist.clone().or_else(|| s.artist_unicode.clone()))
                        .unwrap_or_default();
                    let creator = set.as_ref().and_then(|s| s.creator.clone()).unwrap_or_default();
                    let bid = data.id.unwrap_or(0);
                    let bsid = data.beatmapset_id.or_else(|| set.and_then(|s| s.id)).unwrap_or(0);
                    let version = data.version.unwrap_or_default();

                    let meta = BeatmapMeta {
                        map_md5: clean_md5.to_string(),
                        beatmap_id: bid,
                        beatmapset_id: bsid,
                        artist,
                        title,
                        version,
                        creator,
                    };

                    let _ = save_beatmap(pool, &meta).await;
                    info!("Resolved beatmap {} -> {}", clean_md5, meta.display_name());
                    return meta;
                }
            }
            Ok(resp) => {
                warn!("Mirror returned status {} for md5 {}", resp.status(), clean_md5);
            }
            Err(e) => {
                warn!("Failed to query mirror for beatmap md5 {}: {}", clean_md5, e);
            }
        }
    }

    // Step 4: Fallback
    BeatmapMeta {
        map_md5: clean_md5.to_string(),
        ..Default::default()
    }
}
