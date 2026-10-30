//! OPDS catalogs in the app: adding one in Settings, and the Catalog page
//! for browsing it and picking which of its folders go in the library.
//!
//! Syncing, reading, covers, downloads and offline reading are shared with
//! Jellyfin (`jellyfin_view`, through [`crate::remote::Account`]); what's
//! here is only what OPDS needs on top: a catalog has no fixed layout, so
//! the user walks its folders and adds the ones they want.

use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, icon};
use cosmic::{app::Task, ApplicationExt, Element};

use super::jellyfin_view::JfMsg;
use super::{App, AppView, Message};
use crate::library;
use crate::opds::{self, Feed, Root, Search};
use crate::remote::Account as RemoteAccount;

#[derive(Clone, Debug)]
pub enum OpdsMsg {
    UrlInput(String),
    UserInput(String),
    PasswordInput(String),
    Connect,
    Connected(Result<opds::Account, String>),
    Remove(String),
    /// Opens the Catalog page on an account's start feed.
    Browse(String),
    /// Goes into a folder: (title, url).
    Open(String, String),
    /// Goes back up a folder.
    Back,
    Loaded(u64, Result<Feed, String>),
    More,
    MoreLoaded(u64, Result<Feed, String>),
    SearchInput(String),
    Search,
    SearchFound(u64, String, Result<String, String>),
    /// Adds a folder to the library, or takes it out: (title, url).
    ToggleRoot(String, String),
    OpenInBrowser(String),
}

#[derive(Default)]
pub struct OpdsState {
    url_input: String,
    user_input: String,
    password_input: String,
    busy: bool,
    status: Option<String>,
    pub(super) browse: Option<Browse>,
    /// Counts loads, so an answer for a folder already left is ignored.
    generation: u64,
    /// Catalogs whose folders changed while a sync was running: synced
    /// again when it ends.
    pub(super) resync: std::collections::HashSet<String>,
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
}

fn load_task(account: opds::Account, url: String, generation: u64, more: bool) -> Task<Message> {
    cosmic::task::future(async move {
        let result = opds::fetch_feed(&account, &url).await;
        Message::Opds(if more { OpdsMsg::MoreLoaded(generation, result) } else { OpdsMsg::Loaded(generation, result) })
    })
}

impl App {
    fn browse_account(&self) -> Option<opds::Account> {
        let b = self.op.browse.as_ref()?;
        self.opds.find(&b.account_key).cloned()
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
        self.set_header_title(title);
        load_task(account, url, self.op.generation, false)
    }

    /// Starts a sync of a catalog, or one after the running one.
    fn sync_opds(&mut self, key: String) -> Task<Message> {
        if self.jf.syncing.contains(&key) {
            self.op.resync.insert(key);
            return Task::none();
        }
        Task::done(cosmic::Action::App(Message::Jf(JfMsg::Sync(key))))
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
                self.op.status = Some(fl!("opds-connecting", address = url.as_str()));
                return cosmic::task::future(async move {
                    Message::Opds(OpdsMsg::Connected(opds::connect(&url, &user, &password).await))
                });
            }
            OpdsMsg::Connected(result) => {
                self.op.busy = false;
                match result {
                    Ok(mut account) => {
                        // Added again: keep the folders it had.
                        if let Some(old) = self.opds.find(&account.key()) {
                            account.roots = old.roots.clone();
                        }
                        self.op.url_input.clear();
                        self.op.user_input.clear();
                        self.op.password_input.clear();
                        self.op.status = Some(fl!("opds-added", catalog = account.name()));
                        let key = account.key();
                        self.opds.upsert(account);
                        return self.update_opds(OpdsMsg::Browse(key));
                    }
                    Err(e) => self.op.status = Some(e),
                }
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
                self.op.browse = Some(Browse {
                    account_key: key,
                    trail: vec![(account.name().to_string(), account.url.clone())],
                    feed: None,
                    loading: false,
                    error: None,
                    search: None,
                    search_input: String::new(),
                });
                self.set_show_context(false);
                self.series_page = None;
                self.app_view = AppView::Catalog;
                return self.load_current();
            }
            OpdsMsg::Open(title, url) => {
                if let Some(b) = self.op.browse.as_mut() {
                    b.trail.push((title, url));
                    return self.load_current();
                }
            }
            OpdsMsg::Back => {
                if let Some(b) = self.op.browse.as_mut().filter(|b| b.trail.len() > 1) {
                    b.trail.pop();
                    return self.load_current();
                }
            }
            OpdsMsg::Loaded(generation, result) => {
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
                        b.feed = Some(feed);
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
                        feed.folders.extend(more.folders);
                        feed.books.extend(more.books);
                        feed.next = more.next;
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
                        b.trail.push((fl!("opds-search-results", query = query.as_str()), url));
                        return self.load_current();
                    }
                    Err(e) => {
                        b.loading = false;
                        b.error = Some(e);
                    }
                }
            }
            OpdsMsg::ToggleRoot(title, url) => {
                let Some(key) = self.op.browse.as_ref().map(|b| b.account_key.clone()) else { return Task::none() };
                let Some(account) = self.opds.find_mut(&key) else { return Task::none() };
                if account.roots.iter().any(|r| r.url == url) {
                    account.roots.retain(|r| r.url != url);
                } else {
                    account.roots.push(Root { url, title });
                }
                self.opds.save();
                return self.sync_opds(key);
            }
            OpdsMsg::OpenInBrowser(url) => crate::platform::open_url(&url),
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
            let folders = if account.roots.is_empty() {
                fl!("opds-no-folders")
            } else {
                fl!("opds-folder-count", count = account.roots.len())
            };
            let choose = widget::tooltip(
                widget::button::standard(fl!("opds-choose-folders")).on_press(Message::Opds(OpdsMsg::Browse(key.clone()))),
                widget::text::body(folders),
                widget::tooltip::Position::Top,
            );
            let remove = widget::button::standard(fl!("opds-remove")).on_press(Message::Opds(OpdsMsg::Remove(key)));
            section = section.add(self.remote_account_row(&RemoteAccount::Opds(account.clone()), vec![choose.into()], remove.into()));
        }
        // Download settings live with Jellyfin's when there's a Jellyfin
        // server; here when there are only catalogs.
        if self.jellyfin.accounts.is_empty() && !self.opds.accounts.is_empty() {
            section = section.add(self.downloads_settings());
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

    /// The Catalog page: one folder of a catalog, its folders (each can be
    /// added to the library) and books, and the catalog's search.
    pub(super) fn view_catalog(&self) -> Element<'_, Message> {
        let mut col = widget::Column::new().spacing(16).padding([16, 20]);
        let (Some(b), Some(account)) = (self.op.browse.as_ref(), self.browse_account()) else {
            return col.into();
        };
        let roots = &account.roots;
        let in_library = |url: &str| roots.iter().any(|r| r.url == url);

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
        if b.feed.as_ref().is_some_and(|f| !f.books.is_empty()) {
            top = top.push(root_toggle(in_library(&url), &title, &url));
        }
        col = col.push(top);

        if b.search.is_some() {
            col = col.push(
                widget::search_input(fl!("opds-search", catalog = account.name()), &b.search_input)
                    .on_input(|s| Message::Opds(OpdsMsg::SearchInput(s)))
                    .on_submit(|_| Message::Opds(OpdsMsg::Search)),
            );
        }
        if b.trail.len() == 1 {
            col = col
                .push(widget::text::body(fl!("opds-pick-help")))
                .push(widget::text::caption(fl!("opds-untested")));
        }
        if let Some(e) = &b.error {
            col = col.push(
                widget::row::with_capacity(2)
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(icon::from_name("dialog-warning-symbolic").size(16))
                    .push(widget::text::body(fl!("opds-folder-failed", error = e.as_str()))),
            );
        }

        if let Some(feed) = &b.feed {
            for f in &feed.folders {
                let mut facts = Vec::new();
                if let Some(n) = f.count {
                    facts.push(fl!("opds-book-count", count = n));
                }
                if in_library(&f.url) {
                    facts.push(fl!("opds-in-library"));
                }
                let row = widget::row::with_capacity(4)
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .padding([6, 8])
                    .push(icon::from_name("folder-symbolic").size(20))
                    .push(
                        widget::column::with_capacity(2)
                            .spacing(2)
                            .width(Length::Fill)
                            .push(widget::text::heading(f.title.clone()))
                            .push_maybe((!facts.is_empty()).then(|| widget::text::caption(facts.join("  ·  ")))),
                    )
                    .push(root_toggle(in_library(&f.url), &f.title, &f.url))
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
            for book in &feed.books {
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
                let readable = book.links.readable();
                if !readable {
                    facts.push(fl!("opds-not-readable"));
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
        widget::scrollable(col).into()
    }
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
