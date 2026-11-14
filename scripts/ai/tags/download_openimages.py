"""Downloads the Open Images thumbnails picked by select_openimages.py (development only).

Reads <ai-dev>/openimages/selection.json ({split: {image_id: thumbnail_url}}) and writes
<ai-dev>/openimages/thumbs/<split>/<image_id>.jpg. Resumable: files already there are skipped.
Flickr's old cN.staticflickr.com hosts no longer resolve; URLs are rewritten to
live.staticflickr.com. Deleted photos (404, or a tiny placeholder) are listed in failed.txt.

Images are CC BY 2.0 (filtered by select_openimages.py); they never enter the repository and
are deleted once embedded (see docs/design/Étiquettes apprises (2026-11-13).md).

python -I scripts/ai/tags/download_openimages.py [--dir <ai-dev>] [--workers 16]
"""
import argparse
import concurrent.futures as cf
import json
import os
import re
import time
import urllib.error
import urllib.request

MIN_BYTES = 5000  # Flickr's "photo unavailable" placeholders are smaller.


def live_url(url):
    url = re.sub(r"^https?://c\d+\.staticflickr\.com/\d+/", "https://live.staticflickr.com/", url)
    url = re.sub(r"^https?://farm\d+\.staticflickr\.com/", "https://live.staticflickr.com/", url)
    return url.split("?")[0]


def fetch(url, path):
    if os.path.exists(path):
        return "kept"
    for attempt in range(3):
        try:
            req = urllib.request.Request(live_url(url), headers={"User-Agent": "dataset-fetch/1.0"})
            with urllib.request.urlopen(req, timeout=30) as r:
                data = r.read()
            if len(data) < MIN_BYTES:
                return "placeholder"
            tmp = path + ".part"
            with open(tmp, "wb") as f:
                f.write(data)
            os.replace(tmp, path)
            return "ok"
        except urllib.error.HTTPError as e:
            if e.code in (404, 410):
                return f"http {e.code}"
            time.sleep(2 * (attempt + 1))
        except Exception:
            time.sleep(2 * (attempt + 1))
    return "error"


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--dir", default=os.path.join(os.environ.get("LOCALAPPDATA", ""), "Sortypix", "ai-dev"))
    p.add_argument("--workers", type=int, default=16)
    a = p.parse_args()
    root = os.path.join(a.dir, "openimages")
    selection = json.load(open(os.path.join(root, "selection.json")))
    jobs = []
    for split, images in selection.items():
        os.makedirs(os.path.join(root, "thumbs", split), exist_ok=True)
        for image_id, url in images.items():
            jobs.append((split, image_id, url, os.path.join(root, "thumbs", split, image_id + ".jpg")))
    counts, failed = {}, []
    with cf.ThreadPoolExecutor(a.workers) as ex:
        futures = {ex.submit(fetch, url, path): (split, image_id) for split, image_id, url, path in jobs}
        for n, f in enumerate(cf.as_completed(futures), 1):
            r = f.result()
            counts[r] = counts.get(r, 0) + 1
            if r not in ("ok", "kept"):
                failed.append("%s/%s %s" % (*futures[f], r))
            if n % 2000 == 0:
                print(n, "of", len(jobs), counts, flush=True)
    with open(os.path.join(root, "failed.txt"), "w") as f:
        f.write("\n".join(sorted(failed)))
    print("done", counts)


if __name__ == "__main__":
    main()
