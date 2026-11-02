//! OPDS catalogs in the app: adding one in Settings (as a library or as a
//! catalog) and the Catalog page for browsing it.
//!
//! - **As a library** (a server of your own): the user picks folders on the
//!   Catalog page and they're synced into the library like a Jellyfin
//!   library. Adding a very big folder, or the whole catalog, asks first.
//! - **As a catalog** (a big public collection like Project Gutenberg): the
//!   Catalog page is a place to browse and search. Nothing is kept from it
//!   but what's downloaded: a book page shows what the catalog says about a
//!   book and downloads it, and only then does it join the library (in a
//!   "Downloads · <catalog>" group). Removing the download removes it.
//!
//! Syncing, reading, covers, downloads and offline reading are shared with
//! Jellyfin (`jellyfin_view`, through [`crate::remote::Account`]); what's
//! here is only what OPDS needs on top.

use std::collections::{HashMap, HashSet};

use cosmic::iced::widget::scrollable::{self as scroll_mod, AbsoluteOffset, RelativeOffset, Viewport};
use cosmic::iced::{Alignment, ContentFit, Length};
use cosmic::widget::{self, icon};
use cosmic::{app::Task, ApplicationExt, Element};

use super::downloads::{human_size, DlState};
use super::jellyfin_view::JfMsg;
use super::{App, AppView, DlMsg, Message};
use crate::library;
use crate::opds::{self, Book, FacetGroup, Feed, Folder, Kind, Root, SavedBook, Search};
use crate::remote::Account as RemoteAccount;

/// A folder with at least this many books asks before it's added to a
/// library: syncing it puts every one of them in the library.
const BIG_FOLDER: usize = 1000;
/// The Catalog page's scrolling area (a folder's list, or a book's page).
fn catalog_scroll_id() -> widget::Id {
    widget::Id::new("opds-catalog")
}

fn to_top() -> Task<Message> {
    scroll_mod::snap_to(catalog_scroll_id(), RelativeOffset { x: None, y: Some(0.0) })
}

/// Pictures kept for the Catalog page before starting over.
const MAX_THUMBS: usize = 400;

#[derive(Clone, Debug)]
pub enum OpdsMsg {
    UrlInput(String),
    UserInput(String),
    PasswordInput(String),
    Connect,
    Connected(Result<opds::Account, String>),
    /// Adds the catalog that answered, as a library or as a catalog.
    AddAs(Kind),
    CancelAdd,
    Remove(String),
    /// Opens the Catalog page on an account's start feed.
    Browse(String),
    /// The library's Browse Catalogs button: the last catalog browsed.
    BrowseCatalogs,
    /// Goes into a folder: (title, url).
    Open(String, String),
    /// Goes back up a folder (or out of a book's page).
    Back,
    Loaded(u64, Result<Feed, String>),
    More,
    MoreLoaded(u64, Result<Feed, String>),
    SearchInput(String),
    Search,
    SearchFound(u64, String, Result<String, String>),
    /// Adds a folder to the library, or takes it out: (title, url).
    ToggleRoot(String, String),
    /// Adds the big folder that's waiting for a yes.
    ConfirmRoot,
    CancelRoot,
    OpenInBrowser(String),
    /// Shows the folder sorted or filtered another way (a facet's link).
    Facet(String),
    /// A book's page, by its place in the folder shown.
    ShowBook(usize),
    /// The folder's list scrolled (kept, to come back to from a book).
    ListScrolled(Viewport),
    /// A picture for the Catalog page arrived: (url, image).
    Thumb(String, Option<Vec<u8>>),
    /// Downloads an edition of the book shown.
    Download(usize),
    /// Opens a downloaded book, by its catalog id.
    Read(String),
    /// Removes a downloaded book (or stops its download), by its catalog id.
    RemoveSaved(String),
}

#[derive(Default)]
pub struct OpdsState {
    url_input: String,
    user_input: String,
    password_input: String,
    busy: bool,
    status: Option<String>,
    /// A catalog that answered, waiting to be added as a library or a catalog.
    pending: Option<opds::Account>,
    pub(super) browse: Option<Browse>,
    /// Counts loads, so an answer for a folder already left is ignored.
    generation: u64,
    /// Catalogs whose folders changed while a sync was running: synced
    /// again when it ends.
    pub(super) resync: HashSet<String>,
    /// Covers and thumbnails on the Catalog page, by URL: kept in memory
    /// only, never saved. `None` while loading or when it failed.
    thumbs: HashMap<String, Option<widget::image::Handle>>,
    /// A big folder waiting for a yes before it's added to a library.
    confirm: Option<Confirm>,
    /// The catalog last browsed, for the library's Browse Catalogs button.
    last_catalog: Option<String>,
    /// How far down the folder's list was, for Back from a book's page.
    list_offset: f32,
}

#[derive(Clone)]
struct Confirm {
    title: String,
    url: String,
    count: Option<usize>,
    /// The catalog's start page: everything in it.
    whole: bool,
}

/// Where the Catalog page is.
pub struct Browse {
    account_key: String,
    /// The folders walked into, from the start feed: (title, url).
    trail: Vec<(String, String)>,
    /// The folder shown, with every page loaded so far.
    feed: Option<Feed>,
    loading: bool,
    error: Option<String>,
    search: Option<Search>,
    search_input: String,
    /// Where in the trail the search results are: a new search replaces
    /// them (and anything opened from them) instead of adding a level.
    search_at: Option<usize>,
    /// A book's page, over the folder.
    book: Option<BookPage>,
}

impl Browse {
    /// Shows search results: in place of the last search's (and anything
    /// opened from them), so searching again doesn't add a level.
    fn push_search(&mut self, title: String, url: String) {
        if let Some(at) = self.search_at {
            self.trail.truncate(at);
        }
        self.search_at = Some(self.trail.len());
        self.trail.push((title, url));
    }

    /// Up one folder; false at the catalog's start.
    fn up(&mut self) -> bool {
        if self.trail.len() <= 1 {
            return false;
        }
        self.trail.pop();
        // Out of the search results: the next search starts from here.
        if self.search_at.is_some_and(|at| self.trail.len() <= at) {
            self.search_at = None;
        }
        true
    }
}

/// One book, as the catalog describes it.
struct BookPage {
    /// The book, and its other editions when the catalog has several files
    /// of it ("with images", "no images").
    editions: Vec<Book>,
    /// Other feeds from here: more by the author, its subjects…
    related: Vec<Folder>,
    /// The folder was just this book (a search result opens its own feed):
    /// Back leaves the folder too.
    whole_feed: bool,
}

fn load_task(account: opds::Account, url: String, generation: u64, more: bool) -> Task<Message> {
    cosmic::task::future(async move {
        let result = opds::fetch_feed(&account, &url).await;
        Message::Opds(if more { OpdsMsg::MoreLoaded(generation, result) } else { OpdsMsg::Loaded(generation, result) })
    })
}

/// A feed that's one book, maybe in a few editions, all with one title.
fn single_book(feed: &Feed) -> bool {
    let Some(first) = feed.books.first() else { return false };
    let title = crate::naming::normalize_title(&first.title);
    feed.books.len() <= 4 && feed.books.iter().all(|b| crate::naming::normalize_title(&b.title) == title)
}

fn book_page(books: Vec<Book>, extra: &[Folder], whole_feed: bool) -> BookPage {
    let mut related: Vec<Folder> = Vec::new();
    for f in books.iter().flat_map(|b| b.related.iter()).chain(extra) {
        if !related.iter().any(|r| r.url == f.url) {
            related.push(f.clone());
        }
    }
    BookPage { editions: books, related, whole_feed }
}

impl App {
    fn browse_account(&self) -> Option<opds::Account> {
        let b = self.op.browse.as_ref()?;
        self.opds.find(&b.account_key).cloned()
    }

    /// The catalogs added as catalogs (not libraries).
    fn catalogs(&self) -> impl Iterator<Item = &opds::Account> {
        self.opds.accounts.iter().filter(|a| a.is_catalog())
    }

    pub(super) fn has_catalogs(&self) -> bool {
        self.catalogs().next().is_some()
    }

    /// The catalog the Browse Catalogs button opens: the last one browsed.
    pub(super) fn catalog_to_browse(&self) -> Option<&opds::Account> {
        let last = self.op.last_catalog.as_deref();
        self.catalogs().find(|a| Some(a.key().as_str()) == last).or_else(|| self.catalogs().next())
    }

    /// Loads the folder at the end of the trail.
    fn load_current(&mut self) -> Task<Message> {
        let Some(account) = self.browse_account() else { return Task::none() };
        let Some(b) = self.op.browse.as_mut() else { return Task::none() };
        let Some((title, url)) = b.trail.last().cloned() else { return Task::none() };
        self.op.generation += 1;
        b.loading = true;
        b.error = None;
        b.feed = None;
        b.book = None;
        self.op.confirm = None;
        self.set_header_title(title);
        self.op.list_offset = 0.0;
        Task::batch([load_task(account, url, self.op.generation, false), to_top()])
    }

    /// Whether the Catalog page's Back has somewhere to go (out of a
    /// book's page, or up a folder), for Esc.
    pub(super) fn catalog_can_go_back(&self) -> bool {
        self.op.browse.as_ref().is_some_and(|b| b.book.is_some() || b.trail.len() > 1)
    }

    /// Starts a sync of a catalog, or one after the running one.
    fn sync_opds(&mut self, key: String) -> Task<Message> {
        if self.jf.syncing.contains(&key) {
            self.op.resync.insert(key);
            return Task::none();
        }
        Task::done(cosmic::Action::App(Message::Jf(JfMsg::Sync(key))))
    }

    /// Fetches the pictures not loaded yet (covers, thumbnails).
    fn load_thumbs(&mut self, urls: Vec<String>) -> Task<Message> {
        let Some(account) = self.browse_account() else { return Task::none() };
        if self.op.thumbs.len() + urls.len() > MAX_THUMBS {
            self.op.thumbs.clear();
        }
        let mut tasks = Vec::new();
        for url in urls {
            if self.op.thumbs.contains_key(&url) {
                continue;
            }
            self.op.thumbs.insert(url.clone(), None);
            let account = account.clone();
            tasks.push(cosmic::task::future(async move {
                let image = account.image(&url).await.ok();
                Message::Opds(OpdsMsg::Thumb(url, image))
            }));
        }
        Task::batch(tasks)
    }

    /// The pictures a folder's list shows (only for catalogs: a library's
    /// folders are for picking, not browsing).
    fn feed_thumbs(&mut self, feed: &Feed) -> Task<Message> {
        if !self.browse_account().is_some_and(|a| a.is_catalog()) {
            return Task::none();
        }
        let urls = feed
            .folders
            .iter()
            .filter_map(|f| f.cover.clone())
            .chain(feed.books.iter().filter_map(|b| b.links.cover.clone()))
            .collect();
        self.load_thumbs(urls)
    }

    /// Books downloaded from a catalog whose download was removed, stopped
    /// or failed leave the library: a catalog keeps only its downloads.
    pub(super) fn forget_catalog_books(&mut self, issues: &[String]) -> Task<Message> {
        let mut changed: Vec<String> = Vec::new();
        for issue in issues {
            let Some((account_key, item_id)) = opds::parse_key(issue) else { continue };
            let Some(a) = self.opds.find_mut(account_key).filter(|a| a.is_catalog()) else { continue };
            let before = a.saved.len();
            a.saved.retain(|b| opds::book_id(&b.id) != item_id);
            if a.saved.len() != before && !changed.iter().any(|k| k == account_key) {
                changed.push(account_key.to_string());
            }
        }
        if changed.is_empty() {
            return Task::none();
        }
        self.opds.save();
        let tasks: Vec<Task<Message>> = changed.into_iter().map(|k| self.sync_opds(k)).collect();
        Task::batch(tasks)
    }

    pub(super) fn update_opds(&mut self, msg: OpdsMsg) -> Task<Message> {
        match msg {
            OpdsMsg::UrlInput(s) => self.op.url_input = s,
            OpdsMsg::UserInput(s) => self.op.user_input = s,
            OpdsMsg::PasswordInput(s) => self.op.password_input = s,
            OpdsMsg::Connect => {
                let url = self.op.url_input.trim().to_string();
                if url.is_empty() || self.op.busy {
                    return Task::none();
                }
                let (user, password) = (self.op.user_input.clone(), self.op.password_input.clone());
                self.op.busy = true;
                self.op.pending = None;
                self.op.status = Some(fl!("opds-connecting", address = url.as_str()));
                return cosmic::task::future(async move {
                    Message::Opds(OpdsMsg::Connected(opds::connect(&url, &user, &password).await))
                });
            }
            OpdsMsg::Connected(result) => {
                self.op.busy = false;
                match result {
                    // Asks how to add it.
                    Ok(account) => {
                        self.op.status = None;
                        self.op.pending = Some(account);
                    }
                    Err(e) => self.op.status = Some(e),
                }
            }
            OpdsMsg::AddAs(kind) => {
                let Some(mut account) = self.op.pending.take() else { return Task::none() };
                account.kind = kind;
                // Added again the same way: keep its folders and downloads.
                if let Some(old) = self.opds.find(&account.key()) {
                    account.roots = old.roots.clone();
                    account.saved = old.saved.clone();
                }
                self.op.url_input.clear();
                self.op.user_input.clear();
                self.op.password_input.clear();
                self.op.status = Some(match kind {
                    Kind::Library => fl!("opds-added", catalog = account.name()),
                    Kind::Catalog => fl!("opds-added-catalog", catalog = account.name()),
                });
                let key = account.key();
                self.opds.upsert(account);
                return self.update_opds(OpdsMsg::Browse(key));
            }
            OpdsMsg::CancelAdd => {
                self.op.pending = None;
                self.op.status = None;
            }
            OpdsMsg::Remove(key) => {
                let Some(account) = self.opds.find(&key).cloned() else { return Task::none() };
                self.opds.remove(&key);
                if let Some(db) = &self.db {
                    library::remove_series_under_root_prefix(db, &account.item_key(""));
                }
                // Its downloads go with it.
                self.prune_downloads();
                self.jf.offline.remove(&key);
                self.op.status = Some(fl!("opds-removed", catalog = account.name()));
                if self.op.browse.as_ref().is_some_and(|b| b.account_key == key) {
                    self.op.browse = None;
                    if self.app_view == AppView::Catalog {
                        self.app_view = AppView::Library;
                    }
                }
                return self.refresh_views();
            }
            OpdsMsg::Browse(key) => {
                let Some(account) = self.opds.find(&key) else { return Task::none() };
                if account.is_catalog() {
                    self.op.last_catalog = Some(key.clone());
                }
                self.op.browse = Some(Browse {
                    trail: vec![(account.name().to_string(), account.url.clone())],
                    account_key: key,
                    feed: None,
                    loading: false,
                    error: None,
                    search: None,
                    search_input: String::new(),
                    search_at: None,
                    book: None,
                });
                self.op.thumbs.clear();
                self.set_show_context(false);
                self.series_page = None;
                self.app_view = AppView::Catalog;
                return self.load_current();
            }
            OpdsMsg::BrowseCatalogs => {
                if let Some(key) = self.catalog_to_browse().map(opds::Account::key) {
                    return self.update_opds(OpdsMsg::Browse(key));
                }
            }
            OpdsMsg::Open(title, url) => {
                if let Some(b) = self.op.browse.as_mut() {
                    b.trail.push((title, url));
                    return self.load_current();
                }
            }
            OpdsMsg::Back => {
                let Some(b) = self.op.browse.as_mut() else { return Task::none() };
                if let Some(page) = b.book.take() {
                    // A folder that was only this book goes too.
                    if page.whole_feed && b.up() {
                        return self.load_current();
                    }
                    // Back to the list where it was.
                    return scroll_mod::scroll_to(catalog_scroll_id(), AbsoluteOffset { x: None, y: Some(self.op.list_offset) });
                }
                if b.up() {
                    return self.load_current();
                }
            }
            OpdsMsg::Loaded(generation, result) => {
                let catalog = self.browse_account().is_some_and(|a| a.is_catalog());
                let Some(b) = self.op.browse.as_mut().filter(|_| generation == self.op.generation) else {
                    return Task::none();
                };
                b.loading = false;
                match result {
                    Ok(feed) => {
                        // The start feed's search is kept for the whole catalog.
                        if feed.search.is_some() {
                            b.search = feed.search.clone();
                        }
                        // In a catalog, a folder that's one book is its page.
                        if catalog && single_book(&feed) {
                            b.book = Some(book_page(feed.books.clone(), &feed.folders, true));
                        }
                        let thumbs = self.feed_thumbs(&feed);
                        if let Some(b) = self.op.browse.as_mut() {
                            b.feed = Some(feed);
                        }
                        return Task::batch([thumbs, to_top()]);
                    }
                    Err(e) => b.error = Some(e),
                }
            }
            OpdsMsg::More => {
                let Some(account) = self.browse_account() else { return Task::none() };
                let Some(b) = self.op.browse.as_mut() else { return Task::none() };
                let Some(next) = b.feed.as_ref().and_then(|f| f.next.clone()) else { return Task::none() };
                if b.loading {
                    return Task::none();
                }
                b.loading = true;
                return load_task(account, next, self.op.generation, true);
            }
            OpdsMsg::MoreLoaded(generation, result) => {
                let Some(b) = self.op.browse.as_mut().filter(|_| generation == self.op.generation) else {
                    return Task::none();
                };
                b.loading = false;
                match (result, b.feed.as_mut()) {
                    (Ok(more), Some(feed)) => {
                        feed.folders.extend(more.folders.iter().cloned());
                        feed.books.extend(more.books.iter().cloned());
                        feed.next = more.next.clone();
                        return self.feed_thumbs(&more);
                    }
                    (Err(e), _) => b.error = Some(e),
                    _ => {}
                }
            }
            OpdsMsg::SearchInput(s) => {
                if let Some(b) = self.op.browse.as_mut() {
                    b.search_input = s;
                }
            }
            OpdsMsg::Search => {
                let Some(account) = self.browse_account() else { return Task::none() };
                let Some(b) = self.op.browse.as_mut() else { return Task::none() };
                let query = b.search_input.trim().to_string();
                let Some(search) = b.search.clone().filter(|_| !query.is_empty()) else { return Task::none() };
                self.op.generation += 1;
                let generation = self.op.generation;
                b.loading = true;
                return cosmic::task::future(async move {
                    let url = opds::search_url(&account, &search, &query).await;
                    Message::Opds(OpdsMsg::SearchFound(generation, query, url))
                });
            }
            OpdsMsg::SearchFound(generation, query, result) => {
                let Some(b) = self.op.browse.as_mut().filter(|_| generation == self.op.generation) else {
                    return Task::none();
                };
                match result {
                    Ok(url) => {
                        b.push_search(fl!("opds-search-results", query = query.as_str()), url);
                        return self.load_current();
                    }
                    Err(e) => {
                        b.loading = false;
                        b.error = Some(e);
                    }
                }
            }
            OpdsMsg::ToggleRoot(title, url) => {
                let Some(b) = self.op.browse.as_ref() else { return Task::none() };
                let key = b.account_key.clone();
                let Some(account) = self.opds.find(&key) else { return Task::none() };
                let adding = !account.roots.iter().any(|r| r.url == url);
                let confirmed = self.op.confirm.as_ref().is_some_and(|c| c.url == url);
                if adding && !confirmed {
                    // How big it is, when the catalog says: the folder's own
                    // count, or the total of the folder being shown.
                    let feed = b.feed.as_ref();
                    let count = feed
                        .and_then(|f| f.folders.iter().find(|x| x.url == url))
                        .and_then(|x| x.count)
                        .or_else(|| feed.filter(|_| b.trail.last().is_some_and(|(_, u)| *u == url)).and_then(|f| f.total));
                    let whole = url == account.url;
                    if whole || count.is_some_and(|n| n >= BIG_FOLDER) {
                        self.op.confirm = Some(Confirm { title, url, count, whole });
                        return Task::none();
                    }
                }
                self.op.confirm = None;
                let Some(account) = self.opds.find_mut(&key) else { return Task::none() };
                if adding {
                    account.roots.push(Root { url, title });
                } else {
                    account.roots.retain(|r| r.url != url);
                }
                self.opds.save();
                return self.sync_opds(key);
            }
            OpdsMsg::ConfirmRoot => {
                if let Some(c) = self.op.confirm.clone() {
                    return self.update_opds(OpdsMsg::ToggleRoot(c.title, c.url));
                }
            }
            OpdsMsg::CancelRoot => self.op.confirm = None,
            OpdsMsg::OpenInBrowser(url) => crate::platform::open_url(&url),
            OpdsMsg::Facet(url) => {
                // The same folder, seen another way: it replaces the one
                // shown rather than going a level deeper.
                if let Some((_, last)) = self.op.browse.as_mut().and_then(|b| b.trail.last_mut()) {
                    *last = url;
                    return self.load_current();
                }
            }
            OpdsMsg::ShowBook(i) => {
                let Some(b) = self.op.browse.as_mut() else { return Task::none() };
                let Some(book) = b.feed.as_ref().and_then(|f| f.books.get(i)).cloned() else { return Task::none() };
                let cover = book.links.cover.clone();
                b.book = Some(book_page(vec![book], &[], false));
                return Task::batch([self.load_thumbs(cover.into_iter().collect()), to_top()]);
            }
            OpdsMsg::ListScrolled(v) => {
                if self.op.browse.as_ref().is_some_and(|b| b.book.is_none()) {
                    self.op.list_offset = v.absolute_offset().y;
                }
            }
            OpdsMsg::Thumb(url, image) => {
                if let Some(slot) = self.op.thumbs.get_mut(&url) {
                    *slot = image.map(widget::image::Handle::from_bytes);
                }
            }
            OpdsMsg::Download(i) => {
                let Some(account) = self.browse_account().filter(|a| a.is_catalog()) else { return Task::none() };
                let Some(book) = self.op.browse.as_ref().and_then(|b| b.book.as_ref()).and_then(|p| p.editions.get(i)).cloned()
                else {
                    return Task::none();
                };
                if !book.links.readable_file() {
                    return Task::none();
                }
                let saved = SavedBook::from(&book);
                let key = account.key();
                let item_id = opds::book_id(&book.id);
                let issue_key = account.item_key(&item_id);
                let series_key = account.saved_series_key(&saved);
                if let Some(a) = self.opds.find_mut(&key) {
                    a.saved.retain(|b| b.id != book.id);
                    a.saved.push(saved);
                }
                self.opds.save();
                // Known before the sync that lists it, so the download can start.
                opds::add_links(&key, &item_id, book.links.clone());
                self.jf.extensions.insert(item_id, book.links.ext.clone());
                let sync = self.sync_opds(key);
                return Task::batch([
                    sync,
                    Task::done(cosmic::Action::App(Message::Dl(DlMsg::Download(vec![(issue_key, series_key)])))),
                ]);
            }
            OpdsMsg::Read(id) => {
                let Some(account) = self.browse_account() else { return Task::none() };
                let Some(saved) = account.saved.iter().find(|b| b.id == id) else { return Task::none() };
                let series = account.saved_series_key(saved);
                let issue = account.item_key(&opds::book_id(&id));
                let chapter = self
                    .db
                    .as_ref()
                    .and_then(|db| library::issues_for(db, &series).iter().position(|i| i.path == issue))
                    .unwrap_or(0);
                return Task::done(cosmic::Action::App(Message::OpenEntry { key: series, chapter: Some(chapter) }));
            }
            OpdsMsg::RemoveSaved(id) => {
                let Some(account) = self.browse_account() else { return Task::none() };
                let issue = account.item_key(&opds::book_id(&id));
                return Task::done(cosmic::Action::App(Message::Dl(DlMsg::Remove(vec![issue]))));
            }
        }
        Task::none()
    }

    /// Settings: the catalogs added, and adding one.
    pub(super) fn build_opds_section(&self) -> Element<'_, Message> {
        let mut section = widget::settings::section().header(
            widget::row::with_capacity(2)
                .spacing(8)
                .align_y(Alignment::Center)
                .push(widget::icon(super::library_view::opds_icon()).size(16))
                .push(widget::text::heading("OPDS")),
        );
        for account in &self.opds.accounts {
            let key = account.key();
            let (button, tip) = if account.is_catalog() {
                (fl!("opds-browse"), fl!("opds-as-catalog", count = account.saved.len()))
            } else {
                let folders = if account.roots.is_empty() {
                    fl!("opds-no-folders")
                } else {
                    fl!("opds-folder-count", count = account.roots.len())
                };
                (fl!("opds-choose-folders"), format!("{}\n{folders}", fl!("opds-as-library")))
            };
            let open = widget::tooltip(
                widget::button::standard(button).on_press(Message::Opds(OpdsMsg::Browse(key.clone()))),
                widget::text::body(tip),
                widget::tooltip::Position::Top,
            );
            let remove = widget::button::standard(fl!("opds-remove")).on_press(Message::Opds(OpdsMsg::Remove(key)));
            section = section.add(self.remote_account_row(&RemoteAccount::Opds(account.clone()), vec![open.into()], remove.into()));
        }
        // Download settings live with Jellyfin's when there's a Jellyfin
        // server; here when there are only catalogs.
        if self.jellyfin.accounts.is_empty() && !self.opds.accounts.is_empty() {
            section = section.add(self.downloads_settings());
        }
        if let Some(pending) = &self.op.pending {
            return section.add(add_as_choice(pending)).into();
        }
        let busy = self.op.busy;
        section = section.add(
            widget::Column::new()
                .spacing(8)
                .push(widget::text::caption(fl!("opds-help")))
                .push(
                    widget::text_input(fl!("opds-address"), &self.op.url_input)
                        .on_input(|s| Message::Opds(OpdsMsg::UrlInput(s)))
                        .on_submit(|_| Message::Opds(OpdsMsg::Connect)),
                )
                .push(
                    widget::Row::new()
                        .spacing(8)
                        .push(
                            widget::text_input(fl!("opds-username-optional"), &self.op.user_input)
                                .on_input(|s| Message::Opds(OpdsMsg::UserInput(s)))
                                .width(Length::Fill),
                        )
                        .push(
                            widget::secure_input(fl!("opds-password-optional"), &self.op.password_input, None, true)
                                .on_input(|s| Message::Opds(OpdsMsg::PasswordInput(s)))
                                .on_submit(|_| Message::Opds(OpdsMsg::Connect))
                                .width(Length::Fill),
                        ),
                )
                .push(
                    widget::button::suggested(fl!("connect"))
                        .on_press_maybe((!busy && !self.op.url_input.trim().is_empty()).then_some(Message::Opds(OpdsMsg::Connect))),
                ),
        );
        if let Some(status) = &self.op.status {
            section = section.add(widget::text::caption(status.clone()));
        }
        section.into()
    }

    /// A cover or thumbnail from the Catalog page's pictures, or a symbol
    /// while there's none.
    fn thumb<'a>(&self, url: Option<&str>, fallback: &'static str, w: f32, h: f32) -> Element<'a, Message> {
        match url.and_then(|u| self.op.thumbs.get(u)).cloned().flatten() {
            Some(handle) => widget::image(handle).width(Length::Fixed(w)).height(Length::Fixed(h)).content_fit(ContentFit::Contain).into(),
            None => widget::container(icon::from_name(fallback).size((w.min(h) * 0.6) as u16))
                .width(Length::Fixed(w))
                .height(Length::Fixed(h))
                .center_x(Length::Fixed(w))
                .center_y(Length::Fixed(h))
                .into(),
        }
    }

    /// The Catalog page: one folder of a catalog, its folders and books, and
    /// the catalog's search; or a book's page.
    pub(super) fn view_catalog(&self) -> Element<'_, Message> {
        let mut col = widget::Column::new().spacing(16).padding([16, 20]);
        let (Some(b), Some(account)) = (self.op.browse.as_ref(), self.browse_account()) else {
            return col.into();
        };
        let catalog = account.is_catalog();
        if let (true, Some(page)) = (catalog, b.book.as_ref()) {
            return self.view_book_page(&account, page, b.trail.len() > 1 || !page.whole_feed);
        }
        let roots = &account.roots;
        let in_library = |url: &str| roots.iter().any(|r| r.url == url);

        // Other catalogs, to switch to.
        let others: Vec<&opds::Account> = if catalog { self.catalogs().collect() } else { Vec::new() };
        if others.len() > 1 {
            let mut row = widget::row::with_capacity(others.len()).spacing(8);
            for a in others {
                let key = a.key();
                let button = if key == b.account_key {
                    widget::button::suggested(a.name().to_string())
                } else {
                    widget::button::standard(a.name().to_string())
                };
                row = row.push(button.on_press(Message::Opds(OpdsMsg::Browse(key))));
            }
            col = col.push(widget::scrollable::horizontal(row));
        }

        // Where we are: Back, the trail, and this folder's own toggle.
        let (title, url) = b.trail.last().cloned().unwrap_or_default();
        let crumbs = b.trail.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join("  ›  ");
        let mut top = widget::row::with_capacity(4).spacing(12).align_y(Alignment::Center);
        if b.trail.len() > 1 {
            top = top.push(
                widget::button::icon(icon::from_name("go-previous-symbolic"))
                    .tooltip(fl!("opds-back"))
                    .on_press(Message::Opds(OpdsMsg::Back)),
            );
        }
        top = top.push(
            widget::column::with_capacity(2)
                .width(Length::Fill)
                .push(widget::text::title3(title.clone()))
                .push(widget::text::caption(crumbs)),
        );
        if !catalog && b.feed.as_ref().is_some_and(|f| !f.books.is_empty()) {
            top = top.push(root_toggle(in_library(&url), &title, &url));
        }
        col = col.push(top);

        if let Some(c) = &self.op.confirm {
            col = col.push(confirm_banner(c));
        }
        if b.search.is_some() {
            col = col.push(
                widget::search_input(fl!("opds-search", catalog = account.name()), &b.search_input)
                    .on_input(|s| Message::Opds(OpdsMsg::SearchInput(s)))
                    .on_submit(|_| Message::Opds(OpdsMsg::Search)),
            );
        }
        if let Some(feed) = &b.feed {
            for group in &feed.facets {
                col = col.push(facet_row(group));
            }
        }
        if b.trail.len() == 1 {
            col = col.push(widget::text::body(if catalog { fl!("opds-catalog-help", catalog = account.name()) } else { fl!("opds-pick-help") }));
            col = col.push(widget::text::caption(fl!("opds-untested")));
        }
        if let Some(e) = &b.error {
            // A "folder" that's really a web page (Gutenberg lists its
            // Facebook and Mastodon pages as feeds): offer the browser.
            let web_page = *e == fl!("opds-not-a-catalog");
            let message = if web_page { fl!("opds-web-page") } else { fl!("opds-folder-failed", error = e.as_str()) };
            col = col.push(
                widget::row::with_capacity(3)
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(icon::from_name(if web_page { "web-browser-symbolic" } else { "dialog-warning-symbolic" }).size(16))
                    .push(widget::text::body(message))
                    .push_maybe(web_page.then(|| {
                        widget::button::standard(fl!("opds-open-in-browser")).on_press(Message::Opds(OpdsMsg::OpenInBrowser(url.clone())))
                    })),
            );
        }

        if let Some(feed) = &b.feed {
            for f in &feed.folders {
                let mut facts = Vec::new();
                if !f.subtitle.is_empty() {
                    facts.push(f.subtitle.clone());
                }
                if let Some(n) = f.count {
                    facts.push(fl!("opds-book-count", count = n));
                }
                if in_library(&f.url) {
                    facts.push(fl!("opds-in-library"));
                }
                let picture: Element<'_, Message> = if catalog && f.cover.is_some() {
                    self.thumb(f.cover.as_deref(), "folder-symbolic", 36.0, 52.0)
                } else {
                    icon::from_name("folder-symbolic").size(20).into()
                };
                let row = widget::row::with_capacity(4)
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .padding([6, 8])
                    .push(picture)
                    .push(
                        widget::column::with_capacity(2)
                            .spacing(2)
                            .width(Length::Fill)
                            .push(widget::text::heading(f.title.clone()))
                            .push_maybe((!facts.is_empty()).then(|| widget::text::caption(facts.join("  ·  ")))),
                    )
                    .push_maybe((!catalog).then(|| root_toggle(in_library(&f.url), &f.title, &f.url)))
                    .push(icon::from_name("go-next-symbolic").size(16));
                col = col.push(
                    widget::mouse_area(row)
                        .on_press(Message::Opds(OpdsMsg::Open(f.title.clone(), f.url.clone())))
                        .interaction(cosmic::iced::mouse::Interaction::Pointer),
                );
            }
            if !feed.books.is_empty() {
                col = col.push(widget::text::heading(fl!("opds-book-count", count = feed.books.len())));
            }
            for (i, book) in feed.books.iter().enumerate() {
                let mut facts = Vec::new();
                if !book.authors.is_empty() {
                    facts.push(book.authors.join(", "));
                }
                if !book.date.is_empty() {
                    facts.push(book.date.clone());
                }
                if !book.links.ext.is_empty() {
                    facts.push(book.links.ext.to_uppercase());
                }
                let readable = if catalog { book.links.readable_file() } else { book.links.readable() };
                if !readable {
                    facts.push(fl!("opds-not-readable"));
                }
                if catalog && account.has_saved(&book.id) {
                    facts.push(fl!("downloaded"));
                }
                if catalog {
                    // A catalog's books open their page.
                    let row = widget::row::with_capacity(4)
                        .spacing(12)
                        .align_y(Alignment::Center)
                        .padding([4, 8])
                        .push(self.thumb(book.links.cover.as_deref(), "x-office-document-symbolic", 36.0, 52.0))
                        .push(
                            widget::column::with_capacity(2)
                                .spacing(2)
                                .width(Length::Fill)
                                .push(widget::text::heading(book.title.clone()))
                                .push_maybe((!facts.is_empty()).then(|| widget::text::caption(facts.join("  ·  ")))),
                        )
                        .push(icon::from_name("go-next-symbolic").size(16));
                    col = col.push(
                        widget::mouse_area(row)
                            .on_press(Message::Opds(OpdsMsg::ShowBook(i)))
                            .interaction(cosmic::iced::mouse::Interaction::Pointer),
                    );
                    continue;
                }
                let external = (!readable).then_some(book.external.clone()).flatten();
                col = col.push(
                    widget::row::with_capacity(3)
                        .spacing(12)
                        .align_y(Alignment::Center)
                        .padding([2, 8, 2, 40])
                        .push(
                            widget::column::with_capacity(2)
                                .spacing(2)
                                .width(Length::Fill)
                                .push(widget::text::body(book.title.clone()))
                                .push_maybe((!facts.is_empty()).then(|| widget::text::caption(facts.join("  ·  ")))),
                        )
                        .push_maybe(external.map(|u| {
                            widget::button::standard(fl!("opds-open-in-browser")).on_press(Message::Opds(OpdsMsg::OpenInBrowser(u)))
                        })),
                );
            }
            if feed.next.is_some() {
                col = col.push(
                    widget::button::standard(fl!("opds-load-more"))
                        .on_press_maybe((!b.loading).then_some(Message::Opds(OpdsMsg::More))),
                );
            }
        }
        if b.loading {
            col = col.push(widget::text::body(fl!("opds-loading")));
        }
        widget::scrollable(col)
            .id(catalog_scroll_id())
            .on_scroll(|v| Message::Opds(OpdsMsg::ListScrolled(v)))
            .into()
    }

    /// A catalog book's page: what the catalog says about it, and its
    /// download (one per edition when it has several files).
    fn view_book_page<'a>(&'a self, account: &opds::Account, page: &'a BookPage, back: bool) -> Element<'a, Message> {
        let mut col = widget::Column::new().spacing(16).padding([16, 20]);
        let Some(first) = page.editions.first() else { return col.into() };

        let mut top = widget::row::with_capacity(2).spacing(12).align_y(Alignment::Center);
        if back {
            top = top.push(
                widget::button::icon(icon::from_name("go-previous-symbolic"))
                    .tooltip(fl!("opds-back"))
                    .on_press(Message::Opds(OpdsMsg::Back)),
            );
        }
        col = col.push(top.push(widget::text::caption(account.name().to_string()).width(Length::Fill)));

        // What we know: from whichever edition says the most.
        let summary = page.editions.iter().map(|e| e.summary.as_str()).max_by_key(|s| s.len()).unwrap_or("");
        let cover = page.editions.iter().find_map(|e| e.links.cover.as_deref());
        let mut facts = Vec::new();
        if let Some((name, pos)) = &first.series {
            facts.push(match pos {
                Some(n) => format!("{name} #{n}"),
                None => name.clone(),
            });
        }
        if !first.date.is_empty() {
            facts.push(first.date.clone());
        }
        let mut info = widget::column::with_capacity(5)
            .spacing(6)
            .width(Length::Fill)
            .push(widget::text::title3(first.title.clone()));
        if !first.authors.is_empty() {
            info = info.push(widget::text::body(first.authors.join(", ")));
        }
        if !facts.is_empty() {
            info = info.push(widget::text::caption(facts.join("  ·  ")));
        }
        // One edition: its button goes here; several: listed below.
        if page.editions.len() == 1 {
            info = info.push(widget::container(self.edition_actions(account, first, 0)).padding([6, 0, 0, 0]));
        }
        col = col.push(
            widget::row::with_capacity(2)
                .spacing(20)
                .push(self.thumb(cover, "x-office-document-symbolic", 160.0, 240.0))
                .push(info),
        );

        if page.editions.len() > 1 {
            col = col.push(widget::text::heading(fl!("opds-editions")));
            for (i, e) in page.editions.iter().enumerate() {
                col = col.push(
                    widget::row::with_capacity(2)
                        .spacing(12)
                        .align_y(Alignment::Center)
                        .padding([2, 8])
                        .push(widget::text::body(edition_label(e)).width(Length::Fill))
                        .push(self.edition_actions(account, e, i)),
                );
            }
        }
        if !summary.is_empty() {
            col = col.push(widget::text::body(summary.to_string()));
        }
        if !page.related.is_empty() {
            col = col.push(widget::text::heading(fl!("opds-related")));
            for f in &page.related {
                let row = widget::row::with_capacity(3)
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .padding([4, 8])
                    .push(icon::from_name("folder-symbolic").size(16))
                    .push(widget::text::body(f.title.trim_end_matches('…').to_string()).width(Length::Fill))
                    .push(icon::from_name("go-next-symbolic").size(16));
                col = col.push(
                    widget::mouse_area(row)
                        .on_press(Message::Opds(OpdsMsg::Open(f.title.trim_end_matches('…').to_string(), f.url.clone())))
                        .interaction(cosmic::iced::mouse::Interaction::Pointer),
                );
            }
        }
        widget::scrollable(col).id(catalog_scroll_id()).into()
    }

    /// Download, its progress, or Read / Remove Download for one edition.
    fn edition_actions<'a>(&self, account: &opds::Account, book: &Book, i: usize) -> Element<'a, Message> {
        if !book.links.readable_file() {
            let mut row = widget::row::with_capacity(2).spacing(8).align_y(Alignment::Center);
            row = row.push(widget::text::caption(fl!("opds-not-readable")));
            if let Some(u) = book.external.clone().or_else(|| book.links.file.clone()) {
                row = row.push(widget::button::standard(fl!("opds-open-in-browser")).on_press(Message::Opds(OpdsMsg::OpenInBrowser(u))));
            }
            return row.into();
        }
        let issue = account.item_key(&opds::book_id(&book.id));
        let state = if account.has_saved(&book.id) { self.dl.state(&issue) } else { DlState::None };
        let remove = Message::Opds(OpdsMsg::RemoveSaved(book.id.clone()));
        let busy = |ring: Element<'a, Message>, label: String| -> Element<'a, Message> {
            widget::button::custom(
                widget::row::with_capacity(2).spacing(8).align_y(Alignment::Center).push(ring).push(widget::text::body(label)),
            )
            .class(cosmic::theme::Button::Standard)
            .on_press(remove.clone())
            .into()
        };
        match state {
            DlState::None => {
                let label = match book.links.size {
                    Some(size) => fl!("opds-download-size", size = human_size(size)),
                    None => fl!("opds-download"),
                };
                widget::button::suggested(label)
                    .leading_icon(icon::from_name("folder-download-symbolic"))
                    .on_press(Message::Opds(OpdsMsg::Download(i)))
                    .into()
            }
            DlState::Queued => busy(widget::indeterminate_circular().size(14.0).bar_height(2.0).into(), fl!("dl-waiting-cancel")),
            DlState::Running(f) => busy(
                widget::determinate_circular(f).size(14.0).bar_height(2.0).into(),
                fl!("dl-running-cancel", percent = format!("{:.0}", f * 100.0)),
            ),
            DlState::Done => widget::row::with_capacity(2)
                .spacing(8)
                .push(widget::button::suggested(fl!("read")).on_press(Message::Opds(OpdsMsg::Read(book.id.clone()))))
                .push(widget::button::standard(fl!("remove-download")).on_press(remove))
                .into(),
            DlState::Unavailable => widget::tooltip(
                widget::button::standard(fl!("downloaded")).leading_icon(icon::from_name("drive-removable-media-symbolic")),
                widget::text(fl!("dl-on-missing-drive")),
                widget::tooltip::Position::Bottom,
            )
            .into(),
        }
    }
}

/// An edition's name: the catalog's ("EPUB3 (E-readers…)") or its type,
/// and its size.
fn edition_label(book: &Book) -> String {
    let name = if book.links.label.is_empty() { book.links.ext.to_uppercase() } else { book.links.label.clone() };
    match book.links.size {
        Some(size) => format!("{name}  ·  {}", human_size(size)),
        None => name,
    }
}

/// Settings: how to add a catalog that answered, with the likelier way
/// first: one you sign in to is probably your own server.
fn add_as_choice<'a>(account: &opds::Account) -> Element<'a, Message> {
    let recommended = if account.user.is_empty() { Kind::Catalog } else { Kind::Library };
    let radius = cosmic::theme::active().cosmic().corner_radii.radius_s;
    let card = |kind: Kind| -> Element<'a, Message> {
        let (title, body) = match kind {
            Kind::Library => (fl!("opds-add-as-library"), fl!("opds-add-as-library-help")),
            Kind::Catalog => (fl!("opds-add-as-catalog"), fl!("opds-add-as-catalog-help")),
        };
        let mut content = widget::column::with_capacity(3)
            .spacing(4)
            .padding(8)
            .width(Length::Fill)
            .push(widget::text::heading(title))
            .push(widget::text::caption(body));
        if kind == recommended {
            content = content.push(widget::text::caption_heading(fl!("opds-recommended")));
        }
        // A card, rounded like the settings around it, not a pill.
        widget::button::custom(content)
            .class(cosmic::theme::Button::ListItem(radius))
            .width(Length::Fill)
            .on_press(Message::Opds(OpdsMsg::AddAs(kind)))
            .into()
    };
    let (first, second) = if recommended == Kind::Library { (Kind::Library, Kind::Catalog) } else { (Kind::Catalog, Kind::Library) };
    widget::Column::new()
        .spacing(8)
        .push(widget::text::body(fl!("opds-how-to-add", catalog = account.name())))
        .push(card(first))
        .push(card(second))
        .push(widget::button::link(fl!("cancel")).on_press(Message::Opds(OpdsMsg::CancelAdd)))
        .into()
}

/// Asks before a very big folder, or the whole catalog, goes in a library.
fn confirm_banner<'a>(c: &Confirm) -> Element<'a, Message> {
    let text = if c.whole {
        fl!("opds-big-whole")
    } else {
        fl!("opds-big-folder", folder = c.title.as_str(), count = c.count.unwrap_or(BIG_FOLDER))
    };
    widget::container(
        widget::column::with_capacity(2)
            .spacing(8)
            .push(
                widget::row::with_capacity(2)
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(icon::from_name("dialog-warning-symbolic").size(16))
                    .push(widget::text::body(text)),
            )
            .push(
                widget::row::with_capacity(2)
                    .spacing(8)
                    .push(widget::button::standard(fl!("cancel")).on_press(Message::Opds(OpdsMsg::CancelRoot)))
                    .push(widget::button::destructive(fl!("opds-add-anyway")).on_press(Message::Opds(OpdsMsg::ConfirmRoot))),
            ),
    )
    .padding(12)
    .class(cosmic::theme::Container::Card)
    .width(Length::Fill)
    .into()
}

/// One group of facets: its name, and a button for each way to see the
/// folder (the one shown highlighted).
fn facet_row<'a>(group: &FacetGroup) -> Element<'a, Message> {
    let buttons: Vec<Element<'a, Message>> = group
        .facets
        .iter()
        .map(|f| {
            let label = match f.count {
                Some(n) => format!("{} ({n})", f.title),
                None => f.title.clone(),
            };
            let button = if f.active { widget::button::suggested(label) } else { widget::button::standard(label) };
            button.on_press(Message::Opds(OpdsMsg::Facet(f.url.clone()))).into()
        })
        .collect();
    let mut col = widget::column::with_capacity(2).spacing(6);
    if !group.title.is_empty() {
        col = col.push(widget::text::caption_heading(group.title.clone()));
    }
    col.push(widget::flex_row(buttons).row_spacing(6).column_spacing(6)).into()
}

/// Add to Library / Remove from Library for one folder.
fn root_toggle<'a>(added: bool, title: &str, url: &str) -> Element<'a, Message> {
    let msg = Message::Opds(OpdsMsg::ToggleRoot(title.to_string(), url.to_string()));
    if added {
        widget::button::standard(fl!("opds-remove-folder")).on_press(msg).into()
    } else {
        widget::button::suggested(fl!("opds-add-folder")).on_press(msg).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browse() -> Browse {
        Browse {
            account_key: String::new(),
            trail: vec![("Catalog".into(), "/".into())],
            feed: None,
            loading: false,
            error: None,
            search: None,
            search_input: String::new(),
            search_at: None,
            book: None,
        }
    }

    fn titles(b: &Browse) -> Vec<&str> {
        b.trail.iter().map(|(t, _)| t.as_str()).collect()
    }

    #[test]
    fn searching_again_replaces_the_results() {
        let mut b = browse();
        b.push_search("hamlet".into(), "/s?q=hamlet".into());
        b.push_search("foundation".into(), "/s?q=foundation".into());
        b.push_search("foundation".into(), "/s?q=foundation".into());
        assert_eq!(titles(&b), ["Catalog", "foundation"]);
        // A folder opened from the results goes too.
        b.trail.push(("Authors".into(), "/a".into()));
        b.push_search("dracula".into(), "/s?q=dracula".into());
        assert_eq!(titles(&b), ["Catalog", "dracula"]);
    }

    #[test]
    fn a_search_from_a_folder_comes_back_to_it() {
        let mut b = browse();
        b.trail.push(("Popular".into(), "/p".into()));
        b.push_search("whale".into(), "/s?q=whale".into());
        assert_eq!(titles(&b), ["Catalog", "Popular", "whale"]);
        assert!(b.up());
        assert_eq!(titles(&b), ["Catalog", "Popular"]);
        // Out of the results, a new search is added from here.
        b.push_search("sea".into(), "/s?q=sea".into());
        assert_eq!(titles(&b), ["Catalog", "Popular", "sea"]);
        assert!(b.up() && b.up());
        assert!(!b.up());
        assert_eq!(titles(&b), ["Catalog"]);
    }
}
