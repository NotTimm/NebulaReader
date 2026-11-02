//! Wikipedia, read in the app (opt-in, Settings → Wikipedia): the main
//! page's daily feed, search, random articles and articles themselves,
//! from Wikimedia's public REST API.
//!
//! Articles come as Parsoid HTML and are turned into a small model the
//! app draws natively, laid out like Wikipedia's own pages: sections with
//! their headings, paragraphs with links, hatnotes ("For other uses,
//! see…"), figures with captions, tables, and the infobox with its header
//! colours. Navigation boxes, edit links, maintenance notices and anything
//! the site hides with its stylesheet are left out.
//!
//! Rules this follows (Wikimedia's API etiquette and the text's licence):
//! every request says who's asking (`USER_AGENT`), nothing is crawled or
//! fetched ahead, and every article is shown with its credit and licence
//! (CC BY-SA 4.0) and a link to it on Wikipedia. Wikipedia's name and logo
//! are trademarks: the app uses the name to say where articles come from,
//! never the logo.

use std::sync::OnceLock;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::epub::{event_text, local_name};

/// English for now; every language's Wikipedia has the same API.
const HOST: &str = "https://en.wikipedia.org";
const USER_AGENT: &str = concat!("NebulaReader/", env!("CARGO_PKG_VERSION"), " (https://github.com/NotTimm/NebulaReader)");

pub const LICENSE_URL: &str = "https://creativecommons.org/licenses/by-sa/4.0/";

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("http client")
    })
}

async fn get(url: &str) -> Result<reqwest::Response, String> {
    let resp = client().get(url).send().await.map_err(|e| fl!("jf-unreachable", error = e.to_string()))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    Ok(resp)
}

/// An article's address on Wikipedia.
pub fn page_url(title: &str) -> String {
    format!("{HOST}/wiki/{}", title.replace(' ', "_"))
}

/// A title as it goes in an API path: spaces as underscores, the rest
/// escaped.
fn path_title(title: &str) -> String {
    let title = title.replace(' ', "_");
    url::form_urlencoded::byte_serialize(title.as_bytes()).collect::<String>().replace('+', "%20")
}

// ── The main page ──────────────────────────────────────────────────────────

/// A page in a feed: a title, a line about it and maybe a picture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub title: String,
    pub description: String,
    pub extract: String,
    pub thumbnail: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OnThisDay {
    pub year: i64,
    pub text: String,
    pub pages: Vec<Summary>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Picture {
    pub title: String,
    pub image: String,
    pub description: String,
}

/// What the main page shows for a day.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Featured {
    pub article: Option<Summary>,
    /// "In the news": each story's text, with its links.
    pub news: Vec<Vec<Run>>,
    pub on_this_day: Vec<OnThisDay>,
    pub most_read: Vec<Summary>,
    pub picture: Option<Picture>,
}

fn summary(v: &serde_json::Value) -> Option<Summary> {
    let title = v["titles"]["normalized"].as_str().or_else(|| v["title"].as_str())?.to_string();
    Some(Summary {
        title,
        description: v["description"].as_str().unwrap_or_default().to_string(),
        extract: v["extract"].as_str().unwrap_or_default().to_string(),
        thumbnail: v["thumbnail"]["source"].as_str().map(str::to_string),
    })
}

fn parse_featured(v: &serde_json::Value) -> Featured {
    let list = |v: &serde_json::Value| v.as_array().cloned().unwrap_or_default();
    Featured {
        article: summary(&v["tfa"]),
        news: list(&v["news"])
            .iter()
            .filter_map(|n| n["story"].as_str())
            .map(|html| {
                let root = parse_html(&format!("<div>{html}</div>"));
                let mut b = Builder::default();
                b.inlines(&root, Ctx::default());
                trim_line(std::mem::take(&mut b.line))
            })
            .collect(),
        on_this_day: list(&v["onthisday"])
            .iter()
            .map(|d| OnThisDay {
                year: d["year"].as_i64().unwrap_or_default(),
                text: d["text"].as_str().unwrap_or_default().to_string(),
                pages: list(&d["pages"]).iter().filter_map(summary).collect(),
            })
            .collect(),
        most_read: list(&v["mostread"]["articles"]).iter().filter_map(summary).take(10).collect(),
        picture: v["image"]["image"]["source"].as_str().or_else(|| v["image"]["thumbnail"]["source"].as_str()).map(|image| Picture {
            title: v["image"]["title"].as_str().unwrap_or_default().trim_start_matches("File:").to_string(),
            image: image.to_string(),
            description: v["image"]["description"]["text"].as_str().unwrap_or_default().to_string(),
        }),
    }
}

/// Today's main page (in UTC, like Wikipedia's).
pub async fn featured() -> Result<Featured, String> {
    let today = chrono_today();
    let json: serde_json::Value =
        get(&format!("{HOST}/api/rest_v1/feed/featured/{today}")).await?.json().await.map_err(|e| e.to_string())?;
    Ok(parse_featured(&json))
}

/// `YYYY/MM/DD` for today in UTC.
fn chrono_today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}/{m:02}/{d:02}")
}

/// A random article's title.
pub async fn random_title() -> Result<String, String> {
    let json: serde_json::Value =
        get(&format!("{HOST}/api/rest_v1/page/random/summary")).await?.json().await.map_err(|e| e.to_string())?;
    summary(&json).map(|s| s.title).ok_or_else(|| "no article".to_string())
}

/// Articles matching `query`.
pub async fn search(query: &str) -> Result<Vec<Summary>, String> {
    let q: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
    let json: serde_json::Value =
        get(&format!("{HOST}/w/rest.php/v1/search/page?q={q}&limit=20")).await?.json().await.map_err(|e| e.to_string())?;
    Ok(json["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| Summary {
            title: p["title"].as_str().unwrap_or_default().to_string(),
            description: p["description"].as_str().unwrap_or_default().to_string(),
            // Matches come wrapped in <span class="searchmatch">.
            extract: strip_tags(p["excerpt"].as_str().unwrap_or_default()),
            thumbnail: p["thumbnail"]["url"].as_str().map(fix_url),
        })
        .filter(|s| !s.title.is_empty())
        .collect())
}

fn strip_tags(html: &str) -> String {
    let root = parse_html(&format!("<div>{html}</div>"));
    root.text().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// An image (a thumbnail from Wikimedia's servers).
pub async fn image(url: &str) -> Result<Vec<u8>, String> {
    get(url).await?.bytes().await.map(|b| b.to_vec()).map_err(|e| e.to_string())
}

/// An article, by title.
pub async fn article(title: &str) -> Result<Article, String> {
    let html = get(&format!("{HOST}/api/rest_v1/page/html/{}", path_title(title))).await?.text().await.map_err(|e| e.to_string())?;
    Ok(parse_article(&html, title))
}

// ── Articles ───────────────────────────────────────────────────────────────

/// A run of text with one style.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    /// A footnote mark ("[14]"), drawn small.
    pub reference: bool,
    /// `wiki:<Title>` for another article, or a web address.
    pub link: Option<String>,
}

pub type Line = Vec<Run>;

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub url: String,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Para(Line),
    /// "For other uses, see…": italic and indented.
    Hatnote(Line),
    Quote(Line),
    /// Items with their nesting depth (0 = top).
    List { ordered: bool, items: Vec<(usize, Line)> },
    Figure { image: Image, caption: Line },
    Table(Table),
    /// A bar of a bar chart (Wikipedia draws them with CSS), in em.
    Bar(f32),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub rows: Vec<Row>,
    pub caption: Line,
    /// Floated right on the site (a chart or a small box beside the text).
    pub float_right: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub cells: Vec<Cell>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    pub header: bool,
    pub colspan: usize,
    /// Its background colour, when the page gives one (infobox headers).
    pub background: Option<[u8; 3]>,
    pub center: bool,
    pub content: Vec<Block>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    /// 1 for the lead (no heading), 2 for `==`, 3 for `===`…
    pub level: u8,
    pub title: String,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Article {
    pub title: String,
    /// The box at the top right ("Battle of Gettysburg / Part of… / Date…").
    pub infobox: Option<Table>,
    /// The lead first.
    pub sections: Vec<Section>,
}

impl Article {
    /// The article's pictures with their captions, in reading order (the
    /// infobox's first): what the picture viewer steps through.
    pub fn gallery(&self) -> Vec<(Image, Line)> {
        fn walk(blocks: &[Block], out: &mut Vec<(Image, Line)>) {
            for b in blocks {
                match b {
                    Block::Figure { image, caption } => out.push((image.clone(), caption.clone())),
                    Block::Table(t) => table(t, out),
                    _ => {}
                }
            }
        }
        fn table(t: &Table, out: &mut Vec<(Image, Line)>) {
            for c in t.rows.iter().flat_map(|r| &r.cells) {
                walk(&c.content, out);
            }
        }
        let mut out = Vec::new();
        if let Some(t) = &self.infobox {
            table(t, &mut out);
        }
        for s in &self.sections {
            walk(&s.blocks, &mut out);
        }
        out
    }
}

/// The full-size file behind a thumbnail
/// (`…/thumb/0/0f/Name.jpg/330px-Name.jpg` → `…/0/0f/Name.jpg`), for the
/// picture viewer. Drawings (SVG) and other files the app can't decode
/// keep their thumbnail, asked for wider instead.
pub fn full_size(thumb: &str) -> String {
    let url = thumb.split('?').next().unwrap_or(thumb).replace("//thumb.wikimedia.org/", "//upload.wikimedia.org/");
    let Some((head, rest)) = url.split_once("/thumb/") else { return url };
    let Some((file, _size)) = rest.rsplit_once('/') else { return url };
    let lower = file.to_ascii_lowercase();
    if [".jpg", ".jpeg", ".png", ".gif", ".webp"].iter().any(|e| lower.ends_with(e)) {
        format!("{head}/{file}")
    } else {
        url
    }
}

// A small tree of the page, so tables and nesting are easy to walk.

#[derive(Clone, Debug, Default)]
struct El {
    name: String,
    attrs: Vec<(String, String)>,
    kids: Vec<Node>,
}

#[derive(Clone, Debug)]
enum Node {
    El(El),
    Text(String),
}

impl El {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn has_class(&self, class: &str) -> bool {
        self.attr("class").is_some_and(|c| c.split_whitespace().any(|x| x == class))
    }

    fn text(&self) -> String {
        let mut out = String::new();
        for k in &self.kids {
            match k {
                Node::Text(t) => out.push_str(t),
                Node::El(e) if !hidden(e) => out.push_str(&e.text()),
                _ => {}
            }
        }
        out
    }

    fn find<'a>(&'a self, pred: &dyn Fn(&El) -> bool) -> Option<&'a El> {
        for k in &self.kids {
            if let Node::El(e) = k {
                if pred(e) {
                    return Some(e);
                }
                if let Some(f) = e.find(pred) {
                    return Some(f);
                }
            }
        }
        None
    }
}

fn parse_html(html: &str) -> El {
    let mut reader = Reader::from_str(html);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;
    let mut buf = Vec::new();
    let mut stack: Vec<El> = vec![El { name: "#root".into(), ..Default::default() }];
    let attrs = |e: &quick_xml::events::BytesStart| -> Vec<(String, String)> {
        e.attributes()
            .flatten()
            .map(|a| {
                let key = a.key.local_name().as_ref().to_string();
                let value = a
                    .normalized_value(quick_xml::XmlVersion::default())
                    .map(|v| v.into_owned())
                    .unwrap_or_else(|_| a.value.to_string());
                (key, value)
            })
            .collect()
    };
    loop {
        let ev = reader.read_event_into(&mut buf);
        match ev {
            Ok(Event::Start(ref e)) => {
                let name = local_name(&e.name()).to_ascii_lowercase();
                let el = El { name: name.clone(), attrs: attrs(e), kids: Vec::new() };
                // HTML's empty elements, should one come unclosed.
                if matches!(name.as_str(), "br" | "img" | "hr" | "meta" | "link" | "wbr" | "input" | "source") {
                    stack.last_mut().unwrap().kids.push(Node::El(el));
                } else {
                    stack.push(el);
                }
            }
            Ok(Event::Empty(ref e)) => {
                let el = El { name: local_name(&e.name()).to_ascii_lowercase(), attrs: attrs(e), kids: Vec::new() };
                stack.last_mut().unwrap().kids.push(Node::El(el));
            }
            Ok(ref ev @ (Event::Text(_) | Event::GeneralRef(_))) => {
                let t = event_text(ev);
                stack.last_mut().unwrap().kids.push(Node::Text(t));
            }
            Ok(Event::CData(t)) => {
                stack.last_mut().unwrap().kids.push(Node::Text(String::from(&*t)));
            }
            Ok(Event::End(e)) => {
                let name = local_name(&e.name()).to_ascii_lowercase();
                if let Some(pos) = stack.iter().rposition(|el| el.name == name).filter(|&p| p > 0) {
                    while stack.len() > pos {
                        let done = stack.pop().unwrap();
                        stack.last_mut().unwrap().kids.push(Node::El(done));
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    while stack.len() > 1 {
        let done = stack.pop().unwrap();
        stack.last_mut().unwrap().kids.push(Node::El(done));
    }
    stack.pop().unwrap()
}

/// What Wikipedia's stylesheet hides, or what doesn't belong in a reader:
/// edit links, navigation boxes, maintenance notices, the second form of
/// coordinates, and the like.
fn hidden(e: &El) -> bool {
    if matches!(e.name.as_str(), "style" | "script" | "link" | "meta" | "noscript" | "head" | "title") {
        return true;
    }
    if e.attr("style").is_some_and(|s| s.replace(' ', "").contains("display:none")) {
        return true;
    }
    if e.attr("role") == Some("navigation") {
        return true;
    }
    const CLASSES: &[&str] = &[
        "mw-editsection",
        "navbox",
        "navbox-styles",
        "vertical-navbox",
        "sidebar",
        "noprint",
        "metadata",
        "ambox",
        "mbox-small",
        "sistersitebox",
        "side-box",
        "portalbox",
        "shortdescription",
        "mw-empty-elt",
        "geo-nondefault",
        "geo-multi-punct",
        "mw-cite-backlink",
        "printfooter",
        "catlinks",
        "mw-authority-control",
    ];
    CLASSES.iter().any(|c| e.has_class(c))
}

/// Styles and link around a run being built.
#[derive(Clone, Default)]
struct Ctx {
    bold: bool,
    italic: bool,
    link: Option<String>,
}

#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    line: Line,
    /// The article's infobox, taken from the lead when it's met there.
    infobox: Option<Table>,
    take_infobox: bool,
}

/// Where a link goes: `wiki:<Title>` for an article, a web address, or
/// nowhere (files, this page's own footnotes).
fn link_target(a: &El) -> Option<String> {
    let href = a.attr("href")?;
    let rel = a.attr("rel").unwrap_or_default();
    if rel.contains("mw:WikiLink") || href.starts_with("./") {
        let page = href.trim_start_matches("./");
        let (title, _fragment) = page.split_once('#').unwrap_or((page, ""));
        if title.is_empty() || title.contains(':') && !title.starts_with("Special:") && is_namespace(title) {
            return None;
        }
        let title = url::form_urlencoded::parse(format!("t={title}").as_bytes()).next().map(|(_, v)| v.into_owned())?;
        return Some(format!("wiki:{}", title.replace('_', " ")));
    }
    if href.starts_with("//") {
        return Some(format!("https:{href}"));
    }
    (href.starts_with("http://") || href.starts_with("https://")).then(|| href.to_string())
}

/// `File:`, `Category:`, `Help:` and the like aren't articles.
fn is_namespace(title: &str) -> bool {
    const NS: &[&str] = &["File", "Category", "Help", "Wikipedia", "Template", "Portal", "Talk", "Special", "Module", "Draft", "User", "Image", "Media"];
    title.split_once(':').is_some_and(|(ns, _)| NS.iter().any(|n| n.eq_ignore_ascii_case(ns.trim_end_matches("_talk"))))
}

fn fix_url(src: &str) -> String {
    if src.starts_with("//") { format!("https:{src}") } else { src.to_string() }
}

/// An image element as a picture, unless it's an icon (flags, symbols).
fn image_of(img: &El) -> Option<Image> {
    let url = fix_url(img.attr("src")?);
    let width: f32 = img.attr("width").and_then(|w| w.parse().ok()).unwrap_or(220.0);
    let height: f32 = img.attr("height").and_then(|h| h.parse().ok()).unwrap_or(width);
    (width > 40.0 && height > 40.0).then_some(Image { url, width, height })
}

/// `#C3D6EF` (or `#ccc`) in a style attribute's background.
fn background(style: &str) -> Option<[u8; 3]> {
    let s = style.to_ascii_lowercase();
    let i = s.find("background-color:").map(|i| i + 17).or_else(|| s.find("background:").map(|i| i + 11))?;
    let v = s[i..].trim_start();
    let hex: String = v.strip_prefix('#')?.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    let n = |a: &str| u8::from_str_radix(a, 16).ok();
    match hex.len() {
        6 => Some([n(&hex[0..2])?, n(&hex[2..4])?, n(&hex[4..6])?]),
        3 => {
            let d = |c: char| n(&format!("{c}{c}"));
            let c: Vec<char> = hex.chars().collect();
            Some([d(c[0])?, d(c[1])?, d(c[2])?])
        }
        _ => None,
    }
}

fn trim_line(mut line: Line) -> Line {
    if let Some(first) = line.first_mut() {
        first.text = first.text.trim_start().to_string();
    }
    if let Some(last) = line.last_mut() {
        last.text = last.text.trim_end().to_string();
    }
    line.retain(|r| !r.text.is_empty());
    line
}

impl Builder {
    fn push_text(&mut self, text: &str, ctx: &Ctx, reference: bool) {
        // Runs of spaces and newlines are one space, as in a browser.
        let mut t = String::with_capacity(text.len());
        let mut space = false;
        for c in text.chars() {
            if c.is_whitespace() && c != '\u{a0}' {
                space = true;
            } else {
                if space {
                    t.push(' ');
                }
                space = false;
                t.push(c);
            }
        }
        if space {
            t.push(' ');
        }
        if t.is_empty() {
            return;
        }
        let ends_in_space = self.line.last().is_none_or(|r| r.text.ends_with(' ') || r.text.ends_with('\n'));
        if ends_in_space {
            t = t.trim_start().to_string();
            if t.is_empty() {
                return;
            }
        }
        match self.line.last_mut() {
            Some(last) if last.bold == ctx.bold && last.italic == ctx.italic && last.link == ctx.link && last.reference == reference => {
                last.text.push_str(&t)
            }
            _ => self.line.push(Run { text: t, bold: ctx.bold, italic: ctx.italic, reference, link: ctx.link.clone() }),
        }
    }

    fn flush(&mut self) {
        let line = trim_line(std::mem::take(&mut self.line));
        if !line.is_empty() {
            self.blocks.push(Block::Para(line));
        }
    }

    /// An element's inline content, added to the line.
    fn inlines(&mut self, el: &El, ctx: Ctx) {
        for k in &el.kids {
            match k {
                Node::Text(t) => self.push_text(t, &ctx, false),
                Node::El(e) => self.inline(e, &ctx),
            }
        }
    }

    fn inline(&mut self, e: &El, ctx: &Ctx) {
        if hidden(e) {
            return;
        }
        let mut c = ctx.clone();
        match e.name.as_str() {
            "br" => {
                if let Some(last) = self.line.last_mut() {
                    last.text.push('\n');
                }
                return;
            }
            "b" | "strong" | "th" => c.bold = true,
            "i" | "em" | "cite" | "var" => c.italic = true,
            "a" => c.link = link_target(e).or(c.link),
            "sup" if e.has_class("reference") || e.has_class("mw-ref") => {
                let mark = e.text();
                self.push_text(mark.trim(), ctx, true);
                return;
            }
            "img" => return,
            // Formulas: their text form.
            "math" => {
                let alt = e.attr("alttext").unwrap_or_default();
                let alt = alt.trim().trim_start_matches("{\\displaystyle").trim_end_matches('}').trim();
                self.push_text(alt, &Ctx { italic: true, ..ctx.clone() }, false);
                return;
            }
            _ if e.has_class("mwe-math-element") => {
                if let Some(m) = e.find(&|x| x.name == "math") {
                    self.inline(m, ctx);
                }
                return;
            }
            _ => {}
        }
        self.inlines(e, c);
    }

    /// An element's content as blocks.
    fn blocks(&mut self, el: &El, ctx: &Ctx) {
        for k in &el.kids {
            match k {
                Node::Text(t) => self.push_text(t, ctx, false),
                Node::El(e) => self.block(e, ctx),
            }
        }
    }

    fn block(&mut self, e: &El, ctx: &Ctx) {
        if hidden(e) {
            return;
        }
        let name = e.name.as_str();
        // A picture (a thumbnail with its caption, or one on its own).
        if name == "figure" || (e.attr("typeof").is_some_and(|t| t.starts_with("mw:File")) && name != "a") {
            if let Some(image) = e.find(&|x| x.name == "img").and_then(image_of) {
                self.flush();
                let mut cap = Builder::default();
                if let Some(fc) = e.find(&|x| x.name == "figcaption") {
                    cap.inlines(fc, Ctx::default());
                }
                self.blocks.push(Block::Figure { image, caption: trim_line(cap.line) });
            }
            return;
        }
        match name {
            "p" | "div" | "section" | "center" | "main" | "body" | "html" | "article" | "span" | "dd" => {
                if e.has_class("bar-chart-bar-line") {
                    let em = e
                        .attr("style")
                        .and_then(|s| s.replace(' ', "").split("width:").nth(1).map(str::to_string))
                        .and_then(|w| w.trim_end_matches(';').trim_end_matches("em").parse().ok())
                        .unwrap_or(0.0);
                    self.flush();
                    self.blocks.push(Block::Bar(em));
                    return;
                }
                if e.attr("role") == Some("note") || e.has_class("hatnote") {
                    self.flush();
                    let mut b = Builder::default();
                    b.inlines(e, Ctx { italic: true, ..Ctx::default() });
                    let line = trim_line(b.line);
                    if !line.is_empty() {
                        self.blocks.push(Block::Hatnote(line));
                    }
                    return;
                }
                if name == "span" {
                    return self.inline(e, ctx);
                }
                self.flush();
                self.blocks(e, ctx);
                self.flush();
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "dt" | "caption" => {
                self.flush();
                self.inlines(e, Ctx { bold: true, ..ctx.clone() });
                self.flush();
            }
            "ul" | "ol" => {
                self.flush();
                let mut items = Vec::new();
                list_items(e, 0, &mut items);
                if !items.is_empty() {
                    self.blocks.push(Block::List { ordered: name == "ol", items });
                }
            }
            "dl" => {
                self.flush();
                self.blocks(e, ctx);
                self.flush();
            }
            "blockquote" => {
                self.flush();
                let mut b = Builder::default();
                b.inlines(e, Ctx::default());
                let line = trim_line(b.line);
                if !line.is_empty() {
                    self.blocks.push(Block::Quote(line));
                }
            }
            "table" => {
                self.flush();
                let t = table(e);
                if t.rows.is_empty() {
                    return;
                }
                if self.take_infobox && self.infobox.is_none() && e.has_class("infobox") {
                    self.infobox = Some(t);
                } else {
                    self.blocks.push(Block::Table(t));
                }
            }
            "hr" => self.flush(),
            _ => self.inline(e, ctx),
        }
    }
}

/// A list's items, with nested lists one level deeper.
fn list_items(list: &El, depth: usize, out: &mut Vec<(usize, Line)>) {
    for k in &list.kids {
        let Node::El(li) = k else { continue };
        if hidden(li) || li.name != "li" {
            continue;
        }
        let mut b = Builder::default();
        let mut nested = Vec::new();
        for k in &li.kids {
            match k {
                Node::Text(t) => b.push_text(t, &Ctx::default(), false),
                Node::El(e) if matches!(e.name.as_str(), "ul" | "ol") => nested.push(e),
                Node::El(e) => b.inline(e, &Ctx::default()),
            }
        }
        let line = trim_line(b.line);
        if !line.is_empty() {
            out.push((depth, line));
        }
        for n in nested {
            list_items(n, depth + 1, out);
        }
    }
}

/// A table: its rows (through `tbody`/`thead`) and cells.
fn table(t: &El) -> Table {
    let mut out = Table::default();
    fn rows<'a>(el: &'a El, out: &mut Vec<&'a El>) {
        for k in &el.kids {
            if let Node::El(e) = k {
                match e.name.as_str() {
                    "tr" => out.push(e),
                    "tbody" | "thead" | "tfoot" => rows(e, out),
                    _ => {}
                }
            }
        }
    }
    let mut trs = Vec::new();
    rows(t, &mut trs);
    let class = t.attr("class").unwrap_or_default();
    out.float_right = ["floatright", "tright", "float-right", "bar-chart-right"].iter().any(|c| class.split_whitespace().any(|x| x == *c))
        || t.attr("style").is_some_and(|s| s.replace(' ', "").contains("float:right"));
    if let Some(c) = t.kids.iter().find_map(|k| match k {
        Node::El(e) if e.name == "caption" => Some(e),
        _ => None,
    }) {
        let mut b = Builder::default();
        b.inlines(c, Ctx { bold: true, ..Ctx::default() });
        out.caption = trim_line(b.line);
    }
    for tr in trs {
        if hidden(tr) {
            continue;
        }
        let mut row = Row::default();
        for k in &tr.kids {
            let Node::El(td) = k else { continue };
            if !matches!(td.name.as_str(), "td" | "th") || hidden(td) {
                continue;
            }
            let header = td.name == "th";
            let style = td.attr("style").unwrap_or_default();
            let mut b = Builder::default();
            b.blocks(td, &Ctx { bold: header, ..Ctx::default() });
            b.flush();
            row.cells.push(Cell {
                header,
                colspan: td.attr("colspan").and_then(|c| c.trim().parse().ok()).unwrap_or(1).max(1),
                background: background(style),
                center: style.replace(' ', "").contains("text-align:center"),
                content: b.blocks,
            });
        }
        if row.cells.iter().any(|c| !c.content.is_empty()) {
            out.rows.push(row);
        }
    }
    out
}

/// An article's HTML (Parsoid's) as sections, with the infobox apart.
pub fn parse_article(html: &str, title: &str) -> Article {
    let root = parse_html(html);
    let body = root.find(&|e| e.name == "body").cloned().unwrap_or(root);
    let mut article = Article { title: title.replace('_', " "), ..Default::default() };
    let mut current = Section { level: 1, ..Default::default() };
    let mut b = Builder { take_infobox: true, ..Default::default() };

    fn walk(el: &El, article: &mut Article, current: &mut Section, b: &mut Builder) {
        for k in &el.kids {
            match k {
                Node::El(e) if e.name == "section" => walk(e, article, current, b),
                Node::El(e) if matches!(e.name.as_str(), "h2" | "h3" | "h4" | "h5" | "h6") => {
                    // A new section: the one before it is done.
                    b.flush();
                    current.blocks.append(&mut b.blocks);
                    let done = std::mem::take(current);
                    if done.level == 1 || !done.blocks.is_empty() || !done.title.is_empty() {
                        article.sections.push(done);
                    }
                    b.take_infobox = false;
                    let level = e.name[1..].parse().unwrap_or(2);
                    let title = e.text().split_whitespace().collect::<Vec<_>>().join(" ");
                    *current = Section { level, title, blocks: Vec::new() };
                }
                Node::El(e) if e.name == "div" && e.has_class("mw-heading") => walk(e, article, current, b),
                Node::El(e) => b.block(e, &Ctx::default()),
                Node::Text(t) => b.push_text(t, &Ctx::default(), false),
            }
        }
    }
    walk(&body, &mut article, &mut current, &mut b);
    b.flush();
    current.blocks.append(&mut b.blocks);
    article.sections.push(current);
    article.infobox = b.infobox.take();
    article
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from the Battle of Gettysburg as the REST API serves it.
    const GETTYSBURG: &str = r##"<!DOCTYPE html>
<html><head><title>Battle of Gettysburg</title><style>.x{}</style></head>
<body><section data-mw-section-id="0" id="mwAQ">
<div role="note" class="hatnote navigation-not-searchable">For other uses, see <a rel="mw:WikiLink" href="./Battle_of_Gettysburg_(disambiguation)" title="Battle of Gettysburg (disambiguation)">Battle of Gettysburg (disambiguation)</a>.</div>
<div class="stack"><div><table class="infobox vevent"><tbody>
<tr><th class="summary" colspan="2" style="background-color:#C3D6EF;color:inherit;text-align:center;">Battle of Gettysburg</th></tr>
<tr><td colspan="2" style="background-color:#DCDCDC;text-align:center;">Part of the <a rel="mw:WikiLink" href="./Eastern_theater_of_the_American_Civil_War">Eastern theater of the American Civil War</a></td></tr>
<tr><td colspan="2" style="text-align:center;"><span typeof="mw:File"><a href="./File:Battle.jpg" class="mw-file-description"><img src="//thumb.wikimedia.org/x/330px-Battle.jpg" height="210" width="300" class="mw-file-element"/></a></span><br/>The <i>Battle of Gettysburg</i> by <a rel="mw:WikiLink" href="./Thure_de_Thulstrup">Thure de Thulstrup</a></td></tr>
<tr><td colspan="2"><table style="width:100%"><tbody><tr><th style="padding-right:1em">Date</th><td>July 1–3, 1863</td></tr>
<tr><th>Location</th><td><div class="location"><a rel="mw:WikiLink" href="./Gettysburg,_Pennsylvania">Gettysburg, Pennsylvania</a></div><span class="geo-inline"><style>.y{}</style><span class="geo-default"><span class="latitude">39°48′41″N</span> <span class="longitude">77°13′33″W</span></span><span class="geo-multi-punct"> / </span><span class="geo-nondefault">39.81139; -77.22583</span></span></td></tr></tbody></table></td></tr>
<tr><th colspan="2" style="background-color:#C3D6EF;text-align:center;">Belligerents</th></tr>
<tr><td style="width:50%"> <span typeof="mw:File"><img src="//up/23px-Flag.png" width="23" height="12"/></span> <a rel="mw:WikiLink" href="./Union_(American_Civil_War)">United States</a></td><td style="width:50%"> <a rel="mw:WikiLink" href="./Confederate_States_of_America">Confederate States</a></td></tr>
</tbody></table></div></div>
<p>The <b>Battle of Gettysburg</b> (July 1–3, 1863) was fought in the <a rel="mw:WikiLink" href="./American_Civil_War" title="American Civil War">American Civil War</a>.<sup about="#mwt1" class="mw-ref reference" typeof="mw:Extension/ref"><a href="./Battle_of_Gettysburg#cite_note-14"><span class="mw-reflink-text">[14]</span></a></sup> See <a rel="mw:ExtLink" href="https://www.nps.gov/gett" class="external text">the park</a>.</p>
<div role="navigation" class="navbox"><p>Lots of links</p></div>
</section>
<section data-mw-section-id="1"><h2 id="Background">Background<span class="mw-editsection">edit</span></h2>
<figure class="mw-default-size" typeof="mw:File/Thumb"><a href="./File:Map.png"><img src="//thumb.wikimedia.org/x/250px-Map.png" height="300" width="250"/></a><figcaption>An 1863 map of the <a rel="mw:WikiLink" href="./Gettysburg_Battlefield">Gettysburg Battlefield</a></figcaption></figure>
<p>Shortly after the Army of Northern Virginia won a victory.</p>
<section data-mw-section-id="2"><h3 id="Military_situation">Military situation</h3>
<ul><li>First point<ul><li>Nested point</li></ul></li><li>Second point</li></ul>
</section></section>
<section data-mw-section-id="3"><h2 id="References">References</h2><div class="reflist"><ol class="references"><li><span class="mw-cite-backlink">^</span> A source.</li></ol></div></section>
</body></html>"##;

    #[test]
    fn reads_an_article_like_wikipedia_lays_it_out() {
        let a = parse_article(GETTYSBURG, "Battle_of_Gettysburg");
        assert_eq!(a.title, "Battle of Gettysburg");
        let titles: Vec<(u8, &str)> = a.sections.iter().map(|s| (s.level, s.title.as_str())).collect();
        assert_eq!(titles, [(1, ""), (2, "Background"), (3, "Military situation"), (2, "References")]);

        let lead = &a.sections[0].blocks;
        // The hatnote, then the lead paragraph; the navbox is gone.
        let Block::Hatnote(h) = &lead[0] else { panic!("{:?}", lead[0]) };
        assert_eq!(h.iter().map(|r| r.text.as_str()).collect::<String>(), "For other uses, see Battle of Gettysburg (disambiguation).");
        assert_eq!(h[1].link.as_deref(), Some("wiki:Battle of Gettysburg (disambiguation)"));
        let Block::Para(p) = &lead[1] else { panic!("{:?}", lead[1]) };
        let text: String = p.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "The Battle of Gettysburg (July 1–3, 1863) was fought in the American Civil War.[14] See the park.");
        assert!(p.iter().any(|r| r.bold && r.text == "Battle of Gettysburg"));
        assert!(p.iter().any(|r| r.link.as_deref() == Some("wiki:American Civil War")));
        assert!(p.iter().any(|r| r.reference && r.text == "[14]" && r.link.is_none()));
        assert!(p.iter().any(|r| r.link.as_deref() == Some("https://www.nps.gov/gett")));
        assert_eq!(lead.len(), 2);

        // The infobox, taken out of the lead.
        let ib = a.infobox.as_ref().unwrap();
        let first = &ib.rows[0].cells[0];
        assert!(first.header && first.colspan == 2 && first.center);
        assert_eq!(first.background, Some([0xC3, 0xD6, 0xEF]));
        assert!(matches!(&ib.rows[2].cells[0].content[0], Block::Figure { image, .. } if image.url == "https://thumb.wikimedia.org/x/330px-Battle.jpg"));
        // Date and Location, in the nested table; one form of the coordinates.
        let Block::Table(inner) = &ib.rows[3].cells[0].content[0] else { panic!() };
        let cell_text = |c: &Cell| -> String {
            c.content.iter().map(|b| match b {
                Block::Para(l) => l.iter().map(|r| r.text.as_str()).collect::<String>(),
                _ => String::new(),
            }).collect::<Vec<_>>().join("|")
        };
        assert_eq!(cell_text(&inner.rows[0].cells[0]), "Date");
        assert_eq!(cell_text(&inner.rows[1].cells[1]), "Gettysburg, Pennsylvania|39°48′41″N 77°13′33″W");
        // Belligerents: two columns, the flag icon left out.
        let sides: Vec<String> = ib.rows[5].cells.iter().map(cell_text).collect();
        assert_eq!(sides, ["United States", "Confederate States"]);

        // A figure with its caption, and a nested list.
        let bg = &a.sections[1].blocks;
        let Block::Figure { caption, .. } = &bg[0] else { panic!() };
        assert_eq!(caption.iter().map(|r| r.text.as_str()).collect::<String>(), "An 1863 map of the Gettysburg Battlefield");
        let Block::List { items, .. } = &a.sections[2].blocks[0] else { panic!() };
        let items: Vec<(usize, String)> = items.iter().map(|(d, l)| (*d, l.iter().map(|r| r.text.as_str()).collect())).collect();
        assert_eq!(items, [(0, "First point".into()), (1, "Nested point".into()), (0, "Second point".into())]);
        assert_eq!(a.gallery().len(), 2);
    }

    /// NEBULA_WIKI_TEST=/path/page.html prints what the parser makes of it.
    #[test]
    fn real_article() {
        let Ok(path) = std::env::var("NEBULA_WIKI_TEST") else { return };
        let a = parse_article(&std::fs::read_to_string(path).unwrap(), "Test");
        println!("infobox rows: {:?}", a.infobox.as_ref().map(|t| t.rows.len()));
        for s in &a.sections {
            let words: usize = s.blocks.iter().map(|b| match b {
                Block::Para(l) | Block::Hatnote(l) | Block::Quote(l) => l.iter().map(|r| r.text.split_whitespace().count()).sum(),
                _ => 0,
            }).sum();
            let kinds: Vec<&str> = s.blocks.iter().map(|b| match b {
                Block::Para(_) => "p", Block::Hatnote(_) => "hat", Block::Quote(_) => "q",
                Block::List { .. } => "list", Block::Figure { .. } => "fig", Block::Table(_) => "table", Block::Bar(_) => "bar",
            }).collect();
            println!("{}{} ({} words) {:?}", "  ".repeat(s.level as usize - 1), if s.title.is_empty() { "(lead)" } else { &s.title }, words, &kinds[..kinds.len().min(12)]);
        }
        if let Some(Block::Para(l)) = a.sections[0].blocks.iter().find(|b| matches!(b, Block::Para(_))) {
            println!("lead: {}", l.iter().map(|r| r.text.as_str()).collect::<String>().chars().take(400).collect::<String>());
        }
    }

    /// NEBULA_WIKI_LIVE=1: the real API (main page, search, random, an article).
    #[test]
    fn live_api() {
        if std::env::var("NEBULA_WIKI_LIVE").is_err() {
            return;
        }
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let f = featured().await.unwrap();
            println!("featured: {:?}", f.article.as_ref().map(|a| &a.title));
            println!("news {}, on this day {}, most read {}, picture {:?}", f.news.len(), f.on_this_day.len(), f.most_read.len(), f.picture.as_ref().map(|p| &p.title));
            if let Some(n) = f.news.first() {
                println!("news[0]: {}", n.iter().map(|r| r.text.as_str()).collect::<String>());
            }
            let s = search("gettysburg").await.unwrap();
            println!("search: {:?}", s.iter().take(3).map(|x| (&x.title, &x.extract[..x.extract.len().min(60)])).collect::<Vec<_>>());
            println!("random: {}", random_title().await.unwrap());
            let a = article("Café").await.unwrap();
            println!("Café: {} sections, infobox {}, {} images", a.sections.len(), a.infobox.is_some(), a.gallery().len());
            let img = image(&a.gallery()[0].0.url).await.unwrap();
            println!("first image: {} bytes", img.len());
        });
    }

    #[test]
    fn links_and_dates() {
        let a = |href: &str| El { name: "a".into(), attrs: vec![("href".into(), href.into()), ("rel".into(), "mw:WikiLink".into())], kids: vec![] };
        assert_eq!(link_target(&a("./Robert_E._Lee")).as_deref(), Some("wiki:Robert E. Lee"));
        assert_eq!(link_target(&a("./Caf%C3%A9#History")).as_deref(), Some("wiki:Café"));
        assert_eq!(link_target(&a("./File:Map.png")), None);
        assert_eq!(link_target(&a("./Category:Battles")), None);
        assert_eq!(background("text-align:center;background-color:#ccc"), Some([0xcc, 0xcc, 0xcc]));
        assert_eq!(path_title("Battle of Gettysburg"), "Battle_of_Gettysburg");
        assert_eq!(path_title("AC/DC"), "AC%2FDC");
        assert_eq!(
            full_size("https://thumb.wikimedia.org/wikipedia/commons/thumb/0/0f/Battle_%28cropped%29.jpg/330px-Battle_%28cropped%29.jpg?utm_source=x"),
            "https://upload.wikimedia.org/wikipedia/commons/0/0f/Battle_%28cropped%29.jpg"
        );
        // A drawing stays a (rendered) thumbnail.
        assert_eq!(
            full_size("https://upload.wikimedia.org/wikipedia/commons/thumb/a/ab/Map.svg/250px-Map.svg.png"),
            "https://upload.wikimedia.org/wikipedia/commons/thumb/a/ab/Map.svg/250px-Map.svg.png"
        );
        let today = chrono_today();
        assert_eq!(today.len(), 10);
        assert_eq!(&today[4..5], "/");
    }
}
