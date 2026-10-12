# The local corpus

E3's 43 hand-labeled pairs of real files ([spikes/results/e3-summary.md](../spikes/results/e3-summary.md)) can be run through the app's own fingerprinting, matching and grouping rules, on a PC that has them. The pair list, the labels and the audio are private and never in git, so this check is **local only**. CI runs the made-up library instead (`src-tauri/src/matching/tests/e3_patterns.rs`).

## What it needs
A `spikes` folder holding:
- `e3/pairs.csv`: the pairs and the paths of their audio files,
- `results-private/e3-labels.csv`: each pair's label,
- `results-private/e3-fingerprints.jsonl`: which versions are cuts, and what E3 measured.

The audio files must be where `pairs.csv` says they are. Everything is only read; nothing is written next to them, and no audio file is touched.

## Running it
Set `TLP_E3_DIR` to that `spikes` folder, then run its three tests from `src-tauri`:

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

### Through the grouping rules
A second test puts every pair to `grouping::rules::decide`, as the rules' contract says: the audio is "identical" when the two files share an audio hash or a whole-file hash, else what the matching pass stored, else a direct comparison of the two fingerprints. The names are each file's title tag, artist tag and file name, read by the app's tag reader.

- **The hard line:** no pair labeled anything other than duplicate comes out a duplicate.
- **Per label** (in `labels.rs`, `allowed`): a duplicate merges; a cut gets a cut link; a rework gets a rework link or "different"; different songs get "different", never a link. "Needs review" is allowed for every label.

It prints, per pair, the id, the label, how the rules sort the audio, whether the pass had proposed the pair, the outcome and its reason codes; for a pair the pass never proposed, also what the rules say when nobody compares the two. Then a tally per label, and how many of the real duplicates merge without review, with the reason codes of those that don't.

### Through the grouping job
A third test runs the real thing. All the corpus files go into **one** database with their real facts: the fingerprint of the audio, the audio hash, the whole-file hash, the `raw_tags` the read stage writes, and the real file name. It then runs the steps the app runs, in order: the quick grouping pass, the matching pass, and the deciding job (direct comparisons, the set rule, the index of names). "Sharing a track" means the two files have the same `recording_file.recording_id`, whether the job merged them directly or through a chain of other files.

- **False merge:** two files of a pair not labeled duplicate that share a track. The test asserts there are none. The criterion (ROADMAP section 6, row 1b) is under 1%; with about 31 such pairs that means zero.
- **Pair 42** (a version pair that shares much audio) must not share a track. It is asserted by id.
- **Across pairs:** files of two different pairs in one track without the same audio are printed, not asserted: the labels cover only the pairs, and two files of different pairs can truly be one recording.

It prints one line per pair: id, label, `one track` or `apart`, then the stored outcome, its state and the reason codes (`no pair stored` where the rules said "different"). Then the false merges as `k/N` with the percentage, how many of the duplicates share a track, the pairs held as `set_disagrees`, the pairs still `waiting`, and the cross-pair list by pair id and file id. The database holds real names (in `rel_path` and `raw_tags`), so the test never dumps it and never prints a reason's fields: codes only.

Its made-up counterpart runs in CI (`src-tauri/src/matching/tests/corpus_job.rs`): the made-up library, given invented titles and an invented artist, goes through the same steps; its duplicate groups must merge and nothing else may.

## What it prints
One line per pair: its id, its category, the coverage and found share both ways, the score, both lengths, the number of segments, whether the pass proposed it, the share of lined-up items that are the same bit for bit and the index keys the files share, how the grouping rules would read the audio (full, one-sided, partial, none), what the version check says about the names, and E3's own numbers. Then a tally per category.

**It never prints a title, an artist, a file name or a path**, in its output or in a failure. Keep it that way when changing it, and never paste per-pair lines into a PR, an issue or a commit.

## When a pair fails
Don't bend the requirement, and don't change `matching` to fit one pair. Report the pair by id, category and numbers: the grouping rules need to know.

## The tally it pins
The test also asserts how many pairs share audio without a full or one-sided match, and how many of those the matching pass proposes (15 and 5 today; see the top of `local_corpus.rs`). The rest line up too weakly for the pass to compare them at all. If a change to matching moves these numbers, the run fails: correct the tally there and tell whoever owns the grouping rules.
