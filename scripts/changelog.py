"""User-facing changelog: changelog.json is the single source, CHANGELOG.md is generated.

  changelog.py                 regenerate CHANGELOG.md
  changelog.py --check         validate the JSON, the current version's entry and CHANGELOG.md
  changelog.py --notes 0.2.0   print the English release notes of one version (fails if absent)
"""

from pathlib import Path
import argparse
import datetime
import json
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "changelog.json"
TARGET = ROOT / "CHANGELOG.md"
CARGO = ROOT / "Cargo.toml"
TAGS = {"new": "New", "fix": "Fixed"}
ENTRY_KEYS = {"version", "date", "first", "quiet", "items"}
ITEM_KEYS = {"tag", "en", "fr"}
VERSION = re.compile(r"^\d+\.\d+\.\d+$")
# Visible text never carries a long dash (author's writing rule).
DASH = re.compile(r"\u2014| \u2013 ")
HEADER = "# Changelog\n\nWhat changed in Sortypix, as you would notice it. Newest first.\n"
QUIET = "Nothing you would notice in this one: upkeep under the hood."


def current_version():
    text = CARGO.read_text(encoding="utf-8")
    match = re.search(r'(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"', text)
    if not match:
        raise SystemExit("Cannot read the workspace version from Cargo.toml.")
    return match.group(1)


def version_key(version):
    return tuple(int(part) for part in version.split("."))


def validate(raw, version):
    """Every reason the raw changelog.json bytes are not a valid changelog for `version`."""
    errors = []
    if raw.startswith(b"\xef\xbb\xbf"):
        errors.append("changelog.json starts with a BOM")
    if b"\r" in raw:
        errors.append("changelog.json has CR line endings, LF only")
    try:
        entries = json.loads(raw.decode("utf-8-sig"))
    except (UnicodeDecodeError, ValueError) as error:
        return errors + [f"changelog.json is not valid UTF-8 JSON: {error}"]
    if not isinstance(entries, list) or not entries:
        return errors + ["changelog.json must be a non-empty array"]

    previous = None
    firsts = []
    for index, entry in enumerate(entries):
        where = f"entry {index}"
        if not isinstance(entry, dict):
            errors.append(f"{where} is not an object")
            continue
        number = entry.get("version")
        if not isinstance(number, str) or not VERSION.match(number):
            errors.append(f"{where}: version must look like 1.2.3")
            continue
        where = number
        unknown = set(entry) - ENTRY_KEYS
        if unknown:
            errors.append(f"{where}: unknown keys {sorted(unknown)}")
        try:
            day = datetime.date.fromisoformat(entry.get("date", ""))
        except (TypeError, ValueError):
            errors.append(f"{where}: date must be YYYY-MM-DD")
            day = None
        if previous:
            if version_key(number) >= version_key(previous[0]):
                errors.append(f"{where}: versions must be unique and newest first (after {previous[0]})")
            if day and previous[1] and day > previous[1]:
                errors.append(f"{where}: dates must not increase down the file (after {previous[0]})")
        previous = (number, day)
        for flag in ("first", "quiet"):
            if flag in entry and entry[flag] is not True:
                errors.append(f"{where}: {flag} is either true or absent")
        if entry.get("first") is True:
            firsts.append(index)
        items = entry.get("items")
        if not isinstance(items, list):
            errors.append(f"{where}: items must be an array")
            continue
        if not items and entry.get("quiet") is not True:
            errors.append(f'{where}: no items; a release with nothing visible says "quiet": true')
        if items and entry.get("quiet") is True:
            errors.append(f"{where}: quiet but has items")
        for position, item in enumerate(items, 1):
            spot = f"{where} item {position}"
            if not isinstance(item, dict):
                errors.append(f"{spot} is not an object")
                continue
            unknown = set(item) - ITEM_KEYS
            if unknown:
                errors.append(f"{spot}: unknown keys {sorted(unknown)}")
            if "tag" in item and item["tag"] not in TAGS:
                errors.append(f"{spot}: tag is one of {sorted(TAGS)} or absent")
            for language in ("en", "fr"):
                text = item.get(language)
                if not isinstance(text, str) or not text.strip():
                    errors.append(f"{spot}: {language} text is missing or empty")
                elif DASH.search(text):
                    errors.append(f"{spot}: {language} text has a long dash")
                elif "\n" in text:
                    errors.append(f"{spot}: {language} text is one line")
    if len(firsts) > 1:
        errors.append('"first": true marks one entry only')
    if firsts and firsts[0] != len(entries) - 1:
        errors.append('"first": true belongs to the oldest entry')
    # Past versions may be folded into a later entry; the current one never ships without its own.
    if not any(isinstance(entry, dict) and entry.get("version") == version for entry in entries):
        errors.append(f"version {version} has no entry in changelog.json")
    return errors


def item_line(item, bold):
    label = TAGS.get(item.get("tag"))
    if not label:
        return f"- {item['en']}"
    return f"- **{label}** · {item['en']}" if bold else f"- {label} · {item['en']}"


def notes(entry, bold=False):
    if entry.get("quiet"):
        return [QUIET]
    return [item_line(item, bold) for item in entry["items"]]


def render(entries):
    parts = [HEADER]
    for entry in entries:
        parts.append(f"\n## {entry['version']} · {entry['date']}\n\n")
        if entry.get("first"):
            parts.append("First public release.\n\n")
        parts.append("\n".join(notes(entry, bold=True)) + "\n")
    return "".join(parts)


def self_test(version):
    """A check that cannot refuse checks nothing: each planted defect must be reported."""
    good = {"version": version, "date": "2026-01-02", "items": [{"tag": "new", "en": "A", "fr": "B"}]}
    older = {"version": "0.0.1", "date": "2026-01-01", "first": True, "items": [{"en": "C", "fr": "D"}]}
    planted = {
        "BOM": b"\xef\xbb\xbf" + json.dumps([good]).encode(),
        "CR line endings": json.dumps([good], indent=1).replace("\n", "\r\n").encode(),
        "not valid UTF-8 JSON": b"[",
        "French missing": json.dumps([{**good, "items": [{"en": "A"}]}]).encode(),
        "empty English": json.dumps([{**good, "items": [{"en": " ", "fr": "B"}]}]).encode(),
        "duplicate version": json.dumps([good, good]).encode(),
        "oldest first": json.dumps([older, good]).encode(),
        "current version absent": json.dumps([older]).encode(),
        "silent empty release": json.dumps([{**good, "items": []}]).encode(),
        "unknown tag": json.dumps([{**good, "items": [{"tag": "tweak", "en": "A", "fr": "B"}]}]).encode(),
        "long dash": json.dumps([{**good, "items": [{"en": "A \u2014 B", "fr": "B"}]}]).encode(),
    }
    if validate(json.dumps([good, older]).encode(), version):
        return ["self-test: a valid changelog was refused"]
    if validate(json.dumps([{**good, "items": [], "quiet": True}]).encode(), version):
        return ["self-test: a quiet release was refused"]
    return [f"self-test: {name} was not reported" for name, raw in planted.items() if not validate(raw, version)]


def main():
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--notes", metavar="VERSION")
    args = parser.parse_args()
    version = current_version()
    raw = SOURCE.read_bytes() if SOURCE.is_file() else b""
    errors = validate(raw, args.notes or version)
    if args.check:
        errors += self_test(version)
        if not errors and (not TARGET.is_file() or TARGET.read_bytes() != render(json.loads(raw)).encode()):
            errors.append("CHANGELOG.md is out of date: run python scripts/changelog.py")
    if errors:
        for error in errors:
            print(f"CHANGELOG: {error}", file=sys.stderr)
        return 1
    entries = json.loads(raw)
    if args.notes:
        entry = next(entry for entry in entries if entry["version"] == args.notes)
        sys.stdout.buffer.write(("\n".join(notes(entry)) + "\n").encode())
    elif args.check:
        print(f"Changelog: OK ({version})")
    else:
        TARGET.write_bytes(render(entries).encode())
        print(f"WROTE: {TARGET.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
