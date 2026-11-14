"""Builds a local page where a person judges the tags of a sample of test images in minutes.

Reads target/reports/ai-tags-shown.txt (written by the ai_tags probe) and the images of the test
set, picks --count images at random (fixed seed; real photos and generated images in proportion),
and writes target/ai-tags/review.html: each image (512 px, embedded) with its tags as buttons; a
click marks a tag wrong, a second click marks it unsure; "Copy result" puts a JSON on the
clipboard ({"<key>": {"wrong": [...], "unsure": [...]}, "_comment": "..."}) to paste back.
Nothing leaves the computer: the page has no network access and no external resource.

python -I scripts/ai/tags/make_review_page.py [--dir <ai-dev>] [--count 30]
"""
import argparse
import base64
import html
import io
import json
import os
import random

from PIL import Image, ImageOps

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

PAGE = """<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Sortypix tags review</title>
<style>
:root { --bg:#f6f6f4; --card:#fff; --text:#1d1d1b; --dim:#6b6b66; --line:#d9d9d4; --bad:#c0392b; --unsure:#b7791f; }
@media (prefers-color-scheme: dark) { :root { --bg:#151514; --card:#1f1f1d; --text:#ecece8; --dim:#9a9a94; --line:#3a3a36; --bad:#e8705f; --unsure:#e0b04a; } }
body { margin:0; background:var(--bg); color:var(--text); font:15px/1.45 system-ui, sans-serif; }
main { max-width:900px; margin:0 auto; padding:16px; }
h1 { font-size:20px; margin:8px 0 4px; } p { color:var(--dim); margin:4px 0 16px; }
.item { background:var(--card); border:1px solid var(--line); border-radius:10px; padding:12px; margin:0 0 14px; }
.item img { max-width:100%; max-height:420px; display:block; margin:0 auto 10px; border-radius:6px; }
.key { color:var(--dim); font-size:12px; margin-bottom:6px; }
button.tag { border:1px solid var(--line); background:transparent; color:var(--text); border-radius:14px; padding:4px 10px; margin:3px; cursor:pointer; font-size:14px; }
button.tag.wrong { border-color:var(--bad); color:var(--bad); text-decoration:line-through; }
button.tag.unsure { border-color:var(--unsure); color:var(--unsure); font-style:italic; }
.none { color:var(--dim); font-style:italic; }
textarea { width:100%; box-sizing:border-box; min-height:60px; background:var(--card); color:var(--text); border:1px solid var(--line); border-radius:6px; }
#bar { position:sticky; bottom:0; background:var(--bg); padding:10px 0; border-top:1px solid var(--line); }
#copy { font-size:16px; padding:8px 16px; border-radius:8px; border:1px solid var(--line); background:var(--card); color:var(--text); cursor:pointer; }
</style></head><body><main>
<h1>Sortypix tags review</h1>
<p>__COUNT__ images and the tags Sortypix AI gives them. Click a tag that is wrong (crossed out in red); click again if you are unsure (orange); a third click restores it. Then press <b>Copy result</b> and paste it in the chat. Optional: note a clearly missing tag under an image.</p>
__ITEMS__
<div id="bar"><button id="copy">Copy result</button> <span id="done"></span></div>
</main><script>
document.querySelectorAll('button.tag').forEach(b => b.addEventListener('click', () => {
  if (b.classList.contains('wrong')) { b.classList.remove('wrong'); b.classList.add('unsure'); }
  else if (b.classList.contains('unsure')) { b.classList.remove('unsure'); }
  else { b.classList.add('wrong'); }
}));
document.getElementById('copy').addEventListener('click', async () => {
  const out = {_comment: 'tags review, __STAMP__'};
  document.querySelectorAll('.item').forEach(it => {
    const k = it.dataset.key;
    out[k] = {
      wrong: [...it.querySelectorAll('button.wrong')].map(b => b.textContent),
      unsure: [...it.querySelectorAll('button.unsure')].map(b => b.textContent),
      missing: it.querySelector('textarea').value.trim(),
    };
  });
  const text = JSON.stringify(out);
  try { await navigator.clipboard.writeText(text); document.getElementById('done').textContent = 'Copied.'; }
  catch (e) { prompt('Copy this:', text); }
});
</script></body></html>
"""


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--dir", default=os.path.join(os.environ.get("LOCALAPPDATA", ""), "Sortypix", "ai-dev"))
    p.add_argument("--count", type=int, default=30)
    a = p.parse_args()
    shown = {}
    with open(os.path.join(REPO, "target", "reports", "ai-tags-shown.txt"), encoding="utf8") as f:
        for line in f:
            key, _, tags = line.rstrip("\n").partition(": ")
            shown[key] = [t for t in tags.split(", ") if t]
    test = json.load(open(os.path.join(REPO, "tests", "ai", "tags-test.json"), encoding="utf8"))["images"]
    keys = sorted(k for k in test if k in shown)
    rng = random.Random(11)
    photos = [k for k in keys if not k.startswith("generated/")]
    generated = [k for k in keys if k.startswith("generated/")]
    n_gen = round(a.count * len(generated) / max(1, len(keys)))
    sample = rng.sample(photos, a.count - n_gen) + rng.sample(generated, n_gen)
    rng.shuffle(sample)
    items = []
    for k in sample:
        path = os.path.join(a.dir, "tags-test", k) if k.startswith("generated/") else os.path.join(a.dir, k)
        im = ImageOps.exif_transpose(Image.open(path)).convert("RGB")
        im.thumbnail((640, 640))
        buf = io.BytesIO()
        im.save(buf, "JPEG", quality=85)
        data = base64.b64encode(buf.getvalue()).decode()
        tags = "".join(f'<button class="tag">{html.escape(t)}</button>' for t in shown[k]) or '<span class="none">no tag</span>'
        items.append(f'<div class="item" data-key="{html.escape(k)}"><div class="key">{html.escape(k)}</div>'
                     f'<img alt="" src="data:image/jpeg;base64,{data}"><div>{tags}</div>'
                     f'<textarea placeholder="Missing tags (optional)"></textarea></div>')
    page = PAGE.replace("__ITEMS__", "\n".join(items)).replace("__COUNT__", str(len(sample)))
    page = page.replace("__STAMP__", f"{len(sample)} images")
    out = os.path.join(REPO, "target", "ai-tags", "review.html")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    open(out, "w", encoding="utf8").write(page)
    print(out)


if __name__ == "__main__":
    main()
