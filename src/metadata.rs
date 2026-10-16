//! Series matching against online databases: AniList (manga, free, no key)
//! and ComicVine (Western comics, free key). Produces one [`SeriesMatch`]
//! per series — canonical name, description, and for ComicVine the whole
//! issue list so local issues can be linked by number.

use serde::Deserialize;

use crate::comicvine;
use crate::naming::{normalize_number, similarity};

/// What we know locally about a series, used to search and to judge
/// candidates.
#[derive(Clone, Debug)]
pub struct SeriesQuery {
    pub search: String,
    pub year: Option<u32>,
    pub volume: Option<u32>,
    /// Normalized issue numbers of the local issues.
    pub issue_numbers: Vec<String>,
    /// Chapters named "Ch N…" (manga-tui etc.) rather than "#N"/"001".
    pub looks_manga: bool,
}

#[derive(Clone, Debug, Default)]
pub struct IssueMeta {
    /// Normalized, see [`normalize_number`].
    pub number: String,
    pub title: String,
    pub cover_date: String,
    pub url: String,
}

#[derive(Clone, Debug, Default)]
pub struct SeriesMatch {
    /// "comicvine" or "anilist".
    pub source: String,
    pub source_id: String,
    pub title: String,
    pub publisher: String,
    pub year: Option<u32>,
    pub description: String,
    pub url: String,
    pub status: String,
    pub genres: Vec<String>,
    pub chapter_count: Option<u32>,
    pub score: f32,
    pub issues: Vec<IssueMeta>,
}

/// Below this, a candidate is considered a different series entirely.
pub const ACCEPT_SCORE: f32 = 0.6;
/// At or above this, the online title replaces the cleaned local name.
pub const RENAME_SCORE: f32 = 0.85;

// ── AniList ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct AniListResponse {
    data: Option<AniListData>,
}

#[derive(Deserialize)]
struct AniListData {
    #[serde(rename = "Page")]
    page: AniListPage,
}

#[derive(Deserialize)]
struct AniListPage {
    media: Vec<AniListMedia>,
}

#[derive(Deserialize)]
struct AniListMedia {
    id: u64,
    title: AniListTitle,
    #[serde(default)]
    synonyms: Vec<String>,
    description: Option<String>,
    status: Option<String>,
    chapters: Option<u32>,
    genres: Option<Vec<String>>,
    #[serde(rename = "siteUrl")]
    site_url: Option<String>,
    #[serde(rename = "startDate")]
    start_date: Option<AniListDate>,
}

#[derive(Deserialize)]
struct AniListTitle {
    romaji: Option<String>,
    english: Option<String>,
}

#[derive(Deserialize)]
struct AniListDate {
    year: Option<u32>,
}

async fn anilist_candidates(search: &str) -> Result<Vec<AniListMedia>, String> {
    const QUERY: &str = r#"
        query ($search: String) {
          Page(perPage: 6) {
            media(search: $search, type: MANGA, sort: SEARCH_MATCH) {
              id
              title { romaji english }
              synonyms
              description(asHtml: false)
              status
              chapters
              genres
              siteUrl
              startDate { year }
            }
          }
        }
    "#;
    let body = serde_json::json!({ "query": QUERY, "variables": { "search": search } });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("http client error: {e}"))?;
    let resp = client
        .post("https://graphql.anilist.co")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("network error: {e}"))?;
    if resp.status().as_u16() == 404 {
        return Ok(Vec::new());
    }
    let resp: AniListResponse = resp.json().await.map_err(|e| format!("AniList parse error: {e}"))?;
    Ok(resp.data.map(|d| d.page.media).unwrap_or_default())
}

/// "Jujutsu Kaisen 0: Toukyou Toritsu…" -> "Jujutsu Kaisen 0". Subtitles
/// are where romanization differs most between sources.
fn main_title(search: &str) -> Option<&str> {
    let (head, _) = search.split_once(':')?;
    let head = head.trim();
    (head.len() >= 3).then_some(head)
}

/// Similarity against the full search, or (slightly discounted) against
/// just the part before a colon.
fn title_score(candidate: &str, search: &str) -> f32 {
    let full = similarity(candidate, search);
    match main_title(search) {
        Some(head) => full.max(similarity(candidate, head) - 0.05),
        None => full,
    }
}

/// Publishers whose ComicVine volumes are mostly translated reprints of
/// US/Japanese originals. The original should win a name tie.
const REPRINT_PUBLISHERS: &[&str] = &[
    "editorial televisa", "panini", "norma editorial", "planeta", "ecc ediciones", "semic",
    "egmont", "glénat", "glenat", "urban comics", "hachette", "salvat", "ovni press", "carlsen",
    "kana", "pika", "star comics", "j-pop", "jbc", "conrad", "mythos", "levoir", "rw edizioni",
    "dino comics", "ivrea", "kamite", "planet manga", "editora abril", "unlimited comics",
];

fn score_anilist(m: &AniListMedia, q: &SeriesQuery) -> f32 {
    let names = m
        .title
        .english
        .iter()
        .chain(m.title.romaji.iter())
        .chain(m.synonyms.iter());
    let mut score = names.map(|n| title_score(n, &q.search)).fold(0.0, f32::max);
    if q.looks_manga {
        score += 0.05;
    }
    score
}

fn anilist_match(m: AniListMedia, score: f32) -> SeriesMatch {
    SeriesMatch {
        source: "anilist".into(),
        source_id: m.id.to_string(),
        title: m.title.english.or(m.title.romaji).unwrap_or_default(),
        publisher: String::new(),
        year: m.start_date.and_then(|d| d.year),
        description: m
            .description
            .map(|d| comicvine::truncate(&strip_html(&d), 1500))
            .unwrap_or_default(),
        url: m.site_url.unwrap_or_default(),
        status: m.status.unwrap_or_default(),
        genres: m.genres.unwrap_or_default(),
        chapter_count: m.chapters,
        score,
        issues: Vec::new(),
    }
}

// ── ComicVine scoring ────────────────────────────────────────────────────────

fn score_volume(v: &comicvine::Volume, q: &SeriesQuery) -> f32 {
    let mut score = v.name.as_deref().map(|n| title_score(n, &q.search)).unwrap_or(0.0);

    if let Some(p) = v.publisher.as_ref().and_then(|p| p.name.as_deref()) {
        let p = p.to_lowercase();
        if REPRINT_PUBLISHERS.iter().any(|r| p.contains(r)) {
            score -= 0.15;
        }
    }
    // ComicVine's manga entries are mostly foreign editions; AniList is the
    // better source for anything that looks like manga.
    if q.looks_manga {
        score -= 0.15;
    }

    if let (Some(target), Some(start)) = (q.year, v.start_year()) {
        // A matching start year is a strong tie-breaker between reboots
        // that share a name; tapers off over a few years.
        let diff = target.abs_diff(start) as f32;
        score += 0.15 * (1.0 - (diff / 4.0).min(1.0));
        if diff > 8.0 {
            score -= 0.1;
        }
    }

    if let Some(count) = v.count_of_issues {
        let local = q.issue_numbers.len() as u32;
        if count == 0 {
            score -= 0.2;
        } else if local > 0 && count + 1 < local {
            // Fewer issues online than we have locally: probably the wrong
            // volume (or a TPB collection volume).
            score -= 0.1;
        } else if local > 0 {
            score += 0.05;
        }
    }
    score
}

/// Matches one series. `Ok(None)` means "searched fine, nothing good";
/// `Err` is a transient failure (network, quota) worth retrying later.
pub async fn match_series(q: SeriesQuery) -> Result<Option<SeriesMatch>, String> {
    let mut best: Option<SeriesMatch> = None;
    let mut last_err: Option<String> = None;

    // AniList first: free and unmetered, and conclusive for manga. If the
    // full name finds nothing, retry with just the main title.
    let mut anilist = anilist_candidates(&q.search).await;
    if let (Ok(c), Some(head)) = (&anilist, main_title(&q.search)) {
        if !c.iter().any(|m| score_anilist(m, &q) >= ACCEPT_SCORE) {
            if let Ok(more) = anilist_candidates(head).await {
                anilist = Ok(more);
            }
        }
    }
    match anilist {
        Ok(cands) => {
            if let Some((m, s)) = cands
                .into_iter()
                .map(|m| {
                    let s = score_anilist(&m, &q);
                    (m, s)
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
            {
                if s >= ACCEPT_SCORE {
                    best = Some(anilist_match(m, s));
                }
            }
        }
        Err(e) => last_err = Some(e),
    }

    let anilist_conclusive = q.looks_manga && best.as_ref().is_some_and(|b| b.score >= 0.95);

    if !anilist_conclusive && comicvine::api_key().is_some() {
        match comicvine::search_volumes(&q.search).await {
            Ok(volumes) => {
                let top = volumes
                    .into_iter()
                    .map(|v| {
                        let s = score_volume(&v, &q);
                        (v, s)
                    })
                    .max_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((v, s)) = top {
                    let beats_anilist = best.as_ref().map_or(true, |b| s > b.score);
                    if s >= ACCEPT_SCORE && beats_anilist {
                        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
                        let issues = match comicvine::volume_issues(v.id).await {
                            Ok(list) => list
                                .into_iter()
                                .filter_map(|i| {
                                    Some(IssueMeta {
                                        number: normalize_number(i.issue_number.as_deref()?),
                                        title: i.name.unwrap_or_default(),
                                        cover_date: i.cover_date.unwrap_or_default(),
                                        url: i.site_detail_url.unwrap_or_default(),
                                    })
                                })
                                .collect(),
                            Err(e) => {
                                log::warn!("ComicVine issue list for {}: {e}", v.id);
                                Vec::new()
                            }
                        };
                        best = Some(SeriesMatch {
                            source: "comicvine".into(),
                            source_id: v.id.to_string(),
                            title: v.name.clone().unwrap_or_default(),
                            publisher: v
                                .publisher
                                .as_ref()
                                .and_then(|p| p.name.clone())
                                .unwrap_or_default(),
                            year: v.start_year(),
                            description: v.summary(),
                            url: v.site_detail_url.clone().unwrap_or_default(),
                            status: String::new(),
                            genres: Vec::new(),
                            chapter_count: v.count_of_issues,
                            score: s,
                            issues,
                        });
                    }
                }
            }
            Err(e) => last_err = Some(e),
        }
    }

    match (best, last_err) {
        (Some(b), _) => Ok(Some(b)),
        (None, Some(e)) => Err(e),
        (None, None) => Ok(None),
    }
}

pub(crate) fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live lookup against AniList/ComicVine: NEBULA_META_TEST=1.
    #[test]
    fn live_match() {
        if std::env::var("NEBULA_META_TEST").is_err() {
            return;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cases: &[(&str, Option<u32>, bool, usize)] = &[
            ("Civil War 001-007 (2006-2007) GetComics.INFO", None, false, 7),
            ("Crossed v1 (000-009) (2008-2010)", None, false, 10),
            ("Crossed v2 Family Values (001-007) (2010-2011)", None, false, 7),
            ("Chronological Avengers vs X-Men (Fan-Made)", None, false, 9),
            ("Naruto (Color) 01J76XYBK4HFZMAWWKX52WN8PK", None, true, 10),
            ("Chainsaw Man (Official Colored) e896c48c-3150-437d-ba57-d8567eb399ae", None, true, 97),
            ("Jujutsu Kaisen 0_ Toukyou Toritsu Jujutsu Koutou Senmon Gakkou 8b58f452-4d8a-4aad-a050-349e83fecccb", None, true, 4),
            ("Death Note_ C-Kira-hen 3fc5c653-3fac-4ddb-aebf-f2e3d4d4adf6", None, true, 1),
        ];
        for (raw, _, manga, n) in cases {
            let c = crate::naming::clean_series_name(raw);
            let q = SeriesQuery {
                search: c.search.clone(),
                year: c.year,
                volume: c.volume,
                issue_numbers: (1..=*n).map(|i| i.to_string()).collect(),
                looks_manga: *manga,
            };
            match rt.block_on(match_series(q)) {
                Ok(Some(m)) => println!(
                    "{:<45} -> [{}] {:?} ({:?}, {}) score {:.2} rename={} issues={} first={:?}",
                    c.search, m.source, m.title, m.year, m.publisher, m.score,
                    m.score >= RENAME_SCORE, m.issues.len(),
                    m.issues.iter().find(|i| i.number == "1").map(|i| &i.title)
                ),
                Ok(None) => println!("{:<45} -> no match", c.search),
                Err(e) => println!("{:<45} -> ERROR {e}", c.search),
            }
            std::thread::sleep(std::time::Duration::from_millis(1200));
        }
    }
}
