use serde_json::Value;
use std::collections::HashSet;

use crate::Stream;

/// Return stream payload size for ranking.
pub fn stream_video_size(stream: &Stream) -> u64 {
    stream
        .behavior_hints
        .as_ref()
        .and_then(|hints| hints.video_size)
        .unwrap_or_default()
}

/// Read localization rank from the filename extra hint.
pub fn stream_filename_localization_rank(
    stream: &Stream,
    localization_rank_fn: impl Fn(&str) -> i32,
) -> i32 {
    stream
        .behavior_hints
        .as_ref()
        .and_then(|hints| hints.filename.as_deref())
        .map_or(0, localization_rank_fn)
}

/// Read localization rank from extra hints and fall back to stream name.
pub fn stream_extra_localization_rank_or_name(
    stream: &Stream,
    localization_rank_fn: impl Fn(&str) -> i32,
) -> i32 {
    let localization_rank_fn = &localization_rank_fn;

    stream
        .behavior_hints
        .as_ref()
        .and_then(|hints| parse_localization_rank_from_extra(hints.extra.get("localizationRank")))
        .unwrap_or_else(|| stream.name.as_deref().map_or(0, localization_rank_fn))
}

fn parse_localization_rank_from_extra(value: Option<&Value>) -> Option<i32> {
    let value = value?;
    match value {
        Value::Number(number) => number
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .or_else(|| number.as_u64().and_then(|value| i32::try_from(value).ok())),
        Value::String(raw) => raw.parse().ok(),
        _ => None,
    }
}

/// Sort streams by quality, size, and localization preference.
pub fn sort_streams_by_quality_size_localization<Q, L>(
    streams: &mut [Stream],
    quality_rank_fn: Q,
    localization_rank_fn: L,
) where
    Q: Fn(&Stream) -> i32,
    L: Fn(&Stream) -> i32,
{
    streams.sort_by(|a, b| {
        quality_rank_fn(b)
            .cmp(&quality_rank_fn(a))
            .then_with(|| stream_video_size(b).cmp(&stream_video_size(a)))
            .then_with(|| localization_rank_fn(b).cmp(&localization_rank_fn(a)))
    });
}

/// Remove duplicates and empty URLs. Missing URLs are dropped.
///
/// Missing `url` values are removed, because selection code cannot route them.
pub fn dedupe_streams_by_url(streams: &mut Vec<Stream>) {
    let mut seen: HashSet<String> = HashSet::new();
    streams.retain(|stream| {
        let Some(url) = stream.url.as_deref() else {
            return false;
        };
        seen.insert(url.to_string())
    });
}

/// Keep only up to `limit` entries if configured.
pub fn cap_streams(streams: &mut Vec<Stream>, limit: usize) {
    if streams.len() > limit {
        streams.truncate(limit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_stream(url: &str, quality: i32, size: u64, filename: &str, name: &str) -> Stream {
        Stream {
            url: Some(url.to_string()),
            quality: Some(format!("{quality}p")),
            behavior_hints: Some(crate::StreamBehaviorHints {
                video_size: Some(size),
                filename: Some(filename.to_string()),
                extra: Default::default(),
                ..Default::default()
            }),
            name: Some(name.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn dedupe_streams_by_url_drops_empty_and_duplicates() {
        let mut streams = vec![
            sample_stream("https://example.com/1", 1080, 10, "Movie CZ", "name"),
            sample_stream("https://example.com/1", 720, 20, "Movie EN", "name"),
            sample_stream("https://example.com/2", 720, 5, "Movie CZ", "name"),
            Stream {
                quality: Some("480p".to_string()),
                name: Some("no url".to_string()),
                behavior_hints: Some(crate::StreamBehaviorHints {
                    video_size: Some(1),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ];

        dedupe_streams_by_url(&mut streams);

        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].url.as_deref(), Some("https://example.com/1"));
        assert_eq!(streams[1].url.as_deref(), Some("https://example.com/2"));
    }

    #[test]
    fn sort_streams_by_quality_and_video_size_and_localization() {
        let mut streams = vec![
            sample_stream("https://example.com/low", 720, 100, "Movie EN", "Name EN"),
            sample_stream("https://example.com/high", 1080, 10, "Movie CZ", "CZ"),
            sample_stream(
                "https://example.com/high-cz-large",
                1080,
                20,
                "Movie CZ",
                "CZ",
            ),
            sample_stream("https://example.com/mixed", 1080, 5, "Movie EN", "EN"),
        ];

        sort_streams_by_quality_size_localization(
            &mut streams,
            |stream| {
                stream
                    .quality
                    .as_deref()
                    .and_then(|quality| quality.trim_end_matches('p').parse::<i32>().ok())
                    .unwrap_or_default()
            },
            |stream| {
                stream_filename_localization_rank(stream, |value| {
                    if value.to_lowercase().contains("cz") {
                        10
                    } else {
                        1
                    }
                })
            },
        );

        let order = streams
            .iter()
            .filter_map(|stream| stream.url.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            vec![
                "https://example.com/high-cz-large",
                "https://example.com/high",
                "https://example.com/mixed",
                "https://example.com/low",
            ]
        );
    }

    #[test]
    fn extra_localization_rank_falls_back_to_name_not_filename() {
        let stream = sample_stream(
            "https://example.com/mixed",
            1080,
            1,
            "Movie CZ",
            "Provider EN",
        );

        let rank = stream_extra_localization_rank_or_name(&stream, |value| {
            if value.to_lowercase().contains("cz") {
                10
            } else if value.to_lowercase().contains("en") {
                1
            } else {
                0
            }
        });

        assert_eq!(rank, 1);
    }

    #[test]
    fn cap_streams_limits_length() {
        let mut streams = vec![
            sample_stream("https://example.com/1", 720, 10, "", ""),
            sample_stream("https://example.com/2", 720, 10, "", ""),
            sample_stream("https://example.com/3", 720, 10, "", ""),
        ];
        cap_streams(&mut streams, 2);
        assert_eq!(streams.len(), 2);
    }
}
