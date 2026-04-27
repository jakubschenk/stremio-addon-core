# stremio-addon-core Technical Specification

Status: draft contract for provider adapters.

This document defines the reusable protocol base for Stremio addon servers. It is focused on HTTP routing, Stremio response models, addon auth, config decoding, metadata lookup, query generation, normalized result ranking, card formatting, and playback redirects. Provider API clients stay outside the core.

## Design Goals

- Provide a reusable Rust base for multiple Stremio provider addons.
- Preserve official Stremio protocol routes.
- Support common private-addon route shapes such as config-path auth, query/header auth, key-prefixed routes, POST stream routes, and manifest aliases.
- Keep provider-specific search execution, login, cache, and playback resolution out of the core.
- Provide shared TMDB/Cinemeta title lookup, search query generation, normalized result ranking, and common stream-card formatting because these are reused across provider adapters.
- Make private addon auth easy to enforce consistently.
- Support token-hiding playback redirects for providers that should not expose upstream session tokens.
- Keep models typed for common Stremio fields while allowing provider-specific extensions through flattened JSON maps.

## Non-Goals

- No provider HTTP client abstraction in this crate.
- No provider raw-response parser or cache implementation in this crate.
- No static asset server in this crate yet. Provider apps can layer static routes around the returned Axum router.
- No public Stremio Central publishing support in this crate.

## Compatibility Targets

The core is expected to support these generic addon patterns:

| Provider style | Required core support | Status |
| --- | --- | --- |
| Private stream addon | Authenticated manifest/config routes and stream routes | Supported |
| Stream-only POST addon | POST stream request routes, query/header auth, and manifest aliases | Supported |
| Key-prefixed private addon | `/u/{key}/...` manifest and stream routes | Supported |
| Catalog/meta addon | Catalog and meta routes with config-path support | Supported |
| Redirecting playback addon | Signed playback route verification and redirect response handling | Supported |

## Public API

The crate re-exports:

- `AuthConfig`
- `AuthError`
- `UserConfig`
- `decode_config_segment`
- All protocol models from `models`
- `AddonAdapter`
- `AddonContext`
- `AddonError`
- `PlaybackResponse`
- `RouterOptions`
- `build_router`
- `build_router_with_options`
- `SignedPlayback`
- `SigningError`
- `MetadataClient`
- `MetadataConfig`
- `TitleInfo`
- `QueryProfile`
- `QueryInput`
- search query helpers
- `RankingProfile`
- `RankingOptions`
- `SearchResult`
- `RankedResult`
- `MatchScore`
- ranking helpers
- `CardProfile`
- `StreamCardInput`
- stream card helpers

Provider apps should normally need only the crate root:

```rust
use stremio_addon_core::*;
```

## Adapter Contract

Providers implement:

```rust
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
    ) -> Result<StreamResponse, AddonError>;

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
```

Default `stream_request` behavior:

- If request `type` and `id` are both present, it forwards to `stream`.
- Otherwise it returns an empty stream list.

Provider expectations:

- Return empty Stremio responses for valid no-result cases.
- Return `AddonError::BadRequest` only for malformed caller input.
- Return `AddonError::Provider` for upstream/provider failures that should be visible as HTTP 500.
- Return `AddonError::Playback` for playback redirect failures.

## Context Contract

`AddonContext` currently contains:

```rust
pub struct AddonContext {
    pub user_config: UserConfig,
}
```

`UserConfig` contains:

```rust
pub struct UserConfig {
    pub auth_key: Option<String>,
    pub enable_search: Option<bool>,
    pub extra: Map<String, Value>,
}
```

Rules:

- `authKey` and `enableSearch` are first-class because they are common across private Stremio addon configs.
- Unknown config fields are preserved in `extra`.
- Sensitive provider-specific config fields may exist in `extra`; adapters must avoid logging raw config.

## Router Construction

Simple construction:

```rust
let router = build_router(adapter, AuthConfig::required("secret"));
```

Custom construction:

```rust
let router = build_router_with_options(
    adapter,
    auth,
    RouterOptions {
        catalog_routes: false,
        meta_routes: false,
        playback_routes: false,
        alias_routes: true,
        path_key_routes: true,
        health_routes: true,
        playback_signing_key: None,
    },
);
```

### RouterOptions

| Field | Default | Contract |
| --- | --- | --- |
| `catalog_routes` | `true` | Mount catalog routes. Disable for stream-only addons. |
| `meta_routes` | `true` | Mount meta routes. Disable for stream-only addons. |
| `playback_routes` | `true` | Mount playback redirect routes. Disable for direct-URL-only providers. |
| `alias_routes` | `true` | Mount root/index/API manifest aliases. |
| `path_key_routes` | `true` | Mount key-prefixed `/u/{key}/...` routes. |
| `health_routes` | `true` | Mount unauthenticated health endpoints. |
| `playback_signing_key` | `None` | If set, require and verify `?sig=` for playback routes. |

## Route Contract

### Manifest

| Route | Config source | Auth source |
| --- | --- | --- |
| `GET /manifest.json` | default empty config | query/header |
| `GET /{config}/manifest.json` | encoded JSON path config | config/query/header |
| `GET /u/{path_key}/manifest.json` | default empty config | path key/query/header |
| `GET /` | default empty config | query/header |
| `GET /stremio/manifest.json` | default empty config | query/header |
| `GET /api/manifest` | default empty config | query/header |

All manifest routes call `AddonAdapter::manifest`.

### Streams

| Route | Method | Adapter call |
| --- | --- | --- |
| `/stream/{type}/{id}` | GET | `stream(ctx, type, id)` |
| `/{config}/stream/{type}/{id}` | GET | `stream(ctx, type, id)` with decoded config |
| `/u/{path_key}/stream/{type}/{id}` | GET | `stream(ctx, type, id)` with path-key auth |
| `/api/streams/{type}/{id}` | GET | `stream(ctx, type, id)` |
| `/stream` | POST | `stream_request(ctx, body)` |
| `/api/streams` | POST | `stream_request(ctx, body)` |

Trailing `.json` is stripped from `id`.

### Catalog

| Route | Adapter call |
| --- | --- |
| `/catalog/{type}/{id}/{extra}` | `catalog(ctx, type, id, extra)` |
| `/{config}/catalog/{type}/{id}/{extra}` | `catalog(ctx, type, id, extra)` with decoded config |

`extra` path segments are parsed as slash-separated `key=value` pairs. Example:

```text
search=alien%201979.json
```

becomes:

```rust
extra.get("search") == Some("alien 1979")
```

Trailing `.json` is stripped from the last extra segment before parsing.

### Meta

| Route | Adapter call |
| --- | --- |
| `/meta/{type}/{id}` | `meta(ctx, type, id)` |
| `/{config}/meta/{type}/{id}` | `meta(ctx, type, id)` with decoded config |

Trailing `.json` is stripped from `id`.

### Playback

| Route | Adapter call |
| --- | --- |
| `/play/{ident}` | `playback(ctx, ident)` |
| `/{config}/play/{ident}` | `playback(ctx, ident)` with decoded config |

If `RouterOptions.playback_signing_key` is set:

- Query `sig` is required.
- `sig` must verify with `SignedPlayback::verify`.
- Signed payload `ident` must equal the path `ident`.
- Expired signatures are rejected.

The adapter returns:

```rust
pub struct PlaybackResponse {
    pub location: String,
    pub cache_max_age_seconds: u64,
}
```

Core responds with an HTTP temporary redirect and:

```text
Cache-Control: max-age={cache_max_age_seconds}, must-revalidate, proxy-revalidate
```

### Health

| Route | Auth | Response |
| --- | --- | --- |
| `/health` | none | `{ "status": "ok" }` |
| `/healthz` | none | `{ "status": "ok" }` |

Health routes intentionally bypass addon auth so deployments can use them for liveness checks.

## Auth Contract

`AuthConfig::required(key)` requires a valid key on protected routes.

`AuthConfig::disabled()` accepts all requests.

Accepted key locations in precedence order:

1. Encoded config JSON `authKey`
2. Path key from `/u/{path_key}/...`
3. Query `authKey`
4. Query alias `key`
5. Header `Authorization: Bearer {key}`
6. Header `X-Addon-Auth: {key}`

Comparison is constant-time using `subtle::ConstantTimeEq`.

`AuthConfig` implements redacted `Debug`.

### Auth Error Mapping

| AuthError | HTTP status |
| --- | --- |
| `Misconfigured` | `401` |
| `Missing` | `401` |
| `Invalid` | `401` |

## Encoded Config Contract

Config path segments are percent-decoded then parsed as JSON.

Example:

```text
/%7B%22authKey%22%3A%22secret%22%2C%22enableSearch%22%3Atrue%7D/manifest.json
```

parses to:

```json
{
  "authKey": "secret",
  "enableSearch": true
}
```

Invalid UTF-8 or invalid JSON returns HTTP 400.

## Model Contract

All modeled JSON uses Stremio-compatible camelCase where applicable.

### Manifest

Required:

- `id`
- `version`
- `name`
- `resources`
- `types`

Optional:

- `description`
- `catalogs`
- `idPrefixes`
- `behaviorHints`
- `config`
- `logo`
- `background`
- `contactEmail`
- flattened `extra`

`resources` supports both:

```json
["stream"]
```

and:

```json
[{ "name": "stream", "types": ["movie"], "idPrefixes": ["tt"] }]
```

Use `Manifest.extra` for fields such as:

```json
{
  "stremioAddonsConfig": {
    "issuer": "https://stremio-addons.net",
    "signature": "..."
  }
}
```

### BehaviorHints

Modeled:

- `configurable`
- `configurationRequired`
- `p2p`
- `adult`

Additional behavior-hint fields can be added later if needed.

### Stream

Modeled:

- `ident`
- `name`
- `title`
- `quality`
- `url`
- `externalUrl`
- `infoHash`
- `sources`
- `description`
- `behaviorHints`
- flattened `extra`

Transport rules:

- Direct streaming providers should set `url`.
- External-page providers should set `externalUrl`.
- Torrent providers should set `infoHash` and optionally `sources`.
- Providers should not set dummy empty strings; leave unsupported fields as `None` or empty vecs.

### StreamBehaviorHints

Modeled:

- `countryWhitelist`
- `bingeGroup`
- `videoSize`
- `filename`
- flattened `extra`

## Metadata Contract

`MetadataClient` resolves Stremio title metadata through TMDB and Cinemeta.

Inputs:

- `lookup_imdb(content_type, stremio_id)`
- `lookup_tmdb_id(content_type, stremio_id)`

`content_type` should be `movie` or `series`.

`stremio_id` may contain episode suffixes:

```text
tt0903747:1:2
278
1396:1:2
```

Output:

```rust
pub struct TitleInfo {
    pub content_type: Option<String>,
    pub primary_title: Option<String>,
    pub title_sk: Option<String>,
    pub title_en: Option<String>,
    pub original_title: Option<String>,
    pub year: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}
```

TMDB behavior:

- TMDB is used only when `MetadataConfig.tmdb_api_key` is set.
- IMDb lookup uses `/find/{imdb_id}?external_source=imdb_id`.
- Direct TMDB lookup uses `/movie/{id}` or `/tv/{id}`.
- Czech and Slovak requests are made.
- English fallback is requested when the original language is not English.
- `MetadataClient::new` applies `MetadataConfig.timeout_seconds` to outbound HTTP requests. The default is 10 seconds.

Cinemeta behavior:

- Used when TMDB key is missing.
- Used as IMDb fallback when TMDB returns no usable result.
- Direct TMDB IDs do not fall back to Cinemeta.

Caching:

- Core does not cache metadata.
- Provider adapters should wrap `MetadataClient` with their own cache if needed.

## Search Query Contract

`QueryInput` is built directly or from `TitleInfo`.

```rust
pub struct QueryInput {
    pub content_type: Option<String>,
    pub titles: Vec<String>,
    pub year: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}
```

Profiles:

- `QueryProfile::Balanced`
- Provider-tuned query profiles

Balanced:

- Movie queries are title candidates followed by title plus year variants.
- Series queries are `SxxExx` and `xxXxx` variants for every title.

Provider-tuned profiles may add provider-specific query ordering or fallback variants.

Provider adapters remain responsible for:

- Executing provider searches.
- Mapping raw provider rows into `SearchResult`.
- Provider-specific dedupe strategy before or after ranking.
- Early stop heuristics.

## Ranking Contract

`SearchResult` is the normalized provider-search row consumed by core ranking:

```rust
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub parsed_title: Option<String>,
    pub filename: Option<String>,
    pub year: Option<i32>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub size_bytes: Option<u64>,
    pub positive_votes: Option<i64>,
    pub negative_votes: Option<i64>,
    pub quality_rank: Option<i32>,
    pub protected: bool,
    pub query_index: Option<usize>,
}
```

Providers should fill as many parsed fields as their provider API or filename parser can safely provide. `title` is required and should be the best available provider display title or filename. `id` is the stable provider identifier used later for playback resolution.

`rank_results(info, results, options)` returns sorted `RankedResult` values:

```rust
pub struct RankedResult {
    pub result: SearchResult,
    pub score: MatchScore,
}
```

Profiles:

- `RankingProfile::Balanced`: generic title/filename blend for new adapters.
- Provider-tuned profiles may prioritize parsed title, filename, query order, votes, size, or quality differently.

Optional signals:

- `negative_votes` reduces the vote contribution to ranking.
- `quality_rank` is a provider-normalized quality signal where higher is better.
- `query_index` should be the zero-based index of the query that produced the row. Lower known indexes beat higher indexes, and unknown indexes sort last when scores tie.

Default filtering rules:

- Exclude protected rows.
- Exclude rows without at least a weak title or filename match.
- Exclude movie results whose parsed year is outside `year_tolerance`.
- Exclude series results when target season/episode metadata is present and the row season/episode differs.
- Exclude episode-like movie false positives for profiles that enable that guard.

`MatchScore.strong_match` and `MatchScore.weak_match` are intended to flow into `StreamCardInput.strong_match` and provider-specific display choices.

Provider adapters may still add provider-specific prefilters or postfilters when a source has signals the core does not model. Those rules should operate around `SearchResult` and `RankedResult` rather than duplicating the common title/year/episode scoring.

## Card Formatting Contract

`StreamCardInput` describes provider presentation data:

```rust
pub struct StreamCardInput {
    pub provider: String,
    pub provider_url: Option<String>,
    pub ident: Option<String>,
    pub quality: Option<String>,
    pub filename: Option<String>,
    pub size_bytes: Option<u64>,
    pub language: Option<String>,
    pub audio: Option<String>,
    pub runtime_seconds: Option<u64>,
    pub codec: Option<String>,
    pub positive_votes: Option<i64>,
    pub negative_votes: Option<i64>,
    pub strong_match: bool,
    pub binge_group: Option<String>,
    pub country_whitelist: Option<Vec<String>>,
}
```

Profiles:

- `CardProfile::Compact`
- Provider-tuned card profiles

Rules:

- Card helpers are presentation-only.
- Card helpers must not perform provider API calls.
- Card helpers should run after provider rows have been normalized and ranked.
- Provider adapters may still build `Stream` manually when a profile is not a fit.

### CatalogResponse

Serialized shape:

```json
{
  "metas": [],
  "cacheMaxAge": 3600
}
```

`cacheMaxAge` is optional.

### MetaResponse

Default missing meta serializes as:

```json
{ "meta": {} }
```

This preserves compatibility with clients that expect an empty object instead of `null`.

## Signed Playback Contract

`SignedPlayback` payload:

```rust
pub struct SignedPlayback {
    pub ident: String,
    pub expires_at: i64,
}
```

Token format:

```text
base64url(json_payload).base64url(hmac_sha256(payload))
```

Rules:

- HMAC key is supplied by the caller.
- Verification uses `hmac::Mac::verify_slice`.
- Expired tokens are rejected.
- Malformed tokens are rejected.
- Signed payload does not expose the plain `ident` as a substring of the token due to base64url encoding, but it is not encryption. Do not put upstream secrets in `ident`.

Recommended provider usage:

- Use a stable provider file ID or opaque lookup ID as `ident`.
- Do not put session tokens, passwords, or signed upstream URLs inside the `ident`.
- Use a short TTL for sensitive providers.

## Error Contract

All core errors serialize as:

```json
{ "error": "..." }
```

HTTP status mapping:

| AddonError | HTTP status |
| --- | --- |
| `Auth` | `401` |
| `Config` | `400` |
| `BadRequest` | `400` |
| `Provider` | `500` |
| `Playback` | `500` |

Provider adapters should avoid leaking credentials or upstream tokens in error strings.

## Security Invariants

- Auth key comparisons must remain constant-time.
- Auth keys must not be emitted in `Debug`.
- Provider apps must not log `UserConfig.extra` wholesale if it may contain secrets.
- Playback signing must not be treated as encryption.
- Signed playback identifiers must not contain raw upstream credentials or session tokens.
- Health endpoints are public by design; do not include sensitive runtime state in health responses.

## Extension Points

The core intentionally leaves these to provider apps or future helper crates:

- Static asset routes.
- HTML configure/install pages.
- Provider-specific health payloads.
- Rate-limited HTTP clients.
- Caching.
- Metadata providers beyond TMDB/Cinemeta.
- Provider-specific search-query profiles.
- Provider-specific ranking profiles or overrides.

Provider apps can compose extra Axum routes around the core router:

```rust
let app = build_router(adapter, auth)
    .route("/configure", axum::routing::get(configure))
    .route("/icon.png", axum::routing::get(icon));
```

## Testing Requirements

Core tests should cover:

- Auth from every supported location.
- Missing/invalid auth rejections.
- Encoded config parsing.
- POST stream body routing.
- Path-key routing.
- Catalog `cacheMaxAge` serialization.
- Empty meta `{ "meta": {} }` serialization.
- Signed playback success, missing signature, bad signature, and expired signature.
- Metadata ID parsing and title candidate generation.
- Query generation profile order.
- Ranking filters for title, year, protection, and season/episode mismatches.
- Card formatting profiles.
- CORS/route behavior when integration coverage is added.

Provider tests should cover:

- Manifest snapshot for that provider.
- Route subset selected by `RouterOptions`.
- Provider stream mapping.
- Provider no-result behavior.
- Provider auth deployment shape.
- Provider playback redirect behavior if enabled.
