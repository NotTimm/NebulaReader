# Cosmic Comic

A native [COSMIC desktop](https://system76.com/cosmic) reader for comics,
manga and EPUB books. Point it at the folders your collection already lives
in and it builds a library of series, with covers, per-issue progress and a
Continue Reading shelf. Messy names are cleaned up and matched online for
proper titles and descriptions.

Screenshots: [`res/screenshots/`](res/screenshots/) (store copy and captions
in [`docs/listing.md`](docs/listing.md)).

## Features

**Library**

- Add library folders (browse, or type/paste a path; `~` expands). The
  scanner recognises series folders, manga-tui downloads
  (`Title/English/Ch N ….cbz`), image-folder chapters, publisher container
  folders, and loose issues, which are grouped into series
- Names are cleaned offline (`Civil War 001-007 (2006-2007) GetComics.INFO`
  → `Civil War`), then matched online: AniList for manga, ComicVine for
  Western comics (needs a free key, see below), for canonical titles,
  descriptions, publishers and per-issue titles and cover dates
- Series pages with an issue list, mark read/unread, and re-match with your
  own search terms when the automatic match is wrong
- Per-issue progress (read, unread, or in progress at a page), a Continue
  Reading shelf, and library search. Opening a loose file that belongs to a
  library series continues that series

**Comics**

- CBZ, CBR and image folders, with embedded `ComicInfo.xml`
- Fast CBR: pages are extracted in one pass and cached on disk instead of
  re-streaming the archive per page (see `src/comic.rs`)
- Single/dual page, theater mode, fullscreen, and Fit Page (resizes the
  window's width to the page's aspect ratio)
- Zoom & pan: the mouse wheel zooms directly, trackpad/touchscreen pinch
  zooms (see the libcosmic fork note below), two-finger scroll and
  click-drag pan a zoomed page, `M`/`Ctrl 0` toggle/reset zoom
- Metadata panel (`i`) showing the `ComicInfo.xml` and what was matched online

**Books (EPUB)**

- Kindle-style typography: serif or sans, adjustable size, line spacing and
  line width (about 66 characters by default), justified text with
  first-line indents, large chapter titles
- Paper, Sepia, Dark and Night themes
- The book's own table of contents, position remembered within a chapter,
  time left in the chapter and percentage through the book
- `Space` and arrow keys turn pages

**General**

- Native COSMIC look that follows the system theme
- Drag and drop a file, folder or book onto the window to open it
- Settings drawer: backdrop style and opacity, layout, online lookup,
  library folders
- Data in the standard XDG folders (see below)

## Building

```
just build-release
```

## Packaging a release

```
just package          # every format this machine has tooling for
just package-deb      # or one at a time
just package-rpm
just package-appimage
just package-flatpak
```

Output lands in `./dist`. The version comes from `Cargo.toml` — bump it
there and the packages follow.

The `.deb` and `.rpm` deliberately **do not** bundle libraries: their
dependencies are generated from the binary's actual ELF links
(`dpkg-shlibdeps` and rpm's automatic requires), so they pull
`libarchive`, `libxkbcommon`, `openssl` etc. from the distro. The
AppImage bundles those same libraries since it can't rely on packages,
and the Flatpak builds against the freedesktop 24.08 runtime.

Tooling needed per format (each script tells you if something's missing):

| Format   | Needs |
|----------|-------|
| deb      | `dpkg-dev` |
| rpm      | `rpm-build` |
| AppImage | `curl` (fetches `appimagetool` on first run) |
| Flatpak  | `flatpak`, `flatpak-builder` |

## Where your data lives

Following the XDG spec, so it survives reinstalls and upgrades:

| Path | Contents |
|------|----------|
| `~/.local/share/cosmic-comic/library.db` | Library, reading progress, metadata |
| `~/.config/cosmic-comic/settings.json` | Preferences |
| `~/.cache/cosmic-comic/` | Cover thumbnails, extracted archives — safe to delete |

The database is versioned with SQLite's `user_version` pragma and migrated
in place on startup, so upgrading never requires wiping your library.

## Installing / updating

```
./install.sh
```

Installs to `~/.local` (binary, `.desktop` entry, icon) with no sudo
needed. Re-run the same script any time to update: it pulls the latest
commit (if this is a clean git checkout), rebuilds, and reinstalls.

Override the install prefix with `COSMIC_COMIC_PREFIX=/usr/local ./install.sh`,
or use `just prefix=/usr install` / `just uninstall` directly.

## ComicVine metadata (optional)

AniList only knows about manga/anime, so Western comics (Marvel, DC, etc.)
won't match there. To also try [ComicVine](https://comicvine.gamespot.com/api/)
for those, get a free API key and either:

- set `COMICVINE_API_KEY` in your environment, or
- write the key (nothing else) to `~/.local/share/cosmic-comic/comicvine_api_key.txt`

Without a key, the debug panel just shows that no ComicVine key is set.

## Trackpad/touchscreen pinch-to-zoom fork

Upstream `iced` (vendored inside `pop-os/libcosmic`) silently drops winit's
`PinchGesture` event, so real trackpad/touchscreen pinch was impossible.
`Cargo.toml` points at `github.com/NotTimm/libcosmic`, a small personal fork
with the ~50 lines needed to forward it and gate mouse-wheel zoom behind
`Ctrl`. See the comment above the `libcosmic` dependency for how to re-sync
that fork against a newer upstream commit.
