# Changelog

All notable changes to the **AyanomiBancho** osu! server project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [v0.4.2] - 2026-09-14

### Added
- **Cloudflare Turnstile Bot Protection (`src/server/frontend.rs`, `static/js/login.js`)**:
  - Enforced Turnstile bot verification on web login and user onboarding (`/api/login`).
  - Client-side validation in `login.js` requires challenge completion before submitting form, displaying responsive alerts instead of unhandled errors.
  - Server-side verification via Cloudflare `siteverify` API with flexible hostname handling.
- **Dynamic Seasonal Backgrounds (`src/server/backgrounds.rs`)**:
  - Overhauled seasonal backgrounds engine to dynamically scan and serve all uploaded image formats (`.png`, `.jpg`, `.jpeg`, `.webp`) from `data/backgrounds/`.
  - Eliminated hardcoded image names and 404 dead links, enabling the osu! client to rotate through all uploaded backgrounds seamlessly.
- **Fail2Ban Security Integration & Phone Daemon Scripts (`scripts/phone/`)**:
  - Added dedicated run scripts for Termux (`run_server.sh`, `run_cf.sh`, `run_fail2ban.sh`, `setup_tmux.sh`).
  - Integrated Fail2Ban jails and Nginx filter rules for rate limiting and bot mitigation.

### Fixed
- **Cloudflare Tunnel 502 Bad Gateway Drops (`scripts/phone/run_cf.sh`)**:
  - Resolved 502 connection drops caused by HTTP/2 over TCP multiplexing stream resets. Upgraded tunnel protocol to QUIC over UDP with IPv4 edge selection (`--edge-ip-version 4`).
- **CSRF Middleware & Origin Whitelisting (`src/server/ratelimit.rs`)**:
  - Enhanced CSRF guard middleware with flexible origin and host verification, correctly handling LAN access, localhost, and proxy forwarded hosts.
  - Replaced plain-text 403 rejection bodies with structured JSON `ApiResponse`, eliminating client-side JSON parsing errors.
- **Relaxed Rate Limiter Thresholds (`config.toml`)**:
  - Increased request rate limits across all tiers (General 3600 RPM, Bancho 3600 RPM, Direct 1800 RPM, Sensitive 180 RPM) for smoother gameplay and web navigation.

---

## [Early Preview 2026/09/13 - v0.4.1] - 2026-09-13

### Added
- **In-Game Client Version Tracking & Display (`src/bancho/session.rs`, `src/server/frontend.rs`)**:
  - Bancho handshake parser now captures client build version info from login payload parameters and `osu-version` HTTP headers into active user sessions.
  - Online player cards on the homepage now display the active client version (e.g. `osu!fx [b20241029.1]`, `osu! stable`) directly under the action status in a clean monospace badge.

### Fixed
- **Profile Cover Banner Endpoint Migration (`templates/profile.html`, `static/js/profile.js`)**:
  - Migrated profile banner image sources and AJAX upload/reset handlers from `/b/{id}` to `/banner/{id}`, fixing blank banners caused by beatmap route redirection.

---

## [Early Preview 2026/09/13 - v0.4.0] - 2026-09-13

### Added
- **Dedicated osu!fx Client Architecture (`src/server/osufx.rs`)**:
  - Independent connection and handshake handling for osu!fx client via query detection (`fx=`, `cuttingedge`, `User-Agent`).
  - Added `/web/osu-checktweets.php` and `/web/osu-error.php` endpoints to eliminate client network errors.
  - Bancho gateway ping support on `/` and `/c` returning `cho-protocol: 19`.
- **In-Game Multiplayer Beatmap Updates (`/web/maps/{filename}`)**:
  - Direct proxying of `.osu` difficulty files from official osu! web servers, resolving the orange "Click here to update this beatmap" button in multiplayer lobbies.
- **Direct Download & Streaming Proxy (`src/server/direct.rs`)**:
  - Implemented `/d/{set_id}` proxy with chunk streaming from Cheesegull / Hinamizawa mirrors, bypassing .NET HTTP-to-HTTPS redirect restrictions and filename issues.
  - Tolerant JSON deserialization for mirror search (supporting `HasVideo` as boolean or integer).
- **Pure Rust oppai-ng Integration (`rustpp`)**:
  - Integrated `rustpp` (pure Rust port of oppai-ng from `https://gitlab.com/luminehq/rustpp`) as the official Star Rating and Performance Points calculation engine.

### Fixed
- **Beatmap Route Conflict & User Banner Fix (`src/server/mod.rs`)**:
  - Separated `/b/{raw_id}` and `/s/{raw_id}` from user banner handlers, properly redirecting requests (HTTP 307) to official osu! web pages instead of rendering profile banners.
  - User profile banners are now cleanly served under `/banner/{raw_id}` and `/banners/{raw_id}`.
- **Dynamic Post-Match Ranking URLs (`src/server/web.rs`)**:
  - Replaced hardcoded `http://127.0.0.1:5000/b/1` with dynamic base URLs (`get_public_base_url`), supporting public HTTPS domains (`hatsuneakiko.io.vn`) as well as local development environments.
  - Resolved real `beatmap_id` and `beatmapset_id` in `submit_score` and `get_scores`.

---

## [0.3.1] - 2026-09-11

### Added
- **Algorithmic Performance Points (PP) Engine (`src/utils/score_calc.rs`)**:
  - Implemented standalone PP calculations for **osu! Standard**, **Taiko**, **Catch the Beat**, **osu!mania**, and **Relax (RX)** modes.
  - Accounts for beatmap difficulty (Star Rating), non-linear combo scaling (`combo^0.85`), accuracy threshold curves, miss penalties (`0.97^miss`), and mod multipliers (`HD`, `HR`, `DT`/`NC`, `FL`, `EZ`, `HT`, `NF`, `RX`).
- **Official osu! Weighted Stats Decay**:
  - Profile PP decay formula: $\text{Total PP} = \sum_{i=0}^{N-1} \text{pp}_i \times 0.95^i + \text{Bonus PP}$.
  - Profile Accuracy formula: $\text{Weighted Acc} = \frac{\sum_{i=0}^{N-1} \text{acc}_i \times 0.95^i}{\sum_{i=0}^{N-1} 0.95^i}$.
  - Bonus PP curve based on unique submitted scores: $416.6667 \times (1 - 0.9994^N)$.
- **Dynamic Post-Match Submission Charts (`src/server/web.rs`)**:
  - Post-play charts response now transmits real-time `ppBefore`, `ppAfter`, `accuracyBefore`, and `accuracyAfter` to the osu! client, displaying animated progression on the post-match ranking screen.
- **Database Evolution & Automatic Startup Recalculation**:
  - Added `pp` and `accuracy` columns to `scores` table.
  - Added `stars` and `max_combo` columns to `beatmaps` table.
  - Added `recalculate_all_scores_and_stats` to retroactively update existing database scores and player stats on startup.
- **Web & In-Game User Experience**:
  - Added **Accuracy** and **PP** columns to the Recent Plays section on user profile pages.
  - Updated bot commands `!recent` and `!stats` to output live PP and Accuracy values.

### Fixed
- **Bancho Zero-Stats Bug (`CHO_USER_STATS`)**: Fixed an issue where the server broadcasted `accuracy = 0.00%` and `pp = 0` to client user panels because stats were never calculated upon score submission.
- **Leaderboard Rank Calculation**: Fixed `get_user_rank` to correctly order players by `PP DESC` followed by `ranked_score DESC` tie-breaking.

---

## [0.3.0] - 2026-09-11

### Added
- **Chat Persistence System (`src/db/chat.rs`)**:
  - SQLite database backing for public and private chat messages (`chat.db`).
  - Automatically fetches and replays the 25 most recent messages upon user login or channel join (`#osu`, `#announce`, `#lobby`).
  - Added in-game bot command `!history [n]` allowing users to retrieve older channel conversations.
- **Direct Messaging Security Advisory**:
  - AyanomiBot automatically dispatches a security warning upon starting private conversations:
    *"không được gửi mật khẩu cho bất kì ai, admin/staff/bot sẽ không bao giờ hỏi bạn về mật khẩu"*.
- **Role-Based Admin Authentication**:
  - Retired the legacy `admin_key` configuration parameter.
  - Access to the Administration Panel (`/admin`) is now strictly gated to authenticated accounts possessing the **[AM]** (Administrator) badge.
- **Public Changelog Page**:
  - Added `/changelog` route and template displaying server development history.

---

## [0.2.0] - 2026-09-10

### Added
- **Multiplayer Match Coordination**:
  - Complete Bancho multiplayer match lifecycle: host management, slot toggling, mods sync, password locks, and match history tracking.
- **Web Frontend Modernization**:
  - User profiles with custom avatar uploading, cover banner customization, markdown bio editor, and international country flags.
  - Public leaderboards with mode switching and filter controls.
- **Beatmap Mirror & Direct Integration**:
  - Integrated Catboy API fallback mirror for beatmap metadata resolution and fast in-game osu!direct downloads.
- **Security & DDoS Protection**:
  - Token bucket IP rate limiting, datacenter VPN IP filtering, and client hardware fingerprinting.

---

## [0.1.0] - 2026-09-09

### Added
- Initial project architecture: asynchronous osu! private server written in Rust with Tokio, Axum, and SQLx.
- Bancho binary protocol v19 support with ULEB128 serialization.
- Modular binary layout: Monolith or independent Bancho, Web, and Gateway services.
- Linux ARM64 (Termux) and Windows cross-platform compatibility.