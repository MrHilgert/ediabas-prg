#!/usr/bin/env python3
"""Parse INPA `.ipo` screen scripts (SGDAT/*.IPO).

`.ipo` is a compiled INPA stack p-code module with an inline string pool. We do NOT
execute the bytecode; we mine the ordered string stream (which is reliable, ~4.5/5)
and classify strings by content. This gives, per module: the embedded SGBD (.prg)
name(s), the diagnostic jobs called, the result names to display, screen display
rows (label <-> result-name lists), F-key/menu labels, and activation commands.

Format notes (see docs/IPO-FORMAT.md):
  - Fingerprint: file begins with  05 00 "TEST-Infotext" 0a
  - Strings are printable runs terminated by 0x0a. Some carry a 1-byte type tag
    (05=ident/proc, 06=literal, 01/02=screen/menu name); many operands are bare
    (untagged) — so we scan tag-free and classify by content.
  - Text is Windows-1251 (Cyrillic labels).

This module is imported by tools/gen_catalog.py and is runnable standalone for
inspection:  python tools/ipo_parse.py SGDAT/DDE40.IPO
"""
import os
import re
import sys
import glob

# --- low-level string scan --------------------------------------------------

FINGERPRINT = b"\x05\x00TEST-Infotext\x0a"


def _printable(b):
    # ASCII printable, CP1251 Cyrillic high range, or Ё/ё.
    return (0x20 <= b <= 0x7E) or (0xC0 <= b <= 0xFF) or b in (0xA8, 0xB8)


def scan_strings(data, minlen=2):
    """Yield (offset, text) for every printable run terminated by 0x0a (CP1251)."""
    i, n = 0, len(data)
    while i < n:
        if _printable(data[i]):
            j = i
            while j < n and data[j] != 0x0A and _printable(data[j]):
                j += 1
            if j < n and data[j] == 0x0A and (j - i) >= minlen:
                yield i, data[i:j].decode("cp1251", errors="replace")
                i = j + 1
                continue
        i += 1


def is_screen_script(data):
    """True for the 446 real screen scripts (have m_main / s_main nodes)."""
    if not data.startswith(b"\x05\x00TEST-Infotext"):
        return False
    return (b"m_main\x0a" in data) or (b"s_main\x0a" in data)


# --- content classification -------------------------------------------------

RE_SCREEN = re.compile(r"^[ms]_[a-z0-9_]+$")            # m_main, s_analog1, s_digital_d40m57a1
RE_JOBNAME = re.compile(r"^[A-Z][A-Z0-9_]{2,}$")         # IDENT, FS_LESEN, STATUS_DIGITAL
RE_RESULT = re.compile(r"^(STAT_|STATUS_|ID_|F_|STATUS|STAT)[A-Za-z0-9_]*$")
RE_FKEY = re.compile(r"^<\s*F\d+")                       # "< F2 >  Identification"
RE_HEXPAYLOAD = re.compile(r"^(0x[0-9A-Fa-f]+|\d+;[0-9A-Fa-f ]+|[0-9A-Fa-f]{2}( [0-9A-Fa-f]{2})+)$")
RE_PROC = re.compile(r"^(inpainit|inpaexit|__inpa_|ergebnis|ansteuerung|cabimain|ftx_|kvp_)")


def split_sgbd_tokens(s):
    """Split a compound IDENT/apply string into candidate SGBD tokens.
    e.g. 'GS19/V1.06,GS19A/V3.01' -> ['GS19','GS19A']; 'DDE40KW0,D40M57A1' -> [...]."""
    parts = re.split(r"[,/;\s]+", s.strip())
    return [p for p in parts if p]


def sgbd_candidates(data, prg_upper):
    """Ordered, de-duplicated list of SGBD basenames embedded in the .ipo that exist
    in ecu/*.prg. `prg_upper` maps UPPERCASE basename -> real basename. First entry is
    the best primary guess (prefers a NAME,VARIANT info literal, then frequency)."""
    from collections import Counter

    freq = Counter()
    info_first = []  # SGBDs appearing as the first token of a "NAME,VARIANT" literal
    for _off, s in scan_strings(data):
        toks = split_sgbd_tokens(s)
        # info literal heuristic: 'DDE40KW0,D40M57A1' — comma-joined, first token all-caps SGBD
        if "," in s and len(toks) >= 2 and toks[0].upper() in prg_upper:
            info_first.append(prg_upper[toks[0].upper()])
        for t in toks:
            up = t.upper()
            if up in prg_upper:
                freq[prg_upper[up]] += 1

    # Drop generic helpers that appear in nearly every DS2 file (case-insensitive).
    JUNK = {"FLASH", "UTILITY"}
    freq = Counter({b: c for b, c in freq.items() if b.upper() not in JUNK})
    info_first = [x for x in info_first if x.upper() not in JUNK]

    ordered = []
    for x in info_first + [b for b, _ in freq.most_common()]:
        if x not in ordered:
            ordered.append(x)
    return ordered


def _common_prefix_len(a, b):
    n = 0
    for x, y in zip(a, b):
        if x != y:
            break
        n += 1
    return n


def primary_sgbd(data, prg_upper, script):
    """Best single SGBD (.prg basename) for a CFGDAT script name, or None.

    The script name (e.g. 'MSV70', 'DDE40', 'ascdsc46') is the strong signal: the
    SGBD usually equals it or shares a long prefix. Falls back to the IDENT
    info-literal / most-frequent candidate."""
    cands = sgbd_candidates(data, prg_upper)
    if not cands:
        return None
    su = (script or "").upper()
    # Left/right split-block ECUs (DME 1.7/5.2 …): the script's trailing L/R names the
    # side; prefer the matching-side SGBD (DM5212L0 vs DM5212R0).
    side_m = re.search(r"([LR])$", su)
    side = side_m.group(1) if side_m else None
    other = {"L": "R", "R": "L"}.get(side)

    def score(c, rank):
        cu = c.upper()
        s = 0
        if cu == su:
            s += 1000
        elif cu.startswith(su) or su.startswith(cu):
            s += 300 + _common_prefix_len(cu, su)
        else:
            s += _common_prefix_len(cu, su) * 5
        if side:
            if side in cu and other not in cu:
                s += 200
            elif other in cu and side not in cu:
                s -= 200
        s += max(0, 30 - rank)  # earlier candidate (info-literal / frequent) = slight boost
        return s

    return max(((c, score(c, i)) for i, c in enumerate(cands)), key=lambda x: x[1])[0]


# --- screen / row extraction ------------------------------------------------

def _is_result_list(s):
    head = s.split(";")[0].strip()
    return bool(RE_RESULT.match(head))


def _is_label(s):
    if _is_result_list(s) or RE_FKEY.match(s) or RE_HEXPAYLOAD.match(s) or RE_SCREEN.match(s):
        return False
    # Display text: has a space, a lowercase letter, or Cyrillic — i.e. human-readable.
    return any(c.islower() or c.isspace() or ("Ѐ" <= c <= "ӿ") for c in s)


def _split_cols(label):
    # Multi-column display labels are comma-joined ("Rail-pressure, Charge-air pressure").
    return [p.strip() for p in label.split(",") if p.strip()]


def screen_kind(node):
    """Classify a screen node by its name into a render kind."""
    if node.startswith("s_digital"):
        return "digital"
    if node.startswith("s_analog"):
        return "analog"
    if node.startswith("s_ident"):
        return "ident"
    if node.startswith("s_info"):
        return "info"
    if node.startswith("s_code"):
        return "code"
    if node.startswith("s_fehler") or node.startswith("s_status"):
        return "status"
    return "other"


def extract_module(data, prg_upper, script):
    """Semantic model of a screen script: SGBD(s), menus, display screens (label<->result
    rows), and the raw string buckets (jobs/payloads/fkeys) for later refinement."""
    toks = list(scan_strings(data))
    sgbds = sgbd_candidates(data, prg_upper)
    prim = primary_sgbd(data, prg_upper, script)

    # Node segmentation: every RE_SCREEN token starts a new node segment; a procedure
    # name (inpainit / __inpa_* / ergebnis* / ansteuerung …) closes it, so the trailing
    # code region isn't swept into the last screen.
    nodes = []  # (name, [strings])
    cur = None
    for _off, s in toks:
        if RE_SCREEN.match(s):
            cur = (s, [])
            nodes.append(cur)
        elif RE_PROC.match(s):
            cur = None
        elif cur is not None:
            cur[1].append(s)

    menus, screens = {}, {}
    for name, strs in nodes:
        if name.startswith("m_"):
            items = [s for s in strs if _is_label(s)]
            if items:
                menus.setdefault(name, items)
        elif name.startswith("s_"):
            rows, pending = [], None
            for s in strs:
                if _is_result_list(s):
                    results = [r.strip() for r in s.split(";") if r.strip()]
                    labels = _split_cols(pending) if pending else []
                    rows.append({"labels": labels, "results": results})
                    pending = None
                elif _is_label(s):
                    if pending is not None:
                        rows.append({"labels": _split_cols(pending), "results": []})
                    pending = s
                # jobs/payloads/fkeys ignored for row building
            if pending is not None:
                rows.append({"labels": _split_cols(pending), "results": []})
            if rows:
                screens.setdefault(name, rows)

    # Raw buckets (whole file) for menus/actions refinement in later phases.
    fkeys = [s for _o, s in toks if RE_FKEY.match(s)]
    payloads = [s for _o, s in toks if RE_HEXPAYLOAD.match(s) and len(s) > 3]

    return {
        "script": script,
        "sgbd": prim,
        "sgbds": sgbds,
        "menus": menus,
        "screens": screens,
        "fkeys": fkeys,
        "payloads": payloads,
    }


# --- standalone inspection --------------------------------------------------

def _ecu_prg_upper(root):
    prg = {}
    for p in glob.glob(os.path.join(root, "ecu", "*.prg")):
        base = os.path.splitext(os.path.basename(p))[0]
        prg[base.upper()] = base
    return prg


def main():
    path = sys.argv[1]
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    data = open(path, "rb").read()
    prg_upper = _ecu_prg_upper(root)
    print(f"# {path} ({len(data)} bytes)  screen_script={is_screen_script(data)}")
    print(f"# SGBD candidates: {sgbd_candidates(data, prg_upper)}")
    print("# --- classified strings ---")
    for off, s in scan_strings(data):
        kind = "?"
        if RE_SCREEN.match(s):
            kind = "SCREEN"
        elif RE_FKEY.match(s):
            kind = "FKEY"
        elif RE_RESULT.match(s) or ";STAT" in s or ";STATUS" in s:
            kind = "RESULT"
        elif RE_HEXPAYLOAD.match(s):
            kind = "PAYLOAD"
        elif RE_JOBNAME.match(s):
            kind = "JOB?"
        else:
            kind = "LABEL"
        t = s if len(s) <= 64 else s[:61] + "..."
        print(f"{off:#08x} {kind:7} {t!r}")


if __name__ == "__main__":
    main()
