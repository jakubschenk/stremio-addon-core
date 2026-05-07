use crate::ranking::clean_title;
use regex::Regex;
use std::sync::LazyLock;

const UNKNOWN_QUALITY: &str = "unknown";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedFilename {
    pub title: Option<String>,
    pub year: Option<i32>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub quality: Option<String>,
    pub quality_rank: Option<i32>,
    pub language: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalizationInfo {
    pub badge: &'static str,
    pub description: &'static str,
}

struct FilenameView {
    normalized: String,
    spaced: String,
}

impl FilenameView {
    fn new(value: &str) -> Self {
        let normalized = value.to_lowercase();
        let spaced = normalized
            .chars()
            .map(|char| if char.is_alphanumeric() { char } else { ' ' })
            .collect::<String>();
        Self { normalized, spaced }
    }

    fn has_token(&self, predicate: impl FnMut(&str) -> bool) -> bool {
        self.spaced.split_whitespace().any(predicate)
    }

    fn contains_any(&self, needles: &[&str]) -> bool {
        needles
            .iter()
            .any(|needle| self.normalized.contains(needle))
    }
}

static YEAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[^\p{L}\p{N}])((?:19|20)\d{2})(?:[^\p{L}\p{N}]|$)").expect("valid regex")
});

static SEASON_EPISODE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?iu)(?:^|[^\p{L}\p{N}])(?:(?:s|season\s*)(\d{1,2})[^\p{L}\p{N}]*(?:e|ep|episode\s*)(\d{1,3})|(\d{1,2})[^\p{L}\p{N}]*(?:x|×)[^\p{L}\p{N}]*(\d{1,3}))(?:[^\p{L}\p{N}]|$)")
        .expect("valid regex")
});
static EPISODE_ONLY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?iu)(?:^|[^\p{L}\p{N}])(?:e|ep|episode|#|part|pt)[^\p{L}\p{N}]*(\d{1,3})(?:[^\p{L}\p{N}]|$)").expect("valid regex")
});

static QUALITY_SIZE_RE: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        ("2160", "2160p"),
        ("1080", "1080p"),
        ("720", "720p"),
        ("576", "576p"),
        ("480", "480p"),
    ]
    .into_iter()
    .map(|(size, label)| {
        (
            Regex::new(&format!(r"\b{}p?\b", size)).expect("valid regex"),
            label,
        )
    })
    .collect()
});

static QUALITY_LABELS: &[QualityLabel] = &[
    QualityLabel {
        label: "2160p",
        needles: &["2160", "4k", "uhd"],
    },
    QualityLabel {
        label: "1080p",
        needles: &["1080", "full hd", "fhd"],
    },
    QualityLabel {
        label: "WEB-DL",
        needles: &["web-dl", "webdl"],
    },
    QualityLabel {
        label: "WEBRip",
        needles: &["webrip"],
    },
    QualityLabel {
        label: "BluRay",
        needles: &["bluray", "blu-ray", "brrip", "bdrip"],
    },
    QualityLabel {
        label: "HDRip",
        needles: &["hdrip"],
    },
    QualityLabel {
        label: "DVDRip",
        needles: &["dvdrip"],
    },
    QualityLabel {
        label: "KinoRip",
        needles: &["kinorip", "kino rip"],
    },
    QualityLabel {
        label: "HDCAM",
        needles: &["hdcam", "hd cam"],
    },
    QualityLabel {
        label: "CAMRip",
        needles: &["camrip"],
    },
    QualityLabel {
        label: "Telesync",
        needles: &["telesync", "hdts", "hd-ts"],
    },
];

#[derive(Clone, Copy)]
struct QualityLabel {
    label: &'static str,
    needles: &'static [&'static str],
}

/// Parse filename metadata commonly used by adapters in one pass.
pub fn parse_filename(filename: &str) -> ParsedFilename {
    let view = FilenameView::new(filename);
    let year = parse_year(filename);
    let (season, episode) = parse_season_episode_parts(filename);
    let quality = infer_quality_label(&view);
    let quality_rank = quality.as_deref().and_then(release_quality_rank);

    ParsedFilename {
        title: parsed_title(filename, year, season),
        year,
        season,
        episode,
        quality,
        quality_rank,
        language: Some(localization_badge(filename)),
    }
}

/// Parse a 4-digit year from a filename.
pub fn parse_year(filename: &str) -> Option<i32> {
    YEAR_RE
        .captures(filename)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
}

/// Parse season+episode, where both parts are present, or return None.
pub fn parse_season_episode(filename: &str) -> Option<(u32, u32)> {
    let (season, episode) = parse_season_episode_parts(filename);
    match (season, episode) {
        (Some(season), Some(episode)) => Some((season, episode)),
        _ => None,
    }
}

/// Parse season+episode as separate options, preserving Webshare compatibility.
pub fn parse_season_episode_parts(filename: &str) -> (Option<u32>, Option<u32>) {
    if let Some(caps) = SEASON_EPISODE_RE.captures(filename) {
        if let (Some(season), Some(episode)) = (caps.get(1), caps.get(2)) {
            return (season.as_str().parse().ok(), episode.as_str().parse().ok());
        }
        if let (Some(season), Some(episode)) = (caps.get(3), caps.get(4)) {
            let episode = episode.as_str().parse().ok();
            if matches!(episode, Some(264..=266)) {
                return (None, None);
            }
            return (season.as_str().parse().ok(), episode);
        }
    }

    if let Some(caps) = EPISODE_ONLY_RE.captures(filename) {
        return (
            Some(1),
            caps.get(1).and_then(|value| value.as_str().parse().ok()),
        );
    }

    (None, None)
}

/// Parse a human-friendly localization badge from title tokens.
pub fn localization_badge(title: &str) -> String {
    localization_info(Some(title)).badge.to_string()
}

/// Compute localization preference score.
pub fn localization_rank(title: &str) -> i32 {
    localization_rank_with_mode(title, false)
}

/// Preserve Webshare-style release quality ranking labels.
pub fn release_quality_rank(label: &str) -> Option<i32> {
    match label.to_ascii_lowercase().as_str() {
        "2160p" => Some(400),
        "1080p" => Some(300),
        "720p" => Some(200),
        "576p" => Some(150),
        "480p" => Some(100),
        "web-dl" => Some(80),
        "webrip" => Some(80),
        "bluray" => Some(80),
        "brrip" => Some(80),
        "bdrip" => Some(80),
        "hdrip" => Some(70),
        "dvdrip" => Some(60),
        "kinorip" => Some(40),
        "hdcam" => Some(30),
        "camerip" => Some(30),
        "camrip" => Some(30),
        "telesync" => Some(20),
        "hdts" => Some(20),
        "original" => Some(1),
        _ => {
            if is_unknown_quality_value(label) || label == UNKNOWN_QUALITY {
                Some(0)
            } else {
                None
            }
        }
    }
}

/// Parse quality into rank by encoded resolution height.
pub fn height_quality_rank(label: &str) -> i32 {
    let normalized = label.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return 0;
    }
    if let Ok(height) = normalized.parse::<i32>() {
        return height;
    }
    if normalized.ends_with('p') {
        let raw = normalized.trim_end_matches('p');
        if let Ok(height) = raw.parse::<i32>() {
            return height;
        }
    }
    if normalized.contains("2160") || normalized.contains("4k") || normalized.contains("uhd") {
        return 2160;
    }
    if normalized.contains("1440") {
        return 1440;
    }
    if normalized.contains("1080") || normalized.contains("full hd") || normalized.contains("fhd") {
        return 1080;
    }
    if normalized.contains("720") {
        return 720;
    }
    if normalized.contains("576") {
        return 576;
    }
    if normalized.contains("480") {
        return 480;
    }
    if normalized.contains("360") {
        return 360;
    }
    if normalized.contains("hd") {
        return 540;
    }
    1
}

/// Convert conversion quality values from an API into an internal quality label.
pub fn quality_label_from_conversion(conversion_quality: &str, title: &str) -> String {
    let trimmed = conversion_quality.trim();
    let conversion_view = FilenameView::new(trimmed);
    let title_view = FilenameView::new(title);
    if trimmed.eq_ignore_ascii_case("original") {
        return "original".to_string();
    }

    if is_unknown_value(trimmed) {
        return infer_quality_label(&title_view).unwrap_or_else(unknown_quality);
    }

    if let Ok(height) = trimmed.parse::<u32>() {
        if height > 0 {
            return format!("{height}p");
        }
    }

    infer_quality_label(&conversion_view)
        .or_else(|| infer_quality_label(&title_view))
        .unwrap_or_else(unknown_quality)
}

/// Reverse mapping for Webshare release quality ranks.
pub fn quality_from_release_rank(rank: i32) -> Option<String> {
    match rank {
        400 => Some("2160p".to_string()),
        300 => Some("1080p".to_string()),
        200 => Some("720p".to_string()),
        150 => Some("576p".to_string()),
        100 => Some("480p".to_string()),
        1 => Some("original".to_string()),
        _ => None,
    }
}

/// Rank mixed filename or stream quality text by resolution and release tier.
pub fn media_quality_rank(value: &str) -> i32 {
    let normalized = value.to_lowercase();
    if contains_any(
        &normalized,
        &[
            "kinorip", "kino rip", "camrip", "hdcam", "hd cam", "hd-ts", "hdts", "telesync", " ts ",
        ],
    ) {
        return 240;
    }
    if normalized.contains(" cam ")
        || normalized.starts_with("cam ")
        || normalized.ends_with(" cam")
    {
        return 200;
    }
    if contains_any(&normalized, &["2160", "4k", "uhd"]) {
        return 2160;
    }
    if normalized.contains("1440") {
        return 1440;
    }
    if contains_any(&normalized, &["1080", "full hd", "fhd"]) {
        return 1080;
    }
    if normalized.contains("720") {
        return 720;
    }
    if normalized.contains("576") {
        return 576;
    }
    if normalized.contains("480") {
        return 480;
    }
    if contains_any(
        &normalized,
        &[
            "web-dl", "webdl", "webrip", "bluray", "blu-ray", "brrip", "bdrip",
        ],
    ) {
        return 700;
    }
    if contains_any(&normalized, &["hdrip", "dvdrip"]) {
        return 600;
    }
    if normalized.contains("original") {
        return 1;
    }
    0
}

/// Detect whether parsed titles look too generic for ranking.
pub fn is_generic_parsed_title(title: &str) -> bool {
    let tokens = title
        .split_whitespace()
        .map(|token| token.trim_matches(|token_char: char| !token_char.is_alphanumeric()))
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    !tokens.is_empty()
        && tokens.iter().all(|token| {
            matches!(
                token.to_ascii_lowercase().as_str(),
                "file"
                    | "film"
                    | "movie"
                    | "video"
                    | "stream"
                    | "download"
                    | "webshare"
                    | "unknown"
                    | "sample"
                    | "episode"
                    | "part"
                    | "pt"
                    | "sd"
                    | "hd"
                    | "fhd"
                    | "uhd"
                    | "480p"
                    | "576p"
                    | "720p"
                    | "1080p"
                    | "2160p"
                    | "cz"
                    | "cs"
                    | "cze"
                    | "sk"
                    | "svk"
                    | "en"
                    | "eng"
                    | "dab"
                    | "dub"
                    | "dabing"
                    | "titulky"
                    | "tit"
                    | "sub"
                    | "subs"
                    | "avi"
                    | "mkv"
                    | "mov"
                    | "mp4"
                    | "webm"
            )
        })
}

/// Keep parsed title only when it is non-generic, else replace with matched query.
pub fn parsed_title_for_ranking(
    parsed_title: Option<String>,
    matched_query: Option<&str>,
) -> Option<String> {
    match parsed_title {
        Some(title) if is_generic_parsed_title(&title) => matched_query
            .filter(|query| !query.is_empty())
            .map(str::to_string)
            .or(Some(title)),
        other => other,
    }
}

fn parsed_title(filename: &str, year: Option<i32>, season: Option<u32>) -> Option<String> {
    let mut boundary = filename.len();
    let filename_lower = filename.to_lowercase();
    for needle in [
        year.map(|year| year.to_string()),
        season.map(|season| format!("s{season:02}")),
        season.map(|season| format!("{season:02}x")),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(index) = filename_lower.find(&needle.to_lowercase()) {
            boundary = boundary.min(index);
        }
    }

    let candidate = clean_title(&filename[..boundary]);
    (!candidate.is_empty()).then_some(candidate)
}

fn infer_quality_label(view: &FilenameView) -> Option<String> {
    if view.has_token(|token| token == "original") {
        return Some("original".to_string());
    }

    if let Some(label) = QUALITY_SIZE_RE
        .iter()
        .find(|(regex, _)| regex.is_match(&view.spaced))
        .map(|(_, label)| label.to_string())
    {
        return Some(label);
    }

    if let Some(entry) = QUALITY_LABELS
        .iter()
        .find(|entry| view.contains_any(entry.needles))
    {
        return Some(entry.label.to_string());
    }

    if view.has_token(is_unknown_quality_value) || has_dash_unknown_marker(&view.normalized) {
        return Some(UNKNOWN_QUALITY.to_string());
    }

    None
}

fn is_unknown_quality_value(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized.is_empty()
        || matches!(
            normalized.as_str(),
            "0" | "unknown" | "unk" | "n/a" | "na" | "null" | "none" | "-"
        )
}

fn is_unknown_value(value: &str) -> bool {
    is_unknown_quality_value(value)
}

fn has_dash_unknown_marker(value: &str) -> bool {
    value.contains(".-.") || value.contains(" - ") || value.contains("_-_")
}

fn unknown_quality() -> String {
    UNKNOWN_QUALITY.to_string()
}

fn has_czech_dub_signal(view: &FilenameView) -> bool {
    view.has_token(|token| {
        matches!(
            token,
            "czdub" | "czdab" | "czdabing" | "czechdub" | "ceskydabing"
        )
    }) || view.contains_any(&["czdab", "cz-dab", "cz dab", "cz.dab"])
        || (has_czech_language_marker(view)
            && view.has_token(|token| matches!(token, "dabing" | "dab" | "dub")))
        || view.has_token(|token| {
            (token.starts_with("česk") || token.starts_with("cesk"))
                && (view.spaced.contains("dabing") || view.spaced.contains("dab"))
        })
}

fn has_czech_subtitle_signal(view: &FilenameView) -> bool {
    has_czech_subtitle_signal_with_mode(view, false)
}

fn has_czech_subtitle_signal_lenient(view: &FilenameView) -> bool {
    has_czech_subtitle_signal_with_mode(view, true)
}

fn has_czech_subtitle_signal_with_mode(view: &FilenameView, allow_generic: bool) -> bool {
    view.has_token(|token| {
        matches!(token, "cztit" | "cztitulky" | "czsub" | "czsubs")
            || (allow_generic && matches!(token, "titulky" | "tit"))
    }) || view.contains_any(&[
        "cz tit",
        "cz.tit",
        "cz-tit",
        "cz_tit",
        "cz titulky",
        "ceske titulky",
        "české titulky",
        "české titul",
        "ceske.titul",
    ])
}

fn has_slovak_dub_signal(view: &FilenameView) -> bool {
    view.has_token(|token| {
        matches!(
            token,
            "skdab" | "skdabing" | "slovakdub" | "slovenskydabing"
        ) || token.starts_with("slovensk")
            && (view.spaced.contains("dabing") || view.spaced.contains("dab"))
    }) || view.contains_any(&["skdab", "sk-dab", "sk dab", "sk.dab"])
}

fn has_slovak_subtitle_signal(view: &FilenameView) -> bool {
    view.has_token(|token| matches!(token, "sktit" | "sktitulky" | "sksub" | "sksubs"))
        || view.contains_any(&[
            "sk tit",
            "sk.tit",
            "sk-tit",
            "sk_tit",
            "sk titulky",
            "slovenske titulky",
            "slovenské titulky",
            "slovenské titul",
            "slovenske.titul",
        ])
}

fn has_slovak_language_marker(view: &FilenameView) -> bool {
    view.has_token(|token| {
        matches!(
            token,
            "sk" | "svk" | "slovak" | "slovensky" | "slovenske" | "slovenska"
        ) || token.starts_with("slovensk")
    }) || view.contains_any(&["+sk", "sk+"])
}

fn has_czech_language_marker(view: &FilenameView) -> bool {
    view.has_token(|token| {
        matches!(
            token,
            "cz" | "cs" | "cze" | "czech" | "cesky" | "ceske" | "ceska"
        ) || token.starts_with("česk")
            || token.starts_with("cesk")
    }) || view.contains_any(&["+cz", "cz+"])
}

fn has_english_signal(view: &FilenameView) -> bool {
    view.has_token(|token| {
        matches!(
            token,
            "en" | "eng" | "english" | "anglicky" | "angl" | "entit" | "ensub" | "ensubs"
        )
    }) || view.contains_any(&["+en", "en+"])
}

pub fn localization_info(title: Option<&str>) -> LocalizationInfo {
    localization_info_with_mode(title, false)
}

/// Parse localization with Hellspy-compatible generic subtitle tokens.
pub fn localization_info_lenient(title: Option<&str>) -> LocalizationInfo {
    localization_info_with_mode(title, true)
}

/// Compute localization score with Hellspy-compatible generic subtitle tokens.
pub fn localization_rank_lenient(title: &str) -> i32 {
    localization_rank_with_mode(title, true)
}

fn localization_info_with_mode(
    title: Option<&str>,
    allow_generic_subtitles: bool,
) -> LocalizationInfo {
    let Some(title) = title else {
        return unknown_localization();
    };
    let view = FilenameView::new(title);
    let has_cz_dub = has_czech_dub_signal(&view);
    let has_cz_subs = if allow_generic_subtitles {
        has_czech_subtitle_signal_lenient(&view)
    } else {
        has_czech_subtitle_signal(&view)
    };
    let has_cz = has_cz_dub || has_cz_subs || has_czech_language_marker(&view);
    let has_sk_dub = has_slovak_dub_signal(&view);
    let has_sk_subs = has_slovak_subtitle_signal(&view);
    let has_sk = has_sk_dub || has_sk_subs || has_slovak_language_marker(&view);
    let has_en = has_english_signal(&view);

    if has_cz_dub && has_en {
        LocalizationInfo {
            badge: "CZ/EN",
            description: "CZ dabing / EN",
        }
    } else if has_cz_dub {
        LocalizationInfo {
            badge: "CZ",
            description: "CZ dabing",
        }
    } else if has_cz_subs && has_en {
        LocalizationInfo {
            badge: "CZ TIT/EN",
            description: "CZ titulky / EN",
        }
    } else if has_cz_subs {
        LocalizationInfo {
            badge: "CZ TIT",
            description: "CZ titulky",
        }
    } else if has_cz && has_en {
        LocalizationInfo {
            badge: "CZ/EN",
            description: "CZ/EN",
        }
    } else if has_cz {
        LocalizationInfo {
            badge: "CZ",
            description: "CZ",
        }
    } else if has_sk_dub && has_en {
        LocalizationInfo {
            badge: "SK/EN",
            description: "SK dabing / EN",
        }
    } else if has_sk_dub {
        LocalizationInfo {
            badge: "SK",
            description: "SK dabing",
        }
    } else if has_sk_subs && has_en {
        LocalizationInfo {
            badge: "SK TIT/EN",
            description: "SK titulky / EN",
        }
    } else if has_sk_subs {
        LocalizationInfo {
            badge: "SK TIT",
            description: "SK titulky",
        }
    } else if has_sk && has_en {
        LocalizationInfo {
            badge: "SK/EN",
            description: "SK/EN",
        }
    } else if has_sk {
        LocalizationInfo {
            badge: "SK",
            description: "SK",
        }
    } else if has_en {
        LocalizationInfo {
            badge: "EN",
            description: "EN",
        }
    } else {
        unknown_localization()
    }
}

fn localization_rank_with_mode(title: &str, allow_generic_subtitles: bool) -> i32 {
    let view = FilenameView::new(title);
    let has_cz_dub = has_czech_dub_signal(&view);
    let has_cz_subs = if allow_generic_subtitles {
        has_czech_subtitle_signal_lenient(&view)
    } else {
        has_czech_subtitle_signal(&view)
    };
    let has_cz = has_cz_dub || has_cz_subs || has_czech_language_marker(&view);
    let has_sk_dub = has_slovak_dub_signal(&view);
    let has_sk_subs = has_slovak_subtitle_signal(&view);
    let has_sk = has_sk_dub || has_sk_subs || has_slovak_language_marker(&view);
    let has_en = has_english_signal(&view);

    if has_cz_dub && has_en {
        5
    } else if has_cz_dub || has_sk_dub {
        4
    } else if (has_cz_subs || has_sk_subs || has_cz || has_sk) && has_en {
        3
    } else if has_cz_subs || has_sk_subs || has_cz || has_sk {
        2
    } else if has_en {
        1
    } else {
        0
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn unknown_localization() -> LocalizationInfo {
    LocalizationInfo {
        badge: "UNK",
        description: "Unknown localization",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_movie_filename() {
        let parsed = parse_filename("The.Matrix.1999.1080p.BluRay.CZ.mkv");
        assert_eq!(parsed.title.as_deref(), Some("the matrix"));
        assert_eq!(parsed.year, Some(1999));
        assert_eq!(parsed.quality.as_deref(), Some("1080p"));
        assert_eq!(parsed.quality_rank, Some(300));
        assert_eq!(parsed.language.as_deref(), Some("CZ"));
    }

    #[test]
    fn parses_series_and_ignores_x264_false_positive() {
        let parsed = parse_filename("Show.Name.S01E02.1080p.x264.mkv");
        assert_eq!(parsed.season, Some(1));
        assert_eq!(parsed.episode, Some(2));

        let movie = parse_filename("Movie.Name.2020.1080p.x264.mkv");
        assert_eq!(movie.season, None);
        assert_eq!(movie.episode, None);
    }

    #[test]
    fn localization_info_returns_expected_rank() {
        let local = localization_info(Some("Film [CZ DABING] WEB-DL.mkv"));
        assert_eq!(local.badge, "CZ");
        assert_eq!(local.description, "CZ dabing");

        let local = localization_info(Some("Movie EN TITULKY.mkv"));
        assert_eq!(local.badge, "EN");

        let local = localization_info_lenient(Some("Movie EN TITULKY.mkv"));
        assert_eq!(local.badge, "CZ TIT/EN");
    }

    #[test]
    fn localization_parses_slovak_signals() {
        assert_eq!(
            parse_filename("Film.SK.DABING.WEB-DL.mkv")
                .language
                .as_deref(),
            Some("SK")
        );
        assert_eq!(
            parse_filename("Film.EN.SK.TITULKY.WEB-DL.mkv")
                .language
                .as_deref(),
            Some("SK TIT/EN")
        );
        assert!(localization_rank("Film.SK.DABING.mkv") > localization_rank("Film.EN.mkv"));
    }

    #[test]
    fn release_quality_ranking_remains_webshare() {
        assert_eq!(release_quality_rank("1080p"), Some(300));
        assert_eq!(release_quality_rank("kine"), None);
    }

    #[test]
    fn quality_label_from_conversion_and_ranks() {
        assert_eq!(
            quality_label_from_conversion("1080", "Film (2026) CZtit KinoRip.mkv"),
            "1080p"
        );
        assert_eq!(
            quality_label_from_conversion("0", "Film.2026. WEB-DL.mkv"),
            "WEB-DL"
        );
        assert_eq!(height_quality_rank("1080p"), 1080);
        assert_eq!(height_quality_rank("h264"), 1);
        assert_eq!(media_quality_rank("Film 1080p WEB-DL"), 1080);
        assert_eq!(media_quality_rank("Film WEB-DL"), 700);
        assert_eq!(media_quality_rank("Film KinoRip"), 240);
        assert_eq!(quality_from_release_rank(80), None);
    }

    #[test]
    fn parsed_title_with_generic_token() {
        let parsed = parsed_title_for_ranking(Some("Movie".to_string()), Some("Alien"));
        assert_eq!(parsed.as_deref(), Some("Alien"));
        let parsed = parsed_title_for_ranking(Some("Great Film".to_string()), Some("Alien"));
        assert_eq!(parsed.as_deref(), Some("Great Film"));
    }

    #[test]
    fn parse_season_episode_with_simple_markers() {
        assert_eq!(parse_season_episode("S01E02"), Some((1, 2)));
        assert_eq!(parse_season_episode("01x10"), Some((1, 10)));
        assert_eq!(parse_season_episode("E 03"), Some((1, 3)));
    }
}
