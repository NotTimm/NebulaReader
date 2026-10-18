# Getting Nebula Reader into Flathub (and the COSMIC Store)

The COSMIC Store lists Flathub apps out of the box, so publishing on
Flathub is how Nebula Reader shows up there (and in GNOME Software and
KDE Discover).

## Before submitting

These need a decision or an asset from you:

1. **App ID.** Decided: `io.github.NotTimm.NebulaReader`, which Flathub
   verifies through the GitHub account (no domain needed). The app was
   called Cosmic Comic before; "Cosmic Comics" (fr.nytuo.cosmiccomics) is
   another app on Flathub and "COSMIC Reader" is System76's PDF reader.
2. **Screenshots.** Flathub requires at least one, referenced from
   `res/io.github.NotTimm.NebulaReader.metainfo.xml` by URL, for example:

   ```xml
   <screenshots>
     <screenshot type="default">
       <image>https://raw.githubusercontent.com/NotTimm/NebulaReader/v0.1.0-beta.2/res/screenshots/library.png</image>
       <caption>The library</caption>
     </screenshot>
   </screenshots>
   ```

   `res/screenshots/capture.sh` takes them on a COSMIC desktop.
3. **A stable release.** Flathub's main repo wants a non-beta version; a
   `-beta` can go to the Flathub beta repo instead.

## Submitting

1. Tag the release (or run the *Release packages* workflow with
   **publish** ticked) and note the tag's commit.
2. Fork <https://github.com/flathub/flathub>, branch from `new-pr`, and add:
   - `io.github.NotTimm.NebulaReader.yml` from this folder, with `tag` and
     `commit` filled in
   - `cargo-sources.json`, generated for that tag:
     `git checkout <tag> && bash packaging/gen-cargo-sources.sh`
3. Open the PR against `new-pr`. A reviewer builds it and comments; after
   it's merged you get a `flathub/io.github.NotTimm.NebulaReader` repo, and
   later releases are PRs there that bump `tag`/`commit` and refresh
   `cargo-sources.json`.

Check locally first with Flathub's linter:

```
flatpak install flathub org.flatpak.Builder
flatpak run --command=flathub-build org.flatpak.Builder --install io.github.NotTimm.NebulaReader.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest io.github.NotTimm.NebulaReader.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder repo repo
```

Expect review questions about `--filesystem=home:ro`. The reason it's
there: library folders are scanned and rescanned on every start, which the
one-off file chooser portal doesn't cover.
