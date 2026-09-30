"""Run audio-lab's spectral-cutoff check over every readable file in results/inventory-c.jsonl.

  python cutoff_scan.py <audio-lab.exe>   -> results/cutoff.jsonl (read-only on the music)
"""
import json
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).parent
lab = sys.argv[1]
rows = [json.loads(l) for l in open(HERE / "results/inventory-c.jsonl", encoding="utf-8")]
rows = [r for r in rows if "error" not in r]


def run(r):
    out = subprocess.run([lab, "cutoff", r["path"]], capture_output=True, text=True, encoding="utf-8", timeout=120)
    res = json.loads(out.stdout) if out.returncode == 0 else {"error": out.stderr.strip()[-200:]}
    return {"path": r["path"], "ext": r["ext"], "bitrate": r.get("bitrate"), **res}


with open(HERE / "results/cutoff.jsonl", "w", encoding="utf-8") as f, ThreadPoolExecutor(6) as pool:
    for i, res in enumerate(pool.map(run, rows), 1):
        f.write(json.dumps(res, ensure_ascii=False) + "\n")
        if i % 1000 == 0:
            print(i, flush=True)
print("done", flush=True)
