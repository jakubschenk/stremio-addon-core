use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BingeGroupInput {
    pub provider: String,
    pub tier: String,
    pub content_id: String,
    pub season: Option<u32>,
    pub quality: Option<String>,
    pub language: Option<String>,
    pub source: Option<String>,
    pub codec: Option<String>,
    pub release_group: Option<String>,
}

pub fn binge_group_id(input: &BingeGroupInput) -> String {
    let tier = normalize_field(&input.tier);
    let payload = serialize_binge_payload(input);
    let mut hasher = Sha256::new();
    hasher.update(payload.as_bytes());
    let hash = URL_SAFE_NO_PAD.encode(hasher.finalize());

    format!("stremio-core:bg:v1:{tier}:{hash}")
}

fn serialize_binge_payload(input: &BingeGroupInput) -> String {
    let fields = [
        ("provider", normalize_field(&input.provider)),
        ("content_id", normalize_field(&input.content_id)),
        ("season", input.season.unwrap_or_default().to_string()),
        ("quality", normalize_field_opt(&input.quality)),
        ("language", normalize_field_opt(&input.language)),
        ("source", normalize_field_opt(&input.source)),
        ("codec", normalize_field_opt(&input.codec)),
        ("release_group", normalize_field_opt(&input.release_group)),
    ];
    let mut payload = String::new();
    for (key, value) in fields {
        payload.push_str(key);
        payload.push('=');
        payload.push_str(&value.len().to_string());
        payload.push(':');
        payload.push_str(&value);
        payload.push(';');
    }
    payload
}

fn normalize_field(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_field_opt(value: &Option<String>) -> String {
    value
        .as_deref()
        .map_or_else(|| "na".to_string(), normalize_field)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binge_group_is_stable_for_same_input() {
        let first = BingeGroupInput {
            provider: "test-provider".to_string(),
            tier: "movie".to_string(),
            content_id: "tt123".to_string(),
            season: Some(1),
            quality: Some("1080p".to_string()),
            language: Some("en".to_string()),
            source: Some("web-dl".to_string()),
            codec: Some("h265".to_string()),
            release_group: Some("example-rg".to_string()),
        };

        let left = binge_group_id(&first);
        let right = binge_group_id(&first);
        assert_eq!(left, right);
        assert_eq!(left.split(':').nth(3), Some("movie"));
        assert!(left.contains("stremio-core:bg:v1:movie:"));
    }

    #[test]
    fn binge_group_does_not_leak_raw_values() {
        let input = BingeGroupInput {
            provider: "provider-X".to_string(),
            tier: "series".to_string(),
            content_id: "some-id-999".to_string(),
            season: Some(2),
            quality: Some("720p".to_string()),
            language: Some("CZ".to_string()),
            source: Some("url/segment".to_string()),
            codec: Some("h264".to_string()),
            release_group: Some("RG+token".to_string()),
        };

        let id = binge_group_id(&input);
        assert!(!id.contains("provider-X"));
        assert!(!id.contains("some-id-999"));
        assert!(!id.contains("RG+token"));
        assert!(!id.contains("url/segment"));
    }

    #[test]
    fn binge_group_changes_with_provider_fields() {
        let base = BingeGroupInput {
            provider: "provider-a".to_string(),
            tier: "series".to_string(),
            content_id: "tt123".to_string(),
            season: Some(1),
            quality: Some("1080p".to_string()),
            ..BingeGroupInput::default()
        };
        let mut changed = base.clone();
        changed.quality = Some("720p".to_string());
        assert_ne!(binge_group_id(&base), binge_group_id(&changed));

        changed.quality = None;
        assert_ne!(binge_group_id(&base), binge_group_id(&changed));
    }

    #[test]
    fn binge_group_payload_is_not_delimiter_ambiguous() {
        let left = BingeGroupInput {
            provider: "provider|content_id=2:x".to_string(),
            tier: "series".to_string(),
            content_id: "tt123".to_string(),
            season: Some(1),
            quality: Some("1080p".to_string()),
            ..BingeGroupInput::default()
        };
        let mut right = left.clone();
        right.provider = "provider".to_string();
        right.content_id = "content_id=2:x|tt123".to_string();

        assert_ne!(binge_group_id(&left), binge_group_id(&right));
    }
}
