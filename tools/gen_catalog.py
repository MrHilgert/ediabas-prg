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
out.append("    pub validated: bool,      // confirmed connectable (DS2, tested)")
out.append("}")
out.append("")
out.append("pub struct ChassisEntries {")
out.append("    pub chassis: &'static str,")
out.append("    pub entries: &'static [CatEntry],")
out.append("}")
out.append("")

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
            rows.append(
                f"    CatEntry {{ code: {rs_str(script.strip())}, cat: Category::{cat}, "
                f"ru: {rs_str(ru)}, en: {rs_str(en)}, prg: {prg_rs}, validated: {validated} }},"
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

# --- screens.rs: per-ECU INPA screen trees from SGDAT/*.ipo -----------------
SCREENS_OUT = os.path.join(ROOT, "gui", "src", "screens_gen.rs")

def _rs_strs(items):
    return "&[" + ", ".join(rs_str(s) for s in items) + "]"

def _screen_title(kind, node):
    mm = re.search(r"analog(\d+)", node)
    if kind == "analog" and mm:
        return f"Analog {mm.group(1)}"
    return {"digital": "Digital", "status": "Status", "ident": "Ident",
            "info": "Info", "code": "Coding", "analog": "Analog"}.get(kind, node)

# --- live-measurement labels (EDC15C datasheet, DDE only) --------------------
# The .ipo gives WHICH params + grouping + exact job calls; the Bosch PDF C.1.1
# table gives per-selector unit + German long-name for the DDE (EDC15C) ECU.
_PDF_PATH = os.path.join(ROOT, "docs", "EDC15C-Funktionsbeschreibung-B079.CC0_ (2).pdf")
_UNIT_RE = re.compile(r"^(mm\^3|Nm|hPa|mV|km/h|%|Grad C|1/min|kg/h|V|bar|mbar|-)$")

def _load_pdf_map():
    """(selector.upper()->(unit,long), mnemonic.lower()->(unit,long)); ({},{}) if unavailable."""
    try:
        import fitz  # PyMuPDF
        doc = fitz.open(_PDF_PATH)
        lines = []
        for p in range(544, 552):  # C.1.1 "sorted by message number", pages 545-552
            lines += doc[p].get_text().split("\n")
        # Column headers + page header/footer boilerplate repeat across pages and
        # must not leak into a row's fields (else a param at a page boundary gets a
        # long-name like "Langbezeichner" or "Druckdatum: 05.Januar 2001").
        NOISE = {
            "Message", "Nr.", "Kurzbezeichner", "Format", "Parameter", "block",
            "Quantisierung", "Einheit", "Langbezeichner", "ANH_C079", "RBOS/EDS2",
        }
        NOISE_RE = re.compile(
            r"^(Funktionsbeschreibung|Seite |Abgabe:|Druckdatum|©|RBOS|Y \d|EDC15C|"
            r"Alle Rechte|Jede Verf|Bin/)", re.I)
        rows, cur = [], None
        for ln in lines:
            s = ln.strip()
            if re.fullmatch(r"[0-9a-fA-F]{4}", s):
                cur = [s]; rows.append(cur)
            elif cur is not None and s and s not in NOISE and not NOISE_RE.match(s):
                cur.append(s)
        sel_map, mnem_map = {}, {}
        for r in rows:
            if len(r) < 2:
                continue
            sel, mnem = r[0], r[1]
            unit = next((f for f in r[2:] if _UNIT_RE.match(f)), "")
            long = r[-1] if len(r) > 2 else mnem
            sel_map[sel.upper()] = (unit, long)
            mnem_map[mnem.lower()] = (unit, long)
        return sel_map, mnem_map
    except Exception as e:
        print(f"  (PDF label enrichment skipped: {e})")
        return {}, {}

_PDF_SEL, _PDF_MNEM = _load_pdf_map()

def _range_for_unit(unit):
    """Display bar range [lo,hi] from a unit; (0.0,0.0) = auto (renderer decides)."""
    return {
        "1/min": (0.0, 6000.0), "%": (0.0, 100.0), "V": (0.0, 16.0), "mV": (0.0, 5000.0),
        "mbar": (0.0, 3000.0), "hPa": (0.0, 3000.0), "Nm": (-100.0, 400.0),
        "km/h": (0.0, 250.0), "mm^3": (0.0, 80.0), "bar": (0.0, 2000.0),
        "kg/h": (0.0, 1000.0), "Grad C": (-40.0, 150.0),
    }.get(unit, (0.0, 0.0))

def _clean_bind(bind):
    """STAT_ZUOME_VE_WERT -> 'Zuome Ve' — human fallback label."""
    s = re.sub(r"^STAT(US)?_", "", bind)
    s = re.sub(r"_WERT$", "", s)
    return s.replace("_", " ").strip().title()

def _mnem_from_bind(bind):
    return re.sub(r"_WERT$", "", re.sub(r"^STAT(US)?_", "", bind)).lower()

def enrich_req(job, arg, bind, is_dde):
    """-> (bind_no_wert, label, unit, lo, hi). Value+unit come live from the VM; the
    PDF fills a nicer label/unit/range for DDE, else we clean the bind name."""
    bind_base = re.sub(r"_WERT$", "", bind)
    info = None
    if is_dde:
        if job == "MW_SELECT_LESEN_NORM" and arg:
            info = _PDF_SEL.get(arg.upper())
        if info is None:
            info = _PDF_MNEM.get(_mnem_from_bind(bind))
    if info:
        unit, long = info
        label = long or _clean_bind(bind)
    else:
        unit, label = "", _clean_bind(bind)
    lo, hi = _range_for_unit(unit)
    return bind_base, label, unit, lo, hi

def gen_screens():
    # Unique CFGDAT scripts across all chassis (a script's screens are chassis-independent).
    # Dedup case-insensitively (GS19/gs19 are the same ECU); `get()` is case-insensitive.
    scripts, seen_up = [], set()
    for sc in sorted({sc for cats in tree.values() for scr in cats.values() for sc in scr}):
        if sc.upper() not in seen_up:
            seen_up.add(sc.upper())
            scripts.append(sc)
    mods = []  # (code, sgbd, menus, screens)
    for script in scripts:
        path = SGDAT_IDX.get(script.upper())
        if not path:
            continue
        try:
            data = open(path, "rb").read()
        except Exception:
            continue
        if not ipo_parse.is_screen_script(data):
            continue
        m = ipo_parse.extract_module(data, PRG_UPPER, script)
        # keep only screens that carry rows; classify kind by node name; give a human
        # title and drop SGBD-variant twins (s_analog1 vs s_analog1_d40m57a1) by title.
        scr_list = []
        seen_titles = set()
        for node, rows in m["screens"].items():
            good = [r for r in rows if r["labels"] or r["results"]]
            if not good:
                continue
            kind = ipo_parse.screen_kind(node)
            title = _screen_title(kind, node)
            if title in seen_titles:
                continue
            seen_titles.add(title)
            scr_list.append((node, kind, title, good))
        menu_list = [(n, items) for n, items in m["menus"].items() if items]
        # Live-measurement screens: INPA's own curated, grouped, mirror-INPA job
        # calls from the .ipo handler (MW_SELECT+selector / individual STATUS_*).
        is_dde = (m["sgbd"] or "").upper().startswith("DDE40")
        meas_list = []
        try:
            for sc in meas_extract.extract_meas_screens(data):
                reqs = []
                for r in sc["reqs"]:
                    bb, lab, unit, lo, hi = enrich_req(r["job"], r["arg"], r["bind"], is_dde)
                    reqs.append((r["job"], r["arg"], bb, lab, unit, lo, hi))
                if reqs:
                    meas_list.append((sc["title"], reqs))
        except Exception:
            meas_list = []
        if not scr_list and not menu_list and not meas_list:
            continue
        mods.append((m["script"], m["sgbd"] or "", menu_list, scr_list, meas_list))

    KIND = {"digital": "Digital", "analog": "Analog", "status": "Status",
            "ident": "Ident", "info": "Info", "code": "Code", "other": "Other"}
    o = []
    o.append("//! GENERATED by tools/gen_catalog.py from INPA SGDAT/*.ipo — do not edit by hand.")
    o.append("//! Per-ECU INPA screen trees (menus + display screens: label<->result rows).")
    o.append("//! See docs/IPO-FORMAT.md. Keyed by CFGDAT script code (matches catalog.rs `code`).")
    o.append("")
    o.append("#[derive(Clone, Copy, PartialEq, Eq)]")
    o.append("pub enum SgKind { Digital, Analog, Status, Ident, Info, Code, Other }")
    o.append("")
    o.append("pub struct SgRow {")
    o.append("    pub labels: &'static [&'static str],")
    o.append("    pub results: &'static [&'static str],")
    o.append("}")
    o.append("")
    o.append("pub struct SgScreen {")
    o.append("    pub node: &'static str,")
    o.append("    pub title: &'static str,")
    o.append("    pub kind: SgKind,")
    o.append("    pub rows: &'static [SgRow],")
    o.append("}")
    o.append("")
    o.append("pub struct SgMenu {")
    o.append("    pub node: &'static str,")
    o.append("    pub items: &'static [&'static str],")
    o.append("}")
    o.append("")
    o.append("/// One live-measurement request, mirroring INPA's own call: either")
    o.append("/// `MW_SELECT_LESEN_NORM` with `arg`=2C10 selector, or a `STATUS_*` job (arg=\"\").")
    o.append("/// `bind` is the result base (value=`{bind}_WERT`, unit=`{bind}_EINH`).")
    o.append("pub struct MeasReq {")
    o.append("    pub job: &'static str,")
    o.append("    pub arg: &'static str,")
    o.append("    pub bind: &'static str,")
    o.append("    pub label: &'static str,")
    o.append("    pub unit: &'static str,")
    o.append("    pub lo: f32,")
    o.append("    pub hi: f32,")
    o.append("}")
    o.append("")
    o.append("pub struct MeasScreen {")
    o.append("    pub title: &'static str,")
    o.append("    pub reqs: &'static [MeasReq],")
    o.append("}")
    o.append("")
    o.append("pub struct SgModule {")
    o.append("    pub code: &'static str,")
    o.append("    pub sgbd: &'static str,")
    o.append("    pub menus: &'static [SgMenu],")
    o.append("    pub screens: &'static [SgScreen],")
    o.append("    pub meas: &'static [MeasScreen],")
    o.append("}")
    o.append("")

    n_scr = n_row = 0
    for i, (code, sgbd, menus, screens, meas) in enumerate(mods):
        o.append(f"const M{i}_SCREENS: &[SgScreen] = &[")
        for node, kind, title, rows in screens:
            n_scr += 1
            o.append(f"    SgScreen {{ node: {rs_str(node)}, title: {rs_str(title)}, "
                     f"kind: SgKind::{KIND[kind]}, rows: &[")
            for r in rows:
                n_row += 1
                o.append(f"        SgRow {{ labels: {_rs_strs(r['labels'])}, results: {_rs_strs(r['results'])} }},")
            o.append("    ] },")
        o.append("];")
    o.append("")
    for i, (code, sgbd, menus, screens, meas) in enumerate(mods):
        o.append(f"const M{i}_MENUS: &[SgMenu] = &[")
        for node, items in menus:
            o.append(f"    SgMenu {{ node: {rs_str(node)}, items: {_rs_strs(items)} }},")
        o.append("];")
    o.append("")
    n_meas_scr = n_meas_req = 0
    for i, (code, sgbd, menus, screens, meas) in enumerate(mods):
        o.append(f"const M{i}_MEAS: &[MeasScreen] = &[")
        for title, reqs in meas:
            n_meas_scr += 1
            o.append(f"    MeasScreen {{ title: {rs_str(title)}, reqs: &[")
            for job, arg, bind, lab, unit, lo, hi in reqs:
                n_meas_req += 1
                o.append(
                    f"        MeasReq {{ job: {rs_str(job)}, arg: {rs_str(arg)}, "
                    f"bind: {rs_str(bind)}, label: {rs_str(lab)}, unit: {rs_str(unit)}, "
                    f"lo: {lo:g}f32, hi: {hi:g}f32 }},"
                )
            o.append("    ] },")
        o.append("];")
    o.append("")
    o.append("pub static MODULES: &[SgModule] = &[")
    for i, (code, sgbd, menus, screens, meas) in enumerate(mods):
        o.append(f"    SgModule {{ code: {rs_str(code)}, sgbd: {rs_str(sgbd)}, "
                 f"menus: M{i}_MENUS, screens: M{i}_SCREENS, meas: M{i}_MEAS }},")
    o.append("];")
    o.append("")
    o.append("/// INPA screen tree for a module code (CFGDAT script), or None if not extracted.")
    o.append("/// Case-insensitive (catalog codes vary in case across CFGDAT files).")
    o.append("pub fn get(code: &str) -> Option<&'static SgModule> {")
    o.append("    MODULES.iter().find(|m| m.code.eq_ignore_ascii_case(code))")
    o.append("}")
    o.append("")
    o.append("/// INPA live-measurement screens (curated, grouped, mirror-INPA job calls) for")
    o.append("/// a module code, or empty if none extracted. Case-insensitive.")
    o.append("pub fn meas_screens(code: &str) -> &'static [MeasScreen] {")
    o.append("    get(code).map(|m| m.meas).unwrap_or(&[])")
    o.append("}")
    o.append("")
    with open(SCREENS_OUT, "w", encoding="utf-8") as f:
        f.write("\n".join(o))
    print(f"screens: modules={len(mods)}  screens={n_scr}  rows={n_row}  "
          f"meas_screens={n_meas_scr}  meas_reqs={n_meas_req}  -> {SCREENS_OUT}")

gen_screens()
