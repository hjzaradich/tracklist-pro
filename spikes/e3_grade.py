"""Fingerprint every E3 pair with audio-lab and grade against the user's labels.

  python e3_grade.py <audio-lab.exe>     -> results/e3-fingerprints.jsonl + a summary table
"""
import csv
import json
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).parent
lab = sys.argv[1]
pairs = {r["id"]: r for r in csv.DictReader(open(HERE / "e3/pairs.csv", encoding="utf-8"))}
labels = {r["id"]: r for r in csv.DictReader(open(HERE / "results/e3-labels.csv", encoding="utf-8"))}
CUTS = {"16", "20", "38"}  # cuts per the user's cut/rework split (see e3-summary.md)


def run(pid):
    p = pairs[pid]
    out = subprocess.run([lab, "compare", p["a_path"], p["b_path"]], capture_output=True, text=True, encoding="utf-8")
    res = json.loads(out.stdout) if out.returncode == 0 else {"error": out.stderr.strip()[-200:]}
    return {"id": pid, "label": labels[pid]["label"], "cut": pid in CUTS, **res}


with ThreadPoolExecutor(4) as pool:
    results = list(pool.map(run, sorted(pairs, key=int)))
with open(HERE / "results/e3-fingerprints.jsonl", "w", encoding="utf-8") as f:
    for r in results:
        f.write(json.dumps(r) + "\n")

print(f"{'id':>3} {'label':<10} {'cut':<4} {'matched':>8} {'coverA':>7} {'coverB':>7} {'score':>6} segs")
for r in results:
    if "error" in r:
        print(f"{r['id']:>3} {r['label']:<10} ERROR {r['error']}")
        continue
    print(f"{r['id']:>3} {r['label']:<10} {'cut' if r['cut'] else '':<4} {r['matched_s']:>7.1f}s {r['cover_a']:>7.2f} {r['cover_b']:>7.2f} {r['score']:>6.2f} {r['segments']}")
