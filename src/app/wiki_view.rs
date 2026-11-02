//! The Wikipedia page (opt-in: Settings → Wikipedia): Wikipedia's main
//! page and its articles, drawn natively and laid out like the site. An
//! article is one long scrolling page, as on Wikipedia: the contents in a
//! sidebar that stays put (and can be hidden), the title in serif with
//! "From Wikipedia, the free encyclopedia", the infobox and pictures
//! floated right with the text beside them and then full width again
//! ([`super::float_flow`]), sections, and the credit and licence at the
//! end. Any picture opens full window; Left and Right step through the
//! page's pictures and Esc closes it. No web view: the article model comes
//! from `crate::wikipedia`.
//!
//! Pictures load like NebulaPlayer's posters: fetched once into a disk
//! cache, at most a few at a time, decoded and scaled to the size they're
//! shown on a background thread (so drawing never decodes), and handed to
//! the page in batches so it's rebuilt once per batch, not per picture.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use cosmic::iced::advanced::widget::operation::{Operation, Outcome};
use cosmic::iced::font::{Family, Style as FontStyle, Weight};
use cosmic::iced::widget::scrollable::{self as scroll_mod, AbsoluteOffset, RelativeOffset};
use cosmic::iced::widget::text::{LineHeight, Span};
use cosmic::iced::widget::{rich_text, span};
use cosmic::iced::{Alignment, Background, Border, Color, ContentFit, Font, Length, Pixels, Rectangle, Subscription, Vector};
use cosmic::widget::{self, icon};
use cosmic::{app::Task, Application, ApplicationExt, Element};

use super::float_flow::FloatFlow;
use super::{tip, App, AppView, Message};
use crate::wikipedia::{self, Article, Block, Cell, Featured, Line, Run, Summary, Table};

/// Body text, as on the site (16 px at its default size, a little less here).
const TEXT: f32 = 15.0;
/// Line height, a touch roomier than a browser's default.
const LINE: f32 = 1.7;
const INFOBOX_WIDTH: f32 = 300.0;
const TOC_WIDTH: f32 = 220.0;

/// Longest side, in pixels, pictures are decoded at: small pictures on a
/// page (at twice their size, for sharp high-DPI screens), the main page's
/// picture, and the full-window viewer.
const THUMB: u32 = 640;
const LARGE: u32 = 1600;
const FULL: u32 = 2560;

#[derive(Clone, Debug, PartialEq)]
pub enum WikiPage {
    Home,
    Article(String),
    Search(String),
}

/// A picture ready to draw, or why not yet.
#[derive(Clone, Debug)]
enum Pic {
    Loading,
    Ready(widget::image::Handle),
    Failed,
}

/// A loaded picture (by [`pic_key`]), or why it couldn't be.
pub type Arrival = (String, Result<widget::image::Handle, String>);

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
    /// Pictures that finished loading, in a batch.
    Pictures(Vec<Arrival>),
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
    /// Opens the picture viewer on a picture (its thumbnail's address).
    View(String),
    ViewNext,
    ViewPrev,
    ViewClose,
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
    /// Pictures by [`pic_key`], kept in memory while the page is open.
    pics: HashMap<String, Pic>,
    toc_hidden: bool,
    /// Sections (by index) whose subsections are shown in the contents.
    toc_open: std::collections::HashSet<usize>,
    search_input: String,
    /// The page's pictures (thumbnail address, caption), for the viewer.
    gallery: Vec<(String, Line)>,
    /// The picture shown full window, by its place in `gallery`.
    viewer: Option<usize>,
}

fn scroll_id() -> widget::Id {
    widget::Id::new("wiki-scroll")
}

fn section_id(i: usize) -> widget::Id {
    widget::Id::new(format!("wiki-section-{i}"))
}

// ── Pictures ───────────────────────────────────────────────────────────────

fn pic_key(url: &str, max: u32) -> String {
    format!("{max}|{url}")
}

fn cache_file(url: &str) -> std::path::PathBuf {
    let h = url.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    cache_dir().join(format!("{h:016x}"))
}

/// At most this many pictures download at once.
fn slots() -> &'static tokio::sync::Semaphore {
    static S: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    S.get_or_init(|| tokio::sync::Semaphore::new(6))
}

/// Waits this long after a picture arrives for others to join it.
const BATCH_WINDOW: std::time::Duration = std::time::Duration::from_millis(40);

type Queue = (tokio::sync::mpsc::UnboundedSender<Arrival>, tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<Arrival>>);

fn queue() -> &'static Queue {
    static Q: OnceLock<Queue> = OnceLock::new();
    Q.get_or_init(|| {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (tx, tokio::sync::Mutex::new(rx))
    })
}

/// Loads a picture and hands it to [`arrivals`].
async fn load_to_queue(url: String, max: u32) {
    let result = load(&url, max).await;
    let _ = queue().0.send((pic_key(&url, max), result));
}

/// Loaded pictures, in batches (a subscription).
fn arrivals() -> impl cosmic::iced::futures::Stream<Item = Vec<Arrival>> {
    use cosmic::iced::futures::SinkExt;
    cosmic::iced::stream::channel(4, async |mut out: cosmic::iced::futures::channel::mpsc::Sender<Vec<Arrival>>| {
        let mut rx = queue().1.lock().await;
        while let Some(first) = rx.recv().await {
            let mut batch = vec![first];
            tokio::time::sleep(BATCH_WINDOW).await;
            while let Ok(more) = rx.try_recv() {
                batch.push(more);
            }
            if out.send(batch).await.is_err() {
                break;
            }
        }
    })
}

/// A picture from the disk cache, or downloaded into it, decoded and
/// scaled down to `max` on a background thread.
async fn load(url: &str, max: u32) -> Result<widget::image::Handle, String> {
    let path = cache_file(url);
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(_) => {
            let _slot = slots().acquire().await.map_err(|e| e.to_string())?;
            let b = wikipedia::image(url).await?;
            if let Some(dir) = path.parent() {
                let _ = tokio::fs::create_dir_all(dir).await;
            }
            let _ = tokio::fs::write(&path, &b).await;
            b
        }
    };
    tokio::task::spawn_blocking(move || decode(&bytes, max).map(|(w, h, px)| widget::image::Handle::from_rgba(w, h, px)))
        .await
        .map_err(|e| e.to_string())?
}

/// A picture's pixels, no bigger than `max` on its longer side: the size
/// it's drawn at, so the screen never scales (or holds) more than that.
fn decode(bytes: &[u8], max: u32) -> Result<(u32, u32, Vec<u8>), String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    Ok((w, h, rgba.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_scaled_to_the_size_shown() {
        let big = image::RgbImage::from_pixel(4000, 2000, image::Rgb([200, 100, 50]));
        let mut png = std::io::Cursor::new(Vec::new());
        big.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let (w, h, px) = decode(png.get_ref(), THUMB).unwrap();
        // The longer side at THUMB, in proportion; four bytes a pixel.
        assert_eq!((w, h), (640, 320));
        assert_eq!(px.len(), 640 * 320 * 4);
        // Small ones are left as they are.
        let (w, h, _) = decode(png.get_ref(), 5000).unwrap();
        assert_eq!((w, h), (4000, 2000));
        assert!(decode(b"not a picture", THUMB).is_err());
    }
}

/// Kept under 400 MB at startup (`main`).
fn cache_dir() -> std::path::PathBuf {
    crate::library::cache_dir().join("wikipedia")
}

// ── Finding a section, for the contents ────────────────────────────────────

/// Finds how far down the page a section is.
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

// ── Colours and text ───────────────────────────────────────────────────────

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
    let dark = theme.cosmic().is_dark;
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
        if self.dark { Color { r: c.r * 0.35, g: c.g * 0.35, b: c.b * 0.4, a: 1.0 } } else { c }
    }
}

fn serif() -> Font {
    Font { family: Family::Serif, ..Font::DEFAULT }
}

fn bold() -> Font {
    Font { weight: Weight::Bold, ..Font::DEFAULT }
}

fn run_span<'a>(r: &Run, text: std::borrow::Cow<'a, str>, pal: &Palette, size: f32) -> Span<'a, String, Font> {
    let mut f = Font::DEFAULT;
    if r.bold {
        f.weight = Weight::Bold;
    }
    if r.italic {
        f.style = FontStyle::Italic;
    }
    let s = span(text).font(f);
    if r.reference {
        // Small, but on a line as tall as the text's, so a line with a
        // footnote mark isn't squeezed.
        s.size(size * 0.72).line_height(LineHeight::Absolute(Pixels(size * LINE))).color(pal.link)
    } else if let Some(target) = &r.link {
        s.color(pal.link).link(target.clone())
    } else {
        s
    }
}

fn line_text<'a>(line: &'a Line, pal: &Palette, size: f32) -> Element<'a, Message> {
    let spans: Vec<Span<'a, String, Font>> = line.iter().map(|r| run_span(r, r.text.as_str().into(), pal, size)).collect();
    rich_text(spans)
        .size(size)
        .line_height(LineHeight::Relative(LINE))
        .on_link_click(|l| Message::Wiki(WikiMsg::Link(l)))
        .width(Length::Fill)
        .into()
}

/// [`line_text`] for a line made on the spot.
fn owned_text<'a>(line: Line, pal: &Palette, size: f32) -> Element<'a, Message> {
    let spans: Vec<Span<'a, String, Font>> = line.iter().map(|r| run_span(r, r.text.clone().into(), pal, size)).collect();
    rich_text(spans)
        .size(size)
        .line_height(LineHeight::Relative(LINE))
        .on_link_click(|l| Message::Wiki(WikiMsg::Link(l)))
        .width(Length::Fill)
        .into()
}

fn link_run(title: &str, bold: bool) -> Run {
    Run { text: title.to_string(), bold, link: Some(format!("wiki:{title}")), ..Default::default() }
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

fn muted<'a>(text: String, size: f32, pal: &Palette) -> Element<'a, Message> {
    widget::text(text).size(size).class(cosmic::theme::Text::Color(pal.muted)).into()
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
        self.wiki.viewer = None;
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

    /// Starts loading pictures not loaded yet, at `max` pixels.
    fn want(&mut self, urls: impl IntoIterator<Item = String>, max: u32) -> Task<Message> {
        let mut tasks = Vec::new();
        for url in urls {
            let key = pic_key(&url, max);
            if self.wiki.pics.contains_key(&key) {
                continue;
            }
            self.wiki.pics.insert(key, Pic::Loading);
            // It comes back through `arrivals`, in a batch.
            tasks.push(cosmic::iced::Task::future(load_to_queue(url, max)).discard());
        }
        Task::batch(tasks)
    }

    /// A new page's pictures: the ones from the last page go (they're on
    /// disk if they're wanted again), these start loading.
    fn page_pictures(&mut self, gallery: Vec<(String, Line)>, extra: Vec<(String, u32)>) -> Task<Message> {
        self.wiki.pics.clear();
        let urls: Vec<String> = gallery.iter().map(|(u, _)| u.clone()).collect();
        self.wiki.gallery = gallery;
        let mut tasks = vec![self.want(urls, THUMB)];
        for (url, max) in extra {
            tasks.push(self.want([url], max));
        }
        Task::batch(tasks)
    }

    /// Whether Back has somewhere to go, for Esc.
    pub(super) fn wiki_can_go_back(&self) -> bool {
        !self.wiki.history.is_empty()
    }

    /// Esc, Left and Right while the picture viewer is open.
    pub(super) fn wiki_viewer_key(&mut self, key: &cosmic::iced::keyboard::Key) -> Option<Task<Message>> {
        use cosmic::iced::keyboard::{key::Named, Key};
        self.wiki.viewer?;
        Some(match key {
            Key::Named(Named::Escape) => self.update_wiki(WikiMsg::ViewClose),
            Key::Named(Named::ArrowRight) => self.update_wiki(WikiMsg::ViewNext),
            Key::Named(Named::ArrowLeft) => self.update_wiki(WikiMsg::ViewPrev),
            _ => Task::none(),
        })
    }

    pub(super) fn wiki_subscription(&self) -> Subscription<Message> {
        if self.settings.wikipedia {
            Subscription::run(arrivals).map(|batch| Message::Wiki(WikiMsg::Pictures(batch)))
        } else {
            Subscription::none()
        }
    }

    fn show_picture(&mut self, i: usize) -> Task<Message> {
        let Some((url, _)) = self.wiki.gallery.get(i).cloned() else { return Task::none() };
        self.wiki.viewer = Some(i);
        self.want([wikipedia::full_size(&url)], FULL)
    }

    pub(super) fn update_wiki(&mut self, msg: WikiMsg) -> Task<Message> {
        match msg {
            WikiMsg::Open => {
                if self.wiki.page.is_some() && !self.wiki.loading && self.wiki.error.is_none() {
                    // Back where it was.
                    self.app_view = AppView::Wikipedia;
                    self.set_show_context(false);
                    return Task::none();
                }
                let page = self.wiki.page.clone().unwrap_or(WikiPage::Home);
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
                        // The viewer steps through the picture of the day and
                        // the featured article's.
                        let mut gallery = Vec::new();
                        if let Some(p) = &f.picture {
                            gallery.push((p.image.clone(), vec![Run { text: p.description.clone(), ..Default::default() }]));
                        }
                        if let Some(a) = &f.article {
                            if let Some(t) = &a.thumbnail {
                                gallery.push((t.clone(), vec![Run { text: a.title.clone(), ..Default::default() }]));
                            }
                        }
                        let mut extra: Vec<(String, u32)> = f.most_read.iter().filter_map(|a| a.thumbnail.clone()).map(|u| (u, THUMB)).collect();
                        // The day's picture, large (the full file, scaled here).
                        extra.extend(f.picture.iter().map(|p| (p.image.clone(), LARGE)));
                        self.wiki.featured = Some(f);
                        return self.page_pictures(gallery, extra);
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
                        let gallery = a.gallery().into_iter().map(|(i, c)| (i.url, c)).collect();
                        self.wiki.article = Some(Arc::new(a));
                        return self.page_pictures(gallery, Vec::new());
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
                        let extra = r.iter().filter_map(|s| s.thumbnail.clone()).map(|u| (u, THUMB)).collect();
                        self.wiki.results = r;
                        return self.page_pictures(Vec::new(), extra);
                    }
                    Err(e) => self.wiki.error = Some(e),
                }
            }
            WikiMsg::Pictures(batch) => {
                for (key, result) in batch {
                    // Only pictures this page still wants.
                    if let Some(slot) = self.wiki.pics.get_mut(&key) {
                        *slot = match result {
                            Ok(h) => Pic::Ready(h),
                            Err(e) => {
                                log::warn!("wikipedia picture {key}: {e}");
                                Pic::Failed
                            }
                        };
                    }
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
            WikiMsg::View(url) => {
                let i = match self.wiki.gallery.iter().position(|(u, _)| *u == url) {
                    Some(i) => i,
                    None => {
                        self.wiki.gallery.push((url, Vec::new()));
                        self.wiki.gallery.len() - 1
                    }
                };
                return self.show_picture(i);
            }
            WikiMsg::ViewNext | WikiMsg::ViewPrev => {
                let (Some(i), n) = (self.wiki.viewer, self.wiki.gallery.len()) else { return Task::none() };
                if n > 1 {
                    let next = if matches!(msg, WikiMsg::ViewNext) { (i + 1) % n } else { (i + n - 1) % n };
                    return self.show_picture(next);
                }
            }
            WikiMsg::ViewClose => self.wiki.viewer = None,
        }
        Task::none()
    }

    /// Settings: turning the Wikipedia page on.
    pub(super) fn build_wiki_section(&self) -> Element<'_, Message> {
        widget::settings::section()
            .header(
                widget::row::with_capacity(2)
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(widget::icon(super::library_view::wikipedia_icon()).size(16))
                    .push(widget::text::heading("Wikipedia")),
            )
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

    /// A picture at a given size: clickable to view it full window. While
    /// it loads, a frame of its size holds its place; `None` if it failed.
    fn wiki_pic<'a>(&self, url: &str, max: u32, width: Length, height: Length, pal: &Palette) -> Option<Element<'a, Message>> {
        let el: Element<'a, Message> = match self.wiki.pics.get(&pic_key(url, max)) {
            Some(Pic::Ready(h)) => widget::image(h.clone()).width(width).height(height).content_fit(ContentFit::Contain).into(),
            Some(Pic::Failed) => return None,
            _ => {
                let bg = pal.panel;
                widget::container(widget::Space::new())
                    .width(width)
                    .height(height)
                    .style(move |_: &cosmic::Theme| widget::container::Style { background: Some(Background::Color(bg)), ..Default::default() })
                    .into()
            }
        };
        Some(
            widget::mouse_area(el)
                .on_press(Message::Wiki(WikiMsg::View(url.to_string())))
                .interaction(cosmic::iced::mouse::Interaction::ZoomIn)
                .into(),
        )
    }

    /// The bar above every Wikipedia page: Back, contents, Main page,
    /// Random, search.
    fn wiki_bar(&self) -> Element<'_, Message> {
        let article = matches!(self.wiki.page, Some(WikiPage::Article(_))) && self.wiki.article.is_some();
        widget::row::with_capacity(6)
            .spacing(8)
            .align_y(Alignment::Center)
            .push(tip(
                widget::button::icon(icon::from_name("go-previous-symbolic")).on_press(Message::Wiki(WikiMsg::Back)),
                fl!("opds-back"),
            ))
            .push_maybe((article && self.wiki.toc_hidden).then(|| {
                tip(
                    widget::button::icon(icon::from_name("view-list-symbolic")).on_press(Message::Wiki(WikiMsg::ToggleToc)),
                    fl!("wiki-contents"),
                )
            }))
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
        if let Some(i) = self.wiki.viewer {
            return self.view_wiki_viewer(i, &pal);
        }
        let body: Element<'_, Message> = if let Some(e) = &self.wiki.error {
            widget::column::with_capacity(2).spacing(8).push(widget::text::title3(fl!("wiki-failed"))).push(widget::text::body(e.clone())).into()
        } else if self.wiki.loading {
            widget::container(widget::indeterminate_circular()).width(Length::Fill).center_x(Length::Fill).padding(40).into()
        } else {
            match (&self.wiki.page, &self.wiki.article, &self.wiki.featured) {
                (Some(WikiPage::Article(_)), Some(a), _) => self.view_wiki_article(a, &pal),
                (Some(WikiPage::Search(q)), _, _) => self.view_wiki_results(q, &pal),
                (Some(WikiPage::Home) | None, _, Some(f)) => self.view_wiki_home(f, &pal),
                _ => widget::Space::new().into(),
            }
        };
        let page = widget::column::with_capacity(2).spacing(16).padding([16, 24]).push(self.wiki_bar()).push(body);
        let page = widget::scrollable(widget::container(page).max_width(1500).width(Length::Fill).center_x(Length::Fill))
            .id(scroll_id())
            .width(Length::Fill)
            .height(Length::Fill);
        // The contents stay beside the article as it scrolls, like the site's.
        match (&self.wiki.page, &self.wiki.article) {
            (Some(WikiPage::Article(_)), Some(a)) if !self.wiki.toc_hidden && !self.wiki.loading && self.wiki.error.is_none() => {
                widget::row::with_capacity(2)
                    .push(widget::scrollable(widget::container(self.wiki_toc(a, &pal)).padding([24, 8, 24, 20])).width(Length::Fixed(TOC_WIDTH + 28.0)).height(Length::Fill))
                    .push(page)
                    .into()
            }
            _ => page.into(),
        }
    }

    /// A picture full window: zoomable, with its caption, and Left / Right
    /// for the page's other pictures.
    fn view_wiki_viewer<'a>(&'a self, i: usize, pal: &Palette) -> Element<'a, Message> {
        let Some((url, caption)) = self.wiki.gallery.get(i) else { return widget::Space::new().into() };
        let full = self.wiki.pics.get(&pic_key(&wikipedia::full_size(url), FULL));
        let thumb = self.wiki.pics.get(&pic_key(url, THUMB)).or_else(|| self.wiki.pics.get(&pic_key(url, LARGE)));
        // The full picture, or the small one while it loads.
        let handle = match (full, thumb) {
            (Some(Pic::Ready(h)), _) => Some(h.clone()),
            (_, Some(Pic::Ready(h))) => Some(h.clone()),
            _ => None,
        };
        let loading = !matches!(full, Some(Pic::Ready(_) | Pic::Failed));
        let n = self.wiki.gallery.len();
        let bar = widget::row::with_capacity(6)
            .spacing(8)
            .padding([6, 12])
            .align_y(Alignment::Center)
            .push_maybe((n > 1).then(|| widget::button::icon(icon::from_name("go-previous-symbolic")).on_press(Message::Wiki(WikiMsg::ViewPrev))))
            .push_maybe((n > 1).then(|| widget::text::body(format!("{} / {n}", i + 1))))
            .push_maybe((n > 1).then(|| widget::button::icon(icon::from_name("go-next-symbolic")).on_press(Message::Wiki(WikiMsg::ViewNext))))
            .push_maybe(loading.then(|| widget::indeterminate_circular().size(16.0).bar_height(2.0)))
            .push(widget::text::caption(fl!("wiki-viewer-hint")).width(Length::Fill))
            .push(widget::button::standard(fl!("wiki-close")).on_press(Message::Wiki(WikiMsg::ViewClose)));
        let picture: Element<'a, Message> = match handle {
            Some(h) => widget::image::viewer(h).content_fit(ContentFit::Contain).min_scale(1.0).max_scale(8.0).width(Length::Fill).height(Length::Fill).into(),
            None => widget::container(widget::indeterminate_circular()).width(Length::Fill).height(Length::Fill).center_x(Length::Fill).center_y(Length::Fill).into(),
        };
        let mut col = widget::column::with_capacity(3).push(bar).push(picture);
        if !caption.is_empty() {
            col = col.push(widget::container(line_text(caption, pal, TEXT)).padding([8, 24]).max_height(160.0));
        }
        widget::container(col).width(Length::Fill).height(Length::Fill).class(cosmic::theme::Container::Background).into()
    }

    // ── The main page ─────────────────────────────────────────────────────

    fn view_wiki_home<'a>(&'a self, f: &'a Featured, pal: &Palette) -> Element<'a, Message> {
        // A main-page box: a coloured heading band, then its content.
        let main_box = |title: String, band: [u8; 3], content: Element<'a, Message>| -> Element<'a, Message> {
            let bg = pal.page_color(band);
            let head = widget::container(widget::text(title).size(17.0).font(Font { weight: Weight::Bold, ..serif() }))
                .padding([4, 8])
                .width(Length::Fill)
                .style(move |_: &cosmic::Theme| widget::container::Style { background: Some(Background::Color(bg)), ..Default::default() });
            boxed(widget::column::with_capacity(2).push(head).push(widget::container(content).padding([8, 10])), pal.panel, pal.border)
                .width(Length::Fill)
                .into()
        };

        let mut left = widget::column::with_capacity(3).spacing(16).width(Length::FillPortion(3));
        if let Some(a) = &f.article {
            let mut text = widget::column::with_capacity(3).spacing(6).width(Length::Fill);
            text = text.push(owned_text(vec![link_run(&a.title, true)], pal, TEXT));
            text = text.push(widget::text(a.extract.clone()).size(TEXT).line_height(LineHeight::Relative(LINE)));
            text = text.push(widget::button::link(fl!("wiki-full-article")).on_press(Message::Wiki(WikiMsg::Go(WikiPage::Article(a.title.clone())))));
            let mut row = widget::row::with_capacity(2).spacing(12);
            if let Some(pic) = a.thumbnail.as_deref().and_then(|t| self.wiki_pic(t, THUMB, Length::Fixed(140.0), Length::Fixed(180.0), pal)) {
                row = row.push(pic);
            }
            left = left.push(main_box(fl!("wiki-featured-article"), [0xce, 0xf2, 0xe0], row.push(text).into()));
        }
        if !f.most_read.is_empty() {
            let mut list = widget::column::with_capacity(f.most_read.len()).spacing(8);
            for (i, a) in f.most_read.iter().enumerate() {
                let mut row = widget::row::with_capacity(3).spacing(10).align_y(Alignment::Center);
                row = row.push(widget::text(format!("{}.", i + 1)).size(TEXT).width(Length::Fixed(22.0)));
                if let Some(pic) = a.thumbnail.as_deref().and_then(|t| self.wiki_pic(t, THUMB, Length::Fixed(48.0), Length::Fixed(48.0), pal)) {
                    row = row.push(pic);
                }
                row = row.push(
                    widget::column::with_capacity(2)
                        .width(Length::Fill)
                        .push(owned_text(vec![link_run(&a.title, true)], pal, TEXT))
                        .push(muted(a.description.clone(), TEXT - 2.0, pal)),
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
                let mut line = vec![Run { text: format!("{} – ", e.year), bold: true, ..Default::default() }];
                line.push(Run { text: e.text.clone(), ..Default::default() });
                if let Some(p) = e.pages.first() {
                    line.push(Run { text: " (".into(), ..Default::default() });
                    line.push(link_run(&p.title, false));
                    line.push(Run { text: ")".into(), ..Default::default() });
                }
                list = list.push(owned_text(line, pal, TEXT));
            }
            right = right.push(main_box(fl!("wiki-on-this-day"), [0xce, 0xdf, 0xf2], list.into()));
        }

        let mut col = widget::column::with_capacity(4).spacing(16);
        col = col.push(
            widget::column::with_capacity(2)
                .spacing(4)
                .push(widget::text(fl!("wiki-welcome")).size(30.0).font(serif()))
                .push(muted(fl!("wiki-welcome-sub"), TEXT, pal)),
        );
        col = col.push(widget::row::with_capacity(2).spacing(16).push(left).push(right));
        if let Some(p) = &f.picture {
            // The day's picture across the page, kept in proportion.
            let picture: Option<Element<'a, Message>> = match self.wiki.pics.get(&pic_key(&p.image, LARGE)) {
                Some(Pic::Ready(h)) => Some(
                    widget::mouse_area(widget::image(h.clone()).width(Length::Fill).content_fit(ContentFit::Contain))
                        .on_press(Message::Wiki(WikiMsg::View(p.image.clone())))
                        .interaction(cosmic::iced::mouse::Interaction::ZoomIn)
                        .into(),
                ),
                Some(Pic::Failed) => None,
                _ => Some(
                    widget::container(widget::indeterminate_circular())
                        .width(Length::Fill)
                        .height(Length::Fixed(300.0))
                        .center_x(Length::Fill)
                        .center_y(Length::Fixed(300.0))
                        .into(),
                ),
            };
            let content = widget::column::with_capacity(2)
                .spacing(8)
                .push_maybe(picture.map(|p| widget::container(p).max_width(1100).width(Length::Fill).center_x(Length::Fill)))
                .push(widget::text(p.description.clone()).size(TEXT).line_height(LineHeight::Relative(LINE)).width(Length::Fill));
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
            if let Some(pic) = s.thumbnail.as_deref().and_then(|t| self.wiki_pic(t, THUMB, Length::Fixed(60.0), Length::Fixed(60.0), pal)) {
                row = row.push(pic);
            }
            row = row.push(
                widget::column::with_capacity(3)
                    .spacing(2)
                    .width(Length::Fill)
                    .push(owned_text(vec![link_run(&s.title, false)], pal, TEXT + 2.0))
                    .push_maybe((!s.description.is_empty()).then(|| muted(s.description.clone(), TEXT - 1.0, pal)))
                    .push(widget::text(s.extract.clone()).size(TEXT - 1.0)),
            );
            col = col.push(row);
        }
        col.into()
    }

    // ── Articles ──────────────────────────────────────────────────────────

    /// An article as one flow: headings, text and tables down the page, the
    /// infobox and pictures floated right with the text beside them.
    fn view_wiki_article<'a>(&'a self, a: &'a Article, pal: &Palette) -> Element<'a, Message> {
        let mut flow = FloatFlow::new().spacing(14.0).gap(24.0);
        flow = flow.push(
            widget::container(
                widget::column::with_capacity(3)
                    .spacing(8)
                    .push(widget::text(a.title.clone()).size(32.0).font(serif()))
                    .push(rule(pal.border))
                    .push(muted(fl!("wiki-from"), TEXT - 1.0, pal)),
            )
            .id(section_id(0)),
        );
        for (i, s) in a.sections.iter().enumerate() {
            match s.level {
                1 => {}
                2 => {
                    flow = flow.push(
                        widget::container(
                            widget::column::with_capacity(2)
                                .spacing(4)
                                .push(widget::text(s.title.clone()).size(25.0).font(serif()))
                                .push(rule(pal.border)),
                        )
                        .padding([12, 0, 0, 0])
                        .id(section_id(i)),
                    );
                }
                level => {
                    let size = if level == 3 { 18.0 } else { 16.0 };
                    flow = flow.push(widget::container(widget::text(s.title.clone()).size(size).font(bold())).padding([6, 0, 0, 0]).id(section_id(i)));
                }
            }
            // The lead: hatnotes across the page, then the infobox floats.
            let mut blocks = s.blocks.iter().peekable();
            if i == 0 {
                while let Some(b) = blocks.next_if(|b| matches!(b, Block::Hatnote(_))) {
                    flow = flow.push(self.wiki_block(b, pal));
                }
                if let Some(ib) = &a.infobox {
                    flow = flow.float(self.wiki_table(ib, pal, true));
                }
            }
            for b in blocks {
                flow = match b {
                    Block::Figure { image, caption } => match self.wiki_figure(image, caption, pal) {
                        Some(fig) => flow.float(fig),
                        None if !caption.is_empty() => flow.push(line_text(caption, pal, TEXT - 2.0)),
                        None => flow,
                    },
                    Block::Table(t) if t.float_right => flow.float(widget::container(self.wiki_table(t, pal, false)).width(Length::Fixed(360.0))),
                    other => flow.push(self.wiki_block(other, pal)),
                };
            }
        }
        // Credit and licence.
        flow = flow.push(
            widget::column::with_capacity(2)
                .spacing(10)
                .padding([20, 0, 0, 0])
                .push(rule(pal.border))
                .push(
                    widget::row::with_capacity(3)
                        .spacing(12)
                        .align_y(Alignment::Center)
                        .push(widget::container(muted(fl!("wiki-credit", title = a.title.as_str()), TEXT - 2.0, pal)).width(Length::Fill))
                        .push(widget::button::link(fl!("wiki-license")).on_press(Message::OpenUrl(wikipedia::LICENSE_URL.into())))
                        .push(widget::button::standard(fl!("wiki-view-on")).on_press(Message::Wiki(WikiMsg::OpenOnWikipedia))),
                ),
        );
        flow.into()
    }

    /// The contents: top-level sections, each opening onto its subsections.
    fn wiki_toc<'a>(&'a self, a: &'a Article, pal: &Palette) -> Element<'a, Message> {
        let link = pal.link;
        let item = move |label: String, msg: WikiMsg| -> Element<'a, Message> {
            widget::button::custom(widget::text(label).size(TEXT - 1.0).class(cosmic::theme::Text::Color(link)))
                .class(cosmic::theme::Button::Text)
                .padding([3, 4])
                .on_press(Message::Wiki(msg))
                .into()
        };
        let mut col = widget::column::with_capacity(a.sections.len() + 3).spacing(1).width(Length::Fixed(TOC_WIDTH));
        col = col.push(
            widget::row::with_capacity(2)
                .spacing(8)
                .align_y(Alignment::Center)
                .push(widget::text(fl!("wiki-contents")).size(TEXT).font(bold()))
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
                let toggle: Element<'a, Message> = if has_children {
                    widget::button::icon(icon::from_name(if open { "pan-down-symbolic" } else { "pan-end-symbolic" }))
                        .extra_small()
                        .on_press(Message::Wiki(WikiMsg::ToggleTocSection(i)))
                        .into()
                } else {
                    widget::Space::new().width(Length::Fixed(24.0)).into()
                };
                col = col.push(widget::row::with_capacity(2).align_y(Alignment::Center).push(toggle).push(item(s.title.clone(), WikiMsg::JumpTo(Some(i)))));
            } else if s.level > 2 && parent.is_some_and(|p| self.wiki.toc_open.contains(&p)) {
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

    /// A picture in its frame with its caption, as the site floats it.
    fn wiki_figure<'a>(&'a self, image: &wikipedia::Image, caption: &'a Line, pal: &Palette) -> Option<Element<'a, Message>> {
        let w = image.width.min(280.0);
        let h = image.height * w / image.width.max(1.0);
        let pic = self.wiki_pic(&image.url, THUMB, Length::Fixed(w), Length::Fixed(h), pal)?;
        Some(
            boxed(
                widget::column::with_capacity(2)
                    .spacing(6)
                    .width(Length::Fixed(w))
                    .push(pic)
                    .push_maybe((!caption.is_empty()).then(|| line_text(caption, pal, TEXT - 2.0))),
                pal.panel,
                pal.border,
            )
            .padding(4)
            .into(),
        )
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
                            .push(widget::Space::new().width(Length::Fixed(12.0 + 20.0 * d as f32)))
                            .push(widget::text(mark).size(TEXT).line_height(LineHeight::Relative(LINE)))
                            .push(line_text(line, pal, TEXT)),
                    );
                }
                col.into()
            }
            Block::Figure { image, caption } => match self.wiki_figure(image, caption, pal) {
                Some(fig) => widget::container(fig).width(Length::Fill).align_x(Alignment::End).into(),
                None => line_text(caption, pal, TEXT - 2.0),
            },
            Block::Table(t) => self.wiki_table(t, pal, false),
            Block::Bar(em) => {
                let color = pal.link;
                widget::container(widget::Space::new().width(Length::Fixed((em * TEXT).max(1.0))).height(Length::Fixed(TEXT * 0.9)))
                    .style(move |_: &cosmic::Theme| widget::container::Style { background: Some(Background::Color(color)), ..Default::default() })
                    .into()
            }
        }
    }

    /// A table: each row's cells side by side, wider for wider spans.
    fn wiki_table<'a>(&'a self, t: &'a Table, pal: &Palette, infobox: bool) -> Element<'a, Message> {
        let mut col = widget::column::with_capacity(t.rows.len() + 1).spacing(if infobox { 4 } else { 0 });
        if !t.caption.is_empty() {
            col = col.push(widget::container(line_text(&t.caption, pal, TEXT)).padding([4, 8]));
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
        if infobox {
            boxed(col, pal.panel, pal.border).padding(6).width(Length::Fixed(INFOBOX_WIDTH)).into()
        } else {
            boxed(col, Color::TRANSPARENT, pal.border).width(Length::Fill).into()
        }
    }

    fn wiki_cell<'a>(&'a self, c: &'a Cell, pal: &Palette, size: f32, infobox: bool) -> widget::Container<'a, Message, cosmic::Theme> {
        let mut col = widget::column::with_capacity(c.content.len()).spacing(4).width(Length::Fill);
        if c.center {
            col = col.align_x(Alignment::Center);
        }
        let mut blocks = c.content.iter().peekable();
        while let Some(b) = blocks.next() {
            col = col.push(match b {
                // A chart's bar with its number beside it.
                Block::Bar(_) => {
                    let mut row = widget::row::with_capacity(2).spacing(6).align_y(Alignment::Center).push(self.wiki_block(b, pal));
                    if let Some(Block::Para(l)) = blocks.next_if(|n| matches!(n, Block::Para(_))) {
                        row = row.push(line_text(l, pal, size));
                    }
                    row.into()
                }
                Block::Para(l) => {
                    let spans: Vec<Span<'a, String, Font>> = l.iter().map(|r| run_span(r, r.text.as_str().into(), pal, size)).collect();
                    let t = rich_text(spans).size(size).line_height(LineHeight::Relative(1.45)).on_link_click(|l| Message::Wiki(WikiMsg::Link(l)));
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
                        .push_maybe(self.wiki_pic(&image.url, THUMB, Length::Fixed(w), Length::Fixed(h), pal))
                        .push_maybe((!caption.is_empty()).then(|| line_text(caption, pal, size)))
                        .into()
                }
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
}
