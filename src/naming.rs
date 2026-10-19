//! Offline name cleanup: turns scene/scraper-style folder and file names
//! into something presentable *before* any online lookup, and parses issue
//! / chapter numbers out of filenames so issues can be linked to online
//! metadata by number.
//!
//! e.g. "Civil War 001-007 (2006-2007) GetComics.INFO" -> "Civil War" (2006)
//!      "Crossed v2 Family Values (001-007) (2010-2011)" -> "Crossed Family Values" v2
//!      "Naruto (Color) 01J76XYBK4HFZMAWWKX52WN8PK" -> "Naruto (Color)"

use regex::Regex;
use std::sync::OnceLock;

/// A cleaned-up series name plus whatever extra hints the raw name carried.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CleanName {
    /// What to search online with — no edition tags, no years.
    pub search: String,
    /// What to show when there's no (confident) online match. Same as
    /// `search` plus a meaningful edition tag like "(Color)".
    pub display: String,
    pub year: Option<u32>,
    pub volume: Option<u32>,
}

/// An issue/chapter number (normalized: "001" -> "1", "1.50" -> "1.5") and
/// any subtitle that followed it, e.g. "#4 - Act 2" -> ("4", "Act 2").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IssueName {
    pub number: Option<String>,
    pub subtitle: Option<String>,
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

/// Strips a file extension if it's one we open.
fn strip_ext(raw: &str) -> &str {
    let lower = raw.to_ascii_lowercase();
    for ext in [".cbz", ".cbr", ".zip", ".rar", ".epub", ".pdf"] {
        if lower.ends_with(ext) {
            return &raw[..raw.len() - ext.len()];
        }
    }
    raw
}

/// ULID or UUID tokens (whole or cut short) that manga-tui etc. append.
fn is_id_token(s: &str) -> bool {
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    // ULID: uppercase base32, 26 chars (10+ when a name was truncated).
    if s.len() >= 10 && has_digit && s.chars().all(|c| c.is_ascii_digit() || c.is_ascii_uppercase()) {
        return true;
    }
    // UUID, or a long hex run from one.
    let hex_parts: Vec<&str> = s.split('-').collect();
    if hex_parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_hexdigit())) {
        let hex_len: usize = hex_parts.iter().map(|p| p.len()).sum();
        return (hex_parts.len() == 5 && hex_len == 32) || (hex_len >= 8 && has_digit);
    }
    false
}

/// Drops a scanlation group from the end of a chapter title. Groups are only
/// recognisable by a telltale last word ("TCB Scans", "Binktopia scan",
/// "Weeb Central"), which goes along with the word naming the group.
fn strip_scan_group(title: &str) -> String {
    const GROUP_WORDS: [&str; 10] =
        ["scan", "scans", "scanlation", "scanlations", "translations", "tl", "central", "team", "subs", "fansub"];
    let words: Vec<&str> = title.split_whitespace().collect();
    match words.iter().rposition(|w| GROUP_WORDS.contains(&w.to_ascii_lowercase().as_str())) {
        // "<group> scan" — drop both; whatever came before is the title.
        Some(i) => words[..i.saturating_sub(1)].join(" "),
        None => words.join(" "),
    }
}

/// Splits out `(...)`/`[...]`/`{...}` groups, returning the remaining text
/// and the group contents.
fn split_groups(s: &str) -> (String, Vec<String>) {
    let mut rest = String::with_capacity(s.len());
    let mut groups = Vec::new();
    let mut depth = 0u32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => {
                if depth == 0 {
                    cur.clear();
                }
                depth += 1;
            }
            ')' | ']' | '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    groups.push(cur.trim().to_string());
                    rest.push(' ');
                }
            }
            _ if depth > 0 => cur.push(c),
            _ => rest.push(c),
        }
    }
    (rest, groups)
}

/// First 4-digit year (1900–2100) in a group like "2006" or "2006-2007".
fn year_in(s: &str) -> Option<u32> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let caps = re(&RE, r"^\s*((?:19|20)\d\d)(?:\s*-\s*(?:(?:19|20)\d\d|present|\d\d))?\s*$").captures(s)?;
    caps[1].parse().ok()
}

/// Release-group / site / format noise that should never be part of a name.
fn is_junk_token(tok: &str) -> bool {
    let lower = tok.to_ascii_lowercase();
    let lower = lower.trim_matches(|c: char| !c.is_alphanumeric());
    if matches!(
        lower,
        "getcomics" | "getcomicsinfo" | "digital" | "webrip" | "c2c" | "tpb" | "hc" | "omnibus"
            | "weeb" | "central"
    ) {
        return true;
    }
    // Domain-ish tokens: "GetComics.INFO", "site.com"
    static RE: OnceLock<Regex> = OnceLock::new();
    re(&RE, r"(?i)^[a-z0-9-]+\.(info|com|net|org|to|cc|xyz|io)$").is_match(tok)
}

/// Edition tags worth keeping in the *displayed* name, since they tell two
/// otherwise identically named folders apart.
fn edition_tag(group: &str) -> Option<&'static str> {
    let g = group.to_ascii_lowercase();
    if g.contains("colored") || g.contains("colour") || g == "color" || g.contains("color)") {
        Some("Color")
    } else if g.contains("color") {
        Some("Color")
    } else if g == "fan-made" || g == "fan made" || g.contains("fan made") || g.contains("fan-made") {
        Some("Fan Edit")
    } else {
        None
    }
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cleans a series folder name (or a loose file's name, with its issue
/// number stripped) into a search/display name.
pub fn clean_series_name(raw: &str) -> CleanName {
    let base = strip_ext(raw.trim());
    // manga-tui and friends turn ':' into '_' in folder names.
    let base = base.replace("_ ", ": ").replace('_', " ");

    let (rest, groups) = split_groups(&base);

    let year = groups.iter().find_map(|g| year_in(g));
    let edition = groups.iter().find_map(|g| edition_tag(g));

    let mut volume = None;
    let mut tokens: Vec<String> = Vec::new();
    static VOL: OnceLock<Regex> = OnceLock::new();
    let vol_re = re(&VOL, r"(?i)^v(?:ol(?:ume)?)?\.?(\d{1,3})$");
    static RANGE: OnceLock<Regex> = OnceLock::new();
    let range_re = re(&RANGE, r"^#?\d{1,4}\s*-\s*#?\d{1,4}$");

    let words: Vec<&str> = rest.split_whitespace().collect();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        if is_id_token(w) || is_junk_token(w) || range_re.is_match(w) {
            i += 1;
            continue;
        }
        if let Some(c) = vol_re.captures(w) {
            volume = c[1].parse().ok();
            i += 1;
            continue;
        }
        // "Vol. 01" / "Volume 2" as two tokens
        let lw = w.to_ascii_lowercase();
        if matches!(lw.as_str(), "vol" | "vol." | "volume" | "v") {
            let next = words.get(i + 1).map(|n| n.trim_end_matches(['-', ':', ',']));
            if let Some(n) = next.and_then(|n| n.parse::<u32>().ok()) {
                volume = Some(n);
                i += 2;
                continue;
            }
            // A dangling "Vol." (its number was cut off as an issue number).
            if i + 1 == words.len() && lw != "v" {
                i += 1;
                continue;
            }
        }
        tokens.push(w.to_string());
        i += 1;
    }

    // Trailing "-" / ":" left behind by stripped tokens.
    let mut search = collapse_ws(&tokens.join(" "));
    while let Some(stripped) = search
        .strip_suffix(" -")
        .or_else(|| search.strip_suffix('-'))
        .or_else(|| search.strip_suffix(':'))
        .or_else(|| search.strip_suffix(','))
    {
        search = stripped.trim_end().to_string();
    }
    if search.is_empty() {
        search = collapse_ws(&base);
    }

    let display = match edition {
        Some(tag) if !search.to_ascii_lowercase().contains(&tag.to_ascii_lowercase()) => {
            format!("{search} ({tag})")
        }
        _ => search.clone(),
    };

    CleanName { search, display, year, volume }
}

/// Carries an edition tag from the local name (e.g. "Naruto (Color)") over
/// to an online title that lacks it, so colored/fan editions stay distinct
/// from the regular release on the shelf.
pub fn with_edition(online: &str, local: &str) -> String {
    for tag in ["Color", "Fan Edit"] {
        let suffix = format!(" ({tag})");
        if local.ends_with(&suffix) && !online.to_lowercase().contains(&tag.to_lowercase()) {
            return format!("{online}{suffix}");
        }
    }
    online.to_string()
}

/// Normalizes an issue number for comparison: "001" -> "1", "0" -> "0",
/// "1.50" -> "1.5", "12AU" -> "12au".
pub fn normalize_number(n: &str) -> String {
    let n = n.trim().trim_start_matches('#').to_ascii_lowercase();
    let (int_part, rest) = match n.find(|c: char| !c.is_ascii_digit()) {
        Some(i) => n.split_at(i),
        None => (n.as_str(), ""),
    };
    let int_part = int_part.trim_start_matches('0');
    let int_part = if int_part.is_empty() && !n.is_empty() && n.starts_with('0') { "0" } else { int_part };
    let mut rest = rest.to_string();
    if rest.starts_with('.') {
        rest = rest.trim_end_matches('0').to_string();
        if rest == "." {
            rest.clear();
        }
    }
    format!("{int_part}{rest}")
}

/// Parses an issue/chapter number out of a file or folder name.
pub fn parse_issue(raw: &str) -> IssueName {
    let base = strip_ext(raw.trim()).replace('_', " ");
    let (rest, groups) = split_groups(&base);
    let words: Vec<&str> = rest.split_whitespace().filter(|w| !is_id_token(w)).collect();
    let rest = words.join(" ");

    fn subtitle_after(s: &str) -> Option<String> {
        let s = s.trim().trim_start_matches(['-', ':', '–']).trim();
        (!s.is_empty()).then(|| s.to_string())
    }

    // 1. "#12" anywhere — the most explicit marker.
    static HASH: OnceLock<Regex> = OnceLock::new();
    if let Some(c) = re(&HASH, r"#\s*(\d+(?:\.\d+)?[a-zA-Z]?)\b(.*)$").captures(&rest) {
        return IssueName { number: Some(normalize_number(&c[1])), subtitle: subtitle_after(&c[2]) };
    }

    // 2. "Ch 12", "Chapter 12", "Ch.12", "c012", "Issue 3"
    static CH: OnceLock<Regex> = OnceLock::new();
    if let Some(c) =
        re(&CH, r"(?i)\b(?:ch(?:apter)?|chap|issue|ep(?:isode)?)\.?\s*(\d+(?:\.\d+)?)\b(.*)$").captures(&rest)
    {
        // manga-tui: "Ch 2 Vol 1 Dog And Chainsaw Phantom troop" — drop
        // the "Vol N" part; what's left is the chapter title (plus the
        // scanlation group, which can't be told apart reliably).
        static VOLP: OnceLock<Regex> = OnceLock::new();
        let tail = re(&VOLP, r"(?i)^\s*vol(?:ume)?\.?\s*(?:\d+|none)\b").replace(c[2].trim(), "");
        let subtitle = subtitle_after(&strip_scan_group(&tail));
        return IssueName { number: Some(normalize_number(&c[1])), subtitle };
    }

    // 3. The last bare number in the name (not a year, not "of 07").
    static NUM: OnceLock<Regex> = OnceLock::new();
    let num_re = re(&NUM, r"\b(\d{1,4}(?:\.\d+)?)\b");
    let mut best: Option<(usize, String)> = None;
    for m in num_re.find_iter(&rest) {
        let text = m.as_str();
        let before = rest[..m.start()].trim_end().to_ascii_lowercase();
        if before.ends_with(" of") || before.ends_with("vol") || before.ends_with("v") {
            continue;
        }
        if text.len() == 4 && year_in(text).is_some() {
            continue;
        }
        best = Some((m.end(), text.to_string()));
    }
    if let Some((end, n)) = best {
        return IssueName { number: Some(normalize_number(&n)), subtitle: subtitle_after(&rest[end..]) };
    }

    // 4. A lone number in a group: "Annual (2)" is rare; ignore groups otherwise.
    let _ = groups;
    IssueName::default()
}

/// Folders that only wrap a series' chapters (manga-tui puts them in
/// `Series/English/`), so a file's title should come from the folder above.
fn is_wrapper_dir(name: &str) -> bool {
    const WRAPPERS: [&str; 12] = [
        "english", "en", "eng", "japanese", "jp", "ja", "raw", "raws", "chapters", "volumes", "manga", "comics",
    ];
    WRAPPERS.contains(&name.trim().to_ascii_lowercase().as_str())
}

/// The title for a single comic file, as a library entry or when opened on
/// its own. Usually the cleaned file name; but a bare chapter file
/// ("Ch 0 Vol none Oneshot Binktopia scan <uuid>.cbz") says nothing about
/// its series, so its folder names it instead (skipping wrappers like
/// `English`, and never a library root, which `is_root` identifies). With
/// no usable folder it's "Chapter 0 · Oneshot".
pub fn title_for_file(path: &std::path::Path, is_root: impl Fn(&std::path::Path) -> bool) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let issue = parse_issue(&name);
    let series = series_name_from_issue(&name).display;
    let chapter_only = issue.number.is_some() && (series.is_empty() || starts_with_chapter(&name));
    if !chapter_only {
        let clean = clean_series_name(&name).display;
        return if clean.is_empty() { strip_ext(&name).to_string() } else { clean };
    }
    let mut dir = path.parent();
    while let Some(d) = dir {
        if is_root(d) {
            break;
        }
        let dname = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if dname.is_empty() {
            break;
        }
        if !is_wrapper_dir(&dname) {
            let clean = clean_series_name(&dname).display;
            if !clean.is_empty() {
                return clean;
            }
        }
        dir = d.parent();
    }
    chapter_title(&issue)
}

/// "Chapter 0 · Oneshot", or just "Chapter 12".
/// The volume a chapter file says it's in: "Ch 12 Vol 2 …",
/// "Berserk Vol. 03 Ch 21", "[Volume 4] …". `None` when there isn't one
/// (or it's manga-tui's "Vol none").
pub fn volume_of(name: &str) -> Option<u32> {
    static VOL: OnceLock<Regex> = OnceLock::new();
    let caps = re(&VOL, r"(?i)(?:^|[\s_\-\[(.])vol(?:ume)?\.?\s*(\d{1,3})(?:\b|_)").captures(name)?;
    caps[1].parse().ok()
}

pub fn chapter_title(issue: &IssueName) -> String {
    let number = issue.number.as_deref().unwrap_or("?");
    match &issue.subtitle {
        Some(sub) => format!("Chapter {number} · {sub}"),
        None => format!("Chapter {number}"),
    }
}

/// Whether a file name leads with a chapter marker ("Ch 3 …", "Chapter 3 …").
pub fn starts_with_chapter(name: &str) -> bool {
    static START: OnceLock<Regex> = OnceLock::new();
    re(&START, r"(?i)^\s*(?:ch(?:apter)?|chap)\.?\s*\d").is_match(name)
}

/// Removes the issue number (and anything after it) from a loose issue's
/// filename to get its series name: "Civil War 001 (2006)" -> "Civil War".
pub fn series_name_from_issue(raw: &str) -> CleanName {
    let base = strip_ext(raw.trim());
    let (rest, groups) = split_groups(base);
    static CUT: OnceLock<Regex> = OnceLock::new();
    let cut_re = re(&CUT, r"(?i)(?:#\s*\d|\b(?:ch(?:apter)?|issue)\.?\s*\d|\b\d{1,4}\b(?:\s*$|\s+-))");
    let series_part = match cut_re.find(&rest) {
        Some(m) if m.start() > 0 => &rest[..m.start()],
        _ => rest.as_str(),
    };
    // Re-attach the groups so years/edition tags are still seen.
    let rebuilt = format!(
        "{} {}",
        series_part,
        groups.iter().map(|g| format!("({g})")).collect::<Vec<_>>().join(" ")
    );
    clean_series_name(&rebuilt)
}

// ── Fuzzy matching ───────────────────────────────────────────────────────────

/// Lowercase, punctuation-free, single-spaced, without a leading "the".
pub fn normalize_title(s: &str) -> String {
    let s: String = s
        .to_lowercase()
        .replace('&', " and ")
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let s = collapse_ws(&s);
    let s = s.strip_prefix("the ").map(str::to_string).unwrap_or(s);
    // "vs" and "versus" are the same word for matching purposes.
    s.split(' ').map(|w| if w == "versus" { "vs" } else { w }).collect::<Vec<_>>().join(" ")
}

/// Similarity in `[0, 1]`: 1.0 when the normalized titles are identical,
/// otherwise word-set overlap with a modest bonus when one title's words
/// are all contained in the other's.
pub fn similarity(a: &str, b: &str) -> f32 {
    let a = normalize_title(a);
    let b = normalize_title(b);
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let aw: std::collections::HashSet<&str> = a.split(' ').collect();
    let bw: std::collections::HashSet<&str> = b.split(' ').collect();
    let inter = aw.intersection(&bw).count() as f32;
    let union = aw.union(&bw).count().max(1) as f32;
    let jaccard = inter / union;
    if aw.is_subset(&bw) || bw.is_subset(&aw) {
        0.45 + 0.4 * jaccard
    } else {
        0.8 * jaccard
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapter_volumes() {
        assert_eq!(volume_of("Ch 12 Vol 2 Dog And Chainsaw.cbz"), Some(2));
        assert_eq!(volume_of("Berserk Vol. 03 Ch 21"), Some(3));
        assert_eq!(volume_of("[Volume 4] Chapter 30"), Some(4));
        assert_eq!(volume_of("One_Piece_v12_c101"), None);
        assert_eq!(volume_of("Ch 0 Vol none Oneshot"), None);
        assert_eq!(volume_of("Chainsaw Man #001 (2019)"), None);
        assert_eq!(volume_of("Revolver 2"), None);
    }

    #[test]
    fn cleans_real_folder_names() {
        let c = clean_series_name("Civil War 001-007 (2006-2007) GetComics.INFO");
        assert_eq!(c.search, "Civil War");
        assert_eq!(c.year, Some(2006));

        let c = clean_series_name("Crossed v1 (000-009) (2008-2010)");
        assert_eq!(c.search, "Crossed");
        assert_eq!(c.volume, Some(1));
        assert_eq!(c.year, Some(2008));

        let c = clean_series_name("Crossed v2 Family Values (001-007) (2010-2011)");
        assert_eq!(c.search, "Crossed Family Values");
        assert_eq!(c.volume, Some(2));

        let c = clean_series_name("Chronological Avengers vs X-Men (Fan-Made)");
        assert_eq!(c.search, "Chronological Avengers vs X-Men");
        assert_eq!(c.display, "Chronological Avengers vs X-Men (Fan Edit)");

        let c = clean_series_name("Naruto (Color) 01J76XYBK4HFZMAWWKX52WN8PK");
        assert_eq!(c.search, "Naruto");
        assert_eq!(c.display, "Naruto (Color)");

        let c = clean_series_name("One Piece (Official Colored) a2c1d849-af05-4bbc-b2a7-866ebb10331f");
        assert_eq!(c.display, "One Piece (Color)");

        let c = clean_series_name("Death Note_ C-Kira-hen 3fc5c653-3fac-4ddb-aebf-f2e3d4d4adf6");
        assert_eq!(c.search, "Death Note: C-Kira-hen");

        let c = clean_series_name("Crossed Vol. 02- Family Values (2010) (Fan Made TPB).cbr");
        assert_eq!(c.search, "Crossed Family Values");
        assert_eq!(c.volume, Some(2));

        let c = clean_series_name("Crossed Vol. 01 (2009) (Fan Made TPB).cbr");
        assert_eq!(c.search, "Crossed");
        assert_eq!(c.volume, Some(1));
    }

    #[test]
    fn parses_issue_numbers() {
        let i = parse_issue("Civil War 001 (2006) (Digital) (Zone-Empire).cbr");
        assert_eq!(i.number.as_deref(), Some("1"));
        let i = parse_issue("Crossed 000 (2008) (Digital) (Nahga-Empire).cbr");
        assert_eq!(i.number.as_deref(), Some("0"));
        let i = parse_issue("Crossed - Family Values 03 (of 07) (2010) (9 Covers) (Digital).cbr");
        assert_eq!(i.number.as_deref(), Some("3"));
        let i = parse_issue("Chronological Avengers VS X-Men #4 - Act 2.cbr");
        assert_eq!(i.number.as_deref(), Some("4"));
        assert_eq!(i.subtitle.as_deref(), Some("Act 2"));
        let i = parse_issue("Ch 10 Vol none  Weeb central 01J76XZ5CP8JPTMJPA4GVWDN8F.cbz");
        assert_eq!(i.number.as_deref(), Some("10"));
        assert_eq!(i.subtitle, None);
        let i = parse_issue("Ch 1 Vol 1 Dog And Chainsaw Phantom troop 65bf7ab8-fc4e-43a2-a19a-4ef8b7004af1.cbz");
        assert_eq!(i.subtitle.as_deref(), Some("Dog And Chainsaw Phantom troop"));
        let i = parse_issue("Chapter 12.5 - Extra.cbz");
        assert_eq!(i.number.as_deref(), Some("12.5"));
    }

    #[test]
    fn strips_scanlation_groups_and_ids() {
        let i = parse_issue("Ch 0 Vol none Oneshot Binktopia scan f913c7e9-7c50-4a9f-83dd-cf2830f8350f.cbz");
        assert_eq!(i.number.as_deref(), Some("0"));
        assert_eq!(i.subtitle.as_deref(), Some("Oneshot"));
        let i = parse_issue("Ch 1 Vol none  Weeb central 01J76XZ34EPTCA4QJ8Y0Z1K2M3.cbz");
        assert_eq!(i.subtitle, None);
        // Names cut short (e.g. by a file manager) leave a partial ULID.
        let i = parse_issue("Ch 97 Vol 11 Weeb central 01J76XZ34EPTCA.cbz");
        assert_eq!(i.subtitle, None);
        let i = parse_issue("Ch 12.5 Vol none Extra TCB Scans.cbz");
        assert_eq!(i.subtitle.as_deref(), Some("Extra"));
        // Ordinary words that happen to be short hex stay put.
        let i = parse_issue("Ch 4 Vol 1 Face The Bad Dead.cbz");
        assert_eq!(i.subtitle.as_deref(), Some("Face The Bad Dead"));
    }

    #[test]
    fn titles_for_bare_chapter_files() {
        use std::path::Path;
        let roots = [Path::new("/lib/Manga")];
        let is_root = |p: &Path| roots.contains(&p);
        let oneshot = "Ch 0 Vol none Oneshot Binktopia scan f913c7e9-7c50-4a9f-83dd-cf2830f8350f.cbz";
        // manga-tui layout: Series/English/<chapter>
        let p = format!("/lib/Manga/Death Note/English/{oneshot}");
        assert_eq!(title_for_file(Path::new(&p), is_root), "Death Note");
        // Directly in the library root: no folder to borrow from.
        let p = format!("/lib/Manga/{oneshot}");
        assert_eq!(title_for_file(Path::new(&p), is_root), "Chapter 0 · Oneshot");
        // Normal loose issues keep their own (cleaned) name.
        let p = "/lib/Manga/Civil War 001 (2006) (Digital).cbr";
        assert_eq!(title_for_file(Path::new(p), is_root), "Civil War 001");
    }

    #[test]
    fn loose_issue_series_names() {
        assert_eq!(series_name_from_issue("Civil War 001 (2006) (Digital).cbr").search, "Civil War");
        assert_eq!(
            series_name_from_issue("Chronological Avengers VS X-Men #4 - Act 2.cbr").search,
            "Chronological Avengers VS X-Men"
        );
        assert_eq!(
            series_name_from_issue("Crossed - Family Values 01 (of 07) (2010).cbr").search,
            "Crossed - Family Values"
        );
    }

    #[test]
    fn tpb_volumes_group_as_one_series() {
        let a = series_name_from_issue("Crossed Vol. 01 (2009) (Fan Made TPB).cbr");
        let b = series_name_from_issue("Crossed Vol. 03 - Psychopath (2010) (Fan Made TPB).cbr");
        assert_eq!(a.search, "Crossed");
        assert_eq!(a.search, b.search);
    }

    #[test]
    fn similarity_scores() {
        assert_eq!(similarity("Crossed Family Values", "Crossed: Family Values"), 1.0);
        assert_eq!(similarity("The Walking Dead", "Walking Dead"), 1.0);
        assert!(similarity("Chronological Avengers vs X-Men", "Avengers vs. X-Men") < 0.85);
        assert!(similarity("Chronological Avengers vs X-Men", "Avengers vs. X-Men") > 0.6);
        assert!(similarity("Civil War", "Batman") < 0.1);
    }

    #[test]
    fn normalizes_numbers() {
        assert_eq!(normalize_number("001"), "1");
        assert_eq!(normalize_number("000"), "0");
        assert_eq!(normalize_number("1.50"), "1.5");
        assert_eq!(normalize_number("#12"), "12");
    }
}
