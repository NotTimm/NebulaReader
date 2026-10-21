//! Jellyfin in the app: connecting accounts (Quick Connect), syncing book
//! libraries into the local library, streaming chapters into the reader and
//! writing reading progress back to Jellyfin.
//!
//! Jellyfin series are ordinary library entries keyed `jellyfin://…`, so
//! cards, series pages, Continue Reading and progress all work unchanged.
//! The reader loads a window of chapters (the one being read, one before
//! and a few after) and appends more as you approach the end, so a 400
//! chapter series opens as fast as a short one.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, icon};
use cosmic::{app::Task, Element};

use super::{App, AppView, Message};
use crate::comic::{self, ChapterInfo, PageSource};
use crate::jellyfin::{self, Account, RemoteSeries};
use crate::library::{self, IssueRow, ScannedSeries, SeriesKind};
use crate::naming;
use crate::remote_rar::RemoteRar;
use crate::remote_zip::{RangeSource, RemoteZip};

/// Chapters loaded after the one being opened, and per top-up after that.
const WINDOW_AHEAD: usize = 3;
/// Start loading the next chapters when this close to the last loaded page.
const TOP_UP_PAGES: usize = 40;
/// Quick Connect codes expire on the server after a few minutes.
const QUICK_CONNECT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Progress goes to the server once you've stayed on a page this long (or
/// right away on finishing a chapter, leaving the reader or quitting), like
/// Jellyfin's own players, instead of on every page turn.
const SAVE_AFTER: Duration = Duration::from_secs(10);
/// How often to tell the server we're still reading (it ends a session
/// after 5 minutes without word).
const CHECK_IN_EVERY: Duration = Duration::from_secs(60);
/// How often to check the server is still reachable while reading.
const PING_EVERY: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub enum JfMsg {
    UrlInput(String),
    Connect,
    ServerChecked(String, Result<(jellyfin::ServerInfo, bool), String>),
    QuickConnectStarted(String, String, Result<jellyfin::QuickConnect, String>),
    QuickConnectPolled(String, Result<bool, String>),
    OpenApprovePage,
    UserInput(String),
    PasswordInput(String),
    PasswordSubmit,
    Cancel,
    SignedIn(Result<Account, String>),
    SignOut(String),
    Sync(String),
    Fetched(String, Result<Vec<RemoteSeries>, String>),
    CoverSaved,
    /// Shows the covers saved over the last moment, all at once.
    ShowSavedCovers,
    /// The first chapters of a series came in.
    FirstLoaded(String, Loaded),
    MoreLoaded(String, Result<Loaded, String>),
    /// Fetch the big chapter asked about; true: don't ask again.
    FetchWhole(bool),
    /// Don't fetch it.
    SkipWhole,
    SetAskBigFetch(bool),
    EpubDownloaded(Result<PathBuf, String>),
    /// A progress save came back; the items it carried, to keep for later
    /// if the server couldn't be reached.
    Saved(Result<(), String>, Vec<(String, i64, bool)>),
    /// Progress kept while offline was sent (item key, when it was kept).
    PendingSent(String, i64, Result<(), String>),
    /// Try the servers that couldn't be reached again.
    RetryOffline,
    /// Wakes the reader up to send a due save or ping.
    Timer,
    Pinged(bool),
    /// The window is closing: send any unsaved progress now.
    Closing,
    /// A dashboard reading-session report came back.
    SessionReported(Result<(), String>),
}

/// A Quick Connect sign-in waiting for the user to approve the code.
pub struct PendingSignIn {
    url: String,
    server_name: String,
    secret: String,
    code: String,
    started: Instant,
}

#[derive(Default)]
pub struct JfState {
    /// Covers were saved and a library refresh to show them is coming.
    pub covers_pending_refresh: bool,
    pub url_input: String,
    pub status: Option<String>,
    pub busy: bool,
    pending: Option<PendingSignIn>,
    /// Server (url, name) asking for username/password (Quick Connect off).
    password_for: Option<(String, String)>,
    user_input: String,
    password_input: String,
    pub syncing: HashSet<String>,
    /// Accounts whose server couldn't be reached at the last sync; their
    /// libraries show only what's downloaded until it's back.
    pub offline: HashSet<String>,
    /// Item id -> file extension, from the latest sync.
    pub(super) extensions: HashMap<String, String>,
    /// Library root key -> "Manga · deepdish", for section headings.
    pub library_names: HashMap<String, String>,
    /// Account of the EPUB being downloaded, then of the one open (with its
    /// cached path), for the reader's title bar.
    book_pending: Option<Account>,
    book: Option<(PathBuf, Account)>,
    /// The open book's (issue key, series key), for its download button.
    book_keys: Option<(String, String)>,
    /// Progress saving and reachability of the server being read from.
    live: Live,
    /// A big chapter waiting for the user's go-ahead to be fetched whole.
    pub(super) blocked: Option<Blocked>,
    /// Series whose big chapters may be fetched without asking again.
    allowed_whole: HashSet<String>,
    /// What's being opened: (series key, chapter), to open it again once
    /// a big first chapter is allowed.
    opening: Option<(String, Option<usize>)>,
    /// The item shown as being read on the server's dashboard, and when we
    /// last checked in about it.
    reading: Option<(Account, String, Instant)>,
}

/// What [`load_issues`] got: the pages of the chapters it could load, and
/// the chapter it stopped at to ask first, if any.
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    sources: Vec<PageSource>,
    chapters: Vec<ChapterInfo>,
    blocked: Option<Blocked>,
}

/// A chapter that can't be streamed and is big: (issue key, name, size).
#[derive(Clone, Debug)]
pub struct Blocked {
    series_key: String,
    name: String,
    size: u64,
    /// Nothing of the series is showing yet.
    first: bool,
    /// "Not now" was picked: stop at it without asking again.
    skipped: bool,
}

/// Chapters that have to be fetched whole are asked about above this.
const BIG_FETCH: u64 = 100 * 1024 * 1024;

/// Where the reader is in a Jellyfin item, as it would be reported.
#[derive(Clone)]
struct Report {
    account: Account,
    item_id: String,
    ticks: i64,
    played: bool,
}

impl Report {
    fn id(&self) -> (String, i64, bool) {
        (self.item_id.clone(), self.ticks, self.played)
    }
}

#[derive(Default)]
struct Live {
    /// The latest position and when it last changed.
    pending: Option<(Report, Instant)>,
    /// The last position sent, so an unchanged one isn't sent again.
    sent: Option<(String, i64, bool)>,
    /// Saves sent and not answered yet.
    saving: usize,
    saved_at: Option<Instant>,
    /// The latest thing that went wrong (cleared by the next success).
    error: Option<String>,
    online: Option<bool>,
    pinged_at: Option<Instant>,
    pinging: bool,
}

/// The Jellyfin series open in the reader.
pub struct RemoteReading {
    account: Account,
    pub(super) series_key: String,
    /// Issue keys in reading order.
    issues: Vec<(String, String)>,
    /// Index of the next issue not yet loaded.
    next: usize,
    loading: bool,
}

// ── Background work ─────────────────────────────────────────────────────────

/// Fetches an account's library. `left_open` is an item the last run didn't
/// close: its reading session is ended and its last saved progress put back
/// first, before the server's idle timeout could have marked it read.
fn fetch_task(account: Account, left_open: Option<jellyfin::OpenItem>) -> Task<Message> {
    let key = account.key();
    cosmic::task::future(async move {
        if let Some(open) = left_open {
            let _ = jellyfin::report_reading_stopped(&account, &open.item_id).await;
            if let Some((ticks, played)) = open.saved {
                let _ = jellyfin::save_progress(&account, &open.item_id, ticks, played).await;
            }
        }
        let result = jellyfin::fetch_library(&account).await;
        Message::Jf(JfMsg::Fetched(key, result))
    })
}

/// Saves a series or issue cover from Jellyfin's image, falling back to the
/// archive's first page when Jellyfin has none.
fn cover_task(account: Account, item_id: String, path: PathBuf, done: Message) -> Task<Message> {
    cosmic::task::future(async move {
        let bytes = jellyfin::cover(&account, &item_id, 680).await;
        let _ = tokio::task::spawn_blocking(move || -> Result<(), String> {
            let img = match bytes.ok().and_then(|b| image::load_from_memory(&b).ok()) {
                Some(img) => img.to_rgba8(),
                None => {
                    let zip = RemoteZip::open(Box::new(account.range_source(&item_id)))?;
                    let page = comic::decode_bytes(&zip.entries.first().ok_or("empty")?.name, &zip.read(0)?)?;
                    image::RgbaImage::from_raw(page.width, page.height, page.rgba).ok_or("bad image")?
                }
            };
            crate::scan::save_thumb(&img, &path)
        })
        .await;
        done
    })
}

/// Whether a chapter might have to be fetched whole before it can be read:
/// not downloaded, and not a zip (which always streams page by page). A
/// CBR streams too unless it turns out to be solid.
fn must_fetch_whole(extensions: &HashMap<String, String>, downloaded: &HashMap<String, PathBuf>, key: &str) -> bool {
    let item_id = jellyfin::parse_key(key).map(|(_, id)| id).unwrap_or_default();
    let ext = extensions.get(item_id).map(String::as_str).unwrap_or("");
    !matches!(ext, "" | "cbz" | "zip") && !downloaded.get(key).is_some_and(|f| f.exists())
}

impl App {
    fn must_fetch_whole(&self, key: &str) -> bool {
        must_fetch_whole(&self.jf.extensions, &self.dl.files(), key)
    }

    /// The size above which a chapter that has to be fetched whole is
    /// asked about first, unless the user said not to.
    fn ask_over(&self, series_key: &str) -> Option<u64> {
        (self.settings.ask_before_big_fetch && !self.jf.allowed_whole.contains(series_key)).then_some(BIG_FETCH)
    }
}

/// Lists the pages of some issues: streamed for zip archives, downloaded
/// to the cache first for anything else (RAR can't be read out of order).
fn load_issues(
    account: &Account,
    series_key: &str,
    issues: &[(String, String)],
    extensions: &HashMap<String, String>,
    downloaded: &HashMap<String, PathBuf>,
    ask_over: Option<u64>,
) -> Result<Loaded, String> {
    // Each streamed chapter's table of contents is a round trip or more
    // to the server; ask for all of them at once rather than in turn.
    let opened: Vec<Opened> = std::thread::scope(|scope| {
        let jobs: Vec<_> = issues
            .iter()
            .map(|(key, _)| scope.spawn(move || open_streamed(account, key, extensions, downloaded)))
            .collect();
        jobs.into_iter().map(|j| j.join().expect("opening a chapter panicked")).collect()
    });

    let mut sources = Vec::new();
    let mut chapters = Vec::new();
    for ((key, name), opened) in issues.iter().zip(opened) {
        let item_id = jellyfin::parse_key(key).map(|(_, id)| id).unwrap_or_default();
        let Opened { local, ext, cached, streamed, known_size } = opened;
        let start = sources.len();
        if let Some(file) = local {
            let (pages, _) = comic::collect_sources(&file)?;
            sources.extend(pages);
        } else if let Some(pages) = streamed {
            sources.extend::<Vec<PageSource>>(pages);
        } else {
            // Has to be fetched whole first. Only as the first of a batch,
            // so one big file at a time is fetched as the reader gets to it.
            if !chapters.is_empty() && !cached.exists() {
                return Ok(Loaded { sources, chapters, blocked: None });
            }
            // Ask when it's big.
            if let (Some(limit), false) = (ask_over, cached.exists()) {
                let size = known_size
                    .or_else(|| account.range_source(item_id).tail(1).ok().map(|(_, len)| len))
                    .unwrap_or(0);
                if size > limit {
                    let blocked = Blocked {
                        series_key: series_key.to_string(),
                        name: comic::display_name(name),
                        size,
                        first: false,
                        skipped: false,
                    };
                    return Ok(Loaded { sources, chapters, blocked: Some(blocked) });
                }
            }
            let local = download(account, item_id, &ext)?;
            let (pages, _) = comic::collect_sources(&local)?;
            sources.extend(pages);
        }
        chapters.push(ChapterInfo {
            name: comic::display_name(name),
            path: PathBuf::from(key),
            start,
            page_count: sources.len() - start,
        });
    }
    Ok(Loaded { sources, chapters, blocked: None })
}

/// How one chapter of a batch will be read, worked out before its pages
/// are listed.
struct Opened {
    /// The user's downloaded copy.
    local: Option<PathBuf>,
    ext: String,
    /// Where a whole fetch goes (or went).
    cached: PathBuf,
    /// Its pages, read straight from the server.
    streamed: Option<Vec<PageSource>>,
    /// Its size, when opening it told us (a solid RAR).
    known_size: Option<u64>,
}

/// Opens a chapter for streaming when it can be: a zip (or non-solid RAR)
/// on the server that isn't downloaded or cached already.
fn open_streamed(
    account: &Account,
    key: &str,
    extensions: &HashMap<String, String>,
    downloaded: &HashMap<String, PathBuf>,
) -> Opened {
    let item_id = jellyfin::parse_key(key).map(|(_, id)| id).unwrap_or_default();
    let ext = extensions.get(item_id).map(String::as_str).filter(|e| !e.is_empty()).unwrap_or("cbz").to_string();
    // Downloaded: read the local copy, no network needed.
    let local = downloaded.get(key).filter(|f| f.exists()).cloned();
    let cached = library::cache_dir().join("jellyfin").join(format!("{item_id}.{ext}"));
    let mut known_size = None;
    let streamed = if local.is_some() || cached.exists() {
        None
    } else if matches!(ext.as_str(), "cbz" | "zip") {
        RemoteZip::open(Box::new(account.range_source(item_id)))
            .ok()
            .filter(|z| !z.entries.is_empty())
            .map(|zip| (0..zip.entries.len()).map(|index| PageSource::Remote { archive: zip.clone(), index }).collect())
    } else if matches!(ext.as_str(), "cbr" | "rar") {
        // Pages are read one by one unless the archive is solid.
        let index = super::rar_index::index_path(item_id);
        match RemoteRar::open(Box::new(account.range_source(item_id)), Some(&index)) {
            Ok(rar) if !rar.solid() && !rar.entries().is_empty() => Some(
                (0..rar.entries().len()).map(|index| PageSource::RemoteRar { archive: rar.clone(), index }).collect(),
            ),
            Ok(rar) => {
                known_size = Some(rar.len());
                None
            }
            Err(_) => None,
        }
    } else {
        None
    };
    Opened { local, ext, cached, streamed, known_size }
}

/// Downloads an item into the cache (kept for re-reads; the cache is safe
/// to clear).
fn download(account: &Account, item_id: &str, ext: &str) -> Result<PathBuf, String> {
    let path = library::cache_dir().join("jellyfin").join(format!("{item_id}.{ext}"));
    if path.exists() {
        // Recently read: kept longest when the cache is trimmed.
        let _ = std::fs::File::options().append(true).open(&path).and_then(|f| f.set_modified(std::time::SystemTime::now()));
    } else {
        account.range_source(item_id).download_to(&path)?;
    }
    Ok(path)
}

#[allow(clippy::too_many_arguments)]
fn load_task(
    account: Account,
    series_key: String,
    issues: Vec<(String, String)>,
    extensions: HashMap<String, String>,
    downloaded: HashMap<String, PathBuf>,
    ask_over: Option<u64>,
    more: bool,
) -> Task<Message> {
    cosmic::task::future(async move {
        let key = series_key.clone();
        let result = tokio::task::spawn_blocking(move || {
            load_issues(&account, &series_key, &issues, &extensions, &downloaded, ask_over)
        })
        .await
        .unwrap_or_else(|e| Err(format!("loading panicked: {e}")));
        if more {
            Message::Jf(JfMsg::MoreLoaded(key, result))
        } else {
            match result {
                Ok(loaded) => Message::Jf(JfMsg::FirstLoaded(key, loaded)),
                Err(e) => Message::LoadFailed(e),
            }
        }
    })
}

fn save_task(r: Report) -> Task<Message> {
    cosmic::task::future(async move {
        let result = jellyfin::save_progress(&r.account, &r.item_id, r.ticks, r.played).await;
        Message::Jf(JfMsg::Saved(result, vec![(r.account.item_key(&r.item_id), r.ticks, r.played)]))
    })
}

/// Sends progress kept while the server was offline.
fn pending_task(account: Account, item_key: String, ticks: i64, played: bool, at: i64) -> Task<Message> {
    cosmic::task::future(async move {
        let id = jellyfin::parse_key(&item_key).map(|(_, id)| id.to_string()).unwrap_or_default();
        let result = jellyfin::save_progress(&account, &id, ticks, played).await;
        Message::Jf(JfMsg::PendingSent(item_key, at, result))
    })
}

/// A failed save that means the server couldn't be reached (as opposed to
/// the server answering with an error).
fn unreachable(e: &str) -> bool {
    !e.starts_with("HTTP")
}

fn session_task(report: impl std::future::Future<Output = Result<(), String>> + Send + 'static) -> Task<Message> {
    cosmic::task::future(async move { Message::Jf(JfMsg::SessionReported(report.await)) })
}

fn ping_task(url: String) -> Task<Message> {
    cosmic::task::future(async move { Message::Jf(JfMsg::Pinged(jellyfin::ping(&url).await)) })
}

fn poll_task(url: String, secret: String) -> Task<Message> {
    cosmic::task::future(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let r = jellyfin::quick_connect_approved(&url, &secret).await;
        Message::Jf(JfMsg::QuickConnectPolled(secret, r))
    })
}

impl App {
    /// Syncs every signed-in account (at startup and on demand).
    pub(super) fn sync_all_jellyfin(&mut self) -> Task<Message> {
        let accounts = self.jellyfin.accounts.clone();
        let mut left_open = jellyfin::take_open();
        Task::batch(accounts.into_iter().map(|a| {
            self.jf.syncing.insert(a.key());
            let open = left_open.take_if(|o| o.account_key == a.key());
            fetch_task(a, open)
        }))
    }

    pub(super) fn account_for(&self, key: &str) -> Option<Account> {
        let (account_key, _) = jellyfin::parse_key(key)?;
        self.jellyfin.find(account_key).cloned()
    }

    /// Opens a Jellyfin series at `chapter` (or where reading left off).
    pub(super) fn open_jellyfin(&mut self, key: String, chapter: Option<usize>) -> Task<Message> {
        let Some(account) = self.account_for(&key) else {
            return Task::done(cosmic::Action::App(Message::LoadFailed(
                "Not signed in to this Jellyfin server any more. Add it again in Settings.".into(),
            )));
        };
        let Some(db) = &self.db else { return Task::none() };
        let issues: Vec<(String, String)> =
            library::issues_for(db, &key).into_iter().map(|i| (i.path, i.name)).collect();
        if issues.is_empty() {
            return Task::done(cosmic::Action::App(Message::LoadFailed("This series has no chapters on the server.".into())));
        }

        // EPUBs open in the book reader from a downloaded copy: the user's
        // download if there is one, else one fetched into the cache.
        let first_id = jellyfin::parse_key(&issues[0].0).map(|(_, id)| id.to_string()).unwrap_or_default();
        let saved = self.dl.file(&issues[0].0).cloned();
        let is_epub = self.jf.extensions.get(&first_id).is_some_and(|e| e == "epub")
            || saved.as_ref().is_some_and(|f| f.extension().is_some_and(|e| e == "epub"));
        if issues.len() == 1 && is_epub {
            self.jf.book_keys = Some((issues[0].0.clone(), key.clone()));
            if let Some(path) = saved {
                self.jf.book_pending = Some(account);
                return Task::done(cosmic::Action::App(Message::Jf(JfMsg::EpubDownloaded(Ok(path)))));
            }
            self.jf.book_pending = Some(account.clone());
            self.jf.live = Live::default();
            return cosmic::task::future(async move {
                let r = tokio::task::spawn_blocking(move || download(&account, &first_id, "epub"))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                Message::Jf(JfMsg::EpubDownloaded(r))
            });
        }

        // Resume: the requested chapter, else the last one read (or the one
        // after it if that was finished).
        let entry = library::get_series(db, &key);
        let all = library::issues_for(db, &key);
        let start = chapter.unwrap_or_else(|| {
            let last = entry.as_ref().map(|e| e.last_issue_path.clone()).unwrap_or_default();
            match all.iter().position(|i| i.path == last) {
                Some(i) if all[i].completed && i + 1 < all.len() => i + 1,
                Some(i) => i,
                None => 0,
            }
        });
        // Offline, only the downloaded chapters can be read.
        let (issues, start) = if self.jf.offline.contains(&account.key()) {
            let want = all.get(start).map(|i| i.path.clone());
            let kept: Vec<(String, String)> = issues.into_iter().filter(|(k, _)| self.dl.file(k).is_some()).collect();
            if kept.is_empty() {
                return Task::done(cosmic::Action::App(Message::LoadFailed(format!(
                    "{} can't be reached, and nothing from this series is downloaded.",
                    account.server_name
                ))));
            }
            let start = want.and_then(|w| kept.iter().position(|(k, _)| *k == w)).unwrap_or(0);
            (kept, start)
        } else {
            (issues, start)
        };
        self.jf.blocked = None;
        self.jf.opening = Some((key.clone(), chapter));
        let start = start.min(issues.len() - 1);
        // The chapter before is loaded too, unless either might have to be
        // fetched whole (hundreds of MB for a CBR). `load_issues` stops
        // before a whole fetch that isn't the first in its batch.
        let heavy = self.must_fetch_whole(&issues[start].0)
            || start.checked_sub(1).is_some_and(|p| self.must_fetch_whole(&issues[p].0));
        let first = if heavy { start } else { start.saturating_sub(1) };
        let end = (start + 1 + WINDOW_AHEAD).min(issues.len());
        self.start_chapter = Some(start - first);
        self.remote = Some(RemoteReading {
            account: account.clone(),
            series_key: key.clone(),
            issues: issues.clone(),
            // Moved on by the chapters that actually load.
            next: first,
            loading: false,
        });
        self.jf.live = Live::default();
        let ask = self.ask_over(&key);
        load_task(account, key, issues[first..end].to_vec(), self.jf.extensions.clone(), self.dl.files(), ask, false)
    }

    /// Asks before fetching a big chapter whole: in place of the page when
    /// nothing is showing yet (`first`), else as a banner above it.
    pub(super) fn big_fetch_prompt(&self, first: bool) -> Option<Element<'_, Message>> {
        let b = self.jf.blocked.as_ref().filter(|b| !b.skipped && b.first == first)?;
        if self.app_view != AppView::Reader || self.open_path.as_ref().is_none_or(|p| p.to_string_lossy() != b.series_key) {
            return None;
        }
        let size = super::downloads::human_size(b.size);
        let buttons = widget::row::with_capacity(3)
            .spacing(8)
            .push(widget::button::suggested(if first { "Download and Read" } else { "Download" }).on_press(Message::Jf(JfMsg::FetchWhole(false))))
            .push(widget::button::standard("Don't Ask Again").on_press(Message::Jf(JfMsg::FetchWhole(true))))
            .push(widget::button::standard(if first { "Cancel" } else { "Not Now" }).on_press(Message::Jf(JfMsg::SkipWhole)));
        if first {
            return Some(
                widget::column::with_capacity(4)
                    .spacing(12)
                    .align_x(Alignment::Center)
                    .max_width(520.0)
                    .push(icon::from_name("folder-download-symbolic").size(48))
                    .push(widget::text::title3(format!("Download {size}?")))
                    .push(widget::text::body(format!(
                        "“{}” can't be read a page at a time (it's packed as one solid block), so the whole file has to come from the server before it opens.",
                        b.name
                    )))
                    .push(buttons)
                    .into(),
            );
        }
        Some(
            widget::row::with_capacity(3)
                .spacing(12)
                .padding([4, 12])
                .align_y(Alignment::Center)
                .push(icon::from_name("folder-download-symbolic").size(16))
                .push(
                    widget::text::body(format!("The next chapter, “{}”, has to be downloaded whole ({size}).", b.name))
                        .width(Length::Fill),
                )
                .push(buttons)
                .into(),
        )
    }

    /// The Jellyfin chapter or book on screen: (issue key, series key).
    pub(super) fn current_jellyfin_item(&self) -> Option<(String, String)> {
        if self.app_view != AppView::Reader || self.loading {
            return None;
        }
        let open = self.open_path.as_ref()?;
        if self.jf.book.as_ref().is_some_and(|(p, _)| p == open) {
            return self.jf.book_keys.clone();
        }
        let r = self.remote.as_ref().filter(|r| open.to_string_lossy() == r.series_key)?;
        let (_, ch, _) = self.current_chapter()?;
        Some((ch.path.to_string_lossy().into_owned(), r.series_key.clone()))
    }

    /// The position to report for what's open, if it's from Jellyfin.
    fn current_report(&self) -> Option<Report> {
        if self.app_view != AppView::Reader || self.loading {
            return None;
        }
        let open = self.open_path.as_ref()?;

        // A book: how far through it, as a fraction.
        if let Some((path, account)) = self.jf.book.as_ref().filter(|(p, _)| p == open) {
            let book = self.epub_book.as_ref()?;
            let n = book.chapters.len().max(1);
            let pos = self.book_scroll.rel_y.clamp(0.0, 1.0);
            let last_page = self.is_paged()
                && self.current_book_page().is_some_and(|(page, count, step)| page + step >= count);
            let played = self.epub_chapter + 1 >= n && (pos >= 0.97 || last_page);
            let fraction = ((self.epub_chapter as f64 + pos as f64) / n as f64).clamp(0.0, 1.0);
            return Some(Report {
                account: account.clone(),
                item_id: path.file_stem()?.to_string_lossy().into_owned(),
                ticks: (fraction * 10_000_000.0) as i64,
                played,
            });
        }

        // A comic: the page in the chapter on screen.
        let r = self.remote.as_ref().filter(|r| open.to_string_lossy() == r.series_key)?;
        let (_, ch, pg) = self.current_chapter()?;
        let pg = pg + self.pages_shown_at(self.current_page) - 1;
        let pg = pg.min(ch.page_count.saturating_sub(1));
        let issue = ch.path.to_string_lossy();
        let (_, item_id) = jellyfin::parse_key(&issue)?;
        Some(Report {
            account: r.account.clone(),
            item_id: item_id.to_string(),
            ticks: pg as i64 * 10_000,
            played: pg + 1 >= ch.page_count,
        })
    }

    /// Runs after every message (and on a timer while reading from
    /// Jellyfin): loads more chapters before the reader runs out, sends
    /// progress once it has settled, and checks the server is reachable.
    pub(super) fn jellyfin_tick(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        let report = self.current_report();

        if let Some(r) = self.remote.as_mut() {
            let reading = self.app_view == AppView::Reader
                && !self.loading
                && self.open_path.as_ref().is_some_and(|p| p.to_string_lossy() == r.series_key);
            let near_end = self.current_page + TOP_UP_PAGES >= self.sources.len();
            if reading && near_end && !r.loading && r.next < r.issues.len() && self.jf.blocked.is_none() {
                r.loading = true;
                let end = (r.next + WINDOW_AHEAD).min(r.issues.len());
                let issues = r.issues[r.next..end].to_vec();
                let ask = (self.settings.ask_before_big_fetch && !self.jf.allowed_whole.contains(&r.series_key)).then_some(BIG_FETCH);
                tasks.push(load_task(r.account.clone(), r.series_key.clone(), issues, self.jf.extensions.clone(), self.dl.files(), ask, true));
            }
        }

        let live = &mut self.jf.live;
        let now = Instant::now();
        if let Some(rep) = &report {
            if live.pending.as_ref().map(|(p, _)| p.id()) != Some(rep.id()) {
                live.pending = Some((rep.clone(), now));
            }
        }
        if let Some((rep, since)) = &live.pending {
            // Not reading any more (left the reader, opened something else):
            // send what's left right away.
            let due = report.is_none() || rep.played || since.elapsed() >= SAVE_AFTER;
            if due && live.sent.as_ref() != Some(&rep.id()) {
                live.sent = Some(rep.id());
                live.saving += 1;
                tasks.push(save_task(rep.clone()));
                if self.jf.reading.as_ref().is_some_and(|(_, id, _)| *id == rep.item_id) {
                    jellyfin::remember_open(Some(&jellyfin::OpenItem {
                        account_key: rep.account.key(),
                        item_id: rep.item_id.clone(),
                        saved: Some((rep.ticks, rep.played)),
                    }));
                }
            }
        }
        if report.is_none() {
            live.pending = None;
        }

        tasks.push(self.update_reading_session(report.as_ref()));

        let live = &mut self.jf.live;
        if let Some(rep) = &report {
            if !live.pinging && live.pinged_at.is_none_or(|t| t.elapsed() >= PING_EVERY) {
                live.pinging = true;
                live.pinged_at = Some(now);
                tasks.push(ping_task(rep.account.url.clone()));
            }
        }
        Task::batch(tasks)
    }

    /// Keeps the server's dashboard in step with what's open: starts a
    /// reading session for a newly opened item, checks in about it every
    /// minute, and ends it when the item closes.
    fn update_reading_session(&mut self, report: Option<&Report>) -> Task<Message> {
        let mut tasks = Vec::new();
        let open = report.map(|r| r.item_id.as_str());
        if self.jf.reading.as_ref().map(|(_, id, _)| id.as_str()) != open {
            if let Some((account, item_id, _)) = self.jf.reading.take() {
                tasks.push(session_task(async move { jellyfin::report_reading_stopped(&account, &item_id).await }));
                jellyfin::remember_open(None);
            }
            if let Some(rep) = report {
                let (account, item_id) = (rep.account.clone(), rep.item_id.clone());
                self.jf.reading = Some((account.clone(), item_id.clone(), Instant::now()));
                jellyfin::remember_open(Some(&jellyfin::OpenItem {
                    account_key: account.key(),
                    item_id: item_id.clone(),
                    saved: None,
                }));
                tasks.push(session_task(async move { jellyfin::report_reading_started(&account, &item_id).await }));
            }
        } else if let Some((account, item_id, at)) = self.jf.reading.as_mut() {
            if at.elapsed() >= CHECK_IN_EVERY {
                *at = Instant::now();
                let (account, item_id) = (account.clone(), item_id.clone());
                tasks.push(session_task(async move { jellyfin::report_reading_check_in(&account, &item_id).await }));
            }
        }
        Task::batch(tasks)
    }

    /// A timer while something from Jellyfin is open, so a settled page is
    /// saved and the server pinged even when nothing else happens.
    pub(super) fn jellyfin_subscription(&self) -> cosmic::iced::Subscription<Message> {
        let reading = if self.jellyfin_source().is_some() {
            cosmic::iced::time::every(Duration::from_secs(5)).map(|_| Message::Jf(JfMsg::Timer))
        } else {
            cosmic::iced::Subscription::none()
        };
        // An offline server is tried again every minute.
        let retry = if self.jf.offline.is_empty() {
            cosmic::iced::Subscription::none()
        } else {
            cosmic::iced::time::every(Duration::from_secs(60)).map(|_| Message::Jf(JfMsg::RetryOffline))
        };
        cosmic::iced::Subscription::batch([reading, retry])
    }

    /// Mirrors read/unread marks from the series page to Jellyfin.
    pub(super) fn jellyfin_mark(&self, issue_keys: Vec<String>, played: bool) -> Task<Message> {
        let Some(account) = issue_keys.first().and_then(|k| self.account_for(k)) else { return Task::none() };
        cosmic::task::future(async move {
            let mut result = Ok(());
            let mut failed = Vec::new();
            for key in issue_keys {
                if let Some((_, id)) = jellyfin::parse_key(&key) {
                    if let Err(e) = jellyfin::save_progress(&account, id, 0, played).await {
                        failed.push((key.clone(), 0, played));
                        result = Err(e);
                    }
                }
            }
            Message::Jf(JfMsg::Saved(result, failed))
        })
    }

    /// Cover for one issue on a Jellyfin series page.
    pub(super) fn jellyfin_issue_cover(&self, series_key: String, idx: usize, issue_key: &str) -> Task<Message> {
        let (Some(account), Some((_, item_id))) = (self.account_for(issue_key), jellyfin::parse_key(issue_key)) else {
            return Task::none();
        };
        let path = library::chapter_cover_path(&series_key, idx);
        cover_task(account, item_id.to_string(), path, Message::IssueCoverReady { key: series_key, idx })
    }

    pub(super) fn update_jellyfin(&mut self, msg: JfMsg) -> Task<Message> {
        match msg {
            JfMsg::UrlInput(s) => self.jf.url_input = s,
            JfMsg::Connect => {
                let input = self.jf.url_input.trim().to_string();
                if input.is_empty() {
                    return Task::none();
                }
                self.jf.busy = true;
                self.jf.status = Some(format!("Looking for Jellyfin at {input}…"));
                return cosmic::task::future(async move {
                    match jellyfin::find_server(&input).await {
                        Ok((url, info)) => {
                            let qc = jellyfin::quick_connect_enabled(&url).await;
                            Message::Jf(JfMsg::ServerChecked(url, Ok((info, qc))))
                        }
                        Err(e) => Message::Jf(JfMsg::ServerChecked(input, Err(e))),
                    }
                });
            }
            JfMsg::ServerChecked(url, result) => match result {
                Ok((info, true)) => {
                    let name = info.server_name.clone();
                    self.jf.status = Some(format!("Connected to {name}. Getting a sign-in code…"));
                    return cosmic::task::future(async move {
                        let r = jellyfin::quick_connect_start(&url).await;
                        Message::Jf(JfMsg::QuickConnectStarted(url, name, r))
                    });
                }
                Ok((info, false)) => {
                    self.jf.busy = false;
                    self.jf.status = Some(format!(
                        "{} has Quick Connect turned off, so sign in with your Jellyfin username and password. \
                         They're used once to get a token and not saved.",
                        info.server_name
                    ));
                    self.jf.password_for = Some((url, info.server_name));
                }
                Err(e) => {
                    self.jf.busy = false;
                    self.jf.status = Some(e);
                }
            },
            JfMsg::QuickConnectStarted(url, server_name, result) => match result {
                Ok(qc) => {
                    self.jf.status = None;
                    let poll = poll_task(url.clone(), qc.secret.clone());
                    self.jf.pending = Some(PendingSignIn {
                        url,
                        server_name,
                        secret: qc.secret,
                        code: qc.code,
                        started: Instant::now(),
                    });
                    return poll;
                }
                Err(e) => {
                    self.jf.busy = false;
                    self.jf.status = Some(format!("Couldn't start Quick Connect: {e}"));
                }
            },
            JfMsg::QuickConnectPolled(secret, result) => {
                let Some(p) = self.jf.pending.as_ref().filter(|p| p.secret == secret) else {
                    return Task::none(); // cancelled or superseded
                };
                match result {
                    Ok(true) => {
                        let (url, name, secret) = (p.url.clone(), p.server_name.clone(), p.secret.clone());
                        self.jf.status = Some("Approved. Signing in…".into());
                        return cosmic::task::future(async move {
                            Message::Jf(JfMsg::SignedIn(jellyfin::quick_connect_finish(&url, &name, &secret).await))
                        });
                    }
                    Ok(false) if p.started.elapsed() < QUICK_CONNECT_TIMEOUT => {
                        return poll_task(p.url.clone(), secret);
                    }
                    Ok(false) => {
                        self.jf.pending = None;
                        self.jf.busy = false;
                        self.jf.status = Some("The code expired. Click Connect to get a new one.".into());
                    }
                    Err(e) => {
                        self.jf.pending = None;
                        self.jf.busy = false;
                        self.jf.status = Some(e);
                    }
                }
            }
            JfMsg::OpenApprovePage => {
                if let Some(p) = &self.jf.pending {
                    crate::platform::open_url(&format!("{}/web/#/quickconnect", p.url));
                }
            }
            JfMsg::UserInput(s) => self.jf.user_input = s,
            JfMsg::PasswordInput(s) => self.jf.password_input = s,
            JfMsg::PasswordSubmit => {
                let Some((url, name)) = self.jf.password_for.clone() else { return Task::none() };
                let user = self.jf.user_input.clone();
                let password = std::mem::take(&mut self.jf.password_input);
                self.jf.busy = true;
                self.jf.status = Some("Signing in…".into());
                return cosmic::task::future(async move {
                    Message::Jf(JfMsg::SignedIn(jellyfin::sign_in_with_password(&url, &name, &user, &password).await))
                });
            }
            JfMsg::Cancel => {
                self.jf.pending = None;
                self.jf.password_for = None;
                self.jf.password_input.clear();
                self.jf.busy = false;
                self.jf.status = None;
            }
            JfMsg::SignedIn(result) => {
                self.jf.busy = false;
                self.jf.pending = None;
                match result {
                    Ok(account) => {
                        self.jf.password_for = None;
                        self.jf.password_input.clear();
                        self.jf.url_input.clear();
                        self.jf.status = Some(format!("Signed in as {}. Loading your books…", account.label()));
                        self.jf.syncing.insert(account.key());
                        self.jellyfin.upsert(account.clone());
                        return fetch_task(account, None);
                    }
                    Err(e) => self.jf.status = Some(e),
                }
            }
            JfMsg::SignOut(key) => {
                let Some(account) = self.jellyfin.find(&key).cloned() else { return Task::none() };
                self.jellyfin.remove(&key);
                if let Some(db) = &self.db {
                    library::remove_series_under_root_prefix(db, &format!("{}{key}/", jellyfin::KEY_PREFIX));
                }
                // Its downloads go with it.
                self.prune_downloads();
                self.jf.offline.remove(&key);
                self.jf.status = Some(format!("Signed out of {}.", account.server_name));
                let refresh = self.refresh_views();
                return Task::batch([
                    cosmic::task::future(async move {
                        jellyfin::sign_out(&account).await;
                        Message::Jf(JfMsg::Saved(Ok(()), Vec::new()))
                    }),
                    refresh,
                ]);
            }
            JfMsg::Sync(key) => {
                if let Some(account) = self.jellyfin.find(&key).cloned() {
                    self.jf.syncing.insert(key);
                    return fetch_task(account, None);
                }
            }
            JfMsg::Fetched(key, result) => {
                self.jf.syncing.remove(&key);
                match result {
                    Ok(series) => {
                        self.jf.offline.remove(&key);
                        let task = self.apply_jellyfin_library(&key, series);
                        // Which CBRs can have their page index built ahead.
                        return Task::batch([task, self.refresh_unindexed()]);
                    }
                    Err(e) => {
                        let server = self.jellyfin.find(&key).map(|a| a.server_name.clone()).unwrap_or_default();
                        if unreachable(&e) {
                            // Show what's downloaded from it until it's back.
                            self.jf.offline.insert(key);
                            self.jf.status = Some(format!("{server} can't be reached; showing your downloads from it."));
                            return self.refresh_views();
                        }
                        self.jf.status = Some(format!("Couldn't load {server}: {e}"));
                    }
                }
            }
            JfMsg::RetryOffline => {
                let keys: Vec<String> = self.jf.offline.iter().filter(|k| !self.jf.syncing.contains(*k)).cloned().collect();
                let mut tasks = Vec::new();
                for key in keys {
                    if let Some(account) = self.jellyfin.find(&key).cloned() {
                        self.jf.syncing.insert(key);
                        tasks.push(fetch_task(account, None));
                    }
                }
                return Task::batch(tasks);
            }
            JfMsg::PendingSent(item_key, at, result) => match result {
                Ok(()) => {
                    if let Some(db) = &self.db {
                        library::clear_pending_progress(db, &item_key, at);
                    }
                }
                Err(e) => log::warn!("jellyfin: sending progress kept offline for {item_key}: {e}"),
            },
            // A first sync saves hundreds of covers; reload the library
            // for a batch of them, not for each.
            JfMsg::CoverSaved => {
                if !std::mem::replace(&mut self.jf.covers_pending_refresh, true) {
                    return cosmic::task::future(async {
                        tokio::time::sleep(Duration::from_millis(400)).await;
                        Message::Jf(JfMsg::ShowSavedCovers)
                    });
                }
            }
            JfMsg::ShowSavedCovers => {
                self.jf.covers_pending_refresh = false;
                return self.reload_library();
            }
            JfMsg::FirstLoaded(key, loaded) => {
                if let Some(r) = self.remote.as_mut().filter(|r| r.series_key == key) {
                    r.next = (r.next + loaded.chapters.len()).min(r.issues.len());
                }
                self.jf.blocked = loaded.blocked;
                if loaded.chapters.is_empty() {
                    if let Some(b) = self.jf.blocked.as_mut() {
                        // Ask before showing anything.
                        b.first = true;
                        self.loading = false;
                        self.sources.clear();
                        self.chapters.clear();
                        self.clear_pages();
                        self.open_path = Some(PathBuf::from(&key));
                        return Task::none();
                    }
                    return Task::done(cosmic::Action::App(Message::LoadFailed("No pages found.".into())));
                }
                return Task::done(cosmic::Action::App(Message::Loaded(PathBuf::from(key), loaded.sources, loaded.chapters)));
            }
            JfMsg::FetchWhole(dont_ask) => {
                let Some(b) = self.jf.blocked.take() else { return Task::none() };
                self.jf.allowed_whole.insert(b.series_key.clone());
                if dont_ask {
                    self.settings.ask_before_big_fetch = false;
                    self.settings.save();
                }
                if b.first {
                    let chapter = self.jf.opening.as_ref().filter(|(k, _)| *k == b.series_key).and_then(|(_, c)| *c);
                    return self.open_entry(b.series_key, chapter);
                }
                // The next top-up fetches it.
            }
            JfMsg::SetAskBigFetch(on) => {
                self.settings.ask_before_big_fetch = on;
                self.settings.save();
            }
            JfMsg::SkipWhole => {
                if self.jf.blocked.as_ref().is_some_and(|b| b.first) {
                    self.jf.blocked = None;
                    return Task::done(cosmic::Action::App(Message::GoToLibrary));
                }
                if let Some(b) = self.jf.blocked.as_mut() {
                    b.skipped = true;
                }
            }
            JfMsg::MoreLoaded(key, result) => {
                let Some(r) = self.remote.as_mut().filter(|r| r.series_key == key) else { return Task::none() };
                r.loading = false;
                match result {
                    Ok(Loaded { sources, chapters, blocked }) => {
                        self.jf.blocked = blocked;
                        r.next = (r.next + chapters.len()).min(r.issues.len());
                        let offset = self.sources.len();
                        if let Some(db) = &self.db {
                            for ch in &chapters {
                                let _ = library::set_issue_page_count(db, &key, &ch.path.to_string_lossy(), ch.page_count);
                            }
                        }
                        self.sources.extend(sources);
                        self.chapters.extend(chapters.into_iter().map(|mut ch| {
                            ch.start += offset;
                            ch
                        }));
                    }
                    Err(e) => {
                        log::warn!("jellyfin: loading more chapters: {e}");
                        self.jf.live.error = Some(format!("Couldn't load the next chapters: {e}"));
                    }
                }
            }
            JfMsg::EpubDownloaded(result) => match result {
                Ok(path) => {
                    self.jf.book = self.jf.book_pending.take().map(|a| (path.clone(), a));
                    self.jf.live = Live::default();
                    return super::epub_open_task(path);
                }
                Err(e) => return Task::done(cosmic::Action::App(Message::LoadFailed(e))),
            },
            JfMsg::Saved(result, items) => {
                let live = &mut self.jf.live;
                live.saving = live.saving.saturating_sub(1);
                match result {
                    Ok(()) => {
                        live.saved_at = Some(Instant::now());
                        live.error = None;
                        live.online = Some(true);
                    }
                    Err(e) if unreachable(&e) => {
                        log::warn!("jellyfin: saving progress: {e}");
                        live.online = Some(false);
                        // Kept here and sent when the server is back.
                        if let Some(db) = &self.db {
                            for (item_key, ticks, played) in &items {
                                library::set_pending_progress(db, item_key, *ticks, *played);
                            }
                        }
                        live.error = Some("Server offline: progress is kept here and sent when it's back".into());
                    }
                    Err(e) => {
                        log::warn!("jellyfin: saving progress: {e}");
                        live.error = Some(format!("Couldn't save progress: {e}"));
                    }
                }
            }
            JfMsg::Timer => {}
            JfMsg::Pinged(ok) => {
                let live = &mut self.jf.live;
                live.pinging = false;
                // Back online after a failed save: send the position again.
                if ok && live.error.is_some() && live.online == Some(false) {
                    live.sent = None;
                }
                live.online = Some(ok);
            }
            JfMsg::Closing => {
                let live = &mut self.jf.live;
                if let Some((rep, _)) = live.pending.take() {
                    if live.sent.as_ref() != Some(&rep.id())
                        && !jellyfin::save_progress_now(&rep.account, &rep.item_id, rep.ticks, rep.played)
                    {
                        if let Some(db) = &self.db {
                            library::set_pending_progress(db, &rep.account.item_key(&rep.item_id), rep.ticks, rep.played);
                        }
                    }
                }
                if let Some((account, item_id, _)) = self.jf.reading.take() {
                    jellyfin::report_reading_stopped_now(&account, &item_id);
                    jellyfin::remember_open(None);
                }
            }
            JfMsg::SessionReported(result) => {
                if let Err(e) = result {
                    log::warn!("jellyfin: reading session report: {e}");
                }
            }
        }
        Task::none()
    }

    /// Where the open comic or book comes from, for the reader's title bar:
    /// the server's name, whether it's reachable (`None` until checked) and
    /// a hover text with the details. `None` when it isn't from Jellyfin.
    pub(super) fn jellyfin_source(&self) -> Option<(String, Option<bool>, String)> {
        let open = self.open_path.as_ref()?;
        let live = &self.jf.live;
        let (account, item) = if let Some((_, a)) = self.jf.book.as_ref().filter(|(p, _)| p == open) {
            (a, "This book: downloaded to the cache on this computer")
        } else {
            let r = self.remote.as_ref().filter(|r| open.to_string_lossy() == r.series_key)?;
            let item = match self.sources.get(self.current_page) {
                Some(PageSource::Remote { .. } | PageSource::RemoteRar { .. }) => "This chapter: streamed from the server, not cached",
                Some(_) => "This chapter: downloaded to the cache on this computer",
                None => "This chapter: loading…",
            };
            (&r.account, item)
        };

        let connection = match live.online {
            Some(true) => "Online",
            Some(false) => "Offline: the server isn't answering",
            None => "Checking the connection…",
        };
        let progress = if live.saving > 0 {
            "Progress: saving…".to_string()
        } else if let Some(at) = live.saved_at {
            let secs = at.elapsed().as_secs();
            let ago = match secs {
                0..=59 => "just now".to_string(),
                60..=3599 => format!("{} min ago", secs / 60),
                _ => format!("{} h ago", secs / 3600),
            };
            format!("Progress: saved to the server {ago}")
        } else {
            "Progress: saved to the server after a few seconds on a page".to_string()
        };
        let mut tip = format!(
            "Jellyfin: {} ({})\nSigned in as {}\n{connection}\n{progress}\n{item}",
            account.server_name, account.url, account.user_name
        );
        let mut label = account.server_name.clone();
        if let Some(e) = &live.error {
            tip = format!("⚠ {e}\n\n{tip}");
            label.push_str(" ⚠");
        }
        Some((label, live.online, tip))
    }

    /// Stores a fetched Jellyfin library: series, chapters, read state from
    /// the server (when newer than ours) and covers.
    fn apply_jellyfin_library(&mut self, account_key: &str, series: Vec<RemoteSeries>) -> Task<Message> {
        let Some(account) = self.jellyfin.find(account_key).cloned() else { return Task::none() };
        let Some(db) = &self.db else { return Task::none() };
        let mut tasks = Vec::new();
        let mut by_root: HashMap<String, Vec<String>> = HashMap::new();
        // Everything below is written as one commit, not one per row.
        let _ = db.execute_batch("SAVEPOINT batch");

        for s in &series {
            let clean = if naming::starts_with_chapter(&s.title) {
                let issue = naming::parse_issue(&s.title);
                naming::CleanName { display: naming::chapter_title(&issue), ..Default::default() }
            } else {
                naming::clean_series_name(&s.title)
            };
            let _ = library::upsert_scanned_series(
                db,
                &ScannedSeries {
                    path: s.key.clone(),
                    title: if clean.display.is_empty() { s.title.clone() } else { clean.display },
                    year: clean.year,
                    volume: clean.volume,
                    kind: if s.is_folder { SeriesKind::Dir } else { SeriesKind::File },
                    library_root: s.library_root.clone(),
                    total_pages: None,
                    // The main credits across its chapters.
                    creators: library::format_credits(
                        s.issues
                            .iter()
                            .flat_map(|i| i.creators.lines())
                            .filter_map(|line| line.split_once(": "))
                            .filter(|(role, _)| matches!(*role, "Author" | "Writer" | "Artist" | "Penciller")),
                    ),
                },
            );
            let rows: Vec<IssueRow> = s
                .issues
                .iter()
                .map(|i| {
                    let parsed = naming::parse_issue(&i.name);
                    IssueRow {
                        path: i.key.clone(),
                        name: i.name.clone(),
                        number: parsed.number.unwrap_or_default(),
                        subtitle: parsed.subtitle.unwrap_or_default(),
                        page_count: None,
                        details: Some(library::IssueDetails {
                            release_date: i.release_date.clone(),
                            creators: i.creators.clone(),
                        }),
                    }
                })
                .collect();
            let _ = library::sync_issues(db, &s.key, &rows);
            for i in &s.issues {
                self.jf.extensions.insert(i.item_id.clone(), i.extension.clone());
                let _ = library::apply_remote_progress(db, &s.key, &i.key, i.page, i.played, i.last_read);
            }
            by_root.entry(s.library_root.clone()).or_default().push(s.key.clone());
            self.jf
                .library_names
                .insert(s.library_root.clone(), format!("{} · {}", s.library_name, account.server_name));

            if let Some(item) = &s.cover_item {
                let path = library::chapter_cover_path(&s.key, 0);
                if !path.exists() {
                    tasks.push(cover_task(account.clone(), item.clone(), path, Message::Jf(JfMsg::CoverSaved)));
                }
            }
        }
        // Drop series that are gone from the server, and whole libraries
        // that were deleted or hidden there.
        for (root, keys) in &by_root {
            library::prune_root(db, root, keys);
        }
        let roots: HashSet<&str> = by_root.keys().map(String::as_str).collect();
        library::prune_jellyfin_roots(db, &format!("{}{account_key}/", jellyfin::KEY_PREFIX), &roots);

        // Progress kept while the server was offline goes up now, unless
        // the server has something newer (read elsewhere meanwhile).
        let server_read: HashMap<&str, i64> =
            series.iter().flat_map(|s| s.issues.iter().map(|i| (i.key.as_str(), i.last_read))).collect();
        for (item_key, ticks, played, at) in library::pending_progress(db, &format!("{}{account_key}/", jellyfin::KEY_PREFIX)) {
            match server_read.get(item_key.as_str()) {
                Some(&read) if read < at => tasks.push(pending_task(account.clone(), item_key, ticks, played, at)),
                _ => library::clear_pending_progress(db, &item_key, at),
            }
        }
        let _ = db.execute_batch("RELEASE batch");
        let names: Vec<String> =
            series.iter().filter(|s| library::get_series(db, &s.key).is_some_and(|e| e.meta_fetched_at == 0)).map(|s| s.key.clone()).collect();
        for key in names {
            tasks.push(self.enqueue_meta(key));
        }
        // Downloads of chapters the server no longer has.
        self.prune_downloads();
        self.jf.status = None;
        tasks.push(self.refresh_views());
        Task::batch(tasks)
    }

    /// Settings: signed-in servers and adding one.
    pub(super) fn build_jellyfin_section(&self) -> Element<'_, Message> {
        let mut section = widget::settings::section().header(
            widget::row::with_capacity(2)
                .spacing(8)
                .align_y(Alignment::Center)
                .push(widget::icon(super::library_view::jellyfin_icon()).size(16))
                .push(widget::text::heading("Jellyfin")),
        );

        for account in &self.jellyfin.accounts {
            let key = account.key();
            let syncing = self.jf.syncing.contains(&key);
            section = section.add(
                widget::Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(
                        widget::Column::new()
                            .width(Length::Fill)
                            .push(widget::text::body(account.label()))
                            .push(widget::text::caption(if syncing {
                                "Syncing…".to_string()
                            } else if self.jf.offline.contains(&key) {
                                format!("{}  ·  can't be reached, showing downloads", account.url)
                            } else {
                                account.url.clone()
                            })),
                    )
                    .push(
                        widget::button::icon(icon::from_name("view-refresh-symbolic"))
                            .tooltip("Sync now")
                            .on_press_maybe((!syncing).then(|| Message::Jf(JfMsg::Sync(key.clone())))),
                    )
                    .push(widget::button::standard("Sign out").on_press(Message::Jf(JfMsg::SignOut(key)))),
            );
        }

        if !self.jellyfin.accounts.is_empty() {
            section = section.add(widget::settings::item(
                "Ask before downloading big chapters that can't be streamed",
                widget::toggler(self.settings.ask_before_big_fetch).on_toggle(|on| Message::Jf(JfMsg::SetAskBigFetch(on))),
            ));
            section = section.add(self.downloads_settings());
        }

        if let Some(p) = &self.jf.pending {
            section = section.add(
                widget::Column::new()
                    .spacing(8)
                    .push(widget::text::body(format!("Approve this code in Jellyfin on {}:", p.server_name)))
                    .push(widget::text::title2(p.code.clone()))
                    .push(widget::text::caption(
                        "In Jellyfin, open your profile, then Quick Connect, and enter the code. \
                         This window signs in by itself once you've approved it.",
                    ))
                    .push(
                        widget::Row::new()
                            .spacing(8)
                            .push(widget::button::suggested("Open Jellyfin").on_press(Message::Jf(JfMsg::OpenApprovePage)))
                            .push(widget::button::standard("Cancel").on_press(Message::Jf(JfMsg::Cancel))),
                    ),
            );
        } else if let Some((_, server)) = &self.jf.password_for {
            section = section.add(
                widget::Column::new()
                    .spacing(8)
                    .push(widget::text::body(format!("Sign in to {server}")))
                    .push(
                        widget::text_input("Username", &self.jf.user_input)
                            .on_input(|s| Message::Jf(JfMsg::UserInput(s))),
                    )
                    .push(
                        widget::secure_input("Password", &self.jf.password_input, None, true)
                            .on_input(|s| Message::Jf(JfMsg::PasswordInput(s)))
                            .on_submit(|_| Message::Jf(JfMsg::PasswordSubmit)),
                    )
                    .push(
                        widget::Row::new()
                            .spacing(8)
                            .push(
                                widget::button::suggested("Sign in")
                                    .on_press_maybe((!self.jf.busy).then_some(Message::Jf(JfMsg::PasswordSubmit))),
                            )
                            .push(widget::button::standard("Cancel").on_press(Message::Jf(JfMsg::Cancel))),
                    ),
            );
        } else {
            section = section.add(
                widget::Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(
                        widget::text_input("Jellyfin address", &self.jf.url_input)
                            .on_input(|s| Message::Jf(JfMsg::UrlInput(s)))
                            .on_submit(|_| Message::Jf(JfMsg::Connect))
                            .width(Length::Fill),
                    )
                    .push(
                        widget::button::suggested("Connect")
                            .on_press_maybe((!self.jf.busy).then_some(Message::Jf(JfMsg::Connect))),
                    ),
            );
        }
        if let Some(status) = &self.jf.status {
            section = section.add(widget::text::caption(status.clone()));
        }
        section.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads a CBR from `tools/mock-jellyfin.py` the way the reader does:
    /// `NEBULA_MOCK_ITEM=<item id> cargo test loads_a_cbr_from_the_mock --
    /// --ignored --nocapture` with the mock on port 8097.
    #[test]
    #[ignore]
    fn loads_a_cbr_from_the_mock() {
        let item = std::env::var("NEBULA_MOCK_ITEM").expect("set NEBULA_MOCK_ITEM");
        let account = Account {
            url: "http://localhost:8097".into(),
            server_id: "mockserver".into(),
            server_name: "mockfin".into(),
            user_id: "mockuser".into(),
            user_name: "tester".into(),
            token: "mock-token".into(),
        };
        let key = account.item_key(&item);
        let extensions = HashMap::from([(item.clone(), "cbr".to_string())]);
        let t = Instant::now();
        let loaded = load_issues(&account, "series", &[(key, "Chapter".into())], &extensions, &HashMap::new(), Some(BIG_FETCH)).unwrap();
        println!("{} pages in {:?}, blocked: {}", loaded.sources.len(), t.elapsed(), loaded.blocked.is_some());
        let t = Instant::now();
        let page = comic::decode_source(&loaded.sources[0]).unwrap();
        println!("page 1: {}x{} in {:?}", page.width, page.height, t.elapsed());
        // Something that can't be read in parts: asked about, not fetched.
        if let Ok(junk) = std::env::var("NEBULA_MOCK_JUNK") {
            let key = account.item_key(&junk);
            let extensions = HashMap::from([(junk, "cbr".to_string())]);
            let loaded = load_issues(&account, "series", &[(key, "Junk".into())], &extensions, &HashMap::new(), Some(BIG_FETCH)).unwrap();
            let b = loaded.blocked.expect("should ask first");
            println!("asked first: {} bytes, nothing fetched: {}", b.size, loaded.sources.is_empty());
        }
    }

    /// Times opening a batch of CBZ chapters the way the reader does:
    /// `NEBULA_MOCK_CBZS=<id>,<id>,… cargo test opens_cbz_chapters_from_the_mock --
    /// --ignored --nocapture` with the mock on port 8097.
    #[test]
    #[ignore]
    fn opens_cbz_chapters_from_the_mock() {
        let ids = std::env::var("NEBULA_MOCK_CBZS").expect("set NEBULA_MOCK_CBZS");
        let account = Account {
            url: "http://localhost:8097".into(),
            server_id: "mockserver".into(),
            server_name: "mockfin".into(),
            user_id: "mockuser".into(),
            user_name: "tester".into(),
            token: "mock-token".into(),
        };
        let issues: Vec<(String, String)> = ids.split(',').map(|id| (account.item_key(id), format!("Chapter {id}"))).collect();
        let extensions: HashMap<String, String> = ids.split(',').map(|id| (id.to_string(), "cbz".to_string())).collect();
        let t = Instant::now();
        let loaded = load_issues(&account, "series", &issues, &extensions, &HashMap::new(), None).unwrap();
        println!("{} chapters, {} pages in {:?}", loaded.chapters.len(), loaded.sources.len(), t.elapsed());
        assert_eq!(loaded.chapters.len(), issues.len());
    }
}
