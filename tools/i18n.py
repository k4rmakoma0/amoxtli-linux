#!/usr/bin/env python3
"""Translation helper for Amoxtli (no gettext tools required).

    tools/i18n.py extract          # scan src/*.rs for tr()/trn() -> po/amoxtli.pot
    tools/i18n.py update           # extract, then merge new strings into every po/<lang>.po
    tools/i18n.py compile [DIR]    # po/*.po -> DIR/<lang>/LC_MESSAGES/amoxtli.mo (default target/locale)
    tools/i18n.py check            # report missing / obsolete translations per language
"""

from __future__ import annotations

import ast as _ast
import re
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src"
PO_DIR = ROOT / "po"
LOCALE_DIR = ROOT / "target" / "locale"
DOMAIN = "amoxtli"


# ----------------------------------------------------------------- extract

RUST_STR = r'"((?:[^"\\]|\\.)*)"'
CALL = re.compile(r'\b(trn?)\(\s*' + RUST_STR + r'(?:\s*,\s*' + RUST_STR + r')?')


def unescape(s: str) -> str:
    return re.sub(r'\\(.)', lambda m: {"n": "\n", "t": "\t"}.get(m.group(1), m.group(1)), s)


def extract() -> list[tuple[str, str | None, list[str]]]:
    """Return [(msgid, msgid_plural, [locations])] in source order."""
    found: dict[tuple[str, str | None], list[str]] = {}
    for path in sorted(SRC.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for m in CALL.finditer(text):
            func, a, b = m.group(1), m.group(2), m.group(3)
            if func == "trn" and b is None:
                continue
            key = (unescape(a), unescape(b) if func == "trn" else None)
            line = text.count("\n", 0, m.start()) + 1
            found.setdefault(key, []).append(f"{path.relative_to(ROOT)}:{line}")
    return [(k[0], k[1], locs) for k, locs in found.items()]


def po_quote(s: str) -> str:
    s = s.replace("\\", "\\\\").replace('"', '\\"').replace("\t", "\\t")
    if "\n" in s and s != "\n":
        parts = s.split("\n")
        lines = [p + "\\n" for p in parts[:-1]] + ([parts[-1]] if parts[-1] else [])
        return '""\n' + "\n".join(f'"{line}"' for line in lines)
    return f'"{s.replace(chr(10), chr(92) + "n")}"'


def write_pot(entries):
    out = [
        "# Amoxtli translation template.",
        "# This file is distributed under the same license as the Amoxtli package.",
        'msgid ""',
        'msgstr ""',
        '"Project-Id-Version: amoxtli\\n"',
        '"Content-Type: text/plain; charset=UTF-8\\n"',
        '"Content-Transfer-Encoding: 8bit\\n"',
        "",
    ]
    for msgid, plural, locs in entries:
        out.append("#: " + " ".join(locs))
        if "{" in msgid:
            out.append("#, python-brace-format")
        out.append("msgid " + po_quote(msgid))
        if plural:
            out.append("msgid_plural " + po_quote(plural))
            out.append('msgstr[0] ""')
            out.append('msgstr[1] ""')
        else:
            out.append('msgstr ""')
        out.append("")
    PO_DIR.mkdir(exist_ok=True)
    (PO_DIR / f"{DOMAIN}.pot").write_text("\n".join(out), encoding="utf-8")
    print(f"wrote po/{DOMAIN}.pot ({len(entries)} messages)")


# ------------------------------------------------------------------- parse

def parse_po(path: Path) -> dict[tuple[str, str | None], list[str]]:
    """Parse a .po file into {(msgid, msgid_plural): [msgstr, ...]} (fuzzy entries skipped)."""
    entries = {}
    cur: dict = {}
    field = None
    fuzzy = False

    def flush():
        nonlocal cur, fuzzy
        if "msgid" in cur and not fuzzy:
            strs = [cur[k] for k in sorted(k for k in cur if k.startswith("msgstr"))]
            entries[(cur["msgid"], cur.get("msgid_plural"))] = strs
        cur, fuzzy = {}, False

    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line:
            flush()
            field = None
            continue
        if line.startswith("#,") and "fuzzy" in line:
            fuzzy = True
        if line.startswith("#"):
            continue
        if line.startswith('"'):
            cur[field] += _ast.literal_eval(line)
            continue
        key, _, value = line.partition(" ")
        if key == "msgid" and "msgid" in cur:
            flush()
        field = key.replace("[", "_").replace("]", "")  # msgstr[0] -> msgstr_0
        cur[field] = _ast.literal_eval(value)
    flush()
    return entries


def po_header(path: Path) -> list[str]:
    """The header block (everything before the first real msgid entry)."""
    lines = path.read_text(encoding="utf-8").splitlines()
    out = []
    seen_header = False
    for line in lines:
        if line.startswith("msgid ") and seen_header:
            break
        if line.startswith('msgid ""') and not seen_header:
            seen_header = True
        if line.startswith("#: "):
            break
        out.append(line)
    while out and not out[-1].strip():
        out.pop()
    return out


def nplurals(header: list[str]) -> int:
    m = re.search(r"nplurals=(\d+)", "\n".join(header))
    return int(m.group(1)) if m else 2


def update():
    """Like msgmerge: regenerate every po from the sources, keeping translations."""
    entries = extract()
    write_pot(entries)
    for po in sorted(PO_DIR.glob("*.po")):
        header = po_header(po)
        old = parse_po(po)
        n = nplurals(header)
        out = header + [""]
        new = 0
        for msgid, plural, locs in entries:
            out.append("#: " + " ".join(locs))
            if "{" in msgid:
                out.append("#, python-brace-format")
            out.append("msgid " + po_quote(msgid))
            strs = old.get((msgid, plural))
            if not strs or not any(strs):
                new += msgid != "translator-credits"
            if plural:
                out.append("msgid_plural " + po_quote(plural))
                forms = strs if strs and len(strs) == n else [""] * n
                for i, f in enumerate(forms):
                    out.append(f"msgstr[{i}] " + po_quote(f))
            else:
                out.append("msgstr " + po_quote(strs[0] if strs else ""))
            out.append("")
        po.write_text("\n".join(out), encoding="utf-8")
        print(f"updated {po.stem}: {new} untranslated")


# ----------------------------------------------------------------- compile

def write_mo(entries: dict, dest: Path):
    pairs = []
    for (msgid, plural), strs in entries.items():
        if not any(strs) and msgid:
            continue  # untranslated: fall back to English
        key = msgid if plural is None else msgid + "\0" + plural
        pairs.append((key.encode(), "\0".join(strs).encode()))
    pairs.sort()
    n = len(pairs)
    ids = b"".join(k + b"\0" for k, _ in pairs)
    strs = b"".join(v + b"\0" for _, v in pairs)
    key_start = 7 * 4 + 16 * n
    val_start = key_start + len(ids)
    koffs, voffs = [], []
    o = 0
    for k, _ in pairs:
        koffs += [len(k), key_start + o]
        o += len(k) + 1
    o = 0
    for _, v in pairs:
        voffs += [len(v), val_start + o]
        o += len(v) + 1
    header = struct.pack("Iiiiiii", 0x950412DE, 0, n, 7 * 4, 7 * 4 + 8 * n, 0, 0)
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(header + struct.pack(f"{len(koffs)}i", *koffs)
                     + struct.pack(f"{len(voffs)}i", *voffs) + ids + strs)


def compile_all(dest: Path = LOCALE_DIR):
    for po in sorted(PO_DIR.glob("*.po")):
        lang = po.stem
        entries = parse_po(po)
        write_mo(entries, dest / lang / "LC_MESSAGES" / f"{DOMAIN}.mo")
        print(f"compiled {lang}")


def check() -> int:
    wanted = {(m, p) for m, p, _ in extract() if m != "translator-credits"}
    bad = 0
    for po in sorted(PO_DIR.glob("*.po")):
        entries = parse_po(po)
        have = {k for k, v in entries.items() if k[0] and any(v)}
        missing = wanted - have
        obsolete = {k for k in entries if k[0] and k[0] != "translator-credits"} - wanted
        status = "ok" if not missing else f"{len(missing)} missing"
        print(f"{po.stem:6} {len(have & wanted):3}/{len(wanted)} {status}"
              + (f", {len(obsolete)} obsolete" if obsolete else ""))
        for m in sorted(missing):
            print("   missing:", m[0])
        bad += len(missing)
    return 1 if bad else 0


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "compile"
    if cmd == "extract":
        write_pot(extract())
    elif cmd == "update":
        update()
    elif cmd == "compile":
        compile_all(Path(sys.argv[2]).resolve() if len(sys.argv) > 2 else LOCALE_DIR)
    elif cmd == "check":
        sys.exit(check())
    else:
        sys.exit(__doc__)
