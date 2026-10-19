//! Jellyfin downloads: chapters and books kept on this computer so they
//! can be read when the server can't be reached.
//!
//! Downloads stay Jellyfin items (their library keys don't change), so
//! they show under the Jellyfin libraries, keep syncing progress with the
//! server, and are only listed on their own while their server is offline.
//! Files live in the download folder (the data folder, or one the user
//! picks, which may be on a USB drive), one at a time is downloaded, and
//! the `downloads` table remembers what's there.
//!
//! A download folder on a removable drive is kept safe by: never creating
//! the chosen folder itself (an unplugged drive's empty mount point stays
//! empty), writing each file as `.part` and only renaming it once it's
//! flushed to the drive, checking free space first, and keeping the
//! database in the data folder. Downloads on a drive that's not connected
//! stay recorded (shown as not connected) and are never forgotten or
//! "removed" while their files can't be reached.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cosmic::app::Task;
use cosmic::ApplicationExt;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, icon};
use cosmic::Element;

use super::{App, Message};
use crate::jellyfin;
use crate::library;

#[derive(Clone, Debug)]
pub enum DlMsg {
    /// Queue these (issue key, series key) for download.
    Download(Vec<(String, String)>),
    /// Remove these issues' downloads (or stop them if still running).
    Remove(Vec<String>),
    Finished(String, String, Result<(PathBuf, u64), String>),
    /// Redraw the progress while something is downloading.
    Tick,
    OpenFolder,
    RemoveAll,
    /// The Downloads page.
    OpenManager,
    DismissError,
    ToggleGroup(String),
    ChooseFolder,
    FolderChosen(url::Url),
    /// Back to the data folder.
    UseDefaultFolder,
    /// Moving finished: (issue key, new file) moved, and errors.
    Moved(Vec<(String, PathBuf)>, Vec<String>),
}

/// What one chapter's download is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DlState {
    None,
    Queued,
    Running(f32),
    Done,
    /// Downloaded to a drive that isn't connected.
    Unavailable,
}

/// The Downloads page's list: one group per series.
#[derive(Clone, Debug)]
pub struct DlGroup {
    pub series: String,
    pub name: String,
    pub items: Vec<DlItem>,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct DlItem {
    pub key: String,
    pub title: String,
    pub size: u64,
    pub present: bool,
}

#[derive(Default)]
pub struct Downloads {
    /// Issue key -> download, from the database.
    done: HashMap<String, library::Download>,
    /// Series with at least one download.
    series: HashSet<String>,
    /// Waiting: (issue key, series key).
    queue: VecDeque<(String, String)>,
    /// The one downloading now: issue key, series key and its progress.
    current: Option<(String, String, Arc<jellyfin::Progress>)>,
    /// Finished in this run of downloads, for the progress line.
    finished: usize,
    pub error: Option<String>,
    /// Downloads whose file is there (its drive is connected).
    present: HashSet<String>,
    /// For the Downloads page.
    groups: Vec<DlGroup>,
    expanded: HashSet<String>,
    /// Moving the downloads to a new folder.
    moving: bool,
}

impl Downloads {
    pub fn load(db: &rusqlite::Connection) -> Self {
        let mut d = Downloads::default();
        d.reload(db);
        d
    }

    fn reload(&mut self, db: &rusqlite::Connection) {
        self.done = library::downloads(db).into_iter().map(|d| (d.path.clone(), d)).collect();
        self.series = self.done.values().map(|d| d.series_path.clone()).collect();
        self.present = self.done.iter().filter(|(_, d)| d.file.is_file()).map(|(k, _)| k.clone()).collect();
        self.groups = groups(db, &self.done, &self.present);
    }

    /// The downloaded file for an issue key, if it's still there.
    pub fn file(&self, issue_key: &str) -> Option<&PathBuf> {
        self.done.get(issue_key).map(|d| &d.file).filter(|f| f.exists())
    }

    /// Issue key -> file, for the loader.
    pub fn files(&self) -> HashMap<String, PathBuf> {
        self.done.iter().map(|(k, d)| (k.clone(), d.file.clone())).collect()
    }

    pub fn has_series(&self, series_key: &str) -> bool {
        self.series.contains(series_key)
    }

    pub fn state(&self, issue_key: &str) -> DlState {
        match &self.current {
            Some((k, _, p)) if k == issue_key => DlState::Running(p.fraction()),
            _ if self.present.contains(issue_key) => DlState::Done,
            _ if self.done.contains_key(issue_key) => DlState::Unavailable,
            _ if self.queue.iter().any(|(k, _)| k == issue_key) => DlState::Queued,
            _ => DlState::None,
        }
    }

    pub fn busy(&self) -> bool {
        self.current.is_some()
    }

    /// (finished, total, fraction of the current one) for this run.
    pub fn run_progress(&self) -> Option<(usize, usize, f32)> {
        let (_, _, p) = self.current.as_ref()?;
        Some((self.finished, self.finished + 1 + self.queue.len(), p.fraction()))
    }

    /// How many are downloaded, and their total size in bytes.
    pub fn totals(&self) -> (usize, u64) {
        (self.done.len(), self.done.values().map(|d| d.size).sum())
    }

    pub fn is_empty(&self) -> bool {
        self.done.is_empty() && self.current.is_none() && self.queue.is_empty()
    }
}

/// Groups downloads by series, named as the library shows them.
fn groups(db: &rusqlite::Connection, done: &HashMap<String, library::Download>, present: &HashSet<String>) -> Vec<DlGroup> {
    let mut by_series: HashMap<&str, Vec<&library::Download>> = HashMap::new();
    for d in done.values() {
        by_series.entry(d.series_path.as_str()).or_default().push(d);
    }
    let mut out: Vec<DlGroup> = by_series
        .into_iter()
        .map(|(series, downloads)| {
            let entry = library::get_series(db, series);
            let name = entry.as_ref().map(|e| e.name().to_string()).unwrap_or_else(|| "No longer in the library".into());
            let issues = library::issues_for(db, series);
            let manga = super::library_view::is_manga(&issues);
            let mut items: Vec<(usize, DlItem)> = downloads
                .into_iter()
                .map(|d| {
                    let issue = issues.iter().find(|i| i.path == d.path);
                    let title = match issue {
                        Some(_) if d.path == series => name.clone(),
                        Some(i) => super::library_view::issue_title(i, manga),
                        None => d.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    };
                    let order = issue.map(|i| i.idx).unwrap_or(usize::MAX);
                    (order, DlItem { key: d.path.clone(), title, size: d.size, present: present.contains(&d.path) })
                })
                .collect();
            items.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.title.cmp(&b.1.title)));
            let items: Vec<DlItem> = items.into_iter().map(|(_, i)| i).collect();
            DlGroup { series: series.to_string(), name, size: items.iter().map(|i| i.size).sum(), items }
        })
        .collect();
    out.sort_by_key(|g| g.name.to_lowercase());
    out
}

/// `1.4 GB`, `230 MB`.
pub fn human_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let mb = bytes as f64 / MB;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else if mb >= 10.0 {
        format!("{mb:.0} MB")
    } else {
        format!("{mb:.1} MB")
    }
}

impl App {
    pub(super) fn update_downloads(&mut self, msg: DlMsg) -> Task<Message> {
        match msg {
            DlMsg::Download(items) => {
                self.dl.error = None;
                for (issue, series) in items {
                    if self.dl.state(&issue) == DlState::None {
                        self.dl.queue.push_back((issue, series));
                    }
                }
                return self.pump_downloads();
            }
            DlMsg::Remove(issues) => {
                let Some(db) = &self.db else { return Task::none() };
                let mut unreachable = 0;
                for issue in issues {
                    self.dl.queue.retain(|(k, _)| *k != issue);
                    if let Some((k, _, p)) = &self.dl.current {
                        if *k == issue {
                            p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    // A file on a drive that isn't connected can't be
                    // deleted; keep it recorded so it can be later.
                    if self.dl.done.contains_key(&issue) && !self.dl.present.contains(&issue) {
                        unreachable += 1;
                        continue;
                    }
                    if let Some(file) = library::remove_download(db, &issue) {
                        let _ = std::fs::remove_file(file);
                    }
                }
                if unreachable > 0 {
                    self.dl.error = Some(format!(
                        "{unreachable} {} on a drive that isn't connected. Connect it to remove {}.",
                        if unreachable == 1 { "download is" } else { "downloads are" },
                        if unreachable == 1 { "it" } else { "them" },
                    ));
                }
                self.dl.reload(db);
                return self.refresh_views();
            }
            DlMsg::Finished(issue, series, result) => {
                self.dl.current = None;
                match result {
                    Ok((file, size)) => {
                        if let Some(db) = &self.db {
                            let _ = library::add_download(
                                db,
                                &library::Download { path: issue, series_path: series, file, size },
                            );
                            self.dl.reload(db);
                        }
                        self.dl.finished += 1;
                    }
                    Err(e) if e == "cancelled" => {}
                    Err(e) => {
                        log::warn!("jellyfin download of {issue}: {e}");
                        self.dl.error = Some(e);
                    }
                }
                let next = self.pump_downloads();
                return Task::batch([next, self.refresh_views()]);
            }
            DlMsg::Tick => {
                // On the Downloads page, notice a drive coming or going.
                if !self.dl.busy() && self.app_view == super::AppView::Downloads {
                    if let Some(db) = &self.db {
                        self.dl.reload(db);
                    }
                }
            }
            DlMsg::OpenFolder => match self.download_root() {
                Ok(dir) => crate::platform::open_url(&dir.to_string_lossy()),
                Err(e) => self.dl.error = Some(e),
            },
            DlMsg::RemoveAll => {
                let all: Vec<String> = self.dl.present.iter().cloned().collect();
                self.dl.queue.clear();
                if let Some((_, _, p)) = &self.dl.current {
                    p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                return self.update_downloads(DlMsg::Remove(all));
            }
            DlMsg::OpenManager => {
                if let Some(db) = &self.db {
                    self.dl.reload(db);
                }
                self.set_show_context(false);
                self.series_page = None;
                self.app_view = super::AppView::Downloads;
                self.set_header_title("Downloads".into());
            }
            DlMsg::DismissError => self.dl.error = None,
            DlMsg::ToggleGroup(series) => {
                if !self.dl.expanded.remove(&series) {
                    self.dl.expanded.insert(series);
                }
            }
            DlMsg::ChooseFolder => {
                return cosmic::task::future(async move {
                    let dialog = cosmic::dialog::file_chooser::open::Dialog::new().title("Download Folder");
                    match dialog.open_folder().await {
                        Ok(r) => Message::Dl(DlMsg::FolderChosen(r.url().to_owned())),
                        Err(_) => Message::Dl(DlMsg::Tick),
                    }
                });
            }
            DlMsg::FolderChosen(url) => match url.to_file_path() {
                Ok(path) => return self.set_download_folder(Some(path)),
                Err(()) => self.dl.error = Some(format!("not a local folder: {url}")),
            },
            DlMsg::UseDefaultFolder => return self.set_download_folder(None),
            DlMsg::Moved(moved, errors) => {
                self.dl.moving = false;
                if let Some(db) = &self.db {
                    for (key, new_file) in &moved {
                        let Some(old) = self.dl.done.get(key).cloned() else { continue };
                        let row = library::Download { file: new_file.clone(), ..old.clone() };
                        // The copy is recorded before the original goes, so
                        // a crash in between leaves two copies, not none.
                        if library::add_download(db, &row).is_ok() && old.file != *new_file {
                            let _ = std::fs::remove_file(&old.file);
                            // Tidy the old folders if that emptied them.
                            if let Some(account_dir) = old.file.parent() {
                                let _ = std::fs::remove_dir(account_dir);
                                if let Some(root) = account_dir.parent() {
                                    let _ = std::fs::remove_dir(root);
                                }
                            }
                        }
                    }
                    self.dl.reload(db);
                }
                if let Some(first) = errors.first() {
                    self.dl.error = Some(format!(
                        "{} {} couldn't be moved and stayed in the old folder: {first}",
                        errors.len(),
                        if errors.len() == 1 { "download" } else { "downloads" },
                    ));
                }
                return Task::batch([self.pump_downloads(), self.refresh_views()]);
            }
        }
        Task::none()
    }

    /// Where downloads go now, made ready: the data folder's, or a "Nebula
    /// Reader" folder inside the one the user picked. A picked folder that's
    /// gone (its drive unplugged) is an error, never re-created.
    pub(super) fn download_root(&self) -> Result<PathBuf, String> {
        let root = match &self.settings.download_dir {
            Some(dir) => {
                if !dir.is_dir() {
                    return Err(format!(
                        "The download folder {} isn't available. Connect its drive, or pick another folder on the Downloads page.",
                        dir.display()
                    ));
                }
                dir.join("Nebula Reader")
            }
            None => jellyfin::default_download_root(),
        };
        std::fs::create_dir_all(&root).map_err(|e| format!("can't use the download folder: {e}"))?;
        Ok(root)
    }

    /// The folder shown on the Downloads page, whether or not it's there.
    fn download_root_shown(&self) -> PathBuf {
        match &self.settings.download_dir {
            Some(dir) => dir.join("Nebula Reader"),
            None => jellyfin::default_download_root(),
        }
    }

    /// Switches the download folder and moves what's downloaded there.
    fn set_download_folder(&mut self, dir: Option<PathBuf>) -> Task<Message> {
        if self.dl.busy() || self.dl.moving || !self.dl.queue.is_empty() {
            self.dl.error = Some("Wait for the downloads to finish before changing the folder.".into());
            return Task::none();
        }
        if dir == self.settings.download_dir {
            return Task::none();
        }
        let root = match &dir {
            Some(d) => d.join("Nebula Reader"),
            None => jellyfin::default_download_root(),
        };
        if let Err(e) = std::fs::create_dir_all(&root)
            .map_err(|e| format!("can't use that folder: {e}"))
            .and_then(|()| crate::platform::disk::check_writable(&root))
        {
            self.dl.error = Some(e);
            return Task::none();
        }
        self.dl.error = None;
        self.settings.download_dir = dir;
        self.settings.save();

        // Move what can be reached; anything on a drive that isn't
        // connected stays where it is, still recorded.
        let jobs: Vec<(String, PathBuf, PathBuf)> = self
            .dl
            .done
            .iter()
            .filter(|(k, d)| self.dl.present.contains(*k) && !d.file.starts_with(&root))
            .filter_map(|(k, d)| {
                let account = d.file.parent()?.file_name()?;
                Some((k.clone(), d.file.clone(), root.join(account).join(d.file.file_name()?)))
            })
            .collect();
        if jobs.is_empty() {
            return Task::none();
        }
        self.dl.moving = true;
        cosmic::task::future(async move {
            let (moved, errors) = tokio::task::spawn_blocking(move || move_files(jobs)).await.unwrap_or_default();
            Message::Dl(DlMsg::Moved(moved, errors))
        })
    }

    /// Starts the next queued download, one at a time.
    fn pump_downloads(&mut self) -> Task<Message> {
        if self.dl.current.is_some() || self.dl.moving {
            return Task::none();
        }
        if self.dl.queue.is_empty() {
            self.dl.finished = 0;
            return Task::none();
        }
        let root = match self.download_root() {
            Ok(root) => root,
            Err(e) => {
                self.dl.error = Some(e);
                self.dl.queue.clear();
                self.dl.finished = 0;
                return Task::none();
            }
        };
        while let Some((issue, series)) = self.dl.queue.pop_front() {
            let Some((account_key, item_id)) = jellyfin::parse_key(&issue) else { continue };
            let Some(account) = self.jellyfin.find(account_key).cloned() else { continue };
            let ext = self.jf.extensions.get(item_id).filter(|e| !e.is_empty()).cloned().unwrap_or_else(|| "cbz".into());
            let dir = root.join(account_key);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                self.dl.error = Some(format!("can't use the download folder: {e}"));
                self.dl.queue.clear();
                break;
            }
            let dest = dir.join(format!("{item_id}.{ext}"));
            let item_id = item_id.to_string();
            let progress = Arc::new(jellyfin::Progress::default());
            self.dl.current = Some((issue.clone(), series.clone(), progress.clone()));
            return cosmic::task::future(async move {
                let result = jellyfin::download_item(&account, &item_id, &dest, &progress)
                    .await
                    .map(|size| (dest, size));
                Message::Dl(DlMsg::Finished(issue, series, result))
            });
        }
        self.dl.finished = 0;
        Task::none()
    }

    /// Redraws twice a second while downloading, for the progress rings.
    /// On the Downloads page, also checks every few seconds whether a
    /// drive was connected or removed.
    pub(super) fn downloads_subscription(&self) -> cosmic::iced::Subscription<Message> {
        if self.dl.busy() {
            cosmic::iced::time::every(std::time::Duration::from_millis(500)).map(|_| Message::Dl(DlMsg::Tick))
        } else if self.app_view == super::AppView::Downloads {
            cosmic::iced::time::every(std::time::Duration::from_secs(3)).map(|_| Message::Dl(DlMsg::Tick))
        } else {
            cosmic::iced::Subscription::none()
        }
    }

    /// Deletes downloads whose chapter left the library (removed on the
    /// server, or the account signed out). Ones on a drive that isn't
    /// connected wait until it is.
    pub(super) fn prune_downloads(&mut self) {
        let Some(db) = &self.db else { return };
        for d in library::orphaned_downloads(db) {
            if !d.file.parent().is_some_and(Path::is_dir) {
                continue;
            }
            let _ = std::fs::remove_file(&d.file);
            library::remove_download(db, &d.path);
        }
        self.dl.reload(db);
    }

    /// The download button for one chapter: download, progress (click to
    /// stop) or downloaded (click to remove).
    pub(super) fn download_button<'a>(
        &self,
        issue_key: &str,
        series_key: &str,
        hint_at: widget::tooltip::Position,
    ) -> Element<'a, Message> {
        let (button, hint): (Element<'a, Message>, String) = match self.dl.state(issue_key) {
            DlState::None => (
                widget::button::icon(icon::from_name("folder-download-symbolic"))
                    .on_press(Message::Dl(DlMsg::Download(vec![(issue_key.to_string(), series_key.to_string())])))
                    .into(),
                "Download for reading offline".into(),
            ),
            DlState::Queued => (
                widget::button::custom(widget::indeterminate_circular().size(16.0).bar_height(2.0))
                    .class(cosmic::theme::Button::Icon)
                    .padding(8)
                    .on_press(Message::Dl(DlMsg::Remove(vec![issue_key.to_string()])))
                    .into(),
                "Waiting to download (click to cancel)".into(),
            ),
            DlState::Running(f) => (
                widget::button::custom(widget::determinate_circular(f).size(16.0).bar_height(2.0))
                    .class(cosmic::theme::Button::Icon)
                    .padding(8)
                    .on_press(Message::Dl(DlMsg::Remove(vec![issue_key.to_string()])))
                    .into(),
                format!("Downloading… {:.0}% (click to cancel)", f * 100.0),
            ),
            DlState::Done => (
                widget::button::icon(icon::from_name("emblem-ok-symbolic"))
                    .on_press(Message::Dl(DlMsg::Remove(vec![issue_key.to_string()])))
                    .into(),
                "Downloaded (click to remove the download)".into(),
            ),
            DlState::Unavailable => (
                widget::button::icon(icon::from_name("drive-removable-media-symbolic")).into(),
                "Downloaded to a drive that isn't connected".into(),
            ),
        };
        widget::tooltip(button, widget::text(hint), hint_at).into()
    }

    /// A one-shot's page: one "Download" button that shows its progress.
    pub(super) fn one_shot_download_action<'a>(&self, issue_key: &str, series_key: &str) -> Element<'a, Message> {
        let remove = Message::Dl(DlMsg::Remove(vec![issue_key.to_string()]));
        let busy = |ring: Element<'a, Message>, label: String| -> Element<'a, Message> {
            widget::button::custom(
                widget::row::with_capacity(2).spacing(8).align_y(Alignment::Center).push(ring).push(widget::text::body(label)),
            )
            .class(cosmic::theme::Button::Standard)
            .on_press(remove.clone())
            .into()
        };
        match self.dl.state(issue_key) {
            DlState::None => widget::button::standard("Download")
                .leading_icon(icon::from_name("folder-download-symbolic"))
                .on_press(Message::Dl(DlMsg::Download(vec![(issue_key.to_string(), series_key.to_string())])))
                .into(),
            DlState::Queued => busy(widget::indeterminate_circular().size(14.0).bar_height(2.0).into(), "Waiting… (Cancel)".into()),
            DlState::Running(f) => busy(
                widget::determinate_circular(f).size(14.0).bar_height(2.0).into(),
                format!("Downloading {:.0}% (Cancel)", f * 100.0),
            ),
            DlState::Done => widget::button::standard("Remove Download").on_press(remove).into(),
            DlState::Unavailable => widget::tooltip(
                widget::button::standard("Downloaded").leading_icon(icon::from_name("drive-removable-media-symbolic")),
                widget::text("Downloaded to a drive that isn't connected"),
                widget::tooltip::Position::Bottom,
            )
            .into(),
        }
    }

    /// The series page's "Download All" / "Remove Downloads" buttons.
    pub(super) fn series_download_actions<'a>(&self, series_key: &str, issue_keys: &[String]) -> Vec<Element<'a, Message>> {
        let mut out = Vec::new();
        let missing: Vec<(String, String)> = issue_keys
            .iter()
            .filter(|k| self.dl.state(k) == DlState::None)
            .map(|k| (k.clone(), series_key.to_string()))
            .collect();
        let have: Vec<String> =
            issue_keys.iter().filter(|k| !matches!(self.dl.state(k), DlState::None | DlState::Unavailable)).cloned().collect();
        if !missing.is_empty() {
            let label = if have.is_empty() { "Download All".to_string() } else { format!("Download {} More", missing.len()) };
            out.push(widget::button::standard(label).on_press(Message::Dl(DlMsg::Download(missing))).into());
        }
        if !have.is_empty() {
            out.push(widget::button::standard("Remove Downloads").on_press(Message::Dl(DlMsg::Remove(have))).into());
        }
        out
    }

    /// "Downloading 3 of 12…" with a progress ring, for the library's
    /// status line.
    pub(super) fn download_status<'a>(&self) -> Option<Element<'a, Message>> {
        let (done, total, current) = self.dl.run_progress()?;
        let overall = (done as f32 + current) / total.max(1) as f32;
        Some(
            widget::row::with_capacity(2)
                .spacing(8)
                .align_y(Alignment::Center)
                .push(widget::determinate_circular(overall).size(14.0).bar_height(2.0))
                .push(widget::text::caption(format!("Downloading {} of {total}… {:.0}%", done + 1, current * 100.0)))
                .into(),
        )
    }

    /// Settings → Jellyfin: how much is downloaded, and the way to the
    /// Downloads page.
    pub(super) fn downloads_settings<'a>(&self) -> Element<'a, Message> {
        let (count, bytes) = self.dl.totals();
        let summary = if count == 0 {
            "No downloads yet. Use the download buttons on a Jellyfin series or book to read it offline.".to_string()
        } else {
            format!("Downloads: {count} {} · {}", if count == 1 { "item" } else { "items" }, human_size(bytes))
        };
        widget::column::with_capacity(2)
            .spacing(8)
            .push(widget::text::body(summary))
            .push(widget::button::standard("Manage Downloads").on_press(Message::Dl(DlMsg::OpenManager)))
            .width(Length::Fill)
            .into()
    }

    /// The Downloads page: where they're kept and how much room they take,
    /// what's downloading, and everything downloaded, by series.
    pub(super) fn view_downloads(&self) -> Element<'_, Message> {
        let mut col = widget::Column::new().spacing(16).padding([16, 20]);
        if let Some(err) = self.dl.error.as_deref() {
            col = col.push(super::error_banner(err));
        }

        let (count, bytes) = self.dl.totals();
        col = col.push(widget::text::title2("Downloads")).push(widget::text::body(if count == 0 {
            "Nothing downloaded yet. Jellyfin series and books have a Download button on their page, and downloads can be read when the server can't be reached.".to_string()
        } else {
            format!("{count} {} · {} in all", if count == 1 { "item" } else { "items" }, human_size(bytes))
        }));

        // ── Location ──────────────────────────────────────────────────────
        let shown = self.download_root_shown();
        let chosen = self.settings.download_dir.as_deref();
        let available = chosen.is_none_or(Path::is_dir);
        let mut place = widget::Column::new()
            .spacing(6)
            .push(widget::text::heading("Download folder"))
            .push(widget::text::body(shown.display().to_string()));
        if !available {
            place = place.push(widget::text::caption(
                "This folder isn't available. Connect its drive, or pick another folder. New downloads wait until then; nothing is lost.",
            ));
        } else {
            let probe = if shown.is_dir() { shown.clone() } else { chosen.map(Path::to_path_buf).unwrap_or(shown.clone()) };
            let probe = if probe.is_dir() { probe } else { crate::platform::data_dir() };
            if let Some(free) = crate::platform::disk::free_space(&probe) {
                place = place.push(widget::text::caption(format!("{} free on this drive", human_size(free))));
            }
            if chosen.is_some() && crate::platform::disk::other_drive(&probe) == Some(true) {
                place = place.push(widget::text::caption(
                    "On a separate drive. Each file is only kept once it's completely saved, so unplugging mid-download loses just that file — but eject the drive from your file manager before unplugging it.",
                ));
            }
        }
        let locked = self.dl.busy() || self.dl.moving || !self.dl.queue.is_empty();
        place = place.push(
            widget::row::with_capacity(3)
                .spacing(8)
                .push(widget::button::standard("Change Folder…").on_press_maybe((!locked).then_some(Message::Dl(DlMsg::ChooseFolder))))
                .push_maybe(chosen.map(|_| {
                    widget::button::standard("Use Default Folder").on_press_maybe((!locked).then_some(Message::Dl(DlMsg::UseDefaultFolder)))
                }))
                .push(widget::button::standard("Open Folder").on_press_maybe(available.then_some(Message::Dl(DlMsg::OpenFolder)))),
        );
        if count > 0 {
            place = place.push(widget::text::caption("Changing the folder moves your downloads there."));
        }
        col = col.push(place);

        // ── Running ───────────────────────────────────────────────────────
        if self.dl.moving {
            col = col.push(
                widget::row::with_capacity(2)
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(widget::indeterminate_circular().size(14.0).bar_height(2.0))
                    .push(widget::text::body("Moving downloads to the new folder…")),
            );
        }
        if let Some(status) = self.download_status() {
            let mut stop: Vec<String> = self.dl.queue.iter().map(|(k, _)| k.clone()).collect();
            stop.extend(self.dl.current.as_ref().map(|(k, _, _)| k.clone()));
            col = col.push(
                widget::row::with_capacity(2)
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .push(status)
                    .push(widget::button::standard("Cancel").on_press(Message::Dl(DlMsg::Remove(stop)))),
            );
        }
        col = col.push(widget::divider::horizontal::default());

        // ── Everything downloaded ─────────────────────────────────────────
        let mut list = widget::Column::new().spacing(4);
        for g in &self.dl.groups {
            list = list.push(self.download_group_row(g));
            if self.dl.expanded.contains(&g.series) {
                for item in &g.items {
                    list = list.push(download_item_row(item));
                }
            }
        }
        if !self.dl.present.is_empty() {
            list = list.push(widget::Space::new().height(12)).push(
                widget::button::destructive("Remove All Downloads").on_press(Message::Dl(DlMsg::RemoveAll)),
            );
        }
        col = col.push(widget::scrollable(list.padding([0, 12, 24, 0])).height(Length::Fill));
        widget::container(col).width(Length::Fill).height(Length::Fill).into()
    }

    fn download_group_row<'a>(&self, g: &'a DlGroup) -> Element<'a, Message> {
        let expanded = self.dl.expanded.contains(&g.series);
        let missing = g.items.iter().filter(|i| !i.present).count();
        let mut facts = vec![
            if g.items.len() == 1 && g.items[0].key == g.series { "1 file".to_string() } else { format!("{} chapters", g.items.len()) },
            human_size(g.size),
        ];
        if missing > 0 {
            facts.push(if missing == g.items.len() { "drive not connected".into() } else { format!("{missing} on a drive not connected") });
        }
        let present: Vec<String> = g.items.iter().filter(|i| i.present).map(|i| i.key.clone()).collect();
        let in_library = self.db.as_ref().is_some_and(|db| library::get_series(db, &g.series).is_some());
        let row = widget::row::with_capacity(5)
            .spacing(12)
            .align_y(Alignment::Center)
            .padding([6, 8])
            .push(icon::from_name(if expanded { "pan-down-symbolic" } else { "pan-end-symbolic" }).size(16))
            .push(
                widget::column::with_capacity(2)
                    .spacing(2)
                    .width(Length::Fill)
                    .push(widget::text::heading(g.name.clone()))
                    .push(widget::text::caption(facts.join("  ·  "))),
            )
            .push_maybe(in_library.then(|| widget::button::standard("Open").on_press(Message::OpenSeriesPage(g.series.clone()))))
            .push(widget::button::standard("Remove").on_press_maybe((!present.is_empty()).then(|| Message::Dl(DlMsg::Remove(present)))));
        widget::mouse_area(row)
            .on_press(Message::Dl(DlMsg::ToggleGroup(g.series.clone())))
            .interaction(cosmic::iced::mouse::Interaction::Pointer)
            .into()
    }
}

/// One downloaded chapter under its series on the Downloads page.
fn download_item_row(item: &DlItem) -> Element<'_, Message> {
    let mut facts = vec![human_size(item.size)];
    if !item.present {
        facts.push("drive not connected".into());
    }
    widget::row::with_capacity(3)
        .spacing(12)
        .align_y(Alignment::Center)
        .padding([2, 8, 2, 40])
        .push(widget::text::body(item.title.clone()).width(Length::Fill))
        .push(widget::text::caption(facts.join("  ·  ")))
        .push(
            widget::button::icon(icon::from_name("edit-delete-symbolic"))
                .on_press_maybe(item.present.then(|| Message::Dl(DlMsg::Remove(vec![item.key.clone()])))),
        )
        .into()
}

/// Copies each download to its new place (see `disk::copy_file`); the
/// originals are deleted once the database points at the copies.
fn move_files(jobs: Vec<(String, PathBuf, PathBuf)>) -> (Vec<(String, PathBuf)>, Vec<String>) {
    let mut moved = Vec::new();
    let mut errors = Vec::new();
    for (key, from, to) in jobs {
        let result = to
            .parent()
            .ok_or_else(|| "bad path".to_string())
            .and_then(|dir| std::fs::create_dir_all(dir).map_err(|e| e.to_string()))
            .and_then(|()| crate::platform::disk::copy_file(&from, &to));
        match result {
            Ok(_) => moved.push((key, to)),
            Err(e) => errors.push(e),
        }
    }
    (moved, errors)
}

#[cfg(test)]
mod tests {
    use super::human_size;

    #[test]
    fn sizes() {
        assert_eq!(human_size(5 * 1024 * 1024 / 2), "2.5 MB");
        assert_eq!(human_size(230 * 1024 * 1024), "230 MB");
        assert_eq!(human_size(1536 * 1024 * 1024), "1.5 GB");
    }
}
