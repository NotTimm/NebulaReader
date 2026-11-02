//! The Wikipedia page (opt-in: Settings → Wikipedia): Wikipedia's main
//! page and its articles, drawn natively and laid out like the site. An
//! article is one long scrolling page, as on Wikipedia: a collapsible
//! contents sidebar, the title in serif with "From Wikipedia, the free
//! encyclopedia", the lead beside the infobox, figures beside the text
//! after them, sections, and the credit and licence at the end. No web
//! view: the article model comes from `crate::wikipedia`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cosmic::iced::advanced::widget::operation::{Operation, Outcome};
use cosmic::iced::font::{Family, Style as FontStyle, Weight};
use cosmic::iced::widget::scrollable::{self as scroll_mod, AbsoluteOffset, RelativeOffset};
use cosmic::iced::widget::text::{LineHeight, Span};
use cosmic::iced::widget::{rich_text, span};
use cosmic::iced::{Alignment, Background, Border, Color, ContentFit, Font, Length, Rectangle, Vector};
use cosmic::widget::{self, icon};
use cosmic::{app::Task, Application, ApplicationExt, Element};

use super::{tip, App, AppView, Message};
use crate::wikipedia::{self, Article, Block, Cell, Featured, Line, Summary, Table};

/// Body text, as on the site (16 px at its default size, a little less here).
const TEXT: f32 = 15.0;
const INFOBOX_WIDTH: f32 = 300.0;
const TOC_WIDTH: f32 = 210.0;
/// Pictures kept in memory before starting over.
const MAX_IMAGES: usize = 300;

#[derive(Clone, Debug, PartialEq)]
pub enum WikiPage {
    Home,
    Article(String),
    Search(String),
}

#[derive(Clone, Debug)]
pub enum WikiMsg {
    /// From the library: the main page (or where it was).
    Open,
    Go(WikiPage),
    Back,
    Random,
    RandomFound(Result<String, String>),
    HomeLoaded(u64, Result<Featured, String>),
    ArticleLoaded(u64, Result<Article, String>),
    SearchLoaded(u64, Result<Vec<Summary>, String>),
    Image(String, Option<Vec<u8>>),
    SearchInput(String),
    Search,
    /// A link: `wiki:<Title>` or a web address.
    Link(String),
    ToggleToc,
    /// Shows or hides a section's subsections in the contents.
    ToggleTocSection(usize),
    /// Scrolls to a section (`None`: the top).
    JumpTo(Option<usize>),
    ScrollTo(Option<f32>),
    OpenOnWikipedia,
    SetEnabled(bool),
}

#[derive(Default)]
pub struct WikiState {
    page: Option<WikiPage>,
    history: Vec<WikiPage>,
    generation: u64,
    loading: bool,
    error: Option<String>,
    featured: Option<Featured>,
    article: Option<Arc<Article>>,
    results: Vec<Summary>,
    /// Pictures by URL, in memory only; `None` while loading.
    images: HashMap<String, Option<widget::image::Handle>>,
    toc_hidden: bool,
    /// Sections (by index) whose subsections are shown in the contents.
    toc_open: HashSet<usize>,
    search_input: String,
}

fn scroll_id() -> widget::Id {
    widget::Id::new("wiki-scroll")
}

fn section_id(i: usize) -> widget::Id {
    widget::Id::new(format!("wiki-section-{i}"))
}

/// Finds how far down the page a section is, for the contents' links.
struct FindSection {
    target: widget::Id,
    section: Option<Rectangle>,
    content: Option<Rectangle>,
}

impl Operation<Option<f32>> for FindSection {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<f32>>)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&widget::Id>, bounds: Rectangle) {
        if id == Some(&self.target) {
            self.section = Some(bounds);
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&widget::Id>,
        _bounds: Rectangle,
        content_bounds: Rectangle,
        _translation: Vector,
        _state: &mut dyn cosmic::iced::advanced::widget::operation::Scrollable,
    ) {
        if id == Some(&scroll_id()) {
            self.content = Some(content_bounds);
        }
    }

    fn finish(&self) -> Outcome<Option<f32>> {
        Outcome::Some(match (self.section, self.content) {
            (Some(s), Some(c)) => Some((s.y - c.y - 8.0).max(0.0)),
            _ => None,
        })
    }
}

/// Colours in the spirit of Wikipedia's, for the app's light or dark theme.
struct Palette {
    muted: Color,
    link: Color,
    border: Color,
    /// Infoboxes and figure frames.
    panel: Color,
    dark: bool,
}

fn palette() -> Palette {
    let theme = cosmic::theme::active();
    let c = theme.cosmic();
    let dark = c.is_dark;
    let rgb = |r, g, b| Color::from_rgb8(r, g, b);
    Palette {
        muted: if dark { rgb(0xa2, 0xa9, 0xb1) } else { rgb(0x54, 0x59, 0x5d) },
        link: if dark { rgb(0x88, 0xa3, 0xe8) } else { rgb(0x33, 0x66, 0xcc) },
        border: if dark { rgb(0x54, 0x59, 0x5d) } else { rgb(0xa2, 0xa9, 0xb1) },
        panel: if dark { rgb(0x20, 0x21, 0x22) } else { rgb(0xf8, 0xf9, 0xfa) },
        dark,
    }
}

impl Palette {
    /// A colour the page gives (an infobox header), toned for dark themes.
    fn page_color(&self, rgb: [u8; 3]) -> Color {
        let c = Color::from_rgb8(rgb[0], rgb[1], rgb[2]);
        if self.dark {
            Color { r: c.r * 0.35, g: c.g * 0.35, b: c.b * 0.4, a: 1.0 }
        } else {
            c
        }
    }

    /// The main page's box headings ("From today's featured article"…).
    fn band(&self, rgb: [u8; 3]) -> Color {
        self.page_color(rgb)
    }
}

fn serif() -> Font {
    Font { family: Family::Serif, ..Font::DEFAULT }
}

/// A line of runs as rich-text spans: links coloured, footnote marks small.
fn spans<'a>(line: &'a Line, pal: &Palette, size: f32) -> Vec<Span<'a, String, Font>> {
    line.iter()
        .map(|r| {
            let mut f = Font::DEFAULT;
            if r.bold {
                f.weight = Weight::Bold;
            }
            if r.italic {
                f.style = FontStyle::Italic;
            }
            let s = span(r.text.as_str()).font(f);
            if r.reference {
                s.size(size * 0.72).color(pal.link)
            } else if let Some(target) = &r.link {
                s.color(pal.link).link(target.clone())
            } else {
                s
            }
        })
        .collect()
}

fn line_text<'a>(line: &'a Line, pal: &Palette, size: f32) -> Element<'a, Message> {
    rich_text(spans(line, pal, size))
        .size(size)
        .line_height(LineHeight::Relative(1.6))
        .on_link_click(|l| Message::Wiki(WikiMsg::Link(l)))
        .width(Length::Fill)
        .into()
}

fn rule<'a>(color: Color) -> Element<'a, Message> {
    widget::container(widget::Space::new().height(1.0).width(Length::Fill))
        .style(move |_: &cosmic::Theme| widget::container::Style { background: Some(Background::Color(color)), ..Default::default() })
        .into()
}

fn boxed<'a>(content: impl Into<Element<'a, Message>>, bg: Color, border: Color) -> widget::Container<'a, Message, cosmic::Theme> {
    widget::container(content).style(move |_: &cosmic::Theme| widget::container::Style {
        background: Some(Background::Color(bg)),
        border: Border { color: border, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    })
}

impl App {
    fn wiki_go(&mut self, page: WikiPage, remember: bool) -> Task<Message> {
        if remember {
            if let Some(current) = self.wiki.page.take() {
                if current != page {
                    self.wiki.history.push(current);
                }
            }
        }
        self.wiki.page = Some(page.clone());
        self.wiki.generation += 1;
        self.wiki.error = None;
        self.wiki.loading = true;
        self.app_view = AppView::Wikipedia;
        self.set_show_context(false);
        let generation = self.wiki.generation;
        let top = scroll_mod::snap_to(scroll_id(), RelativeOffset { x: None, y: Some(0.0) });
        let load = match page {
            WikiPage::Home => {
                self.set_header_title(fl!("wiki-main-page"));
                if let Some(f) = self.wiki.featured.clone() {
                    return Task::batch([self.update_wiki(WikiMsg::HomeLoaded(generation, Ok(f))), top]);
                }
                cosmic::task::future(async move { Message::Wiki(WikiMsg::HomeLoaded(generation, wikipedia::featured().await)) })
            }
            WikiPage::Article(title) => {
                self.set_header_title(title.clone());
                self.wiki.article = None;
                self.wiki.toc_open.clear();
                cosmic::task::future(async move { Message::Wiki(WikiMsg::ArticleLoaded(generation, wikipedia::article(&title).await)) })
            }
            WikiPage::Search(query) => {
                self.set_header_title(fl!("wiki-search-results", query = query.as_str()));
                cosmic::task::future(async move { Message::Wiki(WikiMsg::SearchLoaded(generation, wikipedia::search(&query).await)) })
            }
        };
        Task::batch([load, top])
    }

    fn wiki_images(&mut self, urls: Vec<String>) -> Task<Message> {
        if self.wiki.images.len() + urls.len() > MAX_IMAGES {
            self.wiki.images.clear();
        }
        let mut tasks = Vec::new();
        for url in urls {
            if self.wiki.images.contains_key(&url) {
                continue;
            }
            self.wiki.images.insert(url.clone(), None);
            tasks.push(cosmic::task::future(async move {
                let bytes = wikipedia::image(&url).await.ok();
                Message::Wiki(WikiMsg::Image(url, bytes))
            }));
        }
        Task::batch(tasks)
    }

    /// Whether Back has somewhere to go, for Esc.
    pub(super) fn wiki_can_go_back(&self) -> bool {
        !self.wiki.history.is_empty()
    }

    pub(super) fn update_wiki(&mut self, msg: WikiMsg) -> Task<Message> {
        match msg {
            WikiMsg::Open => {
                let page = self.wiki.page.clone().unwrap_or(WikiPage::Home);
                if self.wiki.page.is_some() && !self.wiki.loading && self.wiki.error.is_none() {
                    // Back where it was.
                    self.app_view = AppView::Wikipedia;
                    self.set_show_context(false);
                    return Task::none();
                }
                return self.wiki_go(page, false);
            }
            WikiMsg::Go(page) => return self.wiki_go(page, true),
            WikiMsg::Back => {
                if let Some(page) = self.wiki.history.pop() {
                    return self.wiki_go(page, false);
                }
                return self.update(Message::GoToLibrary);
            }
            WikiMsg::Random => {
                self.wiki.loading = true;
                return cosmic::task::future(async { Message::Wiki(WikiMsg::RandomFound(wikipedia::random_title().await)) });
            }
            WikiMsg::RandomFound(result) => match result {
                Ok(title) => return self.wiki_go(WikiPage::Article(title), true),
                Err(e) => {
                    self.wiki.loading = false;
                    self.wiki.error = Some(e);
                }
            },
            WikiMsg::HomeLoaded(generation, result) => {
                if generation != self.wiki.generation {
                    return Task::none();
                }
                self.wiki.loading = false;
                match result {
                    Ok(f) => {
                        let mut urls: Vec<String> = f.article.iter().filter_map(|a| a.thumbnail.clone()).collect();
                        urls.extend(f.picture.iter().map(|p| p.image.clone()));
                        urls.extend(f.most_read.iter().filter_map(|a| a.thumbnail.clone()));
                        self.wiki.featured = Some(f);
                        return self.wiki_images(urls);
                    }
                    Err(e) => self.wiki.error = Some(e),
                }
            }
            WikiMsg::ArticleLoaded(generation, result) => {
                if generation != self.wiki.generation {
                    return Task::none();
                }
                self.wiki.loading = false;
                match result {
                    Ok(a) => {
                        let urls = a.images();
                        self.wiki.article = Some(Arc::new(a));
                        return self.wiki_images(urls);
                    }
                    Err(e) => self.wiki.error = Some(e),
                }
            }
            WikiMsg::SearchLoaded(generation, result) => {
                if generation != self.wiki.generation {
                    return Task::none();
                }
                self.wiki.loading = false;
                match result {
                    Ok(r) => {
                        let urls = r.iter().filter_map(|s| s.thumbnail.clone()).collect();
                        self.wiki.results = r;
                        return self.wiki_images(urls);
                    }
                    Err(e) => self.wiki.error = Some(e),
                }
            }
            WikiMsg::Image(url, bytes) => {
                if let Some(slot) = self.wiki.images.get_mut(&url) {
                    *slot = bytes.map(widget::image::Handle::from_bytes);
                }
            }
            WikiMsg::SearchInput(s) => self.wiki.search_input = s,
            WikiMsg::Search => {
                let q = self.wiki.search_input.trim().to_string();
                if !q.is_empty() {
                    return self.wiki_go(WikiPage::Search(q), true);
                }
            }
            WikiMsg::Link(target) => {
                if let Some(title) = target.strip_prefix("wiki:") {
                    return self.wiki_go(WikiPage::Article(title.to_string()), true);
                }
                crate::platform::open_url(&target);
            }
            WikiMsg::ToggleToc => self.wiki.toc_hidden = !self.wiki.toc_hidden,
            WikiMsg::ToggleTocSection(i) => {
                if !self.wiki.toc_open.remove(&i) {
                    self.wiki.toc_open.insert(i);
                }
            }
            WikiMsg::JumpTo(None) => return scroll_mod::snap_to(scroll_id(), RelativeOffset { x: None, y: Some(0.0) }),
            WikiMsg::JumpTo(Some(i)) => {
                let op = FindSection { target: section_id(i), section: None, content: None };
                return cosmic::iced::runtime::task::widget(op).map(|y| cosmic::Action::App(Message::Wiki(WikiMsg::ScrollTo(y))));
            }
            WikiMsg::ScrollTo(Some(y)) => return scroll_mod::scroll_to(scroll_id(), AbsoluteOffset { x: None, y: Some(y) }),
            WikiMsg::ScrollTo(None) => {}
            WikiMsg::OpenOnWikipedia => {
                if let Some(WikiPage::Article(title)) = &self.wiki.page {
                    crate::platform::open_url(&wikipedia::page_url(title));
                }
            }
            WikiMsg::SetEnabled(on) => {
                self.settings.wikipedia = on;
                self.settings.save();
                if !on && self.app_view == AppView::Wikipedia {
                    self.app_view = AppView::Library;
                }
            }
        }
        Task::none()
    }

    /// Settings: turning the Wikipedia page on.
    pub(super) fn build_wiki_section(&self) -> Element<'_, Message> {
        widget::settings::section()
            .title("Wikipedia")
            .add(
                widget::settings::item_row(vec![
                    widget::column::with_capacity(2)
                        .spacing(2)
                        .width(Length::Fill)
                        .push(widget::text::body(fl!("wiki-enable")))
                        .push(widget::text::caption(fl!("wiki-enable-note")))
                        .into(),
                    widget::toggler(self.settings.wikipedia).on_toggle(|on| Message::Wiki(WikiMsg::SetEnabled(on))).into(),
                ])
                .align_y(Alignment::Center),
            )
            .into()
    }

    fn wiki_image<'a>(&self, url: &str, w: f32, h: f32) -> Element<'a, Message> {
        match self.wiki.images.get(url).cloned().flatten() {
            Some(handle) => widget::image(handle).width(Length::Fixed(w)).height(Length::Fixed(h)).content_fit(ContentFit::Contain).into(),
            None => widget::Space::new().width(Length::Fixed(w)).height(Length::Fixed(h)).into(),
        }
    }

    /// The bar above every Wikipedia page: Back, Main page, Random, search.
    fn wiki_bar(&self) -> Element<'_, Message> {
        widget::row::with_capacity(5)
            .spacing(8)
            .align_y(Alignment::Center)
            .push(tip(
                widget::button::icon(icon::from_name("go-previous-symbolic")).on_press(Message::Wiki(WikiMsg::Back)),
                fl!("opds-back"),
            ))
            .push(widget::button::text(fl!("wiki-main-page")).on_press(Message::Wiki(WikiMsg::Go(WikiPage::Home))))
            .push(widget::button::text(fl!("wiki-random")).on_press(Message::Wiki(WikiMsg::Random)))
            .push(
                widget::search_input(fl!("wiki-search"), &self.wiki.search_input)
                    .on_input(|s| Message::Wiki(WikiMsg::SearchInput(s)))
                    .on_submit(|_| Message::Wiki(WikiMsg::Search))
                    .width(Length::Fill),
            )
            .push(widget::button::standard(fl!("wiki-search-button")).on_press(Message::Wiki(WikiMsg::Search)))
            .into()
    }

    pub(super) fn view_wiki(&self) -> Element<'_, Message> {
        let pal = palette();
        let body: Element<'_, Message> = if let Some(e) = &self.wiki.error {
            widget::column::with_capacity(2)
                .spacing(8)
                .push(widget::text::title3(fl!("wiki-failed")))
                .push(widget::text::body(e.clone()))
                .into()
        } else if self.wiki.loading {
            widget::container(widget::indeterminate_circular()).width(Length::Fill).center_x(Length::Fill).padding(40).into()
        } else {
            match &self.wiki.page {
                Some(WikiPage::Article(_)) => match &self.wiki.article {
                    Some(a) => return self.view_wiki_article(a, &pal),
                    None => widget::Space::new().into(),
                },
                Some(WikiPage::Search(q)) => self.view_wiki_results(q, &pal),
                _ => match &self.wiki.featured {
                    Some(f) => self.view_wiki_home(f, &pal),
                    None => widget::Space::new().into(),
                },
            }
        };
        let page = widget::column::with_capacity(2).spacing(16).padding([16, 24]).push(self.wiki_bar()).push(body);
        widget::scrollable(widget::container(page).max_width(1400).width(Length::Fill).center_x(Length::Fill))
            .id(scroll_id())
            .height(Length::Fill)
            .into()
    }

    // ── The main page ─────────────────────────────────────────────────────

    fn view_wiki_home<'a>(&'a self, f: &'a Featured, pal: &Palette) -> Element<'a, Message> {
        // A main-page box: a coloured heading band, then its content.
        let main_box = |title: String, band: [u8; 3], content: Element<'a, Message>| -> Element<'a, Message> {
            let head = widget::container(widget::text(title).size(17.0).font(serif()).font(Font { weight: Weight::Bold, ..serif() }))
                .padding([4, 8])
                .width(Length::Fill)
                .style({
                    let bg = pal.band(band);
                    move |_: &cosmic::Theme| widget::container::Style { background: Some(Background::Color(bg)), ..Default::default() }
                });
            boxed(widget::column::with_capacity(2).push(head).push(widget::container(content).padding([8, 10])), pal.panel, pal.border)
                .width(Length::Fill)
                .into()
        };
        let link = |title: &str| -> Element<'a, Message> {
            let mut line = vec![wikipedia::Run { text: title.to_string(), link: Some(format!("wiki:{title}")), ..Default::default() }];
            line[0].bold = true;
            line_text_owned(line, pal, TEXT)
        };

        let mut left = widget::column::with_capacity(3).spacing(16).width(Length::FillPortion(3));
        if let Some(a) = &f.article {
            let mut text = widget::column::with_capacity(3).spacing(6).width(Length::Fill);
            text = text.push(link(&a.title)).push(widget::text(a.extract.clone()).size(TEXT).line_height(LineHeight::Relative(1.6)));
            text = text.push(widget::button::link(fl!("wiki-full-article")).on_press(Message::Wiki(WikiMsg::Go(WikiPage::Article(a.title.clone())))));
            let mut row = widget::row::with_capacity(2).spacing(12);
            if let Some(t) = &a.thumbnail {
                row = row.push(self.wiki_image(t, 120.0, 160.0));
            }
            left = left.push(main_box(fl!("wiki-featured-article"), [0xce, 0xf2, 0xe0], row.push(text).into()));
        }
        if !f.most_read.is_empty() {
            let mut list = widget::column::with_capacity(f.most_read.len()).spacing(8);
            for (i, a) in f.most_read.iter().enumerate() {
                let mut row = widget::row::with_capacity(3).spacing(10).align_y(Alignment::Center);
                row = row.push(widget::text(format!("{}.", i + 1)).size(TEXT).width(Length::Fixed(22.0)));
                if let Some(t) = &a.thumbnail {
                    row = row.push(self.wiki_image(t, 48.0, 48.0));
                }
                row = row.push(
                    widget::column::with_capacity(2)
                        .width(Length::Fill)
                        .push(link(&a.title))
                        .push(widget::text(a.description.clone()).size(TEXT - 2.0).class(cosmic::theme::Text::Color(pal.muted))),
                );
                list = list.push(row);
            }
            left = left.push(main_box(fl!("wiki-most-read"), [0xce, 0xf2, 0xe0], list.into()));
        }

        let mut right = widget::column::with_capacity(2).spacing(16).width(Length::FillPortion(2));
        if !f.news.is_empty() {
            let mut list = widget::column::with_capacity(f.news.len()).spacing(6);
            for story in &f.news {
                list = list.push(widget::row::with_capacity(2).spacing(6).push(widget::text("•").size(TEXT)).push(line_text(story, pal, TEXT)));
            }
            right = right.push(main_box(fl!("wiki-in-the-news"), [0xce, 0xdf, 0xf2], list.into()));
        }
        if !f.on_this_day.is_empty() {
            let mut list = widget::column::with_capacity(f.on_this_day.len()).spacing(6);
            for e in f.on_this_day.iter().take(8) {
                let mut line = vec![wikipedia::Run { text: format!("{} – ", e.year), bold: true, ..Default::default() }];
                line.push(wikipedia::Run { text: e.text.clone(), ..Default::default() });
                if let Some(p) = e.pages.first() {
                    line.push(wikipedia::Run { text: format!(" ({})", p.title), link: Some(format!("wiki:{}", p.title)), ..Default::default() });
                }
                list = list.push(line_text_owned(line, pal, TEXT));
            }
            right = right.push(main_box(fl!("wiki-on-this-day"), [0xce, 0xdf, 0xf2], list.into()));
        }

        let mut col = widget::column::with_capacity(4).spacing(16);
        col = col.push(
            widget::column::with_capacity(2)
                .spacing(4)
                .push(widget::text(fl!("wiki-welcome")).size(30.0).font(serif()))
                .push(widget::text(fl!("wiki-welcome-sub")).size(TEXT).class(cosmic::theme::Text::Color(pal.muted))),
        );
        col = col.push(widget::row::with_capacity(2).spacing(16).push(left).push(right));
        if let Some(p) = &f.picture {
            let content = widget::column::with_capacity(2)
                .spacing(8)
                .align_x(Alignment::Center)
                .push(self.wiki_image(&p.image, 640.0, 400.0))
                .push(widget::text(p.description.clone()).size(TEXT).width(Length::Fill));
            col = col.push(main_box(fl!("wiki-featured-picture"), [0xdd, 0xce, 0xf2], content.into()));
        }
        col = col.push(widget::text::caption(fl!("wiki-credit-home")));
        col.into()
    }

    fn view_wiki_results<'a>(&'a self, query: &str, pal: &Palette) -> Element<'a, Message> {
        let mut col = widget::column::with_capacity(self.wiki.results.len() + 1).spacing(14);
        col = col.push(widget::text(fl!("wiki-search-results", query = query)).size(26.0).font(serif()));
        if self.wiki.results.is_empty() {
            col = col.push(widget::text::body(fl!("no-results")));
        }
        for s in &self.wiki.results {
            let mut row = widget::row::with_capacity(2).spacing(12);
            if let Some(t) = &s.thumbnail {
                row = row.push(self.wiki_image(t, 60.0, 60.0));
            }
            let title = vec![wikipedia::Run { text: s.title.clone(), link: Some(format!("wiki:{}", s.title)), ..Default::default() }];
            row = row.push(
                widget::column::with_capacity(3)
                    .spacing(2)
                    .width(Length::Fill)
                    .push(line_text_owned(title, pal, TEXT + 2.0))
                    .push_maybe((!s.description.is_empty()).then(|| widget::text(s.description.clone()).size(TEXT - 1.0).class(cosmic::theme::Text::Color(pal.muted))))
                    .push(widget::text(s.extract.clone()).size(TEXT - 1.0)),
            );
            col = col.push(row);
        }
        col.into()
    }

    // ── Articles ──────────────────────────────────────────────────────────

    fn view_wiki_article<'a>(&'a self, a: &'a Article, pal: &Palette) -> Element<'a, Message> {
        let mut main = widget::column::with_capacity(a.sections.len() + 4).spacing(10).width(Length::Fill);
        main = main.push(widget::text(a.title.clone()).size(30.0).font(serif()));
        main = main.push(rule(pal.border));
        main = main.push(widget::text(fl!("wiki-from")).size(TEXT - 1.0).class(cosmic::theme::Text::Color(pal.muted)));

        for (i, s) in a.sections.iter().enumerate() {
            let mut sec = widget::column::with_capacity(s.blocks.len() + 2).spacing(10);
            match s.level {
                1 => {}
                2 => {
                    sec = sec
                        .push(widget::Space::new().height(10.0))
                        .push(widget::text(s.title.clone()).size(24.0).font(serif()))
                        .push(rule(pal.border));
                }
                _ => {
                    let size = if s.level == 3 { 18.0 } else { 16.0 };
                    sec = sec.push(widget::text(s.title.clone()).size(size).font(Font { weight: Weight::Bold, ..Font::DEFAULT }));
                }
            }
            let flow = self.wiki_flow(&s.blocks, pal);
            // The lead sits beside the infobox, as on the site.
            if i == 0 {
                if let Some(ib) = &a.infobox {
                    sec = sec.push(
                        widget::row::with_capacity(2)
                            .spacing(20)
                            .push(widget::column::with_capacity(1).width(Length::Fill).push(flow))
                            .push(self.wiki_infobox(ib, pal)),
                    );
                } else {
                    sec = sec.push(flow);
                }
            } else {
                sec = sec.push(flow);
            }
            main = main.push(widget::container(sec).id(section_id(i)));
        }

        // Credit and licence.
        main = main.push(widget::Space::new().height(16.0)).push(rule(pal.border)).push(
            widget::row::with_capacity(2)
                .spacing(12)
                .align_y(Alignment::Center)
                .push(widget::text(fl!("wiki-credit", title = a.title.as_str())).size(TEXT - 2.0).class(cosmic::theme::Text::Color(pal.muted)).width(Length::Fill))
                .push(widget::button::link(fl!("wiki-license")).on_press(Message::OpenUrl(wikipedia::LICENSE_URL.into())))
                .push(widget::button::standard(fl!("wiki-view-on")).on_press(Message::Wiki(WikiMsg::OpenOnWikipedia))),
        );

        let mut row = widget::row::with_capacity(2).spacing(24);
        row = row.push(self.wiki_toc(a, pal));
        row = row.push(main);
        let page = widget::column::with_capacity(2).spacing(16).padding([16, 24]).push(self.wiki_bar()).push(row);
        widget::scrollable(widget::container(page).max_width(1400).width(Length::Fill).center_x(Length::Fill))
            .id(scroll_id())
            .height(Length::Fill)
            .into()
    }

    /// The contents: top-level sections, each opening onto its subsections.
    fn wiki_toc<'a>(&'a self, a: &'a Article, pal: &Palette) -> Element<'a, Message> {
        if self.wiki.toc_hidden {
            return tip(
                widget::button::icon(icon::from_name("view-list-symbolic")).on_press(Message::Wiki(WikiMsg::ToggleToc)),
                fl!("wiki-contents"),
            );
        }
        let item = |label: String, msg: WikiMsg| -> Element<'a, Message> {
            widget::button::custom(widget::text(label).size(TEXT - 1.0).class(cosmic::theme::Text::Color(pal.link)))
                .class(cosmic::theme::Button::Text)
                .padding([2, 4])
                .on_press(Message::Wiki(msg))
                .into()
        };
        let mut col = widget::column::with_capacity(a.sections.len() + 2).spacing(2).width(Length::Fixed(TOC_WIDTH));
        col = col.push(
            widget::row::with_capacity(2)
                .spacing(8)
                .align_y(Alignment::Center)
                .push(widget::text(fl!("wiki-contents")).size(TEXT).font(Font { weight: Weight::Bold, ..Font::DEFAULT }))
                .push(widget::button::link(fl!("wiki-hide")).on_press(Message::Wiki(WikiMsg::ToggleToc))),
        );
        col = col.push(rule(pal.border));
        col = col.push(item(fl!("wiki-top"), WikiMsg::JumpTo(None)));
        let mut parent: Option<usize> = None;
        for (i, s) in a.sections.iter().enumerate().skip(1) {
            if s.level == 2 {
                parent = Some(i);
                let has_children = a.sections.get(i + 1).is_some_and(|n| n.level > 2);
                let open = self.wiki.toc_open.contains(&i);
                let mut row = widget::row::with_capacity(2).spacing(0).align_y(Alignment::Center);
                row = row.push(if has_children {
                    Element::from(
                        widget::button::icon(icon::from_name(if open { "pan-down-symbolic" } else { "pan-end-symbolic" }))
                            .extra_small()
                            .on_press(Message::Wiki(WikiMsg::ToggleTocSection(i))),
                    )
                } else {
                    widget::Space::new().width(Length::Fixed(24.0)).into()
                });
                row = row.push(item(s.title.clone(), WikiMsg::JumpTo(Some(i))));
                col = col.push(row);
            } else if parent.is_some_and(|p| self.wiki.toc_open.contains(&p)) && s.level > 2 {
                let indent = 24.0 + 12.0 * (s.level as f32 - 3.0);
                col = col.push(
                    widget::row::with_capacity(2)
                        .push(widget::Space::new().width(Length::Fixed(indent)))
                        .push(item(s.title.clone(), WikiMsg::JumpTo(Some(i)))),
                );
            }
        }
        col.into()
    }

    /// A section's blocks, with each figure beside the text after it (the
    /// site floats them right).
    fn wiki_flow<'a>(&'a self, blocks: &'a [Block], pal: &Palette) -> Element<'a, Message> {
        let mut col = widget::column::with_capacity(blocks.len()).spacing(12).width(Length::Fill);
        let mut i = 0;
        while i < blocks.len() {
            if let Block::Figure { .. } = &blocks[i] {
                // The figures here, then the text until the next one.
                let mut figs = widget::column::with_capacity(2).spacing(12);
                while let Some(Block::Figure { image, caption }) = blocks.get(i) {
                    figs = figs.push(self.wiki_figure(image, caption, pal));
                    i += 1;
                }
                let mut text = widget::column::with_capacity(4).spacing(12).width(Length::Fill);
                while let Some(b) = blocks.get(i).filter(|b| matches!(b, Block::Para(_) | Block::List { .. } | Block::Quote(_) | Block::Hatnote(_))) {
                    text = text.push(self.wiki_block(b, pal));
                    i += 1;
                }
                col = col.push(widget::row::with_capacity(2).spacing(20).push(text).push(figs));
                continue;
            }
            col = col.push(self.wiki_block(&blocks[i], pal));
            i += 1;
        }
        col.into()
    }

    fn wiki_figure<'a>(&'a self, image: &wikipedia::Image, caption: &'a Line, pal: &Palette) -> Element<'a, Message> {
        let w = image.width.min(260.0);
        let h = image.height * w / image.width.max(1.0);
        boxed(
            widget::column::with_capacity(2)
                .spacing(6)
                .width(Length::Fixed(w + 8.0))
                .push(self.wiki_image(&image.url, w, h))
                .push_maybe((!caption.is_empty()).then(|| line_text(caption, pal, TEXT - 2.0))),
            pal.panel,
            pal.border,
        )
        .padding(4)
        .into()
    }

    fn wiki_block<'a>(&'a self, b: &'a Block, pal: &Palette) -> Element<'a, Message> {
        match b {
            Block::Para(l) => line_text(l, pal, TEXT),
            Block::Hatnote(l) => widget::container(line_text(l, pal, TEXT - 1.0)).padding([0, 0, 0, 24]).into(),
            Block::Quote(l) => widget::container(line_text(l, pal, TEXT)).padding([0, 32]).into(),
            Block::List { ordered, items } => {
                let mut col = widget::column::with_capacity(items.len()).spacing(4);
                let mut n = [0usize; 8];
                for (depth, line) in items {
                    let d = (*depth).min(7);
                    n[d] += 1;
                    for x in n.iter_mut().skip(d + 1) {
                        *x = 0;
                    }
                    let mark = if *ordered && d == 0 { format!("{}.", n[d]) } else { "•".to_string() };
                    col = col.push(
                        widget::row::with_capacity(3)
                            .spacing(8)
                            .push(widget::Space::new().width(Length::Fixed(16.0 + 20.0 * d as f32)))
                            .push(widget::text(mark).size(TEXT))
                            .push(line_text(line, pal, TEXT)),
                    );
                }
                col.into()
            }
            Block::Figure { image, caption } => {
                widget::container(self.wiki_figure(image, caption, pal)).width(Length::Fill).align_x(Alignment::End).into()
            }
            Block::Table(t) => widget::scrollable::horizontal(self.wiki_table(t, pal, false)).into(),
        }
    }

    /// A table: each row's cells side by side, wider for wider spans.
    fn wiki_table<'a>(&'a self, t: &'a Table, pal: &Palette, infobox: bool) -> Element<'a, Message> {
        let mut col = widget::column::with_capacity(t.rows.len() + 1).spacing(if infobox { 4 } else { 0 });
        if !t.caption.is_empty() {
            col = col.push(line_text(&t.caption, pal, TEXT));
        }
        let size = if infobox { TEXT - 2.0 } else { TEXT - 1.0 };
        for r in &t.rows {
            let mut row = widget::row::with_capacity(r.cells.len()).spacing(if infobox { 8 } else { 0 });
            // Label and value: the label narrower, as in an infobox.
            let label_row = infobox && r.cells.len() == 2 && r.cells[0].header && !r.cells[1].header;
            for (j, c) in r.cells.iter().enumerate() {
                let portion = if label_row { if j == 0 { 2 } else { 3 } } else { c.colspan as u16 };
                row = row.push(self.wiki_cell(c, pal, size, infobox).width(Length::FillPortion(portion)));
            }
            col = col.push(row);
        }
        let width = if infobox { Length::Fixed(INFOBOX_WIDTH) } else { Length::Shrink };
        if infobox {
            boxed(col, pal.panel, pal.border).padding(6).width(width).into()
        } else {
            boxed(col, Color::TRANSPARENT, pal.border).into()
        }
    }

    fn wiki_cell<'a>(&'a self, c: &'a Cell, pal: &Palette, size: f32, infobox: bool) -> widget::Container<'a, Message, cosmic::Theme> {
        let mut col = widget::column::with_capacity(c.content.len()).spacing(4).width(Length::Fill);
        if c.center {
            col = col.align_x(Alignment::Center);
        }
        for b in &c.content {
            col = col.push(match b {
                Block::Para(l) => {
                    let t = rich_text(spans(l, pal, size))
                        .size(size)
                        .line_height(LineHeight::Relative(1.4))
                        .on_link_click(|l| Message::Wiki(WikiMsg::Link(l)));
                    if c.center {
                        t.align_x(cosmic::iced::widget::text::Alignment::Center).width(Length::Fill).into()
                    } else {
                        t.width(Length::Fill).into()
                    }
                }
                Block::Figure { image, caption } => {
                    let w = image.width.min(if infobox { INFOBOX_WIDTH - 24.0 } else { 220.0 });
                    let h = image.height * w / image.width.max(1.0);
                    widget::column::with_capacity(2)
                        .spacing(4)
                        .align_x(Alignment::Center)
                        .width(Length::Fill)
                        .push(self.wiki_image(&image.url, w, h))
                        .push_maybe((!caption.is_empty()).then(|| line_text(caption, pal, size)))
                        .into()
                }
                Block::Table(t) => self.wiki_table(t, pal, false),
                other => self.wiki_block(other, pal),
            });
        }
        let bg = c.background.map(|b| pal.page_color(b));
        let border = if infobox { Color::TRANSPARENT } else { pal.border };
        widget::container(col).padding(if infobox { [3, 4] } else { [4, 8] }).style(move |_: &cosmic::Theme| widget::container::Style {
            background: bg.map(Background::Color),
            border: Border { color: border, width: if infobox { 0.0 } else { 1.0 }, radius: 0.0.into() },
            ..Default::default()
        })
    }

    fn wiki_infobox<'a>(&'a self, t: &'a Table, pal: &Palette) -> Element<'a, Message> {
        self.wiki_table(t, pal, true)
    }
}

/// [`line_text`] for a line made on the spot.
fn line_text_owned<'a>(line: Line, pal: &Palette, size: f32) -> Element<'a, Message> {
    let spans: Vec<Span<'a, String, Font>> = line
        .into_iter()
        .map(|r| {
            let mut f = Font::DEFAULT;
            if r.bold {
                f.weight = Weight::Bold;
            }
            if r.italic {
                f.style = FontStyle::Italic;
            }
            let s = span(r.text).font(f);
            match r.link {
                Some(target) => s.color(pal.link).link(target),
                None => s,
            }
        })
        .collect();
    rich_text(spans)
        .size(size)
        .line_height(LineHeight::Relative(1.6))
        .on_link_click(|l| Message::Wiki(WikiMsg::Link(l)))
        .width(Length::Fill)
        .into()
}
