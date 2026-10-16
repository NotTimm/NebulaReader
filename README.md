# Cosmic Comic

A [COSMIC desktop](https://system76.com/cosmic) reader for comics, manga
and EPUB books that also runs on KDE Plasma, GNOME and Windows, taking on
each desktop's look. Point it at the folders your collection already lives
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
- Jellyfin book libraries alongside local folders: pages stream from the
  server, and reading progress is shared with Jellyfin (see below)

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

- Native look per desktop (see below)
- Drag and drop a file, folder or book onto the window to open it
- Settings drawer: backdrop style and opacity, layout, online lookup,
  library folders
- Data in the standard XDG folders (see below)

## Native look per desktop

The UI is libcosmic everywhere, but outside COSMIC it rebuilds its theme
from the desktop it runs on (`src/platform/look.rs`):

| Desktop | Style | Colours from | Title bar |
|---------|-------|--------------|-----------|
| COSMIC | COSMIC | the live COSMIC theme | libcosmic's header bar |
| KDE Plasma | Breeze | your colour scheme (`kdeglobals`), accent included | KWin's |
| Windows 10/11 | Windows 11 (Fluent) | light/dark mode and accent colour | Windows' |
| GNOME, Budgie, ... | Adwaita | light/dark and accent (settings portal) | libcosmic's header bar |

Corner radii follow the desktop too (Breeze's 3px buttons, Fluent's 4px,
Adwaita's 6px, COSMIC's pills). Pick a different style under
**Settings → Appearance → Style**; the title bar style changes on the
next start.

## Download

Prebuilt packages are attached to each
[GitHub Release](https://github.com/NotTimm/cosmic-reader/releases)
(x86_64 only for now):

| Package | For | Install |
|---------|-----|---------|
| `CosmicComic-<version>-windows-x64-setup.exe` | Windows 10/11 | Run it; installs for your user only, no admin needed. Or `winget install NotTimm.CosmicComic` |
| `CosmicComic-<version>-windows-x64.zip` | Windows 10/11 | Portable: unzip anywhere and run `cosmic-comic.exe` |
| `cosmic-comic_<version>_amd64.deb` | Ubuntu/Pop!_OS 24.04+ | `sudo apt install ./cosmic-comic_*_amd64.deb` |
| `cosmic-comic-<version>-1.fc<NN>.x86_64.rpm` | Fedora | `sudo dnf install ./cosmic-comic-*.x86_64.rpm` |
| `CosmicComic-<version>-x86_64.flatpak` | any distro with Flatpak | `flatpak install --user ./CosmicComic-*.flatpak` |
| `CosmicComic-<version>-x86_64.AppImage` | any recent distro | `chmod +x CosmicComic-*.AppImage` and run it |

Windows SmartScreen may warn about an unrecognised app, since the exe
isn't code-signed; choose **More info → Run anyway**.

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

### On Windows

Install Rust (MSVC toolchain) and [vcpkg](https://vcpkg.io), then from
PowerShell:

```
vcpkg install "libarchive[core,bzip2,lz4,lzma,zstd]:x64-windows"
$env:VCPKG_ROOT = "C:\path\to\vcpkg"
./packaging/build-windows.ps1
```

That builds the release exe and zips it with libarchive's DLLs into
`dist\`. For a plain `cargo run`, also set `$env:VCPKGRS_DYNAMIC = "1"`
and put `$env:VCPKG_ROOT\installed\x64-windows\bin` on `PATH`.

## Packaging a release

```
just package          # every format this machine has tooling for
just package-deb      # or one at a time
just package-rpm
just package-appimage
just package-flatpak
```

Output lands in `./dist`. The version comes from `Cargo.toml` — bump it
there and the packages follow. The Windows zip is built on Windows (see
above) or by CI.

The `.deb` and `.rpm` deliberately **do not** bundle libraries: their
dependencies are generated from the binary's actual ELF links
(`dpkg-shlibdeps` and rpm's automatic requires), so they pull
`libarchive`, `libxkbcommon`, `openssl` etc. from the distro. The
AppImage bundles those same libraries since it can't rely on packages,
and the Flatpak builds against the freedesktop 25.08 runtime, offline:
its crates come from `packaging/cargo-sources.json`, which
`packaging/gen-cargo-sources.sh` generates from `Cargo.lock` (the Flatpak
script does this for you when it's missing or stale).
[`packaging/flathub/`](packaging/flathub/) has what's needed to submit to
Flathub, which is also how the app gets into the COSMIC Store.

Tooling needed per format (each script tells you if something's missing):

| Format   | Needs |
|----------|-------|
| deb      | `dpkg-dev` |
| rpm      | `rpm-build` |
| AppImage | `curl` (fetches `appimagetool` on first run) |
| Flatpak  | `flatpak`, `flatpak-builder`, `python3` with `aiohttp`, `tomlkit`, `pyyaml` |

Building also needs the usual native headers: `libarchive`, `openssl` and
`libxkbcommon` development packages plus `pkg-config` and a C compiler
(`sudo apt install build-essential pkg-config libarchive-dev libssl-dev
libxkbcommon-dev` / `sudo dnf install gcc pkgconf-pkg-config
libarchive-devel openssl-devel libxkbcommon-devel`). A `.deb` meant for
distribution should be built on Debian/Ubuntu so `dpkg-shlibdeps` can
compute its dependencies; on other hosts the script falls back to a
fixed list that's only suitable for local testing.

### winget

The Windows installer is published to winget as `NotTimm.CosmicComic`.
Once the first version is in
[microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs), the
`winget` job in `release.yml` submits each new release automatically. It
needs a repository secret `WINGET_TOKEN`: a classic personal access token
with the `public_repo` scope, from the account that owns a fork of
winget-pkgs. Without the secret the job just skips.

The first version is submitted by hand from Windows:

```
winget install Microsoft.WingetCreate
wingetcreate new https://github.com/NotTimm/cosmic-reader/releases/download/v<version>/CosmicComic-<version>-windows-x64-setup.exe
```

### Icons

`res/icons/source/` holds two masters: `book` (the app icon) and
`book-open` (the window icon while reading), each as an `.svg` or a large
`.png`. A PNG on a solid background (typical of image generators) is cut
out automatically. After changing them, regenerate every size and commit
the results:

```
just icons
```

That writes the Linux icon theme sizes (`res/icons/hicolor/`), the Windows
`.ico` embedded in the exe by `build.rs`, and the window icons in
`res/icons/window/`. Earlier designs are kept in `res/icons/alt/`.

### Publishing a release

`.github/workflows/release.yml` builds every package (`.deb` on Ubuntu
24.04, `.rpm` in a Fedora container, AppImage, Flatpak, and the Windows
zip, which also runs the tests on Windows) on every pull request that
touches the code or packaging, and on manual runs, uploading them as
workflow artifacts. To publish:

1. Bump `version` in `Cargo.toml` (and add a `<release>` entry to
   `res/com.tsingel.CosmicComic.metainfo.xml`), commit, push.
2. Tag that commit with the same version prefixed by `v` and push the tag:

   ```
   git tag v0.1.0-beta.1
   git push origin v0.1.0-beta.1
   ```

   Or, instead of tagging by hand: **Actions → Release packages → Run
   workflow**, pick the branch, and tick **publish**. That creates the
   `v<version>` tag on the commit it built.

The workflow refuses to publish if a pushed tag doesn't match
`Cargo.toml`, then creates the GitHub Release (marked as a pre-release
when the version has a `-suffix`) with every package attached. Re-running it updates the
same release. `.github/workflows/ci.yml` runs `cargo test --release` on
pushes to `master` and on pull requests.

## Where your data lives

Outside the install location, so it survives reinstalls and upgrades. On
Linux (XDG):

| Path | Contents |
|------|----------|
| `~/.local/share/cosmic-comic/library.db` | Library, reading progress, metadata |
| `~/.config/cosmic-comic/settings.json` | Preferences |
| `~/.config/cosmic-comic/jellyfin.json` | Jellyfin servers and access tokens (owner-only file, no passwords) |
| `~/.cache/cosmic-comic/` | Cover thumbnails, extracted archives — safe to delete |

On Windows, `library.db` and `settings.json` are in
`%APPDATA%\CosmicComic\`, and the cache is
`%LOCALAPPDATA%\CosmicComic\Cache\`. The Flatpak keeps its own copies
under `~/.var/app/com.tsingel.CosmicComic/`.

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
- paste it into **Settings → Metadata** (saved as
  `comicvine_api_key.txt` in the data folder above)

Without a key, the debug panel just shows that no ComicVine key is set.

## Jellyfin

Cosmic Comic can read the **book** libraries of a Jellyfin server (10.9 or
newer; tested against 12.2). Video libraries are ignored.

1. **Settings → Jellyfin**, type the server address (`192.168.1.20:8096`
   or `jellyfin.example.org`; https is tried first, then http) and press
   **Connect**.
2. A Quick Connect code is shown. In Jellyfin, open your profile →
   **Quick Connect**, enter the code and approve it; the app signs in by
   itself (**Open Jellyfin** takes you straight there). If Quick Connect
   is turned off on the server (Dashboard → General), a password sign-in
   is offered instead; the password is sent once and never saved.
3. Each book library appears as its own section, such as "Manga · myserver".
   Add as many servers and accounts as you like; **Sign out** ends the
   session on the server and forgets the token.

How it works:

- Only Jellyfin's access token is stored, in `jellyfin.json`, which only
  your user can read.
- CBZ chapters are not downloaded: the app reads the archive's index with
  a few HTTP range requests and then fetches one page per request as you
  read, a few chapters ahead. CBR/RAR chapters and EPUBs are downloaded
  to the cache first (RAR has no index to seek in), and EPUB one-shots open
  in the book reader.
- Progress uses the same fields as Jellyfin's own web reader (page in
  `PlaybackPositionTicks`, `Played` when a chapter is finished), so it
  carries over between Cosmic Comic on different machines and the Jellyfin
  web app. Marking chapters read or unread in the app updates Jellyfin too.
- Libraries re-sync on startup and with the refresh button next to the
  account in settings. Server-side progress wins only when it is newer
  than what this machine has.

To try it without a server, `tools/mock-jellyfin.py` serves a folder as a
fake Jellyfin books library (Quick Connect approves itself on the second
poll; password login is `tester` / `secret`):

```
python3 tools/mock-jellyfin.py ~/Comics/Manga --port 8097
```

Then connect to `localhost:8097`. It prints each download range and progress
update the app sends.

## Trackpad/touchscreen pinch-to-zoom fork

Upstream `iced` (vendored inside `pop-os/libcosmic`) silently drops winit's
`PinchGesture` event, so real trackpad/touchscreen pinch was impossible.
`Cargo.toml` points at `github.com/NotTimm/libcosmic`, a small personal fork
with the ~50 lines needed to forward it and gate mouse-wheel zoom behind
`Ctrl`. See the comment above the `libcosmic` dependency for how to re-sync
that fork against a newer upstream commit.
