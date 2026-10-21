//! Everything that differs between operating systems and desktops: where
//! data lives, how to open a URL, and which desktop we're running on (so
//! the UI can take on its look, see [`look`]).

pub mod look;
pub mod disk;
pub mod status;

use std::path::PathBuf;

/// Folder name used under the per-OS data/config/cache roots.
#[cfg(target_os = "linux")]
const DIR_NAME: &str = "nebula-reader";
#[cfg(not(target_os = "linux"))]
const DIR_NAME: &str = "NebulaReader";

/// The folder name from before the app was renamed (Cosmic Comic).
#[cfg(target_os = "linux")]
const OLD_DIR_NAME: &str = "cosmic-comic";
#[cfg(not(target_os = "linux"))]
const OLD_DIR_NAME: &str = "CosmicComic";

/// `root/<DIR_NAME>`, first moving an existing `root/<OLD_DIR_NAME>` there
/// so a library from before the rename carries over. If the move fails
/// (say, a permissions problem) the old folder keeps being used.
fn app_dir(root: PathBuf) -> PathBuf {
    let new = root.join(DIR_NAME);
    if !new.exists() {
        let old = root.join(OLD_DIR_NAME);
        if old.is_dir() && std::fs::rename(&old, &new).is_err() {
            return old;
        }
    }
    new
}

/// The desktop environment the app is running under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desktop {
    Cosmic,
    Kde,
    Gnome,
    Windows,
    MacOs,
    Other,
}

impl Desktop {
    pub fn detect() -> Self {
        if cfg!(windows) {
            return Desktop::Windows;
        }
        if cfg!(target_os = "macos") {
            return Desktop::MacOs;
        }
        let current = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        Self::from_xdg_current_desktop(&current)
    }

    /// Parses `$XDG_CURRENT_DESKTOP`, a `:`-separated list such as
    /// `ubuntu:GNOME` or `KDE`.
    fn from_xdg_current_desktop(value: &str) -> Self {
        for name in value.split(':').map(str::to_ascii_lowercase) {
            match name.as_str() {
                "cosmic" => return Desktop::Cosmic,
                "kde" => return Desktop::Kde,
                "gnome" | "gnome-classic" | "gnome-flashback" | "unity" | "budgie" | "pantheon" => {
                    return Desktop::Gnome;
                }
                _ => {}
            }
        }
        Desktop::Other
    }
}

/// The user's home directory.
pub fn home_dir() -> PathBuf {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(target_os = "linux")]
fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    let root = std::env::var(var)
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home_dir().join(fallback));
    app_dir(root)
}

#[cfg(windows)]
fn known_dir(var: &str, fallback: &str) -> PathBuf {
    let root = std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home_dir().join(fallback));
    app_dir(root)
}

/// Durable user data — the library database. Never auto-cleared.
/// `~/.local/share/nebula-reader`, `%APPDATA%\NebulaReader`, or
/// `~/Library/Application Support/NebulaReader`.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    return xdg_dir("XDG_DATA_HOME", ".local/share");
    #[cfg(windows)]
    return known_dir("APPDATA", r"AppData\Roaming");
    #[cfg(target_os = "macos")]
    return app_dir(home_dir().join("Library/Application Support"));
}

/// User preferences. On Windows and macOS these sit next to the data.
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    return xdg_dir("XDG_CONFIG_HOME", ".config");
    #[cfg(not(target_os = "linux"))]
    return data_dir();
}

/// Derived artifacts, safe to delete. `~/.cache/nebula-reader`,
/// `%LOCALAPPDATA%\NebulaReader\Cache`, or `~/Library/Caches/NebulaReader`.
pub fn cache_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    return xdg_dir("XDG_CACHE_HOME", ".cache");
    #[cfg(windows)]
    return known_dir("LOCALAPPDATA", r"AppData\Local").join("Cache");
    #[cfg(target_os = "macos")]
    return app_dir(home_dir().join("Library/Caches"));
}

/// Opens a web URL in the default browser.
pub fn open_url(url: &str) {
    #[cfg(windows)]
    let result = {
        use std::os::windows::process::CommandExt;
        // `start` is a cmd built-in; the empty "" is its window title.
        // CREATE_NO_WINDOW stops a console from flashing up.
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(0x0800_0000)
            .spawn()
    };
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();

    if let Err(e) = result {
        log::warn!("could not open {url}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_desktops() {
        assert_eq!(Desktop::from_xdg_current_desktop("COSMIC"), Desktop::Cosmic);
        assert_eq!(Desktop::from_xdg_current_desktop("KDE"), Desktop::Kde);
        assert_eq!(Desktop::from_xdg_current_desktop("ubuntu:GNOME"), Desktop::Gnome);
        assert_eq!(Desktop::from_xdg_current_desktop("Hyprland"), Desktop::Other);
        assert_eq!(Desktop::from_xdg_current_desktop(""), Desktop::Other);
    }
}

/// Moves a file to the system Trash. On a Mac this asks the system
/// directly rather than through Finder, which would first ask the user to
/// let Nebula control Finder (and fail if they said no).
pub fn move_to_trash(path: &std::path::Path) -> Result<(), trash::Error> {
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut ctx = trash::TrashContext::default();
        ctx.set_delete_method(DeleteMethod::NsFileManager);
        ctx.delete(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        trash::delete(path)
    }
}
