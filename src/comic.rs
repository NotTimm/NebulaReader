use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use compress_tools::{ArchiveContents, ArchiveIterator};

/// Open zip archives, cached by path, so preloading nearby pages doesn't
/// reopen the file and re-parse the central directory on every single page.
type ZipCache = Mutex<HashMap<PathBuf, Arc<Mutex<zip::ZipArchive<File>>>>>;

fn zip_cache() -> &'static ZipCache {
    static CACHE: OnceLock<ZipCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn open_zip(path: &Path) -> Result<Arc<Mutex<zip::ZipArchive<File>>>, String> {
    let mut cache = zip_cache().lock().unwrap();
    if let Some(archive) = cache.get(path) {
        return Ok(archive.clone());
    }
    let file = File::open(path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    let archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("failed to read zip {}: {e}", path.display()))?;
    let archive = Arc::new(Mutex::new(archive));
    cache.insert(path.to_path_buf(), archive.clone());
    Ok(archive)
}

/// A single decoded page.
#[derive(Debug)]
pub struct Page {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Where to find a page's bytes — cheap to list without decoding.
#[derive(Clone, Debug)]
pub enum PageSource {
    File(PathBuf),
    Zip { archive: PathBuf, entry: String },
    /// A page of an archive on a server, fetched with an HTTP range request.
    Remote { archive: std::sync::Arc<crate::remote_zip::RemoteZip>, index: usize },
    /// A page of a RAR archive: extracted with the rest of its archive in
    /// the background the first time any of its pages is wanted.
    Rar { archive: PathBuf, entry: String },
    /// A page of a (non-solid) RAR archive on a server.
    RemoteRar { archive: std::sync::Arc<crate::remote_rar::RemoteRar>, index: usize },
}

/// The page range one chapter occupies in the flat [`PageSource`] list.
#[derive(Clone, Debug)]
pub struct ChapterInfo {
    pub name: String,
    /// The issue this chapter came from — an archive file or an image
    /// directory. Used to key per-issue progress and metadata.
    pub path: PathBuf,
    pub start: usize,
    pub page_count: usize,
}

pub(crate) fn is_image_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

fn is_zip_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".cbz") || lower.ends_with(".zip")
}

fn is_rar_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".cbr") || lower.ends_with(".rar")
}

pub(crate) fn natural_key(name: &str) -> Vec<(String, u64)> {
    let mut key = Vec::new();
    let mut chars = name.chars().peekable();
    while chars.peek().is_some() {
        let digits: String =
            std::iter::from_fn(|| chars.next_if(|c| c.is_ascii_digit())).collect();
        if !digits.is_empty() {
            key.push((String::new(), digits.parse().unwrap_or(0)));
            continue;
        }
        let rest: String =
            std::iter::from_fn(|| chars.next_if(|c| !c.is_ascii_digit())).collect();
        key.push((rest, 0));
    }
    key
}

fn decode_image(name: &str, data: &[u8]) -> Result<Page, String> {
    let img = image::load_from_memory(data)
        .map_err(|e| format!("failed to decode page {name}: {e}"))?
        .to_rgba8();
    let (width, height) = img.dimensions();
    Ok(Page { width, height, rgba: img.into_raw() })
}

// ── ZIP/CBZ ──────────────────────────────────────────────────────────────────

fn list_zip_entries(path: &Path) -> Result<Vec<String>, String> {
    let archive = open_zip(path)?;
    let mut archive = archive.lock().unwrap();

    let mut names: Vec<String> = (0..archive.len())
        .filter_map(|i| {
            archive.by_index_raw(i).ok().and_then(|entry| {
                let name = entry.name().to_owned();
                (is_image_name(&name) && !name.ends_with('/')).then_some(name)
            })
        })
        .collect();

    names.sort_by(|a, b| natural_key(a).cmp(&natural_key(b)));
    Ok(names)
}

fn decode_zip_entry(archive_path: &Path, entry: &str) -> Result<Page, String> {
    let archive = open_zip(archive_path)?;
    let data = {
        let mut archive = archive.lock().unwrap();
        let mut zf = archive
            .by_name(entry)
            .map_err(|e| format!("entry {entry} not found: {e}"))?;

        let mut data = Vec::with_capacity(zf.size() as usize);
        std::io::Read::read_to_end(&mut zf, &mut data)
            .map_err(|e| format!("failed to read entry {entry}: {e}"))?;
        data
    };

    decode_image(entry, &data)
}

// ── RAR/CBR ──────────────────────────────────────────────────────────────────
//
// RAR has no central directory: reading entry N means streaming through
// entries 1..N first, so decoding pages one at a time (as ZIP does) would be
// O(n^2). Instead, the first time a page of an archive is wanted, one
// background pass extracts all its images to a disk cache, keyed by the
// file's size and mtime. Pages are usable as soon as they're written, so
// the first page shows while the rest is still coming out, and only the
// chapters actually read are extracted. Listing an archive's pages only
// reads its headers, which is quick even for a 400 MB file.

#[derive(serde::Serialize, serde::Deserialize)]
struct RarManifest {
    size: u64,
    mtime: i64,
    /// Original in-archive names, in extraction order; file `i` on disk is
    /// named `{i:05}` alongside this manifest.
    names: Vec<String>,
}

fn rar_cache_dir(archive: &Path) -> PathBuf {
    crate::library::cache_dir()
        .join("rar-cache")
        .join(crate::library::path_hash(&archive.to_string_lossy()))
}

fn rar_source_stamp(archive: &Path) -> Result<(u64, i64), String> {
    let meta = std::fs::metadata(archive)
        .map_err(|e| format!("failed to stat {}: {e}", archive.display()))?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok((meta.len(), mtime))
}

/// A previous, still-valid extraction of `archive`, if there is one.
fn cached_rar_entries(archive: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let cache_dir = rar_cache_dir(archive);
    let (size, mtime) = rar_source_stamp(archive)?;
    let data = std::fs::read(cache_dir.join("manifest.json")).map_err(|e| e.to_string())?;
    let manifest: RarManifest = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
    if manifest.size != size || manifest.mtime != mtime {
        return Err("stale".into());
    }
    let mut entries: Vec<(String, PathBuf)> = manifest
        .names
        .iter()
        .enumerate()
        .map(|(i, name)| (name.clone(), cache_dir.join(format!("{i:05}"))))
        .collect();
    if !entries.iter().all(|(_, p)| p.is_file()) {
        return Err("incomplete".into());
    }
    // Recently read: kept longest when the cache is trimmed.
    let _ = File::options()
        .append(true)
        .open(cache_dir.join("manifest.json"))
        .and_then(|f| f.set_modified(std::time::SystemTime::now()));
    entries.sort_by(|(a, _), (b, _)| natural_key(a).cmp(&natural_key(b)));
    Ok(entries)
}

/// One archive's background extraction: entry name -> extracted file, as
/// they're written.
#[derive(Default)]
struct RarJob {
    state: Mutex<RarState>,
    ready: std::sync::Condvar,
}

#[derive(Default)]
struct RarState {
    files: HashMap<String, PathBuf>,
    done: bool,
    error: Option<String>,
}

fn rar_jobs() -> &'static Mutex<HashMap<PathBuf, Arc<RarJob>>> {
    static JOBS: OnceLock<Mutex<HashMap<PathBuf, Arc<RarJob>>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The extracted file for one page of a RAR archive, waiting for the
/// background extraction to reach it if need be.
fn rar_page(archive: &Path, entry: &str) -> Result<PathBuf, String> {
    let job = {
        let mut jobs = rar_jobs().lock().unwrap();
        match jobs.get(archive) {
            Some(job) => job.clone(),
            None => {
                let job = Arc::new(RarJob::default());
                if let Ok(entries) = cached_rar_entries(archive) {
                    // Extracted by an earlier run.
                    let mut state = job.state.lock().unwrap();
                    state.files = entries.into_iter().collect();
                    state.done = true;
                } else {
                    let (archive, job) = (archive.to_path_buf(), job.clone());
                    std::thread::spawn(move || {
                        let result = extract_rar_into(&archive, &job);
                        if result.is_err() {
                            // Try again the next time it's opened.
                            rar_jobs().lock().unwrap().remove(&archive);
                        }
                        let mut state = job.state.lock().unwrap();
                        state.done = true;
                        state.error = result.err();
                        job.ready.notify_all();
                    });
                }
                jobs.insert(archive.to_path_buf(), job.clone());
                job
            }
        }
    };
    let mut state = job.state.lock().unwrap();
    loop {
        if let Some(path) = state.files.get(entry) {
            return Ok(path.clone());
        }
        if state.done {
            return Err(state.error.clone().unwrap_or_else(|| format!("{entry} isn't in {}", archive.display())));
        }
        state = job.ready.wait(state).unwrap();
    }
}

/// Extracts every image in `archive` to its cache folder in one pass,
/// publishing each page in `job` as soon as it's written.
fn extract_rar_into(archive: &Path, job: &RarJob) -> Result<(), String> {
    let cache_dir = rar_cache_dir(archive);
    let (size, mtime) = rar_source_stamp(archive)?;
    // A half-finished extraction from before is redone from scratch.
    let _ = std::fs::remove_file(cache_dir.join("manifest.json"));
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("failed to create {}: {e}", cache_dir.display()))?;

    let file = File::open(archive).map_err(|e| format!("failed to open {}: {e}", archive.display()))?;
    let iter = ArchiveIterator::from_read(file).map_err(|e| format!("failed to read archive: {e}"))?;

    let mut names = Vec::new();
    let mut current_name = String::new();
    let mut current_data = Vec::new();
    let mut wanted = false;
    let mut is_comic_info = false;

    for content in iter {
        match content {
            ArchiveContents::StartOfEntry(name, _stat) => {
                wanted = is_image_name(&name) && !name.ends_with('/');
                is_comic_info = name.to_ascii_lowercase().ends_with("comicinfo.xml");
                current_name = name;
                current_data.clear();
            }
            ArchiveContents::DataChunk(chunk) => {
                if wanted || is_comic_info {
                    current_data.extend_from_slice(&chunk);
                }
            }
            ArchiveContents::EndOfEntry => {
                if wanted {
                    let out_path = cache_dir.join(format!("{:05}", names.len()));
                    let part = out_path.with_extension("part");
                    std::fs::write(&part, &current_data)
                        .and_then(|()| std::fs::rename(&part, &out_path))
                        .map_err(|e| format!("failed to write {}: {e}", out_path.display()))?;
                    names.push(current_name.clone());
                    job.state.lock().unwrap().files.insert(current_name.clone(), out_path);
                    job.ready.notify_all();
                } else if is_comic_info {
                    let _ = std::fs::write(cache_dir.join("ComicInfo.xml"), &current_data);
                }
            }
            ArchiveContents::Err(e) => return Err(format!("archive error: {e}")),
        }
    }

    let manifest = RarManifest { size, mtime, names };
    if let Ok(json) = serde_json::to_vec(&manifest) {
        let _ = std::fs::write(cache_dir.join("manifest.json"), json);
    }
    Ok(())
}

/// A RAR archive's pages, in reading order, without extracting anything.
fn rar_pages(archive: &Path) -> Result<Vec<PageSource>, String> {
    // An earlier extraction already knows the names.
    let names = match cached_rar_entries(archive) {
        Ok(entries) => entries.into_iter().map(|(name, _)| name).collect(),
        Err(_) => rar_image_names(archive)?,
    };
    Ok(names.into_iter().map(|entry| PageSource::Rar { archive: archive.to_path_buf(), entry }).collect())
}

/// Keeps a cache folder under `limit` bytes, dropping what was read
/// longest ago: each entry is a file (by its mtime) or an extracted
/// archive's folder (by its manifest's mtime). Run at startup, before
/// anything is open.
pub fn prune_cache(root: &Path, limit: u64) {
    let mut entries: Vec<(std::time::SystemTime, u64, PathBuf)> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let meta = e.metadata().ok()?;
            if meta.is_dir() {
                let size = std::fs::read_dir(&path).into_iter().flatten().flatten().filter_map(|f| f.metadata().ok()).map(|m| m.len()).sum();
                let used = std::fs::metadata(path.join("manifest.json")).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                Some((used, size, path))
            } else {
                Some((meta.modified().unwrap_or(std::time::UNIX_EPOCH), meta.len(), path))
            }
        })
        .collect();
    entries.sort_by_key(|(used, _, _)| std::cmp::Reverse(*used));
    let mut total = 0u64;
    for (_, size, path) in entries {
        total += size;
        if total > limit {
            let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
    }
}

/// Keeps the extracted-RAR cache under `limit` bytes.
pub fn prune_rar_cache(limit: u64) {
    prune_cache(&crate::library::cache_dir().join("rar-cache"), limit);
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Decodes image bytes from anywhere (`name` picks the format).
pub fn decode_bytes(name: &str, data: &[u8]) -> Result<Page, String> {
    decode_image(name, data)
}

pub fn decode_source(source: &PageSource) -> Result<Page, String> {
    match source {
        PageSource::File(path) => {
            let data = std::fs::read(path)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
            decode_image(&path.to_string_lossy(), &data)
        }
        PageSource::Zip { archive, entry } => decode_zip_entry(archive, entry),
        PageSource::Remote { archive, index } => {
            let name = archive.entries.get(*index).map(|e| e.name.clone()).unwrap_or_default();
            decode_image(&name, &archive.read(*index)?)
        }
        PageSource::RemoteRar { archive, index } => {
            let name = archive.entries().get(*index).map(|e| e.name.clone()).unwrap_or_default();
            decode_image(&name, &archive.read(*index)?)
        }
        PageSource::Rar { archive, entry } => {
            let path = rar_page(archive, entry)?;
            let data = std::fs::read(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
            decode_image(entry, &data)
        }
    }
}

fn list_image_dir(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .map_err(|e| format!("failed to read {}: {e}", dir.display()))?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            (path.is_file() && is_image_name(&name)).then_some((name, path))
        })
        .collect();

    files.sort_by(|(a, _), (b, _)| natural_key(a).cmp(&natural_key(b)));
    Ok(files.into_iter().map(|(_, path)| path).collect())
}

enum Source {
    Zip(PathBuf),
    Rar(PathBuf),
    ImageDir(PathBuf),
}

fn dir_has_direct_images(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|entry| {
                entry.path().is_file() && is_image_name(&entry.file_name().to_string_lossy())
            })
        })
        .unwrap_or(false)
}

fn find_sources(root: &Path, sources: &mut Vec<(String, Source)>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };

    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_file() {
            if is_zip_name(&name) {
                sources.push((name, Source::Zip(path)));
            } else if is_rar_name(&name) {
                sources.push((name, Source::Rar(path)));
            }
        } else if path.is_dir() {
            subdirs.push(path);
        }
    }

    if dir_has_direct_images(root) {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        sources.push((name, Source::ImageDir(root.to_path_buf())));
    }

    for subdir in subdirs {
        find_sources(&subdir, sources);
    }
}

/// Strip file extension from a chapter filename to get a display name.
/// A chapter's name for the chapter picker and details panel. Manga-style
/// chapter files ("Ch 3 Vol none Weeb central 01J7….cbz") become "Ch. 3",
/// plus the chapter title when there is one; anything else keeps its name.
pub(crate) fn display_name(raw: &str) -> String {
    let stem = std::path::Path::new(raw)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| raw.to_string());
    if crate::naming::starts_with_chapter(raw) {
        let issue = crate::naming::parse_issue(raw);
        if let Some(number) = issue.number {
            return match issue.subtitle {
                Some(sub) => format!("Ch. {number} · {sub}"),
                None => format!("Ch. {number}"),
            };
        }
    }
    stem
}

/// Lists pages without decoding any.  Returns pages and chapter metadata.
pub fn collect_sources(path: &Path) -> Result<(Vec<PageSource>, Vec<ChapterInfo>), String> {
    if path.is_file() {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let is_zip = is_zip_name(&name);

        let ch_name = display_name(&name);

        let archive = path.to_path_buf();
        let pages: Vec<PageSource> = if is_zip {
            let entries = list_zip_entries(path)?;
            entries
                .into_iter()
                .map(|entry| PageSource::Zip { archive: archive.clone(), entry })
                .collect()
        } else {
            rar_pages(path)?
        };

        if pages.is_empty() {
            return Err(format!("no image pages found in {}", path.display()));
        }

        let page_count = pages.len();
        let chapters =
            vec![ChapterInfo { name: ch_name, path: path.to_path_buf(), start: 0, page_count }];
        return Ok((pages, chapters));
    }

    if !path.is_dir() {
        return Err(format!("{} is neither a file nor a directory", path.display()));
    }

    let issues = list_issue_files(path);
    if issues.is_empty() {
        return Err(format!("no comic chapters or images found under {}", path.display()));
    }
    let paths: Vec<PathBuf> = issues.into_iter().map(|i| i.path).collect();
    collect_sources_for(&paths)
}

/// One issue/chapter inside a series folder: an archive or an image dir.
#[derive(Clone, Debug)]
pub struct IssueFile {
    pub path: PathBuf,
    /// File (or directory) name, extension included.
    pub name: String,
}

/// Lists a series folder's issues (recursively), in reading order, without
/// opening any of them. [`collect_sources`] uses the same ordering, so an
/// issue's index here is its chapter index in the reader.
pub fn list_issue_files(dir: &Path) -> Vec<IssueFile> {
    let mut raw = Vec::new();
    find_sources(dir, &mut raw);
    raw.sort_by(|(a, _), (b, _)| natural_key(a).cmp(&natural_key(b)));
    raw.into_iter()
        .map(|(name, source)| {
            let path = match source {
                Source::Zip(p) | Source::Rar(p) | Source::ImageDir(p) => p,
            };
            IssueFile { path, name }
        })
        .collect()
}

/// Lists pages for an explicit, ordered list of issues (archives or image
/// directories), one chapter per issue. Used for series whose issues don't
/// all live under one folder, e.g. loose files grouped by name.
pub fn collect_sources_for(issues: &[PathBuf]) -> Result<(Vec<PageSource>, Vec<ChapterInfo>), String> {
    let mut pages = Vec::new();
    let mut chapters = Vec::new();

    for issue in issues {
        let name = issue.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let start = pages.len();
        if issue.is_dir() {
            pages.extend(list_image_dir(issue)?.into_iter().map(PageSource::File));
        } else if is_zip_name(&name) {
            pages.extend(
                list_zip_entries(issue)?
                    .into_iter()
                    .map(|entry| PageSource::Zip { archive: issue.clone(), entry }),
            );
        } else if is_rar_name(&name) {
            pages.extend(rar_pages(issue)?);
        } else {
            continue;
        }
        chapters.push(ChapterInfo {
            name: display_name(&name),
            path: issue.clone(),
            start,
            page_count: pages.len() - start,
        });
    }

    if pages.is_empty() {
        return Err("no image pages found".to_string());
    }
    Ok((pages, chapters))
}

// ── Cheap library-scan helpers ───────────────────────────────────────────────
//
// The library scan must not go through `collect_sources`: for RAR that
// extracts the entire archive to disk, which for a whole library is
// gigabytes of cache and minutes of I/O just to show a grid of covers.
// These read only what they need, and never touch the zip handle cache
// (which would otherwise hold a file descriptor per issue in the library).

fn zip_image_names_uncached(path: &Path) -> Result<Vec<String>, String> {
    let file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("zip {}: {e}", path.display()))?;
    let mut names: Vec<String> = (0..archive.len())
        .filter_map(|i| {
            archive.by_index_raw(i).ok().and_then(|e| {
                let n = e.name().to_owned();
                (is_image_name(&n) && !n.ends_with('/')).then_some(n)
            })
        })
        .collect();
    names.sort_by(|a, b| natural_key(a).cmp(&natural_key(b)));
    Ok(names)
}

fn rar_image_names(path: &Path) -> Result<Vec<String>, String> {
    let file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut names: Vec<String> = compress_tools::list_archive_files(file)
        .map_err(|e| format!("list {}: {e}", path.display()))?
        .into_iter()
        .filter(|n| is_image_name(n) && !n.ends_with('/'))
        .collect();
    names.sort_by(|a, b| natural_key(a).cmp(&natural_key(b)));
    Ok(names)
}

/// Whether a plain `.zip`/`.rar` is actually a comic: mostly images, a
/// few of them. (`.cbz`/`.cbr` are trusted by extension.) Keeps game
/// ROMs, firmware and other archives out of a library folder's shelf.
pub fn looks_like_comic(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if name.ends_with(".cbz") || name.ends_with(".cbr") {
        return true;
    }
    let all: Vec<String> = if is_zip_name(&name) {
        let Ok(file) = File::open(path) else { return false };
        let Ok(mut archive) = zip::ZipArchive::new(file) else { return false };
        (0..archive.len())
            .filter_map(|i| archive.by_index_raw(i).ok().map(|e| e.name().to_owned()))
            .filter(|n| !n.ends_with('/'))
            .collect()
    } else if is_rar_name(&name) {
        let Ok(file) = File::open(path) else { return false };
        compress_tools::list_archive_files(file).unwrap_or_default().into_iter().filter(|n| !n.ends_with('/')).collect()
    } else {
        return false;
    };
    let images = all.iter().filter(|n| is_image_name(n)).count();
    images >= 3 && images * 10 >= all.len() * 7
}

/// Page count of one issue, reading only archive headers.
pub fn count_pages(issue: &Path) -> Result<usize, String> {
    let name = issue.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if issue.is_dir() {
        Ok(list_image_dir(issue)?.len())
    } else if is_zip_name(&name) {
        Ok(zip_image_names_uncached(issue)?.len())
    } else if is_rar_name(&name) {
        Ok(rar_image_names(issue)?.len())
    } else {
        Err(format!("not a comic: {}", issue.display()))
    }
}

/// Decodes just the first page of one issue (for cover thumbnails). For RAR
/// this decompresses a single entry rather than the whole archive.
pub fn first_page(issue: &Path) -> Result<Page, String> {
    let name = issue.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if issue.is_dir() {
        let first = list_image_dir(issue)?.into_iter().next().ok_or("empty folder")?;
        return decode_source(&PageSource::File(first));
    }
    if is_zip_name(&name) {
        let first = zip_image_names_uncached(issue)?.into_iter().next().ok_or("no pages")?;
        let file = File::open(issue).map_err(|e| format!("open: {e}"))?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("zip: {e}"))?;
        let mut entry = archive.by_name(&first).map_err(|e| format!("entry: {e}"))?;
        let mut data = Vec::with_capacity(entry.size() as usize);
        std::io::Read::read_to_end(&mut entry, &mut data).map_err(|e| format!("read: {e}"))?;
        return decode_image(&first, &data);
    }
    if is_rar_name(&name) {
        // Already extracted by an earlier open? Use that.
        if let Ok(entries) = cached_rar_entries(issue) {
            if let Some((_, p)) = entries.into_iter().next() {
                return decode_source(&PageSource::File(p));
            }
        }
        let first = rar_image_names(issue)?.into_iter().next().ok_or("no pages")?;
        let file = File::open(issue).map_err(|e| format!("open: {e}"))?;
        let mut data = Vec::new();
        compress_tools::uncompress_archive_file(file, &mut data, &first)
            .map_err(|e| format!("extract {first}: {e}"))?;
        return decode_image(&first, &data);
    }
    Err(format!("not a comic: {}", issue.display()))
}

/// Looks for an embedded `ComicInfo.xml` at the top level of `path` (a
/// single archive file, or a series/chapter directory), returning its raw
/// text if found. Cheap: for zip it's one cached-archive lookup, for rar
/// it's a cache-dir file read (already extracted by [`collect_sources`]),
/// for directories it's a single stat.
pub fn find_comic_info(path: &Path) -> Option<String> {
    if path.is_dir() {
        for name in ["ComicInfo.xml", "comicinfo.xml", "COMICINFO.XML"] {
            let candidate = path.join(name);
            if candidate.is_file() {
                return std::fs::read_to_string(candidate).ok();
            }
        }
        return None;
    }

    let name = path.file_name()?.to_string_lossy();
    if is_zip_name(&name) {
        let archive = open_zip(path).ok()?;
        let mut archive = archive.lock().unwrap();
        let idx = (0..archive.len()).find(|&i| {
            archive
                .by_index_raw(i)
                .map(|e| e.name().to_ascii_lowercase().ends_with("comicinfo.xml"))
                .unwrap_or(false)
        })?;
        let mut entry = archive.by_index(idx).ok()?;
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut entry, &mut buf).ok()?;
        Some(buf)
    } else if is_rar_name(&name) {
        let candidate = rar_cache_dir(path).join("ComicInfo.xml");
        if candidate.is_file() {
            return std::fs::read_to_string(candidate).ok();
        }
        // Not extracted yet: pull just that one entry out of the archive.
        let file = File::open(path).ok()?;
        let entry = compress_tools::list_archive_files(file)
            .ok()?
            .into_iter()
            .find(|n| n.to_ascii_lowercase().ends_with("comicinfo.xml"))?;
        let mut data = Vec::new();
        compress_tools::uncompress_archive_file(File::open(path).ok()?, &mut data, &entry).ok()?;
        String::from_utf8(data).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod rar_tests {
    use super::*;

    /// Times opening a real CBR: `NEBULA_RAR_FILE=path cargo test
    /// opens_rar_lazily -- --ignored --nocapture`. Use a copy, so its cache
    /// starts empty.
    #[test]
    #[ignore]
    fn opens_rar_lazily() {
        let path = PathBuf::from(std::env::var("NEBULA_RAR_FILE").expect("set NEBULA_RAR_FILE"));
        let t = std::time::Instant::now();
        let (pages, chapters) = collect_sources(&path).unwrap();
        println!("listed {} pages in {:?}", pages.len(), t.elapsed());
        assert_eq!(chapters[0].page_count, pages.len());
        let t = std::time::Instant::now();
        let PageSource::Rar { archive, entry } = &pages[0] else { panic!("not a RAR page") };
        rar_page(archive, entry).unwrap();
        println!("first page extracted in {:?}", t.elapsed());
        let PageSource::Rar { archive, entry } = pages.last().unwrap() else { panic!() };
        rar_page(archive, entry).unwrap();
        println!("last page extracted in {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let first = decode_source(&pages[0]).unwrap();
        println!("first page {}x{} in {:?}", first.width, first.height, t.elapsed());
        let t = std::time::Instant::now();
        decode_source(pages.last().unwrap()).unwrap();
        println!("last page in {:?}", t.elapsed());
        let t = std::time::Instant::now();
        decode_source(&pages[1]).unwrap();
        println!("second page (already extracted) in {:?}", t.elapsed());
    }
}
