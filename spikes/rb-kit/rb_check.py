"""rekordbox behavior check: ROADMAP §5.2 behaviors plus gate tests T1 and T3 (§7).

Re-run it after every rekordbox update, before trusting sends again. The owner's
step list is CHECK.md. Standard library only.

  python rb_check.py build <music folder> [--kit DIR] [--force]
      Copies 12 tracks from the folder into the kit (the originals are only read),
      renames the copies to awkward names, generates 2 test tones (F# major and
      Bb major, for rekordbox's Classic key spellings) and writes 1-first.xml.
  python rb_check.py next [--kit DIR]
      Reads exports/02-before-resend.xml and writes 2-resend.xml, 3-t1a-playlists-only.xml
      and 4-t1b-mixed.xml from rekordbox's current values (as the app would, ROADMAP 1.9).
  python rb_check.py analyze [--kit DIR]
      Reads the exports and prints PASS / FAIL / INCONCLUSIVE per behavior.
"""

import argparse
import datetime
import json
import os
import re
import shutil
import sys
from dataclasses import dataclass
from pathlib import Path

import rbxml
import tones
from rbxml import folder, playlist

DEFAULT_KIT = Path.home() / "tlp-rb-check"
EXTS = (".mp3", ".flac", ".wav", ".aiff", ".aif", ".m4a")
MIN_SIZE = 500_000  # skip stubs and tiny samples: every track must analyze to a BPM and key
TAG = "TLPCHK"

FIRST_XML, RESEND_XML, T1A_XML, T1B_XML = "1-first.xml", "2-resend.xml", "3-t1a-playlists-only.xml", "4-t1b-mixed.xml"
EXPORTS = ("01-after-first.xml", "02-before-resend.xml", "03-after-resend.xml", "04-after-t1a.xml", "05-after-t1b.xml")

# label, copy's path under the kit (the source's extension is added), title, what it's for.
# The awkward characters in the names are the point: every one must survive both directions.
TRACKS = [
    ("01", "tracks/01 Edit Me - Paper Boats [Kit & Kin's]", "Edit me", "re-import overwrites a rekordbox edit; control for Rating and PlayCount"),
    ("02", "tracks/02 Rating - Crème Brûlée Déjà", "Rating", "omitted Rating"),
    ("03", "tracks/03 Plays - Low Tide #1 (100% Flip)", "Plays", 'PlayCount="0"'),
    ("04", "tracks/04 T3 Both - Glass Harbor (A + B Remix)", "T3 both", "T3: Tonality and AverageBpm omitted; hot cue"),
    ("05", "tracks/05 T3 Key - Quiet Rain 日本語", "T3 key", "T3: Tonality omitted"),
    ("06", "tracks/06 T3 BPM - North Wind, Late Train 🚀", "T3 BPM", "T3: AverageBpm omitted"),
    ("07", "tracks/Sub Folder/Deeper Level/07 T3 Control", "T3 control", "T3 control: both sent"),
    ("08", "tracks/08 T3 Sent Values", "T3 sent values", "made-up BPM and key sent (info)"),
    ("09", "tracks/09 Left Out", "Left out", "left out of the resend; referenced by T1"),
    ("10", "tracks/10 New In T1b", "New in T1b", "new track in the T1b send"),
    ("D1", "drag-me/11 Dragged - Ça #2 & Co's (A+B, Mix) 🎧", None, "added by rekordbox itself; resend and T1"),
    ("D2", "drag-me/12 Dragged - Plain", None, "added by rekordbox itself; T1"),
    ("K2B", "tracks/13 Key Tone - Harbour Lights (F# Major)", "Key 2B", "Classic spelling of 2B, F# major"),
    ("K6B", "tracks/14 Key Tone - Bell Tower (Bb Major)", "Key 6B", "Classic spelling of 6B, Bb major"),
]
LABELS = [t[0] for t in TRACKS]
# Generated test tones, not copies: label -> (Camelot key, the spelling the app's rekordbox key names
# expect (REKORDBOX in src-tauri/src/tags/key.rs), tonic pitch class with C = 0).
KEY_TONES = {"K2B": ("2B", "F#", 6), "K6B": ("6B", "Bb", 10)}
# The resend swaps the tones' keys, in Camelot, so converting, keeping the string and ignoring it all look different.
RESEND_KEYS = {"K2B": "6B", "K6B": "2B"}
FIRST = ["01", "02", "03", "04", "05", "06", "07", "08", "09", "K2B", "K6B"]
RESEND = ["01", "02", "03", "04", "05", "06", "07", "08", "D1", "K2B", "K6B"]
T3_TRACKS = {"04": ("Tonality", "AverageBpm"), "05": ("Tonality",), "06": ("AverageBpm",), "07": ()}
FAKE_BPM = "123.45"
TONE_SECONDS = 48

# Playlists, by label. Orders are deliberately unsorted.
FIRST_PLAYLISTS = {("TLP Check", "pl-replace"): ["01", "02", "03"],
                   ("TLP Check", "pl-left"): ["04", "05"],
                   ("TLP Check", "nested", "pl-deep"): ["06", "01"]}
RESEND_PLAYLISTS = {("TLP Check", "pl-replace"): ["03", "01"],
                    ("TLP Check", "nested", "pl-deep"): ["06", "01"]}
LEFT = ("TLP Check", "pl-left")
AFTER_RESEND_PLAYLISTS = {LEFT: FIRST_PLAYLISTS[LEFT], **RESEND_PLAYLISTS}
T1_PLAYLISTS = {  # folder, playlist, KeyType, labels
    "T1a": [("TLP T1a", "by-id", "0", ["D1", "09", "D2"]),
            ("TLP T1a", "by-location", "1", ["D2", "09", "D1"])],
    "T1b": [("TLP T1b", "mixed-by-id", "0", ["D2", "10", "09"]),
            ("TLP T1b", "mixed-by-location", "1", ["10", "D1"])],
}
T1B_NEW_ID = "1"

PASS, FAIL, INCON, NOTRUN, INFO = "PASS", "FAIL", "INCONCLUSIVE", "NOT RUN", "INFO"


class CheckError(Exception):
    """A problem the owner can fix; printed without a traceback."""


# ---------------------------------------------------------------- the kit

@dataclass
class Kit:
    root: Path
    files: dict  # label -> path relative to the kit

    @classmethod
    def load(cls, root: Path):
        meta = root / "kit.json"
        if not meta.exists():
            raise CheckError(f"No kit at {root}. Build it first: python rb_check.py build <music folder>")
        data = json.loads(meta.read_text(encoding="utf-8"))
        return cls(root, {k: v["file"] for k, v in data["tracks"].items()})

    def path(self, label) -> Path:
        return self.root / self.files[label]

    def key(self, label) -> str:
        return rbxml.path_key(self.path(label))

    def keys(self, labels=LABELS) -> dict:
        return {lab: self.key(lab) for lab in labels}

    def location(self, label) -> str:
        return rbxml.encode_location(self.path(label))


def title(label, version="v1"):
    return f"{TAG} {label} {next(t[2] for t in TRACKS if t[0] == label)} [{version}]"


def first_attrs(kit: Kit, label, track_id, date):
    """Every attribute the first import sends. TotalTime, BitRate and SampleRate are left
    for rekordbox to read from the file (it recomputes them anyway, E1 session)."""
    p = kit.path(label)
    a = dict(TrackID=track_id, Name=title(label), Artist="TLP Check", Album="TLP Check", Genre="TLP Genre",
             Kind=rbxml.KIND[p.suffix.lower()], Size=str(p.stat().st_size), Year="1999",
             TrackNumber=str(LABELS.index(label) + 1), DateAdded=date, Comments=f"{TAG} v1 {label}",
             PlayCount="0", Rating="0")
    if label == "01":
        a.update(Rating="153", PlayCount="5")
    if label == "02":
        a.update(Rating="204", Colour="0xFF0000")
    if label == "03":
        a.update(PlayCount="5")
    if label == "08":
        a.update(AverageBpm=FAKE_BPM, Tonality="1A")
    a["Location"] = kit.location(label)
    return a


def _is_placeholder(path: Path) -> bool:
    """OneDrive online-only files download when read (§5.5); skip them."""
    attrs = getattr(os.stat(path), "st_file_attributes", 0)
    return bool(attrs & (0x400000 | 0x40000 | 0x1000))  # RECALL_ON_DATA_ACCESS, RECALL_ON_OPEN, OFFLINE


def pick_sources(source: Path, count: int):
    """Picks `count` audio files, deterministically, mixing formats where the folder has several."""
    by_ext = {}
    for p in sorted(source.rglob("*"), key=lambda p: str(p.relative_to(source)).casefold()):
        if (p.is_file() and p.suffix.lower() in EXTS and not p.name.startswith("._")
                and p.stat().st_size >= MIN_SIZE and not _is_placeholder(p)):
            by_ext.setdefault(p.suffix.lower(), []).append(p)
    picked, queues = [], [list(v) for v in by_ext.values()]
    while len(picked) < count and any(queues):
        for q in queues:
            if q and len(picked) < count:
                picked.append(q.pop(0))
    if len(picked) < count:
        raise CheckError(f"{source} has {len(picked)} usable audio files; the check needs {count} "
                         f"({', '.join(EXTS)}, at least {MIN_SIZE // 1000} KB, not online-only).")
    return picked


def _inside(child: Path, parent: Path) -> bool:
    try:
        child.relative_to(parent)
        return True
    except ValueError:
        return False


def build(source: Path, kit_root: Path, force=False, today=None):
    source, kit_root = source.resolve(), kit_root.resolve()
    if not source.is_dir():
        raise CheckError(f"Not a folder: {source}")
    if _inside(kit_root, source) or _inside(source, kit_root):
        raise CheckError("The kit folder and the music folder must be separate; the music folder is only read.")
    if kit_root.exists():
        if not (kit_root / "kit.json").exists():
            raise CheckError(f"{kit_root} exists and isn't a kit; pick another --kit folder.")
        if not force:
            raise CheckError(f"{kit_root} already holds a kit. Pass --force to rebuild it "
                             "(restore rekordbox's backup first, see CHECK.md).")
        shutil.rmtree(kit_root)
    date = (today or datetime.date.today()).isoformat()
    copies = [t for t in TRACKS if t[0] not in KEY_TONES]
    sources = pick_sources(source, len(copies))
    files = {}
    for (label, rel, *_), src in zip(copies, sources):
        dst = kit_root / (rel + src.suffix.lower())
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, dst)  # a new file; the original is only read
        files[label] = {"file": dst.relative_to(kit_root).as_posix(), "source": src.relative_to(source).as_posix()}
    for label, rel, *_ in TRACKS:
        if label in KEY_TONES:
            dst = kit_root / (rel + ".wav")
            tones.write_tone(dst, KEY_TONES[label][2], seconds=TONE_SECONDS)
            files[label] = {"file": dst.relative_to(kit_root).as_posix(), "source": f"tone {KEY_TONES[label][0]}"}
    files = {lab: files[lab] for lab in LABELS}
    (kit_root / "exports").mkdir()
    (kit_root / "kit.json").write_text(json.dumps({"built": date, "tracks": files}, indent=2, ensure_ascii=False),
                                       encoding="utf-8")
    kit = Kit.load(kit_root)
    tracks = [first_attrs(kit, lab, str(i + 1), date) for i, lab in enumerate(FIRST)]
    ids = {lab: str(i + 1) for i, lab in enumerate(FIRST)}
    rbxml.write(kit_root / FIRST_XML, tracks, _tree(FIRST_PLAYLISTS, lambda lab: ids[lab]))
    return kit


def _tree(playlists: dict, key_of, key_type="0"):
    """{(folder, ..., name): [labels]} -> nested playlist nodes, keeping insertion order."""
    root = []

    def child(nodes, name):
        for n in nodes:
            if n[0] == "folder" and n[1] == name:
                return n[2]
        nodes.append(folder(name))
        return nodes[-1][2]

    for path, labels in playlists.items():
        nodes = root
        for name in path[:-1]:
            nodes = child(nodes, name)
        nodes.append(playlist(path[-1], [key_of(lab) for lab in labels], key_type))
    return root


# ---------------------------------------------------------------- next: the sends after the first import

def _camelot_shift(key: str) -> str:
    m = re.fullmatch(r"(\d{1,2})([AB])", key.strip())
    if m:
        return f"{(int(m.group(1)) + 5) % 12 + 1}{m.group(2)}"
    return "1A" if key != "1A" else "2A"


_CAMELOT = re.compile(r"(\d{1,2})\s*([AB])", re.IGNORECASE)
_CLASSIC = re.compile(r"([A-G])\s*(#|b|♯|♭)?\s*(m|min|minor|maj|major)?", re.IGNORECASE)
_PITCH = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}


def is_camelot(key: str) -> bool:
    return bool(_CAMELOT.fullmatch((key or "").strip()))


def camelot_of(key: str):
    """'F#' -> '2B', 'Bbm' -> '3A', '2B' -> '2B'. None if it isn't a key we can read."""
    key = (key or "").strip()
    m = _CAMELOT.fullmatch(key)
    if m:
        return f"{int(m.group(1))}{m.group(2).upper()}" if 1 <= int(m.group(1)) <= 12 else None
    m = _CLASSIC.fullmatch(key)
    if not m or not m.group(1).isupper():
        return None
    pc = _PITCH[m.group(1)] + {"#": 1, "♯": 1, "b": -1, "♭": -1}.get(m.group(2) or "", 0)
    minor = (m.group(3) or "").lower() in ("m", "min", "minor") and m.group(3) != "M"
    major_pc = (pc + 3) % 12 if minor else pc % 12  # a minor key sits with its relative major
    return f"{(major_pc * 7 + 7) % 12 + 1}{'A' if minor else 'B'}"


def _bpm(track) -> float:
    try:
        return float(track.attrs.get("AverageBpm") or 0)
    except ValueError:
        return 0.0


def prepare_next(kit: Kit, export: Path):
    """Validates the owner's export 02 and writes the three later sends. Returns warnings."""
    if not export.exists():
        raise CheckError(f"Missing {export.name}. Export the collection to {export} first (CHECK.md step 2).")
    lib = rbxml.parse(export)
    first = rbxml.parse(kit.root / FIRST_XML)
    t = {}
    problems, warnings = [], []
    for lab in FIRST + ["D1", "D2"]:
        found = lib.by_key.get(kit.key(lab), [])
        if len(found) != 1:
            problems.append(f"track {lab} ({kit.files[lab]}) is in the export {len(found)} times, not once")
            continue
        t[lab] = found[0]
        if _bpm(found[0]) <= 0 or not found[0].attrs.get("Tonality"):
            problems.append(f"track {lab} has no BPM or key yet: wait for analysis to finish, then export again")
    if not problems:
        if t["01"].attrs.get("Name") == first.one(kit.key("01")).attrs["Name"]:
            problems.append(f"track 01 still has the title '{title('01')}': do step 2.1, then export again")
        if not t["04"].marks:
            problems.append("track 04 has no hot cue: do step 2.2, then export again")
        if t["03"].attrs.get("PlayCount") in ("0", None):
            warnings.append("rekordbox didn't take PlayCount 5 from the first XML, so the PlayCount check "
                            "will be inconclusive")
        camelot = [f"{lab} = {t[lab].attrs['Tonality']!r}" for lab in KEY_TONES if is_camelot(t[lab].attrs["Tonality"])]
        if camelot:
            warnings.append(f"rekordbox wrote the test tones' keys in Camelot ({', '.join(camelot)}), so the key "
                            "display isn't Classic. Set it to Classic (step 0.5), export 02 again and rerun this; "
                            "otherwise the key spelling check can't answer")
    if problems:
        raise CheckError("Export 02 isn't ready:\n  - " + "\n  - ".join(problems))

    ids = {lab: t[lab].attrs["TrackID"] for lab in t}
    resend = []
    for lab in RESEND:
        a = dict(t[lab].attrs)  # rule 1: every attribute, rekordbox's current value
        a["Location"] = kit.location(lab)  # rule 5: fully percent-encoded
        a["Comments"] = f"{TAG} v2 {lab}"  # marks the track as re-imported ("Yes")
        if lab == "01":
            a["Name"] = title("01")  # stale on purpose: does "Yes" undo the rekordbox edit?
        elif lab == "02":
            a.pop("Rating", None)
        elif lab == "03":
            a["PlayCount"] = "0"
        elif lab in T3_TRACKS:
            for f in T3_TRACKS[lab]:
                a.pop(f, None)
        elif lab == "08":
            a.update(AverageBpm=FAKE_BPM, Tonality=_camelot_shift(a.get("Tonality", "")))
        elif lab in KEY_TONES:
            a["Tonality"] = RESEND_KEYS[lab]  # does rekordbox convert a Camelot key, keep the string, or ignore it?
        resend.append(a)  # no TEMPO or POSITION_MARK for anyone (rule 3, case A)
    rbxml.write(kit.root / RESEND_XML, resend, _tree(RESEND_PLAYLISTS, lambda lab: ids[lab]))

    new_id = T1B_NEW_ID
    while new_id in lib.by_id:
        new_id = str(int(new_id) + 1)
    ids["10"] = new_id
    for name, dest, tracks in (("T1a", T1A_XML, []),
                               ("T1b", T1B_XML, [first_attrs(kit, "10", new_id, datetime.date.today().isoformat())])):
        nodes = {}
        for fold, pl, key_type, labels in T1_PLAYLISTS[name]:
            keys = [ids[lab] if key_type == "0" else kit.location(lab) for lab in labels]
            nodes.setdefault(fold, []).append(playlist(pl, keys, key_type))
        rbxml.write(kit.root / dest, tracks, [folder(f, *children) for f, children in nodes.items()])
    return warnings


# ---------------------------------------------------------------- checks
# Each takes parsed exports and path keys, so they also run against other sessions' exports.

@dataclass
class Result:
    group: str
    name: str
    status: str
    detail: str


def _names(keys: dict) -> dict:
    return {v: k for k, v in keys.items()}


def paths_resolve(lib, keys: dict):
    missing = [lab for lab, k in keys.items() if not lib.by_key.get(k)]
    dups = [lab for lab, k in keys.items() if len(lib.by_key.get(k, [])) > 1]
    if missing or dups:
        return FAIL, _join(missing and f"not found at its path: {', '.join(missing)}",
                           dups and f"duplicated: {', '.join(dups)}")
    raw = [lab for lab, k in keys.items() if _bpm(lib.by_key[k][0]) <= 0]
    if raw:
        return INCON, f"found, but not analyzed yet: {', '.join(raw)}. Export again after analysis finishes."
    return PASS, f"all {len(keys)} tracks found at their exact paths and analyzed"


VALUE_FIELDS = ("Name", "Artist", "Album", "Genre", "Comments", "Rating", "Colour", "Year", "TrackNumber")


def xml_values_win(sent, lib, keys: dict):
    diffs = []
    for lab, k in keys.items():
        s, r = sent.one(k), lib.one(k)
        if s is None or r is None:
            diffs.append(f"{lab} missing")
            continue
        for f in VALUE_FIELDS:
            if f in s.attrs and r.attrs.get(f) != s.attrs[f]:
                diffs.append(f"{lab} {f}: sent {s.attrs[f]!r}, rekordbox has {r.attrs.get(f)!r}")
    if diffs:
        return FAIL, "; ".join(diffs)
    return PASS, f"every sent value arrived ({', '.join(VALUE_FIELDS)})"


def xml_analysis_ignored(sent_t, lib_t):
    if sent_t is None or lib_t is None:
        return INCON, "track missing"
    if _bpm(lib_t) <= 0:
        return INCON, "not analyzed yet"
    got = f"{lib_t.attrs.get('AverageBpm')} BPM, key {lib_t.attrs.get('Tonality')}"
    if lib_t.attrs.get("AverageBpm") == sent_t.attrs.get("AverageBpm"):
        return FAIL, f"rekordbox kept our made-up BPM ({got})"
    return PASS, f"sent {sent_t.attrs.get('AverageBpm')} / {sent_t.attrs.get('Tonality')}, rekordbox has its own {got}"


def ids_reassigned(sent, lib, keys: dict):
    same = [lab for lab, k in keys.items()
            if sent.one(k) and lib.one(k) and sent.one(k).attrs.get("TrackID") == lib.one(k).attrs.get("TrackID")]
    if same:
        return FAIL, f"rekordbox kept our TrackID for {', '.join(same)}"
    return PASS, "every TrackID differs from the one we sent (so match by Location)"


def ids_stable(libs: list, keys: dict):
    changed = []
    for lab, k in keys.items():
        ids = {name: lib.one(k).attrs.get("TrackID") for name, lib in libs if lib.one(k)}
        if len(set(ids.values())) > 1:
            changed.append(f"{lab} " + " → ".join(f"{v} ({n[:2]})" for n, v in ids.items()))
    if changed:
        return FAIL, "; ".join(changed)
    return PASS, f"same TrackIDs across {len(libs)} exports"


def no_duplicates(libs: list, keys: dict):
    dups = [f"{lab} in {name[:2]}" for name, lib in libs for lab, k in keys.items() if len(lib.by_key.get(k, [])) > 1]
    if dups:
        return FAIL, "duplicate collection entries: " + ", ".join(dups)
    return PASS, f"every kit track appears at most once in each of {len(libs)} exports"


def playlists_match(lib, expected: dict, keys: dict, exact_folders=(), allowed=None):
    """expected: {playlist path: [labels]}. exact_folders: folders that may hold nothing but the
    playlists in `allowed` (default: expected), which catches a 'pl-replace (2)' or a merged copy."""
    names = _names(keys)
    problems = []
    for fpath in {p[:i] for p in expected for i in range(1, len(p))}:
        if lib.folders.get(fpath, 0) != 1:
            problems.append(f"folder {'/'.join(fpath)} appears {lib.folders.get(fpath, 0)} times")
    for path, labels in expected.items():
        found = lib.playlists.get(path, [])
        if len(found) != 1:
            problems.append(f"{'/'.join(path)} appears {len(found)} times")
            continue
        got = [names.get(e, "?") for e in found[0].entries]
        if got != labels:
            problems.append(f"{'/'.join(path)} is [{', '.join(got)}], expected [{', '.join(labels)}]")
    for fpath in exact_folders:
        want = {p[len(fpath)] for p in _all_expected_paths(allowed or expected)
                if p[:len(fpath)] == fpath and len(p) > len(fpath)}
        extra = [n for n in lib.root_order.get(fpath, []) if n not in want]
        if extra:
            problems.append(f"unexpected in {'/'.join(fpath)}: {', '.join(extra)}")
    if problems:
        return FAIL, "; ".join(problems)
    return PASS, "; ".join(f"{'/'.join(p)} = [{', '.join(v)}]" for p, v in expected.items())


def _all_expected_paths(expected):
    return list(expected) + [p[:i] for p in expected for i in range(1, len(p))]


def overwritten(before, after, field, sent_value):
    """A rekordbox-side edit, re-imported with the old value and "Yes"."""
    if before is None or after is None:
        return INCON, "track missing"
    if before.attrs.get(field) == sent_value:
        return INCON, f"the rekordbox edit wasn't in the export before the resend ({field} = {sent_value!r})"
    if after.attrs.get(field) == sent_value:
        return PASS, f"{field} {before.attrs.get(field)!r} (rekordbox edit) → {sent_value!r} (the XML's)"
    if after.attrs.get(field) == before.attrs.get(field):
        return FAIL, f"the rekordbox edit survived ({field} = {before.attrs.get(field)!r})"
    return FAIL, f"{field} became {after.attrs.get(field)!r}"


def reset_by_send(before, after, field, control=None, expect="0"):
    """A field that ends up `expect` because the XML omitted it or sent a default."""
    if before is None or after is None:
        return INCON, "track missing"
    if control is not None and None in control:
        return INCON, "the control track is missing"
    if before.attrs.get(field, "0") in ("0", ""):
        return INCON, f"{field} was already 0 before the resend, so there was nothing to lose"
    if control is not None and control[0].attrs.get(field) != control[1].attrs.get(field):
        return INCON, (f"the control's {field} changed too ({control[0].attrs.get(field)} → "
                       f"{control[1].attrs.get(field)}), so something else is going on")
    got = after.attrs.get(field)
    ctl = f"; the control sent its value and kept {control[1].attrs.get(field)}" if control else ""
    if got == expect:
        return PASS, f"{field} {before.attrs.get(field)} → {got}{ctl}"
    return FAIL, f"{field} {before.attrs.get(field)} → {got} (rekordbox no longer resets it){ctl}"


def still_there(before, after):
    if before is None:
        return INCON, "track missing before"
    if after is None:
        return FAIL, "the track left the collection"
    return PASS, "still in the collection" + ("" if before.attrs == after.attrs else " (some fields changed)")


def analysis_kept(before, after, omitted):
    """T3: compares key, BPM, beatgrid and cues before and after a re-import."""
    if before is None or after is None:
        return INCON, "track missing"
    if _bpm(before) <= 0 or not before.attrs.get("Tonality"):
        return INCON, "no BPM or key before the resend"
    lost = []
    for f in ("Tonality", "AverageBpm"):
        if after.attrs.get(f) != before.attrs.get(f):
            lost.append(f"{f} {before.attrs.get(f)!r} → {after.attrs.get(f)!r}")
    if after.tempos != before.tempos:
        lost.append(f"beatgrid {len(before.tempos)} → {len(after.tempos)} TEMPO entries"
                    + (" (changed)" if len(before.tempos) == len(after.tempos) else ""))
    if after.marks != before.marks:
        lost.append(f"cues {len(before.marks)} → {len(after.marks)}")
    sent = "omitted " + " and ".join(omitted) if omitted else "sent both"
    if lost:
        return FAIL, f"{sent}: " + "; ".join(lost)
    kept = f"key {after.attrs.get('Tonality')}, {after.attrs.get('AverageBpm')} BPM, grid"
    return PASS, f"{sent}: kept {kept}" + (f", {len(after.marks)} cue(s)" if after.marks else "")


def t1_playlist(before, after, path, labels, keys: dict, new_labels=()):
    """T1: a playlist whose tracks are (mostly) absent from the XML's COLLECTION."""
    names = _names(keys)
    if after.folders.get(path[:-1], 0) == 0:
        return INCON, (f"folder {'/'.join(path[:-1])} isn't in the export. If you imported it, rekordbox "
                       "refused it: note what you saw")
    found = after.playlists.get(path, [])
    if len(found) != 1:
        return FAIL, f"{'/'.join(path)} appears {len(found)} times"
    got = [names.get(e, "?") if e else "unresolved" for e in found[0].entries]
    problems = []
    if got != labels:
        problems.append(f"holds [{', '.join(got)}], expected [{', '.join(labels)}]")
    for lab in labels:
        k = keys[lab]
        if len(after.by_key.get(k, [])) > 1:
            problems.append(f"{lab} duplicated")
        elif lab not in new_labels and before.one(k) and after.one(k):
            b, a = before.one(k), after.one(k)
            changed = [f for f in set(b.attrs) | set(a.attrs) if b.attrs.get(f) != a.attrs.get(f)]
            if b.tempos != a.tempos:
                changed.append("beatgrid")
            if b.marks != a.marks:
                changed.append("cues")
            if changed:
                problems.append(f"{lab} changed: {', '.join(sorted(changed))}")
    added = [names.get(k, k) for k in set(after.by_key) - set(before.by_key)
             if k in names and names[k] not in new_labels]
    if added:
        problems.append(f"new collection entries: {', '.join(added)}")
    if problems:
        return FAIL, "; ".join(problems)
    return PASS, f"[{', '.join(got)}], existing tracks untouched"


# Found only after step 4, so re-exporting 03 then would capture T1's changes and spoil the T1 baseline.
STEP_34_SKIPPED = (
    "No. In export 03, TLP Check/pl-replace still has its old order, so step 3.4 (Import Playlist) was skipped, "
    "or rekordbox ignored it. The playlist and T1 results from this run can't be judged. Redo the run with "
    "step 3.4: restore the backup (step 5.3), rebuild the kit with --force, and start again from step 0. "
    "If you did step 3.4 and still see this, rekordbox ignored the import: tell Claude.")
STEP_34_T1 = "step 3.4 was skipped, so export 03 isn't a clean starting point for T1 (see above)"


def playlist_import_done(before, after, path=("TLP Check", "pl-replace")):
    """Step 3.4. The resend reorders pl-replace, so an unchanged order means Import Playlist didn't run."""
    b, a = before.playlist(path), after.playlist(path)
    if b is None or a is None:
        return INCON, f"{'/'.join(path)} isn't in exports 02 and 03 exactly once, so this can't tell"
    if a.entries == b.entries:
        return FAIL, STEP_34_SKIPPED
    return PASS, f"{'/'.join(path)} changed after the resend, so Import Playlist ran"


def key_spelling(track, camelot, expected):
    """How rekordbox's own analysis spells a test tone's key, with the key display on Classic."""
    if track is None:
        return INCON, "track missing"
    got = (track.attrs.get("Tonality") or "").strip()
    if not got:
        return INCON, "no key yet: it wasn't analyzed"
    if is_camelot(got):
        return INCON, (f"rekordbox wrote {got!r}, a Camelot key, so the key display wasn't Classic. "
                       "Next time, set it in step 0.5")
    heard = camelot_of(got)
    if heard is None:
        return INCON, f"rekordbox wrote {got!r}, which isn't a key spelling this check knows: note it"
    if heard != camelot:
        return INCON, f"rekordbox heard {got} ({heard}), not {camelot}, so this tone can't settle the spelling"
    if got == expected:
        return PASS, f"rekordbox writes {got!r} for {camelot}, as the app's rekordbox key names say"
    return FAIL, (f"rekordbox writes {got!r} for {camelot}, but the app's rekordbox key names say "
                  f"{expected!r}: change REKORDBOX in src-tauri/src/tags/key.rs to match")


def key_loaded(sent, before, after):
    """A Camelot Tonality sent on a Yes re-import: did rekordbox convert it, keep the string, or ignore it?"""
    s, b, a = (t.attrs.get("Tonality", "") for t in (sent, before, after))
    if b == s:
        what = "it already had that value, so this can't tell"
    elif a == b:
        what = "rekordbox ignored it"
    elif a == s:
        what = "rekordbox kept the string as sent"
    elif camelot_of(a) == camelot_of(s):
        what = f"rekordbox converted it to {a!r}, its Classic spelling of {camelot_of(s)}"
    else:
        what = f"rekordbox changed it to {a!r}, which isn't the key sent"
    return f"sent {s!r}; before {b!r}, after {a!r}: {what}"


def key_spellings(lib, keys: dict):
    """Every key spelling rekordbox wrote for the kit's tracks, in Camelot order."""
    seen = {}
    for k in keys.values():
        t = lib.one(k)
        got = (t.attrs.get("Tonality") or "").strip() if t else ""
        if got:
            seen[got] = camelot_of(got) or "?"
    order = lambda item: (item[1][-1:], int(item[1][:-1]) if item[1][:-1].isdigit() else 99, item[0])
    return ", ".join(f"{v} ({c})" for v, c in sorted(seen.items(), key=order)) or "none"


def _join(*parts):
    return "; ".join(p for p in parts if p)


# ---------------------------------------------------------------- analyze

def analyze(kit: Kit, exports_dir: Path = None):
    ex = exports_dir or kit.root / "exports"
    load = {name: rbxml.parse(ex / name) if (ex / name).exists() else None for name in EXPORTS}
    e1, e2, e3, e4, e5 = (load[n] for n in EXPORTS)
    sent = {n: rbxml.parse(kit.root / n) if (kit.root / n).exists() else None
            for n in (FIRST_XML, RESEND_XML, T1A_XML, T1B_XML)}
    s1, s2 = sent[FIRST_XML], sent[RESEND_XML]
    keys = kit.keys()
    k = keys.__getitem__
    out = []

    def add(group, name, needs, fn):
        missing = [n for n, v in needs.items() if v is None]
        if missing:
            out.append(Result(group, name, NOTRUN, f"needs {', '.join(missing)}"))
        else:
            out.append(Result(group, name, *fn()))

    g = "§5.2 first import"
    first_keys = {lab: k(lab) for lab in FIRST}
    add(g, "Awkward paths resolve", {EXPORTS[0]: e1},
        lambda: paths_resolve(e1, {lab: k(lab) for lab in FIRST + ["D1", "D2"]}))
    add(g, "XML values win on first import", {EXPORTS[0]: e1}, lambda: xml_values_win(s1, e1, first_keys))
    add(g, "XML BPM/key ignored when rekordbox analyzes", {EXPORTS[0]: e1},
        lambda: xml_analysis_ignored(s1.one(k("08")), e1.one(k("08"))))
    add(g, "TrackIDs reassigned on import", {EXPORTS[0]: e1}, lambda: ids_reassigned(s1, e1, first_keys))
    add(g, "Nested folders and playlist order survive", {EXPORTS[0]: e1},
        lambda: playlists_match(e1, FIRST_PLAYLISTS, keys, exact_folders=[("TLP Check",)]))
    present = [(n, lib) for n, lib in load.items() if lib is not None]
    if len(present) >= 2:
        add(g, "TrackIDs stable between exports", {}, lambda: ids_stable(present, keys))
    if present:
        add(g, "No duplicate tracks, ever", {}, lambda: no_duplicates(present, keys))
    if e1:
        out.append(Result("Info", "PlayCount taken from the XML on first import", INFO,
                          ", ".join(f"{lab}: sent 5, rekordbox has {e1.one(k(lab)).attrs.get('PlayCount')}"
                                    for lab in ("01", "03") if e1.one(k(lab)))))

    g = "§5.2 re-import"
    need = {EXPORTS[1]: e2, EXPORTS[2]: e3, RESEND_XML: s2}
    applied = {}
    if all(need.values()):
        for lab in RESEND:
            after = e3.one(k(lab))
            applied[lab] = bool(after and after.attrs.get("Comments") == s2.one(k(lab)).attrs.get("Comments"))
        skipped = [lab for lab in RESEND if not applied[lab]]
        out.append(Result("Info", "Tracks the resend updated", INFO,
                          f"{len(RESEND) - len(skipped)} of {len(RESEND)}"
                          + (f"; not updated: {', '.join(skipped)} (answered No?)" if skipped else "")))

    def when_applied(lab, fn):
        return fn if applied.get(lab) else (lambda: (INCON, f"the resend didn't update {lab} (answered No?)"))

    b, a = (lambda lab: e2.one(k(lab))), (lambda lab: e3.one(k(lab)))
    add(g, "Step 3.4 done (Import Playlist before export 03)", need, lambda: playlist_import_done(e2, e3))
    skipped_34 = out[-1].status == FAIL

    def unless_skipped(fn, why="step 3.4 wasn't done, so this can't be judged (see above)"):
        return (lambda: (INCON, why)) if skipped_34 else fn

    add(g, "Re-import is a full overwrite (Yes undoes a rekordbox edit)", need,
        when_applied("01", lambda: overwritten(b("01"), a("01"), "Name", s2.one(k("01")).attrs["Name"])))
    add(g, "Omitted Rating resets to 0", need,
        when_applied("02", lambda: reset_by_send(b("02"), a("02"), "Rating", control=(b("01"), a("01")))))
    add(g, 'PlayCount="0" wipes plays', need,
        when_applied("03", lambda: reset_by_send(b("03"), a("03"), "PlayCount", control=(b("01"), a("01")))))
    add(g, "Re-import matches tracks rekordbox added itself", need,
        lambda: (FAIL, "D1 duplicated") if len(e3.by_key.get(k("D1"), [])) > 1 else
        (PASS, "D1 updated in place from our fully encoded Location") if applied.get("D1") else
        (INCON, "D1 wasn't updated (answered No?)"))
    add(g, "Same-name playlists are replaced", need, unless_skipped(
        lambda: playlists_match(e3, RESEND_PLAYLISTS, keys, exact_folders=[("TLP Check",), ("TLP Check", "nested")],
                                allowed=AFTER_RESEND_PLAYLISTS)))
    add(g, "Playlists missing from the XML stay", need, unless_skipped(
        lambda: playlists_match(e3, {LEFT: FIRST_PLAYLISTS[LEFT]}, keys)))
    add(g, "Tracks missing from the XML stay", need, lambda: still_there(b("09"), a("09")))

    g = "T3 omitted analysis fields"
    t3 = []
    for lab, omitted in T3_TRACKS.items():
        add(g, f"{lab} {TRACKS[LABELS.index(lab)][3]}", need,
            when_applied(lab, lambda lab=lab, omitted=omitted: analysis_kept(b(lab), a(lab), omitted)))
        t3.append(out[-1])
    out.append(Result(g, "T3 verdict", *_t3_verdict(t3)))
    if all(need.values()):
        s8, b8, a8 = s2.one(k("08")), b("08"), a("08")
        if s8 and b8 and a8:
            took = a8.attrs.get("AverageBpm") == FAKE_BPM
            out.append(Result("Info", "Made-up BPM/key on a re-import (08)", INFO,
                              f"sent {FAKE_BPM} / {s8.attrs.get('Tonality')}; before {b8.attrs.get('AverageBpm')} / "
                              f"{b8.attrs.get('Tonality')}; after {a8.attrs.get('AverageBpm')} / "
                              f"{a8.attrs.get('Tonality')} → rekordbox {'loaded' if took else 'ignored'} them"))

    g = "Classic key spellings (test tones)"
    for lab, (camelot, expected, _) in KEY_TONES.items():
        add(g, f"{lab} {camelot} ({tones.NOTE_NAMES[KEY_TONES[lab][2]]} major) as rekordbox analyzed it",
            {EXPORTS[1]: e2}, lambda lab=lab, camelot=camelot, expected=expected:
            key_spelling(e2.one(k(lab)), camelot, expected))
    if e2:
        out.append(Result("Info", "Key spellings rekordbox wrote (export 02)", INFO, key_spellings(e2, keys)))
    if all(need.values()):
        for lab in KEY_TONES:
            if applied.get(lab) and s2.one(k(lab)) and b(lab) and a(lab):
                out.append(Result("Info", f"Camelot key sent on a re-import ({lab})", INFO,
                                  key_loaded(s2.one(k(lab)), b(lab), a(lab))))

    g = "T1 playlists referencing tracks absent from COLLECTION"
    t1 = []
    for name, before, after, before_name, after_name, new in (("T1a", e3, e4, EXPORTS[2], EXPORTS[3], ()),
                                                               ("T1b", e4, e5, EXPORTS[3], EXPORTS[4], ("10",))):
        for fold, pl, key_type, labels in T1_PLAYLISTS[name]:
            what = "TrackID" if key_type == "0" else "Location"
            add(g, f"{name} {pl} (KeyType {key_type}, {what})", {before_name: before, after_name: after},
                unless_skipped(lambda b_=before, a_=after, p=(fold, pl), labels=labels, new=new:
                               t1_playlist(b_, a_, p, labels, keys, new), STEP_34_T1))
            t1.append((name, key_type, out[-1]))
    out.append(Result(g, "T1 verdict", *_t1_verdict(t1)))
    return out, (e5 or e4 or e3 or e2 or e1)


def _t3_verdict(rows):
    statuses = [r.status for r in rows]
    if all(s == NOTRUN for s in statuses):
        return NOTRUN, "T3 exports missing"
    if FAIL in statuses[:3]:
        return FAIL, "omitting an analysis field lost data: rule 3 (A) must change"
    if statuses[3] == FAIL:
        return INCON, "the control (both fields sent) changed too: was auto-analysis still on?"
    if all(s == PASS for s in statuses):
        return PASS, "omitting Tonality / AverageBpm / TEMPO / POSITION_MARK keeps rekordbox's analysis: rule 3 (A) holds"
    return INCON, "not every T3 track could be judged; see above"


def _t1_verdict(rows):
    ok = {kt: all(r.status == PASS for _, t, r in rows if t == kt) for kt in ("0", "1")}
    if any(r.status == NOTRUN for _, _, r in rows) and not any(ok.values()):
        return NOTRUN, "T1 exports missing"
    works = [("TrackID (KeyType 0)" if kt == "0" else "Location (KeyType 1)") for kt, v in ok.items() if v]
    if works:
        return PASS, f"rule 4 works, both playlists-only and mixed, keyed by {' and by '.join(works)}"
    if any(r.status == INCON for _, _, r in rows):
        return INCON, "see the T1 rows above"
    return FAIL, "neither key type works: fall back to a full send (rule 4)"


def report(results, lib, kit: Kit) -> str:
    version = lib.product.get("Version", "?") if lib else "?"
    lines = [f"rekordbox behavior check: rekordbox {version}, kit {kit.root}",
             "PASS = rekordbox still behaves as ROADMAP §5.2 says (or the gate test's answer is yes).",
             "FAIL = it doesn't: read the detail, then update §5.2 / 1.9 before trusting sends.", ""]
    group = None
    for r in results:
        if r.group != group:
            group = r.group
            lines.append(group)
        lines.append(f"  {r.status:<12} {r.name}")
        lines.append(f"  {'':<12}   {r.detail}")
    counts = {s: sum(r.status == s for r in results) for s in (PASS, FAIL, INCON, NOTRUN)}
    lines += ["", "Summary: " + ", ".join(f"{v} {s}" for s, v in counts.items())]
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------- CLI

def main(argv=None):
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build", help="copy tracks into a new kit and write 1-first.xml")
    b.add_argument("source", type=Path, help="a music folder; it's only read")
    b.add_argument("--force", action="store_true", help="replace an existing kit")
    for name in ("next", "analyze"):
        sub.add_parser(name)
    for p in sub.choices.values():
        p.add_argument("--kit", type=Path, default=DEFAULT_KIT, help=f"kit folder (default {DEFAULT_KIT})")
    args = ap.parse_args(argv)
    try:
        if args.cmd == "build":
            kit = build(args.source, args.kit, args.force)
            print(f"Kit ready: {kit.root}")
            for lab, rel in kit.files.items():
                print(f"  {lab}  {rel}")
            print(f"Next: CHECK.md step 1. Import {kit.root / FIRST_XML}; save exports into {kit.root / 'exports'}")
        elif args.cmd == "next":
            kit = Kit.load(args.kit)
            for w in prepare_next(kit, kit.root / "exports" / EXPORTS[1]):
                print(f"Warning: {w}")
            print(f"Wrote {RESEND_XML}, {T1A_XML} and {T1B_XML} in {kit.root}. Continue with CHECK.md step 3.")
        else:
            kit = Kit.load(args.kit)
            results, lib = analyze(kit)
            text = report(results, lib, kit)
            print(text, end="")
            (kit.root / "exports" / "report.txt").write_text(text, encoding="utf-8")
            return 1 if any(r.status == FAIL for r in results) else 0
    except CheckError as e:
        print(e, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
