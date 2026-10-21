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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const CLIENT: &str = "Nebula Reader";

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

    pub fn auth(&self) -> String {
        auth_header(Some(&self.token))
    }

    /// The item's file. Unlike `/Download`, reading it doesn't add a
    /// "downloading" entry to the server's activity log, which would happen
    /// for every page we fetch.
    pub fn file_url(&self, item_id: &str) -> String {
        format!("{}/Items/{item_id}/File", self.url)
    }

    pub fn download_url(&self, item_id: &str) -> String {
        format!("{}/Items/{item_id}/Download", self.url)
    }

    pub fn cover_url(&self, item_id: &str, max_height: u32) -> String {
        format!("{}/Items/{item_id}/Images/Primary?maxHeight={max_height}&quality=90", self.url)
    }

    /// Byte-range access to an item's file, for streaming pages.
    pub fn range_source(&self, item_id: &str) -> HttpRange {
        // A server that refused `/File` before is sent to `/Download` right
        // away, instead of spending a round trip on every chapter.
        let refuses_file = file_refused().lock().is_ok_and(|s| s.contains(&self.url));
        HttpRange {
            urls: [self.file_url(item_id), self.download_url(item_id)],
            current: AtomicUsize::new(refuses_file as usize),
            auth: self.auth(),
            server: self.url.clone(),
        }
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
    format!("nebula-reader-{:016x}", h.finish())
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

/// The addresses to try for what the user typed, dropping a trailing slash
/// or a pasted `/web/...` path. Without a scheme it's https first, then
/// http (home servers like `192.168.1.5:8096` are usually plain http).
pub fn candidate_urls(input: &str) -> Vec<String> {
    let mut url = input.trim().to_string();
    if let Some(i) = url.find("/web") {
        url.truncate(i);
    }
    let url = url.trim_end_matches('/').to_string();
    if url.is_empty() {
        return Vec::new();
    }
    if url.contains("://") {
        vec![url]
    } else {
        vec![format!("https://{url}"), format!("http://{url}")]
    }
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
    pub server_name: String,
}

/// Finds the Jellyfin server at what the user typed (see [`candidate_urls`]),
/// returning the address that answered.
pub async fn find_server(input: &str) -> Result<(String, ServerInfo), String> {
    let mut last = "Type the address of your Jellyfin server.".to_string();
    for url in candidate_urls(input) {
        match get_json::<ServerInfo>(&format!("{url}/System/Info/Public"), &auth_header(None)).await {
            Ok(info) => return Ok((url, info)),
            Err(e) => last = format!("No Jellyfin server found at {url} ({e})."),
        }
    }
    Err(last)
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
    /// Release date ("2021-03-14T00:00:00.0000000Z").
    pub premiere_date: Option<String>,
    pub people: Vec<Person>,
}

/// Someone credited on an item.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Person {
    pub name: String,
    /// Free text, often empty for books.
    pub role: Option<String>,
    /// "Author", "Writer", "Penciller", "Artist", "Editor", …
    #[serde(rename = "Type")]
    pub kind: Option<String>,
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
        "{}/Items?ParentId={parent_id}&SortBy=SortName&SortOrder=Ascending&Fields=ChildCount,Path,Overview,PremiereDate,People&EnableUserData=true",
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
// ── "Now reading" on the server's dashboard ─────────────────────────────────
//
// Jellyfin lists a client on its dashboard while it reports playback:
// started, progress check-ins (the server stops a session itself after 5
// minutes without one) and stopped. Books and comics have no run time, and
// for such items Jellyfin treats any progress or stop report that changes
// the play state as "played to the end": it marks the item read and clears
// the position. So these reports never carry a position, the stop is sent as
// `Failed` (the one case where the server leaves the user's data alone), and
// reading progress keeps going through `save_progress`.

async fn post_session(account: &Account, path: &str, body: serde_json::Value) -> Result<(), String> {
    let resp = client()
        .post(format!("{}{path}", account.url))
        .header("Authorization", account.auth())
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() { Ok(()) } else { Err(format!("HTTP {}", resp.status())) }
}

/// Shows `item_id` as being read by this device on the dashboard.
pub async fn report_reading_started(account: &Account, item_id: &str) -> Result<(), String> {
    let body = serde_json::json!({
        "ItemId": item_id,
        "CanSeek": false,
        "IsPaused": false,
        "PlayMethod": "DirectPlay",
    });
    post_session(account, "/Sessions/Playing", body).await
}

/// Keeps the dashboard entry alive (no position, so nothing is changed).
pub async fn report_reading_check_in(account: &Account, item_id: &str) -> Result<(), String> {
    let body = serde_json::json!({ "ItemId": item_id, "IsPaused": false, "EventName": "timeupdate" });
    post_session(account, "/Sessions/Playing/Progress", body).await
}

/// Removes the dashboard entry without touching the user's progress.
pub async fn report_reading_stopped(account: &Account, item_id: &str) -> Result<(), String> {
    post_session(account, "/Sessions/Playing/Stopped", stopped_body(item_id)).await
}

fn stopped_body(item_id: &str) -> serde_json::Value {
    serde_json::json!({ "ItemId": item_id, "Failed": true })
}

/// The same, blocking for at most a few seconds, for when the app closes.
pub fn report_reading_stopped_now(account: &Account, item_id: &str) {
    let (url, auth) = (format!("{}/Sessions/Playing/Stopped", account.url), account.auth());
    let body = stopped_body(item_id);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = blocking_client()
            .post(url)
            .header("Authorization", auth)
            .timeout(std::time::Duration::from_secs(3))
            .json(&body)
            .send();
        let _ = tx.send(r.map(|r| r.status().is_success()));
    });
    if !matches!(rx.recv_timeout(std::time::Duration::from_secs(4)), Ok(Ok(true))) {
        log::warn!("jellyfin: couldn't end the reading session on exit");
    }
}

/// What's open on a server, kept on disk while it is: if the app ends
/// without closing it (a crash, a dead battery), the server stops the
/// session 5 minutes later and marks the item read. The next start uses this
/// to end the session properly and put the last saved progress back.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenItem {
    pub account_key: String,
    pub item_id: String,
    /// The last progress saved, if any: (ticks, played).
    pub saved: Option<(i64, bool)>,
}

fn open_item_path() -> std::path::PathBuf {
    crate::library::config_dir().join("jellyfin-reading.json")
}

/// Records the open item, or clears the record with `None`.
pub fn remember_open(item: Option<&OpenItem>) {
    let path = open_item_path();
    match item.and_then(|i| serde_json::to_string(i).ok()) {
        Some(json) => {
            let _ = std::fs::write(path, json);
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The item left open by the last run, if it didn't close cleanly.
pub fn take_open() -> Option<OpenItem> {
    let path = open_item_path();
    let item = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok();
    let _ = std::fs::remove_file(path);
    item
}

/// Whether the server answers. `/System/Ping` needs no sign-in and isn't
/// logged as activity, so it's cheap to call while reading.
pub async fn ping(url: &str) -> bool {
    client()
        .get(format!("{url}/System/Ping"))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

/// Body for a progress update. `ticks` is Jellyfin's
/// `PlaybackPositionTicks`: the page × 10 000 for comics (as Jellyfin's web
/// comic reader stores it), the fraction read × 10 000 000 for books (as its
/// web book reader does).
fn progress_body(ticks: i64, played: bool) -> serde_json::Value {
    serde_json::json!({
        "PlaybackPositionTicks": if played { 0 } else { ticks },
        "Played": played,
        "LastPlayedDate": now_rfc3339(),
    })
}

pub async fn save_progress(account: &Account, item_id: &str, ticks: i64, played: bool) -> Result<(), String> {
    let resp = client()
        .post(format!("{}/UserItems/{item_id}/UserData", account.url))
        .header("Authorization", account.auth())
        .json(&progress_body(ticks, played))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() { Ok(()) } else { Err(format!("HTTP {}", resp.status())) }
}

/// The same, blocking for at most a few seconds: for the last save while
/// the app is closing, when there's no time to wait for a task. False when
/// it didn't go through.
pub fn save_progress_now(account: &Account, item_id: &str, ticks: i64, played: bool) -> bool {
    let (url, auth) = (format!("{}/UserItems/{item_id}/UserData", account.url), account.auth());
    let body = progress_body(ticks, played);
    let (tx, rx) = std::sync::mpsc::channel();
    // Its own thread: reqwest's blocking client can't run on the async
    // runtime's threads.
    std::thread::spawn(move || {
        let r = blocking_client()
            .post(url)
            .header("Authorization", auth)
            .timeout(std::time::Duration::from_secs(3))
            .json(&body)
            .send();
        let _ = tx.send(r.map(|r| r.status().is_success()));
    });
    match rx.recv_timeout(std::time::Duration::from_secs(4)) {
        Ok(Ok(true)) => true,
        other => {
            log::warn!("jellyfin: last progress save on exit didn't go through: {other:?}");
            false
        }
    }
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

/// Parses Jellyfin's dates ("2026-10-16T20:00:00.0000000Z") to Unix seconds.
pub fn parse_date(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.trim_end_matches('Z').split(':');
    let (hh, mm) = (t.next()?.parse::<i64>().ok()?, t.next()?.parse::<i64>().ok()?);
    let ss = t.next().and_then(|x| x.split('.').next()).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
    // Days-from-civil (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

// ── Library sync ────────────────────────────────────────────────────────────

/// A series (or one-shot) on the server, ready to store in the library.
#[derive(Clone, Debug)]
pub struct RemoteSeries {
    pub key: String,
    pub title: String,
    pub is_folder: bool,
    /// `jellyfin://<account>/<library id>`: groups and prunes per library.
    pub library_root: String,
    /// The Jellyfin library's name ("Manga").
    pub library_name: String,
    /// Item whose image (or first page) is the series cover.
    pub cover_item: Option<String>,
    pub issues: Vec<RemoteIssue>,
}

#[derive(Clone, Debug)]
pub struct RemoteIssue {
    pub key: String,
    pub item_id: String,
    pub name: String,
    pub extension: String,
    pub played: bool,
    pub page: usize,
    /// Unix seconds; 0 when never opened.
    pub last_read: i64,
    /// "2021-03-14", empty when the server doesn't know.
    pub release_date: String,
    /// One "Role: Name, Name" per line.
    pub creators: String,
}

fn remote_issue(account: &Account, item: &Item) -> RemoteIssue {
    let ud = item.user_data.clone().unwrap_or_default();
    RemoteIssue {
        key: account.item_key(&item.id),
        item_id: item.id.clone(),
        name: item.name.clone(),
        extension: item.extension(),
        played: ud.played,
        page: item.resume_page(),
        last_read: ud.last_played_date.as_deref().and_then(parse_date).unwrap_or(0),
        release_date: item
            .premiere_date
            .as_deref()
            .and_then(|d| d.split('T').next())
            .unwrap_or_default()
            .to_string(),
        creators: crate::library::format_credits(item.people.iter().map(|p| {
            let role = p.role.clone().filter(|r| !r.is_empty()).or_else(|| p.kind.clone()).unwrap_or_default();
            (role, p.name.clone())
        })),
    }
}

/// Everything in the account's book libraries, organised as series:
/// - a folder of books is a series (its chapters/issues);
/// - a folder holding a single folder (manga-tui's `Series/English/`) is
///   the series named by the outer folder;
/// - a folder of several series folders (a publisher) yields each one;
/// - a book at the top of a library is a one-shot.
pub async fn fetch_library(account: &Account) -> Result<Vec<RemoteSeries>, String> {
    let mut out = Vec::new();
    for lib in book_libraries(account).await? {
        let root = account.item_key(&lib.id);
        for item in children(account, &lib.id).await? {
            if item.is_folder() {
                collect_folder(account, &root, &lib.name, &item.name, &item, 0, &mut out).await?;
            } else {
                out.push(RemoteSeries {
                    key: account.item_key(&item.id),
                    title: item.name.clone(),
                    is_folder: false,
                    library_root: root.clone(),
                    library_name: lib.name.clone(),
                    // Without an image, the cover comes from its first page.
                    cover_item: Some(item.id.clone()),
                    issues: vec![remote_issue(account, &item)],
                });
            }
        }
    }
    Ok(out)
}

async fn collect_folder(
    account: &Account,
    root: &str,
    library_name: &str,
    title: &str,
    folder: &Item,
    depth: u32,
    out: &mut Vec<RemoteSeries>,
) -> Result<(), String> {
    let kids = children(account, &folder.id).await?;
    let (folders, mut books): (Vec<Item>, Vec<Item>) = kids.into_iter().partition(Item::is_folder);
    if books.is_empty() && depth < 2 {
        match folders.len() {
            0 => {}
            // Wrapper folder: keep the outer name.
            1 => return Box::pin(collect_folder(account, root, library_name, title, &folders[0], depth + 1, out)).await,
            _ => {
                for f in &folders {
                    Box::pin(collect_folder(account, root, library_name, &f.name, f, depth + 1, out)).await?;
                }
            }
        }
        return Ok(());
    }
    if books.is_empty() {
        return Ok(());
    }
    books.sort_by(|a, b| crate::comic::natural_key(&a.name).cmp(&crate::comic::natural_key(&b.name)));
    // The folder's image, else a chapter's, else the first chapter's first
    // page (the cover task falls back to that when there's no image).
    let cover_item = if folder.has_cover() {
        Some(folder.id.clone())
    } else {
        books.iter().find(|b| b.has_cover()).or(books.first()).map(|b| b.id.clone())
    };
    out.push(RemoteSeries {
        key: account.item_key(&folder.id),
        title: title.to_string(),
        is_folder: true,
        library_root: root.to_string(),
        library_name: library_name.to_string(),
        cover_item,
        issues: books.iter().map(|b| remote_issue(account, b)).collect(),
    });
    Ok(())
}

// ── Range reads for streaming ───────────────────────────────────────────────

/// An item's file on the server, read with HTTP `Range` requests. Used from
/// the reader's blocking decode threads, hence the blocking client.
///
/// Tries `/File` first and falls back to `/Download` for servers that refuse
/// it, remembering which one worked.
pub struct HttpRange {
    urls: [String; 2],
    current: AtomicUsize,
    auth: String,
    server: String,
}

/// Servers (by URL) that refused `/File` this session.
fn file_refused() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static S: OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    S.get_or_init(Default::default)
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
    /// Sends a GET, moving on to the fallback URL if the server doesn't
    /// offer the first one.
    fn send(&self, range: Option<&str>) -> Result<reqwest::blocking::Response, String> {
        loop {
            let i = self.current.load(Ordering::Relaxed);
            let mut req = blocking_client().get(&self.urls[i]).header("Authorization", &self.auth);
            if let Some(range) = range {
                req = req.header("Range", range);
            }
            let resp = req.send().map_err(|e| format!("couldn't reach the server: {e}"))?;
            let refused = matches!(resp.status().as_u16(), 403 | 404 | 405);
            if refused && i + 1 < self.urls.len() {
                self.current.store(i + 1, Ordering::Relaxed);
                if let Ok(mut s) = file_refused().lock() {
                    s.insert(self.server.clone());
                }
                continue;
            }
            return Ok(resp);
        }
    }

    fn get(&self, range: String) -> Result<reqwest::blocking::Response, String> {
        let resp = self.send(Some(&range))?;
        if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(format!("server didn't return a byte range (HTTP {})", resp.status()));
        }
        Ok(resp)
    }

    /// Downloads the whole file, for formats that can't be streamed.
    pub fn download_to(&self, path: &std::path::Path) -> Result<(), String> {
        let mut resp = self.send(None)?;
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

// ── Downloads for offline reading ──────────────────────────────────────────

/// Where downloads are kept unless the user picks a folder: in the data
/// folder, not the cache, since they're the user's to keep. Each account
/// has a folder in it, and files are named `<item id>.<ext>` (the book
/// reader reads the item id back from the file name).
pub fn default_download_root() -> std::path::PathBuf {
    crate::platform::data_dir().join("jellyfin-downloads")
}

/// A running download's progress, shared with the UI; `cancel` stops it.
#[derive(Debug, Default)]
pub struct Progress {
    pub done: std::sync::atomic::AtomicU64,
    pub total: std::sync::atomic::AtomicU64,
    pub cancel: std::sync::atomic::AtomicBool,
}

impl Progress {
    /// 0.0–1.0, or 0 until the size is known.
    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        (self.done.load(Ordering::Relaxed) as f64 / total as f64).min(1.0) as f32
    }
}

/// No overall time limit (a big chapter on a slow link takes a while), but
/// gives up when the connection stalls.
fn download_client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .read_timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("http client")
    })
}

/// Downloads an item's file to `dest` (via `dest.part`, flushed to the drive
/// and renamed when complete), refusing when the drive is too full. The
/// folder must exist: it's never created here, so a download can't land in
/// an empty mount point when its drive has been unplugged. Returns its size.
pub async fn download_item(
    account: &Account,
    item_id: &str,
    dest: &std::path::Path,
    progress: &Progress,
) -> Result<u64, String> {
    use std::io::Write;

    let mut resp = None;
    for url in [account.file_url(item_id), account.download_url(item_id)] {
        let r = download_client()
            .get(&url)
            .header("Authorization", account.auth())
            .send()
            .await
            .map_err(|e| format!("couldn't reach the server: {e}"))?;
        // Servers without /File answer 403/404/405; try /Download.
        if matches!(r.status().as_u16(), 403 | 404 | 405) {
            continue;
        }
        resp = Some(r);
        break;
    }
    let mut resp = resp.ok_or("the server doesn't offer this file for download")?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {}", resp.status()));
    }
    progress.total.store(resp.content_length().unwrap_or(0), Ordering::Relaxed);

    let dir = dest.parent().filter(|d| d.is_dir()).ok_or("the download folder isn't available")?;
    // Leave some room: a completely full drive is no good to anyone.
    const SPARE: u64 = 64 * 1024 * 1024;
    if let (Some(len), Some(free)) = (resp.content_length(), crate::platform::disk::free_space(dir)) {
        if len + SPARE > free {
            return Err(format!(
                "not enough space on the drive: {} MB needed, {} MB free",
                (len + SPARE) / 1_048_576,
                free / 1_048_576
            ));
        }
    }
    let part = dest.with_extension("part");
    let result = async {
        let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
        let mut size = 0u64;
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("download failed: {e}"))? {
            if progress.cancel.load(Ordering::Relaxed) {
                return Err("cancelled".to_string());
            }
            file.write_all(&chunk).map_err(|e| format!("couldn't write to the download folder: {e}"))?;
            size += chunk.len() as u64;
            progress.done.store(size, Ordering::Relaxed);
        }
        file.sync_all().map_err(|e| e.to_string())?;
        Ok(size)
    }
    .await;
    match result {
        Ok(size) => {
            std::fs::rename(&part, dest).map_err(|e| e.to_string())?;
            crate::platform::disk::sync_dir(dest);
            Ok(size)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// `bytes 1234-5677/5678` → 5678.
fn parse_content_range_total(v: &str) -> Option<u64> {
    v.rsplit('/').next()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-shot HTTP server: answers each request in turn with the given
    /// status and body, and records the request paths.
    fn serve(responses: Vec<(u16, Vec<u8>)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut paths = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                paths.push(line.split_whitespace().nth(1).unwrap_or("").to_string());
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                }
                let head = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
            paths
        });
        (url, handle)
    }

    fn test_account(url: String) -> Account {
        Account {
            url,
            server_id: "s".into(),
            server_name: "Test".into(),
            user_id: "u".into(),
            user_name: "me".into(),
            token: "t".into(),
        }
    }

    #[test]
    fn downloads_a_file_falling_back_to_download_url() {
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        // /File refused (older servers), /Download serves it.
        let (url, server) = serve(vec![(404, Vec::new()), (200, body.clone())]);
        let dir = std::env::temp_dir().join(format!("nebula-dl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("item1.cbz");
        let progress = Progress::default();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let size = rt.block_on(download_item(&test_account(url), "item1", &dest, &progress)).unwrap();
        assert_eq!(size, body.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(!dest.with_extension("part").exists());
        assert_eq!(progress.fraction(), 1.0);
        assert_eq!(server.join().unwrap(), ["/Items/item1/File", "/Items/item1/Download"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cancelled_download_leaves_nothing_behind() {
        let (url, server) = serve(vec![(200, vec![7u8; 100_000])]);
        let dir = std::env::temp_dir().join(format!("nebula-dl-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("item2.cbz");
        let progress = Progress::default();
        progress.cancel.store(true, Ordering::Relaxed);
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let result = rt.block_on(download_item(&test_account(url), "item2", &dest, &progress));
        assert_eq!(result, Err("cancelled".to_string()));
        assert!(!dest.exists() && !dest.with_extension("part").exists());
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn candidate_urls_for_typed_addresses() {
        assert_eq!(candidate_urls("sloppyjoe.org:6969"), ["https://sloppyjoe.org:6969", "http://sloppyjoe.org:6969"]);
        assert_eq!(candidate_urls("http://192.168.50.87:8096/"), ["http://192.168.50.87:8096"]);
        assert_eq!(candidate_urls("https://jf.example/web/#/home.html"), ["https://jf.example"]);
        assert!(candidate_urls("  ").is_empty());
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
        assert_eq!(parse_date("2025-10-16T20:03:20.0000000Z"), Some(1_760_645_000));
        assert_eq!(parse_date("2000-02-29T00:00:00Z"), Some(951_782_400));
        assert_eq!(parse_date("garbage"), None);
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_760_645_000), "2025-10-16T20:03:20Z");
        assert_eq!(rfc3339(4_102_444_799), "2099-12-31T23:59:59Z");
        let now = now_rfc3339();
        assert_eq!(now.len(), 20);
        assert!(now.starts_with("20") && now.ends_with('Z'));
    }
}
