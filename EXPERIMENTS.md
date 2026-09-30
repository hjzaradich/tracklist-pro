# Phase 0 Experiments

These were the experiments that answered the open questions in [ROADMAP.md §7](ROADMAP.md#7-open-questions). They're done; this page is the record of what each one was for. The raw exports and notes were the author's own and aren't published. The verdicts are in ROADMAP §0.4.

| # | What | Status |
|---|---|---|
| E1 | A rekordbox session: load a test XML into an empty rekordbox, edit, re-import, and export again | ✅ Done 2026-09-27. Re-runs use [spikes/rb-kit/CHECK.md](spikes/rb-kit/CHECK.md) |
| E2 | A census of a real music collection: folder layout, formats, OneDrive online-only files, naming conventions, scale | ✅ Done 2026-09-27 |
| E3 | Duplicate and version labels: pairs of files labeled same / version / different, to grade the fingerprinter | ✅ Done 2026-09-27. Write-up in [spikes/results/e3-summary.md](spikes/results/e3-summary.md) |

## E1: the rekordbox session

| Part | What was done | What it answered |
|---|---|---|
| 0 Setup | Note the version and settings, back up the empty library | Is there any "write tags to file" setting? |
| A Import v1 | Load our XML into the empty Collection | Does rekordbox 7 accept our XML? Which fields and awkward paths survive? Does it trust our BPM and key? How does rekordbox write paths itself? |
| B Edit | Change a title, a comment, a rating and color, a My Tag and a playlist in rekordbox | Can we see rekordbox edits in its export? Does XML carry My Tag? |
| C Replace | Set cues, then swap a track's file in place | Do cues and the grid survive when a Library copy is regenerated? |
| D History | Play tracks in Performance mode | Can history get out through XML, or only through a separate export? |
| E Import v2 | Re-import a changed XML on top of the edits | Does re-importing update or duplicate? Do playlists get replaced? Do rekordbox's edits survive our XML? |
| F Ground truth | Add and analyze a folder of tracks | rekordbox's BPM and key, to grade the app's estimates |
| G Reset | Restore the empty backup | n/a |

Files were fingerprinted before and after the session to prove rekordbox never wrote into the originals. It does write into files it manages, so the session worked on copies.

## E2: the music census

**Answers:** what the scanner has to cope with in reality: real folder layouts, OneDrive online-only files, pool naming conventions and scale. A read-only inventory script listed paths, sizes, formats and tags for the folders the author chose.

## E3: duplicate and version labels

**Answers:** the ground truth for fingerprinting and version detection: can the tool separate Extended from Radio Edit, or Clean from Dirty, and what is the false-merge rate? About forty pairs of files were labeled `same` (one track, e.g. a WAV and an MP3 of one mix), `version` (same song, different version) or `different`.

## Other spikes (run by the assistant)

| Spike | Result |
|---|---|
| XML field and path-encoding analysis | Done. Behavior is in ROADMAP §5 |
| Files-untouched check | rekordbox **does** write into files it manages; originals untouched |
| Local BPM and key accuracy | BPM matched rekordbox on nearly every track; key agreement was about half (not shown yet). See ROADMAP 0.4 |
| Fingerprint speed and accuracy | All "same" pairs grouped, no false merges of different songs; Clean/Dirty needs names ([e3-summary.md](spikes/results/e3-summary.md)) |
| Fake-320 and fake-lossless detector | Done. See ROADMAP 1.6 |
| Unreadable files | Found macOS `._` files, bad tags, a mislabeled file and broken files. See ROADMAP 1.1 |
| USB and CDJ history | Readable from a gig stick's `export.pdb`; sessions are undated |
| WebView2 audio format support | AIFF and ALAC need Rust decoding |
| OneDrive placeholder detection | An attribute check, with no downloads |
| GPL-compatible ML models | EfficientAT and LAION-CLAP ([ml-models-research.md](spikes/results/ml-models-research.md)) |

Follow-up tests are listed in ROADMAP §7.
