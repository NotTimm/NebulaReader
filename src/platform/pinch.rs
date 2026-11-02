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

use cosmic::iced::window;
use std::sync::atomic::{AtomicIsize, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::SystemServices::MK_CONTROL;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_LCONTROL,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, SetWindowLongPtrW, GWLP_WNDPROC, WHEEL_DELTA, WM_MOUSEWHEEL, WNDPROC,
};

/// winit's window procedure, which ours passes every message on to.
static ORIGINAL: AtomicIsize = AtomicIsize::new(0);

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
        let old = unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, proc as usize as isize) };
        ORIGINAL.store(old, Ordering::SeqCst);
    })
    .discard()
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: ORIGINAL holds the WNDPROC that SetWindowLongPtrW returned.
    let original: WNDPROC = unsafe { std::mem::transmute(ORIGINAL.load(Ordering::SeqCst)) };
    let pass = |wparam| unsafe { CallWindowProcW(original, hwnd, msg, wparam, lparam) };

    let ctrl_down = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
    if msg != WM_MOUSEWHEEL || wparam as u32 & MK_CONTROL == 0 || ctrl_down {
        return pass(wparam);
    }

    // A pinch. Never a whole notch, so it reads as a touchpad.
    let delta = pinch_delta((wparam >> 16) as u16 as i16);
    let wparam = (wparam & 0xffff) | ((delta as u16 as usize) << 16);

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

/// A pinch's wheel amount, moved off a whole notch (120) if it's on one;
/// only its direction zooms.
fn pinch_delta(delta: i16) -> i16 {
    if delta != 0 && delta % WHEEL_DELTA as i16 == 0 { delta - delta.signum() } else { delta }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_pinch_is_never_a_whole_notch() {
        assert_eq!(super::pinch_delta(120), 119);
        assert_eq!(super::pinch_delta(-240), -239);
        assert_eq!(super::pinch_delta(37), 37);
        assert_eq!(super::pinch_delta(0), 0);
    }
}
