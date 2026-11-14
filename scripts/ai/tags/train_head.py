"""Trains the tag head on Open Images embeddings and writes crates/sortypix-ai/data/tags-head-l14.bin.

Development only (PyTorch, never shipped). Inputs, all under <ai-dev>/openimages/:
- test.bin and validation.bin: CLIP ViT-L/14 int8 embeddings of the thumbnails, made by the
  application's own engine (`ai_embed embed`); keys are "<image id>.jpg";
- vocabulary.json (`ai_embed vocabulary`): the words in index order and their Open Images classes;
- the human-verified image-level label files of the same splits.
And the shipped text embeddings (crates/sortypix-ai/data/tags-l14-int8.bin, `ai_tags --write`),
whose header (model label, vocabulary hash) the head file copies, and whose embeddings start
and anchor the weights.

Method (docs/design/Étiquettes apprises (2026-11-13).md):
- one logistic output per word (linear 768 -> words), started from the zero-shot text weights;
- the weight of unverified pairs (alpha) is chosen by the mean average precision on validation
  with unverified pairs counted absent (strict: sees a word that fires everywhere);
- a word is present on an image when a human verified one of its classes there, absent when a
  human verified one absent and none present; unverified pairs count as absent with a small
  weight (most are absent, some are missed labels);
- trained on the Open Images *test* split, thresholds chosen on its *validation* split (both
  human-verified; the train split is not used);
- per word, the smallest probability whose precision on verified validation pairs reaches
  --precision with at least --support shown pairs; a word that cannot is left to zero-shot.

python -I scripts/ai/tags/train_head.py [--dir <ai-dev>] [--precision 0.85]
"""
import argparse
import csv
import json
import os
import struct
import sys

import numpy as np
import torch

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
TEXT = os.path.join(REPO, "crates", "sortypix-ai", "data", "tags-l14-int8.bin")
HEAD = os.path.join(REPO, "crates", "sortypix-ai", "data", "tags-head-l14.bin")
DIM = 768
TEXT_HEADER = 8 + 4 + 32 + 32 + 4 + 4
MAGIC = b"SPXHEAD\0"
VERSION = 1
LOGIT_SCALE = 100.0


def read_dump(path):
    b = open(path, "rb").read()
    n = struct.unpack("<Q", b[:8])[0]
    keys = b[8:8 + n].decode().splitlines()
    e = np.frombuffer(b[8 + n:], dtype="<f4").reshape(-1, DIM)
    assert len(keys) == len(e), path
    return [k.rsplit(".", 1)[0] for k in keys], e.copy()


def read_text():
    b = open(TEXT, "rb").read()
    count, dim = struct.unpack("<II", b[TEXT_HEADER - 8:TEXT_HEADER])
    assert dim == DIM
    return b[12:12 + 64], np.frombuffer(b[TEXT_HEADER:], dtype="<f4").reshape(count, dim).copy()


def labels(path, ids, vocabulary):
    """+1 verified present, -1 verified absent, 0 unverified; shape images x words."""
    row = {k: i for i, k in enumerate(ids)}
    by_class = {}
    for w, word in enumerate(vocabulary):
        for c in word["classes"]:
            by_class.setdefault(c, []).append(w)
    y = np.zeros((len(ids), len(vocabulary)), np.int8)
    with open(path, encoding="utf8") as f:
        for r in csv.DictReader(f):
            i = row.get(r["ImageID"])
            words = by_class.get(r["LabelName"])
            if i is None or not words:
                continue
            present = float(r["Confidence"]) > 0.5
            for w in words:
                if present:
                    y[i, w] = 1
                elif y[i, w] == 0:
                    y[i, w] = -1
    return y


def train(x, y, text, learned, alpha, anchor, epochs, device, batch=512, lr=1e-2):
    X = torch.tensor(x, device=device)
    Y = torch.tensor(y, device=device)
    pos = (Y == 1).float()
    weight = torch.where(Y == 0, torch.tensor(alpha, device=device), torch.tensor(1.0, device=device))
    weight = weight * torch.tensor(learned, device=device).float()[None, :]
    w0 = torch.tensor(text * LOGIT_SCALE, device=device)
    W = torch.nn.Parameter(w0.clone())
    b = torch.nn.Parameter(torch.full((len(text),), -LOGIT_SCALE * 0.22, device=device))
    opt = torch.optim.Adam([W, b], lr=lr)
    g = torch.Generator(device=device).manual_seed(0)
    n = len(X)
    for _ in range(epochs):
        order = torch.randperm(n, device=device, generator=g)
        for s in range(0, n, batch):
            idx = order[s:s + batch]
            logits = X[idx] @ W.T + b
            loss = torch.nn.functional.binary_cross_entropy_with_logits(logits, pos[idx], weight=weight[idx], reduction="sum")
            loss = loss / weight[idx].sum().clamp(min=1) + anchor * ((W - w0) ** 2).sum() / len(text)
            opt.zero_grad()
            loss.backward()
            opt.step()
    return W.detach().cpu().numpy(), b.detach().cpu().numpy()


def probabilities(x, W, b):
    return 1 / (1 + np.exp(-(x @ W.T + b)))


def average_precision(p, y):
    """Over verified pairs only."""
    m = y != 0
    if (y[m] == 1).sum() == 0:
        return float("nan")
    order = np.argsort(-p[m])
    t = (y[m][order] == 1).astype(np.float64)
    hits = np.cumsum(t)
    return float((hits / np.arange(1, len(t) + 1))[t == 1].mean())


def strict_average_precision(p, y):
    """Unverified pairs counted absent: pessimistic, but sees a word that fires everywhere."""
    if (y == 1).sum() == 0:
        return float("nan")
    order = np.argsort(-p)
    t = (y[order] == 1).astype(np.float64)
    hits = np.cumsum(t)
    return float((hits / np.arange(1, len(t) + 1))[t == 1].mean())


def threshold(p, y, precision, support):
    """Smallest probability reaching `precision` over verified pairs, with `support` shown."""
    m = y != 0
    pv, yv = p[m], y[m] == 1
    order = np.argsort(-pv)
    pv, yv = pv[order], yv[order]
    right = np.cumsum(yv)
    shown = np.arange(1, len(pv) + 1)
    ok = (right / shown >= precision) & (shown >= support)
    if not ok.any():
        return None
    k = np.nonzero(ok)[0].max()
    return float(pv[k]), int(right[k]), int(shown[k])


def main():
    a = argparse.ArgumentParser()
    a.add_argument("--dir", default=os.path.join(os.environ.get("LOCALAPPDATA", ""), "Sortypix", "ai-dev"))
    a.add_argument("--precision", type=float, default=0.85)
    a.add_argument("--support", type=int, default=8)
    a.add_argument("--min-positives", type=int, default=30)
    a.add_argument("--alphas", default="0.1,0.3,1.0")
    a.add_argument("--anchors", default="0.0")
    a.add_argument("--epochs", type=int, default=60)
    a.add_argument("--lr", type=float, default=3e-2)
    a.add_argument("--report", default=os.path.join(REPO, "target", "reports", "ai-tags-head.txt"))
    args = a.parse_args()
    oi = os.path.join(args.dir, "openimages")
    vocabulary = json.load(open(os.path.join(oi, "vocabulary.json"), encoding="utf8"))
    header, text = read_text()
    assert len(text) == len(vocabulary), "regenerate the text embeddings (ai_tags --write)"
    ids_t, x_t = read_dump(os.path.join(oi, "test.bin"))
    ids_v, x_v = read_dump(os.path.join(oi, "validation.bin"))
    y_t = labels(os.path.join(oi, "oidv7-test-annotations-human-imagelabels.csv"), ids_t, vocabulary)
    y_v = labels(os.path.join(oi, "oidv7-val-annotations-human-imagelabels.csv"), ids_v, vocabulary)
    positives = (y_t == 1).sum(0)
    learned = np.array([bool(w["classes"]) and positives[i] >= args.min_positives for i, w in enumerate(vocabulary)])
    device = "cuda" if torch.cuda.is_available() else "cpu"
    report = [f"train_images={len(ids_t)}", f"validation_images={len(ids_v)}", f"words={len(vocabulary)}",
              f"trainable_words={int(learned.sum())}", f"precision_target={args.precision}"]

    # Zero-shot baseline on the same verified validation pairs (softmax over the vocabulary).
    zs = torch.softmax(torch.tensor(x_v @ text.T * LOGIT_SCALE), dim=1).numpy()
    best = None
    for alpha in map(float, args.alphas.split(",")):
        for anchor in map(float, args.anchors.split(",")):
            W, b = train(x_t, y_t, text, learned, alpha, anchor, args.epochs, device, 512, args.lr)
            p = probabilities(x_v, W, b)
            cols = np.nonzero(learned)[0]
            verified = float(np.nanmean([average_precision(p[:, w], y_v[:, w]) for w in cols]))
            m = float(np.nanmean([strict_average_precision(p[:, w], y_v[:, w]) for w in cols]))
            report.append(f"grid alpha={alpha} anchor={anchor} mean_ap={verified:.4f} strict_mean_ap={m:.4f}")
            print(report[-1], flush=True)
            if best is None or m > best[0]:
                best = (m, alpha, anchor, W, b, p)
    m, alpha, anchor, W, b, p = best
    report.append(f"chosen alpha={alpha} anchor={anchor} strict_mean_ap={m:.4f}")
    zs_aps = [average_precision(zs[:, w], y_v[:, w]) for w in np.nonzero(learned)[0]]
    report.append(f"zero_shot_mean_ap={float(np.nanmean(zs_aps)):.4f}")
    zs_strict = [strict_average_precision(zs[:, w], y_v[:, w]) for w in np.nonzero(learned)[0]]
    report.append(f"zero_shot_strict_mean_ap={float(np.nanmean(zs_strict)):.4f}")

    thresholds = np.full(len(vocabulary), np.nan, np.float32)
    rows = []
    for w in range(len(vocabulary)):
        if not learned[w]:
            rows.append(f"word {vocabulary[w]['name']} source=zero-shot positives={int(positives[w])}")
            continue
        t = threshold(p[:, w], y_v[:, w], args.precision, args.support)
        ap = average_precision(p[:, w], y_v[:, w])
        if t is None:
            learned[w] = False
            rows.append(f"word {vocabulary[w]['name']} source=zero-shot positives={int(positives[w])} ap={ap:.3f} (precision not reached)")
            continue
        thresholds[w] = t[0]
        rows.append(f"word {vocabulary[w]['name']} source=learned positives={int(positives[w])} ap={ap:.3f} threshold={t[0]:.3f} right={t[1]} shown={t[2]}")
    report.append(f"learned_words={int(learned.sum())}")
    report += rows

    out = bytearray(MAGIC + struct.pack("<I", VERSION) + header + struct.pack("<II", len(vocabulary), DIM))
    for w in range(len(vocabulary)):
        if learned[w]:
            out += struct.pack("<Iff", 1, float(b[w]), float(thresholds[w])) + W[w].astype("<f4").tobytes()
        else:
            out += struct.pack("<Iff", 0, 0.0, 1.0) + np.zeros(DIM, "<f4").tobytes()
    open(HEAD, "wb").write(out)
    report.append(f"written={HEAD} bytes={len(out)}")
    os.makedirs(os.path.dirname(args.report), exist_ok=True)
    open(args.report, "w", encoding="utf8").write("\n".join(report) + "\n")
    print("\n".join(report[:12]))


if __name__ == "__main__":
    sys.exit(main())
