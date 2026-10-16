# Notes for Claude

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
  packages plus `pkg-config`. Windows: libarchive from vcpkg (see README).
- Packaging scripts are in `packaging/`; `.github/workflows/release.yml` builds
  the .deb, .rpm, AppImage, .flatpak and Windows .zip, and a manual run with
  `publish` ticked creates the GitHub Release `v<Cargo.toml version>`.
- The owner's editor is Neovim; use `nvim`, not `nano`, in instructions.
