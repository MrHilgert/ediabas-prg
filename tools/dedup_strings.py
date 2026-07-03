#!/usr/bin/env python3
"""Merge + de-duplicate the GUI translation targets into a single unique list.

Input : the category masters emitted by `cargo run --example extract_strings`
        (strings/_gui_details.txt + strings/_gui_faults.txt). Each line is one
        exact German string, already trimmed and exact-deduped per file.

Output: strings/_to_translate.txt — the unique set to send to the translator.

Two dedup levels:
  * EXACT (always): byte-identical lines collapse to one. This is the safe key —
    the locale is looked up by the exact string the extractor produces, so the
    translation key MUST stay byte-identical to extraction.
  * NORMALIZED (--normalize, optional): additionally collapse strings that differ
    only by letter case or runs of whitespace (e.g. "6,5  ZIN" vs "6,5 ZIN").
    One representative per group is translated; strings/_variants.tsv maps every
    exact variant -> its representative so the finished locale can be expanded
    back to ALL exact keys (nothing is lost, every original string still resolves).

Usage:
    python tools/dedup_strings.py                # exact dedup only
    python tools/dedup_strings.py --normalize    # also collapse case/space near-dups
"""
import argparse
import collections
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
STRINGS = ROOT / "strings"
INPUTS = [STRINGS / "_gui_details.txt", STRINGS / "_gui_faults.txt"]
OUT_MAIN = STRINGS / "_to_translate.txt"
OUT_VARIANTS = STRINGS / "_variants.tsv"


def norm_key(s: str) -> str:
    """Case/space-insensitive key for near-duplicate grouping."""
    return re.sub(r"\s+", " ", s).strip().casefold()


def load_exact() -> "collections.OrderedDict[str, int]":
    """Read all input lines, exact-dedup (preserving first-seen order), count occurrences."""
    counts: "collections.OrderedDict[str, int]" = collections.OrderedDict()
    total = 0
    for path in INPUTS:
        if not path.exists():
            print(f"warn: missing {path.name}", file=sys.stderr)
            continue
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            total += 1
            counts[line] = counts.get(line, 0) + 1
    return counts, total


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--normalize", action="store_true",
                    help="also collapse case/whitespace near-duplicates (writes a variant map)")
    args = ap.parse_args()

    counts, total_occurrences = load_exact()
    unique = list(counts.keys())
    exact_removed = total_occurrences - len(unique)

    print(f"input occurrences : {total_occurrences}")
    print(f"exact-unique      : {len(unique)}")
    print(f"exact dupes removed: {exact_removed}")

    # Near-duplicate analysis over the exact-unique set.
    groups: "dict[str, list[str]]" = collections.defaultdict(list)
    for s in unique:
        groups[norm_key(s)].append(s)
    near_dup_variants = sum(len(v) - 1 for v in groups.values())
    print(f"normalized groups : {len(groups)}  (case/space near-dups: {near_dup_variants} extra variants)")

    if args.normalize:
        # Pick a stable representative per group: the shortest, then lexicographically
        # first — a clean canonical form to translate once.
        reps: list[str] = []
        variant_map: list[tuple[str, str]] = []  # (exact_variant, representative)
        for key, variants in groups.items():
            rep = sorted(variants, key=lambda x: (len(x), x))[0]
            reps.append(rep)
            for v in variants:
                variant_map.append((v, rep))
        reps.sort()
        OUT_MAIN.write_text("\n".join(reps) + "\n", encoding="utf-8")
        # Only emit rows where variant != representative (the ones needing expansion).
        rows = [f"{v}\t{r}" for v, r in variant_map if v != r]
        rows.sort()
        OUT_VARIANTS.write_text("\n".join(rows) + "\n", encoding="utf-8")
        print(f"--normalize: {len(reps)} representatives -> {OUT_MAIN.name}")
        print(f"            {len(rows)} variant->rep rows -> {OUT_VARIANTS.name}")
        print(f"            translation calls saved vs exact: {len(unique) - len(reps)}")
    else:
        out = sorted(unique)
        OUT_MAIN.write_text("\n".join(out) + "\n", encoding="utf-8")
        print(f"wrote {len(out)} unique strings -> {OUT_MAIN.name}")

    # Show the most-shared strings (translate-once wins the most).
    top = sorted(counts.items(), key=lambda kv: -kv[1])[:10]
    if top and top[0][1] > 1:
        print("\nmost repeated across the two masters:")
        for s, c in top:
            if c > 1:
                print(f"  {c:4d}x  {s[:70]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
