//! Wikipedia, read in the app (opt-in, Settings → Library menu): the main
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

/// The Wikipedias offered (code, name in its own language), biggest
/// first after English. Every language's Wikipedia has the same API.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("es", "Español"),
    ("ja", "日本語"),
    ("ru", "Русский"),
    ("it", "Italiano"),
    ("zh", "中文"),
    ("pt", "Português"),
    ("pl", "Polski"),
    ("nl", "Nederlands"),
    ("ar", "العربية"),
    ("fa", "فارسی"),
    ("uk", "Українська"),
    ("sv", "Svenska"),
    ("vi", "Tiếng Việt"),
    ("ko", "한국어"),
    ("id", "Bahasa Indonesia"),
    ("he", "עברית"),
    ("tr", "Türkçe"),
    ("cs", "Čeština"),
    ("hu", "Magyar"),
    ("fi", "Suomi"),
    ("no", "Norsk bokmål"),
    ("da", "Dansk"),
    ("ro", "Română"),
    ("el", "Ελληνικά"),
    ("ca", "Català"),
    ("th", "ไทย"),
    ("hi", "हिन्दी"),
];

/// The Wikipedia everything is read from (a code from [`LANGUAGES`]).
static LANGUAGE: std::sync::RwLock<&'static str> = std::sync::RwLock::new("en");

/// Switches to another language's Wikipedia; an unknown code means English.
pub fn set_language(code: &str) {
    let code = LANGUAGES.iter().find(|(c, _)| *c == code).map_or("en", |(c, _)| c);
    *LANGUAGE.write().unwrap_or_else(|e| e.into_inner()) = code;
}

pub fn language() -> &'static str {
    *LANGUAGE.read().unwrap_or_else(|e| e.into_inner())
}

fn host() -> String {
    format!("https://{}.wikipedia.org", language())
}
const USER_AGENT: &str = concat!("NebulaReader/", env!("CARGO_PKG_VERSION"), " (https://github.com/NotTimm/NebulaReader)");

pub const LICENSE_URL: &str = "https://creativecommons.org/licenses/by-sa/4.0/";

pub(crate) fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("http client")
    })
}

pub(crate) async fn get(url: &str) -> Result<reqwest::Response, String> {
    let resp = client().get(url).send().await.map_err(|e| fl!("jf-unreachable", error = e.to_string()))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    Ok(resp)
}

/// An article's address on Wikipedia.
pub fn page_url(title: &str) -> String {
    format!("{}/wiki/{}", host(), title.replace(' ', "_"))
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
    let url = format!("{}/api/rest_v1/feed/featured/{today}", host());
    let resp = client().get(&url).send().await.map_err(|e| fl!("jf-unreachable", error = e.to_string()))?;
    // Not every language's Wikipedia has the daily feed: its main page is
    // then just search and Random.
    if !resp.status().is_success() && language() != "en" && resp.status().is_client_error() {
        return Ok(Featured::default());
    }
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    Ok(parse_featured(&json))
}

/// `YYYY/MM/DD` for today in UTC.
pub fn chrono_today() -> String {
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
        get(&format!("{}/api/rest_v1/page/random/summary", host())).await?.json().await.map_err(|e| e.to_string())?;
    summary(&json).map(|s| s.title).ok_or_else(|| "no article".to_string())
}

/// Articles matching `query`.
pub async fn search(query: &str) -> Result<Vec<Summary>, String> {
    let q: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
    let json: serde_json::Value =
        get(&format!("{}/w/rest.php/v1/search/page?q={q}&limit=20", host())).await?.json().await.map_err(|e| e.to_string())?;
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

pub(crate) fn strip_tags(html: &str) -> String {
    let root = parse_html(&format!("<div>{html}</div>"));
    root.text().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// An image (a thumbnail from Wikimedia's servers).
pub async fn image(url: &str) -> Result<Vec<u8>, String> {
    get(url).await?.bytes().await.map(|b| b.to_vec()).map_err(|e| e.to_string())
}

/// An article, by title.
pub async fn article(title: &str) -> Result<Article, String> {
    let html = get(&format!("{}/api/rest_v1/page/html/{}", host(), path_title(title))).await?.text().await.map_err(|e| e.to_string())?;
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
    /// Pictures side by side (a gallery, or a row of picture links).
    Gallery(Vec<GalleryItem>),
    /// Tabs, one shown at a time (Fandom's tabbers): `id` is unique in
    /// the article, for which tab is picked.
    Tabs { id: usize, tabs: Vec<(String, Vec<Block>)> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct GalleryItem {
    pub image: Image,
    pub caption: Line,
    /// Where a picture used as a link goes (`wiki:<Title>`); without one
    /// it opens in the picture viewer.
    pub link: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub rows: Vec<Row>,
    pub caption: Line,
    /// Floated right on the site (a chart or a small box beside the text).
    pub float_right: bool,
    /// Drawn like an infobox (a Fandom infobox that isn't the article's
    /// first).
    pub infobox: bool,
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
        let mut out = Vec::new();
        if let Some(t) = &self.infobox {
            table_pictures(t, &mut out);
        }
        for s in &self.sections {
            block_pictures(&s.blocks, &mut out);
        }
        out
    }
}

/// The full-size file behind a thumbnail
/// (`…/thumb/0/0f/Name.jpg/330px-Name.jpg` → `…/0/0f/Name.jpg`), for the
/// picture viewer. Drawings (SVG) and other files the app can't decode
/// keep their thumbnail, asked for wider instead.
pub fn full_size(thumb: &str) -> String {
    // Fandom (and other wikis on its picture server): `…/File.png/revision/latest/scale-to-width-down/180?cb=…`.
    if let Some(i) = thumb.find("/revision/latest") {
        let head = &thumb[..i + "/revision/latest".len()];
        let file = thumb[..i].rsplit('/').next().unwrap_or_default().to_ascii_lowercase();
        if file.ends_with(".svg") {
            return thumb.to_string();
        }
        return match thumb[i..].split_once('?') {
            Some((_, query)) => format!("{head}?{query}"),
            None => head.to_string(),
        };
    }
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

/// The wiki whose page is being read: how its links to its own articles
/// look, and its colour. Wikipedia's (Parsoid) links are `./Title`;
/// other MediaWiki sites' are `/wiki/Title` or the full address.
#[derive(Clone, Debug, Default)]
pub struct ParseSite {
    /// `https://residentevil.fandom.com`, or empty.
    pub server: String,
    /// `/wiki/`, or `/es/wiki/` for a wiki in another language.
    pub article_prefix: String,
    /// The wiki's own colour, for infobox headings.
    pub accent: Option<[u8; 3]>,
}

thread_local! {
    static SITE: std::cell::RefCell<ParseSite> = std::cell::RefCell::new(ParseSite { article_prefix: "/wiki/".into(), ..Default::default() });
    /// The next [`Block::Tabs`] id.
    static NEXT_TABS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// [`parse_article`] for a page of another MediaWiki site (a Fandom wiki).
pub fn parse_article_on(html: &str, title: &str, site: ParseSite) -> Article {
    let old = SITE.with(|s| s.replace(site));
    let a = parse_article(html, title);
    SITE.with(|s| s.replace(old));
    a
}

/// The colour for an infobox's headings: the wiki's own, lightened so
/// dark text reads on it, or a quiet blue-grey.
fn heading_band() -> [u8; 3] {
    match SITE.with(|s| s.borrow().accent) {
        Some(c) => c.map(|v| (v as f32 * 0.35 + 255.0 * 0.65) as u8),
        None => [0xd5, 0xde, 0xea],
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
        // Other MediaWiki sites (Fandom): the old-style contents box
        // (the app has its own) and print-only bits.
        "toc",
        "printonly",
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
    /// Tables used for layout (the main page's) are read as their cells'
    /// contents, not drawn as tables.
    layout_tables: bool,
    /// The block that's a row of pictures used as links, which the next
    /// such picture joins.
    picture_row: Option<usize>,
}

/// Where a link goes: `wiki:<Title>` for an article, a web address, or
/// nowhere (files, this page's own footnotes).
fn link_target(a: &El) -> Option<String> {
    let href = a.attr("href")?;
    // A page that doesn't exist yet.
    if a.has_class("new") || href.contains("redlink=1") {
        return None;
    }
    let rel = a.attr("rel").unwrap_or_default();
    let local = if rel.contains("mw:WikiLink") || href.starts_with("./") {
        Some(href.trim_start_matches("./").to_string())
    } else {
        SITE.with(|s| {
            let s = s.borrow();
            let bare = s.server.split_once("//").map(|(_, h)| format!("//{h}"));
            let path = [Some(s.server.as_str()), bare.as_deref()]
                .into_iter()
                .flatten()
                .filter(|p| !p.is_empty())
                .find_map(|p| href.strip_prefix(p))
                .unwrap_or(href);
            path.strip_prefix(s.article_prefix.as_str()).filter(|_| !s.article_prefix.is_empty()).map(str::to_string)
        })
    };
    if let Some(page) = local {
        let page = page.split('?').next().unwrap_or_default();
        let (title, _fragment) = page.split_once('#').unwrap_or((page, ""));
        if title.is_empty() || title.contains(':') && !title.starts_with("Special:") && is_namespace(title) {
            return None;
        }
        // A `+` in a page address is a plus, not a space.
        let title = title.replace('+', "%2B");
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
    const NS: &[&str] = &[
        "File", "Category", "Help", "Wikipedia", "Template", "Portal", "Talk", "Special", "Module", "Draft", "User", "Image", "Media",
        // Fandom's.
        "User_blog", "User blog", "Message_Wall", "Message Wall", "Board", "Thread", "Forum", "Blog", "MediaWiki",
    ];
    title.split_once(':').is_some_and(|(ns, _)| NS.iter().any(|n| n.eq_ignore_ascii_case(ns.trim_end_matches("_talk"))))
}

pub(crate) fn fix_url(src: &str) -> String {
    if src.starts_with("//") { format!("https:{src}") } else { src.to_string() }
}

/// An image element as a picture, unless it's an icon (flags, symbols).
fn image_of(img: &El) -> Option<Image> {
    // Fandom loads pictures lazily: the real address is in `data-src`.
    let src = img.attr("src").filter(|s| !s.starts_with("data:")).or_else(|| img.attr("data-src"))?;
    let url = fix_url(src);
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
        // A Fandom infobox: the article's (floated right like Wikipedia's),
        // or another one further down.
        if e.has_class("portable-infobox") {
            self.flush();
            let mut t = portable_infobox(e);
            if t.rows.is_empty() {
                return;
            }
            t.infobox = true;
            t.float_right = true;
            if self.take_infobox && self.infobox.is_none() {
                self.infobox = Some(t);
            } else {
                self.blocks.push(Block::Table(t));
            }
            return;
        }
        if is_gallery(e) {
            self.flush();
            let items = gallery_items(e);
            if !items.is_empty() {
                self.blocks.push(Block::Gallery(items));
            }
            return;
        }
        if let Some(panes) = tabber(e) {
            self.flush();
            let id = NEXT_TABS.with(|n| n.replace(n.get() + 1));
            let tabs: Vec<(String, Vec<Block>)> = panes
                .into_iter()
                .map(|(label, pane)| {
                    let mut b = Builder { layout_tables: self.layout_tables, ..Default::default() };
                    b.blocks(pane, ctx);
                    b.flush();
                    (label, b.blocks)
                })
                .filter(|(_, blocks)| !blocks.is_empty())
                .collect();
            match tabs.len() {
                0 => {}
                // One tab: just its content.
                1 => self.blocks.extend(tabs.into_iter().next().unwrap().1),
                _ => self.blocks.push(Block::Tabs { id, tabs }),
            }
            return;
        }
        // A picture used as a link, or on its own outside any text (a
        // wiki's main page is often rows of them): side by side.
        if let Some(item) = picture_link(e) {
            self.flush();
            // Next to the last one (not added to a real gallery before it).
            let joins = self.picture_row.is_some_and(|i| i + 1 == self.blocks.len());
            match self.blocks.last_mut() {
                Some(Block::Gallery(items)) if joins => items.push(item),
                _ => {
                    self.blocks.push(Block::Gallery(vec![item]));
                    self.picture_row = Some(self.blocks.len() - 1);
                }
            }
            return;
        }
        // An older-style thumbnail: the picture, then its caption beside it
        // (the main page's).
        if name == "div" && e.has_class("thumbinner") {
            if let Some(image) = e.find(&|x| x.name == "img").and_then(image_of) {
                self.flush();
                let mut cap = Builder::default();
                if let Some(c) = e.find(&|x| x.has_class("thumbcaption")) {
                    cap.inlines(c, Ctx::default());
                }
                self.blocks.push(Block::Figure { image, caption: trim_line(cap.line) });
            }
            return;
        }
        // A horizontal list ("a · b · c"): one line, inline when its class
        // says so ("More anniversaries: …").
        if e.has_class("hlist") && matches!(name, "div" | "span") {
            let inline = e.has_class("inline");
            if !inline {
                self.flush();
            }
            let mut items: Vec<&El> = Vec::new();
            fn lis<'a>(e: &'a El, out: &mut Vec<&'a El>) {
                for k in &e.kids {
                    if let Node::El(c) = k {
                        if hidden(c) {
                            continue;
                        }
                        if c.name == "li" {
                            out.push(c);
                        } else {
                            lis(c, out);
                        }
                    }
                }
            }
            lis(e, &mut items);
            for (i, li) in items.iter().enumerate() {
                if i > 0 {
                    self.push_text(" · ", ctx, false);
                }
                self.inlines(li, ctx.clone());
            }
            if !inline {
                self.flush();
            }
            return;
        }
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
            "table" if self.layout_tables && e.attr("role") == Some("presentation") => {
                self.flush();
                fn cells<'a>(e: &'a El, out: &mut Vec<&'a El>) {
                    for k in &e.kids {
                        if let Node::El(c) = k {
                            if matches!(c.name.as_str(), "td" | "th") {
                                out.push(c);
                            } else {
                                cells(c, out);
                            }
                        }
                    }
                }
                let mut found = Vec::new();
                cells(e, &mut found);
                for c in found {
                    self.blocks(c, ctx);
                    self.flush();
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

/// A gallery: Wikipedia's (`ul.gallery`), Fandom's (`div.wikia-gallery`)
/// or a slideshow.
fn is_gallery(e: &El) -> bool {
    e.attr("class").is_some_and(|c| {
        c.split_whitespace()
            .any(|w| w == "gallery" || w == "wikia-gallery" || w.starts_with("mw-gallery") || w.contains("slideshow"))
    })
}

/// A gallery's pictures with their captions (any picture in it, if its
/// items aren't marked).
fn gallery_items(e: &El) -> Vec<GalleryItem> {
    fn item_els<'a>(e: &'a El, out: &mut Vec<&'a El>) {
        for k in &e.kids {
            if let Node::El(c) = k {
                if hidden(c) {
                    continue;
                }
                if c.has_class("gallerybox") || c.has_class("wikia-gallery-item") || c.has_class("gallery-item") {
                    out.push(c);
                } else {
                    item_els(c, out);
                }
            }
        }
    }
    let mut els = Vec::new();
    item_els(e, &mut els);
    let caption_of = |el: &El| -> Line {
        let Some(c) = el.find(&|x| x.has_class("gallerytext") || x.has_class("lightbox-caption") || x.name == "figcaption") else {
            return Line::new();
        };
        let mut b = Builder::default();
        b.inlines(c, Ctx::default());
        trim_line(b.line)
    };
    let link_of = |el: &El| el.find(&|x| x.name == "a" && x.find(&|i| i.name == "img").is_some()).and_then(link_target).filter(|l| l.starts_with("wiki:"));
    if els.is_empty() {
        let mut imgs = Vec::new();
        fn all_imgs<'a>(e: &'a El, out: &mut Vec<&'a El>) {
            for k in &e.kids {
                if let Node::El(c) = k {
                    if hidden(c) {
                        continue;
                    }
                    if c.name == "img" { out.push(c) } else { all_imgs(c, out) }
                }
            }
        }
        all_imgs(e, &mut imgs);
        return imgs.into_iter().filter_map(image_of).map(|image| GalleryItem { image, caption: Line::new(), link: None }).collect();
    }
    els.into_iter()
        .filter_map(|el| {
            let image = el.find(&|x| x.name == "img").and_then(image_of)?;
            Some(GalleryItem { image, caption: caption_of(el), link: link_of(el) })
        })
        .collect()
}

/// A link that's only a picture (`<a href="/wiki/Characters"><img></a>`),
/// or a picture element linking to an article: a gallery item.
fn picture_link(e: &El) -> Option<GalleryItem> {
    let link_el = match e.name.as_str() {
        "a" => e,
        "span" if e.attr("typeof").is_some_and(|t| t.starts_with("mw:File")) => e.find(&|x| x.name == "a")?,
        _ => return None,
    };
    let img = link_el.find(&|x| x.name == "img")?;
    if !e.text().trim().is_empty() {
        return None;
    }
    let link = link_target(link_el).filter(|l| l.starts_with("wiki:"));
    // A picture element that isn't a link is a figure, as before.
    if e.name == "span" && link.is_none() {
        return None;
    }
    Some(GalleryItem { image: image_of(img)?, caption: Line::new(), link })
}

/// Tabs: Fandom's (`wds-tabber`), the older Tabber's (`tabbertab`) and
/// TabberNeue's (`tabber__panel`, on wiki.gg): each tab's label and content.
fn tabber(e: &El) -> Option<Vec<(String, &El)>> {
    if !(e.has_class("tabber") || e.has_class("wds-tabber")) {
        return None;
    }
    fn is_panel(e: &El) -> bool {
        e.has_class("wds-tab__content") || e.has_class("tabbertab") || e.has_class("tabber__panel")
    }
    fn panels<'a>(e: &'a El, out: &mut Vec<&'a El>) {
        for k in &e.kids {
            if let Node::El(c) = k {
                if is_panel(c) {
                    out.push(c);
                } else if !(c.has_class("tabber") || c.has_class("wds-tabber")) {
                    panels(c, out);
                }
            }
        }
    }
    let mut found = Vec::new();
    panels(e, &mut found);
    if found.is_empty() {
        return None;
    }
    // Fandom's labels are a list above the panels.
    let mut labels: Vec<String> = Vec::new();
    fn tab_labels(e: &El, out: &mut Vec<String>) {
        for k in &e.kids {
            if let Node::El(c) = k {
                if c.has_class("wds-tabs__tab") || c.has_class("tabber__tab") {
                    out.push(clean(&c.text()));
                } else if !(c.has_class("wds-tab__content") || c.has_class("tabber__panel")) {
                    tab_labels(c, out);
                }
            }
        }
    }
    tab_labels(e, &mut labels);
    Some(
        found
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                let label = ["data-mw-tabber-title", "data-title", "title", "data-hash"]
                    .iter()
                    .find_map(|a| p.attr(a))
                    .map(clean)
                    .filter(|l| !l.is_empty())
                    .or_else(|| labels.get(i).cloned().filter(|l| !l.is_empty()))
                    .unwrap_or_else(|| (i + 1).to_string());
                (label, p)
            })
            .collect(),
    )
}

/// A Fandom (portable) infobox as a table like Wikipedia's: the title and
/// group headings as coloured rows across, pictures across, and a row of
/// label and value for each item.
fn portable_infobox(aside: &El) -> Table {
    let band = heading_band();
    let blocks_of = |el: &El, bold: bool| -> Vec<Block> {
        let mut b = Builder::default();
        b.blocks(el, &Ctx { bold, ..Ctx::default() });
        b.flush();
        b.blocks
    };
    let across = |content: Vec<Block>, header: bool, background: Option<[u8; 3]>| Row {
        cells: vec![Cell { header, colspan: 2, background, center: true, content }],
    };
    fn walk(el: &El, out: &mut Table, band: [u8; 3], blocks_of: &dyn Fn(&El, bool) -> Vec<Block>, across: &dyn Fn(Vec<Block>, bool, Option<[u8; 3]>) -> Row) {
        for k in &el.kids {
            let Node::El(e) = k else { continue };
            if hidden(e) {
                continue;
            }
            if e.has_class("pi-title") || e.has_class("pi-header") {
                out.rows.push(across(blocks_of(e, true), true, Some(band)));
            } else if e.has_class("pi-image-collection") || e.has_class("pi-panel") {
                // Pictures (or sections) in tabs: as tabs.
                let tabs = tabber(e).unwrap_or_default();
                let tabs: Vec<(String, Vec<Block>)> = tabs
                    .into_iter()
                    .map(|(label, pane)| {
                        let mut inner = Table::default();
                        walk(pane, &mut inner, band, blocks_of, across);
                        // A pane of pictures is just its pictures.
                        let blocks: Vec<Block> = if inner.rows.iter().all(|r| r.cells.len() == 1) {
                            inner.rows.into_iter().flat_map(|r| r.cells).flat_map(|c| c.content).collect()
                        } else {
                            inner.infobox = true;
                            vec![Block::Table(inner)]
                        };
                        (label, blocks)
                    })
                    .filter(|(_, b)| !b.is_empty())
                    .collect();
                if tabs.len() > 1 {
                    let id = NEXT_TABS.with(|n| n.replace(n.get() + 1));
                    out.rows.push(across(vec![Block::Tabs { id, tabs }], false, None));
                } else if let Some((_, blocks)) = tabs.into_iter().next() {
                    out.rows.push(across(blocks, false, None));
                }
            } else if e.has_class("pi-image") {
                if let Some(image) = e.find(&|x| x.name == "img").and_then(image_of) {
                    let mut cap = Builder::default();
                    if let Some(c) = e.find(&|x| x.has_class("pi-caption")) {
                        cap.inlines(c, Ctx::default());
                    }
                    out.rows.push(across(vec![Block::Figure { image, caption: trim_line(cap.line) }], false, None));
                }
            } else if e.has_class("pi-data") {
                let label = e.find(&|x| x.has_class("pi-data-label"));
                let value = e.find(&|x| x.has_class("pi-data-value"));
                let value = value.map(|v| blocks_of(v, false)).unwrap_or_default();
                if value.is_empty() {
                    continue;
                }
                match label.map(|l| blocks_of(l, true)).filter(|l| !l.is_empty()) {
                    Some(label) => out.rows.push(Row {
                        cells: vec![
                            Cell { header: true, colspan: 1, content: label, ..Default::default() },
                            Cell { colspan: 1, content: value, ..Default::default() },
                        ],
                    }),
                    None => out.rows.push(across(value, false, None)),
                }
            } else if e.name == "table" && e.has_class("pi-horizontal-group") {
                let t = table(e);
                if let Some(c) = (!t.caption.is_empty()).then(|| vec![Block::Para(t.caption.clone())]) {
                    out.rows.push(across(c, true, Some(band)));
                }
                for mut r in t.rows {
                    for c in &mut r.cells {
                        c.center = true;
                    }
                    out.rows.push(r);
                }
            } else if e.has_class("pi-smart-group") {
                // Labels side by side, then their values.
                let mut labels = Row::default();
                let mut values = Row::default();
                for k in &e.kids {
                    let Node::El(c) = k else { continue };
                    let span = c.attr("data-item-span").and_then(|s| s.parse().ok()).unwrap_or(1);
                    let cell = |header, content| Cell { header, colspan: span, center: true, content, ..Default::default() };
                    if c.has_class("pi-smart-group-head") || c.has_class("pi-smart-data-label") {
                        labels.cells.push(cell(true, blocks_of(c, true)));
                    } else if c.has_class("pi-smart-group-body") || c.has_class("pi-smart-data-value") {
                        values.cells.push(cell(false, blocks_of(c, false)));
                    } else {
                        // The head and body are wrappers in newer markup.
                        for kk in &c.kids {
                            let Node::El(cc) = kk else { continue };
                            let span = cc.attr("data-item-span").and_then(|s| s.parse().ok()).unwrap_or(1);
                            let cell = |header, content| Cell { header, colspan: span, center: true, content, ..Default::default() };
                            if cc.has_class("pi-smart-data-label") {
                                labels.cells.push(cell(true, blocks_of(cc, true)));
                            } else if cc.has_class("pi-smart-data-value") {
                                values.cells.push(cell(false, blocks_of(cc, false)));
                            }
                        }
                    }
                }
                for r in [labels, values] {
                    if r.cells.iter().any(|c| !c.content.is_empty()) {
                        out.rows.push(r);
                    }
                }
            } else if e.has_class("pi-navigation") || e.has_class("pi-caption") {
                let blocks = blocks_of(e, false);
                if !blocks.is_empty() {
                    out.rows.push(across(blocks, false, None));
                }
            } else {
                // Groups and wrappers.
                walk(e, out, band, blocks_of, across);
            }
        }
    }
    let mut t = Table::default();
    walk(aside, &mut t, band, &blocks_of, &across);
    t
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
/// The pictures in some blocks, with their captions.
fn block_pictures(blocks: &[Block], out: &mut Vec<(Image, Line)>) {
    for b in blocks {
        match b {
            Block::Figure { image, caption } => out.push((image.clone(), caption.clone())),
            Block::Table(t) => table_pictures(t, out),
            Block::Gallery(items) => out.extend(items.iter().filter(|i| i.link.is_none()).map(|i| (i.image.clone(), i.caption.clone()))),
            Block::Tabs { tabs, .. } => {
                for (_, blocks) in tabs {
                    block_pictures(blocks, out);
                }
            }
            _ => {}
        }
    }
}

fn table_pictures(t: &Table, out: &mut Vec<(Image, Line)>) {
    for c in t.rows.iter().flat_map(|r| &r.cells) {
        block_pictures(&c.content, out);
    }
}

// ── The main page ──────────────────────────────────────────────────────────

/// Where a box sits on the main page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Left,
    Right,
    Full,
}

/// One of the main page's boxes ("From today's featured article", "Did you
/// know …", "In the news"…), as the site shows it today.
#[derive(Clone, Debug, PartialEq)]
pub struct MainBox {
    /// Its id on the site without `mp-`: tfa, dyk, itn, otd, tfl, tfp.
    pub id: String,
    pub title: String,
    pub column: Column,
    pub blocks: Vec<Block>,
}

/// English Wikipedia's main page, read from the page itself, so it shows
/// what the site does today, in its order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MainPage {
    /// "Welcome to Wikipedia,"
    pub welcome: String,
    /// "the free encyclopedia that anyone can edit."
    pub tagline: String,
    /// "7,258,440 articles in English"
    pub count: String,
    pub boxes: Vec<MainBox>,
}

impl MainPage {
    pub fn gallery(&self) -> Vec<(Image, Line)> {
        let mut out = Vec::new();
        for b in &self.boxes {
            block_pictures(&b.blocks, &mut out);
        }
        out
    }
}

/// The boxes worth reading, in the site's order. Left out: "Help us
/// improve this article!" (afi), and the boxes about Wikipedia itself
/// (other areas, sister projects, languages).
const MAIN_BOXES: &[&str] = &["tfa", "dyk", "itn", "otd", "tfl", "tfp"];

fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reads a main page laid out like English Wikipedia's (`mp-tfa`,
/// `mp-dyk`…); `None` for one that isn't.
pub fn parse_main_page(html: &str) -> Option<MainPage> {
    let root = parse_html(html);
    let by_id = |id: &str| root.find(&|e| e.attr("id") == Some(id));
    let mut page = MainPage {
        welcome: by_id("mp-welcome").map(|e| clean(&e.text())).unwrap_or_default(),
        tagline: by_id("mp-free").map(|e| clean(&e.text())).unwrap_or_default(),
        count: by_id("articlecount")
            .and_then(|c| c.find(&|e| e.name == "li" && e.text().contains("article")))
            .map(|e| clean(&e.text()))
            .unwrap_or_default(),
        boxes: Vec::new(),
    };
    let columns = [("mp-left", Column::Left), ("mp-right", Column::Right)];
    for id in MAIN_BOXES {
        let Some(el) = by_id(&format!("mp-{id}")) else { continue };
        let title = by_id(&format!("mp-{id}-h2")).map(|h| clean(&h.text())).unwrap_or_default();
        let column = columns
            .iter()
            .find(|(c, _)| by_id(c).is_some_and(|c| c.find(&|e| e.attr("id") == Some(&format!("mp-{id}"))).is_some()))
            .map_or(Column::Full, |(_, col)| *col);
        let mut b = Builder { layout_tables: true, ..Default::default() };
        b.blocks(el, &Ctx::default());
        b.flush();
        let blocks: Vec<Block> = b.blocks.into_iter().filter(|b| !matches!(b, Block::Para(l) if l.is_empty())).collect();
        if !blocks.is_empty() {
            page.boxes.push(MainBox { id: id.to_string(), title, column, blocks });
        }
    }
    (!page.boxes.is_empty()).then_some(page)
}

/// Today's main page: the page itself where it's laid out like English
/// Wikipedia's, and the daily feed (most read, and the main page of a
/// language whose page isn't read).
pub async fn home() -> Result<(Option<MainPage>, Option<Featured>), String> {
    let main = async {
        let html = get(&format!("{}/api/rest_v1/page/html/Main_Page", host())).await?.text().await.map_err(|e| e.to_string())?;
        Ok::<_, String>(parse_main_page(&html))
    };
    let (main, feed) = cosmic::iced::futures::future::join(main, featured()).await;
    let main = main.ok().flatten();
    match (main, feed) {
        (None, Err(e)) => Err(e),
        (main, feed) => Ok((main, feed.ok())),
    }
}

pub fn parse_article(html: &str, title: &str) -> Article {
    NEXT_TABS.with(|n| n.set(0));
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
                // The action=parse API's wrapper (other MediaWiki sites).
                Node::El(e) if e.name == "div" && e.has_class("mw-parser-output") => walk(e, article, current, b),
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
                Block::List { .. } => "list", Block::Figure { .. } => "fig", Block::Table(_) => "table", Block::Bar(_) => "bar", Block::Gallery(_) => "gallery", Block::Tabs { .. } => "tabs",
            }).collect();
            println!("{}{} ({} words) {:?}", "  ".repeat(s.level as usize - 1), if s.title.is_empty() { "(lead)" } else { &s.title }, words, &kinds[..kinds.len().min(12)]);
        }
        if let Some(Block::Para(l)) = a.sections[0].blocks.iter().find(|b| matches!(b, Block::Para(_))) {
            println!("lead: {}", l.iter().map(|r| r.text.as_str()).collect::<String>().chars().take(400).collect::<String>());
        }
    }

    #[test]
    fn the_main_page_is_read_like_the_site() {
        let html = r#"<body><div id="mp-welcomecount"><div id="mp-welcome"><h1>Welcome to <a href="./Wikipedia">Wikipedia</a></h1>,</div>
<div id="mp-free">the free encyclopedia that anyone can edit.</div>
<div id="articlecount"><ul><li><a href="./Special:Statistics">266,512</a> active editors</li><li><a href="./Special:Statistics">7,258,440</a> articles in <a href="./English_language">English</a></li></ul></div></div>
<div id="mp-upper"><div id="mp-left">
<h2 id="mp-tfa-h2" class="mp-h2">From today's featured article</h2>
<div id="mp-tfa"><p>The <b><a rel="mw:WikiLink" href="./Ship">ship</a></b> sank.</p>
<div class="hlist tfa-footer noprint"><ul><li><a href="./Wikipedia:TFA">Archive</a></li><li>By email</li></ul></div></div>
<h2 id="mp-dyk-h2" class="mp-h2">Did you know ...</h2>
<div id="mp-dyk"><ul><li>... that <a rel="mw:WikiLink" href="./Dragon">a dragon</a> exists?</li></ul></div></div>
<div id="mp-right"><h2 id="mp-afi-h2">Help us improve this article!</h2><div id="mp-afi"><p>Edit me</p></div>
<h2 id="mp-itn-h2" class="mp-h2">In the news</h2>
<div id="mp-itn"><div class="itn-img"><div class="thumbinner"><span typeof="mw:File"><a href="./File:X.jpg"><img src="//upload.wikimedia.org/x/120px-X.jpg" width="120" height="150"/></a></span><div class="thumbcaption">Someone in 1999</div></div></div>
<ul><li><a rel="mw:WikiLink" href="./Someone">Someone</a> <i>(pictured)</i> dies.</li></ul>
<div><b>Ongoing</b>: <div class="hlist inline"><ul><li><a rel="mw:WikiLink" href="./War_A">War A</a></li><li><a rel="mw:WikiLink" href="./War_B">War B</a></li></ul></div></div></div>
<h2 id="mp-otd-h2" class="mp-h2">On this day</h2>
<div id="mp-otd"><p><b><a rel="mw:WikiLink" href="./November_2">November 2</a></b></p><ul><li><a rel="mw:WikiLink" href="./1943">1943</a> – A battle.</li></ul>
<div class="hlist"><ul><li><a rel="mw:WikiLink" href="./Matilda">Matilda</a> (d. 1083)</li><li><a rel="mw:WikiLink" href="./James_Dunn">James Dunn</a> (b. 1901)</li></ul></div></div></div></div>
<div id="mp-lower"><h2 id="mp-tfp-h2" class="mp-h2">Today's featured picture</h2>
<div id="mp-tfp"><table role="presentation"><tbody><tr><td><span typeof="mw:File"><a href="./File:M.jpg"><img src="//upload.wikimedia.org/m/500px-M.jpg" width="500" height="235"/></a></span></td></tr><tr><td><p><b><a rel="mw:WikiLink" href="./Modriku">Mõdriku</a></b> is a village.</p></td></tr></tbody></table></div></div>
<div id="mp-other"><h2>Other areas of Wikipedia</h2><p>Community portal</p></div></body>"#;
        let m = parse_main_page(html).unwrap();
        assert_eq!(m.welcome, "Welcome to Wikipedia,");
        assert_eq!(m.count, "7,258,440 articles in English");
        let ids: Vec<(&str, Column)> = m.boxes.iter().map(|b| (b.id.as_str(), b.column)).collect();
        assert_eq!(ids, [("tfa", Column::Left), ("dyk", Column::Left), ("itn", Column::Right), ("otd", Column::Right), ("tfp", Column::Full)]);
        assert_eq!(m.boxes[0].title, "From today's featured article");
        let text = |b: &Block| match b {
            Block::Para(l) => l.iter().map(|r| r.text.as_str()).collect::<String>(),
            Block::List { items, .. } => items.iter().map(|(_, l)| l.iter().map(|r| r.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("|"),
            Block::Figure { caption, .. } => format!("fig:{}", caption.iter().map(|r| r.text.as_str()).collect::<String>()),
            _ => String::new(),
        };
        // The footer (Archive, By email) is gone.
        assert_eq!(m.boxes[0].blocks.iter().map(text).collect::<Vec<_>>(), ["The ship sank."]);
        let itn: Vec<String> = m.boxes[2].blocks.iter().map(text).collect();
        assert_eq!(itn, ["fig:Someone in 1999", "Someone (pictured) dies.", "Ongoing: War A · War B"]);
        let otd: Vec<String> = m.boxes[3].blocks.iter().map(text).collect();
        assert_eq!(otd, ["November 2", "1943 – A battle.", "Matilda (d. 1083) · James Dunn (b. 1901)"]);
        // Links survive.
        let Block::Para(ongoing) = &m.boxes[2].blocks[2] else { panic!() };
        assert_eq!(ongoing.iter().filter_map(|r| r.link.as_deref()).collect::<Vec<_>>(), ["wiki:War A", "wiki:War B"]);
        let tfp: Vec<String> = m.boxes[4].blocks.iter().map(text).collect();
        assert_eq!(tfp, ["fig:", "Mõdriku is a village."]);
        assert_eq!(m.gallery().len(), 2);
    }

    /// NEBULA_WIKI_MAIN=/path/Main_Page.html prints what the main page
    /// parser makes of a saved main page.
    #[test]
    fn real_main_page() {
        let Ok(path) = std::env::var("NEBULA_WIKI_MAIN") else { return };
        let m = parse_main_page(&std::fs::read_to_string(path).unwrap()).expect("a main page");
        println!("{} {} | {}", m.welcome, m.tagline, m.count);
        for b in &m.boxes {
            println!("== {} [{}] {:?}", b.title, b.id, b.column);
            for block in &b.blocks {
                match block {
                    Block::Para(l) => println!("  p: {}", l.iter().map(|r| if r.link.is_some() { format!("[{}]", r.text) } else { r.text.clone() }).collect::<String>()),
                    Block::List { items, .. } => {
                        for (_, l) in items {
                            println!("  - {}", l.iter().map(|r| if r.link.is_some() { format!("[{}]", r.text) } else { r.text.clone() }).collect::<String>());
                        }
                    }
                    Block::Figure { image, caption } => println!("  fig {}x{} {}", image.width, image.height, caption.iter().map(|r| r.text.as_str()).collect::<String>()),
                    other => println!("  {other:?}"),
                }
            }
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
