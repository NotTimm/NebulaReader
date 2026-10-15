# Screenshots

Store screenshots referenced from the metainfo (captions are in
`docs/listing.md`):

| File | Shows |
|------|-------|
| `library.png` | Library grid with the Continue Reading shelf (default shot) |
| `series.png` | A matched series page: description, publisher, issue list with progress |
| `reader.png` | The comic reader on a strong page, not zoomed |
| `book.png` | The EPUB reader mid-chapter, body text visible |

## Capturing

On a COSMIC session:

```
res/screenshots/capture.sh                        # all four
res/screenshots/capture.sh --profile ~/cc-demo    # with a separate demo library
res/screenshots/capture.sh reader                 # redo one
res/screenshots/capture.sh --self-test            # check capture works
```

It launches `cosmic-comic`, tells you what to set up for each view, then
opens COSMIC's screenshot picker (`cosmic-screenshot`, via
xdg-desktop-portal-cosmic). Pick **Window**, click the Cosmic Comic window,
and save. The PNG is copied here with its metadata stripped. `--full` takes
whole-screen captures without the picker if that doesn't work, but then you
crop by hand.

## Guidelines

These follow Flathub's screenshot rules:

- **Window only.** No desktop, panel or dock. The picker's Window mode
  includes the COSMIC window frame and corners, which is fine.
- **The same size for every shot.** The app opens at 1024×768; don't resize
  or maximize between shots. If you resize, make it roughly 16:9 (e.g.
  1280×720) and do the same for all four. Take them on a display at 100%
  scale so the pixel size matches the logical size.
- **Default look.** Use the stock COSMIC theme (dark or light, but the same
  for all four), the default accent colour, and the app's default settings
  (Theme backdrop, no theater mode). Close the settings drawer and info
  panel.
- **No personal data.** Nothing that shows your home folder, username or
  file paths. Use a demo library (`--profile`) made of public domain or
  freely licensed comics and books (e.g. Digital Comic Museum, Project
  Gutenberg EPUBs), not your own downloads. Notifications off.
- **Real content.** A populated library with matched covers and
  descriptions, not an empty state or loading spinners. Wait for
  "Looking up metadata…" to finish first.
- **Small files.** Keep each PNG under 1–3 MB. The script warns above 3 MB.
  Busy art compresses badly, so choose a calmer page if one is too big.
- **PNG, no edits.** No added borders, captions, device frames or mock-ups.
  The caption goes in the metainfo.
