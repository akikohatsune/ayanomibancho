use crate::server::frontend::{get_authenticated_user, ApiResponse};
use crate::state::AppState;
use axum::extract::{Multipart, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::fs;
use std::path::Path as FilePath;
use tracing::{error, info, warn};

static DEFAULT_AVATAR: &[u8] = include_bytes!("../default/marisa2.jpg");

fn strip_png_metadata(bytes: &[u8]) -> Option<Vec<u8>> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE) { return None; }
    let mut output = SIGNATURE.to_vec();
    let mut offset = SIGNATURE.len();
    let mut saw_iend = false;
    while offset.checked_add(12)? <= bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().ok()?) as usize;
        let end = offset.checked_add(12)?.checked_add(length)?;
        if end > bytes.len() { return None; }
        let kind = &bytes[offset + 4..offset + 8];
        if !matches!(kind, b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"iCCP") {
            output.extend_from_slice(&bytes[offset..end]);
        }
        offset = end;
        if kind == b"IEND" { saw_iend = true; break; }
    }
    (saw_iend && offset == bytes.len()).then_some(output)
}

fn strip_jpeg_metadata(bytes: &[u8]) -> Option<Vec<u8>> {
    if !bytes.starts_with(b"\xff\xd8") { return None; }
    let mut output = bytes[..2].to_vec();
    let mut offset = 2usize;
    while offset < bytes.len() {
        if bytes[offset] != 0xff { return None; }
        let marker_start = offset;
        while offset < bytes.len() && bytes[offset] == 0xff { offset += 1; }
        let marker = *bytes.get(offset)?;
        offset += 1;
        if marker == 0xda { output.extend_from_slice(&bytes[marker_start..]); return Some(output); }
        if marker == 0xd9 { output.extend_from_slice(&bytes[marker_start..offset]); return (offset == bytes.len()).then_some(output); }
        if matches!(marker, 0x01 | 0xd0..=0xd7) { output.extend_from_slice(&bytes[marker_start..offset]); continue; }
        let length_end = offset.checked_add(2)?;
        if length_end > bytes.len() { return None; }
        let length = u16::from_be_bytes(bytes[offset..length_end].try_into().ok()?) as usize;
        if length < 2 { return None; }
        let segment_end = offset.checked_add(length)?;
        if segment_end > bytes.len() { return None; }
        if !matches!(marker, 0xe1 | 0xed | 0xfe) { output.extend_from_slice(&bytes[marker_start..segment_end]); }
        offset = segment_end;
    }
    None
}

fn strip_webp_metadata(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" { return None; }
    let declared = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize + 8;
    if declared != bytes.len() { return None; }
    let mut output = b"RIFF\0\0\0\0WEBP".to_vec();
    let mut offset = 12usize;
    while offset.checked_add(8)? <= bytes.len() {
        let kind = &bytes[offset..offset + 4];
        let length = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
        let end = offset.checked_add(8)?.checked_add(length.checked_add(length & 1)?)?;
        if end > bytes.len() { return None; }
        if !matches!(kind, b"EXIF" | b"XMP ") {
            let start = output.len();
            output.extend_from_slice(&bytes[offset..end]);
            if kind == b"VP8X" && length >= 1 { output[start + 8] &= !(0x08 | 0x04); }
        }
        offset = end;
    }
    if offset != bytes.len() { return None; }
    let riff_size = u32::try_from(output.len().checked_sub(8)?).ok()?;
    output[4..8].copy_from_slice(&riff_size.to_le_bytes());
    Some(output)
}

pub(crate) fn sanitize_uploaded_image(bytes: &[u8]) -> Option<Vec<u8>> {
    strip_png_metadata(bytes).or_else(|| strip_jpeg_metadata(bytes)).or_else(|| strip_webp_metadata(bytes))
}

fn sync_to_local_osu_cache(user_id: i32, bytes: Option<&[u8]>) {
    let local_appdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
    if !local_appdata.is_empty() {
        let candidates = [
            format!("{}/osu!test/Data/a/{}", local_appdata, user_id),
            format!("{}/osu!/Data/a/{}", local_appdata, user_id),
        ];
        for path_str in &candidates {
            let p = std::path::Path::new(path_str);
            if let Some(parent) = p.parent() {
                if parent.exists() {
                    match bytes {
                        Some(b) => {
                            let _ = std::fs::write(p, b);
                        }
                        None => {
                            let _ = std::fs::remove_file(p);
                        }
                    }
                }
            }
        }
    }
}

/// Serves user avatar from `data/avatars/:id.png` or falls back to `default/marisa2.jpg`
pub async fn get_avatar(Path(raw_id): Path<String>) -> Response {
    let clean = raw_id.trim();
    let num_part = clean
        .strip_suffix(".png")
        .or_else(|| clean.strip_suffix(".jpg"))
        .or_else(|| clean.strip_suffix(".jpeg"))
        .or_else(|| clean.strip_suffix(".webp"))
        .unwrap_or(clean);

    let user_id = match num_part.parse::<i32>() {
        Ok(id) => id,
        Err(_) => return (StatusCode::NOT_FOUND, "Avatar not found").into_response(),
    };

    let possible_paths = [
        format!("data/avatars/{}.png", user_id),
        format!("data/avatars/{}.jpg", user_id),
        format!("data/avatars/{}.jpeg", user_id),
        format!("data/avatars/{}.webp", user_id),
    ];

    for custom_path in &possible_paths {
        if FilePath::new(custom_path).exists() {
            if let Ok(bytes) = fs::read(custom_path) {
                let content_type = if bytes.starts_with(b"\x89PNG") {
                    "image/png"
                } else if bytes.starts_with(b"\xFF\xD8\xFF") {
                    "image/jpeg"
                } else if bytes.starts_with(b"RIFF") && bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
                    "image/webp"
                } else {
                    "image/png"
                };
                return (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, content_type),
                        (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
                        (header::PRAGMA, "no-cache"),
                        (header::EXPIRES, "0"),
                    ],
                    bytes,
                )
                    .into_response();
            }
        }
    }

    // Default avatar: default/marisa2.jpg (fallback to compile-time embedded bytes)
    let avatar_bytes = match fs::read("default/marisa2.jpg") {
        Ok(b) => b,
        Err(_) => DEFAULT_AVATAR.to_vec(),
    };

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
            (header::PRAGMA, "no-cache"),
            (header::EXPIRES, "0"),
        ],
        avatar_bytes,
    )
        .into_response()
}

/// Fallback for root numeric paths e.g. GET /1 or GET /1.png from osu! client
pub async fn get_root_avatar_or_404(Path(raw_id): Path<String>) -> Response {
    let clean = raw_id.trim();
    let num_part = clean
        .strip_suffix(".png")
        .or_else(|| clean.strip_suffix(".jpg"))
        .or_else(|| clean.strip_suffix(".jpeg"))
        .or_else(|| clean.strip_suffix(".webp"))
        .unwrap_or(clean);

    if num_part.parse::<i32>().is_ok() {
        get_avatar(Path(raw_id)).await
    } else {
        (StatusCode::NOT_FOUND, "Page not found").into_response()
    }
}

/// Endpoint: `POST /api/profile/avatar/reset`
/// Resets the user's avatar to the default avatar (default/marisa2.jpg)
pub async fn reset_avatar_api(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let user = match get_authenticated_user(&state, &headers).await {
        Some(u) => u,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse {
                    success: false,
                    message: "Please log in to manage your avatar.".to_string(),
                }),
            )
                .into_response();
        }
    };

    for ext in &["png", "jpg", "jpeg", "webp"] {
        let p = format!("data/avatars/{}.{}", user.id, ext);
        if FilePath::new(&p).exists() {
            let _ = fs::remove_file(&p);
        }
    }
    sync_to_local_osu_cache(user.id, None);

    info!("Avatar reset to default for user '{}' (ID: {})", user.username, user.id);

    (
        StatusCode::OK,
        Json(ApiResponse {
            success: true,
            message: "Avatar reset to default successfully!".to_string(),
        }),
    )
        .into_response()
}

/// Endpoint: `POST /api/profile/avatar`
/// Authenticated avatar upload for the current user
pub async fn upload_avatar_api(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let user = match get_authenticated_user(&state, &headers).await {
        Some(u) => u,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse {
                    success: false,
                    message: "Please log in to change your avatar.".to_string(),
                }),
            )
                .into_response();
        }
    };

    let dir = "data/avatars";
    let _ = fs::create_dir_all(dir);

    while let Ok(Some(field)) = multipart.next_field().await {
        let field_name = field.name().unwrap_or_default().to_string();
        if field_name == "avatar" || field_name == "file" {
            let filename = field.file_name().unwrap_or("avatar.png").to_string().to_lowercase();
            if !filename.ends_with(".png")
                && !filename.ends_with(".jpg")
                && !filename.ends_with(".jpeg")
                && !filename.ends_with(".webp")
            {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        success: false,
                        message: "Invalid image format. Only PNG, JPG, JPEG, and WebP are supported.".to_string(),
                    }),
                )
                    .into_response();
            }

            match field.bytes().await {
                Ok(bytes) => {
                    // Limit max avatar size to 5MB
                    if bytes.len() > 5 * 1024 * 1024 {
                        return (
                            StatusCode::PAYLOAD_TOO_LARGE,
                            Json(ApiResponse {
                                success: false,
                                message: "Avatar file size exceeds the 5MB limit.".to_string(),
                            }),
                        )
                            .into_response();
                    }

                    if bytes.is_empty() {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ApiResponse {
                                success: false,
                                message: "Uploaded file is empty.".to_string(),
                            }),
                        )
                            .into_response();
                    }

                    let sanitized = match sanitize_uploaded_image(&bytes) {
                        Some(image) => image,
                        None => return (StatusCode::BAD_REQUEST, Json(ApiResponse {
                            success: false,
                            message: "The uploaded file is not a valid PNG, JPEG, or WebP image.".to_string(),
                        })).into_response(),
                    };

                    let dest = format!("{}/{}.png", dir, user.id);
                    if let Err(e) = fs::write(&dest, &sanitized) {
                        error!("Failed to save avatar for user {}: {}", user.id, e);
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiResponse {
                                success: false,
                                message: "Failed to save avatar.".to_string(),
                            }),
                        )
                            .into_response();
                    }

                    sync_to_local_osu_cache(user.id, Some(&sanitized));

                    info!("Avatar updated for user '{}' (ID: {}) [{} bytes]", user.username, user.id, sanitized.len());
                    return (
                        StatusCode::OK,
                        Json(ApiResponse {
                            success: true,
                            message: "Avatar updated successfully!".to_string(),
                        }),
                    )
                        .into_response();
                }
                Err(e) => {
                    warn!("Failed reading multipart avatar data: {}", e);
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ApiResponse {
                            success: false,
                            message: "Failed to read uploaded avatar file.".to_string(),
                        }),
                    )
                        .into_response();
                }
            }
        }
    }

    (
        StatusCode::BAD_REQUEST,
        Json(ApiResponse {
            success: false,
            message: "No avatar image file found in the request.".to_string(),
        }),
    )
        .into_response()
}

// -------------------------------------------------------------------------------------------------
// Profile Header Covers / Banners
// -------------------------------------------------------------------------------------------------

static DEFAULT_BANNER_SVG: &str = r###"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1400 440" width="100%" height="100%">
  <defs>
    <linearGradient id="bgGrad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#141824"/>
      <stop offset="50%" stop-color="#1e1b4b"/>
      <stop offset="100%" stop-color="#0f172a"/>
    </linearGradient>
    <radialGradient id="pinkGlow" cx="20%" cy="30%" r="60%">
      <stop offset="0%" stop-color="rgba(244, 114, 182, 0.22)"/>
      <stop offset="100%" stop-color="rgba(244, 114, 182, 0)"/>
    </radialGradient>
    <radialGradient id="cyanGlow" cx="85%" cy="70%" r="60%">
      <stop offset="0%" stop-color="rgba(56, 189, 248, 0.16)"/>
      <stop offset="100%" stop-color="rgba(56, 189, 248, 0)"/>
    </radialGradient>
    <pattern id="grid" width="40" height="40" patternUnits="userSpaceOnUse">
      <path d="M 40 0 L 0 0 0 40" fill="none" stroke="rgba(255,255,255,0.03)" stroke-width="1"/>
    </pattern>
  </defs>
  <rect width="1400" height="440" fill="url(#bgGrad)"/>
  <rect width="1400" height="440" fill="url(#grid)"/>
  <circle cx="280" cy="120" r="340" fill="url(#pinkGlow)"/>
  <circle cx="1150" cy="280" r="380" fill="url(#cyanGlow)"/>
  <circle cx="1220" cy="180" r="120" fill="none" stroke="rgba(244, 114, 182, 0.12)" stroke-width="8"/>
  <circle cx="1220" cy="180" r="75" fill="none" stroke="rgba(244, 114, 182, 0.08)" stroke-width="4"/>
  <circle cx="1220" cy="180" r="30" fill="rgba(244, 114, 182, 0.05)"/>
  <circle cx="150" cy="320" r="90" fill="none" stroke="rgba(56, 189, 248, 0.1)" stroke-width="6"/>
  <circle cx="150" cy="320" r="45" fill="none" stroke="rgba(56, 189, 248, 0.06)" stroke-width="3"/>
</svg>"###;

/// Serves user banner from `data/banners/:id.png` or falls back to default SVG banner
pub async fn get_banner(Path(raw_id): Path<String>) -> Response {
    let clean = raw_id.trim();
    let num_part = clean
        .strip_suffix(".png")
        .or_else(|| clean.strip_suffix(".jpg"))
        .or_else(|| clean.strip_suffix(".jpeg"))
        .or_else(|| clean.strip_suffix(".webp"))
        .unwrap_or(clean);

    let user_id = match num_part.parse::<i32>() {
        Ok(id) => id,
        Err(_) => return (StatusCode::NOT_FOUND, "Banner not found").into_response(),
    };

    let possible_paths = [
        format!("data/banners/{}.png", user_id),
        format!("data/banners/{}.jpg", user_id),
        format!("data/banners/{}.jpeg", user_id),
        format!("data/banners/{}.webp", user_id),
    ];

    for custom_path in &possible_paths {
        if FilePath::new(custom_path).exists() {
            if let Ok(bytes) = fs::read(custom_path) {
                let content_type = if bytes.starts_with(b"\x89PNG") {
                    "image/png"
                } else if bytes.starts_with(b"\xFF\xD8\xFF") {
                    "image/jpeg"
                } else if bytes.starts_with(b"RIFF") && bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
                    "image/webp"
                } else {
                    "image/png"
                };
                return (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, content_type),
                        (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
                        (header::PRAGMA, "no-cache"),
                        (header::EXPIRES, "0"),
                    ],
                    bytes,
                )
                    .into_response();
            }
        }
    }

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
            (header::PRAGMA, "no-cache"),
            (header::EXPIRES, "0"),
        ],
        DEFAULT_BANNER_SVG.as_bytes().to_vec(),
    )
        .into_response()
}

/// Endpoint: `POST /api/profile/banner/reset`
/// Resets the user's banner to the default banner
pub async fn reset_banner_api(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let user = match get_authenticated_user(&state, &headers).await {
        Some(u) => u,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse {
                    success: false,
                    message: "Please log in to manage your banner.".to_string(),
                }),
            )
                .into_response();
        }
    };

    for ext in &["png", "jpg", "jpeg", "webp"] {
        let p = format!("data/banners/{}.{}", user.id, ext);
        if FilePath::new(&p).exists() {
            let _ = fs::remove_file(&p);
        }
    }

    info!("Banner reset to default for user '{}' (ID: {})", user.username, user.id);

    (
        StatusCode::OK,
        Json(ApiResponse {
            success: true,
            message: "Banner reset to default successfully!".to_string(),
        }),
    )
        .into_response()
}

/// Endpoint: `POST /api/profile/banner`
/// Authenticated banner upload for the current user (Max 10MB)
pub async fn upload_banner_api(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let user = match get_authenticated_user(&state, &headers).await {
        Some(u) => u,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse {
                    success: false,
                    message: "Please log in to change your banner.".to_string(),
                }),
            )
                .into_response();
        }
    };

    let dir = "data/banners";
    let _ = fs::create_dir_all(dir);

    while let Ok(Some(field)) = multipart.next_field().await {
        let field_name = field.name().unwrap_or_default().to_string();
        if field_name == "banner" || field_name == "file" {
            let filename = field.file_name().unwrap_or("banner.png").to_string().to_lowercase();
            if !filename.ends_with(".png")
                && !filename.ends_with(".jpg")
                && !filename.ends_with(".jpeg")
                && !filename.ends_with(".webp")
            {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        success: false,
                        message: "Invalid image format. Only PNG, JPG, JPEG, and WebP are supported.".to_string(),
                    }),
                )
                    .into_response();
            }

            match field.bytes().await {
                Ok(bytes) => {
                    // Limit max banner size to 10MB
                    if bytes.len() > 10 * 1024 * 1024 {
                        return (
                            StatusCode::PAYLOAD_TOO_LARGE,
                            Json(ApiResponse {
                                success: false,
                                message: "Banner file size exceeds the 10MB limit.".to_string(),
                            }),
                        )
                            .into_response();
                    }

                    if bytes.is_empty() {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ApiResponse {
                                success: false,
                                message: "Uploaded file is empty.".to_string(),
                            }),
                        )
                            .into_response();
                    }

                    // Remove any old extension files for this user
                    for ext in &["png", "jpg", "jpeg", "webp"] {
                        let p = format!("{}/{}.{}", dir, user.id, ext);
                        if FilePath::new(&p).exists() {
                            let _ = fs::remove_file(&p);
                        }
                    }

                    let sanitized = match sanitize_uploaded_image(&bytes) {
                        Some(image) => image,
                        None => return (StatusCode::BAD_REQUEST, Json(ApiResponse {
                            success: false,
                            message: "The uploaded file is not a valid PNG, JPEG, or WebP image.".to_string(),
                        })).into_response(),
                    };

                    let dest = format!("{}/{}.png", dir, user.id);
                    if let Err(e) = fs::write(&dest, &sanitized) {
                        error!("Failed to save banner for user {}: {}", user.id, e);
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiResponse {
                                success: false,
                                message: "Failed to save banner.".to_string(),
                            }),
                        )
                            .into_response();
                    }

                    info!("Banner updated for user '{}' (ID: {}) [{} bytes]", user.username, user.id, sanitized.len());
                    return (
                        StatusCode::OK,
                        Json(ApiResponse {
                            success: true,
                            message: "Banner updated successfully!".to_string(),
                        }),
                    )
                        .into_response();
                }
                Err(e) => {
                    warn!("Failed reading multipart banner data: {}", e);
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ApiResponse {
                            success: false,
                            message: "Failed to read uploaded banner file.".to_string(),
                        }),
                    )
                        .into_response();
                }
            }
        }
    }

    (
        StatusCode::BAD_REQUEST,
        Json(ApiResponse {
            success: false,
            message: "No banner image file found in the request.".to_string(),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_sanitizer_rejects_non_images() {
        assert!(sanitize_uploaded_image(b"<script>alert(1)</script>").is_none());
    }

    #[test]
    fn upload_sanitizer_accepts_embedded_jpeg() {
        let sanitized = sanitize_uploaded_image(DEFAULT_AVATAR).expect("embedded JPEG is valid");
        assert!(sanitized.starts_with(b"\xff\xd8"));
    }
}
