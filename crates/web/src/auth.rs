//! One password (argon2), a session cookie.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng};
use argon2::Argon2;

pub fn hash(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default().hash_password(password.as_bytes(), &salt).map_err(|e| anyhow::anyhow!("{e}"))?.to_string())
}

pub fn verify(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}

/// A new session token (sent once, as the cookie).
pub fn new_token() -> String {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// What's kept of a token: its sha256.
pub fn token_hash(t: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(t.as_bytes()).iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_and_tokens() {
        let h = hash("correct horse").unwrap();
        assert!(verify("correct horse", &h) && !verify("wrong", &h) && !verify("x", "not a hash"));
        let (a, b) = (new_token(), new_token());
        assert!(a.len() == 64 && a != b);
        assert_eq!(token_hash(&a), token_hash(&a));
        assert_ne!(token_hash(&a), a);
    }
}
