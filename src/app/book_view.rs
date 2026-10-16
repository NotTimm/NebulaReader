//! The book (EPUB) reader: typography, themes, paging and position memory.
//!
//! Layout follows print-book conventions rather than web ones: a fixed
//! reading measure (~66 characters per line at the default width),
//! generous line height, first-line indents instead of blank lines between
//! paragraphs (no indent after a heading or scene break), justified text,
//! and chapter openings set lower on the page with large, centered titles.

use cosmic::iced::font::{Family, Style as FontStyle, Weight};
use cosmic::iced::widget::scrollable::{self as scroll_mod, AbsoluteOffset, RelativeOffset, Viewport};
use cosmic::iced::widget::text::{LineHeight, Span};
use cosmic::iced::widget::{mouse_area, rich_text, span};
use cosmic::iced::{Alignment, Background, Color, ContentFit, Font, Length};
use cosmic::widget::{self, icon};
use cosmic::{app::Task, Element};

use super::paged::{BoxKind, Paged};
use super::{tip, App, ContextPage, Message};
use crate::epub::{Align, Block, Inline};
use crate::library;
use crate::settings::{BookFont, BookLayout, BookSettings, BookTheme, BookWidth};

/// Reading speed used for "time left" estimates.
const WORDS_PER_MINUTE: f32 = 250.0;

pub(super) fn book_scroll_id() -> widget::Id {
    widget::Id::new("book-scroll")
}

#[derive(Clone, Debug)]
pub enum BookMsg {
    Scrolled(Viewport),
    PageDown,
    PageUp,
    LineDown,
    LineUp,
    NextChapter,
    PrevChapter,
    GoToChapter(usize),
    FontBigger,
    FontSmaller,
    FontReset,
    SetFontSize(f32),
    SetFont(usize),
    SetSpacing(usize),
    SetWidth(usize),
    SetTheme(usize),
    SetJustify(bool),
    SetLayout(usize),
    /// Show a book image full-window, zoomable (maps, photos).
    OpenImage(usize),
    CloseImage,
    ToggleTypography,
    /// The page layout changed (resize, new chapter, new font); just redraw.
    Repaginated,
}

/// Where the book view's scrollable currently is.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct BookScroll {
    pub rel_y: f32,
    pub viewport_h: f32,
    pub content_h: f32,
}

struct Palette {
    bg: Color,
    fg: Color,
    muted: Color,
}

fn palette(theme: BookTheme, cosmic_theme: &cosmic::Theme) -> Palette {
    let rgb = |r: u8, g: u8, b: u8| Color::from_rgb8(r, g, b);
    let (bg, fg) = match theme {
        BookTheme::Auto => {
            let c = cosmic_theme.cosmic();
            (c.bg_color().into(), c.on_bg_color().into())
        }
        BookTheme::Paper => (rgb(0xfb, 0xfa, 0xf7), rgb(0x1f, 0x1f, 0x1f)),
        BookTheme::Sepia => (rgb(0xf4, 0xec, 0xd8), rgb(0x5b, 0x46, 0x36)),
        BookTheme::Dark => (rgb(0x1e, 0x1e, 0x1e), rgb(0xda, 0xda, 0xda)),
        BookTheme::Night => (Color::BLACK, rgb(0xa8, 0xa8, 0xa8)),
    };
    Palette { bg, fg, muted: Color { a: 0.6, ..fg } }
}

fn base_font(f: BookFont) -> Font {
    Font {
        family: match f {
            BookFont::Serif => Family::Serif,
            BookFont::Sans => Family::SansSerif,
        },
        ..Font::DEFAULT
    }
}

/// Styled spans for one block of inline runs.
fn spans<'a>(runs: &'a [Inline], font: Font, size: f32, extra_italic: bool) -> Vec<Span<'a, (), Font>> {
    runs.iter()
        .map(|run| {
            let mut f = font;
            if run.style.italic != extra_italic {
                f.style = FontStyle::Italic;
            }
            if run.style.bold {
                f.weight = Weight::Bold;
            }
            if run.style.small_caps {
                span(run.text.to_uppercase()).font(f).size(size * 0.82)
            } else {
                span(run.text.as_str()).font(f)
            }
        })
        .collect()
}

fn heading_scale(level: u8) -> f32 {
    match level {
        1 => 2.0,
        2 => 1.65,
        3 => 1.35,
        4 => 1.15,
        _ => 1.0,
    }
}

impl App {
    fn book_palette(&self) -> Palette {
        palette(self.settings.book.theme, &cosmic::theme::active())
    }

    pub(super) fn view_epub_reader(&self) -> Element<'_, Message> {
        let book = self.epub_book.as_ref().unwrap();
        let bs = &self.settings.book;
        let pal = self.book_palette();
        let size = f32::from(bs.font_size);
        let font = base_font(bs.font);
        let body_lh = LineHeight::Relative(bs.line_spacing);
        let justify = if bs.justify {
            cosmic::iced::widget::text::Alignment::Justified
        } else {
            cosmic::iced::widget::text::Alignment::Left
        };

        let Some(chapter) = book.chapters.get(self.epub_chapter) else {
            return widget::text("This book has no chapters.").into();
        };

        let paged = bs.layout != BookLayout::Scroll;
        // Tallest an image may be: one page, so it never gets cut off.
        let page_h = if paged { self.book_pages.lock().map(|p| p.page_h).unwrap_or(0.0) } else { 0.0 };

        // ── The page ──────────────────────────────────────────────────────
        // `kinds` says, child by child, where pages may break.
        let mut page = widget::Column::new().width(Length::Fill);
        let mut kinds = Vec::new();
        // Chapter openings start a third of the way down, like print.
        page = page.push(widget::Space::new().height(Length::Fixed(size * 3.0)));
        kinds.push(BoxKind::Gap);

        let mut prev_breaks_flow = true; // start of chapter
        for (i, block) in chapter.blocks.iter().enumerate() {
            let (el, kind, top, breaks_flow): (Element<'_, Message>, BoxKind, f32, bool) = match block {
                Block::Heading { level, spans: runs } => {
                    let hs = size * heading_scale(*level);
                    let mut f = font;
                    f.weight = if *level <= 2 { Weight::Normal } else { Weight::Semibold };
                    let el = rich_text(spans(runs, f, hs, false))
                        .size(hs)
                        .line_height(LineHeight::Relative(1.25))
                        .align_x(cosmic::iced::widget::text::Alignment::Center)
                        .width(Length::Fill)
                        .into();
                    let top = if i == 0 { 0.0 } else { size * 1.2 };
                    (el, BoxKind::Lines(hs * 1.25), top, true)
                }
                Block::Paragraph { spans: runs, align, indent } => {
                    let mut sp = spans(runs, font, size, false);
                    let indented = *indent && !prev_breaks_flow && *align == Align::Default;
                    if indented {
                        // First-line indent (≈1.5em).
                        sp.insert(0, span("\u{2003}\u{2002}").font(font));
                    }
                    let align_x = match align {
                        Align::Center => cosmic::iced::widget::text::Alignment::Center,
                        Align::Right => cosmic::iced::widget::text::Alignment::Right,
                        Align::Default => justify,
                    };
                    let el = rich_text(sp)
                        .size(size)
                        .line_height(body_lh)
                        .align_x(align_x)
                        .width(Length::Fill)
                        .into();
                    // Indented paragraphs run on; others get a half line.
                    let top = if indented { size * 0.1 } else if i == 0 { 0.0 } else { size * 0.6 };
                    (el, BoxKind::Lines(size * bs.line_spacing), top, *align != Align::Default)
                }
                Block::Quote(runs) => {
                    let el = widget::container(
                        rich_text(spans(runs, font, size * 0.95, true))
                            .size(size * 0.95)
                            .line_height(body_lh)
                            .align_x(justify)
                            .width(Length::Fill),
                    )
                    .padding([0.0, size * 1.6])
                    .into();
                    (el, BoxKind::Lines(size * 0.95 * bs.line_spacing), size * 0.7, true)
                }
                Block::ListItem(runs) => {
                    let el = widget::Row::new()
                        .spacing(size * 0.5)
                        .push(widget::text("•").size(size).font(font))
                        .push(rich_text(spans(runs, font, size, false)).size(size).line_height(body_lh).width(Length::Fill))
                        .padding([0.0, 0.0, 0.0, size])
                        .into();
                    (el, BoxKind::Lines(size * bs.line_spacing), size * 0.25, true)
                }
                Block::Rule => {
                    // Next to a heading it's decoration: a short hairline.
                    // Between paragraphs it's a scene break: ⁂.
                    let near_heading = matches!(chapter.blocks.get(i.wrapping_sub(1)), Some(Block::Heading { .. }))
                        || matches!(chapter.blocks.get(i + 1), Some(Block::Heading { .. }));
                    let mark: Element<'_, Message> = if near_heading {
                        let muted = pal.muted;
                        widget::container(widget::Space::new().height(1.0).width(size * 2.5))
                            .style(move |_: &cosmic::Theme| widget::container::Style {
                                background: Some(Background::Color(muted)),
                                ..Default::default()
                            })
                            .into()
                    } else {
                        widget::text("⁂").size(size * 1.1).font(font).into()
                    };
                    let el = widget::container(mark).width(Length::Fill).center_x(Length::Fill).into();
                    (el, BoxKind::Solid, size * 0.9, true)
                }
                Block::Image(idx) => {
                    let el = match self.epub_images.get(*idx) {
                        Some(h) => {
                            let img =
                                widget::image(h.clone()).content_fit(ContentFit::ScaleDown).width(Length::Shrink);
                            let mut c = widget::container(img).width(Length::Fill).center_x(Length::Fill);
                            if page_h > 0.0 {
                                c = c.max_height(page_h);
                            }
                            c.into()
                        }
                        None => widget::Space::new().height(0).into(),
                    };
                    (el, BoxKind::Image(*idx), size * 0.8, true)
                }
            };
            if top > 0.0 {
                page = page.push(widget::Space::new().height(Length::Fixed(top)));
                kinds.push(BoxKind::Gap);
            }
            page = page.push(el);
            kinds.push(kind);
            // After a heading/break/image the next paragraph isn't indented.
            prev_breaks_flow = breaks_flow;
        }

        // ── End of chapter ────────────────────────────────────────────────
        // Paged, the next page is simply the next chapter; scrolling, a
        // button at the bottom saves a trip to the header.
        page = page.push(widget::Space::new().height(Length::Fixed(size * 3.0)));
        kinds.push(BoxKind::Gap);
        let next = book.chapters.get(self.epub_chapter + 1);
        match next {
            Some(next) if !paged => {
                page = page.push(
                    widget::container(
                        widget::button::standard(format!("Next: {}  →", truncate(&next.title, 40)))
                            .on_press(Message::Book(BookMsg::NextChapter)),
                    )
                    .width(Length::Fill)
                    .center_x(Length::Fill),
                );
                kinds.push(BoxKind::Solid);
            }
            Some(_) => {}
            None => {
                page = page.push(
                    widget::container(widget::text("The End").size(size * 1.1).font(font))
                        .width(Length::Fill)
                        .center_x(Length::Fill),
                );
                kinds.push(BoxKind::Solid);
            }
        }
        if !paged {
            page = page.push(widget::Space::new().height(Length::Fixed(size * 4.0)));
        }

        let measure = size * bs.width.ems();
        let reading: Element<'_, Message> = if paged {
            Paged::new(
                page,
                kinds,
                self.book_pages.clone(),
                self.epub_chapter,
                self.book_scroll.rel_y,
                Message::Book(BookMsg::PageDown),
                Message::Book(BookMsg::PageUp),
                Message::Book(BookMsg::Repaginated),
            )
            .measure(measure)
            .two_pages(bs.layout == BookLayout::TwoPages, size * 18.0)
            .margins(size * 2.0, size * 1.5)
            .on_image(|i| Message::Book(BookMsg::OpenImage(i)))
            .into()
        } else {
            let column = widget::container(page)
                .max_width(measure + size * 3.0)
                .padding([0.0, size * 1.5]);
            widget::scrollable(widget::container(column).width(Length::Fill).center_x(Length::Fill))
                .id(book_scroll_id())
                .on_scroll(|v| Message::Book(BookMsg::Scrolled(v)))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

        // ── Status line: where you are in the chapter / book ──────────────
        let pages = if paged { self.book_pages.lock().ok().filter(|p| p.chapter == self.epub_chapter).map(|p| p.clone()) } else { None };
        let rel = match &pages {
            Some(p) => p.read_through(self.book_scroll.rel_y),
            None => self.book_scroll.rel_y.clamp(0.0, 1.0),
        };
        let words_left = chapter.words as f32 * (1.0 - rel);
        let mins = (words_left / WORDS_PER_MINUTE).ceil() as u32;
        let total = book.total_words().max(1) as f32;
        let before: usize = book.chapters[..self.epub_chapter].iter().map(|c| c.words).sum();
        let book_pct = ((before as f32 + chapter.words as f32 * rel) / total * 100.0).clamp(0.0, 100.0);
        let left_label = if chapter.words < 50 {
            String::new()
        } else if mins <= 1 {
            "Less than a minute left in chapter".to_string()
        } else {
            format!("{mins} min left in chapter")
        };
        let status = widget::Row::new()
            .padding([4.0, size])
            .push(
                widget::text(truncate(&chapter.title, 60))
                    .size(12)
                    .width(Length::Fill),
            )
            .push(widget::text(left_label).size(12))
            .push_maybe(pages.as_ref().filter(|p| p.count() > 0).map(|p| {
                let first = p.page_at(self.book_scroll.rel_y);
                let label = if p.step() == 2 && first + 1 < p.count() {
                    format!("   Pages {}–{} of {}", first + 1, first + 2, p.count())
                } else {
                    format!("   Page {} of {}", first + 1, p.count())
                };
                widget::text(label).size(12)
            }))
            .push(widget::text(format!("   {book_pct:.0}%")).size(12));
        let muted = pal.muted;
        let status = widget::container(status).style(move |_: &cosmic::Theme| widget::container::Style {
            text_color: Some(muted),
            ..Default::default()
        });

        let (bg, fg) = (pal.bg, pal.fg);
        let content: Element<'_, Message> = widget::container(
            widget::Column::new().push(reading).push(status),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_: &cosmic::Theme| widget::container::Style {
            background: Some(Background::Color(bg)),
            text_color: Some(fg),
            ..Default::default()
        })
        .into();

        // A picture opened from the page takes over the reading area.
        let content = match self.book_image.and_then(|i| self.epub_images.get(i)) {
            Some(h) => self.view_book_image(h, bg),
            None => content,
        };

        let main_row: Element<'_, Message> = if self.show_chapter_select {
            widget::Row::new()
                .push(widget::container(content).width(Length::Fill).height(Length::Fill))
                .push(self.build_epub_chapter_select())
                .into()
        } else {
            content
        };

        let mut col = widget::Column::new();
        if let Some(err) = self.error.as_deref() {
            col = col.push(super::error_banner(err));
        }
        col.push(main_row).into()
    }

    /// One of the book's images, filling the reading area: scroll or pinch
    /// to zoom, drag to pan.
    fn view_book_image(&self, h: &widget::image::Handle, bg: Color) -> Element<'_, Message> {
        let bar = widget::Row::new()
            .spacing(8)
            .padding([6, 12])
            .align_y(Alignment::Center)
            .push(
                widget::text::caption("Scroll or pinch to zoom · drag to move · Esc to go back")
                    .width(Length::Fill),
            )
            .push(tip(
                widget::button::standard("Back to book").on_press(Message::Book(BookMsg::CloseImage)),
                "Back to the page (Esc)",
            ));
        let viewer = widget::image::viewer(h.clone())
            .content_fit(ContentFit::Contain)
            .min_scale(1.0)
            .max_scale(8.0)
            .width(Length::Fill)
            .height(Length::Fill);
        widget::container(widget::Column::new().push(bar).push(viewer))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_: &cosmic::Theme| widget::container::Style {
                background: Some(Background::Color(bg)),
                ..Default::default()
            })
            .into()
    }

    /// Table of contents, from the book's own navigation.
    pub(super) fn build_epub_chapter_select(&self) -> Element<'_, Message> {
        let book = self.epub_book.as_ref().unwrap();
        let mut col = widget::Column::new().spacing(0);

        col = col.push(
            widget::Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .padding(12)
                .push(widget::text::title4("Contents").width(Length::Fill))
                .push(tip(
                    widget::button::icon(icon::from_name("window-close-symbolic"))
                        .on_press(Message::ToggleChapterSelect),
                    "Close contents (C)",
                )),
        );
        col = col.push(widget::divider::horizontal::light());

        // The entry you're "in" is the last one at or before this chapter.
        let current = book.toc.iter().rposition(|t| t.chapter <= self.epub_chapter);
        let mut list = widget::Column::new().spacing(2).padding(8);
        for (i, entry) in book.toc.iter().enumerate() {
            let is_current = current == Some(i);
            let label = widget::text(entry.title.clone()).size(if entry.level == 0 { 14 } else { 13 });
            let row = widget::container(label)
                .padding([6.0, 6.0, 6.0, 6.0 + 14.0 * entry.level as f32])
                .width(Length::Fill);
            let row_el: Element<'_, Message> = if is_current {
                row.style(|theme: &cosmic::Theme| {
                    let accent: Color = theme.cosmic().accent_color().into();
                    widget::container::Style {
                        background: Some(Background::Color(Color { a: 0.15, ..accent })),
                        border: cosmic::iced::Border { radius: 6.0.into(), ..Default::default() },
                        ..Default::default()
                    }
                })
                .into()
            } else {
                mouse_area(row)
                    .on_press(Message::Book(BookMsg::GoToChapter(entry.chapter)))
                    .interaction(cosmic::iced::mouse::Interaction::Pointer)
                    .into()
            };
            list = list.push(row_el);
        }

        col = col.push(widget::scrollable(list).height(Length::Fill));
        widget::container(col).width(Length::Fixed(300.0)).height(Length::Fill).into()
    }

    pub(super) fn build_epub_info_panel(&self) -> Element<'_, Message> {
        let book = self.epub_book.as_ref().unwrap();
        let mut col = widget::Column::new().spacing(8).padding(12);

        if let Some(cover) = &self.epub_cover {
            col = col.push(widget::image(cover.clone()).width(Length::Fixed(160.0)).content_fit(ContentFit::Contain));
        }
        col = col.push(widget::text::title4(book.title.clone()));
        if let Some(author) = &book.author {
            col = col.push(widget::text::body(author.clone()));
        }
        col = col.push(widget::divider::horizontal::light());
        let total = book.total_words();
        col = col.push(widget::text::caption(format!(
            "{} chapters · about {} words · ~{} h to read",
            book.chapters.len(),
            (total / 1000) * 1000,
            ((total as f32 / WORDS_PER_MINUTE) / 60.0).round().max(1.0)
        )));
        if let Some(chapter) = book.chapters.get(self.epub_chapter) {
            col = col.push(widget::text::body(format!(
                "Chapter {} of {}: {}",
                self.epub_chapter + 1,
                book.chapters.len(),
                chapter.title
            )));
        }
        widget::container(widget::scrollable(col)).width(Length::Fixed(300.0)).height(Length::Fill).into()
    }

    /// The "Aa" panel: live typography controls.
    pub(super) fn build_typography_panel(&self) -> Element<'_, Message> {
        let bs = &self.settings.book;
        let size_row = widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(tip(
                widget::button::standard("A−").on_press(Message::Book(BookMsg::FontSmaller)),
                "Smaller text (Ctrl −)",
            ))
            .push(
                widget::slider(
                    f32::from(BookSettings::MIN_SIZE)..=f32::from(BookSettings::MAX_SIZE),
                    f32::from(bs.font_size),
                    |v| Message::Book(BookMsg::SetFontSize(v)),
                )
                .step(1.0)
                .width(Length::Fill),
            )
            .push(tip(
                widget::button::standard("A+").on_press(Message::Book(BookMsg::FontBigger)),
                "Larger text (Ctrl +)",
            ));

        let spacing_idx = BookSettings::LINE_SPACINGS
            .iter()
            .position(|s| (s - bs.line_spacing).abs() < 0.01);

        let section = widget::settings::section()
            .title("Text")
            .add(widget::settings::item(format!("Size  ({} px)", bs.font_size), widget::text::caption("Ctrl + / Ctrl −")))
            .add(size_row)
            .add(widget::settings::item(
                "Typeface",
                widget::dropdown(
                    &["Serif", "Sans"][..],
                    Some(match bs.font {
                        BookFont::Serif => 0,
                        BookFont::Sans => 1,
                    }),
                    |i| Message::Book(BookMsg::SetFont(i)),
                ),
            ))
            .add(widget::settings::item(
                "Line spacing",
                widget::dropdown(&BookSettings::LINE_LABELS[..], spacing_idx, |i| {
                    Message::Book(BookMsg::SetSpacing(i))
                }),
            ))
            .add(widget::settings::item(
                "Justify text",
                widget::toggler(bs.justify).on_toggle(|v| Message::Book(BookMsg::SetJustify(v))),
            ));

        let layout = widget::settings::section()
            .title("Page")
            .add(widget::settings::item(
                "Layout",
                widget::dropdown(
                    &BookLayout::LABELS[..],
                    BookLayout::ALL.iter().position(|l| *l == bs.layout),
                    |i| Message::Book(BookMsg::SetLayout(i)),
                ),
            ))
            .add(widget::settings::item(
                "Line width",
                widget::dropdown(
                    &BookWidth::LABELS[..],
                    BookWidth::ALL.iter().position(|w| *w == bs.width),
                    |i| Message::Book(BookMsg::SetWidth(i)),
                ),
            ))
            .add(widget::settings::item(
                "Theme",
                widget::dropdown(
                    &BookTheme::LABELS[..],
                    BookTheme::ALL.iter().position(|t| *t == bs.theme),
                    |i| Message::Book(BookMsg::SetTheme(i)),
                ),
            ));

        let help = widget::settings::section().title("Keys").add(widget::text::caption(
            "→ / Space / Page Down, or click the right edge: next page  ·  ← / Page Up, or click the left edge: previous page  ·  Mouse wheel: turn pages (scrolls in Scroll layout)  ·  C: contents  ·  F: full screen  ·  Ctrl 0: default size",
        ));

        widget::settings::view_column(vec![section.into(), layout.into(), help.into()]).into()
    }

    // ── Update ────────────────────────────────────────────────────────────

    pub(super) fn update_book(&mut self, msg: BookMsg) -> Task<Message> {
        let Some(book) = &self.epub_book else { return Task::none() };
        let n = book.chapters.len();
        let id = book_scroll_id();
        let line = f32::from(self.settings.book.font_size) * self.settings.book.line_spacing;
        let page_step = (self.book_scroll.viewport_h - 2.0 * line).max(line);

        match msg {
            BookMsg::Scrolled(v) => {
                self.book_scroll = BookScroll {
                    rel_y: if v.content_bounds().height <= v.bounds().height { 0.0 } else { v.relative_offset().y },
                    viewport_h: v.bounds().height,
                    content_h: v.content_bounds().height,
                };
                if (self.book_scroll.rel_y - self.book_saved_pos).abs() > 0.02 {
                    self.persist_book_position();
                }
            }
            BookMsg::PageDown | BookMsg::LineDown if self.is_paged() => {
                let Some((page, count, step)) = self.current_book_page() else { return Task::none() };
                if page + step >= count {
                    return self.update_book(BookMsg::NextChapter);
                }
                self.book_scroll.rel_y = self.book_pages.lock().map(|p| p.anchor_of(page + step)).unwrap_or(0.0);
                self.persist_book_position();
            }
            BookMsg::PageUp | BookMsg::LineUp if self.is_paged() => {
                let Some((page, _, step)) = self.current_book_page() else { return Task::none() };
                if page == 0 {
                    if self.epub_chapter > 0 {
                        return self.go_to_chapter(self.epub_chapter - 1, 1.0);
                    }
                    return Task::none();
                }
                self.book_scroll.rel_y =
                    self.book_pages.lock().map(|p| p.anchor_of(page.saturating_sub(step))).unwrap_or(0.0);
                self.persist_book_position();
            }
            BookMsg::Repaginated => {}
            BookMsg::OpenImage(i) => self.book_image = Some(i),
            BookMsg::CloseImage => self.book_image = None,
            BookMsg::SetLayout(i) => {
                if let Some(l) = BookLayout::ALL.get(i) {
                    self.settings.book.layout = *l;
                    self.settings.save();
                    return self.keep_book_position();
                }
            }
            BookMsg::PageDown => {
                let at_end = self.book_scroll.content_h <= self.book_scroll.viewport_h + 1.0
                    || self.book_scroll.rel_y >= 0.999;
                if at_end {
                    return self.update_book(BookMsg::NextChapter);
                }
                return scroll_mod::scroll_by(id, AbsoluteOffset { x: 0.0, y: page_step });
            }
            BookMsg::PageUp => {
                if self.book_scroll.rel_y <= 0.001 && self.epub_chapter > 0 {
                    // Back into the previous chapter, at its end.
                    return self.go_to_chapter(self.epub_chapter - 1, 1.0);
                }
                return scroll_mod::scroll_by(id, AbsoluteOffset { x: 0.0, y: -page_step });
            }
            BookMsg::LineDown => return scroll_mod::scroll_by(id, AbsoluteOffset { x: 0.0, y: line * 2.0 }),
            BookMsg::LineUp => return scroll_mod::scroll_by(id, AbsoluteOffset { x: 0.0, y: -line * 2.0 }),
            BookMsg::NextChapter => {
                if self.epub_chapter + 1 < n {
                    return self.go_to_chapter(self.epub_chapter + 1, 0.0);
                }
                // Finished the book.
                self.book_scroll.rel_y = 1.0;
                self.persist_book_position();
            }
            BookMsg::PrevChapter => {
                if self.epub_chapter > 0 {
                    return self.go_to_chapter(self.epub_chapter - 1, 0.0);
                }
            }
            BookMsg::GoToChapter(ch) => {
                self.show_chapter_select = false;
                return self.go_to_chapter(ch.min(n.saturating_sub(1)), 0.0);
            }
            BookMsg::FontBigger => return self.set_book_size(self.settings.book.font_size.saturating_add(1)),
            BookMsg::FontSmaller => return self.set_book_size(self.settings.book.font_size.saturating_sub(1)),
            BookMsg::FontReset => return self.set_book_size(BookSettings::default().font_size),
            BookMsg::SetFontSize(v) => return self.set_book_size(v.round() as u16),
            BookMsg::SetFont(i) => {
                self.settings.book.font = if i == 1 { BookFont::Sans } else { BookFont::Serif };
                self.settings.save();
                return self.keep_book_position();
            }
            BookMsg::SetSpacing(i) => {
                if let Some(s) = BookSettings::LINE_SPACINGS.get(i) {
                    self.settings.book.line_spacing = *s;
                    self.settings.save();
                    return self.keep_book_position();
                }
            }
            BookMsg::SetWidth(i) => {
                if let Some(w) = BookWidth::ALL.get(i) {
                    self.settings.book.width = *w;
                    self.settings.save();
                    return self.keep_book_position();
                }
            }
            BookMsg::SetTheme(i) => {
                if let Some(t) = BookTheme::ALL.get(i) {
                    self.settings.book.theme = *t;
                    self.settings.save();
                }
            }
            BookMsg::SetJustify(v) => {
                self.settings.book.justify = v;
                self.settings.save();
            }
            BookMsg::ToggleTypography => {
                let showing = self.core.window.show_context && self.context_page == ContextPage::Typography;
                self.context_page = ContextPage::Typography;
                cosmic::ApplicationExt::set_show_context(self, !showing);
            }
        }
        Task::none()
    }

    fn is_paged(&self) -> bool {
        self.settings.book.layout != BookLayout::Scroll
    }

    /// (first page on screen, pages in chapter, pages per turn), once the
    /// current chapter has been laid out.
    fn current_book_page(&self) -> Option<(usize, usize, usize)> {
        let p = self.book_pages.lock().ok()?;
        (p.chapter == self.epub_chapter && p.count() > 0)
            .then(|| (p.page_at(self.book_scroll.rel_y), p.count(), p.step()))
    }

    fn set_book_size(&mut self, size: u16) -> Task<Message> {
        let size = size.clamp(BookSettings::MIN_SIZE, BookSettings::MAX_SIZE);
        if size == self.settings.book.font_size {
            return Task::none();
        }
        self.settings.book.font_size = size;
        self.settings.save();
        self.keep_book_position()
    }

    /// Reflowing changes the content height; re-snap so the reader stays at
    /// the same relative spot instead of jumping.
    fn keep_book_position(&self) -> Task<Message> {
        scroll_mod::snap_to(book_scroll_id(), RelativeOffset { x: None, y: Some(self.book_scroll.rel_y) })
    }

    pub(super) fn go_to_chapter(&mut self, ch: usize, rel_y: f32) -> Task<Message> {
        self.book_image = None;
        self.epub_chapter = ch;
        self.book_scroll.rel_y = rel_y;
        self.persist_book_position();
        scroll_mod::snap_to(book_scroll_id(), RelativeOffset { x: None, y: Some(rel_y) })
    }

    pub(super) fn persist_book_position(&mut self) {
        let Some(book) = &self.epub_book else { return };
        let (Some(db), Some(path)) = (&self.db, &self.open_path) else { return };
        let key = path.to_string_lossy();
        let n = book.chapters.len();
        let pos = self.book_scroll.rel_y.clamp(0.0, 1.0);
        let last_page = self.is_paged()
            && self.current_book_page().is_some_and(|(page, count, step)| page + step >= count);
        let finished = self.epub_chapter + 1 >= n && (pos >= 0.97 || last_page);
        let _ = library::save_book_progress(db, &key, self.epub_chapter, n, pos, finished);
        self.book_saved_pos = pos;
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>().trim_end())
    }
}
