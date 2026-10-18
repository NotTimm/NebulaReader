//! Library grid and the per-series page (issue list + tracking).

use std::path::Path;

use cosmic::iced::widget::mouse_area;
use cosmic::iced::{Alignment, Background, Color, ContentFit, Length};
use cosmic::widget::{self, icon};
use cosmic::Element;

use super::{error_banner, App, Message};
use crate::library::{IssueEntry, SeriesEntry};
use crate::settings::LibrarySource;
use crate::scan::{THUMB_H, THUMB_W};

/// Shelf cards are shown at half the thumbnail resolution (crisp on HiDPI).
const CARD_W: f32 = THUMB_W as f32 / 2.0;
const CARD_H: f32 = THUMB_H as f32 / 2.0;

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>().trim_end())
    }
}

fn progress_bar<'a>(fraction: f32) -> Element<'a, Message> {
    cosmic::iced::widget::ProgressBar::new(0.0..=1.0, fraction).girth(4.0).into()
}

fn tint(alpha: f32) -> impl Fn(&cosmic::Theme) -> widget::container::Style {
    move |theme: &cosmic::Theme| {
        let accent: Color = theme.cosmic().accent_color().into();
        widget::container::Style {
            background: Some(Background::Color(Color { a: alpha, ..accent })),
            border: cosmic::iced::Border { radius: 6.0.into(), ..Default::default() },
            ..Default::default()
        }
    }
}

fn placeholder_cover<'a>(w: f32, h: f32, icon_name: &'static str) -> Element<'a, Message> {
    widget::container(icon::from_name(icon_name).size(40))
        .width(Length::Fixed(w))
        .height(Length::Fixed(h))
        .center_x(Length::Fixed(w))
        .center_y(Length::Fixed(h))
        .style(|_: &cosmic::Theme| widget::container::Style {
            background: Some(Background::Color(Color::from_rgba(0.5, 0.5, 0.5, 0.15))),
            border: cosmic::iced::Border { radius: 4.0.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

/// Short status line for a shelf card.
fn card_subtitle(e: &SeriesEntry) -> String {
    if e.is_epub {
        // The author, like a bookshop label; "Book" when unknown.
        let author = e.creators.lines().next().and_then(|l| l.split_once(": ")).map(|(_, names)| names);
        let what = author.unwrap_or("Book");
        return if e.has_progress() { format!("{what} · ch. {}", e.last_read_chapter + 1) } else { what.to_string() };
    }
    if e.issue_count > 1 {
        if e.is_finished() {
            return format!("{} issues · finished", e.issue_count);
        }
        if e.issues_read > 0 {
            return format!("{}/{} read", e.issues_read, e.issue_count);
        }
        return format!("{} issues", e.issue_count);
    }
    if e.is_finished() {
        return "Finished".into();
    }
    if e.total_pages > 0 { format!("{} pages", e.total_pages) } else { String::new() }
}

/// "Continue" line: which issue you're on and how far.
fn continue_label(e: &SeriesEntry) -> String {
    if e.is_epub {
        return format!("Chapter {}", e.last_read_chapter + 1);
    }
    if e.issue_count > 1 && !e.current_issue_label.is_empty() {
        let n = &e.current_issue_label;
        let label = if n.chars().all(|c| c.is_ascii_digit() || c == '.') { format!("#{n}") } else { truncate(n, 14) };
        // A Jellyfin chapter has no page count until it is opened.
        if e.current_issue_fraction <= 0.0 {
            return label;
        }
        return format!("{label} · {:.0}%", e.current_issue_fraction * 100.0);
    }
    format!("{:.0}%", e.progress_fraction() * 100.0)
}

impl App {
    /// The All / Local / Jellyfin picker at the left of the header: a pill
    /// with the current choice's icon, opening a menu of all three.
    pub(super) fn library_source_picker(&self) -> Element<'_, Message> {
        let current = self.settings.library_source;
        let label = |src: LibrarySource| {
            LibrarySource::ALL.iter().position(|s| *s == src).map_or("", |i| LibrarySource::LABELS[i])
        };

        let pill = widget::button::custom(
            widget::row::with_capacity(3)
                .spacing(8)
                .align_y(Alignment::Center)
                .push(widget::icon(source_icon(current)).size(16))
                .push(widget::text::body(label(current)))
                .push(widget::icon(ui_icon(CHEVRON_DOWN)).size(12)),
        )
        .class(cosmic::theme::Button::Standard)
        .padding([4, 12])
        .on_press(Message::ToggleLibraryMenu);

        let mut picker = widget::popover(pill)
            .position(widget::popover::Position::Bottom)
            .on_close(Message::CloseLibraryMenu);
        if self.library_menu {
            let mut menu = widget::column::with_capacity(LibrarySource::ALL.len()).spacing(2);
            for (i, src) in LibrarySource::ALL.into_iter().enumerate() {
                let mut row = widget::row::with_capacity(3)
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .push(widget::icon(source_icon(src)).size(16))
                    .push(widget::text::body(LibrarySource::LABELS[i]).width(Length::Fill));
                if src == current {
                    row = row.push(widget::icon(ui_icon(CHECK)).size(16));
                }
                menu = menu.push(
                    widget::button::custom(row)
                        .class(cosmic::theme::Button::MenuItem)
                        .padding([6, 10])
                        .width(Length::Fill)
                        .on_press(Message::LibrarySource(i)),
                );
            }
            picker = picker.popup(
                widget::container(menu)
                    .padding(4)
                    .width(Length::Fixed(200.0))
                    .class(cosmic::theme::Container::Dropdown),
            );
        }
        picker.into()
    }

    /// Whether the library has both local and Jellyfin series, which is
    /// when the All / Local / Jellyfin dropdown shows.
    pub(super) fn has_local_and_remote(&self) -> bool {
        let mut entries = self.library_entries.iter();
        entries.clone().any(is_remote) && entries.any(|e| !is_remote(e))
    }

    pub(super) fn view_library(&self) -> Element<'_, Message> {
        let mut col = widget::Column::new().spacing(20).padding([16, 20]);

        if let Some(err) = self.error.as_deref() {
            col = col.push(error_banner(err));
        }

        if self.scans_pending > 0 || self.meta_running {
            let mut status = widget::Row::new().spacing(8).align_y(Alignment::Center);
            if self.scans_pending > 0 {
                status = status.push(widget::text::caption(format!(
                    "Scanning library… ({} left)",
                    self.scans_pending
                )));
            }
            if self.meta_running {
                status = status.push(widget::text::caption(format!(
                    "Looking up metadata… ({} queued)",
                    self.meta_queue.len() + 1
                )));
            }
            col = col.push(status);
        }

        let visible: Vec<&SeriesEntry> =
            self.library_entries.iter().filter(|e| self.is_visible(e)).collect();

        // Narrowed by the All / Local / Jellyfin dropdown in the header.
        let visible: Vec<&SeriesEntry> = if self.has_local_and_remote() {
            match self.settings.library_source {
                LibrarySource::All => visible,
                LibrarySource::Local => visible.into_iter().filter(|e| !is_remote(e)).collect(),
                LibrarySource::Jellyfin => visible.into_iter().filter(|e| is_remote(e)).collect(),
            }
        } else {
            visible
        };

        if visible.is_empty() {
            let msg = if self.library_search.is_empty() {
                "Add a library folder in Settings, open a file, or drag one in."
            } else {
                "Nothing matches that search."
            };
            col = col.push(
                widget::container(
                    widget::Column::new()
                        .spacing(12)
                        .align_x(Alignment::Center)
                        .push(icon::from_name("library-symbolic").size(64))
                        .push(widget::text::title3(if self.library_search.is_empty() {
                            "Your library is empty"
                        } else {
                            "No results"
                        }))
                        .push(widget::text::body(msg))
                        .push(
                            widget::Row::new()
                                .spacing(8)
                                .push(widget::button::standard("Open File").on_press(Message::OpenFile))
                                .push(widget::button::standard("Open Folder").on_press(Message::OpenFolder))
                                .push(
                                    widget::button::suggested("Add Library Folder")
                                        .on_press(Message::AddLibraryFolder),
                                ),
                        ),
                )
                .width(Length::Fill)
                .center_x(Length::Fill)
                .padding(40),
            );
            return widget::container(col).width(Length::Fill).height(Length::Fill).into();
        }

        let mut body = widget::Column::new().spacing(24);

        // ── Continue Reading ──────────────────────────────────────────────
        if self.library_search.is_empty() {
            let mut recent: Vec<&SeriesEntry> = visible
                .iter()
                .copied()
                .filter(|e| e.has_progress() && !e.is_finished() && e.last_opened > 0)
                .collect();
            recent.sort_by_key(|e| std::cmp::Reverse(e.last_opened));
            if !recent.is_empty() {
                let mut row = widget::Row::new().spacing(16);
                for e in recent.into_iter().take(10) {
                    row = row.push(self.series_card(e, true));
                }
                body = body.push(widget::text::title3("Continue Reading")).push(
                    widget::scrollable::horizontal(row.padding([0, 0, 12, 0])),
                );
            }
        }

        // ── Sections per library folder, then everything else ─────────────
        let mut sections: Vec<(String, Vec<&SeriesEntry>)> = Vec::new();
        for dir in &self.library_dirs {
            let root = dir.to_string_lossy().into_owned();
            let mut items: Vec<&SeriesEntry> = visible
                .iter()
                .copied()
                .filter(|e| e.library_root == root || (e.library_root.is_empty() && Path::new(&e.path).starts_with(dir)))
                .collect();
            if items.is_empty() {
                continue;
            }
            items.sort_by_key(|e| e.name().to_lowercase());
            let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(root);
            sections.push((name, items));
        }
        // One section per Jellyfin library ("Manga · deepdish").
        let mut remote_roots: Vec<&str> = visible
            .iter()
            .map(|e| e.library_root.as_str())
            .filter(|r| crate::jellyfin::is_key(r))
            .collect();
        remote_roots.sort_unstable();
        remote_roots.dedup();
        for root in remote_roots {
            let mut items: Vec<&SeriesEntry> = visible.iter().copied().filter(|e| e.library_root == root).collect();
            items.sort_by_key(|e| e.name().to_lowercase());
            let name = self.jf.library_names.get(root).cloned().unwrap_or_else(|| "Jellyfin".into());
            sections.push((name, items));
        }

        let mut other: Vec<&SeriesEntry> = visible
            .iter()
            .copied()
            .filter(|e| {
                e.library_root.is_empty() && !self.library_dirs.iter().any(|d| Path::new(&e.path).starts_with(d))
            })
            .collect();
        if !other.is_empty() {
            other.sort_by_key(|e| std::cmp::Reverse(e.last_opened));
            sections.push((if sections.is_empty() { "Library".into() } else { "Opened Elsewhere".into() }, other));
        }

        for (name, items) in sections {
            let count = items.len();
            body = body
                .push(
                    widget::Row::new()
                        .spacing(8)
                        .align_y(Alignment::End)
                        .push(widget::text::title3(name))
                        .push(widget::text::caption(format!("{count}"))),
                )
                .push(
                    widget::flex_row(items.into_iter().map(|e| self.series_card(e, false)).collect())
                        .row_spacing(20)
                        .column_spacing(16)
                        .width(Length::Fill),
                );
        }

        col = col.push(widget::scrollable(body.padding([0, 12, 24, 0])).height(Length::Fill));
        widget::container(col).width(Length::Fill).height(Length::Fill).into()
    }

    fn is_visible(&self, e: &SeriesEntry) -> bool {
        if self.hidden_paths.contains(&e.path) {
            return false;
        }
        !self.missing_roots.iter().any(|r| {
            let r = r.to_string_lossy();
            e.library_root == r || (e.library_root.is_empty() && Path::new(&e.path).starts_with(r.as_ref()))
        })
    }

    fn series_card<'a>(&'a self, e: &'a SeriesEntry, continuing: bool) -> Element<'a, Message> {
        let cover: Element<'_, Message> = match self.library_covers.get(&e.id) {
            Some(h) => widget::image(h.clone())
                .width(Length::Fixed(CARD_W))
                .height(Length::Fixed(CARD_H))
                .content_fit(ContentFit::Cover)
                .border_radius([4.0; 4])
                .into(),
            None => placeholder_cover(
                CARD_W,
                CARD_H,
                if e.is_epub { "x-office-document-symbolic" } else { "image-x-generic-symbolic" },
            ),
        };

        let mut card = widget::Column::new()
            .spacing(4)
            .width(Length::Fixed(CARD_W))
            .push(cover)
            .push(widget::text::body(truncate(e.name(), 34)).width(Length::Fixed(CARD_W)));

        if continuing {
            card = card.push(progress_bar(e.progress_fraction()));
            card = card.push(widget::text::caption(continue_label(e)));
        } else {
            let sub = card_subtitle(e);
            if !sub.is_empty() {
                card = card.push(widget::text::caption(sub));
            }
            if e.has_progress() && !e.is_finished() {
                card = card.push(progress_bar(e.progress_fraction()));
            }
        }

        let msg = if continuing || e.issue_count <= 1 {
            Message::OpenEntry { key: e.path.clone(), chapter: None }
        } else {
            Message::OpenSeriesPage(e.path.clone())
        };
        mouse_area(card).on_press(msg).interaction(cosmic::iced::mouse::Interaction::Pointer).into()
    }

    // ── Series page ───────────────────────────────────────────────────────

    pub(super) fn view_series(&self) -> Element<'_, Message> {
        let Some(series) = &self.series_page else {
            return self.view_library();
        };
        let issues = &self.series_page_issues;
        let manga = issues.iter().any(|i| {
            let n = i.name.to_ascii_lowercase();
            n.starts_with("ch") || n.contains("chapter")
        });

        let mut col = widget::Column::new().spacing(16).padding([16, 20]);
        if let Some(err) = self.error.as_deref() {
            col = col.push(error_banner(err));
        }

        // ── Header: cover + details + actions ─────────────────────────────
        let cover: Element<'_, Message> = match self.series_page_covers.get(&0) {
            Some(h) => widget::image(h.clone())
                .width(Length::Fixed(THUMB_W as f32 * 0.75))
                .height(Length::Fixed(THUMB_H as f32 * 0.75))
                .content_fit(ContentFit::Cover)
                .border_radius([6.0; 4])
                .into(),
            None => placeholder_cover(THUMB_W as f32 * 0.75, THUMB_H as f32 * 0.75, "image-x-generic-symbolic"),
        };

        let mut facts: Vec<String> = Vec::new();
        if !series.publisher.is_empty() {
            facts.push(series.publisher.clone());
        }
        match (series.year, series.end_year) {
            (Some(y), Some(e)) if e > y => facts.push(format!("{y}–{e}")),
            (Some(y), _) => facts.push(y.to_string()),
            _ => {}
        }
        if let Some(v) = series.volume {
            facts.push(format!("Vol. {v}"));
        }
        facts.push(format!("{} {}", issues.len(), if manga { "chapters" } else { "issues" }));
        let read = issues.iter().filter(|i| i.completed).count();
        if read > 0 {
            facts.push(format!("{read} read"));
        }

        let mut details = widget::Column::new()
            .spacing(8)
            .width(Length::Fill)
            .push(widget::text::title2(series.name().to_string()))
            .push(widget::text::body(facts.join("  ·  ")));
        for line in series.creators.lines() {
            details = details.push(widget::text::body(line.to_string()));
        }

        if !series.display_title.is_empty() && series.display_title != series.title {
            details = details.push(widget::text::caption(format!("On disk as “{}”", series.title)));
        }

        let source_line = match series.source.as_str() {
            "comicvine" => "Matched on ComicVine".to_string(),
            "anilist" => "Matched on AniList".to_string(),
            _ if series.meta_status == "nomatch" => "No online match — using the folder name".to_string(),
            _ if series.meta_status.starts_with("error") => format!("Lookup failed: {}", &series.meta_status[6..].trim()),
            _ if self.meta_running && self.meta_current.as_deref() == Some(series.path.as_str()) => {
                "Looking up metadata…".to_string()
            }
            _ => "Not looked up yet".to_string(),
        };
        let mut source_row = widget::Row::new().spacing(8).align_y(Alignment::Center).push(widget::text::caption(source_line));
        if !series.source_url.is_empty() {
            source_row = source_row.push(
                widget::button::link("View page").on_press(Message::OpenUrl(series.source_url.clone())),
            );
        }
        details = details.push(source_row);

        if !series.genres.is_empty() {
            details = details.push(widget::text::caption(series.genres.clone()));
        }
        if !series.description.is_empty() {
            details = details.push(
                widget::container(widget::text::body(truncate(&series.description, 700)))
                    .max_width(760.0),
            );
        }

        // Primary action: continue where you left off, or start.
        let next = next_issue(issues);
        let primary_label = match next {
            Some(i) if i.in_progress() => format!("Continue {}", i.label(manga)),
            Some(i) if read > 0 => format!("Read {}", i.label(manga)),
            Some(_) => "Start Reading".to_string(),
            None => "Read Again".to_string(),
        };
        let primary_idx = next.map(|i| i.idx).unwrap_or(0);
        let mut actions = widget::Row::new()
            .spacing(8)
            .push(
                widget::button::suggested(primary_label)
                    .on_press(Message::OpenEntry { key: series.path.clone(), chapter: Some(primary_idx) }),
            );
        if read < issues.len() {
            actions = actions.push(widget::button::standard("Mark All Read").on_press(Message::MarkSeriesRead(true)));
        }
        if read > 0 || issues.iter().any(|i| i.in_progress()) {
            actions = actions.push(widget::button::standard("Mark All Unread").on_press(Message::MarkSeriesRead(false)));
        }
        details = details.push(actions);

        // Fix-the-match controls.
        details = details.push(
            widget::Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(
                    widget::text_input("Search online as…", &self.series_search_input)
                        .on_input(Message::SeriesSearchInput)
                        .on_submit(|_| Message::RematchSeries)
                        .width(Length::Fixed(260.0)),
                )
                .push(widget::button::standard("Re-match").on_press(Message::RematchSeries))
                .push_maybe((!series.source.is_empty()).then(|| {
                    widget::button::text("Use Folder Name").on_press(Message::ClearSeriesMatch)
                })),
        );

        col = col.push(widget::Row::new().spacing(24).push(cover).push(details));
        col = col.push(widget::divider::horizontal::default());

        // ── Issue list ────────────────────────────────────────────────────
        let mut list = widget::Column::new().spacing(4);
        for issue in issues {
            list = list.push(self.issue_row(series, issue, manga));
        }
        col = col.push(widget::scrollable(list.padding([0, 12, 24, 0])).height(Length::Fill));

        widget::container(col).width(Length::Fill).height(Length::Fill).into()
    }

    fn issue_row<'a>(&'a self, series: &'a SeriesEntry, issue: &'a IssueEntry, manga: bool) -> Element<'a, Message> {
        let thumb: Element<'_, Message> = match self.series_page_covers.get(&issue.idx) {
            Some(h) => widget::image(h.clone())
                .width(Length::Fixed(54.0))
                .height(Length::Fixed(76.0))
                .content_fit(ContentFit::Cover)
                .border_radius([3.0; 4])
                .into(),
            None => placeholder_cover(54.0, 76.0, "image-x-generic-symbolic"),
        };

        let heading = issue.heading();
        let title_line = if heading.is_empty() || issue.number.is_empty() {
            if issue.number.is_empty() { issue.name.clone() } else { issue.label(manga) }
        } else {
            format!("{}  —  {}", issue.label(manga), heading)
        };

        let mut meta: Vec<String> = Vec::new();
        if !issue.date().is_empty() {
            meta.push(crate::library::pretty_date(issue.date()));
        }
        if issue.page_count > 0 {
            meta.push(format!("{} pages", issue.page_count));
        }
        // Show the file name only when it adds something: not for an
        // official title, and not for manga-tui chapter files ("Ch 3 Vol
        // none Weeb central 01J7…"), whose number and title are shown above.
        if !issue.title.is_empty() || issue.number.is_empty() || crate::naming::starts_with_chapter(&issue.name) {
        } else {
            meta.push(truncate(&issue.name, 60));
        }

        let status: Element<'_, Message> = if issue.completed {
            widget::text::caption("✓ Read").into()
        } else if issue.in_progress() {
            widget::Column::new()
                .spacing(2)
                .width(Length::Fixed(120.0))
                .push(progress_bar(issue.fraction()))
                .push(widget::text::caption(format!("Page {} of {}", issue.last_page + 1, issue.page_count.max(1))))
                .into()
        } else {
            widget::text::caption("Unread").into()
        };

        let toggle = widget::tooltip(
            widget::button::icon(icon::from_name(if issue.completed {
                "edit-undo-symbolic"
            } else {
                "object-select-symbolic"
            }))
            .on_press(Message::ToggleIssueRead(issue.id, !issue.completed)),
            widget::text(if issue.completed { "Mark unread" } else { "Mark read" }),
            widget::tooltip::Position::Left,
        );

        let is_current = series.last_issue_path == issue.path && !issue.completed;
        let info = widget::Column::new()
            .spacing(2)
            .width(Length::Fill)
            .push(if issue.completed {
                widget::text::body(title_line)
            } else {
                widget::text::heading(title_line)
            })
            .push_maybe((!meta.is_empty()).then(|| widget::text::caption(meta.join("  ·  "))));

        let row = widget::Row::new()
            .spacing(12)
            .align_y(Alignment::Center)
            .padding([6, 8])
            .push(thumb)
            .push(info)
            .push(status)
            .push(toggle);

        let row: Element<'_, Message> = if is_current {
            widget::container(row).style(tint(0.12)).width(Length::Fill).into()
        } else {
            row.into()
        };

        mouse_area(row)
            .on_press(Message::OpenEntry { key: series.path.clone(), chapter: Some(issue.idx) })
            .interaction(cosmic::iced::mouse::Interaction::Pointer)
            .into()
    }
}

/// The issue to suggest next: the in-progress one, else the first unread
/// after the last read one, else the first unread at all.
pub(super) fn next_issue(issues: &[IssueEntry]) -> Option<&IssueEntry> {
    if let Some(i) = issues.iter().filter(|i| i.in_progress()).max_by_key(|i| i.last_read_at) {
        return Some(i);
    }
    let last_read = issues.iter().filter(|i| i.completed).max_by_key(|i| (i.last_read_at, i.idx));
    if let Some(lr) = last_read {
        if let Some(n) = issues.iter().find(|i| i.idx > lr.idx && !i.completed) {
            return Some(n);
        }
    }
    issues.iter().find(|i| !i.completed)
}

fn is_remote(e: &SeriesEntry) -> bool {
    crate::jellyfin::is_key(&e.library_root) || crate::jellyfin::is_key(&e.path)
}

const ALL_ICON: &[u8] = include_bytes!("../../res/icons/ui/all.svg");
const LOCAL_ICON: &[u8] = include_bytes!("../../res/icons/ui/local.svg");
const JELLYFIN_ICON: &[u8] = include_bytes!("../../res/icons/ui/jellyfin.svg");
const CHEVRON_DOWN: &[u8] = include_bytes!("../../res/icons/ui/chevron-down.svg");
const CHECK: &[u8] = include_bytes!("../../res/icons/ui/check.svg");

/// A single-colour icon of ours, drawn in the theme's text colour.
fn ui_icon(svg: &'static [u8]) -> widget::icon::Handle {
    let mut handle = widget::icon::from_svg_bytes(svg);
    handle.symbolic = true;
    handle
}

/// The Jellyfin logo, in the theme's text colour.
pub(super) fn jellyfin_icon() -> widget::icon::Handle {
    ui_icon(JELLYFIN_ICON)
}

fn source_icon(src: LibrarySource) -> widget::icon::Handle {
    match src {
        LibrarySource::All => ui_icon(ALL_ICON),
        LibrarySource::Local => ui_icon(LOCAL_ICON),
        LibrarySource::Jellyfin => jellyfin_icon(),
    }
}
