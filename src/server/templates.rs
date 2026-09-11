use std::fs;
use std::path::Path;

// -------------------------------------------------------------------------------------------------
// Embedded Fallback Templates (Compiled into binary so server runs even without disk templates)
// -------------------------------------------------------------------------------------------------

const FALLBACK_BASE: &str = include_str!("../../templates/base.html");
const FALLBACK_NAVBAR: &str = include_str!("../../templates/navbar.html");
const FALLBACK_FOOTER: &str = include_str!("../../templates/footer.html");
const FALLBACK_INDEX: &str = include_str!("../../templates/index.html");
const FALLBACK_LEADERBOARD: &str = include_str!("../../templates/leaderboard.html");
const FALLBACK_PROFILE: &str = include_str!("../../templates/profile.html");
const FALLBACK_LOGIN: &str = include_str!("../../templates/login.html");
const FALLBACK_ACCOUNT: &str = include_str!("../../templates/account.html");
const FALLBACK_ADMIN: &str = include_str!("../../templates/admin.html");
const FALLBACK_ADMIN_LOGIN: &str = include_str!("../../templates/admin_login.html");
const FALLBACK_CONNECT: &str = include_str!("../../templates/connect.html");
const FALLBACK_RULES: &str = include_str!("../../templates/rules.html");
const FALLBACK_CHANGELOG: &str = include_str!("../../templates/changelog.html");
const FALLBACK_STAFF: &str = include_str!("../../templates/staff.html");
const FALLBACK_MULTI: &str = include_str!("../../templates/multi.html");
const FALLBACK_404: &str = include_str!("../../templates/404.html");

/// Retrieves template content from disk if present (for zero-recompile live editing),
/// falling back to the compile-time embedded template.
pub fn get_template_source(name: &str) -> String {
    let disk_path = format!("templates/{}.html", name);
    if Path::new(&disk_path).is_file() {
        if let Ok(content) = fs::read_to_string(&disk_path) {
            return content;
        }
    }

    match name {
        "base" => FALLBACK_BASE.to_string(),
        "navbar" => FALLBACK_NAVBAR.to_string(),
        "footer" => FALLBACK_FOOTER.to_string(),
        "index" => FALLBACK_INDEX.to_string(),
        "leaderboard" => FALLBACK_LEADERBOARD.to_string(),
        "profile" => FALLBACK_PROFILE.to_string(),
        "login" => FALLBACK_LOGIN.to_string(),
        "account" => FALLBACK_ACCOUNT.to_string(),
        "admin" => FALLBACK_ADMIN.to_string(),
        "admin_login" => FALLBACK_ADMIN_LOGIN.to_string(),
        "connect" => FALLBACK_CONNECT.to_string(),
        "rules" => FALLBACK_RULES.to_string(),
        "changelog" => FALLBACK_CHANGELOG.to_string(),
        "staff" => FALLBACK_STAFF.to_string(),
        "multi" => FALLBACK_MULTI.to_string(),
        "404" => FALLBACK_404.to_string(),
        _ => String::new(),
    }
}

/// Renders a template by name, replacing all instances of `{{KEY}}` with value.
pub fn render_template(name: &str, vars: &[(&str, &str)]) -> String {
    let mut html = get_template_source(name);
    for (k, v) in vars {
        let placeholder = format!("{{{{{}}}}}", k);
        html = html.replace(&placeholder, v);
    }
    html
}

/// Helper to render a full page using `base.html` layout.
pub fn render_page(
    template_name: &str,
    title: &str,
    server_name: &str,
    navbar: &str,
    footer: &str,
    extra_head: &str,
    extra_js: &str,
    content_vars: &[(&str, &str)],
) -> String {
    let content = render_template(template_name, content_vars);

    let raw_domain = content_vars
        .iter()
        .find(|(k, _)| *k == "DOMAIN")
        .map(|(_, v)| *v)
        .unwrap_or("hatsuneakiko.io.vn");

    let base_url = if raw_domain.starts_with("http://") || raw_domain.starts_with("https://") {
        raw_domain.to_string()
    } else if raw_domain.contains("127.0.0.1") || raw_domain.contains("localhost") {
        format!("http://{}", raw_domain)
    } else {
        format!("https://{}", raw_domain)
    };

    let default_desc = format!(
        "A lightweight, high-performance osu! private server powered by Rust with custom PP calculation, live Bancho multiplayer, and global leaderboards."
    );

    let description = content_vars
        .iter()
        .find(|(k, _)| *k == "META_DESCRIPTION")
        .map(|(_, v)| *v)
        .unwrap_or(&default_desc);

    let default_image = format!("{}/static/logo.png", base_url.trim_end_matches('/'));
    let og_image = content_vars
        .iter()
        .find(|(k, _)| *k == "OG_IMAGE")
        .map(|(_, v)| *v)
        .unwrap_or(&default_image);

    let page_vars = [
        ("TITLE", title),
        ("SERVER_NAME", server_name),
        ("NAVBAR", navbar),
        ("CONTENT", &content),
        ("FOOTER", footer),
        ("EXTRA_HEAD", extra_head),
        ("EXTRA_JS", extra_js),
        ("BASE_URL", &base_url),
        ("META_DESCRIPTION", description),
        ("OG_IMAGE", og_image),
    ];
    render_template("base", &page_vars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_template_basic() {
        let rendered = render_template("rules", &[("SERVER_NAME", "TestBancho")]);
        assert!(rendered.contains("TestBancho"));
        assert!(!rendered.contains("{{SERVER_NAME}}"));
    }

    #[test]
    fn test_render_page_layout() {
        let page = render_page(
            "rules",
            "Rules & Info",
            "AyanomiBancho",
            "<nav>TEST_NAV</nav>",
            "<footer>TEST_FOOTER</footer>",
            "",
            "",
            &[("SERVER_NAME", "AyanomiBancho")],
        );

        assert!(page.contains("<title>Rules & Info - AyanomiBancho</title>"));
        assert!(page.contains("<nav>TEST_NAV</nav>"));
        assert!(page.contains("<footer>TEST_FOOTER</footer>"));
        assert!(page.contains("/static/css/style.css"));
        assert!(page.contains("/static/js/main.js"));
        assert!(page.contains(r##"<meta name="theme-color" content="#f472b6">"##));
        assert!(page.contains(r##"<meta property="og:site_name" content="AyanomiBancho">"##));
        assert!(page.contains(r##"<meta property="og:image" content="https://hatsuneakiko.io.vn/static/logo.png">"##));
        assert!(page.contains(r##"<meta name="twitter:card" content="summary_large_image">"##));
    }

    #[test]
    fn test_fallback_when_file_missing() {
        let fallback = get_template_source("rules");
        assert!(!fallback.is_empty());
        assert!(fallback.contains("{{SERVER_NAME}}"));
    }
}
