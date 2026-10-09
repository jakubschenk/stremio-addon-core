use crate::filename::year_candidates;
use crate::metadata::TitleInfo;
use crate::search::{dedupe_non_empty, search_fold_key};
use std::cmp::Ordering;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RankingProfile {
    Webshare,
    Hellspy,
    Balanced,
}

#[derive(Clone, Debug, Default, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct RankedResult {
    pub result: SearchResult,
    pub score: MatchScore,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MatchScore {
    pub title_match: f64,
    pub filename_match: f64,
    pub fulltext_bucket: f64,
    pub strong_match: bool,
    pub weak_match: bool,
    pub year_penalty: bool,
    pub season_episode_match: bool,
    pub total: f64,
}

#[derive(Clone, Debug)]
pub struct RankingOptions {
    pub profile: RankingProfile,
    pub strong_threshold: f64,
    pub weak_threshold: f64,
    pub year_tolerance: i32,
    pub require_series_episode_match: bool,
    pub exclude_protected: bool,
    pub exclude_episode_like_movies: bool,
}

impl RankingOptions {
    pub fn for_profile(profile: RankingProfile) -> Self {
        match profile {
            RankingProfile::Webshare => Self {
                profile,
                strong_threshold: 0.5,
                weak_threshold: 0.3,
                year_tolerance: 1,
                require_series_episode_match: true,
                exclude_protected: true,
                exclude_episode_like_movies: true,
            },
            RankingProfile::Hellspy => Self {
                profile,
                strong_threshold: 0.45,
                weak_threshold: 0.2,
                year_tolerance: 1,
                require_series_episode_match: true,
                exclude_protected: true,
                exclude_episode_like_movies: false,
            },
            RankingProfile::Balanced => Self {
                profile,
                strong_threshold: 0.5,
                weak_threshold: 0.3,
                year_tolerance: 1,
                require_series_episode_match: true,
                exclude_protected: true,
                exclude_episode_like_movies: true,
            },
        }
    }
}

pub fn rank_results(
    info: &TitleInfo,
    results: Vec<SearchResult>,
    options: &RankingOptions,
) -> Vec<RankedResult> {
    let mut ranked = results
        .into_iter()
        .filter_map(|result| rank_result(info, result, options))
        .collect::<Vec<_>>();

    ranked.sort_by(compare_ranked);
    ranked
}

pub fn rank_result(
    info: &TitleInfo,
    result: SearchResult,
    options: &RankingOptions,
) -> Option<RankedResult> {
    if options.exclude_protected && result.protected {
        return None;
    }

    let score = score_result(info, &result, options);
    if !score.strong_match && !score.weak_match {
        return None;
    }
    if score.year_penalty {
        return None;
    }
    // Without a year to confirm it, a movie result must name the title
    // closely: "Matrix.CZ.dabing.mkv" passes, "Buchty na plech.avi" for
    // "Na plech" (2025) and "Matrix Reloaded CZ.mkv" do not.
    if info.content_type.as_deref() == Some("movie")
        && info.year.is_some()
        && !has_release_year(info, &result)
        && !names_title_exactly(info, &result)
    {
        return None;
    }
    if options.require_series_episode_match && info.is_series() && !score.season_episode_match {
        return None;
    }
    if options.exclude_episode_like_movies && is_episode_like_movie_false_positive(info, &result) {
        return None;
    }

    Some(RankedResult { result, score })
}

pub fn score_result(
    info: &TitleInfo,
    result: &SearchResult,
    options: &RankingOptions,
) -> MatchScore {
    let target_titles = match_titles(info);
    let parsed_title = result.parsed_title.as_deref().unwrap_or(&result.title);
    let filename = result.filename.as_deref().unwrap_or(&result.title);
    let title_match = best_similarity(&clean_title(parsed_title), &target_titles);
    let filename_match = best_similarity(&clean_title(filename), &target_titles);
    let fulltext_bucket = (filename_match * 10.0).round() / 10.0;
    let strong_match = title_match > options.strong_threshold;
    // Bigram similarity alone lets "Roxy Music - Do The Strand" through for
    // "Do the Mæth", so weak matches must also name the title's key words.
    let weak_match = filename_match > options.weak_threshold
        && (contains_title_words(info, filename) || contains_title_words(info, parsed_title));
    let year_penalty = has_year_penalty(info, result, options.year_tolerance);
    let season_episode_match = season_episode_matches(info, result);
    let total = total_score(
        options.profile,
        result,
        title_match,
        filename_match,
        fulltext_bucket,
    );

    MatchScore {
        title_match,
        filename_match,
        fulltext_bucket,
        strong_match,
        weak_match,
        year_penalty,
        season_episode_match,
        total,
    }
}

fn compare_ranked(a: &RankedResult, b: &RankedResult) -> Ordering {
    b.score
        .total
        .partial_cmp(&a.score.total)
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            b.result
                .positive_votes
                .unwrap_or_default()
                .cmp(&a.result.positive_votes.unwrap_or_default())
        })
        .then_with(|| {
            a.result
                .negative_votes
                .unwrap_or_default()
                .cmp(&b.result.negative_votes.unwrap_or_default())
        })
        .then_with(|| {
            b.result
                .quality_rank
                .unwrap_or_default()
                .cmp(&a.result.quality_rank.unwrap_or_default())
        })
        .then_with(|| {
            b.result
                .size_bytes
                .unwrap_or_default()
                .cmp(&a.result.size_bytes.unwrap_or_default())
        })
        .then_with(|| {
            query_order_key(a.result.query_index).cmp(&query_order_key(b.result.query_index))
        })
}

fn query_order_key(query_index: Option<usize>) -> usize {
    query_index.unwrap_or(usize::MAX)
}

fn total_score(
    profile: RankingProfile,
    result: &SearchResult,
    title_match: f64,
    filename_match: f64,
    fulltext_bucket: f64,
) -> f64 {
    let votes = result.positive_votes.unwrap_or_default().max(0) as f64;
    let negative_votes = result.negative_votes.unwrap_or_default().max(0) as f64;
    let vote_score = (votes - negative_votes).max(0.0);
    let quality_score = result.quality_rank.unwrap_or_default().max(0) as f64;
    let size = result.size_bytes.unwrap_or_default() as f64 / 1_000_000_000.0;
    let query_bonus = result
        .query_index
        .map(|index| 1.0 / ((index + 1) as f64))
        .unwrap_or_default();

    match profile {
        RankingProfile::Webshare => {
            title_match * 100.0
                + fulltext_bucket * 10.0
                + vote_score * 0.01
                + quality_score * 0.1
                + size * 0.001
        }
        RankingProfile::Hellspy => {
            filename_match * 100.0
                + title_match * 40.0
                + query_bonus * 5.0
                + quality_score * 0.05
                + size * 0.001
        }
        RankingProfile::Balanced => {
            title_match * 80.0
                + filename_match * 40.0
                + query_bonus
                + vote_score * 0.01
                + quality_score * 0.05
        }
    }
}

fn match_titles(info: &TitleInfo) -> Vec<String> {
    let titles = info
        .title_candidates()
        .into_iter()
        .flat_map(|title| {
            if info.content_type.as_deref() == Some("movie") {
                match info.year.as_deref() {
                    Some(year) if !title_contains_year(&title) => {
                        vec![title.clone(), format!("{title} {year}")]
                    }
                    _ => vec![title],
                }
            } else {
                vec![title]
            }
        })
        .collect::<Vec<_>>();
    dedupe_non_empty(
        titles
            .iter()
            .flat_map(|title| [title.clone(), clean_title(title)])
            .collect(),
    )
}

/// True when every significant word of some title candidate appears in
/// `text`, ignoring case, accents and filler words like "the" or "na".
fn contains_title_words(info: &TitleInfo, text: &str) -> bool {
    let words = folded_words(text).collect::<HashSet<_>>();
    info.title_candidates().iter().any(|title| {
        let title_words = folded_words(title).collect::<Vec<_>>();
        let significant = title_words
            .iter()
            .filter(|word| !is_filler_word(word))
            .collect::<Vec<_>>();
        if significant.is_empty() {
            // "It" or "To": a bare filler word anywhere proves nothing.
            return title_words_before_tags(text) == title_words;
        }
        significant.iter().all(|word| words.contains(*word))
    })
}

fn folded_words(value: &str) -> impl Iterator<Item = String> {
    search_fold_key(value)
        .split(|char: char| !char.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>()
        .into_iter()
}

fn is_filler_word(word: &str) -> bool {
    matches!(
        word,
        "a" | "an"
            | "and"
            | "at"
            | "for"
            | "in"
            | "of"
            | "on"
            | "or"
            | "the"
            | "to"
            | "with"
            | "do"
            | "k"
            | "na"
            | "o"
            | "po"
            | "s"
            | "se"
            | "u"
            | "v"
            | "ve"
            | "z"
            | "za"
            | "ze"
    )
}

fn has_year_penalty(info: &TitleInfo, result: &SearchResult, tolerance: i32) -> bool {
    // Series metadata carries the first-air year, which says nothing about
    // the year printed on a later season's episodes.
    if info.is_series() {
        return false;
    }
    let (Some(target_year), Some(result_year)) = (
        info.year
            .as_deref()
            .and_then(|year| year.get(..4))
            .and_then(|year| year.parse::<i32>().ok()),
        result.year,
    ) else {
        return false;
    };
    let fits = |year: i32| (target_year - year).abs() <= tolerance;
    if fits(result_year) {
        return false;
    }

    // "Blade Runner 2049" parses 2049 as a year; a year that belongs to the
    // title, or any other year in the name that fits, is not a mismatch.
    let title_years = info
        .title_candidates()
        .iter()
        .flat_map(|title| year_candidates(title))
        .collect::<HashSet<_>>();
    if title_years.contains(&result_year) {
        return false;
    }
    let name = result.filename.as_deref().unwrap_or(&result.title);
    !year_candidates(name)
        .into_iter()
        .any(|year| fits(year) && !title_years.contains(&year))
}

/// A parsed year that is really part of the title ("Blade Runner 2049")
/// does not date the release.
fn has_release_year(info: &TitleInfo, result: &SearchResult) -> bool {
    let year = result.year.or_else(|| {
        crate::filename::parse_year(result.filename.as_deref().unwrap_or(&result.title))
    });
    year.is_some_and(|year| {
        !info
            .title_candidates()
            .iter()
            .any(|title| year_candidates(title).contains(&year))
    })
}

/// True when the name, cut at its first release tag, is one of the titles:
/// `Pelisky.CZ.1080p.mkv` names "Pelíšky", `Matrix Reloaded CZ.mkv` does not
/// name "Matrix".
fn names_title_exactly(info: &TitleInfo, result: &SearchResult) -> bool {
    let titles = info
        .title_candidates()
        .iter()
        .map(|title| significant_words(title_words_before_tags(title)))
        .filter(|words| !words.is_empty())
        .collect::<Vec<_>>();
    [
        result.parsed_title.as_deref(),
        result.filename.as_deref(),
        Some(result.title.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(|value| significant_words(title_words_before_tags(value)))
    .any(|words| titles.contains(&words))
}

/// Drops filler words so "Mr. and Mrs. Smith" names "Mr. & Mrs. Smith";
/// all-filler titles such as "It" keep their words.
fn significant_words(words: Vec<String>) -> Vec<String> {
    let significant = words
        .iter()
        .filter(|word| !is_filler_word(word))
        .cloned()
        .collect::<Vec<_>>();
    if significant.is_empty() {
        words
    } else {
        significant
    }
}

fn title_words_before_tags(value: &str) -> Vec<String> {
    folded_words(value)
        .take_while(|word| !is_release_tag(word))
        .collect()
}

fn is_release_tag(word: &str) -> bool {
    matches!(
        word,
        "mkv"
            | "mp4"
            | "avi"
            | "m4v"
            | "ts"
            | "wmv"
            | "mpg"
            | "cz"
            | "sk"
            | "en"
            | "eng"
            | "cze"
            | "czech"
            | "slovak"
            | "dabing"
            | "dab"
            | "czdab"
            | "czdabing"
            | "titulky"
            | "tit"
            | "cztit"
            | "sub"
            | "subs"
            | "4k"
            | "uhd"
            | "fhd"
            | "hd"
            | "sd"
            | "hdr"
            | "dv"
            | "x264"
            | "x265"
            | "h264"
            | "h265"
            | "hevc"
            | "avc"
            | "xvid"
            | "divx"
            | "web"
            | "webrip"
            | "webdl"
            | "bluray"
            | "brrip"
            | "bdrip"
            | "dvdrip"
            | "hdrip"
            | "tvrip"
            | "remux"
            | "dts"
            | "ac3"
            | "aac"
            | "remastered"
            | "extended"
            | "unrated"
            | "uncut"
            | "theatrical"
            | "directors"
            | "imax"
            | "proper"
            | "repack"
            | "multi"
            | "dual"
            | "open"
            | "dc"
    ) || (word.ends_with('p') && word[..word.len() - 1].parse::<u32>().is_ok())
}

fn season_episode_matches(info: &TitleInfo, result: &SearchResult) -> bool {
    match (info.season, info.episode) {
        (Some(season), Some(episode)) => {
            result.season == Some(season) && result.episode == Some(episode)
        }
        _ => true,
    }
}

fn is_episode_like_movie_false_positive(info: &TitleInfo, result: &SearchResult) -> bool {
    if info.content_type.as_deref() != Some("movie") || result.season.is_none() {
        return false;
    }
    let filename = result
        .filename
        .as_deref()
        .unwrap_or(&result.title)
        .to_lowercase();
    let title_context = info.title_candidates().join(" ").to_lowercase();

    !["episode", "part"]
        .iter()
        .any(|keyword| filename.contains(keyword) && title_context.contains(keyword))
}

pub fn clean_title(value: &str) -> String {
    let mut cleaned = String::new();
    for char in value.chars() {
        if char.is_alphanumeric() || char.is_whitespace() {
            cleaned.extend(char.to_lowercase());
        } else {
            cleaned.push(' ');
        }
    }

    cleaned
        .split_whitespace()
        .filter(|part| !matches!(*part, "subtitles" | "subtitle" | "titulky"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn title_contains_year(title: &str) -> bool {
    title
        .split(|char: char| !char.is_ascii_digit())
        .any(|part| part.len() == 4 && part.parse::<u16>().is_ok())
}

fn best_similarity(value: &str, targets: &[String]) -> f64 {
    targets
        .iter()
        .map(|target| dice_similarity(value, target))
        .fold(0.0, f64::max)
}

pub fn dice_similarity(left: &str, right: &str) -> f64 {
    let left = clean_title(left);
    let right = clean_title(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }

    let left_bigrams = bigrams(&left);
    let mut right_bigrams = bigrams(&right);
    if left_bigrams.is_empty() || right_bigrams.is_empty() {
        return 0.0;
    }

    let mut intersection = 0;
    for bigram in &left_bigrams {
        if let Some(index) = right_bigrams
            .iter()
            .position(|candidate| candidate == bigram)
        {
            intersection += 1;
            right_bigrams.remove(index);
        }
    }

    (2 * intersection) as f64 / (left_bigrams.len() + bigrams(&right).len()) as f64
}

fn bigrams(value: &str) -> Vec<String> {
    let chars = value.chars().collect::<Vec<_>>();
    chars
        .windows(2)
        .map(|window| window.iter().collect::<String>())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie_info() -> TitleInfo {
        TitleInfo {
            content_type: Some("movie".to_string()),
            primary_title: Some("Miracle man".to_string()),
            year: Some("2024".to_string()),
            ..TitleInfo::default()
        }
    }

    #[test]
    fn ranks_better_title_match_first() {
        let ranked = rank_results(
            &movie_info(),
            vec![
                SearchResult {
                    id: "1".to_string(),
                    title: "Morcle man 2024.avi".to_string(),
                    parsed_title: Some("Morcle man".to_string()),
                    year: Some(2024),
                    ..SearchResult::default()
                },
                SearchResult {
                    id: "2".to_string(),
                    title: "Miracle man 2024.avi".to_string(),
                    parsed_title: Some("Miracle man".to_string()),
                    year: Some(2024),
                    ..SearchResult::default()
                },
            ],
            &RankingOptions::for_profile(RankingProfile::Webshare),
        );

        assert_eq!(ranked[0].result.id, "2");
    }

    #[test]
    fn filters_wrong_year_beyond_tolerance() {
        let ranked = rank_results(
            &movie_info(),
            vec![
                SearchResult {
                    id: "wrong".to_string(),
                    title: "Miracle man 2021.avi".to_string(),
                    parsed_title: Some("Miracle man".to_string()),
                    year: Some(2021),
                    ..SearchResult::default()
                },
                SearchResult {
                    id: "right".to_string(),
                    title: "Miracle man 2024.avi".to_string(),
                    parsed_title: Some("Miracle man".to_string()),
                    year: Some(2024),
                    ..SearchResult::default()
                },
            ],
            &RankingOptions::for_profile(RankingProfile::Webshare),
        );

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].result.id, "right");
    }

    #[test]
    fn filters_series_episode_mismatch() {
        let info = TitleInfo {
            content_type: Some("series".to_string()),
            primary_title: Some("Breaking Bad".to_string()),
            season: Some(1),
            episode: Some(2),
            ..TitleInfo::default()
        };
        let ranked = rank_results(
            &info,
            vec![
                SearchResult {
                    id: "bad".to_string(),
                    title: "Breaking Bad S01E03.mkv".to_string(),
                    parsed_title: Some("Breaking Bad".to_string()),
                    season: Some(1),
                    episode: Some(3),
                    ..SearchResult::default()
                },
                SearchResult {
                    id: "good".to_string(),
                    title: "Breaking Bad S01E02.mkv".to_string(),
                    parsed_title: Some("Breaking Bad".to_string()),
                    season: Some(1),
                    episode: Some(2),
                    ..SearchResult::default()
                },
            ],
            &RankingOptions::for_profile(RankingProfile::Webshare),
        );

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].result.id, "good");
    }

    #[test]
    fn hellspy_query_order_can_influence_close_matches() {
        let info = TitleInfo {
            content_type: Some("movie".to_string()),
            primary_title: Some("Alien Covenant".to_string()),
            year: Some("2017".to_string()),
            ..TitleInfo::default()
        };
        let ranked = rank_results(
            &info,
            vec![
                SearchResult {
                    id: "later".to_string(),
                    title: "Alien Covenant 2017.mkv".to_string(),
                    query_index: Some(5),
                    ..SearchResult::default()
                },
                SearchResult {
                    id: "first".to_string(),
                    title: "Alien Covenant 2017.mkv".to_string(),
                    query_index: Some(0),
                    ..SearchResult::default()
                },
            ],
            &RankingOptions::for_profile(RankingProfile::Hellspy),
        );

        assert_eq!(ranked[0].result.id, "first");
    }

    #[test]
    fn ranks_against_all_metadata_title_candidates() {
        let info = TitleInfo {
            content_type: Some("movie".to_string()),
            primary_title: Some("Vykoupeni z veznice Shawshank".to_string()),
            title_en: Some("The Shawshank Redemption".to_string()),
            original_title: Some("The Shawshank Redemption".to_string()),
            year: Some("1994".to_string()),
            ..TitleInfo::default()
        };
        let ranked = rank_results(
            &info,
            vec![SearchResult {
                id: "english".to_string(),
                title: "The.Shawshank.Redemption.1994.1080p.mkv".to_string(),
                filename: Some("The.Shawshank.Redemption.1994.1080p.mkv".to_string()),
                year: Some(1994),
                ..SearchResult::default()
            }],
            &RankingOptions::for_profile(RankingProfile::Webshare),
        );

        assert_eq!(ranked[0].result.id, "english");
        assert!(ranked[0].score.weak_match);
    }

    #[test]
    fn weak_matches_require_title_words() {
        let info = TitleInfo {
            content_type: Some("movie".to_string()),
            primary_title: Some("Na plech".to_string()),
            title_cs: Some("Na plech".to_string()),
            title_en: Some("Do the Mæth".to_string()),
            year: Some("2025".to_string()),
            ..TitleInfo::default()
        };
        let result = |name: &str| SearchResult {
            id: name.to_string(),
            title: name.to_string(),
            filename: Some(name.to_string()),
            year: crate::parse_year(name),
            ..SearchResult::default()
        };
        let ranked = rank_results(
            &info,
            [
                "Do.the.Maeth.WEB-DL.CZ.5.1.2025.1080p.mkv",
                "Na plech (2025) CZ Dabing 1080p.mkv",
                "Svatba na Bahamách 2025 CZ dab 1080p.mkv",
                "06. Do the Wrong Thing.mkv",
                "Na horách 2025 cz 1080p.mkv",
                "Nav_do_akc.CZ.2025.1080p.mkv",
                "Roxy Music - Do The Strand (1080).mp4",
                "Poslední nádech 2025.mkv",
                "Do the Right Thing.avi",
                "Jetelín (42) - Střízlivý až na plech.mp4",
                "Deník Dity P. II. - 11. Buchty na plech.avi",
            ]
            .into_iter()
            .map(result)
            .collect(),
            &RankingOptions::for_profile(RankingProfile::Webshare),
        );

        let mut ids = ranked
            .iter()
            .map(|ranked| ranked.result.id.as_str())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(
            ids,
            vec![
                "Do.the.Maeth.WEB-DL.CZ.5.1.2025.1080p.mkv",
                "Na plech (2025) CZ Dabing 1080p.mkv"
            ]
        );
    }

    #[test]
    fn filename_only_result_can_rank() {
        let ranked = rank_results(
            &movie_info(),
            vec![SearchResult {
                id: "filename-only".to_string(),
                title: "opaque provider title".to_string(),
                filename: Some("Miracle.Man.2024.1080p.WEB-DL.mkv".to_string()),
                year: Some(2024),
                ..SearchResult::default()
            }],
            &RankingOptions::for_profile(RankingProfile::Webshare),
        );

        assert_eq!(ranked[0].result.id, "filename-only");
        assert!(ranked[0].score.weak_match);
    }

    #[test]
    fn filters_protected_results_by_default() {
        let ranked = rank_results(
            &movie_info(),
            vec![
                SearchResult {
                    id: "protected".to_string(),
                    title: "Miracle man 2024.avi".to_string(),
                    parsed_title: Some("Miracle man".to_string()),
                    year: Some(2024),
                    protected: true,
                    ..SearchResult::default()
                },
                SearchResult {
                    id: "public".to_string(),
                    title: "Miracle man 2024.avi".to_string(),
                    parsed_title: Some("Miracle man".to_string()),
                    year: Some(2024),
                    ..SearchResult::default()
                },
            ],
            &RankingOptions::for_profile(RankingProfile::Webshare),
        );

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].result.id, "public");
    }

    #[test]
    fn filters_episode_like_movie_false_positive() {
        let ranked = rank_results(
            &movie_info(),
            vec![
                SearchResult {
                    id: "episode".to_string(),
                    title: "Miracle.Man.S01E02.mkv".to_string(),
                    filename: Some("Miracle.Man.S01E02.mkv".to_string()),
                    year: Some(2024),
                    season: Some(1),
                    episode: Some(2),
                    ..SearchResult::default()
                },
                SearchResult {
                    id: "movie".to_string(),
                    title: "Miracle.Man.2024.mkv".to_string(),
                    filename: Some("Miracle.Man.2024.mkv".to_string()),
                    year: Some(2024),
                    ..SearchResult::default()
                },
            ],
            &RankingOptions::for_profile(RankingProfile::Webshare),
        );

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].result.id, "movie");
    }

    #[test]
    fn query_index_tie_breaks_known_order_before_unknown() {
        let ranked = rank_results(
            &movie_info(),
            vec![
                SearchResult {
                    id: "unknown".to_string(),
                    title: "Miracle man 2024.avi".to_string(),
                    query_index: None,
                    ..SearchResult::default()
                },
                SearchResult {
                    id: "second".to_string(),
                    title: "Miracle man 2024.avi".to_string(),
                    query_index: Some(1),
                    ..SearchResult::default()
                },
                SearchResult {
                    id: "first".to_string(),
                    title: "Miracle man 2024.avi".to_string(),
                    query_index: Some(0),
                    ..SearchResult::default()
                },
            ],
            &RankingOptions::for_profile(RankingProfile::Balanced),
        );

        assert_eq!(
            ranked
                .iter()
                .map(|ranked| ranked.result.id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second", "unknown"]
        );
    }

    #[test]
    fn vote_ratio_and_quality_can_break_close_ties() {
        let ranked = rank_results(
            &movie_info(),
            vec![
                SearchResult {
                    id: "bad-votes".to_string(),
                    title: "Miracle man 2024.avi".to_string(),
                    positive_votes: Some(20),
                    negative_votes: Some(20),
                    quality_rank: Some(1),
                    ..SearchResult::default()
                },
                SearchResult {
                    id: "better".to_string(),
                    title: "Miracle man 2024.avi".to_string(),
                    positive_votes: Some(8),
                    negative_votes: Some(0),
                    quality_rank: Some(3),
                    ..SearchResult::default()
                },
            ],
            &RankingOptions::for_profile(RankingProfile::Webshare),
        );

        assert_eq!(ranked[0].result.id, "better");
    }

    #[test]
    fn clean_title_removes_subtitle_markers_case_insensitively() {
        assert_eq!(
            clean_title("Film.TiTuLkY.SUBTITLEs.2024.mkv"),
            "film 2024 mkv"
        );
    }

    fn ranked_ids(info: &TitleInfo, names: &[&str]) -> Vec<String> {
        let mut ids = rank_results(
            info,
            names
                .iter()
                .map(|name| {
                    let parsed = crate::parse_filename(name);
                    SearchResult {
                        id: name.to_string(),
                        title: name.to_string(),
                        parsed_title: parsed.title,
                        filename: Some(name.to_string()),
                        year: parsed.year,
                        season: parsed.season,
                        episode: parsed.episode,
                        ..SearchResult::default()
                    }
                })
                .collect(),
            &RankingOptions::for_profile(RankingProfile::Webshare),
        )
        .into_iter()
        .map(|ranked| ranked.result.id)
        .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    fn movie(title: &str, year: &str) -> TitleInfo {
        TitleInfo {
            content_type: Some("movie".to_string()),
            primary_title: Some(title.to_string()),
            year: Some(year.to_string()),
            ..TitleInfo::default()
        }
    }

    #[test]
    fn titles_containing_years_keep_their_releases() {
        assert_eq!(
            ranked_ids(
                &movie("1917", "2019"),
                &["1917 (2019) 1080p cz sk dab.mkv", "1917.mkv"]
            ),
            vec!["1917 (2019) 1080p cz sk dab.mkv", "1917.mkv"]
        );
        assert_eq!(
            ranked_ids(
                &movie("Blade Runner 2049", "2017"),
                &[
                    "Blade Runner 2049 2017 CZ.mkv",
                    "Blade.Runner.2049.CZ.1080p.mkv"
                ]
            ),
            vec![
                "Blade Runner 2049 2017 CZ.mkv",
                "Blade.Runner.2049.CZ.1080p.mkv"
            ]
        );
        assert_eq!(
            ranked_ids(
                &movie("2001: A Space Odyssey", "1968"),
                &[
                    "2001.A.Space.Odyssey.1968.2160p.REMUX.mkv",
                    "Cosmos - A Spacetime Odyssey.mkv"
                ]
            ),
            vec!["2001.A.Space.Odyssey.1968.2160p.REMUX.mkv"]
        );
    }

    #[test]
    fn yearless_movie_results_must_name_the_title() {
        assert_eq!(
            ranked_ids(
                &movie("Matrix", "1999"),
                &[
                    "Matrix.CZ.dabing.mkv",
                    "Matrix Reloaded CZ.mkv",
                    "Matrix (1999) 1080p.mkv"
                ]
            ),
            vec!["Matrix (1999) 1080p.mkv", "Matrix.CZ.dabing.mkv"]
        );
        let pelisky = TitleInfo {
            title_cs: Some("Pelíšky".to_string()),
            ..movie("Cosy Dens", "1999")
        };
        assert_eq!(
            ranked_ids(&pelisky, &["Pelisky.CZ.1080p.mkv"]),
            vec!["Pelisky.CZ.1080p.mkv"]
        );
    }

    #[test]
    fn yearless_movie_names_must_match_without_filler() {
        assert_eq!(
            ranked_ids(
                &movie("Blade Runner 2049", "2017"),
                &[
                    "Blade Runner 2049 - Futuristic Psybient, Psychill, Downtempo Mix.avi",
                    "Blade.Runner.2049.CZ.1080p.mkv"
                ]
            ),
            vec!["Blade.Runner.2049.CZ.1080p.mkv"]
        );
        assert_eq!(
            ranked_ids(
                &movie("Mr. & Mrs. Smith", "2005"),
                &["Mr. and Mrs. Smith.mp4", "The Matrix Remastered.mkv"]
            ),
            vec!["Mr. and Mrs. Smith.mp4"]
        );
    }

    #[test]
    fn filler_only_titles_need_an_exact_name() {
        assert_eq!(
            ranked_ids(
                &movie("To", "2017"),
                &["To je zivot (2017).mkv", "To (2017) CZ.mkv"]
            ),
            vec!["To (2017) CZ.mkv"]
        );
    }

    #[test]
    fn series_episodes_ignore_the_first_air_year() {
        let info = TitleInfo {
            content_type: Some("series".to_string()),
            primary_title: Some("Stargate SG-1".to_string()),
            year: Some("1997".to_string()),
            season: Some(4),
            episode: Some(10),
            ..TitleInfo::default()
        };
        assert_eq!(
            ranked_ids(&info, &["Stargate SG-1 4x10 (2000).avi"]),
            vec!["Stargate SG-1 4x10 (2000).avi"]
        );
    }
}
