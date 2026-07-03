#!/usr/bin/env python3
"""Merge per-chunk translation TSVs into single locale files.

Reads strings/loc/ru/*.tsv and strings/loc/en/*.tsv (each line
`<german source>\t<translation>`), de-duplicates by source, validates, and
writes locale/de-ru.tsv + locale/de-en.tsv (sorted by source). Reports counts,
RU/EN key parity, malformed lines and conflicting duplicates.
"""
import collections
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "locale"
OUT.mkdir(exist_ok=True)


def load(lang):
    d = {}
    conflicts = 0
    malformed = 0
    files = sorted((ROOT / "strings" / "loc" / lang).glob("*.tsv"))
    for f in files:
        for line in f.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            if "\t" not in line:
                malformed += 1
                continue
            src, tr = line.split("\t", 1)
            if not src or not tr.strip():
                malformed += 1
                continue
            if src in d and d[src] != tr:
                conflicts += 1  # keep first-seen
                continue
            d.setdefault(src, tr)
    return d, files, conflicts, malformed


def main():
    ru, ru_files, ru_conf, ru_mal = load("ru")
    en, en_files, en_conf, en_mal = load("en")

    for lang, d in (("ru", ru), ("en", en)):
        rows = [f"{s}\t{t}" for s, t in sorted(d.items())]
        (OUT / f"de-{lang}.tsv").write_text("\n".join(rows) + "\n", encoding="utf-8")

    ru_keys, en_keys = set(ru), set(en)
    only_ru = ru_keys - en_keys
    only_en = en_keys - ru_keys

    print(f"RU: {len(ru_files)} files -> {len(ru)} unique  (conflicts {ru_conf}, malformed {ru_mal})")
    print(f"EN: {len(en_files)} files -> {len(en)} unique  (conflicts {en_conf}, malformed {en_mal})")
    print(f"both langs (paired): {len(ru_keys & en_keys)}")
    print(f"only RU: {len(only_ru)}   only EN: {len(only_en)}")
    print(f"wrote locale/de-ru.tsv ({len(ru)}), locale/de-en.tsv ({len(en)})")
    if only_ru or only_en:
        ex = list(only_ru)[:3] + list(only_en)[:3]
        print("  parity gaps (examples):", [s[:50] for s in ex])


if __name__ == "__main__":
    main()
