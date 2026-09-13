# Changelog

All notable changes to the **AyanomiBancho** osu! server project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Early Preview 2026/09/14 - v0.4.2] - 2026-09-14

### Added
- **Cloudflare Turnstile Bot Protection (`src/server/frontend.rs`, `static/js/login.js`)**:
  - Enforced Turnstile challenge verification on web authentication and account registration (`/api/login`).
  - Client-side validation in `login.js` requires challenge completion before submitting form, showing interactive error messages rather than unhandled exceptions.
  - Server-side token validation via Cloudflare `siteverify` API with flexible hostname resolution delegating domain restrictions to Cloudflare Dashboard.
- **GitHub-Flavored Markdown (GFM) & Safe HTML Profile Engine (`src/server/frontend.rs`, `static/css/style.css`)**:
  - Completely overhauled `render_bio_markdown` to preserve raw HTML markup instead of escaping tags to plain text.
  - Configured an expansive `ammonia` whitelist supporting `h1`-`h6`, `div`, `span`, `p`, `img`, `b`, `i`, `strong`, `em`, `del`, `s`, `sub`, `sup`, `details`, `summary`, `table`, `thead`, `tbody`, `tr`, `th`, `td`, `hr`, `br`, as well as layout alignment attributes (`align="center|left|right"`), `class`, `id`, `title`, `width`, and `height`.
  - Maintained strict XSS protection by stripping `<script>`, `<iframe>`, `<style>`, `javascript:` protocols, and inline DOM event attributes (`onload`, `onerror`, `onclick`).
  - Added comprehensive GitHub-style CSS typography to `static/css/style.css` including subtle header dividers, responsive tables, blockquotes, and custom alignment rules.
- **Dynamic Seasonal Backgrounds Architecture (`src/server/backgrounds.rs`)**:
  - Rewrote `/web/osu-getseasonal.php` and `/api/v2/seasonal-backgrounds` to dynamically scan all uploaded image formats (`.png`, `.jpg`, `.jpeg`, `.webp`) in `data/backgrounds/`.
  - Eliminated hardcoded file references and 404 dead links, enabling the osu! client to rotate through all uploaded backgrounds on the main menu.
- **Fail2Ban Security Integration & Phone Daemon Suite (`scripts/phone/`, `data/jail.local`)**:
  - Added automated background daemon management scripts for Android/Termux: `run_server.sh`, `run_cf.sh`, `run_fail2ban.sh`, and `setup_tmux.sh`.
  - Configured Fail2Ban jails (`jail.local`) with custom Nginx access log filters (`nginx-auth-filter.conf`, `nginx-botsearch-filter.conf`, `nginx-deny.conf`) to mitigate automated scanners, dictionary attacks, and aggressive crawlers.
- **Session Revocation Blacklist (`src/db/mod.rs`, `src/server/frontend.rs`)**:
  - Introduced the `revoked_sessions` SQLite database table.
  - Web logout handlers and authenticated API middleware now record and verify invalidated session tokens against the blacklist to prevent session reuse.
- **Hardware ID Pseudonymization (`src/utils/crypto.rs`, `src/db/users.rs`)**:
  - Added HMAC-SHA256 privacy hashing for player hardware IDs (`privacy_fingerprint`), shielding players' raw hardware details from metadata disclosure.

### Fixed
- **Cloudflare Tunnel 502 Bad Gateway Drops (`scripts/phone/run_cf.sh`)**:
  - Diagnosed and resolved 502 connection drops caused by HTTP/2 over TCP multiplexing stream resets on Android/Termux.
  - Upgraded Cloudflare Tunnel protocol to QUIC over UDP (`--protocol auto`), pinned to `--edge-ip-version 4`, enabled `termux-wake-lock`, raised `ulimit -n 4096`, and wrapped processes in auto-recovery watchdog loops.
- **CSRF Middleware & Origin Whitelisting (`src/server/ratelimit.rs`)**:
  - Enhanced `csrf_guard_middleware` with flexible origin verification permitting requests matching the configured domain (`hatsuneakiko.io.vn`), `www.` subdomains, incoming `Host` headers, and local private LAN IP addresses.
  - Replaced plain-text 403 error strings with structured JSON `ApiResponse`, preventing client-side `SyntaxError` crashes during login.
- **Password Verification Hardening & Automated Legacy Migration (`src/utils/crypto.rs`, `src/db/users.rs`)**:
  - Eliminated raw plaintext MD5 password matching in favor of industry-standard bcrypt hashing.
  - Added automatic startup migration (`migrate_legacy_md5_passwords`) to upgrade existing MD5 rows to bcrypt without user disruption.
- **Score Submission Authentication & Anti-Replay Guard (`src/server/web.rs`)**:
  - Enforced bcrypt credential checks on `submit_score` requests.
  - Removed insecure single-session fallback handling and introduced score checksum deduplication to block replay attacks.
- **Sensitive User Metadata Protection (`src/db/users.rs`)**:
  - Added `#[serde(skip_serializing)]` annotations to internal security fields on `User` structs, preventing password hashes, session tokens, and security flags from being serialized into public API responses.
- **Host Header Poisoning Mitigation (`src/server/web.rs`)**:
  - Sanitized untrusted proxy headers (`x-forwarded-host`, `x-forwarded-proto`) at the gateway layer, enforcing canonical domain resolution via `config.server.domain`.
- **Relaxed Rate Limiter Thresholds (`config.toml`, `src/config.rs`)**:
  - Raised default RPM limits across all tiers (General: 3600 RPM, Bancho: 3600 RPM, Direct: 1800 RPM, Sensitive: 180 RPM) to eliminate false-positive 429 rate limit triggers during active gameplay and client sync.
- **Git Security & Secret Sanitization**:
  - Cleaned all sample configuration files (`config.toml`, `config.phone.toml`, `scripts/phone/run_cf.sh`) to ensure session secrets, Cloudflare Turnstile keys, and tunnel tokens are never committed to version control.
  - Removed the legacy `data/https_proxy.py` script.

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