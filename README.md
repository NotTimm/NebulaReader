<p align="center">
  <img src="res/icons/hicolor/256x256/apps/io.github.NotTimm.NebulaReader.png" width="128" alt="Nebula Reader icon">
</p>

<h1 align="center">Nebula Reader</h1>

<p align="center">
  <b>Comics, manga and books in one library.</b><br>
  Made for the COSMIC desktop, at home on KDE Plasma, GNOME, Windows and macOS.
</p>

<p align="center">
  <a href="https://github.com/NotTimm/nebula-reader/releases/latest"><img src="https://img.shields.io/github/v/release/NotTimm/nebula-reader?label=download&color=e8857f" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/NotTimm/nebula-reader?color=7a7fd6" alt="GPL-3.0 license"></a>
  <img src="https://img.shields.io/badge/platforms-Linux%20%C2%B7%20Windows%20%C2%B7%20macOS-555" alt="Linux, Windows and macOS">
</p>

<p align="center">
  <img src="res/screenshots/library.png" width="820" alt="The library: a Continue Reading shelf of comics, manga and a book, with All, Local and Jellyfin tabs">
</p>

Point Nebula at the folders your collection already lives in, or sign in
to your Jellyfin server, and it builds a library of series with covers,
progress and a **Continue Reading** shelf. Messy file names are cleaned up
and matched online for proper titles and descriptions. Then it gets out of
the way so you can read.

> Nebula Reader was called **Cosmic Comic** up to 0.1.0-beta.4. Upgrading
> keeps your library, progress and settings: the first start moves them over.

## Highlights

### 📚 One library for everything

- Add the folders you already have. Series folders, loose issues, chapter
  folders of images and manga downloader layouts
  (`Title/English/Ch N ….cbz`) are all recognised and grouped into series.
- Names are tidied offline (`Civil War 001-007 (2006-2007) GetComics.INFO`
  becomes `Civil War`), then matched with **AniList** for manga and
  **ComicVine** for Western comics: canonical titles, descriptions,
  publishers, issue titles and cover dates. Wrong match? Search again with
  your own words.
- Read, unread or "page 12 of 30" for every issue, a Continue Reading
  shelf, and search across the library.
- Drag a file or folder onto the window to open it.

### 📖 Books that read like a book

<p align="center">
  <img src="res/screenshots/book.png" width="820" alt="Midnight in Chernobyl open on its first chapter as two facing pages">
</p>

- EPUBs turn pages like an e-reader: **one page, or two side by side**
  like an open book, or scroll if you prefer. Lines are never cut in half
  at the bottom of a page.
- Print-style typography: justified text, first-line indents, chapter
  openings set lower on the page.
- Pick the typeface, size, line spacing, line width and theme (Paper,
  Sepia, Dark, Night, or follow the desktop).
- Maps and photos fit the page; click one to open it full-window and zoom.
- The book's own table of contents, time left in the chapter and how far
  through the book you are.

<p align="center">
  <img src="res/screenshots/book-settings.png" width="640" alt="The Reading panel: text size, typeface, line spacing, justify, layout, line width and theme">
</p>

### 🖼️ Comics and manga, your way

<p align="center">
  <img src="res/screenshots/reader.png" width="820" alt="A comic open as a two-page spread">
</p>

- CBZ, CBR and folders of images, with embedded `ComicInfo.xml`.
- Single or two-page spreads, theater mode, full screen, and Fit Page,
  which sizes the window to the page.
- Zoom with Ctrl + scroll or a trackpad/touchscreen pinch, and pan by
  dragging.
- Fast CBR: archives are unpacked in one pass and cached, so page turns
  stay instant.
- An info panel with the issue's `ComicInfo.xml` and what was matched
  online.

<p align="center">
  <img src="res/screenshots/reader-zoom.png" width="640" alt="A comic page zoomed in to one panel">
</p>

### 🪼 Jellyfin built in

Sign in to your Jellyfin server with **Quick Connect** (no password
typing) and its book libraries sit right next to your local folders, with
an **All / Local / Jellyfin** switch at the top. Comic pages stream as you
read instead of downloading whole chapters, and progress syncs both ways
with Jellyfin's own web reader. [How it works](#jellyfin)

### 🎨 At home on your desktop

<p align="center">
  <img src="res/screenshots/settings.png" width="640" alt="The Settings drawer: style, background, theater mode, metadata lookup and library folders">
</p>

Nebula takes on the look of the desktop it runs on, including your
accent colour, light or dark mode and button shapes:

| Desktop | Style | Colours from | Title bar |
|---------|-------|--------------|-----------|
| COSMIC | COSMIC | the live COSMIC theme | libcosmic's header bar |
| KDE Plasma | Breeze | your colour scheme (`kdeglobals`), accent included | KWin's |
| Windows 10/11 | Windows 11 (Fluent) | light/dark mode and accent colour | Windows' |
| GNOME, Budgie, ... | Adwaita | light/dark and accent (settings portal) | libcosmic's header bar |
| macOS | Adwaita shapes | light/dark mode and accent colour | macOS's |

Prefer another look? **Settings → Appearance → Style**.

## Download

Grab a package from the
[latest release](https://github.com/NotTimm/nebula-reader/releases/latest):

| Your system | Download | Install |
|-------------|----------|---------|
| **Fedora** | `nebula-reader-<version>-1.fc<NN>.x86_64.rpm` | `sudo dnf install ./nebula-reader-*.x86_64.rpm` |
| **Ubuntu, Pop!_OS, Debian** (24.04+) | `nebula-reader_<version>_amd64.deb` | `sudo apt install ./nebula-reader_*_amd64.deb` |
| **Any Linux, Flatpak** | `NebulaReader-<version>-x86_64.flatpak` | `flatpak install --user ./NebulaReader-*.flatpak` |
| **Any Linux, AppImage** | `NebulaReader-<version>-x86_64.AppImage` | `chmod +x NebulaReader-*.AppImage`, then run it |
| **Windows 10/11** | `NebulaReader-<version>-windows-x64-setup.exe` | Run it. It installs for your user, no admin needed |
| **Windows, portable** | `NebulaReader-<version>-windows-x64.zip` | Unzip anywhere, run `nebula-reader.exe` |
| **macOS** (Apple Silicon) | `NebulaReader-<version>-macos-arm64.dmg` | Open it, drag Nebula Reader into Applications |

Coming soon: `winget install NotTimm.NebulaReader`, and Flathub (which is
also how apps reach the COSMIC Store).

<details>
<summary>Notes on installing, upgrading and unsigned builds</summary>

- Use `dnf`/`apt` as above rather than `rpm -i`/`dpkg -i`, so the
  libraries it needs (`libarchive`, `libxkbcommon`, `openssl`, ...) come
  along. The `.rpm` and `.deb` also replace an old `cosmic-comic` package.
- To upgrade, install the newer package the same way. To remove,
  `sudo dnf remove nebula-reader` or `sudo apt remove nebula-reader`. Your
  library and settings are kept either way.
- Windows SmartScreen may warn about an unrecognised app because the
  installer isn't code-signed: **More info → Run anyway**.
- The Mac app isn't notarized, so the first launch is blocked. Open it
  once, then **System Settings → Privacy & Security → Open Anyway** (on
  macOS 14 and older, right-click the app and choose **Open**). Or run
  `xattr -dr com.apple.quarantine "/Applications/Nebula Reader.app"`.
- Beta versions such as `0.1.0-beta.1` are packaged as `0.1.0~beta.1`, so
  they sort before the final release and upgrade cleanly.
- Another distro? Build from source with `./install.sh` (below).

</details>

## Keyboard and mouse

| | Comics | Books |
|---|---|---|
| Next / previous page | `→` `←` | `→` `←` `Space`, mouse wheel, or click the page edge |
| Contents / chapters | `C` | `C` |
| Full screen | `F` (`Esc` to leave) | `F` (`Esc` to leave) |
| Reading settings | `S` | `A`; text size `Ctrl +` / `Ctrl −`, `Ctrl 0` to reset |
| Dual page / zoom / theater | `L` / `M` / `T` | |
| Info panel / copy the page | `I` / `P` | |

Every reader button also has a hover hint with its shortcut.

## Jellyfin

Nebula reads the **book** libraries of a Jellyfin server (10.9 or newer;
tested with 12.2). Video libraries are ignored.

1. **Settings → Jellyfin**, type the server address (`192.168.1.20:8096`
   or `jellyfin.example.org`; https is tried first, then http) and press
   **Connect**.
2. A Quick Connect code appears. In Jellyfin, open your profile →
   **Quick Connect**, enter the code and approve it; Nebula signs in by
   itself (**Open Jellyfin** takes you straight there). If Quick Connect is
   turned off on the server, a password sign-in is offered instead. The
   password is sent once and never saved.
3. Each book library shows up as its own section, such as
   "Manga · myserver". Add as many servers and accounts as you like.
   **Sign out** ends the session on the server and forgets the token.

How it works:

- Only Jellyfin's access token is stored, in `jellyfin.json`, which only
  your user can read.
- CBZ chapters aren't downloaded whole: Nebula reads the archive's index
  with a few HTTP range requests, then fetches pages as you read, a few
  ahead. CBR/RAR chapters and EPUBs are downloaded to the cache first,
  since RAR has no index to seek in.
- Progress uses the same fields as Jellyfin's web reader (the page in
  `PlaybackPositionTicks`, `Played` when a chapter is finished), so it
  carries over between machines and the Jellyfin web app. Marking chapters
  read or unread in Nebula updates Jellyfin too.
- Libraries re-sync on startup and from the refresh button next to the
  account in settings. The server's progress wins only when it's newer
  than this machine's.

No server handy? `tools/mock-jellyfin.py` serves a folder as a fake
Jellyfin books library (Quick Connect approves itself on the second poll;
password login is `tester` / `secret`):

```
python3 tools/mock-jellyfin.py ~/Comics/Manga --port 8097
```

## ComicVine metadata (optional)

AniList only knows manga, so Western comics (Marvel, DC and so on) are
matched on [ComicVine](https://comicvine.gamespot.com/api/), which needs
a free API key. Either set `COMICVINE_API_KEY` in your environment or
paste the key into **Settings → Metadata**.

## Where your data lives

Outside the install location, so it survives reinstalls and upgrades. On
Linux:

| Path | Contents |
|------|----------|
| `~/.local/share/nebula-reader/library.db` | Library, reading progress, metadata |
| `~/.config/nebula-reader/settings.json` | Preferences |
| `~/.config/nebula-reader/jellyfin.json` | Jellyfin servers and access tokens (owner-only file, no passwords) |
| `~/.cache/nebula-reader/` | Cover thumbnails, extracted archives; safe to delete |

On Windows, `library.db` and `settings.json` are in
`%APPDATA%\NebulaReader\` and the cache is
`%LOCALAPPDATA%\NebulaReader\Cache\`. On macOS they're in
`~/Library/Application Support/NebulaReader/` and the cache is
`~/Library/Caches/NebulaReader/`. The Flatpak keeps its own copies under
`~/.var/app/io.github.NotTimm.NebulaReader/`.

The database is migrated in place on startup, so upgrading never means
wiping your library.

## Building from source

On Linux, with Rust and the development packages for `libarchive`,
`openssl` and `libxkbcommon` installed:

```
./install.sh
```

That builds Nebula and installs it to `~/.local` (binary, launcher entry
and icon) without sudo. Run it again any time to update: it pulls the
latest commit, rebuilds and reinstalls. Use
`NEBULA_PREFIX=/usr/local ./install.sh` for another prefix.

Windows builds, packaging, releases and the libcosmic fork are covered in
[docs/DEVELOPING.md](docs/DEVELOPING.md).

## Contributing

Bug reports and ideas are welcome in
[issues](https://github.com/NotTimm/nebula-reader/issues). Nebula is built
with Rust and [libcosmic](https://github.com/pop-os/libcosmic).

## License

[GPL-3.0](LICENSE).
