#!/usr/bin/env python3
"""A tiny fake Jellyfin server for testing Cosmic Comic's Jellyfin support.

Serves a folder of comics as a "books" library, with the same endpoints and
reply shapes a real Jellyfin 12.x server gives (checked against one), byte
range downloads, Quick Connect (auto-approved after a couple of polls) and
progress saving. Every progress update is printed, so you can watch what
the app sends.

    python3 tools/mock-jellyfin.py ~/Comics/Manga --port 8097

Then add http://localhost:8097 in Settings > Jellyfin. Folders become
series, comic files become chapters/one-shots. Not for real use: there is
no real authentication.
"""

import argparse
import hashlib
import json
import os
import re
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

TOKEN = "mock-token"
USER = {"Id": "mockuser", "Name": "tester"}
SERVER = {"Id": "mockserver", "ServerName": "mockfin", "Version": "12.2.0", "StartupWizardCompleted": True}
COMIC_EXT = (".cbz", ".cbr", ".zip", ".rar", ".epub")

ROOT = ""
ITEMS = {}  # id -> {"path", "parent"}
USER_DATA = {}  # id -> dict
QC = {"polls": 0}
LOCK = threading.Lock()


def item_id(path):
    return hashlib.md5(path.encode()).hexdigest()


def index_tree():
    ITEMS.clear()
    ITEMS["manga-lib"] = {"path": ROOT, "parent": None}
    for dirpath, dirnames, filenames in os.walk(ROOT):
        parent = "manga-lib" if dirpath == ROOT else item_id(dirpath)
        for d in dirnames:
            p = os.path.join(dirpath, d)
            ITEMS[item_id(p)] = {"path": p, "parent": parent}
        for f in filenames:
            if f.lower().endswith(COMIC_EXT):
                p = os.path.join(dirpath, f)
                ITEMS[item_id(p)] = {"path": p, "parent": parent}


def to_item(iid):
    path = ITEMS[iid]["path"]
    is_dir = os.path.isdir(path)
    name = os.path.basename(path)
    if not is_dir:
        name = os.path.splitext(name)[0]
    ud = USER_DATA.get(iid, {"Played": False, "PlaybackPositionTicks": 0})
    item = {
        "Name": name,
        "Id": iid,
        "Type": "Folder" if is_dir else "Book",
        "ImageTags": {},
        "UserData": ud,
    }
    if is_dir:
        item["ChildCount"] = sum(1 for v in ITEMS.values() if v["parent"] == iid)
    else:
        item["Path"] = path
    return item


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        pass

    def authed(self):
        return f'Token="{TOKEN}"' in self.headers.get("Authorization", "")

    def send_json(self, obj, status=200):
        body = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def send_empty(self, status):
        self.send_response(status)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def body(self):
        n = int(self.headers.get("Content-Length", 0) or 0)
        return json.loads(self.rfile.read(n) or b"{}") if n else {}

    def do_GET(self):
        url = urlparse(self.path)
        q = {k: v[0] for k, v in parse_qs(url.query).items()}
        p = url.path
        if p == "/System/Info/Public":
            return self.send_json(SERVER)
        if p == "/QuickConnect/Enabled":
            return self.send_json(True)
        if p == "/QuickConnect/Connect":
            with LOCK:
                QC["polls"] += 1
                ok = QC["polls"] >= 2
            return self.send_json({"Secret": q.get("Secret", ""), "Code": "424242", "Authenticated": ok})
        if p == "/opds":
            return self.send_empty(500)  # like the broken plugin on 12.2
        if not self.authed():
            return self.send_empty(401)
        if p == "/UserViews":
            return self.send_json({"Items": [
                {"Name": "Anime", "Id": "anime-lib", "Type": "CollectionFolder", "CollectionType": "tvshows"},
                {"Name": "Manga", "Id": "manga-lib", "Type": "CollectionFolder", "CollectionType": "books"},
            ], "TotalRecordCount": 2})
        if p == "/Items":
            parent = q.get("ParentId")
            kids = [i for i, v in ITEMS.items() if v["parent"] == parent]
            kids.sort(key=lambda i: os.path.basename(ITEMS[i]["path"]).lower())
            items = [to_item(i) for i in kids]
            return self.send_json({"Items": items, "TotalRecordCount": len(items)})
        m = re.fullmatch(r"/Items/([^/]+)/Images/Primary", p)
        if m:
            return self.send_empty(404)  # no covers: the app falls back to page 1
        m = re.fullmatch(r"/Items/([^/]+)/Download", p)
        if m and m.group(1) in ITEMS:
            return self.download(ITEMS[m.group(1)]["path"])
        return self.send_empty(404)

    def download(self, path):
        size = os.path.getsize(path)
        rng = self.headers.get("Range")
        start, end = 0, size - 1
        if rng:
            m = re.fullmatch(r"bytes=(\d*)-(\d*)", rng.strip())
            if not m:
                return self.send_empty(416)
            a, b = m.groups()
            if a == "":  # suffix range: last N bytes
                start = max(0, size - int(b))
            else:
                start = int(a)
                if b:
                    end = min(int(b), size - 1)
        with open(path, "rb") as f:
            f.seek(start)
            data = f.read(end - start + 1)
        self.send_response(206 if rng else 200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Accept-Ranges", "bytes")
        if rng:
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
        print(f"download {os.path.basename(path)[:40]:40} {rng or 'whole file'} -> {len(data)} bytes", flush=True)

    def do_HEAD(self):
        self.send_empty(405)  # real Jellyfin refuses HEAD on downloads too

    def do_POST(self):
        p = urlparse(self.path).path
        body = self.body()
        auth = {"User": USER, "AccessToken": TOKEN, "ServerId": SERVER["Id"]}
        if p == "/QuickConnect/Initiate":
            QC["polls"] = 0
            return self.send_json({"Secret": "s3cr3t", "Code": "424242", "Authenticated": False})
        if p == "/Users/AuthenticateWithQuickConnect":
            return self.send_json(auth)
        if p == "/Users/AuthenticateByName":
            ok = body.get("Username") == "tester" and body.get("Pw") == "secret"
            return self.send_json(auth) if ok else self.send_empty(401)
        if not self.authed():
            return self.send_empty(401)
        if p == "/Sessions/Logout":
            print("logout", flush=True)
            return self.send_empty(204)
        m = re.fullmatch(r"/UserItems/([^/]+)/UserData", p)
        if m:
            iid = m.group(1)
            with LOCK:
                ud = USER_DATA.setdefault(iid, {"Played": False, "PlaybackPositionTicks": 0})
                ud.update({k: v for k, v in body.items() if v is not None})
            name = os.path.basename(ITEMS.get(iid, {}).get("path", iid))[:40]
            page = body.get("PlaybackPositionTicks", 0) // 10000
            print(f"progress {name:40} page={page} played={body.get('Played')}", flush=True)
            return self.send_json(ud)
        return self.send_empty(404)


def main():
    global ROOT
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("library", help="folder to serve as the Manga library")
    ap.add_argument("--port", type=int, default=8097)
    args = ap.parse_args()
    ROOT = os.path.abspath(args.library)
    index_tree()
    print(f"mock Jellyfin on http://localhost:{args.port} serving {ROOT} ({len(ITEMS) - 1} items)", flush=True)
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
