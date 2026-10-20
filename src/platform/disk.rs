//! Free space and drive checks for the download folder, which can be on a
//! USB stick or SD card.

use std::path::Path;

/// Bytes free for this user on the drive holding `path`.
pub fn free_space(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)]
        Some(s.f_bavail as u64 * s.f_frsize as u64)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free = 0u64;
        let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
        (ok != 0).then_some(free)
    }
}

/// Whether `path` is on another drive than the app's own data, e.g. a USB
/// stick. `None` when it can't tell (inside a Flatpak, a chosen folder is
/// a portal path, which always looks like another drive).
pub fn other_drive(path: &Path) -> Option<bool> {
    if std::env::var_os("FLATPAK_ID").is_some() {
        return None;
    }
    let data = super::data_dir();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(std::fs::metadata(path).ok()?.dev() != std::fs::metadata(&data).ok()?.dev())
    }
    #[cfg(windows)]
    {
        let drive = |p: &Path| match p.components().next() {
            Some(std::path::Component::Prefix(prefix)) => Some(prefix.as_os_str().to_string_lossy().to_lowercase()),
            _ => None,
        };
        Some(drive(path)? != drive(&data)?)
    }
}

/// Checks a folder can take new files: writes, flushes and deletes a small
/// test file in it.
pub fn check_writable(dir: &Path) -> Result<(), String> {
    use std::io::Write;

    let probe = dir.join(".nebula-write-test");
    let result = std::fs::File::create(&probe)
        .and_then(|mut f| {
            f.write_all(b"ok")?;
            f.sync_all()
        })
        .map_err(|e| format!("can't save files in {}: {e}", dir.display()));
    let _ = std::fs::remove_file(&probe);
    result
}

/// Copies `from` to `to` safely: through `to.part`, flushed to the drive and
/// size-checked before it takes its name. The source is left alone.
pub fn copy_file(from: &Path, to: &Path) -> Result<u64, String> {
    let part = to.with_extension("part");
    let result = (|| {
        let size = std::fs::copy(from, &part).map_err(|e| e.to_string())?;
        // Windows only flushes a file opened for writing.
        std::fs::File::options().write(true).open(&part).and_then(|f| f.sync_all()).map_err(|e| e.to_string())?;
        let expected = std::fs::metadata(from).map_err(|e| e.to_string())?.len();
        if size != expected {
            return Err(format!("copied {size} of {expected} bytes"));
        }
        std::fs::rename(&part, to).map_err(|e| e.to_string())?;
        sync_dir(to);
        Ok(size)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

/// Flushes a renamed file's folder entry, so the new name survives the
/// drive being pulled. Best effort: not every file system supports it.
pub fn sync_dir(file: &Path) {
    #[cfg(unix)]
    if let Some(dir) = file.parent() {
        let _ = std::fs::File::open(dir).and_then(|d| d.sync_all());
    }
    #[cfg(not(unix))]
    let _ = file;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_and_checks() {
        let dir = std::env::temp_dir().join(format!("nebula-disk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.cbz"), b"pages").unwrap();
        assert!(check_writable(&dir).is_ok());
        assert!(!dir.join(".nebula-write-test").exists());
        assert_eq!(copy_file(&dir.join("a.cbz"), &dir.join("b.cbz")), Ok(5));
        assert_eq!(std::fs::read(dir.join("b.cbz")).unwrap(), b"pages");
        assert!(!dir.join("b.part").exists());
        assert!(copy_file(&dir.join("missing.cbz"), &dir.join("c.cbz")).is_err());
        assert!(free_space(&dir).is_some_and(|f| f > 0));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
