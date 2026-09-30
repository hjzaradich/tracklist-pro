"""Read-only music inventory for E2/E3 (see EXPERIMENTS.md).

  python inventory.py <kit-helper.exe> <out.jsonl> <dir> [<dir> ...]

Walks each dir, and for every audio file records path, size, mtime and
kit-helper's `info` (duration, bitrate, sample rate, basic tags). Opens files
for reading only; writes nothing but <out.jsonl>.
"""

import json
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

AUDIO = {".mp3", ".wav", ".flac", ".m4a", ".aif", ".aiff", ".ogg", ".opus", ".aac", ".wma", ".alac"}


def probe(helper, path: Path):
    st = path.stat()
    row = {"path": str(path), "ext": path.suffix.lower(), "size": st.st_size, "mtime_ns": st.st_mtime_ns}
    try:
        out = subprocess.run([helper, "info", str(path)], capture_output=True, text=True,
                             encoding="utf-8", timeout=60)
        if out.returncode == 0:
            info = json.loads(out.stdout)
            info.pop("path", None)
            info.pop("size", None)
            row.update(info)
        else:
            row["error"] = out.stderr.strip()[-300:]
    except Exception as e:  # one bad file never stops the inventory
        row["error"] = repr(e)
    return row


def main():
    helper, out, *dirs = sys.argv[1:]
    files = [p for d in dirs for p in Path(d).rglob("*") if p.is_file() and p.suffix.lower() in AUDIO]
    print(f"{len(files)} audio files", flush=True)
    with open(out, "w", encoding="utf-8") as f, ThreadPoolExecutor(8) as pool:
        for i, row in enumerate(pool.map(lambda p: probe(helper, p), files), 1):
            f.write(json.dumps(row, ensure_ascii=False) + "\n")
            if i % 1000 == 0:
                print(f"{i}/{len(files)}", flush=True)
    print("done", flush=True)


if __name__ == "__main__":
    main()
