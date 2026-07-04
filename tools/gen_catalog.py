#!/usr/bin/env python3
"""Generate gui/src/catalog.rs from INPA's CFGDAT selection tree.

Parses every CFGDAT chassis/series file (both the flat per-chassis form
`[ROOT_MOTOR]` and the nested per-series form `[ROOT_E39_MOTOR]`), across the
Latin-1 English/German files and the CP1251 Russian series files, into a
chassis -> category -> [ENTRY] tree, maps each ENTRY script to a real .prg in
ecu/ (prefix match + override table), and emits a static Rust catalog.

Re-run:  python tools/gen_catalog.py
"""
import os
import re
import glob
import sys
import subprocess

import ipo_parse    # sibling module: .ipo screen-script parsing
import meas_extract # sibling module: .ipo live-measurement screen extraction

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CFGDAT = os.path.join(ROOT, "CFGDAT")
SGDAT = os.path.join(ROOT, "SGDAT")
ECU = os.path.join(ROOT, "ecu")
OUT = os.path.join(ROOT, "gui", "src", "catalog.rs")

# Chassis codes we recognise (mirrors gui/src/data.rs DATA).
CHASSIS = [
    "E81","E82","E87","E88","E21","E30","E36","E46","E90","E12","E28","E34",
    "E39","E60","E24","E63","E23","E32","E38","E65","E31","E53","E70","E83",
    "E71","E36/7","E85","E52",
]
# Longest-first so "E36/7" wins over "E36" when matching a section prefix.
CHASSIS_BY_LEN = sorted(CHASSIS, key=len, reverse=True)

# The one mapping we've validated live (DS2 DDE4.0).
VALIDATED = {"DDE40KW0.prg"}

# Descriptive script names whose SGBD differs from the script name.
# (Grows as variants are validated; empty = pure prefix matching.)
OVERRIDES = {
    # "ASCDSC46": "ABSKWP...prg",
}

def cat_of(section):
    s = section.upper()
    if "MOTOR" in s: return "Pwr"
    if "GETRIEBE" in s: return "Pwr"
    if "FAHRWERK" in s: return "Chs"
    if "KAROSSERIE" in s: return "Bdy"
    if "KOMMUNIKATION" in s or "КОММУНИКАЦ" in s: return "Inf"
    if "SICHERHEIT" in s: return "Saf"
    return None

def chassis_of(section):
    """Return the chassis code a section belongs to, or None (flat file)."""
    # section like ROOT_E39_MOTOR / ROOT_E39 / ROOT_MOTOR
    tail = section[len("ROOT_"):] if section.upper().startswith("ROOT_") else section
    for code in CHASSIS_BY_LEN:
        key = code.upper()
        if tail.upper() == key or tail.upper().startswith(key + "_"):
            return code
    return None

# --- .prg index -------------------------------------------------------------
PRG = sorted(os.path.splitext(os.path.basename(p))[0]
             for p in glob.glob(os.path.join(ECU, "*.prg")))
PRG_UP = [(p.upper(), p) for p in PRG]
PRG_UPPER = {up: orig for up, orig in PRG_UP}  # UPPER basename -> real basename

# --- SGDAT (.ipo screen script) index --------------------------------------
# Map UPPERCASE script/stem -> full .ipo path. The 446 real screen scripts are the
# uppercase *.IPO; we index every *.ipo/*.IPO and let the parser confirm.
SGDAT_IDX = {}
for _p in glob.glob(os.path.join(SGDAT, "*.[iI][pP][oO]")):
    _stem = os.path.splitext(os.path.basename(_p))[0]
    SGDAT_IDX.setdefault(_stem.upper(), _p)

_IPO_CACHE = {}  # path -> primary SGBD basename (or None), memoised per script

def _sgbd_from_ipo(script):
    """Resolve a CFGDAT script to its SGBD via the embedded names in SGDAT/<script>.IPO."""
    path = SGDAT_IDX.get(script.strip().upper())
    if not path:
        return None
    if path not in _IPO_CACHE:
        try:
            data = open(path, "rb").read()
            _IPO_CACHE[path] = ipo_parse.primary_sgbd(data, PRG_UPPER, script)
        except Exception:
            _IPO_CACHE[path] = None
    return _IPO_CACHE[path]

def _match_prg_prefix(script):
    s = script.strip().upper()
    cands = [orig for up, orig in PRG_UP if up.startswith(s)]
    if not cands:
        return None
    cands.sort(key=lambda x: (len(x), x))  # shortest/first = base variant
    return cands[0]

def match_prg(script):
    """Map a CFGDAT ENTRY script name to a real ecu/*.prg (returns 'NAME.prg' or None).

    Priority: manual OVERRIDES > authoritative SGBD from the .ipo screen script >
    legacy filename-prefix heuristic."""
    s = script.strip().upper()
    if not s:
        return None
    if s in OVERRIDES:
        return OVERRIDES[s]
    base = _sgbd_from_ipo(script) or _match_prg_prefix(script)
    return base + ".prg" if base else None

# --- CFGDAT parsing ---------------------------------------------------------
def file_lang_enc(name):
    low = name.lower()
    if "серия" in low or name in ("Двигатели.ENG", "Все_двигатели.eng", "Мини.ENG", "Moto.ENG"):
        return "ru", "cp1251"
    if name.upper().endswith(".GER"):
        return "de", "latin-1"
    return "en", "latin-1"

def clean(s):
    return re.sub(r"\s+", " ", s).strip()

# tree[chassis][cat][script] = {"en":.., "ru":.., "de":..}
tree = {}

def add(chassis, cat, script, lang, label):
    script = script.strip()
    if not script or not cat:
        return
    tree.setdefault(chassis, {}).setdefault(cat, {}).setdefault(script, {})
    lbl = clean(label)
    if lbl:
        tree[chassis][cat][script][lang] = lbl

for path in glob.glob(os.path.join(CFGDAT, "*")):
    name = os.path.basename(path)
    if not (name.upper().endswith(".ENG") or name.upper().endswith(".GER")):
        continue
    lang, enc = file_lang_enc(name)
    try:
        with open(path, encoding=enc, errors="replace") as f:
            lines = f.read().splitlines()
    except Exception:
        continue
    # A flat file's chassis is inferred from its name (E46.ENG -> E46).
    stem = os.path.splitext(name)[0]
    flat_chassis = stem if stem in CHASSIS else None
    cur_cat = None
    cur_chassis = flat_chassis
    for line in lines:
        st = line.strip()
        if st.startswith("[") and st.endswith("]"):
            sec = st[1:-1]
            c = chassis_of(sec)
            if c is not None:
                cur_chassis = c
            elif flat_chassis is not None:
                cur_chassis = flat_chassis
            cur_cat = cat_of(sec)
            continue
        if st.startswith(";") or not st.startswith("ENTRY="):
            continue
        body = st[len("ENTRY="):]
        parts = body.split(",")
        if len(parts) < 2:
            continue
        script = parts[0]
        label = parts[1]
        # Some CFGDAT files right-pad an EDIABAS group token into a fixed column of the
        # label field: `ENTRY=<script>,<description>      <GROUP>,`. We don't use the
        # group (variant identification is driven by the .ipo's `D_<ADDR>.GRP`), but we
        # still strip that trailing ALL-CAPS token so it doesn't leak into the UI label.
        m = re.search(r"\s{2,}([A-Z][A-Z0-9_]{1,15})\s*$", label)
        if m:
            label = label[: m.start()]
        if cur_chassis and cur_cat:
            add(cur_chassis, cur_cat, script, lang, label)

# --- emit Rust --------------------------------------------------------------
def rs_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'

def label(langs, script):
    en = langs.get("en") or langs.get("de") or langs.get("ru") or script
    ru = langs.get("ru") or langs.get("en") or langs.get("de") or script
    return ru, en

out = []
out.append("//! GENERATED by tools/gen_catalog.py from INPA CFGDAT — do not edit by hand.")
out.append("//! Per-chassis ECU selection tree (categories + entries + .prg mapping).")
out.append("")
out.append("use crate::ecu::Category;")
out.append("")
out.append("pub struct CatEntry {")
out.append("    pub code: &'static str,   // INPA script / SGBD base name")
out.append("    pub cat: Category,")
out.append("    pub ru: &'static str,")
out.append("    pub en: &'static str,")
out.append("    pub prg: Option<&'static str>,")
out.append("    pub bus: &'static str,    // protocol family from the .prg CommParameter (or \"—\")")
out.append("    pub addr: &'static str,   // ECU diagnostic address, hex, from the .prg (or \"—\")")
out.append("    pub validated: bool,      // confirmed connectable (DS2, tested)")
out.append("}")
out.append("")
out.append("pub struct ChassisEntries {")
out.append("    pub chassis: &'static str,")
out.append("    pub entries: &'static [CatEntry],")
out.append("}")
out.append("")

def prg_meta():
    """Map each `ecu/*.prg` basename → (bus, addr) via the Rust `ediabas-prg meta`
    command — the SINGLE source of truth for the CommParameter concept, so the Python
    generator never re-implements (and drifts from) the decoder. A missing binary or a
    .prg with no CommParameter just stays out of the map (caller defaults to '—')."""
    exe = os.path.join(ROOT, "target", "release", "ediabas-prg")
    if not os.path.exists(exe) and os.path.exists(exe + ".exe"):
        exe += ".exe"
    if not os.path.exists(exe):
        print(f"WARNING: {exe} not found — `cargo build --release` for real bus/addr; using '—'", file=sys.stderr)
        return {}
    files = glob.glob(os.path.join(ECU, "*.prg")) + glob.glob(os.path.join(ECU, "*.PRG"))
    meta = {}
    for i in range(0, len(files), 200):  # chunk to stay under command-line length limits
        chunk = files[i:i + 200]
        try:
            out = subprocess.run([exe, "meta", *chunk], capture_output=True, text=True, timeout=600).stdout
        except Exception as e:
            print(f"WARNING: `meta` failed: {e}", file=sys.stderr)
            break
        for line in out.splitlines():
            parts = line.split("\t")
            if len(parts) == 4 and parts[1] not in ("-", "?", ""):
                meta[parts[0].lower()] = (parts[1], parts[2])
    return meta

PRG_META = prg_meta()

n_ch = 0
n_en = 0
n_prg = 0
blocks = []
for chassis in sorted(tree.keys()):
    cats = tree[chassis]
    rows = []
    # stable order: category then script
    catorder = ["Pwr", "Chs", "Saf", "Bdy", "Gwy", "Inf"]
    for cat in catorder:
        if cat not in cats:
            continue
        for script in sorted(cats[cat].keys()):
            langs = cats[cat][script]
            ru, en = label(langs, script)
            prg = match_prg(script)
            n_en += 1
            if prg:
                n_prg += 1
            validated = "true" if (prg in VALIDATED) else "false"
            prg_rs = f"Some({rs_str(prg)})" if prg else "None"
            bus, addr = PRG_META.get((prg or "").lower(), ("—", "—"))
            rows.append(
                f"    CatEntry {{ code: {rs_str(script.strip())}, cat: Category::{cat}, "
                f"ru: {rs_str(ru)}, en: {rs_str(en)}, prg: {prg_rs}, "
                f"bus: {rs_str(bus)}, addr: {rs_str(addr)}, validated: {validated} }},"
            )
    if not rows:
        continue
    n_ch += 1
    const = "ENTRIES_" + re.sub(r"[^A-Z0-9]", "_", chassis.upper())
    blocks.append(f"const {const}: &[CatEntry] = &[\n" + "\n".join(rows) + "\n];\n")

out.append("\n".join(blocks))
out.append("pub const CATALOG: &[ChassisEntries] = &[")
for chassis in sorted(tree.keys()):
    if not any(tree[chassis].get(c) for c in tree[chassis]):
        continue
    const = "ENTRIES_" + re.sub(r"[^A-Z0-9]", "_", chassis.upper())
    if const + ":" in "\n".join(blocks) or (const + ": ") in "\n".join(blocks):
        out.append(f"    ChassisEntries {{ chassis: {rs_str(chassis)}, entries: {const} }},")
out.append("];")
out.append("")
out.append("/// Entries for a chassis code, or empty if not in the catalog.")
out.append("pub fn entries_for(chassis: &str) -> &'static [CatEntry] {")
out.append("    CATALOG.iter().find(|c| c.chassis == chassis).map(|c| c.entries).unwrap_or(&[])")
out.append("}")
out.append("")

with open(OUT, "w", encoding="utf-8") as f:
    f.write("\n".join(out))

print(f"chassis={n_ch}  entries={n_en}  with_prg={n_prg}  -> {OUT}")
