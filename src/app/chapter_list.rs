//! The comic reader's chapter panel (C): every chapter of the series, not
//! just the ones loaded so far (a Jellyfin series streams a few at a time),
//! grouped by volume when the file names give one, otherwise in blocks of
//! 25, with a dropdown to jump between groups.

use cosmic::app::Task;
use cosmic::iced::widget::scrollable::{self as scroll_mod, AbsoluteOffset};
use cosmic::iced::{Alignment, Background, Color, ContentFit, Length};
use cosmic::iced::widget::mouse_area;
use cosmic::widget::{self, icon};
use cosmic::Element;

use super::{App, Message};
use crate::{comic, library, naming};

/// Chapters per jump-to block when there are no volumes.
const BLOCK: usize = 25;
/// Every row is this tall, so a row's scroll offset is easy to work out.
const ROW_H: f32 = 97.0;
const ROW_GAP: f32 = 2.0;
const LIST_PAD: f32 = 8.0;

fn list_id() -> widget::Id {
    widget::Id::new("chapter-list")
}

/// One chapter in the panel.
struct Row {
    /// 1-based position in the series.
    number: usize,
    title: String,
    pages: usize,
    volume: Option<u32>,
    cover: Option<widget::image::Handle>,
    current: bool,
    status: Option<String>,
    on_press: Message,
}

impl App {
    /// The series' chapters in reading order.
    fn chapter_rows(&self) -> Vec<Row> {
        let current_ch = self.current_chapter().map(|(i, _, _)| i);
        let status = |issue: Option<&library::IssueEntry>| {
            let i = issue?;
            if i.completed {
                Some("✓ Read".to_string())
            } else if i.in_progress() {
                Some(format!("Page {} of {}", i.last_page + 1, i.page_count.max(1)))
            } else {
                None
            }
        };

        // From Jellyfin: the whole series from the library, loaded or not.
        let remote = self
            .remote
            .as_ref()
            .filter(|r| self.open_path.as_ref().is_some_and(|p| p.to_string_lossy() == r.series_key));
        if let Some(r) = remote {
            let offline = self.jellyfin_offline(&r.series_key);
            // Looked up once, not per row: a long manga has 1000+ chapters.
            let loaded_at: std::collections::HashMap<std::borrow::Cow<'_, str>, usize> =
                self.chapters.iter().enumerate().map(|(i, c)| (c.path.to_string_lossy(), i)).collect();
            let mut rows = Vec::new();
            for issue in &self.reader_issues {
                let loaded = loaded_at.get(issue.path.as_str()).copied();
                // Offline, only what's loaded or downloaded can be opened.
                if offline && loaded.is_none() && self.dl.file(&issue.path).is_none() {
                    continue;
                }
                let cover = loaded.and_then(|i| self.chapter_covers.get(&i).cloned()).or_else(|| {
                    let path = library::chapter_cover_path(&r.series_key, issue.idx);
                    path.exists().then(|| widget::image::Handle::from_path(path))
                });
                rows.push(Row {
                    number: issue.idx + 1,
                    title: comic::display_name(&issue.name),
                    pages: loaded.map_or(issue.page_count, |i| self.chapters[i].page_count),
                    volume: naming::volume_of(&issue.name),
                    cover,
                    current: loaded.is_some() && loaded == current_ch,
                    status: status(Some(issue)),
                    on_press: match loaded {
                        Some(i) => Message::SelectChapter(i),
                        None => Message::OpenEntry { key: r.series_key.clone(), chapter: Some(issue.idx) },
                    },
                });
            }
            return rows;
        }

        // Local: every chapter is loaded already.
        let issues: std::collections::HashMap<&str, &library::IssueEntry> =
            self.reader_issues.iter().map(|e| (e.path.as_str(), e)).collect();
        self.chapters
            .iter()
            .enumerate()
            .map(|(i, ch)| {
                let path = ch.path.to_string_lossy();
                let issue = issues.get(path.as_ref()).copied();
                Row {
                    number: i + 1,
                    title: ch.name.clone(),
                    pages: ch.page_count,
                    volume: naming::volume_of(&ch.path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default())
                        .or_else(|| naming::volume_of(&ch.name)),
                    cover: self.chapter_covers.get(&i).cloned(),
                    current: current_ch == Some(i),
                    status: status(issue),
                    on_press: Message::SelectChapter(i),
                }
            })
            .collect()
    }

    /// Opens or closes the panel; opening scrolls to the current chapter.
    pub(super) fn toggle_chapter_list(&mut self) -> Task<Message> {
        self.show_chapter_select = !self.show_chapter_select;
        if !self.show_chapter_select {
            return Task::none();
        }
        self.chapter_group = None;
        let current = self.chapter_rows().iter().position(|r| r.current).unwrap_or(0);
        // Leave the chapter before it in view too.
        let scroll = scroll_to_row(current.saturating_sub(1));
        Task::batch([self.queue_missing_covers(), scroll])
    }

    /// Jumps the list to a volume or block.
    pub(super) fn jump_to_chapter_group(&mut self, group: usize) -> Task<Message> {
        let rows = self.chapter_rows();
        let Some((_, first)) = chapter_groups(&rows, self.manga_names()).get(group).cloned() else {
            return Task::none();
        };
        self.chapter_group = Some(group);
        scroll_to_row(first)
    }

    /// Whether this series names its chapters like manga ("Ch. 12").
    fn manga_names(&self) -> bool {
        self.reader_issues.iter().any(|i| {
            let n = i.name.to_ascii_lowercase();
            n.starts_with("ch") || n.contains("chapter")
        })
    }

    pub(super) fn build_chapter_select(&self) -> Element<'_, Message> {
        let rows = self.chapter_rows();
        let manga = self.manga_names();
        let groups = chapter_groups(&rows, manga);
        let current = rows.iter().position(|r| r.current);

        let series = self.reader_entry.as_ref().map(|e| e.name().to_string()).unwrap_or_default();
        let position = match current {
            Some(i) => format!("{} {} of {}", if manga { "Ch." } else { "#" }, rows[i].number, self.reader_issues.len().max(rows.len())),
            None => format!("{} chapters", rows.len()),
        };

        let mut col = widget::Column::new().spacing(0);
        col = col.push(
            widget::Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .padding(12)
                .push(
                    widget::Column::new()
                        .width(Length::Fill)
                        .push(widget::text::title4("Chapters"))
                        .push(widget::text::caption(if series.is_empty() { position } else { format!("{series}  ·  {position}") })),
                )
                .push(widget::button::icon(icon::from_name("window-close-symbolic")).on_press(Message::ToggleChapterSelect)),
        );

        // Jump to a volume or a block of chapters.
        if groups.len() > 1 {
            let labels: Vec<String> = groups.iter().map(|(label, _)| label.clone()).collect();
            let selected = self.chapter_group.or_else(|| {
                let cur = current?;
                groups.iter().rposition(|(_, first)| *first <= cur)
            });
            col = col.push(
                widget::container(widget::dropdown(labels, selected, Message::ChapterGroup).width(Length::Fill))
                    .padding([0, 12, 10, 12]),
            );
        }
        col = col.push(widget::divider::horizontal::light());

        let mut list = widget::Column::new().spacing(ROW_GAP).padding(LIST_PAD);
        let mut last_volume = None;
        for row in rows {
            let cover: Element<'_, Message> = match row.cover {
                Some(h) => widget::image(h).width(Length::Fixed(60.0)).height(Length::Fixed(85.0)).content_fit(ContentFit::Cover).into(),
                None => widget::container(widget::text("…").size(20))
                    .width(Length::Fixed(60.0))
                    .height(Length::Fixed(85.0))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                    .style(|_: &cosmic::Theme| widget::container::Style {
                        background: Some(Background::Color(Color::from_rgba(0.5, 0.5, 0.5, 0.2))),
                        ..Default::default()
                    })
                    .into(),
            };
            let title = if row.title.chars().count() > 30 {
                format!("{}…", row.title.chars().take(28).collect::<String>())
            } else {
                row.title
            };
            // "Volume 3 · Chapter 17"; a chapter without a volume of its own
            // belongs to the one before it.
            let volume = row.volume.or(last_volume);
            let number = format!("{} {}", if manga { "Chapter" } else { "#" }, row.number);
            let heading = match volume {
                Some(v) => format!("Volume {v}  ·  {number}"),
                None => number,
            };
            last_volume = volume;
            let mut facts = vec![format!("{} pages", row.pages)];
            if row.pages == 0 {
                facts.clear();
            }
            if let Some(s) = row.status {
                facts.push(s);
            }

            let content = widget::Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(cover)
                .push(
                    widget::Column::new()
                        .spacing(2)
                        .push(widget::text(heading).size(11))
                        .push(widget::text(title).size(if row.current { 14 } else { 13 }))
                        .push(widget::text(facts.join("  ·  ")).size(11)),
                );
            let boxed = widget::container(content).padding(6).width(Length::Fill).height(Length::Fixed(ROW_H));
            let el: Element<'_, Message> = if row.current {
                boxed
                    .style(|_: &cosmic::Theme| widget::container::Style {
                        background: Some(Background::Color(Color::from_rgba(0.3, 0.5, 1.0, 0.15))),
                        ..Default::default()
                    })
                    .into()
            } else {
                mouse_area(boxed)
                    .on_press(row.on_press)
                    .interaction(cosmic::iced::mouse::Interaction::Pointer)
                    .into()
            };
            list = list.push(el);
        }
        col = col.push(widget::scrollable::scrollable(list).id(list_id()).height(Length::Fill));

        // Its own background: in theater mode libcosmic's content frame is
        // off, and without this the window shows through the panel.
        widget::container(col)
            .width(Length::Fixed(300.0))
            .height(Length::Fill)
            .style(|theme: &cosmic::Theme| widget::container::Style {
                background: Some(Background::Color(theme.cosmic().bg_color().into())),
                ..Default::default()
            })
            .into()
    }
}

fn scroll_to_row(row: usize) -> Task<Message> {
    let y = if row == 0 { 0.0 } else { LIST_PAD + row as f32 * (ROW_H + ROW_GAP) };
    scroll_mod::scroll_to(list_id(), AbsoluteOffset { x: None, y: Some(y) })
}

/// The jump-to groups: (label, index of the first row). Volumes when most
/// chapters name one; otherwise blocks of 25 once there are more than 25.
fn chapter_groups(rows: &[Row], manga: bool) -> Vec<(String, usize)> {
    group_numbers(&rows.iter().map(|r| (r.number, r.volume)).collect::<Vec<_>>(), manga)
}

/// [`chapter_groups`] on (number, volume) pairs, testable without widgets.
fn group_numbers(rows: &[(usize, Option<u32>)], manga: bool) -> Vec<(String, usize)> {
    let prefix = if manga { "Ch. " } else { "#" };
    let range = |a: usize, b: usize| if a == b { format!("{prefix}{a}") } else { format!("{prefix}{a}–{b}") };
    let with_volume = rows.iter().filter(|(_, v)| v.is_some()).count();
    if rows.len() > 1 && with_volume * 4 >= rows.len() * 3 {
        // Consecutive runs of the same volume; a chapter without one stays
        // with the volume before it.
        let mut groups: Vec<(Option<u32>, usize, usize)> = Vec::new();
        let mut volume = None;
        for (i, (_, v)) in rows.iter().enumerate() {
            let v = v.or(volume);
            match groups.last_mut() {
                Some((gv, _, end)) if *gv == v => *end = i,
                _ => groups.push((v, i, i)),
            }
            volume = v;
        }
        return groups
            .into_iter()
            .map(|(v, start, end)| {
                let chapters = range(rows[start].0, rows[end].0);
                let label = match v {
                    Some(v) => format!("Volume {v}  ({chapters})"),
                    None => chapters,
                };
                (label, start)
            })
            .collect();
    }
    if rows.len() <= BLOCK {
        return Vec::new();
    }
    rows.chunks(BLOCK)
        .enumerate()
        .map(|(i, chunk)| (range(chunk[0].0, chunk[chunk.len() - 1].0), i * BLOCK))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::group_numbers;

    #[test]
    fn blocks_without_volumes() {
        let rows: Vec<_> = (1..=60).map(|n| (n, None)).collect();
        let groups = group_numbers(&rows, true);
        assert_eq!(groups, [("Ch. 1–25".to_string(), 0), ("Ch. 26–50".to_string(), 25), ("Ch. 51–60".to_string(), 50)]);
        assert!(group_numbers(&rows[..20], true).is_empty());
    }

    #[test]
    fn volumes_when_named() {
        let rows = [(1, Some(1)), (2, Some(1)), (3, None), (4, Some(2)), (5, Some(2))];
        let groups = group_numbers(&rows, true);
        assert_eq!(groups, [("Volume 1  (Ch. 1–3)".to_string(), 0), ("Volume 2  (Ch. 4–5)".to_string(), 3)]);
    }

    #[test]
    fn too_few_volumes_falls_back_to_blocks() {
        let mut rows: Vec<_> = (1..=30).map(|n| (n, None)).collect();
        rows[0].1 = Some(1);
        assert_eq!(group_numbers(&rows, false)[1], ("#26–30".to_string(), 25));
    }
}
