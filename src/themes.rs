//! Colour themes, in formats people already have files for:
//!
//! - **COSMIC themes** (`.ron`): what COSMIC Settings exports under
//!   Appearance → Export, and what cosmic-themes.org hosts. It's
//!   libcosmic's own `ThemeBuilder`, so these load exactly as COSMIC
//!   draws them.
//! - **KDE colour schemes** (`.colors`): Plasma's format, the same INI
//!   layout as `kdeglobals`, from System Settings or the KDE Store. Their
//!   window, view, text and accent colours are used.
//!
//! The built-in themes are COSMIC `.ron` files too (`res/themes/`), so
//! they double as examples to copy and edit. User themes live in
//! [`dir`]; Settings can import one or open the folder.
//!
//! A theme sets colours only. The Style setting still decides the shapes
//! and title bar, so a theme looks at home next to the desktop's own
//! windows.

use std::path::{Path, PathBuf};

use cosmic::cosmic_theme::ThemeBuilder;

use crate::platform::look::HostColors;

/// Built-in themes: (id, file contents).
const BUILTIN: &[(&str, &str)] = &[
    ("cosmic-dark", include_str!("../res/themes/cosmic-dark.ron")),
    ("cosmic-light", include_str!("../res/themes/cosmic-light.ron")),
    ("catppuccin-mocha", include_str!("../res/themes/catppuccin-mocha.ron")),
    ("catppuccin-latte", include_str!("../res/themes/catppuccin-latte.ron")),
    ("nord", include_str!("../res/themes/nord.ron")),
    ("dracula", include_str!("../res/themes/dracula.ron")),
    ("gruvbox-dark", include_str!("../res/themes/gruvbox-dark.ron")),
    ("solarized-light", include_str!("../res/themes/solarized-light.ron")),
];

/// One theme the user can pick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// What Settings stores: `builtin:<id>` or the file name in [`dir`].
    pub id: String,
    pub name: String,
}

/// A theme's colours, ready to build a libcosmic theme from.
#[derive(Clone, Debug)]
pub enum Colors {
    Cosmic(Box<ThemeBuilder>),
    Kde(HostColors),
}

/// Where user themes go: `~/.config/nebula-reader/themes` and the like.
pub fn dir() -> PathBuf {
    crate::platform::config_dir().join("themes")
}

/// Built-in themes, then the files in [`dir`] by name.
pub fn list() -> Vec<Entry> {
    let mut out: Vec<Entry> = BUILTIN
        .iter()
        .map(|(id, _)| Entry { id: format!("builtin:{id}"), name: pretty_name(id) })
        .collect();
    let mut files: Vec<Entry> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| kind(p).is_some())
        .filter_map(|p| {
            let file = p.file_name()?.to_str()?.to_string();
            let stem = p.file_stem()?.to_str()?;
            Some(Entry { name: pretty_name(stem), id: file })
        })
        .collect();
    files.sort_by_key(|e| e.name.to_lowercase());
    out.extend(files);
    out
}

/// Loads a theme by its [`Entry::id`].
pub fn load(id: &str) -> Result<Colors, String> {
    if let Some(builtin) = id.strip_prefix("builtin:") {
        let (_, text) = BUILTIN.iter().find(|(b, _)| *b == builtin).ok_or("unknown built-in theme")?;
        return parse_cosmic(text);
    }
    // Only a bare file name inside the themes folder.
    if Path::new(id).file_name().and_then(|n| n.to_str()) != Some(id) {
        return Err(fl!("theme-bad-name"));
    }
    load_file(&dir().join(id))
}

/// Checks a theme file and copies it into [`dir`]; returns its id.
pub fn import(path: &Path) -> Result<String, String> {
    load_file(path)?;
    let name = path.file_name().and_then(|n| n.to_str()).ok_or("bad file name")?.to_string();
    let target = dir();
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    std::fs::copy(path, target.join(&name)).map_err(|e| e.to_string())?;
    Ok(name)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Cosmic,
    Kde,
}

fn kind(path: &Path) -> Option<Kind> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "ron" => Some(Kind::Cosmic),
        "colors" => Some(Kind::Kde),
        _ => None,
    }
}

fn load_file(path: &Path) -> Result<Colors, String> {
    let kind = kind(path).ok_or("theme files end in .ron (COSMIC) or .colors (KDE)")?;
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    match kind {
        Kind::Cosmic => parse_cosmic(&text),
        Kind::Kde => {
            let colors = crate::platform::look::parse_kde_colors(&text);
            if colors.window.is_none() && colors.accent.is_none() {
                return Err(fl!("theme-no-colors"));
            }
            Ok(Colors::Kde(colors))
        }
    }
}

fn parse_cosmic(text: &str) -> Result<Colors, String> {
    ron::from_str::<ThemeBuilder>(text)
        .map(|b| Colors::Cosmic(Box::new(b)))
        .map_err(|e| format!("not a COSMIC theme: {e}"))
}

/// `catppuccin-mocha` → `Catppuccin Mocha`, `My_Theme` → `My Theme`.
fn pretty_name(stem: &str) -> String {
    stem.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(first) => first.to_uppercase().chain(c).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
        .replace("Cosmic", "COSMIC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_themes_parse() {
        for (id, text) in BUILTIN {
            assert!(parse_cosmic(text).is_ok(), "{id} doesn't parse");
        }
    }

    #[test]
    fn names() {
        assert_eq!(pretty_name("catppuccin-mocha"), "Catppuccin Mocha");
        assert_eq!(pretty_name("cosmic-dark"), "COSMIC Dark");
        assert_eq!(pretty_name("My_theme"), "My Theme");
    }

    #[test]
    fn kde_colors_file() {
        let dir = std::env::temp_dir().join(format!("nebula-theme-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Test.colors");
        std::fs::write(&file, "[Colors:Window]\nBackgroundNormal=32,35,38\n[General]\nAccentColor=233,100,58\n").unwrap();
        assert!(matches!(load_file(&file), Ok(Colors::Kde(c)) if c.dark == Some(true)));
        std::fs::write(dir.join("Empty.colors"), "[General]\nName=x\n").unwrap();
        assert!(load_file(&dir.join("Empty.colors")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Checks any theme file: `NEBULA_THEME_FILE=path cargo test
    /// loads_theme_file -- --ignored`.
    #[test]
    #[ignore]
    fn loads_theme_file() {
        let path = std::env::var("NEBULA_THEME_FILE").expect("set NEBULA_THEME_FILE");
        if let Err(e) = load_file(Path::new(&path)) {
            panic!("{e}");
        }
    }

    #[test]
    fn rejects_paths_outside_the_folder() {
        assert!(load("../settings.json").is_err());
    }

    /// Writes `res/themes/*.ron` from the palettes below. Run with
    /// `cargo test write_builtin_themes -- --ignored` after changing them.
    #[test]
    #[ignore]
    fn write_builtin_themes() {
        use cosmic::cosmic_theme::palette::{Srgb, Srgba};
        use cosmic::cosmic_theme::CosmicPalette;

        fn rgb(hex: u32) -> Srgb {
            let [_, r, g, b] = hex.to_be_bytes();
            Srgb::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
        }
        fn rgba(hex: u32) -> Srgba {
            let c = rgb(hex);
            Srgba::new(c.red, c.green, c.blue, 1.0)
        }
        // (id, dark, window, containers, text, accent)
        let palettes: &[(&str, bool, u32, u32, u32, u32)] = &[
            ("catppuccin-mocha", true, 0x1e1e2e, 0x313244, 0xcdd6f4, 0xcba6f7),
            ("catppuccin-latte", false, 0xe6e9ef, 0xeff1f5, 0x4c4f69, 0x8839ef),
            ("nord", true, 0x2e3440, 0x3b4252, 0xeceff4, 0x88c0d0),
            ("dracula", true, 0x21222c, 0x282a36, 0xf8f8f2, 0xbd93f9),
            ("gruvbox-dark", true, 0x282828, 0x3c3836, 0xebdbb2, 0xfabd2f),
            ("solarized-light", false, 0xeee8d5, 0xfdf6e3, 0x586e75, 0x268bd2),
        ];
        let named = |mut b: ThemeBuilder, id: &str| {
            match &mut b.palette {
                CosmicPalette::Dark(p)
                | CosmicPalette::Light(p)
                | CosmicPalette::HighContrastDark(p)
                | CosmicPalette::HighContrastLight(p) => p.name = id.to_string(),
            }
            b
        };
        let mut themes = vec![
            ("cosmic-dark", named(ThemeBuilder::dark(), "cosmic-dark")),
            ("cosmic-light", named(ThemeBuilder::light(), "cosmic-light")),
        ];
        for &(id, dark, window, view, text, accent) in palettes {
            let base = if dark { ThemeBuilder::dark() } else { ThemeBuilder::light() };
            let b = base
                .neutral_tint(rgb(window))
                .bg_color(rgba(window))
                .primary_container_bg(rgba(view))
                .text_tint(rgb(text))
                .accent(rgb(accent));
            themes.push((id, named(b, id)));
        }
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("res/themes");
        std::fs::create_dir_all(&out).unwrap();
        for (id, builder) in themes {
            let text = ron::ser::to_string_pretty(&builder, ron::ser::PrettyConfig::default()).unwrap();
            std::fs::write(out.join(format!("{id}.ron")), text + "\n").unwrap();
        }
    }
}
