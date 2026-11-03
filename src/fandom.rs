//! Fandom wikis, read in the app (opt-in, like Wikipedia): the wikis a
//! reader follows, each one's main page, search, random pages and
//! articles.
//!
//! Every Fandom wiki is a MediaWiki site with the standard API at
//! `https://<wiki>.fandom.com/api.php` (`/<lang>/api.php` for one in
//! another language). Pages come from `action=parse` and go through the
//! same parser as Wikipedia's articles (`crate::wikipedia`), which also
//! knows Fandom's infoboxes, galleries and tabs. Any other MediaWiki site
//! with an `api.php` (wiki.gg, say) works the same way when it's added
//! by its address.
//!
//! Like Wikipedia: a page is fetched only when the reader opens it, with
//! the app's User-Agent; nothing is crawled or fetched ahead. Every page
//! is shown with its wiki's name, its licence (most Fandom wikis' text is
//! CC BY-SA) and a link to it on the site. Fandom's name and logo are
//! trademarks: the app uses the name to say where pages come from, never
//! the logo.

use serde::{Deserialize, Serialize};

use crate::wikipedia::{self, Article, ParseSite, Summary};

/// A wiki someone follows (or is reading), as its API describes it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Wiki {
    /// "Resident Evil Wiki".
    pub name: String,
    /// `https://residentevil.fandom.com`.
    pub server: String,
    /// Empty, or `/es` for a wiki in another language.
    pub script_path: String,
    /// `/wiki/$1`, or `/es/wiki/$1`.
    pub article_path: String,
    pub main_page: String,
    /// The wiki's own logo (its name, drawn), if it has one.
    pub logo: Option<String>,
    /// "CC BY-SA" and where it's explained, as the wiki states them.
    pub license_name: String,
    pub license_url: String,
    /// The wiki's theme colour, for headings.
    pub accent: Option<[u8; 3]>,
}

impl PartialEq for Wiki {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Wiki {
    /// What makes a wiki itself: its address up to `api.php`.
    pub fn id(&self) -> String {
        format!("{}{}", self.server, self.script_path)
    }

    fn api(&self) -> String {
        format!("{}/api.php", self.id())
    }

    /// The address without `https://`, as the wiki is known:
    /// `residentevil.fandom.com/es`.
    pub fn address(&self) -> String {
        self.id().split_once("://").map_or(self.id(), |(_, a)| a.to_string())
    }

    /// Whether it's on Fandom (and not another MediaWiki site).
    pub fn on_fandom(&self) -> bool {
        host_of(&self.server).is_some_and(|h| h == "fandom.com" || h.ends_with(".fandom.com"))
    }

    /// A page's address on the site.
    pub fn page_url(&self, title: &str) -> String {
        let t = title.replace(' ', "_");
        let t: String = url::form_urlencoded::byte_serialize(t.as_bytes()).collect::<String>().replace("%2F", "/").replace("%3A", ":");
        format!("{}{}", self.server, self.article_path.replace("$1", &t))
    }

    fn parse_site(&self) -> ParseSite {
        ParseSite {
            server: self.server.clone(),
            article_prefix: self.article_path.split("$1").next().unwrap_or("/wiki/").to_string(),
            accent: self.accent,
        }
    }

    /// The licence's name, or CC BY-SA when the wiki doesn't say.
    pub fn license(&self) -> (String, String) {
        if self.license_name.is_empty() {
            ("CC BY-SA".into(), "https://www.fandom.com/licensing".into())
        } else {
            (self.license_name.clone(), self.license_url.clone())
        }
    }
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url).ok()?.host_str().map(str::to_ascii_lowercase)
}

/// API query parameters, escaped.
fn query(params: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new()).extend_pairs(params).finish()
}

async fn api_json(wiki: &Wiki, params: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let url = format!("{}?{}&format=json&formatversion=2", wiki.api(), query(params));
    let json: serde_json::Value = wikipedia::get(&url).await?.json().await.map_err(|e| e.to_string())?;
    if let Some(e) = json.get("error") {
        return Err(match e["code"].as_str() {
            Some("missingtitle") => fl!("fandom-no-page"),
            _ => e["info"].as_str().unwrap_or("error").to_string(),
        });
    }
    Ok(json)
}

// ── Finding a wiki ─────────────────────────────────────────────────────────

/// Where to look for a wiki someone typed: a name (`Resident Evil`), a
/// Fandom subdomain (`residentevil`) or any address on the site
/// (`attackontitan.fandom.com/es/wiki/Eren_Yeager`). Each is the address
/// of an `api.php` to try, in order.
pub fn candidates(input: &str) -> Vec<String> {
    let input = input.trim();
    if input.is_empty() {
        return Vec::new();
    }
    if input.contains('.') || input.contains("://") {
        let with_scheme = if input.contains("://") { input.to_string() } else { format!("https://{input}") };
        let Ok(url) = url::Url::parse(&with_scheme) else { return Vec::new() };
        let Some(host) = url.host_str() else { return Vec::new() };
        // Plain http only for a wiki on this computer (tools/mock-mediawiki.py).
        let local = matches!(host, "localhost" | "127.0.0.1");
        let scheme = if local && url.scheme() == "http" { "http" } else { "https" };
        let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
        let origin = format!("{scheme}://{}{port}", host.to_ascii_lowercase());
        // A language's wiki: `/es/wiki/…`, `/pt-br/…`.
        let lang = url
            .path_segments()
            .and_then(|mut s| s.next())
            .filter(|s| (2..=7).contains(&s.len()) && *s != "wiki" && s.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
            .map(|s| format!("/{s}"));
        let mut out = Vec::new();
        if let Some(lang) = &lang {
            out.push(format!("{origin}{lang}/api.php"));
        }
        out.push(format!("{origin}/api.php"));
        // Wikipedia-style sites keep it under /w/.
        out.push(format!("{origin}/w/api.php"));
        return out;
    }
    // A name: its letters and digits as a Fandom subdomain, then with
    // hyphens between the words. "Wiki" or "Fandom" at the end isn't part
    // of it.
    let lower = input.to_lowercase();
    let mut words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    while words.len() > 1 && matches!(words.last(), Some(&"wiki") | Some(&"fandom")) {
        words.pop();
    }
    let joined: String = words.concat();
    let hyphens = words.join("-");
    let mut out = vec![format!("https://{joined}.fandom.com/api.php")];
    if hyphens != joined {
        out.push(format!("https://{hyphens}.fandom.com/api.php"));
    }
    out.retain(|u| !u.starts_with("https://.fandom"));
    out
}

/// A wiki from its API's site information.
fn wiki_from_siteinfo(json: &serde_json::Value) -> Option<Wiki> {
    let g = &json["query"]["general"];
    let name = g["sitename"].as_str()?.to_string();
    let server = g["server"].as_str()?;
    let local = server.starts_with("http://localhost") || server.starts_with("http://127.0.0.1");
    let server = if server.starts_with("//") {
        format!("https:{server}")
    } else if local {
        server.to_string()
    } else {
        server.replacen("http://", "https://", 1)
    };
    let logo = g["logo"].as_str().filter(|l| !l.is_empty()).map(wikipedia::fix_url);
    let r = &json["query"]["rightsinfo"];
    Some(Wiki {
        name,
        server: server.trim_end_matches('/').to_string(),
        script_path: g["scriptpath"].as_str().unwrap_or_default().trim_end_matches('/').to_string(),
        article_path: g["articlepath"].as_str().unwrap_or("/wiki/$1").to_string(),
        main_page: g["mainpage"].as_str().unwrap_or("Main Page").to_string(),
        logo,
        license_name: r["text"].as_str().unwrap_or_default().to_string(),
        license_url: r["url"].as_str().unwrap_or_default().to_string(),
        accent: None,
    })
}

/// Finds the wiki someone typed (see [`candidates`]).
pub async fn find(input: &str) -> Result<Wiki, String> {
    for api in candidates(input) {
        let url = format!("{api}?action=query&meta=siteinfo&siprop=general%7Crightsinfo&format=json&formatversion=2");
        let Ok(resp) = wikipedia::client().get(&url).send().await else { continue };
        // A Fandom subdomain that isn't a wiki redirects to a page saying so.
        if !resp.status().is_success() || !resp.url().path().ends_with("/api.php") {
            continue;
        }
        let Ok(json) = resp.json::<serde_json::Value>().await else { continue };
        if let Some(mut wiki) = wiki_from_siteinfo(&json) {
            wiki.accent = accent(&wiki).await;
            return Ok(wiki);
        }
    }
    Err(fl!("fandom-not-found", name = input.trim()))
}

/// A Fandom wiki's theme colour, from the stylesheet variables its pages
/// load (`--theme-accent-color`). `None` off Fandom or when it can't be read.
async fn accent(wiki: &Wiki) -> Option<[u8; 3]> {
    if !wiki.on_fandom() {
        return None;
    }
    let url = format!("{}/wikia.php?controller=ThemeApi&method=themeVariables", wiki.id());
    let css = wikipedia::get(&url).await.ok()?.text().await.ok()?;
    css_color(&css, "--theme-accent-color")
}

/// `#rrggbb` (or `#rgb`) after `name:` in a stylesheet.
fn css_color(css: &str, name: &str) -> Option<[u8; 3]> {
    let at = css.find(&format!("{name}:"))? + name.len() + 1;
    let v = css[at..].trim_start().strip_prefix('#')?;
    let hex: String = v.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    let n = |a: &str| u8::from_str_radix(a, 16).ok();
    match hex.len() {
        6 | 8 => Some([n(&hex[0..2])?, n(&hex[2..4])?, n(&hex[4..6])?]),
        3 => {
            let c: Vec<String> = hex.chars().map(|c| format!("{c}{c}")).collect();
            Some([n(&c[0])?, n(&c[1])?, n(&c[2])?])
        }
        _ => None,
    }
}

// ── Pages ──────────────────────────────────────────────────────────────────

/// A page, by title (redirects followed: the article has the title it
/// ended up at).
pub async fn article(wiki: &Wiki, title: &str) -> Result<Article, String> {
    let json = api_json(
        wiki,
        &[
            ("action", "parse"),
            ("page", title),
            ("prop", "text"),
            ("redirects", "1"),
            ("disableeditsection", "1"),
            ("disabletoc", "1"),
            ("disablelimitreport", "1"),
        ],
    )
    .await?;
    Ok(parse_response(wiki, &json, title))
}

fn parse_response(wiki: &Wiki, json: &serde_json::Value, title: &str) -> Article {
    let p = &json["parse"];
    let title = p["title"].as_str().unwrap_or(title);
    // formatversion=2 gives the HTML as a string; 1 as {"*": …}.
    let html = p["text"].as_str().or_else(|| p["text"]["*"].as_str()).unwrap_or_default();
    wikipedia::parse_article_on(html, title, wiki.parse_site())
}

/// Pages matching `query`, with a line of each and its picture when the
/// wiki has them.
pub async fn search(wiki: &Wiki, q: &str) -> Result<Vec<Summary>, String> {
    let json = api_json(
        wiki,
        &[
            ("action", "query"),
            ("list", "search"),
            ("srsearch", q),
            ("srlimit", "20"),
            ("srprop", "snippet"),
            ("generator", "search"),
            ("gsrsearch", q),
            ("gsrlimit", "20"),
            ("prop", "pageimages"),
            ("piprop", "thumbnail"),
            ("pithumbsize", "160"),
        ],
    )
    .await?;
    Ok(search_results(&json))
}

fn search_results(json: &serde_json::Value) -> Vec<Summary> {
    let thumbs: std::collections::HashMap<&str, &str> = json["query"]["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p["title"].as_str()?, p["thumbnail"]["source"].as_str()?)))
        .collect();
    json["query"]["search"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let title = r["title"].as_str()?.to_string();
            Some(Summary {
                thumbnail: thumbs.get(title.as_str()).map(|t| wikipedia::fix_url(t)),
                extract: wikipedia::strip_tags(r["snippet"].as_str().unwrap_or_default()),
                description: String::new(),
                title,
            })
        })
        .collect()
}

/// A random article's title.
pub async fn random_title(wiki: &Wiki) -> Result<String, String> {
    let json = api_json(wiki, &[("action", "query"), ("list", "random"), ("rnnamespace", "0"), ("rnlimit", "1")]).await?;
    json["query"]["random"][0]["title"].as_str().map(str::to_string).ok_or_else(|| fl!("fandom-no-page"))
}

/// A link to a page on a Fandom wiki (`https://x.fandom.com/wiki/Title`):
/// the wiki's address and the page's title, to open it here.
pub fn page_link(url: &str) -> Option<(String, String)> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?.to_ascii_lowercase();
    if !host.ends_with(".fandom.com") {
        return None;
    }
    let path = u.path();
    let (before, title) = path.split_once("/wiki/")?;
    if title.is_empty() || title.contains(':') {
        return None;
    }
    let title = title.replace('+', "%2B");
    let title = url::form_urlencoded::parse(format!("t={title}").as_bytes()).next()?.1.replace('_', " ");
    Some((format!("https://{host}{before}"), title))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wikipedia::Block;

    fn wiki() -> Wiki {
        Wiki {
            name: "Attack on Titan Wiki".into(),
            server: "https://attackontitan.fandom.com".into(),
            script_path: String::new(),
            article_path: "/wiki/$1".into(),
            main_page: "Attack on Titan Wiki".into(),
            accent: Some([0x80, 0x00, 0x00]),
            ..Default::default()
        }
    }

    #[test]
    fn a_wiki_is_found_by_name_subdomain_or_address() {
        assert_eq!(candidates("Resident Evil"), ["https://residentevil.fandom.com/api.php", "https://resident-evil.fandom.com/api.php"]);
        assert_eq!(candidates("attackontitan"), ["https://attackontitan.fandom.com/api.php"]);
        assert_eq!(candidates("Star Wars Wiki")[0], "https://starwars.fandom.com/api.php");
        assert_eq!(
            candidates("https://attackontitan.fandom.com/es/wiki/Eren_Jaeger"),
            [
                "https://attackontitan.fandom.com/es/api.php",
                "https://attackontitan.fandom.com/api.php",
                "https://attackontitan.fandom.com/w/api.php"
            ]
        );
        assert_eq!(candidates("terraria.wiki.gg")[0], "https://terraria.wiki.gg/api.php");
        assert!(candidates("  ").is_empty());
        assert!(candidates("!!!").is_empty());
    }

    #[test]
    fn site_information_makes_a_wiki() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"query":{"general":{"mainpage":"Resident Evil Wiki","sitename":"Resident Evil Wiki","logo":"https://static.wikia.nocookie.net/residentevil/images/e/e6/Site-logo.png/revision/latest?cb=1",
            "server":"https://residentevil.fandom.com","scriptpath":"/es","articlepath":"/es/wiki/$1","lang":"es"},
            "rightsinfo":{"url":"https://www.fandom.com/licensing","text":"CC-BY-SA"}}}"#,
        )
        .unwrap();
        let w = wiki_from_siteinfo(&json).unwrap();
        assert_eq!(w.name, "Resident Evil Wiki");
        assert_eq!(w.id(), "https://residentevil.fandom.com/es");
        assert_eq!(w.address(), "residentevil.fandom.com/es");
        assert_eq!(w.api(), "https://residentevil.fandom.com/es/api.php");
        assert_eq!(w.page_url("Leon S. Kennedy"), "https://residentevil.fandom.com/es/wiki/Leon_S._Kennedy");
        assert_eq!(w.license(), ("CC-BY-SA".to_string(), "https://www.fandom.com/licensing".to_string()));
        assert!(w.on_fandom());
        assert_eq!(w.parse_site().article_prefix, "/es/wiki/");
    }

    #[test]
    fn theme_colours_and_links_to_other_wikis() {
        assert_eq!(css_color(":root{--theme-page-background-color:#fff;--theme-accent-color:#C00A0A;}", "--theme-accent-color"), Some([0xc0, 0x0a, 0x0a]));
        assert_eq!(css_color("--theme-accent-color: #abc", "--theme-accent-color"), Some([0xaa, 0xbb, 0xcc]));
        assert_eq!(css_color("nothing", "--theme-accent-color"), None);
        assert_eq!(
            page_link("https://residentevil.fandom.com/wiki/Leon_S._Kennedy#Biography"),
            Some(("https://residentevil.fandom.com".into(), "Leon S. Kennedy".into()))
        );
        assert_eq!(page_link("https://starwars.fandom.com/es/wiki/Luke_Skywalker").map(|l| l.0), Some("https://starwars.fandom.com/es".into()));
        assert_eq!(page_link("https://starwars.fandom.com/wiki/Category:Jedi"), None);
        assert_eq!(page_link("https://example.com/wiki/X"), None);
    }

    /// Trimmed from a character page as Fandom's action=parse serves it:
    /// a portable infobox with tabbed pictures, a lazily loaded thumbnail,
    /// a tabber, a gallery and links in the site's own form.
    const EREN: &str = r##"<div class="mw-parser-output"><aside role="region" class="portable-infobox pi-background pi-border-color pi-theme-character pi-layout-default">
<h2 class="pi-item pi-item-spacing pi-title pi-secondary-background" data-source="name">Eren Yeager</h2>
<div class="pi-media-collection pi-image-collection wds-tabber">
<div class="wds-tabs__wrapper"><ul class="wds-tabs"><li class="wds-tabs__tab wds-is-current" data-pi-image-collection-tab-key="Anime"><div class="wds-tabs__tab-label">Anime</div></li><li class="wds-tabs__tab"><div class="wds-tabs__tab-label">Manga</div></li></ul></div>
<div class="wds-tab__content wds-is-current"><figure class="pi-item pi-image" data-source="image"><a href="https://static.wikia.nocookie.net/shingekinokyojin/images/a/a1/Eren_anime.png/revision/latest?cb=1" class="image image-thumbnail" title=""><img src="data:image/gif;base64,R0lGODlhAQABAIABAAAAAP///yH5BAEAAAEALAAAAAABAAEAQAICTAEAOw%3D%3D" data-src="https://static.wikia.nocookie.net/shingekinokyojin/images/a/a1/Eren_anime.png/revision/latest/scale-to-width-down/268?cb=1" width="268" height="350" class="pi-image-thumbnail lazyload" /></a></figure></div>
<div class="wds-tab__content"><figure class="pi-item pi-image" data-source="image2"><a href="x" class="image image-thumbnail"><img src="https://static.wikia.nocookie.net/shingekinokyojin/images/b/b2/Eren_manga.png/revision/latest/scale-to-width-down/268?cb=2" width="268" height="380" class="pi-image-thumbnail" /></a></figure></div>
</div>
<section class="pi-item pi-group pi-border-color"><h2 class="pi-item pi-header pi-secondary-font pi-item-spacing pi-secondary-background">Character</h2>
<div class="pi-item pi-data pi-item-spacing pi-border-color" data-source="species"><h3 class="pi-data-label pi-secondary-font">Species</h3><div class="pi-data-value pi-font"><a href="/wiki/Human" title="Human">Human</a><br />Titan Shifter</div></div>
<div class="pi-item pi-data pi-item-spacing pi-border-color" data-source="relatives"><h3 class="pi-data-label pi-secondary-font">Relatives</h3><div class="pi-data-value pi-font"><a href="/wiki/Grisha_Yeager" title="Grisha Yeager">Grisha Yeager</a> (Father)</div></div>
<div class="pi-item pi-data pi-item-spacing pi-border-color" data-source="empty"><h3 class="pi-data-label pi-secondary-font">Empty</h3><div class="pi-data-value pi-font"></div></div>
</section>
<table class="pi-horizontal-group"><caption class="pi-header">Debut</caption><thead><tr><th class="pi-horizontal-group-item pi-data-label">Manga</th><th class="pi-horizontal-group-item pi-data-label">Anime</th></tr></thead><tbody><tr><td class="pi-horizontal-group-item pi-data-value">Chapter 1</td><td class="pi-horizontal-group-item pi-data-value">Episode 1</td></tr></tbody></table>
</aside>
<p><b>Eren Yeager</b> is the main protagonist of <i><a href="/wiki/Attack_on_Titan_(Manga)" title="Attack on Titan (Manga)">Attack on Titan</a></i>, who joined the <a href="https://attackontitan.fandom.com/wiki/Survey_Corps" title="Survey Corps">Survey Corps</a>.<sup id="cite_ref-1" class="reference"><a href="#cite_note-1">[1]</a></sup> His <a href="/wiki/Rumbling?action=edit&amp;redlink=1" class="new" title="Rumbling (page does not exist)">Rumbling</a> began on <a href="https://other.fandom.com/wiki/Day">a day</a>.</p>
<h2><span class="mw-headline" id="Appearance">Appearance</span></h2>
<figure class="thumb tright show-info-icon" style="width: 182px"><a href="https://static.wikia.nocookie.net/shingekinokyojin/images/c/c3/Eren_child.png/revision/latest?cb=3" class="image"><img alt="Eren child" src="data:image/gif;base64,R0lGODlhAQABAIABAAAAAP" decoding="async" loading="lazy" width="180" height="240" data-image-name="Eren child.png" data-src="https://static.wikia.nocookie.net/shingekinokyojin/images/c/c3/Eren_child.png/revision/latest/scale-to-width-down/180?cb=3" class="thumbimage lazyload" /></a><figcaption class="thumbcaption"><p class="caption">Eren as a child</p></figcaption></figure>
<div class="tabber wds-tabber"><div class="wds-tabs__wrapper with-bottom-border"><ul class="wds-tabs"><li class="wds-tabs__tab wds-is-current" data-hash="Anime"><div class="wds-tabs__tab-label"><a href="#">Anime</a></div></li><li class="wds-tabs__tab" data-hash="Manga"><div class="wds-tabs__tab-label"><a href="#">Manga</a></div></li></ul></div>
<div class="wds-tab__content wds-is-current"><p>Green eyes in the anime.</p></div><div class="wds-tab__content"><p>Grey eyes in the manga.</p></div></div>
<h3><span class="mw-headline" id="Gallery">Gallery</span></h3>
<div id="gallery-0" class="wikia-gallery wikia-gallery-caption-below wikia-gallery-position-center"><div class="wikia-gallery-row"><span class="wikia-gallery-item"><div class="thumb"><div class="gallery-image-wrapper accent"><a class="image lightbox" href="https://static.wikia.nocookie.net/x/images/1.png/revision/latest?cb=4"><img data-src="https://static.wikia.nocookie.net/x/images/1.png/revision/latest/scale-to-width-down/185?cb=4" src="data:image/gif;base64,R0" width="185" height="104" class="thumbimage lazyload" /></a></div></div><div class="lightbox-caption">Eren transforms</div></span>
<span class="wikia-gallery-item"><div class="thumb"><div class="gallery-image-wrapper accent"><a class="image lightbox" href="https://static.wikia.nocookie.net/x/images/2.png/revision/latest?cb=5"><img src="https://static.wikia.nocookie.net/x/images/2.png/revision/latest/scale-to-width-down/185?cb=5" width="185" height="104" /></a></div></div><div class="lightbox-caption">Eren fights</div></span></div></div>
<p><a href="/wiki/Characters" title="Characters"><img src="https://static.wikia.nocookie.net/x/images/c.png/revision/latest/scale-to-width-down/150?cb=6" width="150" height="150" /></a> <a href="/wiki/Episodes" title="Episodes"><img src="https://static.wikia.nocookie.net/x/images/e.png/revision/latest/scale-to-width-down/150?cb=7" width="150" height="150" /></a></p>
<table class="navbox"><tr><td>Lots of links</td></tr></table>
</div>"##;

    fn text(l: &wikipedia::Line) -> String {
        l.iter().map(|r| r.text.as_str()).collect()
    }

    #[test]
    fn reads_a_fandom_page_like_the_site() {
        let json = serde_json::json!({"parse": {"title": "Eren Yeager", "pageid": 1, "text": EREN}});
        let a = parse_response(&wiki(), &json, "Eren_Yeager");
        assert_eq!(a.title, "Eren Yeager");
        let titles: Vec<(u8, &str)> = a.sections.iter().map(|s| (s.level, s.title.as_str())).collect();
        assert_eq!(titles, [(1, ""), (2, "Appearance"), (3, "Gallery")]);

        // The infobox: title, tabbed pictures, group heading, label/value rows,
        // the horizontal group; the empty item left out.
        let ib = a.infobox.as_ref().expect("infobox");
        let c0 = &ib.rows[0].cells[0];
        assert!(c0.header && c0.colspan == 2);
        // The wiki's colour, lightened.
        assert_eq!(c0.background, Some([(0x80 as f32 * 0.35 + 255.0 * 0.65) as u8, 165, 165]));
        let Block::Tabs { tabs, .. } = &ib.rows[1].cells[0].content[0] else { panic!("{:?}", ib.rows[1]) };
        assert_eq!(tabs.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["Anime", "Manga"]);
        let Block::Figure { image, .. } = &tabs[0].1[0] else { panic!() };
        assert!(image.url.ends_with("Eren_anime.png/revision/latest/scale-to-width-down/268?cb=1"), "{}", image.url);
        assert_eq!(wikipedia::full_size(&image.url), "https://static.wikia.nocookie.net/shingekinokyojin/images/a/a1/Eren_anime.png/revision/latest?cb=1");
        let Block::Para(h) = &ib.rows[2].cells[0].content[0] else { panic!() };
        assert_eq!(text(h), "Character");
        let species = &ib.rows[3].cells;
        assert_eq!(species.len(), 2);
        assert!(species[0].header);
        let Block::Para(v) = &species[1].content[0] else { panic!() };
        assert_eq!(text(v), "Human\nTitan Shifter");
        assert_eq!(v[0].link.as_deref(), Some("wiki:Human"));
        // Title, pictures, "Character", Species, Relatives, then "Debut"
        // with its labels and values side by side.
        assert_eq!(ib.rows.len(), 8, "{:#?}", ib.rows.iter().map(|r| r.cells.len()).collect::<Vec<_>>());
        let Block::Para(d) = &ib.rows[5].cells[0].content[0] else { panic!() };
        assert_eq!(text(d), "Debut");
        assert_eq!(ib.rows[6].cells.len(), 2);
        assert!(ib.rows[6].cells[0].header && !ib.rows[7].cells[0].header);

        // The lead: links to this wiki (relative or full) are articles; red
        // links and other wikis aren't; the footnote mark is small.
        let Block::Para(p) = &a.sections[0].blocks[0] else { panic!("{:?}", a.sections[0].blocks) };
        assert_eq!(text(p), "Eren Yeager is the main protagonist of Attack on Titan, who joined the Survey Corps.[1] His Rumbling began on a day.");
        let links: Vec<Option<&str>> = p.iter().map(|r| r.link.as_deref()).collect();
        assert!(links.contains(&Some("wiki:Attack on Titan (Manga)")));
        assert!(links.contains(&Some("wiki:Survey Corps")));
        assert!(links.contains(&Some("https://other.fandom.com/wiki/Day")));
        assert!(!links.iter().flatten().any(|l| l.contains("Rumbling")));
        assert!(p.iter().any(|r| r.reference && r.text == "[1]"));

        // A lazily loaded thumbnail, then tabs.
        let blocks = &a.sections[1].blocks;
        let Block::Figure { image, caption } = &blocks[0] else { panic!("{blocks:?}") };
        assert!(image.url.contains("Eren_child.png"));
        assert_eq!(text(caption), "Eren as a child");
        let Block::Tabs { tabs, .. } = &blocks[1] else { panic!("{blocks:?}") };
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[1].0, "Manga");
        let Block::Para(m) = &tabs[1].1[0] else { panic!() };
        assert_eq!(text(m), "Grey eyes in the manga.");

        // The gallery with captions, then two pictures that are links; the
        // navbox is gone.
        let blocks = &a.sections[2].blocks;
        let Block::Gallery(g) = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(g.iter().map(|i| text(&i.caption)).collect::<Vec<_>>(), ["Eren transforms", "Eren fights"]);
        assert!(g.iter().all(|i| i.link.is_none()));
        let Block::Gallery(nav) = &blocks[1] else { panic!("{blocks:?}") };
        assert_eq!(nav.iter().map(|i| i.link.as_deref()).collect::<Vec<_>>(), [Some("wiki:Characters"), Some("wiki:Episodes")]);
        assert_eq!(blocks.len(), 2);

        // The viewer steps through the pictures that aren't links.
        assert_eq!(a.gallery().len(), 2 + 1 + 2);
        assert_eq!(a.link_pictures().len(), 2);
    }

    #[test]
    fn search_results_get_their_pictures() {
        let json = serde_json::json!({"query": {
            "search": [{"title": "Leon S. Kennedy", "snippet": "<span class=\"searchmatch\">Leon</span> is a &quot;hero&quot;"}, {"title": "Leon (disambiguation)", "snippet": ""}],
            "pages": [{"pageid": 2, "title": "Leon S. Kennedy", "thumbnail": {"source": "https://static.wikia.nocookie.net/re/images/l.png/revision/latest/scale-to-width-down/160?cb=1"}}]
        }});
        let r = search_results(&json);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].extract, "Leon is a \"hero\"");
        assert!(r[0].thumbnail.is_some());
        assert!(r[1].thumbnail.is_none());
    }

    /// NEBULA_FANDOM_LIVE=1 (or =<wiki name>): the real API.
    #[test]
    fn live_api() {
        let Ok(name) = std::env::var("NEBULA_FANDOM_LIVE") else { return };
        let name = if name == "1" { "attackontitan".to_string() } else { name };
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let w = find(&name).await.unwrap();
            println!("{w:?}");
            let main = article(&w, &w.main_page).await.unwrap();
            println!("main page: {} sections, {} pictures", main.sections.len(), main.gallery().len());
            let r = search(&w, "the").await.unwrap();
            println!("search: {:?}", r.iter().take(3).map(|s| (&s.title, s.thumbnail.is_some())).collect::<Vec<_>>());
            let t = random_title(&w).await.unwrap();
            let a = article(&w, &t).await.unwrap();
            println!("random {t}: {} sections, infobox {:?} rows, {} pictures", a.sections.len(), a.infobox.as_ref().map(|i| i.rows.len()), a.gallery().len());
        });
    }

    /// NEBULA_FANDOM_TEST=page.json: what the parser makes of a saved
    /// action=parse answer.
    #[test]
    fn real_page() {
        let Ok(path) = std::env::var("NEBULA_FANDOM_TEST") else { return };
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let a = parse_response(&Wiki { server: "https://x.fandom.com".into(), article_path: "/wiki/$1".into(), ..Default::default() }, &json, "Test");
        println!("infobox rows: {:?}", a.infobox.as_ref().map(|t| t.rows.len()));
        for s in &a.sections {
            println!("{}{} {:?}", "  ".repeat(s.level as usize - 1), if s.title.is_empty() { "(lead)" } else { &s.title }, s.blocks.iter().map(|b| match b {
                Block::Para(_) => "p", Block::Hatnote(_) => "hat", Block::Quote(_) => "q", Block::List { .. } => "list", Block::Figure { .. } => "fig",
                Block::Table(_) => "table", Block::Bar(_) => "bar", Block::Gallery(_) => "gallery", Block::Tabs { .. } => "tabs",
            }).collect::<Vec<_>>());
        }
    }
}
