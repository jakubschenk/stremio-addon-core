use crate::metadata::TitleInfo;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryProfile {
    Webshare,
    Hellspy,
    Balanced,
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
        QueryProfile::Balanced => balanced_queries(input),
    }
}

pub fn balanced_queries(input: &QueryInput) -> Vec<String> {
    title_year_episode_queries(input)
}

pub fn webshare_queries(input: &QueryInput) -> Vec<String> {
    title_year_episode_queries(input)
}

fn title_year_episode_queries(input: &QueryInput) -> Vec<String> {
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
        let dotted = dotted_title(name);
        queries.extend([
            format!("{name}{suffix}"),
            format!("{simplified_name}{suffix}"),
            name.to_string(),
            simplified_name.to_string(),
            format!("{dotted}{dotted_suffix}"),
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
    let folded = titles
        .iter()
        .filter_map(|title| ascii_fold_latin(title))
        .collect::<Vec<_>>();
    titles.extend(folded);

    if let Some(fallback_name) = fallback_name.filter(|name| !name.is_empty()) {
        let should_add_fallback = id.is_none_or(|id| id != fallback_name)
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

fn ascii_fold_latin(value: &str) -> Option<String> {
    let mut folded = String::new();
    let mut changed = false;
    for char in value.chars() {
        let replacement = match char {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ā' | 'ă' | 'ą' => "a",
            'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' | 'Ā' | 'Ă' | 'Ą' => "A",
            'č' | 'ć' | 'ç' => "c",
            'Č' | 'Ć' | 'Ç' => "C",
            'ď' => "d",
            'Ď' => "D",
            'é' | 'è' | 'ê' | 'ë' | 'ě' | 'ē' | 'ė' | 'ę' => "e",
            'É' | 'È' | 'Ê' | 'Ë' | 'Ě' | 'Ē' | 'Ė' | 'Ę' => "E",
            'í' | 'ì' | 'î' | 'ï' | 'ī' | 'į' => "i",
            'Í' | 'Ì' | 'Î' | 'Ï' | 'Ī' | 'Į' => "I",
            'ľ' | 'ĺ' | 'ł' => "l",
            'Ľ' | 'Ĺ' | 'Ł' => "L",
            'ň' | 'ń' | 'ñ' => "n",
            'Ň' | 'Ń' | 'Ñ' => "N",
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ő' | 'ø' | 'ō' => "o",
            'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' | 'Ő' | 'Ø' | 'Ō' => "O",
            'ř' => "r",
            'Ř' => "R",
            'š' | 'ś' => "s",
            'Š' | 'Ś' => "S",
            'ť' => "t",
            'Ť' => "T",
            'ú' | 'ù' | 'û' | 'ü' | 'ů' | 'ű' | 'ū' => "u",
            'Ú' | 'Ù' | 'Û' | 'Ü' | 'Ů' | 'Ű' | 'Ū' => "U",
            'ý' | 'ÿ' => "y",
            'Ý' | 'Ÿ' => "Y",
            'ž' | 'ź' | 'ż' => "z",
            'Ž' | 'Ź' | 'Ż' => "Z",
            'æ' => "ae",
            'Æ' => "AE",
            'œ' => "oe",
            'Œ' => "OE",
            'ß' => "ss",
            _ => {
                folded.push(char);
                continue;
            }
        };
        changed = true;
        folded.push_str(replacement);
    }

    changed.then_some(folded)
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

    #[test]
    fn select_search_titles_keeps_czech_and_english_candidates() {
        let info = TitleInfo {
            title_cs: Some("Hvezdna brana".to_string()),
            title_en: Some("Stargate SG-1".to_string()),
            ..TitleInfo::default()
        };

        assert_eq!(
            select_search_titles(&info, Some("tt0118480:4:10"), Some("tt0118480")),
            vec!["Hvezdna brana", "Stargate SG-1"]
        );
    }

    #[test]
    fn select_search_titles_adds_ascii_folded_localized_candidates() {
        let info = TitleInfo {
            title_cs: Some("Hvězdná brána".to_string()),
            title_en: Some("Stargate SG-1".to_string()),
            ..TitleInfo::default()
        };

        assert_eq!(
            select_search_titles(&info, None, None),
            vec!["Hvězdná brána", "Stargate SG-1", "Hvezdna brana"]
        );
    }
}
