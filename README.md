# stremio-addon-core

Reusable Rust base for Stremio addon HTTP servers.

This crate owns the protocol-level work common to provider addons:

- Stremio manifest, stream, catalog, and meta response models.
- GET and POST stream routes.
- Encoded Stremio config path decoding.
- Private addon auth from config, path, query, bearer header, or custom header.
- CORS.
- Health routes.
- Optional playback redirect routes.
- Optional HMAC-signed playback tokens.
- Router options for Webshare-style, Hellspy-style, and Sosac-style route surfaces.

Provider crates should keep provider-specific login, search API execution, caching, and playback resolution outside this crate. The core includes shared title lookup, search query generation, normalized result ranking, and stream-card formatting helpers that multiple providers can reuse.

## Current Consumers

The crate was shaped against three addon styles in the local Stremio workspaces:

- `apps/webshare-addon`: new Rust Webshare addon.
- `~/stremio-hellspy/stremio-hellspy-rewrite`: stream-only Rust addon with POST stream and `?key=` auth.
- `~/stremio-sosac`: stream-only Node addon with `/u/{key}/...` path auth.

## Install Shape

Add the crate to a provider app:

```toml
[dependencies]
stremio-addon-core = { path = "../stremio-addon-core" }
async-trait = "0.1"
axum = "0.7"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## CI/CD

The crate includes GitHub Actions workflows:

- `.github/workflows/ci.yml`: runs on pushes, pull requests, and manual dispatch. It checks formatting, `cargo check`, clippy, tests, and `cargo package`.
- `.github/workflows/release.yml`: runs on `v*.*.*` tags or manual dispatch. It repeats validation, runs `cargo publish --dry-run`, then publishes to crates.io when triggered by a matching tag or manual `publish=true`.

Crates.io publishing requires a repository secret:

```text
CARGO_REGISTRY_TOKEN
```

Release flow:

1. Update `version` in `Cargo.toml`.
2. Run `cargo test --locked --all-targets`.
3. Create and push a matching tag, for example `v0.1.0`.
4. The release workflow verifies the tag matches `Cargo.toml` and publishes the crate.

## Minimal Adapter

```rust
use async_trait::async_trait;
use std::sync::Arc;
use stremio_addon_core::{
    build_router, AddonAdapter, AddonContext, AddonError, AuthConfig, BehaviorHints,
    CatalogExtraArgs, CatalogResponse, Manifest, MetaResponse, PlaybackResponse, ResourceSpec,
    Stream, StreamResponse,
};

struct MyAddon;

#[async_trait]
impl AddonAdapter for MyAddon {
    async fn manifest(&self, _ctx: AddonContext) -> Result<Manifest, AddonError> {
        Ok(Manifest {
            id: "example.my-addon".to_string(),
            version: "0.1.0".to_string(),
            name: "My Addon".to_string(),
            description: Some("Example addon".to_string()),
            resources: vec![ResourceSpec::from("stream")],
            types: vec!["movie".to_string(), "series".to_string()],
            catalogs: vec![],
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
        content_type: String,
        id: String,
    ) -> Result<StreamResponse, AddonError> {
        Ok(StreamResponse {
            streams: vec![Stream {
                name: Some(format!("Example {content_type}")),
                title: Some(id),
                url: Some("https://cdn.example/video.mp4".to_string()),
                ..Stream::default()
            }],
        })
    }

    async fn catalog(
        &self,
        _ctx: AddonContext,
        _content_type: String,
        _id: String,
        _extra: CatalogExtraArgs,
    ) -> Result<CatalogResponse, AddonError> {
        Ok(CatalogResponse::default())
    }

    async fn meta(
        &self,
        _ctx: AddonContext,
        _content_type: String,
        _id: String,
    ) -> Result<MetaResponse, AddonError> {
        Ok(MetaResponse::default())
    }

    async fn playback(
        &self,
        _ctx: AddonContext,
        _ident: String,
    ) -> Result<PlaybackResponse, AddonError> {
        Err(AddonError::Playback("playback unsupported".to_string()))
    }
}

#[tokio::main]
async fn main() {
    let auth = AuthConfig::required(std::env::var("ADDON_AUTH_KEY").unwrap());
    let app = build_router(Arc::new(MyAddon), auth);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:61613").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

## Routes

Core Stremio routes:

| Route | Method | Handler |
| --- | --- | --- |
| `/manifest.json` | GET | `AddonAdapter::manifest` |
| `/{config}/manifest.json` | GET | `AddonAdapter::manifest` with decoded config |
| `/stream/{type}/{id}` | GET | `AddonAdapter::stream` |
| `/{config}/stream/{type}/{id}` | GET | `AddonAdapter::stream` with decoded config |
| `/stream` | POST | `AddonAdapter::stream_request` |
| `/api/streams` | POST | `AddonAdapter::stream_request` |
| `/api/streams/{type}/{id}` | GET | `AddonAdapter::stream` |
| `/catalog/{type}/{id}/{extra}` | GET | `AddonAdapter::catalog` |
| `/{config}/catalog/{type}/{id}/{extra}` | GET | `AddonAdapter::catalog` with decoded config |
| `/meta/{type}/{id}` | GET | `AddonAdapter::meta` |
| `/{config}/meta/{type}/{id}` | GET | `AddonAdapter::meta` with decoded config |
| `/play/{ident}` | GET | `AddonAdapter::playback` |
| `/{config}/play/{ident}` | GET | `AddonAdapter::playback` with decoded config |

Alias routes enabled by default:

| Route | Purpose |
| --- | --- |
| `/` | manifest alias |
| `/index.php` | manifest alias for PHP-compatible addons |
| `/stremio/manifest.json` | manifest alias |
| `/api/manifest` | manifest alias |
| `/health` | unauthenticated health |
| `/healthz` | unauthenticated health |
| `/u/{authKey}/manifest.json` | Sosac-style path-key manifest |
| `/u/{authKey}/stream/{type}/{id}` | Sosac-style path-key stream |

The route parser strips a trailing `.json` from Stremio `id` path segments.

## Router Options

Use `build_router_with_options` when the provider needs a narrower route surface:

```rust
use stremio_addon_core::{build_router_with_options, RouterOptions};

let router = build_router_with_options(
    adapter,
    auth,
    RouterOptions {
        catalog_routes: false,
        meta_routes: false,
        playback_routes: false,
        alias_routes: true,
        path_key_routes: false,
        health_routes: true,
        playback_signing_key: None,
    },
);
```

Options:

| Option | Default | Meaning |
| --- | --- | --- |
| `catalog_routes` | `true` | Mount catalog routes. Disable for stream-only addons. |
| `meta_routes` | `true` | Mount meta routes. Disable for stream-only addons. |
| `playback_routes` | `true` | Mount `/play` redirect routes. Disable for direct-URL providers. |
| `alias_routes` | `true` | Mount root/index/API manifest aliases. |
| `path_key_routes` | `true` | Mount `/u/{authKey}/...` routes. |
| `health_routes` | `true` | Mount `/health` and `/healthz`. |
| `playback_signing_key` | `None` | If set, `/play/{ident}` requires `?sig=` signed by this key. |

## Auth

`AuthConfig::required(key)` applies to Stremio routes. `AuthConfig::disabled()` skips addon auth, useful for tests or public addons.

Accepted auth locations, in precedence order:

1. Encoded config JSON: `/{%7B%22authKey%22%3A%22secret%22%7D}/manifest.json`
2. Path key: `/u/secret/manifest.json`
3. Query string: `?authKey=secret`
4. Query string alias: `?key=secret`
5. Bearer header: `Authorization: Bearer secret`
6. Header: `X-Addon-Auth: secret`

Key comparisons use constant-time equality. `AuthConfig` redacts the configured key from `Debug`.

Health routes are intentionally unauthenticated.

## Encoded Config

Stremio supports a configuration object encoded as a path segment:

```json
{"authKey":"secret","enableSearch":true}
```

The core decodes this into:

```rust
pub struct UserConfig {
    pub auth_key: Option<String>,
    pub enable_search: Option<bool>,
    pub extra: serde_json::Map<String, serde_json::Value>,
}
```

Unknown fields are preserved in `extra`, so provider adapters can read custom fields without changing the core model.

## Stream Requests

GET stream routes call:

```rust
async fn stream(ctx, content_type, id) -> Result<StreamResponse, AddonError>
```

POST stream routes call:

```rust
async fn stream_request(ctx, request) -> Result<StreamResponse, AddonError>
```

The default `stream_request` implementation forwards to `stream` when `type` and `id` are present. Override it for Hellspy-style requests that use `name`, `episode`, `year`, or custom body fields.

POST body shape:

```json
{
  "type": "movie",
  "id": "tt0111161",
  "name": "The Shawshank Redemption",
  "year": "1994",
  "episode": null
}
```

## Stream Responses

`Stream` supports the common Stremio transport shapes:

- `url`: direct playable URL or addon playback URL.
- `externalUrl`: external page.
- `infoHash` plus `sources`: torrent/P2P style.

It also has typed provider fields used by local addons:

- `quality`
- `behaviorHints.countryWhitelist`
- `behaviorHints.bingeGroup`
- `behaviorHints.videoSize`
- `behaviorHints.filename`

Provider-specific fields can be added through `extra` maps.

## Metadata Lookups

`metadata` provides a small TMDB/Cinemeta resolver:

```rust
use stremio_addon_core::{MetadataClient, MetadataConfig};

let metadata = MetadataClient::new(MetadataConfig {
    tmdb_api_key: std::env::var("TMDB_API_KEY").ok(),
    timeout_seconds: 10,
    ..MetadataConfig::default()
});

let info = metadata.lookup_imdb("movie", "tt0111161").await?;
let episode = metadata.lookup_imdb("series", "tt0903747:1:2").await?;
let tmdb = metadata.lookup_tmdb_id("movie", "278").await?;
```

Resolution behavior:

- Uses TMDB first when `tmdb_api_key` is set.
- Falls back to Cinemeta for IMDb IDs when TMDB returns no usable result.
- Uses Cinemeta only when no TMDB key is configured.
- Preserves `season` and `episode` from Stremio IDs like `tt123:1:2`.
- Returns `TitleInfo` with Czech/primary title, Slovak title, English title, original title, year, type, season, and episode.

The core performs metadata HTTP requests, but it does not cache metadata. Provider apps should add caching around `MetadataClient` if needed.

## Search Query Generation

`search` builds provider-friendly query variants from `TitleInfo`:

```rust
use stremio_addon_core::{build_search_queries, QueryInput, QueryProfile};

let input = QueryInput::from_title_info(&info);
let webshare_queries = build_search_queries(QueryProfile::Webshare, &input);
let hellspy_queries = build_search_queries(QueryProfile::Hellspy, &input);
```

Profiles:

- `Webshare`: current Webshare query shape.
  - Movies: each title, plus each title with year.
  - Series: `S01E02` and `01x02` variants for each title.
- `Hellspy`: current Hellspy rewrite query shape.
  - Movies: year suffix, simplified title, raw title, dotted title.
  - Series: `S01E02`, `01x02`, dotted season/episode, episode-only fallback.

Providers still execute provider searches and normalize raw provider rows into core `SearchResult` values.

## Search Result Ranking

`ranking` scores normalized provider results against the metadata returned by TMDB/Cinemeta before cards are built:

```rust
use stremio_addon_core::{rank_results, RankingOptions, RankingProfile, SearchResult};

let ranked = rank_results(
    &info,
    vec![SearchResult {
        id: "provider-file-id".to_string(),
        title: "Movie.2024.1080p.mkv".to_string(),
        parsed_title: Some("Movie".to_string()),
        filename: Some("Movie.2024.1080p.mkv".to_string()),
        year: Some(2024),
        size_bytes: Some(2_000_000_000),
        positive_votes: Some(12),
        negative_votes: Some(1),
        quality_rank: Some(3),
        query_index: Some(0),
        ..SearchResult::default()
    }],
    &RankingOptions::for_profile(RankingProfile::Webshare),
);
```

Profiles:

- `Webshare`: title-first ranking with votes and size as small tie-breakers.
- `Hellspy`: filename-first ranking with query order as a stronger signal.
- `Balanced`: generic title and filename blend for new adapters.

Ranking behavior:

- Filters protected rows by default.
- Filters results with a year outside the configured tolerance.
- Requires exact season/episode matches for series when metadata has episode information.
- Filters episode-like movie false positives for Webshare-style ranking.
- Preserves `strong_match` and `weak_match` flags for card formatting.
- Uses optional vote ratio and quality rank as small tie-breakers.

The recommended provider pipeline is:

1. Resolve `TitleInfo` with `MetadataClient`.
2. Build queries with `build_search_queries`.
3. Execute provider searches.
4. Normalize raw provider rows into `SearchResult`.
5. Rank and filter with `rank_results`.
6. Convert ranked rows to `StreamCardInput` and build `Stream` cards.

## Stream Card Formatting

`cards` converts common provider presentation data into `Stream` values:

```rust
use stremio_addon_core::{stream_card, CardProfile, StreamCardInput};

let stream = stream_card(CardProfile::Webshare, StreamCardInput {
    provider: "Webshare".to_string(),
    provider_url: Some("https://addon/play/file-id?sig=...".to_string()),
    filename: Some("Movie.2024.1080p.mkv".to_string()),
    quality: Some("1080p".to_string()),
    size_bytes: Some(2_000_000_000),
    positive_votes: Some(10),
    negative_votes: Some(0),
    strong_match: true,
    ..StreamCardInput::default()
});
```

Profiles:

- `Webshare`: filename, language, votes, decimal file size, check mark for strong match.
- `Hellspy`: `Hellspy\nCZ 720p` style name, CZ/EN audio detection, binary size, country whitelist.
- `Compact`: generic one-line card useful for simple providers.

The helpers are intentionally presentation-only. They should run after metadata lookup, query generation, provider search, and ranking. They do not resolve playback URLs.

## Playback Redirects

Use playback routes when stream URLs should point back to the addon instead of exposing provider tokens or expensive direct URLs.

Typical private provider flow:

1. Provider search returns stream URL `/play/{ident}?authKey=...&sig=...`.
2. Core validates addon auth.
3. If `RouterOptions.playback_signing_key` is set, core verifies `sig`.
4. Core calls `AddonAdapter::playback(ctx, ident)`.
5. Adapter resolves the real provider URL.
6. Core redirects with `Cache-Control: max-age={seconds}, must-revalidate, proxy-revalidate`.

Create signatures with `SignedPlayback`:

```rust
use stremio_addon_core::SignedPlayback;

let payload = SignedPlayback::new("provider-file-id", 18_000);
let sig = payload.sign(b"redirect-signing-key")?;
```

If signing is enabled, the signed payload `ident` must match the `/play/{ident}` path.

For providers that already return safe direct playable URLs, disable playback routes:

```rust
RouterOptions {
    playback_routes: false,
    ..RouterOptions::default()
}
```

## Manifest Compatibility

`Manifest.resources` accepts both official Stremio shapes:

```json
"resources": ["stream"]
```

and:

```json
"resources": [
  { "name": "stream", "types": ["movie", "series"], "idPrefixes": ["tt"] }
]
```

`Manifest.extra` can carry fields that are not modeled explicitly, such as `stremioAddonsConfig`.

## Error Behavior

Errors are JSON:

```json
{"error":"..."}
```

Status mapping:

| Error | Status |
| --- | --- |
| auth failure | `401` |
| invalid config or bad request | `400` |
| provider/playback failure | `500` |

Provider adapters should return empty Stremio responses for normal no-result cases, not errors:

```rust
Ok(StreamResponse::default())
Ok(CatalogResponse::default())
Ok(MetaResponse::default())
```

## Provider Design Guidance

Keep these in provider or utility crates:

- Provider HTTP clients.
- Login/session/token caching.
- Filename parsing.
- Cache storage.
- Provider-specific ranking overrides that do not fit the built-in profiles.
- Provider-specific stream-card formatting that does not fit the built-in card profiles.

This keeps the core stable enough to reuse across Webshare, Hellspy, Sosac, and future provider adapters.
