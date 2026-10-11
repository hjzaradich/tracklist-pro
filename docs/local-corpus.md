# The local corpus

E3's 43 hand-labeled pairs of real files ([spikes/results/e3-summary.md](../spikes/results/e3-summary.md)) can be run through the app's own fingerprinting and matching, on a PC that has them. The pair list, the labels and the audio are private and never in git, so this check is **local only**. CI runs the made-up library instead (`src-tauri/src/matching/tests/e3_patterns.rs`).

## What it needs
A `spikes` folder holding:
- `e3/pairs.csv`: the pairs and the paths of their audio files,
- `results-private/e3-labels.csv`: each pair's label,
- `results-private/e3-fingerprints.jsonl`: which versions are cuts, and what E3 measured.

The audio files must be where `pairs.csv` says they are. Everything is only read; nothing is written next to them, and no audio file is touched.

## Running it
Set `TLP_E3_DIR` to that `spikes` folder, then run the one test from `src-tauri`:

```powershell
$env:TLP_E3_DIR = "C:\dev\tracklist-pro\spikes"
cargo test --lib local_corpus -- --nocapture
```

It fingerprints all 86 files, so it takes a few minutes.

- With `TLP_E3_DIR` unset, the test prints `local corpus skipped` and passes. That's what CI sees.
- With it set, the run is meant. A folder that isn't there **fails** it, and so does a file that is missing or can't be decoded (by pair id), so the corpus never shrinks quietly.

## What it checks
What each label requires is written in `src-tauri/src/matching/tests/labels.rs`:

| Label | Required |
|---|---|
| duplicate | The duplicate rule holds (coverage ≥ 90% both ways, score ≤ 4), in one alignment, and the matching pass proposed the pair |
| cut, another length | Not a duplicate; the pass proposed it; at least half of the shorter file is found in the longer |
| cut, same length (Clean and Dirty) | The audio passes the duplicate rule; the version check, run on the real names, says "cut" on at least one reading (title tags, file names) and "same" on none |
| rework | Not a duplicate |
| different | Not a duplicate; at most 5 seconds line up |

## What it prints
One line per pair: its id, its category, the coverage and found share both ways, the score, both lengths, the number of segments, whether the pass proposed it, the share of lined-up items that are the same bit for bit and the index keys the files share, how the grouping rules would read the audio (full, one-sided, partial, none), what the version check says about the names, and E3's own numbers. Then a tally per category.

**It never prints a title, an artist, a file name or a path**, in its output or in a failure. Keep it that way when changing it, and never paste per-pair lines into a PR, an issue or a commit.

## When a pair fails
Don't bend the requirement, and don't change `matching` to fit one pair. Report the pair by id, category and numbers: the grouping rules need to know.

## The tally it pins
The test also asserts how many pairs share audio without a full or one-sided match, and how many of those the matching pass proposes (15 and 5 today; see the top of `local_corpus.rs`). The rest line up too weakly for the pass to compare them at all. If a change to matching moves these numbers, the run fails: correct the tally there and tell whoever owns the grouping rules.
