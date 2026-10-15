//! Library scanning: works out which folders/files under a library root
//! are series, lists their issues, and makes cover thumbnails — cheaply.
//!
//! Layouts it understands:
//! - `Root/Series/*.cbz`                     a series folder
//! - `Root/Series/English/Ch 1 ….cbz`        manga-tui (one wrapper dir)
//! - `Root/Series/Chapter 1/*.jpg`           image-folder chapters
//! - `Root/Marvel/Civil War/*.cbr`           container folders (≥2 series)
//! - `Root/Civil War 001.cbr, …002.cbr`      loose issues, grouped by name
//! - `Root/Book.epub`, `Root/One-shot.cbz`   standalone items

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::comic;
use crate::epub;
use crate::library::{self, IssueRow, ScannedSeries, SeriesKind};
use crate::naming;

pub const THUMB_W: u32 = 240;
pub const THUMB_H: u32 = 340;

/// A series found on disk, before scanning its contents.
#[derive(Clone, Debug)]
pub struct Discovered {
    /// DB key: the folder/file path, or a `group://` key for loose issues.
    pub key: String,
    pub kind: SeriesKind,
    pub root: String,
    /// The name to clean into a title (folder name / group name / file name).
    pub raw_name: String,
    /// Group members (only for [`SeriesKind::Group`]).
    pub members: Vec<PathBuf>,
}

fn lower_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn is_comic_file(p: &Path) -> bool {
    let n = lower_name(p);
    [".cbz", ".cbr", ".zip", ".rar"].iter().any(|e| n.ends_with(e))
}

fn is_image(p: &Path) -> bool {
    let n = lower_name(p);
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"].iter().any(|e| n.ends_with(e))
}

fn is_hidden(p: &Path) -> bool {
    p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(true)
}

struct DirInfo {
    archives: usize,
    images: usize,
    subdirs: Vec<PathBuf>,
}

fn dir_info(dir: &Path) -> DirInfo {
    let mut info = DirInfo { archives: 0, images: 0, subdirs: Vec::new() };
    let Ok(rd) = std::fs::read_dir(dir) else { return info };
    for e in rd.flatten() {
        let p = e.path();
        if is_hidden(&p) {
            continue;
        }
        if p.is_dir() {
            info.subdirs.push(p);
        } else if is_comic_file(&p) {
            info.archives += 1;
        } else if is_image(&p) {
            info.images += 1;
        }
    }
    info
}

/// Does `dir` contain comic content anywhere within `depth` levels?
fn has_content(dir: &Path, depth: u32) -> bool {
    let info = dir_info(dir);
    if info.archives > 0 || info.images > 0 {
        return true;
    }
    depth > 0 && info.subdirs.iter().any(|d| has_content(d, depth - 1))
}

/// Finds every series under `root`.
pub fn discover(root: &Path) -> Vec<Discovered> {
    let root_s = root.to_string_lossy().into_owned();
    let mut out = Vec::new();
    discover_in(root, &root_s, 0, &mut out);
    out
}

fn discover_in(dir: &Path, root: &str, depth: u32, out: &mut Vec<Discovered>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut loose: Vec<PathBuf> = Vec::new();
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if is_hidden(&p) {
            continue;
        }
        if p.is_dir() {
            subdirs.push(p);
        } else if is_comic_file(&p) {
            loose.push(p);
        } else if epub::is_epub_name(&lower_name(&p)) {
            out.push(Discovered {
                key: p.to_string_lossy().into_owned(),
                kind: SeriesKind::File,
                root: root.to_string(),
                raw_name: p.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                members: Vec::new(),
            });
        }
    }

    for sub in subdirs {
        classify_dir(&sub, root, depth, out);
    }

    // Loose issues: group by cleaned series name. Two or more issues of the
    // same series become one Group entry; singles stay standalone.
    let mut groups: HashMap<String, (String, Vec<PathBuf>)> = HashMap::new();
    for p in loose {
        let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let has_number = naming::parse_issue(&name).number.is_some();
        let clean = naming::series_name_from_issue(&name);
        let key = if has_number { naming::normalize_title(&clean.search) } else { name.clone() };
        groups.entry(key).or_insert_with(|| (clean.display.clone(), Vec::new())).1.push(p);
    }
    for (_, (display, mut members)) in groups {
        if members.len() >= 2 {
            members.sort_by(|a, b| natural_cmp(a, b));
            out.push(Discovered {
                key: format!("group://{}/{}", dir.to_string_lossy(), display),
                kind: SeriesKind::Group,
                root: root.to_string(),
                raw_name: display,
                members,
            });
        } else {
            let p = members.remove(0);
            out.push(Discovered {
                key: p.to_string_lossy().into_owned(),
                kind: SeriesKind::File,
                root: root.to_string(),
                raw_name: p.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                members: Vec::new(),
            });
        }
    }
}

fn classify_dir(dir: &Path, root: &str, depth: u32, out: &mut Vec<Discovered>) {
    let info = dir_info(dir);
    let series = |out: &mut Vec<Discovered>| {
        out.push(Discovered {
            key: dir.to_string_lossy().into_owned(),
            kind: SeriesKind::Dir,
            root: root.to_string(),
            raw_name: dir.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            members: Vec::new(),
        })
    };

    // Issues (or pages) directly inside: it's a series.
    if info.archives > 0 || info.images > 0 {
        return series(out);
    }
    let content_subdirs: Vec<&PathBuf> = info.subdirs.iter().filter(|d| has_content(d, 2)).collect();
    if content_subdirs.is_empty() {
        return;
    }
    // Subfolders made of images are chapters of this series (manga).
    let image_chapters = content_subdirs.iter().filter(|d| dir_info(d).images > 0).count();
    if image_chapters * 2 >= content_subdirs.len() || content_subdirs.len() == 1 || depth >= 2 {
        return series(out);
    }
    // Several subfolders that each hold issues: a container (publisher,
    // collection…) — each subfolder is its own series.
    discover_in(dir, root, depth + 1, out);
}

fn natural_cmp(a: &Path, b: &Path) -> std::cmp::Ordering {
    let an = a.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    let bn = b.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    natural_key(&an).cmp(&natural_key(&bn))
}

fn natural_key(s: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while chars.peek().is_some() {
        let digits: String = std::iter::from_fn(|| chars.next_if(|c| c.is_ascii_digit())).collect();
        if !digits.is_empty() {
            out.push((String::new(), digits.parse().unwrap_or(0)));
            continue;
        }
        let rest: String = std::iter::from_fn(|| chars.next_if(|c| !c.is_ascii_digit())).collect();
        out.push((rest, 0));
    }
    out
}

// ── Scanning one series ──────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct ScanResult {
    pub series: ScannedSeries,
    pub issues: Vec<IssueRow>,
}

/// Issue files of a discovered series, in reading order.
pub fn issue_paths(d: &Discovered) -> Vec<PathBuf> {
    match d.kind {
        SeriesKind::Dir => comic::list_issue_files(Path::new(&d.key)).into_iter().map(|i| i.path).collect(),
        SeriesKind::File => vec![PathBuf::from(&d.key)],
        SeriesKind::Group => d.members.clone(),
    }
}

/// Turns issue paths into DB rows (numbers parsed, page counts filled in
/// for anything not already known).
pub fn issue_rows(paths: &[PathBuf], known: &HashMap<String, usize>, count: bool) -> Vec<IssueRow> {
    paths
        .iter()
        .map(|p| {
            let path = p.to_string_lossy().into_owned();
            let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let parsed = naming::parse_issue(&name);
            let page_count = known
                .get(&path)
                .copied()
                .or_else(|| count.then(|| comic::count_pages(p).ok()).flatten());
            let display = std::path::Path::new(&name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .filter(|_| !p.is_dir())
                .unwrap_or(name.clone());
            IssueRow {
                path,
                name: display,
                number: parsed.number.unwrap_or_default(),
                subtitle: parsed.subtitle.unwrap_or_default(),
                page_count,
            }
        })
        .collect()
}

/// Scans one series: issues, page counts (only for new issues), title from
/// ComicInfo.xml or the cleaned name, and a cover thumbnail if missing.
/// Runs on a blocking thread.
pub fn scan_series(d: &Discovered, known: HashMap<String, usize>) -> Result<ScanResult, String> {
    let is_epub = d.kind == SeriesKind::File && epub::is_epub_name(&d.key);

    if is_epub {
        let meta = epub::read_meta(Path::new(&d.key))?;
        let cover_path = library::chapter_cover_path(&d.key, 0);
        if !cover_path.exists() {
            if let Some(bytes) = &meta.cover {
                if let Ok(img) = image::load_from_memory(bytes) {
                    let _ = save_thumb(&img.to_rgba8(), &cover_path);
                }
            }
        }
        return Ok(ScanResult {
            series: ScannedSeries {
                path: d.key.clone(),
                title: meta.title,
                year: None,
                volume: None,
                kind: SeriesKind::File,
                library_root: d.root.clone(),
                total_pages: Some(meta.chapter_count),
            },
            issues: vec![IssueRow {
                path: d.key.clone(),
                name: d.raw_name.clone(),
                number: String::new(),
                subtitle: String::new(),
                page_count: Some(meta.chapter_count),
            }],
        });
    }

    let mut paths = issue_paths(d);
    // Plain .zip/.rar must really hold pages; image folders need a few.
    paths.retain(|p| if p.is_dir() { comic::count_pages(p).unwrap_or(0) >= 3 } else { comic::looks_like_comic(p) });
    if paths.is_empty() {
        return Err(format!("no issues found in {}", d.key));
    }
    let is_new = known.is_empty();
    let issues = issue_rows(&paths, &known, true);

    let mut clean = naming::clean_series_name(&d.raw_name);

    // Embedded ComicInfo.xml beats any name guessing. Only checked for new
    // series — it's an extra archive read (a full header pass for RAR).
    if is_new {
        if let Some(info) = comic::find_comic_info(&paths[0]).map(|x| crate::comicinfo::parse(&x)) {
            if let Some(series) = info.series.filter(|s| !s.trim().is_empty()) {
                clean.display = series.clone();
                clean.search = series;
            }
            if let Some(y) = info.year.filter(|y| *y > 1900) {
                clean.year = Some(y as u32);
            }
        }
    }

    let cover_path = library::chapter_cover_path(&d.key, 0);
    if !cover_path.exists() {
        if let Ok(page) = comic::first_page(&paths[0]) {
            if let Some(img) = image::RgbaImage::from_raw(page.width, page.height, page.rgba) {
                let _ = save_thumb(&img, &cover_path);
            }
        }
    }

    let total = issues.iter().map(|i| i.page_count.unwrap_or(0)).sum::<usize>();
    Ok(ScanResult {
        series: ScannedSeries {
            path: d.key.clone(),
            title: clean.display,
            year: clean.year,
            volume: clean.volume,
            kind: d.kind,
            library_root: d.root.clone(),
            total_pages: (total > 0).then_some(total),
        },
        issues,
    })
}

/// Resizes to a cover thumbnail (cropping to the thumbnail's aspect so
/// covers fill their card) and writes it as PNG.
pub fn save_thumb(img: &image::RgbaImage, path: &Path) -> Result<(), String> {
    let thumb = make_thumb(img);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    thumb.save(path).map_err(|e| format!("save cover: {e}"))
}

pub fn make_thumb(img: &image::RgbaImage) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    // Two-page spreads used as covers: take the right-hand (front) half.
    let view = if w > h && w > 0 {
        image::imageops::crop_imm(img, w / 2, 0, w - w / 2, h).to_image()
    } else {
        img.clone()
    };
    image::DynamicImage::ImageRgba8(view)
        .resize_to_fill(THUMB_W, THUMB_H, image::imageops::FilterType::Triangle)
        .to_rgba8()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs discovery + scanning against a real folder: set
    /// `COSMIC_COMIC_SCAN_TEST=/path/to/library` and run with --nocapture.
    #[test]
    fn scan_real_library() {
        let Ok(root) = std::env::var("COSMIC_COMIC_SCAN_TEST") else { return };
        for d in discover(Path::new(&root)) {
            let paths = issue_paths(&d);
            let rows = issue_rows(&paths, &HashMap::new(), false);
            let clean = naming::clean_series_name(&d.raw_name);
            println!("{:?} {:?}  ->  {:?} (year {:?}, vol {:?}) [{} issues]", d.kind, d.raw_name, clean.display, clean.year, clean.volume, rows.len());
            for r in rows.iter().take(4) {
                println!("      #{:<5} {:?}  sub={:?}", r.number, r.name, r.subtitle);
            }
        }
    }
}
