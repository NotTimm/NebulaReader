use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

// ── Types ─────────────────────────────────────────────────────────────────────

/// How a series' issues are found on disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeriesKind {
    /// A folder; issues are everything under it.
    #[default]
    Dir,
    /// A single standalone file (one issue, or an EPUB).
    File,
    /// Loose files in a library root grouped by series name. The series
    /// `path` is a synthetic `group://` key; issues are listed in the DB.
    Group,
}

impl SeriesKind {
    fn as_str(self) -> &'static str {
        match self {
            SeriesKind::Dir => "dir",
            SeriesKind::File => "file",
            SeriesKind::Group => "group",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "file" => SeriesKind::File,
            "group" => SeriesKind::Group,
            _ => SeriesKind::Dir,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SeriesEntry {
    pub id: i64,
    pub path: String,
    /// Cleaned local name (always set).
    pub title: String,
    /// Canonical online name when the match was confident; else empty.
    pub display_title: String,
    pub description: String,
    pub status: String,
    pub genres: String,
    pub publisher: String,
    pub year: Option<u32>,
    pub volume: Option<u32>,
    /// "comicvine" / "anilist" / "" (no match).
    pub source: String,
    pub source_url: String,
    /// "", "matched", "nomatch", or "error: …"
    pub meta_status: String,
    pub meta_fetched_at: i64,
    pub search_override: String,
    pub library_root: String,
    pub kind: SeriesKind,
    pub last_read_page: usize,
    pub last_read_chapter: usize,
    pub last_issue_path: String,
    pub total_pages: usize,
    pub date_added: i64,
    pub last_opened: i64,
    pub is_series: bool,
    pub is_epub: bool,
    // Derived from the issues table.
    pub issue_count: usize,
    pub issues_read: usize,
    /// Fraction of the in-progress issue, for the progress bar.
    pub current_issue_fraction: f32,
    pub current_issue_label: String,
    /// Unfinished issues with a saved page, wherever it was saved: progress
    /// from a server only reaches the issues, not the series' own fields.
    pub issues_in_progress: usize,
    /// Who made it, one "Role: Name, Name" per line.
    pub creators: String,
    /// Last year of publication, for finished series ("2018–2020").
    pub end_year: Option<u32>,
}

impl SeriesEntry {
    pub fn name(&self) -> &str {
        if self.display_title.is_empty() { &self.title } else { &self.display_title }
    }

    /// Overall progress: finished issues plus the fraction of the one in
    /// progress, over the issue count. Single files use page progress.
    pub fn progress_fraction(&self) -> f32 {
        if self.issue_count > 1 {
            let done = self.issues_read as f32 + self.current_issue_fraction;
            return (done / self.issue_count as f32).clamp(0.0, 1.0);
        }
        if self.issues_read >= 1 && self.issue_count == 1 {
            return 1.0;
        }
        if self.issue_count == 1 && self.current_issue_fraction > 0.0 {
            return self.current_issue_fraction;
        }
        if self.total_pages == 0 {
            return 0.0;
        }
        (self.last_read_page as f32 / self.total_pages as f32).clamp(0.0, 1.0)
    }

    pub fn is_finished(&self) -> bool {
        self.issue_count > 0 && self.issues_read >= self.issue_count
    }

    pub fn has_progress(&self) -> bool {
        self.last_read_page > 0 || self.last_read_chapter > 0 || self.issues_read > 0
            || self.current_issue_fraction > 0.0 || self.issues_in_progress > 0
    }
}

#[derive(Clone, Debug, Default)]
pub struct IssueEntry {
    pub id: i64,
    pub series_path: String,
    pub path: String,
    pub idx: usize,
    pub name: String,
    pub number: String,
    /// Subtitle parsed from the filename ("#4 - Act 2" -> "Act 2").
    pub subtitle: String,
    /// Official title from online metadata / ComicInfo.
    pub title: String,
    pub cover_date: String,
    pub url: String,
    /// Release date from the file or the server ("2021-03-14", "2021-03",
    /// "2021"); `cover_date` (online) wins when both exist.
    pub release_date: String,
    /// Who made it, one "Role: Name, Name" per line.
    pub creators: String,
    pub page_count: usize,
    pub last_page: usize,
    pub completed: bool,
    pub last_read_at: i64,
    /// Books: relative scroll position (0–1) inside chapter `last_page`.
    pub position: f32,
}

impl IssueEntry {
    /// "#3", "Ch. 12", or the file name when no number was found.
    pub fn label(&self, manga_style: bool) -> String {
        if self.number.is_empty() {
            return self.name.clone();
        }
        if manga_style { fl!("issue-label-manga", number = self.number.as_str()) } else { fl!("issue-label-comic", number = self.number.as_str()) }
    }

    /// The best release date known: online cover date, else the file's or
    /// server's.
    pub fn date(&self) -> &str {
        if self.cover_date.is_empty() { &self.release_date } else { &self.cover_date }
    }

    pub fn heading(&self) -> String {
        if !self.title.is_empty() && !is_placeholder_title(&self.title) {
            self.title.clone()
        } else if !self.subtitle.is_empty() && !is_placeholder_title(&self.subtitle) {
            self.subtitle.clone()
        } else {
            String::new()
        }
    }

    pub fn fraction(&self) -> f32 {
        if self.completed {
            return 1.0;
        }
        if self.page_count <= 1 {
            return 0.0;
        }
        (self.last_page as f32 / (self.page_count - 1) as f32).clamp(0.0, 1.0)
    }

    pub fn in_progress(&self) -> bool {
        !self.completed && self.last_page > 0
    }
}

/// One issue as found by a scan or an open, before it hits the DB.
#[derive(Clone, Debug)]
pub struct IssueRow {
    pub path: String,
    pub name: String,
    pub number: String,
    pub subtitle: String,
    /// `None` when it hasn't been counted (cheap scans skip known issues).
    pub page_count: Option<usize>,
    /// Date and creators from the file or server; `None` when not looked
    /// at this time (what's stored is kept).
    pub details: Option<IssueDetails>,
}

#[derive(Clone, Debug, Default)]
pub struct IssueDetails {
    pub release_date: String,
    pub creators: String,
}

/// Formats credits as one "Role: Name, Name" line per role, in first-seen
/// order, skipping duplicates.
pub fn format_credits<R: AsRef<str>, N: AsRef<str>>(credits: impl IntoIterator<Item = (R, N)>) -> String {
    let mut roles: Vec<(String, Vec<String>)> = Vec::new();
    for (role, name) in credits {
        let (role, name) = (role.as_ref().trim(), name.as_ref().trim());
        if role.is_empty() || name.is_empty() {
            continue;
        }
        let entry = match roles.iter_mut().position(|(r, _)| r == role) {
            Some(i) => &mut roles[i].1,
            None => {
                roles.push((role.to_string(), Vec::new()));
                &mut roles.last_mut().expect("just pushed").1
            }
        };
        for n in name.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            if !entry.iter().any(|e| e == n) {
                entry.push(n.to_string());
            }
        }
    }
    roles.iter().map(|(r, n)| format!("{r}: {}", n.join(", "))).collect::<Vec<_>>().join("\n")
}

// ── Paths ─────────────────────────────────────────────────────────────────────

/// Durable user data — the library database. Never auto-cleared.
pub fn app_data_dir() -> PathBuf {
    crate::platform::data_dir()
}

/// User preferences.
pub fn config_dir() -> PathBuf {
    crate::platform::config_dir()
}

/// Derived artifacts (cover thumbnails, extracted RAR pages). Safe to
/// delete at any time — everything here is regenerated on demand.
pub fn cache_dir() -> PathBuf {
    crate::platform::cache_dir()
}

pub fn db_path() -> PathBuf {
    app_data_dir().join("library.db")
}

/// Moves cache artifacts written by older versions (which put them in the
/// data directory) into the XDG cache directory, so upgrading doesn't
/// silently orphan a user's already-generated covers.
fn migrate_legacy_cache() {
    let legacy_root = app_data_dir();
    let cache_root = cache_dir();
    for name in ["covers", "rar-cache"] {
        let legacy = legacy_root.join(name);
        let target = cache_root.join(name);
        if legacy.is_dir() && !target.exists() {
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::rename(&legacy, &target);
        } else if legacy.is_dir() {
            // Both exist (older builds wrote to the data dir): it's all
            // regenerable cache, so drop the stray copy.
            let _ = std::fs::remove_dir_all(&legacy);
        }
    }
    // v1 covers were low-res and stretched; they're regenerated as needed.
    let _ = std::fs::remove_dir_all(cache_root.join("covers"));
}

/// A stable hash of a path/string, used to derive cache directory names that
/// don't change across runs.
pub fn path_hash(s: &str) -> String {
    let hash: u64 = s.bytes().enumerate().fold(0u64, |acc, (i, b)| {
        acc.wrapping_add((b as u64).wrapping_mul(31u64.wrapping_pow(i as u32 & 0x1f)))
    });
    format!("{hash:016x}")
}

/// Returns the directory where chapter-cover thumbnails for a series are cached.
pub fn series_cover_dir(series_path: &str) -> PathBuf {
    cache_dir().join("covers-v2").join(path_hash(series_path))
}

pub fn chapter_cover_path(series_path: &str, chapter_idx: usize) -> PathBuf {
    series_cover_dir(series_path).join(format!("ch{chapter_idx:04}.png"))
}

// ── Schema ────────────────────────────────────────────────────────────────────

pub fn open_db() -> Result<Connection, String> {
    let path = db_path();
    std::fs::create_dir_all(path.parent().unwrap())
        .map_err(|e| format!("failed to create data dir: {e}"))?;
    let conn = Connection::open(&path).map_err(|e| format!("open db: {e}"))?;
    init_schema(&conn)?;
    migrate_legacy_cache();
    run_migrations(&conn)?;
    Ok(conn)
}

/// Creates the tables a new database needs (a no-op for existing ones,
/// which `run_migrations` brings up to date).
fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS series (
             id              INTEGER PRIMARY KEY AUTOINCREMENT,
             path            TEXT    UNIQUE NOT NULL,
             title           TEXT    NOT NULL DEFAULT '',
             description     TEXT    DEFAULT '',
             status          TEXT    DEFAULT '',
             genres          TEXT    DEFAULT '',
             chapter_count   INTEGER,
             anilist_score   INTEGER,
             last_read_page  INTEGER DEFAULT 0,
             last_read_ch    INTEGER DEFAULT 0,
             total_pages     INTEGER DEFAULT 0,
             date_added      INTEGER DEFAULT 0,
             last_opened     INTEGER DEFAULT 0,
             is_series       INTEGER DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS library_dirs (
             path TEXT UNIQUE NOT NULL
         );
         CREATE TABLE IF NOT EXISTS issues (
             id            INTEGER PRIMARY KEY AUTOINCREMENT,
             series_path   TEXT    NOT NULL,
             path          TEXT    NOT NULL,
             idx           INTEGER NOT NULL DEFAULT 0,
             name          TEXT    NOT NULL DEFAULT '',
             number        TEXT    NOT NULL DEFAULT '',
             subtitle      TEXT    NOT NULL DEFAULT '',
             title         TEXT    NOT NULL DEFAULT '',
             cover_date    TEXT    NOT NULL DEFAULT '',
             url           TEXT    NOT NULL DEFAULT '',
             page_count    INTEGER NOT NULL DEFAULT 0,
             last_page     INTEGER NOT NULL DEFAULT 0,
             completed     INTEGER NOT NULL DEFAULT 0,
             last_read_at  INTEGER NOT NULL DEFAULT 0,
             position      REAL    NOT NULL DEFAULT 0,
             UNIQUE(series_path, path)
         );
         CREATE INDEX IF NOT EXISTS issues_by_path ON issues(path);
         CREATE TABLE IF NOT EXISTS downloads (
             path          TEXT    PRIMARY KEY,
             series_path   TEXT    NOT NULL,
             file          TEXT    NOT NULL,
             size          INTEGER NOT NULL DEFAULT 0,
             added_at      INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS pending_progress (
             item_key      TEXT    PRIMARY KEY,
             ticks         INTEGER NOT NULL,
             played        INTEGER NOT NULL,
             at            INTEGER NOT NULL
         );",
    )
    .map_err(|e| format!("init schema: {e}"))
}

/// Current expected schema version. Bump this and add a matching arm in
/// [`run_migrations`] whenever the schema changes.
const SCHEMA_VERSION: i64 = 7;

/// Brings an existing database up to [`SCHEMA_VERSION`] without touching
/// user data. Uses SQLite's `user_version` pragma as the version marker, so
/// upgrading the app never requires deleting the library.
fn run_migrations(conn: &Connection) -> Result<(), String> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| format!("read schema version: {e}"))?;

    if current >= SCHEMA_VERSION {
        return Ok(());
    }

    // v0 -> v1: `is_series` distinguishes series folders from single files.
    // `ALTER TABLE ... ADD COLUMN` errors if it already exists (fresh
    // databases get it from CREATE TABLE), so a failure here is expected
    // and harmless.
    if current < 1 {
        let _ = conn.execute("ALTER TABLE series ADD COLUMN is_series INTEGER DEFAULT 0", []);
    }

    // v1 -> v2: series metadata/matching + library-root columns. (The
    // issues table is created by CREATE TABLE IF NOT EXISTS above.)
    if current < 2 {
        for col in [
            "display_title TEXT NOT NULL DEFAULT ''",
            "publisher TEXT NOT NULL DEFAULT ''",
            "year INTEGER",
            "volume INTEGER",
            "source TEXT NOT NULL DEFAULT ''",
            "source_id TEXT NOT NULL DEFAULT ''",
            "source_url TEXT NOT NULL DEFAULT ''",
            "meta_status TEXT NOT NULL DEFAULT ''",
            "meta_fetched_at INTEGER NOT NULL DEFAULT 0",
            "search_override TEXT NOT NULL DEFAULT ''",
            "library_root TEXT NOT NULL DEFAULT ''",
            "kind TEXT NOT NULL DEFAULT ''",
            "last_issue_path TEXT NOT NULL DEFAULT ''",
        ] {
            let _ = conn.execute(&format!("ALTER TABLE series ADD COLUMN {col}"), []);
        }
    }

    // v2 -> v3: legacy rows get a cleaned title and a real kind (rows
    // outside library folders are never rescanned, so do it here).
    if current < 3 {
        let rows: Vec<(String, String, i64)> = conn
            .prepare("SELECT path, title, is_series FROM series WHERE kind = '' OR title = ''")
            .and_then(|mut st| {
                st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map(|it| it.flatten().collect())
            })
            .unwrap_or_default();
        for (path, title, is_series) in rows {
            let p = std::path::Path::new(&path);
            let kind = if p.is_dir() || (is_series != 0 && !p.is_file()) { "dir" } else { "file" };
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let clean = crate::naming::clean_series_name(&name).display;
            let title = if title.is_empty() || title == name || title == clean { clean } else { title };
            let _ = conn.execute(
                "UPDATE series SET kind = CASE WHEN kind = '' THEN ?2 ELSE kind END, title = ?3 WHERE path = ?1",
                params![path, kind, title],
            );
        }
    }

    // v3 -> v4: scroll position inside a book chapter.
    if current < 4 {
        let _ = conn.execute("ALTER TABLE issues ADD COLUMN position REAL NOT NULL DEFAULT 0", []);
    }

    // v4 -> v5: single chapter files were titled with their raw name
    // ("Ch 0 Vol none Oneshot Binktopia scan …"); name them after their
    // folder now, as new scans do. Titles stick once set, so fix them here.
    if current < 5 {
        let roots: Vec<String> = conn
            .prepare("SELECT path FROM library_dirs")
            .and_then(|mut st| st.query_map([], |r| r.get(0)).map(|it| it.flatten().collect()))
            .unwrap_or_default();
        let rows: Vec<(String, String)> = conn
            .prepare("SELECT path, title FROM series WHERE kind = 'file'")
            .and_then(|mut st| st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|it| it.flatten().collect()))
            .unwrap_or_default();
        for (path, title) in rows {
            if !crate::naming::starts_with_chapter(&title) {
                continue;
            }
            let fixed = crate::naming::title_for_file(std::path::Path::new(&path), |p| {
                roots.iter().any(|r| std::path::Path::new(r) == p)
            });
            let _ = conn.execute("UPDATE series SET title = ?2 WHERE path = ?1", params![path, fixed]);
        }
    }

    // v5 -> v6: creators and release dates. `details_read` marks issues
    // whose file has been checked for them, so the next scan reads the
    // ComicInfo.xml of issues that were scanned before this existed.
    if current < 6 {
        let _ = conn.execute("ALTER TABLE series ADD COLUMN creators TEXT NOT NULL DEFAULT ''", []);
        let _ = conn.execute("ALTER TABLE series ADD COLUMN end_year INTEGER", []);
        let _ = conn.execute("ALTER TABLE issues ADD COLUMN release_date TEXT NOT NULL DEFAULT ''", []);
        let _ = conn.execute("ALTER TABLE issues ADD COLUMN creators TEXT NOT NULL DEFAULT ''", []);
        let _ = conn.execute("ALTER TABLE issues ADD COLUMN details_read INTEGER NOT NULL DEFAULT 0", []);
    }

    // v6 -> v7: `downloads` (Jellyfin chapters kept for reading offline)
    // and `pending_progress` (progress that couldn't reach the server yet)
    // come from the CREATE TABLE IF NOT EXISTS above.

    conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))
        .map_err(|e| format!("set schema version: {e}"))?;
    Ok(())
}

// ── Library directories ──────────────────────────────────────────────────────

pub fn add_library_dir(conn: &Connection, path: &str) -> Result<(), String> {
    conn.execute("INSERT OR IGNORE INTO library_dirs (path) VALUES (?1)", params![path])
        .map_err(|e| format!("add_library_dir: {e}"))?;
    Ok(())
}

pub fn remove_library_dir(conn: &Connection, path: &str) -> Result<(), String> {
    conn.execute("DELETE FROM library_dirs WHERE path = ?1", params![path])
        .map_err(|e| format!("remove_library_dir: {e}"))?;
    Ok(())
}

pub fn list_library_dirs(conn: &Connection) -> Vec<String> {
    let sql = "SELECT path FROM library_dirs ORDER BY path";
    match conn.prepare(sql) {
        Err(_) => vec![],
        Ok(mut stmt) => stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map(|it| it.flatten().collect())
            .unwrap_or_default(),
    }
}

// ── Write operations ──────────────────────────────────────────────────────────

pub fn upsert_series(
    conn: &Connection,
    path: &str,
    title: &str,
    total_pages: usize,
    is_series: bool,
) -> Result<(), String> {
    let now = now_unix();
    let kind = if is_series { "dir" } else { "file" };
    conn.execute(
        "INSERT INTO series (path, title, total_pages, date_added, last_opened, is_series, kind)
         VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6)
         ON CONFLICT(path) DO UPDATE SET
             title       = CASE WHEN title = '' THEN excluded.title ELSE title END,
             total_pages = excluded.total_pages,
             is_series   = excluded.is_series,
             kind        = CASE WHEN kind = '' THEN excluded.kind ELSE kind END",
        params![path, title, total_pages as i64, now, is_series as i64, kind],
    )
    .map_err(|e| format!("upsert_series: {e}"))?;
    Ok(())
}

/// What a library scan learned about one series.
#[derive(Clone, Debug)]
pub struct ScannedSeries {
    pub path: String,
    pub title: String,
    /// Creators from the file (ComicInfo.xml, EPUB); empty when unknown.
    /// Only fills in a series that has none, so online credits stay.
    pub creators: String,
    pub year: Option<u32>,
    pub volume: Option<u32>,
    pub kind: SeriesKind,
    pub library_root: String,
    pub total_pages: Option<usize>,
}

/// Registers/updates a scanned series without touching reading progress
/// or `last_opened`. The cleaned title is always refreshed (it's derived
/// from the name on disk); online-matched names live in `display_title`.
pub fn upsert_scanned_series(conn: &Connection, s: &ScannedSeries) -> Result<(), String> {
    let now = now_unix();
    let is_series = s.kind != SeriesKind::File;
    conn.execute(
        "INSERT INTO series (path, title, total_pages, date_added, last_opened, is_series,
                             kind, library_root, year, volume, creators)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(path) DO UPDATE SET
             title        = excluded.title,
             total_pages  = CASE WHEN ?3 > 0 THEN excluded.total_pages ELSE total_pages END,
             is_series    = excluded.is_series,
             kind         = excluded.kind,
             library_root = excluded.library_root,
             year         = COALESCE(year, excluded.year),
             volume       = COALESCE(volume, excluded.volume),
             creators     = CASE WHEN creators = '' THEN excluded.creators ELSE creators END",
        params![
            s.path,
            s.title,
            s.total_pages.unwrap_or(0) as i64,
            now,
            is_series as i64,
            s.kind.as_str(),
            s.library_root,
            s.year,
            s.volume,
            s.creators,
        ],
    )
    .map_err(|e| format!("upsert_scanned_series: {e}"))?;
    Ok(())
}

/// Runs `f` as one unit: all of its writes land, or none do. The
/// outermost call is a single commit (one disk flush however many rows it
/// writes); calls inside it nest.
pub fn batch<T>(conn: &Connection, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    conn.execute_batch("SAVEPOINT batch").map_err(|e| format!("savepoint: {e}"))?;
    match f() {
        Ok(v) => {
            conn.execute_batch("RELEASE batch").map_err(|e| format!("commit: {e}"))?;
            Ok(v)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK TO batch; RELEASE batch");
            Err(e)
        }
    }
}

/// Replaces a series' issue list with `issues` (in order), keeping reading
/// progress and linked metadata for issues that are still there.
pub fn sync_issues(conn: &Connection, series_path: &str, issues: &[IssueRow]) -> Result<(), String> {
    batch(conn, || {
        let mut upsert = conn
            .prepare_cached(
                "INSERT INTO issues (series_path, path, idx, name, number, subtitle, page_count,
                                     release_date, creators, details_read)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, COALESCE(?7, 0), COALESCE(?8, ''), COALESCE(?9, ''), ?10)
                 ON CONFLICT(series_path, path) DO UPDATE SET
                     idx          = excluded.idx,
                     name         = excluded.name,
                     number       = excluded.number,
                     subtitle     = excluded.subtitle,
                     page_count   = CASE WHEN ?7 IS NOT NULL THEN ?7 ELSE page_count END,
                     release_date = CASE WHEN ?10 THEN COALESCE(?8, '') ELSE release_date END,
                     creators     = CASE WHEN ?10 THEN COALESCE(?9, '') ELSE creators END,
                     details_read = MAX(details_read, ?10)",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        for (i, row) in issues.iter().enumerate() {
            upsert
                .execute(params![
                    series_path,
                    row.path,
                    i as i64,
                    row.name,
                    row.number,
                    row.subtitle,
                    row.page_count.map(|c| c as i64),
                    row.details.as_ref().map(|d| d.release_date.as_str()),
                    row.details.as_ref().map(|d| d.creators.as_str()),
                    row.details.is_some() as i64,
                ])
                .map_err(|e| format!("upsert issue: {e}"))?;
        }
        let keep: std::collections::HashSet<&str> = issues.iter().map(|r| r.path.as_str()).collect();
        let existing: Vec<String> = {
            let mut st = conn
                .prepare_cached("SELECT path FROM issues WHERE series_path = ?1")
                .map_err(|e| format!("prepare: {e}"))?;
            let rows = st
                .query_map(params![series_path], |r| r.get::<_, String>(0))
                .map_err(|e| format!("query: {e}"))?;
            rows.flatten().collect()
        };
        for p in existing.iter().filter(|p| !keep.contains(p.as_str())) {
            let _ = conn.execute(
                "DELETE FROM issues WHERE series_path = ?1 AND path = ?2",
                params![series_path, p],
            );
        }
        Ok(())
    })
}

/// Issues whose file hasn't been checked for a date and creators yet.
pub fn issues_missing_details(conn: &Connection, series_path: &str) -> std::collections::HashSet<String> {
    conn.prepare("SELECT path FROM issues WHERE series_path = ?1 AND details_read = 0")
        .and_then(|mut st| st.query_map(params![series_path], |r| r.get(0)).map(|it| it.flatten().collect()))
        .unwrap_or_default()
}

/// Known page counts per issue path, so rescans only count new issues.
pub fn known_page_counts(conn: &Connection, series_path: &str) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    if let Ok(mut st) =
        conn.prepare("SELECT path, page_count FROM issues WHERE series_path = ?1 AND page_count > 0")
    {
        if let Ok(rows) = st.query_map(params![series_path], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
        }) {
            out.extend(rows.flatten());
        }
    }
    out
}

pub fn issues_for(conn: &Connection, series_path: &str) -> Vec<IssueEntry> {
    let sql = "SELECT id, series_path, path, idx, name, number, subtitle, title, cover_date, url,
                      page_count, last_page, completed, last_read_at, position, release_date, creators
               FROM issues WHERE series_path = ?1 ORDER BY idx";
    let Ok(mut st) = conn.prepare(sql) else { return vec![] };
    st.query_map(params![series_path], |r| {
        Ok(IssueEntry {
            id: r.get(0)?,
            series_path: r.get(1)?,
            path: r.get(2)?,
            idx: r.get::<_, i64>(3)? as usize,
            name: r.get(4)?,
            number: r.get(5)?,
            subtitle: r.get(6)?,
            title: r.get(7)?,
            cover_date: r.get(8)?,
            url: r.get(9)?,
            page_count: r.get::<_, i64>(10)? as usize,
            last_page: r.get::<_, i64>(11)? as usize,
            completed: r.get::<_, i64>(12)? != 0,
            last_read_at: r.get(13)?,
            position: r.get::<_, f64>(14)? as f32,
            release_date: r.get(15)?,
            creators: r.get(16)?,
        })
    })
    .map(|it| it.flatten().collect())
    .unwrap_or_default()
}

/// Records reading position inside one issue, plus the series-level
/// "where was I" pointer. Reaching the last page marks the issue read.
pub fn save_issue_progress(
    conn: &Connection,
    series_path: &str,
    issue_path: &str,
    page_in_issue: usize,
    page_count: usize,
    global_page: usize,
    chapter_idx: usize,
) -> Result<(), String> {
    let now = now_unix();
    let finished = page_count > 0 && page_in_issue + 1 >= page_count;
    conn.execute(
        "UPDATE issues SET last_page = ?3, page_count = CASE WHEN ?4 > 0 THEN ?4 ELSE page_count END,
                completed = CASE WHEN ?5 THEN 1 ELSE completed END, last_read_at = ?6
         WHERE series_path = ?1 AND path = ?2",
        params![series_path, issue_path, page_in_issue as i64, page_count as i64, finished, now],
    )
    .map_err(|e| format!("save_issue_progress: {e}"))?;
    conn.execute(
        "UPDATE series SET last_read_page = ?2, last_read_ch = ?3, last_issue_path = ?4 WHERE path = ?1",
        params![series_path, global_page as i64, chapter_idx as i64, issue_path],
    )
    .map_err(|e| format!("save_progress: {e}"))?;
    Ok(())
}

/// Applies read state from a server (Jellyfin) to one issue, but only when
/// the server's copy is newer than ours, so a sync never overwrites newer
/// local reading. A newer remote read also makes it the series' current
/// issue, so Continue Reading follows reading done in other apps.
pub fn apply_remote_progress(
    conn: &Connection,
    series_path: &str,
    issue_path: &str,
    page: usize,
    completed: bool,
    read_at: i64,
) -> Result<(), String> {
    if read_at <= 0 {
        return Ok(());
    }
    let changed = conn
        .execute(
            "UPDATE issues SET last_page = ?3, completed = ?4, last_read_at = ?5
             WHERE series_path = ?1 AND path = ?2 AND last_read_at < ?5",
            params![series_path, issue_path, page as i64, completed, read_at],
        )
        .map_err(|e| format!("apply_remote_progress: {e}"))?;
    if changed > 0 {
        conn.execute(
            "UPDATE series SET last_opened = ?2 WHERE path = ?1 AND last_opened < ?2",
            params![series_path, read_at],
        )
        .map_err(|e| format!("apply_remote_progress: {e}"))?;
        // "Continue" points at the chapter being read, not one just ticked
        // off, so only an unfinished chapter takes over, and only from one
        // read less recently.
        if !completed {
            conn.execute(
                "UPDATE series SET last_issue_path = ?2 WHERE path = ?1 AND NOT EXISTS (
                     SELECT 1 FROM issues i WHERE i.series_path = ?1 AND i.path = series.last_issue_path
                     AND i.completed = 0 AND i.last_read_at > ?3)",
                params![series_path, issue_path, read_at],
            )
            .map_err(|e| format!("apply_remote_progress: {e}"))?;
        }
    }
    Ok(())
}

/// Books: chapter + position within it. The book's single issue row holds
/// it; `page_count` is the chapter count so shelf progress works as-is.
pub fn save_book_progress(
    conn: &Connection,
    key: &str,
    chapter: usize,
    chapters: usize,
    position: f32,
    finished: bool,
) -> Result<(), String> {
    let now = now_unix();
    conn.execute(
        "UPDATE issues SET last_page = ?2, page_count = ?3, position = ?4,
                completed = CASE WHEN ?5 THEN 1 ELSE completed END, last_read_at = ?6
         WHERE series_path = ?1 AND path = ?1",
        params![key, chapter as i64, chapters as i64, position as f64, finished, now],
    )
    .map_err(|e| format!("save_book_progress: {e}"))?;
    conn.execute(
        "UPDATE series SET last_read_ch = ?2, last_issue_path = ?1 WHERE path = ?1",
        params![key, chapter as i64],
    )
    .map_err(|e| format!("save_book_progress: {e}"))?;
    Ok(())
}

pub fn set_issue_completed(conn: &Connection, issue_id: i64, completed: bool) -> Result<(), String> {
    conn.execute(
        "UPDATE issues SET completed = ?2, last_page = CASE WHEN ?2 THEN last_page ELSE 0 END,
                last_read_at = CASE WHEN ?2 THEN ?3 ELSE last_read_at END
         WHERE id = ?1",
        params![issue_id, completed, now_unix()],
    )
    .map_err(|e| format!("set_issue_completed: {e}"))?;
    Ok(())
}

pub fn set_series_completed(conn: &Connection, series_path: &str, completed: bool) -> Result<(), String> {
    conn.execute(
        "UPDATE issues SET completed = ?2, last_page = CASE WHEN ?2 THEN last_page ELSE 0 END
         WHERE series_path = ?1",
        params![series_path, completed],
    )
    .map_err(|e| format!("set_series_completed: {e}"))?;
    if !completed {
        let _ = conn.execute(
            "UPDATE series SET last_read_page = 0, last_read_ch = 0, last_issue_path = '' WHERE path = ?1",
            params![series_path],
        );
    }
    Ok(())
}

/// If `path` is an issue of some *other* series (e.g. a file opened on its
/// own that also lives in a library series folder), returns that series and
/// the issue's chapter index — so opening it continues the series instead
/// of creating a duplicate shelf entry.
pub fn series_for_issue(conn: &Connection, path: &str) -> Option<(String, usize)> {
    conn.query_row(
        "SELECT i.series_path, i.idx FROM issues i JOIN series s ON s.path = i.series_path
         WHERE i.path = ?1 AND i.series_path != ?1 AND s.kind != 'file'
         ORDER BY s.library_root != '' DESC LIMIT 1",
        params![path],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize)),
    )
    .ok()
}

/// Bumps `last_opened` without touching anything else — call when a series
/// is actually opened in the reader (bulk library scans shouldn't bump it).
pub fn touch_opened(conn: &Connection, path: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE series SET last_opened = ?2 WHERE path = ?1",
        params![path, now_unix()],
    )
    .map_err(|e| format!("touch_opened: {e}"))?;
    Ok(())
}

/// Stores an online match on the series and links its issues by number.
pub fn apply_match(
    conn: &Connection,
    series_path: &str,
    m: &crate::metadata::SeriesMatch,
    display_title: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE series SET
             display_title   = ?2,
             description     = ?3,
             status          = ?4,
             genres          = ?5,
             chapter_count   = ?6,
             publisher       = ?7,
             year            = COALESCE(?8, year),
             end_year        = COALESCE(?13, end_year),
             creators        = CASE WHEN ?14 != '' THEN ?14 ELSE creators END,
             source          = ?9,
             source_id       = ?10,
             source_url      = ?11,
             meta_status     = 'matched',
             meta_fetched_at = ?12
         WHERE path = ?1",
        params![
            series_path,
            display_title.unwrap_or(""),
            m.description,
            m.status,
            m.genres.join(", "),
            m.chapter_count.map(|c| c as i64),
            m.publisher,
            m.year,
            m.source,
            m.source_id,
            m.url,
            now_unix(),
            m.end_year,
            format_credits(m.creators.iter().map(|(r, n)| (r, n))),
        ],
    )
    .map_err(|e| format!("apply_match: {e}"))?;

    if !m.issues.is_empty() {
        let mut st = conn
            .prepare(
                "UPDATE issues SET title = ?3, cover_date = ?4, url = ?5
                 WHERE series_path = ?1 AND number = ?2",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        for issue in &m.issues {
            let _ = st.execute(params![series_path, issue.number, issue.title, issue.cover_date, issue.url]);
        }
    }
    Ok(())
}

/// Records that a lookup ran without a usable match (or failed).
pub fn mark_meta(conn: &Connection, series_path: &str, status: &str, clear_match: bool) {
    let now = now_unix();
    if clear_match {
        let _ = conn.execute(
            "UPDATE series SET display_title = '', description = '', status = '', genres = '',
                    publisher = '', source = '', source_id = '', source_url = '' WHERE path = ?1",
            params![series_path],
        );
        let _ = conn.execute(
            "UPDATE issues SET title = '', cover_date = '', url = '' WHERE series_path = ?1",
            params![series_path],
        );
    }
    let _ = conn.execute(
        "UPDATE series SET meta_status = ?2, meta_fetched_at = ?3 WHERE path = ?1",
        params![series_path, status, now],
    );
}

pub fn set_search_override(conn: &Connection, series_path: &str, query: &str) {
    let _ = conn.execute(
        "UPDATE series SET search_override = ?2 WHERE path = ?1",
        params![series_path, query.trim()],
    );
}

/// Forgets every lookup so the next scan re-matches everything.
pub fn reset_all_metadata(conn: &Connection) {
    let _ = conn.execute("UPDATE series SET meta_fetched_at = 0, meta_status = ''", []);
}

/// Removes series under `root` that the latest scan didn't find (deleted
/// or moved on disk). Only call when `root` itself is reachable, so an
/// unmounted network drive never wipes its part of the library.
pub fn prune_root(conn: &Connection, root: &str, found: &[String]) -> usize {
    let keep: std::collections::HashSet<&str> = found.iter().map(String::as_str).collect();
    let paths: Vec<String> = conn
        .prepare("SELECT path FROM series WHERE library_root = ?1")
        .and_then(|mut st| {
            st.query_map(params![root], |r| r.get::<_, String>(0)).map(|it| it.flatten().collect())
        })
        .unwrap_or_default();
    let gone: Vec<&String> = paths.iter().filter(|p| !keep.contains(p.as_str())).collect();
    let _ = batch(conn, || {
        gone.iter().for_each(|p| delete_series(conn, p));
        Ok(())
    });
    gone.len()
}

/// Series this library folder holds, so a scan that suddenly finds nothing
/// can be told apart from a drive that isn't there.
pub fn series_count_under_root(conn: &Connection, root: &str) -> usize {
    conn.query_row("SELECT COUNT(*) FROM series WHERE library_root = ?1", params![root], |r| r.get::<_, i64>(0))
        .map(|n| n as usize)
        .unwrap_or(0)
}

/// Removes the series of one Jellyfin account (or OPDS catalog) whose
/// library isn't among `roots` any more (the library was deleted or hidden
/// on the server, or the folder taken out of the library).
pub fn prune_remote_roots(conn: &Connection, account_prefix: &str, roots: &std::collections::HashSet<&str>) {
    let rows: Vec<(String, String)> = conn
        .prepare("SELECT path, library_root FROM series WHERE substr(library_root, 1, length(?1)) = ?1")
        .and_then(|mut st| {
            st.query_map(params![account_prefix], |r| Ok((r.get(0)?, r.get(1)?))).map(|it| it.flatten().collect())
        })
        .unwrap_or_default();
    let _ = batch(conn, || {
        for (path, root) in &rows {
            if !roots.contains(root.as_str()) {
                delete_series(conn, path);
            }
        }
        Ok(())
    });
}

/// Forgets files opened outside any library folder that have since been
/// deleted. A file whose folder is gone too (an unplugged drive) is kept,
/// since it may come back.
pub fn prune_missing_loose_files(conn: &Connection) -> usize {
    let paths: Vec<String> = conn
        .prepare("SELECT path FROM series WHERE library_root = '' AND path NOT LIKE 'jellyfin://%' AND path NOT LIKE 'opds://%'")
        .and_then(|mut st| st.query_map([], |r| r.get(0)).map(|it| it.flatten().collect()))
        .unwrap_or_default();
    let mut removed = 0;
    for p in paths {
        let path = std::path::Path::new(&p);
        let folder_there = path.parent().is_some_and(|d| d.is_dir());
        if !path.exists() && folder_there {
            delete_series(conn, &p);
            removed += 1;
        }
    }
    removed
}

pub fn delete_series(conn: &Connection, path: &str) {
    let _ = conn.execute("DELETE FROM issues WHERE series_path = ?1", params![path]);
    let _ = conn.execute("DELETE FROM series WHERE path = ?1", params![path]);
}

pub fn remove_series_under_root(conn: &Connection, root: &str) {
    let paths: Vec<String> = conn
        .prepare("SELECT path FROM series WHERE library_root = ?1")
        .and_then(|mut st| {
            st.query_map(params![root], |r| r.get::<_, String>(0)).map(|it| it.flatten().collect())
        })
        .unwrap_or_default();
    for p in paths {
        delete_series(conn, &p);
    }
}

/// Removes every series whose library root starts with `prefix` (all of a
/// Jellyfin account's libraries when signing out).
pub fn remove_series_under_root_prefix(conn: &Connection, prefix: &str) {
    let paths: Vec<String> = conn
        .prepare("SELECT path FROM series WHERE substr(library_root, 1, length(?1)) = ?1")
        .and_then(|mut st| {
            st.query_map(params![prefix], |r| r.get::<_, String>(0)).map(|it| it.flatten().collect())
        })
        .unwrap_or_default();
    for p in paths {
        delete_series(conn, &p);
    }
}

/// Records an issue's page count once it's known (remote issues are listed
/// before anyone has opened them).
pub fn set_issue_page_count(conn: &Connection, series_path: &str, issue_path: &str, pages: usize) -> Result<(), String> {
    conn.execute(
        "UPDATE issues SET page_count = ?3 WHERE series_path = ?1 AND path = ?2",
        params![series_path, issue_path, pages as i64],
    )
    .map(|_| ())
    .map_err(|e| format!("set_issue_page_count: {e}"))
}

// ── Read operations ───────────────────────────────────────────────────────────

pub fn get_progress(conn: &Connection, path: &str) -> Option<(usize, usize)> {
    conn.query_row(
        "SELECT last_read_page, last_read_ch FROM series WHERE path = ?1",
        params![path],
        |row| Ok((row.get::<_, i64>(0)? as usize, row.get::<_, i64>(1)? as usize)),
    )
    .ok()
}

const SERIES_COLS: &str = "s.id, s.path, s.title, s.display_title, s.description, s.status, s.genres,
    s.publisher, s.year, s.volume, s.source, s.source_url, s.meta_status, s.meta_fetched_at,
    s.search_override, s.library_root, s.kind, s.last_read_page, s.last_read_ch, s.last_issue_path,
    s.total_pages, s.date_added, s.last_opened, s.is_series,
    (SELECT COUNT(*) FROM issues i WHERE i.series_path = s.path),
    (SELECT COUNT(*) FROM issues i WHERE i.series_path = s.path AND i.completed),
    (SELECT CASE WHEN s.path LIKE '%.epub' AND i.page_count > 0 AND NOT i.completed
                 THEN (i.last_page + i.position) / i.page_count
                 WHEN i.page_count > 1 AND NOT i.completed
                 THEN CAST(i.last_page AS REAL) / (i.page_count - 1) ELSE 0 END
       FROM issues i WHERE i.series_path = s.path AND i.path = s.last_issue_path),
    (SELECT CASE WHEN i.number != '' THEN i.number ELSE i.name END
       FROM issues i WHERE i.series_path = s.path AND i.path = s.last_issue_path),
    s.creators, s.end_year,
    (SELECT COUNT(*) FROM issues i WHERE i.series_path = s.path AND NOT i.completed AND i.last_page > 0)";

/// Single files that are also an issue of a folder/group series are hidden
/// from the shelf — that series already represents them.
const NOT_DUPLICATE: &str = "NOT (s.kind = 'file' AND EXISTS (
    SELECT 1 FROM issues i JOIN series o ON o.path = i.series_path
    WHERE i.path = s.path AND o.path != s.path AND o.kind != 'file'))";

pub fn all_series(conn: &Connection) -> Vec<SeriesEntry> {
    let sql = format!("SELECT {SERIES_COLS} FROM series s WHERE {NOT_DUPLICATE} ORDER BY s.last_opened DESC");
    match conn.prepare(&sql) {
        Err(e) => {
            log::error!("all_series: {e}");
            vec![]
        }
        Ok(mut stmt) => stmt
            .query_map([], row_to_entry)
            .map(|it| it.flatten().collect())
            .unwrap_or_default(),
    }
}

pub fn get_series(conn: &Connection, path: &str) -> Option<SeriesEntry> {
    let sql = format!("SELECT {SERIES_COLS} FROM series s WHERE s.path = ?1");
    conn.query_row(&sql, params![path], row_to_entry).ok()
}

pub fn search_series(conn: &Connection, query: &str) -> Vec<SeriesEntry> {
    let pattern = format!("%{}%", query.to_lowercase());
    let sql = format!(
        "SELECT {SERIES_COLS} FROM series s WHERE {NOT_DUPLICATE} AND (
             lower(s.title) LIKE ?1 OR lower(s.display_title) LIKE ?1 OR lower(s.description) LIKE ?1
             OR lower(s.genres) LIKE ?1 OR lower(s.publisher) LIKE ?1
             OR EXISTS (SELECT 1 FROM issues i WHERE i.series_path = s.path AND lower(i.title) LIKE ?1))
         ORDER BY s.last_opened DESC"
    );
    match conn.prepare(&sql) {
        Err(_) => vec![],
        Ok(mut stmt) => stmt
            .query_map(params![pattern], row_to_entry)
            .map(|it| it.flatten().collect())
            .unwrap_or_default(),
    }
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<SeriesEntry> {
    let path: String = row.get(1)?;
    let is_epub = path.to_ascii_lowercase().ends_with(".epub");
    Ok(SeriesEntry {
        id: row.get(0)?,
        title: row.get(2)?,
        display_title: row.get(3)?,
        description: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        status: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        genres: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        publisher: row.get(7)?,
        year: row.get::<_, Option<i64>>(8)?.map(|v| v as u32),
        volume: row.get::<_, Option<i64>>(9)?.map(|v| v as u32),
        source: row.get(10)?,
        source_url: row.get(11)?,
        meta_status: row.get(12)?,
        meta_fetched_at: row.get(13)?,
        search_override: row.get(14)?,
        library_root: row.get(15)?,
        kind: SeriesKind::parse(&row.get::<_, String>(16)?),
        last_read_page: row.get::<_, i64>(17)? as usize,
        last_read_chapter: row.get::<_, i64>(18)? as usize,
        last_issue_path: row.get(19)?,
        total_pages: row.get::<_, i64>(20)? as usize,
        date_added: row.get(21)?,
        last_opened: row.get(22)?,
        is_series: row.get::<_, i64>(23)? != 0,
        issue_count: row.get::<_, i64>(24)? as usize,
        issues_read: row.get::<_, i64>(25)? as usize,
        current_issue_fraction: row.get::<_, Option<f64>>(26)?.unwrap_or(0.0) as f32,
        current_issue_label: row.get::<_, Option<String>>(27)?.unwrap_or_default(),
        creators: row.get(28)?,
        end_year: row.get::<_, Option<i64>>(29)?.map(|v| v as u32),
        issues_in_progress: row.get::<_, i64>(30)? as usize,
        path,
        is_epub,
    })
}

// ── Downloads (Jellyfin chapters kept for offline reading) ─────────────────

/// A downloaded Jellyfin chapter or book. `path` is the issue's key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub path: String,
    pub series_path: String,
    pub file: PathBuf,
    pub size: u64,
}

pub fn add_download(conn: &Connection, d: &Download) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO downloads (path, series_path, file, size, added_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![d.path, d.series_path, d.file.to_string_lossy(), d.size as i64, now_unix()],
    )
    .map(|_| ())
    .map_err(|e| format!("add_download: {e}"))
}

pub fn downloads(conn: &Connection) -> Vec<Download> {
    conn.prepare("SELECT path, series_path, file, size FROM downloads ORDER BY added_at")
        .and_then(|mut st| {
            st.query_map([], |r| {
                Ok(Download {
                    path: r.get(0)?,
                    series_path: r.get(1)?,
                    file: PathBuf::from(r.get::<_, String>(2)?),
                    size: r.get::<_, i64>(3)?.max(0) as u64,
                })
            })
            .map(|it| it.flatten().collect())
        })
        .unwrap_or_default()
}

/// Forgets a download; returns its file so the caller can delete it.
pub fn remove_download(conn: &Connection, path: &str) -> Option<PathBuf> {
    let file: String = conn
        .query_row("SELECT file FROM downloads WHERE path = ?1", params![path], |r| r.get(0))
        .ok()?;
    let _ = conn.execute("DELETE FROM downloads WHERE path = ?1", params![path]);
    Some(PathBuf::from(file))
}

/// Downloads whose chapter is no longer in the library (removed on the
/// server, or its account signed out).
pub fn orphaned_downloads(conn: &Connection) -> Vec<Download> {
    downloads(conn)
        .into_iter()
        .filter(|d| {
            !conn
                .query_row(
                    "SELECT 1 FROM issues WHERE series_path = ?1 AND path = ?2",
                    params![d.series_path, d.path],
                    |_| Ok(()),
                )
                .is_ok()
        })
        .collect()
}

// ── Progress waiting for its server ────────────────────────────────────────

/// Jellyfin progress that couldn't be sent (server unreachable), keyed by
/// the item's library key, to send once the server is back.
pub fn set_pending_progress(conn: &Connection, item_key: &str, ticks: i64, played: bool) {
    let _ = conn.execute(
        "INSERT OR REPLACE INTO pending_progress (item_key, ticks, played, at) VALUES (?1, ?2, ?3, ?4)",
        params![item_key, ticks, played, now_unix()],
    );
}

/// Pending progress under a key prefix: (item key, ticks, played, when).
pub fn pending_progress(conn: &Connection, prefix: &str) -> Vec<(String, i64, bool, i64)> {
    conn.prepare("SELECT item_key, ticks, played, at FROM pending_progress WHERE substr(item_key, 1, length(?1)) = ?1")
        .and_then(|mut st| {
            st.query_map(params![prefix], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map(|it| it.flatten().collect())
        })
        .unwrap_or_default()
}

/// Clears pending progress, but only if it's still the one that was sent
/// (`at`), not a newer one saved meanwhile.
pub fn clear_pending_progress(conn: &Connection, item_key: &str, at: i64) {
    let _ = conn.execute("DELETE FROM pending_progress WHERE item_key = ?1 AND at <= ?2", params![item_key, at]);
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Records a transient lookup failure without marking the series as
/// looked-up, so it's retried next launch.
pub fn set_meta_error(conn: &Connection, series_path: &str, err: &str) {
    let _ = conn.execute(
        "UPDATE series SET meta_status = ?2 WHERE path = ?1",
        params![series_path, format!("error: {err}")],
    );
}

/// "2021-03-14" -> "Mar 14, 2021", "2021-03" -> "Mar 2021", "2021" -> "2021".
/// A month's short name, 1 = January.
fn month_name(month: usize) -> String {
    match month {
        1 => fl!("month-1"),
        2 => fl!("month-2"),
        3 => fl!("month-3"),
        4 => fl!("month-4"),
        5 => fl!("month-5"),
        6 => fl!("month-6"),
        7 => fl!("month-7"),
        8 => fl!("month-8"),
        9 => fl!("month-9"),
        10 => fl!("month-10"),
        11 => fl!("month-11"),
        _ => fl!("month-12"),
    }
}

pub fn pretty_date(date: &str) -> String {
    let mut parts = date.trim().split('-');
    let (Some(y), m, d) = (parts.next().filter(|y| y.len() == 4), parts.next(), parts.next()) else {
        return date.to_string();
    };
    let month = m.and_then(|m| m.parse::<usize>().ok()).filter(|m| (1..=12).contains(m)).map(month_name);
    let day = d.and_then(|d| d.parse::<u32>().ok()).filter(|d| (1..=31).contains(d));
    match (month, day) {
        (Some(m), Some(d)) => fl!("date-day", month = m, day = d, year = y),
        (Some(m), None) => fl!("date-month", month = m, year = y),
        _ => y.to_string(),
    }
}

#[cfg(test)]
mod credit_tests {
    use super::{format_credits, pretty_date};

    #[test]
    fn formats_credits_by_role() {
        let c = format_credits([("Writer", "Garth Ennis"), ("Artist", "Jacen Burrows"), ("Writer", "Garth Ennis, Si Spurrier")]);
        assert_eq!(c, "Writer: Garth Ennis, Si Spurrier\nArtist: Jacen Burrows");
        assert_eq!(format_credits(Vec::<(&str, &str)>::new()), "");
    }

    #[test]
    fn prettifies_dates() {
        assert_eq!(pretty_date("2021-03-14"), "Mar 14, 2021");
        assert_eq!(pretty_date("2021-03"), "Mar 2021");
        assert_eq!(pretty_date("2021"), "2021");
        assert_eq!(pretty_date("2021-00-00"), "2021");
        assert_eq!(pretty_date("someday"), "someday");
    }
}

/// A stand-in some databases use for an issue with no story title, like
/// ComicVine's "[untitled]": better shown as no title at all.
pub fn is_placeholder_title(title: &str) -> bool {
    let t = title.trim().trim_matches(|c| matches!(c, '[' | ']' | '(' | ')')).trim().to_ascii_lowercase();
    matches!(t.as_str(), "untitled" | "no title" | "none" | "n/a" | "tba" | "tbd" | "" )
}

#[cfg(test)]
mod download_tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn placeholder_titles() {
        for t in ["[untitled]", "Untitled", "(No Title)", "[ ]", "TBA"] {
            assert!(is_placeholder_title(t), "{t}");
        }
        for t in ["Whose Side Are You On?", "Untitled Love Story", "None More Black"] {
            assert!(!is_placeholder_title(t), "{t}");
        }
    }

    #[test]
    fn downloads_and_orphans() {
        let conn = db();
        conn.execute("INSERT INTO issues (series_path, path) VALUES ('jellyfin://a/s', 'jellyfin://a/i1')", []).unwrap();
        let d = |path: &str| Download {
            path: path.into(),
            series_path: "jellyfin://a/s".into(),
            file: PathBuf::from(format!("/tmp/{path}.cbz")),
            size: 10,
        };
        add_download(&conn, &d("jellyfin://a/i1")).unwrap();
        add_download(&conn, &d("jellyfin://a/gone")).unwrap();
        assert_eq!(downloads(&conn).len(), 2);
        let orphans: Vec<String> = orphaned_downloads(&conn).into_iter().map(|d| d.path).collect();
        assert_eq!(orphans, ["jellyfin://a/gone"]);
        assert_eq!(remove_download(&conn, "jellyfin://a/gone"), Some(PathBuf::from("/tmp/jellyfin://a/gone.cbz")));
        assert_eq!(remove_download(&conn, "jellyfin://a/gone"), None);
        assert_eq!(downloads(&conn).len(), 1);
    }

    #[test]
    fn remote_progress_alone_counts_as_progress() {
        let conn = db();
        conn.execute("INSERT INTO series (path, title, last_opened) VALUES ('jellyfin://a/s', 'S', 0)", []).unwrap();
        for i in ["jellyfin://a/i1", "jellyfin://a/i2"] {
            conn.execute("INSERT INTO issues (series_path, path) VALUES ('jellyfin://a/s', ?1)", [i]).unwrap();
        }
        assert!(!get_series(&conn, "jellyfin://a/s").unwrap().has_progress());
        // Read on another device: page 12 of a chapter never opened here,
        // so its page count is still unknown.
        apply_remote_progress(&conn, "jellyfin://a/s", "jellyfin://a/i2", 12, false, 1_700_000_000).unwrap();
        let e = get_series(&conn, "jellyfin://a/s").unwrap();
        assert!(e.has_progress() && !e.is_finished());
        assert_eq!((e.last_opened, e.last_issue_path.as_str()), (1_700_000_000, "jellyfin://a/i2"));
    }

    #[test]
    fn pending_progress_keeps_newer_saves() {
        let conn = db();
        set_pending_progress(&conn, "jellyfin://a/i1", 50_000, false);
        set_pending_progress(&conn, "jellyfin://b/i9", 10_000, true);
        let mine = pending_progress(&conn, "jellyfin://a/");
        assert_eq!(mine.len(), 1);
        let (key, ticks, played, at) = mine[0].clone();
        assert_eq!((key.as_str(), ticks, played), ("jellyfin://a/i1", 50_000, false));
        // Sent while a newer one was saved: the newer one stays.
        clear_pending_progress(&conn, "jellyfin://a/i1", at - 1);
        assert_eq!(pending_progress(&conn, "jellyfin://a/").len(), 1);
        clear_pending_progress(&conn, "jellyfin://a/i1", at);
        assert!(pending_progress(&conn, "jellyfin://a/").is_empty());
        assert_eq!(pending_progress(&conn, "jellyfin://").len(), 1);
    }
}
