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
//! The host's colours are read once at startup ([`init`]) and cached, so
//! switching looks in Settings never blocks on D-Bus or the registry.

use std::sync::{Arc, OnceLock};

use cosmic::cosmic_theme::palette::{Srgb, Srgba};
use cosmic::cosmic_theme::{CornerRadii, ThemeBuilder};
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
    /// of libcosmic's header bar with window buttons.
    pub fn native_titlebar(self) -> bool {
        matches!(self, Look::Breeze | Look::Fluent)
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

static HOST: OnceLock<HostColors> = OnceLock::new();

/// Reads the host desktop's colours. Call once, before the app starts
/// (it may briefly block on the settings portal or registry).
pub fn init(desktop: Desktop) {
    let _ = HOST.set(read_host_colors(desktop));
}

fn host() -> &'static HostColors {
    HOST.get_or_init(HostColors::default)
}

/// Builds the libcosmic theme for an already-resolved look.
pub fn theme(look: Look) -> cosmic::Theme {
    let host = host();
    let preset = match look {
        Look::Auto | Look::Cosmic => return cosmic::theme::system_preference(),
        Look::Breeze => Preset::BREEZE,
        Look::Fluent => Preset::FLUENT,
        Look::Adwaita => Preset::ADWAITA,
    };
    let dark = host
        .dark
        .or_else(|| host.window.map(|c| luminance(c) < 0.5))
        .unwrap_or(preset.default_dark);
    let colors = if dark { &preset.dark } else { &preset.light };

    let builder = if dark { ThemeBuilder::dark() } else { ThemeBuilder::light() };
    let built = builder
        .corner_radii(preset.radii)
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
        Desktop::Cosmic | Desktop::MacOs => HostColors::default(),
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
        // Stored as 0xAABBGGRR.
        let accent = CURRENT_USER
            .open(r"Software\Microsoft\Windows\DWM")
            .and_then(|k| k.get_u32("AccentColor"))
            .ok()
            .map(|abgr| {
                Srgb::new(
                    (abgr & 0xff) as f32 / 255.0,
                    ((abgr >> 8) & 0xff) as f32 / 255.0,
                    ((abgr >> 16) & 0xff) as f32 / 255.0,
                )
            });
        HostColors { dark, accent, ..HostColors::default() }
    }

    #[cfg(not(windows))]
    pub fn read() -> HostColors {
        HostColors::default()
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
    fn titlebar_choice() {
        assert!(Look::Breeze.native_titlebar());
        assert!(Look::Fluent.native_titlebar());
        assert!(!Look::Adwaita.native_titlebar());
        assert!(!Look::Cosmic.native_titlebar());
    }
}
