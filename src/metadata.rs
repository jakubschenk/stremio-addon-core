use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct MetadataConfig {
    pub cinemeta_url: String,
    pub tmdb_url: String,
    pub tmdb_api_key: Option<String>,
    pub timeout_seconds: u64,
}

impl Default for MetadataConfig {
    fn default() -> Self {
        Self {
            cinemeta_url: "https://v3-cinemeta.strem.io".to_string(),
            tmdb_url: "https://api.themoviedb.org/3".to_string(),
            tmdb_api_key: None,
            timeout_seconds: 10,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct TitleInfo {
    pub content_type: Option<String>,
    pub primary_title: Option<String>,
    pub title_cs: Option<String>,
    pub title_sk: Option<String>,
    pub title_en: Option<String>,
    pub original_title: Option<String>,
    pub year: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

impl TitleInfo {
    pub fn title_candidates(&self) -> Vec<String> {
        dedupe_non_empty([
            self.primary_title.clone(),
            self.title_cs.clone(),
            self.title_sk.clone(),
            self.title_en.clone(),
            self.original_title.clone(),
        ])
    }

    pub fn is_series(&self) -> bool {
        self.content_type.as_deref() == Some("series")
    }
}

#[derive(Clone)]
pub struct MetadataClient {
    http: Client,
    config: MetadataConfig,
}

impl MetadataClient {
    pub fn new(config: MetadataConfig) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { http, config }
    }

    pub fn with_client(http: Client, config: MetadataConfig) -> Self {
        Self { http, config }
    }

    pub async fn lookup_imdb(
        &self,
        content_type: &str,
        stremio_id: &str,
    ) -> Result<Option<TitleInfo>, MetadataError> {
        let parsed = ParsedStremioId::parse(stremio_id);
        let imdb_id = parsed.base_id.as_str();
        let mut info = if self.config.tmdb_api_key.is_some() {
            match self.lookup_tmdb_by_imdb(content_type, imdb_id).await? {
                Some(info) => Some(info),
                None => self.lookup_cinemeta(content_type, imdb_id).await?,
            }
        } else {
            self.lookup_cinemeta(content_type, imdb_id).await?
        };

        if let Some(info) = info.as_mut() {
            info.season = parsed.season;
            info.episode = parsed.episode;
        }

        Ok(info)
    }

    pub async fn lookup_tmdb_id(
        &self,
        content_type: &str,
        stremio_id: &str,
    ) -> Result<Option<TitleInfo>, MetadataError> {
        let parsed = ParsedStremioId::parse(stremio_id);
        let Some(_) = self.config.tmdb_api_key else {
            return Ok(None);
        };

        let mut info = match content_type {
            "movie" => self.lookup_tmdb_movie(&parsed.base_id).await?,
            "series" => self.lookup_tmdb_series(&parsed.base_id).await?,
            _ => None,
        };

        if let Some(info) = info.as_mut() {
            info.season = parsed.season;
            info.episode = parsed.episode;
        }

        Ok(info)
    }

    async fn lookup_cinemeta(
        &self,
        content_type: &str,
        imdb_id: &str,
    ) -> Result<Option<TitleInfo>, MetadataError> {
        let url = format!(
            "{}/meta/{}/{}.json",
            self.config.cinemeta_url.trim_end_matches('/'),
            content_type,
            imdb_id
        );
        let body = self
            .http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        let Some(meta) = body.get("meta") else {
            return Ok(None);
        };

        Ok(Some(TitleInfo {
            content_type: Some(content_type.to_string()),
            primary_title: string_value_at(meta, &["name"]),
            original_title: string_value_at(meta, &["originalName"])
                .or_else(|| string_value_at(meta, &["originalTitle"])),
            year: string_value_at(meta, &["year"]).or_else(|| release_year(meta)),
            ..TitleInfo::default()
        }))
    }

    async fn lookup_tmdb_by_imdb(
        &self,
        content_type: &str,
        imdb_id: &str,
    ) -> Result<Option<TitleInfo>, MetadataError> {
        let Some(api_key) = self.config.tmdb_api_key.as_deref() else {
            return Ok(None);
        };
        let path = format!(
            "{}/find/{imdb_id}",
            self.config.tmdb_url.trim_end_matches('/')
        );
        let kind_key = if content_type == "series" {
            "tv_results"
        } else {
            "movie_results"
        };
        let kind = if content_type == "series" {
            "tv"
        } else {
            "movie"
        };

        let cs = self
            .http
            .get(&path)
            .query(&[
                ("api_key", api_key),
                ("external_source", "imdb_id"),
                ("language", "cs"),
            ])
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        let sk = self
            .http
            .get(&path)
            .query(&[
                ("api_key", api_key),
                ("external_source", "imdb_id"),
                ("language", "sk"),
            ])
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;

        let Some(cs_item) = first_array_item(&cs, kind_key) else {
            return Ok(None);
        };
        let sk_item = first_array_item(&sk, kind_key);
        let original_language = string_value_at(cs_item, &["original_language"]);
        let en_item = if original_language
            .as_deref()
            .is_some_and(|lang| lang != "en")
        {
            let en = self
                .http
                .get(&path)
                .query(&[("api_key", api_key), ("external_source", "imdb_id")])
                .send()
                .await?
                .error_for_status()?
                .json::<Value>()
                .await?;
            first_array_item(&en, kind_key).cloned()
        } else {
            None
        };

        Ok(Some(title_info_from_tmdb_values(
            content_type,
            kind,
            cs_item,
            sk_item,
            en_item.as_ref(),
        )))
    }

    async fn lookup_tmdb_movie(&self, tmdb_id: &str) -> Result<Option<TitleInfo>, MetadataError> {
        self.lookup_tmdb_direct("movie", "movie", tmdb_id).await
    }

    async fn lookup_tmdb_series(&self, tmdb_id: &str) -> Result<Option<TitleInfo>, MetadataError> {
        self.lookup_tmdb_direct("series", "tv", tmdb_id).await
    }

    async fn lookup_tmdb_direct(
        &self,
        content_type: &str,
        kind: &str,
        tmdb_id: &str,
    ) -> Result<Option<TitleInfo>, MetadataError> {
        let Some(api_key) = self.config.tmdb_api_key.as_deref() else {
            return Ok(None);
        };
        let path = format!(
            "{}/{kind}/{tmdb_id}",
            self.config.tmdb_url.trim_end_matches('/')
        );
        let cs = self
            .http
            .get(&path)
            .query(&[("api_key", api_key), ("language", "cs")])
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        let sk = self
            .http
            .get(&path)
            .query(&[("api_key", api_key), ("language", "sk")])
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        let original_language = string_value_at(&cs, &["original_language"]);
        let en = if original_language
            .as_deref()
            .is_some_and(|lang| lang != "en")
        {
            Some(
                self.http
                    .get(&path)
                    .query(&[("api_key", api_key)])
                    .send()
                    .await?
                    .error_for_status()?
                    .json::<Value>()
                    .await?,
            )
        } else {
            None
        };

        Ok(Some(title_info_from_tmdb_values(
            content_type,
            kind,
            &cs,
            Some(&sk),
            en.as_ref(),
        )))
    }
}

#[derive(Debug, Error)]
pub enum MetadataError {
    #[error("metadata http error: {0}")]
    Http(#[from] reqwest::Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedStremioId {
    pub base_id: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

impl ParsedStremioId {
    pub fn parse(id: &str) -> Self {
        let mut parts = id.split(':');
        let base_id = parts.next().unwrap_or_default().to_string();
        let season = parts.next().and_then(|value| value.parse().ok());
        let episode = parts.next().and_then(|value| value.parse().ok());
        Self {
            base_id,
            season,
            episode,
        }
    }
}

fn title_info_from_tmdb_values(
    content_type: &str,
    kind: &str,
    primary: &Value,
    sk: Option<&Value>,
    en: Option<&Value>,
) -> TitleInfo {
    let title_key = if kind == "tv" { "name" } else { "title" };
    let original_key = if kind == "tv" {
        "original_name"
    } else {
        "original_title"
    };
    let date_key = if kind == "tv" {
        "first_air_date"
    } else {
        "release_date"
    };

    TitleInfo {
        content_type: Some(content_type.to_string()),
        primary_title: string_value_at(primary, &[title_key]),
        title_cs: string_value_at(primary, &[title_key]),
        title_sk: sk.and_then(|value| string_value_at(value, &[title_key])),
        title_en: en.and_then(|value| string_value_at(value, &[title_key])),
        original_title: string_value_at(primary, &[original_key])
            .or_else(|| en.and_then(|value| string_value_at(value, &[original_key]))),
        year: string_value_at(primary, &[date_key])
            .or_else(|| en.and_then(|value| string_value_at(value, &[date_key])))
            .and_then(|date| date.get(0..4).map(str::to_string)),
        ..TitleInfo::default()
    }
}

fn first_array_item<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key)?.as_array()?.first()
}

fn release_year(meta: &Value) -> Option<String> {
    string_value_at(meta, &["releaseInfo"])
        .and_then(|value| value.get(0..4).map(str::to_string))
        .or_else(|| {
            string_value_at(meta, &["released"])
                .and_then(|value| value.get(0..4).map(str::to_string))
        })
}

fn string_value_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for part in path {
        current = current.get(*part)?;
    }
    match current {
        Value::String(value) => Some(value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn dedupe_non_empty(values: impl IntoIterator<Item = Option<String>>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values.into_iter().flatten() {
        if !value.is_empty() && !out.contains(&value) {
            out.push(value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_stremio_episode_ids() {
        assert_eq!(
            ParsedStremioId::parse("tt1234567:2:8"),
            ParsedStremioId {
                base_id: "tt1234567".to_string(),
                season: Some(2),
                episode: Some(8),
            }
        );
    }

    #[test]
    fn tmdb_movie_title_info_matches_webshare_shape() {
        let cs = json!({
            "title": "Naprosti cizinci",
            "original_title": "Perfetti sconosciuti",
            "release_date": "2016-02-11"
        });
        let sk = json!({ "title": "Uplni cudzinci" });
        let en = json!({ "title": "Perfect Strangers" });

        assert_eq!(
            title_info_from_tmdb_values("movie", "movie", &cs, Some(&sk), Some(&en)),
            TitleInfo {
                content_type: Some("movie".to_string()),
                primary_title: Some("Naprosti cizinci".to_string()),
                title_cs: Some("Naprosti cizinci".to_string()),
                title_sk: Some("Uplni cudzinci".to_string()),
                title_en: Some("Perfect Strangers".to_string()),
                original_title: Some("Perfetti sconosciuti".to_string()),
                year: Some("2016".to_string()),
                season: None,
                episode: None,
            }
        );
    }

    #[test]
    fn title_candidates_are_deduped() {
        let info = TitleInfo {
            primary_title: Some("Alien".to_string()),
            title_cs: Some("Vetřelec".to_string()),
            original_title: Some("Alien".to_string()),
            title_en: Some("Alien Covenant".to_string()),
            ..TitleInfo::default()
        };

        assert_eq!(
            info.title_candidates(),
            vec!["Alien", "Vetřelec", "Alien Covenant"]
        );
    }
}
