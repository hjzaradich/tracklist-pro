"""Pick ~40 candidate file pairs for E3 labeling from results/inventory-c.jsonl.

Candidates come from names/tags/durations/sizes only (no fingerprints), so the
labels can later grade the fingerprinter without having been chosen by it.
Writes e3/pairs.csv (with the category that selected each pair, hidden from
the user during labeling). Read-only on the music.
"""

import csv
import json
import random
import re
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).parent
rows = [json.loads(l) for l in open(HERE / "results/inventory-c.jsonl", encoding="utf-8")]
rows = [r for r in rows if "error" not in r and r.get("duration_ms")]

VERSION_WORDS = r"(extended|original|radio|club|mix|edit|remix|flip|vip|bootleg|dub|clean|dirty|intro|outro|instrumental|acapella|a cappella|version|rework|refix|mashup|redrum|short|quick hit|final|master)"
FEAT = re.compile(r"\s*[\(\[]?\s*(feat\.?|ft\.?|featuring)\s[^\)\]]*[\)\]]?", re.I)


def stem(r):
    return Path(r["path"]).stem


def title_of(r):
    t = r.get("title") or stem(r)
    return t


def base_title(t):
    t = FEAT.sub("", t)
    t = re.sub(r"[\(\[][^\)\]]*[\)\]]", " ", t)  # drop bracketed version info
    t = re.sub(r"\s-\s.*" + VERSION_WORDS + r".*$", " ", t, flags=re.I)
    t = re.sub(r"[^\w\s]", " ", t.lower())
    return " ".join(t.split())


def version_part(t):
    found = re.findall(r"[\(\[]([^\)\]]*)[\)\]]", t)
    return " ".join(found).lower()


def artist_key(r):
    a = (r.get("artist") or "").lower()
    a = re.split(r",|&| x | vs\.? | feat| ft\.?", a)[0]
    return " ".join(re.sub(r"[^\w\s]", " ", a).split())


groups = defaultdict(list)
for r in rows:
    b = base_title(title_of(r))
    if len(b) >= 3:
        groups[b].append(r)

random.seed(27)
pairs = []
used_songs = set()  # one pair per song, so the user never labels the same song twice


def add(cat, a, b, n):
    songs = {base_title(title_of(a)), base_title(title_of(b))}
    if songs & used_songs:
        return
    if sum(1 for p in pairs if p[0] == cat) < n and a["path"] != b["path"]:
        used_songs.update(songs)
        key = tuple(sorted((a["path"], b["path"])))
        if key not in {tuple(sorted((p[1]["path"], p[2]["path"]))) for p in pairs}:
            pairs.append((cat, a, b))


items = list(groups.items())
random.shuffle(items)
for base, rs in items:
    if len(rs) < 2:
        continue
    for i in range(len(rs)):
        for j in range(i + 1, len(rs)):
            a, b = rs[i], rs[j]
            same_artist = artist_key(a) and artist_key(a) == artist_key(b)
            dd = abs(a["duration_ms"] - b["duration_ms"]) / 1000
            va, vb = version_part(title_of(a)), version_part(title_of(b))
            fa, fb = a["ext"], b["ext"]
            joined = (title_of(a) + " " + title_of(b)).lower()
            if a["size"] == b["size"] and dd < 1:
                add("exact-copy", a, b, 5)
            elif same_artist and fa != fb and dd < 2:
                add("format-pair", a, b, 5)
            elif same_artist and fa == fb and dd < 2 and abs((a.get("bitrate") or 0) - (b.get("bitrate") or 0)) >= 64:
                add("bitrate-pair", a, b, 4)
            elif same_artist and dd >= 15 and re.search(r"extended|radio|original|club", joined):
                add("extended-vs-radio", a, b, 5)
            elif re.search(r"flip|bootleg|edit|rework|refix", va + " " + vb) and dd >= 3:
                add("flip-or-edit", a, b, 5)
            elif re.search(r"\bvip\b", va + " " + vb) and dd >= 3:
                add("vip", a, b, 4)
            elif re.search(r"clean|dirty", joined):
                add("clean-dirty", a, b, 4)
            elif same_artist and 2 <= dd < 15:
                add("near-duration-same-artist", a, b, 4)
            elif not same_artist and artist_key(a) and artist_key(b):
                add("same-title-different-artist", a, b, 5)

# mashups: a title containing " x " paired with a file whose base title matches one side
for r in rows:
    t = title_of(r)
    if re.search(r"\s[xX]\s", t) and sum(1 for p in pairs if p[0] == "mashup") < 4:
        for part in re.split(r"\s[xX]\s", base_title(t)):
            part = part.strip()
            if part in groups:
                other = random.choice(groups[part])
                if other["path"] != r["path"]:
                    add("mashup", r, other, 4)
                    break

with open(HERE / "e3/pairs.csv", "w", newline="", encoding="utf-8") as f:
    w = csv.writer(f)
    w.writerow(["id", "category", "a_path", "b_path", "a_dur_s", "b_dur_s", "a_ext", "b_ext", "a_kbps", "b_kbps", "a_size", "b_size", "a_title", "b_title", "a_artist", "b_artist"])
    random.shuffle(pairs)  # mix categories so the user can't infer the expected answer
    for i, (cat, a, b) in enumerate(pairs, 1):
        w.writerow([i, cat, a["path"], b["path"], round(a["duration_ms"] / 1000), round(b["duration_ms"] / 1000),
                    a["ext"], b["ext"], a.get("bitrate"), b.get("bitrate"), a["size"], b["size"],
                    title_of(a), title_of(b), a.get("artist"), b.get("artist")])
from collections import Counter
print(len(pairs), "pairs:", dict(Counter(p[0] for p in pairs)))
