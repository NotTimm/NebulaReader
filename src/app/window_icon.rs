//! The window's own icon (title bar, taskbar): a closed book in the
//! library, an open one while reading. Desktop files and the Windows exe
//! carry the static closed-book icon; this swaps it on the live window.
//!
//! On Windows the icons embedded in the exe (build.rs) are set straight on
//! the window with `WM_SETICON`, which is what the title bar and taskbar
//! show. Elsewhere it goes through iced: X11 always shows it; on Wayland it
//! needs the compositor to support `xdg-toplevel-icon` (KDE Plasma does),
//! otherwise the desktop file's icon stays. Note libcosmic's iced currently
//! drops these (it hands winit an icon type winit doesn't accept); that
//! needs fixing in the NotTimm/libcosmic fork.

use cosmic::iced::window::{self, Icon};

#[cfg_attr(windows, allow(dead_code))] // Windows uses set_native
static CLOSED: &[u8] = include_bytes!("../../res/icons/window/book.png");
#[cfg_attr(windows, allow(dead_code))] // Windows uses set_native
static OPEN: &[u8] = include_bytes!("../../res/icons/window/book-open.png");

#[cfg_attr(windows, allow(dead_code))] // Windows uses set_native
pub fn load(open: bool) -> Option<Icon> {
    let bytes = if open { OPEN } else { CLOSED };
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    window::icon::from_rgba(image.into_raw(), width, height).ok()
}

/// Windows: sets the window's small (title bar) and large (taskbar,
/// Alt+Tab) icons from the exe's embedded icon resources: 1 is the closed
/// book, 2 the open one.
#[cfg(windows)]
pub fn set_native<T: Send + 'static>(id: window::Id, open: bool) -> cosmic::iced::Task<T> {
    use cosmic::iced::window::raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, LoadImageW, SendMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR,
        SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, WM_SETICON,
    };

    let resource = if open { 2usize } else { 1 };
    window::run_with_handle(id, move |handle| {
        let RawWindowHandle::Win32(win) = handle.as_raw() else { return };
        let hwnd = win.hwnd.get() as *mut core::ffi::c_void;
        // SAFETY: plain Win32 calls on our own live window, with icon
        // resources that build.rs embedded in this exe.
        unsafe {
            let module = GetModuleHandleW(core::ptr::null());
            for (which, cx, cy) in [(ICON_SMALL, SM_CXSMICON, SM_CYSMICON), (ICON_BIG, SM_CXICON, SM_CYICON)] {
                let icon = LoadImageW(
                    module,
                    resource as *const u16, // MAKEINTRESOURCEW
                    IMAGE_ICON,
                    GetSystemMetrics(cx),
                    GetSystemMetrics(cy),
                    LR_DEFAULTCOLOR,
                );
                if !icon.is_null() {
                    SendMessageW(hwnd, WM_SETICON, which as usize, icon as isize);
                }
            }
        }
    })
    .discard()
}

#[cfg(test)]
mod tests {
    #[test]
    fn icons_decode() {
        assert!(super::load(false).is_some());
        assert!(super::load(true).is_some());
    }
}
