use md5::{Digest, Md5};

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
    // If hash in db is direct md5 (e.g. legacy), check direct equality
    if hash == password {
        return true;
    }
    bcrypt::verify(password, hash).unwrap_or(false)
}

/// Signs a user session cookie token
pub fn sign_session(user_id: i32, password_hash: &str, secret: &str) -> String {
    let token = md5_hex(&format!("{}:{}:{}", user_id, password_hash, secret));
    format!("{}.{}", user_id, token)
}

/// Verifies a signed user session cookie token
pub fn verify_session(cookie: &str, password_hash: &str, secret: &str) -> Option<i32> {
    let parts: Vec<&str> = cookie.split('.').collect();
    if parts.len() != 2 {
        return None;
    }
    let user_id = parts[0].parse::<i32>().ok()?;
    let expected = md5_hex(&format!("{}:{}:{}", user_id, password_hash, secret));
    if parts[1] == expected {
        Some(user_id)
    } else {
        None
    }
}
