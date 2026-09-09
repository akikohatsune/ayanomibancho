use crate::state::AppState;
use axum::extract::{Multipart, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path as FilePath;
use tracing::{error, info, warn};

#[derive(Debug, Serialize, Deserialize)]
pub struct SeasonalBackgroundsResponse {
    pub ends_at: String,
    pub backgrounds: Vec<SeasonalBackgroundItem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SeasonalBackgroundItem {
    pub url: String,
    pub user: SeasonalBackgroundUser,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SeasonalBackgroundUser {
    pub id: i32,
    pub username: String,
    pub country_code: String,
    pub avatar_url: String,
    pub default_group: String,
    pub is_active: bool,
    pub is_bot: bool,
    pub is_deleted: bool,
    pub is_online: bool,
    pub is_supporter: bool,
    pub last_visit: String,
    pub pm_friends_only: bool,
    pub profile_colour: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackgroundFileInfo {
    pub filename: String,
    pub url: String,
    pub size_bytes: u64,
}

/// Helper to sanitize a filename and check extension
pub fn is_valid_image_filename(filename: &str) -> bool {
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return false;
    }
    let lower = filename.to_lowercase();
    lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png") || lower.ends_with(".webp")
}

/// Helper to determine MIME type from filename
pub fn get_image_mime(filename: &str) -> &'static str {
    let lower = filename.to_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

/// Helper to scan background directory
pub fn scan_backgrounds(dir: &str) -> Vec<(String, u64)> {
    let mut list = Vec::new();
    let path = FilePath::new(dir);
    if !path.exists() {
        let _ = fs::create_dir_all(path);
    }
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() {
                if ft.is_file() {
                    if let Some(fname) = entry.file_name().to_str() {
                        if is_valid_image_filename(fname) {
                            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                            list.push((fname.to_string(), size));
                        }
                    }
                }
            }
        }
    }
    list
}

/// Helper to extract clean hostname without internal port or subdomains
pub fn extract_clean_domain(headers: &HeaderMap, fallback: &str) -> String {
    let host_hdr = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok());

    let raw = if let Some(h) = host_hdr {
        h.to_string()
    } else {
        fallback.to_string()
    };

    let host_only = raw.split(':').next().unwrap_or(&raw).to_string();
    let mut cleaned = host_only.as_str();
    for prefix in &["osu.", "c.", "assets.", "a."] {
        if cleaned.starts_with(prefix) {
            cleaned = &cleaned[prefix.len()..];
            break;
        }
    }
    cleaned.to_string()
}

/// `GET /api/v2/seasonal-backgrounds`, `GET /seasonal-backgrounds`, `GET /web/osu-seasonal.php`
/// Endpoint queried by the osu! client for main menu backgrounds
pub async fn get_seasonal_backgrounds(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<SeasonalBackgroundsResponse> {
    let domain = extract_clean_domain(&headers, &state.config.server.domain);

    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("https");

    let dir = &state.config.backgrounds.directory;
    let mut images = scan_backgrounds(dir);

    // Prioritize ayanomi_welcome.png as the first and primary background
    images.sort_by(|(a, _), (b, _)| {
        if a == "ayanomi_welcome.png" {
            std::cmp::Ordering::Less
        } else if b == "ayanomi_welcome.png" {
            std::cmp::Ordering::Greater
        } else {
            a.cmp(b)
        }
    });

    let artist_name = state.config.backgrounds.artist_name.clone();
    let now = chrono::Utc::now();
    let ends_at = (now + chrono::Duration::days(365)).to_rfc3339();
    let last_visit = now.to_rfc3339();

    let welcome_bg = if FilePath::new("data/backgrounds/ayanomi_welcome.jpg").exists() {
        "ayanomi_welcome.jpg"
    } else {
        "ayanomi_welcome.png"
    };

    let backgrounds = vec![
        SeasonalBackgroundItem {
            url: format!("{}://{}/backgrounds/{}", proto, domain, welcome_bg),
            user: SeasonalBackgroundUser {
                id: 1,
                username: artist_name.clone(),
                country_code: "VN".to_string(),
                avatar_url: format!("{}://{}/a/1", proto, domain),
                default_group: "default".to_string(),
                is_active: true,
                is_bot: false,
                is_deleted: false,
                is_online: true,
                is_supporter: true,
                last_visit: last_visit.clone(),
                pm_friends_only: false,
                profile_colour: None,
            },
        }
    ];

    Json(SeasonalBackgroundsResponse {
        ends_at,
        backgrounds,
    })
}

/// `GET /web/osu-getseasonal.php`
/// Endpoint queried specifically by osu! stable client when logging into server
pub async fn get_seasonal_backgrounds_stable(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<Vec<String>> {
    let domain = extract_clean_domain(&headers, &state.config.server.domain);

    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("https");

    let welcome_bg = if FilePath::new("data/backgrounds/ayanomi_welcome.jpg").exists() {
        "ayanomi_welcome.jpg"
    } else {
        "ayanomi_welcome.png"
    };

    // Return ONLY the user's welcome background
    Json(vec![format!("{}://{}/backgrounds/{}", proto, domain, welcome_bg)])
}

/// `GET /menu-content.json`
/// Endpoint queried by osu! client for bottom promo banner (empty to disable banner)
pub async fn get_menu_content_json() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "images": []
    }))
}


/// `GET /backgrounds/{filename}`
/// Serves the actual image file to the osu! client and web dashboard
pub async fn serve_background_file(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> Response {
    if !is_valid_image_filename(&filename) {
        return (StatusCode::BAD_REQUEST, "Invalid or unsafe filename").into_response();
    }

    let file_path = FilePath::new(&state.config.backgrounds.directory).join(&filename);
    if !file_path.exists() {
        return (StatusCode::NOT_FOUND, "Background image not found").into_response();
    }

    match fs::read(&file_path) {
        Ok(bytes) => {
            let mime = get_image_mime(&filename);
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, "public, max-age=86400"),
                ],
                bytes,
            )
                .into_response()
        }
        Err(e) => {
            error!("Failed to read background image {}: {}", filename, e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read image").into_response()
        }
    }
}

/// `GET /api/backgrounds`
/// List all currently configured background files
pub async fn list_backgrounds_api(State(state): State<AppState>) -> Json<Vec<BackgroundFileInfo>> {
    let images = scan_backgrounds(&state.config.backgrounds.directory);
    let list = images
        .into_iter()
        .map(|(filename, size_bytes)| {
            let url = format!("/backgrounds/{}", filename);
            BackgroundFileInfo {
                filename,
                url,
                size_bytes,
            }
        })
        .collect();
    Json(list)
}

/// `POST /api/backgrounds/upload`
/// Handles file upload for background images (multipart/form-data)
pub async fn upload_background_api(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let is_browser = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|a| a.contains("text/html"))
        .unwrap_or(false);

    let dir = &state.config.backgrounds.directory;
    let _ = fs::create_dir_all(dir);

    let mut saved_filename: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let field_name = field.name().unwrap_or_default().to_string();
        if field_name == "file" || field_name == "background" {
            let raw_fname = field
                .file_name()
                .unwrap_or("uploaded_bg.jpg")
                .to_string();

            // Sanitize filename: extract basename and strip hazardous chars
            let raw_base = FilePath::new(&raw_fname)
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("bg.jpg");

            let safe_name: String = raw_base
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
                .collect();

            if !is_valid_image_filename(&safe_name) {
                return (
                    StatusCode::BAD_REQUEST,
                    "Invalid file extension. Only .jpg, .jpeg, .png, and .webp are supported.",
                )
                    .into_response();
            }

            match field.bytes().await {
                Ok(bytes) => {
                    // Limit max upload size to 15MB
                    if bytes.len() > 15 * 1024 * 1024 {
                        return (StatusCode::PAYLOAD_TOO_LARGE, "File exceeds 15MB limit")
                            .into_response();
                    }

                    let dest = FilePath::new(dir).join(&safe_name);
                    if let Err(e) = fs::write(&dest, &bytes) {
                        error!("Failed to save uploaded background {}: {}", safe_name, e);
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Failed to write image file",
                        )
                            .into_response();
                    }

                    info!("New menu background uploaded: {} ({} bytes)", safe_name, bytes.len());
                    saved_filename = Some(safe_name);
                }
                Err(e) => {
                    warn!("Failed reading multipart field: {}", e);
                    return (StatusCode::BAD_REQUEST, "Failed to read upload data").into_response();
                }
            }
        }
    }

    if let Some(fname) = saved_filename {
        if is_browser {
            Redirect::to("/").into_response()
        } else {
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "success",
                    "filename": fname,
                    "message": "Background uploaded successfully"
                })),
            )
                .into_response()
        }
    } else {
        (StatusCode::BAD_REQUEST, "No file uploaded in form").into_response()
    }
}

/// `DELETE /api/backgrounds/{filename}` and `POST /api/backgrounds/delete/{filename}`
/// Deletes a background image
pub async fn delete_background_api(
    State(state): State<AppState>,
    Path(filename): Path<String>,
    headers: HeaderMap,
) -> Response {
    let is_browser = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|a| a.contains("text/html"))
        .unwrap_or(false);

    if !is_valid_image_filename(&filename) {
        return (StatusCode::BAD_REQUEST, "Invalid filename").into_response();
    }

    let file_path = FilePath::new(&state.config.backgrounds.directory).join(&filename);
    if file_path.exists() {
        if let Err(e) = fs::remove_file(&file_path) {
            error!("Failed to delete background {}: {}", filename, e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete file").into_response();
        }
        info!("Background deleted: {}", filename);
    }

    if is_browser {
        Redirect::to("/").into_response()
    } else {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "message": format!("Background '{}' removed", filename)
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_image_filename_security() {
        assert!(is_valid_image_filename("wallpaper.jpg"));
        assert!(is_valid_image_filename("bg-1.PNG"));
        assert!(is_valid_image_filename("image_2026.webp"));

        // Dangerous / traversal attempts
        assert!(!is_valid_image_filename("../secret.jpg"));
        assert!(!is_valid_image_filename("folder/bg.png"));
        assert!(!is_valid_image_filename("..\\evil.jpg"));
        assert!(!is_valid_image_filename("script.php"));
        assert!(!is_valid_image_filename("virus.exe"));
    }

    #[test]
    fn test_seasonal_backgrounds_response_serialization() {
        let resp = SeasonalBackgroundsResponse {
            ends_at: "2030-01-01T00:00:00Z".to_string(),
            backgrounds: vec![SeasonalBackgroundItem {
                url: "http://127.0.0.1:5000/backgrounds/bg.jpg".to_string(),
                user: SeasonalBackgroundUser {
                    id: 1,
                    username: "AyanomiBancho".to_string(),
                    country_code: "VN".to_string(),
                    avatar_url: "http://127.0.0.1:5000/a/1".to_string(),
                    default_group: "default".to_string(),
                    is_active: true,
                    is_bot: false,
                    is_deleted: false,
                    is_online: true,
                    is_supporter: true,
                    last_visit: "2026-09-07T00:00:00Z".to_string(),
                    pm_friends_only: false,
                    profile_colour: None,
                },
            }],
        };

        let json_str = serde_json::to_string(&resp).unwrap();
        assert!(json_str.contains("\"ends_at\":\"2030-01-01T00:00:00Z\""));
        assert!(json_str.contains("\"url\":\"http://127.0.0.1:5000/backgrounds/bg.jpg\""));
        assert!(json_str.contains("\"username\":\"AyanomiBancho\""));
    }
}
