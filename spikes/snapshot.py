"""Read-only file snapshots, to prove whether rekordbox writes into audio files.

  python snapshot.py take <out.json> <dir> [<dir> ...]   hash every file (read-only)
  python snapshot.py diff <before.json> <after.json>     list added / removed / changed files
"""

import hashlib
import json
import sys
from pathlib import Path


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def take(out: Path, dirs: list[str]) -> None:
    files = {}
    for d in map(Path, dirs):
        for p in sorted(d.rglob("*")):
            if p.is_file():
                st = p.stat()
                files[str(p)] = {"size": st.st_size, "mtime_ns": st.st_mtime_ns, "sha256": sha256(p)}
    out.write_text(json.dumps({"dirs": dirs, "files": files}, indent=1, ensure_ascii=False), encoding="utf-8")
    print(f"{len(files)} files -> {out}")


def diff(before: Path, after: Path) -> None:
    a = json.loads(before.read_text(encoding="utf-8"))["files"]
    b = json.loads(after.read_text(encoding="utf-8"))["files"]
    for path in sorted(a.keys() - b.keys()):
        print(f"REMOVED  {path}")
    for path in sorted(b.keys() - a.keys()):
        print(f"ADDED    {path}")
    for path in sorted(a.keys() & b.keys()):
        if a[path]["sha256"] != b[path]["sha256"]:
            print(f"CHANGED  {path}  (content)")
        elif a[path]["mtime_ns"] != b[path]["mtime_ns"]:
            print(f"TOUCHED  {path}  (modified time only)")
    print(f"compared {len(a)} -> {len(b)} files")


if __name__ == "__main__":
    cmd, *rest = sys.argv[1:]
    if cmd == "take":
        take(Path(rest[0]), rest[1:])
    elif cmd == "diff":
        diff(Path(rest[0]), Path(rest[1]))
    else:
        sys.exit(__doc__)
