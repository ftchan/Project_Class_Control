//! Passphrase hashing for the client access gate.
//!
//! The teacher console hashes a passphrase once and ships only the hash to
//! clients; the client verifies user input against it. We use Argon2id with
//! library defaults, which is appropriate for this low-value, LAN-only gate.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

/// Hash a passphrase into a PHC string (`$argon2id$...`).
pub fn hash_passphrase(passphrase: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(passphrase.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash failed: {e}"))?
        .to_string();
    Ok(hash)
}

/// Verify `attempt` against a stored PHC hash. Returns `Ok(false)` on mismatch,
/// `Err` only if the stored hash is malformed.
pub fn verify_passphrase(attempt: &str, stored_hash: &str) -> anyhow::Result<bool> {
    let parsed =
        PasswordHash::new(stored_hash).map_err(|e| anyhow::anyhow!("bad stored hash: {e}"))?;
    Ok(Argon2::default()
        .verify_password(attempt.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let h = hash_passphrase("lab-2016").unwrap();
        assert!(verify_passphrase("lab-2016", &h).unwrap());
        assert!(!verify_passphrase("wrong", &h).unwrap());
    }
}
