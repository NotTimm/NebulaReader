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
use quick_xml::Reader;

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

#[derive(Debug, Clone)]
pub struct EpubBook {
    pub title: String,
    pub author: Option<String>,
    /// Raw encoded cover image bytes, if the book declares one.
    pub cover: Option<Vec<u8>>,
    pub chapters: Vec<EpubChapter>,
    pub toc: Vec<TocEntry>,
    /// Encoded image bytes referenced by [`Block::Image`]; the app turns
    /// each into one image handle up front, not per frame.
    pub images: Vec<Vec<u8>>,
}

impl EpubBook {
    pub fn total_words(&self) -> usize {
        self.chapters.iter().map(|c| c.words).sum()
    }
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

    for idref in &opf.spine {
        let Some(item) = opf.manifest.get(idref) else { continue };
        let chapter_path = join_epub_path(&opf_dir, &item.href);
        let Ok(xhtml) = read_zip_text(&mut zip, &chapter_path) else { continue };
        let chapter_dir = Path::new(&chapter_path).parent().unwrap_or(Path::new("")).to_path_buf();

        let parsed = parse_xhtml_body(&xhtml, &css);
        let mut blocks = Vec::with_capacity(parsed.len());
        for block in parsed {
            match block {
                Parsed::Block(b) => blocks.push(b),
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
                        blocks.push(Block::Image(i));
                    }
                }
            }
        }
        if blocks.is_empty() {
            continue;
        }

        let words = blocks
            .iter()
            .map(|b| match b {
                Block::Heading { spans, .. }
                | Block::Paragraph { spans, .. }
                | Block::Quote(spans)
                | Block::ListItem(spans) => spans.iter().map(|s| s.text.split_whitespace().count()).sum(),
                _ => 0,
            })
            .sum();

        let title = toc_raw
            .iter()
            .find(|(p, _, _)| *p == chapter_path)
            .map(|(_, t, _)| t.clone())
            .or_else(|| {
                blocks.iter().find_map(|b| match b {
                    Block::Heading { spans, .. } => Some(spans_text(spans)),
                    _ => None,
                })
            })
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_default();

        chapter_for_path.insert(chapter_path, chapters.len());
        chapters.push(EpubChapter { title, blocks, words });
    }

    if chapters.is_empty() {
        return Err(format!("no readable chapters found in {}", path.display()));
    }

    // Untitled chapters get a positional name.
    for (i, ch) in chapters.iter_mut().enumerate() {
        if ch.title.is_empty() {
            ch.title = format!("Section {}", i + 1);
        }
    }

    let mut toc: Vec<TocEntry> = toc_raw
        .into_iter()
        .filter_map(|(p, title, level)| {
            chapter_for_path.get(&p).map(|&chapter| TocEntry { title, chapter, level })
        })
        .collect();
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
                    if local_name_bytes(attr.key.as_ref()) == "full-path" {
                        return Ok(attr.unescape_value().unwrap_or_default().into_owned());
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

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| local_name_bytes(a.key.as_ref()) == name)
        .map(|a| a.unescape_value().map(|v| v.into_owned()).unwrap_or_else(|_| {
            String::from_utf8_lossy(&a.value).into_owned()
        }))
}

fn parse_opf(xml: &str) -> Opf {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
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

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(&e.name()).as_str() {
                "title" => in_title = opf.title.is_empty(),
                "creator" => in_creator = opf.author.is_none(),
                "date" => in_date = opf.date.is_none(),
                "spine" => opf.ncx_id = attr(&e, "toc"),
                _ => {}
            },
            Ok(Event::Text(t)) => {
                let text = decode_text(&t);
                let text = text.trim();
                if in_title {
                    opf.title = text.to_string();
                } else if in_creator {
                    opf.author = Some(text.to_string());
                } else if in_date && !text.is_empty() {
                    // Keep just the date part of "2019-02-12T00:00:00Z".
                    opf.date = Some(text.split('T').next().unwrap_or(text).to_string());
                }
            }
            Ok(Event::End(e)) => match local_name(&e.name()).as_str() {
                "title" => in_title = false,
                "creator" => in_creator = false,
                "date" => in_date = false,
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
fn read_toc(
    zip: &mut zip::ZipArchive<std::fs::File>,
    opf: &Opf,
    opf_dir: &Path,
) -> Vec<(String, String, usize)> {
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

fn parse_ncx(xml: &str, dir: &Path) -> Vec<(String, String, usize)> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
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
            Ok(Event::Text(t)) if in_text => label.push_str(decode_text(&t).trim()),
            Ok(Event::End(e)) => match local_name(&e.name()).as_str() {
                "navPoint" => depth = depth.saturating_sub(1),
                "text" => in_text = false,
                _ => {}
            },
            Ok(Event::Empty(e)) if local_name(&e.name()) == "content" => {
                if let Some(src) = attr(&e, "src") {
                    if !label.is_empty() {
                        out.push((join_epub_path(dir, &src), label.clone(), depth.saturating_sub(1)));
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

fn parse_nav(xml: &str, dir: &Path) -> Vec<(String, String, usize)> {
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
                    current = attr(&e, "href").map(|h| (join_epub_path(dir, &h), String::new()));
                }
                _ => {}
            },
            Ok(Event::Text(t)) => {
                if let Some((_, label)) = current.as_mut() {
                    label.push_str(&decode_text(&t));
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
                    if let Some((path, label)) = current.take() {
                        let label = collapse(&label);
                        if !label.is_empty() {
                            out.push((path, label, ol_depth.saturating_sub(1)));
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
            let mut text = String::new();
            if self.pending_space && has_content {
                text.push(' ');
            }
            self.pending_space = false;
            text.push_str(piece);
            match self.spans.last_mut() {
                Some(last) if last.style == style => last.text.push_str(&text),
                _ => self.spans.push(Inline { text, style }),
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
            self.stack.push(Frame { name, hints, block });
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

fn parse_xhtml_body(xhtml: &str, css: &CssHints) -> Vec<Parsed> {
    let mut reader = Reader::from_str(xhtml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;
    let mut buf = Vec::new();

    let mut p = BodyParser { css, out: Vec::new(), stack: Vec::new(), spans: Vec::new(), pending_space: false };
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
            Ok(Event::Text(t)) => {
                if skip_depth == 0 && in_body {
                    p.push_text(&decode_text(&t));
                }
            }
            Ok(Event::CData(t)) => {
                if skip_depth == 0 && in_body {
                    p.push_text(&String::from_utf8_lossy(&t));
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

/// Decodes a text node, resolving XML *and* common HTML entities
/// (`&nbsp;`, `&mdash;`… are everywhere in EPUBs but aren't XML).
fn decode_text(t: &quick_xml::events::BytesText) -> String {
    let raw = String::from_utf8_lossy(t.as_ref());
    if !raw.contains('&') {
        return raw.into_owned();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest: &str = &raw;
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

fn local_name(name: &quick_xml::name::QName) -> String {
    local_name_bytes(name.as_ref())
}

fn local_name_bytes(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    s.rsplit(':').next().unwrap_or(&s).to_string()
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
    use super::*;

    #[test]
    fn inline_runs_and_css_classes() {
        let mut css = CssHints::default();
        css.add_stylesheet("span.txit{ font-style: italic; } .noindent{text-indent:0em} .h2{text-align:center}");
        let html = r#"<html><head><title>x</title><style>p{}</style></head><body>
            <h2 class="h2"><a id="p"/>3</h2>
            <p class="noindent"><span class="stickup">T</span>he afternoon &amp; <span class="txit">Hai bude</span> atom&nbsp;robitnikom.</p>
            <p>Second   line<br/>broken.</p></body></html>"#;
        let blocks: Vec<Block> = parse_xhtml_body(html, &css)
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
