//! User settings, persisted as JSON in the XDG config directory.
//!
//! Kept deliberately separate from the library database: settings live in
//! `$XDG_CONFIG_HOME`, the library lives in `$XDG_DATA_HOME`, and derived
//! artifacts (covers, extracted archives) live in `$XDG_CACHE_HOME`. That
//! split is what makes state survive reinstalls and upgrades predictably —
//! a user can clear the cache without losing their library or preferences.

use serde::{Deserialize, Serialize};

/// How the reader fills space around a page that doesn't match the window
/// aspect ratio.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Background {
    /// Follow the COSMIC theme.
    Theme,
    /// Solid black, like a cinema.
    Black,
}

impl Background {
    pub const ALL: [Background; 2] = [Background::Theme, Background::Black];
}

/// Typeface for books. The generic families resolve through fontconfig to
/// the system's preferred serif/sans (Noto Serif etc.).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BookFont {
    Serif,
    Sans,
}

/// Page color scheme for books.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BookTheme {
    /// Follow the COSMIC light/dark theme.
    Auto,
    Paper,
    Sepia,
    Dark,
    /// Pure black background, dimmed text (OLED / night reading).
    Night,
}

impl BookTheme {
    pub const ALL: [BookTheme; 5] =
        [BookTheme::Auto, BookTheme::Paper, BookTheme::Sepia, BookTheme::Dark, BookTheme::Night];
    pub const LABELS: [&'static str; 5] = ["Auto", "Paper", "Sepia", "Dark", "Night"];
}

/// Which books the library shows when it has both local and Jellyfin ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibrarySource {
    #[default]
    All,
    Local,
    Jellyfin,
}

impl LibrarySource {
    pub const ALL: [LibrarySource; 3] = [LibrarySource::All, LibrarySource::Local, LibrarySource::Jellyfin];
    pub const LABELS: [&'static str; 3] = ["All libraries", "Local", "Jellyfin"];
}

/// How a book's chapter is laid out on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BookLayout {
    /// One page at a time; arrows, clicks and the wheel turn pages.
    Page,
    /// Two pages side by side, read left column then right, like an open
    /// book. Falls back to one page when the window is too narrow.
    TwoPages,
    /// One long scrolling column per chapter.
    Scroll,
}

impl BookLayout {
    pub const ALL: [BookLayout; 3] = [BookLayout::Page, BookLayout::TwoPages, BookLayout::Scroll];
    pub const LABELS: [&'static str; 3] = ["Single page", "Two pages", "Scroll"];
}

/// Text column width, as a reading measure (characters per line).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BookWidth {
    Narrow,
    Medium,
    Wide,
}

impl BookWidth {
    pub const ALL: [BookWidth; 3] = [BookWidth::Narrow, BookWidth::Medium, BookWidth::Wide];
    pub const LABELS: [&'static str; 3] = ["Narrow", "Medium", "Wide"];

    /// Column width in ems. Average Latin glyphs are ~0.5em wide, so
    /// Medium (34em) lands near the classic ~66 characters per line.
    pub fn ems(self) -> f32 {
        match self {
            BookWidth::Narrow => 28.0,
            BookWidth::Medium => 34.0,
            BookWidth::Wide => 42.0,
        }
    }
}

/// Book typography. Kept separate from comic settings; every option is a
/// reader override, the publisher's CSS only contributes semantics
/// (italics, centering, indents).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct BookSettings {
    /// Body text size in pixels.
    pub font_size: u16,
    pub font: BookFont,
    /// Line height as a multiple of the font size.
    pub line_spacing: f32,
    pub width: BookWidth,
    pub theme: BookTheme,
    pub justify: bool,
    pub layout: BookLayout,
}

impl BookSettings {
    pub const MIN_SIZE: u16 = 12;
    pub const MAX_SIZE: u16 = 40;
    pub const LINE_SPACINGS: [f32; 4] = [1.3, 1.5, 1.7, 1.9];
    pub const LINE_LABELS: [&'static str; 4] = ["Tight", "Normal", "Relaxed", "Loose"];
}

impl Default for BookSettings {
    fn default() -> Self {
        Self {
            font_size: 19,
            font: BookFont::Serif,
            line_spacing: 1.5,
            width: BookWidth::Medium,
            theme: BookTheme::Auto,
            justify: true,
            layout: BookLayout::Page,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Opacity of the reader's backdrop (the area around the page), 0.0–1.0.
    /// Applies to the background only, never to the page image itself.
    pub background_opacity: f32,
    pub background: Background,
    /// Restore these reader preferences across sessions.
    pub dual_page: bool,
    pub theater_mode: bool,
    /// Look up metadata online (AniList / ComicVine) when opening a comic.
    pub fetch_metadata: bool,
    pub book: BookSettings,
    /// Which desktop's look the UI takes on (see `platform::look`).
    pub look: crate::platform::look::Look,
    /// Button/corner shape override for the non-COSMIC styles.
    pub corners: crate::platform::look::Corners,
    /// Colour theme (`crate::themes::Entry::id`); `None` follows the system.
    pub theme: Option<String>,
    /// The library's All / Local / Jellyfin switch.
    pub library_source: LibrarySource,
    /// Ask GitHub for a newer release when the app starts.
    pub check_for_updates: bool,
    /// Show the time and battery in a corner while reading in full screen.
    pub fullscreen_status: bool,
    /// Where Jellyfin downloads go; `None` is the app's data folder.
    pub download_dir: Option<std::path::PathBuf>,
    /// The version that last ran, to say "updated" once after an update.
    pub last_version: Option<String>,
    /// A newer release whose banner was closed; a later one shows again.
    pub dismissed_update: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            background_opacity: 1.0,
            background: Background::Theme,
            dual_page: false,
            theater_mode: false,
            fetch_metadata: true,
            book: BookSettings::default(),
            look: crate::platform::look::Look::Auto,
            corners: crate::platform::look::Corners::Auto,
            theme: None,
            library_source: LibrarySource::All,
            check_for_updates: true,
            fullscreen_status: true,
            download_dir: None,
            last_version: None,
            dismissed_update: None,
        }
    }
}

fn config_path() -> std::path::PathBuf {
    crate::library::config_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read(config_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Whether a settings file was saved before, i.e. not a fresh install.
    pub fn exists() -> bool {
        config_path().exists()
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}
