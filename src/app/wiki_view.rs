//! The Wikipedia page (opt-in: Settings → Library menu): Wikipedia's main
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
use crate::fandom::{self, Wiki};
use crate::wikipedia::{self, Article, Block, Cell, Featured, GalleryItem, Line, Run, Summary, Table};

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
    /// Wikipedia's main page, a Fandom wiki's main page, or (with no wiki)
    /// the list of wikis followed.
    Home,
    Article(String),
    Search(String),
}

/// Where pages come from: Wikipedia, or a Fandom wiki (`None`: the list
/// of wikis followed, before one is picked).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Site {
    #[default]
    Wikipedia,
    Fandom(Option<Wiki>),
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
    /// From the library: Wikipedia's main page, or the Fandom page (`true`),
    /// or where it was.
    Open(bool),
    Go(WikiPage),
    Back,
    Random,
    RandomFound(Result<String, String>),
    HomeLoaded(u64, Result<Home, String>),
    ArticleLoaded(u64, Result<Article, String>),
    SearchLoaded(u64, Result<Vec<Summary>, String>),
    /// Pictures that finished loading, in a batch.
    Pictures(Vec<Arrival>),
    SearchInput(String),
    Search,
    /// A link: `wiki:<Title>` or a web address.
    Link(String),
    ToggleToc,
    /// The contents shown or hidden, keeping this place in view.
    TocToggled(Option<Anchor>),
    /// Shows or hides a section's subsections in the contents.
    ToggleTocSection(usize),
    /// Scrolls to a section (`None`: the top).
    JumpTo(Option<usize>),
    ScrollTo(Option<f32>),
    OpenOnWikipedia,
    /// Opens the picture viewer on a picture (its thumbnail's address).
    View(String),
    ViewNext,
    ViewPrev,
    ViewClose,
    /// The Wikipedia to read, by its place in `wikipedia::LANGUAGES`.
    SetLanguage(usize),
    /// Up (-1.0) or Down (1.0) pressed, not a repeat.
    ArrowDown(f32),
    /// Up or Down let go.
    ArrowUp(f32),
    /// A frame while an arrow scrolls the page.
    ScrollFrame(std::time::Instant),
    /// The window's height, for the arrows' speed.
    WindowHeight(f32),
    /// How far down the page is scrolled.
    Scrolled(f32),
    /// Pressing on or dragging over a text (by its key).
    Select(usize, cosmic::iced::widget::text::Select),
    /// The left button let go.
    SelectEnd,
    /// A press somewhere that isn't text.
    SelectClear,
    SelectAll,
    /// The mouse's height in the window while a selection is dragged.
    DragAt(f32),
    /// Where the page's scrolling area is in the window.
    ScrollBounds(Option<Rectangle>),
    Copy,
    /// A tab picked in a set of tabs (its id), by its place.
    Tab(usize, usize),
    /// Fandom: the box for adding a wiki.
    AddInput(String),
    AddWiki,
    /// A wiki found, to follow (`None`) or to open at a page (from a link).
    WikiFound(Result<Wiki, String>, Option<String>),
    /// Opens a followed wiki's main page (by its id).
    OpenWiki(String),
    /// Follows the wiki being read, or stops following one (by its id).
    Follow,
    Unfollow(String),
    /// The list of wikis followed.
    Following,
}

/// Up or Down held on a Wikipedia page: it scrolls at a speed that follows
/// the window's height, like the comic reader's arrows (libcosmic patch 16).
#[derive(Clone, Copy, Debug)]
struct KeyScroll {
    /// -1.0 for Up, 1.0 for Down.
    dir: f32,
    /// Pixels per second now.
    speed: f32,
    last: std::time::Instant,
    /// Still held; let go, it glides to a stop.
    held: bool,
}

impl KeyScroll {
    const SCREENS_PER_SECOND: f32 = 0.9;
    const MIN_SPEED: f32 = 450.0;
    const MAX_SPEED: f32 = 1400.0;
    /// The share of the top speed it starts at, so a tap moves too.
    const START: f32 = 0.6;
    /// Seconds from the start to the top speed.
    const RAMP: f32 = 0.15;
    /// After letting go, the speed falls to about a third every this many
    /// seconds.
    const GLIDE: f32 = 0.1;
    const STOP: f32 = 20.0;

    fn top_speed(height: f32) -> f32 {
        (height * Self::SCREENS_PER_SECOND).clamp(Self::MIN_SPEED, Self::MAX_SPEED)
    }

    /// How far the page moves this frame; `None` once it has stopped.
    fn step(&mut self, now: std::time::Instant, height: f32) -> Option<f32> {
        let dt = now.saturating_duration_since(self.last).as_secs_f32().min(0.05);
        self.last = now;
        let moved = self.dir * self.speed * dt;
        if self.held {
            let top = Self::top_speed(height);
            self.speed = (self.speed + top * (1.0 - Self::START) * dt / Self::RAMP).min(top);
        } else {
            self.speed *= (-dt / Self::GLIDE).exp();
            if self.speed < Self::STOP {
                return None;
            }
        }
        Some(moved)
    }
}

/// The right-click menu on a Wikipedia page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAction {
    Copy,
    SelectAll,
}

impl cosmic::widget::menu::Action for TextAction {
    type Message = Message;

    fn message(&self) -> Message {
        Message::Wiki(match self {
            TextAction::Copy => WikiMsg::Copy,
            TextAction::SelectAll => WikiMsg::SelectAll,
        })
    }
}

/// The main page for a day: the site's own (English), and the daily feed
/// (most read; the whole page for a language whose page isn't read).
#[derive(Clone, Debug, Default)]
pub struct Home {
    /// `YYYY/MM/DD` (UTC) it's for: a new day loads it again.
    day: String,
    main: Option<wikipedia::MainPage>,
    feed: Option<Featured>,
}

/// How much width each column of a table wants: its longest text (capped,
/// so one long cell doesn't take the table), and never less than its
/// longest word, so headings like "Attendance" don't break.
fn column_weights(t: &Table) -> Vec<f32> {
    fn text(blocks: &[Block], out: &mut String) {
        for b in blocks {
            match b {
                Block::Para(l) | Block::Hatnote(l) | Block::Quote(l) => {
                    out.extend(l.iter().map(|r| r.text.as_str()));
                    out.push(' ');
                }
                Block::List { items, .. } => {
                    for (_, l) in items {
                        out.extend(l.iter().map(|r| r.text.as_str()));
                        out.push(' ');
                    }
                }
                Block::Figure { image, .. } => out.push_str(&"m".repeat((image.width / 8.0) as usize)),
                Block::Table(_) | Block::Bar(_) => out.push_str("mmmmmmmmmm"),
                Block::Gallery(items) => out.push_str(&"m".repeat(items.len().min(4) * 20)),
                Block::Tabs { tabs, .. } => {
                    if let Some((_, first)) = tabs.first() {
                        text(first, out);
                    }
                }
            }
        }
    }
    let columns = t.rows.iter().map(|r| r.cells.iter().map(|c| c.colspan.max(1)).sum::<usize>()).max().unwrap_or(0);
    let mut weights = vec![0.0f32; columns];
    for r in &t.rows {
        let mut at = 0;
        for c in &r.cells {
            let span = c.colspan.max(1);
            // A cell across columns says little about any one of them.
            if span == 1 {
                let mut s = String::new();
                text(&c.content, &mut s);
                let len = s.trim().chars().count() as f32;
                let word = s.split_whitespace().map(|w| w.chars().count()).max().unwrap_or(0) as f32;
                let want = len.min(32.0).max(word + 2.0).max(3.0);
                if let Some(w) = weights.get_mut(at) {
                    *w = w.max(want);
                }
            }
            at += span;
        }
    }
    for w in &mut weights {
        if *w == 0.0 {
            *w = 6.0;
        }
    }
    weights
}

/// The main page's box colours, as the site's headings.
fn box_band(id: &str) -> [u8; 3] {
    match id {
        "tfa" | "dyk" => [0xce, 0xf2, 0xe0],
        "itn" | "otd" => [0xce, 0xdf, 0xf2],
        "tfl" => [0xf2, 0xce, 0xdd],
        _ => [0xdd, 0xce, 0xf2],
    }
}

/// The languages' own names, for the dropdown.
fn language_names() -> &'static [&'static str] {
    static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| wikipedia::LANGUAGES.iter().map(|(_, name)| *name).collect())
}

#[derive(Default)]
pub struct WikiState {
    /// Wikipedia or Fandom: each has a state of its own (see
    /// `App::wiki_parked`).
    site: Site,
    page: Option<WikiPage>,
    /// Pages to go Back to, each with where it was and how far down it
    /// was scrolled.
    history: Vec<(Site, WikiPage, f32)>,
    /// How far down the page is scrolled now (from the scrollable's
    /// reports, throttled: exact once scrolling settles).
    offset: f32,
    /// Where to scroll once the page being loaded arrives (going Back).
    restore: Option<f32>,
    generation: u64,
    loading: bool,
    error: Option<String>,
    featured: Option<Home>,
    article: Option<Arc<Article>>,
    results: Vec<Summary>,
    /// Pictures by [`pic_key`], kept in memory while the page is open.
    pics: HashMap<String, Pic>,
    /// The contents sidebar is open (it starts closed; the button or C opens it).
    toc_shown: bool,
    /// Sections (by index) whose subsections are shown in the contents.
    toc_open: std::collections::HashSet<usize>,
    search_input: String,
    /// The page's pictures (thumbnail address, caption), for the viewer.
    gallery: Vec<(String, Line)>,
    /// The picture shown full window, by its place in `gallery`.
    viewer: Option<usize>,
    key_scroll: Option<KeyScroll>,
    sel: Option<TextSel>,
    /// The mouse's height while dragging a selection, and the page's
    /// scrolling area, to scroll when it's held near or past an edge.
    drag_y: Option<f32>,
    scroll_bounds: Option<Rectangle>,
    /// The last frame scrolled by a drag.
    drag_frame: Option<std::time::Instant>,
    /// The page's selectable texts in reading order, as the last `view`
    /// laid them out (a text's place here is its key in [`TextSel`]).
    texts: std::cell::RefCell<Vec<String>>,
    /// The window's height (0 until it's known).
    window_height: f32,
    /// The tab picked in each set of tabs on the page (by its id).
    tabs: HashMap<usize, usize>,
    /// Fandom: the box for adding a wiki, a search for one going on, and
    /// why the last one found nothing.
    add_input: String,
    adding: bool,
    add_error: Option<String>,
}

impl WikiState {
    /// The Fandom page's state, before it's opened.
    pub fn fandom() -> Self {
        WikiState { site: Site::Fandom(None), ..Default::default() }
    }

    /// The Fandom wiki being read, if any.
    fn wiki(&self) -> Option<&Wiki> {
        match &self.site {
            Site::Fandom(w) => w.as_ref(),
            Site::Wikipedia => None,
        }
    }

    /// An article is shown (a Fandom wiki's main page is one too).
    fn on_article(&self) -> bool {
        self.article.is_some()
            && match &self.page {
                Some(WikiPage::Article(_)) => true,
                Some(WikiPage::Home) => self.wiki().is_some(),
                _ => false,
            }
    }
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
    fn a_selection_covers_the_texts_between_its_ends() {
        let sel = TextSel { anchor: (4, 7), focus: (2, 3), dragging: false };
        assert_eq!(sel.range(1), None);
        assert_eq!(sel.range(2), Some((3, usize::MAX)));
        assert_eq!(sel.range(3), Some((0, usize::MAX)));
        assert_eq!(sel.range(4), Some((0, 7)));
        assert_eq!(sel.range(5), None);
        let one = TextSel { anchor: (1, 9), focus: (1, 2), dragging: true };
        assert_eq!(one.range(1), Some((2, 9)));
        let empty = TextSel { anchor: (1, 2), focus: (1, 2), dragging: true };
        assert_eq!(empty.range(1), None);
    }

    #[test]
    fn a_selection_dragged_past_an_edge_scrolls_faster_further_out() {
        let area = Rectangle::new(cosmic::iced::Point::new(0.0, 100.0), cosmic::iced::Size::new(800.0, 600.0));
        assert_eq!(drag_scroll_speed(400.0, area), 0.0);
        assert!(drag_scroll_speed(120.0, area) < 0.0);
        assert!(drag_scroll_speed(50.0, area) < drag_scroll_speed(120.0, area));
        assert!(drag_scroll_speed(690.0, area) > 0.0);
        assert!(drag_scroll_speed(900.0, area) > drag_scroll_speed(690.0, area));
        assert_eq!(drag_scroll_speed(-5000.0, area), -3000.0);
    }

    #[test]
    fn a_place_on_the_page_survives_rewrapping() {
        // Section 1 has no heading; 3 ends at the page's end.
        let before = Layout { sections: vec![Some(0.0), None, Some(1000.0), Some(3000.0)], offset: 2500.0, height: 4000.0 };
        let a = before.anchor().unwrap();
        assert_eq!(a, Anchor { section: 2, through: 0.75 });
        // Wider: everything shorter.
        let after = Layout { sections: vec![Some(0.0), None, Some(800.0), Some(2000.0)], offset: 0.0, height: 2600.0 };
        assert_eq!(after.resolve(a), Some(1700.0));
        let end = Layout { offset: 3500.0, ..before.clone() };
        assert_eq!(after.resolve(end.anchor().unwrap()), Some(2300.0));
    }

    #[test]
    fn table_columns_are_as_wide_as_what_is_in_them() {
        let cell = |t: &str, header: bool| Cell {
            header,
            colspan: 1,
            background: None,
            center: false,
            content: vec![Block::Para(vec![Run { text: t.into(), ..Default::default() }])],
        };
        let row = |cells: Vec<Cell>| wikipedia::Row { cells };
        let t = Table {
            rows: vec![
                row(vec![cell("Game", true), cell("Score", true), cell("Attendance", true)]),
                row(vec![cell("1", false), cell("Tampa Bay Rays – 2, Milwaukee Brewers – 3", false), cell("43,250", false)]),
            ],
            caption: Vec::new(),
            float_right: false,
            infobox: false,
        };
        let w = column_weights(&t);
        assert_eq!(w.len(), 3);
        // The score takes the most room; "Game" and "Attendance" keep theirs.
        assert!(w[1] > w[2] && w[2] > w[0]);
        assert!(w[0] >= 6.0 && w[2] >= 12.0);
    }

    #[test]
    fn double_click_takes_the_word() {
        let t = "The cat’s café, 1990.";
        assert_eq!(&t[..][word_at(t, 6).0..word_at(t, 6).1], "cat’s");
        let (a, b) = word_at(t, t.find("caf").unwrap() + 1);
        assert_eq!(&t[a..b], "café");
        assert_eq!(word_at(t, 3), (0, 3));
        assert_eq!(slice("héllo", 2, usize::MAX), "éllo");
        assert_eq!(slice("héllo", 0, 2), "hé");
    }

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

/// Where every section starts on the page (from the top of the content;
/// `None` for one without a heading) and how far down it's scrolled.
#[derive(Clone, Debug, Default)]
struct Layout {
    sections: Vec<Option<f32>>,
    offset: f32,
    height: f32,
}

/// A place on the page that survives the text re-wrapping: a section and
/// how far through it (0 at its heading, 1 at the next).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    section: usize,
    through: f32,
}

impl Layout {
    /// Where the section that holds `y` starts and ends.
    fn span(&self, i: usize) -> Option<(f32, f32)> {
        let start = self.sections.get(i).copied().flatten()?;
        let end = self.sections[i + 1..].iter().flatten().next().copied().unwrap_or(self.height);
        Some((start, end.max(start)))
    }

    fn anchor(&self) -> Option<Anchor> {
        let (section, _) = self.sections.iter().enumerate().filter_map(|(i, y)| Some((i, (*y)?))).take_while(|(_, y)| *y <= self.offset).last()?;
        let (start, end) = self.span(section)?;
        let through = if end > start { ((self.offset - start) / (end - start)).clamp(0.0, 1.0) } else { 0.0 };
        Some(Anchor { section, through })
    }

    fn resolve(&self, a: Anchor) -> Option<f32> {
        let (start, end) = self.span(a.section)?;
        Some(start + a.through * (end - start))
    }
}

/// Measures the page's sections (see [`Layout`]).
struct MeasureSections {
    ids: Vec<widget::Id>,
    found: Vec<Option<Rectangle>>,
    content: Option<Rectangle>,
    offset: f32,
}

impl MeasureSections {
    fn new(count: usize) -> Self {
        Self { ids: (0..count).map(section_id).collect(), found: vec![None; count], content: None, offset: 0.0 }
    }
}

impl Operation<Option<Layout>> for MeasureSections {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<Layout>>)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&widget::Id>, bounds: Rectangle) {
        if let Some(i) = id.and_then(|id| self.ids.iter().position(|x| x == id)) {
            self.found[i] = Some(bounds);
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&widget::Id>,
        _bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn cosmic::iced::advanced::widget::operation::Scrollable,
    ) {
        if id == Some(&scroll_id()) {
            self.content = Some(content_bounds);
            self.offset = translation.y;
        }
    }

    fn finish(&self) -> Outcome<Option<Layout>> {
        Outcome::Some(self.content.map(|c| Layout {
            sections: self.found.iter().map(|b| b.map(|b| b.y - c.y)).collect(),
            offset: self.offset,
            height: c.height,
        }))
    }
}

/// Finds where the page's scrolling area is in the window.
struct FindScrollBounds(Option<Rectangle>);

impl Operation<Option<Rectangle>> for FindScrollBounds {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<Rectangle>>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        _content_bounds: Rectangle,
        _translation: Vector,
        _state: &mut dyn cosmic::iced::advanced::widget::operation::Scrollable,
    ) {
        if id == Some(&scroll_id()) {
            self.0 = Some(bounds);
        }
    }

    fn finish(&self) -> Outcome<Option<Rectangle>> {
        Outcome::Some(self.0)
    }
}

/// How fast a selection dragged near or past the top or bottom edge
/// scrolls the page (pixels per second, up negative): faster the further
/// out the mouse is, like a browser.
fn drag_scroll_speed(y: f32, area: Rectangle) -> f32 {
    const EDGE: f32 = 30.0;
    const PER_PIXEL: f32 = 14.0;
    const MAX: f32 = 3000.0;
    let top = area.y + EDGE;
    let bottom = area.y + area.height - EDGE;
    if y < top {
        -((top - y) * PER_PIXEL).min(MAX)
    } else if y > bottom {
        ((y - bottom) * PER_PIXEL).min(MAX)
    } else {
        0.0
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

type Rich<'a> = cosmic::iced::widget::text::Rich<'a, String, Message, cosmic::Theme, cosmic::Renderer>;

/// A place in the page's selectable text: which text (in reading order)
/// and a byte offset into it.
type TextPos = (usize, usize);

/// Text selected on a Wikipedia page, possibly across many paragraphs.
#[derive(Clone, Copy, Debug)]
struct TextSel {
    anchor: TextPos,
    focus: TextPos,
    /// The left button is still down.
    dragging: bool,
}

impl TextSel {
    fn ordered(&self) -> (TextPos, TextPos) {
        if self.anchor <= self.focus { (self.anchor, self.focus) } else { (self.focus, self.anchor) }
    }

    /// The selected part of text `k`; an end of `usize::MAX` is its end.
    fn range(&self, k: usize) -> Option<(usize, usize)> {
        let (s, e) = self.ordered();
        if s == e || k < s.0 || k > e.0 {
            return None;
        }
        Some((if k == s.0 { s.1 } else { 0 }, if k == e.0 { e.1 } else { usize::MAX }))
    }
}

/// The word around a byte offset of `text`.
fn word_at(text: &str, offset: usize) -> (usize, usize) {
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '\'' || c == '’';
    let offset = offset.min(text.len());
    let start = text[..offset].char_indices().rev().take_while(|(_, c)| word(*c)).last().map_or(offset, |(i, _)| i);
    let end = text[offset..].char_indices().find(|(_, c)| !word(*c)).map_or(text.len(), |(i, _)| offset + i);
    (start, end)
}

/// `text[start..end]`, with the ends clamped to the text and its characters.
fn slice(text: &str, start: usize, end: usize) -> &str {
    let mut start = start.min(text.len());
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = end.min(text.len()).max(start);
    while !text.is_char_boundary(end) {
        end += 1;
    }
    &text[start..end]
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
        let site = self.wiki.site.clone();
        self.wiki_visit(site, page, remember)
    }

    /// Shows a page of a site (Wikipedia, a Fandom wiki, or the wikis
    /// followed), remembering the one shown now for Back.
    fn wiki_visit(&mut self, site: Site, page: WikiPage, remember: bool) -> Task<Message> {
        if remember {
            if let Some(current) = self.wiki.page.take() {
                if current != page || self.wiki.site != site {
                    self.wiki.history.push((self.wiki.site.clone(), current, self.wiki.offset));
                }
            }
        }
        self.wiki.site = site;
        self.wiki.offset = 0.0;
        self.wiki.page = Some(page.clone());
        self.wiki.generation += 1;
        self.wiki.error = None;
        self.wiki.loading = true;
        self.wiki.viewer = None;
        self.wiki.sel = None;
        self.wiki.tabs.clear();
        self.app_view = AppView::Wikipedia;
        self.set_show_context(false);
        let generation = self.wiki.generation;
        let top = scroll_mod::snap_to(scroll_id(), RelativeOffset { x: None, y: Some(0.0) });
        let wiki = match &self.wiki.site {
            Site::Wikipedia => None,
            Site::Fandom(None) => {
                // The wikis followed: nothing to load but their logos.
                self.set_header_title(fl!("fandom-following"));
                self.wiki.loading = false;
                self.wiki.article = None;
                let logos = self.settings.fandom_wikis.iter().filter_map(|w| w.logo.clone()).map(|l| (l, THUMB)).collect();
                let pictures = self.page_pictures(Vec::new(), logos);
                let restore = self.wiki_restore();
                return Task::batch([pictures, if self.wiki.offset > 0.0 { restore } else { top }]);
            }
            Site::Fandom(Some(w)) => Some(w.clone()),
        };
        let load = match (page, wiki) {
            (WikiPage::Home, None) => {
                self.set_header_title(fl!("wiki-main-page"));
                if let Some(f) = self.wiki.featured.clone().filter(|h| h.day == wikipedia::chrono_today()) {
                    // Going Back to it: `HomeLoaded` scrolls to where it was.
                    let top = if self.wiki.restore.is_some() { Task::none() } else { top };
                    return Task::batch([self.update_wiki(WikiMsg::HomeLoaded(generation, Ok(f))), top]);
                }
                cosmic::task::future(async move {
                    let day = wikipedia::chrono_today();
                    let home = wikipedia::home().await.map(|(main, feed)| Home { day, main, feed });
                    Message::Wiki(WikiMsg::HomeLoaded(generation, home))
                })
            }
            (WikiPage::Home, Some(w)) => {
                self.set_header_title(w.name.clone());
                self.wiki.article = None;
                self.wiki.toc_open.clear();
                cosmic::task::future(async move { Message::Wiki(WikiMsg::ArticleLoaded(generation, fandom::article(&w, &w.main_page).await)) })
            }
            (WikiPage::Article(title), wiki) => {
                self.set_header_title(title.clone());
                self.wiki.article = None;
                self.wiki.toc_open.clear();
                cosmic::task::future(async move {
                    let a = match wiki {
                        Some(w) => fandom::article(&w, &title).await,
                        None => wikipedia::article(&title).await,
                    };
                    Message::Wiki(WikiMsg::ArticleLoaded(generation, a))
                })
            }
            (WikiPage::Search(query), wiki) => {
                self.set_header_title(fl!("wiki-search-results", query = query.as_str()));
                cosmic::task::future(async move {
                    let r = match wiki {
                        Some(w) => fandom::search(&w, &query).await,
                        None => wikipedia::search(&query).await,
                    };
                    Message::Wiki(WikiMsg::SearchLoaded(generation, r))
                })
            }
        };
        Task::batch([load, top])
    }

    /// Wikipedia and Fandom each keep their own place: switching to the
    /// other brings its state out and puts this one away.
    fn wiki_enter(&mut self, fandom: bool) {
        if matches!(self.wiki.site, Site::Fandom(_)) != fandom {
            std::mem::swap(&mut self.wiki, &mut self.wiki_parked);
            self.wiki.sel = None;
            self.wiki.key_scroll = None;
        }
    }

    /// Whether the Fandom page is the one shown (or last shown).
    pub(super) fn wiki_is_fandom(&self) -> bool {
        matches!(self.wiki.site, Site::Fandom(_))
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
        // C shows or hides an article's contents.
        if self.wiki.viewer.is_none() {
            let article = self.wiki.on_article();
            return match key {
                Key::Character(c) if c.eq_ignore_ascii_case("c") && self.modifiers.command() && self.wiki.sel.is_some() => {
                    Some(self.update_wiki(WikiMsg::Copy))
                }
                Key::Character(c) if c.eq_ignore_ascii_case("a") && self.modifiers.command() => Some(self.update_wiki(WikiMsg::SelectAll)),
                Key::Character(c) if article && !self.modifiers.command() && c.eq_ignore_ascii_case("c") => {
                    Some(self.update_wiki(WikiMsg::ToggleToc))
                }
                _ => None,
            };
        }
        Some(match key {
            Key::Named(Named::Escape) => self.update_wiki(WikiMsg::ViewClose),
            Key::Named(Named::ArrowRight) => self.update_wiki(WikiMsg::ViewNext),
            Key::Named(Named::ArrowLeft) => self.update_wiki(WikiMsg::ViewPrev),
            _ => Task::none(),
        })
    }

    pub(super) fn wiki_subscription(&self) -> Subscription<Message> {
        if self.settings.wikipedia || self.settings.fandom {
            use cosmic::iced::{event, keyboard, mouse, window};
            let pictures = Subscription::run(arrivals).map(|batch| Message::Wiki(WikiMsg::Pictures(batch)));
            let height = event::listen_with(|event, _, _| match event {
                event::Event::Window(window::Event::Opened { size, .. } | window::Event::Resized(size)) => {
                    Some(Message::Wiki(WikiMsg::WindowHeight(size.height)))
                }
                _ => None,
            });
            if self.app_view != AppView::Wikipedia || self.wiki.viewer.is_some() {
                return Subscription::batch([pictures, height]);
            }
            fn arrow(key: &keyboard::Key) -> Option<f32> {
                match key {
                    keyboard::Key::Named(keyboard::key::Named::ArrowUp) => Some(-1.0),
                    keyboard::Key::Named(keyboard::key::Named::ArrowDown) => Some(1.0),
                    _ => None,
                }
            }
            // Not while the search box has the keys (it captures them).
            let arrows = event::listen_with(|event, status, _| match event {
                event::Event::Keyboard(keyboard::Event::KeyPressed { key, repeat: false, .. }) if status == event::Status::Ignored => {
                    arrow(&key).map(|d| Message::Wiki(WikiMsg::ArrowDown(d)))
                }
                event::Event::Keyboard(keyboard::Event::KeyReleased { key, .. }) => arrow(&key).map(|d| Message::Wiki(WikiMsg::ArrowUp(d))),
                // Selecting text: a press on text is captured by it.
                event::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(Message::Wiki(WikiMsg::SelectEnd)),
                event::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if status == event::Status::Ignored => {
                    Some(Message::Wiki(WikiMsg::SelectClear))
                }
                _ => None,
            });
            let frames = if self.wiki.key_scroll.is_some() || self.wiki_drag_speed().is_some() {
                window::frames().map(|(_, now)| Message::Wiki(WikiMsg::ScrollFrame(now)))
            } else {
                Subscription::none()
            };
            // Where the mouse is while a selection is dragged.
            let drag = if self.wiki.sel.is_some_and(|s| s.dragging) {
                event::listen_with(|event, _, _| match event {
                    event::Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::Wiki(WikiMsg::DragAt(position.y))),
                    _ => None,
                })
            } else {
                Subscription::none()
            };
            Subscription::batch([pictures, height, arrows, frames, drag])
        } else {
            Subscription::none()
        }
    }

    /// How fast a dragged selection scrolls the page now, if it does.
    fn wiki_drag_speed(&self) -> Option<f32> {
        if !self.wiki.sel.is_some_and(|s| s.dragging) {
            return None;
        }
        let speed = drag_scroll_speed(self.wiki.drag_y?, self.wiki.scroll_bounds?);
        (speed != 0.0).then_some(speed)
    }

    /// Scrolls a page shown again back to where it was.
    fn wiki_scroll_back(&self) -> Task<Message> {
        scroll_mod::scroll_to(scroll_id(), AbsoluteOffset { x: None, y: Some(self.wiki.offset) })
    }

    /// Going Back: once the page has arrived, scrolls to where it was.
    fn wiki_restore(&mut self) -> Task<Message> {
        match self.wiki.restore.take() {
            Some(y) => {
                self.wiki.offset = y;
                self.wiki_scroll_back()
            }
            None => Task::none(),
        }
    }

    /// The page's height, near enough: the window's.
    fn wiki_height(&self) -> f32 {
        if self.wiki.window_height > 0.0 { self.wiki.window_height } else { 800.0 }
    }

    fn show_picture(&mut self, i: usize) -> Task<Message> {
        let Some((url, _)) = self.wiki.gallery.get(i).cloned() else { return Task::none() };
        self.wiki.viewer = Some(i);
        self.want([wikipedia::full_size(&url)], FULL)
    }

    pub(super) fn update_wiki(&mut self, msg: WikiMsg) -> Task<Message> {
        match msg {
            WikiMsg::Open(fandom) => {
                self.wiki_enter(fandom);
                if self.wiki.page.is_some() && !self.wiki.loading && self.wiki.error.is_none() {
                    // Back where it was, scrolled as it was.
                    self.app_view = AppView::Wikipedia;
                    self.set_show_context(false);
                    return self.wiki_scroll_back();
                }
                let page = self.wiki.page.clone().unwrap_or(WikiPage::Home);
                return self.wiki_go(page, false);
            }
            WikiMsg::Go(page) => return self.wiki_go(page, true),
            WikiMsg::Back => {
                if let Some((site, page, offset)) = self.wiki.history.pop() {
                    self.wiki.restore = Some(offset);
                    return self.wiki_visit(site, page, false);
                }
                return self.update(Message::GoToLibrary);
            }
            WikiMsg::Random => {
                self.wiki.loading = true;
                let wiki = self.wiki.wiki().cloned();
                return cosmic::task::future(async move {
                    let t = match wiki {
                        Some(w) => fandom::random_title(&w).await,
                        None => wikipedia::random_title().await,
                    };
                    Message::Wiki(WikiMsg::RandomFound(t))
                });
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
                    Ok(home) => {
                        // The viewer steps through the page's pictures.
                        let mut gallery = Vec::new();
                        let mut extra: Vec<(String, u32)> = Vec::new();
                        if let Some(m) = &home.main {
                            gallery.extend(m.gallery().into_iter().map(|(i, c)| (i.url, c)));
                            // The featured picture, large, across the page.
                            if let Some(tfp) = m.boxes.iter().find(|b| b.id == "tfp") {
                                if let Some(Block::Figure { image, .. }) = tfp.blocks.iter().find(|b| matches!(b, Block::Figure { .. })) {
                                    extra.push((wikipedia::full_size(&image.url), LARGE));
                                }
                            }
                        } else if let Some(f) = &home.feed {
                            if let Some(p) = &f.picture {
                                gallery.push((p.image.clone(), vec![Run { text: p.description.clone(), ..Default::default() }]));
                            }
                            if let Some(a) = &f.article {
                                if let Some(t) = &a.thumbnail {
                                    gallery.push((t.clone(), vec![Run { text: a.title.clone(), ..Default::default() }]));
                                }
                            }
                            // The day's picture, large (the full file, scaled here).
                            extra.extend(f.picture.iter().map(|p| (p.image.clone(), LARGE)));
                        }
                        if let Some(f) = &home.feed {
                            extra.extend(f.most_read.iter().filter_map(|a| a.thumbnail.clone()).map(|u| (u, THUMB)));
                        }
                        let f = home;
                        self.wiki.featured = Some(f);
                        let pictures = self.page_pictures(gallery, extra);
                        return Task::batch([pictures, self.wiki_restore()]);
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
                        let links = a.link_pictures().into_iter().map(|u| (u, THUMB)).collect();
                        self.wiki.article = Some(Arc::new(a));
                        let pictures = self.page_pictures(gallery, links);
                        return Task::batch([pictures, self.wiki_restore()]);
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
                        let pictures = self.page_pictures(Vec::new(), extra);
                        return Task::batch([pictures, self.wiki_restore()]);
                    }
                    Err(e) => self.wiki.error = Some(e),
                }
            }
            WikiMsg::Pictures(batch) => {
                for (key, result) in batch {
                    // Only pictures a page still wants (the one put away
                    // too: Wikipedia's or Fandom's, whichever isn't shown).
                    if let Some(slot) = self.wiki_parked.pics.get_mut(&key).filter(|s| matches!(s, Pic::Loading)) {
                        *slot = match &result {
                            Ok(h) => Pic::Ready(h.clone()),
                            Err(_) => Pic::Failed,
                        };
                    }
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
                // A page on a Fandom wiki, from a Fandom wiki: opened here.
                if let (Some(_), Some((base, title))) = (self.wiki.wiki(), fandom::page_link(&target)) {
                    if let Some(w) = self.settings.fandom_wikis.iter().find(|w| w.id() == base).cloned() {
                        return self.wiki_visit(Site::Fandom(Some(w)), WikiPage::Article(title), true);
                    }
                    self.wiki.loading = true;
                    return cosmic::task::future(async move {
                        Message::Wiki(WikiMsg::WikiFound(fandom::find(&base).await.map_err(|_| target), Some(title)))
                    });
                }
                crate::platform::open_url(&target);
            }
            WikiMsg::ToggleToc => {
                // The article gets wider or narrower and its text wraps
                // anew: find the place being read first, to stay on it.
                let Some(a) = self.wiki.article.as_ref().filter(|_| self.wiki.on_article()) else {
                    self.wiki.toc_shown = !self.wiki.toc_shown;
                    return Task::none();
                };
                return cosmic::iced::runtime::task::widget(MeasureSections::new(a.sections.len()))
                    .map(|l| cosmic::Action::App(Message::Wiki(WikiMsg::TocToggled(l.and_then(|l| l.anchor())))));
            }
            WikiMsg::TocToggled(anchor) => {
                self.wiki.toc_shown = !self.wiki.toc_shown;
                let (Some(anchor), Some(a)) = (anchor, self.wiki.article.as_ref()) else { return Task::none() };
                return cosmic::iced::runtime::task::widget(MeasureSections::new(a.sections.len()))
                    .map(move |l| cosmic::Action::App(Message::Wiki(WikiMsg::ScrollTo(l.and_then(|l| l.resolve(anchor))))));
            }
            WikiMsg::SetLanguage(i) => {
                let Some((code, _)) = wikipedia::LANGUAGES.get(i) else { return Task::none() };
                if *code == wikipedia::language() {
                    return Task::none();
                }
                self.settings.wikipedia_language = code.to_string();
                self.settings.save();
                wikipedia::set_language(code);
                // Titles in the history belong to the old language.
                self.wiki.history.clear();
                self.wiki.featured = None;
                self.wiki.article = None;
                self.wiki.results.clear();
                return self.wiki_go(WikiPage::Home, false);
            }
            WikiMsg::ArrowDown(dir) => {
                let now = std::time::Instant::now();
                let start = KeyScroll::top_speed(self.wiki_height()) * KeyScroll::START;
                // Still gliding the same way: carry on from that speed.
                let speed = match self.wiki.key_scroll {
                    Some(k) if k.dir == dir => k.speed.max(start),
                    _ => start,
                };
                self.wiki.key_scroll = Some(KeyScroll { dir, speed, last: now, held: true });
            }
            WikiMsg::ArrowUp(dir) => {
                if let Some(k) = &mut self.wiki.key_scroll {
                    if k.dir == dir {
                        k.held = false;
                    }
                }
            }
            WikiMsg::ScrollFrame(now) => {
                let height = self.wiki_height();
                let mut dy = 0.0;
                if let Some(k) = &mut self.wiki.key_scroll {
                    match k.step(now, height) {
                        Some(d) => dy += d,
                        None => self.wiki.key_scroll = None,
                    }
                }
                // A selection held near or past an edge.
                if let Some(speed) = self.wiki_drag_speed() {
                    let dt = self.wiki.drag_frame.map_or(0.0, |last| now.saturating_duration_since(last).as_secs_f32().min(0.05));
                    self.wiki.drag_frame = Some(now);
                    dy += speed * dt;
                } else {
                    self.wiki.drag_frame = None;
                }
                if dy != 0.0 {
                    return scroll_mod::scroll_by(scroll_id(), AbsoluteOffset { x: 0.0, y: dy });
                }
            }
            WikiMsg::WindowHeight(h) => self.wiki.window_height = h,
            WikiMsg::Scrolled(y) => {
                // Only the page's own reports: not while it's loading (a
                // spinner is short) or hidden behind the viewer.
                if self.app_view == AppView::Wikipedia && !self.wiki.loading && self.wiki.viewer.is_none() {
                    self.wiki.offset = y;
                }
            }
            WikiMsg::Select(k, cosmic::iced::widget::text::Select::Press { offset, clicks }) => {
                let len = self.wiki.texts.borrow().get(k).map_or(0, String::len);
                let (anchor, focus) = match clicks {
                    1 => (offset, offset),
                    2 => self.wiki.texts.borrow().get(k).map_or((offset, offset), |t| word_at(t, offset)),
                    _ => (0, len),
                };
                self.wiki.sel = Some(TextSel { anchor: (k, anchor), focus: (k, focus), dragging: true });
                self.wiki.drag_y = None;
                self.wiki.drag_frame = None;
                return cosmic::iced::runtime::task::widget(FindScrollBounds(None))
                    .map(|b| cosmic::Action::App(Message::Wiki(WikiMsg::ScrollBounds(b))));
            }
            WikiMsg::ScrollBounds(b) => self.wiki.scroll_bounds = b,
            WikiMsg::DragAt(y) => {
                if self.wiki.sel.is_some_and(|s| s.dragging) {
                    self.wiki.drag_y = Some(y);
                }
            }
            WikiMsg::Select(k, cosmic::iced::widget::text::Select::Drag { offset }) => {
                if let Some(sel) = &mut self.wiki.sel {
                    if sel.dragging {
                        sel.focus = (k, offset);
                    }
                }
            }
            WikiMsg::SelectEnd => {
                let Some(sel) = &mut self.wiki.sel else { return Task::none() };
                if !sel.dragging {
                    return Task::none();
                }
                sel.dragging = false;
                self.wiki.drag_y = None;
                self.wiki.drag_frame = None;
                // Linux's middle-click paste takes what's selected.
                #[cfg(target_os = "linux")]
                if let Some(text) = self.selected_text() {
                    return cosmic::iced::clipboard::write_primary(text);
                }
            }
            WikiMsg::SelectClear => {
                if self.wiki.sel.is_some_and(|s| !s.dragging) {
                    self.wiki.sel = None;
                }
            }
            WikiMsg::SelectAll => {
                let texts = self.wiki.texts.borrow();
                if let Some(last) = texts.len().checked_sub(1) {
                    let end = (last, texts[last].len());
                    drop(texts);
                    self.wiki.sel = Some(TextSel { anchor: (0, 0), focus: end, dragging: false });
                }
            }
            WikiMsg::Copy => {
                if let Some(text) = self.selected_text() {
                    return cosmic::iced::clipboard::write(text);
                }
            }
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
            WikiMsg::ScrollTo(Some(y)) => {
                // A jump isn't a scroll the page reports: remember it here.
                self.wiki.offset = y;
                return scroll_mod::scroll_to(scroll_id(), AbsoluteOffset { x: None, y: Some(y) });
            }
            WikiMsg::ScrollTo(None) => {}
            WikiMsg::OpenOnWikipedia => {
                let title = match &self.wiki.page {
                    Some(WikiPage::Article(t)) => Some(t.clone()),
                    _ => self.wiki.article.as_ref().map(|a| a.title.clone()),
                };
                match (self.wiki.wiki(), title) {
                    (Some(w), Some(t)) => crate::platform::open_url(&w.page_url(&t)),
                    (Some(w), None) => crate::platform::open_url(&w.page_url(&w.main_page)),
                    (None, Some(t)) => crate::platform::open_url(&wikipedia::page_url(&t)),
                    (None, None) => {}
                }
            }
            WikiMsg::Tab(id, i) => {
                self.wiki.tabs.insert(id, i);
                self.wiki.sel = None;
            }
            WikiMsg::AddInput(s) => {
                self.wiki.add_input = s;
                self.wiki.add_error = None;
            }
            WikiMsg::AddWiki => {
                let input = self.wiki.add_input.trim().to_string();
                if input.is_empty() || self.wiki.adding {
                    return Task::none();
                }
                self.wiki.adding = true;
                self.wiki.add_error = None;
                return cosmic::task::future(async move { Message::Wiki(WikiMsg::WikiFound(fandom::find(&input).await, None)) });
            }
            WikiMsg::WikiFound(result, page) => match (result, page) {
                // Added from the box: followed, and shown in the list.
                (Ok(w), None) => {
                    self.wiki.adding = false;
                    self.wiki.add_input.clear();
                    let logo = w.logo.clone();
                    match self.settings.fandom_wikis.iter_mut().find(|f| **f == w) {
                        Some(f) => *f = w,
                        None => self.settings.fandom_wikis.push(w),
                    }
                    self.settings.save();
                    return self.want(logo, THUMB);
                }
                (Err(e), None) => {
                    self.wiki.adding = false;
                    self.wiki.add_error = Some(e);
                }
                // A link to another wiki: opened here, or on the site if
                // it can't be.
                (Ok(w), Some(title)) => {
                    self.wiki.loading = false;
                    return self.wiki_visit(Site::Fandom(Some(w)), WikiPage::Article(title), true);
                }
                (Err(url), Some(_)) => {
                    self.wiki.loading = false;
                    crate::platform::open_url(&url);
                }
            },
            WikiMsg::OpenWiki(id) => {
                if let Some(w) = self.settings.fandom_wikis.iter().find(|w| w.id() == id).cloned() {
                    return self.wiki_visit(Site::Fandom(Some(w)), WikiPage::Home, true);
                }
            }
            WikiMsg::Follow => {
                if let Some(w) = self.wiki.wiki().cloned() {
                    if !self.settings.fandom_wikis.contains(&w) {
                        self.settings.fandom_wikis.push(w);
                        self.settings.save();
                    }
                }
            }
            WikiMsg::Unfollow(id) => {
                self.settings.fandom_wikis.retain(|w| w.id() != id);
                self.settings.save();
            }
            WikiMsg::Following => return self.wiki_visit(Site::Fandom(None), WikiPage::Home, true),
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
            WikiMsg::ViewClose => {
                // The page was taken down for the viewer: back where it was.
                self.wiki.viewer = None;
                return self.wiki_scroll_back();
            }
        }
        Task::none()
    }

    /// A picture at a given size: clickable to view it full window. While
    /// it loads, a frame of its size holds its place; `None` if it failed.
    /// A text that can be selected: it takes the next key in reading order.
    fn sel_rich<'a>(&self, spans: Vec<Span<'a, String, Font>>, pal: &Palette) -> Rich<'a> {
        let k = {
            let mut texts = self.wiki.texts.borrow_mut();
            texts.push(spans.iter().map(|s| s.text.as_ref()).collect());
            texts.len() - 1
        };
        let color = Color { a: 0.3, ..pal.link };
        rich_text(spans)
            .on_select(move |s| Message::Wiki(WikiMsg::Select(k, s)))
            .dragging(self.wiki.sel.is_some_and(|s| s.dragging))
            .selection(self.wiki.sel.and_then(|s| s.range(k)), color)
            .on_link_click(|l| Message::Wiki(WikiMsg::Link(l)))
    }

    fn line_text<'a>(&self, line: &'a Line, pal: &Palette, size: f32) -> Element<'a, Message> {
        let spans: Vec<Span<'a, String, Font>> = line.iter().map(|r| run_span(r, r.text.as_str().into(), pal, size)).collect();
        self.sel_rich(spans, pal).size(size).line_height(LineHeight::Relative(LINE)).width(Length::Fill).into()
    }

    /// [`App::line_text`] for a line made on the spot.
    fn owned_text<'a>(&self, line: Line, pal: &Palette, size: f32) -> Element<'a, Message> {
        let spans: Vec<Span<'a, String, Font>> = line.iter().map(|r| run_span(r, r.text.clone().into(), pal, size)).collect();
        self.sel_rich(spans, pal).size(size).line_height(LineHeight::Relative(LINE)).width(Length::Fill).into()
    }

    /// A heading or title, selectable like the text.
    fn heading<'a>(&self, text: &str, size: f32, font: Font, pal: &Palette) -> Element<'a, Message> {
        self.sel_rich(vec![span(text.to_string()).font(font)], pal).size(size).into()
    }

    /// The selected text, the texts' parts on their own lines.
    fn selected_text(&self) -> Option<String> {
        let sel = self.wiki.sel?;
        let (s, e) = sel.ordered();
        if s == e {
            return None;
        }
        let texts = self.wiki.texts.borrow();
        let parts: Vec<&str> = (s.0..=e.0.min(texts.len().saturating_sub(1)))
            .filter_map(|k| {
                let (a, b) = sel.range(k)?;
                Some(slice(texts.get(k)?, a, b))
            })
            .collect();
        let text = parts.join("\n");
        (!text.trim().is_empty()).then_some(text)
    }

    fn wiki_pic<'a>(&self, url: &str, max: u32, width: Length, height: Length, pal: &Palette) -> Option<Element<'a, Message>> {
        let view = Message::Wiki(WikiMsg::View(url.to_string()));
        self.wiki_pic_on(url, max, width, height, pal, view, cosmic::iced::mouse::Interaction::ZoomIn)
    }

    /// [`App::wiki_pic`] doing something else when clicked (a picture
    /// that's a link).
    #[allow(clippy::too_many_arguments)]
    fn wiki_pic_on<'a>(
        &self,
        url: &str,
        max: u32,
        width: Length,
        height: Length,
        pal: &Palette,
        on_press: Message,
        interaction: cosmic::iced::mouse::Interaction,
    ) -> Option<Element<'a, Message>> {
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
        Some(widget::mouse_area(el).on_press(on_press).interaction(interaction).into())
    }

    /// The bar above every Wikipedia page: Back, contents, Main page,
    /// Random, search. On a Fandom wiki the same for that wiki, plus the
    /// way to the wikis followed and Follow; on the list of wikis, just Back.
    fn wiki_bar(&self) -> Element<'_, Message> {
        let article = self.wiki.on_article();
        let back = tip(widget::button::icon(icon::from_name("go-previous-symbolic")).on_press(Message::Wiki(WikiMsg::Back)), fl!("opds-back"));
        let wiki = match &self.wiki.site {
            Site::Fandom(None) => return widget::row::with_capacity(1).push(back).into(),
            Site::Fandom(Some(w)) => Some(w),
            Site::Wikipedia => None,
        };
        let following = wiki.is_some_and(|w| self.settings.fandom_wikis.contains(w));
        widget::row::with_capacity(8)
            .spacing(8)
            .align_y(Alignment::Center)
            .push(back)
            .push_maybe(wiki.map(|_| {
                tip(
                    widget::button::icon(icon::from_name("view-grid-symbolic")).on_press(Message::Wiki(WikiMsg::Following)),
                    fl!("fandom-following"),
                )
            }))
            .push_maybe((article && !self.wiki.toc_shown).then(|| {
                tip(
                    widget::button::icon(icon::from_name("view-list-symbolic")).on_press(Message::Wiki(WikiMsg::ToggleToc)),
                    fl!("wiki-contents"),
                )
            }))
            .push(widget::button::text(fl!("wiki-main-page")).on_press(Message::Wiki(WikiMsg::Go(WikiPage::Home))))
            .push(widget::button::text(fl!("wiki-random")).on_press(Message::Wiki(WikiMsg::Random)))
            .push(
                widget::search_input(
                    match wiki {
                        Some(w) => fl!("fandom-search", wiki = w.name.as_str()),
                        None => fl!("wiki-search"),
                    },
                    &self.wiki.search_input,
                )
                .on_input(|s| Message::Wiki(WikiMsg::SearchInput(s)))
                .on_submit(|_| Message::Wiki(WikiMsg::Search))
                .width(Length::Fill),
            )
            .push(widget::button::standard(fl!("wiki-search-button")).on_press(Message::Wiki(WikiMsg::Search)))
            // A wiki opened from a link can be followed from here.
            .push_maybe((wiki.is_some() && !following).then(|| widget::button::suggested(fl!("fandom-follow")).on_press(Message::Wiki(WikiMsg::Follow))))
            // Which language's Wikipedia, on the main page like the site's.
            .push_maybe((wiki.is_none() && matches!(self.wiki.page, Some(WikiPage::Home) | None)).then(|| {
                let current = wikipedia::LANGUAGES.iter().position(|(c, _)| *c == wikipedia::language());
                tip(
                    widget::dropdown(language_names(), current, |i| Message::Wiki(WikiMsg::SetLanguage(i))),
                    fl!("wiki-language"),
                )
            }))
            .into()
    }

    pub(super) fn view_wiki(&self) -> Element<'_, Message> {
        // Texts take their keys afresh as the page is laid out.
        self.wiki.texts.borrow_mut().clear();
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
                (_, _, _) if self.wiki.site == Site::Fandom(None) => self.view_fandom_following(&pal),
                (Some(WikiPage::Article(_)), Some(a), _) => self.view_wiki_article(a, &pal),
                (Some(WikiPage::Home), Some(a), _) if self.wiki.wiki().is_some() => self.view_wiki_article(a, &pal),
                (Some(WikiPage::Search(q)), _, _) => self.view_wiki_results(q, &pal),
                (Some(WikiPage::Home) | None, _, Some(h)) => match (&h.main, &h.feed) {
                    (Some(m), feed) => self.view_wiki_main_page(m, feed.as_ref(), &pal),
                    (None, Some(f)) => self.view_wiki_home(f, &pal),
                    (None, None) => widget::Space::new().into(),
                },
                _ => widget::Space::new().into(),
            }
        };
        let page = widget::column::with_capacity(2).spacing(16).padding([16, 24]).push(self.wiki_bar()).push(body);
        let page = widget::scrollable(widget::container(page).max_width(1500).width(Length::Fill).center_x(Length::Fill))
            .id(scroll_id())
            .on_scroll(|v| Message::Wiki(WikiMsg::Scrolled(v.absolute_offset().y)))
            .width(Length::Fill)
            .height(Length::Fill);
        // Right-click: Copy (when something's selected) and Select All.
        let page = {
            use cosmic::widget::menu::{self, Item};
            let copy = if self.wiki.sel.is_some_and(|s| { let (a, b) = s.ordered(); a != b }) {
                Item::Button(fl!("wiki-copy"), None, TextAction::Copy)
            } else {
                Item::ButtonDisabled(fl!("wiki-copy"), None, TextAction::Copy)
            };
            let items = vec![copy, Item::Button(fl!("wiki-select-all"), None, TextAction::SelectAll)];
            widget::context_menu(page, Some(menu::items(&HashMap::new(), items)))
        };
        // The contents stay beside the article as it scrolls, like the site's.
        // Both scroll smoothly, gliding like a browser's (see `gestures`).
        // The page always second in the row, so showing or hiding the
        // contents keeps its scrolling where it is.
        let toc: Element<'_, Message> = match (&self.wiki.page, &self.wiki.article) {
            (_, Some(a)) if self.wiki.on_article() && self.wiki.toc_shown && !self.wiki.loading && self.wiki.error.is_none() => {
                widget::scrollable(widget::container(self.wiki_toc(a, &pal)).padding([24, 8, 24, 20])).width(Length::Fixed(TOC_WIDTH + 28.0)).height(Length::Fill).into()
            }
            _ => widget::Space::new().width(Length::Fixed(0.0)).into(),
        };
        let page = widget::row::with_capacity(2).push(toc).push(page);
        // Where the page is scrolled reaches the app every 400 px, and
        // exactly once it settles (not on every frame: each report lays the
        // page out again).
        super::gestures::gestures(page)
            .throttle(|m| match m {
                Message::Wiki(WikiMsg::Scrolled(y)) => Some((*y / 400.0).floor() as i32),
                _ => None,
            })
            .into()
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
            col = col.push(widget::container(self.line_text(caption, pal, TEXT)).padding([8, 24]).max_height(160.0));
        }
        widget::container(col).width(Length::Fill).height(Length::Fill).class(cosmic::theme::Container::Background).into()
    }

    // ── The main page ─────────────────────────────────────────────────────

    /// A main-page box: a coloured heading band, then its content.
    fn main_box<'a>(&self, title: String, band: [u8; 3], content: Element<'a, Message>, pal: &Palette) -> Element<'a, Message> {
        let bg = pal.page_color(band);
        let head = widget::container(self.heading(&title, 17.0, Font { weight: Weight::Bold, ..serif() }, pal))
            .padding([4, 8])
            .width(Length::Fill)
            .style(move |_: &cosmic::Theme| widget::container::Style { background: Some(Background::Color(bg)), ..Default::default() });
        boxed(widget::column::with_capacity(2).push(head).push(widget::container(content).padding([8, 10])), pal.panel, pal.border)
            .width(Length::Fill)
            .into()
    }

    /// "Most read today", from the daily feed (not on the site's page).
    fn most_read_box<'a>(&'a self, f: &'a Featured, pal: &Palette) -> Option<Element<'a, Message>> {
        if f.most_read.is_empty() {
            return None;
        }
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
                    .push(self.owned_text(vec![link_run(&a.title, true)], pal, TEXT))
                    .push(muted(a.description.clone(), TEXT - 2.0, pal)),
            );
            list = list.push(row);
        }
        Some(self.main_box(fl!("wiki-most-read"), [0xce, 0xf2, 0xe0], list.into(), pal))
    }

    /// The day's picture across the page, kept in proportion (it opens in
    /// the viewer).
    /// `pic` is the file shown (loaded at `LARGE`), `view` the picture's
    /// key in the viewer.
    fn wide_picture<'a>(&self, pic: &str, view: &str) -> Option<Element<'a, Message>> {
        let el: Element<'a, Message> = match self.wiki.pics.get(&pic_key(pic, LARGE)) {
            Some(Pic::Ready(h)) => widget::mouse_area(widget::image(h.clone()).width(Length::Fill).content_fit(ContentFit::Contain))
                .on_press(Message::Wiki(WikiMsg::View(view.to_string())))
                .interaction(cosmic::iced::mouse::Interaction::ZoomIn)
                .into(),
            Some(Pic::Failed) => return None,
            _ => widget::container(widget::indeterminate_circular())
                .width(Length::Fill)
                .height(Length::Fixed(300.0))
                .center_x(Length::Fill)
                .center_y(Length::Fixed(300.0))
                .into(),
        };
        Some(widget::container(el).max_width(1100).width(Length::Fill).center_x(Length::Fill).into())
    }

    /// English Wikipedia's main page as the site shows it today: the
    /// featured article and Did you know on the left, In the news and On
    /// this day on the right, then the featured list and picture across.
    fn view_wiki_main_page<'a>(&'a self, m: &'a wikipedia::MainPage, feed: Option<&'a Featured>, pal: &Palette) -> Element<'a, Message> {
        let content = |b: &'a wikipedia::MainBox| -> Element<'a, Message> {
            let mut flow = FloatFlow::new().spacing(8.0).gap(16.0);
            let mut wide_done = false;
            for block in &b.blocks {
                flow = match block {
                    // The featured picture: large, across the box.
                    Block::Figure { image, .. } if b.id == "tfp" && !wide_done => {
                        wide_done = true;
                        match self.wide_picture(&wikipedia::full_size(&image.url), &image.url) {
                            Some(p) => flow.push(p),
                            None => flow,
                        }
                    }
                    Block::Figure { image, caption } => match self.wiki_figure(image, caption, pal) {
                        Some(fig) => flow.float(fig),
                        None => flow,
                    },
                    other => flow.push(self.wiki_block(other, pal)),
                };
            }
            flow.into()
        };
        let boxes = |column: wikipedia::Column| -> Vec<Element<'a, Message>> {
            m.boxes.iter().filter(|b| b.column == column).map(|b| self.main_box(b.title.clone(), box_band(&b.id), content(b), pal)).collect()
        };

        let mut col = widget::column::with_capacity(6).spacing(16);
        let welcome = m.welcome.trim_end_matches(',').to_string();
        col = col.push(
            widget::column::with_capacity(3)
                .spacing(4)
                .push(self.heading(if welcome.is_empty() { "Wikipedia" } else { &welcome }, 30.0, serif(), pal))
                .push_maybe((!m.tagline.is_empty()).then(|| muted(m.tagline.clone(), TEXT, pal)))
                .push_maybe((!m.count.is_empty()).then(|| muted(m.count.clone(), TEXT - 2.0, pal))),
        );
        let left = widget::column::with_children(boxes(wikipedia::Column::Left)).spacing(16).width(Length::FillPortion(11));
        let right = widget::column::with_children(boxes(wikipedia::Column::Right)).spacing(16).width(Length::FillPortion(9));
        col = col.push(widget::row::with_capacity(2).spacing(16).push(left).push(right));
        for b in boxes(wikipedia::Column::Full) {
            col = col.push(b);
        }
        if let Some(mr) = feed.and_then(|f| self.most_read_box(f, pal)) {
            col = col.push(mr);
        }
        col = col.push(widget::text::caption(fl!("wiki-credit-home")));
        col.into()
    }

    fn view_wiki_home<'a>(&'a self, f: &'a Featured, pal: &Palette) -> Element<'a, Message> {
        let main_box = |title: String, band: [u8; 3], content: Element<'a, Message>| self.main_box(title, band, content, pal);

        let mut left = widget::column::with_capacity(3).spacing(16).width(Length::FillPortion(3));
        if let Some(a) = &f.article {
            let mut text = widget::column::with_capacity(3).spacing(6).width(Length::Fill);
            text = text.push(self.owned_text(vec![link_run(&a.title, true)], pal, TEXT));
            text = text.push(widget::text(a.extract.clone()).size(TEXT).line_height(LineHeight::Relative(LINE)));
            text = text.push(widget::button::link(fl!("wiki-full-article")).on_press(Message::Wiki(WikiMsg::Go(WikiPage::Article(a.title.clone())))));
            let mut row = widget::row::with_capacity(2).spacing(12);
            if let Some(pic) = a.thumbnail.as_deref().and_then(|t| self.wiki_pic(t, THUMB, Length::Fixed(140.0), Length::Fixed(180.0), pal)) {
                row = row.push(pic);
            }
            left = left.push(main_box(fl!("wiki-featured-article"), [0xce, 0xf2, 0xe0], row.push(text).into()));
        }
        if let Some(mr) = self.most_read_box(f, pal) {
            left = left.push(mr);
        }

        let mut right = widget::column::with_capacity(2).spacing(16).width(Length::FillPortion(2));
        if !f.news.is_empty() {
            let mut list = widget::column::with_capacity(f.news.len()).spacing(6);
            for story in &f.news {
                list = list.push(widget::row::with_capacity(2).spacing(6).push(widget::text("•").size(TEXT)).push(self.line_text(story, pal, TEXT)));
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
                list = list.push(self.owned_text(line, pal, TEXT));
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
            let content = widget::column::with_capacity(2)
                .spacing(8)
                .push_maybe(self.wide_picture(&p.image, &p.image))
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
                    .push(self.owned_text(vec![link_run(&s.title, false)], pal, TEXT + 2.0))
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
        let wiki = self.wiki.wiki();
        // A wiki's main page is headed with the wiki's name.
        let title = match (wiki, &self.wiki.page) {
            (Some(w), Some(WikiPage::Home)) => w.name.as_str(),
            _ => a.title.as_str(),
        };
        let from = match wiki {
            Some(w) => fl!("fandom-from", wiki = w.name.as_str()),
            None => fl!("wiki-from"),
        };
        // A wiki's colour under its title, as its pages have it.
        let accent = wiki.and_then(|w| w.accent).map(|c| Color::from_rgb8(c[0], c[1], c[2])).unwrap_or(pal.border);
        flow = flow.push(
            widget::container(
                widget::column::with_capacity(3)
                    .spacing(8)
                    .push(self.heading(title, 32.0, serif(), pal))
                    .push(rule(accent))
                    .push(muted(from, TEXT - 1.0, pal)),
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
                                .push(self.heading(&s.title, 25.0, serif(), pal))
                                .push(rule(pal.border)),
                        )
                        .padding([12, 0, 0, 0])
                        .id(section_id(i)),
                    );
                }
                level => {
                    let size = if level == 3 { 18.0 } else { 16.0 };
                    flow = flow.push(widget::container(self.heading(&s.title, size, bold(), pal)).padding([6, 0, 0, 0]).id(section_id(i)));
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
                        None if !caption.is_empty() => flow.push(self.line_text(caption, pal, TEXT - 2.0)),
                        None => flow,
                    },
                    Block::Table(t) if t.infobox => flow.float(self.wiki_table(t, pal, true)),
                    Block::Table(t) if t.float_right => flow.float(widget::container(self.wiki_table(t, pal, false)).width(Length::Fixed(360.0))),
                    other => flow.push(self.wiki_block(other, pal)),
                };
            }
        }
        // Credit and licence.
        let (credit, license, view_on) = match wiki {
            Some(w) => {
                let (name, url) = w.license();
                (
                    fl!("fandom-credit", title = a.title.as_str(), wiki = w.name.as_str(), license = name.as_str()),
                    (name, url),
                    if w.on_fandom() { fl!("fandom-view-on") } else { fl!("fandom-view-on-site") },
                )
            }
            None => (fl!("wiki-credit", title = a.title.as_str()), (fl!("wiki-license"), wikipedia::LICENSE_URL.to_string()), fl!("wiki-view-on")),
        };
        flow = flow.push(
            widget::column::with_capacity(2)
                .spacing(10)
                .padding([20, 0, 0, 0])
                .push(rule(pal.border))
                .push(
                    widget::row::with_capacity(3)
                        .spacing(12)
                        .align_y(Alignment::Center)
                        .push(widget::container(muted(credit, TEXT - 2.0, pal)).width(Length::Fill))
                        .push(widget::button::link(license.0).on_press(Message::OpenUrl(license.1)))
                        .push(widget::button::standard(view_on).on_press(Message::Wiki(WikiMsg::OpenOnWikipedia))),
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
                // Past the arrows' column, so subsections sit inside their section.
                let indent = 24.0 + 14.0 + 12.0 * (s.level as f32 - 3.0);
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
                    .push_maybe((!caption.is_empty()).then(|| self.line_text(caption, pal, TEXT - 2.0))),
                pal.panel,
                pal.border,
            )
            .padding(4)
            .into(),
        )
    }

    fn wiki_block<'a>(&'a self, b: &'a Block, pal: &Palette) -> Element<'a, Message> {
        match b {
            Block::Para(l) => self.line_text(l, pal, TEXT),
            Block::Hatnote(l) => widget::container(self.line_text(l, pal, TEXT - 1.0)).padding([0, 0, 0, 24]).into(),
            Block::Quote(l) => widget::container(self.line_text(l, pal, TEXT)).padding([0, 32]).into(),
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
                            .push(self.line_text(line, pal, TEXT)),
                    );
                }
                col.into()
            }
            Block::Figure { image, caption } => match self.wiki_figure(image, caption, pal) {
                Some(fig) => widget::container(fig).width(Length::Fill).align_x(Alignment::End).into(),
                None => self.line_text(caption, pal, TEXT - 2.0),
            },
            Block::Table(t) => self.wiki_table(t, pal, t.infobox),
            Block::Gallery(items) => self.wiki_gallery(items, pal),
            Block::Tabs { id, tabs } => self.wiki_tabs(*id, tabs, pal, None),
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
            col = col.push(widget::container(self.line_text(&t.caption, pal, TEXT)).padding([4, 8]));
        }
        let size = if infobox { TEXT - 2.0 } else { TEXT - 1.0 };
        // Columns as wide as what's in them, as a browser sizes them.
        let columns = column_weights(t);
        for r in &t.rows {
            // Every cell as tall as the row's tallest, so the grid lines up.
            let mut row = super::table_row::TableRow::new().spacing(if infobox { 8.0 } else { 0.0 });
            // Label and value: the label narrower, as in an infobox.
            let label_row = infobox && r.cells.len() == 2 && r.cells[0].header && !r.cells[1].header;
            let mut at = 0;
            for (j, c) in r.cells.iter().enumerate() {
                let span = c.colspan.max(1);
                let weight = if label_row {
                    if j == 0 { 2.0 } else { 3.0 }
                } else if infobox {
                    span as f32
                } else {
                    columns.iter().skip(at).take(span).sum::<f32>().max(1.0)
                };
                at += span;
                row = row.push(weight, self.wiki_cell(c, pal, size, infobox).width(Length::Fill));
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
        let col = self.cell_blocks(&c.content, pal, size, infobox, c.center);
        // A heading cell gets the site's grey band unless it has a colour.
        let bg = c.background.map(|b| pal.page_color(b)).or((c.header && !infobox).then_some(pal.panel));
        let border = if infobox { Color::TRANSPARENT } else { pal.border };
        widget::container(col).padding(if infobox { [3, 4] } else { [4, 8] }).style(move |_: &cosmic::Theme| widget::container::Style {
            background: bg.map(Background::Color),
            border: Border { color: border, width: if infobox { 0.0 } else { 1.0 }, radius: 0.0.into() },
            ..Default::default()
        })
    }

    /// What's in a table cell (or in a tab inside one), smaller than the
    /// page's text.
    fn cell_blocks<'a>(&'a self, content: &'a [Block], pal: &Palette, size: f32, infobox: bool, center: bool) -> widget::Column<'a, Message, cosmic::Theme> {
        let mut col = widget::column::with_capacity(content.len()).spacing(4).width(Length::Fill);
        if center {
            col = col.align_x(Alignment::Center);
        }
        let mut blocks = content.iter().peekable();
        while let Some(b) = blocks.next() {
            col = col.push(match b {
                // A chart's bar with its number beside it.
                Block::Bar(_) => {
                    let mut row = widget::row::with_capacity(2).spacing(6).align_y(Alignment::Center).push(self.wiki_block(b, pal));
                    if let Some(Block::Para(l)) = blocks.next_if(|n| matches!(n, Block::Para(_))) {
                        row = row.push(self.line_text(l, pal, size));
                    }
                    row.into()
                }
                Block::Para(l) => {
                    let spans: Vec<Span<'a, String, Font>> = l.iter().map(|r| run_span(r, r.text.as_str().into(), pal, size)).collect();
                    let t = self.sel_rich(spans, pal).size(size).line_height(LineHeight::Relative(1.45));
                    if center {
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
                        .push_maybe((!caption.is_empty()).then(|| self.line_text(caption, pal, size)))
                        .into()
                }
                Block::Tabs { id, tabs } => self.wiki_tabs(*id, tabs, pal, Some((size, infobox))),
                other => self.wiki_block(other, pal),
            });
        }
        col
    }

    /// Pictures side by side, wrapping: a gallery's (each opens in the
    /// viewer), or pictures that are links (a wiki's main page).
    fn wiki_gallery<'a>(&'a self, items: &'a [GalleryItem], pal: &Palette) -> Element<'a, Message> {
        let cards: Vec<Element<'a, Message>> = items
            .iter()
            .filter_map(|it| {
                // As the site sizes it, up to 200 px either way.
                let mut w = it.image.width.min(200.0);
                let mut h = it.image.height * w / it.image.width.max(1.0);
                if h > 200.0 {
                    w *= 200.0 / h;
                    h = 200.0;
                }
                let (press, pointer) = match &it.link {
                    Some(l) => (Message::Wiki(WikiMsg::Link(l.clone())), cosmic::iced::mouse::Interaction::Pointer),
                    None => (Message::Wiki(WikiMsg::View(it.image.url.clone())), cosmic::iced::mouse::Interaction::ZoomIn),
                };
                let pic = self.wiki_pic_on(&it.image.url, THUMB, Length::Fixed(w), Length::Fixed(h), pal, press, pointer)?;
                let width = if it.caption.is_empty() { w } else { w.max(140.0) };
                Some(
                    widget::column::with_capacity(2)
                        .spacing(4)
                        .width(Length::Fixed(width))
                        .align_x(Alignment::Center)
                        .push(pic)
                        .push_maybe((!it.caption.is_empty()).then(|| self.line_text(&it.caption, pal, TEXT - 3.0)))
                        .into(),
                )
            })
            .collect();
        widget::flex_row(cards).column_spacing(12).row_spacing(12).into()
    }

    /// Tabs: a button for each, then the one picked. `cell` is the text
    /// size and whether it's in an infobox, for tabs inside a table.
    fn wiki_tabs<'a>(&'a self, id: usize, tabs: &'a [(String, Vec<Block>)], pal: &Palette, cell: Option<(f32, bool)>) -> Element<'a, Message> {
        let Some(last) = tabs.len().checked_sub(1) else { return widget::Space::new().into() };
        let current = self.wiki.tabs.get(&id).copied().unwrap_or(0).min(last);
        let buttons: Vec<Element<'a, Message>> = tabs
            .iter()
            .enumerate()
            .map(|(i, (label, _))| {
                let b = widget::button::text(label.clone()).on_press(Message::Wiki(WikiMsg::Tab(id, i)));
                if i == current { b.class(cosmic::theme::Button::Suggested).into() } else { b.into() }
            })
            .collect();
        let pane: Element<'a, Message> = match cell {
            Some((size, infobox)) => self.cell_blocks(&tabs[current].1, pal, size, infobox, infobox).into(),
            None => widget::column::with_children(tabs[current].1.iter().map(|b| self.wiki_block(b, pal))).spacing(14).width(Length::Fill).into(),
        };
        widget::column::with_capacity(3)
            .spacing(8)
            .width(Length::Fill)
            .push(widget::flex_row(buttons).column_spacing(4).row_spacing(4))
            .push(rule(pal.border))
            .push(pane)
            .into()
    }

    // ── Fandom ────────────────────────────────────────────────────────────

    /// The Fandom page: the wikis followed, each opening its main page, and
    /// a box to add one by name or address.
    fn view_fandom_following<'a>(&'a self, pal: &Palette) -> Element<'a, Message> {
        let mut col = widget::column::with_capacity(6).spacing(16);
        col = col.push(
            widget::column::with_capacity(2)
                .spacing(4)
                .push(widget::text(fl!("fandom-following")).size(30.0).font(serif()))
                .push(muted(fl!("fandom-following-sub"), TEXT, pal)),
        );
        let add = widget::row::with_capacity(3)
            .spacing(8)
            .align_y(Alignment::Center)
            .push(
                widget::text_input(fl!("fandom-add-placeholder"), &self.wiki.add_input)
                    .on_input(|s| Message::Wiki(WikiMsg::AddInput(s)))
                    .on_submit(|_| Message::Wiki(WikiMsg::AddWiki))
                    .width(Length::Fill),
            )
            .push_maybe(self.wiki.adding.then(|| widget::indeterminate_circular().size(20.0).bar_height(2.0)))
            .push(widget::button::suggested(fl!("fandom-add")).on_press_maybe((!self.wiki.adding).then_some(Message::Wiki(WikiMsg::AddWiki))));
        col = col.push(widget::container(add).max_width(720));
        if let Some(e) = &self.wiki.add_error {
            col = col.push(widget::text::body(e.clone()).class(cosmic::theme::Text::Accent));
        }
        if self.settings.fandom_wikis.is_empty() {
            col = col.push(muted(fl!("fandom-none-yet"), TEXT, pal));
        }
        let cards: Vec<Element<'a, Message>> = self.settings.fandom_wikis.iter().map(|w| self.fandom_card(w, pal)).collect();
        col = col.push(widget::flex_row(cards).column_spacing(16).row_spacing(16));
        col = col.push(widget::text::caption(fl!("fandom-note")));
        col.into()
    }

    /// A wiki followed: its colour, logo (or name), address, and a way to
    /// stop following it.
    fn fandom_card<'a>(&'a self, w: &'a Wiki, pal: &Palette) -> Element<'a, Message> {
        let accent = w.accent.map(|c| Color::from_rgb8(c[0], c[1], c[2])).unwrap_or(pal.link);
        let band = widget::container(widget::Space::new().height(4.0).width(Length::Fill))
            .style(move |_: &cosmic::Theme| widget::container::Style { background: Some(Background::Color(accent)), ..Default::default() });
        let logo: Option<Element<'a, Message>> = w.logo.as_deref().and_then(|l| match self.wiki.pics.get(&pic_key(l, THUMB)) {
            Some(Pic::Ready(h)) => Some(widget::image(h.clone()).height(Length::Fixed(48.0)).content_fit(ContentFit::Contain).into()),
            _ => None,
        });
        let body = widget::column::with_capacity(3)
            .spacing(6)
            .padding([10, 12])
            .width(Length::Fill)
            .push(widget::container(logo.unwrap_or_else(|| widget::Space::new().height(Length::Fixed(48.0)).into())).height(Length::Fixed(48.0)).center_y(Length::Fixed(48.0)))
            .push(widget::text(w.name.clone()).size(TEXT + 1.0).font(bold()))
            .push(
                widget::row::with_capacity(2)
                    .align_y(Alignment::Center)
                    .push(widget::container(muted(w.address(), TEXT - 3.0, pal)).width(Length::Fill))
                    .push(tip(
                        widget::button::icon(icon::from_name("user-trash-symbolic")).extra_small().on_press(Message::Wiki(WikiMsg::Unfollow(w.id()))),
                        fl!("fandom-unfollow"),
                    )),
            );
        let card = boxed(widget::column::with_capacity(2).push(band).push(body), pal.panel, pal.border).width(Length::Fixed(260.0));
        widget::mouse_area(card)
            .on_press(Message::Wiki(WikiMsg::OpenWiki(w.id())))
            .interaction(cosmic::iced::mouse::Interaction::Pointer)
            .into()
    }
}
