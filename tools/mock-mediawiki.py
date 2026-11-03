#!/usr/bin/env python3
"""A tiny stand-in for a Fandom wiki's API, to try Nebula's Fandom page
without the internet: site information, one main page, one character page
(an infobox with tabbed pictures, a tabber, a gallery, picture links),
search and random. Pictures are plain coloured squares.

    python3 tools/mock-mediawiki.py --port 8099

then follow `http://localhost:8099` on the Fandom page.
"""
import argparse
import json
import struct
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

PORT = 8099


def png(w, h, rgb):
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))

    def chunk(kind, data):
        c = kind + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def img(name, w, h):
    base = f"http://localhost:{PORT}/img/{name}/revision/latest"
    return (f'<a href="{base}?cb=1" class="image"><img src="data:image/gif;base64,R0lGOD" '
            f'data-src="{base}/scale-to-width-down/{w}?cb=1" width="{w}" height="{h}" class="lazyload" /></a>')


MAIN = f"""<div class="mw-parser-output"><p><b>Welcome to the Mock Wiki</b>, the wiki about a made-up series that anyone can edit.</p>
<h2><span class="mw-headline" id="Explore">Explore</span></h2>
<p><a href="/wiki/Characters"><img src="http://localhost:{PORT}/img/characters.png/revision/latest/scale-to-width-down/150?cb=1" width="150" height="150" /></a>
<a href="/wiki/Eren_Yeager"><img src="http://localhost:{PORT}/img/eren.png/revision/latest/scale-to-width-down/150?cb=1" width="150" height="150" /></a>
<a href="/wiki/Episodes"><img src="http://localhost:{PORT}/img/episodes.png/revision/latest/scale-to-width-down/150?cb=1" width="150" height="150" /></a></p>
<h2><span class="mw-headline" id="Featured">Featured article</span></h2>
<p><a href="/wiki/Eren_Yeager" title="Eren Yeager">Eren Yeager</a> is the main character of the series.</p></div>"""

EREN = f"""<div class="mw-parser-output"><aside role="region" class="portable-infobox pi-background pi-theme-character">
<h2 class="pi-item pi-title" data-source="name">Eren Yeager</h2>
<div class="pi-media-collection pi-image-collection wds-tabber"><div class="wds-tabs__wrapper"><ul class="wds-tabs">
<li class="wds-tabs__tab wds-is-current"><div class="wds-tabs__tab-label">Anime</div></li><li class="wds-tabs__tab"><div class="wds-tabs__tab-label">Manga</div></li></ul></div>
<div class="wds-tab__content wds-is-current"><figure class="pi-item pi-image">{img("eren-anime.png", 268, 350)}<figcaption class="pi-item-spacing pi-caption">In the anime</figcaption></figure></div>
<div class="wds-tab__content"><figure class="pi-item pi-image">{img("eren-manga.png", 268, 380)}</figure></div></div>
<section class="pi-item pi-group"><h2 class="pi-item pi-header">Character</h2>
<div class="pi-item pi-data" data-source="species"><h3 class="pi-data-label">Species</h3><div class="pi-data-value"><a href="/wiki/Human" title="Human">Human</a><br />Titan Shifter</div></div>
<div class="pi-item pi-data"><h3 class="pi-data-label">Relatives</h3><div class="pi-data-value"><a href="/wiki/Grisha_Yeager">Grisha Yeager</a> (Father)<br /><a href="/wiki/Carla_Yeager">Carla Yeager</a> (Mother)</div></div>
<div class="pi-item pi-data"><h3 class="pi-data-label">Status</h3><div class="pi-data-value">Alive</div></div></section>
<table class="pi-horizontal-group"><caption class="pi-header">Debut</caption><thead><tr><th class="pi-data-label">Manga</th><th class="pi-data-label">Anime</th></tr></thead>
<tbody><tr><td class="pi-data-value">Chapter 1</td><td class="pi-data-value">Episode 1</td></tr></tbody></table></aside>
<p><b>Eren Yeager</b> is the main protagonist of <i><a href="/wiki/Mock_Series" title="Mock Series">the Mock Series</a></i>. He joined the <a href="http://localhost:{PORT}/wiki/Survey_Corps">Survey Corps</a> to fight.<sup id="cite_ref-1" class="reference"><a href="#cite_note-1">[1]</a></sup> His <a href="/wiki/Rumbling?action=edit&amp;redlink=1" class="new">Rumbling</a> began later. """ + "Lorem ipsum dolor sit amet, consectetur adipiscing elit. " * 12 + f"""</p>
<h2><span class="mw-headline" id="Appearance">Appearance</span></h2>
<figure class="thumb tright show-info-icon">{img("eren-child.png", 180, 240)}<figcaption class="thumbcaption"><p class="caption">Eren as a child</p></figcaption></figure>
<div class="tabber wds-tabber"><div class="wds-tabs__wrapper with-bottom-border"><ul class="wds-tabs"><li class="wds-tabs__tab wds-is-current"><div class="wds-tabs__tab-label"><a href="#">Anime</a></div></li><li class="wds-tabs__tab"><div class="wds-tabs__tab-label"><a href="#">Manga</a></div></li></ul></div>
<div class="wds-tab__content wds-is-current"><p>In the anime, Eren has green eyes and dark brown hair. """ + "Sed do eiusmod tempor incididunt ut labore. " * 8 + """</p></div>
<div class="wds-tab__content"><p>In the manga, Eren's eyes are drawn grey.</p></div></div>
<h3><span class="mw-headline" id="Gallery">Gallery</span></h3>
<div id="gallery-0" class="wikia-gallery"><div class="wikia-gallery-row">""" + "".join(
    f'<span class="wikia-gallery-item"><div class="thumb"><div class="gallery-image-wrapper">{img(f"g{i}.png", 185, 104)}</div></div><div class="lightbox-caption">Scene {i}</div></span>'
    for i in range(1, 7)) + """</div></div>
<h2><span class="mw-headline" id="References">References</span></h2><ol class="references"><li id="cite_note-1">Chapter 1.</li></ol>
<table class="navbox"><tr><td>Lots of links</td></tr></table></div>"""

PAGES = {"Mock Wiki": MAIN, "Eren Yeager": EREN}


class Handler(BaseHTTPRequestHandler):
    def send(self, body, kind="application/json"):
        if isinstance(body, str):
            body = body.encode()
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        u = urlparse(self.path)
        if u.path.startswith("/img/"):
            name = u.path.split("/")[2]
            h = zlib.crc32(name.encode())
            return self.send(png(64, 64, ((h >> 16) & 255, (h >> 8) & 255, h & 255)), "image/png")
        if u.path == "/wikia.php":
            return self.send(":root{--theme-accent-color:#2f6f4f;}", "text/css")
        if u.path != "/api.php":
            self.send_response(404)
            self.end_headers()
            return
        q = {k: v[0] for k, v in parse_qs(u.query).items()}
        if q.get("meta") == "siteinfo":
            return self.send(json.dumps({"query": {
                "general": {"sitename": "Mock Wiki", "mainpage": "Mock Wiki", "server": f"http://localhost:{PORT}",
                            "scriptpath": "", "articlepath": "/wiki/$1",
                            "logo": f"http://localhost:{PORT}/img/logo.png/revision/latest?cb=1"},
                "rightsinfo": {"url": "https://creativecommons.org/licenses/by-sa/3.0/", "text": "CC BY-SA"}}}))
        if q.get("action") == "parse":
            title = q.get("page", "").replace("_", " ")
            if title not in PAGES:
                return self.send(json.dumps({"error": {"code": "missingtitle", "info": "The page you specified doesn't exist."}}))
            return self.send(json.dumps({"parse": {"title": title, "text": PAGES[title]}}))
        if q.get("list") == "random":
            return self.send(json.dumps({"query": {"random": [{"title": "Eren Yeager"}]}}))
        if q.get("list") == "search":
            return self.send(json.dumps({"query": {
                "search": [{"title": "Eren Yeager", "snippet": '<span class="searchmatch">Eren</span> is the main protagonist'}],
                "pages": [{"title": "Eren Yeager", "thumbnail": {"source": f"http://localhost:{PORT}/img/eren.png/revision/latest/scale-to-width-down/160?cb=1"}}]}}))
        self.send(json.dumps({}))


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=PORT)
    PORT = ap.parse_args().port
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
