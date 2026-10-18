# Store listing — Nebula Reader

Source text for the AppStream metainfo (`res/io.github.NotTimm.NebulaReader.metainfo.xml`),
Flathub and the COSMIC Store. Everything inside the fenced `xml` blocks is valid
AppStream description markup (`<p>`, `<ul>`/`<li>` only — no links, no
emphasis, no nested lists) and can be pasted as-is.

## Name

```
Nebula Reader
```

## Summary

28 characters, no trailing period, doesn't repeat the app name:

```xml
<summary>Read comics, manga and books</summary>
```

## Description

Paste between `<description>` and `</description>`:

```xml
<p>
  Nebula Reader is a reader for comic books, manga and EPUB books, built for
  the COSMIC desktop. Point it at the folders where your collection already
  lives and it turns them into a library of series, with covers, per-issue
  reading progress and a Continue Reading shelf.
</p>
<p>
  Messy folder and file names are cleaned up automatically, so a folder
  called “Civil War 001-007 (2006-2007)” shows up as Civil War. Series are
  then matched online, against AniList for manga and, with a free API key,
  ComicVine for Western comics, to fill in proper titles, descriptions,
  publishers and issue titles. If a match is wrong you can search again with
  your own terms.
</p>
<p>
  Comics open fast, even large CBR archives, and can be read one page or two
  at a time, zoomed and panned with the mouse wheel, a trackpad or a
  touchscreen. Books get a calm, Kindle-like page with adjustable type and
  paper-style themes. The app follows your COSMIC theme and keeps its data in
  the standard XDG folders.
</p>
<ul>
  <li>Library built from your own folders: series folders, chapter folders, publisher folders and loose issues</li>
  <li>Offline name clean-up, then online matching via AniList and ComicVine</li>
  <li>Series pages with an issue list, cover dates and read or unread marks</li>
  <li>Reading progress saved per issue, with a Continue Reading shelf</li>
  <li>CBZ, CBR and image folders, with ComicInfo.xml metadata</li>
  <li>Single or dual page view, fit page, theater mode and fullscreen</li>
  <li>Zoom and pan with the mouse wheel, trackpad or pinch</li>
  <li>EPUB books with the book’s own table of contents</li>
  <li>Adjustable font, size, line spacing and line width, with justified text</li>
  <li>Paper, Sepia, Dark and Night reading themes</li>
  <li>Time left in the chapter and progress through the book</li>
  <li>Drag and drop files or folders to open them</li>
</ul>
```

Notes on honesty, so the text stays true as the code changes:

- ComicVine needs the user's own (free) API key; the text says so.
- "Opens fast" refers to the one-pass CBR extraction + disk cache in
  `src/comic.rs`.
- Book-reader bullets (typography, themes, TOC, time left) describe the EPUB
  reader work landing alongside this listing; drop them if that slips.
- No claims about syncing, editing, PDF, CB7/CBT, or OPDS — none exist.

## Keywords

For metainfo:

```xml
<keywords>
  <keyword>comic</keyword>
  <keyword>comics</keyword>
  <keyword>manga</keyword>
  <keyword>cbz</keyword>
  <keyword>cbr</keyword>
  <keyword>epub</keyword>
  <keyword>ebook</keyword>
  <keyword>book</keyword>
  <keyword>reader</keyword>
  <keyword>library</keyword>
  <keyword>comicvine</keyword>
  <keyword>anilist</keyword>
</keywords>
```

The `.desktop` file carries the same list in `Keywords=`.

## Categories

`Graphics;Viewer;` is the primary pair (that's where comic viewers such as
YACReader and MComix sit on Flathub). `Office;Viewer;` or `Literature` aren't
needed. `COSMIC;` is kept in the `.desktop` file so the COSMIC launcher
groups it with other COSMIC apps.

```xml
<categories>
  <category>Graphics</category>
  <category>Viewer</category>
</categories>
```

## Screenshots

Captured with `res/screenshots/capture.sh` (see `res/screenshots/README.md`).
The library shot is the default (`type="default"`). Captions are short,
sentence case, no trailing period:

| File | Caption |
|------|---------|
| `library.png` | Your comics, manga and books as one library |
| `series.png` | Series pages with descriptions, chapters and progress |
| `book.png` | Books turn pages like an e-reader, one or two at a time |
| `book-settings.png` | Choose the typeface, size, spacing and theme |
| `reader.png` | Read comics one page or two at a time |
| `reader-zoom.png` | Zoom in on any panel |
| `settings.png` | Takes on the look of your desktop |

```xml
<screenshots>
  <screenshot type="default">
    <image>https://raw.githubusercontent.com/NotTimm/NebulaReader/v0.1.3/res/screenshots/library.png</image>
    <caption>Your comics, manga, books and Jellyfin server in one library</caption>
  </screenshot>
  <screenshot>
    <image>https://raw.githubusercontent.com/NotTimm/NebulaReader/v0.1.3/res/screenshots/series.png</image>
    <caption>Series pages with descriptions, release dates and progress</caption>
  </screenshot>
  <screenshot>
    <image>https://raw.githubusercontent.com/NotTimm/NebulaReader/v0.1.3/res/screenshots/book.png</image>
    <caption>Books turn pages like an e-reader, one or two at a time</caption>
  </screenshot>
  <screenshot>
    <image>https://raw.githubusercontent.com/NotTimm/NebulaReader/v0.1.3/res/screenshots/book-settings.png</image>
    <caption>Choose the typeface, size, spacing and theme</caption>
  </screenshot>
  <screenshot>
    <image>https://raw.githubusercontent.com/NotTimm/NebulaReader/v0.1.3/res/screenshots/reader.png</image>
    <caption>Read comics and manga in theater mode, one page or two</caption>
  </screenshot>
  <screenshot>
    <image>https://raw.githubusercontent.com/NotTimm/NebulaReader/v0.1.3/res/screenshots/reader-zoom.png</image>
    <caption>Zoom in on any panel</caption>
  </screenshot>
  <screenshot>
    <image>https://raw.githubusercontent.com/NotTimm/NebulaReader/v0.1.3/res/screenshots/settings.png</image>
    <caption>Takes on your desktop's look, or a theme of your own</caption>
  </screenshot>
</screenshots>
```

Flathub wants stable image URLs; once a release that includes them is tagged (0.1.2 or later), point these at the
tag (`.../NebulaReader/v0.1.3/res/screenshots/...`) instead of `master`.
