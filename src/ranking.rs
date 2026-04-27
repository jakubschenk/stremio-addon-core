use crate::metadata::TitleInfo;
use crate::search::dedupe_non_empty;
use std::cmp::Ordering;

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
    let weak_match = filename_match > options.weak_threshold;
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

fn has_year_penalty(info: &TitleInfo, result: &SearchResult, tolerance: i32) -> bool {
    let (Some(target_year), Some(result_year)) = (
        info.year
            .as_deref()
            .and_then(|year| year.parse::<i32>().ok()),
        result.year,
    ) else {
        return false;
    };

    (target_year - result_year).abs() > tolerance
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
}
