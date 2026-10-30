use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use cosmic::app::{Core, Task};
use cosmic::dialog::file_chooser::{self, FileFilter};
use cosmic::iced::{
    event,
    keyboard::{self, key::Named, Key},
    mouse, touch, window, Alignment, Background, Color, ContentFit, Length, Subscription,
};
use cosmic::iced::widget::mouse_area;
use cosmic::widget::{self, icon};
use cosmic::{executor, Application, ApplicationExt, Element};
use url::Url;

use crate::comic::{self, ChapterInfo, PageSource};
use crate::epub;
use crate::comicinfo::{self, ComicInfo};
use crate::comicvine;
use crate::library::{self, IssueEntry, IssueRow, SeriesEntry, SeriesKind};
use crate::metadata::{self, SeriesMatch};
use crate::naming;
use crate::scan;
use crate::platform::look::{self, Corners, Look};
use crate::platform::Desktop;
use crate::settings::{Background as BgStyle, LibrarySource, Settings};
use crate::themes;

mod book_view;
mod paged;
mod library_view;
mod window_icon;
mod jellyfin_view;
mod opds_view;
mod downloads;
mod properties;
mod rar_index;
mod chapter_list;

use book_view::{BookMsg, BookScroll};
use jellyfin_view::{JfMsg, JfState, RemoteReading};
use opds_view::{OpdsMsg, OpdsState};
use downloads::{DlMsg, Downloads};

/// The modifier key named in hints: Cmd on a Mac, Ctrl elsewhere.
const MODIFIER_KEY: &str = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };

/// Dropdown labels in the interface language. Dropdowns borrow their
/// labels, so they live on the app and are rebuilt when the language
/// changes; each list is in the order of the matching `ALL` array.
struct Labels {
    /// "Match system", then each language by its own name.
    languages: Vec<String>,
    look: Vec<String>,
    corners: Vec<String>,
    background: Vec<String>,
    library_source: Vec<String>,
    book_line_spacing: Vec<String>,
    book_layout: Vec<String>,
    book_width: Vec<String>,
    book_theme: Vec<String>,
}

impl Labels {
    fn new(languages: &[crate::i18n::Language]) -> Self {
        Labels {
            languages: std::iter::once(fl!("language-system")).chain(languages.iter().map(|l| l.name.clone())).collect(),
            // The desktop styles are proper names.
            look: vec![fl!("style-auto"), "COSMIC".into(), "KDE Breeze".into(), "Windows 11".into(), "GNOME Adwaita".into()],
            corners: vec![fl!("corners-auto"), fl!("corners-round"), fl!("corners-slightly-round"), fl!("corners-square")],
            background: vec![fl!("background-theme"), fl!("background-black")],
            library_source: vec![fl!("source-all"), fl!("source-local"), "Jellyfin".into(), "OPDS".into()],
            book_line_spacing: vec![fl!("spacing-tight"), fl!("spacing-normal"), fl!("spacing-relaxed"), fl!("spacing-loose")],
            book_layout: vec![fl!("layout-single"), fl!("layout-two"), fl!("layout-scroll")],
            book_width: vec![fl!("width-narrow"), fl!("width-medium"), fl!("width-wide")],
            book_theme: vec![fl!("theme-auto"), fl!("theme-paper"), fl!("theme-sepia"), fl!("theme-dark"), fl!("theme-night")],
        }
    }
}

// The viewer's zoom scale is a multiplier on top of the already-computed
// fit-to-window size (ContentFit::Contain), so 1.0 *is* the fit-page view —
// flooring at 1.0 stops you from zooming out past the page's own edges.
const MIN_ZOOM: f32 = 1.0;
const MAX_ZOOM: f32 = 8.0;

pub const APP_ID: &str = "io.github.NotTimm.NebulaReader";

const PRELOAD_NEAR: usize = 1;
const PRELOAD_FAR: usize = 6;
const CACHE_RADIUS: usize = 14;

// ── View modes ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum AppView { Library, Series, Reader, Downloads, Catalog }

#[derive(Clone, Debug, PartialEq)]
pub enum Layout { Single, Dual }

/// Which panel the context drawer is currently showing.
#[derive(Clone, Debug, PartialEq)]
pub enum ContextPage { Settings, Info, Typography, Properties }

// ── Messages ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum Message {
    // Navigation
    GoToLibrary,
    OpenFile,
    OpenFolder,
    PathSelected(Url),
    /// Open a library entry in the reader; `chapter` picks an issue,
    /// `None` resumes where you left off.
    OpenEntry { key: String, chapter: Option<usize> },
    OpenSeriesPage(String),
    // Loading
    Loaded(PathBuf, Vec<PageSource>, Vec<ChapterInfo>),
    EpubLoaded(PathBuf, Result<epub::EpubBook, String>),
    /// A page decoded for the comic opened as `.0` (see `App::page_gen`).
    PageDecoded(u64, usize, Result<(widget::image::Handle, PageSize), String>),
    /// A decoded page is on the GPU, ready to draw the moment it's shown.
    PageAllocated(u64, usize, Option<cosmic::iced::runtime::image::Allocation>),
    LoadFailed(String),
    Cancelled,
    /// No-op message that just forces an extra render pass. Newly-decoded
    /// page textures sometimes don't actually appear on the frame they're
    /// inserted into (looks like a blank page until the next redraw, e.g.
    /// from pressing an arrow key) — this nudges iced to redraw once more
    /// shortly after a page lands in the cache.
    Redraw,
    // Reading
    NextPage,
    PrevPage,
    CloseError,
    // View toggles
    ToggleFullscreen,
    /// The pointer's height in the window, tracked only in full screen to
    /// reveal the hidden header bar at the top edge.
    PointerY(f32),
    ToggleTheater,
    ToggleLayout,
    ToggleZoom,
    /// Space: Fit to Width on or off.
    ToggleFitWidth,
    /// Fit to Width has finished easing back to the whole page.
    FitWidthEased,
    /// Hides the note saying Fit to Width went on or off (its number).
    HideFitNote(u64),
    ToggleInfo,
    ToggleChapterSelect,
    /// Jump the chapter panel to a volume or block of chapters.
    ChapterGroup(usize),
    SelectChapter(usize),
    // Copy
    CopyPage,
    CopiedPage(Result<(), String>),
    // Metadata
    MetaNext,
    MetaMatched(String, Result<Option<SeriesMatch>, String>),
    SeriesSearchInput(String),
    RematchSeries,
    ClearSeriesMatch,
    RefreshAllMetadata,
    ComicVineKeyInput(String),
    SaveComicVineKey,
    OpenUrl(String),
    // Tracking
    ToggleIssueRead(i64, bool),
    MarkSeriesRead(bool),
    // Chapter covers
    /// The open comic's ComicInfo.xml, for the comic opened as `.0`.
    ComicInfoRead(u64, Option<ComicInfo>),
    ChapterCoverReady { gen: u64, chapter_idx: usize, result: Result<(u32, u32, Vec<u8>), String> },
    // Library
    LibrarySearchChanged(String),
    LibrarySource(usize),
    ToggleLibraryMenu,
    CloseLibraryMenu,
    RootDiscovered(PathBuf, Option<Vec<scan::Discovered>>),
    SeriesScanned(Result<scan::ScanResult, String>),
    IssueCoverReady { key: String, idx: usize },
    HideMissing(Vec<String>),
    RescanLibrary,
    AddLibraryFolder,
    LibraryFolderSelected(Url),
    RemoveLibraryFolder(PathBuf),
    LibraryPathInputChanged(String),
    LibraryPathInputSubmitted,
    // Settings
    ToggleSettings,
    CloseContext,
    SetBackgroundOpacity(f32),
    SetBackground(usize),
    SetLook(usize),
    SetCorners(usize),
    /// Index into the Colors dropdown: 0 follows the system, then
    /// `App::themes`.
    SetTheme(usize),
    /// Index into Settings → Language: 0 follows the system.
    SetLanguage(usize),
    ImportTheme,
    ThemeFileSelected(Url),
    OpenThemesFolder,
    /// Time to refresh the full-screen clock and battery.
    StatusTick,
    StatusRead(String, Option<crate::platform::status::Battery>),
    SetFullscreenStatus(bool),
    /// Time to look for a changed system dark mode or accent colour.
    CheckSystemColors,
    /// Whether that check found a change.
    SystemColorsChecked(bool),
    /// A window opened; (re)apply the window icon to it.
    WindowOpened,
    SetFetchMetadata(bool),
    SetCheckForUpdates(bool),
    CheckForUpdates,
    UpdateChecked(Result<Option<crate::updates::Release>, String>),
    DismissUpdateBanner,
    UpdateNow,
    UpdateProgress(crate::updater::Progress),
    RestartToUpdate,
    DismissUpdatedBanner,
    // Keys / pointer / touch
    Key(Key),
    ModifiersChanged(keyboard::Modifiers),
    ZoomBootstrap,
    /// A wheel tick outside the zoom viewer; true for a physical mouse
    /// wheel (line steps), false for a trackpad (pixels).
    ZoomBootstrapWheel(mouse::ScrollDelta),
    ZoomChanged(f32),
    Touch(touch::Event),
    // Drag and drop
    FilesDropped(Vec<PathBuf>),
    DndDataReceived(String, Vec<u8>),
    // Books
    Book(BookMsg),
    Jf(JfMsg),
    /// OPDS catalogs: adding them and the Catalog page.
    Opds(OpdsMsg),
    /// Jellyfin downloads for offline reading.
    Dl(DlMsg),
    /// A library card was right-clicked.
    CardMenuOpened(String),
    CardMenu(properties::CardAction),
    Index(rar_index::IdxMsg),
    IssueMenuOpened(String),
    IssueMenu(rar_index::IssueAction),
    PageSizeRead(String, Option<(u32, u32)>),
    // Fit page
    FitPage,
    FitPageSize(cosmic::iced::Size),
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct App {
    core: Core,
    /// Whether this window was created with the system title bar. Fixed
    /// at startup: decorations can't be swapped on a live window.
    native_titlebar: bool,
    /// Which window icon is showing (open book while reading); `None`
    /// until the first one is set.
    icon_open: Option<bool>,
    // view
    app_view: AppView,
    // reader
    title: String,
    open_path: Option<PathBuf>,
    sources: Vec<PageSource>,
    chapters: Vec<ChapterInfo>,
    cache: HashMap<usize, (widget::image::Handle, PageSize)>,
    /// Pages kept on the GPU (near the current one), so turning to them
    /// draws at once instead of uploading first.
    allocations: HashMap<usize, cosmic::iced::runtime::image::Allocation>,
    /// Counts the comics opened, so a page decoded for one never lands in
    /// the next.
    page_gen: u64,
    pending: HashSet<usize>,
    current_page: usize,
    /// Last navigation direction (>0 forward, <0 backward, 0 unknown), used
    /// to bias page preloading toward where the reader is actually headed.
    nav_dir: i8,
    loading: bool,
    /// The comic being loaded is a Jellyfin CBR without a page index yet,
    /// so the loading screen says one is being created.
    creating_index: bool,
    error: Option<String>,
    fullscreen: bool,
    /// The header bar is revealed in full screen (pointer at the top edge).
    fullscreen_header: bool,
    /// The clock and battery shown in a corner while reading in full screen.
    status_clock: Option<String>,
    battery: Option<crate::platform::status::Battery>,
    /// Colour themes on offer, and the picked one's colours.
    themes: Vec<themes::Entry>,
    theme_labels: Vec<String>,
    /// Translations on offer, and dropdown labels in the chosen one.
    languages: Vec<crate::i18n::Language>,
    labels: Labels,
    theme_colors: Option<themes::Colors>,
    theme_error: Option<String>,
    theater_mode: bool,
    layout: Layout,
    zoom_active: bool,
    /// Fit to Width: the page as wide as the window, read top to bottom.
    /// Never remembered: each comic opens whole.
    fit_width: bool,
    /// Still easing back to the whole page after Fit to Width.
    fit_width_leaving: bool,
    /// The brief note saying Fit to Width went on or off, and its number
    /// (so an older note's timer doesn't hide a newer one).
    fit_note: Option<(String, u64)>,
    fit_note_counter: u64,
    /// Tells a mouse wheel from a touchpad scroll, for entering zoom.
    wheel: cosmic::iced::widget::image::viewer::WheelSource,
    zoom: f32,
    show_chapter_select: bool,
    /// The volume or block picked in the chapter panel's jump list.
    chapter_group: Option<usize>,
    comic_info: Option<ComicInfo>,
    /// The DB view of what's open in the reader (series + its issues).
    reader_entry: Option<SeriesEntry>,
    reader_issues: Vec<IssueEntry>,
    /// Issue to start at for the next `Loaded` (None = resume).
    start_chapter: Option<usize>,
    /// Whether "back" from the reader returns to a series page.
    back_to_series: bool,
    /// The library card whose right-click menu is open.
    menu_target: Option<String>,
    properties: Option<properties::Props>,
    /// Building Jellyfin CBR page indexes ahead of time.
    indexing: Option<rar_index::Indexing>,
    index_note: Option<String>,
    /// Jellyfin series -> their CBR chapters without a page index.
    unindexed: HashMap<String, Vec<String>>,
    /// Streamed Jellyfin CBRs with a page index -> whether they're solid.
    indexed: HashMap<String, bool>,
    /// The chapter whose right-click menu is open on the series page.
    issue_menu_target: Option<String>,
    // epub
    epub_book: Option<epub::EpubBook>,
    epub_chapter: usize,
    epub_cover: Option<widget::image::Handle>,
    /// One handle per book image, made once (a fresh handle per frame
    /// would re-upload the texture every redraw).
    epub_images: Vec<widget::image::Handle>,
    book_scroll: BookScroll,
    book_saved_pos: f32,
    /// Page breaks of the current chapter, from the last layout.
    book_pages: paged::Pages,
    /// A book image opened full-window, by index into `epub_images`.
    book_image: Option<usize>,
    // pointer / touch state
    modifiers: keyboard::Modifiers,
    touches: HashSet<touch::Finger>,
    // chapter covers (decoded for the currently open series)
    chapter_covers: HashMap<usize, widget::image::Handle>,
    covers_pending: HashSet<usize>,
    // library
    db: Option<rusqlite::Connection>,
    library_entries: Vec<SeriesEntry>,
    library_search: String,
    /// The All / Local / Jellyfin menu is open.
    library_menu: bool,
    /// The latest update check, shown in Settings → About.
    update: UpdateStatus,
    /// The first start of a new version: the library says it updated
    /// until the banner is closed.
    just_updated: bool,
    update_job: UpdateJob,
    /// The All / Local / Jellyfin switch above the library.
    library_covers: HashMap<i64, widget::image::Handle>,
    library_dirs: Vec<PathBuf>,
    library_path_input: String,
    /// Library roots that weren't reachable (unmounted drive, deleted) —
    /// their series are hidden, never deleted.
    missing_roots: HashSet<PathBuf>,
    /// One-off entries whose file is gone.
    hidden_paths: HashSet<String>,
    scans_pending: usize,
    // series page
    series_page: Option<SeriesEntry>,
    series_page_issues: Vec<IssueEntry>,
    series_page_covers: HashMap<usize, widget::image::Handle>,
    series_covers_pending: HashSet<usize>,
    series_search_input: String,
    // online metadata queue (one series at a time, rate-limit friendly)
    meta_queue: VecDeque<String>,
    meta_running: bool,
    /// Series finished in the current run of lookups, for the progress
    /// line; back to 0 once the queue is empty.
    meta_done: usize,
    meta_current: Option<String>,
    meta_attempted: HashSet<String>,
    cv_key_input: String,
    // settings
    settings: Settings,
    /// Signed-in Jellyfin accounts (tokens only).
    jellyfin: crate::jellyfin::Accounts,
    jf: JfState,
    /// OPDS catalogs added.
    opds: crate::opds::Accounts,
    op: OpdsState,
    dl: Downloads,
    /// The Jellyfin series being read, if any.
    remote: Option<RemoteReading>,
    context_page: ContextPage,
}

// ── Task helpers ──────────────────────────────────────────────────────────────

fn open_task(path: PathBuf) -> Task<Message> {
    cosmic::task::future(async move {
        let list_path = path.clone();
        match tokio::task::spawn_blocking(move || comic::collect_sources(&list_path)).await {
            Ok(Ok((sources, chapters))) => Message::Loaded(path, sources, chapters),
            Ok(Err(e)) => Message::LoadFailed(e),
            Err(e) => Message::LoadFailed(format!("listing task panicked: {e}")),
        }
    })
}

/// Wraps a header control in a hover hint, so every button says what it does.
pub(crate) fn tip<'a>(el: impl Into<Element<'a, Message>>, hint: impl Into<Cow<'a, str>>) -> Element<'a, Message> {
    widget::tooltip(el, widget::text(hint.into()), widget::tooltip::Position::Bottom).into()
}

/// The icon button that enters or leaves full screen. One button that
/// stays highlighted while full screen is on, so it can't be mistaken for
/// the window's own maximize button.
fn fullscreen_button<'a>(on: bool) -> Element<'a, Message> {
    tip(
        widget::button::icon(icon::from_name("view-fullscreen-symbolic"))
            .selected(on)
            .on_press(Message::ToggleFullscreen),
        if on { fl!("leave-full-screen") } else { fl!("full-screen") },
    )
}

fn epub_open_task(path: PathBuf) -> Task<Message> {
    cosmic::task::future(async move {
        let open_path = path.clone();
        match tokio::task::spawn_blocking(move || epub::open(&open_path)).await {
            Ok(Ok(book)) => Message::EpubLoaded(path, Ok(book)),
            Ok(Err(e)) => Message::EpubLoaded(path, Err(e)),
            Err(e) => Message::EpubLoaded(path, Err(format!("epub open task panicked: {e}"))),
        }
    })
}

/// Opens `path`, routing to the epub or comic pipeline based on extension.
fn open_any(path: PathBuf) -> Task<Message> {
    let is_epub = path
        .file_name()
        .map(|n| epub::is_epub_name(&n.to_string_lossy()))
        .unwrap_or(false);
    if is_epub {
        epub_open_task(path)
    } else {
        open_task(path)
    }
}

/// Opens a group series (loose files grouped by name) from its member list.
fn open_list_task(key: PathBuf, issues: Vec<PathBuf>) -> Task<Message> {
    cosmic::task::future(async move {
        match tokio::task::spawn_blocking(move || comic::collect_sources_for(&issues)).await {
            Ok(Ok((sources, chapters))) => Message::Loaded(key, sources, chapters),
            Ok(Err(e)) => Message::LoadFailed(e),
            Err(e) => Message::LoadFailed(format!("listing task panicked: {e}")),
        }
    })
}

/// Limits concurrent library-scan work so a big library (or a network
/// drive) doesn't get hammered by hundreds of parallel archive reads.
fn scan_semaphore() -> &'static tokio::sync::Semaphore {
    static SEM: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    SEM.get_or_init(|| tokio::sync::Semaphore::new(3))
}

fn discover_task(root: PathBuf) -> Task<Message> {
    cosmic::task::future(async move {
        let r = root.clone();
        let found = tokio::task::spawn_blocking(move || r.is_dir().then(|| scan::discover(&r)))
            .await
            .ok()
            .flatten();
        Message::RootDiscovered(root, found)
    })
}

fn scan_series_task(
    d: scan::Discovered,
    known: HashMap<String, usize>,
    missing_details: std::collections::HashSet<String>,
) -> Task<Message> {
    cosmic::task::future(async move {
        let _permit = scan_semaphore().acquire().await;
        let result = tokio::task::spawn_blocking(move || scan::scan_series(&d, known, missing_details)).await;
        Message::SeriesScanned(result.unwrap_or_else(|e| Err(format!("scan panicked: {e}"))))
    })
}

/// Cover thumbnail for one issue on the series page.
fn issue_cover_task(key: String, idx: usize, issue: PathBuf) -> Task<Message> {
    cosmic::task::future(async move {
        let _permit = scan_semaphore().acquire().await;
        let k = key.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let page = comic::first_page(&issue)?;
            let img = image::RgbaImage::from_raw(page.width, page.height, page.rgba)
                .ok_or_else(|| "bad rgba".to_string())?;
            scan::save_thumb(&img, &library::chapter_cover_path(&k, idx))
        })
        .await;
        Message::IssueCoverReady { key, idx }
    })
}

fn missing_check_task(paths: Vec<String>) -> Task<Message> {
    cosmic::task::future(async move {
        let missing = tokio::task::spawn_blocking(move || {
            paths.into_iter().filter(|p| !p.starts_with("group://") && !Path::new(p).exists()).collect()
        })
        .await
        .unwrap_or_default();
        Message::HideMissing(missing)
    })
}

/// Builds the online search for a series from what's in the DB.
fn build_query(entry: &SeriesEntry, issues: &[IssueEntry]) -> metadata::SeriesQuery {
    let search = if entry.search_override.is_empty() {
        naming::clean_series_name(&entry.title).search
    } else {
        entry.search_override.clone()
    };
    let looks_manga = issues.iter().any(|i| {
        let n = i.name.to_ascii_lowercase();
        n.starts_with("ch") || n.contains("chapter")
    });
    metadata::SeriesQuery {
        search,
        year: entry.year,
        volume: entry.volume,
        issue_numbers: issues.iter().filter(|i| !i.number.is_empty()).map(|i| i.number.clone()).collect(),
        looks_manga,
    }
}

fn issue_rows_from_chapters(chapters: &[ChapterInfo]) -> Vec<IssueRow> {
    chapters
        .iter()
        .map(|ch| {
            let file = ch.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let parsed = naming::parse_issue(&file);
            IssueRow {
                path: ch.path.to_string_lossy().into_owned(),
                name: ch.name.clone(),
                number: parsed.number.unwrap_or_default(),
                subtitle: parsed.subtitle.unwrap_or_default(),
                page_count: Some(ch.page_count),
                details: None,
            }
        })
        .collect()
}

/// Where to start reading: an explicit issue (resuming inside it if it's
/// in progress), else the last issue you were in (moving on to the next
/// one if you'd finished it), else the old global page index.
fn resume_page(
    chapters: &[ChapterInfo],
    issues: &[IssueEntry],
    entry: Option<&SeriesEntry>,
    start_chapter: Option<usize>,
    total: usize,
) -> usize {
    let issue_for = |ch: &ChapterInfo| {
        let p = ch.path.to_string_lossy();
        issues.iter().find(|i| i.path == p)
    };
    let clamp = |p: usize| p.min(total.saturating_sub(1));

    if let Some(idx) = start_chapter {
        if let Some(ch) = chapters.get(idx) {
            let offset = issue_for(ch)
                .filter(|i| i.in_progress())
                .map(|i| i.last_page.min(ch.page_count.saturating_sub(1)))
                .unwrap_or(0);
            return clamp(ch.start + offset);
        }
    }
    let Some(entry) = entry else { return 0 };
    if !entry.last_issue_path.is_empty() {
        if let Some((i, ch)) =
            chapters.iter().enumerate().find(|(_, ch)| ch.path.to_string_lossy() == entry.last_issue_path)
        {
            if let Some(issue) = issue_for(ch) {
                if issue.completed {
                    if let Some(next) = chapters.get(i + 1) {
                        return clamp(next.start);
                    }
                }
                return clamp(ch.start + issue.last_page.min(ch.page_count.saturating_sub(1)));
            }
        }
    }
    clamp(entry.last_read_page)
}

fn decode_semaphore() -> &'static tokio::sync::Semaphore {
    static SEM: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    SEM.get_or_init(|| {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        tokio::sync::Semaphore::new((cores / 2).max(2))
    })
}

/// The comic the reader has open (`App::page_gen`) and the page it's on,
/// so decodes queued earlier can tell they're no longer wanted.
static DECODE_GEN: AtomicU64 = AtomicU64::new(0);
/// Whether the full-screen header bar is showing (`App::fullscreen_header`),
/// for the pointer subscription.
static HEADER_SHOWN: AtomicBool = AtomicBool::new(false);
static DECODE_PAGE: AtomicUsize = AtomicUsize::new(0);

/// A decoded page's size in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSize {
    width: u32,
    height: u32,
}

fn decode_task(gen: u64, index: usize, source: PageSource) -> Task<Message> {
    cosmic::task::future(async move {
        let _permit = decode_semaphore().acquire().await;
        // Paged far past it, or opened something else, while it waited:
        // skip the fetch and decode, so the page on screen isn't queued
        // behind work nobody will see.
        if DECODE_GEN.load(Ordering::Relaxed) != gen
            || index.abs_diff(DECODE_PAGE.load(Ordering::Relaxed)) > CACHE_RADIUS
        {
            return Message::PageDecoded(gen, index, Err("no longer needed".into()));
        }
        let result = tokio::task::spawn_blocking(move || {
            // The pixels move into the handle: no second copy of the page.
            comic::decode_source(&source).map(|page| {
                let size = PageSize { width: page.width, height: page.height };
                (widget::image::Handle::from_rgba(page.width, page.height, page.rgba), size)
            })
        })
        .await;
        Message::PageDecoded(gen, index, result.unwrap_or_else(|e| Err(format!("decode panicked: {e}"))))
    })
}

fn copy_task(page: widget::image::Handle) -> Task<Message> {
    cosmic::task::future(async move {
        let result = tokio::task::spawn_blocking(move || {
            let widget::image::Handle::Rgba { width, height, pixels, .. } = &page else {
                return Err("copy: page isn't decoded".to_string());
            };
            let mut cb = arboard::Clipboard::new()
                .map_err(|e| format!("clipboard: {e}"))?;
            cb.set_image(arboard::ImageData {
                width: *width as usize,
                height: *height as usize,
                bytes: Cow::Borrowed(pixels),
            })
            .map_err(|e| format!("copy: {e}"))
        })
        .await;
        Message::CopiedPage(match result {
            Ok(r) => r,
            Err(e) => Err(format!("copy panicked: {e}")),
        })
    })
}

/// Decode the first page of `chapter_idx`, thumbnail it, save to disk, return rgba.
fn cover_task(gen: u64, chapter_idx: usize, source: PageSource, series_path: PathBuf) -> Task<Message> {
    // Two at a time: a series with hundreds of chapters would otherwise
    // decode hundreds of full pages at once, ahead of the pages being read.
    static COVERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
    cosmic::task::future(async move {
        let _permit = COVERS.acquire().await;
        if DECODE_GEN.load(Ordering::Relaxed) != gen {
            return Message::ChapterCoverReady { gen, chapter_idx, result: Err("no longer needed".into()) };
        }
        let result = tokio::task::spawn_blocking(move || {
            let page = comic::decode_source(&source)?;
            let img = image::RgbaImage::from_raw(page.width, page.height, page.rgba)
                .ok_or_else(|| "bad rgba".to_string())?;
            let thumb = scan::make_thumb(&img);
            // Save to disk cache
            let cover_path = library::chapter_cover_path(
                &series_path.to_string_lossy(),
                chapter_idx,
            );
            if let Some(parent) = cover_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("mkdir: {e}"))?;
            }
            // Save as PNG
            thumb.save(&cover_path)
                .map_err(|e| format!("save cover: {e}"))?;
            let (w, h) = thumb.dimensions();
            Ok::<_, String>((w, h, thumb.into_raw()))
        })
        .await;
        Message::ChapterCoverReady {
            gen,
            chapter_idx,
            result: match result {
                Ok(r) => r,
                Err(e) => Err(format!("cover panicked: {e}")),
            },
        }
    })
}

/// The Open File dialog's filter. The XDG portal (Linux) takes globs;
/// native dialogs on Windows/macOS (rfd) take bare extensions.
/// A small battery outline that fills with the charge, drawn in the text
/// colour (red when low and not charging), so it looks the same on every
/// platform and icon theme.
fn battery_glyph<'a>(b: crate::platform::status::Battery) -> Element<'a, Message> {
    let low = b.percent <= 15 && !b.charging;
    let fill = (16.0 * f32::from(b.percent) / 100.0).max(1.5);
    let color = move |theme: &cosmic::Theme| -> Color {
        if low { theme.cosmic().destructive_color().into() } else { theme.cosmic().on_bg_color().into() }
    };
    let level = widget::container(widget::Space::new().width(Length::Fixed(fill)).height(Length::Fill))
        .style(move |theme: &cosmic::Theme| widget::container::Style {
            background: Some(Background::Color(color(theme))),
            border: cosmic::iced::Border { radius: 1.0.into(), ..Default::default() },
            ..Default::default()
        });
    let body = widget::container(level)
        .padding(2)
        .width(Length::Fixed(22.0))
        .height(Length::Fixed(11.0))
        .style(move |theme: &cosmic::Theme| widget::container::Style {
            border: cosmic::iced::Border { color: color(theme), width: 1.2, radius: 2.5.into() },
            ..Default::default()
        });
    let nub = widget::container(widget::Space::new().width(Length::Fixed(2.0)).height(Length::Fixed(5.0)))
        .style(move |theme: &cosmic::Theme| widget::container::Style {
            background: Some(Background::Color(color(theme))),
            border: cosmic::iced::Border { radius: [0.0, 1.0, 1.0, 0.0].into(), ..Default::default() },
            ..Default::default()
        });
    widget::row::with_capacity(2).spacing(1).align_y(Alignment::Center).push(body).push(nub).into()
}

fn comic_file_filter() -> FileFilter {
    const EXTENSIONS: [&str; 5] = ["cbz", "cbr", "zip", "rar", "epub"];
    let mut filter = FileFilter::new(&fl!("file-filter-comics"));
    for ext in EXTENSIONS {
        #[cfg(target_os = "linux")]
        {
            filter = filter.glob(&format!("*.{ext}"));
        }
        #[cfg(not(target_os = "linux"))]
        {
            filter = filter.extension(ext);
        }
    }
    filter
}

/// Expand a leading `~` or `~/...` to the user's home directory, so the
/// typed library-path field accepts what a shell would.
fn shellexpand_home(path: &str) -> String {
    let home = crate::platform::home_dir();
    if path == "~" {
        return home.to_string_lossy().into_owned();
    }
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        return home.join(rest).to_string_lossy().into_owned();
    }
    path.to_string()
}

// ── App helpers ───────────────────────────────────────────────────────────────

impl App {
    /// Small text next to the home button saying where the open comic or
    /// book comes from: "Local" (hover for the path) or the Jellyfin server
    /// with its sync state (hover for details).
    fn source_label(&self) -> Element<'_, Message> {
        let (label, online, hint) = self.jellyfin_source().unwrap_or_else(|| {
            let path = self.open_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            (fl!("source-local"), None, path)
        });
        let mut content = widget::row::with_capacity(2).spacing(4).align_y(Alignment::Center).push(widget::text::caption(label));
        if let Some(online) = online {
            let color = if online { Color::from_rgb8(0x3f, 0xb9, 0x50) } else { Color::from_rgb8(0xe0, 0x4a, 0x4a) };
            content = content.push(widget::text::caption("●").class(cosmic::theme::Text::Color(color)));
        }
        tip(
            widget::container(content).padding([0, 6]).height(Length::Fill).align_y(Alignment::Center),
            hint,
        )
    }

    /// The look in use, with `Auto` resolved for this desktop.
    fn look(&self) -> Look {
        self.settings.look.resolve(Desktop::detect())
    }

    /// Whether the full-screen clock and battery are on screen.
    fn status_shown(&self) -> bool {
        self.fullscreen && self.app_view == AppView::Reader && self.settings.fullscreen_status
    }

    /// A small pill with the battery and the time.
    fn status_overlay(&self) -> Option<Element<'_, Message>> {
        if !self.status_shown() {
            return None;
        }
        let clock = self.status_clock.clone()?;
        let mut row = widget::row::with_capacity(4).spacing(6).align_y(Alignment::Center);
        if let Some(b) = self.battery {
            row = row.push(battery_glyph(b));
            if b.charging {
                row = row.push(widget::icon::icon(library_view::bolt_icon()).size(12));
            }
            row = row.push(widget::text::caption(format!("{}%", b.percent)));
            row = row.push(widget::Space::new().width(Length::Fixed(4.0)));
        }
        row = row.push(widget::text::caption(clock));
        Some(
            widget::container(row)
                .padding([4, 10])
                .style(|theme: &cosmic::Theme| {
                    let mut bg: Color = theme.cosmic().bg_color().into();
                    bg.a = 0.7;
                    widget::container::Style {
                        background: Some(Background::Color(bg)),
                        text_color: Some(theme.cosmic().on_bg_color().into()),
                        border: cosmic::iced::Border { radius: 12.0.into(), ..Default::default() },
                        ..Default::default()
                    }
                })
                .into(),
        )
    }

    /// Rebuilds the theme from the style, corners and colour theme.
    fn apply_theme(&self) -> Task<Message> {
        cosmic::command::set_theme(look::theme(self.look(), self.settings.corners, self.theme_colors.as_ref()))
    }

    /// Re-lists the colour themes (built-in plus the themes folder).
    fn refresh_themes(&mut self) {
        self.themes = themes::list();
        self.theme_labels = std::iter::once(fl!("colors-system"))
            .chain(self.themes.iter().map(|t| t.name.clone()))
            .collect();
    }

    /// Switches to a colour theme (`None`: follow the system).
    fn select_theme(&mut self, id: Option<String>) -> Task<Message> {
        let colors = match id.as_deref().map(themes::load).transpose() {
            Ok(colors) => colors,
            Err(e) => {
                self.theme_error = Some(e);
                return Task::none();
            }
        };
        self.theme_error = None;
        self.theme_colors = colors;
        self.settings.theme = id;
        self.settings.save();
        self.apply_theme()
    }

    /// Queues decodes for pages around the current one, biased toward
    /// whichever direction the reader is actually flipping — reading is
    /// overwhelmingly sequential, so weighting the preload window that way
    /// makes it much less likely fast paging outruns what's already decoded.
    fn refresh_window(&mut self) -> Task<Message> {
        let len = self.sources.len();
        if len == 0 {
            return Task::none();
        }
        let keep_start = self.current_page.saturating_sub(CACHE_RADIUS);
        let keep_end = (self.current_page + CACHE_RADIUS).min(len - 1);
        self.cache.retain(|idx, _| (keep_start..=keep_end).contains(idx));
        let gpu = self.current_page.saturating_sub(PRELOAD_FAR)..=self.current_page + PRELOAD_FAR;
        self.allocations.retain(|idx, _| gpu.contains(idx));
        DECODE_GEN.store(self.page_gen, Ordering::Relaxed);
        DECODE_PAGE.store(self.current_page, Ordering::Relaxed);

        let (ahead, behind) = if self.nav_dir < 0 {
            (PRELOAD_NEAR, PRELOAD_FAR)
        } else {
            (PRELOAD_FAR, PRELOAD_NEAR)
        };

        let mut order = vec![self.current_page];
        for step in 1..=ahead.max(behind) {
            if step <= ahead {
                if let Some(idx) = self.current_page.checked_add(step) {
                    order.push(idx);
                }
            }
            if step <= behind {
                if let Some(idx) = self.current_page.checked_sub(step) {
                    order.push(idx);
                }
            }
        }

        let mut tasks = Vec::new();
        for idx in order {
            if idx >= len || self.cache.contains_key(&idx) || self.pending.contains(&idx) {
                continue;
            }
            self.pending.insert(idx);
            tasks.push(decode_task(self.page_gen, idx, self.sources[idx].clone()));
        }
        Task::batch(tasks)
    }

    /// Whether a decoded page is a two-page spread: scanned as one image
    /// wider than it is tall. Pages not decoded yet count as normal ones.
    fn is_spread(&self, idx: usize) -> bool {
        self.cache.get(&idx).is_some_and(|(_, p)| p.width as f32 > p.height as f32 * 1.05)
    }

    /// Pages on screen from `idx` on: two in two-page mode, except that a
    /// spread is shown alone, and so is the page just before one.
    fn pages_shown_at(&self, idx: usize) -> usize {
        let pair = self.layout == Layout::Dual
            && idx + 1 < self.sources.len()
            && !self.is_spread(idx)
            && !self.is_spread(idx + 1);
        if pair { 2 } else { 1 }
    }

    /// Where the view before `current_page` starts: two pages back when
    /// they make a pair, one when that page is a spread (or stands alone).
    fn prev_view_start(&self) -> usize {
        let p = self.current_page;
        if p >= 2 && self.pages_shown_at(p - 2) == 2 { p - 2 } else { p.saturating_sub(1) }
    }

    /// Enters zoom/pan mode (at 100%) if not already active. The actual
    /// zoom-to-cursor math (mouse wheel, trackpad pinch, touchscreen pinch,
    /// click-drag pan) all lives inside `widget::image::viewer`, which is
    /// only present in the tree once zoom is active — this just handles
    /// the very first gesture tick that switches into that widget.
    fn enter_zoom(&mut self) {
        if !self.zoom_active {
            self.zoom_active = true;
            self.zoom = 1.0;
        }
    }

    /// Tracks active touch points only to detect the two-finger-down that
    /// bootstraps zoom mode from the fit-to-window view.
    fn handle_touch(&mut self, event: touch::Event) {
        match event {
            touch::Event::FingerPressed { id, .. } => {
                self.touches.insert(id);
                if self.app_view == AppView::Reader && self.touches.len() == 2 {
                    self.enter_zoom();
                }
            }
            touch::Event::FingerLifted { id, .. } | touch::Event::FingerLost { id, .. } => {
                self.touches.remove(&id);
            }
            touch::Event::FingerMoved { .. } => {}
        }
    }

    fn handle(&self, idx: usize) -> Option<&widget::image::Handle> {
        self.cache.get(&idx).map(|(h, _)| h)
    }

    /// The closest already-decoded page to `current_page`, used as a dimmed
    /// placeholder while the actual page is still decoding.
    fn nearest_cached_handle(&self) -> Option<&widget::image::Handle> {
        self.cache
            .iter()
            .min_by_key(|(&idx, _)| idx.abs_diff(self.current_page))
            .map(|(_, (h, _))| h)
    }

    fn page_size(&self, idx: usize) -> Option<PageSize> {
        self.cache.get(&idx).map(|(_, p)| *p)
    }

    /// Forgets the open comic's pages, including decodes still under way.
    pub(super) fn clear_pages(&mut self) {
        self.cache.clear();
        self.allocations.clear();
        self.pending.clear();
        self.page_gen += 1;
        DECODE_GEN.store(self.page_gen, Ordering::Relaxed);
    }

    fn current_chapter(&self) -> Option<(usize, &ChapterInfo, usize)> {
        self.chapters.iter().enumerate().find_map(|(i, ch)| {
            if self.current_page >= ch.start && self.current_page < ch.start + ch.page_count {
                Some((i, ch, self.current_page - ch.start))
            } else {
                None
            }
        })
    }

    /// Register `path` as a library folder (if not already) and scan it.
    /// Shared by the "Browse…" file-picker flow and the typed-path field.
    fn add_library_folder(&mut self, path: PathBuf) -> Task<Message> {
        if let Some(db) = &self.db {
            let _ = library::add_library_dir(db, &path.to_string_lossy());
        }
        if !self.library_dirs.contains(&path) {
            self.library_dirs.push(path.clone());
        }
        self.scan_root(path)
    }

    fn scan_root(&mut self, root: PathBuf) -> Task<Message> {
        self.scans_pending += 1;
        discover_task(root)
    }

    /// Opens a library entry (series, group, single file or book) in the
    /// reader. `chapter` jumps to an issue; `None` resumes.
    fn open_entry(&mut self, key: String, chapter: Option<usize>) -> Task<Message> {
        self.loading = true;
        self.creating_index = false;
        self.error = None;
        self.back_to_series = self.app_view == AppView::Series;
        self.app_view = AppView::Reader;
        self.start_chapter = chapter;
        self.current_page = 0;
        self.show_chapter_select = false;

        if crate::remote::is_key(&key) {
            return self.open_remote(key, chapter);
        }
        let entry = self.db.as_ref().and_then(|db| library::get_series(db, &key));
        if let Some(e) = entry.as_ref().filter(|e| e.kind == SeriesKind::Group) {
            let issues: Vec<PathBuf> = self
                .db
                .as_ref()
                .map(|db| library::issues_for(db, &e.path).into_iter().map(|i| PathBuf::from(i.path)).collect())
                .unwrap_or_default();
            return open_list_task(PathBuf::from(&key), issues);
        }
        open_any(PathBuf::from(key))
    }

    /// Opens a path picked/dropped by the user. If that file is an issue of
    /// a series already in the library, open the series at that issue so
    /// progress and "next issue" carry on there (issue linking).
    fn open_linked(&mut self, path: PathBuf) -> Task<Message> {
        let key = path.to_string_lossy().into_owned();
        if let Some((series, idx)) = self.db.as_ref().and_then(|db| library::series_for_issue(db, &key)) {
            return self.open_entry(series, Some(idx));
        }
        self.open_entry(key, None)
    }

    /// A series' chapters for its page: all of them, or only the downloaded
    /// ones while its Jellyfin server can't be reached.
    fn series_issues(&self, key: &str) -> Vec<IssueEntry> {
        let Some(db) = &self.db else { return Vec::new() };
        let mut issues = library::issues_for(db, key);
        if self.jellyfin_offline(key) {
            issues.retain(|i| self.dl.file(&i.path).is_some());
        }
        issues
    }

    /// Whether a library key belongs to a Jellyfin server that couldn't be
    /// reached at the last sync.
    fn jellyfin_offline(&self, key: &str) -> bool {
        crate::remote::parse_key(key).is_some_and(|(account, _)| self.jf.offline.contains(account))
    }

    fn open_series_page(&mut self, key: String) -> Task<Message> {
        let Some(db) = &self.db else { return Task::none() };
        let Some(entry) = library::get_series(db, &key) else { return Task::none() };
        if self.series_page.as_ref().map(|s| &s.path) != Some(&key) {
            self.series_page_covers.clear();
            self.series_covers_pending.clear();
            self.series_search_input = entry.search_override.clone();
        }
        self.series_page_issues = self.series_issues(&key);
        self.set_header_title(entry.name().to_string());
        self.series_page = Some(entry);
        self.app_view = AppView::Series;

        let mut tasks = Vec::new();
        for issue in &self.series_page_issues {
            if self.series_page_covers.contains_key(&issue.idx) || self.series_covers_pending.contains(&issue.idx) {
                continue;
            }
            let cover = library::chapter_cover_path(&key, issue.idx);
            if cover.exists() {
                self.series_page_covers.insert(issue.idx, widget::image::Handle::from_path(cover));
            } else {
                self.series_covers_pending.insert(issue.idx);
                if crate::remote::is_key(&issue.path) {
                    tasks.push(self.remote_issue_cover(key.clone(), issue.idx, &issue.path));
                } else {
                    tasks.push(issue_cover_task(key.clone(), issue.idx, PathBuf::from(&issue.path)));
                }
            }
        }
        Task::batch(tasks)
    }

    /// Re-reads DB-backed state for whatever's on screen.
    fn refresh_views(&mut self) -> Task<Message> {
        let mut tasks = vec![self.reload_library()];
        if let (Some(db), Some(key)) = (&self.db, self.series_page.as_ref().map(|s| s.path.clone())) {
            self.series_page = library::get_series(db, &key);
            self.series_page_issues = self.series_issues(&key);
        }
        if let (Some(db), Some(key)) = (&self.db, self.open_path.as_ref()) {
            let key = key.to_string_lossy();
            self.reader_entry = library::get_series(db, &key);
            self.reader_issues = library::issues_for(db, &key);
        }
        tasks.push(Task::none());
        Task::batch(tasks)
    }

    fn enqueue_meta(&mut self, key: String) -> Task<Message> {
        if !self.settings.fetch_metadata || self.meta_attempted.contains(&key) {
            return Task::none();
        }
        self.meta_attempted.insert(key.clone());
        self.meta_queue.push_back(key);
        self.pump_meta()
    }

    fn pump_meta(&mut self) -> Task<Message> {
        if self.meta_running {
            return Task::none();
        }
        let Some(db) = &self.db else { return Task::none() };
        while let Some(key) = self.meta_queue.pop_front() {
            let Some(entry) = library::get_series(db, &key) else {
                self.meta_done += 1;
                continue;
            };
            if entry.is_epub {
                self.meta_done += 1;
                continue;
            }
            let issues = library::issues_for(db, &key);
            let query = build_query(&entry, &issues);
            if query.search.trim().is_empty() {
                // Nothing to search with (yet) — let a later scan retry.
                self.meta_attempted.remove(&key);
                self.meta_done += 1;
                continue;
            }
            self.meta_running = true;
            self.meta_current = Some(key.clone());
            return cosmic::task::future(async move {
                let result = metadata::match_series(query).await;
                Message::MetaMatched(key, result)
            });
        }
        self.meta_done = 0;
        Task::none()
    }

    /// Lookups still to do in this run, including the one in flight.
    fn meta_left(&self) -> usize {
        self.meta_queue.len() + usize::from(self.meta_running)
    }

    /// Queue cover generation for chapters that don't have covers yet.
    fn queue_missing_covers(&mut self) -> Task<Message> {
        let Some(series_path) = &self.open_path else { return Task::none() };
        let series_path = series_path.clone();
        let path_str = series_path.to_string_lossy().to_string();

        let mut tasks = Vec::new();
        for (ch_idx, ch) in self.chapters.iter().enumerate() {
            if self.chapter_covers.contains_key(&ch_idx)
                || self.covers_pending.contains(&ch_idx)
            {
                continue;
            }
            // Check disk cache first
            let cover_path = library::chapter_cover_path(&path_str, ch_idx);
            if cover_path.exists() {
                let handle = widget::image::Handle::from_path(&cover_path);
                self.chapter_covers.insert(ch_idx, handle);
                continue;
            }
            // Need to generate
            if ch.page_count > 0 {
                self.covers_pending.insert(ch_idx);
                let source = self.sources[ch.start].clone();
                tasks.push(cover_task(self.page_gen, ch_idx, source, series_path.clone()));
            }
        }
        Task::batch(tasks)
    }

    /// Save reading progress to DB (cheap, called on page change). Tracks
    /// the page *within* the current issue, so adding issues to a folder
    /// later never shifts anyone's place.
    fn persist_progress(&self) {
        let Some(db) = &self.db else { return };
        let Some(path) = &self.open_path else { return };
        let key = path.to_string_lossy();
        if self.epub_book.is_some() {
            return; // books save chapter + scroll position in book_view
        }
        if let Some((ch_idx, ch, pg)) = self.current_chapter() {
            // In two-page mode the right-hand page is on screen too.
            let visible = pg + self.pages_shown_at(self.current_page) - 1;
            let visible = visible.min(ch.page_count.saturating_sub(1));
            let _ = library::save_issue_progress(
                db,
                &key,
                &ch.path.to_string_lossy(),
                visible,
                ch.page_count,
                self.current_page,
                ch_idx,
            );
        }
    }

    fn reload_library(&mut self) -> Task<Message> {
        let Some(db) = &self.db else { return Task::none() };

        let entries = if self.library_search.is_empty() {
            library::all_series(db)
        } else {
            library::search_series(db, &self.library_search)
        };

        for entry in &entries {
            if self.library_covers.contains_key(&entry.id) {
                continue;
            }
            let cover_path = library::chapter_cover_path(&entry.path, 0);
            if cover_path.exists() {
                self.library_covers.insert(entry.id, widget::image::Handle::from_path(cover_path));
            }
        }

        self.library_entries = entries;
        Task::none()
    }
}

/// Quitting with Cmd+Q on a Mac closes the window without the close
/// request that sends the last Jellyfin progress (`JfMsg::Closing`), so
/// it's sent here too. It does nothing the second time.
impl Drop for App {
    fn drop(&mut self) {
        let _ = self.update_jellyfin(JfMsg::Closing);
    }
}

/// Where the update check stands (Settings → About).
enum UpdateStatus {
    NotChecked,
    Checking,
    UpToDate,
    Available(crate::updates::Release),
    Failed(String),
}

/// An in-app update (`crate::updater`) the user started.
enum UpdateJob {
    Idle,
    Downloading { done: u64, total: Option<u64> },
    /// Downloaded and checked; installs when the app restarts.
    Ready(crate::updater::Staged),
    Failed(String),
}

// ── Application ───────────────────────────────────────────────────────────────

impl Application for App {
    type Executor = executor::Default;
    type Flags = Option<PathBuf>;
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core { &self.core }
    fn core_mut(&mut self) -> &mut Core { &mut self.core }

    fn init(core: Core, flags: Self::Flags) -> (Self, Task<Self::Message>) {
        let db = library::open_db().ok();
        if let Some(db) = &db {
            let removed = library::prune_missing_loose_files(db);
            if removed > 0 {
                log::info!("forgot {removed} opened files that were deleted");
            }
        }
        let mut settings = Settings::load();
        // Settings saved by an older version (before `last_version`
        // existed, too) mean this is the first start after an update; a
        // fresh install has none.
        let just_updated = settings.last_version.as_deref() != Some(crate::updates::VERSION)
            && (settings.last_version.is_some() || Settings::exists());
        // Theater mode became the default in 0.1.10; turn it on once for
        // settings saved before that, which all hold the old `false`.
        if just_updated
            && settings.last_version.as_deref().is_none_or(|v| crate::updates::is_newer("0.1.10", v))
        {
            settings.theater_mode = true;
        }
        if settings.last_version.as_deref() != Some(crate::updates::VERSION) {
            settings.last_version = Some(crate::updates::VERSION.into());
            settings.save();
        }
        let library_dirs: Vec<PathBuf> = db
            .as_ref()
            .map(|db| library::list_library_dirs(db).into_iter().map(PathBuf::from).collect())
            .unwrap_or_default();

        let native_titlebar = settings.look.resolve(Desktop::detect()).native_titlebar();
        let dl = db.as_ref().map(Downloads::load).unwrap_or_default();
        let mut app = App {
            core,
            native_titlebar,
            icon_open: None,
            app_view: if flags.is_none() { AppView::Library } else { AppView::Reader },
            title: "Nebula".to_string(),
            open_path: None,
            sources: Vec::new(),
            chapters: Vec::new(),
            cache: HashMap::new(),
            allocations: HashMap::new(),
            page_gen: 0,
            pending: HashSet::new(),
            current_page: 0,
            nav_dir: 0,
            loading: flags.is_some(),
            creating_index: false,
            error: None,
            fullscreen: false,
            fullscreen_header: false,
            status_clock: None,
            battery: None,
            themes: Vec::new(),
            theme_labels: Vec::new(),
            languages: crate::i18n::available(),
            labels: Labels::new(&crate::i18n::available()),
            theme_colors: settings.theme.as_deref().and_then(|id| themes::load(id).ok()),
            theme_error: None,
            theater_mode: settings.theater_mode,
            layout: if settings.dual_page { Layout::Dual } else { Layout::Single },
            zoom_active: false,
            fit_width: false,
            fit_width_leaving: false,
            fit_note: None,
            fit_note_counter: 0,
            wheel: Default::default(),
            zoom: 1.0,
            show_chapter_select: false,
            chapter_group: None,
            comic_info: None,
            reader_entry: None,
            reader_issues: Vec::new(),
            start_chapter: None,
            back_to_series: false,
            menu_target: None,
            properties: None,
            indexing: None,
            index_note: None,
            unindexed: HashMap::new(),
            indexed: HashMap::new(),
            issue_menu_target: None,
            epub_book: None,
            epub_chapter: 0,
            epub_cover: None,
            epub_images: Vec::new(),
            book_scroll: BookScroll::default(),
            book_saved_pos: 0.0,
            book_pages: paged::Pages::default(),
            book_image: None,
            modifiers: keyboard::Modifiers::default(),
            touches: HashSet::new(),
            chapter_covers: HashMap::new(),
            covers_pending: HashSet::new(),
            db,
            library_entries: Vec::new(),
            library_search: String::new(),
            library_menu: false,
            update: UpdateStatus::NotChecked,
            just_updated,
            update_job: UpdateJob::Idle,
            library_covers: HashMap::new(),
            library_dirs: library_dirs.clone(),
            library_path_input: String::new(),
            missing_roots: HashSet::new(),
            hidden_paths: HashSet::new(),
            scans_pending: 0,
            series_page: None,
            series_page_issues: Vec::new(),
            series_page_covers: HashMap::new(),
            series_covers_pending: HashSet::new(),
            series_search_input: String::new(),
            meta_queue: VecDeque::new(),
            meta_running: false,
            meta_done: 0,
            meta_current: None,
            meta_attempted: HashSet::new(),
            cv_key_input: String::new(),
            settings,
            jellyfin: crate::jellyfin::Accounts::load(),
            jf: JfState::default(),
            opds: crate::opds::Accounts::load(),
            op: OpdsState::default(),
            dl,
            remote: None,
            context_page: ContextPage::Settings,
        };
        app.set_header_title("Nebula".into());
        if app.native_titlebar {
            // The system title bar has the window buttons and title, so
            // libcosmic's header bar becomes a plain toolbar.
            let window = &mut app.core.window;
            window.show_close = false;
            window.show_maximize = false;
            window.show_minimize = false;
            window.sharp_corners = true;
        }

        let mut tasks = vec![match app.core.main_window_id() {
            Some(id) => app.set_window_title("Nebula".into(), id),
            None => Task::none(),
        }];

        tasks.push(app.reload_library());
        // Files an earlier update left behind.
        std::thread::spawn(crate::updater::clean_up);
        tasks.push(app.sync_all_remote());
        if app.settings.check_for_updates && crate::updates::Install::detect().checks_github() {
            tasks.push(app.check_for_updates());
        }
        if let Some(path) = flags {
            tasks.push(app.open_linked(path));
        }

        // Rescan every library folder in the background (cheap for
        // anything already known), and hide one-off entries whose files
        // have since disappeared.
        for dir in library_dirs {
            tasks.push(app.scan_root(dir));
        }
        let one_offs: Vec<String> =
            app.library_entries.iter().filter(|e| e.library_root.is_empty()).map(|e| e.path.clone()).collect();
        tasks.push(missing_check_task(one_offs));
        // Retry lookups for anything never matched (e.g. added offline).
        let pending: Vec<String> = app
            .library_entries
            .iter()
            .filter(|e| e.meta_fetched_at == 0 && !e.is_epub && !e.title.is_empty())
            .map(|e| e.path.clone())
            .collect();
        for key in pending {
            tasks.push(app.enqueue_meta(key));
        }

        app.refresh_themes();
        (app, Task::batch(tasks))
    }

    fn context_drawer(&self) -> Option<cosmic::app::context_drawer::ContextDrawer<'_, Message>> {
        if !self.core.window.show_context {
            return None;
        }
        Some(match self.context_page {
            ContextPage::Settings => cosmic::app::context_drawer::context_drawer(
                self.build_settings_panel(),
                Message::CloseContext,
            )
            .title(fl!("settings")),
            ContextPage::Info => cosmic::app::context_drawer::context_drawer(
                if self.epub_book.is_some() {
                    self.build_epub_info_panel()
                } else {
                    self.build_info_panel()
                },
                Message::CloseContext,
            )
            .title(fl!("details")),
            ContextPage::Typography => cosmic::app::context_drawer::context_drawer(
                self.build_typography_panel(),
                Message::CloseContext,
            )
            .title(fl!("reading-panel")),
            ContextPage::Properties => cosmic::app::context_drawer::context_drawer(
                self.build_properties_panel(),
                Message::CloseContext,
            )
            .title(fl!("properties")),
        })
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        match self.app_view {
            AppView::Reader if self.back_to_series => vec![
                tip(
                    widget::button::icon(icon::from_name("go-previous-symbolic")).on_press(Message::GoToLibrary),
                    fl!("back-to-series"),
                ),
                self.source_label(),
            ],
            AppView::Reader => vec![
                tip(
                    widget::button::icon(icon::from_name("go-home-symbolic")).on_press(Message::GoToLibrary),
                    fl!("back-to-library-esc"),
                ),
                self.source_label(),
            ],
            AppView::Series | AppView::Downloads | AppView::Catalog => vec![tip(
                widget::button::icon(icon::from_name("go-home-symbolic")).on_press(Message::GoToLibrary),
                fl!("back-to-library"),
            )],
            // All / Local / Jellyfin, once the library has some of each.
            AppView::Library if self.has_local_and_remote() => vec![self.library_source_picker()],
            AppView::Library => vec![],
        }
    }

    fn header_end(&self) -> Vec<Element<'_, Self::Message>> {
        let mut els: Vec<Element<'_, Message>> = Vec::new();

        match self.app_view {
            AppView::Series | AppView::Downloads | AppView::Catalog => {
                els.push(
                    widget::button::icon(icon::from_name("emblem-system-symbolic"))
                        .on_press(Message::ToggleSettings)
                        .into(),
                );
            }
            AppView::Library => {
                els.push(
                    widget::search_input(fl!("search-library"), &self.library_search)
                        .on_input(Message::LibrarySearchChanged)
                        .width(Length::Fixed(220.0))
                        .into(),
                );
                // Downloads, once there are some.
                if !self.dl.is_empty() {
                    els.push(tip(
                        widget::button::icon(icon::from_name("folder-download-symbolic"))
                            .on_press(Message::Dl(DlMsg::OpenManager)),
                        fl!("downloads"),
                    ));
                }
                els.push(
                    widget::button::standard(fl!("open-file"))
                        .on_press(Message::OpenFile)
                        .into(),
                );
                els.push(
                    widget::button::suggested(fl!("add-library-folder"))
                        .on_press(Message::AddLibraryFolder)
                        .into(),
                );
                els.push(
                    widget::button::icon(icon::from_name("emblem-system-symbolic"))
                        .on_press(Message::ToggleSettings)
                        .into(),
                );
            }
            AppView::Reader if self.epub_book.is_some() => {
                let book = self.epub_book.as_ref().unwrap();
                els.push(tip(
                    widget::button::icon(icon::from_name("go-previous-symbolic"))
                        .on_press(Message::Book(BookMsg::PrevChapter)),
                    fl!("previous-chapter"),
                ));
                els.push(tip(
                    widget::text(format!("{} / {}", self.epub_chapter + 1, book.chapters.len())),
                    fl!("chapter"),
                ));
                els.push(tip(
                    widget::button::icon(icon::from_name("go-next-symbolic"))
                        .on_press(Message::Book(BookMsg::NextChapter)),
                    fl!("next-chapter"),
                ));
                els.push(widget::divider::vertical::light().into());
                els.push(tip(
                    widget::button::icon(icon::from_name("view-list-symbolic"))
                        .selected(self.show_chapter_select)
                        .on_press(Message::ToggleChapterSelect),
                    fl!("contents-tip"),
                ));
                els.push(tip(
                    widget::button::standard("Aa").on_press(Message::Book(BookMsg::ToggleTypography)),
                    fl!("typography-tip"),
                ));
                els.push(tip(
                    widget::button::icon(icon::from_name("dialog-information-symbolic"))
                        .on_press(Message::ToggleInfo),
                    fl!("book-details-tip"),
                ));
                els.push(fullscreen_button(self.fullscreen));
                els.push(tip(
                    widget::button::icon(icon::from_name("emblem-system-symbolic"))
                        .on_press(Message::ToggleSettings),
                    fl!("settings-tip"),
                ));
            }
            AppView::Reader => {
                if !self.sources.is_empty() {
                    // Chapter-aware counter
                    let counter = if let Some((ch_idx, ch, pg)) = self.current_chapter() {
                        if self.chapters.len() == 1 {
                            format!("{} / {}", pg + 1, ch.page_count)
                        } else {
                            fl!("chapter-page-counter", chapter = ch_idx + 1, page = pg + 1, pages = ch.page_count)
                        }
                    } else {
                        format!("{} / {}", self.current_page + 1, self.sources.len())
                    };

                    els.push(tip(
                        widget::button::icon(icon::from_name("go-previous-symbolic")).on_press(Message::PrevPage),
                        fl!("previous-page"),
                    ));
                    els.push(tip(widget::text(counter), fl!("page")));
                    els.push(tip(
                        widget::button::icon(icon::from_name("go-next-symbolic")).on_press(Message::NextPage),
                        fl!("next-page"),
                    ));
                    els.push(widget::divider::vertical::light().into());

                    // Chapter select (only for multi-chapter series)
                    if self.chapters.len() > 1 || self.reader_issues.len() > 1 {
                        els.push(tip(
                            widget::button::icon(icon::from_name("view-list-symbolic"))
                                .selected(self.show_chapter_select)
                                .on_press(Message::ToggleChapterSelect),
                            fl!("chapters-tip"),
                        ));
                    }
                    els.push(tip(
                        widget::button::icon(icon::from_name("view-dual-symbolic"))
                            .selected(self.layout == Layout::Dual)
                            .on_press(Message::ToggleLayout),
                        if self.layout == Layout::Dual { fl!("show-one-page") } else { fl!("show-two-pages") },
                    ));
                    els.push(tip(
                        widget::button::icon(icon::from_name("zoom-in-symbolic"))
                            .selected(self.zoom_active)
                            .on_press(Message::ToggleZoom),
                        if self.zoom_active {
                            fl!("stop-zooming")
                        } else {
                            fl!("zoom-tip", modifier = MODIFIER_KEY)
                        },
                    ));
                    els.push(tip(
                        widget::button::icon(library_view::fit_width_icon())
                            .selected(self.fit_width)
                            .on_press(Message::ToggleFitWidth),
                        if self.fit_width { fl!("whole-page-tip") } else { fl!("fit-width-tip") },
                    ));
                    els.push(tip(
                        widget::button::icon(icon::from_name("edit-copy-symbolic")).on_press(Message::CopyPage),
                        fl!("copy-page"),
                    ));
                    els.push(widget::divider::vertical::light().into());
                }

                els.extend(self.reader_common_controls());
            }
        }
        els
    }

    fn on_close_requested(&self, _id: window::Id) -> Option<Self::Message> {
        Some(Message::Jf(JfMsg::Closing))
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let events = event::listen_with(|event, status, _| match event {
            event::Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => match status {
                event::Status::Ignored => Some(Message::Key(key)),
                event::Status::Captured => None,
            },
            event::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
                Some(Message::ModifiersChanged(m))
            }
            // Mouse wheel, Ctrl+scroll and trackpad pinch: while zoom mode
            // is already active, `widget::image::viewer` owns and captures
            // these itself. While inactive, the first tick just bootstraps
            // into zoom mode (see `App::enter_zoom`); the viewer then takes
            // over for the rest of the gesture. A trackpad's plain two-finger
            // scroll (no Ctrl) is filtered in `update` so it never
            // bootstraps zoom.
            event::Event::Mouse(mouse::Event::WheelScrolled { delta }) => match status {
                event::Status::Ignored => Some(Message::ZoomBootstrapWheel(delta)),
                event::Status::Captured => None,
            },
            event::Event::Mouse(mouse::Event::WheelZoomed { .. }) => match status {
                event::Status::Ignored => Some(Message::ZoomBootstrap),
                event::Status::Captured => None,
            },
            event::Event::Touch(t) => Some(Message::Touch(t)),
            // The window icon can only be set once the window exists.
            event::Event::Window(window::Event::Opened { .. }) => Some(Message::WindowOpened),
            event::Event::Window(window::Event::FileDropped(paths)) => {
                Some(Message::FilesDropped(paths))
            }
            _ => None,
        });
        // In full screen, follow the pointer to reveal the hidden header.
        let pointer = if self.fullscreen && self.app_view == AppView::Reader {
            event::listen_with(|event, _, _| match event {
                // Only when it would show or hide the header: a mouse
                // reports hundreds of moves a second, and each message
                // redraws the reader.
                event::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    let show = if position.y <= 4.0 {
                        true
                    } else if position.y > 72.0 {
                        false
                    } else {
                        return None;
                    };
                    (HEADER_SHOWN.load(Ordering::Relaxed) != show).then_some(Message::PointerY(position.y))
                }
                _ => None,
            })
        } else {
            Subscription::none()
        };
        // Outside COSMIC (which pushes its theme to us), look for a changed
        // dark mode or accent colour now and then.
        let system_colors = if self.settings.theme.is_none() && self.look() != Look::Cosmic {
            cosmic::iced::time::every(std::time::Duration::from_secs(5)).map(|_| Message::CheckSystemColors)
        } else {
            Subscription::none()
        };
        // The full-screen clock: often enough that the minute is never far
        // behind.
        let status = if self.status_shown() {
            cosmic::iced::time::every(std::time::Duration::from_secs(10)).map(|_| Message::StatusTick)
        } else {
            Subscription::none()
        };
        Subscription::batch([events, self.jellyfin_subscription(), self.downloads_subscription(), pointer, system_colors, status])
    }

    #[allow(clippy::too_many_lines)]
    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        let task = self.handle_message(message);
        // libcosmic pads the content area with the window colour, which
        // shows as a strip beside the black in theater mode; drop that frame
        // while reading a comic in theater mode.
        self.core.window.content_container =
            !(self.app_view == AppView::Reader && self.theater_mode && self.epub_book.is_none());
        // Reading in full screen hides the header bar so the page fills the
        // screen; it comes back while the pointer is at the top edge or a
        // drawer is open.
        if !self.fullscreen || self.app_view != AppView::Reader {
            self.fullscreen_header = false;
        }
        HEADER_SHOWN.store(self.fullscreen_header, Ordering::Relaxed);
        self.core.window.show_headerbar = !self.fullscreen
            || self.app_view != AppView::Reader
            || self.fullscreen_header
            || self.core.window.show_context;
        Task::batch([task, self.sync_window_icon(), self.jellyfin_tick()])
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let content = match self.app_view {
            AppView::Library => self.view_library(),
            AppView::Series => self.view_series(),
            AppView::Reader => self.view_reader(),
            AppView::Downloads => self.view_downloads(),
            AppView::Catalog => self.view_catalog(),
        };

        // While the settings or details drawer is open, a click anywhere
        // outside it closes it (and doesn't also reach what's underneath).
        let content: Element<'_, Message> = if self.core.window.show_context {
            cosmic::iced::widget::stack![
                content,
                // The interaction makes the stack hide the cursor from the
                // content below; on_press alone would let the click through.
                mouse_area(widget::Space::new().width(Length::Fill).height(Length::Fill))
                    .on_press(Message::CloseContext)
                    .interaction(cosmic::iced::mouse::Interaction::Idle),
            ]
            .into()
        } else {
            content
        };

        // The time and battery in the top-right corner while reading in full
        // screen, like an e-reader; hidden while the header bar is showing.
        let content: Element<'_, Message> = match self.status_overlay() {
            Some(status) if !self.fullscreen_header && !self.core.window.show_context => {
                cosmic::iced::widget::stack![
                    content,
                    widget::container(status).width(Length::Fill).align_right(Length::Fill).padding([10, 14]),
                ]
                .into()
            }
            _ => content,
        };

        widget::dnd_destination(content, vec![Cow::Borrowed("text/uri-list")])
            .on_finish(|mime, data, _action, _x, _y| Message::DndDataReceived(mime, data))
            .on_data_received(Message::DndDataReceived)
            .into()
    }
}

// ── Key handling ──────────────────────────────────────────────────────────────

impl App {
    fn handle_key(&mut self, key: Key) -> Task<Message> {
        if self.app_view == AppView::Reader && self.epub_book.is_some() {
            if self.book_image.is_some() {
                if matches!(key, Key::Named(Named::Escape | Named::Backspace)) {
                    self.book_image = None;
                }
                return Task::none();
            }
            // Cmd on a Mac, Ctrl elsewhere.
            let ctrl = self.modifiers.command();
            let shift = self.modifiers.shift();
            let msg = match &key {
                Key::Character(c) if c.as_str() == " " && shift => Some(BookMsg::PageUp),
                Key::Character(c) if c.as_str() == " " => Some(BookMsg::PageDown),
                Key::Named(Named::ArrowRight | Named::PageDown) => Some(BookMsg::PageDown),
                Key::Named(Named::ArrowLeft | Named::PageUp) => Some(BookMsg::PageUp),
                Key::Named(Named::ArrowDown) => Some(BookMsg::LineDown),
                Key::Named(Named::ArrowUp) => Some(BookMsg::LineUp),
                Key::Character(c) if ctrl && matches!(c.as_str(), "=" | "+") => Some(BookMsg::FontBigger),
                Key::Character(c) if ctrl && c.as_str() == "-" => Some(BookMsg::FontSmaller),
                Key::Character(c) if ctrl && c.as_str() == "0" => Some(BookMsg::FontReset),
                _ => None,
            };
            if let Some(msg) = msg {
                return self.update_book(msg);
            }
            match &key {
                Key::Character(c) if !ctrl && matches!(c.as_str(), "c" | "C") => {
                    self.show_chapter_select = !self.show_chapter_select;
                    return Task::none();
                }
                Key::Character(c) if !ctrl && matches!(c.as_str(), "a" | "A") => {
                    return self.update_book(BookMsg::ToggleTypography);
                }
                _ => {}
            }
        }
        match &key {
            Key::Named(Named::ArrowRight) | Key::Named(Named::PageDown) => {
                if self.app_view != AppView::Reader { return Task::none(); }
                return self.update(Message::NextPage);
            }
            Key::Named(Named::ArrowLeft) | Key::Named(Named::PageUp) => {
                if self.app_view != AppView::Reader { return Task::none(); }
                return self.update(Message::PrevPage);
            }
            // Comics only: in a book Space turns the page (above).
            Key::Character(c)
                if c.as_str() == " " && self.app_view == AppView::Reader && !self.sources.is_empty() && !self.modifiers.command() =>
            {
                return self.update(Message::ToggleFitWidth);
            }
            Key::Named(Named::Escape) => {
                if self.fullscreen {
                    return self.update(Message::ToggleFullscreen);
                }
                if self.fit_width { return self.update(Message::ToggleFitWidth); }
                if self.zoom_active { self.zoom_active = false; self.zoom = 1.0; }
                else if self.show_chapter_select { self.show_chapter_select = false; }
                else if self.app_view != AppView::Library { return self.update(Message::GoToLibrary); }
            }
            Key::Character(c)
                if self.app_view == AppView::Reader
                    && self.modifiers.command()
                    && c.as_str() == "0" =>
            {
                self.zoom_active = false;
                self.fit_width = false;
                self.zoom = 1.0;
            }
            // Single-letter shortcuts, unless Cmd/Ctrl is held (Cmd+C on a
            // Mac copies, it doesn't open the chapter list).
            Key::Character(c) if self.app_view == AppView::Reader && !self.modifiers.command() => match c.as_str() {
                "f" | "F" => return self.update(Message::ToggleFullscreen),
                "t" | "T" => self.theater_mode = !self.theater_mode,
                "l" | "L" => {
                    self.layout = match self.layout { Layout::Single => Layout::Dual, Layout::Dual => Layout::Single };
                    if self.layout == Layout::Dual && self.current_page % 2 != 0 && !self.is_spread(self.current_page - 1) {
                        self.current_page = self.current_page.saturating_sub(1);
                    }
                    return self.refresh_window();
                }
                "m" | "M" => return self.update(Message::ToggleZoom),
                "i" | "I" => return self.update(Message::ToggleInfo),
                "s" | "S" => return self.update(Message::ToggleSettings),
                "c" | "C" if self.chapters.len() > 1 || self.reader_issues.len() > 1 => {
                    return self.toggle_chapter_list();
                }
                "p" | "P" => return self.update(Message::CopyPage),
                _ => {}
            },
            _ => {}
        }
        Task::none()
    }
}

// ── Reader view ───────────────────────────────────────────────────────────────

impl App {
    fn view_reader(&self) -> Element<'_, Message> {
        if self.epub_book.is_some() {
            return self.view_epub_reader();
        }

        // ── Page content ──────────────────────────────────────────────────────
        let viewer: Element<'_, Message> =
            if !self.sources.is_empty() && self.handle(self.current_page).is_some() {
                let use_viewer = self.zoom_active || self.fit_width || self.fit_width_leaving;
                match (&self.layout, use_viewer) {
                    (_, true) => {
                        let h = self.handle(self.current_page).unwrap();
                        widget::image::viewer(h.clone())
                            .fit_width(self.fit_width)
                            .content_fit(ContentFit::Contain)
                            .min_scale(MIN_ZOOM)
                            .max_scale(MAX_ZOOM)
                            .on_zoom(Message::ZoomChanged)
                            // Keep the zoom across page turns but start
                            // each new page at its top.
                            .page(self.current_page as u64)
                            // A spread is shown whole, then the zoom
                            // comes back on the next page.
                            .whole_page(self.is_spread(self.current_page))
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .into()
                    }
                    (Layout::Single, false) => widget::image(
                        self.handle(self.current_page).unwrap().clone(),
                    )
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .content_fit(ContentFit::Contain)
                    .into(),
                    (Layout::Dual, false) => {
                        let mut row = widget::Row::new().spacing(4).height(Length::Fill);
                        if let Some(h) = self.handle(self.current_page) {
                            row = row.push(
                                widget::image(h.clone())
                                    .width(Length::FillPortion(1))
                                    .height(Length::Fill)
                                    .content_fit(ContentFit::Contain),
                            );
                        }
                        // A spread (or the page before one) is shown alone,
                        // centred.
                        let right = self.current_page + 1;
                        if self.pages_shown_at(self.current_page) == 2 {
                            if let Some(h) = self.handle(right) {
                                row = row.push(
                                    widget::image(h.clone())
                                        .width(Length::FillPortion(1))
                                        .height(Length::Fill)
                                        .content_fit(ContentFit::Contain),
                                );
                            } else {
                                row = row.push(
                                    widget::container(widget::indeterminate_circular())
                                        .width(Length::FillPortion(1))
                                        .height(Length::Fill)
                                        .center_x(Length::Fill)
                                        .center_y(Length::Fill),
                                );
                            }
                        }
                        row.into()
                    }
                }
            } else if !self.sources.is_empty() {
                // A page is still decoding — show the nearest already-decoded
                // page dimmed underneath the spinner instead of a blank
                // screen, so something is visible immediately while the
                // real page loads in.
                let spinner = widget::container(
                    widget::Column::new()
                        .spacing(12)
                        .align_x(Alignment::Center)
                        .push(widget::indeterminate_circular())
                        .push(widget::text(fl!("decoding-page"))),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill);

                match self.nearest_cached_handle() {
                    Some(h) => cosmic::iced::widget::stack![
                        widget::image(h.clone())
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .content_fit(ContentFit::Contain)
                            .opacity(0.35),
                        spinner,
                    ]
                    .into(),
                    None => spinner.into(),
                }
            } else if let Some(prompt) = self.big_fetch_prompt(true) {
                prompt
            } else if self.loading {
                widget::Column::new()
                    .spacing(12)
                    .align_x(Alignment::Center)
                    .push(widget::indeterminate_circular())
                    .push(widget::text(if self.creating_index { fl!("creating-index") } else { fl!("loading-comic") }))
                    .into()
            } else {
                widget::Column::new()
                    .spacing(12)
                    .align_x(Alignment::Center)
                    .push(icon::from_name("image-x-generic-symbolic").size(64))
                    .push(widget::text::title3(fl!("start-reading")))
                    .push(widget::text(fl!("reader-keys", modifier = MODIFIER_KEY)).size(12))
                    .push(widget::text(fl!("drop-hint")).size(12))
                    .push(widget::button::suggested(fl!("open-comic-or-book")).on_press(Message::OpenFile))
                    .into()
            };

        // ── Backdrop (never tints the page itself) ────────────────────────────
        let viewer_bg: Element<'_, Message> = match &self.fit_note {
            Some((note, _)) if !self.sources.is_empty() => {
                cosmic::iced::widget::stack![self.backdrop(viewer), fit_note(note)].into()
            }
            _ => self.backdrop(viewer),
        };

        // ── Right panel (chapter select; Info lives in the context drawer) ────
        let main_row: Element<'_, Message> = if self.show_chapter_select {
            widget::Row::new()
                .push(
                    widget::container(viewer_bg)
                        .width(Length::Fill)
                        .height(Length::Fill),
                )
                .push(self.build_chapter_select())
                .into()
        } else {
            viewer_bg
        };

        let mut col = widget::Column::new().spacing(8);
        if let Some(err) = self.error.as_deref() {
            col = col.push(error_banner(err));
        }
        if let Some(banner) = self.big_fetch_prompt(false) {
            col = col.push(banner);
        }
        // In theater mode libcosmic's content frame is off (see `update`),
        // so nothing else paints behind the reader; without this, anything
        // not on the backdrop (the error banner, the gap below it) would
        // show the window through.
        widget::container(col.push(main_row))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|theme: &cosmic::Theme| widget::container::Style {
                background: Some(Background::Color(theme.cosmic().bg_color().into())),
                ..Default::default()
            })
            .into()
    }

    /// Wraps reader content in the configured backdrop. Only the area
    /// *around* the page is tinted — the page image itself is a child of
    /// this container and is never affected by the opacity setting.
    fn backdrop<'a>(&self, content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
        let black = self.theater_mode || self.settings.background == BgStyle::Black;
        // Theater mode is always fully black.
        let alpha = if self.theater_mode { 1.0 } else { self.settings.background_opacity.clamp(0.0, 1.0) };

        widget::container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(move |theme: &cosmic::Theme| {
                let base = if black {
                    Color::BLACK
                } else {
                    theme.cosmic().bg_color().into()
                };
                widget::container::Style {
                    background: Some(Background::Color(Color { a: alpha, ..base })),
                    ..Default::default()
                }
            })
            .into()
    }

    /// Controls shared by both reader modes, kept to compact icon buttons —
    /// everything with a persistent preference now lives in the settings
    /// drawer instead of the header bar.
    fn reader_common_controls(&self) -> Vec<Element<'_, Message>> {
        // From Jellyfin: download what's open for reading offline.
        let download = self
            .current_jellyfin_item()
            .map(|(issue, series)| self.download_button(&issue, &series, widget::tooltip::Position::Bottom));
        download.into_iter().chain([
            tip(
                widget::button::icon(icon::from_name("zoom-fit-best-symbolic")).on_press(Message::FitPage),
                fl!("fit-page"),
            ),
            tip(
                widget::button::icon(icon::from_name("dialog-information-symbolic")).on_press(Message::ToggleInfo),
                fl!("details-tip"),
            ),
            fullscreen_button(self.fullscreen),
            tip(
                widget::button::icon(icon::from_name("emblem-system-symbolic")).on_press(Message::ToggleSettings),
                fl!("settings-tip"),
            ),
            widget::divider::vertical::light().into(),
            tip(
                widget::button::icon(icon::from_name("document-open-symbolic")).on_press(Message::OpenFile),
                fl!("open-a-file"),
            ),
        ])
        .collect()
    }

    fn build_settings_panel(&self) -> Element<'_, Message> {
        let s = &self.settings;

        let appearance = widget::settings::section()
            .title(fl!("appearance"))
            .add(widget::settings::item(
                fl!("language"),
                widget::dropdown(
                    &self.labels.languages[..],
                    Some(match &s.language {
                        None => 0,
                        Some(id) => self.languages.iter().position(|l| &l.id == id).map_or(0, |i| i + 1),
                    }),
                    Message::SetLanguage,
                ),
            ))
            .add(widget::settings::item(
                fl!("style"),
                widget::dropdown(
                    &self.labels.look[..],
                    Look::ALL.iter().position(|l| *l == s.look),
                    Message::SetLook,
                ),
            ))
            .add_maybe((self.look().native_titlebar() != self.native_titlebar).then(|| {
                widget::text::caption(fl!("title-bar-restart"))
            }))
            .add(widget::settings::item(
                fl!("colors"),
                widget::dropdown(
                    &self.theme_labels[..],
                    Some(match &s.theme {
                        None => 0,
                        Some(id) => self.themes.iter().position(|t| &t.id == id).map_or(0, |i| i + 1),
                    }),
                    Message::SetTheme,
                ),
            ))
            .add(
                widget::column::with_capacity(3)
                    .spacing(8)
                    .push(widget::text::caption(fl!("themes-hint")))
                    .push(
                        widget::row::with_capacity(2)
                            .spacing(8)
                            .push(widget::button::standard(fl!("import-theme")).on_press(Message::ImportTheme))
                            .push(widget::button::standard(fl!("open-themes-folder")).on_press(Message::OpenThemesFolder)),
                    )
                    .push_maybe(self.theme_error.as_deref().map(|e| widget::text::caption(fl!("theme-error", error = e)))),
            )
            // COSMIC Settings already sets roundness for the COSMIC style.
            .add_maybe((self.look() != Look::Cosmic).then(|| {
                widget::settings::item(
                    fl!("buttons"),
                    widget::dropdown(
                        &self.labels.corners[..],
                        Corners::ALL.iter().position(|c| *c == s.corners),
                        Message::SetCorners,
                    ),
                )
            }))
            .add(widget::settings::item(
                fl!("background"),
                widget::dropdown(
                    &self.labels.background[..],
                    BgStyle::ALL.iter().position(|b| *b == s.background),
                    Message::SetBackground,
                ),
            ))
            .add(widget::settings::item(
                fl!("background-opacity", percent = format!("{:.0}", s.background_opacity * 100.0)),
                widget::slider(0.0..=1.0, s.background_opacity, Message::SetBackgroundOpacity)
                    .step(0.05)
                    .width(Length::Fixed(180.0)),
            ))
            .add(widget::settings::item(
                fl!("theater-mode"),
                widget::toggler(self.theater_mode).on_toggle(|_| Message::ToggleTheater),
            ));

        let reading = widget::settings::section()
            .title(fl!("reading"))
            .add(widget::settings::item(
                fl!("two-page-spread"),
                widget::toggler(self.layout == Layout::Dual)
                    .on_toggle(|_| Message::ToggleLayout),
            ))
            .add(widget::settings::item(
                fl!("fullscreen-status"),
                widget::toggler(s.fullscreen_status).on_toggle(Message::SetFullscreenStatus),
            ));

        let has_key = comicvine::api_key().is_some();
        let metadata_section = widget::settings::section()
            .title(fl!("metadata"))
            .add(widget::settings::item(
                fl!("fetch-metadata"),
                widget::toggler(s.fetch_metadata).on_toggle(Message::SetFetchMetadata),
            ))
            .add(
                widget::Column::new()
                    .spacing(6)
                    .push(widget::text::body(if has_key {
                        fl!("comicvine-key-set")
                    } else {
                        fl!("comicvine-key-unset")
                    }))
                    .push(
                        widget::Row::new()
                            .spacing(8)
                            .align_y(Alignment::Center)
                            .push(
                                widget::secure_input(
                                    if has_key { fl!("replace-key") } else { fl!("paste-key") },
                                    &self.cv_key_input,
                                    None,
                                    true,
                                )
                                .on_input(Message::ComicVineKeyInput)
                                .on_submit(|_| Message::SaveComicVineKey)
                                .width(Length::Fill),
                            )
                            .push(widget::button::standard(fl!("save")).on_press(Message::SaveComicVineKey)),
                    ),
            )
            .add(
                widget::Row::new()
                    .spacing(8)
                    .push(widget::button::standard(fl!("rescan-library")).on_press(Message::RescanLibrary))
                    .push(widget::button::standard(fl!("refresh-metadata")).on_press(Message::RefreshAllMetadata)),
            );

        let mut library_section = widget::settings::section().title(fl!("library-folders"));
        if self.library_dirs.is_empty() {
            library_section = library_section.add(
                widget::text(fl!("no-library-folders"))
                    .size(12),
            );
        } else {
            for dir in &self.library_dirs {
                let mut name = dir.to_string_lossy().into_owned();
                if self.missing_roots.contains(dir) {
                    name = fl!("folder-not-found", folder = name);
                }
                library_section = library_section.add(
                    widget::Row::new()
                        .spacing(8)
                        .align_y(Alignment::Center)
                        .push(widget::text(name).size(12).width(Length::Fill))
                        .push(
                            widget::button::icon(icon::from_name("user-trash-symbolic"))
                                .on_press(Message::RemoveLibraryFolder(dir.clone())),
                        ),
                );
            }
        }

        let add_folder_row = widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(
                widget::text_input(fl!("folder-path-placeholder"), &self.library_path_input)
                    .on_input(Message::LibraryPathInputChanged)
                    .on_submit(|_| Message::LibraryPathInputSubmitted)
                    .width(Length::Fill),
            )
            .push(
                widget::button::standard(fl!("add"))
                    .on_press(Message::LibraryPathInputSubmitted),
            )
            .push(
                widget::button::standard(fl!("browse"))
                    .on_press(Message::AddLibraryFolder),
            );

        widget::settings::view_column(vec![
            appearance.into(),
            reading.into(),
            metadata_section.into(),
            library_section.into(),
            add_folder_row.into(),
            self.build_jellyfin_section(),
            self.build_opds_section(),
            self.build_about_section(),
        ])
        .into()
    }

    fn check_for_updates(&mut self) -> Task<Message> {
        self.update = UpdateStatus::Checking;
        cosmic::task::future(async { Message::UpdateChecked(crate::updates::check().await) })
    }

    /// Settings → About: version and build, who made it, and updates.
    fn build_about_section(&self) -> Element<'_, Message> {
        use crate::updates::{self, Install};
        let install = Install::detect();
        let version = match updates::build_commit() {
            Some(commit) => fl!("version-build", version = updates::VERSION, commit = commit),
            None => fl!("version", version = updates::VERSION),
        };

        let mut section = widget::settings::section()
            .title(fl!("about"))
            .add(
                widget::column::with_capacity(3)
                    .spacing(2)
                    .push(widget::text::title4("Nebula Reader"))
                    .push(widget::text::body(version))
                    .push(widget::text::caption(fl!("made-by", install = install.label()))),
            )
            .add(
                widget::row::with_capacity(2)
                    .spacing(8)
                    .push(widget::button::standard("GitHub").on_press(Message::OpenUrl(updates::REPO_URL.into())))
                    .push(
                        widget::button::standard(fl!("release-notes"))
                            .on_press(Message::OpenUrl(updates::RELEASES_URL.into())),
                    ),
            );

        if !install.checks_github() {
            return section.add(widget::text::caption(install.how_to_update())).into();
        }

        let status: Element<'_, Message> = match &self.update {
            UpdateStatus::NotChecked => widget::text::body(fl!("update-not-checked")).into(),
            UpdateStatus::Checking => widget::text::body(fl!("update-checking")).into(),
            UpdateStatus::UpToDate => widget::text::body(fl!("update-up-to-date")).into(),
            UpdateStatus::Failed(e) => widget::text::body(fl!("update-check-failed", error = e)).into(),
            UpdateStatus::Available(release) => {
                let (msg, action) = self.update_message(release);
                widget::column::with_capacity(4)
                    .spacing(6)
                    .push(widget::text::heading(fl!("update-available-heading", version = release.version.as_str())))
                    .push_maybe((!matches!(self.update_job, UpdateJob::Idle)).then(|| widget::text::body(msg)))
                    .push(action)
                    .push(widget::text::caption(install.how_to_update()))
                    .into()
            }
        };
        section = section
            .add(
                widget::row::with_capacity(2)
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(widget::container(status).width(Length::Fill))
                    .push(
                        widget::button::standard(fl!("check-now")).on_press_maybe(
                            (!matches!(self.update, UpdateStatus::Checking)).then_some(Message::CheckForUpdates),
                        ),
                    ),
            )
            .add(widget::settings::item(
                fl!("check-on-start"),
                widget::toggler(self.settings.check_for_updates).on_toggle(Message::SetCheckForUpdates),
            ));
        section.into()
    }

    fn build_info_panel(&self) -> Element<'_, Message> {
        let mut col = widget::Column::new().spacing(8).padding(12);

        let current = self.current_chapter();
        if let Some((ch_idx, ch, pg)) = current {
            let issue = self.reader_issues.iter().find(|i| i.path == ch.path.to_string_lossy());
            let heading = match issue {
                Some(i) if !i.number.is_empty() => {
                    let h = i.heading();
                    if h.is_empty() { fl!("issue-number", number = i.number.as_str()) } else { fl!("issue-heading", number = i.number.as_str(), title = h) }
                }
                _ if self.chapters.len() > 1 => fl!("chapter-heading", chapter = ch_idx + 1, name = ch.name.as_str()),
                _ => ch.name.clone(),
            };
            col = col.push(widget::text::title4(heading));
            col = col.push(widget::text::body(fl!("page-of", page = pg + 1, pages = ch.page_count)));
            if let Some(i) = issue {
                if !i.date().is_empty() {
                    col = col.push(widget::text::caption(fl!("released", date = library::pretty_date(i.date()))));
                }
                for line in i.creators.lines() {
                    col = col.push(widget::text::caption(line.to_string()));
                }
                if self.chapters.len() > 1 {
                    col = col.push(widget::text::caption(fl!(
                        "issue-of-series",
                        issue = ch_idx + 1,
                        issues = self.chapters.len()
                    )));
                }
                if !i.url.is_empty() {
                    col = col.push(widget::button::link(fl!("view-issue-page")).on_press(Message::OpenUrl(i.url.clone())));
                }
            }
            col = col.push(widget::divider::horizontal::light());
        }

        if let Some(e) = &self.reader_entry {
            col = col.push(widget::text::title4(e.name().to_string()));
            let mut facts = Vec::new();
            if !e.publisher.is_empty() {
                facts.push(e.publisher.clone());
            }
            if let Some(y) = e.year {
                facts.push(y.to_string());
            }
            if e.issue_count > 1 {
                facts.push(fl!("issues-read", read = e.issues_read, total = e.issue_count));
            }
            if !facts.is_empty() {
                col = col.push(widget::text::body(facts.join(" · ")));
            }
            let source = match e.source.as_str() {
                "comicvine" => fl!("source-comicvine"),
                "anilist" => fl!("source-anilist"),
                _ if e.meta_status == "nomatch" => fl!("no-online-match"),
                _ if e.meta_status.starts_with("error") => e.meta_status.clone(),
                _ => fl!("not-looked-up"),
            };
            col = col.push(widget::text::caption(source));
            if !e.genres.is_empty() {
                col = col.push(widget::text::caption(e.genres.clone()));
            }
            if !e.description.is_empty() {
                col = col.push(widget::text::body(e.description.clone()));
            }
            col = col.push(widget::divider::horizontal::light());
        }

        // ── Embedded ComicInfo.xml, when present (free, offline, authoritative) ─
        if let Some(info) = self.comic_info.as_ref().filter(|i| !i.is_empty()) {
            col = col.push(widget::text::title4("ComicInfo.xml"));
            for (label, value) in [
                (fl!("ci-series"), info.series.clone()),
                (fl!("ci-number"), info.number.clone()),
                (fl!("ci-volume"), info.volume.clone()),
                (fl!("ci-year"), info.year.map(|y| y.to_string())),
                (fl!("ci-publisher"), info.publisher.clone()),
                (fl!("ci-writer"), info.writer.clone()),
                (fl!("ci-artist"), info.penciller.clone()),
                (fl!("ci-inker"), info.inker.clone()),
                (fl!("ci-colorist"), info.colorist.clone()),
                (fl!("ci-letterer"), info.letterer.clone()),
                (fl!("ci-cover"), info.cover_artist.clone()),
                (fl!("ci-genre"), info.genre.clone()),
            ] {
                if let Some(v) = value {
                    col = col.push(widget::text::caption(fl!("ci-field", label = label, value = v)));
                }
            }
            if let Some(summary) = &info.summary {
                col = col.push(widget::text::body(summary.clone()));
            }
            col = col.push(widget::divider::horizontal::light());
        }

        // ── What the name parser made of this file (for fixing matches) ──
        if let Some((_, ch, _)) = current {
            let file = ch.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let parsed = naming::parse_issue(&file);
            let clean = naming::series_name_from_issue(&file);
            col = col.push(widget::text::heading(fl!("name-parsing")));
            col = col.push(widget::text::caption(file));
            col = col.push(widget::text::caption(fl!(
                "name-parsed",
                series = clean.search.as_str(),
                issue = parsed.number.as_deref().unwrap_or("—"),
                year = clean.year.map(|y| y.to_string()).unwrap_or_else(|| "—".into())
            )));
        }

        widget::container(widget::scrollable(col))
            .width(Length::Fixed(320.0))
            .height(Length::Fill)
            .into()
    }
}

// ── Standalone helpers ────────────────────────────────────────────────────────

/// Parses a `text/uri-list` payload (as delivered by drag-and-drop) and
/// returns the first `file://` entry as a local path.
fn first_path_from_uri_list(data: &[u8]) -> Option<PathBuf> {
    let text = String::from_utf8_lossy(data);
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .and_then(|line| Url::parse(line).ok())
        .and_then(|url| url.to_file_path().ok())
}

impl App {
    /// The library's "new version available" and "updated" banners.
    pub(super) fn version_banners(&self) -> Vec<Element<'_, Message>> {
        use crate::updates;
        let mut banners = Vec::new();
        if let UpdateStatus::Available(release) = &self.update {
            // A started update stays visible until it's done.
            let busy = !matches!(self.update_job, UpdateJob::Idle);
            if busy || self.settings.dismissed_update.as_deref() != Some(release.version.as_str()) {
                let (msg, action) = self.update_message(release);
                banners.push(notice_banner(
                    "software-update-available-symbolic",
                    msg,
                    action,
                    (!matches!(self.update_job, UpdateJob::Downloading { .. })).then_some(Message::DismissUpdateBanner),
                ));
            }
        }
        if self.just_updated {
            banners.push(notice_banner(
                "emblem-ok-symbolic",
                fl!("updated-banner", version = updates::VERSION),
                widget::button::standard(fl!("whats-new"))
                    .on_press(Message::OpenUrl(format!("{}/tag/v{}", updates::RELEASES_URL, updates::VERSION))),
                Some(Message::DismissUpdatedBanner),
            ));
        }
        banners
    }

    /// What the update banner and Settings → About say about `release`, and
    /// the button next to it: Update now where this install can update
    /// itself, otherwise the release page.
    fn update_message(&self, release: &crate::updates::Release) -> (String, Element<'_, Message>) {
        let install = crate::updates::Install::detect();
        let in_app = crate::updater::Plan::for_release(install, release).is_some();
        let release_page = || -> Element<'_, Message> {
            widget::button::standard(fl!("release-page")).on_press(Message::OpenUrl(release.url.clone())).into()
        };
        match &self.update_job {
            UpdateJob::Idle if in_app => (
                fl!("update-available", version = release.version.as_str()),
                widget::button::suggested(fl!("update-now")).on_press(Message::UpdateNow).into(),
            ),
            UpdateJob::Idle => {
                (fl!("update-available-how", version = release.version.as_str(), how = install.how_to_update()), release_page())
            }
            UpdateJob::Downloading { done, total } => {
                let msg = match total {
                    Some(total) if *total > 0 => fl!(
                        "update-downloading-percent",
                        version = release.version.as_str(),
                        percent = done * 100 / total
                    ),
                    _ => fl!(
                        "update-downloading-mb",
                        version = release.version.as_str(),
                        mb = format!("{:.1}", *done as f64 / 1e6)
                    ),
                };
                let fraction = total.filter(|t| *t > 0).map_or(0.0, |t| *done as f32 / t as f32);
                (msg, widget::container(library_view::progress_bar(fraction)).width(Length::Fixed(140.0)).into())
            }
            UpdateJob::Ready(_) => (
                fl!("update-ready", version = release.version.as_str()),
                widget::button::suggested(fl!("restart-now")).on_press(Message::RestartToUpdate).into(),
            ),
            UpdateJob::Failed(e) => (
                fl!("update-failed", error = e.as_str()),
                widget::row::with_capacity(2)
                    .spacing(8)
                    .push(widget::button::standard(fl!("try-again")).on_press(Message::UpdateNow))
                    .push(release_page())
                    .into(),
            ),
        }
    }
}

fn notice_banner<'a>(
    icon_name: &'static str,
    msg: String,
    action: impl Into<Element<'a, Message>>,
    on_close: Option<Message>,
) -> Element<'a, Message> {
    widget::container(
        widget::Row::new()
            .spacing(12)
            .align_y(Alignment::Center)
            .push(icon::from_name(icon_name).size(20))
            .push(widget::text::body(msg).width(Length::Fill))
            .push(action)
            .push_maybe(on_close.map(|m| widget::button::icon(icon::from_name("window-close-symbolic")).on_press(m))),
    )
    .padding([8, 12])
    .width(Length::Fill)
    .class(cosmic::theme::Container::Card)
    .into()
}

pub(super) fn error_banner(msg: &str) -> Element<'_, Message> {
    widget::container(
        widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(widget::text(msg).width(Length::Fill))
            .push(
                widget::button::icon(icon::from_name("window-close-symbolic"))
                    .on_press(Message::CloseError),
            ),
    )
    .padding(8)
    .width(Length::Fill)
    .into()
}

impl App {
    fn handle_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::GoToLibrary => {
                self.show_chapter_select = false;
                // Reading a Jellyfin CBR built its page index.
                let reindexed = if self.app_view == AppView::Reader && self.jellyfin_source().is_some() {
                    self.refresh_unindexed()
                } else {
                    Task::none()
                };
                // Leaving the reader also leaves full screen, so the library
                // is a normal window again.
                let windowed = match self.core.main_window_id() {
                    Some(id) if self.fullscreen => {
                        self.fullscreen = false;
                        window::set_mode(id, window::Mode::Windowed)
                    }
                    _ => Task::none(),
                };
                let windowed = Task::batch([windowed, reindexed]);
                if self.app_view == AppView::Reader && self.epub_book.is_some() {
                    self.persist_book_position();
                }
                if self.app_view == AppView::Reader && self.back_to_series {
                    if let Some(key) = self.open_path.as_ref().map(|p| p.to_string_lossy().into_owned()) {
                        let t = self.refresh_views();
                        return Task::batch([windowed, t, self.open_series_page(key)]);
                    }
                }
                self.app_view = AppView::Library;
                self.series_page = None;
                self.set_header_title("Nebula".into());
                return Task::batch([windowed, self.refresh_views()]);
            }
            Message::OpenSeriesPage(key) => {
                if self.context_page == ContextPage::Properties {
                    self.set_show_context(false);
                }
                return self.open_series_page(key);
            }
            Message::OpenFile => {
                return cosmic::task::future(async move {
                    let filter = comic_file_filter();
                    let dialog = file_chooser::open::Dialog::new()
                        .title(fl!("open-comic-or-book")).filter(filter);
                    match dialog.open_file().await {
                        Ok(r) => Message::PathSelected(r.url().to_owned()),
                        Err(file_chooser::Error::Cancelled) => Message::Cancelled,
                        Err(e) => Message::LoadFailed(e.to_string()),
                    }
                });
            }
            Message::OpenFolder => {
                return cosmic::task::future(async move {
                    let dialog = file_chooser::open::Dialog::new().title("Open Comic Series");
                    match dialog.open_folder().await {
                        Ok(r) => Message::PathSelected(r.url().to_owned()),
                        Err(file_chooser::Error::Cancelled) => Message::Cancelled,
                        Err(e) => Message::LoadFailed(e.to_string()),
                    }
                });
            }
            Message::PathSelected(url) => {
                let path = match url.scheme() {
                    "file" => match url.to_file_path() {
                        Ok(p) => p,
                        Err(()) => {
                            self.error = Some(fl!("invalid-file-path", url = url.to_string()));
                            return Task::none();
                        }
                    },
                    other => {
                        self.error = Some(fl!("unsupported-scheme", scheme = other));
                        return Task::none();
                    }
                };
                return self.open_linked(path);
            }
            Message::OpenEntry { key, chapter } => {
                return self.open_entry(key, chapter);
            }
            Message::Loaded(path, sources, chapters) => {
                let key = path.to_string_lossy().to_string();
                let total = sources.len();
                let start_chapter = self.start_chapter.take();

                self.loading = false;
                self.sources = sources;
                self.chapters = chapters;
                self.clear_pages();
                self.chapter_covers.clear();
                self.covers_pending.clear();
                self.zoom_active = false;
                self.fit_width = false;
                self.zoom = 1.0;
                self.comic_info = None;
                self.epub_book = None;
                self.book_image = None;
                self.epub_chapter = 0;
                self.epub_cover = None;
                self.show_chapter_select = false;
                self.open_path = Some(path.clone());

                // Register (or refresh) this series and its issues — any
                // open counts, not just library folders, so one-off files
                // get tracked and a shelf cover too.
                let mut tasks = Vec::new();
                if let (Some(db), true) = (&self.db, crate::remote::is_key(&key)) {
                    // A Jellyfin series: its chapter list comes from the
                    // server (only a window of it is loaded), so just record
                    // the page counts we now know.
                    for ch in &self.chapters {
                        let _ = library::set_issue_page_count(db, &key, &ch.path.to_string_lossy(), ch.page_count);
                    }
                    let _ = library::touch_opened(db, &key);
                    self.reader_entry = library::get_series(db, &key);
                    self.reader_issues = library::issues_for(db, &key);
                } else if let Some(db) = &self.db {
                    let raw_name =
                        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let title = if path.is_dir() {
                        naming::clean_series_name(&raw_name).display
                    } else {
                        // Opened from a file manager: a bare chapter file is
                        // named after its folder, not "Ch 0 Vol none …".
                        let roots = &self.library_dirs;
                        naming::title_for_file(&path, |p| roots.iter().any(|r| r == p))
                    };
                    let is_series = path.is_dir() || self.chapters.len() > 1;
                    let _ = library::upsert_series(db, &key, &title, total, is_series);
                    let _ = library::sync_issues(db, &key, &issue_rows_from_chapters(&self.chapters));
                    let _ = library::touch_opened(db, &key);
                    self.reader_entry = library::get_series(db, &key);
                    self.reader_issues = library::issues_for(db, &key);

                    if !library::chapter_cover_path(&key, 0).exists() {
                        if let Some(source) = self.sources.first().cloned() {
                            self.covers_pending.insert(0);
                            tasks.push(cover_task(self.page_gen, 0, source, path.clone()));
                        }
                    }
                }

                self.current_page = resume_page(
                    &self.chapters,
                    &self.reader_issues,
                    self.reader_entry.as_ref(),
                    start_chapter,
                    total,
                );
                self.nav_dir = 1;

                self.title = self
                    .reader_entry
                    .as_ref()
                    .map(|e| e.name().to_string())
                    .unwrap_or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                self.set_header_title(self.title.clone());

                // Embedded ComicInfo.xml of the issue being read, looked up
                // off the UI thread: in a CBR it can mean unpacking much of
                // the archive.
                if let Some((_, ch, _)) = self.current_chapter() {
                    let (gen, path) = (self.page_gen, ch.path.clone());
                    tasks.push(cosmic::task::future(async move {
                        let info = tokio::task::spawn_blocking(move || {
                            comic::find_comic_info(&path).map(|xml| comicinfo::parse(&xml))
                        })
                        .await
                        .ok()
                        .flatten();
                        Message::ComicInfoRead(gen, info)
                    }));
                }
                self.persist_progress();

                if self.reader_entry.as_ref().is_some_and(|e| e.meta_fetched_at == 0) {
                    tasks.push(self.enqueue_meta(key));
                }
                tasks.push(self.refresh_window());
                if let Some(id) = self.core.main_window_id() {
                    tasks.push(self.set_window_title(self.title.clone(), id));
                }
                return Task::batch(tasks);
            }
            Message::EpubLoaded(path, result) => {
                self.loading = false;
                match result {
                    Ok(book) => {
                        let path_str = path.to_string_lossy().to_string();
                        self.sources.clear();
                        self.chapters.clear();
                        self.clear_pages();
                        self.chapter_covers.clear();
                        self.covers_pending.clear();
                        self.zoom_active = false;
                        self.fit_width = false;
                        self.zoom = 1.0;
                        self.comic_info = None;
                        self.start_chapter = None;
                        self.show_chapter_select = false;
                        self.open_path = Some(path.clone());
                        self.title = book.title.clone();
                        self.set_header_title(self.title.clone());

                        self.epub_cover = book
                            .cover
                            .clone()
                            .map(widget::image::Handle::from_bytes);

                        let chapter_count = book.chapters.len();
                        self.epub_images =
                            book.images.iter().map(|b| widget::image::Handle::from_bytes(b.clone())).collect();
                        // Resume at the saved chapter *and* scroll position.
                        let saved = self.db.as_ref().and_then(|db| {
                            library::issues_for(db, &path_str).into_iter().find(|i| i.path == path_str)
                        });
                        let (chapter, position) = match saved {
                            Some(i) if !i.completed => (i.last_page, i.position),
                            _ => (
                                self.db
                                    .as_ref()
                                    .and_then(|db| library::get_progress(db, &path_str))
                                    .map(|(_, ch)| ch)
                                    .unwrap_or(0),
                                0.0,
                            ),
                        };
                        self.epub_chapter = chapter.min(chapter_count.saturating_sub(1));
                        self.book_scroll = BookScroll { rel_y: position, ..BookScroll::default() };
                        self.book_saved_pos = position;
                        let cover_bytes = book.cover.clone();
                        self.epub_book = Some(book);

                        // As with comics: any opened EPUB gets tracked and a
                        // shelf cover, not just ones under a library folder.
                        if let Some(db) = &self.db {
                            let _ = library::upsert_series(db, &path_str, &self.title, chapter_count, false);
                            let _ = library::sync_issues(
                                db,
                                &path_str,
                                &[IssueRow {
                                    path: path_str.clone(),
                                    name: self.title.clone(),
                                    number: String::new(),
                                    subtitle: String::new(),
                                    page_count: Some(chapter_count),
                                    details: None,
                                }],
                            );
                            let _ = library::touch_opened(db, &path_str);
                            self.reader_entry = library::get_series(db, &path_str);
                            self.reader_issues = library::issues_for(db, &path_str);

                            let cover_path = library::chapter_cover_path(&path_str, 0);
                            if !cover_path.exists() {
                                if let Some(img) = cover_bytes.and_then(|b| image::load_from_memory(&b).ok()) {
                                    let _ = scan::save_thumb(&img.to_rgba8(), &cover_path);
                                }
                            }
                        }

                        let title_task = self.core.main_window_id()
                            .map(|id| self.set_window_title(self.title.clone(), id));

                        let restore = cosmic::iced::widget::scrollable::snap_to(
                            book_view::book_scroll_id(),
                            cosmic::iced::widget::scrollable::RelativeOffset { x: None, y: Some(position) },
                        );
                        return Task::batch(title_task.into_iter().chain([restore]));
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            Message::PageDecoded(gen, index, result) => {
                if gen != self.page_gen {
                    return Task::none();
                }
                self.pending.remove(&index);
                let is_current = index == self.current_page;
                let mut tasks = Vec::new();
                if let Ok((handle, size)) = result {
                    // Upload it now, while it isn't on screen yet.
                    if index.abs_diff(self.current_page) <= PRELOAD_FAR {
                        tasks.push(
                            cosmic::iced::runtime::image::allocate(handle.clone())
                                .map(move |a| cosmic::Action::App(Message::PageAllocated(gen, index, a.ok()))),
                        );
                    }
                    self.cache.insert(index, (handle, size));
                }
                if is_current {
                    // The freshly-uploaded texture sometimes doesn't paint on
                    // the very frame it lands in the cache, so force one more
                    // render pass shortly after.
                    tasks.push(cosmic::task::future(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        Message::Redraw
                    }));
                }
                return Task::batch(tasks);
            }
            Message::PageAllocated(gen, index, allocation) => {
                if let Some(a) = allocation.filter(|_| gen == self.page_gen && self.cache.contains_key(&index)) {
                    self.allocations.insert(index, a);
                }
            }
            Message::Redraw => {}
            Message::LoadFailed(why) => {
                self.loading = false;
                self.error = Some(why);
            }
            Message::Cancelled => { self.loading = false; }
            Message::MetaNext => {
                return self.pump_meta();
            }
            Message::MetaMatched(key, result) => {
                self.meta_running = false;
                self.meta_done += 1;
                self.meta_current = None;
                let mut used_comicvine = false;
                if let Some(db) = &self.db {
                    match result {
                        Ok(Some(m)) => {
                            used_comicvine = m.source == "comicvine";
                            let entry = library::get_series(db, &key);
                            let forced = entry.as_ref().is_some_and(|e| !e.search_override.is_empty());
                            let rename = forced || m.score >= metadata::RENAME_SCORE;
                            let display = rename.then(|| {
                                naming::with_edition(&m.title, entry.as_ref().map_or("", |e| e.title.as_str()))
                            });
                            let _ = library::apply_match(db, &key, &m, display.as_deref());
                        }
                        Ok(None) => library::mark_meta(db, &key, "nomatch", true),
                        Err(e) => {
                            log::warn!("metadata lookup for {key} failed: {e}");
                            library::set_meta_error(db, &key, &e);
                        }
                    }
                }
                let refresh = self.refresh_views();
                if self.series_page.as_ref().is_some_and(|s| s.path == key) {
                    if let Some(s) = &self.series_page {
                        self.set_header_title(s.name().to_string());
                    }
                }
                // Space requests out (ComicVine asks for ~1/s).
                let delay = if used_comicvine { 1500 } else { 400 };
                let next = cosmic::task::future(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    Message::MetaNext
                });
                return Task::batch([refresh, next]);
            }
            Message::SeriesSearchInput(text) => {
                self.series_search_input = text;
            }
            Message::RematchSeries => {
                let Some(key) = self.series_page.as_ref().map(|s| s.path.clone()) else { return Task::none() };
                if let Some(db) = &self.db {
                    library::set_search_override(db, &key, &self.series_search_input);
                    library::mark_meta(db, &key, "", false);
                }
                self.meta_attempted.remove(&key);
                // Jump the queue: this is what the user is looking at.
                self.meta_queue.retain(|k| k != &key);
                self.meta_queue.push_front(key.clone());
                self.meta_attempted.insert(key);
                let t = self.refresh_views();
                return Task::batch([t, self.pump_meta()]);
            }
            Message::ClearSeriesMatch => {
                if let (Some(db), Some(key)) = (&self.db, self.series_page.as_ref().map(|s| s.path.clone())) {
                    library::mark_meta(db, &key, "nomatch", true);
                }
                let t = self.refresh_views();
                if let Some(s) = &self.series_page {
                    self.set_header_title(s.name().to_string());
                }
                return t;
            }
            Message::RefreshAllMetadata => {
                if let Some(db) = &self.db {
                    library::reset_all_metadata(db);
                }
                self.meta_attempted.clear();
                let keys: Vec<String> =
                    self.library_entries.iter().filter(|e| !e.is_epub).map(|e| e.path.clone()).collect();
                let tasks: Vec<_> = keys.into_iter().map(|k| self.enqueue_meta(k)).collect();
                return Task::batch(tasks);
            }
            Message::ComicVineKeyInput(text) => {
                self.cv_key_input = text;
            }
            Message::SaveComicVineKey => {
                if let Err(e) = comicvine::save_api_key(&self.cv_key_input) {
                    self.error = Some(e);
                } else {
                    self.cv_key_input.clear();
                    // Series that found nothing without a key may match now.
                    return self.update(Message::RefreshAllMetadata);
                }
            }
            Message::OpenUrl(url) => {
                crate::platform::open_url(&url);
            }
            Message::ToggleIssueRead(id, completed) => {
                if let Some(db) = &self.db {
                    let _ = library::set_issue_completed(db, id, completed);
                }
                let remote: Vec<String> = self
                    .series_page_issues
                    .iter()
                    .filter(|i| i.id == id && crate::jellyfin::is_key(&i.path))
                    .map(|i| i.path.clone())
                    .collect();
                return Task::batch([self.refresh_views(), self.jellyfin_mark(remote, completed)]);
            }
            Message::MarkSeriesRead(completed) => {
                if let Some(key) = self.series_page.as_ref().map(|s| s.path.clone()) {
                    return self.mark_series(&key, completed);
                }
            }
            Message::CardMenuOpened(key) => self.menu_target = Some(key),
            Message::CardMenu(action) => return self.card_action(action),
            Message::Index(msg) => return self.update_index(msg),
            Message::IssueMenuOpened(key) => self.issue_menu_target = Some(key),
            Message::IssueMenu(rar_index::IssueAction::BuildIndex) => {
                if let Some(key) = self.issue_menu_target.clone() {
                    return self.update_index(rar_index::IdxMsg::Build(vec![key]));
                }
            }
            Message::PageSizeRead(key, size) => self.page_size_read(key, size),
            Message::ComicInfoRead(gen, info) => {
                if gen == self.page_gen {
                    self.comic_info = info;
                }
            }
            Message::ChapterCoverReady { gen, chapter_idx, result } => {
                if gen != self.page_gen {
                    return Task::none();
                }
                self.covers_pending.remove(&chapter_idx);
                if let Ok((w, h, rgba)) = result {
                    let handle = widget::image::Handle::from_rgba(w, h, rgba);
                    self.chapter_covers.insert(chapter_idx, handle);
                }
            }
            Message::AddLibraryFolder => {
                return cosmic::task::future(async move {
                    let dialog = file_chooser::open::Dialog::new().title("Add Library Folder");
                    match dialog.open_folder().await {
                        Ok(r) => Message::LibraryFolderSelected(r.url().to_owned()),
                        Err(file_chooser::Error::Cancelled) => Message::Cancelled,
                        Err(e) => Message::LoadFailed(e.to_string()),
                    }
                });
            }
            Message::LibraryFolderSelected(url) => {
                let path = match url.scheme() {
                    "file" => match url.to_file_path() {
                        Ok(p) => p,
                        Err(()) => {
                            self.error = Some(fl!("invalid-folder-path", url = url.to_string()));
                            return Task::none();
                        }
                    },
                    other => {
                        self.error = Some(fl!("unsupported-scheme", scheme = other));
                        return Task::none();
                    }
                };
                return self.add_library_folder(path);
            }
            Message::LibraryPathInputChanged(text) => {
                self.library_path_input = text;
            }
            Message::LibraryPathInputSubmitted => {
                let text = self.library_path_input.trim().to_string();
                if text.is_empty() {
                    return Task::none();
                }
                let path = PathBuf::from(shellexpand_home(&text));
                if !path.is_dir() {
                    self.error = Some(fl!("not-a-folder", path = path.display().to_string()));
                    return Task::none();
                }
                self.library_path_input.clear();
                return self.add_library_folder(path);
            }
            Message::RootDiscovered(root, found) => {
                self.scans_pending = self.scans_pending.saturating_sub(1);
                let Some(found) = found else {
                    // Unreachable root (unmounted drive?): hide, never delete.
                    self.missing_roots.insert(root);
                    return self.reload_library();
                };
                // A drive mounted on a folder that stays behind when it's
                // unplugged (/mnt/usb) looks empty rather than missing.
                // Never treat "suddenly empty" as "everything was deleted":
                // hide the library instead, as for a missing folder.
                let known = self.db.as_ref().map_or(0, |db| library::series_count_under_root(db, &root.to_string_lossy()));
                if found.is_empty() && known > 0 {
                    log::warn!("{} is empty now; keeping its {known} series hidden until it's back", root.display());
                    self.missing_roots.insert(root);
                    return self.reload_library();
                }
                self.missing_roots.remove(&root);
                let mut tasks = Vec::new();
                if let Some(db) = &self.db {
                    let keys: Vec<String> = found.iter().map(|d| d.key.clone()).collect();
                    let removed = library::prune_root(db, &root.to_string_lossy(), &keys);
                    if removed > 0 {
                        log::info!("removed {removed} series no longer under {}", root.display());
                    }
                    for d in found {
                        let known = library::known_page_counts(db, &d.key);
                        let missing = library::issues_missing_details(db, &d.key);
                        self.scans_pending += 1;
                        tasks.push(scan_series_task(d, known, missing));
                    }
                }
                tasks.push(self.reload_library());
                return Task::batch(tasks);
            }
            Message::SeriesScanned(result) => {
                self.scans_pending = self.scans_pending.saturating_sub(1);
                let mut tasks = Vec::new();
                match result {
                    Ok(r) => {
                        if let Some(db) = &self.db {
                            let _ = library::batch(db, || {
                                library::upsert_scanned_series(db, &r.series)?;
                                library::sync_issues(db, &r.series.path, &r.issues)
                            });
                            if library::get_series(db, &r.series.path).is_some_and(|e| e.meta_fetched_at == 0) {
                                tasks.push(self.enqueue_meta(r.series.path.clone()));
                            }
                        }
                    }
                    Err(e) => log::warn!("scan: {e}"),
                }
                // Refresh the grid in batches rather than once per series.
                if self.scans_pending == 0 || self.scans_pending % 8 == 0 {
                    tasks.push(self.refresh_views());
                }
                return Task::batch(tasks);
            }
            Message::IssueCoverReady { key, idx } => {
                self.series_covers_pending.remove(&idx);
                if self.series_page.as_ref().is_some_and(|s| s.path == key) {
                    let cover = library::chapter_cover_path(&key, idx);
                    if cover.exists() {
                        self.series_page_covers.insert(idx, widget::image::Handle::from_path(cover));
                    }
                }
                if idx == 0 {
                    return self.reload_library();
                }
            }
            Message::HideMissing(paths) => {
                self.hidden_paths.extend(paths);
            }
            Message::RescanLibrary => {
                let dirs = self.library_dirs.clone();
                let tasks: Vec<_> = dirs.into_iter().map(|d| self.scan_root(d)).collect();
                return Task::batch(tasks);
            }
            Message::RemoveLibraryFolder(path) => {
                if let Some(db) = &self.db {
                    let root = path.to_string_lossy();
                    let _ = library::remove_library_dir(db, &root);
                    library::remove_series_under_root(db, &root);
                }
                self.library_dirs.retain(|d| d != &path);
                self.missing_roots.remove(&path);
                return self.reload_library();
            }
            Message::ToggleSettings => {
                let showing_settings =
                    self.core.window.show_context && self.context_page == ContextPage::Settings;
                self.context_page = ContextPage::Settings;
                self.set_show_context(!showing_settings);
                // Pick up theme files dropped into the folder meanwhile.
                self.refresh_themes();
            }
            Message::CloseContext => {
                self.set_show_context(false);
            }
            Message::SetBackgroundOpacity(value) => {
                self.settings.background_opacity = value.clamp(0.0, 1.0);
                self.settings.save();
            }
            Message::SetBackground(idx) => {
                if let Some(bg) = BgStyle::ALL.get(idx).copied() {
                    self.settings.background = bg;
                    self.settings.save();
                }
            }
            Message::SetCorners(idx) => {
                if let Some(corners) = Corners::ALL.get(idx).copied() {
                    self.settings.corners = corners;
                    self.settings.save();
                    return self.apply_theme();
                }
            }
            Message::WindowOpened => {
                // `update` re-sends the icon after this.
                self.icon_open = None;
            }
            Message::SetLook(idx) => {
                if let Some(look) = Look::ALL.get(idx).copied() {
                    self.settings.look = look;
                    self.settings.save();
                    return self.apply_theme();
                }
            }
            Message::SetTheme(idx) => {
                let id = idx.checked_sub(1).and_then(|i| self.themes.get(i)).map(|t| t.id.clone());
                return self.select_theme(id);
            }
            Message::SetLanguage(idx) => {
                let id = idx.checked_sub(1).and_then(|i| self.languages.get(i)).map(|l| l.id.clone());
                crate::i18n::select(id.as_deref());
                self.settings.language = id;
                self.settings.save();
                // Everything else is translated as it's drawn.
                self.labels = Labels::new(&self.languages);
                self.refresh_themes();
            }
            Message::ImportTheme => {
                return cosmic::task::future(async move {
                    let mut filter = FileFilter::new(&fl!("file-filter-themes"));
                    for ext in ["ron", "colors"] {
                        #[cfg(target_os = "linux")]
                        {
                            filter = filter.glob(&format!("*.{ext}"));
                        }
                        #[cfg(not(target_os = "linux"))]
                        {
                            filter = filter.extension(ext);
                        }
                    }
                    let dialog = file_chooser::open::Dialog::new().title("Import Theme").filter(filter);
                    match dialog.open_file().await {
                        Ok(r) => Message::ThemeFileSelected(r.url().to_owned()),
                        Err(file_chooser::Error::Cancelled) => Message::Cancelled,
                        Err(e) => Message::LoadFailed(e.to_string()),
                    }
                });
            }
            Message::ThemeFileSelected(url) => {
                let imported = url.to_file_path().map_err(|()| fl!("not-a-local-file")).and_then(|p| themes::import(&p));
                match imported {
                    Ok(id) => {
                        self.refresh_themes();
                        return self.select_theme(Some(id));
                    }
                    Err(e) => self.theme_error = Some(e),
                }
            }
            Message::OpenThemesFolder => {
                let dir = themes::dir();
                let _ = std::fs::create_dir_all(&dir);
                crate::platform::open_url(&dir.to_string_lossy());
            }
            Message::CheckSystemColors => {
                return cosmic::task::future(async {
                    let changed = tokio::task::spawn_blocking(|| look::reread(Desktop::detect()))
                        .await
                        .unwrap_or(false);
                    Message::SystemColorsChecked(changed)
                });
            }
            Message::SystemColorsChecked(changed) => {
                if changed && self.settings.theme.is_none() {
                    return self.apply_theme();
                }
            }
            Message::SetCheckForUpdates(enabled) => {
                self.settings.check_for_updates = enabled;
                self.settings.save();
            }
            Message::CheckForUpdates => return self.check_for_updates(),
            Message::UpdateChecked(result) => {
                self.update = match result {
                    Ok(Some(release)) => UpdateStatus::Available(release),
                    Ok(None) => UpdateStatus::UpToDate,
                    Err(e) => {
                        log::warn!("update check: {e}");
                        UpdateStatus::Failed(e)
                    }
                };
            }
            Message::DismissUpdateBanner => {
                if let UpdateStatus::Available(release) = &self.update {
                    self.settings.dismissed_update = Some(release.version.clone());
                    self.settings.save();
                }
            }
            Message::DismissUpdatedBanner => self.just_updated = false,
            Message::UpdateNow => {
                let plan = match &self.update {
                    UpdateStatus::Available(release) => {
                        crate::updater::Plan::for_release(crate::updates::Install::detect(), release)
                    }
                    _ => None,
                };
                if let Some(plan) = plan {
                    if !matches!(self.update_job, UpdateJob::Downloading { .. } | UpdateJob::Ready(_)) {
                        self.update_job = UpdateJob::Downloading { done: 0, total: None };
                        return cosmic::task::stream(cosmic::iced::futures::StreamExt::map(
                            crate::updater::start(plan),
                            Message::UpdateProgress,
                        ))
                            .map(cosmic::Action::App);
                    }
                }
            }
            Message::UpdateProgress(progress) => {
                self.update_job = match progress {
                    crate::updater::Progress::Downloading { done, total } => UpdateJob::Downloading { done, total },
                    crate::updater::Progress::Ready(staged) => UpdateJob::Ready(staged),
                    crate::updater::Progress::Failed(e) => UpdateJob::Failed(e),
                };
            }
            Message::RestartToUpdate => {
                if matches!(self.update_job, UpdateJob::Ready(_)) {
                    // What quitting normally does first: save the place and
                    // end a Jellyfin reading session.
                    self.persist_progress();
                    let _ = self.update(Message::Jf(JfMsg::Closing));
                }
                if let UpdateJob::Ready(staged) = std::mem::replace(&mut self.update_job, UpdateJob::Idle) {
                    match crate::updater::apply(&staged) {
                        Ok(()) => return cosmic::iced::exit(),
                        Err(e) => {
                            log::warn!("update: {e}");
                            self.update_job = UpdateJob::Failed(e);
                        }
                    }
                }
            }
            Message::SetFetchMetadata(enabled) => {
                self.settings.fetch_metadata = enabled;
                self.settings.save();
            }
            Message::NextPage => {
                if self.epub_book.is_some() {
                    return self.update_book(BookMsg::NextChapter);
                }
                let step = self.pages_shown_at(self.current_page);
                let new = (self.current_page + step).min(self.sources.len().saturating_sub(1));
                if new != self.current_page {
                    self.current_page = new;
                    self.nav_dir = 1;
                    self.persist_progress();
                    return self.refresh_window();
                }
            }
            Message::PrevPage => {
                if self.epub_book.is_some() {
                    return self.update_book(BookMsg::PrevChapter);
                }
                let new = self.prev_view_start();
                if new != self.current_page {
                    self.current_page = new;
                    self.nav_dir = -1;
                    self.persist_progress();
                    return self.refresh_window();
                }
            }
            Message::PointerY(y) => {
                // Reveal at the very top; hide once the pointer leaves the
                // bar (it's about 48 px tall).
                if y <= 4.0 {
                    self.fullscreen_header = true;
                } else if y > 72.0 {
                    self.fullscreen_header = false;
                }
            }
            Message::ToggleFullscreen => {
                self.fullscreen = !self.fullscreen;
                let mode = if self.fullscreen { window::Mode::Fullscreen } else { window::Mode::Windowed };
                // Fresh time and battery right away, not at the next tick.
                let status = if self.status_shown() { self.update(Message::StatusTick) } else { Task::none() };
                if let Some(id) = self.core.main_window_id() {
                    return Task::batch([window::set_mode(id, mode), status]);
                }
                return status;
            }
            Message::StatusTick => {
                return cosmic::task::future(async {
                    let read = tokio::task::spawn_blocking(|| {
                        (crate::platform::status::clock(), crate::platform::status::battery())
                    });
                    match read.await {
                        Ok((clock, battery)) => Message::StatusRead(clock, battery),
                        Err(_) => Message::StatusRead(String::new(), None),
                    }
                });
            }
            Message::StatusRead(clock, battery) => {
                self.status_clock = (!clock.is_empty()).then_some(clock);
                self.battery = battery;
            }
            Message::SetFullscreenStatus(on) => {
                self.settings.fullscreen_status = on;
                self.settings.save();
                if on && self.status_shown() {
                    return self.update(Message::StatusTick);
                }
            }
            Message::ToggleTheater => {
                self.theater_mode = !self.theater_mode;
                self.settings.theater_mode = self.theater_mode;
                self.settings.save();
            }
            Message::ToggleLayout => {
                self.layout = match self.layout { Layout::Single => Layout::Dual, Layout::Dual => Layout::Single };
                self.settings.dual_page = self.layout == Layout::Dual;
                self.settings.save();
                if self.layout == Layout::Dual && self.current_page % 2 != 0 && !self.is_spread(self.current_page - 1) {
                    self.current_page = self.current_page.saturating_sub(1);
                }
                return self.refresh_window();
            }
            Message::ToggleZoom => {
                // From Fit to Width, back to the whole page.
                if self.fit_width {
                    return self.update(Message::ToggleFitWidth);
                }
                self.zoom_active = !self.zoom_active;
                self.zoom = 1.0;
            }
            Message::ToggleFitWidth => {
                self.fit_width = !self.fit_width;
                self.fit_note_counter += 1;
                let note = self.fit_note_counter;
                self.fit_note = Some((if self.fit_width { fl!("fit-width-note") } else { fl!("whole-page-note") }, note));
                let mut tasks = vec![cosmic::task::future(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
                    Message::HideFitNote(note)
                })];
                // The viewer stays while it eases back to the whole page,
                // then the plain page takes over.
                if !self.fit_width && !self.zoom_active {
                    self.fit_width_leaving = true;
                    tasks.push(cosmic::task::future(async {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        Message::FitWidthEased
                    }));
                }
                return Task::batch(tasks);
            }
            Message::FitWidthEased => self.fit_width_leaving = false,
            Message::HideFitNote(note) => {
                if self.fit_note.as_ref().is_some_and(|(_, n)| *n == note) {
                    self.fit_note = None;
                }
            }
            Message::ToggleInfo => {
                let showing_info =
                    self.core.window.show_context && self.context_page == ContextPage::Info;
                self.context_page = ContextPage::Info;
                self.set_show_context(!showing_info);
                if !showing_info {
                    self.show_chapter_select = false;
                }
            }
            Message::ToggleChapterSelect => {
                if self.epub_book.is_some() {
                    self.show_chapter_select = !self.show_chapter_select;
                } else {
                    return self.toggle_chapter_list();
                }
            }
            Message::ChapterGroup(group) => return self.jump_to_chapter_group(group),
            Message::SelectChapter(ch_idx) => {
                if self.epub_book.is_some() {
                    return self.update_book(BookMsg::GoToChapter(ch_idx));
                } else if let Some(ch) = self.chapters.get(ch_idx) {
                    self.current_page = ch.start;
                    self.nav_dir = 1;
                    self.show_chapter_select = false;
                    self.persist_progress();
                    return self.refresh_window();
                }
            }
            Message::CopyPage => {
                if let Some(page) = self.handle(self.current_page) {
                    return copy_task(page.clone());
                }
            }
            Message::CopiedPage(r) => { if let Err(e) = r { self.error = Some(e); } }
            Message::CloseError => { self.error = None; }
            Message::LibrarySearchChanged(q) => {
                self.library_search = q;
                return self.reload_library();
            }
            Message::ToggleLibraryMenu => self.library_menu = !self.library_menu,
            Message::CloseLibraryMenu => self.library_menu = false,
            Message::LibrarySource(i) => {
                self.library_menu = false;
                if let Some(src) = LibrarySource::ALL.get(i) {
                    self.settings.library_source = *src;
                    self.settings.save();
                }
            }
            Message::Key(key) => {
                return self.handle_key(key);
            }
            Message::ModifiersChanged(m) => {
                self.modifiers = m;
            }
            Message::ZoomBootstrapWheel(delta) => {
                // Same mouse-or-touchpad call as the viewer (Windows reports
                // touchpads as fractional lines).
                let physical = self.wheel.is_physical_wheel(&delta, cosmic::iced::time::Instant::now());
                // Ctrl (Cmd) + wheel scrolls a zoomed page; a whole page has
                // nowhere to scroll, so it doesn't start zooming either.
                let moving = physical && self.modifiers.command();
                if self.app_view == AppView::Reader
                    && (physical || self.modifiers.control())
                    && !moving
                    && !self.sources.is_empty()
                {
                    self.enter_zoom();
                }
            }
            Message::ZoomBootstrap => {
                if self.app_view == AppView::Reader && !self.sources.is_empty() {
                    self.enter_zoom();
                }
            }
            Message::ZoomChanged(scale) => {
                self.zoom = scale.clamp(MIN_ZOOM, MAX_ZOOM);
            }
            Message::Touch(event) => self.handle_touch(event),
            Message::FilesDropped(paths) => {
                if let Some(path) = paths.into_iter().next() {
                    return self.open_linked(path);
                }
            }
            Message::DndDataReceived(mime, data) => {
                if mime.contains("uri-list") {
                    if let Some(path) = first_path_from_uri_list(&data) {
                        return self.open_linked(path);
                    }
                }
            }
            Message::Book(msg) => return self.update_book(msg),
            Message::Jf(msg) => return self.update_jellyfin(msg),
            Message::Opds(msg) => return self.update_opds(msg),
            Message::Dl(msg) => return self.update_downloads(msg),
            Message::FitPage => {
                let Some(page) = self.page_size(self.current_page) else { return Task::none() };
                let Some(id) = self.core.main_window_id() else { return Task::none() };
                if page.height == 0 {
                    return Task::none();
                }
                let aspect = page.width as f32 / page.height as f32;
                return window::size(id)
                    .map(move |size| {
                        Message::FitPageSize(cosmic::iced::Size::new(size.height * aspect, size.height))
                    })
                    .map(cosmic::Action::App);
            }
            Message::FitPageSize(size) => {
                if let Some(id) = self.core.main_window_id() {
                    return window::resize(id, size).map(cosmic::Action::App);
                }
            }
        }
        Task::none()
    }

    /// Shows an open book as the window icon while reading, a closed one
    /// otherwise. Only sends anything when that changes.
    fn sync_window_icon(&mut self) -> Task<Message> {
        let open = self.app_view == AppView::Reader;
        if self.icon_open == Some(open) {
            return Task::none();
        }
        let Some(id) = self.core.main_window_id() else { return Task::none() };
        self.icon_open = Some(open);
        #[cfg(windows)]
        return window_icon::set_native(id, open);
        #[cfg(not(windows))]
        match window_icon::load(open) {
            Some(icon) => window::set_icon(id, icon),
            None => Task::none(),
        }
    }
}

/// The brief note at the bottom of the page saying Fit to Width went on
/// or off.
fn fit_note(note: &str) -> Element<'_, Message> {
    widget::container(
        widget::container(widget::text(note).size(13))
            .padding([6, 14])
            .style(|theme: &cosmic::Theme| {
                let c = theme.cosmic();
                widget::container::Style {
                    background: Some(Background::Color(Color { a: 0.85, ..c.bg_color().into() })),
                    text_color: Some(c.on_bg_color().into()),
                    border: cosmic::iced::Border { radius: 999.0.into(), ..Default::default() },
                    ..Default::default()
                }
            }),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Alignment::Center)
    .align_y(Alignment::End)
    .padding([0, 0, 24, 0])
    .into()
}
