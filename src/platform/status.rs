//! Battery level and the clock's 12/24-hour setting, for the status shown
//! while reading in full screen.

use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

/// The laptop's battery, or `None` on a machine without one (or when it
/// can't be read).
pub fn battery() -> Option<Battery> {
    #[cfg(target_os = "linux")]
    return linux_battery(Path::new("/sys/class/power_supply"));
    #[cfg(windows)]
    return windows_battery();
    #[cfg(target_os = "macos")]
    return macos_battery();
}

/// The time as the system shows it: `14:05` or `2:05 PM`.
pub fn clock() -> String {
    let format = if uses_24_hour() { "%H:%M" } else { "%-I:%M %p" };
    chrono::Local::now().format(format).to_string()
}

fn uses_24_hour() -> bool {
    static H24: OnceLock<bool> = OnceLock::new();
    *H24.get_or_init(read_24_hour)
}

/// Batteries in `/sys/class/power_supply`: the first one that powers the
/// system (not a mouse or headset, which have `scope` set to `Device`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_battery(root: &Path) -> Option<Battery> {
    let read = |dir: &Path, name: &str| std::fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_string());
    let mut dirs: Vec<_> = std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).collect();
    dirs.sort();
    dirs.into_iter().find_map(|dir| {
        if read(&dir, "type")? != "Battery" || read(&dir, "scope").as_deref() == Some("Device") {
            return None;
        }
        let percent: u8 = read(&dir, "capacity")?.parse().ok()?;
        let charging = read(&dir, "status").as_deref() == Some("Charging");
        Some(Battery { percent: percent.min(100), charging })
    })
}

#[cfg(windows)]
fn windows_battery() -> Option<Battery> {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
        return None;
    }
    // 128: no system battery; 255: unknown.
    if status.BatteryFlag & 128 != 0 || status.BatteryFlag == 255 || status.BatteryLifePercent > 100 {
        return None;
    }
    Some(Battery { percent: status.BatteryLifePercent, charging: status.BatteryFlag & 8 != 0 })
}

#[cfg(target_os = "macos")]
fn macos_battery() -> Option<Battery> {
    let out = std::process::Command::new("pmset").args(["-g", "batt"]).output().ok()?;
    parse_pmset(&String::from_utf8_lossy(&out.stdout))
}

/// `pmset -g batt`: ` -InternalBattery-0 (id=…)\t82%; charging; 1:05 remaining`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_pmset(text: &str) -> Option<Battery> {
    let line = text.lines().find(|l| l.contains("InternalBattery"))?;
    let after_tab = line.split('\t').nth(1)?;
    let mut parts = after_tab.split(';').map(str::trim);
    let percent: u8 = parts.next()?.strip_suffix('%')?.parse().ok()?;
    let charging = parts.next() == Some("charging");
    Some(Battery { percent: percent.min(100), charging })
}

fn read_24_hour() -> bool {
    #[cfg(target_os = "linux")]
    {
        // COSMIC's clock applet setting, also from inside a Flatpak.
        let homes = [
            std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from),
            Some(super::home_dir().join(".config")),
        ];
        for home in homes.into_iter().flatten() {
            let file = home.join("com.system76.CosmicAppletTime/v1/military_time");
            if let Ok(value) = std::fs::read_to_string(file) {
                return value.trim() == "true";
            }
        }
        locale_uses_24_hour(&locale())
    }
    #[cfg(windows)]
    {
        // The short time format, e.g. "HH:mm" or "h:mm tt".
        windows_registry::CURRENT_USER
            .open(r"Control Panel\International")
            .and_then(|k| k.get_string("sShortTime"))
            .map(|f| f.contains('H'))
            .unwrap_or(false)
    }
    #[cfg(target_os = "macos")]
    {
        let read = |key: &str| {
            std::process::Command::new("defaults")
                .args(["read", "-g", key])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        };
        if read("AppleICUForce24HourTime").as_deref() == Some("1") {
            return true;
        }
        locale_uses_24_hour(&read("AppleLocale").unwrap_or_default())
    }
}

#[cfg(target_os = "linux")]
fn locale() -> String {
    ["LC_ALL", "LC_TIME", "LANG"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()))
        .unwrap_or_default()
}

/// Most of the world uses a 24-hour clock; these locales default to 12.
#[cfg_attr(windows, allow(dead_code))]
fn locale_uses_24_hour(locale: &str) -> bool {
    const TWELVE: [&str; 6] = ["en_US", "en_CA", "en_AU", "en_NZ", "en_IN", "en_PH"];
    !TWELVE.iter().any(|l| locale.starts_with(l))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_linux_batteries() {
        let root = std::env::temp_dir().join(format!("nebula-power-{}", std::process::id()));
        let mouse = root.join("hidpp_battery_0");
        let bat = root.join("BAT0");
        let ac = root.join("AC");
        for d in [&mouse, &bat, &ac] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(mouse.join("type"), "Battery\n").unwrap();
        std::fs::write(mouse.join("scope"), "Device\n").unwrap();
        std::fs::write(mouse.join("capacity"), "5\n").unwrap();
        std::fs::write(ac.join("type"), "Mains\n").unwrap();
        assert_eq!(linux_battery(&root), None);
        std::fs::write(bat.join("type"), "Battery\n").unwrap();
        std::fs::write(bat.join("capacity"), "70\n").unwrap();
        std::fs::write(bat.join("status"), "Charging\n").unwrap();
        assert_eq!(linux_battery(&root), Some(Battery { percent: 70, charging: true }));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn parses_pmset() {
        let text = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1234)\t82%; discharging; 4:10 remaining present: true\n";
        assert_eq!(parse_pmset(text), Some(Battery { percent: 82, charging: false }));
        let text = " -InternalBattery-0 (id=1)\t100%; charging; 0:00 remaining\n";
        assert_eq!(parse_pmset(text), Some(Battery { percent: 100, charging: true }));
        assert_eq!(parse_pmset("Now drawing from 'AC Power'\n"), None);
    }

    #[test]
    fn locale_clock() {
        assert!(!locale_uses_24_hour("en_US.UTF-8"));
        assert!(locale_uses_24_hour("en_GB.UTF-8"));
        assert!(locale_uses_24_hour("de_DE.UTF-8"));
        assert!(locale_uses_24_hour("C.UTF-8"));
    }
}
