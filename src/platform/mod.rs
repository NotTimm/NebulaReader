//! Everything that differs between operating systems and desktops: where
//! data lives, how to open a URL, and which desktop we're running on (so
//! the UI can take on its look, see [`look`]).

pub mod look;
pub mod disk;
pub mod status;
#[cfg(windows)]
pub mod pinch;

use std::path::PathBuf;

/// Folder name used under the per-OS data/config/cache roots.
#[cfg(target_os = "linux")]
const DIR_NAME: &str = "nebula-reader";
#[cfg(not(target_os = "linux"))]
const DIR_NAME: &str = "NebulaReader";

/// The beta's own folder name (see `crate::updates::beta_data`).
#[cfg(target_os = "linux")]
const BETA_DIR_NAME: &str = "nebula-reader-beta";
#[cfg(not(target_os = "linux"))]
const BETA_DIR_NAME: &str = "NebulaReader Beta";

/// The folder name from before the app was renamed (Cosmic Comic).
#[cfg(target_os = "linux")]
const OLD_DIR_NAME: &str = "cosmic-comic";
#[cfg(not(target_os = "linux"))]
const OLD_DIR_NAME: &str = "CosmicComic";

/// `root/<DIR_NAME>`, first moving an existing `root/<OLD_DIR_NAME>` there
/// so a library from before the rename carries over. If the move fails
/// (say, a permissions problem) the old folder keeps being used.
fn app_dir(root: PathBuf) -> PathBuf {
    if crate::updates::beta_data() {
        return root.join(BETA_DIR_NAME);
    }
    regular_app_dir(root)
}

/// Whether a beta has made its own data folder here: this install has run
/// a beta (see `crate::updates::channel`).
pub fn beta_data_exists() -> bool {
    data_root().join(BETA_DIR_NAME).is_dir()
}

/// The regular (not beta) version's folder under `root`.
fn regular_app_dir(root: PathBuf) -> PathBuf {
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
fn xdg_root(var: &str, fallback: &str) -> PathBuf {
    std::env::var(var)
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home_dir().join(fallback))
}

#[cfg(windows)]
fn known_root(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home_dir().join(fallback))
}

/// Where the data folder goes.
fn data_root() -> PathBuf {
    #[cfg(target_os = "linux")]
    return xdg_root("XDG_DATA_HOME", ".local/share");
    #[cfg(windows)]
    return known_root("APPDATA", r"AppData\Roaming");
    #[cfg(target_os = "macos")]
    return home_dir().join("Library/Application Support");
}

/// Where the settings folder goes (the data folder's, off Linux).
fn config_root() -> PathBuf {
    #[cfg(target_os = "linux")]
    return xdg_root("XDG_CONFIG_HOME", ".config");
    #[cfg(not(target_os = "linux"))]
    return data_root();
}

/// Durable user data — the library database. Never auto-cleared.
/// `~/.local/share/nebula-reader`, `%APPDATA%\NebulaReader`, or
/// `~/Library/Application Support/NebulaReader`; a beta's are
/// `nebula-reader-beta` / `NebulaReader Beta`.
pub fn data_dir() -> PathBuf {
    app_dir(data_root())
}

/// User preferences. On Windows and macOS these sit next to the data.
pub fn config_dir() -> PathBuf {
    app_dir(config_root())
}

/// The regular version's settings and data folders, whichever version is
/// running: where the update channel is kept, and what a beta's data is
/// first copied from.
pub fn regular_config_dir() -> PathBuf {
    regular_app_dir(config_root())
}

pub fn regular_data_dir() -> PathBuf {
    regular_app_dir(data_root())
}

/// Derived artifacts, safe to delete. `~/.cache/nebula-reader`,
/// `%LOCALAPPDATA%\NebulaReader\Cache`, or `~/Library/Caches/NebulaReader`.
pub fn cache_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    return app_dir(xdg_root("XDG_CACHE_HOME", ".cache"));
    #[cfg(windows)]
    return app_dir(known_root("LOCALAPPDATA", r"AppData\Local")).join("Cache");
    #[cfg(target_os = "macos")]
    return app_dir(home_dir().join("Library/Caches"));
}

/// The first time a beta runs with its own data (`crate::updates::beta_data`),
/// its folders start as a copy of the regular version's: library, reading
/// progress, settings and sign-ins. Downloaded files aren't copied (they
/// can be big, and the beta mustn't delete the regular version's), so the
/// copy's list of downloads is emptied, and catalogs' downloaded books with
/// it. Returns whether it copied.
pub fn prepare_beta_data() -> bool {
    if !crate::updates::beta_data() {
        return false;
    }
    let (data, regular_data) = (data_dir(), regular_data_dir());
    let (config, regular_config) = (config_dir(), regular_config_dir());
    if data.exists() || !regular_data.is_dir() {
        return false;
    }
    let skip = [crate::library::DOWNLOADS_DIR, crate::library::OLD_DOWNLOADS_DIR];
    if let Err(e) = copy_tree(&regular_data, &data, &skip) {
        log::warn!("beta: copying {} failed: {e}", regular_data.display());
        let _ = std::fs::remove_dir_all(&data);
        return false;
    }
    if config != data && regular_config.is_dir() && !config.exists() {
        if let Err(e) = copy_tree(&regular_config, &config, &skip) {
            log::warn!("beta: copying {} failed: {e}", regular_config.display());
        }
    }
    // The channel choice stays in the regular folder only.
    let _ = std::fs::remove_file(config.join("update-channel"));
    true
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path, skip: &[&str]) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if skip.iter().any(|s| name == *s) {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        if entry.file_type()?.is_dir() {
            copy_tree(&src, &dst, skip)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
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
    fn copies_a_folder_without_its_downloads() {
        let root = std::env::temp_dir().join(format!("nebula-copy-{}", std::process::id()));
        let (from, to) = (root.join("regular"), root.join("beta"));
        std::fs::create_dir_all(from.join("downloads/acct")).unwrap();
        std::fs::create_dir_all(from.join("covers")).unwrap();
        std::fs::write(from.join("library.db"), b"db").unwrap();
        std::fs::write(from.join("covers/a.jpg"), b"jpg").unwrap();
        std::fs::write(from.join("downloads/acct/b.cbz"), b"big").unwrap();
        copy_tree(&from, &to, &["downloads"]).unwrap();
        assert_eq!(std::fs::read(to.join("library.db")).unwrap(), b"db");
        assert_eq!(std::fs::read(to.join("covers/a.jpg")).unwrap(), b"jpg");
        assert!(!to.join("downloads").exists());
        // The regular folder is left as it was.
        assert!(from.join("downloads/acct/b.cbz").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn detects_desktops() {
        assert_eq!(Desktop::from_xdg_current_desktop("COSMIC"), Desktop::Cosmic);
        assert_eq!(Desktop::from_xdg_current_desktop("KDE"), Desktop::Kde);
        assert_eq!(Desktop::from_xdg_current_desktop("ubuntu:GNOME"), Desktop::Gnome);
        assert_eq!(Desktop::from_xdg_current_desktop("Hyprland"), Desktop::Other);
        assert_eq!(Desktop::from_xdg_current_desktop(""), Desktop::Other);
    }
}
