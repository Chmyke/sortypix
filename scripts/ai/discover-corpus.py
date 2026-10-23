#!/usr/bin/env python3
"""Build tests/ai/corpus.json: CC0 / public domain photos from Wikimedia Commons (maintainer tool).

Usage: python -I scripts/ai/discover-corpus.py [--dir ROOT]

Uses the Commons API (imageinfo with extmetadata), keeps only files whose LicenseShortName is CC0
or Public domain, downloads the 1600 px rendition into ROOT/corpus/, and records the source page,
license, author and SHA256 of every image. Requests are sequential and spaced out. Run once;
fetch-models.py then only replays the pinned list.
"""
import argparse
import hashlib
import html
import importlib.util
import json
import re
import sys
import time
from collections import Counter
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import Request, urlopen

REPO = Path(__file__).resolve().parent.parent.parent
OUT = REPO / "tests" / "ai" / "corpus.json"
API = "https://commons.wikimedia.org/w/api.php"
USER_AGENT = "SortypixCorpusBuilder/1.0 (https://github.com/Chmyke/sortypix/issues)"
DELAY = 1.5
MAX_PER_AUTHOR = 2
# Titles that match a keyword but are not a photo of the subject (museum objects, art, plants).
DENY = re.compile(r"(?i)\b(rhyton|shrimp|statue|statuette|sculpture|museum|MET|painting|drawing|illustration|"
                  r"figurine|toy|logo|stamp|coin|vase|relief|mosaic|engraving|poster|cartoon|mural|"
                  r"tattoo|map|diagram|icon|silhouette|lego|model)\b")
# Titles removed after looking at the contact sheets (wrong subject).
EXCLUDE = {
    "Cat graffiti art by ONE LOVE (Dudi Shoval) at Kivunim Youth Centre.jpg",
    "Palm Beach in Swakopmund.jpg",
    "Candolim Beach in Goa.jpg",
    "Flower in stones.jpg",
    "Dog park Jelitkowo.jpg",
    "Vintage motobikes at the car show, Lemon Street, Truro, Cornwall - July 2024 (1).jpg",
    "Retro black and white portrait photos.jpg",
    "Huntington Art Collections 05 - Portrait of a Woman.jpg",
    "Luis Alvarez Roure. Self Portrait. 2018.jpg",
    "Vintage motobikes at the car show, Lemon Street, Truro, Cornwall - July 2024 (2).jpg",
    "Portrait of a girl - design by András Nagy, c 1910.jpg",
    "Portrait of the Boy Eutyches, Fayum, lower Egypt. ca. 2nd century AD ~Explored - Flickr - Pete Tillman.jpg",
    "Retro black and white portrait photo in mirror.jpg",
    "Portrait of Gregor Baci.jpg",
    "Photo-OutdoorImage-Portrait-of-Tree.jpg",
    "Piazza del Rinascimento, Portrait, Urbino, Italy.jpg",
    "Antonello da Messina - Portrait of a Man (Il Condottiere) cleaned version.jpg",
    "Portrait of a Man - Giovanni Bellini - Louvre RF 1344.jpg",
    "Portrait of a girl, Luttichuys.jpg",
    "Tropical portrait of young happy woman with straw hat on a road with coconut palms and tropical trees. "
    "Bali island. (44644919695).jpg",
}

# Extra per-category deny list for off-subject words (a cable car is not a car).
DENY_BY_CATEGORY = {
    "car": r"(?i)\b(cable|tram|railway|rail|sleeping|carousel|carpark|antenna|radio|upholstering|cat|dog)\b",
    "dog": r"(?i)\b(walkers?|walking|mountains?|pier|hiking)\b",
    "portrait": r"(?i)\b(gallery|museum|museu|louvre|oil|canvas|group|family|christening|cadets)\b",
    "flower": r"(?i)\b(landscape|garden|park|field|farm|house|x-?ray)\b",
    "beach": r"(?i)\b(pool|fountain|market)\b",
    "apple": r"(?i)\b(tree|trees|orchard|blossoms?|iphone|ipad|mac|store|inc|pie|juice|cider)\b",
}

# category -> (count, keyword that must appear in the title, search terms)
PLAN = {
    "cat": (30, r"\b(cats?|kittens?)\b", ["cat", "kitten", "tabby cat", "domestic cat", "black cat", "white cat"]),
    "dog": (15, r"\b(dogs?|puppy|puppies)\b", ["dog", "puppy", "dog portrait", "dog running"]),
    "car": (15, r"\bcars?\b", ["car", "vintage car", "sports car", "parked car"]),
    "beach": (15, r"\bbeach(es)?\b", ["beach", "sandy beach", "beach sunset"]),
    "portrait": (15, r"\bportrait\b", ["portrait", "portrait woman", "portrait man", "portrait smile", "portrait girl", "portrait boy", "portrait photo", "portrait outdoor", "portrait young", "portrait beard", "portrait studio", "portrait child"]),
    "food": (15, r"\b(food|dish|meal|pizza|salad|soup|cake|bread|pasta|burger|sandwich|breakfast|dessert)\b",
             ["food", "dish", "meal", "pizza", "salad", "cake", "bread", "pasta", "breakfast"]),
    "flower": (14, r"\b(flowers?|blossoms?|tulips?|roses?|daisy|daisies|sunflowers?)\b",
               ["flower", "blossom", "tulip", "rose flower", "sunflower", "daisy"]),
    "apple": (1, r"\bapples?\b", ["apple fruit", "red apple", "apple"]),
}


def api(params: dict) -> dict:
    time.sleep(DELAY)
    req = Request(API + "?" + urlencode(params), headers={"User-Agent": USER_AGENT})
    with urlopen(req, timeout=60) as resp:
        return json.load(resp)


def search(term: str, license_category: str, offset: int = 0) -> list:
    data = api({
        "action": "query", "format": "json", "generator": "search",
        "gsrsearch": f'{term} incategory:"{license_category}" filetype:bitmap',
        "gsrnamespace": 6, "gsrlimit": 50, "gsroffset": offset,
        "prop": "imageinfo", "iiprop": "url|size|mime|extmetadata|commonmetadata", "iiurlwidth": 1600,
        "iiextmetadatafilter": "LicenseShortName|ImageDescription|Artist|ObjectName",
    })
    pages = data.get("query", {}).get("pages", {})
    return sorted(pages.values(), key=lambda p: p.get("index", 0))


def text(value: str) -> str:
    return html.unescape(re.sub(r"<[^>]+>", "", value or "")).strip()


def stem(title: str) -> str:
    return re.sub(r"[\d\W_]+", " ", title.lower()).strip()[:24]


def candidate(page: dict, keyword: re.Pattern, category: str):
    info = (page.get("imageinfo") or [None])[0]
    if not info or info.get("mime") != "image/jpeg" or info.get("width", 0) < 1600 or info.get("height", 0) < 900:
        return None
    meta = info.get("extmetadata", {})
    license_name = text(meta.get("LicenseShortName", {}).get("value"))
    if not re.fullmatch(r"(?i)(CC0( 1\.0)?|Public domain)", license_name):
        return None
    title = page["title"].removeprefix("File:")
    if DENY.search(title) or title in EXCLUDE:
        return None
    if category in DENY_BY_CATEGORY and re.search(DENY_BY_CATEGORY[category], title):
        return None
    # A real camera photo carries a camera model in its EXIF: this drops scans, prints and art.
    if not any(c.get("name") == "Model" for c in info.get("commonmetadata") or []):
        return None
    if not keyword.search(title.lower()):
        return None
    description = text(meta.get("ImageDescription", {}).get("value"))
    if DENY.search(description[:300]):
        return None
    author = text(meta.get("Artist", {}).get("value")) or "unknown"
    return {
        "title": title, "license": license_name, "author": author,
        "source_page": info["descriptionurl"], "url": info["thumburl"],
    }


def download(url: str, dest: Path) -> str:
    for attempt in range(5):
        time.sleep(DELAY)
        try:
            req = Request(url, headers={"User-Agent": USER_AGENT})
            with urlopen(req, timeout=120) as resp:
                data = resp.read()
            dest.write_bytes(data)
            return hashlib.sha256(data).hexdigest()
        except Exception as exc:  # noqa: BLE001 - retry on any transient network error
            print(f"  retry {attempt + 1} for {url}: {exc}")
            time.sleep(10 * (attempt + 1))
    raise SystemExit(f"cannot download {url}")


def main() -> int:
    sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser()
    parser.add_argument("--dir", type=Path)
    args = parser.parse_args()
    sys.dont_write_bytecode = True  # importing fetch-models.py must not leave a __pycache__ in the repo
    spec = importlib.util.spec_from_file_location("fetch_models", Path(__file__).with_name("fetch-models.py"))
    fetch = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fetch)
    corpus_dir = (args.dir or fetch.default_root()) / "corpus"
    corpus_dir.mkdir(parents=True, exist_ok=True)

    images = []
    seen_titles = set()
    for category, (count, pattern, terms) in PLAN.items():
        keyword = re.compile(pattern)
        picked, authors, stems = [], Counter(), set()
        for license_category in ("CC-zero", "PD-self"):
            for term in terms:
                for offset in (0, 50):
                    if len(picked) >= count:
                        break
                    for page in search(term, license_category, offset):
                        c = candidate(page, keyword, category)
                        if not c or c["title"] in seen_titles:
                            continue
                        key = stem(c["title"])
                        if authors[c["author"]] >= MAX_PER_AUTHOR or key in stems:
                            continue
                        picked.append(c)
                        seen_titles.add(c["title"])
                        authors[c["author"]] += 1
                        stems.add(key)
                        if len(picked) >= count:
                            break
        if len(picked) < count:
            print(f"WARNING: {category}: only {len(picked)} of {count}")
        for n, c in enumerate(picked, 1):
            name = f"{category}-{n:02d}.jpg"
            print(f"{name}: {c['title']} [{c['license']}]")
            c.update({"file": name, "category": category, "sha256": download(c["url"], corpus_dir / name)})
            images.append(c)

    order = ["file", "category", "sha256", "url", "source_page", "license", "author", "title"]
    doc = {
        "schema": 1,
        "comment": "CC0 or public domain photos from Wikimedia Commons at 1600 px; license read from "
                   "extmetadata on each file page. Built by scripts/ai/discover-corpus.py.",
        "images": [{k: c[k] for k in order} for c in images],
    }
    OUT.write_text(json.dumps(doc, indent=2, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n")
    print(f"{len(images)} images written to {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
