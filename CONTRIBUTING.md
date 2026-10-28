# Contributing to Nebula Reader

Thanks for wanting to help! Nebula is a small project and every bit of
help counts: a bug report, an idea, a screenshot from a desktop I don't
use, a fix for a typo, or a whole new feature. You don't need to ask
before opening an issue or a pull request.

## Ways to help

- **Report a bug.** Open an
  [issue](https://github.com/NotTimm/NebulaReader/issues/new/choose) and
  say what you did, what you expected and what happened. Your system
  (Fedora COSMIC, Windows 11, macOS 15…), how you installed Nebula and
  its version (Settings → About) help a lot. If it's about one file, say
  what kind it is (CBZ, CBR, EPUB, a Jellyfin library…).
- **Suggest something.** Ideas, wishes and "it would be nice if…" are all
  welcome as issues, big or small.
- **Try it where I can't.** Nebula is built on Linux, so reports from
  Windows, Macs (especially Intel ones), KDE, GNOME, touchscreens and
  tablets, HiDPI screens and big Jellyfin servers are especially useful.
- **Send a pull request.** Fixes, features, docs, packaging for another
  distro: all fine. For something large, opening an issue first to talk
  it over saves you from building something that clashes with other
  plans, but it's not required.

## Ideas to pick up

Some things that are known and wanted but nobody's working on yet:

- **macOS "Open With"**: opening a comic from Finder (needs
  `CFBundleDocumentTypes` in the app bundle and handling the open event).
- **Follow the system theme without polling**: listen for the desktop's
  change signals instead of checking every few seconds.
- **Faster library scans**: a cap on how many archives are opened at
  once, and not reopening archives that haven't changed.
- **Jellyfin**: fetch a server's libraries in parallel, and sync a book's
  reading position *from* the server, not just to it.
- **Cleaner names**: scan-group tags such as `Jaimini's~box~` in file
  names aren't stripped yet.
- **Translations**: the interface is English only for now.
- **Packaging** for distros that aren't covered (Arch/AUR, NixOS,
  openSUSE…).

If you start on one, say so in an issue so nobody does it twice.

## Building and running

You need Rust (stable) and, on Linux, the development packages for
`libarchive`, `libxkbcommon`, `wayland` and `openssl`, plus `pkg-config`:

| Distro | Packages |
|--------|----------|
| Fedora | `sudo dnf install libarchive-devel libxkbcommon-devel wayland-devel openssl-devel pkgconf-pkg-config` |
| Ubuntu, Debian, Pop!_OS | `sudo apt install libarchive-dev libxkbcommon-dev libwayland-dev libssl-dev pkg-config` |

Then:

```
cargo run              # build and start a debug build
cargo test             # run the tests
```

Windows (libarchive from vcpkg), macOS and packaging are covered in
[docs/DEVELOPING.md](docs/DEVELOPING.md).

To try the Jellyfin side without a server, `tools/mock-jellyfin.py`
serves a folder of your own comics as a small fake Jellyfin server:
`python3 tools/mock-jellyfin.py ~/Comics --port 8097`, then add
`http://localhost:8097` in Settings → Jellyfin.

## Where things live

- `src/app.rs` and `src/app/`: the window, the library, the reader and
  the settings, each view in its own file.
- `src/comic.rs`, `src/epub.rs`: reading comic archives and EPUBs.
- `src/library.rs`, `src/scan.rs`: the library database and folder scans.
- `src/jellyfin.rs`, `src/remote_zip.rs`, `src/remote_rar.rs`: the
  Jellyfin client and streaming pages over the network.
- `src/platform/`: anything specific to Linux, Windows or macOS, and
  following each desktop's colours (`look.rs`).

The interface uses [libcosmic](https://github.com/pop-os/libcosmic),
through a small fork with a few changes for the reader (listed above the
dependency in `Cargo.toml`). If your change belongs in libcosmic itself,
mention it in the PR and it can go into the fork.

## Pull requests

- Keep each PR to one thing; small PRs get merged faster.
- Run `cargo test` (and `cargo clippy` if you can) before sending it.
- Say what you changed and how you tried it. For anything visible, a
  screenshot or short clip is great.
- If it only works on, or was only tried on, one platform, say which.
- Write commit messages that say what changed and why; the history is
  kept as it is, not squashed.
- Don't bump the version or edit release notes: that happens when a
  release is made.

Much of Nebula was written with the help of AI (Claude), and its commits
say so. PRs made with AI tools are welcome too; just mention it in the PR
and make sure you've tried the change yourself.

## License

Nebula is GPL-3.0. By sending a pull request you agree that your
contribution is released under the same license.
