# Notes for Claude

The app is **Nebula Reader** ("Nebula" in launchers and the title bar),
app ID `io.github.NotTimm.NebulaReader`, binary `nebula-reader`, repo
`NotTimm/NebulaReader` (renamed from `nebula-reader` for Flathub's app ID
check; old URLs redirect). It was
Cosmic Comic up to 0.1.0-beta.4; data folders are moved over on first
start (`src/platform/mod.rs`). winget: `NotTimm.NebulaReader` (PR #457615; #457241 and #454452 were auto-closed as empty); the old `NotTimm.CosmicComic` PR #454352 was closed unmerged.

## Workflow

- Commit as the owner: `git config user.name "Tim Singel"` and
  `git config user.email "gamer.tdogtim@gmail.com"` before the first commit
  (cloud sessions default to a Claude identity); Claude goes in the
  `Co-Authored-By` trailer.
- Commit small, logical steps with descriptive messages and push often, so
  work can be picked up locally at any time.
- Push to `master` regularly, not just at the end: the owner follows
  progress from the repo page. Each commit that builds and passes
  `cargo test` goes to `master` right away (fast-forward or a merge commit;
  never squash, the history is the log). Keep the session branch in sync.
- Don't make releases (version bumps, tags, publish runs) unless the owner
  asks for one. Every release gets detailed, user-facing release notes in
  `packaging/release-notes/<version>.md` (the publish job uses them and
  fails without them); give the owner the same notes in the chat. Push features to `master`; a test build without `publish`
  is fine when useful.
- Claude sessions' GitHub tokens may lack the `workflow` scope, so changes
  under `.github/workflows/` can be refused on push or merge. When that
  happens, park the new file under `packaging/pending/`, push that, and ask
  the owner to paste it over the workflow on github.com.

## Project

- Rust + libcosmic (the owner's fork `NotTimm/libcosmic`, pinned by `rev` in
  `Cargo.toml`). Goal: stay in sync with `pop-os/libcosmic`. Run
  `tools/libcosmic-drift.sh` at the start of a session and before every
  release. **Syncing is a release blocker:** never bump, tag or publish
  while the drift script reports upstream commits missing; merge them
  first (merge, never rebase; steps in docs/DEVELOPING.md), tell the owner
  what's new, and check the fork's patches (listed above the libcosmic
  dependency in `Cargo.toml`) are all still there before pinning it. Runs on COSMIC, KDE, GNOME and Windows; per-OS code lives in
  `src/platform/`, per-desktop theming in `src/platform/look.rs`.
- Linux build deps: `libarchive`, `libxkbcommon`, `wayland`, `openssl` dev
  packages plus `pkg-config`. Windows: libarchive from vcpkg (see docs/DEVELOPING.md, which holds the
  build/packaging/release docs; README.md is the user-facing page).
- Packaging scripts are in `packaging/`; `.github/workflows/release.yml` builds
  the .deb, .rpm, AppImage, .flatpak, Windows .zip/setup and macOS .dmg, and a manual run with
  `publish` ticked creates the GitHub Release `v<Cargo.toml version>`.
- The owner's editor is Neovim; use `nvim`, not `nano`, in instructions.
  Their shell on Linux is fish (give `set -x VAR value`, not `VAR=value`).
- Jellyfin: `src/jellyfin.rs` (API client, accounts in `jellyfin.json`),
  `src/remote_zip.rs` (CBZ pages over HTTP ranges) and
  `src/app/jellyfin_view.rs` (sync, reader window, settings UI). Library keys
  are `jellyfin://<serverId>-<userId>/<itemId>`; never call
  `library::sync_issues` on them from the reader, since it deletes chapters
  that aren't loaded. Test against `tools/mock-jellyfin.py`.
- Jellyfin dashboard: while a server item is open the app reports a reading
  session (`/Sessions/Playing`, check-ins, `/Playing/Stopped`). Never send a
  position in these, and send the stop with `Failed: true`: books have no
  run time, so Jellyfin otherwise marks them read and clears the position
  (`UserDataManager.UpdatePlayState`). Progress goes through
  `save_progress` only. `jellyfin-reading.json` in the config dir marks an
  item left open; the next start ends that session and re-saves progress.
- Jellyfin downloads (`src/app/downloads.rs`): files in
  `data_dir()/jellyfin-downloads/<account>/<itemId>.<ext>` (books must keep
  the item id as the file stem), rows in the `downloads` table, issues keep
  their `jellyfin://` keys. `load_issues` prefers the downloaded file. A
  server whose sync fails to connect goes in `jf.offline`: its series and
  chapters are filtered to downloads, retried every 60 s. Saves that can't
  reach the server go to `pending_progress` and are sent at the next sync
  unless the server's `LastPlayedDate` is newer.
  The folder can be changed (`settings.download_dir`; files go in its
  `Nebula Reader` subfolder) and may be removable: never create the chosen
  folder itself, write `.part` + sync + rename (`platform/disk.rs`), keep
  rows whose files are on a disconnected drive (`DlState::Unavailable`),
  and never prune or remove those until the drive is back. The Downloads
  page is `AppView::Downloads` (`view_downloads`).

## Flathub: hands off

Flathub's generative AI policy
(https://docs.flathub.org/docs/for-app-authors/requirements) forbids AI
tools from opening or automating submission PRs or writing their
descriptions, commit messages or replies; Flathub manifests must not
contain AI-generated or AI-assisted content (disclosure doesn't exempt
them); AI-generated app code must be disclosed and may be rejected. So
Claude must not prepare, open or reply on the Flathub submission, and the
manifest under `packaging/flathub/` can't be what gets submitted. The owner
decides whether and how to submit. Research and debugging help is fine.

## The owner's machines

- Home PC: Fedora COSMIC (gaming). Laptop: Fedora COSMIC, with the repo at
  `~/Projects/cosmic-comic` and the libcosmic fork at `~/Projects/libcosmic`
  (what `tools/libcosmic-drift.sh` expects), and now also a fresh Windows
  install. Work: Windows only (PowerShell, VS Code, Visual Studio), where
  the owner tests the winget/installer builds.
- On the laptop, don't build unless asked to test: CI builds the packages.
  When asked, build gently (`nice -n 19 cargo build -j 4`,
  `CARGO_BUILD_JOBS=4 nice -n 19 just rpm`) and warn before deleting
  target dirs. Installing dev tools system-wide with `dnf` is fine (the
  owner runs the sudo). Don't touch `~/Projects/flathub`.
- Cloud sessions can't push `.github/workflows/` (see Workflow). They can
  attach `NotTimm/libcosmic` (add it to the session) and clone it next to
  this checkout, where `tools/libcosmic-drift.sh` finds it.

## Where things stand

Keep this short and current; it is the handoff between sessions (cloud or
the owner's laptop). Last updated 2026-10-30.

- Released: 0.1.7 (2026-10-19): in-app updates (`src/updater.rs`) for the
  Windows installer, Windows zip and Mac app, and minisign signatures on
  every release file (`sign` job, secrets MINISIGN_KEY/MINISIGN_PASSWORD,
  key ID AFE5BA3AD81D4B31; see docs/DEVELOPING.md). 0.1.7 is the first
  version that can update itself; the first real in-app update will be
  0.1.7 -> 0.1.8, so check that one on Windows and a Mac. 0.1.6 had the
  square corners, click-beside-drawer close, update banners and the
  volume-grouped chapter list (`src/app/chapter_list.rs`, not yet seen in
  the running app).
- The library banner says when a new version is out (closing remembers it
  in `dismissed_update`) and, on the first start of a new version, that
  it updated (`last_version`; not on a fresh install).
- The installer uses `CloseApplications=force`: winit ignores Windows'
  shutdown message, so silent installs over a running copy cancelled.
- Not verified in the GUI: Jellyfin downloads/offline reading (0.1.5) are
  unit-tested only. Unfetched remote chapter covers show a placeholder.
- Released 0.1.8 (2026-10-19) with everything below. Library cards open
  the series/details page (one-shots get a page without the chapter list,
  with a Download button); Continue Reading opens the reader. Downloads
  page with a choosable folder (USB-safe, see Jellyfin downloads above).
  Right-click menu on library cards and a Properties drawer
  (`src/app/properties.rs`, `ContextPage::Properties`).
  CBRs: local ones list pages from headers and extract lazily per archive
  (`comic::rar_page`); Jellyfin ones stream via `src/remote_rar.rs`
  (header walk, index cached in `cache/jellyfin-index`), solid ones are
  fetched whole after asking above 100 MB (`jf.blocked`). Caches trimmed
  to 2 GB at startup. Convert to CBZ was removed in 0.1.15.
- winget: microsoft/winget-pkgs#457615 (`NotTimm.NebulaReader` 0.1.11, opened
  2026-10-21). Two earlier PRs (#454452, #457241) had passed validation but
  were auto-closed as empty after the branch was pushed back to fcb0e6200
  (the commit that only deletes 0.1.1); GitHub won't reopen them. Never
  re-run that old reopen command block. Branch `NotTimm.NebulaReader-0.1.0`
  in `~/Projects/winget-pkgs` must stay at 5a8ab004b (or later). Once merged, re-run
  the release workflow's `winget` job from the newest release to file
  updates; until then that job fails, which is expected.
- Fedora/COPR: `packaging/fedora/nebula-reader.spec` (offline build from
  the release + vendor tarballs, `NEBULA_DISTRO` turns off the GitHub
  update check), CI-built cleanly. COPR project `timbo-slice/nebula-reader`
  (owner's account), SCM + make srpm; the release workflow's `copr` job
  starts it (secret COPR_CONFIG, expires every 180 days). 0.1.11 built on
  Fedora 43, 44, 45 and rawhide (build 11180031).
- awesome-jellyfin: listing PR jellyfin/awesome-jellyfin#707 is open.
- Flathub: owner's call only, see "Flathub: hands off". The manifest in
  `packaging/flathub/` tracks the latest release (v0.1.5).
- Released 0.1.11 (2026-10-20): packaging only, the first release with a
  vendor tarball, so the first COPR build.
- Released 0.1.10 (2026-10-20): theater mode on by default
  (`Settings::default`), switched on once for settings saved before
  0.1.10 (`init` in `src/app.rs`), plus one-finger touch panning.
- Released 0.1.9 (2026-10-20): the Windows touchpad fix and Build Page
  Indexes below, both untried on the owner's machines so far.
- Build Page Indexes (`src/app/rar_index.rs`): right-click a Jellyfin
  series card or a chapter row to walk its CBRs' headers ahead of time
  (same `remote_rar` index file the reader uses). `App::unindexed` is
  worked out in the background (after a sync, a run, or leaving the reader).
  Tested against the mock with hand-built RAR5 files; solid detection is
  covered by `remote_rar`'s tests only.
- Windows touchpad: winit reports it as fractional `Lines`, like a wheel;
  libcosmic patch 11 (`WheelSource`) pans on those instead of zooming.
  Unit-tested only: the owner is to try it on a Windows laptop.
- Touchscreen (patch 12, after 0.1.9): one-finger drag pans a zoomed page
  on both axes with a flick glide, two fingers pan while pinching, and a
  pinch that starts zoom mode works at once. Unit-tested with simulated
  fingers only (no touchscreen here); the owner is to try it.
- Fit to Width (patch 13, in 0.1.12): `Space` in the comic reader (not
  books) or its header button; `App::fit_width`, never saved. The viewer
  eases in/out (160 ms), the wheel scrolls instead of zooming, new pages
  start at the top. Seen working in Xvfb; untried on real machines.
- Released 0.1.16 (2026-10-28): Continue Reading counts chapters with a
  page saved on the Jellyfin server (`SeriesEntry::issues_in_progress`);
  before, a series started elsewhere showed only once a chapter was
  finished or opened here. Still one card per series. Fedora spec links
  the system libzstd.
- Released 0.1.15 (2026-10-28; 0.1.14 was tagged but never published: its
  vendor job failed on libcosmic crates from two sources, fixed by the
  `[patch."https://github.com/pop-os/libcosmic"]` section in Cargo.toml,
  whose rev must follow the libcosmic pin): Convert to CBZ removed (it needed write
  access to home; the Flatpak has `home:ro`), with `convert.rs`, its view,
  the `trash` crate and `library::rename_issue_file`. Opening a Jellyfin CBR
  without an index says "Creating index of CBR comic…"
  (`App::creating_index`, `rar_index::needs_index`); the ahead-of-time job
  says "Creating index(es) of CBR comic(s)…". Also the Dependabot batch and
  libcosmic synced through upstream 95236a94 (fork `756cd0f`). The owner is
  writing their own Flathub manifest against this release.
- Released 0.1.13 (2026-10-23): Ctrl (Cmd on a Mac) + a mouse wheel scrolls a
  zoomed comic page, with Shift sideways (patch 14); a touchpad's
  Ctrl + scroll still zooms, and on a whole page it does nothing. Seen
  working in Xvfb (give the window focus first: there's no window manager).
- Released 0.1.12 (2026-10-21): Fit to Width, the audit below, and card
  menus that download/remove one item for one-shots ("Download Book")
  and Continue Reading cards ("Download Issue"/"Download Book" for the
  chapter shown, "Remove Download" for just it:
  `CardAction::DownloadIssue`/`RemoveIssueDownload`); other series cards
  keep Download All / Remove Downloads. First release whose GitHub text
  comes from `packaging/release-notes/<version>.md`. COPR build started;
  the winget job failed as expected (first PR not merged yet).
- Audit 2026-10-21 (in 0.1.12): one DB commit per
  sync/scan (`library::batch`, synchronous=NORMAL); a failed dark-mode
  read never means light (Mac `defaults`, Windows registry, portal);
  Jellyfin chapters open in parallel (`open_streamed`); decoded pixels
  move into the handle (cache holds `PageSize`), nearby pages are
  `allocate`d on the GPU, `page_gen` drops decodes/covers/ComicInfo from
  a previous comic; Mac `._`/`__MACOSX` skipped (`comic::is_mac_junk`),
  Windows `$RECYCLE.BIN`/hidden skipped in scans; Cmd shortcuts on Mac;
  Trash via NSFileManager on Mac; at most 8 CBZs open (`forget_zip`
  before deleting); Drop for App sends Jellyfin progress (Mac Cmd+Q).
  Not done yet: Finder "Open With" on Mac (no CFBundleDocumentTypes,
  no openURLs handling), downloads/db in Roaming AppData on Windows,
  theme polling every 5 s instead of change signals, library grid
  rebuilt on every message, Jellyfin library fetch one folder at a time,
  scans re-open known archives with no concurrency cap.
- `tools/check-targets.sh` type-checks Windows and macOS from Linux.
- Translations (2026-10-30, unreleased): Fluent via i18n-embed
  (`src/i18n.rs`, `i18n/<lang>/nebula_reader.ftl`, `i18n.toml`); every
  user-facing string is `fl!("id", var = value)`, English is the reference
  (a missing id doesn't compile). Settings → Appearance → Language
  (`Settings::language`, `None` = system; `i18n::select` also switches
  libcosmic's strings via fork patch 15). Dropdown labels live in
  `App::labels`, rebuilt on a language change. English and Spanish are
  complete (452 messages); tests check parse, ids and variables.
  `i18n/README.md` is the translator guide. Not translated yet: the
  .desktop and metainfo files.
- OPDS (2026-10-30, unreleased, **untested on real servers**; README and
  CONTRIBUTING say so): native like Jellyfin, no plugin system or provider
  docs (owner's call: only native code, contributors work from the code).
  `src/remote.rs` is the shared layer (`remote::Account` = Jellyfin | OPDS:
  range source, whole download, page stream, cover, ping, fetch_library,
  download_item); `jellyfin_view.rs` runs reader/sync/covers/offline/
  account rows for both, Jellyfin-only parts (progress, sessions, marks)
  match `RemoteAccount::Jellyfin`. Pages for both kinds should share
  code like this (owner, 2026-10-30). `src/opds.rs`: feeds (Atom 1.2,
  JSON 2.0), PSE + Divina (`PageSource::Web`), Basic auth only to the
  catalog's own host, accounts in `opds.json` (password stored, 0600),
  item links in `cache/opds/<account>.json`. Sync walks only the user's
  picked folders (`Account::roots`) level by level, 6 at once, depth 4,
  20 000 pages (too big = error, never a partial prune); series = feed,
  else `belongsTo`, else the name in titles. Keys
  `opds://opds-<fnv>/<o|s|f|b|r><fnv>`: never change the hash. UI:
  Settings → OPDS and `AppView::Catalog` (`opds_view.rs`: browse,
  search, Add to/Remove from Library). Progress stays local. No mock
  server yet (tests serve feeds in-process). Next: try it on Komga/Kavita.
- Contributions: the owner welcomes outside issues and PRs (2026-10-28).
  `CONTRIBUTING.md` (build steps, code map, an "Ideas to pick up" list to
  keep in step with the open items here), the README's Contributing
  section, issue forms in `.github/ISSUE_TEMPLATE/` and a PR template.
- Dependabot (2026-10-24): `.github/dependabot.yml` (weekly, crates
  grouped minor/patch, Actions grouped, libcosmic ignored); alerts and
  security-fix PRs switched on. How to handle the PRs: docs/DEVELOPING.md.
  First batch merged 2026-10-26 (PRs #1–#7: Actions v4→v7 etc., zip 9,
  rusqlite 0.40, ashpd 0.13, quick-xml, the minor group), plus what
  `cargo audit` still flagged: quick-xml 0.42 and h2 0.4.20. Code
  changes: zip `name()` returns a Result; quick-xml reports `&amp;` etc.
  as `Event::GeneralRef` (epub `event_text`) and hands out `str` from
  0.40; ashpd needs the `settings` feature.
  Second batch merged 2026-10-30 (PRs #8–#11: tokio, reqwest 0.13,
  windows-registry 0.100, compress-tools 0.16). reqwest stays on
  native-tls (0.13's default is rustls + bundled aws-lc), with "query" for
  ComicVine; checked against api.github.com. Unreleased. `cargo audit`
  leftovers are all from libcosmic/iced (lru via cryoglyph, memmap2 via
  xkbcommon, and unmaintained notices) or unbuilt (quick-xml 0.39).
- libcosmic: fork pinned at `5062b8b` (patches 1–15, see Cargo.toml),
  merged with upstream through `95236a94` (2026-10-27; iced at 229186b5); 0.1.2 was built from
  tag `nebula-0.1.2`.
- Decisions: Continue Reading shows one card per series (the most recently
  touched chapter); a per-chapter shelf would be an option, not default.
- Open: Jellyfin book position isn't synced *from* the server yet; runtime
  window icons on Linux are dropped by an iced bug (`conversion::icon`
  builds a RawImage winit ignores); code signing; scan-group names like
  `Jaimini's~box~` aren't stripped.
