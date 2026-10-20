//! "Build Page Index" for Jellyfin CBRs: walks the archives' headers ahead
//! of time (see `crate::remote_rar`) instead of when a chapter is first
//! opened, so it opens right away later. It also shows which chapters are
//! "solid" and will be fetched whole when read. One chapter at a time in
//! the background, with a progress line in the library and series page.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use cosmic::app::Task;
use cosmic::iced::Alignment;
use cosmic::widget::{self, icon};
use cosmic::Element;

use super::{App, Message};
use crate::remote_rar::RemoteRar;
use crate::{jellyfin, library};

#[derive(Clone, Debug)]
pub enum IdxMsg {
    /// Index these chapters (Jellyfin issue keys).
    Build(Vec<String>),
    /// One chapter is done: whether it's solid, or why it failed.
    Done(String, Result<bool, String>),
    Stop,
    DismissNote,
}

/// The series page's right-click menu on a chapter; it acts on
/// `App::issue_menu_target`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueAction {
    BuildIndex,
}

impl cosmic::widget::menu::Action for IssueAction {
    type Message = Message;

    fn message(&self) -> Message {
        Message::IssueMenu(*self)
    }
}

pub struct Indexing {
    queue: VecDeque<String>,
    done: usize,
    total: usize,
    solid: usize,
    failed: usize,
    stopped: bool,
}

/// Where a Jellyfin CBR's page index is kept.
pub fn index_path(item_id: &str) -> PathBuf {
    library::cache_dir().join("jellyfin-index").join(format!("{item_id}.json"))
}

impl App {
    /// Whether this Jellyfin chapter is a CBR that would be walked when
    /// opened: not downloaded, not fetched whole, no index yet.
    fn needs_index(&self, key: &str, downloaded: &HashMap<String, PathBuf>) -> bool {
        let Some((_, item_id)) = jellyfin::parse_key(key) else { return false };
        let Some(ext) = self.jf.extensions.get(item_id).filter(|e| matches!(e.as_str(), "cbr" | "rar")) else {
            return false;
        };
        !downloaded.get(key).is_some_and(|f| f.exists())
            && !library::cache_dir().join("jellyfin").join(format!("{item_id}.{ext}")).exists()
            && !index_path(item_id).exists()
    }

    /// Works out which Jellyfin series have CBR chapters without an index,
    /// for the right-click menus. Called when the library reloads and after
    /// indexing, not while drawing.
    pub(super) fn refresh_unindexed(&mut self) {
        self.unindexed.clear();
        self.indexed.clear();
        let Some(db) = &self.db else { return };
        if self.jf.extensions.is_empty() {
            return;
        }
        let downloaded = self.dl.files();
        let series: Vec<String> =
            library::all_series(db).into_iter().map(|s| s.path).filter(|p| jellyfin::is_key(p)).collect();
        let mut indexed = HashMap::new();
        for key in series {
            let mut missing = Vec::new();
            for issue in library::issues_for(db, &key) {
                if self.needs_index(&issue.path, &downloaded) {
                    missing.push(issue.path);
                } else if let Some(solid) = self.index_solid(&issue.path, &downloaded) {
                    indexed.insert(issue.path, solid);
                }
            }
            if !missing.is_empty() {
                self.unindexed.insert(key, missing);
            }
        }
        self.indexed = indexed;
    }

    /// For a streamed Jellyfin CBR with a page index: whether it's solid.
    /// `None` when it has no index or is read from a local copy anyway.
    fn index_solid(&self, key: &str, downloaded: &HashMap<String, PathBuf>) -> Option<bool> {
        #[derive(serde::Deserialize)]
        struct Flags {
            solid: bool,
        }
        let (_, item_id) = jellyfin::parse_key(key)?;
        let ext = self.jf.extensions.get(item_id).filter(|e| matches!(e.as_str(), "cbr" | "rar"))?;
        if downloaded.get(key).is_some_and(|f| f.exists())
            || library::cache_dir().join("jellyfin").join(format!("{item_id}.{ext}")).exists()
        {
            return None;
        }
        let bytes = std::fs::read(index_path(item_id)).ok()?;
        serde_json::from_slice::<Flags>(&bytes).ok().map(|f| f.solid)
    }

    /// The mark on a chapter row for a CBR whose page index is built.
    pub(super) fn index_mark(&self, issue: &str) -> Option<Element<'_, Message>> {
        let solid = *self.indexed.get(issue)?;
        let (name, hint) = if solid {
            (
                "package-x-generic-symbolic",
                "Packed as one block (solid): downloaded whole when you read it",
            )
        } else {
            ("emblem-documents-symbolic", "Page index built: opens right away and streams page by page")
        };
        Some(
            widget::tooltip(
                widget::container(icon::from_name(name).size(16)).padding(8),
                widget::text(hint),
                widget::tooltip::Position::Left,
            )
            .into(),
        )
    }

    /// Whether a chapter on the series page can have its index built.
    pub(super) fn issue_unindexed(&self, series: &str, issue: &str) -> bool {
        self.unindexed.get(series).is_some_and(|v| v.iter().any(|k| k == issue))
    }

    pub(super) fn update_index(&mut self, msg: IdxMsg) -> Task<Message> {
        match msg {
            IdxMsg::Build(keys) => {
                self.index_note = None;
                // The menus can be a little behind (a chapter downloaded or
                // opened since), so check again.
                let downloaded = self.dl.files();
                let keys: Vec<String> = keys.into_iter().filter(|k| self.needs_index(k, &downloaded)).collect();
                if keys.is_empty() {
                    self.index_note = Some("Those chapters already have their page index.".into());
                    self.refresh_unindexed();
                    return Task::none();
                }
                match self.indexing.as_mut() {
                    // Already running: add to the queue.
                    Some(job) => {
                        for k in keys {
                            if !job.queue.contains(&k) {
                                job.total += 1;
                                job.queue.push_back(k);
                            }
                        }
                        return Task::none();
                    }
                    None => {
                        let total = keys.len();
                        self.indexing = Some(Indexing {
                            queue: keys.into(),
                            done: 0,
                            total,
                            solid: 0,
                            failed: 0,
                            stopped: false,
                        });
                        return self.next_index();
                    }
                }
            }
            IdxMsg::Done(key, result) => {
                if let Some(job) = self.indexing.as_mut() {
                    job.done += 1;
                    match result {
                        Ok(true) => job.solid += 1,
                        Ok(false) => {}
                        Err(e) => {
                            log::warn!("page index for {key}: {e}");
                            job.failed += 1;
                        }
                    }
                }
                return self.next_index();
            }
            IdxMsg::Stop => {
                if let Some(job) = self.indexing.as_mut() {
                    job.stopped = true;
                }
            }
            IdxMsg::DismissNote => self.index_note = None,
        }
        Task::none()
    }

    /// Starts the next chapter, or finishes.
    fn next_index(&mut self) -> Task<Message> {
        loop {
            let Some(job) = self.indexing.as_mut() else { return Task::none() };
            let next = if job.stopped { None } else { job.queue.pop_front() };
            let Some(key) = next else {
                self.finish_index();
                return Task::none();
            };
            let Some(account) = self.account_for(&key) else {
                if let Some(job) = self.indexing.as_mut() {
                    job.done += 1;
                    job.failed += 1;
                }
                continue;
            };
            let item_id = jellyfin::parse_key(&key).map(|(_, id)| id.to_string()).unwrap_or_default();
            return cosmic::task::future(async move {
                let result = tokio::task::spawn_blocking(move || {
                    RemoteRar::open(Box::new(account.range_source(&item_id)), Some(&index_path(&item_id)))
                        .map(|rar| rar.solid())
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                Message::Index(IdxMsg::Done(key, result))
            });
        }
    }

    fn finish_index(&mut self) {
        let Some(job) = self.indexing.take() else { return };
        let built = job.done - job.failed;
        let mut note = if job.stopped {
            format!("Stopped after {built} of {} page indexes.", job.total)
        } else if built == 1 {
            "Page index built.".to_string()
        } else {
            format!("Built {built} page indexes.")
        };
        if job.solid > 0 {
            note.push_str(&if job.solid == 1 {
                " 1 chapter is packed as one block, so it's fetched whole when you read it.".to_string()
            } else {
                format!(" {} chapters are packed as one block, so they're fetched whole when you read them.", job.solid)
            });
        }
        if job.failed > 0 {
            note.push_str(&format!(" {} couldn't be read from the server.", job.failed));
        }
        self.index_note = Some(note);
        self.refresh_unindexed();
    }

    /// The progress line, or the note when it's done.
    pub(super) fn index_status(&self) -> Option<Element<'_, Message>> {
        if let Some(job) = &self.indexing {
            return Some(
                widget::row::with_capacity(3)
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(widget::indeterminate_circular().size(14.0).bar_height(2.0))
                    .push(widget::text::caption(format!(
                        "Building page indexes… {} of {}",
                        (job.done + 1).min(job.total),
                        job.total
                    )))
                    .push_maybe(
                        (!job.stopped).then(|| widget::button::link("Stop").on_press(Message::Index(IdxMsg::Stop))),
                    )
                    .into(),
            );
        }
        let note = self.index_note.as_deref()?;
        Some(
            widget::row::with_capacity(3)
                .spacing(8)
                .align_y(Alignment::Center)
                .push(icon::from_name("emblem-ok-symbolic").size(16))
                .push(widget::text::caption(note.to_string()))
                .push(
                    widget::button::icon(icon::from_name("window-close-symbolic"))
                        .on_press(Message::Index(IdxMsg::DismissNote)),
                )
                .into(),
        )
    }
}
