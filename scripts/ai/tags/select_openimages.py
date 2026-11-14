"""Picks the Open Images thumbnails to download for the tag head (development only).

Reads <ai-dev>/openimages/vocabulary.json (`ai_embed vocabulary`), the image lists and the
human-verified image-level labels of the test and validation splits (CC BY 4.0 annotations), and
writes selection.json ({split: {image_id: thumbnail_url}}): for every word, up to --cap-test
(--cap-validation) images where one of its classes is verified present, rarest words first so
their few examples are all in, plus --random-* images drawn at random. Only images listed under
CC BY 2.0 with a thumbnail are considered.

python -I scripts/ai/tags/select_openimages.py [--dir <ai-dev>]
"""
import argparse
import collections
import csv
import json
import os
import random

SPLITS = [
    ("test", "test-images-with-rotation.csv", "oidv7-test-annotations-human-imagelabels.csv"),
    ("validation", "validation-images-with-rotation.csv", "oidv7-val-annotations-human-imagelabels.csv"),
]


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--dir", default=os.path.join(os.environ.get("LOCALAPPDATA", ""), "Sortypix", "ai-dev"))
    p.add_argument("--cap-test", type=int, default=400)
    p.add_argument("--cap-validation", type=int, default=150)
    p.add_argument("--random-test", type=int, default=5000)
    p.add_argument("--random-validation", type=int, default=2000)
    a = p.parse_args()
    root = os.path.join(a.dir, "openimages")
    rng = random.Random(1)
    vocabulary = json.load(open(os.path.join(root, "vocabulary.json"), encoding="utf8"))
    words_of = collections.defaultdict(list)
    for i, w in enumerate(vocabulary):
        for c in w["classes"]:
            words_of[c].append(i)
    cap = {"test": a.cap_test, "validation": a.cap_validation}
    extra = {"test": a.random_test, "validation": a.random_validation}
    plan = {}
    for split, images, labels in SPLITS:
        urls = {}
        with open(os.path.join(root, images), encoding="utf8") as f:
            for r in csv.DictReader(f):
                if r["License"].rstrip("/").endswith("licenses/by/2.0") and r["Thumbnail300KURL"]:
                    urls[r["ImageID"]] = r["Thumbnail300KURL"]
        positives = collections.defaultdict(set)
        with open(os.path.join(root, labels), encoding="utf8") as f:
            for r in csv.DictReader(f):
                if r["ImageID"] in urls and float(r["Confidence"]) > 0.5:
                    for w in words_of.get(r["LabelName"], []):
                        positives[w].add(r["ImageID"])
        chosen = set()
        for w in sorted(positives, key=lambda k: (len(positives[k]), k)):
            have = len(positives[w] & chosen)
            rest = sorted(positives[w] - chosen)
            rng.shuffle(rest)
            chosen.update(rest[:max(0, cap[split] - have)])
        others = sorted(set(urls) - chosen)
        rng.shuffle(others)
        chosen.update(others[:extra[split]])
        plan[split] = {k: urls[k] for k in sorted(chosen)}
        print(split, len(chosen))
    with open(os.path.join(root, "selection.json"), "w") as f:
        json.dump(plan, f)


if __name__ == "__main__":
    main()
