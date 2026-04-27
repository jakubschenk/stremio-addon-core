use crate::{Stream, StreamBehaviorHints};

#[derive(Clone, Debug, Default)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardProfile {
    Webshare,
    Hellspy,
    Compact,
}

pub fn stream_card(profile: CardProfile, input: StreamCardInput) -> Stream {
    match profile {
        CardProfile::Webshare => webshare_card(input),
        CardProfile::Hellspy => hellspy_card(input),
        CardProfile::Compact => compact_card(input),
    }
}

pub fn webshare_card(input: StreamCardInput) -> Stream {
    let quality = input.quality.clone().unwrap_or_default();
    let filename = input.filename.clone().unwrap_or_default();
    let mut description = filename.clone();
    if let Some(language) = input.language.as_deref().filter(|value| !value.is_empty()) {
        description.push_str(&format!("\n🌐 {language}"));
    }
    if input.positive_votes.is_some() || input.negative_votes.is_some() {
        description.push_str(&format!(
            "\n👍 {} 👎 {}",
            input.positive_votes.unwrap_or_default(),
            input.negative_votes.unwrap_or_default()
        ));
    }
    if let Some(size) = input.size_bytes {
        description.push_str(&format!("\n💾 {}", format_decimal_size(size)));
    }

    Stream {
        ident: input.ident,
        name: Some(format!(
            "{}{} {}",
            input.provider,
            if input.strong_match { " ✅" } else { "" },
            quality
        )),
        quality: empty_to_none(quality),
        url: input.provider_url,
        description: Some(description),
        behavior_hints: Some(StreamBehaviorHints {
            binge_group: Some(input.binge_group.unwrap_or_else(|| {
                format!("{}|{}", input.provider, input.language.unwrap_or_default())
            })),
            video_size: input.size_bytes,
            filename: input.filename,
            country_whitelist: input.country_whitelist,
            extra: Default::default(),
        }),
        ..Stream::default()
    }
}

pub fn hellspy_card(input: StreamCardInput) -> Stream {
    let quality = input
        .quality
        .clone()
        .unwrap_or_else(|| "original".to_string());
    let audio = input.audio.clone().unwrap_or_else(|| {
        detect_audio(input.filename.as_deref())
            .description
            .to_string()
    });
    let badge = detect_audio(input.filename.as_deref()).badge;
    let filename = input
        .filename
        .clone()
        .unwrap_or_else(|| "Unknown file".to_string());
    let size = input
        .size_bytes
        .map(format_binary_size)
        .unwrap_or_else(|| "Unknown size".to_string());
    let description = format!("📺 {quality}  •  💾 {size}\n🌐 {audio}\n📄 {filename}");

    Stream {
        ident: input.ident,
        name: Some(format!("{}\n{} {}", input.provider, badge, quality)),
        title: Some(description.clone()),
        quality: Some(quality.clone()),
        url: input.provider_url,
        description: Some(description),
        behavior_hints: Some(StreamBehaviorHints {
            country_whitelist: input
                .country_whitelist
                .or_else(|| Some(vec!["cze".to_string()])),
            binge_group: Some(input.binge_group.unwrap_or_else(|| {
                format!(
                    "{}-{}",
                    input.provider.to_lowercase(),
                    normalize_quality(&quality)
                )
            })),
            filename: input.filename,
            video_size: input.size_bytes,
            extra: Default::default(),
        }),
        ..Stream::default()
    }
}

pub fn compact_card(input: StreamCardInput) -> Stream {
    let mut details = Vec::new();
    if let Some(quality) = input.quality.as_deref() {
        details.push(format!("📺 {quality}"));
    }
    if let Some(size) = input.size_bytes {
        details.push(format!("💾 {}", format_decimal_size(size)));
    }
    if let Some(language) = input.language.as_deref() {
        details.push(format!("🌐 {language}"));
    }

    Stream {
        ident: input.ident,
        name: Some(input.provider),
        quality: input.quality,
        url: input.provider_url,
        description: (!details.is_empty()).then(|| details.join("  •  ")),
        behavior_hints: Some(StreamBehaviorHints {
            country_whitelist: input.country_whitelist,
            binge_group: input.binge_group,
            video_size: input.size_bytes,
            filename: input.filename,
            extra: Default::default(),
        }),
        ..Stream::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioLabel {
    pub badge: &'static str,
    pub description: &'static str,
}

pub fn detect_audio(filename: Option<&str>) -> AudioLabel {
    let has_cz = filename.is_some_and(filename_has_czech_audio_signal);
    let has_en = filename.is_some_and(filename_has_english_audio_signal);
    match (has_cz, has_en) {
        (true, true) => AudioLabel {
            badge: "CZ/EN",
            description: "CZ/EN audio",
        },
        (true, false) => AudioLabel {
            badge: "CZ",
            description: "CZ audio",
        },
        (false, true) => AudioLabel {
            badge: "EN",
            description: "EN audio",
        },
        (false, false) => AudioLabel {
            badge: "UNK",
            description: "Unknown audio",
        },
    }
}

pub fn format_decimal_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    format_size(bytes, 1000.0, &UNITS)
}

pub fn format_binary_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MB", "GB", "TB"];
    format_size(bytes, 1024.0, &UNITS)
}

fn format_size(bytes: u64, base: f64, units: &[&str]) -> String {
    let mut value = bytes as f64;
    let mut unit = units[0];
    for next_unit in units.iter().skip(1) {
        if value < base {
            break;
        }
        value /= base;
        unit = next_unit;
    }

    if value >= 10.0 || unit == "B" {
        format!("{value:.0} {unit}")
    } else {
        let formatted = format!("{value:.2}");
        format!(
            "{} {unit}",
            formatted.trim_end_matches('0').trim_end_matches('.')
        )
    }
}

fn empty_to_none(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn filename_has_czech_audio_signal(title: &str) -> bool {
    let normalized = title.to_lowercase();
    let spaced = normalized
        .chars()
        .map(|char| if char.is_alphanumeric() { char } else { ' ' })
        .collect::<String>();
    let tokens = spaced.split_whitespace().collect::<Vec<_>>();

    tokens.iter().any(|token| {
        matches!(
            *token,
            "cz" | "cs" | "cze" | "czech" | "cesky" | "ceske" | "ceska" | "dabing" | "dab"
        ) || token.starts_with("česk")
            || token.starts_with("cesk")
    }) || normalized.contains("+cz")
        || normalized.contains("cz+")
        || normalized.contains("czdab")
        || normalized.contains("cz-dab")
        || normalized.contains("cz dab")
        || normalized.contains("cz.dab")
}

fn filename_has_english_audio_signal(title: &str) -> bool {
    let normalized = title.to_lowercase();
    let spaced = normalized
        .chars()
        .map(|char| if char.is_alphanumeric() { char } else { ' ' })
        .collect::<String>();
    spaced
        .split_whitespace()
        .any(|token| matches!(token, "en" | "eng" | "english" | "anglicky" | "angl"))
        || normalized.contains("+en")
        || normalized.contains("en+")
}

fn normalize_quality(quality: &str) -> String {
    quality
        .chars()
        .filter(|char| char.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_size_matches_webshare_style() {
        assert_eq!(format_decimal_size(30_000_000_000), "30 GB");
        assert_eq!(format_decimal_size(2_500_000_000), "2.5 GB");
        assert_eq!(format_decimal_size(180_000_000), "180 MB");
    }

    #[test]
    fn detects_audio_from_filename() {
        assert_eq!(
            detect_audio(Some("movie.CZ.EN.mkv")),
            AudioLabel {
                badge: "CZ/EN",
                description: "CZ/EN audio"
            }
        );
        assert_eq!(detect_audio(Some("movie.CZdab.mkv")).badge, "CZ");
        assert_eq!(detect_audio(Some("movie.eng.mkv")).badge, "EN");
    }

    #[test]
    fn webshare_card_matches_existing_shape() {
        let stream = webshare_card(StreamCardInput {
            provider: "Webshare".to_string(),
            provider_url: Some("https://addon/play/1".to_string()),
            filename: Some("The.Relic.1999.2160p.mkv".to_string()),
            quality: Some("2160p".to_string()),
            size_bytes: Some(30_000_000_000),
            positive_votes: Some(15),
            negative_votes: Some(0),
            strong_match: true,
            ..StreamCardInput::default()
        });

        assert_eq!(stream.name.as_deref(), Some("Webshare ✅ 2160p"));
        assert!(stream.description.unwrap().contains("💾 30 GB"));
    }

    #[test]
    fn hellspy_card_matches_existing_shape() {
        let stream = hellspy_card(StreamCardInput {
            provider: "Hellspy".to_string(),
            provider_url: Some("https://cdn.example/720.mp4".to_string()),
            filename: Some("movie.CZ.mkv".to_string()),
            quality: Some("720p".to_string()),
            size_bytes: Some(2_147_483_648),
            ..StreamCardInput::default()
        });

        assert_eq!(stream.name.as_deref(), Some("Hellspy\nCZ 720p"));
        assert_eq!(
            stream.title.as_deref(),
            Some("📺 720p  •  💾 2 GB\n🌐 CZ audio\n📄 movie.CZ.mkv")
        );
    }
}
