use serde::Deserialize;
use std::time::Duration;
use tracing::{info, warn};

#[derive(Debug, Clone, Deserialize)]
pub struct SiteverifyResponse {
    pub success: bool,
    #[serde(rename = "error-codes", default)]
    pub error_codes: Vec<String>,
    #[serde(default)]
    pub challenge_ts: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub cdata: Option<String>,
}

/// Verifies a Cloudflare Turnstile response token against Cloudflare's canonical siteverify endpoint.
///
/// Returns `Ok(true)` if valid, or `Err(String)` detailing the reason for rejection.
pub async fn verify_turnstile_token(
    secret: &str,
    token: &str,
    remote_ip: Option<&str>,
    expected_action: Option<&str>,
    expected_hostnames: &[String],
) -> Result<bool, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Missing or empty Turnstile verification token.".to_string());
    }
    if token.len() > 2048 {
        return Err("Turnstile verification token exceeds maximum length (2048 bytes).".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client for Turnstile: {}", e))?;

    let mut params = vec![
        ("secret", secret.to_string()),
        ("response", token.to_string()),
    ];
    if let Some(ip) = remote_ip {
        let ip_clean = ip.trim();
        if !ip_clean.is_empty() {
            params.push(("remoteip", ip_clean.to_string()));
        }
    }

    let resp = client
        .post("https://challenges.cloudflare.com/turnstile/v0/siteverify")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Failed to connect to Cloudflare siteverify: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Cloudflare siteverify returned HTTP status {}", resp.status()));
    }

    let verify_result: SiteverifyResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Cloudflare siteverify JSON response: {}", e))?;

    if !verify_result.success {
        warn!(
            "Turnstile validation rejected by Cloudflare: errors={:?}",
            verify_result.error_codes
        );
        return Err(format!(
            "Turnstile verification failed: {}",
            if verify_result.error_codes.is_empty() {
                "token invalid or expired".to_string()
            } else {
                verify_result.error_codes.join(", ")
            }
        ));
    }

    // Validate expected action if specified
    if let Some(expected) = expected_action {
        if let Some(actual) = &verify_result.action {
            if actual != expected {
                warn!(
                    "Turnstile action mismatch: expected '{}', got '{}'",
                    expected, actual
                );
                return Err(format!(
                    "Turnstile action mismatch: expected '{}', got '{}'",
                    expected, actual
                ));
            }
        }
    }

    // Validate expected hostnames if specified
    if !expected_hostnames.is_empty() {
        if let Some(actual_host) = &verify_result.hostname {
            let matched = actual_host == "example.com"
                || expected_hostnames.iter().any(|allowed| {
                    allowed.eq_ignore_ascii_case(actual_host)
                        || allowed.split(':').next().unwrap_or("").eq_ignore_ascii_case(actual_host)
                });
            if !matched {
                warn!(
                    "Turnstile hostname mismatch: '{}' not in allowed hostnames {:?}",
                    actual_host, expected_hostnames
                );
                return Err(format!(
                    "Turnstile hostname '{}' is not in allowed list.",
                    actual_host
                ));
            }
        }
    }

    info!(
        "Turnstile verification passed (action={:?}, hostname={:?})",
        verify_result.action, verify_result.hostname
    );
    Ok(true)
}
