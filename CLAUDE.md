# Notes for Claude

The app is **Nebula Reader** ("Nebula" in launchers and the title bar),
app ID `io.github.NotTimm.NebulaReader`, binary `nebula-reader`, repo
`NotTimm/NebulaReader` (renamed from `nebula-reader` for Flathub's app ID
check; old URLs redirect). It was
Cosmic Comic up to 0.1.0-beta.4; data folders are moved over on first
start (`src/platform/mod.rs`). winget: `NotTimm.NebulaReader` (PR #454452); the old `NotTimm.CosmicComic` PR #454352 was closed unmerged.

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
  asks for one. Push features to `master`; a test build without `publish`
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
the owner's laptop). Last updated 2026-10-19.

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
  to 2 GB at startup. Convert to CBZ: `src/convert.rs` +
  `src/app/convert_view.rs` (CBRs to the Trash via the `trash` crate).
- winget: microsoft/winget-pkgs#454452 (`NotTimm.NebulaReader` 0.1.1) has
  passed validation and waits for a moderator. Decision: keep it at 0.1.1
  (changing it re-runs validation). Once merged, re-run the release
  workflow's `winget` job from the newest release to file the update; until
  then that job fails, which is expected.
- awesome-jellyfin: listing PR jellyfin/awesome-jellyfin#707 is open.
- Flathub: owner's call only, see "Flathub: hands off". The manifest in
  `packaging/flathub/` tracks the latest release (v0.1.5).
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
- libcosmic: fork pinned at `6a574db` (patches 1–11, see Cargo.toml),
  merged with upstream through `430f3d48` (2026-10-19; iced at 229186b5); 0.1.2 was built from
  tag `nebula-0.1.2`.
- Decisions: Continue Reading shows one card per series (the most recently
  touched chapter); a per-chapter shelf would be an option, not default.
- Open: Jellyfin book position isn't synced *from* the server yet; runtime
  window icons on Linux are dropped by an iced bug (`conversion::icon`
  builds a RawImage winit ignores); code signing; scan-group names like
  `Jaimini's~box~` aren't stripped.
