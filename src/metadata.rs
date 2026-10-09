use futures_util::future::join;
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

pub const WIKIDATA_API_URL: &str = "https://www.wikidata.org/w/api.php";
const WIKIDATA_TIMEOUT: Duration = Duration::from_secs(3);
const WIKIDATA_USER_AGENT: &str = concat!(
    "stremio-addon-core/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/jakubschenk/stremio-addon-core)"
);

/// Localized labels of a Wikidata item, looked up by IMDb ID.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WikidataTitles {
    pub cs: Option<String>,
    pub sk: Option<String>,
    pub en: Option<String>,
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
    wikidata_url: Option<String>,
}

impl MetadataClient {
    pub fn new(config: MetadataConfig) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self::with_client(http, config)
    }

    pub fn with_client(http: Client, config: MetadataConfig) -> Self {
        Self {
            http,
            config,
            wikidata_url: Some(WIKIDATA_API_URL.to_string()),
        }
    }

    /// Overrides the Wikidata API used to fill Czech and Slovak titles when
    /// TMDB is unavailable. `None` disables the lookup.
    pub fn with_wikidata_url(mut self, wikidata_url: Option<String>) -> Self {
        self.wikidata_url = wikidata_url;
        self
    }

    pub async fn lookup_imdb(
        &self,
        content_type: &str,
        stremio_id: &str,
    ) -> Result<Option<TitleInfo>, MetadataError> {
        let parsed = ParsedStremioId::parse(stremio_id);
        let imdb_id = parsed.base_id.as_str();
        let tmdb_info = if self.config.tmdb_api_key.is_some() {
            self.lookup_tmdb_by_imdb(content_type, imdb_id).await?
        } else {
            None
        };
        let mut info = match tmdb_info {
            Some(info) => Some(info),
            None => {
                // Cinemeta only knows the English title, so localized titles
                // (e.g. "Na plech" for "Do the Mæth") come from Wikidata.
                let (cinemeta, wikidata) = join(
                    self.lookup_cinemeta(content_type, imdb_id),
                    self.lookup_wikidata_titles(imdb_id),
                )
                .await;
                cinemeta?.map(|info| merge_wikidata_titles(info, wikidata))
            }
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

    /// Looks up Czech, Slovak and English labels for an IMDb ID. Failures are
    /// treated as missing data so they never block the primary lookup.
    pub async fn lookup_wikidata_titles(&self, imdb_id: &str) -> Option<WikidataTitles> {
        let wikidata_url = self.wikidata_url.as_deref()?;
        if !imdb_id.starts_with("tt") {
            return None;
        }
        match self.fetch_wikidata_titles(wikidata_url, imdb_id).await {
            Ok(titles) => titles,
            Err(error) => {
                tracing::debug!(imdb_id, %error, "wikidata title lookup failed");
                None
            }
        }
    }

    async fn fetch_wikidata_titles(
        &self,
        wikidata_url: &str,
        imdb_id: &str,
    ) -> Result<Option<WikidataTitles>, MetadataError> {
        let search = format!("haswbstatement:P345={imdb_id}");
        let found = self
            .http
            .get(wikidata_url)
            .timeout(WIKIDATA_TIMEOUT)
            .header(reqwest::header::USER_AGENT, WIKIDATA_USER_AGENT)
            .query(&[
                ("action", "query"),
                ("list", "search"),
                ("srsearch", search.as_str()),
                ("srlimit", "1"),
                ("format", "json"),
            ])
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        let Some(entity_id) = found
            .pointer("/query/search/0/title")
            .and_then(Value::as_str)
            .filter(|id| id.starts_with('Q'))
        else {
            return Ok(None);
        };

        let entities = self
            .http
            .get(wikidata_url)
            .timeout(WIKIDATA_TIMEOUT)
            .header(reqwest::header::USER_AGENT, WIKIDATA_USER_AGENT)
            .query(&[
                ("action", "wbgetentities"),
                ("ids", entity_id),
                ("props", "labels"),
                ("languages", "cs|sk|en"),
                ("format", "json"),
            ])
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        let Some(labels) = entities
            .get("entities")
            .and_then(|entities| entities.get(entity_id))
            .and_then(|entity| entity.get("labels"))
        else {
            return Ok(None);
        };
        let label = |lang: &str| string_value_at(labels, &[lang, "value"]);
        Ok(Some(WikidataTitles {
            cs: label("cs"),
            sk: label("sk"),
            en: label("en"),
        }))
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
        let (base_id, season, episode) =
            parse_episode_id_suffix(id).unwrap_or_else(|| (id.to_string(), None, None));
        Self {
            base_id,
            season,
            episode,
        }
    }
}

fn parse_episode_id_suffix(id: &str) -> Option<(String, Option<u32>, Option<u32>)> {
    let (without_episode, episode) = id.rsplit_once(':')?;
    let episode = episode.parse().ok()?;
    let (base_id, season) = without_episode.rsplit_once(':')?;
    let season = season.parse().ok()?;
    Some((base_id.to_string(), Some(season), Some(episode)))
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

/// Fills missing localized titles from Wikidata. A Czech title becomes the
/// primary title, matching the TMDB path, and the previous primary title is
/// kept as the English one.
pub fn merge_wikidata_titles(mut info: TitleInfo, titles: Option<WikidataTitles>) -> TitleInfo {
    let Some(titles) = titles else {
        return info;
    };
    if info.title_en.is_none() {
        info.title_en = info.primary_title.clone().or(titles.en);
    }
    if info.title_sk.is_none() {
        info.title_sk = titles.sk;
    }
    if info.title_cs.is_none() {
        if let Some(cs) = titles.cs {
            info.primary_title = Some(cs.clone());
            info.title_cs = Some(cs);
        }
    }
    info
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
    fn parses_namespaced_stremio_episode_ids() {
        assert_eq!(
            ParsedStremioId::parse("tmdb:12345:2:8"),
            ParsedStremioId {
                base_id: "tmdb:12345".to_string(),
                season: Some(2),
                episode: Some(8),
            }
        );
        assert_eq!(
            ParsedStremioId::parse("tmdb:12345"),
            ParsedStremioId {
                base_id: "tmdb:12345".to_string(),
                season: None,
                episode: None,
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

    #[test]
    fn wikidata_titles_make_czech_primary_for_cinemeta_info() {
        let info = TitleInfo {
            content_type: Some("movie".to_string()),
            primary_title: Some("Do the Mæth".to_string()),
            year: Some("2025".to_string()),
            ..TitleInfo::default()
        };
        let titles = WikidataTitles {
            cs: Some("Na plech".to_string()),
            sk: Some("Na plech".to_string()),
            en: Some("Do the Mæth".to_string()),
        };

        let merged = merge_wikidata_titles(info, Some(titles));

        assert_eq!(merged.primary_title.as_deref(), Some("Na plech"));
        assert_eq!(merged.title_cs.as_deref(), Some("Na plech"));
        assert_eq!(merged.title_en.as_deref(), Some("Do the Mæth"));
        assert_eq!(merged.title_candidates(), vec!["Na plech", "Do the Mæth"]);
    }

    #[test]
    fn wikidata_titles_do_not_override_existing_czech_title() {
        let info = TitleInfo {
            primary_title: Some("Vetřelec".to_string()),
            title_cs: Some("Vetřelec".to_string()),
            ..TitleInfo::default()
        };
        let titles = WikidataTitles {
            cs: Some("Votrelec".to_string()),
            ..WikidataTitles::default()
        };

        let merged = merge_wikidata_titles(info, Some(titles));

        assert_eq!(merged.title_cs.as_deref(), Some("Vetřelec"));
        assert_eq!(merged.primary_title.as_deref(), Some("Vetřelec"));
    }

    #[tokio::test]
    async fn imdb_lookup_without_tmdb_adds_wikidata_czech_title() {
        use axum::{extract::Query, routing::get, Json, Router};
        use std::collections::HashMap;

        let app = Router::new()
            .route(
                "/meta/movie/tt34483589.json",
                get(|| async {
                    Json(json!({ "meta": { "name": "Do the Mæth", "year": "2025" } }))
                }),
            )
            .route(
                "/w/api.php",
                get(|Query(params): Query<HashMap<String, String>>| async move {
                    match params.get("action").map(String::as_str) {
                        Some("query") => {
                            assert_eq!(
                                params.get("srsearch").map(String::as_str),
                                Some("haswbstatement:P345=tt34483589")
                            );
                            Json(json!({ "query": { "search": [{ "title": "Q130739697" }] } }))
                        }
                        _ => Json(json!({ "entities": { "Q130739697": { "labels": {
                            "cs": { "language": "cs", "value": "Na plech" },
                            "sk": { "language": "sk", "value": "Na plech" },
                            "en": { "language": "en", "value": "Do the Mæth" }
                        } } } })),
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let client = MetadataClient::new(MetadataConfig {
            cinemeta_url: base.clone(),
            ..MetadataConfig::default()
        })
        .with_wikidata_url(Some(format!("{base}/w/api.php")));

        let info = client
            .lookup_imdb("movie", "tt34483589")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(info.primary_title.as_deref(), Some("Na plech"));
        assert_eq!(info.title_cs.as_deref(), Some("Na plech"));
        assert_eq!(info.title_en.as_deref(), Some("Do the Mæth"));
        assert_eq!(info.year.as_deref(), Some("2025"));
    }
}
