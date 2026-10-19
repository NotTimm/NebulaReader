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
    Download,
    RemoveDownloads,
    MarkRead,
    MarkUnread,
    ShowInFolder,
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
    pub rows: Vec<(&'static str, String)>,
    /// The first page's size in pixels, read in the background.
    pub page_size: Option<String>,
    /// Its folder, for "Show in Folder".
    pub folder: Option<PathBuf>,
}

impl App {
    /// The right-click menu for a library card.
    pub(super) fn card_menu(&self, e: &SeriesEntry) -> Vec<menu::Tree<Message>> {
        use menu::Item;

        let one_shot = e.issue_count <= 1;
        let mut items = vec![
            Item::Button(if one_shot { "Show Details" } else { "Show Series" }, None, CardAction::OpenPage),
            Item::Button(
                if e.has_progress() && !e.is_finished() { "Continue Reading" } else { "Read" },
                None,
                CardAction::Read,
            ),
        ];
        if crate::jellyfin::is_key(&e.path) {
            items.push(Item::Divider);
            let has = self.dl.has_series(&e.path);
            if !(one_shot && has) {
                items.push(Item::Button(if one_shot { "Download" } else { "Download All" }, None, CardAction::Download));
            }
            if has {
                items.push(Item::Button(
                    if one_shot { "Remove Download" } else { "Remove Downloads" },
                    None,
                    CardAction::RemoveDownloads,
                ));
            }
        }
        items.push(Item::Divider);
        if !e.is_finished() {
            items.push(Item::Button("Mark as Read", None, CardAction::MarkRead));
        }
        if e.has_progress() || e.is_finished() {
            items.push(Item::Button("Mark as Unread", None, CardAction::MarkUnread));
        }
        if local_folder(e).is_some() {
            items.push(Item::Button("Show in Folder", None, CardAction::ShowInFolder));
        }
        items.push(Item::Divider);
        items.push(Item::Button("Properties", None, CardAction::Properties));
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
            CardAction::Properties => return self.open_properties(&entry),
        }
        Task::none()
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
        let mut rows: Vec<(&'static str, String)> = Vec::new();

        let kind = match (remote, e.kind) {
            (true, _) if e.issue_count > 1 => "Series on a Jellyfin server",
            (true, _) if e.is_epub => "Book on a Jellyfin server",
            (true, _) => "Comic on a Jellyfin server",
            (false, SeriesKind::Dir) => "Series (a folder)",
            (false, SeriesKind::Group) => "Series (files grouped by name)",
            (false, SeriesKind::File) if e.is_epub => "Book (EPUB)",
            (false, SeriesKind::File) => "Single file",
        };
        rows.push(("Type", kind.into()));

        // Formats by file extension: "CBZ × 12, CBR × 2".
        let mut formats: BTreeMap<String, usize> = BTreeMap::new();
        for i in &issues {
            let file = self.dl.file(&i.path).cloned().unwrap_or_else(|| PathBuf::from(&i.path));
            let ext = if !remote && file.is_dir() {
                "Image folder".to_string()
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
            rows.push(("Format", text));
        }

        if remote {
            let account = crate::jellyfin::parse_key(&e.path).and_then(|(a, _)| self.jellyfin.find(a));
            if let Some(a) = account {
                rows.push(("Server", format!("{} ({})", a.server_name, a.url)));
            }
            if let Some(lib) = self.jf.library_names.get(&e.library_root) {
                rows.push(("Library", lib.clone()));
            }
        } else {
            let location = match e.kind {
                SeriesKind::Group => e.library_root.clone(),
                _ => e.path.clone(),
            };
            rows.push(("Location", location));
        }

        if issues.len() > 1 {
            rows.push((if manga { "Chapters" } else { "Issues" }, issues.len().to_string()));
        }
        let known: Vec<usize> = issues.iter().map(|i| i.page_count).filter(|&n| n > 0).collect();
        if !known.is_empty() {
            let pages: usize = known.iter().sum();
            let all = known.len() == issues.len();
            rows.push(("Pages", if all { pages.to_string() } else { format!("{pages} in {} of {} opened so far", known.len(), issues.len()) }));
        }

        if !remote {
            let bytes: u64 = issues.iter().map(|i| size_on_disk(Path::new(&i.path))).sum();
            if bytes > 0 {
                rows.push(("Size", human_size(bytes)));
            }
        } else {
            let downloaded: Vec<u64> =
                issues.iter().filter(|i| self.dl.file(&i.path).is_some()).map(|i| size_on_disk(self.dl.file(&i.path).unwrap())).collect();
            if !downloaded.is_empty() {
                let size = human_size(downloaded.iter().sum());
                rows.push((
                    "Downloaded",
                    if issues.len() > 1 { format!("{} of {} · {size}", downloaded.len(), issues.len()) } else { size },
                ));
            }
        }

        let read = issues.iter().filter(|i| i.completed).count();
        if issues.len() > 1 {
            rows.push(("Read", format!("{read} of {} · {:.0}%", issues.len(), e.progress_fraction() * 100.0)));
        } else if let Some(i) = issues.first() {
            let state = if i.completed {
                "Finished".to_string()
            } else if i.in_progress() {
                format!("Page {} of {}", i.last_page + 1, i.page_count.max(1))
            } else {
                "Not started".to_string()
            };
            rows.push(("Read", state));
        }
        if e.date_added > 0 {
            rows.push(("Added", day(e.date_added)));
        }
        if e.last_opened > 0 {
            rows.push(("Last opened", day(e.last_opened)));
        }
        rows.push((
            "Metadata",
            match e.source.as_str() {
                "anilist" => "AniList".into(),
                "comicvine" => "ComicVine".into(),
                _ if !e.description.is_empty() => "From the files".into(),
                _ => "None".into(),
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
        let mut rows: Vec<(&str, &str)> = p.rows.iter().map(|(k, v)| (*k, v.as_str())).collect();
        if let Some(size) = &p.page_size {
            let at = rows.iter().position(|(k, _)| *k == "Pages").map_or(rows.len(), |i| i + 1);
            rows.insert(at, ("Page size", size.as_str()));
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
                    .push(widget::button::standard("Open Page").on_press(Message::OpenSeriesPage(p.key.clone())))
                    .push_maybe(p.folder.as_ref().map(|dir| {
                        widget::button::standard("Show in Folder").on_press(Message::OpenUrl(dir.to_string_lossy().into_owned()))
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
