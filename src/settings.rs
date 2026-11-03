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
}

/// Which books the library shows when it has books from more than one
/// place (this computer, Jellyfin, OPDS).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibrarySource {
    #[default]
    All,
    Local,
    Jellyfin,
    Opds,
    /// Not in the library: picking it opens the Wikipedia page (when that's
    /// turned on). Never saved as the library's choice.
    Wikipedia,
    /// Like Wikipedia: the Fandom wikis followed.
    Fandom,
}

impl LibrarySource {
    pub const ALL: [LibrarySource; 6] = [
        LibrarySource::All,
        LibrarySource::Local,
        LibrarySource::Jellyfin,
        LibrarySource::Opds,
        LibrarySource::Wikipedia,
        LibrarySource::Fandom,
    ];

    /// Pages of their own (not books in the library).
    pub fn is_page(self) -> bool {
        matches!(self, LibrarySource::Wikipedia | LibrarySource::Fandom)
    }

    /// Whether a library entry (by its key and library root) is from here.
    pub fn includes(self, path: &str, root: &str) -> bool {
        let from = |is: fn(&str) -> bool| is(path) || is(root);
        match self {
            LibrarySource::All => true,
            LibrarySource::Local => !from(crate::remote::is_key),
            LibrarySource::Jellyfin => from(crate::jellyfin::is_key),
            LibrarySource::Opds => from(crate::opds::is_key),
            LibrarySource::Wikipedia | LibrarySource::Fandom => false,
        }
    }
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
    /// The page lets the window's background show through (frosted glass
    /// on COSMIC): with the System theme it's the window's own, with the
    /// others their colour, tinting it. On by default; it only applies
    /// where the window is frosted (`App::book_frosted`).
    pub frosted: bool,
}

impl BookSettings {
    pub const MIN_SIZE: u16 = 12;
    pub const MAX_SIZE: u16 = 40;
    pub const LINE_SPACINGS: [f32; 4] = [1.3, 1.5, 1.7, 1.9];
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
            // On, but it only takes effect where the window is frosted.
            frosted: true,
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
    /// Ask before fetching a big Jellyfin chapter that can't be streamed
    /// (a solid CBR, say) whole.
    pub ask_before_big_fetch: bool,
    /// Where Jellyfin downloads go; `None` is the app's data folder.
    pub download_dir: Option<std::path::PathBuf>,
    /// The version that last ran, to say "updated" once after an update.
    pub last_version: Option<String>,
    /// A newer release whose banner was closed; a later one shows again.
    pub dismissed_update: Option<String>,
    /// Books whose web links open without asking (their file paths).
    pub open_links_from: Vec<String>,
    /// The Wikipedia page is on (it's opt-in).
    pub wikipedia: bool,
    /// Which language's Wikipedia (a code from `wikipedia::LANGUAGES`).
    pub wikipedia_language: String,
    /// The Fandom page is on (it's opt-in, like Wikipedia).
    pub fandom: bool,
    /// The Fandom wikis followed, in the order they were added.
    pub fandom_wikis: Vec<crate::fandom::Wiki>,
    /// Interface language (an `i18n/` folder name); `None` follows the
    /// system.
    pub language: Option<String>,
    /// The library menu's sections in the order chosen (All, always
    /// first, isn't in it); see [`Settings::menu`].
    pub menu_order: Vec<LibrarySource>,
    /// Library sections left out of the menu (Wikipedia and Fandom are
    /// shown by their own switches).
    pub menu_hidden: Vec<LibrarySource>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            background_opacity: 1.0,
            background: Background::Theme,
            dual_page: false,
            // A black backdrop suits comics, and saves power on OLED screens.
            theater_mode: true,
            fetch_metadata: true,
            book: BookSettings::default(),
            look: crate::platform::look::Look::Auto,
            corners: crate::platform::look::Corners::Auto,
            theme: None,
            library_source: LibrarySource::All,
            check_for_updates: true,
            fullscreen_status: true,
            ask_before_big_fetch: true,
            download_dir: None,
            last_version: None,
            dismissed_update: None,
            open_links_from: Vec::new(),
            wikipedia: false,
            wikipedia_language: "en".to_string(),
            fandom: false,
            fandom_wikis: Vec::new(),
            language: None,
            menu_order: Vec::new(),
            menu_hidden: Vec::new(),
        }
    }
}

fn config_path() -> std::path::PathBuf {
    crate::library::config_dir().join("settings.json")
}

impl Settings {
    /// The library menu's sections after All, in the order chosen: any not
    /// placed yet (new ones, or all of them before it was changed) where
    /// they are by default.
    pub fn menu(&self) -> Vec<LibrarySource> {
        let mut out: Vec<LibrarySource> = Vec::new();
        for src in self.menu_order.iter().copied().filter(|s| *s != LibrarySource::All) {
            if !out.contains(&src) {
                out.push(src);
            }
        }
        for (i, src) in LibrarySource::ALL[1..].iter().copied().enumerate() {
            if !out.contains(&src) {
                // After the one before it by default, so Fandom lands under Wikipedia.
                let at = if i == 0 { 0 } else { out.iter().position(|s| *s == LibrarySource::ALL[i]).map_or(out.len(), |p| p + 1) };
                out.insert(at, src);
            }
        }
        out
    }

    /// Whether a section is turned on for the menu (it still only shows
    /// when the library has books from there).
    pub fn shown(&self, src: LibrarySource) -> bool {
        match src {
            LibrarySource::All => true,
            LibrarySource::Wikipedia => self.wikipedia,
            LibrarySource::Fandom => self.fandom,
            _ => !self.menu_hidden.contains(&src),
        }
    }

    /// Moves a section up (`-1`) or down (`1`) in the menu.
    pub fn move_in_menu(&mut self, src: LibrarySource, by: i32) {
        let mut order = self.menu();
        let Some(i) = order.iter().position(|s| *s == src) else { return };
        let j = i as i32 + by;
        if j >= 0 && (j as usize) < order.len() {
            order.swap(i, j as usize);
            self.menu_order = order;
        }
    }

    /// Turns a section on or off in the menu.
    pub fn show_in_menu(&mut self, src: LibrarySource, on: bool) {
        match src {
            LibrarySource::All => {}
            LibrarySource::Wikipedia => self.wikipedia = on,
            LibrarySource::Fandom => self.fandom = on,
            _ => {
                self.menu_hidden.retain(|s| *s != src);
                if !on {
                    self.menu_hidden.push(src);
                }
            }
        }
        // The library shouldn't stay filtered to a section that's gone.
        if !on && self.library_source == src {
            self.library_source = LibrarySource::All;
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_keeps_its_order_and_places_new_sections() {
        let mut s = Settings::default();
        use LibrarySource::*;
        assert_eq!(s.menu(), [Local, Jellyfin, Opds, Wikipedia, Fandom]);
        // Saved before Fandom existed: it goes under Wikipedia.
        s.menu_order = vec![Wikipedia, Local, Opds, Jellyfin];
        assert_eq!(s.menu(), [Wikipedia, Fandom, Local, Opds, Jellyfin]);
        s.move_in_menu(Fandom, -1);
        assert_eq!(s.menu(), [Fandom, Wikipedia, Local, Opds, Jellyfin]);
        s.move_in_menu(Fandom, -1);
        assert_eq!(s.menu()[0], Fandom);
        assert!(!s.shown(Fandom) && s.shown(Jellyfin));
        s.library_source = Jellyfin;
        s.show_in_menu(Jellyfin, false);
        s.show_in_menu(Fandom, true);
        assert!(!s.shown(Jellyfin) && s.shown(Fandom));
        assert_eq!(s.library_source, All);
        s.show_in_menu(Jellyfin, true);
        assert!(s.shown(Jellyfin));
    }

    #[test]
    fn book_frosted_glass_is_on_unless_turned_off() {
        // Saved before the switch existed: on.
        let old: BookSettings = serde_json::from_str(r#"{"font_size": 21, "justify": false}"#).unwrap();
        assert!(old.frosted);
        assert_eq!(old.font_size, 21);
        // Turned off: stays off.
        let off: BookSettings = serde_json::from_str(r#"{"frosted": false}"#).unwrap();
        assert!(!off.frosted);
    }
}
