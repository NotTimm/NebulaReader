# Nebula Reader: English (the reference and fallback for every language).
# How to translate: i18n/README.md.

# This language's name in itself, shown in Settings → Language.
language-name = English

## Header bar and reader buttons

back-to-series = Back to series (Esc)
back-to-library-esc = Back to library (Esc)
back-to-library = Back to library
search-library = Search library…
downloads = Downloads
open-file = Open File
add-library-folder = Add Library Folder
previous-chapter = Previous chapter
chapter = Chapter
next-chapter = Next chapter
contents-tip = Contents (C)
typography-tip = Text, layout and page settings (A)
book-details-tip = Book details (I)
settings-tip = Settings (S)
# Page counter when a series has several chapters: chapter, page, pages.
chapter-page-counter = Ch.{ $chapter }  ·  { $page } / { $pages }
previous-page = Previous page (←)
page = Page
next-page = Next page (→)
chapters-tip = Chapters (C)
show-one-page = Show one page (L)
show-two-pages = Show two pages (L)
stop-zooming = Stop zooming (M)
# $modifier is the key name: Ctrl, or Cmd on a Mac.
zoom-tip = Zoom and pan (M, or scroll; Shift + scroll for fine steps, { $modifier } + scroll to move the page)
whole-page-tip = Whole page (Space)
fit-width-tip = Fit to Width (Space)
copy-page = Copy page image (P)
fit-page = Fit page to window
details-tip = Details (I)
open-a-file = Open a file
full-screen = Full screen (F)
leave-full-screen = Leave full screen (F or Esc)
# Shown briefly at the bottom of the reader.
fit-width-note = Fit to Width
whole-page-note = Whole Page
# Where the open comic comes from, next to the home button.
source-local = Local

## Side panels

settings = Settings
details = Details
reading-panel = Reading
properties = Properties

## Reader placeholders

decoding-page = Decoding page…
creating-index = Creating index of CBR comic…
loading-comic = Loading comic…
start-reading = Open a comic or book to start reading
# $modifier: Ctrl, or Cmd on a Mac.
reader-keys = ← → page  ·  Space fit to width  ·  C chapters  ·  L layout  ·  M zoom  ·  scroll or pinch to zoom (Shift: finer, { $modifier }: move)  ·  T theater  ·  F fullscreen  ·  I info  ·  P copy
drop-hint = You can also drag and drop a file or folder here.
open-comic-or-book = Open Comic or Book
file-filter-comics = Comics & Books
file-filter-themes = COSMIC themes and KDE colour schemes

## Settings

appearance = Appearance
language = Language
language-system = Match system
style = Style
style-auto = Match desktop
title-bar-restart = The title bar changes the next time Nebula starts.
colors = Colors
colors-system = Match system
themes-hint = Add your own: COSMIC themes (.ron, exported from COSMIC Settings or from cosmic-themes.org) and KDE color schemes (.colors).
import-theme = Import Theme…
open-themes-folder = Open Themes Folder
theme-error = Couldn't use that theme: { $error }
buttons = Buttons
corners-auto = Match style
corners-round = Round
corners-slightly-round = Slightly round
corners-square = Square
background = Background
background-theme = Theme
background-black = Black
background-opacity = Background opacity  ({ $percent }%)
theater-mode = Theater mode
reading = Reading
two-page-spread = Two-page spread
fullscreen-status = Clock and battery in full screen
metadata = Metadata
fetch-metadata = Look up series online (AniList, ComicVine)
comicvine-key-set = ComicVine API key: set ✓
comicvine-key-unset = ComicVine API key: not set — needed for Western comics (free at comicvine.gamespot.com/api)
replace-key = Replace key…
paste-key = Paste API key…
save = Save
rescan-library = Rescan Library
refresh-metadata = Refresh All Metadata
library-folders = Library folders
no-library-folders = No library folders yet. Add one to have its comics scanned in automatically.
folder-not-found = { $folder }  (not found — hidden)
folder-path-placeholder = Type or paste a folder path…
add = Add
browse = Browse…
invalid-file-path = invalid file path: { $url }
invalid-folder-path = invalid folder path: { $url }
unsupported-scheme = unsupported scheme: { $scheme }
not-a-folder = not a folder: { $path }
not-a-local-file = not a local file

## Settings → About and updates

about = About
version = Version { $version }
version-build = Version { $version } (build { $commit })
made-by = Made by Tim Singel  ·  { $install }
release-notes = Release notes
update-not-checked = Not checked yet
update-checking = Checking…
update-up-to-date = You have the latest version
update-check-failed = Couldn't check: { $error }
update-available-heading = Version { $version } is available
check-now = Check now
check-on-start = Check for updates on start
updated-banner = Nebula updated to { $version }.
whats-new = What's new
release-page = Release page
update-available = Nebula { $version } is available.
update-available-how = Nebula { $version } is available. { $how }
update-now = Update now
update-downloading-percent = Downloading Nebula { $version }… { $percent }%
update-downloading-mb = Downloading Nebula { $version }… { $mb } MB
update-ready = Nebula { $version } is ready. Restart Nebula to finish updating.
restart-now = Restart now
update-failed = Couldn't update: { $error }
try-again = Try again

## Details panel (comics)

issue-heading = #{ $number } — { $title }
issue-number = #{ $number }
chapter-heading = Chapter { $chapter } — { $name }
page-of = Page { $page } of { $pages }
released = Released { $date }
issue-of-series = Issue { $issue } of { $issues } in this series
view-issue-page = View issue page
issues-read = { $read }/{ $total } read
source-comicvine = Source: ComicVine
source-anilist = Source: AniList
no-online-match = No online match
not-looked-up = Not looked up yet
ci-series = Series
ci-number = Number
ci-volume = Volume
ci-year = Year
ci-publisher = Publisher
ci-writer = Writer
ci-artist = Artist
ci-inker = Inker
ci-colorist = Colorist
ci-letterer = Letterer
ci-cover = Cover
ci-genre = Genre
ci-field = { $label }: { $value }
name-parsing = Name parsing
name-parsed = Series “{ $series }” · issue { $issue } · year { $year }

## Dropdown choices

source-all = All libraries
spacing-tight = Tight
spacing-normal = Normal
spacing-relaxed = Relaxed
spacing-loose = Loose
layout-single = Single page
layout-two = Two pages
layout-scroll = Scroll
width-narrow = Narrow
width-medium = Medium
width-wide = Wide
theme-auto = Auto
theme-paper = Paper
theme-sepia = Sepia
theme-dark = Dark
theme-night = Night

## Library

# A book card's subtitle when its author isn't known.
book = Book
# A book card with progress: author (or "Book"), then the chapter reached.
book-chapter = { $what } · ch. { $chapter }
issues-finished = { $count } issues · finished
issue-count = { $count ->
    [one] { $count } issue
   *[other] { $count } issues
}
finished = Finished
page-count = { $count ->
    [one] { $count } page
   *[other] { $count } pages
}
chapter-number = Chapter { $chapter }
# A Continue Reading card: issue label and percent read.
issue-progress = { $label } · { $percent }%
downloads-error = Downloads: { $error }
manage-downloads = Manage Downloads
servers-offline = { $servers } can't be reached right now, so only your downloads from it are shown. Nebula keeps trying.
scanning-library = Scanning library… ({ $left } left)
looking-up-metadata-progress = Looking up metadata… { $percent }% ({ $left } of { $total } left)
library-empty-hint = Add a library folder in Settings, open a file, or drag one in.
no-search-match = Nothing matches that search.
library-empty = Your library is empty
no-results = No results
open-folder = Open Folder
continue-reading = Continue Reading
library = Library
opened-elsewhere = Opened Elsewhere

## Series page

volume = Vol. { $volume }
read = Read
chapter-count = { $count ->
    [one] { $count } chapter
   *[other] { $count } chapters
}
read-count = { $count } read
on-disk-as = On disk as “{ $name }”
matched-comicvine = Matched on ComicVine
matched-anilist = Matched on AniList
no-match-folder-name = No online match — using the folder name
lookup-failed = Lookup failed: { $error }
looking-up-metadata = Looking up metadata…
view-page = View page
# $issue is "#3" or "Ch. 3".
continue-issue = Continue { $issue }
read-issue = Read { $issue }
start-reading-button = Start Reading
read-again = Read Again
mark-read-one = Mark Read
mark-unread-one = Mark Unread
mark-all-read = Mark All Read
mark-all-unread = Mark All Unread
search-online-as = Search online as…
rematch = Re-match
use-folder-name = Use Folder Name
read-check = ✓ Read
unread = Unread
mark-unread = Mark unread
mark-read = Mark read
build-page-index = Build Page Index
# Issue labels in lists: chapter style for manga, number style for comics.
issue-label-manga = Ch. { $number }
issue-label-comic = #{ $number }

## Books

no-chapters = This book has no chapters.
next-chapter-button = Next: { $title }  →
the-end = The End
less-than-minute-left = Less than a minute left in chapter
minutes-left = { $minutes } min left in chapter
pages-of = Pages { $first }–{ $last } of { $pages }
image-hint = Scroll or pinch to zoom · drag to move · Esc to go back
back-to-book = Back to book
back-to-page = Back to the page (Esc)
contents = Contents
close-contents = Close contents (C)
book-stats = { $chapters } chapters · about { $words } words · ~{ $hours } h to read
chapter-of = Chapter { $chapter } of { $chapters }: { $title }
smaller-text = Smaller text (Ctrl −)
larger-text = Larger text (Ctrl +)
text = Text
text-size = Size  ({ $size } px)
typeface = Typeface
font-serif = Serif
font-sans = Sans
line-spacing = Line spacing
justify-text = Justify text
page-section = Page
layout = Layout
line-width = Line width
theme = Theme
keys = Keys
book-keys = → / Space / Page Down, or click the right edge: next page  ·  ← / Page Up, or click the left edge: previous page  ·  Mouse wheel: turn pages (scrolls in Scroll layout)  ·  C: contents  ·  F: full screen  ·  Ctrl 0: default size

## Chapter list

chapters = Chapters
# Where you are in a series: "Ch. 3 of 40" (manga) or "# 3 of 40".
chapter-position-manga = Ch. { $number } of { $total }
chapter-position-comic = # { $number } of { $total }
chapter-row-manga = Chapter { $number }
chapter-row-comic = # { $number }
volume-heading = Volume { $volume }  ·  { $chapter }
volume-group = Volume { $volume }  ({ $chapters })
# Chapter ranges for jumping in long lists: "Ch. 1–25", "#1–25".
range-one-manga = Ch. { $from }
range-many-manga = Ch. { $from }–{ $to }
range-one-comic = #{ $from }
range-many-comic = #{ $from }–{ $to }

## Page indexes (Jellyfin CBRs)

index-solid = Packed as one block (solid): downloaded whole when you read it
index-built = Page index built: opens right away and streams page by page
index-already-built = Those chapters already have their page index.
index-stopped = Stopped after { $built } of { $total } page indexes.
index-built-one = Page index built.
index-built-many = Built { $count } page indexes.
index-solid-count = { $count ->
    [one] 1 chapter is packed as one block, so it's fetched whole when you read it.
   *[other] { $count } chapters are packed as one block, so they're fetched whole when you read them.
}
index-failed = { $count } couldn't be read from the server.
creating-indexes = Creating indexes of CBR comics… { $done } of { $total }
stop = Stop

## Jellyfin

jf-not-signed-in = Not signed in to this Jellyfin server any more. Add it again in Settings.
jf-no-chapters = This series has no chapters on the server.
jf-offline-nothing-downloaded = { $server } can't be reached, and nothing from this series is downloaded.
no-pages-found = No pages found.
download-and-read = Download and Read
download = Download
dont-ask-again = Don't Ask Again
cancel = Cancel
not-now = Not Now
download-size-question = Download { $size }?
solid-explainer = “{ $name }” can't be read a page at a time (it's packed as one solid block), so the whole file has to come from the server before it opens.
solid-next-chapter = The next chapter, “{ $name }”, has to be downloaded whole ({ $size }).
jf-looking-for = Looking for Jellyfin at { $address }…
jf-connected = Connected to { $server }. Getting a sign-in code…
jf-quick-connect-failed = Couldn't start Quick Connect: { $error }
jf-approved = Approved. Signing in…
jf-no-quick-connect = { $server } has Quick Connect turned off, so sign in with your Jellyfin username and password. They're used once to get a token and not saved.
jf-code-expired = The code expired. Click Connect to get a new one.
jf-signing-in = Signing in…
jf-signed-in = Signed in as { $user }. Loading your books…
jf-signed-out = Signed out of { $server }.
jf-unreachable-showing-downloads = { $server } can't be reached; showing your downloads from it.
jf-load-failed = Couldn't load { $server }: { $error }
jf-next-chapters-failed = Couldn't load the next chapters: { $error }
jf-progress-offline = Server offline: progress is kept here and sent when it's back
jf-progress-failed = Couldn't save progress: { $error }
# Hover text on the server name while reading from Jellyfin.
jf-item-book-cached = This book: downloaded to the cache on this computer
jf-item-streamed = This chapter: streamed from the server, not cached
jf-item-cached = This chapter: downloaded to the cache on this computer
jf-item-loading = This chapter: loading…
jf-online = Online
jf-offline = Offline: the server isn't answering
jf-checking = Checking the connection…
jf-progress-saving = Progress: saving…
jf-just-now = just now
jf-minutes-ago = { $minutes } min ago
jf-hours-ago = { $hours } h ago
jf-progress-saved = Progress: saved to the server { $ago }
jf-progress-saved-later = Progress: saved to the server after a few seconds on a page
jf-signed-in-as = Signed in as { $user }
jf-syncing = Syncing…
jf-unreachable-short = { $url }  ·  can't be reached, showing downloads
jf-sync-now = Sync now
sign-out = Sign out
jf-ask-big-fetch = Ask before downloading big chapters that can't be streamed
jf-approve-code = Approve this code in Jellyfin on { $server }:
jf-quick-connect-help = In Jellyfin, open your profile, then Quick Connect, and enter the code. This window signs in by itself once you've approved it.
jf-open = Open Jellyfin
jf-sign-in-to = Sign in to { $server }
username = Username
password = Password
sign-in = Sign in
jf-address = Jellyfin address
connect = Connect

## OPDS catalogs

opds-address = Catalog address
opds-username-optional = Username (if it asks for one)
opds-password-optional = Password (if it asks for one)
opds-help = Komga, Kavita, calibre-web, Stump and many other servers have an OPDS catalog. A username and password are kept on this computer and only sent to that server.
opds-untested = OPDS is new in Nebula and hasn't been tried on many servers yet. Please report what works and what doesn't.
opds-connecting = Opening { $address }…
opds-added = Added { $catalog }. Pick the folders to put in your library.
opds-removed = Removed { $catalog }.
opds-choose-folders = Choose Folders
opds-no-folders = No folders in the library yet
opds-folder-count = { $count ->
    [one] 1 folder in the library
   *[other] { $count } folders in the library
}
opds-remove = Remove
opds-no-file = this book has no file Nebula can get
opds-sign-in-refused = the catalog refused the username or password
opds-not-a-catalog = that address isn't an OPDS catalog
opds-no-search = this catalog can't be searched
opds-too-big = a folder in the library has too much in it to sync whole; take it out and add smaller folders inside it
opds-not-signed-in = This catalog isn't in Settings any more. Add it again there.
# The page for browsing a catalog and picking folders.
opds-loading = Loading…
opds-folder-failed = Couldn't open this folder: { $error }
opds-search = Search { $catalog }
opds-search-results = Results for “{ $query }”
opds-pick-help = Folders you add are synced into your library like a Jellyfin library: each folder of books becomes a series. Everything else stays here to browse and search.
opds-in-library = In your library
opds-add-folder = Add to Library
opds-remove-folder = Remove from Library
opds-load-more = Load More
opds-book-count = { $count ->
    [one] 1 book
   *[other] { $count } books
}
opds-not-readable = Can't be read in Nebula
opds-open-in-browser = Open in Browser
opds-back = Back
# Hover text on the catalog's name while reading from it.
opds-progress-local = Progress: kept on this computer (OPDS can't save it to the server)
opds-item-pages = This book: streamed from the catalog page by page
kind-opds-series = Series in an OPDS catalog
kind-opds-book = Book in an OPDS catalog
kind-opds-comic = Comic in an OPDS catalog

## Downloads

dl-not-in-library = No longer in the library
dl-unreachable = { $count ->
    [one] { $count } download is on a drive that isn't connected. Connect it to remove it.
   *[other] { $count } downloads are on a drive that isn't connected. Connect it to remove them.
}
dl-move-failed = { $count ->
    [one] { $count } download couldn't be moved and stayed in the old folder: { $error }
   *[other] { $count } downloads couldn't be moved and stayed in the old folder: { $error }
}
dl-folder-unavailable = The download folder { $folder } isn't available. Connect its drive, or pick another folder on the Downloads page.
dl-folder-unusable = can't use the download folder: { $error }
dl-that-folder-unusable = can't use that folder: { $error }
dl-wait-to-change = Wait for the downloads to finish before changing the folder.
dl-not-local-folder = not a local folder: { $url }
dl-choose-folder = Download Folder
dl-tip-download = Download for reading offline
dl-tip-queued = Waiting to download (click to cancel)
dl-tip-running = Downloading… { $percent }% (click to cancel)
dl-tip-done = Downloaded (click to remove the download)
dl-on-missing-drive = Downloaded to a drive that isn't connected
download-book = Download Book
dl-waiting-cancel = Waiting… (Cancel)
dl-running-cancel = Downloading { $percent }% (Cancel)
remove-download = Remove Download
downloaded = Downloaded
download-all = Download All
download-more = Download { $count } More
remove-downloads = Remove Downloads
dl-progress = Downloading { $current } of { $total }… { $percent }%
dl-none-yet-settings = No downloads yet. Use the download buttons on a Jellyfin series or book to read it offline.
dl-summary-settings = { $count ->
    [one] Downloads: { $count } item · { $size }
   *[other] Downloads: { $count } items · { $size }
}
dl-none-yet = Nothing downloaded yet. Jellyfin series and books have a Download button on their page, and downloads can be read when the server can't be reached.
dl-summary = { $count ->
    [one] { $count } item · { $size } in all
   *[other] { $count } items · { $size } in all
}
dl-folder = Download folder
dl-folder-missing = This folder isn't available. Connect its drive, or pick another folder. New downloads wait until then; nothing is lost.
dl-free-space = { $size } free on this drive
dl-separate-drive = On a separate drive. Each file is only kept once it's completely saved, so unplugging mid-download loses just that file — but eject the drive from your file manager before unplugging it.
change-folder = Change Folder…
use-default-folder = Use Default Folder
dl-change-moves = Changing the folder moves your downloads there.
dl-moving = Moving downloads to the new folder…
remove-all-downloads = Remove All Downloads
one-file = 1 file
drive-not-connected = drive not connected
dl-some-missing = { $count } on a drive not connected
open = Open
remove = Remove

## Card menu and Properties

show-details = Show Details
show-series = Show Series
download-issue = Download Issue
mark-as-read = Mark as Read
mark-as-unread = Mark as Unread
show-in-folder = Show in Folder
build-page-indexes = Build Page Indexes
kind-jellyfin-series = Series on a Jellyfin server
kind-jellyfin-book = Book on a Jellyfin server
kind-jellyfin-comic = Comic on a Jellyfin server
kind-folder-series = Series (a folder)
kind-group-series = Series (files grouped by name)
kind-epub = Book (EPUB)
kind-single-file = Single file
image-folder = Image folder
prop-type = Type
prop-format = Format
prop-server = Server
prop-library = Library
prop-location = Location
prop-chapters = Chapters
prop-issues = Issues
prop-pages = Pages
prop-pages-partial = { $pages } in { $known } of { $total } opened so far
prop-size = Size
prop-downloaded = Downloaded
prop-downloaded-of = { $count } of { $total } · { $size }
prop-read = Read
prop-read-of = { $read } of { $total } · { $percent }%
not-started = Not started
prop-added = Added
prop-last-opened = Last opened
prop-metadata = Metadata
metadata-from-files = From the files
metadata-none = None
prop-page-size = Page size
open-page = Open Page

## Dates: "Oct 19, 2026" and "Oct 2026"

month-1 = Jan
month-2 = Feb
month-3 = Mar
month-4 = Apr
month-5 = May
month-6 = Jun
month-7 = Jul
month-8 = Aug
month-9 = Sep
month-10 = Oct
month-11 = Nov
month-12 = Dec
date-day = { $month } { $day }, { $year }
date-month = { $month } { $year }

## How to update, by how Nebula was installed

how-flatpak = Installed from Flathub: your software center (or `flatpak update`) brings it in. Installed from a .flatpak file: download the new one from the release page.
how-appimage = Download the new AppImage from the release page and replace this one.
how-rpm = Download the new .rpm from the release page and install it with `sudo dnf install ./nebula-reader-*.rpm`.
how-deb = Download the new .deb from the release page and install it with `sudo apt install ./nebula-reader_*.deb`.
how-distro = Your system's package manager brings in new versions (`sudo dnf upgrade` on Fedora).
how-source = Run ./install.sh again in your checkout; it pulls the latest code and rebuilds.
how-windows-installer = Run `winget upgrade NotTimm.NebulaReader`, or download and run the new installer from the release page.
how-windows-portable = Download the new .zip from the release page and replace this folder.
how-mac = Download the new .dmg from the release page and drag Nebula Reader into Applications again.
how-unknown = Get the new version from the release page.
install-rpm = RPM package
install-deb = Debian package
install-distro = Distribution package
install-source = Built from source
install-windows-installer = Windows installer
install-windows-portable = Windows (portable)
install-mac = macOS app
install-unknown = Unknown
github-unreachable = couldn't reach GitHub: { $error }
github-http = GitHub answered HTTP { $status }

## Errors

jf-account = { $user } on { $server }
jf-unreachable = couldn't reach the server: { $error }
jf-signed-out-by-server = signed out by the server (sign in again)
jf-not-authorized = not authorized
jf-server-error = server error: HTTP { $status }
jf-unexpected-reply = unexpected reply: { $error }
jf-type-address = Type the address of your Jellyfin server.
jf-not-found = No Jellyfin server found at { $url } ({ $error }).
update-signature-mismatch = the signature is for another file ({ $file })
update-too-large = the update is unexpectedly large
update-missing-exe = the update has no { $file }
update-unpack-failed = couldn't unpack the update: { $error }
update-no-program = the update's app has no program in it
update-mac-location = Nebula is running from the disk image or Downloads, where it can't update itself. Drag Nebula Reader into Applications, open it from there and try again.
update-replace-failed = couldn't replace { $file }: { $error }
update-restart-failed = Nebula was updated but couldn't restart itself ({ $error }); start it again
update-place-failed = couldn't put the new app in place: { $error }
no-image-pages-in = no image pages found in { $path }
no-image-pages = no image pages found
not-file-or-folder = { $path } is neither a file nor a directory
no-chapters-under = no comic chapters or images found under { $path }
not-a-comic = not a comic: { $path }
no-readable-chapters = no readable chapters found in { $path }
# A book chapter without a title of its own.
section-number = Section { $number }
# A chapter file's name in the reader: number and subtitle.
chapter-with-subtitle = { $chapter } · { $subtitle }
theme-bad-name = not a theme file name
theme-no-colors = no colours found in this KDE colour scheme
