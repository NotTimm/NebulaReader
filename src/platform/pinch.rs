//! Windows: touchpad pinch-to-zoom.
//!
//! A precision touchpad's pinch reaches an app that doesn't use Direct
//! Manipulation as mouse-wheel messages with `MK_CONTROL` set in the
//! message, while the Ctrl key itself isn't down. winit reads modifiers
//! from the keyboard state (`GetKeyState`), not from the message, so the
//! viewer saw a plain two-finger scroll: it panned (or did nothing on a
//! whole page) instead of zooming.
//!
//! This hooks the window procedure. For a wheel message flagged Ctrl while
//! Ctrl isn't held, it marks Ctrl as down in this thread's keyboard state
//! for just that message, so winit reports Ctrl + wheel, and makes sure the
//! amount isn't a whole notch, so the viewer takes it as a touchpad's
//! (Ctrl + touchpad zooms; Ctrl + a mouse wheel scrolls). The next mouse
//! move or key press gives winit the real modifiers again. A real Ctrl +
//! wheel is left as it is.
//!
//! Touchpad or mouse wheel: the viewer tells them apart by whether a
//! scroll is a whole number of lines. winit hands on a wheel message's
//! amount over 120, times the system's "lines per notch" (3 by default),
//! so a touchpad's 40 became exactly 1 line and read as a mouse wheel
//! (a pinch then scrolled instead of zooming). Any amount that isn't whole
//! notches is moved by the least it takes to stay fractional after that.
//!
//! `NEBULA_WHEEL_LOG=<file>` writes every wheel message seen to that file,
//! to see what a touchpad sends.

use cosmic::iced::window;
use std::sync::atomic::{AtomicIsize, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::SystemServices::MK_CONTROL;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_LCONTROL,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, SetWindowLongPtrW, SystemParametersInfoW, GWLP_WNDPROC, SPI_GETWHEELSCROLLLINES, WHEEL_DELTA,
    WM_MOUSEHWHEEL, WM_MOUSEWHEEL, WNDPROC,
};

/// winit's window procedure, which ours passes every message on to.
static ORIGINAL: AtomicIsize = AtomicIsize::new(0);

/// Whether the window procedure is hooked yet.
pub fn hooked() -> bool {
    ORIGINAL.load(Ordering::SeqCst) != 0
}

/// The system's lines per wheel notch, as winit reads it (3 when unset or
/// set to scroll pages).
fn wheel_lines() -> i32 {
    let mut lines: u32 = 3;
    // SAFETY: SPI_GETWHEELSCROLLLINES writes one u32.
    let ok = unsafe { SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, &mut lines as *mut u32 as *mut _, 0) };
    // WHEEL_PAGESCROLL (u32::MAX) scrolls pages; winit counts that as 3.
    if ok == 0 || lines == u32::MAX { 3 } else { lines as i32 }
}

/// `NEBULA_WHEEL_LOG`: appends a line about a wheel message.
fn log_wheel(line: &str) {
    use std::io::Write;
    static PATH: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    let Some(path) = PATH.get_or_init(|| std::env::var_os("NEBULA_WHEEL_LOG").map(Into::into)) else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

/// Hooks the main window once it exists.
pub fn hook<T: Send + 'static>(id: window::Id) -> cosmic::iced::Task<T> {
    use cosmic::iced::window::raw_window_handle::RawWindowHandle;
    window::run_with_handle(id, |handle| {
        let RawWindowHandle::Win32(win) = handle.as_raw() else { return };
        if ORIGINAL.load(Ordering::SeqCst) != 0 {
            return;
        }
        let hwnd = win.hwnd.get() as HWND;
        // SAFETY: our own live window, on its own thread; the old procedure
        // is kept and every message goes on to it.
        let old = unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, proc as *const () as isize) };
        ORIGINAL.store(old, Ordering::SeqCst);
    })
    .discard()
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: ORIGINAL holds the WNDPROC that SetWindowLongPtrW returned.
    let original: WNDPROC = unsafe { std::mem::transmute(ORIGINAL.load(Ordering::SeqCst)) };
    let pass = |wparam| unsafe { CallWindowProcW(original, hwnd, msg, wparam, lparam) };

    if msg != WM_MOUSEWHEEL && msg != WM_MOUSEHWHEEL {
        return pass(wparam);
    }
    let ctrl_down = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
    let flagged = wparam as u32 & MK_CONTROL != 0;
    let pinch = msg == WM_MOUSEWHEEL && flagged && !ctrl_down;
    let raw = (wparam >> 16) as u16 as i16;
    let lines = wheel_lines();
    // A touchpad's (or a pinch's) amount stays fractional in lines.
    let delta = if pinch || raw % WHEEL_DELTA as i16 != 0 { touchpad_delta(raw, lines) } else { raw };
    log_wheel(&format!(
        "{} raw={raw} sent={delta} lines={lines} mk_control={flagged} ctrl_held={ctrl_down} pinch={pinch}",
        if msg == WM_MOUSEWHEEL { "wheel" } else { "hwheel" }
    ));
    let wparam = (wparam & 0xffff) | ((delta as u16 as usize) << 16);
    if !pinch {
        return pass(wparam);
    }

    let mut keys = [0u8; 256];
    unsafe {
        if GetKeyboardState(keys.as_mut_ptr()) == 0 {
            return pass(wparam);
        }
        let saved = keys;
        keys[VK_CONTROL as usize] |= 0x80;
        keys[VK_LCONTROL as usize] |= 0x80;
        SetKeyboardState(keys.as_ptr());
        let result = pass(wparam);
        SetKeyboardState(saved.as_ptr());
        result
    }
}

/// A touchpad's wheel amount, moved toward zero by the least it takes so
/// that winit's `amount / 120 * lines` isn't a whole number of lines (only
/// a mouse wheel's is). Never 0: a pinch's direction is what zooms.
fn touchpad_delta(delta: i16, lines: i32) -> i16 {
    if delta == 0 || lines <= 0 {
        return delta;
    }
    let whole = |d: i16| (d as i32 * lines) % WHEEL_DELTA as i32 == 0;
    let mut d = delta;
    while d != 0 && whole(d) {
        d -= d.signum();
    }
    if d == 0 { delta.signum() } else { d }
}

#[cfg(test)]
mod tests {
    use super::touchpad_delta;

    #[test]
    fn a_touchpad_amount_is_never_whole_lines() {
        // 3 lines a notch: 40 would be exactly 1 line.
        assert_eq!(touchpad_delta(40, 3), 39);
        assert_eq!(touchpad_delta(-80, 3), -79);
        assert_eq!(touchpad_delta(120, 3), 119);
        assert_eq!(touchpad_delta(37, 3), 37);
        assert_eq!(touchpad_delta(120, 1), 119);
        assert_eq!(touchpad_delta(0, 3), 0);
        for lines in 1..=10 {
            for d in [-240i16, -121, -40, -1, 1, 12, 24, 40, 60, 119, 120, 360] {
                let out = touchpad_delta(d, lines);
                assert_ne!((out as i32 * lines) % 120, 0, "{d} at {lines} lines");
                assert_eq!(out.signum(), d.signum());
            }
        }
    }
}
