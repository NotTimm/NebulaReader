//! Native look and feel per desktop.
//!
//! The UI is libcosmic everywhere, but outside COSMIC it would otherwise
//! fall back to COSMIC's stock dark theme: big pill buttons, COSMIC's
//! colours and its own client-side title bar. Here the theme is rebuilt
//! from the host desktop instead:
//!
//! - **Breeze** (KDE Plasma): colours from `kdeglobals` (so custom colour
//!   schemes carry over), small Breeze corner radii, KWin's title bar.
//! - **Fluent** (Windows 11): light/dark mode and accent colour from the
//!   registry, Fluent's 4px controls, the native Windows title bar.
//! - **Adwaita** (GNOME and friends): light/dark and accent from the XDG
//!   settings portal, Adwaita's radii, and libcosmic's own header bar
//!   (Mutter doesn't draw server-side title bars on Wayland, and the
//!   header bar already looks like a GNOME one).
//! - **COSMIC**: the live system theme, unchanged.
//!
//! macOS gets the Adwaita shapes with the system's light/dark mode and
//! accent colour, and always the native title bar.
//!
//! The host's colours are read at startup ([`init`]) and cached, so
//! switching looks in Settings never blocks on D-Bus or the registry. While
//! the app runs, [`reread`] picks up changes (dark mode switched on, a new
//! accent colour) off the UI thread.
//!
//! A colour theme picked in Settings (`crate::themes`) replaces the host's
//! colours; the look still decides shapes and the title bar.

use std::sync::{Arc, RwLock};

use cosmic::cosmic_theme::palette::{Srgb, Srgba};
use cosmic::cosmic_theme::{CornerRadii, Roundness, ThemeBuilder};
use serde::{Deserialize, Serialize};

use super::Desktop;

/// The look the user picked in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Look {
    /// Match whatever desktop we're running on.
    #[default]
    Auto,
    Cosmic,
    Breeze,
    Fluent,
    Adwaita,
}

impl Look {
    pub const ALL: [Look; 5] = [Look::Auto, Look::Cosmic, Look::Breeze, Look::Fluent, Look::Adwaita];
    pub const LABELS: [&'static str; 5] =
        ["Match desktop", "COSMIC", "KDE Breeze", "Windows 11", "GNOME Adwaita"];

    /// Picks a concrete look for `Auto`.
    pub fn resolve(self, desktop: Desktop) -> Look {
        match self {
            Look::Auto => match desktop {
                Desktop::Kde => Look::Breeze,
                Desktop::Windows => Look::Fluent,
                Desktop::Gnome | Desktop::MacOs => Look::Adwaita,
                Desktop::Cosmic | Desktop::Other => Look::Cosmic,
            },
            other => other,
        }
    }

    /// Whether the window should use the system's own title bar instead
    /// of libcosmic's header bar with window buttons. Always on macOS,
    /// where window buttons belong on the left of a real title bar.
    pub fn native_titlebar(self) -> bool {
        cfg!(target_os = "macos") || matches!(self, Look::Breeze | Look::Fluent)
    }
}

/// Button and corner shape. `Auto` keeps the style's own shape (Breeze's
/// small corners, Fluent's 4px, Adwaita's 6px); the others are COSMIC's
/// three roundness options, for people who'd rather have those anywhere.
/// The COSMIC style ignores this: COSMIC Settings already controls it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Corners {
    #[default]
    Auto,
    Round,
    SlightlyRound,
    Square,
}

impl Corners {
    pub const ALL: [Corners; 4] = [Corners::Auto, Corners::Round, Corners::SlightlyRound, Corners::Square];
    pub const LABELS: [&'static str; 4] = ["Match style", "Round", "Slightly round", "Square"];

    fn radii(self) -> Option<CornerRadii> {
        match self {
            Corners::Auto => None,
            Corners::Round => Some(Roundness::Round.into()),
            Corners::SlightlyRound => Some(Roundness::SlightlyRound.into()),
            Corners::Square => Some(Roundness::Square.into()),
        }
    }
}

/// What we could learn about the host desktop's colours. Every field is
/// optional; the look's built-in defaults fill the gaps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostColors {
    pub dark: Option<bool>,
    pub accent: Option<Srgb>,
    /// Window background.
    pub window: Option<Srgb>,
    /// Content (list/view) background.
    pub view: Option<Srgb>,
    pub text: Option<Srgb>,
}

static HOST: RwLock<Option<HostColors>> = RwLock::new(None);

/// Reads the host desktop's colours. Call once, before the app starts
/// (it may briefly block on the settings portal or registry).
pub fn init(desktop: Desktop) {
    *HOST.write().unwrap_or_else(|e| e.into_inner()) = Some(read_host_colors(desktop));
}

/// Reads the host's colours again; true when they changed since the last
/// read. Blocks briefly, so call it off the UI thread.
pub fn reread(desktop: Desktop) -> bool {
    let fresh = read_host_colors(desktop);
    let mut host = HOST.write().unwrap_or_else(|e| e.into_inner());
    if host.as_ref() == Some(&fresh) {
        return false;
    }
    *host = Some(fresh);
    true
}

fn host() -> HostColors {
    HOST.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// Reads a KDE colour scheme (`.colors` file or `kdeglobals`).
pub fn parse_kde_colors(text: &str) -> HostColors {
    kde::parse(text)
}

/// Builds the libcosmic theme for an already-resolved look, with the
/// colours of a picked theme or, without one, the host's.
pub fn theme(look: Look, corners: Corners, colors: Option<&crate::themes::Colors>) -> cosmic::Theme {
    use crate::themes::Colors;

    let preset = match look {
        Look::Auto | Look::Cosmic => None,
        Look::Breeze => Some(Preset::BREEZE),
        Look::Fluent => Some(Preset::FLUENT),
        Look::Adwaita => Some(Preset::ADWAITA),
    };
    // COSMIC's shapes come from COSMIC Settings, like its colours.
    let radii = match &preset {
        Some(p) => corners.radii().unwrap_or(p.radii),
        None => cosmic::theme::system_preference().cosmic().corner_radii,
    };
    let host = match colors {
        Some(Colors::Cosmic(builder)) => {
            let built = builder.as_ref().clone().corner_radii(radii).build();
            return cosmic::Theme::custom(Arc::new(built));
        }
        Some(Colors::Kde(c)) => c.clone(),
        None => match preset {
            None => return cosmic::theme::system_preference(),
            Some(_) => host(),
        },
    };
    let preset = preset.unwrap_or(Preset::COSMIC);
    let dark = host
        .dark
        .or_else(|| host.window.map(|c| luminance(c) < 0.5))
        .unwrap_or(preset.default_dark);
    let colors = if dark { &preset.dark } else { &preset.light };

    let builder = if dark { ThemeBuilder::dark() } else { ThemeBuilder::light() };
    let built = builder
        .corner_radii(radii)
        .accent(host.accent.unwrap_or_else(|| rgb(preset.accent)))
        .bg_color(opaque(host.window.unwrap_or_else(|| rgb(colors.window))))
        .primary_container_bg(opaque(host.view.unwrap_or_else(|| rgb(colors.view))))
        .text_tint(host.text.unwrap_or_else(|| rgb(colors.text)))
        .build();
    cosmic::Theme::custom(Arc::new(built))
}

// ── Presets ───────────────────────────────────────────────────────────────────

struct Shades {
    window: u32,
    view: u32,
    text: u32,
}

struct Preset {
    radii: CornerRadii,
    accent: u32,
    default_dark: bool,
    light: Shades,
    dark: Shades,
}

/// `[xs, s, m, l, xl]`; libcosmic draws buttons and text inputs with `xl`,
/// which is the radius that most decides whether it "looks like" a desktop.
const fn radii(r: [f32; 5]) -> CornerRadii {
    CornerRadii {
        radius_0: [0.0; 4],
        radius_xs: [r[0]; 4],
        radius_s: [r[1]; 4],
        radius_m: [r[2]; 4],
        radius_l: [r[3]; 4],
        radius_xl: [r[4]; 4],
    }
}

impl Preset {
    /// Only for a KDE colour scheme picked under the COSMIC style; the
    /// scheme supplies the colours, these just fill gaps.
    const COSMIC: Preset = Preset {
        radii: radii([4.0, 8.0, 8.0, 16.0, 160.0]),
        accent: 0x94ebeb,
        default_dark: true,
        light: Shades { window: 0xe8e8e8, view: 0xf6f6f6, text: 0x1b1b1b },
        dark: Shades { window: 0x1b1b1b, view: 0x272727, text: 0xe6e6e6 },
    };
    /// Plasma 6 Breeze / Breeze Dark.
    const BREEZE: Preset = Preset {
        radii: radii([2.0, 3.0, 5.0, 8.0, 3.0]),
        accent: 0x3daee9,
        default_dark: false,
        light: Shades { window: 0xeff0f1, view: 0xffffff, text: 0x232629 },
        dark: Shades { window: 0x202326, view: 0x141618, text: 0xfcfcfc },
    };
    /// Windows 11 (Fluent / WinUI 3).
    const FLUENT: Preset = Preset {
        radii: radii([2.0, 4.0, 8.0, 8.0, 4.0]),
        accent: 0x0078d4,
        default_dark: false,
        light: Shades { window: 0xf3f3f3, view: 0xfbfbfb, text: 0x1b1b1b },
        dark: Shades { window: 0x202020, view: 0x2b2b2b, text: 0xffffff },
    };
    /// libadwaita 1.6+.
    const ADWAITA: Preset = Preset {
        radii: radii([4.0, 6.0, 12.0, 12.0, 6.0]),
        accent: 0x3584e4,
        default_dark: false,
        light: Shades { window: 0xfafafb, view: 0xffffff, text: 0x2e2e32 },
        dark: Shades { window: 0x222226, view: 0x2e2e32, text: 0xffffff },
    };
}

fn rgb(hex: u32) -> Srgb {
    Srgb::new(
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
    )
}

fn opaque(c: Srgb) -> Srgba {
    Srgba::new(c.red, c.green, c.blue, 1.0)
}

/// Relative luminance (WCAG), for guessing dark vs light from a colour.
fn luminance(c: Srgb) -> f32 {
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c.red) + 0.7152 * lin(c.green) + 0.0722 * lin(c.blue)
}

// ── Reading the host ──────────────────────────────────────────────────────────

fn read_host_colors(desktop: Desktop) -> HostColors {
    match desktop {
        Desktop::Kde => {
            let mut colors = kde::read().unwrap_or_default();
            // A missing kdeglobals (fresh account, sandbox without access)
            // still has the portal for dark mode and accent.
            if colors.dark.is_none() && colors.window.is_none() {
                colors = portal::read();
            }
            colors
        }
        Desktop::Windows => windows::read(),
        Desktop::Gnome | Desktop::Other => portal::read(),
        Desktop::MacOs => macos::read(),
        Desktop::Cosmic => HostColors::default(),
    }
}

/// KDE Plasma: `kdeglobals` is an INI file holding the active colour
/// scheme, e.g. `[Colors:Window]` / `BackgroundNormal=32,35,38`.
mod kde {
    use super::*;

    pub fn read() -> Option<HostColors> {
        // Inside a Flatpak $XDG_CONFIG_HOME is the sandbox's, so also try
        // the real ~/.config (exposed read-only by the manifest).
        let candidates = [
            std::env::var_os("XDG_CONFIG_HOME").map(|d| std::path::PathBuf::from(d).join("kdeglobals")),
            Some(crate::platform::home_dir().join(".config/kdeglobals")),
        ];
        candidates
            .into_iter()
            .flatten()
            .find_map(|p| std::fs::read_to_string(p).ok())
            .map(|text| parse(&text))
    }

    pub fn parse(text: &str) -> HostColors {
        let mut section = "";
        let mut out = HostColors::default();
        let mut selection = None;
        for line in text.lines().map(str::trim) {
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name;
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = parse_color(value.trim());
            match (section, key.trim()) {
                ("General", "AccentColor") => out.accent = value,
                ("Colors:Window", "BackgroundNormal") => out.window = value,
                ("Colors:Window", "ForegroundNormal") => out.text = value,
                ("Colors:View", "BackgroundNormal") => out.view = value,
                ("Colors:Selection", "BackgroundNormal") => selection = value,
                _ => {}
            }
        }
        // Without an explicit accent, Plasma's accent *is* the selection colour.
        out.accent = out.accent.or(selection);
        out.dark = out.window.map(|c| luminance(c) < 0.5);
        out
    }

    /// `r,g,b` or `r,g,b,a` in 0–255.
    fn parse_color(value: &str) -> Option<Srgb> {
        let mut parts = value.split(',').map(|p| p.trim().parse::<u8>().ok());
        let (r, g, b) = (parts.next()??, parts.next()??, parts.next()??);
        Some(Srgb::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0))
    }
}

/// The XDG desktop settings portal (`org.freedesktop.appearance`), which
/// GNOME, KDE and others implement, and which works inside a Flatpak.
mod portal {
    use super::*;

    #[cfg(target_os = "linux")]
    pub fn read() -> HostColors {
        use ashpd::desktop::settings::{ColorScheme, Settings};

        let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
            return HostColors::default();
        };
        let read = async {
            let settings = Settings::new().await.ok()?;
            let dark = match settings.color_scheme().await {
                Ok(ColorScheme::PreferDark) => Some(true),
                Ok(ColorScheme::PreferLight) => Some(false),
                _ => None,
            };
            // `accent_color` is newer (GNOME 47+); older portals just don't have it.
            let accent = settings.accent_color().await.ok().and_then(|c| {
                let (r, g, b) = (c.red(), c.green(), c.blue());
                // Out-of-range values mean "unset" per the portal spec.
                [r, g, b]
                    .iter()
                    .all(|v| (0.0..=1.0).contains(v))
                    .then(|| Srgb::new(r as f32, g as f32, b as f32))
            });
            Some(HostColors { dark, accent, ..HostColors::default() })
        };
        // Never hold up startup for long if no portal is running.
        rt.block_on(async { tokio::time::timeout(std::time::Duration::from_millis(500), read).await })
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    #[cfg(not(target_os = "linux"))]
    pub fn read() -> HostColors {
        HostColors::default()
    }
}

/// Windows: personalisation settings live in the current user's registry.
mod windows {
    use super::*;

    #[cfg(windows)]
    pub fn read() -> HostColors {
        use windows_registry::CURRENT_USER;

        let dark = CURRENT_USER
            .open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
            .and_then(|k| k.get_u32("AppsUseLightTheme"))
            .ok()
            .map(|light| light == 0);
        // The shade Windows 11's own apps use for this mode.
        let shade = CURRENT_USER
            .open(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent")
            .and_then(|k| k.get_value("AccentPalette"))
            .ok()
            .and_then(|v| accent_shade(&v, dark.unwrap_or(false)));
        // Stored as 0xAABBGGRR.
        let accent = shade.or_else(|| CURRENT_USER
            .open(r"Software\Microsoft\Windows\DWM")
            .and_then(|k| k.get_u32("AccentColor"))
            .ok()
            .map(|abgr| {
                Srgb::new(
                    (abgr & 0xff) as f32 / 255.0,
                    ((abgr >> 8) & 0xff) as f32 / 255.0,
                    ((abgr >> 16) & 0xff) as f32 / 255.0,
                )
            }));
        HostColors { dark, accent, ..HostColors::default() }
    }

    /// Picks from `AccentPalette`: eight RGBA entries from lightest to
    /// darkest, the accent itself fourth. WinUI's controls use Light 2 in
    /// dark mode and Dark 1 in light mode.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn accent_shade(palette: &[u8], dark: bool) -> Option<Srgb> {
        let i = if dark { 1 } else { 4 };
        let px = palette.get(i * 4..i * 4 + 3)?;
        Some(Srgb::new(px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0))
    }

    #[cfg(not(windows))]
    pub fn read() -> HostColors {
        HostColors::default()
    }
}

/// macOS: appearance and accent colour from the global user defaults.
mod macos {
    use super::*;

    #[cfg(target_os = "macos")]
    pub fn read() -> HostColors {
        let read = |key: &str| {
            std::process::Command::new("defaults")
                .args(["read", "-g", key])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        };
        // The key only exists in dark mode (Auto included, while it's dark).
        let dark = Some(read("AppleInterfaceStyle").is_some_and(|v| v == "Dark"));
        let accent = accent(read("AppleAccentColor").and_then(|v| v.parse().ok()));
        HostColors { dark, accent: Some(accent), ..HostColors::default() }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn read() -> HostColors {
        HostColors::default()
    }

    /// `AppleAccentColor` is an index into the system palette; unset means
    /// the default "multicolor", which is blue.
    pub fn accent(index: Option<i32>) -> Srgb {
        let hex = match index {
            Some(-1) => 0x8c8c8c, // graphite
            Some(0) => 0xe0383e,  // red
            Some(1) => 0xf7821b,  // orange
            Some(2) => 0xffc600,  // yellow
            Some(3) => 0x62ba46,  // green
            Some(5) => 0xa550a7,  // purple
            Some(6) => 0xf74f9e,  // pink
            _ => 0x007aff,        // blue
        };
        let [_, r, g, b] = u32::to_be_bytes(hex);
        Srgb::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_resolves_per_desktop() {
        assert_eq!(Look::Auto.resolve(Desktop::Kde), Look::Breeze);
        assert_eq!(Look::Auto.resolve(Desktop::Windows), Look::Fluent);
        assert_eq!(Look::Auto.resolve(Desktop::Gnome), Look::Adwaita);
        assert_eq!(Look::Auto.resolve(Desktop::Cosmic), Look::Cosmic);
        assert_eq!(Look::Auto.resolve(Desktop::Other), Look::Cosmic);
        assert_eq!(Look::Breeze.resolve(Desktop::Cosmic), Look::Breeze);
    }

    #[test]
    fn parses_breeze_dark_kdeglobals() {
        let text = "\
[ColorEffects:Disabled]
Color=56,56,56

[Colors:Selection]
BackgroundNormal=61,174,233

[Colors:View]
BackgroundNormal=20,22,24

[Colors:Window]
BackgroundNormal=32,35,38
ForegroundNormal=252,252,252

[General]
ColorScheme=BreezeDark
";
        let c = kde::parse(text);
        assert_eq!(c.dark, Some(true));
        assert_eq!(c.window, Some(rgb(0x202326)));
        assert_eq!(c.view, Some(rgb(0x141618)));
        assert_eq!(c.text, Some(rgb(0xfcfcfc)));
        // No AccentColor key: falls back to the selection colour.
        assert_eq!(c.accent, Some(rgb(0x3daee9)));
    }

    #[test]
    fn kde_accent_color_wins_and_alpha_is_ignored() {
        let text = "[General]\nAccentColor=233,100,58\n[Colors:Selection]\nBackgroundNormal=1,2,3\n[Colors:Window]\nBackgroundNormal=239,240,241,255\n";
        let c = kde::parse(text);
        assert_eq!(c.accent, Some(rgb(0xe9643a)));
        assert_eq!(c.window, Some(rgb(0xeff0f1)));
        assert_eq!(c.dark, Some(false));
    }

    #[test]
    fn corner_override() {
        assert_eq!(Corners::Auto.radii(), None);
        // Round gives COSMIC's pill buttons (buttons use radius_xl).
        assert!(Corners::Round.radii().unwrap().radius_xl[0] > 100.0);
        assert_eq!(Corners::Square.radii().unwrap().radius_xl[0], 2.0);
    }

    #[test]
    fn macos_accent_colors() {
        assert_eq!(macos::accent(None), macos::accent(Some(4)));
        assert_ne!(macos::accent(Some(0)), macos::accent(None));
        assert!(macos::accent(Some(2)).red > 0.99);
    }

    #[test]
    fn windows_accent_shades() {
        // The default blue accent's palette.
        let palette = [
            0x99, 0xeb, 0xff, 0, 0x4c, 0xc2, 0xff, 0, 0x00, 0x91, 0xf8, 0, 0x00, 0x78, 0xd4, 0,
            0x00, 0x67, 0xc0, 0, 0x00, 0x3e, 0x92, 0, 0x00, 0x1a, 0x68, 0, 0xf7, 0x63, 0x0c, 0,
        ];
        assert_eq!(windows::accent_shade(&palette, true), Some(rgb(0x4cc2ff)));
        assert_eq!(windows::accent_shade(&palette, false), Some(rgb(0x0067c0)));
        assert_eq!(windows::accent_shade(&palette[..6], true), None);
    }

    #[test]
    fn titlebar_choice() {
        assert!(Look::Breeze.native_titlebar());
        assert!(Look::Fluent.native_titlebar());
        let drawn_header = !cfg!(target_os = "macos");
        assert_eq!(Look::Adwaita.native_titlebar(), !drawn_header);
        assert_eq!(Look::Cosmic.native_titlebar(), !drawn_header);
    }
}
