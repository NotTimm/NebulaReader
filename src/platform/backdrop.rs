//! Windows 11's backdrop behind the window: Mica (the wallpaper, blurred
//! and tinted, as in Settings and File Explorer) or Acrylic (frosted glass
//! over whatever is behind the window). Windows only; nothing here runs on
//! COSMIC or anywhere else.
//!
//! How it fits together:
//! - [`start`] (at the top of `main`, before any thread) decides whether
//!   the backdrop is on: picked in Settings, Windows' own Transparency
//!   effects switch on, Windows 11 22H2 or later (build 22621), and drawing
//!   through Direct3D 12. If so it asks wgpu to present through
//!   DirectComposition (`WGPU_DX12_PRESENTATION_SYSTEM=Visual`), the only
//!   way a Direct3D 12 window can be see-through.
//! - `main` then creates the window transparent, with the backdrop and no
//!   GDI redirection surface (fork patch 19: `Settings::windows_window`).
//! - `look::theme` makes the window's own background see-through
//!   ([`BACKGROUND_ALPHA`] of its colour), so the backdrop shows wherever
//!   Nebula draws nothing of its own, as frosted glass does on COSMIC.
//! - [`extend_frame`] (when the window opens) lets the backdrop reach into
//!   the window's client area, not only its title bar.
//!
//! The choice takes effect at the next start: the drawing surface is made
//! once, when the window is.

use std::sync::atomic::{AtomicBool, Ordering};

use cosmic::iced::window;
use cosmic::iced::window::settings::platform::{PlatformSpecific, SystemBackdrop};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows_sys::Win32::UI::Controls::MARGINS;

use crate::settings::Backdrop;

/// How much of the theme's window colour stays over the backdrop: enough
/// to keep a colour theme's look, little enough for Mica to show.
pub const BACKGROUND_ALPHA: f32 = 0.3;
/// Translucent alpha for elevated card surfaces (wiki cards, OPDS cards,
/// banners) layered over the Windows 11 Mica/Acrylic backdrop.
pub const CARD_ALPHA_DARK: f32 = 0.45;
pub const CARD_ALPHA_LIGHT: f32 = 0.58;

/// The first Windows 11 build with system backdrops (22H2).
const FIRST_BUILD: u32 = 22621;

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether the window has a backdrop this run.
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Decides whether this run has a backdrop (see the module notes) and, if
/// so, sets up Direct3D 12 for it and returns the window's settings.
/// Call before any thread starts, after `gpu_defaults`.
pub fn start(choice: Backdrop) -> Option<PlatformSpecific> {
    let backdrop = match choice {
        Backdrop::Solid => return None,
        Backdrop::Mica => SystemBackdrop::Mica,
        Backdrop::Acrylic => SystemBackdrop::Acrylic,
    };
    // Someone who picked another backend (WGPU_BACKEND=vulkan to compare)
    // gets a plain window: only Direct3D 12 can be see-through here.
    let dx12 = std::env::var("WGPU_BACKEND").is_ok_and(|b| b.eq_ignore_ascii_case("dx12"));
    if !dx12 || !transparency_on() || windows_build().is_none_or(|b| b < FIRST_BUILD) {
        log::info!("backdrop off (dx12 {dx12}, transparency {}, build {:?})", transparency_on(), windows_build());
        return None;
    }
    // WGPU_DX12_PRESENTATION_SYSTEM=Hwnd is the way out if the see-through
    // surface misbehaves: a plain window, as before.
    match std::env::var("WGPU_DX12_PRESENTATION_SYSTEM") {
        Ok(v) if !matches!(v.to_ascii_lowercase().as_str(), "visual" | "dxgifromvisual") => {
            log::info!("backdrop off (WGPU_DX12_PRESENTATION_SYSTEM={v})");
            return None;
        }
        Ok(_) => {}
        // SAFETY (set_var): called at the top of `main`, before any thread.
        Err(_) => std::env::set_var("WGPU_DX12_PRESENTATION_SYSTEM", "Visual"),
    }
    ACTIVE.store(true, Ordering::Relaxed);
    log::info!("backdrop {backdrop:?}");
    Some(PlatformSpecific { system_backdrop: backdrop, no_redirection_bitmap: true, ..PlatformSpecific::default() })
}

/// Windows' Settings → Personalization → Colors → Transparency effects
/// (also turned off by battery saver). On when it can't be read, as it is
/// by default.
pub fn transparency_on() -> bool {
    windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_u32("EnableTransparency"))
        .map_or(true, |v| v != 0)
}

/// The Windows build number (22631 for Windows 11 23H2).
fn windows_build() -> Option<u32> {
    windows_registry::LOCAL_MACHINE
        .open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|k| k.get_string("CurrentBuildNumber"))
        .ok()
        .and_then(|b| b.trim().parse().ok())
}

/// Extends the window frame over the whole client area, so the backdrop
/// is drawn behind all of it (where Nebula draws see-through), not just
/// the title bar.
pub fn extend_frame<T: Send + 'static>(id: window::Id) -> cosmic::iced::Task<T> {
    use cosmic::iced::window::raw_window_handle::RawWindowHandle;
    window::run_with_handle(id, |handle| {
        let RawWindowHandle::Win32(win) = handle.as_raw() else { return };
        let hwnd = win.hwnd.get() as HWND;
        // -1 on every side: the whole window.
        let margins = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
        // SAFETY: our own live window; the margins outlive the call.
        let hr = unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) };
        if hr < 0 {
            log::warn!("DwmExtendFrameIntoClientArea failed: 0x{hr:08x}");
        }
    })
    .discard()
}
