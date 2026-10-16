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
use crate::remote_zip::RemoteZip;

/// Chapters loaded after the one being opened, and per top-up after that.
const WINDOW_AHEAD: usize = 3;
/// Start loading the next chapters when this close to the last loaded page.
const TOP_UP_PAGES: usize = 40;
/// Quick Connect codes expire on the server after a few minutes.
const QUICK_CONNECT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

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
    MoreLoaded(String, Result<(Vec<PageSource>, Vec<ChapterInfo>), String>),
    EpubDownloaded(Result<PathBuf, String>),
    Saved(Result<(), String>),
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
    pub url_input: String,
    pub status: Option<String>,
    pub busy: bool,
    pending: Option<PendingSignIn>,
    /// Server (url, name) asking for username/password (Quick Connect off).
    password_for: Option<(String, String)>,
    user_input: String,
    password_input: String,
    pub syncing: HashSet<String>,
    /// Item id -> file extension, from the latest sync.
    extensions: HashMap<String, String>,
    /// Library root key -> "Manga · deepdish", for section headings.
    pub library_names: HashMap<String, String>,
}

/// The Jellyfin series open in the reader.
pub struct RemoteReading {
    account: Account,
    series_key: String,
    /// Issue keys in reading order.
    issues: Vec<(String, String)>,
    /// Index of the next issue not yet loaded.
    next: usize,
    loading: bool,
    last_saved: Option<(String, usize)>,
}

// ── Background work ─────────────────────────────────────────────────────────

fn fetch_task(account: Account) -> Task<Message> {
    let key = account.key();
    cosmic::task::future(async move {
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

/// Lists the pages of some issues: streamed for zip archives, downloaded
/// to the cache first for anything else (RAR can't be read out of order).
fn load_issues(
    account: &Account,
    issues: &[(String, String)],
    extensions: &HashMap<String, String>,
) -> Result<(Vec<PageSource>, Vec<ChapterInfo>), String> {
    let mut sources = Vec::new();
    let mut chapters = Vec::new();
    for (key, name) in issues {
        let item_id = jellyfin::parse_key(key).map(|(_, id)| id).unwrap_or_default();
        let ext = extensions.get(item_id).map(String::as_str).unwrap_or("");
        let start = sources.len();

        let streamed = if matches!(ext, "" | "cbz" | "zip") {
            RemoteZip::open(Box::new(account.range_source(item_id)))
                .ok()
                .filter(|z| !z.entries.is_empty())
        } else {
            None
        };
        if let Some(zip) = streamed {
            sources.extend((0..zip.entries.len()).map(|index| PageSource::Remote { archive: zip.clone(), index }));
        } else {
            let local = download(account, item_id, if ext.is_empty() { "cbz" } else { ext })?;
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
    Ok((sources, chapters))
}

/// Downloads an item into the cache (kept for re-reads; the cache is safe
/// to clear).
fn download(account: &Account, item_id: &str, ext: &str) -> Result<PathBuf, String> {
    let path = library::cache_dir().join("jellyfin").join(format!("{item_id}.{ext}"));
    if !path.exists() {
        account.range_source(item_id).download_to(&path)?;
    }
    Ok(path)
}

fn load_task(account: Account, series_key: String, issues: Vec<(String, String)>, extensions: HashMap<String, String>, more: bool) -> Task<Message> {
    cosmic::task::future(async move {
        let key = series_key.clone();
        let result = tokio::task::spawn_blocking(move || load_issues(&account, &issues, &extensions))
            .await
            .unwrap_or_else(|e| Err(format!("loading panicked: {e}")));
        if more {
            Message::Jf(JfMsg::MoreLoaded(key, result))
        } else {
            match result {
                Ok((sources, chapters)) => Message::Loaded(PathBuf::from(key), sources, chapters),
                Err(e) => Message::LoadFailed(e),
            }
        }
    })
}

fn save_task(account: Account, item_id: String, page: usize, played: bool) -> Task<Message> {
    cosmic::task::future(async move {
        Message::Jf(JfMsg::Saved(jellyfin::save_progress(&account, &item_id, page, played).await))
    })
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
        Task::batch(accounts.into_iter().map(|a| {
            self.jf.syncing.insert(a.key());
            fetch_task(a)
        }))
    }

    fn account_for(&self, key: &str) -> Option<Account> {
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

        // EPUBs open in the book reader from a downloaded copy.
        let first_id = jellyfin::parse_key(&issues[0].0).map(|(_, id)| id.to_string()).unwrap_or_default();
        if issues.len() == 1 && self.jf.extensions.get(&first_id).is_some_and(|e| e == "epub") {
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
        let start = start.min(issues.len() - 1);
        let first = start.saturating_sub(1);
        let end = (start + 1 + WINDOW_AHEAD).min(issues.len());
        self.start_chapter = Some(start - first);
        self.remote = Some(RemoteReading {
            account: account.clone(),
            series_key: key.clone(),
            issues: issues.clone(),
            next: end,
            loading: false,
            last_saved: None,
        });
        load_task(account, key, issues[first..end].to_vec(), self.jf.extensions.clone(), false)
    }

    /// While reading a Jellyfin series: load more chapters before the
    /// reader runs out, and save progress to the server on page changes.
    pub(super) fn jellyfin_tick(&mut self) -> Task<Message> {
        if self.app_view != AppView::Reader || self.loading {
            return Task::none();
        }
        let open = self.open_path.as_ref().map(|p| p.to_string_lossy().into_owned());
        let visible = self.current_chapter().map(|(_, ch, pg)| {
            let pg = if self.layout == super::Layout::Dual { pg + 1 } else { pg };
            let pg = pg.min(ch.page_count.saturating_sub(1));
            (ch.path.to_string_lossy().into_owned(), pg, pg + 1 >= ch.page_count)
        });
        let near_end = self.current_page + TOP_UP_PAGES >= self.sources.len();
        let extensions = self.jf.extensions.clone();
        let Some(r) = self.remote.as_mut().filter(|r| open.as_deref() == Some(r.series_key.as_str())) else {
            return Task::none();
        };

        let mut tasks = Vec::new();
        if near_end && !r.loading && r.next < r.issues.len() {
            r.loading = true;
            let end = (r.next + WINDOW_AHEAD).min(r.issues.len());
            tasks.push(load_task(r.account.clone(), r.series_key.clone(), r.issues[r.next..end].to_vec(), extensions, true));
        }
        if let Some((issue, page, finished)) = visible {
            if r.last_saved.as_ref() != Some(&(issue.clone(), page)) {
                r.last_saved = Some((issue.clone(), page));
                if let Some((_, item_id)) = jellyfin::parse_key(&issue) {
                    tasks.push(save_task(r.account.clone(), item_id.to_string(), page, finished));
                }
            }
        }
        Task::batch(tasks)
    }

    /// Mirrors read/unread marks from the series page to Jellyfin.
    pub(super) fn jellyfin_mark(&self, issue_keys: Vec<String>, played: bool) -> Task<Message> {
        let Some(account) = issue_keys.first().and_then(|k| self.account_for(k)) else { return Task::none() };
        cosmic::task::future(async move {
            let mut result = Ok(());
            for key in issue_keys {
                if let Some((_, id)) = jellyfin::parse_key(&key) {
                    if let Err(e) = jellyfin::save_progress(&account, id, 0, played).await {
                        result = Err(e);
                    }
                }
            }
            Message::Jf(JfMsg::Saved(result))
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
                        return fetch_task(account);
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
                self.jf.status = Some(format!("Signed out of {}.", account.server_name));
                let refresh = self.refresh_views();
                return Task::batch([
                    cosmic::task::future(async move {
                        jellyfin::sign_out(&account).await;
                        Message::Jf(JfMsg::Saved(Ok(())))
                    }),
                    refresh,
                ]);
            }
            JfMsg::Sync(key) => {
                if let Some(account) = self.jellyfin.find(&key).cloned() {
                    self.jf.syncing.insert(key);
                    return fetch_task(account);
                }
            }
            JfMsg::Fetched(key, result) => {
                self.jf.syncing.remove(&key);
                match result {
                    Ok(series) => return self.apply_jellyfin_library(&key, series),
                    Err(e) => {
                        let server = self.jellyfin.find(&key).map(|a| a.server_name.clone()).unwrap_or_default();
                        self.jf.status = Some(format!("Couldn't load {server}: {e}"));
                    }
                }
            }
            JfMsg::CoverSaved => return self.reload_library(),
            JfMsg::MoreLoaded(key, result) => {
                let Some(r) = self.remote.as_mut().filter(|r| r.series_key == key) else { return Task::none() };
                r.loading = false;
                match result {
                    Ok((sources, chapters)) => {
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
                    Err(e) => log::warn!("jellyfin: loading more chapters: {e}"),
                }
            }
            JfMsg::EpubDownloaded(result) => match result {
                Ok(path) => return super::epub_open_task(path),
                Err(e) => return Task::done(cosmic::Action::App(Message::LoadFailed(e))),
            },
            JfMsg::Saved(result) => {
                if let Err(e) = result {
                    log::warn!("jellyfin: saving progress: {e}");
                }
            }
        }
        Task::none()
    }

    /// Stores a fetched Jellyfin library: series, chapters, read state from
    /// the server (when newer than ours) and covers.
    fn apply_jellyfin_library(&mut self, account_key: &str, series: Vec<RemoteSeries>) -> Task<Message> {
        let Some(account) = self.jellyfin.find(account_key).cloned() else { return Task::none() };
        let Some(db) = &self.db else { return Task::none() };
        let mut tasks = Vec::new();
        let mut by_root: HashMap<String, Vec<String>> = HashMap::new();

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
        // Drop series that are gone from the server.
        for (root, keys) in &by_root {
            library::prune_root(db, root, keys);
        }
        let names: Vec<String> =
            series.iter().filter(|s| library::get_series(db, &s.key).is_some_and(|e| e.meta_fetched_at == 0)).map(|s| s.key.clone()).collect();
        for key in names {
            tasks.push(self.enqueue_meta(key));
        }
        self.jf.status = None;
        tasks.push(self.refresh_views());
        Task::batch(tasks)
    }

    /// Settings: signed-in servers and adding one.
    pub(super) fn build_jellyfin_section(&self) -> Element<'_, Message> {
        let mut section = widget::settings::section().title("Jellyfin");

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
                            .push(widget::text::caption(if syncing { "Syncing…".to_string() } else { account.url.clone() })),
                    )
                    .push(
                        widget::button::icon(icon::from_name("view-refresh-symbolic"))
                            .tooltip("Sync now")
                            .on_press_maybe((!syncing).then(|| Message::Jf(JfMsg::Sync(key.clone())))),
                    )
                    .push(widget::button::standard("Sign out").on_press(Message::Jf(JfMsg::SignOut(key)))),
            );
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
