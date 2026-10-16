//! The window's own icon (title bar, taskbar): a closed book in the
//! library, an open one while reading. Desktop files and the Windows exe
//! carry the static closed-book icon; this swaps it on the live window.
//!
//! Windows and X11 always show it. On Wayland it needs the compositor to
//! support `xdg-toplevel-icon` (KDE Plasma does); elsewhere it's a no-op
//! and the desktop file's icon stays.

use cosmic::iced::window::{self, Icon};

static CLOSED: &[u8] = include_bytes!("../../res/icons/window/book.png");
static OPEN: &[u8] = include_bytes!("../../res/icons/window/book-open.png");

pub fn load(open: bool) -> Option<Icon> {
    let bytes = if open { OPEN } else { CLOSED };
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    window::icon::from_rgba(image.into_raw(), width, height).ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn icons_decode() {
        assert!(super::load(false).is_some());
        assert!(super::load(true).is_some());
    }
}
