//! Jellyfin as a library source: sign-in, browsing book libraries, covers,
//! streaming downloads and reading progress.
//!
//! Sign-in uses Quick Connect where the server allows it: the app shows a
//! code, the user approves it in Jellyfin, and the app gets its own access
//! token, so no password passes through here. Servers with Quick Connect
//! off fall back to username and password, used once and never saved. Only
//! tokens are stored (`jellyfin.json` in the config folder, owner-only);
//! each one shows up in Jellyfin as a device and can be revoked there.
//!
//! Progress uses Jellyfin's own fields the way its web comic reader does,
//! `PlaybackPositionTicks = page × 10000` and `Played`, so either reader can
//! resume where the other left off.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const CLIENT: &str = "Cosmic Comic";

/// One signed-in user on one server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Account {
    /// Base URL without a trailing slash, e.g. `https://jellyfin.example:8920`.
    pub url: String,
    pub server_id: String,
    pub server_name: String,
    pub user_id: String,
    pub user_name: String,
    pub token: String,
}

impl Account {
    /// Stable id for this server + user, used in library keys.
    pub fn key(&self) -> String {
        format!("{}-{}", self.server_id, self.user_id)
    }

    /// Library key for one of this account's items.
    pub fn item_key(&self, item_id: &str) -> String {
        format!("{KEY_PREFIX}{}/{item_id}", self.key())
    }

    pub fn label(&self) -> String {
        format!("{} on {}", self.user_name, self.server_name)
    }

    fn auth(&self) -> String {
        auth_header(Some(&self.token))
    }

    pub fn download_url(&self, item_id: &str) -> String {
        format!("{}/Items/{item_id}/Download", self.url)
    }

    pub fn cover_url(&self, item_id: &str, max_height: u32) -> String {
        format!("{}/Items/{item_id}/Images/Primary?maxHeight={max_height}&quality=90", self.url)
    }

    /// Byte-range access to an item's file, for streaming pages.
    pub fn range_source(&self, item_id: &str) -> HttpRange {
        HttpRange { url: self.download_url(item_id), auth: self.auth() }
    }
}

pub const KEY_PREFIX: &str = "jellyfin://";

/// Splits a `jellyfin://<account>/<item>` key.
pub fn parse_key(key: &str) -> Option<(&str, &str)> {
    key.strip_prefix(KEY_PREFIX)?.split_once('/')
}

pub fn is_key(key: &str) -> bool {
    key.starts_with(KEY_PREFIX)
}

// ── Stored accounts ─────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Accounts {
    /// Identifies this install to Jellyfin (one device entry per install).
    pub device_id: String,
    pub accounts: Vec<Account>,
}

fn accounts_path() -> std::path::PathBuf {
    crate::library::config_dir().join("jellyfin.json")
}

impl Accounts {
    pub fn load() -> Self {
        let mut a: Accounts = std::fs::read(accounts_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if a.device_id.is_empty() {
            a.device_id = new_device_id();
            a.save();
        }
        a
    }

    pub fn save(&self) {
        let path = accounts_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(&path, json);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }

    pub fn find(&self, key: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.key() == key)
    }

    /// Adds or replaces (same server + user) an account.
    pub fn upsert(&mut self, account: Account) {
        self.accounts.retain(|a| a.key() != account.key());
        self.accounts.push(account);
        self.save();
    }

    pub fn remove(&mut self, key: &str) {
        self.accounts.retain(|a| a.key() != key);
        self.save();
    }
}

fn new_device_id() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut h);
    std::process::id().hash(&mut h);
    device_name().hash(&mut h);
    format!("cosmic-comic-{:016x}", h.finish())
}

fn device_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Desktop".into())
}

static DEVICE_ID: OnceLock<String> = OnceLock::new();

fn auth_header(token: Option<&str>) -> String {
    let device_id = DEVICE_ID.get_or_init(|| Accounts::load().device_id);
    let mut h = format!(
        r#"MediaBrowser Client="{CLIENT}", Device="{}", DeviceId="{device_id}", Version="{}""#,
        device_name().replace('"', ""),
        env!("CARGO_PKG_VERSION")
    );
    if let Some(t) = token {
        h.push_str(&format!(r#", Token="{t}""#));
    }
    h
}

// ── HTTP ────────────────────────────────────────────────────────────────────

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("http client")
    })
}

/// Normalises what the user typed: adds `https://` when no scheme was given
/// and drops a trailing slash or a pasted `/web/...` path.
pub fn normalize_url(input: &str) -> String {
    let mut url = input.trim().to_string();
    if !url.contains("://") {
        url = format!("https://{url}");
    }
    if let Some(i) = url.find("/web") {
        url.truncate(i);
    }
    url.trim_end_matches('/').to_string()
}

async fn get_json<T: serde::de::DeserializeOwned>(url: &str, auth: &str) -> Result<T, String> {
    let resp = client()
        .get(url)
        .header("Authorization", auth)
        .send()
        .await
        .map_err(|e| format!("couldn't reach the server: {e}"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("signed out by the server (sign in again)".into());
    }
    if !status.is_success() {
        return Err(format!("server error: HTTP {status}"));
    }
    resp.json().await.map_err(|e| format!("unexpected reply: {e}"))
}

async fn post_json<T: serde::de::DeserializeOwned>(url: &str, auth: &str, body: &serde_json::Value) -> Result<T, String> {
    let resp = client()
        .post(url)
        .header("Authorization", auth)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("couldn't reach the server: {e}"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("not authorized".into());
    }
    if !status.is_success() {
        return Err(format!("server error: HTTP {status}"));
    }
    resp.json().await.map_err(|e| format!("unexpected reply: {e}"))
}

// ── Sign-in ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ServerInfo {
    pub id: String,
    pub server_name: String,
    #[serde(default)]
    pub version: String,
}

/// Checks that `url` is a Jellyfin server (no sign-in needed).
pub async fn server_info(url: &str) -> Result<ServerInfo, String> {
    get_json(&format!("{url}/System/Info/Public"), &auth_header(None))
        .await
        .map_err(|e| format!("{e} — is this a Jellyfin address?"))
}

pub async fn quick_connect_enabled(url: &str) -> bool {
    get_json::<bool>(&format!("{url}/QuickConnect/Enabled"), &auth_header(None)).await.unwrap_or(false)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QuickConnect {
    pub secret: String,
    pub code: String,
    #[serde(default)]
    pub authenticated: bool,
}

/// Starts Quick Connect: the returned `code` is what the user approves.
pub async fn quick_connect_start(url: &str) -> Result<QuickConnect, String> {
    post_json(&format!("{url}/QuickConnect/Initiate"), &auth_header(None), &serde_json::json!({})).await
}

/// Whether the user has approved the code yet.
pub async fn quick_connect_approved(url: &str, secret: &str) -> Result<bool, String> {
    let state: QuickConnect =
        get_json(&format!("{url}/QuickConnect/Connect?Secret={secret}"), &auth_header(None)).await?;
    Ok(state.authenticated)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AuthResult {
    user: AuthUser,
    access_token: String,
    server_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AuthUser {
    id: String,
    name: String,
}

fn account_from(url: &str, server_name: &str, r: AuthResult) -> Account {
    Account {
        url: url.to_string(),
        server_id: r.server_id,
        server_name: server_name.to_string(),
        user_id: r.user.id,
        user_name: r.user.name,
        token: r.access_token,
    }
}

/// Finishes Quick Connect once approved.
pub async fn quick_connect_finish(url: &str, server_name: &str, secret: &str) -> Result<Account, String> {
    let r = post_json(
        &format!("{url}/Users/AuthenticateWithQuickConnect"),
        &auth_header(None),
        &serde_json::json!({ "Secret": secret }),
    )
    .await?;
    Ok(account_from(url, server_name, r))
}

/// Username/password sign-in, for servers with Quick Connect turned off.
/// The password is sent once to get a token and not kept.
pub async fn sign_in_with_password(url: &str, server_name: &str, user: &str, password: &str) -> Result<Account, String> {
    let r = post_json(
        &format!("{url}/Users/AuthenticateByName"),
        &auth_header(None),
        &serde_json::json!({ "Username": user, "Pw": password }),
    )
    .await
    .map_err(|e| if e == "not authorized" { "wrong username or password".into() } else { e })?;
    Ok(account_from(url, server_name, r))
}

/// Ends this app's session on the server (revokes the token).
pub async fn sign_out(account: &Account) {
    let _ = client()
        .post(format!("{}/Sessions/Logout", account.url))
        .header("Authorization", account.auth())
        .send()
        .await;
}

// ── Browsing ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserData {
    pub played: bool,
    pub playback_position_ticks: i64,
    pub last_played_date: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Item {
    pub id: String,
    pub name: String,
    /// "Folder", "Book", "CollectionFolder", …
    #[serde(rename = "Type")]
    pub kind: String,
    pub collection_type: Option<String>,
    pub child_count: Option<u32>,
    pub path: Option<String>,
    pub overview: Option<String>,
    pub image_tags: HashMap<String, String>,
    pub user_data: Option<UserData>,
}

impl Item {
    pub fn is_folder(&self) -> bool {
        self.kind.ends_with("Folder")
    }

    pub fn has_cover(&self) -> bool {
        self.image_tags.contains_key("Primary")
    }

    /// File extension of the underlying file, lowercase ("cbz", "epub"…).
    pub fn extension(&self) -> String {
        self.path
            .as_deref()
            .and_then(|p| p.rsplit(['/', '\\']).next())
            .and_then(|f| f.rsplit_once('.'))
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default()
    }

    /// The page to resume at, stored the way Jellyfin's web reader does.
    pub fn resume_page(&self) -> usize {
        self.user_data.as_ref().map(|u| (u.playback_position_ticks / 10_000).max(0) as usize).unwrap_or(0)
    }

    pub fn played(&self) -> bool {
        self.user_data.as_ref().is_some_and(|u| u.played)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ItemsPage {
    items: Vec<Item>,
}

/// The user's book libraries. Video and music libraries are left out.
pub async fn book_libraries(account: &Account) -> Result<Vec<Item>, String> {
    let page: ItemsPage = get_json(&format!("{}/UserViews", account.url), &account.auth()).await?;
    Ok(page.items.into_iter().filter(|v| v.collection_type.as_deref() == Some("books")).collect())
}

/// Direct children of a library or folder, with the user's read state.
pub async fn children(account: &Account, parent_id: &str) -> Result<Vec<Item>, String> {
    let url = format!(
        "{}/Items?ParentId={parent_id}&SortBy=SortName&SortOrder=Ascending&Fields=ChildCount,Path,Overview&EnableUserData=true",
        account.url
    );
    let page: ItemsPage = get_json(&url, &account.auth()).await?;
    Ok(page.items)
}

/// Downloads an item's cover image.
pub async fn cover(account: &Account, item_id: &str, max_height: u32) -> Result<Vec<u8>, String> {
    let resp = client()
        .get(account.cover_url(item_id, max_height))
        .header("Authorization", account.auth())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.bytes().await.map(|b| b.to_vec()).map_err(|e| e.to_string())
}

/// Saves reading progress to Jellyfin: the page (as Jellyfin's web reader
/// stores it) and whether the item is finished.
pub async fn save_progress(account: &Account, item_id: &str, page: usize, played: bool) -> Result<(), String> {
    let body = serde_json::json!({
        "PlaybackPositionTicks": if played { 0 } else { page as i64 * 10_000 },
        "Played": played,
        "LastPlayedDate": now_rfc3339(),
    });
    let resp = client()
        .post(format!("{}/UserItems/{item_id}/UserData", account.url))
        .header("Authorization", account.auth())
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() { Ok(()) } else { Err(format!("HTTP {}", resp.status())) }
}

/// UTC "now" as RFC 3339, without pulling in a date crate.
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    rfc3339(secs)
}

fn rfc3339(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

// ── Range reads for streaming ───────────────────────────────────────────────

/// An item's file on the server, read with HTTP `Range` requests. Used from
/// the reader's blocking decode threads, hence the blocking client.
pub struct HttpRange {
    url: String,
    auth: String,
}

fn blocking_client() -> &'static reqwest::blocking::Client {
    static C: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("http client")
    })
}

impl HttpRange {
    fn get(&self, range: String) -> Result<reqwest::blocking::Response, String> {
        let resp = blocking_client()
            .get(&self.url)
            .header("Authorization", &self.auth)
            .header("Range", range)
            .send()
            .map_err(|e| format!("couldn't reach the server: {e}"))?;
        if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(format!("server didn't return a byte range (HTTP {})", resp.status()));
        }
        Ok(resp)
    }

    /// Downloads the whole file, for formats that can't be streamed.
    pub fn download_to(&self, path: &std::path::Path) -> Result<(), String> {
        let mut resp = blocking_client()
            .get(&self.url)
            .header("Authorization", &self.auth)
            .send()
            .map_err(|e| format!("couldn't reach the server: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("download failed: HTTP {}", resp.status()));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("part");
        let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        resp.copy_to(&mut file).map_err(|e| format!("download failed: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
}

impl crate::remote_zip::RangeSource for HttpRange {
    fn tail(&self, n: u64) -> Result<(Vec<u8>, u64), String> {
        let resp = self.get(format!("bytes=-{n}"))?;
        let total = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_range_total)
            .ok_or("server didn't say how big the file is")?;
        let bytes = resp.bytes().map_err(|e| e.to_string())?.to_vec();
        Ok((bytes, total))
    }

    fn range(&self, start: u64, end: u64) -> Result<Vec<u8>, String> {
        let resp = self.get(format!("bytes={start}-{}", end.saturating_sub(1)))?;
        Ok(resp.bytes().map_err(|e| e.to_string())?.to_vec())
    }
}

/// `bytes 1234-5677/5678` → 5678.
fn parse_content_range_total(v: &str) -> Option<u64> {
    v.rsplit('/').next()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_typed_urls() {
        assert_eq!(normalize_url("sloppyjoe.org:6969"), "https://sloppyjoe.org:6969");
        assert_eq!(normalize_url("http://192.168.50.87:8096/"), "http://192.168.50.87:8096");
        assert_eq!(normalize_url("https://jf.example/web/#/home.html"), "https://jf.example");
    }

    #[test]
    fn keys_round_trip() {
        let a = Account {
            url: "https://x".into(),
            server_id: "srv".into(),
            server_name: "deepdish".into(),
            user_id: "usr".into(),
            user_name: "tsingel".into(),
            token: "t".into(),
        };
        let key = a.item_key("281a47ce");
        assert!(is_key(&key));
        assert_eq!(parse_key(&key), Some(("srv-usr", "281a47ce")));
        assert_eq!(a.label(), "tsingel on deepdish");
    }

    /// Shapes copied from a real Jellyfin 12.2 server's replies.
    #[test]
    fn parses_real_items() {
        let json = r#"{"Items":[
            {"Name":"Attack on Titan","Id":"f30ce3b2cd5563760c669618438d9b6c","Type":"Folder","ChildCount":118,
             "ImageTags":{"Primary":"abc"},"UserData":{"Played":false,"PlaybackPositionTicks":0}},
            {"Name":"Ch 1 Vol none  Weeb central 01J76XYYGM51FEZH8PK35QB3SA","Id":"281a47ceb2466267cd2ab0a4c949e3ad",
             "Type":"Book","Path":"/media/Manga/Attack on Titan/Ch 1 Vol none  Weeb central 01J76XYYGM51FEZH8PK35QB3SA.cbz",
             "ImageTags":{},"UserData":{"Played":false,"PlaybackPositionTicks":120000,"LastPlayedDate":"2026-10-16T20:00:00Z"}}
        ],"TotalRecordCount":2}"#;
        let page: ItemsPage = serde_json::from_str(json).unwrap();
        let (series, chapter) = (&page.items[0], &page.items[1]);
        assert!(series.is_folder() && series.has_cover());
        assert_eq!(series.child_count, Some(118));
        assert!(!chapter.is_folder() && !chapter.has_cover());
        assert_eq!(chapter.extension(), "cbz");
        assert_eq!(chapter.resume_page(), 12);
    }

    #[test]
    fn only_book_libraries() {
        let json = r#"{"Items":[
            {"Name":"Anime","Id":"1","Type":"CollectionFolder","CollectionType":"tvshows"},
            {"Name":"Manga","Id":"2","Type":"CollectionFolder","CollectionType":"books"},
            {"Name":"Movies","Id":"3","Type":"CollectionFolder","CollectionType":"movies"}
        ]}"#;
        let page: ItemsPage = serde_json::from_str(json).unwrap();
        let books: Vec<_> = page.items.into_iter().filter(|v| v.collection_type.as_deref() == Some("books")).collect();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].name, "Manga");
    }

    #[test]
    fn content_range_and_dates() {
        assert_eq!(parse_content_range_total("bytes 7275-7374/7375"), Some(7375));
        assert_eq!(parse_content_range_total("bytes 0-99/*"), None);
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_760_645_000), "2025-10-16T20:03:20Z");
        assert_eq!(rfc3339(4_102_444_799), "2099-12-31T23:59:59Z");
        let now = now_rfc3339();
        assert_eq!(now.len(), 20);
        assert!(now.starts_with("20") && now.ends_with('Z'));
    }
}
