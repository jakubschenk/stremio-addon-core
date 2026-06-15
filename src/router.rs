use crate::auth::{AuthConfig, AuthError, AuthQuery};
use crate::config::{decode_config_segment, strip_json_suffix, ConfigError, UserConfig};
use crate::models::{
    CatalogExtraArgs, CatalogResponse, Manifest, MetaResponse, StreamRequest, StreamResponse,
    SubtitlesExtraArgs, SubtitlesResponse,
};
use crate::signing::{SignedPlayback, SigningError};
use async_trait::async_trait;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use percent_encoding::percent_decode_str;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

#[derive(Clone, Debug, Default)]
pub struct AddonContext {
    pub user_config: UserConfig,
}

#[async_trait]
pub trait AddonAdapter: Send + Sync + 'static {
    async fn manifest(&self, ctx: AddonContext) -> Result<Manifest, AddonError>;

    async fn stream(
        &self,
        ctx: AddonContext,
        content_type: String,
        id: String,
    ) -> Result<StreamResponse, AddonError>;

    async fn stream_request(
        &self,
        ctx: AddonContext,
        request: StreamRequest,
    ) -> Result<StreamResponse, AddonError> {
        let Some(content_type) = request.content_type else {
            return Ok(StreamResponse::default());
        };
        let Some(id) = request.id else {
            return Ok(StreamResponse::default());
        };
        self.stream(ctx, content_type, id).await
    }

    async fn subtitles(
        &self,
        _ctx: AddonContext,
        _content_type: String,
        _id: String,
        _extra: SubtitlesExtraArgs,
    ) -> Result<SubtitlesResponse, AddonError> {
        Ok(SubtitlesResponse::default())
    }

    async fn catalog(
        &self,
        ctx: AddonContext,
        content_type: String,
        id: String,
        extra: CatalogExtraArgs,
    ) -> Result<CatalogResponse, AddonError>;

    async fn meta(
        &self,
        ctx: AddonContext,
        content_type: String,
        id: String,
    ) -> Result<MetaResponse, AddonError>;

    async fn playback(
        &self,
        ctx: AddonContext,
        ident: String,
    ) -> Result<PlaybackResponse, AddonError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AddonError {
    #[error("addon auth failed: {0}")]
    Auth(#[from] AuthError),
    #[error("invalid config: {0}")]
    Config(#[from] ConfigError),
    #[error("not found")]
    NotFound,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("playback error: {0}")]
    Playback(String),
}

impl IntoResponse for AddonError {
    fn into_response(self) -> Response {
        let status = match self {
            AddonError::Auth(_) => StatusCode::UNAUTHORIZED,
            AddonError::NotFound => StatusCode::NOT_FOUND,
            AddonError::Config(_) | AddonError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AddonError::Provider(_) | AddonError::Playback(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        let body = ErrorBody {
            error: self.to_string(),
        };

        (status, Json(body)).into_response()
    }
}

#[derive(Clone, Debug)]
pub struct PlaybackResponse {
    pub location: String,
    pub cache_max_age_seconds: u64,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Clone)]
struct AppState {
    adapter: Arc<dyn AddonAdapter>,
    auth: AuthConfig,
    options: RouterOptions,
}

struct SubtitlesRouteParts {
    user_config: Option<UserConfig>,
    content_type: String,
    id: String,
    extra: String,
    path_key: Option<String>,
}

pub fn build_router(adapter: Arc<dyn AddonAdapter>, auth: AuthConfig) -> Router {
    build_router_with_options(adapter, auth, RouterOptions::default())
}

#[derive(Clone, Debug)]
pub struct RouterOptions {
    pub catalog_routes: bool,
    pub meta_routes: bool,
    pub subtitles_routes: bool,
    pub playback_routes: bool,
    pub alias_routes: bool,
    pub path_key_routes: bool,
    pub health_routes: bool,
    pub playback_signing_key: Option<String>,
    pub require_stream_json_suffix: bool,
}

impl Default for RouterOptions {
    fn default() -> Self {
        Self {
            catalog_routes: true,
            meta_routes: true,
            subtitles_routes: true,
            playback_routes: true,
            alias_routes: true,
            path_key_routes: true,
            health_routes: true,
            playback_signing_key: None,
            require_stream_json_suffix: false,
        }
    }
}

pub fn build_router_with_options(
    adapter: Arc<dyn AddonAdapter>,
    auth: AuthConfig,
    options: RouterOptions,
) -> Router {
    let mut router = Router::new()
        .route("/manifest.json", get(manifest))
        .route("/:config/manifest.json", get(manifest_with_config))
        .route("/stream", post(stream_post))
        .route("/stream/:type/:id", get(stream_without_config))
        .route("/api/streams", post(stream_post))
        .route("/api/streams/:type/:id", get(stream_api_without_config))
        .route("/:config/stream/:type/:id", get(stream_with_config));

    if options.alias_routes {
        router = router
            .route("/", get(manifest))
            .route("/stremio/manifest.json", get(manifest))
            .route("/api/manifest", get(manifest));
    }

    if options.path_key_routes {
        router = router
            .route("/u/:path_key/manifest.json", get(manifest_with_path_key))
            .route("/u/:path_key/stream/:type/:id", get(stream_with_path_key));
    }

    if options.catalog_routes {
        router = router
            .route("/catalog/:type/:id/*extra", get(catalog_without_config))
            .route(
                "/:config/catalog/:type/:id/*extra",
                get(catalog_with_config),
            );
    }

    if options.meta_routes {
        router = router
            .route("/meta/:type/:id", get(meta_without_config))
            .route("/:config/meta/:type/:id", get(meta_with_config));
    }

    if options.subtitles_routes {
        router = router
            .route("/subtitles/:type/:id", get(subtitles_without_config_empty))
            .route("/subtitles/:type/:id/*extra", get(subtitles_without_config))
            .route(
                "/:config/subtitles/:type/:id",
                get(subtitles_with_config_empty),
            )
            .route(
                "/:config/subtitles/:type/:id/*extra",
                get(subtitles_with_config),
            );

        if options.path_key_routes {
            router = router
                .route(
                    "/u/:path_key/subtitles/:type/:id",
                    get(subtitles_with_path_key_empty),
                )
                .route(
                    "/u/:path_key/subtitles/:type/:id/*extra",
                    get(subtitles_with_path_key),
                );
        }
    }

    if options.playback_routes {
        router = router
            .route("/play/:ident", get(play_without_config))
            .route("/:config/play/:ident", get(play_with_config));
    }

    if options.health_routes {
        router = router
            .route("/health", get(health))
            .route("/healthz", get(health));
    }

    let state = AppState {
        adapter,
        auth,
        options,
    };

    router.layer(CorsLayer::permissive()).with_state(state)
}

async fn manifest(
    State(state): State<AppState>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<Manifest>, AddonError> {
    let ctx = context_from_parts(&state.auth, None, Some(&query), &headers, None)?;
    state.adapter.manifest(ctx).await.map(Json)
}

async fn stream_post(
    State(state): State<AppState>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    Json(body): Json<StreamRequest>,
) -> Result<Json<StreamResponse>, AddonError> {
    let ctx = context_from_parts(&state.auth, None, Some(&query), &headers, None)?;
    state.adapter.stream_request(ctx, body).await.map(Json)
}

async fn manifest_with_config(
    State(state): State<AppState>,
    Path(config): Path<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<Manifest>, AddonError> {
    let cfg = decode_config_segment(&config)?;
    let ctx = context_from_parts(&state.auth, Some(cfg), Some(&query), &headers, None)?;
    state.adapter.manifest(ctx).await.map(Json)
}

async fn manifest_with_path_key(
    State(state): State<AppState>,
    Path(path_key): Path<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<Manifest>, AddonError> {
    let ctx = context_from_parts(
        &state.auth,
        None,
        Some(&query),
        &headers,
        Some(path_key.as_str()),
    )?;
    state.adapter.manifest(ctx).await.map(Json)
}

async fn stream_without_config(
    State(state): State<AppState>,
    Path((content_type, id)): Path<(String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<StreamResponse>, AddonError> {
    let ctx = context_from_parts(&state.auth, None, Some(&query), &headers, None)?;
    let id = stream_path_id(&state.options, &id)?;
    state.adapter.stream(ctx, content_type, id).await.map(Json)
}

async fn stream_api_without_config(
    State(state): State<AppState>,
    Path((content_type, id)): Path<(String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<StreamResponse>, AddonError> {
    let ctx = context_from_parts(&state.auth, None, Some(&query), &headers, None)?;
    state
        .adapter
        .stream(ctx, content_type, strip_json_suffix(&id).to_string())
        .await
        .map(Json)
}

async fn stream_with_path_key(
    State(state): State<AppState>,
    Path((path_key, content_type, id)): Path<(String, String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<StreamResponse>, AddonError> {
    let ctx = context_from_parts(
        &state.auth,
        None,
        Some(&query),
        &headers,
        Some(path_key.as_str()),
    )?;
    let id = stream_path_id(&state.options, &id)?;
    state.adapter.stream(ctx, content_type, id).await.map(Json)
}

async fn stream_with_config(
    State(state): State<AppState>,
    Path((config, content_type, id)): Path<(String, String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<StreamResponse>, AddonError> {
    let cfg = decode_config_segment(&config)?;
    let ctx = context_from_parts(&state.auth, Some(cfg), Some(&query), &headers, None)?;
    let id = stream_path_id(&state.options, &id)?;
    state.adapter.stream(ctx, content_type, id).await.map(Json)
}

async fn catalog_without_config(
    State(state): State<AppState>,
    Path((content_type, id, extra)): Path<(String, String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<CatalogResponse>, AddonError> {
    let ctx = context_from_parts(&state.auth, None, Some(&query), &headers, None)?;
    state
        .adapter
        .catalog(ctx, content_type, id, parse_extra_args(&extra)?)
        .await
        .map(Json)
}

async fn catalog_with_config(
    State(state): State<AppState>,
    Path((config, content_type, id, extra)): Path<(String, String, String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<CatalogResponse>, AddonError> {
    let cfg = decode_config_segment(&config)?;
    let ctx = context_from_parts(&state.auth, Some(cfg), Some(&query), &headers, None)?;
    state
        .adapter
        .catalog(ctx, content_type, id, parse_extra_args(&extra)?)
        .await
        .map(Json)
}

async fn subtitles_without_config_empty(
    State(state): State<AppState>,
    Path((content_type, id)): Path<(String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<SubtitlesResponse>, AddonError> {
    subtitles_response(
        state,
        query,
        headers,
        SubtitlesRouteParts {
            user_config: None,
            content_type,
            id,
            extra: String::new(),
            path_key: None,
        },
    )
    .await
}

async fn subtitles_without_config(
    State(state): State<AppState>,
    Path((content_type, id, extra)): Path<(String, String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<SubtitlesResponse>, AddonError> {
    subtitles_response(
        state,
        query,
        headers,
        SubtitlesRouteParts {
            user_config: None,
            content_type,
            id,
            extra,
            path_key: None,
        },
    )
    .await
}

async fn subtitles_with_config_empty(
    State(state): State<AppState>,
    Path((config, content_type, id)): Path<(String, String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<SubtitlesResponse>, AddonError> {
    let cfg = decode_config_segment(&config)?;
    subtitles_response(
        state,
        query,
        headers,
        SubtitlesRouteParts {
            user_config: Some(cfg),
            content_type,
            id,
            extra: String::new(),
            path_key: None,
        },
    )
    .await
}

async fn subtitles_with_config(
    State(state): State<AppState>,
    Path((config, content_type, id, extra)): Path<(String, String, String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<SubtitlesResponse>, AddonError> {
    let cfg = decode_config_segment(&config)?;
    subtitles_response(
        state,
        query,
        headers,
        SubtitlesRouteParts {
            user_config: Some(cfg),
            content_type,
            id,
            extra,
            path_key: None,
        },
    )
    .await
}

async fn subtitles_with_path_key_empty(
    State(state): State<AppState>,
    Path((path_key, content_type, id)): Path<(String, String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<SubtitlesResponse>, AddonError> {
    subtitles_response(
        state,
        query,
        headers,
        SubtitlesRouteParts {
            user_config: None,
            content_type,
            id,
            extra: String::new(),
            path_key: Some(path_key),
        },
    )
    .await
}

async fn subtitles_with_path_key(
    State(state): State<AppState>,
    Path((path_key, content_type, id, extra)): Path<(String, String, String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<SubtitlesResponse>, AddonError> {
    subtitles_response(
        state,
        query,
        headers,
        SubtitlesRouteParts {
            user_config: None,
            content_type,
            id,
            extra,
            path_key: Some(path_key),
        },
    )
    .await
}

async fn subtitles_response(
    state: AppState,
    query: AuthQuery,
    headers: HeaderMap,
    parts: SubtitlesRouteParts,
) -> Result<Json<SubtitlesResponse>, AddonError> {
    let ctx = context_from_parts(
        &state.auth,
        parts.user_config,
        Some(&query),
        &headers,
        parts.path_key.as_deref(),
    )?;
    state
        .adapter
        .subtitles(
            ctx,
            parts.content_type,
            strip_json_suffix(&parts.id).to_string(),
            parse_subtitles_extra_args(&parts.extra)?,
        )
        .await
        .map(Json)
}

async fn meta_without_config(
    State(state): State<AppState>,
    Path((content_type, id)): Path<(String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<MetaResponse>, AddonError> {
    let ctx = context_from_parts(&state.auth, None, Some(&query), &headers, None)?;
    state
        .adapter
        .meta(ctx, content_type, strip_json_suffix(&id).to_string())
        .await
        .map(Json)
}

async fn meta_with_config(
    State(state): State<AppState>,
    Path((config, content_type, id)): Path<(String, String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<MetaResponse>, AddonError> {
    let cfg = decode_config_segment(&config)?;
    let ctx = context_from_parts(&state.auth, Some(cfg), Some(&query), &headers, None)?;
    state
        .adapter
        .meta(ctx, content_type, strip_json_suffix(&id).to_string())
        .await
        .map(Json)
}

async fn play_without_config(
    State(state): State<AppState>,
    Path(ident): Path<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Response, AddonError> {
    let ctx = context_from_parts(&state.auth, None, Some(&query), &headers, None)?;
    let ident = verified_playback_ident(&state.options, &ident, &query)?;
    playback_response(state.adapter.playback(ctx, ident).await?)
}

async fn play_with_config(
    State(state): State<AppState>,
    Path((config, ident)): Path<(String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Response, AddonError> {
    let cfg = decode_config_segment(&config)?;
    let ctx = context_from_parts(&state.auth, Some(cfg), Some(&query), &headers, None)?;
    let ident = verified_playback_ident(&state.options, &ident, &query)?;
    playback_response(state.adapter.playback(ctx, ident).await?)
}

fn context_from_parts(
    auth: &AuthConfig,
    user_config: Option<UserConfig>,
    query: Option<&AuthQuery>,
    headers: &HeaderMap,
    path_key: Option<&str>,
) -> Result<AddonContext, AddonError> {
    let user_config = user_config.unwrap_or_default();
    auth.validate(Some(&user_config), query, headers, path_key)?;
    Ok(AddonContext { user_config })
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

fn verified_playback_ident(
    options: &RouterOptions,
    path_ident: &str,
    query: &AuthQuery,
) -> Result<String, AddonError> {
    let Some(signing_key) = options.playback_signing_key.as_deref() else {
        return Ok(path_ident.to_string());
    };

    let sig = query
        .sig
        .as_deref()
        .ok_or_else(|| AddonError::Playback("missing signed playback token".to_string()))?;
    let payload = SignedPlayback::verify(sig, signing_key.as_bytes()).map_err(signing_error)?;

    if payload.ident != path_ident {
        return Err(AddonError::Playback(
            "signed playback token does not match path ident".to_string(),
        ));
    }

    Ok(payload.ident)
}

fn signing_error(error: SigningError) -> AddonError {
    AddonError::Playback(error.to_string())
}

fn parse_extra_args(extra: &str) -> Result<CatalogExtraArgs, AddonError> {
    Ok(CatalogExtraArgs {
        values: parse_extra_values(extra, "catalog")?,
    })
}

fn parse_subtitles_extra_args(extra: &str) -> Result<SubtitlesExtraArgs, AddonError> {
    Ok(SubtitlesExtraArgs {
        values: parse_extra_values(extra, "subtitles")?,
    })
}

fn parse_extra_values(
    extra: &str,
    resource: &'static str,
) -> Result<BTreeMap<String, String>, AddonError> {
    let mut values = BTreeMap::new();
    let extra = strip_json_suffix(extra).trim_start_matches('/');
    if extra.is_empty() {
        return Ok(values);
    }

    for segment in extra.split('/') {
        if segment.is_empty() {
            continue;
        }
        for pair in segment.split('&') {
            if pair.is_empty() {
                continue;
            }
            let Some((key, value)) = pair.split_once('=') else {
                return Err(AddonError::BadRequest(format!(
                    "invalid {resource} extra segment: {pair}"
                )));
            };

            values.insert(percent_decode(key)?, percent_decode(value)?);
        }
    }

    Ok(values)
}

fn percent_decode(value: &str) -> Result<String, AddonError> {
    percent_decode_str(value)
        .decode_utf8()
        .map(|value| value.into_owned())
        .map_err(|_| AddonError::BadRequest("invalid percent encoding".to_string()))
}

fn stream_path_id(options: &RouterOptions, id: &str) -> Result<String, AddonError> {
    if options.require_stream_json_suffix && !id.ends_with(".json") {
        return Err(AddonError::NotFound);
    }

    Ok(strip_json_suffix(id).to_string())
}

fn playback_response(playback: PlaybackResponse) -> Result<Response, AddonError> {
    let mut response = Redirect::temporary(&playback.location).into_response();
    let headers = response.headers_mut();
    let cache_control = format!(
        "max-age={}, must-revalidate, proxy-revalidate",
        playback.cache_max_age_seconds
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str(&cache_control)
            .map_err(|_| AddonError::Playback("invalid cache control header".to_string()))?,
    );

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        BehaviorHints, CatalogDef, Meta, MetaPreview, Resource, ResourceSpec, Stream, Subtitle,
    };
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    struct DummyAdapter;

    #[async_trait]
    impl AddonAdapter for DummyAdapter {
        async fn manifest(&self, _ctx: AddonContext) -> Result<Manifest, AddonError> {
            Ok(Manifest {
                id: "test.addon".to_string(),
                version: "0.1.0".to_string(),
                name: "Test Addon".to_string(),
                description: None,
                resources: vec![ResourceSpec::Object(Resource {
                    name: "stream".to_string(),
                    types: vec!["movie".to_string()],
                    id_prefixes: vec!["tt".to_string()],
                })],
                types: vec!["movie".to_string()],
                catalogs: vec![CatalogDef {
                    id: "direct".to_string(),
                    r#type: "movie".to_string(),
                    name: "Direct".to_string(),
                    extra: vec![],
                }],
                id_prefixes: vec!["tt".to_string()],
                behavior_hints: Some(BehaviorHints {
                    configurable: Some(false),
                    configuration_required: Some(false),
                    p2p: Some(false),
                    adult: Some(false),
                }),
                config: vec![],
                logo: None,
                background: None,
                contact_email: None,
                extra: Default::default(),
            })
        }

        async fn stream(
            &self,
            _ctx: AddonContext,
            _content_type: String,
            id: String,
        ) -> Result<StreamResponse, AddonError> {
            Ok(StreamResponse {
                streams: vec![Stream {
                    ident: Some(id),
                    url: Some("https://example.com/play".to_string()),
                    ..Stream::default()
                }],
            })
        }

        async fn catalog(
            &self,
            _ctx: AddonContext,
            content_type: String,
            _id: String,
            extra: CatalogExtraArgs,
        ) -> Result<CatalogResponse, AddonError> {
            Ok(CatalogResponse {
                metas: vec![MetaPreview {
                    id: extra.get("search").unwrap_or_default().to_string(),
                    r#type: content_type,
                    name: "Result".to_string(),
                    poster: None,
                }],
                cache_max_age: Some(3600),
            })
        }

        async fn subtitles(
            &self,
            _ctx: AddonContext,
            content_type: String,
            id: String,
            extra: SubtitlesExtraArgs,
        ) -> Result<SubtitlesResponse, AddonError> {
            Ok(SubtitlesResponse {
                subtitles: vec![Subtitle {
                    id: format!(
                        "{content_type}:{id}:{}:{}",
                        extra.get("videoHash").unwrap_or_default(),
                        extra.get("filename").unwrap_or_default()
                    ),
                    url: "https://example.com/sub.vtt".to_string(),
                    lang: "cze".to_string(),
                    label: Some(extra.get("videoSize").unwrap_or_default().to_string()),
                    extra: Default::default(),
                }],
            })
        }

        async fn meta(
            &self,
            _ctx: AddonContext,
            content_type: String,
            id: String,
        ) -> Result<MetaResponse, AddonError> {
            Ok(MetaResponse {
                meta: Meta {
                    id,
                    r#type: content_type,
                    name: "Meta".to_string(),
                    ..Meta::default()
                },
            })
        }

        async fn playback(
            &self,
            _ctx: AddonContext,
            ident: String,
        ) -> Result<PlaybackResponse, AddonError> {
            Ok(PlaybackResponse {
                location: format!("https://example.com/{ident}"),
                cache_max_age_seconds: 18_000,
            })
        }
    }

    #[tokio::test]
    async fn manifest_requires_auth() {
        let app = build_router(Arc::new(DummyAdapter), AuthConfig::required("secret"));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/manifest.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn manifest_accepts_query_auth() {
        let app = build_router(Arc::new(DummyAdapter), AuthConfig::required("secret"));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/manifest.json?authKey=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn manifest_accepts_path_key() {
        let app = build_router(Arc::new(DummyAdapter), AuthConfig::required("secret"));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/u/secret/manifest.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn manifest_does_not_mount_php_alias() {
        let app = build_router(Arc::new(DummyAdapter), AuthConfig::required("secret"));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/index.php?authKey=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn stream_accepts_config_auth_and_strips_json_suffix() {
        let app = build_router(Arc::new(DummyAdapter), AuthConfig::required("secret"));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/%7B%22authKey%22%3A%22secret%22%7D/stream/movie/tt123.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn stream_can_require_json_suffix_without_requiring_it_for_api_alias() {
        let app = build_router_with_options(
            Arc::new(DummyAdapter),
            AuthConfig::disabled(),
            RouterOptions {
                require_stream_json_suffix: true,
                ..RouterOptions::default()
            },
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/stream/movie/tt123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/streams/movie/tt123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn stream_post_accepts_body_and_key_alias() {
        let app = build_router(Arc::new(DummyAdapter), AuthConfig::required("secret"));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/stream?key=secret")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"type":"movie","id":"tt123","name":"Movie","year":"2024"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn parses_catalog_extra_args() {
        let extra = parse_extra_args("search=hello%20world.json").unwrap();
        assert_eq!(extra.get("search"), Some("hello world"));
    }

    #[tokio::test]
    async fn subtitles_accept_stremio_extra_args_path() {
        let app = build_router(Arc::new(DummyAdapter), AuthConfig::required("secret"));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/subtitles/series/tt32550889%3A1%3A7/filename=Witch%20Hat%20Atelier%20s01e07_CZ-%20Who%20Is%20Magic%20For-..1080p.Czech.mkv&videoSize=403386418&videoHash=0ea7b10764f492d6.json?authKey=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json["subtitles"][0]["id"],
            "series:tt32550889:1:7:0ea7b10764f492d6:Witch Hat Atelier s01e07_CZ- Who Is Magic For-..1080p.Czech.mkv"
        );
        assert_eq!(json["subtitles"][0]["lang"], "cze");
        assert_eq!(json["subtitles"][0]["label"], "403386418");
    }

    #[tokio::test]
    async fn subtitles_require_auth() {
        let app = build_router(Arc::new(DummyAdapter), AuthConfig::required("secret"));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/subtitles/movie/tt123.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn parses_ampersand_extra_args() {
        let extra = parse_subtitles_extra_args(
            "filename=Movie%20Name.mkv&videoSize=123&videoHash=abc.json",
        )
        .unwrap();
        assert_eq!(extra.get("filename"), Some("Movie Name.mkv"));
        assert_eq!(extra.get("videoSize"), Some("123"));
        assert_eq!(extra.get("videoHash"), Some("abc"));
    }

    #[tokio::test]
    async fn play_requires_auth() {
        let app = build_router(Arc::new(DummyAdapter), AuthConfig::required("secret"));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/play/abc123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn play_redirects_with_cache_header() {
        let app = build_router(Arc::new(DummyAdapter), AuthConfig::required("secret"));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/play/abc123?authKey=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "https://example.com/abc123"
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "max-age=18000, must-revalidate, proxy-revalidate"
        );
    }

    #[tokio::test]
    async fn signed_playback_rejects_missing_sig() {
        let app = build_router_with_options(
            Arc::new(DummyAdapter),
            AuthConfig::required("secret"),
            RouterOptions {
                playback_signing_key: Some("signing-key".to_string()),
                ..RouterOptions::default()
            },
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/play/abc123?authKey=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn signed_playback_accepts_matching_sig() {
        let token = crate::signing::SignedPlayback::new("abc123", 60)
            .sign(b"signing-key")
            .unwrap();
        let app = build_router_with_options(
            Arc::new(DummyAdapter),
            AuthConfig::required("secret"),
            RouterOptions {
                playback_signing_key: Some("signing-key".to_string()),
                ..RouterOptions::default()
            },
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/play/abc123?authKey=secret&sig={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    }
}
