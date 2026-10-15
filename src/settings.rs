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
