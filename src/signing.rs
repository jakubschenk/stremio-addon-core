use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use time::{Duration, OffsetDateTime};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedPlayback {
    pub ident: String,
    pub expires_at: i64,
}

impl SignedPlayback {
    pub fn new(ident: impl Into<String>, ttl_seconds: i64) -> Self {
        Self {
            ident: ident.into(),
            expires_at: (OffsetDateTime::now_utc() + Duration::seconds(ttl_seconds))
                .unix_timestamp(),
        }
    }

    pub fn sign(&self, key: &[u8]) -> Result<String, SigningError> {
        let payload = serde_json::to_vec(self).map_err(SigningError::Serialize)?;
        let payload = URL_SAFE_NO_PAD.encode(payload);
        let signature = sign_bytes(key, payload.as_bytes())?;
        Ok(format!("{payload}.{signature}"))
    }

    pub fn verify(token: &str, key: &[u8]) -> Result<Self, SigningError> {
        let (payload, signature) = token.split_once('.').ok_or(SigningError::MalformedToken)?;

        verify_signature(key, payload.as_bytes(), signature)?;

        let payload_bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| SigningError::MalformedToken)?;
        let decoded: SignedPlayback =
            serde_json::from_slice(&payload_bytes).map_err(|_| SigningError::MalformedToken)?;

        if decoded.expires_at <= OffsetDateTime::now_utc().unix_timestamp() {
            return Err(SigningError::Expired);
        }

        Ok(decoded)
    }
}

#[derive(Debug, Error)]
pub enum SigningError {
    #[error("failed to serialize signed playback payload: {0}")]
    Serialize(serde_json::Error),
    #[error("invalid signing key")]
    InvalidKey,
    #[error("malformed signed playback token")]
    MalformedToken,
    #[error("invalid signed playback signature")]
    InvalidSignature,
    #[error("signed playback token expired")]
    Expired,
}

fn sign_bytes(key: &[u8], bytes: &[u8]) -> Result<String, SigningError> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| SigningError::InvalidKey)?;
    mac.update(bytes);
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn verify_signature(key: &[u8], bytes: &[u8], signature: &str) -> Result<(), SigningError> {
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| SigningError::MalformedToken)?;
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| SigningError::InvalidKey)?;
    mac.update(bytes);
    mac.verify_slice(&signature)
        .map_err(|_| SigningError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_payload_round_trips() {
        let payload = SignedPlayback::new("abc123", 60);
        let token = payload.sign(b"secret").unwrap();
        let verified = SignedPlayback::verify(&token, b"secret").unwrap();

        assert_eq!(verified.ident, "abc123");
    }

    #[test]
    fn rejects_wrong_key() {
        let payload = SignedPlayback::new("abc123", 60);
        let token = payload.sign(b"secret").unwrap();
        assert!(matches!(
            SignedPlayback::verify(&token, b"wrong").unwrap_err(),
            SigningError::InvalidSignature
        ));
    }

    #[test]
    fn rejects_expired_payload() {
        let payload = SignedPlayback {
            ident: "abc123".to_string(),
            expires_at: OffsetDateTime::now_utc().unix_timestamp() - 1,
        };
        let token = payload.sign(b"secret").unwrap();
        assert!(matches!(
            SignedPlayback::verify(&token, b"secret").unwrap_err(),
            SigningError::Expired
        ));
    }

    #[test]
    fn token_does_not_include_raw_ident_text() {
        let payload = SignedPlayback::new("webshare-token-like-value", 60);
        let token = payload.sign(b"secret").unwrap();
        assert!(!token.contains("webshare-token-like-value"));
    }
}
