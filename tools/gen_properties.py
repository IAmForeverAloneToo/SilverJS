"""Generates what RegExp's \\p{...} matches (src/regexp/property_tables.rs)
from the Unicode Character Database: every name it accepts and the
characters (and, for the properties of strings, the strings) of each.

    python -I tools/gen_properties.py <ucd dir> <emoji dir> src/regexp/property_tables.rs

<ucd dir> holds UCD_VERSION's files (https://www.unicode.org/Public/<version>/ucd/):
PropertyValueAliases.txt, extracted/DerivedGeneralCategory.txt,
Scripts.txt, ScriptExtensions.txt, PropList.txt, DerivedCoreProperties.txt,
DerivedNormalizationProps.txt, extracted/DerivedBinaryProperties.txt and
emoji/emoji-data.txt; <emoji dir> holds the same version's
emoji-sequences.txt and emoji-zwj-sequences.txt
(https://www.unicode.org/Public/<version>/emoji/).
That version is the one the reference browser ships and every generated
table uses (Chrome 158: Unicode 17.0.0, the J4.4d probe); each file must
say it is of that version, and the output records the version and each
file's SHA-256. General_Category and Script values come from the UCD,
every alias of each; the binary properties and the properties of
strings are ECMA-262's own lists (section 22.2.2.9's tables). The output
is checked in; the UCD files are not. Run this again with the UCD's next
version, in a commit of its own.
"""

import hashlib
import os
import re
import sys

UCD_VERSION = "17.0.0"
EMOJI_VERSION = "17.0"

# ECMA-262's table of binary Unicode property aliases: canonical name
# and short alias.
BINARY = [
    ("ASCII", None), ("ASCII_Hex_Digit", "AHex"), ("Alphabetic", "Alpha"),
    ("Any", None), ("Assigned", None), ("Bidi_Control", "Bidi_C"),
    ("Bidi_Mirrored", "Bidi_M"), ("Case_Ignorable", "CI"), ("Cased", None),
    ("Changes_When_Casefolded", "CWCF"), ("Changes_When_Casemapped", "CWCM"),
    ("Changes_When_Lowercased", "CWL"), ("Changes_When_NFKC_Casefolded", "CWKCF"),
    ("Changes_When_Titlecased", "CWT"), ("Changes_When_Uppercased", "CWU"),
    ("Dash", None), ("Default_Ignorable_Code_Point", "DI"), ("Deprecated", "Dep"),
    ("Diacritic", "Dia"), ("Emoji", None), ("Emoji_Component", "EComp"),
    ("Emoji_Modifier", "EMod"), ("Emoji_Modifier_Base", "EBase"),
    ("Emoji_Presentation", "EPres"), ("Extended_Pictographic", "ExtPict"),
    ("Extender", "Ext"), ("Grapheme_Base", "Gr_Base"), ("Grapheme_Extend", "Gr_Ext"),
    ("Hex_Digit", "Hex"), ("IDS_Binary_Operator", "IDSB"),
    ("IDS_Trinary_Operator", "IDST"), ("ID_Continue", "IDC"), ("ID_Start", "IDS"),
    ("Ideographic", "Ideo"), ("Join_Control", "Join_C"),
    ("Logical_Order_Exception", "LOE"), ("Lowercase", "Lower"), ("Math", None),
    ("Noncharacter_Code_Point", "NChar"), ("Pattern_Syntax", "Pat_Syn"),
    ("Pattern_White_Space", "Pat_WS"), ("Quotation_Mark", "QMark"), ("Radical", None),
    ("Regional_Indicator", "RI"), ("Sentence_Terminal", "STerm"),
    ("Soft_Dotted", "SD"), ("Terminal_Punctuation", "Term"),
    ("Unified_Ideograph", "UIdeo"), ("Uppercase", "Upper"),
    ("Variation_Selector", "VS"), ("White_Space", "space"),
    ("XID_Continue", "XIDC"), ("XID_Start", "XIDS"),
]

# ECMA-262's table of binary Unicode properties of strings (`v` only).
STRINGS = [
    "Basic_Emoji", "Emoji_Keycap_Sequence", "RGI_Emoji_Modifier_Sequence",
    "RGI_Emoji_Flag_Sequence", "RGI_Emoji_Tag_Sequence",
    "RGI_Emoji_ZWJ_Sequence", "RGI_Emoji",
]

# General_Category's groups (UAX #44, section 5.7.1), by short name.
GC_GROUPS = {
    "C": ["Cc", "Cf", "Cn", "Co", "Cs"],
    "L": ["Ll", "Lm", "Lo", "Lt", "Lu"],
    "LC": ["Ll", "Lt", "Lu"],
    "M": ["Mc", "Me", "Mn"],
    "N": ["Nd", "Nl", "No"],
    "P": ["Pc", "Pd", "Pe", "Pf", "Pi", "Po", "Ps"],
    "S": ["Sc", "Sk", "Sm", "So"],
    "Z": ["Zl", "Zp", "Zs"],
}

MAX = 0x10FFFF
inputs = []


def read(path, version):
    """The file's lines, after checking it is of `version`; its digest is
    recorded."""
    with open(path, "rb") as f:
        data = f.read()
    text = data.decode("utf-8")
    head = "\n".join(text.splitlines()[:12])
    if version not in head:
        sys.exit(f"{path} does not say it is of {version}")
    inputs.append((hashlib.sha256(data).hexdigest(), path))
    return text.splitlines()


def fields(lines):
    """Each data line's fields, comments and blanks dropped."""
    for line in lines:
        line = line.split("#", 1)[0].strip()
        if line:
            yield [x.strip() for x in line.split(";")]


def span(text):
    """`0041..005A` or `00AA` as a range."""
    if ".." in text:
        a, b = text.split("..")
        return int(a, 16), int(b, 16)
    return int(text, 16), int(text, 16)


def merged(ranges):
    out = []
    for lo, hi in sorted(ranges):
        if out and lo <= out[-1][1] + 1:
            out[-1] = (out[-1][0], max(out[-1][1], hi))
        else:
            out.append((lo, hi))
    return out


def complement(ranges):
    out, at = [], 0
    for lo, hi in merged(ranges):
        if lo > at:
            out.append((at, lo - 1))
        at = hi + 1
    if at <= MAX:
        out.append((at, MAX))
    return out


def by_value(lines, column=1):
    """The ranges of each value of a `range ; value` file."""
    out = {}
    for f in fields(lines):
        out.setdefault(f[column], []).append(span(f[0]))
    return {k: merged(v) for k, v in out.items()}


def aliases(lines, prop):
    """Each value of `prop` as (short name, every alias)."""
    out = []
    for f in fields(lines):
        if f[0] == prop:
            names = [x for x in f[1:] if x and x != "n/a"]
            out.append((f[1], names))
    return out


def main():
    ucd, emoji, dst = sys.argv[1], sys.argv[2], sys.argv[3]
    p = lambda name: os.path.join(ucd, name)
    pva = read(p("PropertyValueAliases.txt"), UCD_VERSION)
    gc_values = aliases(pva, "gc")
    sc_values = aliases(pva, "sc")

    tables = {}  # a table's Rust name -> its ranges

    gc = by_value(read(p("extracted/DerivedGeneralCategory.txt"), UCD_VERSION))
    gc_map = []  # every alias -> its table
    for short, names in gc_values:
        if short in GC_GROUPS:
            ranges = merged(r for g in GC_GROUPS[short] for r in gc.get(g, []))
        else:
            ranges = gc.get(short, [])
        tables["GC_" + short] = ranges
        gc_map += [(n, "GC_" + short) for n in names]

    long_of = {names[1]: short for short, names in sc_values}
    scripts = {long_of[k]: v for k, v in by_value(read(p("Scripts.txt"), UCD_VERSION)).items()}
    # Unknown (Zzzz) is every code point Scripts.txt does not list.
    scripts["Zzzz"] = complement([r for v in scripts.values() for r in v])
    extended = {s: list(r) for s, r in scripts.items()}
    for f in fields(read(p("ScriptExtensions.txt"), UCD_VERSION)):
        lo, hi = span(f[0])
        listed = f[1].split()
        # A code point listed has exactly these scripts: it leaves its own.
        for s in extended:
            if s not in listed:
                extended[s] = complement_within(extended[s], lo, hi)
        for s in listed:
            extended.setdefault(s, []).append((lo, hi))
    sc_map = []
    for short, names in sc_values:
        tables["SC_" + short] = merged(scripts.get(short, []))
        tables["SCX_" + short] = merged(extended.get(short, []))
        sc_map += [(n, short) for n in names]

    binary = {}
    for name in ("PropList.txt", "DerivedCoreProperties.txt",
                 "DerivedNormalizationProps.txt", "extracted/DerivedBinaryProperties.txt",
                 "emoji/emoji-data.txt"):
        version = EMOJI_VERSION if name.startswith("emoji") else UCD_VERSION
        for f in fields(read(p(name), version)):
            if len(f) == 2:
                binary.setdefault(f[1], []).append(span(f[0]))
    binary["ASCII"] = [(0, 0x7F)]
    binary["Any"] = [(0, MAX)]
    binary["Assigned"] = complement(gc.get("Cn", []))
    binary_map = []
    for canon, short in BINARY:
        if canon not in binary:
            sys.exit(f"no data for the binary property {canon}")
        tables["B_" + canon] = merged(binary[canon])
        binary_map += [(canon, "B_" + canon)] + ([(short, "B_" + canon)] if short else [])

    singles, seqs = {}, {}
    for name in ("emoji-sequences.txt", "emoji-zwj-sequences.txt"):
        for f in fields(read(os.path.join(emoji, name), EMOJI_VERSION)):
            prop = f[1]
            if prop not in STRINGS:
                sys.exit(f"{name}: an unknown property {prop}")
            points = f[0].split()
            if len(points) == 1:
                singles.setdefault(prop, []).append(span(points[0]))
            else:
                seqs.setdefault(prop, []).append(tuple(int(x, 16) for x in points))
    for prop in STRINGS[:-1]:
        singles.setdefault(prop, [])
        seqs.setdefault(prop, [])
    singles["RGI_Emoji"] = [r for k in STRINGS[:-1] for r in singles[k]]
    seqs["RGI_Emoji"] = [s for k in STRINGS[:-1] for s in seqs[k]]
    for prop in STRINGS:
        tables["S_" + prop] = merged(singles[prop])

    out = [
        "// Generated by tools/gen_properties.py from the Unicode Character",
        f"// Database {UCD_VERSION}. The sequences are Unicode emoji {EMOJI_VERSION}'s, the",
        "// names of the binary properties and of the properties of strings",
        "// ECMA-262's. Do not edit: run the generator again.",
        "// Inputs (SHA-256):",
    ]
    for digest, path in inputs:
        rel = os.path.relpath(path, ucd if path.startswith(ucd) else emoji).replace(os.sep, "/")
        out.append(f"//   {digest}  {rel}")
    out.append("")
    out.append("// What `\\p{...}` matches, each table of characters as inclusive")
    out.append("// ranges, written flat: first, last, first, last, ...")
    out.append("")
    first = {}
    for name in sorted(tables):
        flat = [x for r in tables[name] for x in r]
        # A table equal to one written before (a script's extensions
        # that are its own characters, most often) names that one.
        if tuple(flat) in first:
            out.append(f"const {name.upper()}: &[u32] = {first[tuple(flat)].upper()};")
            continue
        first[tuple(flat)] = name
        out.append(f"const {name.upper()}: &[u32] = &[")
        for i in range(0, len(flat), 8):
            out.append("    " + ", ".join(f"0x{x:X}" for x in flat[i:i + 8]) + ",")
        out.append("];")
    out.append("")
    out.append("/// Every name of a General_Category value (and group), sorted.")
    out.append("pub(super) const GENERAL_CATEGORY: &[(&str, &[u32])] = &[")
    out += [f'    ("{n}", {t.upper()}),' for n, t in sorted(gc_map)]
    out.append("];")
    out.append("")
    out.append("/// Every name of a Script value with its Script and its")
    out.append("/// Script_Extensions characters, sorted.")
    out.append("pub(super) const SCRIPT: &[(&str, &[u32], &[u32])] = &[")
    out += [f'    ("{n}", SC_{s.upper()}, SCX_{s.upper()}),' for n, s in sorted(sc_map)]
    out.append("];")
    out.append("")
    out.append("/// Every name of a binary property, sorted.")
    out.append("pub(super) const BINARY: &[(&str, &[u32])] = &[")
    out += [f'    ("{n}", {t.upper()}),' for n, t in sorted(binary_map)]
    out.append("];")
    out.append("")
    out.append("/// A property of strings: its name, its single characters and its")
    out.append("/// strings of more than one.")
    out.append("pub(super) type StringProperty = (&'static str, &'static [u32], &'static [&'static [u32]]);")
    out.append("")
    out.append("/// The properties of strings, sorted.")
    out.append("pub(super) const STRINGS: &[StringProperty] = &[")
    for prop in sorted(STRINGS):
        out.append(f'    ("{prop}", S_{prop.upper()}, &[')
        for s in sorted(set(seqs[prop])):
            out.append("        &[" + ", ".join(f"0x{x:X}" for x in s) + "],")
        out.append("    ]),")
    out.append("];")
    with open(dst, "w", encoding="ascii", newline="\n") as f:
        f.write("\n".join(out) + "\n")


def complement_within(ranges, lo, hi):
    """`ranges` without `lo..=hi`."""
    out = []
    for a, b in ranges:
        if b < lo or a > hi:
            out.append((a, b))
            continue
        if a < lo:
            out.append((a, lo - 1))
        if b > hi:
            out.append((hi + 1, b))
    return out


main()
