# Notes for Claude

The app is **Nebula Reader** ("Nebula" in launchers and the title bar),
app ID `io.github.NotTimm.NebulaReader`, binary `nebula-reader`. It was
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
- Claude sessions' GitHub tokens may lack the `workflow` scope, so changes
  under `.github/workflows/` can be refused on push or merge. When that
  happens, park the new file under `packaging/pending/`, push that, and ask
  the owner to paste it over the workflow on github.com.

## Project

- Rust + libcosmic (the owner's fork `NotTimm/libcosmic`, pinned by `rev` in
  `Cargo.toml`). Runs on COSMIC, KDE, GNOME and Windows; per-OS code lives in
  `src/platform/`, per-desktop theming in `src/platform/look.rs`.
- Linux build deps: `libarchive`, `libxkbcommon`, `wayland`, `openssl` dev
  packages plus `pkg-config`. Windows: libarchive from vcpkg (see docs/DEVELOPING.md, which holds the
  build/packaging/release docs; README.md is the user-facing page).
- Packaging scripts are in `packaging/`; `.github/workflows/release.yml` builds
  the .deb, .rpm, AppImage, .flatpak, Windows .zip/setup and macOS .dmg, and a manual run with
  `publish` ticked creates the GitHub Release `v<Cargo.toml version>`.
- The owner's editor is Neovim; use `nvim`, not `nano`, in instructions.
- Jellyfin: `src/jellyfin.rs` (API client, accounts in `jellyfin.json`),
  `src/remote_zip.rs` (CBZ pages over HTTP ranges) and
  `src/app/jellyfin_view.rs` (sync, reader window, settings UI). Library keys
  are `jellyfin://<serverId>-<userId>/<itemId>`; never call
  `library::sync_issues` on them from the reader, since it deletes chapters
  that aren't loaded. Test against `tools/mock-jellyfin.py`.

## Where things stand

Keep this short and current; it is the handoff between sessions (cloud or
the owner's laptop).

- Done: per-desktop looks and button roundness; Windows build, icons and
  Inno installer; release workflow for all formats; winget submission
  (microsoft/winget-pkgs#454352, awaiting review); manga-tui name cleanup;
  Jellyfin reading (owner confirms it works on the real server; pages
  stream from `/Items/{id}/File`, since `/Download` logs an activity entry
  per request); books
  paginate like a Kindle (`src/app/paged.rs`, single/two pages, scroll as
  an option) with click-to-zoom images; hover hints on reader buttons;
  All/Local/Jellyfin library switch; v0.1.0-beta.4 released.
- Next: 0.1.1 released to fix winget validation (0.1.0's exe needed
  VCRUNTIME140.dll, and winget's test runs with --skip-dependencies, so a
  VCRedist dependency doesn't help; the DLL now ships app-locally). PR
  microsoft/winget-pkgs#454452 is moved to 0.1.1; once merged, the
  workflow's `winget` job submits later releases. Then Flathub (owner is
  being walked through it; `packaging/flathub/`). Jellyfin: title-bar
  source label with online dot, progress saved after 10 s on a page;
  syncing a book's position *from* the server isn't done yet.
- macOS (Apple Silicon): `packaging/build-macos.sh` builds the .app and
  .dmg; the `macos` job in `release.yml` is live (first run 39006216181
  was its first real build). Windows packages now bundle vcruntime140.dll.
- Owner's laptop: don't run release builds there unless asked to test;
  `cargo check` is fine, CI builds the packages. `just rpm` builds an rpm.
- Open: runtime window icons on Linux are dropped by a libcosmic/iced bug
  (`conversion::icon` builds a RawImage that winit ignores); a patch for the
  fork is pending. Flathub submission. Code signing.
  Scan-group names like `Jaimini's~box~` aren't stripped yet.
