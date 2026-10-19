# Developing Nebula Reader

Building from source, packaging, releasing and the libcosmic fork.
For using the app, see the [README](../README.md).

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
[`packaging/flathub/`](../packaging/flathub/) has what's needed to submit to
Flathub, which is also how the app gets into the COSMIC Store.

Tooling needed per format (each script tells you if something's missing):

| Format   | Needs |
|----------|-------|
| deb      | `dpkg-dev` |
| rpm      | `rpm-build` |
| AppImage | `curl` (fetches `appimagetool` on first run) |
| Flatpak  | `flatpak`, `flatpak-builder`, `python3` with `aiohttp`, `tomlkit`, `pyyaml` |
| macOS dmg | a Mac with Xcode's command line tools and `brew install libarchive pkgconf dylibbundler` (`just package-macos`) |

Building also needs the usual native headers: `libarchive`, `openssl` and
`libxkbcommon` development packages plus `pkg-config` and a C compiler
(`sudo apt install build-essential pkg-config libarchive-dev libssl-dev
libxkbcommon-dev` / `sudo dnf install gcc pkgconf-pkg-config
libarchive-devel openssl-devel libxkbcommon-devel`). A `.deb` meant for
distribution should be built on Debian/Ubuntu so `dpkg-shlibdeps` can
compute its dependencies; on other hosts the script falls back to a
fixed list that's only suitable for local testing.

### winget

The Windows installer is published to winget as `NotTimm.NebulaReader`.
Once the first version is in
[microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs), the
`winget` job in `release.yml` submits each new release automatically. It
needs a repository secret `WINGET_TOKEN`: a classic personal access token
with the `public_repo` scope, from the account that owns a fork of
winget-pkgs. Without the secret the job just skips.

The first version is submitted by hand from Windows:

```
winget install Microsoft.WingetCreate
wingetcreate new https://github.com/NotTimm/NebulaReader/releases/download/v<version>/NebulaReader-<version>-windows-x64-setup.exe
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
   `res/io.github.NotTimm.NebulaReader.metainfo.xml`), commit, push.
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

### Signing and in-app updates

Every published file has a minisign signature next to it
(`<name>.minisig`, trusted comment `file:<name>`), made by the `sign` job
in `release.yml` from two repository secrets: `MINISIGN_KEY` (the whole
encrypted secret key file) and `MINISIGN_PASSWORD`. The job checks every
signature against `PUBLIC_KEY` in `src/updater.rs` (key ID
`AFE5BA3AD81D4B31`), and a publish fails without the secrets.

The Windows installer, the Windows zip and the Mac app update themselves
(`src/updater.rs`): they download their package for the newest release
(the setup `.exe`, the `.zip`, or `NebulaReader-<version>-macos-arm64.app.tar.gz`,
which `build-macos.sh` packs), refuse it unless its signature checks out,
and replace themselves when the user restarts. The Windows job tests the
installer the way the updater runs it, over a running older install.

The secret key's only copies are the owner's (backed up) and the secret.
If it's ever lost or leaked: make a new pair, ship a release signed with
the old key whose `PUBLIC_KEY` is the new one, then switch the secrets.
Without the old key, installed copies can't verify updates and have to be
updated by hand once.

## The libcosmic fork

`Cargo.toml` points at `github.com/NotTimm/libcosmic`, a small fork of
`pop-os/libcosmic`. The patches it carries (pinch forwarding, the
zoom viewer changes, the header bar fix) are listed in the comment above
the `libcosmic` dependency.

Upstream keeps `iced` and `cosmic-icons` as git submodules. The fork
copies them into the repo instead, since our patches change iced.

**Keep it in sync.** Run `tools/libcosmic-drift.sh` before each release
and now and then in between. It reports how many upstream commits the fork
is missing, lists upstream fixes since the last sync, and checks that
`Cargo.toml` pins the fork's latest commit.

To sync, in a clone of the fork with `upstream` pointing at
`pop-os/libcosmic`:

1. Note the iced and cosmic-icons revisions upstream pins:
   `git ls-tree upstream/master iced cosmic-icons`.
2. Save our iced patch: download the iced revision the fork was last synced
   to (`git ls-tree $(git merge-base master upstream/master) iced`) and
   `diff -ruN -x .cargo -x Cargo.lock` it against the fork's `iced/`.
3. In a separate worktree, `git merge --no-commit upstream/master`.
   Resolve the conflicts by deleting upstream's `iced` and `cosmic-icons`
   gitlinks and its `.github/workflows`, then replace `iced/` and
   `cosmic-icons/` with the new revisions (drop `iced/.cargo` and
   `iced/Cargo.lock`) and apply the saved patch in `iced/`.
4. Check that `git diff upstream/master` outside `iced/` and
   `cosmic-icons/` shows only our patches. Commit the merge, push it to a
   branch, point `Cargo.toml` at it and run `cargo check` and `cargo test`.
5. Fast-forward the fork's `master` to the merge and push. Never rebase or
   force-push: released Nebula versions pin old fork commits, which must
   stay fetchable. `nebula-0.1.2` tags the commit 0.1.2 was built from.
