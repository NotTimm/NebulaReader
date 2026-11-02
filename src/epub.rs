//! EPUB parsing into a simple, styled block model the book view renders.
//!
//! - Inline styling (italic/bold/small caps) from tags *and* from the
//!   book's CSS classes — most commercial EPUBs italicize with
//!   `<span class="txit">`, not `<em>`.
//! - Whitespace is collapsed the way a browser does it, so inline runs like
//!   a drop cap `<span>T</span>he` stay one word.
//! - Chapter titles and the table of contents come from the book's own
//!   navigation (EPUB 3 nav document or EPUB 2 NCX), falling back to the
//!   first heading.
//! - Images stay at their position in the text.
//!
//! There's deliberately no general CSS engine: the reader applies its own
//! typography (font, size, spacing, width, theme) and only takes semantic
//! hints from the publisher's styles.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub italic: bool,
    pub bold: bool,
    pub small_caps: bool,
}

/// A run of text with one style.
#[derive(Debug, Clone)]
pub struct Inline {
    pub text: String,
    pub style: Style,
    /// Where it links to: a place in the book (`OEBPS/ch3.xhtml#note-2`,
    /// see [`EpubBook::chapter_for_link`]) or a web address.
    pub link: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Default,
    Center,
    Right,
}

/// One renderable unit of chapter content.
#[derive(Debug, Clone)]
pub enum Block {
    /// Heading level 1-6 (1 = biggest).
    Heading { level: u8, spans: Vec<Inline> },
    /// `indent` is false when the publisher marked it "noindent" (first
    /// paragraph of a chapter, after a break…).
    Paragraph { spans: Vec<Inline>, align: Align, indent: bool },
    Quote(Vec<Inline>),
    ListItem(Vec<Inline>),
    /// A scene break / horizontal rule.
    Rule,
    /// Index into [`EpubBook::images`].
    Image(usize),
}

#[derive(Debug, Clone)]
pub struct EpubChapter {
    pub title: String,
    pub blocks: Vec<Block>,
    pub words: usize,
}

#[derive(Debug, Clone)]
pub struct TocEntry {
    pub title: String,
    pub chapter: usize,
    /// Nesting depth, 0 = top level.
    pub level: usize,
}

#[derive(Debug, Clone, Default)]
pub struct EpubBook {
    pub title: String,
    pub author: Option<String>,
    /// Raw encoded cover image bytes, if the book declares one.
    pub cover: Option<Vec<u8>>,
    pub chapters: Vec<EpubChapter>,
    pub toc: Vec<TocEntry>,
    /// Places links can point to, `file` or `file#id`, and the chapter
    /// each is in.
    pub anchors: HashMap<String, usize>,
    /// Encoded image bytes referenced by [`Block::Image`]; the app turns
    /// each into one image handle up front, not per frame.
    pub images: Vec<Vec<u8>>,
}

/// What the reader's top bar calls a part of a book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChapterLabel {
    /// A real chapter: the `n`th of the book's `of` numbered chapters (the
    /// cover, contents and preface before them don't count).
    Numbered { n: usize, of: usize },
    /// Anything else, by its own title ("Contents", "Preface").
    Named(String),
    /// A part with no title: its place among all of them.
    Section { n: usize, of: usize },
}

/// Whether a title names a numbered chapter: "Chapter I", "CHAPTER 12.
/// Loomings", "Chapter One", "Capítulo 3", or a bare "IV." or "12".
pub fn is_numbered_chapter(title: &str) -> bool {
    const WORDS: &[&str] = &["chapter", "ch", "chap", "capítulo", "capitulo", "chapitre", "kapitel", "capitolo", "hoofdstuk", "rozdział", "глава"];
    let split = |c: char| c.is_whitespace() || matches!(c, '.' | ':' | '—' | '–' | '-' | ',');
    let lower = title.trim().to_lowercase();
    let mut tokens = lower.split(split).filter(|t| !t.is_empty());
    let Some(first) = tokens.next() else { return false };
    if WORDS.contains(&first) {
        // "Chapter" and then anything: a numeral, a word ("One"), "the First".
        return tokens.next().is_some();
    }
    // A bare numeral, alone or before the chapter's name: digits, or a
    // Roman numeral in capitals ("IV", not "Mid").
    // It's the whole title or punctuation follows ("IV.", "XIV: The
    // Storm"); a word after it is a name ("I Am Legend", "12 Angry Men").
    let trimmed = title.trim();
    let first = trimmed.split(split).find(|t| !t.is_empty()).unwrap_or_default();
    let after = trimmed[first.len()..].chars().next();
    let alone = after.is_none_or(|c| matches!(c, '.' | ':' | '—' | '–' | '-' | ')'));
    alone && ((first.len() <= 4 && first.chars().all(|c| c.is_ascii_digit())) || roman(first).is_some())
}

/// A Roman numeral in capitals, as a number ("XIV" → 14); `None` for
/// anything that isn't one written the usual way.
fn roman(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 9 || !s.chars().all(|c| "IVXLCDM".contains(c)) {
        return None;
    }
    let value = |c| match c {
        'I' => 1,
        'V' => 5,
        'X' => 10,
        'L' => 50,
        'C' => 100,
        'D' => 500,
        _ => 1000,
    };
    let digits: Vec<u32> = s.chars().map(value).collect();
    let n: u32 = digits.iter().enumerate().map(|(i, &d)| if digits.get(i + 1).is_some_and(|&next| next > d) { 0u32.wrapping_sub(d) } else { d }).fold(0u32, u32::wrapping_add);
    // Written back the usual way, it must come out the same.
    const TABLE: &[(u32, &str)] = &[(1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"), (50, "L"), (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I")];
    let mut rest = n;
    let mut out = String::new();
    for &(v, r) in TABLE {
        while rest >= v {
            out.push_str(r);
            rest -= v;
        }
    }
    // No book has a chapter MIX (1009): high ones are words that happen
    // to be numerals.
    (n > 0 && n <= 300 && out == s).then_some(n)
}

impl EpubBook {
    /// How the top bar names part `i`: "Chapter n of m" for a real chapter,
    /// counting only those, else its title.
    pub fn chapter_label(&self, i: usize) -> ChapterLabel {
        let numbered: Vec<bool> = self.chapters.iter().map(|c| is_numbered_chapter(&c.title)).collect();
        let of = numbered.iter().filter(|n| **n).count();
        let title = self.chapters.get(i).map(|c| c.title.trim()).unwrap_or_default();
        // Two or more numbered chapters: the book counts its chapters.
        if of >= 2 && numbered.get(i).copied().unwrap_or(false) {
            let n = numbered[..=i].iter().filter(|n| **n).count();
            return ChapterLabel::Numbered { n, of };
        }
        if title.is_empty() {
            ChapterLabel::Section { n: i + 1, of: self.chapters.len() }
        } else {
            ChapterLabel::Named(title.to_string())
        }
    }

    pub fn total_words(&self) -> usize {
        self.chapters.iter().map(|c| c.words).sum()
    }

    /// The chapter a link inside the book goes to: the one its anchor is
    /// in, else the start of its file.
    pub fn chapter_for_link(&self, target: &str) -> Option<usize> {
        self.anchors.get(target).or_else(|| self.anchors.get(target.split('#').next()?)).copied()
    }
}

/// A link's target from a file at `base`: a web address as it is, a place
/// in the book as `file#id`.
fn resolve_link(base: &str, href: &str) -> String {
    let href = href.trim();
    if href.contains("://") || href.starts_with("mailto:") {
        return href.to_string();
    }
    let (file, fragment) = href.split_once('#').unwrap_or((href, ""));
    let path = if file.is_empty() {
        base.to_string()
    } else {
        join_epub_path(Path::new(base).parent().unwrap_or(Path::new("")), file)
    };
    if fragment.is_empty() { path } else { format!("{path}#{fragment}") }
}

pub fn is_epub_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".epub")
}

pub fn open(path: &Path) -> Result<EpubBook, String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| format!("failed to read epub {}: {e}", path.display()))?;

    let opf_path = find_opf_path(&mut zip)?;
    let opf_dir = Path::new(&opf_path).parent().unwrap_or(Path::new("")).to_path_buf();
    let opf_xml = read_zip_text(&mut zip, &opf_path)?;

    let opf = parse_opf(&opf_xml);

    let cover = opf
        .cover_id
        .as_ref()
        .and_then(|id| opf.manifest.get(id))
        .map(|item| join_epub_path(&opf_dir, &item.href))
        .and_then(|p| read_zip_bytes(&mut zip, &p).ok());

    // Publisher CSS, only for semantic hints (italic classes etc.).
    let mut css = CssHints::default();
    for item in opf.manifest.values().filter(|i| i.media_type == "text/css") {
        if let Ok(text) = read_zip_text(&mut zip, &join_epub_path(&opf_dir, &item.href)) {
            css.add_stylesheet(&text);
        }
    }

    // Book navigation: chapter-file path -> [(title, level)].
    let toc_raw = read_toc(&mut zip, &opf, &opf_dir);

    let mut images: Vec<Vec<u8>> = Vec::new();
    let mut image_index: HashMap<String, usize> = HashMap::new();
    let mut chapters = Vec::new();
    let mut chapter_for_path: HashMap<String, usize> = HashMap::new();

    // Where each table-of-contents anchor landed: (file, id) -> chapter.
    let mut chapter_for_anchor: HashMap<(String, String), usize> = HashMap::new();

    for idref in &opf.spine {
        let Some(item) = opf.manifest.get(idref) else { continue };
        let chapter_path = join_epub_path(&opf_dir, &item.href);
        let Ok(xhtml) = read_zip_text(&mut zip, &chapter_path) else { continue };
        let chapter_dir = Path::new(&chapter_path).parent().unwrap_or(Path::new("")).to_path_buf();

        // A file holding several chapters (Project Gutenberg puts a dozen
        // in each) is split where the table of contents points into it,
        // at its top level there, so each chapter is one of ours: its own
        // title, pages and place in the contents. Lower-level entries
        // (sections) stay inside their chapter.
        let entries: Vec<&RawToc> = toc_raw.iter().filter(|t| t.path == chapter_path).collect();
        let split_level = entries.iter().filter(|t| t.fragment.is_some()).map(|t| t.level).min();
        let wanted: std::collections::HashSet<String> = entries.iter().filter_map(|t| t.fragment.clone()).collect();
        let split_at: HashMap<&str, &str> = entries
            .iter()
            .filter(|t| Some(t.level) == split_level)
            .filter_map(|t| Some((t.fragment.as_deref()?, t.title.as_str())))
            .collect();

        // (title from the contents, blocks), and which part each anchor is in.
        let file_title = entries.iter().find(|t| t.fragment.is_none()).map(|t| t.title.clone());
        let mut parts: Vec<(Option<String>, Vec<Block>)> = vec![(file_title, Vec::new())];
        let mut anchor_part: Vec<(String, usize)> = Vec::new();
        // Ids seen since the last text or picture. A link target is often
        // an empty marker a little before what it's for (Gutenberg puts
        // `<a id="link2HCH0001">` before some spacing and the chapter's
        // heading), so an id goes with the next content, in whichever
        // chapter that turns out to be, not the content before it.
        let mut waiting: Vec<String> = Vec::new();
        let place = |waiting: &mut Vec<String>, anchor_part: &mut Vec<(String, usize)>, part: usize| {
            anchor_part.extend(waiting.drain(..).map(|id| (id, part)));
        };
        for block in parse_xhtml_body(&xhtml, &chapter_path, &css, &wanted) {
            match block {
                Parsed::Anchor(id) => {
                    if let Some(title) = split_at.get(id.as_str()) {
                        if parts.last().is_some_and(|(_, b)| !b.is_empty()) {
                            parts.push((Some(title.to_string()), Vec::new()));
                        } else if let Some(last) = parts.last_mut() {
                            // Nothing before it: this part is that chapter.
                            last.0 = Some(title.to_string());
                        }
                    }
                    waiting.push(id);
                }
                Parsed::Id(id) => waiting.push(id),
                Parsed::Block(b) => {
                    place(&mut waiting, &mut anchor_part, parts.len() - 1);
                    parts.last_mut().unwrap().1.push(b);
                }
                Parsed::ImageSrc(src) => {
                    let p = join_epub_path(&chapter_dir, &src);
                    let idx = match image_index.get(&p) {
                        Some(&i) => Some(i),
                        None => read_zip_bytes(&mut zip, &p).ok().map(|bytes| {
                            images.push(bytes);
                            image_index.insert(p.clone(), images.len() - 1);
                            images.len() - 1
                        }),
                    };
                    // Glyph stand-ins and social icons (a few pixels
                    // tall) mean nothing as a block of their own.
                    let tiny = idx.and_then(|i| image_size(&images[i])).is_some_and(|(w, h)| w.max(h) <= 32);
                    if let (Some(i), false) = (idx, tiny) {
                        place(&mut waiting, &mut anchor_part, parts.len() - 1);
                        parts.last_mut().unwrap().1.push(Block::Image(i));
                    }
                }
            }
        }
        // Ids at the very end: the next chapter (the start of the next file).
        place(&mut waiting, &mut anchor_part, parts.len());

        // Parts become chapters (empty ones are dropped: an anchor in one
        // means the next chapter).
        let mut part_chapter = Vec::with_capacity(parts.len());
        for (title, blocks) in parts {
            part_chapter.push(chapters.len());
            if blocks.is_empty() {
                continue;
            }
            let words = blocks
                .iter()
                .map(|b| match b {
                    Block::Heading { spans, .. }
                    | Block::Paragraph { spans, .. }
                    | Block::Quote(spans)
                    // The whole block's text: a link ending mid-word or
                    // before a comma doesn't make an extra word.
                    | Block::ListItem(spans) => spans.iter().map(|s| s.text.as_str()).collect::<String>().split_whitespace().count(),
                    _ => 0,
                })
                .sum();
            let title = title
                .or_else(|| {
                    blocks.iter().find_map(|b| match b {
                        Block::Heading { spans, .. } => Some(spans_text(spans)),
                        _ => None,
                    })
                })
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_default();
            chapter_for_path.entry(chapter_path.clone()).or_insert(chapters.len());
            chapters.push(EpubChapter { title, blocks, words });
        }
        for (id, part) in anchor_part {
            // Past the last part: the next chapter, the next file's first.
            let chapter = part_chapter.get(part).copied().unwrap_or(chapters.len());
            chapter_for_anchor.insert((chapter_path.clone(), id), chapter);
        }
    }

    if chapters.is_empty() {
        return Err(fl!("no-readable-chapters", path = path.display().to_string()));
    }

    // Untitled chapters get a positional name.
    for (i, ch) in chapters.iter_mut().enumerate() {
        if ch.title.is_empty() {
            ch.title = fl!("section-number", number = i + 1);
        }
    }

    let mut toc: Vec<TocEntry> = toc_raw
        .into_iter()
        .filter_map(|t| {
            let at_anchor = t.fragment.and_then(|f| chapter_for_anchor.get(&(t.path.clone(), f)).copied());
            let chapter = at_anchor.or_else(|| chapter_for_path.get(&t.path).copied())?;
            // An anchor past the last chapter (empty end of a file).
            (chapter < chapters.len()).then_some(TocEntry { title: t.title, chapter, level: t.level })
        })
        .collect();
    // Where links can go: each file's start and each id in it.
    let mut anchors: HashMap<String, usize> = chapter_for_path.clone();
    for ((path, id), chapter) in &chapter_for_anchor {
        if *chapter < chapters.len() {
            anchors.insert(format!("{path}#{id}"), *chapter);
        }
    }
    if toc.is_empty() {
        toc = chapters
            .iter()
            .enumerate()
            .map(|(i, c)| TocEntry { title: c.title.clone(), chapter: i, level: 0 })
            .collect();
    }

    Ok(EpubBook {
        title: if opf.title.is_empty() {
            path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
        } else {
            opf.title
        },
        author: opf.author,
        cover,
        chapters,
        toc,
        anchors,
        images,
    })
}

pub fn spans_text(spans: &[Inline]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect::<String>().trim().to_string()
}

/// Title/author/cover/chapter count without parsing any chapter — what the
/// library scan needs, at a fraction of the cost of [`open`].
pub struct EpubMeta {
    pub title: String,
    pub author: Option<String>,
    pub date: Option<String>,
    pub cover: Option<Vec<u8>>,
    pub chapter_count: usize,
}

pub fn read_meta(path: &Path) -> Result<EpubMeta, String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| format!("failed to read epub {}: {e}", path.display()))?;
    let opf_path = find_opf_path(&mut zip)?;
    let opf_dir = Path::new(&opf_path).parent().unwrap_or(Path::new("")).to_path_buf();
    let opf_xml = read_zip_text(&mut zip, &opf_path)?;
    let opf = parse_opf(&opf_xml);
    let cover = opf
        .cover_id
        .as_ref()
        .and_then(|id| opf.manifest.get(id))
        .map(|item| join_epub_path(&opf_dir, &item.href))
        .and_then(|p| read_zip_bytes(&mut zip, &p).ok());
    Ok(EpubMeta {
        title: if opf.title.is_empty() {
            path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
        } else {
            opf.title
        },
        author: opf.author,
        date: opf.date,
        cover,
        chapter_count: opf.spine.len(),
    })
}

// ── container.xml / OPF ──────────────────────────────────────────────────────

fn find_opf_path(zip: &mut zip::ZipArchive<std::fs::File>) -> Result<String, String> {
    let xml = read_zip_text(zip, "META-INF/container.xml")
        .map_err(|e| format!("missing container.xml: {e}"))?;

    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) if local_name(&e.name()) == "rootfile" => {
                for attr in e.attributes().flatten() {
                    if local_name_str(attr.key.as_ref()) == "full-path" {
                        return Ok(attr.normalized_value(XmlVersion::default()).unwrap_or_default().into_owned());
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    Err("no <rootfile> found in container.xml".to_string())
}

struct ManifestItem {
    href: String,
    media_type: String,
    properties: String,
}

struct Opf {
    title: String,
    author: Option<String>,
    /// dc:date, usually the publication date ("2019", "2019-02-12", …).
    date: Option<String>,
    manifest: HashMap<String, ManifestItem>,
    spine: Vec<String>,
    cover_id: Option<String>,
    /// `toc="…"` on <spine>: the NCX's manifest id (EPUB 2).
    ncx_id: Option<String>,
}

pub(crate) fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| local_name_str(a.key.as_ref()) == name)
        .map(|a| a.normalized_value(XmlVersion::default()).map(|v| v.into_owned()).unwrap_or_else(|_| {
            a.value.to_string()
        }))
}

fn parse_opf(xml: &str) -> Opf {
    let mut reader = Reader::from_str(xml);
    // Not trimmed: the spaces around an entity (`A &amp; B`) are text
    // events of their own. The whole text is trimmed instead.
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();

    let mut opf = Opf {
        title: String::new(),
        author: None,
        date: None,
        manifest: HashMap::new(),
        spine: Vec::new(),
        cover_id: None,
        ncx_id: None,
    };
    let mut in_title = false;
    let mut in_creator = false;
    let mut in_date = false;
    // Text arrives in pieces (an entity like `&amp;` is its own event).
    let mut text = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(&e.name()).as_str() {
                "title" => {
                    in_title = opf.title.is_empty();
                    text.clear();
                }
                "creator" => {
                    in_creator = opf.author.is_none();
                    text.clear();
                }
                "date" => {
                    in_date = opf.date.is_none();
                    text.clear();
                }
                "spine" => opf.ncx_id = attr(&e, "toc"),
                _ => {}
            },
            Ok(ref ev @ (Event::Text(_) | Event::GeneralRef(_))) => {
                if in_title || in_creator || in_date {
                    text.push_str(&event_text(ev));
                }
            }
            Ok(Event::End(e)) => match local_name(&e.name()).as_str() {
                "title" if in_title => {
                    opf.title = text.trim().to_string();
                    in_title = false;
                }
                "creator" if in_creator => {
                    opf.author = Some(text.trim().to_string());
                    in_creator = false;
                }
                "date" if in_date => {
                    let date = text.trim();
                    if !date.is_empty() {
                        // Keep just the date part of "2019-02-12T00:00:00Z".
                        opf.date = Some(date.split('T').next().unwrap_or(date).to_string());
                    }
                    in_date = false;
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                let name = local_name(&e.name());
                if name == "item" {
                    if let (Some(id), Some(href)) = (attr(&e, "id"), attr(&e, "href")) {
                        let properties = attr(&e, "properties").unwrap_or_default();
                        if properties.contains("cover-image") {
                            opf.cover_id = Some(id.clone());
                        }
                        opf.manifest.insert(
                            id,
                            ManifestItem {
                                href,
                                media_type: attr(&e, "media-type").unwrap_or_default(),
                                properties,
                            },
                        );
                    }
                } else if name == "itemref" {
                    if attr(&e, "linear").as_deref() != Some("no") {
                        if let Some(idref) = attr(&e, "idref") {
                            opf.spine.push(idref);
                        }
                    }
                } else if name == "meta" && attr(&e, "name").as_deref() == Some("cover") {
                    if opf.cover_id.is_none() {
                        opf.cover_id = attr(&e, "content");
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    opf
}

// ── Table of contents ────────────────────────────────────────────────────────

/// (chapter file path in the zip, title, nesting level), in reading order.
/// A table-of-contents entry as the book's navigation gives it.
#[derive(Debug, Clone, PartialEq)]
struct RawToc {
    /// The file it points into.
    path: String,
    /// The anchor in that file (`#chapter-3`), when it points inside it.
    fragment: Option<String>,
    title: String,
    level: usize,
}

impl RawToc {
    fn new(dir: &Path, href: &str, title: String, level: usize) -> Self {
        let fragment = href.split_once('#').map(|(_, f)| f.to_string()).filter(|f| !f.is_empty());
        RawToc { path: join_epub_path(dir, href), fragment, title, level }
    }
}

fn read_toc(
    zip: &mut zip::ZipArchive<std::fs::File>,
    opf: &Opf,
    opf_dir: &Path,
) -> Vec<RawToc> {
    // EPUB 3 nav document.
    if let Some(item) = opf.manifest.values().find(|i| i.properties.split_whitespace().any(|p| p == "nav")) {
        let nav_path = join_epub_path(opf_dir, &item.href);
        if let Ok(xml) = read_zip_text(zip, &nav_path) {
            let dir = Path::new(&nav_path).parent().unwrap_or(Path::new("")).to_path_buf();
            let toc = parse_nav(&xml, &dir);
            if !toc.is_empty() {
                return toc;
            }
        }
    }
    // EPUB 2 NCX.
    let ncx = opf
        .ncx_id
        .as_ref()
        .and_then(|id| opf.manifest.get(id))
        .or_else(|| opf.manifest.values().find(|i| i.media_type == "application/x-dtbncx+xml"));
    if let Some(item) = ncx {
        let ncx_path = join_epub_path(opf_dir, &item.href);
        if let Ok(xml) = read_zip_text(zip, &ncx_path) {
            let dir = Path::new(&ncx_path).parent().unwrap_or(Path::new("")).to_path_buf();
            return parse_ncx(&xml, &dir);
        }
    }
    Vec::new()
}

fn parse_ncx(xml: &str, dir: &Path) -> Vec<RawToc> {
    let mut reader = Reader::from_str(xml);
    // Not trimmed: the spaces around an entity (`A &amp; B`) are text
    // events of their own. The whole text is trimmed instead.
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut in_text = false;
    let mut label = String::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(&e.name()).as_str() {
                "navPoint" => {
                    depth += 1;
                    label.clear();
                }
                "text" => in_text = true,
                _ => {}
            },
            Ok(ref ev @ (Event::Text(_) | Event::GeneralRef(_))) if in_text => label.push_str(&event_text(ev)),
            Ok(Event::End(e)) => match local_name(&e.name()).as_str() {
                "navPoint" => depth = depth.saturating_sub(1),
                "text" => in_text = false,
                _ => {}
            },
            Ok(Event::Empty(e)) if local_name(&e.name()) == "content" => {
                if let Some(src) = attr(&e, "src") {
                    let label = collapse(&label);
                    if !label.is_empty() {
                        out.push(RawToc::new(dir, &src, label, depth.saturating_sub(1)));
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

fn parse_nav(xml: &str, dir: &Path) -> Vec<RawToc> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut in_toc_nav = false;
    let mut nav_depth = 0usize;
    let mut ol_depth = 0usize;
    let mut current: Option<(String, String)> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(&e.name()).as_str() {
                "nav" => {
                    nav_depth += 1;
                    let kind = attr(&e, "type").unwrap_or_default();
                    if kind.contains("toc") {
                        in_toc_nav = true;
                    }
                }
                "ol" if in_toc_nav => ol_depth += 1,
                "a" if in_toc_nav => {
                    current = attr(&e, "href").map(|h| (h, String::new()));
                }
                _ => {}
            },
            Ok(ref ev @ (Event::Text(_) | Event::GeneralRef(_))) => {
                if let Some((_, label)) = current.as_mut() {
                    label.push_str(&event_text(ev));
                }
            }
            Ok(Event::End(e)) => match local_name(&e.name()).as_str() {
                "nav" => {
                    nav_depth = nav_depth.saturating_sub(1);
                    if in_toc_nav {
                        in_toc_nav = false;
                        if !out.is_empty() {
                            break;
                        }
                    }
                }
                "ol" if in_toc_nav => ol_depth = ol_depth.saturating_sub(1),
                "a" => {
                    if let Some((href, label)) = current.take() {
                        let label = collapse(&label);
                        if !label.is_empty() {
                            out.push(RawToc::new(dir, &href, label, ol_depth.saturating_sub(1)));
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    let _ = nav_depth;
    out
}

// ── Publisher CSS hints ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default)]
struct Hints {
    italic: Option<bool>,
    bold: Option<bool>,
    small_caps: bool,
    align: Option<Align>,
    no_indent: bool,
    hidden: bool,
}

impl Hints {
    fn merge(&mut self, o: &Hints) {
        if o.italic.is_some() {
            self.italic = o.italic;
        }
        if o.bold.is_some() {
            self.bold = o.bold;
        }
        self.small_caps |= o.small_caps;
        if o.align.is_some() {
            self.align = o.align;
        }
        self.no_indent |= o.no_indent;
        self.hidden |= o.hidden;
    }
}

fn parse_declarations(decls: &str) -> Hints {
    let mut h = Hints::default();
    for decl in decls.split(';') {
        let Some((prop, value)) = decl.split_once(':') else { continue };
        let prop = prop.trim().to_ascii_lowercase();
        let value = value.trim().to_ascii_lowercase();
        let value = value.trim_end_matches("!important").trim();
        match prop.as_str() {
            "font-style" => h.italic = Some(value == "italic" || value == "oblique"),
            "font-weight" => {
                h.bold = Some(value == "bold" || value == "bolder" || value.parse::<u32>().is_ok_and(|w| w >= 600))
            }
            "font-variant" | "font-variant-caps" => h.small_caps = value.contains("small-caps"),
            "text-align" => {
                h.align = Some(match value {
                    "center" => Align::Center,
                    "right" | "end" => Align::Right,
                    _ => Align::Default,
                })
            }
            "text-indent" => {
                h.no_indent = value.starts_with('0') || value.starts_with('-');
            }
            "display" => h.hidden = value == "none",
            _ => {}
        }
    }
    h
}

/// Class-selector rules from the book's stylesheets.
#[derive(Default)]
struct CssHints {
    by_class: HashMap<String, Hints>,
}

impl CssHints {
    fn add_stylesheet(&mut self, css: &str) {
        // Strip comments.
        let mut text = String::with_capacity(css.len());
        let mut rest = css;
        while let Some(start) = rest.find("/*") {
            text.push_str(&rest[..start]);
            rest = match rest[start..].find("*/") {
                Some(end) => &rest[start + end + 2..],
                None => "",
            };
        }
        text.push_str(rest);

        for rule in text.split('}') {
            let Some((selectors, decls)) = rule.split_once('{') else { continue };
            if selectors.trim_start().starts_with('@') {
                continue;
            }
            let hints = parse_declarations(decls);
            for sel in selectors.split(',') {
                // Only simple `.class` / `tag.class` selectors (the last
                // compound of a descendant selector).
                let last = sel.split_whitespace().last().unwrap_or("");
                if last.contains(':') || last.contains('[') {
                    continue;
                }
                if let Some((_, class)) = last.split_once('.') {
                    let class = class.split('.').next().unwrap_or(class);
                    if !class.is_empty() {
                        self.by_class.entry(class.to_string()).or_default().merge(&hints);
                    }
                }
            }
        }
    }

    fn for_element(&self, e: &BytesStart) -> Hints {
        let mut h = Hints::default();
        if let Some(classes) = attr(e, "class") {
            for c in classes.split_whitespace() {
                if let Some(ch) = self.by_class.get(c) {
                    h.merge(ch);
                }
                // Common class names, for books whose CSS we can't read.
                let lc = c.to_ascii_lowercase();
                if lc == "noindent" || lc == "no-indent" || lc.starts_with("first") {
                    h.no_indent = true;
                }
                if lc == "center" || lc == "centered" {
                    h.align = Some(Align::Center);
                }
            }
        }
        if let Some(style) = attr(e, "style") {
            h.merge(&parse_declarations(&style));
        }
        h
    }
}

// ── XHTML chapter body ───────────────────────────────────────────────────────

enum Parsed {
    Block(Block),
    ImageSrc(String),
    /// An element with one of the ids asked for: where a chapter the
    /// table of contents points to starts.
    Anchor(String),
    /// Any other element's id: somewhere a link can go.
    Id(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Para,
    Heading(u8),
    Quote,
    Li,
}

struct Frame {
    name: String,
    hints: Hints,
    /// Is this element block-level (starts/ends a block)?
    block: Option<Kind>,
    /// A link (`<a href>`): its resolved target.
    link: Option<String>,
}

fn block_kind(name: &str) -> Option<Kind> {
    Some(match name {
        "h1" => Kind::Heading(1),
        "h2" => Kind::Heading(2),
        "h3" => Kind::Heading(3),
        "h4" => Kind::Heading(4),
        "h5" => Kind::Heading(5),
        "h6" => Kind::Heading(6),
        "blockquote" => Kind::Quote,
        "li" | "dt" | "dd" => Kind::Li,
        "p" | "div" | "section" | "article" | "aside" | "header" | "footer" | "figure"
        | "figcaption" | "pre" | "table" | "tr" | "td" | "th" | "ul" | "ol" | "dl" | "body"
        | "main" | "address" | "center" => Kind::Para,
        _ => return None,
    })
}

struct BodyParser<'c> {
    css: &'c CssHints,
    /// The file being read, for resolving links.
    base: &'c str,
    /// Ids to report where they are (table-of-contents targets).
    anchors: &'c std::collections::HashSet<String>,
    out: Vec<Parsed>,
    stack: Vec<Frame>,
    spans: Vec<Inline>,
    /// Whitespace seen but not yet emitted (collapsed to one space).
    pending_space: bool,
}

impl BodyParser<'_> {
    fn style(&self) -> (Style, bool) {
        let mut h = Hints::default();
        for f in &self.stack {
            h.merge(&f.hints);
        }
        let mut st = Style { italic: h.italic.unwrap_or(false), bold: h.bold.unwrap_or(false), small_caps: h.small_caps };
        for f in &self.stack {
            match f.name.as_str() {
                "em" | "i" | "cite" | "var" | "dfn" => st.italic = true,
                "strong" | "b" => st.bold = true,
                _ => {}
            }
        }
        (st, h.hidden)
    }

    fn push_text(&mut self, raw: &str) {
        let (style, hidden) = self.style();
        if hidden {
            return;
        }
        for (i, piece) in raw.split(|c: char| c.is_whitespace() && c != '\u{a0}').enumerate() {
            if i > 0 {
                self.pending_space = true;
            }
            if piece.is_empty() {
                continue;
            }
            let has_content = !self.spans.is_empty();
            let link = self.stack.iter().rev().find_map(|f| f.link.clone());
            // The space between two runs goes with the first, unless that's
            // a link and this isn't (a link's underline stops at its words).
            if self.pending_space && has_content {
                match self.spans.last_mut() {
                    Some(last) if last.link.is_some() && last.link != link => {
                        self.spans.push(Inline { text: " ".into(), style, link: None })
                    }
                    Some(last) => last.text.push(' '),
                    None => {}
                }
            }
            self.pending_space = false;
            match self.spans.last_mut() {
                Some(last) if last.style == style && last.link == link => last.text.push_str(piece),
                _ => self.spans.push(Inline { text: piece.to_string(), style, link }),
            }
        }
    }

    fn line_break(&mut self) {
        if let Some(last) = self.spans.last_mut() {
            last.text.push('\n');
            self.pending_space = false;
        }
    }

    /// Ends the current block, emitting it with the innermost block
    /// ancestor's kind and hints.
    fn flush(&mut self) {
        self.pending_space = false;
        let spans: Vec<Inline> = std::mem::take(&mut self.spans);
        if spans.iter().all(|s| s.text.trim().is_empty()) {
            return;
        }
        let mut kind = Kind::Para;
        let mut align = Align::Default;
        let mut no_indent = false;
        for f in self.stack.iter() {
            if let Some(k) = f.block {
                match k {
                    Kind::Quote | Kind::Li => kind = k,
                    Kind::Heading(_) => kind = k,
                    Kind::Para => {
                        if !matches!(kind, Kind::Quote | Kind::Li) {
                            kind = Kind::Para;
                        }
                    }
                }
            }
            if let Some(a) = f.hints.align {
                align = a;
            }
            no_indent |= f.hints.no_indent;
        }
        let mut spans = spans;
        if let Some(first) = spans.first_mut() {
            first.text = first.text.trim_start().to_string();
        }
        if let Some(last) = spans.last_mut() {
            last.text = last.text.trim_end().to_string();
        }
        let block = match kind {
            Kind::Heading(level) => Block::Heading { level, spans },
            Kind::Quote => Block::Quote(spans),
            Kind::Li => Block::ListItem(spans),
            Kind::Para => Block::Paragraph { spans, align, indent: !no_indent && align == Align::Default },
        };
        self.out.push(Parsed::Block(block));
    }

    fn open(&mut self, e: &BytesStart, empty: bool) {
        let name = local_name(&e.name()).to_ascii_lowercase();
        // A block element ends the one before it first, so its id comes
        // after that block, with its own content.
        if block_kind(&name).is_some() {
            self.flush();
        }
        if let Some(id) = attr(e, "id").or_else(|| if name == "a" { attr(e, "name") } else { None }) {
            if self.anchors.contains(&id) {
                self.flush();
                self.out.push(Parsed::Anchor(id));
            } else {
                self.out.push(Parsed::Id(id));
            }
        }
        match name.as_str() {
            "br" => return self.line_break(),
            "hr" => {
                self.flush();
                self.out.push(Parsed::Block(Block::Rule));
                return;
            }
            "img" | "image" => {
                if self.style().1 {
                    return;
                }
                let src = attr(e, "src").or_else(|| attr(e, "href"));
                if let Some(src) = src {
                    self.flush();
                    self.out.push(Parsed::ImageSrc(src));
                }
                return;
            }
            _ => {}
        }
        let block = block_kind(&name);
        if block.is_some() {
            self.flush();
        }
        if !empty {
            let hints = self.css.for_element(e);
            let link = if name == "a" { attr(e, "href").filter(|h| !h.trim().is_empty()).map(|h| resolve_link(self.base, &h)) } else { None };
            self.stack.push(Frame { name, hints, block, link });
        }
    }

    fn close(&mut self, name: &str) {
        let name = name.to_ascii_lowercase();
        let Some(pos) = self.stack.iter().rposition(|f| f.name == name) else { return };
        if self.stack[pos..].iter().any(|f| f.block.is_some()) {
            self.flush();
        }
        self.stack.truncate(pos);
    }
}

fn parse_xhtml_body(xhtml: &str, base: &str, css: &CssHints, anchors: &std::collections::HashSet<String>) -> Vec<Parsed> {
    let mut reader = Reader::from_str(xhtml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;
    let mut buf = Vec::new();

    let mut p = BodyParser { css, base, anchors, out: Vec::new(), stack: Vec::new(), spans: Vec::new(), pending_space: false };
    let mut in_body = false;
    let mut skip_depth = 0usize; // inside <head>/<style>/<script>

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(&e.name()).to_ascii_lowercase();
                if matches!(name.as_str(), "head" | "style" | "script" | "title") {
                    skip_depth += 1;
                } else if skip_depth == 0 {
                    if name == "body" {
                        in_body = true;
                    }
                    if in_body || name == "svg" {
                        in_body = true;
                        p.open(&e, false);
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                if skip_depth == 0 && in_body {
                    p.open(&e, true);
                } else if skip_depth == 0 {
                    // Some books put an <image> in a bare <svg> cover page.
                    let name = local_name(&e.name());
                    if name == "image" || name == "img" {
                        p.open(&e, true);
                    }
                }
            }
            Ok(ref ev @ (Event::Text(_) | Event::GeneralRef(_))) => {
                if skip_depth == 0 && in_body {
                    p.push_text(&event_text(ev));
                }
            }
            Ok(Event::CData(t)) => {
                if skip_depth == 0 && in_body {
                    p.push_text(&t);
                }
            }
            Ok(Event::End(e)) => {
                let name = local_name(&e.name()).to_ascii_lowercase();
                if matches!(name.as_str(), "head" | "style" | "script" | "title") {
                    skip_depth = skip_depth.saturating_sub(1);
                } else if skip_depth == 0 {
                    p.close(&name);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    p.flush();
    p.out
}

/// The text of a text node or of an entity reference, which quick-xml
/// (since 0.38) reports as an event of its own; anything else is empty.
pub(crate) fn event_text(ev: &Event) -> String {
    match ev {
        Event::Text(t) => decode_text(t),
        Event::GeneralRef(r) => decode_text(&format!("&{};", &**r)),
        _ => String::new(),
    }
}

/// Resolves XML *and* common HTML entities in text (`&nbsp;`, `&mdash;`…
/// are everywhere in EPUBs but aren't XML).
fn decode_text(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_owned();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest: &str = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let Some(semi) = tail[..tail.len().min(12)].find(';') else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..semi];
        let ch = match ent {
            "amp" => Some("&".to_string()),
            "lt" => Some("<".into()),
            "gt" => Some(">".into()),
            "quot" => Some("\"".into()),
            "apos" => Some("'".into()),
            "nbsp" => Some("\u{a0}".into()),
            "mdash" => Some("—".into()),
            "ndash" => Some("–".into()),
            "hellip" => Some("…".into()),
            "lsquo" => Some("‘".into()),
            "rsquo" => Some("’".into()),
            "ldquo" => Some("“".into()),
            "rdquo" => Some("”".into()),
            "copy" => Some("©".into()),
            "shy" => Some(String::new()),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32).map(String::from)
            }
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32).map(String::from),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push_str(&c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── helpers ──────────────────────────────────────────────────────────────────

pub(crate) fn local_name(name: &quick_xml::name::QName) -> String {
    local_name_str(name.as_ref())
}

fn local_name_str(s: &str) -> String {
    s.rsplit(':').next().unwrap_or(s).to_string()
}

fn join_epub_path(base: &Path, href: &str) -> String {
    let href = href.split(['#', '?']).next().unwrap_or(href);
    let joined = if href.starts_with('/') {
        PathBuf::from(href.trim_start_matches('/'))
    } else {
        base.join(href)
    };
    // Normalize `..`/`.` components (zip entry names are always forward-slash,
    // but `Path::join` uses `\` on Windows, so split on both).
    let joined = joined.to_string_lossy().into_owned();
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn read_zip_text(zip: &mut zip::ZipArchive<std::fs::File>, path: &str) -> Result<String, String> {
    let bytes = read_zip_bytes(zip, path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_zip_bytes(zip: &mut zip::ZipArchive<std::fs::File>, path: &str) -> Result<Vec<u8>, String> {
    let mut entry = zip
        .by_name(path)
        .map_err(|e| format!("entry {path} not found in epub: {e}"))?;
    let mut data = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut data).map_err(|e| format!("failed to read {path}: {e}"))?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    #[test]
    fn real_chapters_are_told_from_the_rest() {
        for t in ["CHAPTER I", "Chapter I. Jonathan Harker's Journal", "CHAPTER 12. Loomings.", "Chapter One", "Capítulo 3", "IV.", "12", "XIV: The Storm", "chapter the first"] {
            assert!(is_numbered_chapter(t), "{t}");
        }
        for t in ["Contents", "PREFACE", "Dracula", "Title Page", "", "Chapter", "Ivanhoe", "Mid-Winter", "Dim Lights", "MIX", "DIM", "I Am Legend", "12 Angry Men"] {
            assert!(!is_numbered_chapter(t), "{t}");
        }
    }

    #[test]
    fn the_top_bar_counts_only_real_chapters() {
        let ch = |t: &str| EpubChapter { title: t.into(), blocks: Vec::new(), words: 0 };
        let book = EpubBook {
            chapters: vec![ch("Cover"), ch("Contents"), ch(""), ch("CHAPTER I"), ch("CHAPTER II"), ch("CHAPTER III"), ch("Appendix")],
            ..Default::default()
        };
        assert_eq!(book.chapter_label(1), ChapterLabel::Named("Contents".into()));
        assert_eq!(book.chapter_label(2), ChapterLabel::Section { n: 3, of: 7 });
        assert_eq!(book.chapter_label(3), ChapterLabel::Numbered { n: 1, of: 3 });
        assert_eq!(book.chapter_label(5), ChapterLabel::Numbered { n: 3, of: 3 });
        assert_eq!(book.chapter_label(6), ChapterLabel::Named("Appendix".into()));
    }

    use super::*;

    /// A file with several chapters, like Project Gutenberg's EPUBs, is
    /// split where the table of contents points into it.
    #[test]
    fn splits_files_at_contents_anchors() {
        use std::io::Write;
        let files: [(&str, &str); 4] = [
            ("META-INF/container.xml", r#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#),
            (
                "OEBPS/content.opf",
                r#"<package><metadata><title>Whale</title></metadata><manifest>
<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="a" href="a.xhtml" media-type="application/xhtml+xml"/>
</manifest><spine><itemref idref="a"/></spine></package>"#,
            ),
            (
                "OEBPS/nav.xhtml",
                r##"<html><body><nav epub:type="toc"><ol>
<li><a href="a.xhtml">Front</a></li>
<li><a href="a.xhtml#c1">Chapter 1</a><ol><li><a href="a.xhtml#s1">A section</a></li></ol></li>
<li><a href="a.xhtml#c2">Chapter 2</a></li>
</ol></nav></body></html>"##,
            ),
            (
                "OEBPS/a.xhtml",
                r##"<html><body><p>Title page. <a href="#link-c2">Chapter 2</a> <a href="https://www.gutenberg.org">Gutenberg</a> <a href="#m2">Two</a></p>
<h2 id="c1">CHAPTER 1</h2><p>Call me Ishmael.</p><p id="s1">A section.</p><p>More of one.</p>
<p><a id="m2"></a></p><div style="height: 4em"><br/><br/></div>
<div><a id="c2"></a></div><h2 id="link-c2">CHAPTER 2</h2><p>The carpet-bag.</p></body></html>"##,
            ),
        ];
        let path = std::env::temp_dir().join(format!("nebula-split-{}.epub", std::process::id()));
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            for (name, body) in files {
                w.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                w.write_all(body.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        let book = open(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let titles: Vec<&str> = book.chapters.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Front", "Chapter 1", "Chapter 2"]);
        // The section stays inside its chapter.
        assert_eq!(book.chapters[1].words, 10);
        let toc: Vec<(&str, usize, usize)> = book.toc.iter().map(|t| (t.title.as_str(), t.chapter, t.level)).collect();
        assert_eq!(toc, [("Front", 0, 0), ("Chapter 1", 1, 0), ("A section", 1, 1), ("Chapter 2", 2, 0)]);

        // The title page's links: one into the book, one to the web.
        let Block::Paragraph { spans, .. } = &book.chapters[0].blocks[0] else { panic!("not a paragraph") };
        let runs: Vec<(&str, Option<&str>)> = spans.iter().map(|s| (s.text.as_str(), s.link.as_deref())).collect();
        assert_eq!(
            runs,
            [
                ("Title page. ", None),
                ("Chapter 2", Some("OEBPS/a.xhtml#link-c2")),
                (" ", None),
                ("Gutenberg", Some("https://www.gutenberg.org")),
                (" ", None),
                ("Two", Some("OEBPS/a.xhtml#m2"))
            ]
        );
        // An empty marker before some spacing and the next chapter (how
        // Gutenberg marks its link targets) goes with that chapter.
        assert_eq!(book.chapter_for_link("OEBPS/a.xhtml#m2"), Some(2));
        assert_eq!(book.chapter_for_link("OEBPS/a.xhtml#link-c2"), Some(2));
        assert_eq!(book.chapter_for_link("OEBPS/a.xhtml#s1"), Some(1));
        // An unknown id in a known file: that file's start.
        assert_eq!(book.chapter_for_link("OEBPS/a.xhtml#nowhere"), Some(0));
    }

    #[test]
    fn entities_in_titles_and_contents() {
        let opf = parse_opf(r#"<package><metadata>
            <dc:title>Pride &amp; Prejudice&#x2009;&mdash;&nbsp;Vol. 1</dc:title>
            <dc:creator>Jane &amp; Co</dc:creator></metadata></package>"#);
        assert_eq!(opf.title, "Pride & Prejudice\u{2009}—\u{a0}Vol. 1");
        assert_eq!(opf.author.as_deref(), Some("Jane & Co"));

        let ncx = parse_ncx(
            r#"<ncx><navMap><navPoint><navLabel><text>Tom &amp; Jerry</text></navLabel>
            <content src="a.xhtml"/></navPoint></navMap></ncx>"#,
            Path::new(""),
        );
        assert_eq!(ncx[0].title, "Tom & Jerry");

        let nav = parse_nav(
            r#"<nav epub:type="toc"><ol><li><a href="b.xhtml">Salt &amp;
            Pepper</a></li></ol></nav>"#,
            Path::new(""),
        );
        assert_eq!(nav[0].title, "Salt & Pepper");
    }

    #[test]
    fn inline_runs_and_css_classes() {
        let mut css = CssHints::default();
        css.add_stylesheet("span.txit{ font-style: italic; } .noindent{text-indent:0em} .h2{text-align:center}");
        let html = r#"<html><head><title>x</title><style>p{}</style></head><body>
            <h2 class="h2"><a id="p"/>3</h2>
            <p class="noindent"><span class="stickup">T</span>he afternoon &amp; <span class="txit">Hai bude</span> atom&nbsp;robitnikom.</p>
            <p>Second   line<br/>broken.</p></body></html>"#;
        let blocks: Vec<Block> = parse_xhtml_body(html, "", &css, &Default::default())
            .into_iter()
            .filter_map(|p| match p { Parsed::Block(b) => Some(b), _ => None })
            .collect();
        assert!(matches!(&blocks[0], Block::Heading { level: 2, spans } if spans_text(spans) == "3"));
        match &blocks[1] {
            Block::Paragraph { spans, indent, .. } => {
                assert!(!indent);
                assert_eq!(spans_text(spans), "The afternoon & Hai bude atom\u{a0}robitnikom.");
                assert!(spans.iter().any(|s| s.style.italic && s.text.contains("Hai bude")));
            }
            b => panic!("{b:?}"),
        }
        match &blocks[2] {
            Block::Paragraph { spans, indent, .. } => {
                assert!(indent);
                assert_eq!(spans_text(spans), "Second line\nbroken.");
            }
            b => panic!("{b:?}"),
        }
    }

    /// NEBULA_EPUB_TEST=/path/book.epub prints what the parser sees.
    #[test]
    fn real_book() {
        let Ok(path) = std::env::var("NEBULA_EPUB_TEST") else { return };
        let book = open(Path::new(&path)).unwrap();
        println!("{} by {:?}: {} chapters, {} toc, {} images, {} words",
            book.title, book.author, book.chapters.len(), book.toc.len(), book.images.len(), book.total_words());
        for t in book.toc.iter().take(12) {
            println!("  toc {}{} -> ch {}", "  ".repeat(t.level), t.title, t.chapter);
        }
        // Where the book's own links go (its printed contents).
        let mut shown = 0;
        for ch in &book.chapters {
            for b in &ch.blocks {
                let (Block::Paragraph { spans, .. } | Block::Heading { spans, .. } | Block::Quote(spans) | Block::ListItem(spans)) = b else {
                    continue;
                };
                for sp in spans.iter().filter(|s| s.link.as_deref().is_some_and(|l| !l.contains("://"))) {
                    if shown < 12 {
                        let to = sp.link.as_deref().and_then(|l| book.chapter_for_link(l));
                        println!("  link {:?} -> {:?}", sp.text, to.map(|c| book.chapters[c].title.as_str()));
                        shown += 1;
                    }
                }
            }
        }
        // Links named like a chapter that open another one.
        let norm = |t: &str| t.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
        let (mut named, mut wrong) = (0, 0);
        for ch in &book.chapters {
            for b in &ch.blocks {
                let (Block::Paragraph { spans, .. } | Block::Heading { spans, .. } | Block::Quote(spans) | Block::ListItem(spans)) = b else {
                    continue;
                };
                for sp in spans {
                    let Some(to) = sp.link.as_deref().and_then(|l| book.chapter_for_link(l)) else { continue };
                    if book.chapters.iter().any(|c| norm(&c.title) == norm(&sp.text)) {
                        named += 1;
                        if norm(&book.chapters[to].title) != norm(&sp.text) {
                            wrong += 1;
                            println!("  WRONG {:?} -> {:?}", sp.text, book.chapters[to].title);
                        }
                    }
                }
            }
        }
        println!("  {named} links named like a chapter, {wrong} open another");
        if let Some(ch) = book.chapters.iter().find(|c| c.words > 2000) {
            println!("== {} ({} words)", ch.title, ch.words);
            for b in ch.blocks.iter().take(6) {
                println!("  {:?}", b);
            }
        }
    }
}

/// Pixel size of an encoded image, read from its header only.
fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()
}
