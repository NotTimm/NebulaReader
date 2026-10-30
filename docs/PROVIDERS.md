# Providers

How Nebula gets comics and books from somewhere other than a folder on
disk, and how to add a new source.

> **Status: design, not built yet.** Today Jellyfin is the only online
> source and is written directly into the app. This page is the plan for
> turning it into one *provider* among several, with OPDS as the second.
> Names in the code sketches may change; the rules won't change much.
> The order of work is at the end.

## The idea

The app already does the hard parts for Jellyfin: library cards, series
pages, Continue Reading, the reader (streaming pages, loading chapters
ahead), downloads (folder choice, USB drives, `.part` + rename), offline
mode, progress queued while offline, CBR page indexes, the cover cache
and the right-click menus. None of that needs to know which server a book
came from.

A provider is the small part that does: it talks to one kind of server and
describes what's there in a shape the app understands. A new provider is
mostly an API client that fills in the structs below.

There are two ways to add a source:

1. **An OPDS bridge** (works as soon as OPDS lands, no app changes): run a
   small server that speaks OPDS and point Nebula at it. See
   [Writing an OPDS bridge](#writing-an-opds-bridge).
2. **A built-in provider** in Rust, sent as a pull request. See
   [Writing a built-in provider](#writing-a-built-in-provider).

Loadable plugins (sandboxed WASM) may come later on top of the same
interface. They'd be one more way to implement it, not a different design.

## Words

- **Provider**: a kind of source (Jellyfin, OPDS). Code in the app.
- **Account**: one server you've added ("Jellyfin – Home", "OPDS – Komga").
  A provider can have any number.
- **Series**: what a library card shows. A one-shot is a series with one
  item.
- **Item**: one readable thing (a chapter, an issue, a book).
- **Synced** account: everything is mirrored into your library.
- **Catalog** account: the server is browsed; things enter your library
  only when you add, read or download them.

## What a provider declares

```rust
pub struct ProviderInfo {
    id: &'static str,       // "jellyfin", "opds": also the key scheme
    name: String,           // fl!() for built-ins
    icon: &'static [u8],    // symbolic SVG, see Icons
    connect: ConnectForm,   // what to ask when adding an account
    caps: Caps,
}

pub struct Caps {
    sync: bool,             // can list everything (synced accounts)
    browse: bool,           // has a home page / search (catalog accounts)
    progress: Progress,     // None | ReadOnly | ReadWrite
    sessions: bool,         // on_open / on_close hooks
}
```

`ConnectForm` lists fields (server URL, user name, password) or a code
flow like Jellyfin's Quick Connect. The app draws the form. Providers
never draw their own settings UI, so every source is added the same way.

## Keys

Every series and item gets a key `<provider>://<account>/<id>`:

- `jellyfin://<serverId>-<userId>/<itemId>` stays exactly as it is.
- `opds://<account>/<id>`, where the account is a short hash of the
  catalog URL and user name and the id is the entry's `id`,
  percent-encoded (OPDS ids are often URNs or URLs).

Keys are stored in the library, downloads and progress tables, so **ids
must be stable**: if a server renames or reorders things and an id
changes, the app sees a new book, and the old one's progress and download
go with the next sync. Jellyfin item ids and OPDS entry ids are stable.

## What a provider must give

Three things for every item: where it sits, a cover, and a way to get it.

### Series and items

```rust
pub struct Series {
    id: String,
    title: String,
    group: String,          // the server's library ("Manga"); may be ""
    cover: Cover,
    items: Vec<Item>,       // in reading order
    // optional: description, creators, year, volume, status, genres
}

pub struct Item {
    id: String,
    name: String,           // what the chapter row shows
    format: Format,         // Cbz | Cbr | Epub | Other(String)
    access: Vec<Access>,
    // optional: number, volume, subtitle, release date, creators,
    // page count, progress (page, finished, last read)
}
```

The reader opens CBZ/ZIP, CBR/RAR and EPUB. Items in other formats (PDF,
CB7…) are still listed, shown as "can't be opened" rather than hidden, so
it's clear why.

### Covers

```rust
pub enum Cover {
    Url(String),            // fetched and cached by the app
    FirstPage,              // use the first item's first page
    None,                   // placeholder
}
```

### Access

How an item can be read. A provider lists every way it has; the app picks.

```rust
pub enum Access {
    Pages { count: usize, page_url: Template }, // ".../page/{n}"
    RangedFile { url: String },                 // server honours Range
    DownloadOnly { url: String },               // fetched whole
    External { url: String },                   // buy/borrow page
}
```

| Offered | The user gets |
|---|---|
| `Pages` or `RangedFile` | **Read** streams it at once; **Download** too |
| `DownloadOnly` | **Download to read**, then it opens like any other |
| only `External` | **Open in browser** |
| an EPUB | always fetched whole first, then opened (as Jellyfin EPUBs are now) |

The app doesn't trust a `RangedFile` blindly: if the server ignores the
Range header it falls back to downloading. A downloaded item always opens
from disk, whatever the provider offers.

All requests (pages, files, covers) go through the provider's HTTP client,
which adds its sign-in (Jellyfin's token header, OPDS Basic auth). The
reader, downloads and cover cache never see credentials.

## What a provider may give

Each of these switches on something:

| Given | Enables |
|---|---|
| number, volume, dates, creators, description | volume grouping in the chapter list, Properties, the series page |
| page count per item | progress bars before a book is opened |
| progress from the server | Continue Reading shows books started elsewhere |
| saving progress (`ReadWrite`) | progress goes back to the server, queued while offline |
| a home page and search | catalog accounts, see [Sections](#sections) |
| `on_open` / `on_close` | e.g. Jellyfin's "now reading" sessions |
| the server's own icon | shown next to the account |

## Progress

Progress is always saved locally. With `ReadWrite` it also goes to the
server through the provider. If the server can't be reached it waits in
`pending_progress` and is sent at the next sync, unless the server has a
newer read. That table moves from Jellyfin ticks to a page number and a
finished flag; each provider converts to its server's format.

Provider-specific rules stay inside the provider. Jellyfin's (never send a
position in session reports, stop with `Failed: true`) live in its
`on_open`/`on_close` and nowhere else.

## Sections

A catalog with tens of thousands of books can't be shown as one grid. A
catalog account's page is a home page of sections instead, with the rest
behind search:

```
[ Search Project Gutenberg…            ]

New this week                     See all ›
[▢] [▢] [▢] [▢] [▢] [▢] →

Popular                           See all ›
[▢] [▢] [▢] [▢] [▢] [▢] →

Browse by genre                          ›
```

```rust
pub struct Home {
    sections: Vec<Section>,
    search: Option<SearchInfo>,
    facets: Vec<Facet>,         // optional filters: genre, language…
}

pub struct Section {
    id: String,
    title: String,              // the server's own words
    kind: SectionKind,          // Row (covers) | Links (sub-pages)
    preview: Vec<Series>,       // the first ~20
    more: Option<Cursor>,       // See all: paged as you scroll
}
```

The provider decides which sections exist, what they're called, what's
in them, in what order, and whether they change per day or per account
("Free this Friday", "Because you read Saga"). How the lists are made is
up to the provider: the service's API, a tag on a server, a curated file.
A section can hold any number of books; the row shows the first ~20 and
See all pages through the rest, 50 at a time.

Rules, so every source looks and behaves like the app:

- At most 12 sections on a home page. More goes behind a `Links` section
  or search.
- Plain-text titles, no custom layouts or styling.
- Continue Reading and the All view belong to the app. Catalog books show
  up there once they're opened, added or downloaded, never by themselves.
- The home page is cached: it shows at once (and offline, as it was last
  time) and refreshes in the background when opened.
- The app decides when to refresh. Providers can't schedule background
  work.
- A synced account may have sections too (Jellyfin's "Recently added").
- A catalog with no sections gets "All books" and the search box.

Opening a book from a section shows its details page with **Read** /
**Download** / **Add to library**. Added books become normal library
entries, the same as synced ones.

Whether an account is synced or a catalog is a switch per account. It
defaults to catalog when the first listing finds more than a few thousand
books.

## Icons

Every provider has an icon, like Local and Jellyfin in the source picker.

- **A symbolic SVG, required.** One colour (`fill="#000"`), no gradients,
  text, embedded images or external links, viewBox cropped to the shape,
  readable at 16 px, under 8 KB. The app draws it in the theme's text
  colour (see `ui_icon` in `src/app/library_view.rs` and the files in
  `res/icons/ui/`).
- **A licence comment in the file**: where the logo is from, its licence,
  and what was changed (see `res/icons/ui/jellyfin.svg`). Getting
  permission to use a service's logo is the provider author's job.
- **A full-colour version, optional**, for the "Add a source" page.
- **A fallback**: if the icon is missing or breaks these rules the app
  shows a generic "online source" glyph.

OPDS has no official logo we know of, so it gets its own glyph in the
style of `local.svg`. An OPDS catalog's own icon (`atom:icon`, or `icon`
in OPDS 2) is shown next to its account, so a bridge or a Komga server
still looks like itself.

The source picker lists All, Local, then one entry per provider you have
an account for.

## OPDS

OPDS maps onto the above almost directly.

- **Versions**: OPDS 1.2 (Atom XML) and 2.0 (JSON).
- **Sign-in**: none, HTTP Basic, or the OPDS Authentication document
  (`application/opds-authentication+json`).
- **Listing**: acquisition feeds become series and items, following
  `next` links. Navigation feeds become sections.
- **Covers**: `http://opds-spec.org/image` (or `/image/thumbnail`), or
  `images` in OPDS 2.
- **Access**:
  - `Pages` from the Page Streaming Extension (OPDS 1.2: link rel
    `http://vaemendis.net/opds-pse/stream`, `pse:count`,
    `{pageNumber}` counting from 0, optional `{maxWidth}`; served by
    Komga, Kavita, Stump, Codex, Ubooquity) or from a Divina/WebPub
    manifest's `readingOrder` (OPDS 2).
  - `RangedFile` / `DownloadOnly` from `http://opds-spec.org/acquisition`
    and `/acquisition/open-access` links: tried with a Range request, else
    downloaded.
  - `External` from `/acquisition/buy`, `/borrow`, `/subscribe`.
- **Sections**: OPDS 2 `groups`; in 1.2, navigation links such as
  `http://opds-spec.org/sort/new`, `/sort/popular`, `/featured` and
  `/recommended`, each previewed from its first page (one request per
  section, hence the cap).
- **Search**: OpenSearch descriptions (1.2) or a templated `search` link
  (2.0). **Facets**: `opds:facetGroup` links (1.2) or `facets` (2.0).
- **Progress**: OPDS has no standard way to save it, so `ReadOnly` at
  best (PSE's `pse:lastRead`). Some servers may record progress as pages
  are fetched; that's to be checked per server. Native Komga or Kavita
  providers could sync properly later through their own APIs.

## Writing an OPDS bridge

To bring another service into Nebula without touching the app, serve an
OPDS 2 feed. A minimal home page with one section:

```json
{
  "metadata": { "title": "My Comics" },
  "links": [
    { "rel": "self", "href": "/opds", "type": "application/opds+json" },
    { "rel": "search", "href": "/opds/search{?query}",
      "type": "application/opds+json", "templated": true }
  ],
  "groups": [
    {
      "metadata": { "title": "Free this Friday" },
      "links": [{ "rel": "self", "href": "/opds/sections/free-friday",
                  "type": "application/opds+json" }],
      "publications": [
        {
          "metadata": { "identifier": "urn:mycomics:issue:1234",
                        "title": "Example #1", "author": "Someone" },
          "images": [{ "href": "/covers/1234.jpg", "type": "image/jpeg" }],
          "links": [{ "rel": "http://opds-spec.org/acquisition/open-access",
                      "href": "/files/1234.cbz",
                      "type": "application/vnd.comicbook+zip" }]
        }
      ]
    }
  ]
}
```

- Keep `identifier`s stable forever.
- The section's `self` link is the full list, paged with `next` links.
- Serve files with `Accept-Ranges: bytes` so they stream instead of
  downloading whole, or offer a Divina manifest for page-by-page reading.
- Give the feed an `icon` so your source looks like itself in the app.

## Writing a built-in provider

Built-in providers live in `src/providers/<name>.rs` and implement the
`Provider` trait (to be added with the Jellyfin refactor; this section
will grow then). Rough shape:

```rust
trait Provider {
    fn info(&self) -> &ProviderInfo;
    async fn connect(&self, form: Filled) -> Result<Account, String>;
    async fn list(&self, a: &Account) -> Result<Vec<Series>, String>;   // sync
    async fn home(&self, a: &Account) -> Result<Home, String>;          // browse
    async fn section(&self, a: &Account, id: &str, c: Cursor) -> Result<Page, String>;
    async fn search(&self, a: &Account, q: &str, c: Cursor) -> Result<Page, String>;
    async fn save_progress(&self, a: &Account, item: &str, p: Progress) -> Result<(), String>;
    fn client(&self, a: &Account) -> HttpClient;  // adds sign-in to every request
    // optional hooks: on_open, on_close
}
```

Each provider gets a mock server in `tools/` (like
`tools/mock-jellyfin.py`) and tests against it.

## What we won't take

Providers that get around DRM, scrape services whose terms forbid it, or
serve content without the rights to it. Besides the legal risk (Tachiyomi
shut down in 2024 under this kind of pressure), they break whenever the
service changes. Nothing stops a third party running such a bridge, but
the project won't ship, list or link to one.

## Plan

1. This page.
2. The `Provider` trait, with Jellyfin moved behind it and no change in
   behaviour. Keys, downloads, offline and Properties stop checking for
   `jellyfin://`; `pending_progress` moves to pages; `LibrarySource` is
   built from the providers in use. Checked against the Jellyfin mock
   before anything new goes in.
3. The OPDS provider (`src/opds.rs`) and `tools/mock-opds.py` (1.2, 2.0,
   PSE, ranged and download-only files): synced accounts first.
4. Catalog accounts: home page, sections, search, facets.
5. Trying it against a real Komga or Kavita.

Open questions:

- Whether Komga and Kavita record progress from PSE page fetches.
- How "Add to library" from a catalog shows in the series page when the
  catalog later drops the book (keep it, marked unavailable, like
  downloads on an unplugged drive?).
- WASM plugins: only if people ask for them once bridges exist.
