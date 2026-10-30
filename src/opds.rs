//! OPDS catalogs as a library source: reading feeds (OPDS 1.2 Atom and
//! OPDS 2.0 JSON), walking the folders the user picked into series and
//! chapters, and getting the books themselves.
//!
//! An OPDS catalog is a web of feeds: navigation feeds (folders that link to
//! other feeds) and acquisition feeds (lists of books). There's no fixed
//! depth and the same book can be reached several ways ("By author", "By
//! series", "Newest"), so the app doesn't guess: the user picks which
//! folders go in the library (`Account::roots`), and only those are walked,
//! with a list of feeds already seen and limits on depth and size.
//!
//! A feed of books is a series named after the feed, unless the books say
//! which series they're in (OPDS 2 `belongsTo`). Books listed straight in a
//! picked folder are grouped by the series name in their titles.
//!
//! Books are read, best first: streamed from their file with HTTP range
//! requests (CBZ, CBR), page by page as images the server renders (the
//! Page Streaming Extension, or a Divina manifest), or downloaded whole.
//! OPDS has no way to save reading progress to the server, so progress is
//! kept on this computer; a server's own page-streaming position
//! (`pse:lastRead`) is read when it's there.
//!
//! Accounts are kept in `opds.json` in the config folder (owner-only). A
//! catalog with sign-in uses HTTP Basic, so the password is stored there
//! too: OPDS has no tokens. It's only ever sent to the catalog's own host.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, RwLock};

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use crate::comic::PageSource;
use crate::epub::{attr, event_text, local_name};
use crate::remote::{Progress, RemoteIssue, RemoteSeries};

pub const KEY_PREFIX: &str = "opds://";

/// Folders below a picked one that are walked, at most.
const MAX_DEPTH: usize = 4;
/// Feed pages fetched in one sync, at most, so a huge catalog can't run away.
const MAX_FEEDS: usize = 20_000;
/// Pages of one feed followed through `next` links, at most.
const MAX_PAGES: usize = 400;
/// Stands for the page number in a page-streaming link until it's filled in.
const PAGE_MARK: &str = "NEBULAPAGENUMBER";

/// Splits an `opds://<account>/<item>` key.
pub fn parse_key(key: &str) -> Option<(&str, &str)> {
    key.strip_prefix(KEY_PREFIX)?.split_once('/')
}

pub fn is_key(key: &str) -> bool {
    key.starts_with(KEY_PREFIX)
}

/// A stable 64-bit FNV-1a hash, hex: item ids are derived from the
/// catalog's own ids and must never change between runs or versions.
fn hash(s: &str) -> String {
    let h = s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    format!("{h:016x}")
}

/// The item id of a book, from its OPDS `id`: short, and safe as a file name.
pub fn book_id(opds_id: &str) -> String {
    format!("o{}", hash(opds_id))
}

// ── Accounts ────────────────────────────────────────────────────────────────

/// A folder of the catalog that goes in the library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Root {
    pub url: String,
    pub title: String,
}

/// One catalog (and user, when it has sign-in).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    /// The catalog's start feed.
    pub url: String,
    /// The catalog's own name, from its start feed.
    pub title: String,
    pub user: String,
    pub password: String,
    /// The folders put in the library.
    pub roots: Vec<Root>,
}

impl Account {
    /// Stable id for this catalog + user, used in library keys.
    pub fn key(&self) -> String {
        format!("opds-{}", hash(&format!("{}\n{}", self.url, self.user)))
    }

    /// Library key for one of this account's items.
    pub fn item_key(&self, item_id: &str) -> String {
        format!("{KEY_PREFIX}{}/{item_id}", self.key())
    }

    /// The catalog's name, or its host when it has none.
    pub fn name(&self) -> &str {
        if !self.title.is_empty() {
            return &self.title;
        }
        let rest = self.url.split_once("://").map_or(self.url.as_str(), |(_, r)| r);
        rest.split('/').next().unwrap_or(rest)
    }

    pub fn label(&self) -> String {
        if self.user.is_empty() {
            self.name().to_string()
        } else {
            fl!("jf-account", user = self.user.as_str(), server = self.name())
        }
    }

    /// The sign-in for a request to `url`: only the catalog's own host gets
    /// it, never a CDN or another site a feed links to.
    fn auth_for(&self, url: &str) -> Option<(String, String)> {
        if self.user.is_empty() {
            return None;
        }
        let host = |u: &str| url::Url::parse(u).ok().map(|u| (u.scheme().to_string(), u.host_str().map(str::to_string), u.port_or_known_default()));
        (host(url).is_some() && host(url) == host(&self.url)).then(|| (self.user.clone(), self.password.clone()))
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let mut req = client().get(url).header("Accept", ACCEPT);
        if let Some((user, password)) = self.auth_for(url) {
            req = req.basic_auth(user, Some(password));
        }
        req
    }

    fn links(&self, item_id: &str) -> Option<Links> {
        links(&self.key(), item_id)
    }

    /// An item's file, for streaming its pages or fetching it whole.
    pub fn file(&self, item_id: &str) -> HttpFile {
        let url = self.links(item_id).and_then(|l| l.file).unwrap_or_default();
        HttpFile { auth: self.auth_for(&url), url }
    }

    /// Its pages as images from the server, when it has page streaming.
    pub fn page_stream(&self, item_id: &str) -> Option<Vec<PageSource>> {
        let links = self.links(item_id)?;
        let page = |url: String| PageSource::Web(Arc::new(WebPage { auth: self.auth_for(&url), url }));
        if let Some(p) = links.pages.filter(|p| p.count > 0) {
            return Some((0..p.count).map(|n| page(p.template.replace(PAGE_MARK, &n.to_string()))).collect());
        }
        let manifest = links.manifest?;
        let mut req = blocking_client().get(&manifest);
        if let Some((user, password)) = self.auth_for(&manifest) {
            req = req.basic_auth(user, Some(password));
        }
        let json: serde_json::Value = req.send().ok()?.error_for_status().ok()?.json().ok()?;
        let base = url::Url::parse(&manifest).ok()?;
        let pages: Vec<PageSource> = json["readingOrder"]
            .as_array()?
            .iter()
            .filter_map(|l| l["href"].as_str())
            .filter_map(|href| base.join(href).ok())
            .map(|u| page(u.to_string()))
            .collect();
        (!pages.is_empty()).then_some(pages)
    }

    /// An item's cover image.
    pub async fn cover(&self, item_id: &str) -> Result<Vec<u8>, String> {
        let url = self.links(item_id).and_then(|l| l.cover).ok_or("no cover")?;
        let resp = self.get(&url).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        resp.bytes().await.map(|b| b.to_vec()).map_err(|e| e.to_string())
    }

    /// Whether the catalog answers.
    pub async fn ping(&self) -> bool {
        self.get(&self.url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    /// Downloads an item for reading offline, see [`crate::remote::save_response`].
    pub async fn download_item(&self, item_id: &str, dest: &std::path::Path, progress: &Progress) -> Result<u64, String> {
        let url = self.links(item_id).and_then(|l| l.file).ok_or_else(|| fl!("opds-no-file"))?;
        let resp = download_client()
            .get(&url)
            .basic_auth_maybe(self.auth_for(&url))
            .send()
            .await
            .map_err(|e| fl!("jf-unreachable", error = e.to_string()))?;
        crate::remote::save_response(resp, dest, progress).await
    }
}

trait BasicAuthMaybe {
    fn basic_auth_maybe(self, auth: Option<(String, String)>) -> Self;
}

impl BasicAuthMaybe for reqwest::RequestBuilder {
    fn basic_auth_maybe(self, auth: Option<(String, String)>) -> Self {
        match auth {
            Some((user, password)) => self.basic_auth(user, Some(password)),
            None => self,
        }
    }
}

impl BasicAuthMaybe for reqwest::blocking::RequestBuilder {
    fn basic_auth_maybe(self, auth: Option<(String, String)>) -> Self {
        match auth {
            Some((user, password)) => self.basic_auth(user, Some(password)),
            None => self,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Accounts {
    pub accounts: Vec<Account>,
}

fn accounts_path() -> std::path::PathBuf {
    crate::library::config_dir().join("opds.json")
}

impl Accounts {
    pub fn load() -> Self {
        std::fs::read(accounts_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
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

    pub fn find_mut(&mut self, key: &str) -> Option<&mut Account> {
        self.accounts.iter_mut().find(|a| a.key() == key)
    }

    /// Adds or replaces (same catalog + user) an account.
    pub fn upsert(&mut self, account: Account) {
        match self.find_mut(&account.key()) {
            Some(a) => *a = account,
            None => self.accounts.push(account),
        }
        self.save();
    }

    pub fn remove(&mut self, key: &str) {
        self.accounts.retain(|a| a.key() != key);
        self.save();
        forget_links(key);
    }
}

// ── Where each book's file, cover and pages are ─────────────────────────────

/// Where to get a book, as the last sync found it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Links {
    /// Its file (an open-access acquisition link).
    pub file: Option<String>,
    /// The file's type as an extension: "cbz", "cbr", "epub", "pdf"…
    pub ext: String,
    pub cover: Option<String>,
    /// Page streaming: one image per page.
    pub pages: Option<PageLinks>,
    /// A Divina/WebPub manifest listing the pages.
    pub manifest: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PageLinks {
    /// The page's URL with [`PAGE_MARK`] for its number (counting from 0).
    pub template: String,
    pub count: usize,
}

impl Links {
    /// The reader can open the file itself.
    pub fn readable_file(&self) -> bool {
        self.file.is_some() && matches!(self.ext.as_str(), "cbz" | "zip" | "cbr" | "rar" | "epub")
    }

    pub fn streams_pages(&self) -> bool {
        !self.readable_file() && (self.pages.as_ref().is_some_and(|p| p.count > 0) || self.manifest.is_some())
    }

    /// Something the app can read at all.
    pub fn readable(&self) -> bool {
        self.readable_file() || self.streams_pages()
    }
}

type LinkMap = HashMap<String, HashMap<String, Links>>;

/// Per account: item id -> links. Kept in the cache folder so books open
/// before the first sync of a run, and loaded the first time it's needed.
fn link_store() -> &'static RwLock<LinkMap> {
    static S: OnceLock<RwLock<LinkMap>> = OnceLock::new();
    S.get_or_init(Default::default)
}

fn links_path(account_key: &str) -> std::path::PathBuf {
    crate::library::cache_dir().join("opds").join(format!("{account_key}.json"))
}

pub fn links(account_key: &str, item_id: &str) -> Option<Links> {
    if let Some(found) = link_store().read().ok()?.get(account_key) {
        return found.get(item_id).cloned();
    }
    let loaded: HashMap<String, Links> =
        std::fs::read(links_path(account_key)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let found = loaded.get(item_id).cloned();
    link_store().write().ok()?.insert(account_key.to_string(), loaded);
    found
}

/// The file extension of every known book of an account (for the reader's
/// "stream or fetch whole" choice and the CBR index).
pub fn extensions(account_key: &str) -> HashMap<String, String> {
    let _ = links(account_key, "");
    link_store()
        .read()
        .ok()
        .and_then(|s| s.get(account_key).map(|m| m.iter().map(|(id, l)| (id.clone(), l.ext.clone())).collect()))
        .unwrap_or_default()
}

fn remember_links(account_key: &str, map: HashMap<String, Links>) {
    let path = links_path(account_key);
    // Tests keep it in memory, out of the real cache folder.
    if cfg!(test) {
        if let Ok(mut s) = link_store().write() {
            s.insert(account_key.to_string(), map);
        }
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec(&map) {
        let _ = std::fs::write(&path, json);
    }
    if let Ok(mut s) = link_store().write() {
        s.insert(account_key.to_string(), map);
    }
}

fn forget_links(account_key: &str) {
    let _ = std::fs::remove_file(links_path(account_key));
    if let Ok(mut s) = link_store().write() {
        s.remove(account_key);
    }
}

/// The extension for a file's media type, empty when it's nothing known.
fn ext_for(mime: &str) -> &'static str {
    let mime = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match mime.as_str() {
        "application/vnd.comicbook+zip" | "application/x-cbz" => "cbz",
        "application/zip" | "application/x-zip-compressed" => "zip",
        "application/vnd.comicbook-rar" | "application/x-cbr" => "cbr",
        "application/x-rar-compressed" | "application/vnd.rar" | "application/x-rar" => "rar",
        "application/epub+zip" => "epub",
        "application/pdf" => "pdf",
        "application/x-cb7" | "application/x-7z-compressed" => "cb7",
        "application/x-mobipocket-ebook" => "mobi",
        _ => "",
    }
}

/// Which of a book's files to take, lower is better.
fn file_rank(ext: &str) -> u8 {
    match ext {
        "cbz" => 0,
        "zip" => 1,
        "cbr" => 2,
        "rar" => 3,
        "epub" => 4,
        "" => 9,
        _ => 6,
    }
}

// ── HTTP ────────────────────────────────────────────────────────────────────

/// What the app asks catalogs for: OPDS 2 if they have it, else 1.2.
const ACCEPT: &str = "application/opds+json, application/atom+xml;q=0.9, application/xml;q=0.8, */*;q=0.5";

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("http client")
    })
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

/// No overall time limit (a big book on a slow link takes a while), but
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

/// A book's file on the server, read with HTTP `Range` requests. Used from
/// the reader's blocking decode threads, hence the blocking client.
pub struct HttpFile {
    url: String,
    auth: Option<(String, String)>,
}

impl HttpFile {
    fn send(&self, range: Option<&str>) -> Result<reqwest::blocking::Response, String> {
        if self.url.is_empty() {
            return Err(fl!("opds-no-file"));
        }
        let mut req = blocking_client().get(&self.url).basic_auth_maybe(self.auth.clone());
        if let Some(range) = range {
            req = req.header("Range", range);
        }
        req.send().map_err(|e| fl!("jf-unreachable", error = e.to_string()))
    }

    fn get(&self, range: String) -> Result<reqwest::blocking::Response, String> {
        let resp = self.send(Some(&range))?;
        if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(format!("server didn't return a byte range (HTTP {})", resp.status()));
        }
        Ok(resp)
    }

    /// Downloads the whole file (to the cache), via `.part`.
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

impl crate::remote_zip::RangeSource for HttpFile {
    fn tail(&self, n: u64) -> Result<(Vec<u8>, u64), String> {
        let resp = self.get(format!("bytes=-{n}"))?;
        let total = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(crate::remote::parse_content_range_total)
            .ok_or("server didn't say how big the file is")?;
        let bytes = resp.bytes().map_err(|e| e.to_string())?.to_vec();
        Ok((bytes, total))
    }

    fn range(&self, start: u64, end: u64) -> Result<Vec<u8>, String> {
        let resp = self.get(format!("bytes={start}-{}", end.saturating_sub(1)))?;
        Ok(resp.bytes().map_err(|e| e.to_string())?.to_vec())
    }
}

/// One page served as an image (page streaming).
pub struct WebPage {
    url: String,
    auth: Option<(String, String)>,
}

impl std::fmt::Debug for WebPage {
    // Without the password.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebPage").field("url", &self.url).finish()
    }
}

impl WebPage {
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn fetch(&self) -> Result<Vec<u8>, String> {
        let resp = blocking_client()
            .get(&self.url)
            .header("Accept", "image/*")
            .basic_auth_maybe(self.auth.clone())
            .send()
            .map_err(|e| fl!("jf-unreachable", error = e.to_string()))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        resp.bytes().map(|b| b.to_vec()).map_err(|e| e.to_string())
    }
}

/// Fetches and reads one feed page.
pub async fn fetch_feed(account: &Account, url: &str) -> Result<Feed, String> {
    let resp = account.get(url).send().await.map_err(|e| fl!("jf-unreachable", error = e.to_string()))?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(format!("HTTP {}: {}", status.as_u16(), fl!("opds-sign-in-refused")));
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let base = resp.url().clone();
    let body = resp.text().await.map_err(|e| fl!("jf-unreachable", error = e.to_string()))?;
    parse_feed(&body, &base).ok_or_else(|| fl!("opds-not-a-catalog"))
}

/// Checks an address is a catalog and gets its name: what's typed, then
/// with `https://` or `http://` in front when it has no scheme.
pub async fn connect(input: &str, user: &str, password: &str) -> Result<Account, String> {
    let input = input.trim().trim_end_matches('/');
    let candidates: Vec<String> = if input.contains("://") {
        vec![input.to_string()]
    } else {
        vec![format!("https://{input}"), format!("http://{input}")]
    };
    let mut last = fl!("opds-not-a-catalog");
    for url in candidates {
        let account = Account { url: url.clone(), user: user.trim().to_string(), password: password.to_string(), ..Default::default() };
        match fetch_feed(&account, &url).await {
            Ok(feed) => return Ok(Account { title: feed.title, ..account }),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// The address to search a catalog for `query`, when it can be searched.
/// An OpenSearch description is fetched to find its template.
pub async fn search_url(account: &Account, search: &Search, query: &str) -> Result<String, String> {
    let template = match search {
        Search::Template(t) => t.clone(),
        Search::Description(url) => {
            let resp = account.get(url).send().await.map_err(|e| fl!("jf-unreachable", error = e.to_string()))?;
            let base = resp.url().clone();
            let xml = resp.text().await.map_err(|e| e.to_string())?;
            let t = opensearch_template(&xml).ok_or_else(|| fl!("opds-no-search"))?;
            base.join(&t).map(|u| u.to_string()).unwrap_or(t).replace("%7B", "{").replace("%7D", "}")
        }
    };
    Ok(fill_search(&template, query))
}

fn encode_query(q: &str) -> String {
    url::form_urlencoded::byte_serialize(q.as_bytes()).collect()
}

/// Fills in an OpenSearch (`{searchTerms}`) or URI (`{?query}`) template.
fn fill_search(template: &str, query: &str) -> String {
    let q = encode_query(query);
    let mut out = template.replace("{searchTerms}", &q).replace("{?query}", &format!("?query={q}")).replace("{query}", &q);
    // Optional OpenSearch parameters left over ({startPage?} etc.) go.
    while let Some(start) = out.find('{') {
        let Some(len) = out[start..].find('}') else { break };
        out.replace_range(start..start + len + 1, "");
    }
    out
}

/// The Atom search template in an OpenSearch description.
fn opensearch_template(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut fallback = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) if local_name(&e.name()) == "Url" => {
                let kind = attr(&e, "type").unwrap_or_default();
                if let Some(t) = attr(&e, "template") {
                    if kind.contains("atom") || kind.contains("opds") {
                        return Some(t);
                    }
                    fallback.get_or_insert(t);
                }
            }
            Ok(Event::Eof) | Err(_) => return fallback,
            _ => {}
        }
        buf.clear();
    }
}

// ── Feeds ───────────────────────────────────────────────────────────────────

/// One page of a feed: its folders, books, and where the rest is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Feed {
    pub title: String,
    pub icon: Option<String>,
    pub folders: Vec<Folder>,
    pub books: Vec<Book>,
    /// The next page of this feed.
    pub next: Option<String>,
    pub search: Option<Search>,
}

/// A link to another feed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Folder {
    pub title: String,
    pub url: String,
    /// How many items the server says are in it, when it says.
    pub count: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Search {
    /// A template with `{searchTerms}` or `{?query}`.
    Template(String),
    /// An OpenSearch description to fetch first.
    Description(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Book {
    /// The catalog's own id for it.
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    /// "2021-03-14", or empty.
    pub date: String,
    pub summary: String,
    /// The series it says it's in, and its number there.
    pub series: Option<(String, Option<f32>)>,
    pub links: Links,
    /// Where it can be bought or borrowed instead, when it can't be read.
    pub external: Option<String>,
    /// The server's page-streaming position: last page read, and when.
    pub last_page: Option<usize>,
    pub last_read: i64,
}

/// Reads a feed: OPDS 2 JSON or OPDS 1.2 Atom. `None` when it's neither.
pub fn parse_feed(body: &str, base: &url::Url) -> Option<Feed> {
    let trimmed = body.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with('{') {
        parse_json(&serde_json::from_str(trimmed).ok()?, base)
    } else if trimmed.starts_with('<') {
        parse_atom(trimmed, base)
    } else {
        None
    }
}

fn join(base: &url::Url, href: &str) -> Option<String> {
    base.join(href).ok().map(|u| u.to_string())
}

/// A page-streaming link with its page number marked: joined to the base
/// with the braces out of the way (they'd be percent-encoded).
fn page_template(base: &url::Url, href: &str) -> Option<String> {
    let href = href.replace("{pageNumber}", PAGE_MARK).replace("{maxWidth}", "4096");
    join(base, &href)
}

/// "2021-03-14T00:00:00Z" → "2021-03-14".
fn day(s: &str) -> String {
    let d = s.trim().split('T').next().unwrap_or("");
    if d.len() >= 4 && d[..4].bytes().all(|b| b.is_ascii_digit()) { d.to_string() } else { String::new() }
}

/// A link in an Atom feed or entry.
#[derive(Default)]
struct AtomLink {
    rel: String,
    href: String,
    kind: String,
    title: String,
    count: Option<usize>,
    last_read: Option<usize>,
    last_read_date: Option<String>,
}

#[derive(Default)]
struct AtomEntry {
    id: String,
    title: String,
    authors: Vec<String>,
    date: String,
    summary: String,
    links: Vec<AtomLink>,
}

fn is_feed_type(kind: &str) -> bool {
    kind.contains("atom+xml") || kind.contains("opds")
}

fn parse_atom(xml: &str, base: &url::Url) -> Option<Feed> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut feed = Feed::default();
    let mut seen_feed = false;
    let mut entry: Option<AtomEntry> = None;
    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();

    let read_link = |e: &quick_xml::events::BytesStart| AtomLink {
        rel: attr(e, "rel").unwrap_or_default(),
        href: attr(e, "href").unwrap_or_default(),
        kind: attr(e, "type").unwrap_or_default(),
        title: attr(e, "title").unwrap_or_default(),
        count: attr(e, "count").and_then(|c| c.trim().parse().ok()),
        last_read: attr(e, "lastRead").and_then(|c| c.trim().parse().ok()),
        last_read_date: attr(e, "lastReadDate"),
    };

    loop {
        let ev = reader.read_event_into(&mut buf);
        match ev {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name = local_name(&e.name());
                let empty = matches!(ev, Ok(Event::Empty(_)));
                match name.as_str() {
                    "feed" => seen_feed = true,
                    "entry" => entry = Some(AtomEntry::default()),
                    "link" => {
                        let link = read_link(e);
                        match entry.as_mut() {
                            Some(en) => en.links.push(link),
                            None => feed_link(&mut feed, link, base),
                        }
                    }
                    _ => {}
                }
                if !empty {
                    path.push(name);
                    text.clear();
                }
            }
            Ok(ref ev @ (Event::Text(_) | Event::GeneralRef(_))) => text.push_str(&event_text(ev)),
            Ok(Event::CData(t)) => text.push_str(&t),
            Ok(Event::End(_)) => {
                let name = path.pop().unwrap_or_default();
                let parent = path.last().map(String::as_str).unwrap_or("");
                let value = text.trim().to_string();
                match (entry.as_mut(), name.as_str(), parent) {
                    (Some(_), "entry", _) => {
                        if let Some(done) = entry.take() {
                            add_entry(&mut feed, done, base);
                        }
                    }
                    (Some(en), "id", "entry") => en.id = value,
                    (Some(en), "title", "entry") => en.title = value,
                    (Some(en), "name", "author") => en.authors.push(value),
                    (Some(en), "issued" | "published", "entry") => en.date = day(&value),
                    (Some(en), "updated", "entry") if en.date.is_empty() => en.date = day(&value),
                    (Some(en), "summary" | "content", "entry") if en.summary.is_empty() => en.summary = value,
                    (None, "title", "feed") => feed.title = value,
                    (None, "icon", "feed") => feed.icon = join(base, &value),
                    _ => {}
                }
                text.clear();
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
    seen_feed.then_some(feed)
}

/// A link at the top of a feed: its next page or its search.
fn feed_link(feed: &mut Feed, link: AtomLink, base: &url::Url) {
    match link.rel.as_str() {
        "next" => feed.next = join(base, &link.href),
        "search" if link.kind.contains("opensearchdescription") => {
            feed.search = join(base, &link.href).map(Search::Description);
        }
        "search" if link.href.contains("{searchTerms}") => {
            let href = link.href.replace("{searchTerms}", "NEBULASEARCHTERMS");
            feed.search = join(base, &href).map(|u| Search::Template(u.replace("NEBULASEARCHTERMS", "{searchTerms}")));
        }
        _ => {}
    }
}

const ACQUISITION: &str = "http://opds-spec.org/acquisition";
const PSE_STREAM: &str = "http://vaemendis.net/opds-pse/stream";

/// An Atom entry becomes a book (it has something to read, buy or stream)
/// or a folder (it links to another feed).
fn add_entry(feed: &mut Feed, en: AtomEntry, base: &url::Url) {
    let mut book = Book {
        id: if en.id.is_empty() { en.title.clone() } else { en.id },
        title: en.title,
        authors: en.authors,
        date: en.date,
        summary: en.summary,
        ..Default::default()
    };
    let mut is_book = false;
    let mut rank = u8::MAX;
    let mut folder = None;
    for l in &en.links {
        let rel = l.rel.as_str();
        if rel == PSE_STREAM {
            is_book = true;
            if let (Some(template), Some(count)) = (page_template(base, &l.href), l.count) {
                book.links.pages = Some(PageLinks { template, count });
            }
            book.last_page = l.last_read;
            book.last_read = l.last_read_date.as_deref().and_then(crate::jellyfin::parse_date).unwrap_or(0);
        } else if rel == ACQUISITION || rel == "http://opds-spec.org/acquisition/open-access" {
            is_book = true;
            let ext = ext_for(&l.kind);
            if file_rank(ext) < rank {
                rank = file_rank(ext);
                book.links.file = join(base, &l.href);
                book.links.ext = ext.to_string();
            }
        } else if rel.starts_with(ACQUISITION) {
            // buy, borrow, subscribe, sample
            is_book = true;
            book.external = book.external.take().or_else(|| join(base, &l.href));
        } else if rel == "http://opds-spec.org/image" {
            book.links.cover = join(base, &l.href);
        } else if rel == "http://opds-spec.org/image/thumbnail" || rel == "http://opds-spec.org/thumbnail" {
            book.links.cover = book.links.cover.take().or_else(|| join(base, &l.href));
        } else if is_feed_type(&l.kind) && folder.is_none() {
            folder = join(base, &l.href).map(|url| Folder {
                title: if book.title.is_empty() { l.title.clone() } else { book.title.clone() },
                url,
                count: l.count,
            });
        }
    }
    if is_book {
        // An extension from the file name when the type was generic.
        if book.links.ext.is_empty() {
            if let Some(f) = &book.links.file {
                book.links.ext = ext_from_url(f);
            }
        }
        feed.books.push(book);
    } else if let Some(f) = folder {
        feed.folders.push(f);
    }
}

fn ext_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let file = path.rsplit('/').next().unwrap_or("");
    match file.rsplit_once('.') {
        Some((_, e)) => {
            let e = e.to_ascii_lowercase();
            if matches!(e.as_str(), "cbz" | "zip" | "cbr" | "rar" | "epub" | "pdf" | "cb7") { e } else { String::new() }
        }
        None => String::new(),
    }
}

// OPDS 2: JSON. Fields can be a string, an object or a list of either.

fn as_list(v: &serde_json::Value) -> Vec<&serde_json::Value> {
    match v {
        serde_json::Value::Array(a) => a.iter().collect(),
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    }
}

fn rels(link: &serde_json::Value) -> Vec<&str> {
    as_list(&link["rel"]).into_iter().filter_map(|r| r.as_str()).collect()
}

fn name_of(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(_) => v["name"].as_str().map(str::to_string),
        _ => None,
    }
}

fn parse_json(json: &serde_json::Value, base: &url::Url) -> Option<Feed> {
    let meta = &json["metadata"];
    if !json.is_object() || !(meta.is_object() || json.get("links").is_some()) {
        return None;
    }
    let mut feed = Feed { title: meta["title"].as_str().unwrap_or_default().to_string(), ..Default::default() };
    for l in as_list(&json["links"]) {
        let href = l["href"].as_str().unwrap_or_default();
        for rel in rels(l) {
            match rel {
                "next" => feed.next = join(base, href),
                "search" if l["templated"].as_bool() == Some(true) => {
                    let marked = href.replace("{?query}", "NEBULAQUERY").replace("{searchTerms}", "NEBULAQUERY");
                    feed.search = join(base, &marked).map(|u| Search::Template(u.replace("NEBULAQUERY", "{?query}")));
                    if href.contains("{searchTerms}") {
                        feed.search = join(base, &marked).map(|u| Search::Template(u.replace("NEBULAQUERY", "{searchTerms}")));
                    }
                }
                "search" => feed.search = join(base, href).map(Search::Description),
                "icon" => feed.icon = join(base, href),
                _ => {}
            }
        }
    }
    let folders = |list: &serde_json::Value| -> Vec<Folder> {
        as_list(list)
            .into_iter()
            .filter_map(|l| {
                Some(Folder {
                    title: l["title"].as_str().unwrap_or_default().to_string(),
                    url: join(base, l["href"].as_str()?)?,
                    count: l["properties"]["numberOfItems"].as_u64().map(|n| n as usize),
                })
            })
            .collect()
    };
    feed.folders = folders(&json["navigation"]);
    feed.books = as_list(&json["publications"]).into_iter().filter_map(|p| json_book(p, base)).collect();
    // Groups ("New this week", "Popular"): their books as shown, and each
    // group's full list as a folder.
    for g in as_list(&json["groups"]) {
        feed.folders.extend(folders(&g["navigation"]));
        if let Some(own) = as_list(&g["links"]).into_iter().find(|l| rels(l).contains(&"self")) {
            if let Some(url) = own["href"].as_str().and_then(|h| join(base, h)) {
                let title = g["metadata"]["title"].as_str().unwrap_or_default().to_string();
                feed.folders.push(Folder { title, url, count: g["metadata"]["numberOfItems"].as_u64().map(|n| n as usize) });
            }
        }
        feed.books.extend(as_list(&g["publications"]).into_iter().filter_map(|p| json_book(p, base)));
    }
    Some(feed)
}

fn json_book(p: &serde_json::Value, base: &url::Url) -> Option<Book> {
    let m = &p["metadata"];
    let title = m["title"].as_str()?.to_string();
    let mut book = Book {
        id: m["identifier"].as_str().map(str::to_string).unwrap_or_else(|| title.clone()),
        title,
        authors: as_list(&m["author"]).into_iter().filter_map(name_of).collect(),
        date: m["published"].as_str().map(day).unwrap_or_default(),
        summary: m["description"].as_str().unwrap_or_default().to_string(),
        series: as_list(&m["belongsTo"]["series"])
            .first()
            .and_then(|s| Some((name_of(s)?, s["position"].as_f64().map(|n| n as f32)))),
        ..Default::default()
    };
    let mut rank = u8::MAX;
    for l in as_list(&p["links"]) {
        let href = l["href"].as_str().unwrap_or_default();
        let kind = l["type"].as_str().unwrap_or_default();
        for rel in rels(l) {
            if rel == ACQUISITION || rel == "http://opds-spec.org/acquisition/open-access" {
                let ext = ext_for(kind);
                if file_rank(ext) < rank {
                    rank = file_rank(ext);
                    book.links.file = join(base, href);
                    book.links.ext = ext.to_string();
                }
            } else if rel.starts_with(ACQUISITION) {
                book.external = book.external.take().or_else(|| join(base, href));
            } else if rel == PSE_STREAM {
                if let (Some(template), Some(count)) = (page_template(base, href), l["properties"]["count"].as_u64()) {
                    book.links.pages = Some(PageLinks { template, count: count as usize });
                }
            }
        }
        if kind.starts_with("application/divina+json") || kind.starts_with("application/webpub+json") {
            book.links.manifest = join(base, href);
        }
    }
    if book.links.ext.is_empty() {
        if let Some(f) = &book.links.file {
            book.links.ext = ext_from_url(f);
        }
    }
    book.links.cover = as_list(&p["images"]).first().and_then(|i| i["href"].as_str()).and_then(|h| join(base, h));
    Some(book)
}

// ── Library sync ────────────────────────────────────────────────────────────

/// Walks the account's picked folders into series.
#[derive(Default)]
struct Walk {
    visited: HashSet<String>,
    feeds: usize,
    /// Books already placed, by their catalog id: the first place wins.
    seen: HashSet<String>,
    out: Vec<RemoteSeries>,
    /// Series key -> its index in `out`.
    index: HashMap<String, usize>,
    /// Item key -> number in its series, for sorting.
    positions: HashMap<String, f32>,
    links: HashMap<String, Links>,
}

/// Feeds fetched at once while walking.
const PARALLEL: usize = 6;

/// A feed with all its pages.
struct WholeFeed {
    title: String,
    folders: Vec<Folder>,
    books: Vec<Book>,
    pages: usize,
}

/// Everything under the account's picked folders, organised as series.
/// Fails rather than returning part of a folder, since what's missing
/// would be taken out of the library.
pub async fn fetch_library(account: &Account) -> Result<Vec<RemoteSeries>, String> {
    let mut walk = Walk::default();
    for root in &account.roots {
        let root_key = account.item_key(&format!("r{}", hash(&root.url)));
        // Folder by folder, a level at a time, so the order books are
        // placed in (the first place a book is seen wins) is always the same.
        let mut level = vec![(root.url.clone(), String::new())];
        walk.visited.insert(root.url.clone());
        for depth in 0..=MAX_DEPTH {
            if level.is_empty() {
                break;
            }
            let fetched = fetch_level(account, &level).await?;
            let mut next = Vec::new();
            for ((url, title), feed) in level.iter().zip(fetched) {
                walk.feeds += feed.pages;
                let feed_title = if title.is_empty() { feed.title.clone() } else { title.clone() };
                // A picked folder of books that are all one series (or have
                // no series in their names: "Vol. 1", "Vol. 2") is that series.
                let one_series = depth == 0 && feed.folders.is_empty() && feed.books.len() > 1 && {
                    let names: HashSet<String> = feed
                        .books
                        .iter()
                        .map(|b| crate::naming::normalize_title(&crate::naming::series_name_from_issue(&b.title).display))
                        .filter(|n| !n.is_empty())
                        .collect();
                    names.len() <= 1
                };
                for book in feed.books {
                    walk.place(account, book, url, &feed_title, &root_key, root, depth == 0 && !one_series);
                }
                if depth < MAX_DEPTH {
                    for f in feed.folders {
                        if walk.visited.insert(f.url.clone()) {
                            next.push((f.url, f.title));
                        }
                    }
                }
            }
            if walk.feeds + next.len() > MAX_FEEDS {
                return Err(fl!("opds-too-big"));
            }
            level = next;
        }
    }
    let Walk { mut out, positions, links, .. } = walk;
    for s in &mut out {
        // Without an image, the cover comes from the first book's first page.
        if s.cover_item.is_none() {
            s.cover_item = s.issues.first().map(|i| i.item_id.clone());
        }
        // Books that said which series they're in go in its order.
        if s.issues.iter().all(|i| positions.contains_key(&i.key)) {
            s.issues.sort_by(|a, b| positions[&a.key].total_cmp(&positions[&b.key]));
        }
    }
    remember_links(&account.key(), links);
    Ok(out)
}

/// Fetches a level's feeds, a few at a time, in their order.
async fn fetch_level(account: &Account, level: &[(String, String)]) -> Result<Vec<WholeFeed>, String> {
    let permits = Arc::new(tokio::sync::Semaphore::new(PARALLEL));
    let mut jobs = tokio::task::JoinSet::new();
    for (i, (url, _)) in level.iter().enumerate() {
        let (account, url, permits) = (account.clone(), url.clone(), permits.clone());
        jobs.spawn(async move {
            let _permit = permits.acquire_owned().await;
            (i, fetch_whole(&account, &url).await)
        });
    }
    let mut out: Vec<Option<WholeFeed>> = (0..level.len()).map(|_| None).collect();
    while let Some(done) = jobs.join_next().await {
        let (i, result) = done.map_err(|e| e.to_string())?;
        out[i] = Some(result?);
    }
    Ok(out.into_iter().flatten().collect())
}

/// A feed and the pages after it.
async fn fetch_whole(account: &Account, url: &str) -> Result<WholeFeed, String> {
    let mut whole = WholeFeed { title: String::new(), folders: Vec::new(), books: Vec::new(), pages: 0 };
    let mut seen = HashSet::from([url.to_string()]);
    let mut page_url = Some(url.to_string());
    while let Some(u) = page_url.take() {
        let page = fetch_feed(account, &u).await?;
        whole.pages += 1;
        if whole.title.is_empty() {
            whole.title = page.title;
        }
        whole.books.extend(page.books);
        whole.folders.extend(page.folders);
        page_url = page.next.filter(|n| whole.pages < MAX_PAGES && seen.insert(n.clone()));
    }
    Ok(whole)
}

impl Walk {
    /// Puts a book in its series.
    #[allow(clippy::too_many_arguments)]
    fn place(&mut self, account: &Account, book: Book, feed_url: &str, feed_title: &str, root_key: &str, root: &Root, in_root: bool) {
        if !book.links.readable() || !self.seen.insert(book.id.clone()) {
            return;
        }
        let item_id = book_id(&book.id);
        let (series_id, series_title, one_shot) = series_for(&book, feed_url, feed_title, &root.url, in_root);
        let key = account.item_key(&series_id);
        let issue = RemoteIssue {
            key: account.item_key(&item_id),
            item_id: item_id.clone(),
            name: book.title.clone(),
            extension: book.links.ext.clone(),
            played: matches!((book.last_page, book.links.pages.as_ref()), (Some(p), Some(l)) if l.count > 1 && p + 1 >= l.count),
            page: book.last_page.unwrap_or(0),
            last_read: book.last_read,
            release_date: book.date.clone(),
            creators: crate::library::format_credits(book.authors.iter().map(|a| ("Author", a.as_str()))),
        };
        if let Some(pos) = book.series.as_ref().and_then(|(_, p)| *p) {
            self.positions.insert(issue.key.clone(), pos);
        }
        let has_cover = book.links.cover.is_some();
        self.links.insert(item_id.clone(), book.links);
        let i = *self.index.entry(key.clone()).or_insert_with(|| {
            self.out.push(RemoteSeries {
                key: key.clone(),
                title: series_title,
                is_folder: !one_shot,
                library_root: root_key.to_string(),
                library_name: root.title.clone(),
                cover_item: None,
                issues: Vec::new(),
            });
            self.out.len() - 1
        });
        let s = &mut self.out[i];
        if s.cover_item.is_none() && has_cover {
            s.cover_item = Some(item_id);
        }
        s.issues.push(issue);
    }
}

/// Which series a book goes in: (id, title, whether it's a one-shot).
/// - the series it names itself;
/// - else the feed it's listed in, when that's a folder below a picked one;
/// - else (listed straight in a picked folder) the series named in its
///   title ("Saga #12" → "Saga"), or on its own when there's none.
fn series_for(book: &Book, feed_url: &str, feed_title: &str, root_url: &str, in_root: bool) -> (String, String, bool) {
    if let Some((name, _)) = book.series.as_ref().filter(|(n, _)| !n.trim().is_empty()) {
        return (format!("s{}", hash(&format!("{root_url}\n{}", crate::naming::normalize_title(name)))), name.clone(), false);
    }
    if !in_root {
        return (format!("f{}", hash(feed_url)), feed_title.to_string(), false);
    }
    let parsed = crate::naming::series_name_from_issue(&book.title).display;
    let norm = crate::naming::normalize_title(&parsed);
    if !norm.is_empty() && norm != crate::naming::normalize_title(&book.title) {
        return (format!("s{}", hash(&format!("{root_url}\n{norm}"))), parsed, false);
    }
    (format!("b{}", hash(&book.id)), book.title.clone(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> url::Url {
        url::Url::parse("https://komga.example/opds/v1.2/series/42").unwrap()
    }

    const KOMGA_SERIES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:pse="http://vaemendis.net/opds-pse/ns" xmlns:opds="http://opds-spec.org/2010/catalog">
  <id>series/42</id>
  <title>Saga</title>
  <link rel="next" href="/opds/v1.2/series/42?page=1" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
  <link rel="search" href="/opds/v1.2/search" type="application/opensearchdescription+xml"/>
  <entry>
    <id>book-1</id>
    <title>Saga #001</title>
    <updated>2024-01-02T10:00:00Z</updated>
    <author><name>Brian K. Vaughan</name></author>
    <content type="text">The first one &amp; more.</content>
    <link rel="http://opds-spec.org/image" href="/api/v1/books/1/thumbnail" type="image/jpeg"/>
    <link rel="http://opds-spec.org/acquisition" href="/api/v1/books/1/file" type="application/zip"/>
    <link rel="http://vaemendis.net/opds-pse/stream" href="/api/v1/books/1/pages/{pageNumber}?zero_based=true" type="image/jpeg" pse:count="24" pse:lastRead="23" pse:lastReadDate="2024-02-01T08:00:00Z"/>
  </entry>
  <entry>
    <id>book-2</id>
    <title>Saga #002</title>
    <link rel="http://opds-spec.org/acquisition" href="/api/v1/books/2/file" type="application/pdf"/>
    <link rel="http://vaemendis.net/opds-pse/stream" href="/api/v1/books/2/pages/{pageNumber}" pse:count="30"/>
  </entry>
  <entry>
    <id>book-3</id>
    <title>For sale</title>
    <link rel="http://opds-spec.org/acquisition/buy" href="/buy/3" type="text/html"/>
  </entry>
</feed>"#;

    #[test]
    fn reads_an_atom_acquisition_feed() {
        let feed = parse_feed(KOMGA_SERIES, &base()).unwrap();
        assert_eq!(feed.title, "Saga");
        assert_eq!(feed.next.as_deref(), Some("https://komga.example/opds/v1.2/series/42?page=1"));
        assert_eq!(feed.search, Some(Search::Description("https://komga.example/opds/v1.2/search".into())));
        assert!(feed.folders.is_empty());
        assert_eq!(feed.books.len(), 3);

        let b = &feed.books[0];
        assert_eq!((b.id.as_str(), b.title.as_str(), b.date.as_str()), ("book-1", "Saga #001", "2024-01-02"));
        assert_eq!(b.authors, ["Brian K. Vaughan"]);
        assert_eq!(b.summary, "The first one & more.");
        assert_eq!(b.links.file.as_deref(), Some("https://komga.example/api/v1/books/1/file"));
        assert_eq!(b.links.ext, "zip");
        assert_eq!(b.links.cover.as_deref(), Some("https://komga.example/api/v1/books/1/thumbnail"));
        let pages = b.links.pages.as_ref().unwrap();
        assert_eq!(pages.count, 24);
        assert_eq!(pages.template.replace(PAGE_MARK, "3"), "https://komga.example/api/v1/books/1/pages/3?zero_based=true");
        assert_eq!(b.last_page, Some(23));
        assert!(b.last_read > 0);
        // A zip is read from its file; a PDF page by page.
        assert!(b.links.readable_file() && !b.links.streams_pages());
        assert!(feed.books[1].links.streams_pages());
        // Only for sale: listed, not readable.
        assert!(!feed.books[2].links.readable());
        assert_eq!(feed.books[2].external.as_deref(), Some("https://komga.example/buy/3"));
    }

    #[test]
    fn reads_atom_navigation_entries_as_folders() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Catalog</title>
  <icon>/favicon.png</icon>
  <entry><id>a</id><title>Libraries</title>
    <link rel="subsection" href="libraries" type="application/atom+xml;profile=opds-catalog;kind=navigation"/></entry>
  <entry><id>b</id><title>Newest</title>
    <link rel="http://opds-spec.org/sort/new" href="/new" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/></entry>
</feed>"#;
        let feed = parse_feed(xml, &url::Url::parse("https://c.example/opds/").unwrap()).unwrap();
        assert_eq!(feed.title, "Catalog");
        assert_eq!(feed.icon.as_deref(), Some("https://c.example/favicon.png"));
        let f: Vec<(&str, &str)> = feed.folders.iter().map(|f| (f.title.as_str(), f.url.as_str())).collect();
        assert_eq!(f, [("Libraries", "https://c.example/opds/libraries"), ("Newest", "https://c.example/new")]);
        assert!(feed.books.is_empty());
    }

    #[test]
    fn reads_an_opds2_feed() {
        let json = r#"{
          "metadata": {"title": "Comics"},
          "links": [
            {"rel": "self", "href": "/opds/v2/catalog", "type": "application/opds+json"},
            {"rel": "search", "href": "/opds/v2/search{?query}", "type": "application/opds+json", "templated": true},
            {"rel": "next", "href": "/opds/v2/catalog?page=2"}
          ],
          "navigation": [{"href": "/opds/v2/libraries", "title": "Libraries", "type": "application/opds+json"}],
          "groups": [{
            "metadata": {"title": "Free this Friday"},
            "links": [{"rel": "self", "href": "/opds/v2/free", "type": "application/opds+json"}],
            "publications": [{
              "metadata": {"identifier": "urn:x:1", "title": "Example #1", "author": [{"name": "Someone"}],
                           "published": "2020-05-06", "belongsTo": {"series": {"name": "Example", "position": 1}}},
              "links": [{"rel": "http://opds-spec.org/acquisition/open-access", "href": "/files/1.cbz", "type": "application/vnd.comicbook+zip"}],
              "images": [{"href": "/covers/1.jpg", "type": "image/jpeg"}]
            }]
          }]
        }"#;
        let feed = parse_feed(json, &url::Url::parse("https://x.example/opds/v2/catalog").unwrap()).unwrap();
        assert_eq!(feed.title, "Comics");
        assert_eq!(feed.next.as_deref(), Some("https://x.example/opds/v2/catalog?page=2"));
        let Some(Search::Template(t)) = &feed.search else { panic!("no search") };
        assert_eq!(fill_search(t, "a b"), "https://x.example/opds/v2/search?query=a+b");
        let titles: Vec<&str> = feed.folders.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(titles, ["Libraries", "Free this Friday"]);
        let b = &feed.books[0];
        assert_eq!((b.id.as_str(), b.authors.as_slice(), b.date.as_str()), ("urn:x:1", &["Someone".to_string()][..], "2020-05-06"));
        assert_eq!(b.series, Some(("Example".into(), Some(1.0))));
        assert_eq!((b.links.file.as_deref(), b.links.ext.as_str()), (Some("https://x.example/files/1.cbz"), "cbz"));
        assert_eq!(b.links.cover.as_deref(), Some("https://x.example/covers/1.jpg"));
    }

    #[test]
    fn fills_search_templates() {
        assert_eq!(fill_search("https://c/s?q={searchTerms}&p={startPage?}", "one piece"), "https://c/s?q=one+piece&p=");
        assert_eq!(
            opensearch_template(r#"<OpenSearchDescription><Url type="text/html" template="/h?q={searchTerms}"/><Url type="application/atom+xml;profile=opds-catalog" template="/s?q={searchTerms}"/></OpenSearchDescription>"#),
            Some("/s?q={searchTerms}".into())
        );
    }

    #[test]
    fn rejects_what_isnt_a_feed() {
        assert!(parse_feed("<html><body>hi</body></html>", &base()).is_none());
        assert!(parse_feed("not xml", &base()).is_none());
    }

    #[test]
    fn places_books_in_series() {
        let book = |id: &str, title: &str| Book { id: id.into(), title: title.into(), ..Default::default() };
        // Below a picked folder: the feed is the series.
        let (id, title, one_shot) = series_for(&book("1", "Vol. 1"), "https://c/s/9", "Berserk", "https://c/r", false);
        assert_eq!((id.starts_with('f'), title.as_str(), one_shot), (true, "Berserk", false));
        // Straight in a picked folder: by the series in the title.
        let a = series_for(&book("1", "Saga #001"), "https://c/r", "All", "https://c/r", true);
        let b = series_for(&book("2", "Saga #002"), "https://c/r", "All", "https://c/r", true);
        assert_eq!(a.0, b.0);
        assert!(!a.2);
        // No series in the title: a one-shot.
        assert!(series_for(&book("3", "Watchmen"), "https://c/r", "All", "https://c/r", true).2);
        // The series the book names wins.
        let mut named = book("4", "Whatever #1");
        named.series = Some(("Example".into(), Some(1.0)));
        let (_, title, _) = series_for(&named, "https://c/s/9", "Feed", "https://c/r", false);
        assert_eq!(title, "Example");
    }

    #[test]
    fn signs_in_only_to_its_own_host() {
        let a = Account { url: "https://komga.example/opds/v1.2/catalog".into(), user: "me".into(), password: "pw".into(), ..Default::default() };
        assert!(a.auth_for("https://komga.example/api/v1/books/1/file").is_some());
        assert!(a.auth_for("https://cdn.example/covers/1.jpg").is_none());
        assert!(a.auth_for("http://komga.example/api").is_none());
        let anon = Account { user: String::new(), ..a };
        assert!(anon.auth_for("https://komga.example/x").is_none());
    }

    /// Serves `pages` (path with query -> Atom body) until the test ends.
    fn serve(pages: Vec<(&'static str, String)>) -> String {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let pages: HashMap<&str, String> = pages.into_iter().collect();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let _ = reader.read_line(&mut line);
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
                        break;
                    }
                }
                let (status, body) = match pages.get(path.as_str()) {
                    Some(b) => (200, b.clone()),
                    None => (404, String::new()),
                };
                let head = format!("HTTP/1.1 {status} X\r\nContent-Type: application/atom+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body.as_bytes());
            }
        });
        url
    }

    fn atom(title: &str, extra: &str) -> String {
        format!(r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:pse="http://vaemendis.net/opds-pse/ns"><title>{title}</title>{extra}</feed>"#)
    }

    fn nav(title: &str, href: &str) -> String {
        format!(r#"<entry><id>{href}</id><title>{title}</title><link rel="subsection" href="{href}" type="application/atom+xml;profile=opds-catalog;kind=navigation"/></entry>"#)
    }

    fn cbz(id: &str, title: &str) -> String {
        format!(r#"<entry><id>{id}</id><title>{title}</title><link rel="http://opds-spec.org/acquisition" href="/f/{id}.cbz" type="application/vnd.comicbook+zip"/></entry>"#)
    }

    #[test]
    fn walks_picked_folders_into_series() {
        let pdf = r#"<entry><id>b9</id><title>Berserk Vol. 1</title><link rel="http://opds-spec.org/acquisition" href="/f/b9.pdf" type="application/pdf"/><link rel="http://vaemendis.net/opds-pse/stream" href="/p/b9/{pageNumber}" pse:count="3"/></entry>"#;
        let url = serve(vec![
            ("/opds", atom("Home", &nav("Libraries", "/lib"))),
            // A link back up: walked once only.
            ("/lib", atom("Libraries", &(nav("Saga", "/s/saga") + &nav("Berserk", "/s/berserk") + &nav("Home", "/opds")))),
            (
                "/s/saga",
                atom("Saga", &(r#"<link rel="next" href="/s/saga?page=2"/>"#.to_string() + &cbz("a1", "Saga #1") + &cbz("a2", "Saga #2"))),
            ),
            ("/s/saga?page=2", atom("Saga", &cbz("a3", "Saga #3"))),
            // Saga #1 again, under another folder: kept where it was first.
            ("/s/berserk", atom("Berserk", &(pdf.to_string() + &cbz("a1", "Saga #1")))),
        ]);
        let account = Account {
            url: format!("{url}/opds"),
            roots: vec![Root { url: format!("{url}/lib"), title: "Libraries".into() }],
            ..Default::default()
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let series = rt.block_on(fetch_library(&account)).unwrap();

        let got: Vec<(&str, Vec<&str>)> =
            series.iter().map(|s| (s.title.as_str(), s.issues.iter().map(|i| i.name.as_str()).collect())).collect();
        assert_eq!(got, [("Saga", vec!["Saga #1", "Saga #2", "Saga #3"]), ("Berserk", vec!["Berserk Vol. 1"])]);
        assert!(series.iter().all(|s| s.library_name == "Libraries" && s.library_root == series[0].library_root));
        // Covers come from the first page when the catalog has no images.
        assert_eq!(series[1].cover_item.as_deref(), Some(book_id("b9").as_str()));

        // Where each book is, for reading.
        let saga1 = links(&account.key(), &book_id("a1")).unwrap();
        assert_eq!(saga1.file, Some(format!("{url}/f/a1.cbz")));
        let berserk = links(&account.key(), &book_id("b9")).unwrap();
        assert!(berserk.streams_pages());
        let pages = account.page_stream(&book_id("b9")).unwrap();
        assert_eq!(pages.len(), 3);
        let PageSource::Web(p) = &pages[2] else { panic!("not a web page") };
        assert_eq!(p.url(), format!("{url}/p/b9/2"));
    }

    #[test]
    fn a_picked_folder_of_volumes_is_one_series() {
        let url = serve(vec![("/s/1", atom("Berserk", &(cbz("v1", "Vol. 1") + &cbz("v2", "Vol. 2"))))]);
        let account = Account {
            url: format!("{url}/s/1"),
            roots: vec![Root { url: format!("{url}/s/1"), title: "Berserk".into() }],
            ..Default::default()
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let series = rt.block_on(fetch_library(&account)).unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!((series[0].title.as_str(), series[0].issues.len()), ("Berserk", 2));
    }

    #[test]
    fn keys_are_stable() {
        // Changing these would orphan every OPDS book's progress and download.
        assert_eq!(hash(""), "cbf29ce484222325");
        assert_eq!(book_id("urn:x:1"), format!("o{}", hash("urn:x:1")));
        let key = Account { url: "https://c".into(), ..Default::default() }.item_key("o1");
        assert_eq!(parse_key(&key).map(|(_, id)| id), Some("o1"));
    }
}
