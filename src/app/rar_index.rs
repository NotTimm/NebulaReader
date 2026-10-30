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
    /// A background check finished (see `App::refresh_unindexed`).
    Refreshed(HashMap<String, Vec<String>>, HashMap<String, bool>),
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

enum IndexState {
    /// Not a streamed CBR (another format, downloaded, or fetched whole).
    NotApplicable,
    Missing,
    Built { solid: bool },
}

/// Where a Jellyfin chapter stands. Touches the disk, so not for drawing.
fn index_state(key: &str, extensions: &HashMap<String, String>, downloaded: &HashMap<String, PathBuf>) -> IndexState {
    #[derive(serde::Deserialize)]
    struct Flags {
        solid: bool,
    }
    let Some((_, item_id)) = jellyfin::parse_key(key) else { return IndexState::NotApplicable };
    let Some(ext) = extensions.get(item_id).filter(|e| matches!(e.as_str(), "cbr" | "rar")) else {
        return IndexState::NotApplicable;
    };
    if downloaded.get(key).is_some_and(|f| f.exists())
        || library::cache_dir().join("jellyfin").join(format!("{item_id}.{ext}")).exists()
    {
        return IndexState::NotApplicable;
    }
    match std::fs::read(index_path(item_id)) {
        Err(_) => IndexState::Missing,
        Ok(bytes) => match serde_json::from_slice::<Flags>(&bytes) {
            Ok(f) => IndexState::Built { solid: f.solid },
            // Damaged: the reader would walk it again.
            Err(_) => IndexState::Missing,
        },
    }
}

/// Whether opening this Jellyfin chapter starts by walking its CBR's
/// headers (no index yet). Touches the disk, so not for drawing.
pub(super) fn needs_index(key: &str, extensions: &HashMap<String, String>, downloaded: &HashMap<String, PathBuf>) -> bool {
    matches!(index_state(key, extensions, downloaded), IndexState::Missing)
}

/// Series -> chapters without an index, and chapter -> solid for those with one.
type Scan = (HashMap<String, Vec<String>>, HashMap<String, bool>);

fn scan(
    series: Vec<(String, Vec<String>)>,
    extensions: &HashMap<String, String>,
    downloaded: &HashMap<String, PathBuf>,
) -> Scan {
    let mut unindexed = HashMap::new();
    let mut indexed = HashMap::new();
    for (key, issues) in series {
        let mut missing = Vec::new();
        for issue in issues {
            match index_state(&issue, extensions, downloaded) {
                IndexState::Missing => missing.push(issue),
                IndexState::Built { solid } => {
                    indexed.insert(issue, solid);
                }
                IndexState::NotApplicable => {}
            }
        }
        if !missing.is_empty() {
            unindexed.insert(key, missing);
        }
    }
    (unindexed, indexed)
}

impl App {
    fn needs_index(&self, key: &str, downloaded: &HashMap<String, PathBuf>) -> bool {
        matches!(index_state(key, &self.jf.extensions, downloaded), IndexState::Missing)
    }

    /// Works out, in the background, which Jellyfin CBR chapters have no
    /// page index (for the right-click menus) and which have one (for the
    /// marks), so the interface never waits on the file checks. Run after a
    /// server sync, after indexing and after reading, not while drawing.
    pub(super) fn refresh_unindexed(&mut self) -> Task<Message> {
        let Some(db) = &self.db else { return Task::none() };
        if self.jf.extensions.is_empty() {
            self.unindexed.clear();
            self.indexed.clear();
            return Task::none();
        }
        let series: Vec<(String, Vec<String>)> = library::all_series(db)
            .into_iter()
            .map(|s| s.path)
            .filter(|p| jellyfin::is_key(p))
            .map(|key| {
                let issues = library::issues_for(db, &key).into_iter().map(|i| i.path).collect();
                (key, issues)
            })
            .collect();
        let extensions = self.jf.extensions.clone();
        let downloaded = self.dl.files();
        cosmic::task::future(async move {
            let (unindexed, indexed) =
                tokio::task::spawn_blocking(move || scan(series, &extensions, &downloaded))
                    .await
                    .unwrap_or_default();
            Message::Index(IdxMsg::Refreshed(unindexed, indexed))
        })
    }

    /// The mark on a chapter row for a CBR whose page index is built.
    pub(super) fn index_mark(&self, issue: &str) -> Option<Element<'_, Message>> {
        let solid = *self.indexed.get(issue)?;
        let (name, hint) = if solid {
            ("package-x-generic-symbolic", fl!("index-solid"))
        } else {
            ("emblem-documents-symbolic", fl!("index-built"))
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
                    self.index_note = Some(fl!("index-already-built"));
                    return self.refresh_unindexed();
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
            IdxMsg::Refreshed(unindexed, indexed) => {
                self.unindexed = unindexed;
                self.indexed = indexed;
            }
        }
        Task::none()
    }

    /// Starts the next chapter, or finishes.
    fn next_index(&mut self) -> Task<Message> {
        loop {
            let Some(job) = self.indexing.as_mut() else { return Task::none() };
            let next = if job.stopped { None } else { job.queue.pop_front() };
            let Some(key) = next else {
                return self.finish_index();
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

    fn finish_index(&mut self) -> Task<Message> {
        let Some(job) = self.indexing.take() else { return Task::none() };
        let built = job.done - job.failed;
        let mut note = if job.stopped {
            fl!("index-stopped", built = built, total = job.total)
        } else if built == 1 {
            fl!("index-built-one")
        } else {
            fl!("index-built-many", count = built)
        };
        if job.solid > 0 {
            note = format!("{note} {}", fl!("index-solid-count", count = job.solid));
        }
        if job.failed > 0 {
            note = format!("{note} {}", fl!("index-failed", count = job.failed));
        }
        self.index_note = Some(note);
        self.refresh_unindexed()
    }

    /// The progress line, or the note when it's done.
    pub(super) fn index_status(&self) -> Option<Element<'_, Message>> {
        if let Some(job) = &self.indexing {
            return Some(
                widget::row::with_capacity(3)
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(widget::indeterminate_circular().size(14.0).bar_height(2.0))
                    .push(widget::text::caption(if job.total == 1 {
                        fl!("creating-index")
                    } else {
                        fl!("creating-indexes", done = (job.done + 1).min(job.total), total = job.total)
                    }))
                    .push_maybe(
                        (!job.stopped).then(|| widget::button::link(fl!("stop")).on_press(Message::Index(IdxMsg::Stop))),
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
