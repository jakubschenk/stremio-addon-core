use crate::config::UserConfig;
use axum::http::HeaderMap;
use serde::Deserialize;
use std::fmt;
use subtle::ConstantTimeEq;
use thiserror::Error;

#[derive(Clone)]
pub struct AuthConfig {
    key: Option<String>,
    required: bool,
}

impl AuthConfig {
    pub fn required(key: impl Into<String>) -> Self {
        Self {
            key: Some(key.into()),
            required: true,
        }
    }

    pub fn disabled() -> Self {
        Self {
            key: None,
            required: false,
        }
    }

    pub fn is_required(&self) -> bool {
        self.required
    }

    pub fn validate(
        &self,
        user_config: Option<&UserConfig>,
        query: Option<&AuthQuery>,
        headers: &HeaderMap,
        path_key: Option<&str>,
    ) -> Result<(), AuthError> {
        if !self.required {
            return Ok(());
        }

        let expected = self.key.as_deref().ok_or(AuthError::Misconfigured)?;
        let candidate = auth_from_config(user_config)
            .or(path_key)
            .or_else(|| auth_from_query(query))
            .or_else(|| auth_from_bearer(headers))
            .or_else(|| auth_from_header(headers));

        match candidate {
            Some(value) if constant_time_eq(value.as_bytes(), expected.as_bytes()) => Ok(()),
            Some(_) => Err(AuthError::Invalid),
            None => Err(AuthError::Missing),
        }
    }
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field("key", &self.key.as_ref().map(|_| "<redacted>"))
            .field("required", &self.required)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthQuery {
    pub auth_key: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub sig: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("addon auth is not configured")]
    Misconfigured,
    #[error("missing addon auth key")]
    Missing,
    #[error("invalid addon auth key")]
    Invalid,
}

fn auth_from_config(config: Option<&UserConfig>) -> Option<&str> {
    config.and_then(|cfg| cfg.auth_key.as_deref())
}

fn auth_from_query(query: Option<&AuthQuery>) -> Option<&str> {
    query.and_then(|q| q.auth_key.as_deref().or(q.key.as_deref()))
}

fn auth_from_bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ")
}

fn auth_from_header(headers: &HeaderMap) -> Option<&str> {
    headers.get("x-addon-auth")?.to_str().ok()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.ct_eq(right).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    #[test]
    fn accepts_config_auth() {
        let auth = AuthConfig::required("secret");
        let cfg = UserConfig {
            auth_key: Some("secret".to_string()),
            ..UserConfig::default()
        };

        assert!(auth
            .validate(Some(&cfg), None, &HeaderMap::new(), None)
            .is_ok());
    }

    #[test]
    fn accepts_query_auth() {
        let auth = AuthConfig::required("secret");
        let query = AuthQuery {
            auth_key: Some("secret".to_string()),
            key: None,
            sig: None,
        };

        assert!(auth
            .validate(None, Some(&query), &HeaderMap::new(), None)
            .is_ok());
    }

    #[test]
    fn accepts_bearer_auth() {
        let auth = AuthConfig::required("secret");
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret"),
        );

        assert!(auth.validate(None, None, &headers, None).is_ok());
    }

    #[test]
    fn rejects_missing_auth() {
        let auth = AuthConfig::required("secret");
        assert_eq!(
            auth.validate(None, None, &HeaderMap::new(), None)
                .unwrap_err(),
            AuthError::Missing
        );
    }

    #[test]
    fn rejects_invalid_auth() {
        let auth = AuthConfig::required("secret");
        let query = AuthQuery {
            auth_key: Some("wrong".to_string()),
            key: None,
            sig: None,
        };

        assert_eq!(
            auth.validate(None, Some(&query), &HeaderMap::new(), None)
                .unwrap_err(),
            AuthError::Invalid
        );
    }

    #[test]
    fn accepts_path_key_auth() {
        let auth = AuthConfig::required("secret");
        assert!(auth
            .validate(None, None, &HeaderMap::new(), Some("secret"))
            .is_ok());
    }
}
