#!/usr/bin/env python3
"""Regenerate data/emoji.tsv from Unicode's published data.

There is no emoji "API" - Unicode ships static files, which is better for us: the
generated table is checked into git and compiled into the binary, so builds stay offline
and startup does no IO. Run this when a new Emoji version lands (roughly annually).

Sources:
  emoji-test.txt   the authoritative list, in the official display order, grouped
  CLDR annotations short names and search keywords, per locale
"""

import argparse
import re
import sys
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

EMOJI_TEST = "https://unicode.org/Public/emoji/{ver}/emoji-test.txt"
CLDR_ANNOTATIONS = (
    "https://raw.githubusercontent.com/unicode-org/cldr/{cldr}/common/annotations/{loc}.xml"
)
CLDR_DERIVED = (
    "https://raw.githubusercontent.com/unicode-org/cldr/{cldr}"
    "/common/annotationsDerived/{loc}.xml"
)

# Skin-tone modifiers and hair components are not standalone emoji.
SKIP_GROUPS = {"Component"}


def fetch(url: str) -> str:
    print(f"  {url}", file=sys.stderr)
    with urllib.request.urlopen(url, timeout=60) as r:
        return r.read().decode("utf-8")


def parse_emoji_test(text: str):
    """Yield (char, group, subgroup, name) for fully-qualified emoji, in file order."""
    group = subgroup = ""
    # e.g. "1F600 ; fully-qualified # 😀 E1.0 grinning face"
    line_re = re.compile(
        r"^(?P<codes>[0-9A-F ]+?)\s*;\s*(?P<status>[\w-]+)\s*#\s*(?P<char>\S+)\s+E\d+\.\d+\s+(?P<name>.+)$"
    )
    for line in text.splitlines():
        if line.startswith("# group:"):
            group = line.split(":", 1)[1].strip()
            continue
        if line.startswith("# subgroup:"):
            subgroup = line.split(":", 1)[1].strip()
            continue
        if not line or line.startswith("#"):
            continue
        m = line_re.match(line)
        if not m or m.group("status") != "fully-qualified":
            continue
        if group in SKIP_GROUPS:
            continue
        yield m.group("char"), group, subgroup, m.group("name")


def parse_annotations(text: str, names: dict, keywords: dict) -> None:
    """CLDR carries the short name as type="tts" and search keywords untyped."""
    for ann in ET.fromstring(text).iter("annotation"):
        cp = ann.get("cp")
        if not cp or not ann.text:
            continue
        if ann.get("type") == "tts":
            names[cp] = ann.text.strip()
        else:
            keywords[cp] = [k.strip() for k in ann.text.split("|") if k.strip()]


def lookup(table: dict, ch: str):
    """CLDR keys most sequences without the U+FE0F variation selector that
    emoji-test.txt carries, so fall back to the stripped form."""
    if ch in table:
        return table[ch]
    return table.get(ch.replace("\ufe0f", ""))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--emoji-version", default="latest")
    ap.add_argument("--cldr-ref", default="main", help="tag or branch of unicode-org/cldr")
    ap.add_argument("--locale", default="en")
    ap.add_argument(
        "--out", type=Path, default=Path(__file__).resolve().parent.parent / "data/emoji.tsv"
    )
    args = ap.parse_args()

    print("fetching:", file=sys.stderr)
    entries = list(parse_emoji_test(fetch(EMOJI_TEST.format(ver=args.emoji_version))))

    names: dict[str, str] = {}
    keywords: dict[str, list[str]] = {}
    for url in (CLDR_ANNOTATIONS, CLDR_DERIVED):
        parse_annotations(
            fetch(url.format(cldr=args.cldr_ref, loc=args.locale)), names, keywords
        )

    rows = []
    for ch, group, subgroup, fallback_name in entries:
        # CLDR's name is the human one ("grinning face"); emoji-test's is the fallback
        # for anything CLDR has not annotated yet.
        name = lookup(names, ch) or fallback_name
        kw = lookup(keywords, ch) or [w for w in re.split(r"[\s-]+", name) if w]
        rows.append("\t".join([ch, group, subgroup, name, " | ".join(kw)]))

    previous = args.out.read_text().splitlines() if args.out.exists() else []
    args.out.write_text("\n".join(rows) + "\n")

    print(
        f"wrote {len(rows)} emoji to {args.out} "
        f"({len(rows) - len(previous):+d} vs previous {len(previous)})",
        file=sys.stderr,
    )
    missing = sum(1 for ch, *_ in entries if lookup(names, ch) is None)
    if missing:
        print(f"note: {missing} entries fell back to emoji-test names", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
