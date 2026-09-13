use base64::prelude::{Engine as _, BASE64_URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

const SESSION_VERSION: &str = "v1";
pub const SESSION_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
const SESSION_CLOCK_SKEW_SECONDS: u64 = 300;
type HmacSha256 = Hmac<Sha256>;

pub fn internal_auth_token(secret: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any size");
    mac.update(b"ayanomi-internal-api-v1");
    BASE64_URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

pub fn verify_internal_auth_token(secret: &str, token: &str) -> bool {
    let Ok(signature) = BASE64_URL_SAFE_NO_PAD.decode(token) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(b"ayanomi-internal-api-v1");
    mac.verify_slice(&signature).is_ok()
}

pub fn privacy_fingerprint(secret: &str, namespace: &str, value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any size");
    mac.update(namespace.as_bytes());
    mac.update(b"\0");
    mac.update(value.as_bytes());
    BASE64_URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

/// Generates lowercase MD5 hex string for given input
pub fn md5_hex(input: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)
}

/// Hashes password or client md5 hash with bcrypt
pub fn hash_password(password: &str) -> Result<String, bcrypt::BcryptError> {
    bcrypt::hash(password, bcrypt::DEFAULT_COST)
}

/// Verifies whether a candidate plaintext/md5 matches a stored bcrypt hash
pub fn verify_password(password: &str, hash: &str) -> bool {
    bcrypt::verify(password, hash).unwrap_or(false)
}

/// Signs a user session cookie token
pub fn sign_session(user_id: i32, password_hash: &str, secret: &str) -> String {
    let issued_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let payload = format!("{}:{}:{}:{}", SESSION_VERSION, user_id, issued_at, password_hash);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any size");
    mac.update(payload.as_bytes());
    let signature = BASE64_URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{}.{}.{}.{}", SESSION_VERSION, user_id, issued_at, signature)
}

pub fn session_user_id(cookie: &str) -> Option<i32> {
    let mut parts = cookie.split('.');
    if parts.next()? != SESSION_VERSION {
        return None;
    }
    parts.next()?.parse::<i32>().ok()
}

pub fn session_expires_at(cookie: &str) -> Option<i64> {
    let mut parts = cookie.split('.');
    if parts.next()? != SESSION_VERSION {
        return None;
    }
    parts.next()?.parse::<i32>().ok()?;
    let issued_at = parts.next()?.parse::<u64>().ok()?;
    parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    issued_at
        .checked_add(SESSION_TTL_SECONDS)
        .and_then(|value| i64::try_from(value).ok())
}

/// Verifies a signed user session cookie token
pub fn verify_session(cookie: &str, password_hash: &str, secret: &str) -> Option<i32> {
    let user_id = session_user_id(cookie)?;
    let mut parts = cookie.split('.');
    parts.next()?;
    parts.next()?;
    let issued_at = parts.next()?.parse::<u64>().ok()?;
    let signature = BASE64_URL_SAFE_NO_PAD.decode(parts.next()?).ok()?;
    if parts.next().is_some() {
        return None;
    }

    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if issued_at > now.saturating_add(SESSION_CLOCK_SKEW_SECONDS)
        || now.saturating_sub(issued_at) > SESSION_TTL_SECONDS
    {
        return None;
    }

    let payload = format!("{}:{}:{}:{}", SESSION_VERSION, user_id, issued_at, password_hash);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(payload.as_bytes());
    mac.verify_slice(&signature).ok()?;
    Some(user_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_tokens_are_authenticated_and_bound_to_the_password_hash() {
        let secret = "a-test-secret-that-is-at-least-32-bytes";
        let token = sign_session(42, "password-hash", secret);
        assert_eq!(verify_session(&token, "password-hash", secret), Some(42));
        assert_eq!(verify_session(&token, "different-hash", secret), None);
        assert_eq!(verify_session(&token, "password-hash", "wrong-secret"), None);
    }

    #[test]
    fn legacy_md5_session_tokens_are_rejected() {
        assert_eq!(verify_session("42.deadbeef", "hash", "secret"), None);
    }

    #[test]
    fn arbitrary_plaintext_and_raw_md5_rows_are_not_accepted() {
        assert!(!verify_password("plaintext", "plaintext"));
        assert!(!verify_password(
            "5f4dcc3b5aa765d61d8327deb882cf99",
            "5f4dcc3b5aa765d61d8327deb882cf99"
        ));
        let hashed = hash_password("5f4dcc3b5aa765d61d8327deb882cf99").unwrap();
        assert!(verify_password("5f4dcc3b5aa765d61d8327deb882cf99", &hashed));
    }

    #[test]
    fn internal_api_tokens_are_verified_without_plaintext_comparison() {
        let secret = "a-test-secret-that-is-at-least-32-bytes";
        let token = internal_auth_token(secret);
        assert!(verify_internal_auth_token(secret, &token));
        assert!(!verify_internal_auth_token(secret, "invalid"));
        assert!(!verify_internal_auth_token("different-secret", &token));
    }
}
