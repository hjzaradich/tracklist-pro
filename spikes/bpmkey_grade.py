"""Grade audio-lab's BPM/key estimates against rekordbox's analysis (E1 part F).

  python bpmkey_grade.py <audio-lab.exe>   -> results/bpmkey.jsonl + summary

Ground truth: results/f-ground-truth.xml (rekordbox analysed copies of a folder of tracks,
since deleted). The same files still exist, untouched, in the source folder (SRC below);
match by filename.
"""
import json
import os
import subprocess
import sys
import xml.etree.ElementTree as ET
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from urllib.parse import unquote

HERE = Path(__file__).parent
SRC = Path(os.environ.get("TLP_SOURCE_DIR", "music-source"))  # the folder that was analysed
lab = sys.argv[1]

truth = {}
for t in ET.parse(HERE / "results/f-ground-truth.xml").getroot().find("COLLECTION"):
    loc = t.attrib["Location"]
    if "/tlp-gt/" in loc:
        name = unquote(loc.rsplit("/", 1)[1])
        truth[name] = (float(t.attrib["AverageBpm"]), t.attrib.get("Tonality", ""))


def run(name):
    out = subprocess.run([lab, "bpmkey", str(SRC / name)], capture_output=True, text=True, encoding="utf-8")
    est = json.loads(out.stdout) if out.returncode == 0 else {"error": out.stderr.strip()[-200:]}
    return {"file": name, "rb_bpm": truth[name][0], "rb_key": truth[name][1], **est}


with ThreadPoolExecutor(6) as pool:
    rows = list(pool.map(run, sorted(truth)))
with open(HERE / "results/bpmkey.jsonl", "w", encoding="utf-8") as f:
    for r in rows:
        f.write(json.dumps(r, ensure_ascii=False) + "\n")

ok = [r for r in rows if "error" not in r]
print("tracks", len(rows), "| errors", len(rows) - len(ok))


def bpm_class(r):
    d = abs(r["bpm"] - r["rb_bpm"])
    if d <= 0.5:
        return "exact (±0.5)"
    if d <= 1.5:
        return "close (±1.5)"
    for m in (2, 0.5, 1.5, 2 / 3, 4 / 3, 0.75):
        if abs(r["bpm"] * m - r["rb_bpm"]) <= 1.5:
            return "octave/ratio error"
    return "wrong"


def key_class(r):
    a, b = r["key"], r["rb_key"]
    if a == b:
        return "exact"
    try:
        na, la, nb, lb = int(a[:-1]), a[-1], int(b[:-1]), b[-1]
    except ValueError:
        return "unparsed"
    if na == nb:
        return "relative (same number)"
    if la == lb and (na - nb) % 12 in (1, 11):
        return "fifth (±1)"
    return "wrong"


print("BPM:", dict(Counter(bpm_class(r) for r in ok)))
print("Key:", dict(Counter(key_class(r) for r in ok)))
print("avg ms per track (150 s decoded, analysis only):", round(sum(r["ms"] for r in ok) / len(ok)))
print("worst BPM:", [(r["file"][:40], r["rb_bpm"], round(r["bpm"], 2)) for r in ok if bpm_class(r) == "wrong"][:8])
