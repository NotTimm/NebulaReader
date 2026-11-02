//! What version this is, how it was installed, and whether GitHub has a
//! newer release.
//!
//! The check is one unauthenticated request to GitHub's releases API. The
//! Windows installer, the Windows zip and the Mac app can update themselves
//! (`crate::updater`); every other install updates through its own channel
//! (Flathub, a .flatpak file, rpm/deb, AppImage, or built from source), so
//! the About section says what to do for that install.

use std::sync::OnceLock;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO_URL: &str = "https://github.com/NotTimm/NebulaReader";
pub const RELEASES_URL: &str = "https://github.com/NotTimm/NebulaReader/releases";
const LATEST_API: &str = "https://api.github.com/repos/NotTimm/NebulaReader/releases/latest";
/// The newest releases, betas (pre-releases) included.
const RELEASES_API: &str = "https://api.github.com/repos/NotTimm/NebulaReader/releases?per_page=30";

/// The commit this was built from (set by build.rs), if it was built from a
/// git checkout.
pub fn build_commit() -> Option<&'static str> {
    option_env!("NEBULA_COMMIT").filter(|c| !c.is_empty())
}

/// Set by a distribution's package build, for example
/// `NEBULA_DISTRO="Fedora package"`: that distro's repositories bring new
/// versions, so the app doesn't ask GitHub, and About shows this label.
pub fn distro() -> Option<&'static str> {
    option_env!("NEBULA_DISTRO").filter(|d| !d.is_empty())
}

/// How this copy was installed, as far as we can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Install {
    Flatpak,
    AppImage,
    Rpm,
    Deb,
    /// Built by a distribution for its repositories (`distro`).
    Distro,
    /// Under ~/.local, from install.sh.
    Source,
    /// Per-user install from the setup .exe or winget.
    WindowsInstaller,
    WindowsPortable,
    MacApp,
    Unknown,
}

impl Install {
    pub fn detect() -> Self {
        if distro().is_some() {
            return Install::Distro;
        }
        if std::env::var_os("FLATPAK_ID").is_some() || std::path::Path::new("/.flatpak-info").exists() {
            return Install::Flatpak;
        }
        if std::env::var_os("APPIMAGE").is_some() {
            return Install::AppImage;
        }
        let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        if cfg!(windows) {
            // Inno Setup leaves its uninstaller next to the exe, for
            // per-user and all-users installs alike.
            let installed = std::env::current_exe()
                .ok()
                .and_then(|p| Some(p.parent()?.join("unins000.exe").exists()))
                .unwrap_or(false);
            return if installed || exe.to_ascii_lowercase().contains("\\appdata\\local\\programs\\") {
                Install::WindowsInstaller
            } else {
                Install::WindowsPortable
            };
        }
        if cfg!(target_os = "macos") {
            return if exe.contains(".app/Contents/MacOS/") { Install::MacApp } else { Install::Unknown };
        }
        if exe.starts_with("/usr/") {
            if std::path::Path::new("/etc/debian_version").exists() {
                return Install::Deb;
            }
            if std::path::Path::new("/usr/bin/rpm").exists() {
                return Install::Rpm;
            }
        }
        if let Some(home) = std::env::var_os("HOME") {
            if exe.starts_with(&*std::path::Path::new(&home).join(".local").to_string_lossy()) {
                return Install::Source;
            }
        }
        Install::Unknown
    }

    /// One line on how to get a newer version for this install.
    pub fn how_to_update(self) -> String {
        match self {
            Install::Flatpak => fl!("how-flatpak"),
            Install::AppImage => fl!("how-appimage"),
            Install::Rpm => fl!("how-rpm"),
            Install::Deb => fl!("how-deb"),
            Install::Distro => fl!("how-distro"),
            Install::Source => fl!("how-source"),
            Install::WindowsInstaller => fl!("how-windows-installer"),
            Install::WindowsPortable => fl!("how-windows-portable"),
            Install::MacApp => fl!("how-mac"),
            Install::Unknown => fl!("how-unknown"),
        }
    }

    /// Whether this install looks for new releases on GitHub. A distro's
    /// package waits for its repositories, which may lag behind.
    pub fn checks_github(self) -> bool {
        self != Install::Distro
    }

    pub fn label(self) -> String {
        match self {
            Install::Flatpak => "Flatpak".into(),
            Install::AppImage => "AppImage".into(),
            Install::Rpm => fl!("install-rpm"),
            Install::Deb => fl!("install-deb"),
            Install::Distro => distro().map_or_else(|| fl!("install-distro"), str::to_string),
            Install::Source => fl!("install-source"),
            Install::WindowsInstaller => fl!("install-windows-installer"),
            Install::WindowsPortable => fl!("install-windows-portable"),
            Install::MacApp => fl!("install-mac"),
            Install::Unknown => fl!("install-unknown"),
        }
    }
}

/// The newest release on GitHub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
    /// The files attached to the release.
    pub assets: Vec<Asset>,
    /// Not newer: the regular version to go back to from a beta, after
    /// beta versions were turned off.
    pub back_to_regular: bool,
}

/// A file attached to a release.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct Asset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub url: String,
}

#[derive(serde::Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
    #[serde(default)]
    draft: bool,
}

impl GhRelease {
    fn version(&self) -> String {
        self.tag_name.trim_start_matches('v').to_string()
    }

    fn into_release(self, back_to_regular: bool) -> Release {
        Release { version: self.version(), url: self.html_url, assets: self.assets, back_to_regular }
    }
}

// ── Beta versions ───────────────────────────────────────────────────────────
//
// The installs that update themselves (Windows installer and zip, Mac app)
// can follow the betas. It's one install either way: turning betas on
// updates it to the newest beta, turning them off puts the newest regular
// version back over it. A beta keeps its own data (`beta_data`), made as a
// copy of the regular data the first time, so a beta can't harm the real
// library, and going back finds it as it was.

/// Which releases an install updates to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Regular,
    Beta,
}

/// A beta (pre-release) version: `0.2.0-beta.1`.
pub fn is_prerelease(version: &str) -> bool {
    version.contains('-')
}

/// The choice, kept next to the regular settings so a beta and a regular
/// version of the app read the same one.
fn channel_file() -> std::path::PathBuf {
    crate::platform::regular_config_dir().join("update-channel")
}

/// The channel chosen in Settings → About; a beta nobody chose for (a beta
/// installed by hand) follows the betas.
pub fn channel() -> Channel {
    match std::fs::read_to_string(channel_file()).ok().as_deref().map(str::trim) {
        Some("beta") => Channel::Beta,
        Some("regular") => Channel::Regular,
        _ if is_prerelease(VERSION) => Channel::Beta,
        _ => Channel::Regular,
    }
}

pub fn set_channel(channel: Channel) -> Result<(), String> {
    let path = channel_file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, if channel == Channel::Beta { "beta\n" } else { "regular\n" }).map_err(|e| e.to_string())
}

/// Whether this install can follow the betas (it updates itself).
pub fn can_follow_betas(install: Install) -> bool {
    matches!(install, Install::WindowsInstaller | Install::WindowsPortable | Install::MacApp)
}

/// Whether this run uses the beta's own data folder: a beta version, or
/// betas turned on, on an install that can follow them. Decided once at
/// start, so a folder never changes under a running app.
pub fn beta_data() -> bool {
    static B: OnceLock<bool> = OnceLock::new();
    *B.get_or_init(|| can_follow_betas(Install::detect()) && (is_prerelease(VERSION) || channel() == Channel::Beta))
}

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            // GitHub's API refuses requests without a User-Agent.
            .user_agent(concat!("nebula-reader/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("http client")
    })
}

async fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let resp = client()
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| fl!("github-unreachable", error = e.to_string()))?;
    if !resp.status().is_success() {
        return Err(fl!("github-http", status = resp.status().to_string()));
    }
    resp.json().await.map_err(|e| e.to_string())
}

/// What to update to, or `None` when this is already it: the newest
/// regular release, or with betas on the newest release of any kind. A
/// beta with betas turned off is offered the newest regular release even
/// though its number is lower.
pub async fn check() -> Result<Option<Release>, String> {
    if channel() == Channel::Beta && can_follow_betas(Install::detect()) {
        let releases: Vec<GhRelease> = get_json(RELEASES_API).await?;
        return Ok(pick(releases, Channel::Beta, VERSION));
    }
    let latest: GhRelease = get_json(LATEST_API).await?;
    Ok(pick(vec![latest], Channel::Regular, VERSION))
}

/// The release to offer from `releases` (see [`check`]).
fn pick(releases: Vec<GhRelease>, channel: Channel, current: &str) -> Option<Release> {
    let mut candidates = releases.into_iter().filter(|r| !r.draft);
    let candidates: Vec<GhRelease> = match channel {
        Channel::Beta => candidates.collect(),
        Channel::Regular => candidates.find(|r| !is_prerelease(&r.version())).into_iter().collect(),
    };
    let newest = candidates.into_iter().reduce(|a, b| if is_newer(&b.version(), &a.version()) { b } else { a })?;
    let version = newest.version();
    if is_newer(&version, current) {
        return Some(newest.into_release(false));
    }
    // Back from a beta to the regular version.
    (channel == Channel::Regular && is_prerelease(current) && version != current).then(|| newest.into_release(true))
}

/// Whether `candidate` is a later version than `current` (semver-style:
/// `0.1.10` > `0.1.9`, and `0.2.0` > `0.2.0-beta.3`).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parse(v: &str) -> Option<([u64; 3], Option<&str>)> {
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (v, None),
        };
        let mut nums = [0u64; 3];
        for (i, part) in core.split('.').enumerate() {
            *nums.get_mut(i)? = part.parse().ok()?;
        }
        Some((nums, pre))
    }
    let (Some((a, a_pre)), Some((b, b_pre))) = (parse(candidate), parse(current)) else { return false };
    match a.cmp(&b) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match (a_pre, b_pre) {
            (None, Some(_)) => true,
            (Some(x), Some(y)) => pre_newer(x, y),
            _ => false,
        },
    }
}

/// Compares pre-release tags like `beta.3` and `beta.10` part by part.
fn pre_newer(a: &str, b: &str) -> bool {
    for (x, y) in a.split('.').zip(b.split('.')) {
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            _ => x.cmp(y),
        };
        if ord != std::cmp::Ordering::Equal {
            return ord == std::cmp::Ordering::Greater;
        }
    }
    a.split('.').count() > b.split('.').count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gh(tag: &str) -> GhRelease {
        GhRelease { tag_name: format!("v{tag}"), html_url: String::new(), assets: Vec::new(), draft: false }
    }

    #[test]
    fn picks_what_to_update_to() {
        let all = || vec![gh("0.2.0-beta.2"), gh("0.2.0-beta.1"), gh("0.1.16"), gh("0.1.15")];
        let picked = |releases, channel, current| pick(releases, channel, current).map(|r| (r.version, r.back_to_regular));
        // Regular: only regular releases, only newer ones.
        assert_eq!(picked(vec![gh("0.1.16")], Channel::Regular, "0.1.15"), Some(("0.1.16".into(), false)));
        assert_eq!(picked(vec![gh("0.1.16")], Channel::Regular, "0.1.16"), None);
        // Betas: the newest of everything.
        assert_eq!(picked(all(), Channel::Beta, "0.1.16"), Some(("0.2.0-beta.2".into(), false)));
        assert_eq!(picked(all(), Channel::Beta, "0.2.0-beta.1"), Some(("0.2.0-beta.2".into(), false)));
        assert_eq!(picked(all(), Channel::Beta, "0.2.0-beta.2"), None);
        // The regular release after the betas is offered to beta users too.
        assert_eq!(picked(vec![gh("0.2.0"), gh("0.2.0-beta.2")], Channel::Beta, "0.2.0-beta.2"), Some(("0.2.0".into(), false)));
        // Betas turned off on a beta: back to the regular one, though older.
        assert_eq!(picked(vec![gh("0.1.16")], Channel::Regular, "0.2.0-beta.2"), Some(("0.1.16".into(), true)));
        // Once the regular release is out, its betas are never offered on
        // it, whatever order GitHub lists them in, with betas on or off.
        let after = || vec![gh("0.2.0-beta.4"), gh("0.2.0"), gh("0.2.0-beta.3"), gh("0.1.16")];
        assert_eq!(picked(after(), Channel::Beta, "0.2.0"), None);
        assert_eq!(picked(vec![gh("0.2.0")], Channel::Regular, "0.2.0"), None);
        // A beta of the next version is (betas on only).
        assert_eq!(picked(vec![gh("0.2.1-beta.1"), gh("0.2.0")], Channel::Beta, "0.2.0"), Some(("0.2.1-beta.1".into(), false)));
        assert_eq!(picked(vec![gh("0.2.0")], Channel::Regular, "0.2.0"), None);
        // A draft is never offered.
        let mut draft = gh("0.3.0");
        draft.draft = true;
        assert_eq!(picked(vec![draft, gh("0.1.16")], Channel::Beta, "0.1.16"), None);
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.1.3", "0.1.2"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("0.2.0", "0.1.12"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.2", "0.1.2"));
        assert!(!is_newer("0.1.1", "0.1.2"));
        assert!(is_newer("0.2.0", "0.2.0-beta.3"));
        assert!(!is_newer("0.2.0-beta.3", "0.2.0"));
        assert!(is_newer("0.1.0-beta.10", "0.1.0-beta.9"));
        assert!(!is_newer("garbage", "0.1.2"));
    }
}
