//! ComicVine — the standard database for Western (non-manga) comics. Used
//! to give series a canonical name/publisher/description, and to fetch a
//! volume's full issue list so each local issue can be linked to its
//! official number, title and cover date.
//!
//! ComicVine asks clients to stay around one request per second and caps
//! each resource at 200 requests/hour, so callers fetch at most a couple of
//! requests per series and the app runs lookups one series at a time.

use serde::Deserialize;

use crate::metadata::strip_html;

#[derive(Deserialize)]
struct Response<T> {
    error: String,
    #[serde(default)]
    number_of_total_results: u32,
    results: Vec<T>,
}

/// A single resource (`/volume/4050-…/`) answers with an object, not a list.
#[derive(Deserialize)]
struct One<T> {
    error: String,
    results: T,
}

#[derive(Deserialize, Default)]
struct VolumePeople {
    #[serde(default)]
    people: Vec<Person>,
}

#[derive(Deserialize)]
struct Person {
    name: Option<String>,
    /// Issues they're credited on, as a string ("12").
    count: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Volume {
    pub id: u64,
    pub name: Option<String>,
    pub start_year: Option<String>,
    pub publisher: Option<NameRef>,
    pub count_of_issues: Option<u32>,
    pub deck: Option<String>,
    pub description: Option<String>,
    pub site_detail_url: Option<String>,
}

impl Volume {
    pub fn start_year(&self) -> Option<u32> {
        self.start_year.as_deref().and_then(|s| s.trim().parse().ok())
    }

    pub fn summary(&self) -> String {
        let text = self
            .deck
            .as_deref()
            .filter(|d| !d.trim().is_empty())
            .or(self.description.as_deref())
            .map(strip_html)
            .unwrap_or_default();
        truncate(&text, 1500)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct NameRef {
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Issue {
    pub name: Option<String>,
    pub issue_number: Option<String>,
    pub cover_date: Option<String>,
    pub site_detail_url: Option<String>,
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max).collect::<String>().trim_end())
}

/// Reads the ComicVine API key from `COMICVINE_API_KEY`, falling back to
/// `<data dir>/comicvine_api_key.txt` (handy for desktop-launched instances
/// where env vars aren't set). Get a free key at comicvine.gamespot.com/api/.
pub fn api_key() -> Option<String> {
    if let Ok(k) = std::env::var("COMICVINE_API_KEY") {
        let k = k.trim();
        if !k.is_empty() {
            return Some(k.to_string());
        }
    }
    std::fs::read_to_string(api_key_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn api_key_path() -> std::path::PathBuf {
    crate::library::app_data_dir().join("comicvine_api_key.txt")
}

/// Saves (or, if empty, removes) the key file used by [`api_key`].
pub fn save_api_key(key: &str) -> Result<(), String> {
    let path = api_key_path();
    let key = key.trim();
    if key.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, key).map_err(|e| format!("save key: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("nebula-reader/0.1 (+https://github.com/NotTimm/NebulaReader)")
        .build()
        .map_err(|e| format!("http client error: {e}"))
}

async fn get<T: for<'de> Deserialize<'de>>(
    url: &str,
    query: &[(&str, &str)],
) -> Result<Response<T>, String> {
    let key = api_key().ok_or("no ComicVine API key")?;
    let mut q: Vec<(&str, &str)> = vec![("api_key", key.as_str()), ("format", "json")];
    q.extend_from_slice(query);
    let resp: Response<T> = client()?
        .get(url)
        .query(&q)
        .send()
        .await
        .map_err(|e| format!("network error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("ComicVine parse error: {e}"))?;
    if resp.error != "OK" {
        return Err(format!("ComicVine error: {}", resp.error));
    }
    Ok(resp)
}

/// Searches ComicVine volumes (series) by name, best text matches first.
pub async fn search_volumes(query: &str) -> Result<Vec<Volume>, String> {
    let resp: Response<Volume> = get(
        "https://comicvine.gamespot.com/api/search/",
        &[
            ("resources", "volume"),
            ("query", query),
            ("limit", "10"),
            (
                "field_list",
                "id,name,start_year,publisher,count_of_issues,deck,description,site_detail_url",
            ),
        ],
    )
    .await?;
    Ok(resp.results)
}

/// The people credited most often across a volume: its main creators.
pub async fn volume_creators(volume_id: u64) -> Result<Vec<String>, String> {
    let key = api_key().ok_or("no ComicVine API key")?;
    let url = format!("https://comicvine.gamespot.com/api/volume/4050-{volume_id}/");
    let resp: One<VolumePeople> = client()?
        .get(url)
        .query(&[("api_key", key.as_str()), ("format", "json"), ("field_list", "people")])
        .send()
        .await
        .map_err(|e| format!("network error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("ComicVine parse error: {e}"))?;
    if resp.error != "OK" {
        return Err(format!("ComicVine error: {}", resp.error));
    }
    let mut people: Vec<(u32, String)> = resp
        .results
        .people
        .into_iter()
        .filter_map(|p| Some((p.count.and_then(|c| c.parse().ok()).unwrap_or(0), p.name?)))
        .collect();
    people.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(people.into_iter().take(4).map(|(_, name)| name).collect())
}

/// Every issue in a volume (paginated, capped at 500 — longer runs are
/// rare and would burn through the hourly request quota).
pub async fn volume_issues(volume_id: u64) -> Result<Vec<Issue>, String> {
    let filter = format!("volume:{volume_id}");
    let mut out = Vec::new();
    let mut offset = 0u32;
    loop {
        let off = offset.to_string();
        let resp: Response<Issue> = get(
            "https://comicvine.gamespot.com/api/issues/",
            &[
                ("filter", filter.as_str()),
                ("field_list", "name,issue_number,cover_date,site_detail_url"),
                ("limit", "100"),
                ("offset", off.as_str()),
            ],
        )
        .await?;
        let got = resp.results.len() as u32;
        out.extend(resp.results);
        offset += got;
        if got == 0 || offset >= resp.number_of_total_results || offset >= 500 {
            break;
        }
        // Be polite between pages.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    }
    Ok(out)
}
