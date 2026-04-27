use crate::metadata::TitleInfo;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryProfile {
    Webshare,
    Hellspy,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueryInput {
    pub content_type: Option<String>,
    pub titles: Vec<String>,
    pub year: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

impl QueryInput {
    pub fn from_title_info(info: &TitleInfo) -> Self {
        Self {
            content_type: info.content_type.clone(),
            titles: info.title_candidates(),
            year: info.year.clone(),
            season: info.season,
            episode: info.episode,
        }
    }
}

pub fn build_search_queries(profile: QueryProfile, input: &QueryInput) -> Vec<String> {
    match profile {
        QueryProfile::Webshare => webshare_queries(input),
        QueryProfile::Hellspy => hellspy_queries_for_titles(input),
    }
}

pub fn webshare_queries(input: &QueryInput) -> Vec<String> {
    let titles = dedupe_non_empty(input.titles.clone());
    if input.content_type.as_deref() == Some("series") {
        let (Some(season), Some(episode)) = (input.season, input.episode) else {
            return Vec::new();
        };
        let season = format!("{season:02}");
        let episode = format!("{episode:02}");
        return titles
            .into_iter()
            .flat_map(|title| {
                [
                    format!("{title} S{season}E{episode}"),
                    format!("{title} {season}x{episode}"),
                ]
            })
            .collect();
    }

    let mut queries = titles.clone();
    if let Some(year) = input.year.as_deref().filter(|year| !year.is_empty()) {
        queries.extend(titles.into_iter().map(|title| format!("{title} {year}")));
    }
    dedupe_non_empty(queries)
}

pub fn hellspy_queries_for_titles(input: &QueryInput) -> Vec<String> {
    let mut queries = Vec::new();
    for title in &input.titles {
        queries.extend(hellspy_queries(
            input.content_type.as_deref(),
            title,
            input.year.as_deref(),
            input.season,
            input.episode,
        ));
    }
    dedupe_non_empty(queries)
}

pub fn hellspy_queries(
    content_type: Option<&str>,
    name: &str,
    year: Option<&str>,
    season: Option<u32>,
    episode: Option<u32>,
) -> Vec<String> {
    let simplified_name = name.split(':').next().unwrap_or(name);
    let mut queries = Vec::new();

    if let (Some("series"), Some(season), Some(episode)) = (content_type, season, episode) {
        let season_str = format!("{season:02}");
        let episode_str = format!("{episode:02}");
        queries.extend([
            format!("{name} S{season_str}E{episode_str}"),
            format!("{name} {season_str}x{episode_str}"),
            format!("{name}.S{season_str}E{episode_str}"),
            format!("{name} - {episode_str}"),
            format!("{simplified_name} S{season_str}E{episode_str}"),
        ]);
    } else {
        let suffix = year
            .filter(|year| !year.is_empty())
            .map(|year| format!(" {year}"))
            .unwrap_or_default();
        let dotted_suffix = year
            .filter(|year| !year.is_empty())
            .map(|year| format!(".{year}"))
            .unwrap_or_default();
        queries.extend([
            format!("{name}{suffix}"),
            format!("{simplified_name}{suffix}"),
            name.to_string(),
            simplified_name.to_string(),
            format!("{}{}", dotted_title(name), dotted_suffix),
        ]);
    }

    dedupe_non_empty(queries)
}

pub fn select_search_titles(
    info: &TitleInfo,
    fallback_name: Option<&str>,
    id: Option<&str>,
) -> Vec<String> {
    let mut titles = Vec::new();
    titles.extend(info.title_candidates());

    if let Some(fallback_name) = fallback_name.filter(|name| !name.is_empty()) {
        let should_add_fallback = !id.is_some_and(|id| id == fallback_name)
            && !fallback_name.contains(":")
            && !titles.iter().any(|title| title == fallback_name);
        if should_add_fallback {
            titles.push(fallback_name.to_string());
        }
    }

    if titles.is_empty() {
        fallback_name.map(str::to_string).into_iter().collect()
    } else {
        dedupe_non_empty(titles)
    }
}

pub fn dotted_title(name: &str) -> String {
    name.chars()
        .map(|char| {
            if char.is_whitespace() || matches!(char, ':' | '-' | '/') {
                '.'
            } else {
                char
            }
        })
        .collect::<String>()
        .split('.')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

pub fn dedupe_non_empty(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        if !value.is_empty() && !out.contains(&value) {
            out.push(value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webshare_movie_queries_include_year_variants() {
        let input = QueryInput {
            content_type: Some("movie".to_string()),
            titles: vec!["Soul".to_string(), "Duše".to_string()],
            year: Some("2020".to_string()),
            ..QueryInput::default()
        };

        assert_eq!(
            webshare_queries(&input),
            vec!["Soul", "Duše", "Soul 2020", "Duše 2020"]
        );
    }

    #[test]
    fn webshare_series_queries_match_existing_addon() {
        let input = QueryInput {
            content_type: Some("series".to_string()),
            titles: vec!["Breaking Bad".to_string()],
            season: Some(1),
            episode: Some(2),
            ..QueryInput::default()
        };

        assert_eq!(
            webshare_queries(&input),
            vec!["Breaking Bad S01E02", "Breaking Bad 01x02"]
        );
    }

    #[test]
    fn hellspy_movie_queries_match_rewrite_order() {
        assert_eq!(
            hellspy_queries(Some("movie"), "Alien: Covenant", Some("2017"), None, None),
            vec![
                "Alien: Covenant 2017",
                "Alien 2017",
                "Alien: Covenant",
                "Alien",
                "Alien.Covenant.2017"
            ]
        );
    }

    #[test]
    fn hellspy_series_queries_match_rewrite_order() {
        assert_eq!(
            hellspy_queries(Some("series"), "English Title", None, Some(1), Some(2)),
            vec![
                "English Title S01E02",
                "English Title 01x02",
                "English Title.S01E02",
                "English Title - 02"
            ]
        );
    }
}
