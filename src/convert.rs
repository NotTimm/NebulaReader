//! Repacks CBR (RAR) comics as CBZ (zip), which opens faster, streams
//! from Jellyfin page by page and is readable everywhere.
//!
//! The pages are copied byte for byte (stored, not recompressed: they're
//! JPEGs and PNGs already), along with anything else in the archive such
//! as `ComicInfo.xml`. The new file is written next to the old one as
//! `.cbz.part`, flushed to the drive and checked against what was written
//! before it takes its name, so a failure or a pulled drive never leaves a
//! half-written comic behind. What happens to the original is up to the
//! caller (the app moves it to the Trash).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use compress_tools::{ArchiveContents, ArchiveIterator};

/// Room to leave free on the drive besides the new file.
const SPARE: u64 = 64 * 1024 * 1024;

pub fn is_rar(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cbr") || e.eq_ignore_ascii_case("rar"))
}

/// Where the CBZ for `cbr` goes: same folder, same name.
pub fn cbz_path(cbr: &Path) -> PathBuf {
    cbr.with_extension("cbz")
}

/// Writes `cbr`'s contents to a new `.cbz` beside it and returns its path.
/// `written` counts bytes as they're written (for a progress bar);
/// `cancel` stops it, leaving nothing behind.
pub fn to_cbz(cbr: &Path, cancel: &AtomicBool, written: &AtomicU64) -> Result<PathBuf, String> {
    let out = cbz_path(cbr);
    if out.exists() {
        return Err(format!("{} already exists", out.display()));
    }
    let size = std::fs::metadata(cbr).map_err(|e| format!("can't read {}: {e}", cbr.display()))?.len();
    let dir = cbr.parent().ok_or("no folder")?;
    if let Some(free) = crate::platform::disk::free_space(dir) {
        if size + SPARE > free {
            return Err(format!(
                "not enough space next to {}: {} MB needed, {} MB free",
                cbr.display(),
                (size + SPARE) / 1_048_576,
                free / 1_048_576
            ));
        }
    }

    let part = cbr.with_extension("cbz.part");
    let result = write_zip(cbr, &part, cancel, written).and_then(|expected| {
        verify(&part, &expected)?;
        std::fs::rename(&part, &out).map_err(|e| e.to_string())?;
        crate::platform::disk::sync_dir(&out);
        Ok(out)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

/// Streams every file of the RAR into a stored zip; returns (name, size)
/// of each, to check the result against.
fn write_zip(cbr: &Path, part: &Path, cancel: &AtomicBool, written: &AtomicU64) -> Result<Vec<(String, u64)>, String> {
    let input = std::fs::File::open(cbr).map_err(|e| format!("can't open {}: {e}", cbr.display()))?;
    let output = std::fs::File::create(part).map_err(|e| format!("can't write next to {}: {e}", cbr.display()))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(output));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);
    let mut files: Vec<(String, u64)> = Vec::new();
    let mut open = false;
    let iter = ArchiveIterator::from_read(input).map_err(|e| format!("not a readable RAR: {e}"))?;
    for content in iter {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        match content {
            ArchiveContents::StartOfEntry(name, _) => {
                open = !name.ends_with('/');
                if open {
                    zip.start_file(name.replace('\\', "/"), options).map_err(|e| e.to_string())?;
                    files.push((name, 0));
                }
            }
            ArchiveContents::DataChunk(chunk) => {
                if open {
                    zip.write_all(&chunk).map_err(|e| format!("couldn't write the CBZ: {e}"))?;
                    if let Some(f) = files.last_mut() {
                        f.1 += chunk.len() as u64;
                    }
                    written.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                }
            }
            ArchiveContents::EndOfEntry => open = false,
            ArchiveContents::Err(e) => return Err(format!("the CBR is damaged: {e}")),
        }
    }
    if files.is_empty() {
        return Err("the CBR is empty".into());
    }
    let file = zip.finish().map_err(|e| e.to_string())?.into_inner().map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| format!("couldn't write the CBZ: {e}"))?;
    Ok(files)
}

/// The finished zip must list exactly the files written, at their sizes.
fn verify(part: &Path, expected: &[(String, u64)]) -> Result<(), String> {
    let file = std::fs::File::open(part).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("the new CBZ doesn't read back: {e}"))?;
    if zip.len() != expected.len() {
        return Err(format!("the new CBZ has {} files, expected {}", zip.len(), expected.len()));
    }
    for (i, (name, size)) in expected.iter().enumerate() {
        let entry = zip.by_index_raw(i).map_err(|e| e.to_string())?;
        if entry.size() != *size {
            return Err(format!("{name} came out at {} bytes, expected {size}", entry.size()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Converts a copy of a real CBR: `NEBULA_RAR_FILE=path cargo test
    /// converts_a_real_cbr -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn converts_a_real_cbr() {
        let src = PathBuf::from(std::env::var("NEBULA_RAR_FILE").expect("set NEBULA_RAR_FILE"));
        let dir = std::env::temp_dir().join(format!("nebula-convert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cbr = dir.join("Test 01.cbr");
        std::fs::copy(&src, &cbr).unwrap();
        let written = AtomicU64::new(0);
        let t = std::time::Instant::now();
        let cbz = to_cbz(&cbr, &AtomicBool::new(false), &written).unwrap();
        println!("{} MB in {:?}", written.load(Ordering::Relaxed) / 1_048_576, t.elapsed());
        assert!(!cbr.with_extension("cbz.part").exists());
        // Same pages, same bytes.
        let names = crate::comic::count_pages(&cbz).unwrap();
        assert_eq!(names, crate::comic::count_pages(&cbr).unwrap());
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&cbz).unwrap()).unwrap();
        let first = zip.by_index(1).unwrap().name().to_string();
        let mut from_zip = Vec::new();
        std::io::Read::read_to_end(&mut zip.by_name(&first).unwrap(), &mut from_zip).unwrap();
        let from_rar = std::process::Command::new("bsdtar").arg("-xOf").arg(&cbr).arg(&first).output().unwrap().stdout;
        assert_eq!(from_zip, from_rar);
        // A second run refuses to overwrite.
        assert!(to_cbz(&cbr, &AtomicBool::new(false), &written).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cancel_leaves_nothing() {
        let dir = std::env::temp_dir().join(format!("nebula-convert-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cbr = dir.join("x.cbr");
        std::fs::write(&cbr, b"not really a rar").unwrap();
        assert!(to_cbz(&cbr, &AtomicBool::new(true), &AtomicU64::new(0)).is_err());
        assert!(!dir.join("x.cbz").exists() && !dir.join("x.cbz.part").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
