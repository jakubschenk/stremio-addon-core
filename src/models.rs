use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub resources: Vec<ResourceSpec>,
    pub types: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub catalogs: Vec<CatalogDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub id_prefixes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior_hints: Option<BehaviorHints>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config: Vec<ConfigField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_email: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ResourceSpec {
    Name(String),
    Object(Resource),
}

impl From<Resource> for ResourceSpec {
    fn from(value: Resource) -> Self {
        Self::Object(value)
    }
}

impl From<&str> for ResourceSpec {
    fn from(value: &str) -> Self {
        Self::Name(value.to_string())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub id_prefixes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDef {
    pub id: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<CatalogExtra>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogExtra {
    pub name: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_required: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BehaviorHints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configurable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration_required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p2p: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adult: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigField {
    pub key: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamResponse {
    pub streams: Vec<Stream>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ident: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior_hints: Option<StreamBehaviorHints>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamBehaviorHints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country_whitelist: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binge_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitlesResponse {
    pub subtitles: Vec<Subtitle>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Subtitle {
    pub id: String,
    pub url: String,
    pub lang: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogResponse {
    pub metas: Vec<MetaPreview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_max_age: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MetaResponse {
    pub meta: Meta,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MetaPreview {
    pub id: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(rename = "type", default, skip_serializing_if = "String::is_empty")]
    pub r#type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default)]
pub struct CatalogExtraArgs {
    pub values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct SubtitlesExtraArgs {
    pub values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamRequest {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl CatalogExtraArgs {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
}

impl SubtitlesExtraArgs {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_response_uses_stremio_cache_max_age_name() {
        let response = CatalogResponse {
            metas: vec![],
            cache_max_age: Some(3600),
        };

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({ "metas": [], "cacheMaxAge": 3600 })
        );
    }

    #[test]
    fn stream_transport_fields_are_optional() {
        let stream = Stream {
            name: Some("Provider".to_string()),
            external_url: Some("https://example.com".to_string()),
            ..Stream::default()
        };

        assert_eq!(
            serde_json::to_value(stream).unwrap(),
            json!({ "name": "Provider", "externalUrl": "https://example.com" })
        );
    }

    #[test]
    fn subtitles_response_matches_stremio_shape() {
        let response = SubtitlesResponse {
            subtitles: vec![Subtitle {
                id: "sub-1".to_string(),
                url: "https://example.com/sub.vtt".to_string(),
                lang: "cze".to_string(),
                label: Some("Czech".to_string()),
                extra: Map::new(),
            }],
        };

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "subtitles": [{
                    "id": "sub-1",
                    "url": "https://example.com/sub.vtt",
                    "lang": "cze",
                    "label": "Czech"
                }]
            })
        );
    }

    #[test]
    fn missing_meta_matches_empty_object_shape() {
        assert_eq!(
            serde_json::to_value(MetaResponse::default()).unwrap(),
            json!({ "meta": {} })
        );
    }
}
