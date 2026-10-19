//! Jellyfin downloads: chapters and books kept on this computer so they
//! can be read when the server can't be reached.
//!
//! Downloads stay Jellyfin items (their library keys don't change), so
//! they show under the Jellyfin libraries, keep syncing progress with the
//! server, and are only listed on their own while their server is offline.
//! Files live in `jellyfin::download_dir`, one at a time is downloaded, and
//! the `downloads` table remembers what's there.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use cosmic::app::Task;
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
}

/// What one chapter's download is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DlState {
    None,
    Queued,
    Running(f32),
    Done,
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
            _ if self.done.contains_key(issue_key) => DlState::Done,
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
                for issue in issues {
                    self.dl.queue.retain(|(k, _)| *k != issue);
                    if let Some((k, _, p)) = &self.dl.current {
                        if *k == issue {
                            p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    if let Some(file) = library::remove_download(db, &issue) {
                        let _ = std::fs::remove_file(file);
                    }
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
            DlMsg::Tick => {}
            DlMsg::OpenFolder => {
                let dir = crate::platform::data_dir().join("jellyfin-downloads");
                let _ = std::fs::create_dir_all(&dir);
                crate::platform::open_url(&dir.to_string_lossy());
            }
            DlMsg::RemoveAll => {
                let all: Vec<String> = self.dl.done.keys().cloned().collect();
                self.dl.queue.clear();
                return self.update_downloads(DlMsg::Remove(all));
            }
        }
        Task::none()
    }

    /// Starts the next queued download, one at a time.
    fn pump_downloads(&mut self) -> Task<Message> {
        if self.dl.current.is_some() {
            return Task::none();
        }
        while let Some((issue, series)) = self.dl.queue.pop_front() {
            let Some((account_key, item_id)) = jellyfin::parse_key(&issue) else { continue };
            let Some(account) = self.jellyfin.find(account_key).cloned() else { continue };
            let ext = self.jf.extensions.get(item_id).filter(|e| !e.is_empty()).cloned().unwrap_or_else(|| "cbz".into());
            let dest = jellyfin::download_dir(account_key).join(format!("{item_id}.{ext}"));
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
    pub(super) fn downloads_subscription(&self) -> cosmic::iced::Subscription<Message> {
        if self.dl.busy() {
            cosmic::iced::time::every(std::time::Duration::from_millis(500)).map(|_| Message::Dl(DlMsg::Tick))
        } else {
            cosmic::iced::Subscription::none()
        }
    }

    /// Deletes downloads whose chapter left the library (removed on the
    /// server, or the account signed out).
    pub(super) fn prune_downloads(&mut self) {
        let Some(db) = &self.db else { return };
        for d in library::orphaned_downloads(db) {
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
        };
        widget::tooltip(button, widget::text(hint), hint_at).into()
    }

    /// The series page's "Download All" / "Remove Downloads" buttons.
    pub(super) fn series_download_actions<'a>(&self, series_key: &str, issue_keys: &[String]) -> Vec<Element<'a, Message>> {
        let mut out = Vec::new();
        let missing: Vec<(String, String)> = issue_keys
            .iter()
            .filter(|k| self.dl.state(k) == DlState::None)
            .map(|k| (k.clone(), series_key.to_string()))
            .collect();
        let have: Vec<String> = issue_keys.iter().filter(|k| self.dl.state(k) != DlState::None).cloned().collect();
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

    /// Settings → Jellyfin: how much is downloaded, and buttons to manage it.
    pub(super) fn downloads_settings<'a>(&self) -> Element<'a, Message> {
        let (count, bytes) = self.dl.totals();
        let summary = if count == 0 {
            "No downloads yet. Use the download buttons on a Jellyfin series to read it offline.".to_string()
        } else {
            format!("Downloads: {count} {} · {}", if count == 1 { "item" } else { "items" }, human_size(bytes))
        };
        widget::column::with_capacity(3)
            .spacing(8)
            .push(widget::text::body(summary))
            .push(
                widget::row::with_capacity(2)
                    .spacing(8)
                    .push(widget::button::standard("Open Folder").on_press(Message::Dl(DlMsg::OpenFolder)))
                    .push_maybe((count > 0).then(|| {
                        widget::button::destructive("Remove All Downloads").on_press(Message::Dl(DlMsg::RemoveAll))
                    })),
            )
            .push_maybe(self.dl.error.as_deref().map(|e| widget::text::caption(format!("Last download failed: {e}"))))
            .width(Length::Fill)
            .into()
    }
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
