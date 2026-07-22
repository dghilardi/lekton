//! Sealed quiz-answer-key tokens for ephemeral (non-persisted) lessons.
//!
//! Persisted lessons are graded by looking their answer key up in Mongo, so
//! the key never leaves the server. Ephemeral lessons have nothing in Mongo,
//! so to grade them server-side without exposing the answers we round-trip the
//! key through the client **sealed** with authenticated encryption: the client
//! holds an opaque token it can neither read nor forge, and hands it back at
//! grading time.
//!
//! The sealing key is random per process — ephemeral tokens are meant to live
//! only for the current browser session, so surviving a restart is a non-goal.

use base64::Engine;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// The answer key sealed inside an ephemeral quiz token.
#[derive(Debug, Serialize, Deserialize)]
pub struct QuizKey {
    /// Correct option index per question, in quiz order.
    pub correct: Vec<usize>,
    /// Explanation per question, in quiz order.
    pub explanations: Vec<String>,
}

/// Seals and opens [`QuizKey`]s with a process-random AEAD key.
pub struct QuizSealer {
    key: LessSafeKey,
    rng: SystemRandom,
}

impl QuizSealer {
    /// Create a sealer with a fresh random key.
    pub fn new() -> Self {
        let rng = SystemRandom::new();
        let mut key_bytes = [0u8; 32];
        rng.fill(&mut key_bytes)
            .expect("system RNG must produce a key");
        let unbound = UnboundKey::new(&CHACHA20_POLY1305, &key_bytes)
            .expect("32 bytes is a valid ChaCha20-Poly1305 key");
        Self {
            key: LessSafeKey::new(unbound),
            rng,
        }
    }

    /// Seal an answer key into an opaque, URL-safe token string.
    pub fn seal(&self, key: &QuizKey) -> Result<String, AppError> {
        let plaintext = serde_json::to_vec(key)
            .map_err(|e| AppError::Internal(format!("learn: could not encode quiz key: {e}")))?;

        let mut nonce_bytes = [0u8; NONCE_LEN];
        self.rng
            .fill(&mut nonce_bytes)
            .map_err(|_| AppError::Internal("learn: RNG failure sealing quiz token".into()))?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        let mut in_out = plaintext;
        self.key
            .seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| AppError::Internal("learn: failed to seal quiz token".into()))?;

        // token = nonce || ciphertext || tag, base64url.
        let mut sealed = Vec::with_capacity(NONCE_LEN + in_out.len());
        sealed.extend_from_slice(&nonce_bytes);
        sealed.extend_from_slice(&in_out);
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sealed))
    }

    /// Open a token back into its answer key. Fails on tampering, a wrong key,
    /// or a malformed token.
    pub fn open(&self, token: &str) -> Result<QuizKey, AppError> {
        let sealed = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(token.trim())
            .map_err(|_| AppError::BadRequest("learn: malformed quiz token".into()))?;
        if sealed.len() <= NONCE_LEN {
            return Err(AppError::BadRequest("learn: truncated quiz token".into()));
        }
        let (nonce_bytes, ciphertext) = sealed.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce_bytes)
            .map_err(|_| AppError::BadRequest("learn: malformed quiz token".into()))?;

        let mut in_out = ciphertext.to_vec();
        let plaintext = self
            .key
            .open_in_place(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| AppError::BadRequest("learn: invalid quiz token".into()))?;

        serde_json::from_slice(plaintext)
            .map_err(|e| AppError::Internal(format!("learn: could not decode quiz key: {e}")))
    }
}

impl Default for QuizSealer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> QuizKey {
        QuizKey {
            correct: vec![0, 2, 1],
            explanations: vec!["a".into(), "b".into(), "c".into()],
        }
    }

    #[test]
    fn seal_then_open_roundtrips() {
        let sealer = QuizSealer::new();
        let token = sealer.seal(&key()).unwrap();
        let opened = sealer.open(&token).unwrap();
        assert_eq!(opened.correct, vec![0, 2, 1]);
        assert_eq!(opened.explanations, vec!["a", "b", "c"]);
    }

    #[test]
    fn token_does_not_leak_the_answers_in_cleartext() {
        // A base64url token must not contain the plaintext key material.
        let sealer = QuizSealer::new();
        let token = sealer.seal(&key()).unwrap();
        assert!(!token.contains("correct"));
        assert!(!token.contains("explanations"));
    }

    #[test]
    fn tampered_token_is_rejected() {
        let sealer = QuizSealer::new();
        let mut token = sealer.seal(&key()).unwrap();
        // Flip the last character to corrupt the ciphertext/tag.
        let last = token.pop().unwrap();
        token.push(if last == 'A' { 'B' } else { 'A' });
        assert!(sealer.open(&token).is_err());
    }

    #[test]
    fn a_token_from_another_sealer_is_rejected() {
        let a = QuizSealer::new();
        let b = QuizSealer::new();
        let token = a.seal(&key()).unwrap();
        // Different process key → cannot open.
        assert!(b.open(&token).is_err());
    }

    #[test]
    fn garbage_token_is_rejected() {
        let sealer = QuizSealer::new();
        assert!(sealer.open("not-a-real-token").is_err());
    }
}
