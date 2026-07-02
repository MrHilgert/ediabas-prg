#!/usr/bin/env python3
"""Extract INPA's per-screen live-measurement definitions from an `.ipo` handler.

INPA ships, per data-stream screen, an ordered list of exactly the parameters it
displays, grouped by screen title ("Аналоговые значения #N", "Цифровые значения",
"Коррекции ..."). Each parameter carries its job call verbatim — either
`MW_SELECT_LESEN_NORM` with an explicit 2C10 selector literal, or an individual
`STATUS_*` job — plus the result name it binds and (nearby) a label/unit.

We replicate INPA exactly: the request list per screen is what we poll. Where a
parameter appears in both a slow (all-`STATUS_*`) and a fast (`MW_SELECT`) variant
of the same screen, the fast one wins (INPA's own batched screen).

Labels come from the EDC15C datasheet (PDF C.1.1, keyed by selector) when
available, else the cleaned result mnemonic. Values + units come from the VM at
runtime (`STAT_<x>_WERT` / `_EINH`), so labels are cosmetic.
"""
import re
import sys
import json

# cp1251 printable: ASCII + full upper range (0x80-0xff covers Cyrillic incl. ё=0xb8).
def _pr(b):
    return (0x20 <= b <= 0x7e) or (0x80 <= b <= 0xff)

def scan_strings(data, start=0):
    """Ordered (offset, text) for every 0x0a-terminated printable run."""
    out = []
    i, hi = start, len(data)
    while i < hi:
        if _pr(data[i]):
            j = i
            while j < hi and data[j] != 0x0a and _pr(data[j]):
                j += 1
            if j < hi and data[j] == 0x0a and (j - i) >= 1:
                out.append((i, data[i:j].decode("cp1251", "replace")))
                i = j + 1
                continue
        i += 1
    return out

_JOB = re.compile(r"^STATUS_[A-Z0-9_]+$")
_RES = re.compile(r"^STAT(US)?_.+_WERT$", re.I)
_TITLE = re.compile(r"(значения|Коррекции|values|Korrektur)", re.I)
_SEL = re.compile(r"^[0-9A-Fa-f]{4}$")

def _norm_title(t):
    if not t:
        return "Поток данных"
    t = t.strip()
    t = t.replace("зачения", "значения")  # INPA typo in DDE40
    return t

def extract_meas_screens(data, handler_start=0x8000):
    """Return [ {title, reqs:[{job,arg,bind}]} ] — INPA's fast-variant screens.

    In the `.ipo` handler the screen title is a TRAILING marker (it follows the
    block it names), so we accumulate parameters and flush them under each title
    as we hit it. INPA emits each screen twice — a slow all-`STATUS_*` variant and
    a fast `MW_SELECT` variant; we globally dedup per bound result, preferring the
    fast occurrence, and drop segments left empty.
    """
    strs = scan_strings(data, handler_start)
    n = len(strs)
    # Segment params by trailing title marker. Each param: (job, arg, bind).
    segments = []       # (title, [param...])
    pending = []
    k = 0
    while k < n:
        _, s = strs[k]
        if _TITLE.search(s) and len(s) < 44:
            segments.append((_norm_title(s), pending))
            pending = []
            k += 1
            continue
        job = arg = None
        base = k
        if s == "MW_SELECT_LESEN_NORM" and k + 1 < n and _SEL.match(strs[k + 1][1]):
            job, arg, base = "MW_SELECT_LESEN_NORM", strs[k + 1][1].upper(), k + 1
        elif _JOB.match(s) and not s.startswith("STATUS_DIGITAL"):
            # STATUS_DIGITAL* are boolean bitfield jobs → the "Состояние" screen,
            # not a scalar bar in the measurement stream. Skip.
            job, arg = s, ""
        if job:
            bind = next(
                (strs[w][1] for w in range(base + 1, min(base + 6, n)) if _RES.match(strs[w][1])),
                None,
            )
            if bind:
                pending.append({"job": job, "arg": arg, "bind": bind})
                k = base + 1
                continue
        k += 1
    if pending:
        segments.append((_norm_title(None), pending))

    # Global dedup per bind: pick the winning occurrence (MW_SELECT preferred,
    # else earliest) and remember which segment it belongs to.
    win = {}  # bind -> (seg_idx, param)
    for si, (_, params) in enumerate(segments):
        for p in params:
            b = p["bind"]
            cur = win.get(b)
            better = cur is None or (
                p["job"] == "MW_SELECT_LESEN_NORM" and cur[1]["job"] != "MW_SELECT_LESEN_NORM"
            )
            if better:
                win[b] = (si, p)

    # Rebuild each segment keeping only its winning binds, in first-seen order.
    out = []
    for si, (title, params) in enumerate(segments):
        seen = set()
        reqs = []
        for p in params:
            b = p["bind"]
            if b in seen or win[b][0] != si:
                continue
            seen.add(b)
            reqs.append(p)
        if reqs:
            out.append({"title": title, "reqs": reqs})

    # Source titles are noisy trailing markers (duplicated #N, mixed language).
    # Renumber sequentially for a clean, meaningful group list.
    for i, s in enumerate(out, 1):
        inj = any("LAUFUNRUHE" in r["bind"] or "Injector" in s["title"] for r in s["reqs"])
        s["title"] = "Коррекция форсунок" if inj else f"Аналоговые значения {i}"
    return out


if __name__ == "__main__":
    path = sys.argv[1] if len(sys.argv) > 1 else "SGDAT/DDE40.IPO"
    data = open(path, "rb").read()
    screens = extract_meas_screens(data)
    total = sum(len(s["reqs"]) for s in screens)
    for s in screens:
        print(f"\n### {s['title']}  ({len(s['reqs'])})")
        for r in s["reqs"]:
            via = f"MW:{r['arg']}" if r["job"].startswith("MW") else r["job"]
            print(f"   {via:26} {r['bind']}")
    print(f"\nTOTAL {total} params in {len(screens)} screens")
