#!/usr/bin/env python3
"""Fetch and prepare the models of the optional Sortypix AI edition (maintainer tool).

Usage (always with -I, so nothing is imported from the current directory):

    python -I scripts/ai/fetch-models.py [--dir ROOT] [--check] [--pin]

ROOT is the ai-dev root (default: %LOCALAPPDATA%\\Sortypix\\ai-dev on Windows,
$XDG_CACHE_HOME/sortypix/ai-dev or ~/.cache/sortypix/ai-dev elsewhere; the environment variable
SORTYPIX_AI_DIR overrides the default). Layout:

    ROOT/models/     onnxruntime.dll | libonnxruntime.so.1.31.0, visual-{fp32,fp16,int8}.onnx,
                     textual-{fp32,int8}.onnx, tokenizer.json, aesthetic-l14.bin, SHA256SUMS.json
    ROOT/corpus/     the images listed in tests/ai/corpus.json
    ROOT/calibration/<set>/  the photo days listed in tests/ai/calibration.json
    ROOT/downloads/  the pinned source files, as downloaded (untrusted data)
    ROOT/tools/venv  throwaway virtual environment used to quantize and convert

--check   no network: verify presence and SHA256 of everything under ROOT; exit 1 otherwise.
--pin     first download only: fill sha256 and bytes of packaging/ai/models.json from the files
          actually downloaded. Without it, an unpinned manifest is refused.

Downloaded files are untrusted: the only pickle (the LAION head) is loaded with
torch.load(weights_only=True), and archives are read member by member, never extracted blindly.
"""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path
from urllib.request import Request, urlopen

REPO = Path(__file__).resolve().parent.parent.parent
MANIFEST = REPO / "packaging" / "ai" / "models.json"
CORPUS = REPO / "tests" / "ai" / "corpus.json"
CALIBRATION = REPO / "tests" / "ai" / "calibration.json"
TOKENS = REPO / "tests" / "ai" / "tokens.json"
USER_AGENT = "SortypixModelFetcher/1.0 (https://github.com/Chmyke/sortypix/issues)"

ORT_VERSION = "1.31.0"
ORT_WIN = ("onnxruntime-win-x64-1.31.0.zip", "onnxruntime-win-x64-1.31.0/lib/onnxruntime.dll", "onnxruntime.dll")
ORT_LINUX = (
    "onnxruntime-linux-x64-1.31.0.tgz",
    "onnxruntime-linux-x64-1.31.0/lib/libonnxruntime.so.1.31.0",
    "libonnxruntime.so.1.31.0",
)

# Versions installed in the throwaway venv (torch from the CPU wheel index).
TORCH_INDEX = "https://download.pytorch.org/whl/cpu"
VENV_PACKAGES = ["onnxruntime", "onnx", "transformers", "numpy"]

TOKEN_TEXTS = [
    "a photo of a cat",
    "cat",
    "Café crème",
    "a \U0001f34e on a table",
    "",
]
# 120 words, deterministic, so the sentence is truncated at 77 tokens.
LONG_WORDS = ("the quick brown fox jumps over the lazy dog while a small red car drives along "
              "the quiet beach at sunset and two children eat sweet apples under tall green "
              "trees").split()
LONG_TEXT = " ".join((LONG_WORDS * 6)[:120])
EOT = 49407


# ---------------------------------------------------------------- paths and hashing

def default_root() -> Path:
    env = os.environ.get("SORTYPIX_AI_DIR")
    if env:
        return Path(env)
    if os.name == "nt":
        base = os.environ.get("LOCALAPPDATA") or str(Path.home() / "AppData" / "Local")
        return Path(base) / "Sortypix" / "ai-dev"
    base = os.environ.get("XDG_CACHE_HOME") or str(Path.home() / ".cache")
    return Path(base) / "sortypix" / "ai-dev"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            digest.update(chunk)
    return digest.hexdigest()


def host_ort():
    return ORT_WIN if os.name == "nt" else ORT_LINUX


def venv_python(root: Path) -> Path:
    sub = "Scripts/python.exe" if os.name == "nt" else "bin/python"
    return root / "tools" / "venv" / sub


def expected_model_files():
    """Names expected in ROOT/models/ on this host."""
    return [
        host_ort()[2],
        "visual-fp32.onnx", "visual-fp16.onnx", "visual-int8.onnx",
        "textual-fp32.onnx", "textual-int8.onnx",
        "tokenizer.json", "aesthetic-l14.bin",
    ]


# ---------------------------------------------------------------- check (no network)

def check(root: Path) -> int:
    problems = []
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    pinned = {s["name"]: s["sha256"] for s in manifest["sources"]}
    for name, sha in pinned.items():
        if not sha:
            problems.append(f"manifest: {name} is not pinned")

    models = root / "models"
    record_path = models / "SHA256SUMS.json"
    record = {}
    if record_path.is_file():
        record = json.loads(record_path.read_text(encoding="utf-8"))
    else:
        problems.append(f"missing {record_path}")
    for name, sha in record.get("sources", {}).items():
        if pinned.get(name) != sha:
            problems.append(f"{name}: built from a source that differs from the manifest")
    for name in expected_model_files():
        path = models / name
        want = record.get("files", {}).get(name)
        if not path.is_file():
            problems.append(f"missing {path}")
        elif want is None:
            problems.append(f"{name}: no recorded SHA256")
        elif sha256_file(path) != want["sha256"]:
            problems.append(f"{name}: SHA256 mismatch")
    bin_path = models / "aesthetic-l14.bin"
    if bin_path.is_file() and bin_path.stat().st_size != 3076:
        problems.append("aesthetic-l14.bin: size is not 3076 bytes")

    corpus = json.loads(CORPUS.read_text(encoding="utf-8"))
    for item in corpus["images"]:
        path = root / "corpus" / item["file"]
        if not path.is_file():
            problems.append(f"missing {path}")
        elif sha256_file(path) != item["sha256"]:
            problems.append(f"corpus/{item['file']}: SHA256 mismatch")
    if CALIBRATION.is_file():
        calibration = json.loads(CALIBRATION.read_text(encoding="utf-8"))
        for day in calibration["sets"]:
            for item in day["images"]:
                path = root / "calibration" / day["id"] / item["file"]
                if not path.is_file():
                    problems.append(f"missing {path}")
                elif sha256_file(path) != item["sha256"]:
                    problems.append(f"calibration/{day['id']}/{item['file']}: SHA256 mismatch")
    if not TOKENS.is_file():
        problems.append(f"missing {TOKENS}")

    if problems:
        for p in problems:
            print(f"AI models: {p}")
        print("AI models: NOT OK (run scripts/ai/fetch-models.py to fetch)")
        return 1
    print("AI models: OK")
    return 0


# ---------------------------------------------------------------- download

def download(url: str, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_name(dest.name + ".part")
    req = Request(url, headers={"User-Agent": USER_AGENT})
    with urlopen(req, timeout=60) as resp, open(part, "wb") as out:
        shutil.copyfileobj(resp, out, 1 << 20)
    os.replace(part, dest)


def fetch_sources(root: Path, pin: bool) -> dict:
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    changed = False
    downloaded = {}
    for src in manifest["sources"]:
        path = root / "downloads" / src["name"]
        if not src["sha256"] and not pin:
            sys.exit(f"{src['name']} is not pinned in {MANIFEST}; run once with --pin")
        ok = path.is_file() and (
            (src["sha256"] and sha256_file(path) == src["sha256"]) or (pin and not src["sha256"])
        )
        if not ok:
            print(f"downloading {src['name']}")
            download(src["url"], path)
        digest = sha256_file(path)
        if not src["sha256"]:
            src["sha256"], src["bytes"] = digest, path.stat().st_size
            changed = True
        elif digest != src["sha256"]:
            path.unlink()
            sys.exit(f"{src['name']}: SHA256 {digest} does not match the manifest")
        elif path.stat().st_size != src["bytes"]:
            sys.exit(f"{src['name']}: size does not match the manifest")
        downloaded[src["name"]] = path
    if changed:
        MANIFEST.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8", newline="\n")
        print(f"pinned SHA256 values written to {MANIFEST}")
    return {s["name"]: s["sha256"] for s in manifest["sources"]} | {"_paths": downloaded}


def fetch_corpus(root: Path) -> None:
    corpus = json.loads(CORPUS.read_text(encoding="utf-8"))
    pinned = [(f"corpus/{item['file']}", item) for item in corpus["images"]]
    if CALIBRATION.is_file():
        calibration = json.loads(CALIBRATION.read_text(encoding="utf-8"))
        pinned += [(f"calibration/{day['id']}/{item['file']}", item)
                   for day in calibration["sets"] for item in day["images"]]
    for name, item in pinned:
        path = root / name
        if path.is_file() and sha256_file(path) == item["sha256"]:
            continue
        print(f"downloading {name}")
        download(item["url"], path)
        if sha256_file(path) != item["sha256"]:
            path.unlink()
            sys.exit(f"{name}: SHA256 does not match its list")


# ---------------------------------------------------------------- build

def extract_member(archive: Path, member: str, dest: Path) -> None:
    """Copy one named member of the archive to dest (the member path is never used on disk)."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as z, z.open(member) as src, open(dest, "wb") as out:
            shutil.copyfileobj(src, out, 1 << 20)
    else:
        with tarfile.open(archive) as t:
            f = t.extractfile(member)
            if f is None:
                sys.exit(f"{member} is not a regular file in {archive.name}")
            with open(dest, "wb") as out:
                shutil.copyfileobj(f, out, 1 << 20)


def ensure_venv(root: Path) -> Path:
    py = venv_python(root)
    if not py.is_file():
        print("creating the throwaway venv")
        subprocess.check_call([sys.executable, "-m", "venv", str(root / "tools" / "venv")])
        subprocess.check_call([str(py), "-m", "pip", "install", "--quiet", "--index-url", TORCH_INDEX, "torch"])
        subprocess.check_call([str(py), "-m", "pip", "install", "--quiet", *VENV_PACKAGES])
    return py


def run_worker(py: Path, step: str, *args: str) -> None:
    subprocess.check_call([str(py), "-I", str(Path(__file__).resolve()), "--worker", step, *args])


def build(root: Path, sources: dict) -> None:
    paths = sources["_paths"]
    models = root / "models"
    models.mkdir(parents=True, exist_ok=True)
    archive_name, member, target = host_ort()
    extract_member(paths[archive_name], member, models / target)
    shutil.copyfile(paths["tokenizer.json"], models / "tokenizer.json")
    shutil.copyfile(paths["clip-vit-l14-visual.onnx"], models / "visual-fp32.onnx")
    shutil.copyfile(paths["clip-vit-l14-textual.onnx"], models / "textual-fp32.onnx")

    py = ensure_venv(root)
    run_worker(py, "quantize", str(models))
    run_worker(py, "head", str(paths["aesthetic-l14.pth"]), str(paths["aesthetic-head-source.py"]),
               str(models / "aesthetic-l14.bin"))
    run_worker(py, "tokens", str(models / "tokenizer.json"), str(TOKENS))

    files = {n: {"sha256": sha256_file(models / n), "bytes": (models / n).stat().st_size}
             for n in expected_model_files()}
    record = {"sources": {k: v for k, v in sources.items() if k != "_paths"}, "files": files}
    (models / "SHA256SUMS.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8", newline="\n")


# ---------------------------------------------------------------- workers (run inside the venv)

def worker_quantize(models: Path) -> None:
    import numpy as np
    import onnx
    import onnxruntime as ort
    from onnxruntime.quantization import QuantType, quantize_dynamic
    from onnxruntime.transformers.float16 import convert_float_to_float16

    def cosine(a, b):
        a, b = a.ravel().astype(np.float64), b.ravel().astype(np.float64)
        return float(a @ b / (np.linalg.norm(a) * np.linalg.norm(b)))

    def session(path):
        options = ort.SessionOptions()
        options.log_severity_level = 3  # the quantized graphs log harmless constant-folding warnings
        return ort.InferenceSession(str(path), options, providers=["CPUExecutionProvider"])

    def run(path, feed):
        return session(path).run(None, feed)[0]

    visual_in = session(models / "visual-fp32.onnx").get_inputs()[0]
    textual_in = session(models / "textual-fp32.onnx").get_inputs()[0]
    print("visual input:", visual_in.name, visual_in.shape, visual_in.type)
    print("textual input:", textual_in.name, textual_in.shape, textual_in.type)

    if not (models / "visual-int8.onnx").is_file():
        quantize_dynamic(models / "visual-fp32.onnx", models / "visual-int8.onnx",
                         weight_type=QuantType.QInt8, op_types_to_quantize=["MatMul"])
    if not (models / "visual-fp16.onnx").is_file():
        model = onnx.load(str(models / "visual-fp32.onnx"))
        onnx.save(convert_float_to_float16(model, keep_io_types=True), str(models / "visual-fp16.onnx"))
        del model
    if not (models / "textual-int8.onnx").is_file():
        quantize_dynamic(models / "textual-fp32.onnx", models / "textual-int8.onnx",
                         weight_type=QuantType.QInt8, op_types_to_quantize=["MatMul"])

    rng = np.random.default_rng(1)
    image = rng.standard_normal((1, 3, 224, 224)).astype(np.float32)
    ref = run(models / "visual-fp32.onnx", {visual_in.name: image})
    for name in ("visual-fp16.onnx", "visual-int8.onnx"):
        print(f"{name}: cosine to fp32 = {cosine(ref, run(models / name, {visual_in.name: image})):.5f}")
    dtype = np.int32 if "int32" in textual_in.type else np.int64
    ids = np.zeros((1, 77), dtype=dtype)
    ids[0, :4] = [49406, 320, 2368, EOT]
    ref = run(models / "textual-fp32.onnx", {textual_in.name: ids})
    print(f"textual-int8.onnx: cosine to fp32 = "
          f"{cosine(ref, run(models / 'textual-int8.onnx', {textual_in.name: ids})):.5f}")


def worker_head(pth: Path, source: Path, out: Path) -> None:
    import re
    import numpy as np
    import torch

    # 1. The state dict cannot show activations: read the layer list of the published model.
    text = source.read_text(encoding="utf-8", errors="replace")
    block = re.search(r"nn\.Sequential\((.*?)\n\s*\)\n", text, re.S)
    if not block:
        sys.exit("cannot find the nn.Sequential block in the head source")
    layers = [l.strip().rstrip(",") for l in block.group(1).splitlines()
              if l.strip() and not l.strip().startswith("#")]
    kinds = [re.match(r"nn\.(\w+)", l).group(1) if re.match(r"nn\.(\w+)", l) else l for l in layers]
    print("head layers:", kinds)
    extra = [k for k in kinds if k not in ("Linear", "Dropout")]
    if extra:
        sys.exit(f"the LAION head contains {extra}: not a pure linear chain; "
                 "the plan then exports the head to ONNX (aesthetic.onnx) and task 4 runs it with ort")

    # 2. Load the weights (a pickle: weights_only is mandatory) and chain the Linear layers.
    state = torch.load(pth, map_location="cpu", weights_only=True)
    keys = list(state.keys())
    print("state dict:", [(k, tuple(state[k].shape)) for k in keys])
    linears = [k[:-len(".weight")] for k in keys if k.endswith(".weight")]
    if len(linears) != kinds.count("Linear") or len(keys) != 2 * len(linears):
        sys.exit("state dict does not match the layer list (unexpected parameters)")
    w_total = None
    b_total = None
    for name in linears:
        w = state[name + ".weight"].double().numpy()  # (out, in)
        b = state[name + ".bias"].double().numpy()
        if w_total is None:
            w_total, b_total = w, b
        else:
            if w.shape[1] != w_total.shape[0]:
                sys.exit(f"{name}: shape mismatch in the chain")
            w_total, b_total = w @ w_total, w @ b_total + b
    if w_total.shape != (1, 768) or b_total.shape != (1,):
        sys.exit(f"collapsed head has shape {w_total.shape}, expected (1, 768)")
    w32 = w_total[0].astype(np.float32)
    b32 = b_total.astype(np.float32)

    # 3. Compare against the real head on 1000 random normalized vectors; refuse to write on error.
    head = torch.nn.Sequential(*[
        torch.nn.Linear(state[n + ".weight"].shape[1], state[n + ".weight"].shape[0]) for n in linears])
    with torch.no_grad():
        for layer, name in zip(head, linears):
            layer.weight.copy_(state[name + ".weight"])
            layer.bias.copy_(state[name + ".bias"])
    g = torch.Generator().manual_seed(7)
    x = torch.randn(1000, 768, generator=g)
    x = x / x.norm(dim=-1, keepdim=True)
    with torch.no_grad():
        real = head(x).squeeze(-1).double().numpy()
    approx = x.double().numpy() @ w32.astype(np.float64) + float(b32[0])
    error = float(np.max(np.abs(real - approx)))
    print(f"head collapse max error over 1000 vectors: {error:.3e}")
    if error > 1e-4:
        sys.exit(f"collapsed head differs from the real head (max error {error:.3e} > 1e-4): not written")
    out.write_bytes(w32.astype("<f4").tobytes() + b32.astype("<f4").tobytes())
    print(f"wrote {out} ({out.stat().st_size} bytes)")


def worker_tokens(tokenizer_json: Path, out: Path) -> None:
    from transformers import CLIPTokenizerFast

    tok = CLIPTokenizerFast(tokenizer_file=str(tokenizer_json))
    entries = []
    for text in TOKEN_TEXTS + [LONG_TEXT]:
        ids = tok(text, truncation=True, max_length=77)["input_ids"]
        entries.append({"text": text, "ids": ids})
        print(repr(text[:40]), len(ids), ids[:6], ids[-2:])
    long_ids = entries[-1]["ids"]
    if len(long_ids) != 77 or long_ids[-1] != EOT or long_ids[0] != 49406:
        sys.exit("the long sentence is not truncated to 77 tokens ending with 49407")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(entries, indent=2) + "\n", encoding="utf-8", newline="\n")


# ---------------------------------------------------------------- entry point

def main() -> int:
    sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dir", type=Path, help="ai-dev root")
    parser.add_argument("--check", action="store_true", help="verify only, no network")
    parser.add_argument("--pin", action="store_true", help="fill the manifest SHA256 values")
    parser.add_argument("--worker", help=argparse.SUPPRESS)
    parser.add_argument("rest", nargs="*", help=argparse.SUPPRESS)
    args = parser.parse_args()

    if args.worker == "quantize":
        worker_quantize(Path(args.rest[0]))
        return 0
    if args.worker == "head":
        worker_head(Path(args.rest[0]), Path(args.rest[1]), Path(args.rest[2]))
        return 0
    if args.worker == "tokens":
        worker_tokens(Path(args.rest[0]), Path(args.rest[1]))
        return 0

    root = args.dir or default_root()
    if args.check:
        return check(root)
    sources = fetch_sources(root, args.pin)
    fetch_corpus(root)
    build(root, sources)
    return check(root)


if __name__ == "__main__":
    sys.exit(main())
