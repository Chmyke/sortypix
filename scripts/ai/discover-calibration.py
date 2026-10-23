#!/usr/bin/env python3
"""Build tests/ai/calibration.json: real "photo days" from Wikimedia Commons (maintainer tool).

Usage:
    python -I scripts/ai/discover-calibration.py scan DATE [DATE ...]
    python -I scripts/ai/discover-calibration.py pick [--dir ROOT] SET_ID=CANDIDATE [...]

A day set is the photos one photographer uploaded from one outing: same uploader, same camera
model (EXIF), capture times within one day, 60 to 120 photos, licensed CC0, CC BY or CC BY-SA.
`scan` walks the "<Country> photographs taken on DATE" categories, groups their files by uploader
and prints the groups that qualify (with burst counts and sample titles) as candidates
`DATE|CATEGORY|USER`. `pick` downloads the 1600 px rendition of the chosen candidates into
ROOT/calibration/SET_ID/NNN.jpg (shooting order) and writes the list with source page, author,
license, capture time and SHA256. Requests are sequential and spaced out; nothing else is fetched.
"""
import argparse
import hashlib
import html
import importlib.util
import json
import re
import sys
import time
from collections import defaultdict
from datetime import datetime
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import Request, urlopen

REPO = Path(__file__).resolve().parent.parent.parent
OUT = REPO / "tests" / "ai" / "calibration.json"
API = "https://commons.wikimedia.org/w/api.php"
USER_AGENT = "SortypixCalibrationBuilder/1.0 (https://github.com/Chmyke/sortypix/issues)"
DELAY = 1.5
MIN_SET, MAX_SET = 60, 120
# Shots at most this many seconds apart count as one burst.
BURST_GAP = 2
LICENSE = re.compile(r"(?i)(CC0( 1\.0)?|CC BY(-SA)? [0-9.]+( [a-z]+)?)")


def api(params: dict) -> dict:
    time.sleep(DELAY)
    # POST: fifty long titles do not fit in a URL.
    req = Request(API, data=urlencode(params).encode(), headers={"User-Agent": USER_AGENT})
    for attempt in range(5):
        try:
            with urlopen(req, timeout=60) as resp:
                return json.load(resp)
        except Exception as exc:  # noqa: BLE001 - retry on any transient network error
            print(f"  retry {attempt + 1}: {exc}")
            time.sleep(10 * (attempt + 1))
    raise SystemExit("Commons API unreachable")


def text(value: str) -> str:
    return html.unescape(re.sub(r"<[^>]+>", "", value or "")).strip()


def members(category: str) -> list:
    """Files of a category with their uploader, all pages."""
    out, cont = [], {}
    while True:
        data = api({
            "action": "query", "format": "json", "generator": "categorymembers", "gcmtitle": category,
            "gcmtype": "file", "gcmlimit": 500, "prop": "imageinfo", "iiprop": "user|mime|size", **cont,
        })
        for page in data.get("query", {}).get("pages", {}).values():
            info = (page.get("imageinfo") or [None])[0]
            if info:
                out.append({"title": page["title"], "user": info.get("user"), "mime": info.get("mime"),
                            "width": info.get("width", 0), "height": info.get("height", 0)})
        if "continue" not in data:
            return out
        cont = data["continue"]


def details(titles: list) -> list:
    """License, author, camera, capture time and 1600 px URL, 50 titles per request."""
    out = []
    for i in range(0, len(titles), 50):
        data = api({
            "action": "query", "format": "json", "titles": "|".join(titles[i:i + 50]),
            "prop": "imageinfo", "iiprop": "url|size|mime|extmetadata|commonmetadata", "iiurlwidth": 1600,
            "iiextmetadatafilter": "LicenseShortName|Artist",
        })
        for page in data.get("query", {}).get("pages", {}).values():
            info = (page.get("imageinfo") or [None])[0]
            if not info:
                continue
            meta = info.get("extmetadata", {})
            common = {c.get("name"): c.get("value") for c in info.get("commonmetadata") or []}
            taken = common.get("DateTimeOriginal") or ""
            try:
                when = datetime.strptime(str(taken)[:19], "%Y:%m:%d %H:%M:%S")
            except ValueError:
                when = None
            out.append({
                "title": page["title"].removeprefix("File:"),
                "license": text(meta.get("LicenseShortName", {}).get("value")),
                "author": text(meta.get("Artist", {}).get("value")) or "unknown",
                "model": str(common.get("Model") or "").strip(),
                "when": when,
                "source_page": info.get("descriptionurl"),
                "url": info.get("thumburl"),
                "mime": info.get("mime"),
                "width": info.get("width", 0),
            })
    return out


def qualify(photos: list):
    """Keeps a coherent day: JPEG >= 1600 px, accepted license, one camera, capture time known."""
    ok = [p for p in photos if p["mime"] == "image/jpeg" and p["width"] >= 1600 and p["when"]
          and p["url"] and LICENSE.fullmatch(p["license"])]
    if not ok:
        return None
    models = defaultdict(list)
    for p in ok:
        models[p["model"]].append(p)
    model, best = max(models.items(), key=lambda kv: len(kv[1]))
    if not model:
        return None
    best.sort(key=lambda p: (p["when"], p["title"]))
    day = best[0]["when"].date()
    best = [p for p in best if p["when"].date() == day][:MAX_SET]
    if len(best) < MIN_SET:
        return None
    return best


def bursts(photos: list) -> int:
    count, run = 0, 1
    for a, b in zip(photos, photos[1:]):
        if (b["when"] - a["when"]).total_seconds() <= BURST_GAP:
            run += 1
        else:
            count += run >= 2
            run = 1
    return count + (run >= 2)


def scan(dates: list) -> None:
    for date in dates:
        data = api({"action": "query", "format": "json", "generator": "categorymembers",
                    "gcmtitle": f"Category:Photographs taken on {date}", "gcmtype": "subcat", "gcmlimit": 500,
                    "prop": "categoryinfo"})
        for page in data.get("query", {}).get("pages", {}).values():
            if not MIN_SET <= page.get("categoryinfo", {}).get("files", 0) <= 2000:
                continue
            groups = defaultdict(list)
            for m in members(page["title"]):
                if m["mime"] == "image/jpeg" and m["width"] >= 1600:
                    groups[m["user"]].append(m["title"])
            for user, titles in groups.items():
                if len(titles) < MIN_SET:
                    continue
                day = qualify(details(titles))
                if not day:
                    continue
                print(f"CANDIDATE {date}|{page['title']}|{user}: {len(day)} photos, {day[0]['model']}, "
                      f"{bursts(day)} bursts, {day[0]['license']}, {day[0]['when']:%H:%M}-{day[-1]['when']:%H:%M}")
                for p in day[:: max(1, len(day) // 6)][:6]:
                    print(f"    {p['title']}")
                sys.stdout.flush()


def download(url: str, dest: Path) -> str:
    for attempt in range(5):
        time.sleep(DELAY)
        try:
            with urlopen(Request(url, headers={"User-Agent": USER_AGENT}), timeout=120) as resp:
                data = resp.read()
            part = dest.with_name(dest.name + ".part")
            part.write_bytes(data)
            part.replace(dest)
            return hashlib.sha256(data).hexdigest()
        except Exception as exc:  # noqa: BLE001 - retry on any transient network error
            print(f"  retry {attempt + 1} for {url}: {exc}")
            time.sleep(10 * (attempt + 1))
    raise SystemExit(f"cannot download {url}")


def pick(root: Path, choices: list) -> None:
    sets = []
    for choice in choices:
        set_id, candidate = choice.split("=", 1)
        date, category, user = candidate.split("|")
        titles = [m["title"] for m in members(category)
                  if m["user"] == user and m["mime"] == "image/jpeg" and m["width"] >= 1600]
        day = qualify(details(titles))
        if not day:
            raise SystemExit(f"{candidate} no longer qualifies")
        folder = root / "calibration" / set_id
        folder.mkdir(parents=True, exist_ok=True)
        images = []
        for n, p in enumerate(day, 1):
            name = f"{n:03d}.jpg"
            dest = folder / name
            print(f"{set_id}/{name}: {p['title']}")
            images.append({
                "file": name, "taken": p["when"].strftime("%Y-%m-%dT%H:%M:%S"),
                "sha256": download(p["url"], dest), "url": p["url"], "source_page": p["source_page"],
                "license": p["license"], "author": p["author"], "title": p["title"],
            })
        sets.append({"id": set_id, "date": date, "category": category, "uploader": user,
                     "camera": day[0]["model"], "images": images})
    doc = {
        "schema": 1,
        "comment": "Photo days (one photographer, one camera, one day) from Wikimedia Commons at 1600 px, "
                   "CC0 / CC BY / CC BY-SA, files in <ai-dev>/calibration/<id>/, numbered in shooting order. "
                   "Built by scripts/ai/discover-calibration.py.",
        "sets": sets,
    }
    OUT.write_text(json.dumps(doc, indent=2, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {OUT} ({sum(len(s['images']) for s in sets)} images)")


def main() -> int:
    sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dir", type=Path)
    parser.add_argument("mode", choices=["scan", "pick"])
    parser.add_argument("items", nargs="+")
    args = parser.parse_args()
    if args.mode == "scan":
        scan(args.items)
        return 0
    sys.dont_write_bytecode = True  # importing fetch-models.py must not leave a __pycache__ in the repo
    spec = importlib.util.spec_from_file_location("fetch_models", Path(__file__).with_name("fetch-models.py"))
    fetch = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fetch)
    pick(args.dir or fetch.default_root(), args.items)
    return 0


if __name__ == "__main__":
    sys.exit(main())
