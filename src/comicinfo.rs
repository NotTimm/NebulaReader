//! Parses the `ComicInfo.xml` sidecar many scanned/scraped .cbz/.cbr files
//! embed (the ComicRack/ComicTagger standard). When present it's free,
//! offline, and authoritative — Jellyfin and other comic readers prefer it
//! over any online lookup, so we do too.

/// The subset of ComicInfo.xml fields we care about.
#[derive(Debug, Default, Clone)]
pub struct ComicInfo {
    pub series: Option<String>,
    pub number: Option<String>,
    pub volume: Option<String>,
    pub year: Option<i32>,
    pub summary: Option<String>,
    pub publisher: Option<String>,
    pub writer: Option<String>,
    pub genre: Option<String>,
    pub month: Option<u32>,
    pub day: Option<u32>,
    pub penciller: Option<String>,
    pub inker: Option<String>,
    pub colorist: Option<String>,
    pub letterer: Option<String>,
    pub cover_artist: Option<String>,
}

impl ComicInfo {
    pub fn is_empty(&self) -> bool {
        self.series.is_none()
            && self.number.is_none()
            && self.volume.is_none()
            && self.year.is_none()
            && self.summary.is_none()
            && self.publisher.is_none()
            && self.writer.is_none()
            && self.genre.is_none()
            && self.penciller.is_none()
    }

    /// "2021-03-14", "2021-03" or "2021"; empty when there's no year.
    pub fn release_date(&self) -> String {
        match (self.year.filter(|y| *y > 1800), self.month.filter(|m| (1..=12).contains(m)), self.day) {
            (Some(y), Some(m), Some(d)) if (1..=31).contains(&d) => format!("{y:04}-{m:02}-{d:02}"),
            (Some(y), Some(m), _) => format!("{y:04}-{m:02}"),
            (Some(y), None, _) => format!("{y:04}"),
            _ => String::new(),
        }
    }

    /// Who made it, as (role, names) pairs in the usual credits order.
    pub fn credits(&self) -> Vec<(String, String)> {
        [
            ("Writer", &self.writer),
            ("Artist", &self.penciller),
            ("Inker", &self.inker),
            ("Colorist", &self.colorist),
            ("Letterer", &self.letterer),
            ("Cover", &self.cover_artist),
        ]
        .into_iter()
        .filter_map(|(role, names)| Some((role.to_string(), names.clone()?)))
        .collect()
    }
}

pub fn parse(xml: &str) -> ComicInfo {
    ComicInfo {
        series: tag(xml, "Series"),
        number: tag(xml, "Number"),
        volume: tag(xml, "Volume"),
        year: tag(xml, "Year").and_then(|s| s.parse().ok()),
        summary: tag(xml, "Summary"),
        publisher: tag(xml, "Publisher"),
        writer: tag(xml, "Writer"),
        genre: tag(xml, "Genre"),
        month: tag(xml, "Month").and_then(|s| s.parse().ok()),
        day: tag(xml, "Day").and_then(|s| s.parse().ok()),
        penciller: tag(xml, "Penciller"),
        inker: tag(xml, "Inker"),
        colorist: tag(xml, "Colorist"),
        letterer: tag(xml, "Letterer"),
        cover_artist: tag(xml, "CoverArtist"),
    }
}

/// Extracts the text content of the first `<name>...</name>` element,
/// unescaping basic XML entities and unwrapping a CDATA section if present.
/// ComicInfo.xml is a flat schema (no nested nor attributed value tags), so
/// this simple approach avoids pulling in a full XML parser dependency.
fn tag(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let mut raw = xml[start..end].trim();

    if let Some(inner) = raw.strip_prefix("<![CDATA[").and_then(|s| s.strip_suffix("]]>")) {
        raw = inner.trim();
    }

    if raw.is_empty() {
        return None;
    }
    Some(unescape(raw))
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}
