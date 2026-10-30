//! Right-clicking a library card: a menu (open its page, read, download,
//! mark read, show in folder, properties) and the Properties panel with
//! where it is, its files, pages, page size and so on.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use cosmic::app::Task;
use cosmic::iced::Length;
use cosmic::widget::{self, menu};
use cosmic::{ApplicationExt, Element};

use super::downloads::{human_size, DlMsg};
use super::{App, ContextPage, Message};
use crate::library::{self, SeriesEntry, SeriesKind};

/// What a card's menu can do; it acts on the card that was right-clicked
/// (`App::menu_target`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardAction {
    OpenPage,
    Read,
    /// The whole series.
    Download,
    RemoveDownloads,
    /// Just one: a one-shot, or the chapter a Continue Reading card shows.
    DownloadIssue,
    RemoveIssueDownload,
    MarkRead,
    MarkUnread,
    ShowInFolder,
    BuildIndex,
    Properties,
}

impl menu::Action for CardAction {
    type Message = Message;

    fn message(&self) -> Message {
        Message::CardMenu(*self)
    }
}

/// The Properties panel's contents for one library entry.
#[derive(Clone, Debug, Default)]
pub struct Props {
    pub key: String,
    pub name: String,
    pub rows: Vec<(String, String)>,
    /// The first page's size in pixels, read in the background.
    pub page_size: Option<String>,
    /// Its folder, for "Show in Folder".
    pub folder: Option<PathBuf>,
}

impl App {
    /// The right-click menu for a library card. On a Continue Reading card
    /// (`continuing`), downloading means the chapter it shows, not the
    /// whole series.
    pub(super) fn card_menu(&self, e: &SeriesEntry, continuing: bool) -> Vec<menu::Tree<Message>> {
        use menu::Item;

        let one_shot = e.issue_count <= 1;
        let mut items = vec![
            Item::Button(if one_shot { fl!("show-details") } else { fl!("show-series") }, None, CardAction::OpenPage),
            Item::Button(
                if e.has_progress() && !e.is_finished() { fl!("continue-reading") } else { fl!("read") },
                None,
                CardAction::Read,
            ),
        ];
        if crate::jellyfin::is_key(&e.path) {
            items.push(Item::Divider);
            // A one-shot, or the chapter a Continue Reading card shows, is
            // downloaded or removed on its own; the whole series only from
            // its other cards (and the Downloads page).
            let single = one_shot || (continuing && !e.last_issue_path.is_empty());
            if single {
                let downloaded = if one_shot { self.dl.has_series(&e.path) } else { self.dl.has(&e.last_issue_path) };
                if downloaded {
                    items.push(Item::Button(fl!("remove-download"), None, CardAction::RemoveIssueDownload));
                } else {
                    let label = if one_shot || e.is_epub { fl!("download-book") } else { fl!("download-issue") };
                    items.push(Item::Button(label, None, CardAction::DownloadIssue));
                }
            } else {
                items.push(Item::Button(fl!("download-all"), None, CardAction::Download));
                if self.dl.has_series(&e.path) {
                    items.push(Item::Button(fl!("remove-downloads"), None, CardAction::RemoveDownloads));
                }
            }
        }
        items.push(Item::Divider);
        if !e.is_finished() {
            items.push(Item::Button(fl!("mark-as-read"), None, CardAction::MarkRead));
        }
        if e.has_progress() || e.is_finished() {
            items.push(Item::Button(fl!("mark-as-unread"), None, CardAction::MarkUnread));
        }
        if local_folder(e).is_some() {
            items.push(Item::Button(fl!("show-in-folder"), None, CardAction::ShowInFolder));
        }
        // Jellyfin CBRs whose pages haven't been indexed yet.
        if self.unindexed.contains_key(&e.path) {
            items.push(Item::Button(
                if one_shot { fl!("build-page-index") } else { fl!("build-page-indexes") },
                None,
                CardAction::BuildIndex,
            ));
        }
        items.push(Item::Divider);
        items.push(Item::Button(fl!("properties"), None, CardAction::Properties));
        menu::items(&HashMap::new(), items)
    }

    pub(super) fn card_action(&mut self, action: CardAction) -> Task<Message> {
        let Some(key) = self.menu_target.clone() else { return Task::none() };
        let Some(entry) = self.db.as_ref().and_then(|db| library::get_series(db, &key)) else { return Task::none() };
        match action {
            CardAction::OpenPage => return self.open_series_page(key),
            CardAction::Read => return self.open_entry(key, None),
            CardAction::Download => {
                let items = self.series_issues(&key).into_iter().map(|i| (i.path, key.clone())).collect();
                return self.update_downloads(DlMsg::Download(items));
            }
            CardAction::DownloadIssue => {
                let issues = self.card_issue(&entry).into_iter().map(|i| (i, key.clone())).collect();
                return self.update_downloads(DlMsg::Download(issues));
            }
            CardAction::RemoveIssueDownload => {
                let issues = self.card_issue(&entry);
                return self.update_downloads(DlMsg::Remove(issues));
            }
            CardAction::RemoveDownloads => {
                let issues = self.series_issues(&key).into_iter().map(|i| i.path).collect();
                return self.update_downloads(DlMsg::Remove(issues));
            }
            CardAction::MarkRead => return self.mark_series(&key, true),
            CardAction::MarkUnread => return self.mark_series(&key, false),
            CardAction::ShowInFolder => {
                if let Some(dir) = local_folder(&entry) {
                    crate::platform::open_url(&dir.to_string_lossy());
                }
            }
            CardAction::BuildIndex => {
                let keys = self.unindexed.get(&key).cloned().unwrap_or_default();
                return self.update_index(super::rar_index::IdxMsg::Build(keys));
            }
            CardAction::Properties => return self.open_properties(&entry),
        }
        Task::none()
    }

    /// The one issue a card's Download Issue / Book stands for: a
    /// one-shot's only issue, else the chapter being read.
    fn card_issue(&self, entry: &SeriesEntry) -> Vec<String> {
        if entry.issue_count <= 1 {
            self.series_issues(&entry.path).into_iter().map(|i| i.path).collect()
        } else if entry.last_issue_path.is_empty() {
            Vec::new()
        } else {
            vec![entry.last_issue_path.clone()]
        }
    }

    /// Marks a whole series (or a one-shot) read or unread, on its
    /// Jellyfin server too.
    pub(super) fn mark_series(&mut self, key: &str, completed: bool) -> Task<Message> {
        if let Some(db) = &self.db {
            let _ = library::set_series_completed(db, key, completed);
        }
        let remote: Vec<String> =
            self.series_issues(key).into_iter().map(|i| i.path).filter(|p| crate::jellyfin::is_key(p)).collect();
        Task::batch([self.refresh_views(), self.jellyfin_mark(remote, completed)])
    }

    fn open_properties(&mut self, e: &SeriesEntry) -> Task<Message> {
        let issues = self.series_issues(&e.path);
        let remote = crate::jellyfin::is_key(&e.path);
        let manga = super::library_view::is_manga(&issues);
        let mut rows: Vec<(String, String)> = Vec::new();

        let kind = match (remote, e.kind) {
            (true, _) if e.issue_count > 1 => fl!("kind-jellyfin-series"),
            (true, _) if e.is_epub => fl!("kind-jellyfin-book"),
            (true, _) => fl!("kind-jellyfin-comic"),
            (false, SeriesKind::Dir) => fl!("kind-folder-series"),
            (false, SeriesKind::Group) => fl!("kind-group-series"),
            (false, SeriesKind::File) if e.is_epub => fl!("kind-epub"),
            (false, SeriesKind::File) => fl!("kind-single-file"),
        };
        rows.push((fl!("prop-type"), kind));

        // Formats by file extension: "CBZ × 12, CBR × 2".
        let mut formats: BTreeMap<String, usize> = BTreeMap::new();
        for i in &issues {
            let file = self.dl.file(&i.path).cloned().unwrap_or_else(|| PathBuf::from(&i.path));
            let ext = if !remote && file.is_dir() {
                fl!("image-folder")
            } else {
                let item = crate::jellyfin::parse_key(&i.path).map(|(_, id)| id);
                item.and_then(|id| self.jf.extensions.get(id).cloned())
                    .or_else(|| file.extension().map(|x| x.to_string_lossy().into_owned()))
                    .map(|x| x.to_uppercase())
                    .unwrap_or_default()
            };
            if !ext.is_empty() {
                *formats.entry(ext).or_default() += 1;
            }
        }
        if !formats.is_empty() {
            let text = formats
                .iter()
                .map(|(f, n)| if issues.len() > 1 { format!("{f} × {n}") } else { f.clone() })
                .collect::<Vec<_>>()
                .join(", ");
            rows.push((fl!("prop-format"), text));
        }

        if remote {
            let account = crate::jellyfin::parse_key(&e.path).and_then(|(a, _)| self.jellyfin.find(a));
            if let Some(a) = account {
                rows.push((fl!("prop-server"), format!("{} ({})", a.server_name, a.url)));
            }
            if let Some(lib) = self.jf.library_names.get(&e.library_root) {
                rows.push((fl!("prop-library"), lib.clone()));
            }
        } else {
            let location = match e.kind {
                SeriesKind::Group => e.library_root.clone(),
                _ => e.path.clone(),
            };
            rows.push((fl!("prop-location"), location));
        }

        if issues.len() > 1 {
            rows.push((if manga { fl!("prop-chapters") } else { fl!("prop-issues") }, issues.len().to_string()));
        }
        let known: Vec<usize> = issues.iter().map(|i| i.page_count).filter(|&n| n > 0).collect();
        if !known.is_empty() {
            let pages: usize = known.iter().sum();
            let all = known.len() == issues.len();
            rows.push((
                fl!("prop-pages"),
                if all { pages.to_string() } else { fl!("prop-pages-partial", pages = pages, known = known.len(), total = issues.len()) },
            ));
        }

        if !remote {
            let bytes: u64 = issues.iter().map(|i| size_on_disk(Path::new(&i.path))).sum();
            if bytes > 0 {
                rows.push((fl!("prop-size"), human_size(bytes)));
            }
        } else {
            let downloaded: Vec<u64> =
                issues.iter().filter(|i| self.dl.file(&i.path).is_some()).map(|i| size_on_disk(self.dl.file(&i.path).unwrap())).collect();
            if !downloaded.is_empty() {
                let size = human_size(downloaded.iter().sum());
                rows.push((
                    fl!("prop-downloaded"),
                    if issues.len() > 1 {
                        fl!("prop-downloaded-of", count = downloaded.len(), total = issues.len(), size = size)
                    } else {
                        size
                    },
                ));
            }
        }

        let read = issues.iter().filter(|i| i.completed).count();
        if issues.len() > 1 {
            rows.push((
                fl!("prop-read"),
                fl!("prop-read-of", read = read, total = issues.len(), percent = format!("{:.0}", e.progress_fraction() * 100.0)),
            ));
        } else if let Some(i) = issues.first() {
            let state = if i.completed {
                fl!("finished")
            } else if i.in_progress() {
                fl!("page-of", page = i.last_page + 1, pages = i.page_count.max(1))
            } else {
                fl!("not-started")
            };
            rows.push((fl!("prop-read"), state));
        }
        if e.date_added > 0 {
            rows.push((fl!("prop-added"), day(e.date_added)));
        }
        if e.last_opened > 0 {
            rows.push((fl!("prop-last-opened"), day(e.last_opened)));
        }
        rows.push((
            fl!("prop-metadata"),
            match e.source.as_str() {
                "anilist" => "AniList".into(),
                "comicvine" => "ComicVine".into(),
                _ if !e.description.is_empty() => fl!("metadata-from-files"),
                _ => fl!("metadata-none"),
            },
        ));

        self.properties = Some(Props {
            key: e.path.clone(),
            name: e.name().to_string(),
            rows,
            page_size: None,
            folder: local_folder(e),
        });
        self.context_page = ContextPage::Properties;
        self.set_show_context(true);

        // The first page's size, for comics on this computer.
        let first = issues
            .first()
            .map(|i| self.dl.file(&i.path).cloned().unwrap_or_else(|| PathBuf::from(&i.path)))
            .filter(|f| f.exists() && !crate::epub::is_epub_name(&f.to_string_lossy()));
        let Some(first) = first else { return Task::none() };
        let key = e.path.clone();
        cosmic::task::future(async move {
            let size = tokio::task::spawn_blocking(move || crate::comic::first_page(&first).ok().map(|p| (p.width, p.height)))
                .await
                .ok()
                .flatten();
            Message::PageSizeRead(key, size)
        })
    }

    pub(super) fn page_size_read(&mut self, key: String, size: Option<(u32, u32)>) {
        if let (Some(p), Some((w, h))) = (self.properties.as_mut(), size) {
            if p.key == key {
                p.page_size = Some(format!("{w} × {h} px"));
            }
        }
    }

    /// The Properties panel.
    pub(super) fn build_properties_panel(&self) -> Element<'_, Message> {
        let Some(p) = &self.properties else { return widget::Space::new().into() };
        let mut section = widget::settings::section();
        let mut rows: Vec<(String, &str)> = p.rows.iter().map(|(k, v)| (k.clone(), v.as_str())).collect();
        if let Some(size) = &p.page_size {
            let pages = fl!("prop-pages");
            let at = rows.iter().position(|(k, _)| *k == pages).map_or(rows.len(), |i| i + 1);
            rows.insert(at, (fl!("prop-page-size"), size.as_str()));
        }
        for (label, value) in rows {
            section = section.add(
                widget::column::with_capacity(2)
                    .spacing(2)
                    .push(widget::text::caption(label))
                    .push(widget::text::body(value.to_string()).wrapping(cosmic::iced::widget::text::Wrapping::WordOrGlyph)),
            );
        }
        widget::column::with_capacity(3)
            .spacing(16)
            .push(widget::text::title4(p.name.clone()))
            .push(section)
            .push(
                widget::row::with_capacity(2)
                    .spacing(8)
                    .push(widget::button::standard(fl!("open-page")).on_press(Message::OpenSeriesPage(p.key.clone())))
                    .push_maybe(p.folder.as_ref().map(|dir| {
                        widget::button::standard(fl!("show-in-folder")).on_press(Message::OpenUrl(dir.to_string_lossy().into_owned()))
                    })),
            )
            .width(Length::Fill)
            .into()
    }
}

/// The folder to show for an entry on this computer.
fn local_folder(e: &SeriesEntry) -> Option<PathBuf> {
    if crate::jellyfin::is_key(&e.path) || crate::jellyfin::is_key(&e.library_root) {
        return None;
    }
    let dir = match e.kind {
        SeriesKind::Dir => PathBuf::from(&e.path),
        SeriesKind::File => Path::new(&e.path).parent()?.to_path_buf(),
        SeriesKind::Group => PathBuf::from(&e.library_root),
    };
    dir.is_dir().then_some(dir)
}

/// A file's size, or an image folder's (one level deep).
fn size_on_disk(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.metadata().ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .sum(),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// A Unix time as "Oct 19, 2026".
fn day(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .map(|t| library::pretty_date(&t.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string()))
        .unwrap_or_default()
}
