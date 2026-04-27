use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_search: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config path is not valid utf-8")]
    InvalidUtf8,
    #[error("config path is not valid json: {0}")]
    InvalidJson(serde_json::Error),
}

pub fn decode_config_segment(segment: &str) -> Result<UserConfig, ConfigError> {
    let decoded = percent_decode_str(segment)
        .decode_utf8()
        .map_err(|_| ConfigError::InvalidUtf8)?;
    serde_json::from_str(&decoded).map_err(ConfigError::InvalidJson)
}

pub fn strip_json_suffix(value: &str) -> &str {
    value.strip_suffix(".json").unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_encoded_config() {
        let cfg = decode_config_segment("%7B%22authKey%22%3A%22secret%22%7D").unwrap();
        assert_eq!(cfg.auth_key.as_deref(), Some("secret"));
    }

    #[test]
    fn preserves_unknown_fields() {
        let cfg =
            decode_config_segment("%7B%22authKey%22%3A%22secret%22%2C%22x%22%3A1%7D").unwrap();
        assert_eq!(cfg.extra.get("x").and_then(Value::as_i64), Some(1));
    }
}
