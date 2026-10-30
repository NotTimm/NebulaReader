//! What the online sources (Jellyfin servers and OPDS catalogs) have in
//! common, so the library, the reader, downloads and offline reading work
//! the same for both.
//!
//! Each source's series and chapters are ordinary library entries keyed
//! `<scheme>://<account>/<item>` (`jellyfin://…`, `opds://…`).

use std::sync::atomic::Ordering;

use crate::comic::PageSource;
use crate::remote_zip::RangeSource;
use crate::{jellyfin, opds};

/// A series (or one-shot) on a server, ready to store in the library.
#[derive(Clone, Debug)]
pub struct RemoteSeries {
    pub key: String,
    pub title: String,
    pub is_folder: bool,
    /// `<scheme>://<account>/<library>`: groups and prunes per library.
    pub library_root: String,
    /// The library's name on the server ("Manga").
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

pub fn is_key(key: &str) -> bool {
    jellyfin::is_key(key) || opds::is_key(key)
}

/// Splits a `<scheme>://<account>/<item>` key into account and item.
pub fn parse_key(key: &str) -> Option<(&str, &str)> {
    jellyfin::parse_key(key).or_else(|| opds::parse_key(key))
}

/// An account on either kind of server.
#[derive(Clone, Debug, PartialEq)]
pub enum Account {
    Jellyfin(jellyfin::Account),
    Opds(opds::Account),
}

impl Account {
    /// Stable id, the `<account>` part of library keys.
    pub fn key(&self) -> String {
        match self {
            Account::Jellyfin(a) => a.key(),
            Account::Opds(a) => a.key(),
        }
    }

    pub fn item_key(&self, item_id: &str) -> String {
        match self {
            Account::Jellyfin(a) => a.item_key(item_id),
            Account::Opds(a) => a.item_key(item_id),
        }
    }

    /// What every key of this account starts with.
    pub fn key_prefix(&self) -> String {
        self.item_key("")
    }

    /// "Jellyfin" or "OPDS".
    pub fn kind(&self) -> &'static str {
        match self {
            Account::Jellyfin(_) => "Jellyfin",
            Account::Opds(_) => "OPDS",
        }
    }

    pub fn server_name(&self) -> &str {
        match self {
            Account::Jellyfin(a) => &a.server_name,
            Account::Opds(a) => a.name(),
        }
    }

    pub fn url(&self) -> &str {
        match self {
            Account::Jellyfin(a) => &a.url,
            Account::Opds(a) => &a.url,
        }
    }

    /// The user signed in, empty for a catalog without sign-in.
    pub fn user_name(&self) -> &str {
        match self {
            Account::Jellyfin(a) => &a.user_name,
            Account::Opds(a) => &a.user,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Account::Jellyfin(a) => a.label(),
            Account::Opds(a) => a.label(),
        }
    }

    /// Byte-range access to an item's file, for streaming pages.
    pub fn range_source(&self, item_id: &str) -> Box<dyn RangeSource> {
        match self {
            Account::Jellyfin(a) => Box::new(a.range_source(item_id)),
            Account::Opds(a) => Box::new(a.file(item_id)),
        }
    }

    /// Downloads an item's whole file to `path` (for the cache).
    pub fn download_to(&self, item_id: &str, path: &std::path::Path) -> Result<(), String> {
        match self {
            Account::Jellyfin(a) => a.range_source(item_id).download_to(path),
            Account::Opds(a) => a.file(item_id).download_to(path),
        }
    }

    /// Pages served one by one as images, for items the server renders
    /// itself (OPDS page streaming). Blocking: may fetch a manifest.
    pub fn page_stream(&self, item_id: &str) -> Option<Vec<PageSource>> {
        match self {
            Account::Jellyfin(_) => None,
            Account::Opds(a) => a.page_stream(item_id),
        }
    }

    pub async fn cover(&self, item_id: &str, max_height: u32) -> Result<Vec<u8>, String> {
        match self {
            Account::Jellyfin(a) => jellyfin::cover(a, item_id, max_height).await,
            Account::Opds(a) => a.cover(item_id).await,
        }
    }

    pub async fn ping(&self) -> bool {
        match self {
            Account::Jellyfin(a) => jellyfin::ping(&a.url).await,
            Account::Opds(a) => a.ping().await,
        }
    }

    pub async fn fetch_library(&self) -> Result<Vec<RemoteSeries>, String> {
        match self {
            Account::Jellyfin(a) => jellyfin::fetch_library(a).await,
            Account::Opds(a) => opds::fetch_library(a).await,
        }
    }

    /// Downloads an item for reading offline, see [`save_response`].
    pub async fn download_item(&self, item_id: &str, dest: &std::path::Path, progress: &Progress) -> Result<u64, String> {
        match self {
            Account::Jellyfin(a) => jellyfin::download_item(a, item_id, dest, progress).await,
            Account::Opds(a) => a.download_item(item_id, dest, progress).await,
        }
    }

    /// Whether reading progress is saved to the server (Jellyfin) or only
    /// on this computer (OPDS has no way to send it).
    pub fn saves_progress(&self) -> bool {
        matches!(self, Account::Jellyfin(_))
    }
}

// ── Downloads ──────────────────────────────────────────────────────────────

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

/// Saves a download's body to `dest` (via `dest.part`, flushed to the drive
/// and renamed when complete), refusing when the drive is too full. The
/// folder must exist: it's never created here, so a download can't land in
/// an empty mount point when its drive has been unplugged. Returns its size.
pub async fn save_response(mut resp: reqwest::Response, dest: &std::path::Path, progress: &Progress) -> Result<u64, String> {
    use std::io::Write;

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
pub fn parse_content_range_total(v: &str) -> Option<u64> {
    v.rsplit('/').next()?.trim().parse().ok()
}
