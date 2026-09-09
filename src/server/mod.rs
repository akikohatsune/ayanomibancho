#![allow(dead_code)]

use crate::state::AppState;
use axum::extract::DefaultBodyLimit;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub mod avatars;
pub mod backgrounds;
pub mod bancho;
pub mod direct;
pub mod frontend;
pub mod ratelimit;
pub mod web;

/// Router for Bancho packet engine (runs on port 5001)
pub fn build_bancho_router(state: AppState) -> Router {
    Router::new()
        .route("/", post(bancho::bancho_post_handler))
        .route("/c", post(bancho::bancho_post_handler))
        .route("/health/bancho", get(bancho::bancho_health))
        .route("/internal/stats_update", post(bancho::internal_stats_update))
        .layer(from_fn_with_state(state.clone(), ratelimit::ratelimit_middleware))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Router for Web, Leaderboards, Scores, Direct Proxy, and Dashboard (runs on port 5002)
pub fn build_web_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(frontend::index_page))
        .route("/leaderboard", get(frontend::leaderboard_page))
        .route("/connect", get(frontend::connect_page))
        .route("/rule", get(frontend::rule_page))
        .route("/rules", get(frontend::rule_page))
        .route("/multi", get(frontend::multi_page))
        .route("/staff", get(frontend::staff_page))
        .route("/static/rust_logo.png", get(frontend::rust_logo_handler))
        .route("/static/logo.png", get(frontend::server_logo_handler))
        .route("/logo.png", get(frontend::server_logo_handler))
        .route("/static/menu-osu.png", get(frontend::menu_osu_handler))
        .route("/menu-osu.png", get(frontend::menu_osu_handler))
        .route("/certificate", get(frontend::download_ca_cert))
        .route("/ca.crt", get(frontend::download_ca_cert))
        .route("/u/{id}", get(frontend::profile_page))
        .route("/login", get(frontend::login_page))
        .route("/logout", get(frontend::logout_handler))
        .route("/api/login", post(frontend::api_login))
        .route("/api/logout", post(frontend::api_logout))
        .route("/api/profile/update", post(frontend::update_profile_api))
        .route("/api/profile/avatar", post(avatars::upload_avatar_api))
        .route("/admin", get(frontend::admin_page))
        .route("/health", get(web::web_health))
        .route("/health/web", get(web::web_health))

        // osu! In-game Account Registration & Live Validation
        .route("/users", post(web::osu_register_user))
        .route("/users/", post(web::osu_register_user))
        .route("/users/{id}", get(frontend::profile_page))
        .route("/favicon.ico", get(web::favicon))
        .route("/static/favicon.png", get(web::favicon_png))
        .route("/favicon.png", get(web::favicon_png))
        
        // osu! Web endpoints
        .route("/web/bancho_connect.php", get(web::bancho_connect).post(web::bancho_connect))
        .route("/web/check-updates.php", get(web::check_updates).post(web::check_updates))
        .route("/web/lastfm.php", get(web::lastfm).post(web::lastfm))
        .route("/web/osu-getfriends.php", get(web::get_friends).post(web::get_friends))
        .route("/web/osu-markasread.php", get(web::osu_markasread).post(web::osu_markasread))
        .route("/web/osu-getbeatmapinfo.php", get(web::osu_getbeatmapinfo).post(web::osu_getbeatmapinfo))
        .route("/web/osu-osz2-getscores.php", get(web::get_scores).post(web::get_scores))
        .route("/web/osu-submit-modular-selector.php", post(web::submit_score))
        .route("/web/osu-submit-modular.php", post(web::submit_score))
        
        // osu!Direct search & download endpoints (with 3s timeout & circuit breaker)
        .route("/web/osu-search.php", get(direct::search_beatmaps))
        .route("/web/osu-search-set.php", get(direct::search_beatmap_set))
        .route("/d/{set_id}", get(direct::download_beatmap))
        
        // User avatars, banners & custom profile
        .route("/a/{raw_id}", get(avatars::get_avatar))
        .route("/avatar/{raw_id}", get(avatars::get_avatar))
        .route("/b/{raw_id}", get(avatars::get_banner))
        .route("/banner/{raw_id}", get(avatars::get_banner))
        .route("/{raw_id}", get(avatars::get_root_avatar_or_404))
        .route("/api/profile/avatar/reset", post(avatars::reset_avatar_api))
        .route("/api/profile/banner", post(avatars::upload_banner_api))
        .route("/api/profile/banner/reset", post(avatars::reset_banner_api))
        
        // Telemetry & Registration API
        .route("/api/status", get(frontend::get_server_status))
        .route("/api/register", post(frontend::register_user))
        
        // Multiplayer Match History API
        .route("/api/matches", get(web::get_matches_api))
        .route("/api/matches/{id}", get(web::get_match_detail_api))
        
        // osu! Seasonal & Menu Backgrounds
        .route("/api/v2/seasonal-backgrounds", get(backgrounds::get_seasonal_backgrounds))
        .route("/seasonal-backgrounds", get(backgrounds::get_seasonal_backgrounds))
        .route("/web/osu-seasonal.php", get(backgrounds::get_seasonal_backgrounds))
        .route("/web/osu-getseasonal.php", get(backgrounds::get_seasonal_backgrounds_stable))
        .route("/menu-content.json", get(backgrounds::get_menu_content_json))
        .route("/backgrounds/{filename}", get(backgrounds::serve_background_file))
        .route("/api/backgrounds", get(backgrounds::list_backgrounds_api))
        .route("/api/backgrounds/upload", post(backgrounds::upload_background_api))
        .route("/api/backgrounds/{filename}", delete(backgrounds::delete_background_api))
        .route("/api/backgrounds/delete/{filename}", post(backgrounds::delete_background_api))
        
        // Chat History API (Dedicated SQLite DB)
        .route("/api/chat/history", get(web::get_chat_history_api))
        
        // Badges Management API (Dedicated SQLite DB)
        .route("/api/badges", get(web::list_badges_api))
        .route("/api/users/{id}/badges", get(web::get_user_badges_api))
        .route("/api/badges/create", post(web::create_badge_api))
        .route("/api/badges/award", post(web::award_badge_api))
        .route("/api/badges/revoke", post(web::revoke_badge_api))
        
        .layer(from_fn(ratelimit::admin_local_guard_middleware))
        .layer(from_fn_with_state(state.clone(), ratelimit::ratelimit_middleware))
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Unified router combining both services
pub fn build_unified_router(state: AppState) -> Router {
    Router::new()
        .merge(build_bancho_router(state.clone()))
        .merge(build_web_router(state))
}
