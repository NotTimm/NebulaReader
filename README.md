# Cosmic Comic

A native [COSMIC desktop](https://system76.com/cosmic) reader for comics,
manga and EPUB books. Point it at the folders your collection already lives
in and it builds a library of series, with covers, per-issue progress and a
Continue Reading shelf. Messy names are cleaned up and matched online for
proper titles and descriptions.

Screenshots: [`res/screenshots/`](res/screenshots/) (store copy and captions
in [`docs/listing.md`](docs/listing.md)).

## Features

**Library and collection management**

- Add library folders from the Settings drawer or by typing a path; `~` expands
  to your home directory.
- The scanner understands series folders, manga-tui chapter layouts,
  publisher/container folders, image directories and loose issues, then groups
  them into a single series entry when it makes sense.
- Every series keeps per-issue progress, read/unread state, cover art and a
  Continue Reading shelf.
- Search the library by title and reopen any series or issue from the library
  grid.
- If a file or folder is later removed, the app hides missing items without
  deleting your library metadata.

**Comics and manga**

- CBZ, CBR, image folders and embedded `ComicInfo.xml` metadata are supported.
- Fast CBR reading: large archives are extracted once and cached, so page turns
  don't keep re-streaming the same archive.
- Single-page or dual-page layout, theater mode, fullscreen and fit-to-height/
  fit-page window sizing.
- Zoom and pan work with the mouse wheel, trackpad, touchscreen pinch and
  click-drag pan. `M` or `Ctrl+0` toggles/resets zoom mode.
- The info panel shows the embedded metadata and the online match result.

**EPUB books**

- Full EPUB support with the book's own table of contents and chapter jump.
- Kindle-like typography: serif or sans, adjustable font size, line spacing,
  width, justification and theme.
- Paper, Sepia, Dark and Night themes, plus chapter progress and saved reading
  position across sessions.

**Metadata and online lookup**

- Offline name cleanup makes messy filenames readable before lookup.
- AniList is used for manga-like series; ComicVine can fill in more Western
  titles and publisher metadata when you add a free API key.
- Re-match a series with your own search terms if the automatic result is wrong.

**General app behavior**

- Native COSMIC styling that follows the system theme.
- Drag and drop files, folders or books directly into the window to open them.
- Settings drawer configures background style/opacity, dual-page mode,
  metadata lookup and library roots.
- Data stays in the standard XDG locations (`~/.local/share`,
  `~/.config`, `~/.cache`), so it survives reinstalls and upgrades.

## Quick start

1. Open the app and either add a library folder from the Settings drawer or open
   a file/folder directly.
2. Drop in an existing comic, manga or EPUB, or point the app at a local folder
   containing a collection.
3. The library builds a series view automatically, with cover art, issue lists,
   progress and a Continue Reading shelf.
4. While reading, use the toolbar or keyboard shortcuts to flip pages, toggle
   layouts, zoom, open details, or jump chapters.

## Reader controls

- Page navigation: Arrow keys, Page Up/Down, or the on-screen buttons.
- Book paging: `Space` moves forward; `Shift+Space` moves backward.
- Fullscreen: `F`.
- Theater mode: `T`.
- Dual-page layout: `L`.
- Zoom mode: `M` (or `Ctrl+scroll` / pinch when the reader is in view).
- Reset zoom: `Ctrl+0` or `M` again.
- Toggle metadata/info panel: `I`.
- Toggle Settings drawer: `S`.
- EPUB typography panel: `A`.
- Chapter picker: `C` (for multi-chapter books/series).
- Copy the current page to the clipboard: `P`.

## Download

Prebuilt packages are attached to each
[GitHub Release](https://github.com/NotTimm/cosmic-reader/releases)
(x86_64 only for now):

| Package | Built on | Install |
|---------|----------|---------|
| `cosmic-comic_<version>_amd64.deb` | Ubuntu 24.04 | `sudo apt install ./cosmic-comic_*_amd64.deb` |
| `cosmic-comic-<version>-1.fc<NN>.x86_64.rpm` | latest Fedora | `sudo dnf install ./cosmic-comic-*.x86_64.rpm` |

Install with `apt`/`dnf` as above (not `dpkg -i`/`rpm -i`) so the library
dependencies (`libarchive`, `libxkbcommon`, `openssl`, ...) are pulled in
automatically. The `.deb` targets Ubuntu 24.04 / Pop!_OS 24.04 and newer
Debian-based releases; the `.rpm` targets the Fedora release it was built
on and newer. To upgrade, download the newer package and run the same
command; to remove, `sudo apt remove cosmic-comic` or
`sudo dnf remove cosmic-comic`. Your library and settings are kept either
way (see [Where your data lives](#where-your-data-lives)).

Pre-release versions such as `0.1.0-beta.1` are packaged as `0.1.0~beta.1`,
which both package managers sort before the final `0.1.0`, so upgrading
from a beta to the release works as expected.

On another distro, build from source (below) or use `./install.sh`.

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

Building also needs the usual native headers: `libarchive`, `openssl` and
`libxkbcommon` development packages plus `pkg-config` and a C compiler
(`sudo apt install build-essential pkg-config libarchive-dev libssl-dev
libxkbcommon-dev` / `sudo dnf install gcc pkgconf-pkg-config
libarchive-devel openssl-devel libxkbcommon-devel`). A `.deb` meant for
distribution should be built on Debian/Ubuntu so `dpkg-shlibdeps` can
compute its dependencies; on other hosts the script falls back to a
fixed list that's only suitable for local testing.

### Publishing a release

`.github/workflows/release.yml` builds the `.deb` (Ubuntu 24.04) and
`.rpm` (Fedora container) on every pull request that touches the code or
packaging, and on manual runs, uploading them as workflow artifacts. To
publish:

1. Bump `version` in `Cargo.toml` (and add a `<release>` entry to
   `res/com.tsingel.CosmicComic.metainfo.xml`), commit, push.
2. Tag that commit with the same version prefixed by `v` and push the tag:

   ```
   git tag v0.1.0-beta.1
   git push origin v0.1.0-beta.1
   ```

The workflow refuses to publish if the tag doesn't match `Cargo.toml`,
then creates the GitHub Release (marked as a pre-release when the version
has a `-suffix`) with both packages attached. Re-running it updates the
same release. `.github/workflows/ci.yml` runs `cargo test --release` on
pushes to `master` and on pull requests.

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
