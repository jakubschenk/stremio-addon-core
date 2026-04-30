# Continuation Review

Date: 2026-04-27

Scope reviewed:

- `src/ranking.rs`
- `src/metadata.rs`
- `src/search.rs`
- `README.md`
- `SPEC.md`

Original verified state:

- `cargo fmt --check` passed in `/home/abuk/stremio/plugins/rust/stremio-addon-core`.
- `cargo test` passed in `/home/abuk/stremio/plugins/rust/stremio-addon-core`: 40 tests.
- `cargo test --workspace` passed in `/home/abuk/stremio/plugins/rust/stremio-webshare-rs`.
- `/home/abuk/stremio/plugins/rust/stremio-addon-core` is not a git repo, so no `git diff` or `git status` was available there.

## Summary

The addon core now owns the reusable pipeline through metadata lookup, search query generation, normalized result ranking, and card formatting. Provider apps are still responsible for login/session handling, provider HTTP search execution, mapping raw rows into `SearchResult`, caching, and playback URL resolution.

The ranking addition is reasonable as a first shared base, but it should get a second pass before Webshare is fully ported onto it. The main risks are heuristic behavior, missing provider-specific fields, and metadata HTTP robustness.

## Findings

### Medium: ranking thresholds are heuristic and need fixture-based validation

`RankingOptions::for_profile` currently uses hard-coded thresholds:

- Webshare: strong `0.5`, weak `0.3`
- Hellspy: strong `0.45`, weak `0.2`
- Balanced: strong `0.5`, weak `0.3`

These pass unit tests, but the tests are synthetic. Before porting the provider, capture representative raw search result fixtures from the current Node Webshare addon and Hellspy rewrite, normalize them to `SearchResult`, and assert the expected ordering/filtering. This is the most important next review item because ranking bugs directly affect stream quality and false positives.

Suggested tests:

- Czech title vs English title vs original title.
- Movie with remake years, where wrong year should be filtered.
- Movie filename containing `S01E02`, where it should be filtered unless the movie title itself contains `episode` or `part`.
- Series result with correct title but wrong episode.
- Search result with only filename populated and no parsed title.
- Protected Webshare rows.
- Tie cases where `query_index: Some(0)` beats `Some(1)` and both beat `None`.

### Medium: `SearchResult` may need negative votes and provider quality signals

`SearchResult` currently includes `positive_votes`, `size_bytes`, and `query_index`, but not:

- `negative_votes`
- parsed `quality`
- provider-specific confidence
- language/audio signal

This is acceptable for the first base, but Webshare previously used both positive and negative votes in card display, and provider ranking may want to demote poor vote ratios. Consider adding optional `negative_votes` and maybe `quality_rank` before adapters are locked in.

### Medium: metadata HTTP client has no default timeout

`MetadataClient::new` uses `reqwest::Client::new()`. That means metadata lookups can wait on reqwest defaults rather than a deliberate addon timeout. Since stream requests are user-facing, add either:

- `MetadataConfig.timeout_seconds`, applied in `MetadataClient::new`, or
- document that provider apps should inject a configured client with `MetadataClient::with_client`.

The safer default is a configured timeout in core.

### Low: ranking title cleaner removes subtitle words with case-specific replacements

`clean_title` removes `subtitles` and `titulky` through fixed case variants. It now handles Unicode lowercasing after that, but the removal should probably lowercase first or use a token-based stopword pass. This matters for mixed-case or localized subtitle markers.

### Low: Hellspy simplified series query is deduped out for simple titles

`hellspy_queries(Some("series"), "English Title", ...)` generates a simplified title equal to the full title, then `dedupe_non_empty` removes the duplicate. The test expects four queries, not five. This is probably correct behavior, but when validating against the current Hellspy rewrite, check whether duplicate queries were intentionally sent or just incidental.

## Follow-Up Applied On 2026-04-28

- Added `MetadataConfig.timeout_seconds`; `MetadataClient::new` now builds a reqwest client with a default 10 second timeout.
- Added `SearchResult.negative_votes` and `SearchResult.quality_rank`.
- Ranking now uses positive minus negative votes, optional quality rank, and a corrected query-index tie-breaker where unknown query order sorts last.
- Added ranking tests for metadata title candidates, filename-only matches, protected rows, episode-like movie false positives, known query order vs unknown order, vote/quality tie-breaking, and case-insensitive subtitle marker cleanup.
- Updated README and SPEC for the expanded ranking and metadata contracts.

## Remaining Recommended Next Steps

1. Add real fixture-based ranking tests from captured Webshare and Hellspy provider rows before wiring Webshare to `rank_results`.
2. Port Webshare search normalization into `SearchResult`.
3. Build cards from `RankedResult.score.strong_match`.
4. Re-run compatibility checks against Webshare, Hellspy, and Sosac route shapes after the adapter uses the core pipeline.

## Current Core Pipeline Contract

Provider stream flow should be:

1. Resolve `TitleInfo` with `MetadataClient`.
2. Select/build search titles with `select_search_titles` and `build_search_queries`.
3. Execute provider search APIs.
4. Normalize rows into `SearchResult`.
5. Call `rank_results`.
6. Build `StreamCardInput` from ranked rows.
7. Build `Stream` with `stream_card`.
8. Use signed `/play/{ident}` redirects when raw provider URLs or session tokens should not be exposed.
