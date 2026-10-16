# tracklist-pro Roadmap

> **Draft 5 (2026-09-27).** A rebuild of `../musicmanager`, which serves as a loose blueprint.
> Rules Claude Code must always follow: [CLAUDE.md](CLAUDE.md). Current work: [TASKS.md](TASKS.md). Public notice: [NOTICE.md](NOTICE.md). Spike write-ups: [`spikes/results/`](spikes/results/). Document history: git.
>
> Figures marked *(ref)* come from the **reference library**: one real DJ collection (thousands of audio files, a rekordbox 7.2.19 collection and one gig stick) used for the Phase 0 spikes.
> Only the clean write-ups in `spikes/results/` are published; the raw spike data is private, and CI must never depend on it.

## 1. What we're building

A free, open-source (GPL-3.0) library manager for DJs who use **rekordbox 7** on Windows. It sits upstream of rekordbox and does four jobs DJs now do by hand:

1. **Build a Library out of the mess.** Start from your rekordbox collection or from nothing. The app finds music across folders and drives and groups duplicates, even re-encoded or re-tagged ones. It tells versions apart, picks the best file, gathers the best tags from every copy, and carries over what rekordbox already knows (BPM, key, cues, play counts).
2. **Keep it in step with rekordbox.** Send the Library via XML with correct tags and art. Read rekordbox's analysis and edits back. Flag fields edited on both sides for you to settle.
3. **Get tracks in.** Turn wishlists into owned files, and catch, identify and tag downloads as they land.
4. **Prep sets.** Crates and smart crates, your own tag vocabulary, harmonic "what's next", and playlists with an energy arc, all informed by what you've played.

**Audience: any DJ who uses rekordbox on Windows, whatever they play.** No genre or style is primary: bass, open format, house, techno, hip-hop, mobile and wedding DJs, and everything in between. Other DJ software and macOS aren't supported yet. The export layer is kept pluggable so they can come later (backlog). That means:
- **Versions are first-class:** edits, flips, bootlegs, VIPs, mashups, dubs, covers and pool edits (Clean/Dirty/Intro/Outro…), because many kinds of DJ use them.
- **Defaults are genre-neutral**, including the genre seed and the "what's next" tolerances.
- **Every setup is covered.** Anything that touches history or export handles laptop play (Performance mode), USB on CDJs/XDJs, and B2B sets.

### Guiding principles

- **The Library is the point.** The app builds order out of the mess; it is not a drive-tidying tool. Most DJs have years of work in rekordbox, so it's read from day one. Starting from nothing is fully supported.
- **Read-only outside the Library.** The app never moves, renames, retags or deletes files outside the Library folder. That includes Downloads and the files that linked Library tracks point to. Duplicate resolution is database-only.
- **Library tracks are durable.** A linked track depends on its file: it's flagged "file missing" if the file disappears, and the app warns when a track lives somewhere fragile, like Downloads. A managed copy (from Phase 2) survives its original disappearing, flagged "original file missing". Either kind is removed only by an explicit in-app delete.
- **Build what the current phase needs.** Tables, ports and services arrive in the phase that first uses them, not before.
- **rekordbox writes into files, so never assume a file is unchanged.** Identify files by their audio, re-read tags, and never treat a new modified time as new audio (§5.1).
- **A track is not a file, and versions are not duplicates.** Duplicates merge. Versions are separate tracks that are only linked, and nothing is inherited across a link. Identity is the fingerprint plus the version; paths only point to instances.
- **rekordbox performs, we prep.** XML is the guaranteed base. Reading the database and gig sticks are optional extras that fail gracefully. A field edited on both sides becomes a conflict you settle, never a silent overwrite.
- **No black boxes.** Every suggestion shows a short, plain-language reason alongside it at the moment it's made, e.g. "same key, +2 BPM, tagged Peak Time". That covers tags, genres, crate or playlist adds, best files, matches and next tracks. If the app can't say why, it doesn't suggest. ML output is labeled as the audio model's opinion and is never the only reason.
- **Everything in the app is reversible** through the operation log. The exception is what rekordbox does after you import a send, which the app can't undo. So the app merges conflicts first and guides the import (1.9).
- **Estimates are labeled** until rekordbox's analysis replaces them.
- **Offline by default, private always.** Every online service is opt-in and says what it sends. No telemetry.
- **Optional features are the user's choice.** The wishlist, metadata lookup, the audio model and any later integration are switched on by the user, one by one, and the app works fully with each of them off. A light feature ships inside the app and is only switched on; a heavy one (the audio model) is downloaded when the user asks for it.
- **Portable code, Windows release.** Paths are stored as volume identity plus a relative path. CI builds macOS and Linux, but only Windows is supported.

### 1.1 Project decisions

| Area | Decision | Notes |
|---|---|---|
| Name | `tracklist-pro`, locked | Bundle ID `com.tracklistpro.desktop`. Data in `%APPDATA%\com.tracklistpro.desktop\`. Never renamed after the first release. |
| License | GPL-3.0 | Every dependency and every ML model license must be GPL-compatible, checked by `cargo-deny` in CI. |
| Visibility | **Public from Stage 1aP**, right after 1aC (owner, 2026-09-30; moved up from the end of 1c so CI runs free on GitHub's runners) | A fresh public repo with one clean starting commit, once the pre-public checklist is done. The full private history stays in the private `tracklist-pro-archive`, and private notes stay private. |
| Distribution | $0, unsigned | Onboarding explains SmartScreen's "More info → Run anyway". The updater signature (Tauri's own key) still protects updates. |
| Platforms | Windows 10/11 only | macOS and Linux are built in CI but not released. |
| rekordbox | 7 only | |
| Scale | ~100k files | Performance tests use a synthetic 100k-file fixture from Phase 1. |
| Audience | Any rekordbox-on-Windows DJ, no single persona | §1. The reference library is one test case, not a definition of the user. |
| Suggestions | Always explained | Every suggestion carries a reason string, and a test fails if one reaches the UI without it. |
| Data authority | Detect and review conflicts | Three-way merge using `sync_base` (1.10). |
| Files outside the Library | Strictly read-only | Enforced in code (§5.6). |
| Library tracks | **MVP: linked only.** Managed copies arrive in Phase 2 (2.6) | Adding a track links it to its existing file, so rekordbox gets no new entry and the MVP never writes an audio file. From Phase 2, a managed copy is made on request, for Inbox files, or on an accepted upgrade. Tracks rekordbox already knows stay linked by default. |
| Library format (Phase 2) | One Library folder, configurable format | MP3 320 by default. AIFF or the original lossless are settings. |
| Energy | 1–10 | Compatible with Mixed In Key's `Energy N`. How it reaches rekordbox is still open (§7). |
| Network | Offline by default, opt-in per service | Model downloads and update checks count as services. Every request goes through the network gate (0.1). |
| Telemetry | None, ever | The diagnostics bundle (4.3) is the only source of bug-report detail. |
| Optional features | Each one is off until the user turns it on; the app works with any mix of them on or off | Wishlists (2.3–2.5), metadata lookup (2.2), the audio model (3.8), and reach goals such as a second DJ app. Light features ship in the installer and are only switched on. Heavy ones are a separate download on request: the audio model is never bundled (owner decision, 2026-10-02). No core screen, send or suggestion may need an optional feature. |
| AI / ML | Local audio ML only; labeled and optional | No cloud LLM. ML signals can be switched off, and nothing depends on them (3.8). |
| In-app audio | Simple preview in the webview | No native engine and no headphone routing. AIFF and ALAC are decoded in Rust. |
| musicmanager | **A blueprint only; never part of the owner's workflow** (owner, 2026-09-30) | Nobody relies on it, so there's no freeze to time. Separate bundle IDs, data folders, updater endpoints and credential prefixes, so both can be installed side by side. No shared DB and no migration. Ported modules are copies (0.2); nothing is ported back. |
| Frontend | React | |
| Language | English, translation-ready | All UI strings go through `i18next`. |
| Theme | Dark first, light available | Design tokens; both themes are tested. |
| Layout | Sidebar · table · Details panel · player | The sidebar lists stages in workflow order with count badges, then crates and playlists. The top bar holds search/palette and Activity (background work). The Details panel, the player and the search box are not drawn until they are built in 1c (owner decision, 2026-10-10: placeholders that look like features are hidden). |
| Visual personality | "Studio gear" | Graphite neutrals and one restrained accent, with color only where it means something. No pure black, no pure-white text. |
| Key notation | Camelot by default; one global setting picks the notation | Options: **Camelot**, **Musical (standard)** (conventional key-signature spelling), **Musical (rekordbox)** (rekordbox's own spelling, captured by the behavior check), **Musical (sharps)**, **Musical (flats)**. Accidentals are written with plain `#` and `b` (more accessible than ♯/♭; owner's choice). "Standard" uses the spelling with the fewest sharps or flats, so it differs from rekordbox only at 1A (G#m) and 12A (C#m). rekordbox's Classic spellings are confirmed for all 24 keys, including 2B = F# and 6B = Bb (owner, 2026-09-30). More (e.g. Open Key, Serato or Mixed In Key spellings) can be added later. Key color is a small dot, not a filled cell. Decided 2026-09-28. |
| Density | Compact (~28px rows) | A comfortable option exists. Tabular numerals for BPM, time and key. |
| Starting point | "Start from rekordbox" (main) or "Start fresh" | 1.3. |
| Naming | DJs' words, one word per concept | §1.2. |
| rekordbox database (`master.db`) | **Not read; not planned** | It's encrypted. XML plus `.m3u8` history and gig-stick `export.pdb` cover what the app needs. |
| Gig USBs / OneLibrary | `export.pdb` read only; never written | OneLibrary `exportLibrary.db` is encrypted, so it isn't read either (not planned). |
| Performance setups | All | Laptop and USB history are both covered. |
| Plays vs sessions | Separate | Plays are imported per source; a session combines sources (B2B). |
| Versions | Linked, never merged; cuts vs reworks | §1.2, 1.5. |
| Contributions | **Sole author for now** | Issues and ideas are welcome (pool naming patterns, genre aliases); code contributions aren't accepted yet. |

### 1.2 Vocabulary

Use the words DJs already use, with exactly one word per concept. Synonyms are allowed only for words that never name a screen, button or data type (gig / set). Code uses the same words except where the "In code" column differs.

| Word | Means | In code | Not |
|---|---|---|---|
| **All music** | Every track the scan found in your music folders | `recording` (all) | Collection, archive, sources |
| **Library** | The tracks you've added to play with | `library_track` | gig library |
| **Track** | One piece of music in one version, however many files hold it | `recording` | |
| **File** | One audio file on disk | `file` | copy |
| **Library copy** | A clean file the app makes in the Library folder (from Phase 2) | `library_track.kind = copy` | gig copy |
| **Linked** (track) | A Library track that points at an existing file, with no copy. In the MVP every Library track is linked, so the UI just says "Library track"; a UI word for the difference is needed from Phase 2 (§7) | `library_track.kind = linked` | |
| **Add to Library** | Make a track a Library track, either linked or a copy | `promote` | |
| **Duplicates** | Several files that hold the same track | `recording_file` | cluster |
| **Versions** | Separate tracks of the same song, linked but never merged. Each keeps its own BPM, key, genre, tags, cues and history | `version_link` | family, related tracks |
| **Cut** | A version with the same production but a different length or lyrics: Extended, Radio Edit, Club Edit, Clean/Dirty, Intro/Outro, Short, a plain "Edit", Remaster. Nearly interchangeable | `version_link.kind = cut` | edit |
| **Rework** | A different production: remix, VIP, flip, bootleg, rework, dub, cover, live, alternate arrangement, instrumental, acapella, mashup. Never a stand-in | `version_link.kind = rework` | remix (as a catch-all) |
| **Best file** | The duplicate whose audio is used. Tags and art can come from any of the track's files | `recording_file.role = best` | keep-best |
| **Extra files** | The other duplicates: hidden in the app, never touched on disk | `recording_file.role = extra` | superseded |
| **Hide** / **Hidden** | Not shown, and always recoverable from the Hidden filter | `file.hidden_reason` | ignore |
| **Not now** | The Inbox action for "leave it in All music, decide later" | `inbox_item.status = not_now` | keep |
| **Crate** | An unordered group of tracks collected for a purpose; can be smart | `crate` | |
| **Playlist** | An ordered list where the order matters; also works as a crate | `playlist` | set |
| **Music folders** | The folders you point the app at | `music_folder` | sources, roots |
| **Estimate** | A BPM, key or energy value the app worked out itself, until rekordbox confirms it | `analysis` (source ≠ rekordbox) | provisional |
| **Session** | A performance assembled from one or more play sources | `session` | |
| **Gig**, **set** | UI words for a session. Never the name of a screen or data type | — | |
| **Details** (panel) | The right-hand panel showing the selected track's info | `DetailsPanel` | inspector, side panel |
| **Activity** | The top-bar name for background work (scanning, fingerprinting…). Idle text: "No background tasks" (owner's wording) | `job` | jobs (as its name) |

---

## 2. Library model

Three layers, fed by two read-only inputs:

```
┌─────────────────────────────┐   ┌─────────────────────────────┐
│ MUSIC FOLDERS (read-only)   │   │ REKORDBOX COLLECTION        │
│ every file → file record    │   │ (read-only) BPM, key, grid, │
│                             │   │ cues, plays, rating, color, │
│                             │   │ comments, My Tag, playlists │
└──────────────┬──────────────┘   └──────────────┬──────────────┘
               │ fingerprint +                   │ matched by location,
               │ version grouping                │ then relink (1.2)
               ▼                                 ▼
┌───────────────────────────────────────────────────────────────┐
│  TRACKS (`recording`)   one track ← N duplicate files         │
│  canonical tags + rekordbox data, genre, energy, history      │
└───────────────┬───────────────────────────────────────────────┘
                │ add to Library (linked, or rules gate + convert)
                ▼
┌───────────────────────────────────────────────────────────────┐
│  LIBRARY (linked tracks 1.8; managed copies 2.6)              │
│  durable; removed only in-app                                 │
│  → rekordbox XML send  ⇄ conflicts ⇄  rekordbox read-back     │
└───────────────────────────────────────────────────────────────┘
          ▲
   INBOX (watched folders): files stay put and get indexed
```

In the UI, **All music** shows the tracks layer and **Library** shows Library tracks. **Add to Library** moves a track from one to the other.

### Core entities (sketch)

| Entity | Key fields | Notes |
|---|---|---|
| `volume` | identity (serial-first; never a drive letter), label, guid, last_mount_path, kind (internal / external / network) | Portable path identity. `last_mount_path` is shown for an offline volume, and relink uses it to match a rekordbox Location to a file the scan already indexed on a drive that isn't plugged in, only when exactly one known volume was last mounted there (1aC-3, 2026-09-30). It's never used to find files on disk. |
| `music_folder` | volume_id, rel_path, watch (bool), role (scan / inbox), unreadable_folders, unreadable_files, walked_at | The counts are from the last finished walk (1aB-14). |
| `file` | music_folder_id, rel_path (on-disk form), rel_path_key (NFC, for matching), size, mtime, file_id (volume serial + NTFS file index, as text), blake3, **audio_hash**, partial_hash, fingerprint, fingerprint_audio_hash, fingerprint_rev, **sniffed_format**, codec, bitrate, sample_rate, duration, quality_verdict, raw_tags (JSON: one array of {key, value} per tag block type, e.g. id3v2, ape; `{}` when untagged, NULL when the tags couldn't be read), present (bool), online_only (bool: a OneDrive placeholder, never opened unless the user opts in), hidden_reason (null / user / inbox_cleared) | One row per physical file, never written to by the app. `audio_hash` covers only the audio frames, because rekordbox rewrites tags. `partial_hash` is the cheap check for an mtime-only change (1.1). `fingerprint_audio_hash` is the `audio_hash` the fingerprint was made from, set only when the hash stage was current at that stat (1aC-10). `fingerprint_rev` is a stamp the database changes whenever the stored `fingerprint` bytes change, to or from NULL included (a tag rewrite, a new modified time or the same bytes written again change nothing), so matching can tell which fingerprints to read again without reading them (1bA-18, migration 0020). It's random, and 0 for a row that existed before migration 0020 and for a row with no fingerprint, until its fingerprint changes. `sniffed_format` comes from the bytes. |
| `file_stage` | file_id, stage (read / hash / fingerprint), version, size, mtime, status (done / failed / skipped), reason, done_at | What each scan stage last did to a file, and at which size, mtime and stage version, so a stage redoes only changed files (1aB, 2026-09-29). **Failed** means the content couldn't be processed and waits until the file changes; a file that couldn't be reached is never failed, only skipped or left unrecorded, so it's retried. Deleted with its file. |
| `fingerprint_match` | file_a, file_b (the lower file id first), version, items_a, items_b, coverage_a, coverage_b, score, segments (JSON: offset_a, offset_b, items, score, alignment per matched stretch), compared_at | What comparing two files' full-track fingerprints found (1.4; 1bA-2, 2026-10-02): the share of each file the main alignment matched, the difference score over it, and every matched stretch with its offsets in fingerprint items. One row per compared pair, a pair where nothing matched included. The matching pass compares only the pairs its block index proposes; the grouping job adds the pairs it compares directly (same-named files and a mashup's named parts that the pass never proposed, and pairs inside a set of duplicates that nobody compared; 1bC-1, 2026-10-11), never every pair. Files with the very same fingerprint are linked to the first of them, which holds their results with other files. Derived state, measurements only: the grouping rules (1bB) read it and decide. The database deletes a file's rows when its `fingerprint` bytes change, and with the file row; a tag rewrite or a new modified time leaves them (§5.1). Not in the operation log. The definitions are in the `matching` module's docs. |
| `fingerprint_matched` | file_id, version, stands (the file whose results stand for this one: itself, or the first of the files with the very same fingerprint), matched_at | Which files' current fingerprints have been through matching, every result stored (1.4; 1bA-14, 2026-10-04). A present file with a fingerprint and no row is what makes matching due. Derived state: its row goes when the file's fingerprint changes or is cleared, when the file row or the row of the file standing for it goes, and when a missing file comes back (files that arrived meanwhile were never compared with it); it stays while a file is merely missing, as its results do. A row of another version than the comparison's counts as no row. |
| `file_quality` | file_id, audio_hash, method_version, cutoff_hz, cutoff_gap (too_short / silent / low_rate: why there's no cutoff), decoded_ms, header_ms, decode_errors, error_kinds (JSON counts by kind), ended (complete / cut_short / gave_up), failure, measured_at | The quality job's measurements for a file (1.6; 1bA-10, 1bA-11), one row per file, measurements only: the verdicts are worked out from them (1bB-6) and kept in `file_verdict`, never in `file.quality_verdict`. A row belongs to the **audio** it was measured from (`audio_hash`), not to the file's mtime (5.1): a file whose `audio_hash` still equals the row's is never measured again, and one with other audio is. `method_version` makes every row due again when the method changes. A file whose content can't be decoded gets a row with a `failure` and no measurements, and isn't tried again until its audio changes; a file that couldn't be reached gets nothing. Deleted with its file. |
| `file_verdict` | file_id, rules_version, inputs, verdict (ok / low_bitrate / suspect_transcode / truncated / broken, or NULL for none), findings (JSON array of {code, numbers}), assessed_at | The stored quality verdict of each file and the findings behind it (1.6; 1bC-5, migration 0021), worked out by `quality::verdict::assess` from the file's codec, bitrate and sample rate, the read stage's verdict and its `file_quality` row. **Derived state, in its own table:** `file.quality_verdict` is the read stage's own verdict (`truncated` / `broken`, cleared by every read) and an input to `assess`, so nothing writes the answer there. A row is current only while its `rules_version` is the build's (a version constant bumped with `assess`'s rules, so new thresholds re-assess every file from the stored measurements without decoding audio) and its `inputs` key equals the inputs now (any new measurement, re-read, audio hash or read-stage verdict makes it stale). It is settled by the quality job (at its start for every stale file, and for a file as its measurement lands) and by `quality::settle`, which whatever reads verdicts calls first; a stale verdict alone queues no job. A file with no row, a stale row or a verdict NULL has **no verdict**, read by `quality::stored_verdict` as none and never as `ok`. Deleted with its file. |
| `recording` | canonical title / artist / version / remixers / label / year, genre_id, per-field provenance; display bpm / key / energy | A **track**. The display bpm/key/energy are **derived from `analysis`** and never edited directly. |
| `analysis` | recording_id, source (rekordbox / tag / MIK / user / local / ml), bpm, key, energy, confidence | **The source of truth** for these values: several per track. Precedence: BPM and key go rekordbox > file tag > MIK > local; energy goes user > MIK > local > ML (the audio model is optional and never the only reason, so it ranks last). Disagreements are flagged. The track's shown values come from the `recording_display` view, so they can't drift or be edited directly. **rekordbox rows** (1aD-4): one per track, written by the attach job from the track's trusted rekordbox entries only (never a `relink_probable` row or one with no track). The entry matched to the track's best file gives the row, else the most played, else the lowest TrackID; the others stay in `rekordbox_track` for Review. A row goes when its entry turns probable, unmatched or matched elsewhere. A merge drops the emptied track's rekordbox rows (attach recomputes them) and moves its other sources' rows to the surviving track, which keeps its own when both have one. |
| `recording_file` | recording_id, file_id, role (best / undecided / extra), match_confidence | A track's duplicates. The best-ranked file (1.7) is `best` and every other file `extra`; `undecided` is only a file not ranked yet. "Extra" is purely a DB state. |
| `version_link` | recording_a, recording_b, kind (cut / rework), label (extended, radio, clean/dirty, edit, remaster, remix, VIP, flip, bootleg, cover, live, arrangement, instrumental, acapella, mashup-contains…), source (parser / fingerprint / user), confirmed | Nothing is inherited across a link. The grouping job makes the `parser` and `fingerprint` rows from its decided pairs (`grouping_pair`) and takes its own away when no pair asks for them any more; a row the user made or confirmed is theirs: the job never touches it, never merges its two tracks and never asks about them again (1bC-1, 2026-10-11). `recording_a` is the mashup where the label is `mashup-contains`. |
| `grouping_pair` | file_a, file_b (the lower file id first), outcome (duplicate / cut / rework / review / different), reasons (JSON: the rules' reason codes with their parameters, the deciding one first), link_label, link_source, link_first, state (waiting / merged / both_in_library / set_disagrees / linked / open / in_one_track / decided / apart), decided_at | What the grouping rules said about two files (1.4; 1bC-1, 2026-10-11): every pair the job decided that isn't "different", plus the "different" pairs it had to decide inside a set of duplicates. `state` says where the pair stands after the last grouping pass; the pairs for Review are the ones that are `open`, `both_in_library`, `set_disagrees` or `in_one_track`. A `waiting` pair is a duplicate whose set isn't decided in full yet, or one of whose two files isn't on disk. Derived state, not in the operation log. The database deletes a file's rows when its fingerprint or the audio it holds changes, and with the file row. |
| `grouping_file` | file_id, placed_key, decided_as, rev, quality_verdict | What grouping knows about one file (1bC-1). `placed_key` is the audio the file held when it was put in its track (its `audio_hash`, or its `blake3` when it has none): a file that holds other audio now leaves its track, and nothing else ever splits one. `decided_as` is the version of the rules and of the comparison all of the file's pairs were decided under, NULL when the file is **due**; the database clears it when the file's fingerprint, audio or names (tags, file name) change and when a missing file comes back, and it's set only in the transaction that writes the last of the file's pairs. `quality_verdict` is the quality verdict (`file_verdict`) the file's pairs were decided at, written with `decided_as`; the grouping job clears `decided_as` when the file's settled verdict now differs from it in being `truncated` or not (a cut-off copy is a duplicate of the whole file, 1.4), at the end of a quality job and at the start of a deciding job, and never on a verdict that is out of date (1bC-6). A due file is decided once it's ready: it has no fingerprint, or its fingerprint is of the audio it holds now (`fingerprint_audio_hash` is its `audio_hash`, or the fingerprint stage's `file_stage` row is done at its size and modified time) and matching has been through it. Derived state. |
| `grouping_decision` | key_a, key_b (each a file's `audio_hash`, or its `blake3` when it has none; the lower first), decision (apart / same), decided_at | What the user decided about two tracks (1.4; 1bC-1, 2026-10-11), kept by what the files hold: one row for every pair of audio between the two tracks when the decision was made. So it covers every pair of files between the two tracks, survives a retag, a move, and a music folder removed and added again, and holds when the file it was made on is overwritten or gone. A file with no `audio_hash` is named by its `blake3`, which a tag rewrite changes: a decision resting on such a file alone is lost with one. `apart`: never merged, never asked again; for two files that already share a track it changes nothing. `same`: duplicates whatever the rules say, and a link the job made between the two tracks goes (two Library tracks still aren't merged by the job). A decision between a piece of audio and itself is between the Library tracks holding it. A rowid table with a unique pair, so the operation log can record it; written only through `grouping::remember` (Review, 1bE). |
| `review_decision` | kind (dismissed_better / keep_anyway / want_better), linked_key, other_key (each a file's `audio_hash`, or its `blake3` when it has none; `other_key` is the dismissed better file's, empty for the other two kinds), decided_at | What the user decided in Review about a Library track's linked file (1.12; 1bE-5, 1bE-7, migration 0027), kept by what the files hold, as `grouping_decision` is: it survives a rescan, a retag, a move, and a music folder removed and added again, and it doesn't cover a file that is encoded again (other audio, so another key: a new item). `dismissed_better`: "Better file available" was dismissed for the audio the track is linked to and that one better file; the track is then offered its next better file that isn't dismissed (1.12), and a database trigger makes the tracks linked to that audio due for the roles job when such a row is made or deleted. `keep_anyway`: the flagged linked file is kept; the verdict and the ranking don't change. `want_better`: the user wants the track in better quality; nothing reads it until the wishlist (2.3). The Review queue leaves out what a row answers. A rowid table with a unique (kind, linked_key, other_key), so the operation log can record it: every row is made by one user action and taken away by its Undo. |
| `search_track` | title, artist, album, remixer, label, genre, comment, file_name (an FTS5 table, one row per track, its rowid the track's id) | The full-text search index (1.11; 1cA-6, migration 0024). **Derived state:** a row holds the text the lists show for the track (its title and artist as the lists work them out, so rekordbox's for a Library track it knows; the other fields from the same source) and the names of all its files without folder or extension. It is contentless (the text is not kept), ignores capitals and accents, and is searched only through a query built by `search::find`, never from raw user text. Hidden files are not marked in it: `find` reads them from `file` and `recording_file` when it runs. The `index` job keeps it current and `search::rebuild` works every row out again. |
| `search_due` | recording_id | The tracks whose `search_track` row is missing or may be out of date (1.11; 1cA-6). Triggers note a track here when a file's tags, name or presence change, a file joins, leaves or moves between tracks, a track appears, goes or is named, or its Library track changes; `-1` stands for every Library track and is left by any change to the rekordbox snapshot. The `index` job works the text out in Rust and removes a track in the transaction that writes its row. Derived state, not in the operation log. |
| `library_track` | recording_id, kind (copy / linked), managed rel_path (copy), linked file_id (linked), format, source_file_id (nullable), source_status (ok / missing), **last_sent_location**, last_exported_at | Deleted only by an explicit in-app action (the track's crate entries, sync bases and conflicts go with it, in the same undoable operation). `source_status` follows the linked file's `present`, kept current by database triggers whichever code changes it; a file on an unplugged drive stays present, so its track isn't flagged. The rekordbox TrackID isn't stored, because it's reassigned on import (§5.2). |
| `library_removal` | recording_id (unique), removed_at, last_sent_location, last_exported_at | A track the user removed from the Library and hasn't added back. Made in the removal's operation (undo takes it away) and deleted when the track is added back. The offer to add rekordbox's tracks leaves these out (1.3), and a send lists the ones with a `last_sent_location` for the user to remove in rekordbox by hand (1.9 rule 7). When two tracks merge, the record moves to the surviving track (dropped if that track is in the Library; the earlier `removed_at` wins if both had one). A removed track left with no files is deleted with its record, unless it was sent before. |
| `replaced_entry` | recording_id (the track; not a foreign key), location (the entry's `Location` as rekordbox wrote it), location_key (unique), replaced_at | A rekordbox entry left behind when the user switched a Library track's linked file in Review and the track is no longer sent as that entry (1.12, 1.9 rule 7; 1bE-7, migration 0027). Made in the switch's own operation, so Undo takes it away. A send never reads it. It is listed with the tracks to remove in rekordbox while a read holds an entry at that `Location` and no Library track is sent as it. It stays when the track is removed from the Library or merged into another: rekordbox still holds the entry. |
| `sent_playlist` | path (names from the top-level folder down, as last sent), path_key (the names normalized as the writer compares siblings), kind (folder / playlist), seen, sent_at | Every folder and playlist a send has written under `Crates` and `Playlists`, kept across sends (written by `record_send` in the send's transaction). A path is marked `seen` once a read shows it in rekordbox and deleted when a later read no longer has it; one never seen stays. It's how the app tells what it put in rekordbox from what the user made there, so only the former is ever listed for deletion (1.9 rule 6). Not in the operation log. |
| `rekordbox_track` | TrackID, location, location_key, matched file_id and recording_id, relink method, confidence, **relink_probable** and relink_audio_hash, bpm, key, beatgrid, cues, play_count, last_played, rating, color, comments, My Tags, playlists, read_at | A read-only snapshot, refreshed on every read. The TrackID is valid within this read only. `relink_probable` marks a match that isn't trusted yet (filename only, a unique duration or a fingerprint duplicate no title tag agrees with, or a confirmation whose file's audio changed): its file is taken, but no rekordbox data is attached until the user confirms it, and a confirmation (a `relink` row) re-applies it as trusted (1aC-5, 2026-09-30). `relink_audio_hash` is the matched file's `audio_hash` when the match was made (only a hash that's current for the file), NULL until that's known; it's what a match kept between runs is checked against (1.2; 1aD-1, 2026-10-01). `recording_id` is derived from the matched file: grouping keeps it in sync (and clears it on rows with no match), and relink sets it at match time. `my_tags` is parsed at read time from the `/* … */` block at the end of Comments (`rekordbox::my_tags`; `Comments` itself stays as read; several tags are separated by " / ", confirmed in rekordbox 7 on 2026-10-01). Only BPM and key are copied (to `analysis`); the rest is read per track from here (`attach::for_recording`). |
| `relink` | location_key (rekordbox Location decoded by hand, NFC), file_key, file_id (a pointer, nullable), verdict (same / not_same), method, confidence, audio_hash, confirmed_at | What the user said about a rekordbox entry's file, kept apart from the `rekordbox_track` snapshot so it survives every fresh read and is re-applied by Location (0D-1 review, 2026-09-28). `same` is a confirmed relink, one per Location. `audio_hash` is the file's `audio_hash` when the user confirmed it; if the file holds other audio now, the confirmation is applied as probable until confirmed again, and never deleted by relink (1.2; 1aD-1, 2026-09-30). `not_same` is "this audio is not that entry" (1bE-6, 2026-10-13), one per Location and audio: relink never matches the entry to a file holding that audio again in steps 2 to 5 (1.2). Both are kept by what the file held (`file_key`: its `audio_hash`, or its `blake3` when it has none), as `review_decision` is (foreman's ruling, 2026-10-14), so they survive a move, a rename, a rescan and the music folder removed and added again; a file encoded again holds other audio, another question. `file_id` only points at the file the user looked at while that row exists: what a confirmation is applied to and checked against for other audio. It never holds the file in place: removing a music folder is never refused for a `relink` row (the pointer goes, `ON DELETE SET NULL`), and a confirmation without its file is applied to a file on disk with its audio. "Not the same" needs known audio: a file not hashed yet can't be refused. A confirmation made before its file was hashed is kept by the file alone until the relink job records its `file_key`. A rowid table since migration 0028 (it was keyed by the Location alone), so the operation log can record it: a confirmation, a "not the same" and a withdrawal are each one undoable operation (`relink::decide`). Its ids are never reused, and `audio_hash`, `file_key` and `file_id` are derived columns to Undo: the relink job fills the first two in outside the log once the file is hashed, and a folder removal lets go of the third (see `operation` / `change`); Undo of a decision works after either. |
| `sync_base` | library_track_id, field, value, synced_at | Tells "app changed", "rekordbox changed" and "both changed" apart. A rowid table with a unique (library_track_id, field), so the operation log can record it. `field` is the rekordbox XML attribute name and the value is the text sent (`Location` in its encoded form); `TrackID` and the analysis fields are never recorded (1.9 rule 8). Its ids are never reused, because a send inserts rows outside the operation log (see `operation` / `change`). |
| `conflict` | library_track_id, field, app_value, rekordbox_value, base_value, status (open / kept_app / kept_rekordbox / edited), resolved_value, detected_at, resolved_at | The review queue (1.10): a field both sides changed, to different values, with every side's value (the database refuses a row whose three values aren't all different). `field` is the rekordbox XML attribute name, as in `sync_base`. `base_value` is NULL when the field was never sent; `rekordbox_value` is empty when rekordbox's entry has no such attribute. One open conflict per track and field; a settled one stays, with all three values and, for `edited`, the value given by hand. The database refuses one on an analysis field (`AverageBpm`, `Tonality`, `TEMPO`, `POSITION_MARK`). Its ids are never reused, because detecting conflicts inserts rows outside the operation log (see `operation` / `change`; 1cA-2, 2026-10-12). |
| `tag` / `tag_group` | name, group (vibe / role / situation / vocal…), color, rekordbox mapping | |
| `genre` / `genre_alias` | tree (parent_id), aliases | |
| `crate` | kind (static / smart), rules JSON, folder tree, notes | Exported inside the `Crates` folder. Smart crates are exported as static playlists. |
| `crate_entry` | crate_id, library_track_id, kind (member / always_include / never_include) | Smart crates use the always/never lists (3.3). A rowid table with a unique (crate_id, library_track_id), so the operation log can record it. |
| `playlist` / `playlist_entry` | order, target energy, notes, origin (app / rekordbox) | Exported inside the `Playlists` folder. |
| `play_source` | kind (rekordbox_db / history_m3u8 / usb_pdb / xml_playcount), label, device, imported_at, dated (bool) | USB sessions are numbered but undated (§5.4). |
| `play_event` | recording_id (nullable), raw artist/title/path, play_source_id, position, played_at (nullable), prev_play_event_id | A null `recording_id` means a partner's track you don't own. |
| `session` | name, date, venue?, notes; many play_sources | The UI calls it "gig" or "set". |
| `wishlist` / `wish_item` | source (Spotify / CSV / paste / session), raw artist/title, matched recording_id, source_play_event_id (nullable), status | |
| `inbox_batch` | kind (store order / pool pack / zip / loose), label, arrived_at | 2.1 grouping and "Add all". |
| `inbox_item` | file_id, batch_id, status (new / not_now / hidden / added), label (new / better_file / other_cut / rework / on_wishlist / already_have), reason, arrived_at | |
| `job` | kind, target, priority, status, progress, error, attempts, timestamps | Persisted so a job the last run left running starts again after a crash (0.1; 1cA-13). |
| `setting` / `service_optin` | key/value; service, enabled, enabled_at | The network gate checks `service_optin`. |
| `operation` / `change` | carried over from musicmanager; `change` records an action (set / insert / delete) so a real NULL value is never mistaken for a created or removed row | Undo and history. `operation` stores a kind plus details, never UI text. **Undo is multi-step** (owner decisions 2026-10-02): it takes back the newest operation not yet undone (`undone_at`), one per step, with no limit and across restarts; there is no redo. A step whose rows have changed since is refused with its reason, nothing changes, and undo doesn't skip past it to older steps. A send isn't an operation: undoing the add of a track that was sent since is refused; undoing a removal after a send works. The same holds for a switch of a linked file that made the track a new one for rekordbox (1.12): once the track was sent since, its undo is refused, with its own reason. A row is named by its id, so a logged table that code outside the log writes never reuses ids (`AUTOINCREMENT`: `sync_base`, which a send writes, `conflict`, which detecting conflicts writes, and `relink`, where the relink job fills in a missing `audio_hash`). **Derived columns** (one list, `ops::DERIVED_FIELDS`: `library_track.source_status`, which follows the linked file, and `relink.audio_hash`, `relink.file_key` and `relink.file_id`): something outside the log keeps them current, so undoing an insert doesn't count a change to one as "changed since" (1bE-6, 2026-10-13). |
| `embedding` | recording_id, model_id, vector | 3.8. |

**When tables arrive:** each migration is added in the phase that first needs it (§1 principles).
- **Phase 0–1:** `volume`, `music_folder`, `file`, `fingerprint_match`, `fingerprint_matched`, `file_quality`, `file_verdict`, `recording`, `recording_file`, `version_link`, `grouping_pair`, `grouping_file`, `grouping_decision`, `review_decision`, `search_track`, `search_due`, `analysis`, `library_track`, `rekordbox_track`, `relink`, `library_removal`, `replaced_entry`, `sent_playlist`, `sync_base`, `conflict`, `crate`, `crate_entry`, `operation`/`change`, `job`, `setting`, `service_optin`.
- **Phase 2:** `inbox_batch`, `inbox_item`, `wishlist`, `wish_item`.
- **Phase 3:** `tag`/`tag_group`, `genre`/`genre_alias` (until then, genre is a plain canonical tag), `playlist`/`playlist_entry`, `play_source`, `play_event`, `session`, `embedding`.

**Provenance:** every field records where its value came from and how confident that is. BPM and key from rekordbox are `analysis` rows with `source = 'rekordbox'`; the rest of rekordbox's data keeps its provenance by living in `rekordbox_track` (1aD-4). Review can then show "Key 8A (rekordbox) vs 9A (file tag)" instead of silently picking one, and every suggestion can state its reason.

---

## 3. Phases

| Phase | Theme | Outcome |
|---|---|---|
| **0** | Foundations | Private repo, CI (Windows tests; macOS/Linux builds), the architecture pieces that are cheap now and costly later, and the ports Phase 1 needs. Spikes are done (0.4). |
| **1a** | Prove the loop | Scan music folders, read rekordbox with relink, start a Library of linked tracks, send to rekordbox, and prove it with a real cycle on an installed build (the gig test comes at the end of Phase 1, in 1c). The gate tests T1 and T3 (§7) come first. The MVP never writes an audio file. |
| **1b** | Order out of the mess | Fingerprint grouping, version awareness, quality verdicts, best file, Review. |
| **1c** | Round trip and daily use: **the MVP** | Conflict review, track browser and keyboard layer, Overview, crates, 100k performance pass. (Going public moved up to Stage 1aP, 2026-09-30.) |
| **2** | Getting tracks in | Managed copies and upgrades, Inbox, identification and tag cleanup, wishlists, store links, auto tick-off. |
| **3** | Set prep | Tags, genre taxonomy, smart crates, "what's next", playlists with an energy arc, play history, local estimates, local ML. |
| **4** | Public v1 (Windows) | Portability bundle, onboarding polish, diagnostics, docs. |
| Later | Backlog | See the backlog below. |

The MVP is the full round trip: a real Library, started from rekordbox or from nothing, goes to rekordbox and comes back. Managed copies (2.6), identification (2.2), the genre taxonomy (3.2) and local estimates (3.7) wait until after it. Disk cleanup is in the backlog. Complexity sizes (S/M/L/XL) describe relative size, not schedule. There are no time estimates.

---

## 4. Features

### Phase 0: Foundations

#### 0.1 Repo and architecture
- **Stack:** Tauri 2 + Rust + React/TS.
- **Bindings:** `tauri-specta` generates the TS bindings from the Rust types. Never hand-edit the generated files.
- **Database:** one writer connection plus a pool of WAL read connections. No global `Mutex<Connection>`.
- **Job queue:** typed (scan, read, hash, fingerprint, read_rekordbox, analyze, embed, convert, export), with priorities, progress events and cancellation, persisted in the `job` table. A job the last run left running starts again at the next start, and work that is due while no job is queued is asked for then (1cA-13). Every long operation goes through it.
- **Frontend state:** TanStack Query for server state, Zustand for UI state, and one route per workflow stage. No monolithic `App.tsx`.
- **i18n:** `i18next` for all strings, with a lint rule forbidding raw strings in JSX.
- **Theme:** tokens only, with no hardcoded colors in components.
- **Network gate:** one `net` module checks `service_optin`. A test fails if any other module makes HTTP calls.
- **Migrations:** append-only. A test fails if an applied migration is edited.
- **Write guard:** only the Library module gets a writable file handle (§5.6).
- **Suggestions:** every suggestion type carries a reason string. A test fails if one reaches the UI without it.
- **Complexity:** M. **Reuse:** musicmanager's DB layer patterns and its `fields.rs` single-source-of-truth pattern.

#### 0.2 Port proven modules
These are copies, not a shared library. Each is ported in the phase that first needs it. Nothing is ported back to musicmanager (§1.1).

| Module | From | When | Changes |
|---|---|---|---|
| Tag **reading** | `tags/mod.rs` | Phase 0 | Lenient reading (§5.5). |
| Camelot key normalization | `tags/key.rs` | Phase 0 | As is. |
| Operation log / undo | `ops/mod.rs` | Phase 0 | Multi-step undo added in 1b (1bA-12): one Undo in the top bar that names the action, and Ctrl+Z. |
| Playlist tree | `playlists.rs` | 1c | Becomes the crate folder tree (and the playlist tree in Phase 3). |
| Tag **writing**, preserving unknown frames | `tags/write.rs` | Phase 2 | With the byte-preservation tests, sorted frame order (§5.6) and ID3v2.3. Writes only ever target Library copies. |
| MP3 encode / transcode (never upconvert) | `transcode.rs`, `encode.rs` | Phase 2 | Add an AIFF target. |
| Rules / policy engine | `libraries/policy.rs` | Phase 2 | Rules apply when a managed copy is made. |
| Naming templates | `libraries/naming.rs` | Phase 2 | Add a UI for choosing the template. |
| MusicBrainz / Discogs / Cover Art clients | `integrations/` | Phase 2 | Behind the network gate and a rate-limited queue. |
| Updater + release script | `useUpdater.ts`, `release.mjs` | Phase 4 | Windows only. Separate release repo and endpoint. Until then, builds are local. |

#### 0.3 Portability groundwork
- Identify volumes by UUID or serial, never by drive letter (`sysinfo` or platform APIs).
- Store each path twice: the **exact on-disk relative path** (used to open the file) and an **NFC match key** (used to compare, dedupe and match rekordbox `Location`). NTFS doesn't normalize names, so an NFD name (e.g. copied from a Mac) and its NFC twin can be two different files in one folder; storing only NFC would open the wrong file or none. Decided 2026-09-28 (0C-9).
- CI: Windows runs the full tests. macOS and Linux run a build plus the platform-neutral tests, weekly and on demand rather than on every push, to save Actions minutes (0E-13).
- **Complexity:** S–M.

#### 0.4 Spike verdicts (done 2026-09-27, rekordbox 7.2.19)
Spike code is in `spikes/`. The evidence is in `spikes/results/`.

| Spike | Verdict |
|---|---|
| XML round trip | ✅ Works. Import behavior and its traps are in §5.2. |
| Detecting rekordbox edits | ✅ Diff two XML exports and match by Location. |
| History | ✅ Three routes, none complete alone (3.6). |
| My Tag | ⚠️ Not in the XML by default. Comes in via the "Add My Tag to the comments" setting. Creating My Tags via XML is untested (§7 T5). |
| Fingerprinting | ✅ `rusty-chromaprint` 0.3 (pure Rust, MIT). All 13 "same" pairs matched strongly after relabeling pair #42, and no "different" pair matched. About 1 s per track per core, including decode. Length can't separate duplicates from versions. Thresholds are in 1.4. |
| Local BPM / key | BPM ✅ within ±0.5 of rekordbox on about 99% of tracks, about 0.5 s per track. Key ❌ about 55% exact, 77% exact-or-adjacent → not shown (3.7). |
| Fake-320 detector | ✅ Spectral cutoff, about 0.15 s per file. *(ref)*: 1.7% of "320" MP3s and 5.5% of lossless files are suspect. Thresholds are in 1.6. |
| Local ML | ✅ EfficientAT `mn10_as` (MIT) and LAION-CLAP music (Apache-2.0/CC0). Essentia, MERT and MuQ are ruled out (non-commercial weights). No permissive energy model exists, so energy comes from DSP. |
| Preview formats | ✅ MP3, AAC/M4A, FLAC, WAV, Ogg and Opus play natively (Chromium 152). AIFF and ALAC don't. |

---

### Phase 1: MVP, your first Library
Each feature is tagged with its sub-phase.

#### 1.1 Multi-root in-place scan · 1a
- **What:** add any number of folders or drives. Each is indexed read-only, with incremental re-scans and an optional watcher per root. Every online folder is rechecked at app start and whenever its drive comes back; "Watch for changes" (off by default) adds live updates while the app runs (owner, 2026-09-30).
- **Stages:**
  1. A walk with stat only, so the UI fills in within seconds.
  2. Tags and audio properties.
  3. Hash and fingerprint as background jobs, visible tracks first.
- **Unchanged check:** (size, mtime, file-id), made inside the walk before it updates a row. A different size, or a different file id together with a different mtime, is a change: the stages redo the file. On an mtime-only change, or a new file id with the same size and mtime (restores, sync tools and shares with unstable ids), the walk compares a **partial hash** (`file.partial_hash`, stored by the hash stage): the file's size, every byte of metadata (all bytes outside the audio) and the first and last 64 KiB of the audio. Equal means the content is the same, so the row's mtime and the stages' recorded mtimes move on and nothing is redone; different, or none stored (unknown format, Ogg, over 4 MiB of metadata), means changed. A same-length edit in the middle of the audio, away from both ends, is the known blind spot. rekordbox bumps mtimes (§5.1); a tag rewrite still counts as a change, since the metadata really changed. These comparisons are the only reads a walk makes of a file (1aC-1, 2026-09-29).
- **Offline volumes:** they stay in the index, greyed out ("Drive not connected"), and their files stay present. A file is set to `present = false` only when a walk could list its folder and the file wasn't there, never under a folder the walk couldn't list; it's never dropped. Drives coming and going are noticed through WM_DEVICECHANGE, and a walk stops if its drive is swapped mid-walk (same letter, other serial or GUID). A cloned backup the library knows is never taken for the original: plugged in alone, it keeps its own identity and the original's folders show offline (1aB-9, 2026-09-29).
- **A folder is scanned when it is added,** from whichever screen adds it: the add command queues the scan itself (1aG-10, 2026-10-02). "Scan again" on a folder's row scans it on request.
- **After a kill or crash** (1aG-10, 2026-10-02): a job the last run left running is put back in the queue at startup and runs again from the top, if its kind can (the scan stages, quality checks, grouping, relink, attach, a rekordbox read); any other ends as failed. Left as it was, such a job blocked its scan stage for good. Send jobs are still dropped at startup (1.9). A job put back does only what is still due, and that is all the resuming there is (1cA-13, 2026-10-12): no job keeps a cursor. A job the user cancelled before the kill is ended as cancelled, not put back. At startup, once the queue is up, each stage that can have work due with no job queued is asked once and queues its job if so (the hashes, grouping, the fingerprints, quality, matching, deciding, the file roles, the search index); the reads come from the catch-up scan every online music folder gets at start. Relink and attach are not asked: an app that dies after a rekordbox read ends but before their jobs are queued waits for the next read.
- **Fingerprint progress is the whole library's** (DOGFOOD C1-2, 2026-10-10): a fingerprint job over every due file counts the files settled before it started as done, so a second pass for files that became ready later carries on from the first's figure instead of starting at 0%. The figure can still dip when a scan finds more files. A job for named files (the tracks on screen) reports on those alone.
- **Scanning rules:** see §5.5 (skip AppleDouble files, read tags leniently, sniff the format, flag broken files).
- **OneDrive placeholders:** reading one triggers a download. Detect the placeholder attributes and skip unless the user opts in. Onboarding explains the risk (4.2).
- **Target:** first results in under 5 s on 100k files, and a full index without fingerprints in minutes.
- **Measured** (2026-09-30, 1aC-7 and 1aC-12; 100k generated files on the owner's laptop, release build, Windows Security on): first results in 0.3 s. Finding and reading every file: 6.7 min on a never-read tree, 32 s once the files are cached. Reading uses up to 4 threads shared with fingerprinting, and the cold time is disk plus antivirus on first open; serial reading took 15 min. Hashes take about 1 min, a rescan with nothing changed 12 s, peak memory about 100 MB.
- **Complexity:** M. **Reuse:** `reconcile.rs`, the `walkdir` scanning, tag reading.

#### 1.2 Read your rekordbox collection · 1a
- **What:** read the rekordbox 7 collection without changing it, and match each entry to a file in the music folders. rekordbox entries whose files are in no music folder are listed, with an offer to add their folder.
- **Carried to the track (with provenance):** BPM, key, beatgrid, cues, play count and last played, rating, color, comments, My Tags and playlist membership. It follows the track, not just the file.
- **When:** at first run, after each send (1.9), and on demand. This is also how rekordbox's analysis of new Library tracks comes back.
- **Sources:**
  - **Always:** the XML export. The user saves it via File → Export Collection, and the app asks for the file or watches for it.
  - My Tags come in via the "Add My Tag to the comments" setting.
  - **Optional:** gig sticks (`export.pdb`).
- **Relink** (paths go stale when DJs change computers or accounts). Try in order:
  1. The path is still valid.
  2. Filename + duration (±0.5 s, read from the file). The XML's `TotalTime` is truncated to whole seconds, so a `TotalTime` of T matches a file lasting from T − 0.5 s up to (not including) T + 1.5 s (1aC-3, 2026-09-30).
  3. A unique duration within the candidate set: the files directly in the folder the Location's folder still names, plus the folders its neighbours (tracks from the same rekordbox folder, matched by path, filename + duration or a gig stick, or confirmed) were matched into; at most 50 files, and none of unknown duration. Accepted only when a title tag agrees with rekordbox's Name (featuring credits, case, punctuation and spacing folded; bracket contents kept); otherwise **probable**, like step 5 (1aC-5, 2026-09-30).
  4. The acoustic fingerprint (1.4). A track whose file is gone has no audio to fingerprint, so the evidence is the file record the scan kept when the file went (the one at the Location, or the one of a confirmed relink): a file nobody else holds with the same `audio_hash` is accepted as the same audio (only while that file's hash is current: a file that changed since it was hashed isn't trusted; and the gone file's own hash and fingerprint count only if they were current when it went). Failing that, another file of the gone file's track that is on disk (on a drive that isn't plugged in too, as for the same audio), on grouping's own current answer for the two files: a stored duplicate pair of the two, both files decided under the current rules and sitting in the track with the audio they hold (the gone file: held when it went), the mate's hash current. Accepted only when it lasts within 1 s of the gone file (the entry's cues and grid are positions in time, so a mate of another length, or where either length isn't known, is **probable** at most) and a title tag agrees with rekordbox's Name; otherwise **probable**, like a fingerprint duplicate: grouping compared the two files' names, never rekordbox's. A mate overwritten since, one due to be decided again, and one that only shares the track (never compared with the gone file, or kept there though the rules no longer call it a duplicate) aren't matched. Two limits: once the mate has been decided again while the gone file is away, their pair goes with that decision; and every new version of the grouping rules ends this for the files already gone, which can't be decided again. The fingerprint duplicate is then what remains (1bC-7, 2026-10-12). Failing that, the one fingerprint duplicate among all files lasting within 1 s of it, accepted only when a title tag agrees with rekordbox's Name (a Clean and a Dirty cut fingerprint alike), otherwise **probable**; a duplicate another track holds still counts, so the one left over isn't "the one". Two rekordbox entries never share the one file that's left. A drive that isn't plugged in is never taken for gone files: step 1 handles it. A match is never made on a partial comparison: a track waits while a file of about its length hasn't been fingerprinted yet, and one relink run compares at most 20,000 fingerprint pairs (it runs on the database's one writer; about 0.8 ms a pair for five-minute tracks in a debug build, measured in review), tracks with the fewest candidates first, leaving the rest undecided for that run and counted in the log. A waiting or undecided track still lays claim to every file of about its length, so none of them is given to another track meanwhile (1aD-1, 2026-10-01).
  5. Filename only: the only file with that name, counted over every file, and wanted by no other missing track. A file whose known duration doesn't fit `TotalTime` (step 2's window) isn't guessed: the same name at another length is more likely another cut; an unknown duration on either side doesn't block it. Always **probable**, and queued in Review as "rekordbox match uncertain" before any rekordbox data is attached (1aD-2, 2026-10-01).
  6. A connected gig stick as a recovery source.
  
  **Never match on size:** rekordbox rewrites tags, which changes the size, and the XML's `Size` can be stale (§5.3). Use `audio_hash` as a confirming check where available. Whatever is left is listed as "missing" with its last known path.

  **Every relink run decides steps 1–5 again from scratch**, so the result depends only on what's on disk and in the rekordbox snapshot, never on earlier runs: running again, or reading rekordbox again, with nothing changed gives the same matches. Only a confirmed relink (and, later, a gig-stick match) is kept between runs, and what the user said is not an entry's file.

  **The audio a match was made to:** a confirmed relink records the matched file's `audio_hash` (so will a gig-stick match). If the file later holds other audio (replaced under the same name), a confirmed relink is kept and still holds its file, but counts as probable, with no rekordbox data attached, until the user confirms again (owner, 2026-09-30); a kept gig-stick match is dropped and decided again. A tag-only edit changes nothing, and a hash that's missing or not current means unknown, never changed; only a current hash is ever recorded.

  **Confirm, "Not the same", withdraw** (1bE-6, 2026-10-13; `relink::decide`). A probable match waits in Review (1.12) until the user answers; each answer is one undoable operation, and a relink run follows it and its Undo. *Confirm* stores the confirmation (above); confirming again after the audio changed records the audio of now. *Not the same* stores that the audio the file holds is not that entry's: no run matches the entry to a file holding that audio again in steps 2 to 5, wherever the file is now (moved, renamed, its music folder added again), and a kept gig-stick match to one is dropped. Step 1 isn't affected (a file at the Location's own path is the file rekordbox itself opens, and is never probable). The step that would have made the match makes none for that entry, the entry goes on through the later steps, and another file may be proposed: a new item. The refused file still counts wherever files are counted (two files with one name are still ambiguous), so saying no can only take a match away. A file encoded again holds other audio, and is asked about again. Confirming takes back a "not the same" about that audio, and "not the same" takes back the confirmation of that file; every other decision about the Location stays. Each answer and its rows of the snapshot are written in one transaction. *Withdrawing* a confirmation deletes it, and the entry is decided again from scratch; no screen offers it yet (the Details panel will, 1cB), and it is what Undo of a confirmation amounts to. The entry's rows in the snapshot follow each answer at once (confirmed: trusted; not the same: no match; withdrawn: probable again), so Review is right before the relink run ends; that run decides them again.

  **Among files that are equally right** (exact copies that fit in step 2, 3 or 5, or the candidates of one step-4 rule), the track's best file is matched, else the lowest file id; and the file a Library track is linked to instead, wherever a match to it gets the same trust (1bE-10, 2026-10-12). A Library track is sent as rekordbox's entry only while the entry is matched to its linked file (1.9), so a copy that turns up later, and ranks ahead, can't pull the entry off it. It is a tie-break only: equally right includes equally trusted. Steps 3 and 4 trust a match on the file's own title tag and length, so being linked never turns a match that would be trusted into a probable one, or a probable one into a trusted one, and an entry none of whose candidates is linked gets the file it always got. Relink itself never changes a Library track's linked file; the step that follows it in the same job does (1.8).
- **Skip what isn't a track:** deleted rows, rekordbox's demo tracks and samples, the "CUE Analysis Playlist", and streaming entries, which are shown as "streaming, no file" (§5.3). Streaming tracks are out of scope beyond that: they're never linked, sent or checked in the behavior check (owner, 2026-09-29).
- **Parsing:** see §5.3.
- **Complexity:** M, plus relink (M).

#### 1.3 Start your Library · 1a
- **First run:** point the app at your music folders, then choose:
  - **Start from rekordbox (main):** your whole collection or chosen playlists become **linked** Library tracks (1.8). There are no new rekordbox entries, no re-encoding and no file writes, and the Library looks like what you already play. Their duplicates and problems queue in Review (1b).
  - **Start fresh:** an empty Library, for new DJs, DJs switching software, or a clean start. Tracks you add are linked to their files where they are.
- **Fragile locations:** a Library track whose file lives in Downloads, a temp folder, an external drive or a network drive (owner, 2026-10-01) gets a visible warning with its reason ("this file is in Downloads; if you clear Downloads, the track goes missing"). Making a safe copy arrives in Phase 2 (2.6).
- **Tracks whose file is missing** when you start from rekordbox don't become Library tracks yet. They wait in Review's **Missing** list (1aD-5) with their rekordbox data intact (BPM, key, cues, play counts, playlists), until a file is found and married to them by relink; then they're offered for the Library like any other rekordbox track (below). From Phase 2 they can also go on the wishlist. Decided 2026-09-28.
- **Adding rekordbox tracks is always offered, never automatic** (owner, 2026-10-01). After any rekordbox read, the app says how many rekordbox tracks with a trusted file match aren't in the Library and offers to add them. "Start from rekordbox" is the first use of that offer, for the whole collection or for chosen playlists; later reads use the same offer, however the Library was started. Left out of the offer: tracks whose match is still unconfirmed (until it's confirmed), tracks whose file is missing, and tracks the user removed from the Library in the app (they can still be added back by hand).
- The Library folder and format are chosen when managed copies arrive (2.6). Importing playlists with their order comes in 3.5.
- **Complexity:** S–M.

#### 1.4 Acoustic fingerprinting + duplicate grouping · 1b
- **How:** exact blake3 or audio-hash matches are the fast path. Otherwise, index fingerprints by blocks to find candidate pairs, compare them, and group.
- **Decision rule:**
  - **Duplicate:** matched coverage ≥ 90% on *both* files and a difference score ≤ 4.
  - **The version check (1.5) can veto** a merge, because Clean vs Dirty fingerprint as 100% identical.
  - **One-sided coverage** (a Radio Edit found inside an Extended mix) means a **cut**: link it, don't merge. Unless the inner file is a cut-off copy (verdict `truncated`, 1.6) that begins where the other begins, under the same names and version words: then it is a duplicate, and the best file is the whole one (1bC-6, 2026-10-11).
  - **Low or no coverage:** names decide whether it's a rework link.
  - **Two Library tracks are never merged by the job** (owner decision, 2026-10-11). When two tracks that are both in the Library turn out to be the same recording (rekordbox had two entries for it, or both were added before their fingerprints were compared), they stay two tracks and Review shows one item for the pair. The user decides there: keep both, or merge into one, choosing which stays; the crates of both then hold the one that stays, and the other goes on the list to remove in rekordbox. Nothing in a crate or a send changes before that. Outside the Library, duplicates merge without asking.
  - **A decision sticks.** "These aren't the same" (a split) and "keep both" are remembered, so a later run doesn't merge the pair again or ask again. A pair the rules send to review stays apart until the user decides. A decision names what the two files hold, not the files, and covers the two tracks: the user is asked once per pair of tracks, never once per pair of files (`grouping_decision`, §2). It's kept for every pair of audio between the two tracks, so it still holds when the file it was made on is overwritten or its music folder is removed. "Apart" for two files that already share a track changes nothing: taking a file out of a track is an action of its own (Review, 1bE).
  - **A track is files that are duplicates pair by pair** (1bC-1, 2026-10-11). A set of files joined by duplicate pairs merges only if every pair in it is a duplicate; one stored pair that is a cut, a rework, a review or different, and nothing in the set merges: its duplicate pairs go to Review (`set_disagrees`). A pair in such a set that nobody decided is decided first (compared directly where nobody compared the two), and the set waits until it is. A set also waits, and never merges, while one of its files is due to be decided again, so a tag rewrite on one file doesn't release a held set. A file that isn't on disk keeps its track and takes no part: nothing can be decided about it, so it holds no set back, and a duplicate pair of its whose files are in two tracks waits for it to come back (`waiting`, not shown in Review). A set of more than 50 files holding several kinds of audio is held as if it disagreed. Files with the very same `audio_hash` are together without any of this.
  - **A merge is never undone by the job.** A new file or a retag never splits a track; only a file that holds other audio than it joined with leaves: its `audio_hash` changed, or, for a file with none, its `blake3` (so a file with no `audio_hash` leaves on a tag rewrite, and is decided again). A file whose audio changed is not decided, and not asked about as another file's partner, until its fingerprint is of what it holds now: nothing is decided on the fingerprint of audio a file no longer holds. Because merges are kept, a set whose pairs disagree can come out differently depending on the order its files were decided in (two files that merged before anything was stored about a third stay one track), and a fresh database may differ from an old one there. A run over an unchanged database changes nothing, the user's decisions and links included.
  - **The full table** (every combination of audio and names, its outcome and its reason) is in the `grouping::rules` module docs (1bB, 2026-10-11). It settles what the lines above leave open. Files with the very same audio (the same `audio_hash` or `blake3`) are always duplicates, whatever their names say. The same audio under another title is a duplicate, and says the names differ. "One-sided" is at least 90% of exactly one file found in the other, over every alignment, with a difference score ≤ 4. A link made by names alone needs the artists to agree. A file's version labels are those of its title tag and its file name together; with the audio matched, two files whose labels differ by a cut are linked, never merged, and by a rework go to review, whatever their titles. As a last guard, two files with a full audio match are merged without asking only if their names carry the same version words: every label word the parser knows, in brackets or not, anywhere in the title tag or the file name, counted per file (the file's artist is set aside only where a name spells the whole artist out). If the version words differ, a pair the table already links as cuts stays a cut link, and any other goes to review. Identical audio still merges whatever the names. "Original" is no version word here. Not seen: a label word fused to another word ("VeloDirty"), a hyphenated spelling in the middle of a name ("Re-Fix"), a version given by a folder further up than the one a file sits in, a folder word for a file that sits directly in a music folder (the music folder's own name isn't seen), and, in a file with no title tag, a file name whose second part is only a label ("Velo - Radio Edit.mp3" stays artist and title, though its label word is still counted). A pair whose fingerprints were never compared goes to review if its names are the same, and is otherwise decided as if it shared no audio; so before asking, the grouping job compares directly every pair whose names are the same or where one is a mashup naming the other as a part. It finds those pairs by title; a title more than 50 files share ("Intro", "Track 01") is narrowed to the files that also share an artist, and where that is still more than 50 the pairs are skipped: not compared, not stored, left apart, and counted in the run's result. A title with no letter or digit brings no files together.
- **Speed-up:** a short-window fingerprint (e.g. the first 120 s) may be used **only to find candidates**. Duplicate and cut decisions always use full-track fingerprints: one E3 pair only diverges after about 2 minutes.
- **Cost:** measured in 1aB-7 (release build, one core, decode included): about 0.3 s (WAV), 0.4 s (FLAC) and 0.6 s (MP3 320) per minute of audio, so 1.5–3 s for a five-minute track. A large real library (thousands of tracks) takes 1–2 hours of background time on 4 threads. Runs in the background at below-normal priority on a shared budget of at most 4 threads, visible tracks first. The short-window candidate pass is the lever if big libraries need it.
- **Risk:** false merges. Mitigated by version awareness and by always letting the user split a group.
- **Complexity:** L.

#### 1.5 Version awareness · 1b
- **What:** never merge different versions. Link them instead, as one of two kinds (§1.2):
  - **Cuts:** Original ↔ Extended / Radio Edit / Club Edit / Short, Clean ↔ Dirty, Intro / Outro / Intro-Outro, Quick Hit, a plain "(Edit)", and Remaster (with or without a year; two remasters of different years are two cuts, and one that gives no year is the same as any; 2026-10-11).
  - **Reworks:** remix (per remixer; a named mix, "(Some Name Mix)", is a remix by that name unless the words are a cut's name; a "name" made only of ordinary words such as Main, Album, Vocal or DJ is no one's name, so that bracket is not recognized and the pair can't be told), VIP, flip, bootleg (including "(Re-Edit)" and a named edit, "(Some Name Edit)"), rework/refix/reboot, dub, cover (labeled "cover"), live, alternate arrangement, instrumental, acapella (neither is a stand-in for the full track), and mashups (`A x B`, linked to each source they contain).
- **Signals, combined:** the version parsed from title and filename; fingerprint coverage (full both ways = duplicate candidate, one-sided = cut, none = rework or a different song); and credits. The fingerprint alone decides nothing across versions. When signals disagree, the pair goes to "needs review", with the reason shown.
- **Rules from the E3 labels** (all in the test corpus):
  - Length decides nothing.
  - Version words can be the real title (a title that really ends in "(dirty)"). Compare against the other file; never strip blindly.
  - Remixes are often credited only to the remixer.
  - The same title can be different songs, even as remixes.
  - Mashup part names can match unrelated tracks.
  - **Identical tags can hide a different recording**, so the fingerprint can veto a name-based merge. A partial match between files with identical tags goes to review, with both files playable.
  - Strip store and rip junk: HTML entities (`&#40;Original Mix&#41;`), Beatport ID prefixes (`10000001_`), and site-name prefixes or suffixes such as `RipSite.example - ` or `[ example-ripper ]`.
  - Naming conventions vary across pools (each record pool has its own) and bootlegs (`(@handle edit)`, `[Artist Flip]`). Users can suggest patterns through issues.
- **Complexity:** M–L. This is the parsing library the rest of the app leans on, so it needs a large test corpus. The 43 E3 labeled pairs use real library names and aren't published, so they run as a **local** corpus; CI uses synthetic cases derived from the same patterns.

#### 1.6 Quality verification · 1b
- **Verdicts:** `ok` / `low bitrate` / `suspect transcode` / `truncated` / `broken`. A `clipped` verdict is held back (owner decision, 2026-10-10): nothing measures clipping yet, and loud masters clip on purpose.
- **How:** FFT spectral cutoff (the highest frequency within 50 dB of the 2–8 kHz level, from a 60 s sample: the 60 s starting 30 s in, to skip intros and fade-ins, or the first 60 s for a file under 50 s), decoded vs header duration, and decode errors.
- **Thresholds:**
  - MP3 at ≥256 kbps with a cutoff below 17 kHz → `suspect transcode` ("this 320 was made from a ~128 kbps source").
  - 17–19 kHz → shown as "possibly lower quality", not flagged.
  - Lossless with a cutoff below 19.5 kHz → `suspect transcode` ("this FLAC/WAV was made from an MP3").
  - **Bitrate of a lossy file, two levels** (owner decision, 2026-10-10). Below 192 kbps → `low bitrate`: seriously bad, flagged, and the app warns before such a file is added to the Library. From 192 kbps up to full quality → shown as "could be better", not flagged: a note that a better file exists somewhere and a candidate for "Find a better file". Full quality is 320 kbps; for AAC it is 256 kbps, the rate stores sell (foreman's default, the owner may overrule). A lossless file has no bitrate level.
- **How the verdict is reached** (1bB-6, 2026-10-11; the table is in the `quality::verdict` module):
  - One verdict per file, the first that applies: `broken`, then `truncated`, then `suspect transcode`, then `low bitrate`, then `ok`. Everything else that applied is kept as a list of findings.
  - **Rounding room.** Files store an average bitrate, so the lines sit a little under the nominal figures: low below 190 kbps, full quality from 318 kbps (AAC from 240 kbps, because store AAC is variable), and the transcode check from 254 kbps.
  - **Which codecs.** Vorbis, Opus and WMA use MP3's bitrate lines. AC-3, E-AC-3 and Musepack get no bitrate level: no line was set for them.
  - **`truncated` by length:** the audio is shorter than the length the file states by more than 2 s and by more than 1%. An MP3 without a length header states an estimate, so such a file can be flagged wrongly; "Keep anyway" covers it.
  - **A transcode needs room to show:** a file whose sample rate can't carry audio above the cutoff line (32 kHz and below) is not flagged.
  - A few decode errors in a complete file are a note on an `ok` file, not `broken`.
  - Without a measurement only what the file's header shows is judged (bitrate, a read that failed); such a file is never called `ok`.
- Suspect files are reviewed in Review (1.12): "Keep anyway" handles genuinely rolled-off masters, and "Find a better file" sends the track to the wishlist.
- **Complexity:** M.

#### 1.7 Best-file picker · 1b
- **Ranking** (`best_file`): each step only breaks the ties of the steps before it. In order: the file is on disk (a file on an unplugged drive is); the quality verdict (ok, not checked, flagged, truncated, broken; low bitrate and suspect transcode are one tier); plays in the DJ app (foreman's default, the owner may overrule); the measured cutoff, not the header bitrate; fewer decode errors; format (owner decision, 2026-10-11): lossless over lossy, then among lossy files full quality for the codec first (foreman's default, the owner may overrule), then FLAC, WAV and AIFF, float WAV, ALAC, WavPack, APE and MP3, AAC, Vorbis, Opus, WMA, then header bitrate; tag completeness; embedded art; whether rekordbox knows the file; location (scan folder over Inbox, internal over external over network, folder added first; whether a drive is connected right now does not count in the stored best file, so plugging a drive in or out never moves it); last, the lower file id. Every suggestion states its reason.
- **Audio and tags are picked separately.** The best file decides only the audio. Each tag and the cover art come from the best-ranked of the track's files that has a value, including extra files; there is no majority vote. When files disagree, the app picks one by rule and Review shows the disagreement.
- **Linked tracks:** their audio *is* the file rekordbox uses, so in the MVP a better duplicate shows as **"Better file available"**, with its reason ("FLAC vs your 320 MP3"). A file counts as better only for its audio or for being there: it is on disk when the linked file isn't, its quality verdict, whether the DJ app plays it, the measured cutoff, decode errors, or its format. A copy that only has more tags, more art, a rekordbox link or a better place is not a better file: tags and art are picked per field, apart from the audio. The linked file is pinned: a better file never moves the link, and neither does grouping, relink or a send. A link moves on its own in one case only, when the linked file is gone from disk and the very same audio is on disk (1.8, "A Library track follows its file"). The only other way a link moves is the user's own switch of a gone linked file to another copy, in Review (1.12). The **Upgrade** action arrives with managed copies (2.6). Chosen tags reach rekordbox only via the XML, because the file is read-only.
- **Resolving changes DB state only.** Nothing on disk changes.
- **Complexity:** M.

#### 1.8 Add to Library · 1a
- **In the MVP every Library track is linked:** it points at an existing file. For tracks rekordbox knows, that's the file rekordbox already uses, so nothing changes in rekordbox and cues and history stay safe. For tracks rekordbox doesn't know, the next send adds them at their current Location.
- **Which file** (1aD-6, 2026-09-30): the file of a trusted rekordbox match; otherwise the track's best file. A probable match is never the file linked. **A track whose only rekordbox match is probable can't be added by hand** until the match is confirmed in Review, or answered "Not the same" (1bE-6, built 2026-10-13; owner decision, 2026-10-02): linked to its best file it would be sent as a new track, beside the rekordbox entry that is probably its own, a second entry without the cues. The add refuses with its own reason, and All music greys out the row's "Add to Library" with the reason on the row. A track that also has a trusted match is linked to that file as before. The add is refused when the track has neither, or when that file isn't on disk; such tracks wait in the Missing list (1.3). **Which file is "best" when one is gone** (owner decision, 2026-10-11; 1bC-4): the ranking (1.7) puts a file that isn't on disk below every file that is, so a track rekordbox doesn't know, whose better copy was deleted, is added linked to the copy that is there. There is still no fallback for a trusted match: when rekordbox's own file is the one that's gone, the add refuses, because another file would become a second rekordbox entry without the cues. A file on an unplugged drive counts as on disk (unplugged isn't missing, 1.1): its track can be added and shows no "File missing". Adding a track that's already in the Library changes nothing.
- **Read-only:** the file stays where it is and is never written. Tag fixes reach rekordbox only via the XML (on first import, XML values win over file tags, §5.2).
- **Carried over:** rekordbox's data (1.2) and the best tags gathered from all the track's files (1.7), sent through the XML.
- **Fragile locations** are warned about (1.3).
- **Durability:** "file missing" when the file disappears. An in-app delete asks for confirmation, can be undone, and drops the track from the next send.
- **A Library track follows its file** (owner decisions, 2026-10-12; 1bE-10). When the linked file is gone from disk and a file with the very same audio (an equal `audio_hash`, current for the new file and current for the old one when it went) is on disk in any folder the app scans, another drive included, the track's linked file moves to that file on its own. Nothing else moves a link on its own in Phase 1, and it only ever moves to the same audio: another encode never follows, even one of the same length and title. That is a "Clearly best file" item in Review (1.12), the Library list shows the track's file as missing meanwhile, and the track is still sent as rekordbox's own entry (1.9). The rules:
  - **Gone means gone.** Only a file a walk looked for and didn't find counts. A drive that isn't plugged in is never taken for gone files (unplugged isn't missing, 1.1). While the linked file is on disk nothing moves, so a file that comes back later moves nothing back. A file that was moved back (the new place is gone, the old one is there again) is a move like any other: the track follows it back, and a track rekordbox knows is sent as its own entry again.
  - **A file of the track itself.** The new file is one grouping has put in the Library track's own track (files with the same `audio_hash` are one track, 1.4), so no file changes track. A file grouping keeps apart isn't followed.
  - **One track, one file.** A file a Library track is linked to is never followed to, and when two Library tracks hold that audio (the same recording twice, 1.4) nobody follows.
  - **The automatic follow never changes which rekordbox entry a track is sent as.** The hold is about a second entry and about a change of entry, nothing else. After relink has committed, the follow looks at the rekordbox entries matched to the new file (whatever the match: trusted, probable or confirmed) and asks the send what the track is sent as today and what it would be sent as linked to the new file. With two or more entries matched to the new file, nothing moves, whichever of them a send would pick. When the track is sent as one entry today and would be sent as another, or as none, on the new file, nothing moves: a track rekordbox knows follows only where relink has matched its entry to the new file and trusts the match, and is then sent as that entry, with rekordbox's old `Location` (the check before a send lists it, 1.9; there is no other notice). The link follows when the track is sent as the same entry before and after, as no entry before and after (a track rekordbox doesn't know: the next send adds it at the new place), or as no entry today and, on the new file, as the only entry matched to it: that is a file moved back to its entry's path, and a track rekordbox didn't know whose moved file rekordbox has since learned. Two more holds: a follow never takes a file that the entry another Library track is sent as is matched to, and a track that would go out as new doesn't follow while a file of its track is the probable match of an entry, as an add is refused there. Where nothing moves the track stays a Review item: a switch that changes rekordbox's entry is the user's choice (1.12).
  - **When it runs:** as the second step of the relink job, after relink has committed and in transactions of its own: after the hash stage, after every grouping run and after every read of rekordbox. Never while the user waits, and it reads only the database: the job asks where the drives are once, before it takes the database's writer, and both steps work from that answer, so no other write ever waits on a drive. Running it again changes nothing.
  - **In the log, skipped by Undo.** Each follow is an operation of its own that the app made ("automatic"). The Undo button never lands on one: the next Undo takes back the user's own last action. Undoing an earlier action on the same track (its add) takes the follows made since back with it, all or nothing; a follow on another track stays. They are taken back as one net change, not step by step: undoing the add deletes the track without first linking it back to the file it followed from, so it still works after the old music folder, and with it that file's record, has been removed from the app. Undo is refused, changing nothing, when something else changed the link since, or when a value it has to write back no longer exists.
  - **Known limit:** when the old file is on disk again while the new one still is, rekordbox's entry is matched to the file at its own path again (1.2, step 1), and the link, whose file is on disk, stays. The track would then go to rekordbox as a new one, which the check before a send lists ("rekordbox has another file of the same track"). To be closed by 1bE-11 (owner decision, 2026-10-13): a Library track whose linked file has no rekordbox entry, while a byte-identical copy of it in the same track has one that no other Library track is sent as, is sent as that entry, whether or not a follow was involved.
- Managed copies (converting, clean tags written into a new file, upgrades) are Phase 2 (2.6).
- **Complexity:** S.

#### 1.9 Send to rekordbox (XML) · 1a
- **What:** write the Library and its folders as rekordbox XML:
  - **COLLECTION:** tracks, per rule 4.
  - **PLAYLISTS:** two top-level folders, `Crates` and `Playlists`, holding their trees as NODEs.
  
  The user then imports it through rekordbox's `rekordbox xml` sidebar.
- **XML writer rules** (the behavior behind them is in §5.2):
  1. **Every attribute of every track sent, every time.** Fill each with rekordbox's current value from a fresh read, unless the app means to change it. Never omit, never default. The only exception is the analysis fields in rule 3.
  2. **Read → merge (1.10) → send in one step.** Tell the user not to play or edit in rekordbox from the export until the import is finished: the values sent come from that export, so anything changed in rekordbox after it is overwritten by a Yes.
  3. **Analysis fields** (`TEMPO`, `POSITION_MARK`, `Tonality`, `AverageBpm`) depend on the case:
     - **(A) The track is already in rekordbox at this Location:** omit them.
     - **(B) A managed copy at a new Location** (Phase 2, 2.6): send them deliberately, carried over from rekordbox's entry for the old file.
     
     **Confirmed by gate test T3** (2026-09-28, rekordbox 7.2.19): omitting them keeps rekordbox's key, BPM, grid and cues. **Never send the app's own BPM or key estimates for a track rekordbox already has**: a Yes re-import loads any `Tonality`/`AverageBpm` the XML carries, over rekordbox's analysis and out of step with its grid.
  4. **COLLECTION holds every track that a sent playlist or crate references, plus new tracks, each with every attribute (rule 1).** Gate test T1 failed (2026-09-28, rekordbox 7.2.19): a playlist entry only resolves if its track is in the same XML's COLLECTION; other entries are dropped silently, whether keyed by TrackID or by Location. So playlists-only sends are impossible. To keep a send small, it may carry only the playlists and crates that changed plus their tracks. Every track already in rekordbox raises the Yes/No dialog (or the user ticks "Don't ask me again"), so the checklist states how many dialogs to expect.
  5. Write `Location` fully percent-encoded.
  6. **App-owned playlists live only under `Crates` and `Playlists`.** When a crate is renamed or deleted, the next send lists the stale rekordbox playlists to delete by hand: only ones the app itself sent before. Something the user made in rekordbox is never listed, even inside those two folders.
  7. **Removing a Library track can't reach rekordbox via XML.** The app says so and lists what to remove.
  8. **Record `sync_base`** for every field sent.
- **How the writer applies them** (`rekordbox_write`, 1aE-7 to 1aE-10, 2026-10-01):
  - **Never analysis, for any track in Phase 1.** `AverageBpm` and `Tonality` are dropped from every track's values, and the writer can't write a `TEMPO` or `POSITION_MARK` at all. Case B arrives with 2.6.
  - **A known track that no sent crate or playlist names isn't sent** (it would only raise a Yes/No dialog). New tracks are always sent.
  - **`Location` encoding:** everything but ASCII letters, digits and `- . _ ~ / :` as uppercase `%XX`, read back with the reader before it's used. A new track's path is always written this way. **A known track's `Location` is re-encoded only when it's a drive path in rekordbox's usual shape** (`file://localhost/C:/…`), starting with exactly that prefix and escaping only characters rekordbox itself escapes, so the two spellings differ in escaping alone: that's the form the behavior check sent and rekordbox updated in place. In any other shape (a network or macOS path, a server or nothing where `localhost` goes, backslashes, an uppercase prefix, an escaped `/`, `:`, `.` or letter) it's sent back exactly as rekordbox wrote it, because re-spelling those is unverified (§5.2).
  - **TrackIDs are file-local.** A known track keeps rekordbox's; a new one is numbered above the highest TrackID in the whole last rekordbox read. Entries use `KeyType="0"`.
  - **A track that can't be sent is left out and reported with a reason code**, along with every entry naming it; the other tracks still go. That covers a missing file or Location (but not a missing file of a track rekordbox already has; see the next point), a value holding a character XML 1.0 can't carry (the value is never stripped or rewritten), and two tracks at one Location. A crate or playlist that loses every entry is still written, empty, and listed.
  - **A known track whose file is missing is still sent** (owner decision, 2026-10-01; found by Location, 2026-10-01), so a crate or playlist naming it stays whole: rekordbox's own entry goes back unchanged, every attribute exactly as rekordbox wrote it, `Location` included (not re-encoded), analysis fields omitted as for any known track. Relink never matches a rekordbox row to a missing file (it drops such a match on every run), so the entry is found by the linked file's own path instead: the one row of the newest rekordbox read at that `Location` (compared as relink compares a path: NFC, NTFS letter case). The entry at the linked file's own `Location` is this track's entry, whatever relink has made of it since: it is used when it is matched to no file, and when relink matched it to a file no Library track is linked to, such as another copy of the track that is on disk, or a look-alike elsewhere (owner decision, 2026-10-12; 1bE-10; until then such a track was left out of the send). The file relink matched it to is never what is sent. The path is the volume's mount point plus the file's path on it, with the mount point chosen by relink's own rule (where the volume is mounted now, or where it was last mounted only if no other known volume was), so a file on an unplugged volume whose drive letter another volume shares isn't looked up. This only echoes rekordbox's entry back at its own `Location`; it attaches nothing to any file, so it's no match in relink's sense. With two rows at that `Location`, a row matched (trusted or probable) to a file another Library track is linked to (that track sends it: one entry is never sent for two tracks, so the gone-file track is left out from the moment the other track is in the Library, and sent again once it is removed), a row kept from an earlier, incomplete read, a row two Library files lead to (it goes to neither), or no row at all, the track is left out with its missing-file reason, as above. The track's `sync_base` is recorded as for any known track (rekordbox's own values), and its `last_sent_location` is rekordbox's `Location`.
  - **`Crates` and `Playlists` are always both written**, even empty. Two siblings with the same name (compared in NFC, ignoring letter case and trailing whitespace, §5.2), or a nameless node, refuse the whole send.
  - **Characters:** `& < > " '` are escaped, and tab, line feed and carriage return are written as character references, so a value reads back exactly.
  - **The file is read back with the app's own reader before it's written**, and written through the write guard to a new file beside the destination, renamed into place once complete.
  - **Rule 8** (`record_send`, 1aE-11, 2026-10-01): once the file is in place, each sent track's `sync_base` becomes exactly the fields written (a field sent before and not this time is dropped), and its `last_sent_location` and `last_exported_at` are set, in one transaction. All of it or none. The `Location` base is the string sent, which for a re-encoded drive path differs from rekordbox's own spelling, so 1.10 must compare `Location` by its decoded key, never as text. An analysis field is never given a base, whatever is sent. A send isn't in the operation log. Its new bases never take the row id of a deleted one (`sync_base` ids are never reused, §2), so removing a track can still be undone after a later send: the track comes back with the bases it had. Undoing the add of a track that was sent since is refused, changing nothing (the track is removed with "Remove from Library" instead).
  - **Rules 6 and 7, after a send** (`after_send`, 1aF-2, 2026-10-01): two lists computed from the database, so they never depend on a send having just happened. **Stale playlists:** a playlist or folder is listed only if the app itself sent it before (owner decision, 2026-10-01), a read has shown it in rekordbox under the top-level `Crates` or `Playlists` folder, the latest read still has it, and the app's tree no longer has it (names compared as the writer compares them: NFC, letter case and trailing whitespace ignored; a folder never stands for a playlist of the same name). Something the user made in rekordbox is never listed, even inside those two folders, and nothing outside them is. A stale folder is listed once, and only when everything rekordbox has inside it, at any depth, was sent by the app too, so its count is exactly what deleting it deletes; a folder holding anything the user made isn't listed, and the recorded playlists and folders inside it are listed one by one. "The app's tree" is the whole crate tree, not the subset one send carries, so an unchanged crate isn't stale; the app has no playlists of its own until Phase 3. What a send wrote is kept by `record_send` in the table `sent_playlist` (§2), in the send's own transaction and cumulative across sends: a path is marked seen once a read shows it in rekordbox and forgotten when a later read no longer has it (any read counts, including one of a different export, which can only list less), and a path never seen stays (that send may not be imported yet) but is never listed until a read shows it. The last read's names for the two folders (empty ones too) are kept as one setting, `rekordbox_playlist_tree`, written in the read's own transaction and replaced whole by the next read; an incomplete export (§5.3) keeps its tree too, since only its tracks can be missing. Until a read has kept them, the list says it wasn't checked. **Manual removals:** every track removed from the Library in the app that rekordbox still holds, whether or not a send ever wrote it (owner decision, 2026-10-02; before, only tracks sent before were listed, which in a Library started from rekordbox left out every track no crate named). A track that was sent is matched to rekordbox's tracks by the decoded `Location` it was sent to (a `Location` that can't be read back is matched by the read's rows matched to the track, and shows no path); it leaves the list once a read made after its removal no longer has it, and a read from before says nothing. A track that was never sent is listed while the latest read holds rekordbox's own entry for it, found through a trusted match to one of the track's files, and shows that entry's path; it leaves once a read no longer has the entry. With only a probable match, or no rekordbox entry at all, it isn't listed. **An entry a switch left behind** (1bE-7, 2026-10-13): when the user switches a Library track's gone linked file to another copy in Review (1.12) and the track then goes to rekordbox as another track than the entry it was sent as, that old entry is recorded (`replaced_entry`, §2) and listed here too, while a read holds an entry at its `Location` and no Library track is sent as it; so one entry is never both sent and listed. One entry is listed once: where a removed track is already listed at that `Location`, the entry isn't listed a second time. The track itself stays in the Library.
- **How a send runs** (`send`, 1aF-1, 2026-10-01):
  - **Two steps, each a job.** *Prepare* reads rekordbox's export again, always (a send never uses an old read), matches its tracks to files, and builds the **preflight** without writing anything. The matching and the preflight are one database job, so no other read can land between them and make a known track look new. A send job never runs in a later run of the app: every job names the run that queued it and does nothing in another, and one left queued or running is ended at startup, so a write only ever follows a click in this run. *Write*, on the user's go, builds the send again and refuses unless it is byte for byte the send that was reviewed, from the same read, of an export that hasn't changed on disk since. A preflight is good for one send; the next send starts with a new read.
  - **The preflight shows:** how many tracks are new and how many rekordbox already has (the Yes/No dialogs to expect); every track left out, with its reason; tracks rekordbox has that are sent although their file is missing; tracks rekordbox has that are sent with rekordbox's own `Location` while no file is at that path (the app paired the entry with a file elsewhere, and a send never changes a known track's `Location`, so rekordbox will still show "file not found"; the path is looked up on disk once, by the prepare step, after its database job; a track isn't listed when that can't be told (its drive isn't connected, or is a network drive, mapped to a letter or not; a network or macOS path; a lookup that fails for another reason than the file not being there); a track sent although its file is missing is never listed here as well; and the list is review information only: it isn't part of what the write checks, so a file that comes back before the go changes nothing about the go; DOGFOOD C1-9, 2026-10-10); new tracks sent without a `Kind`, because their file is in a format rekordbox isn't known to play (Ogg, Opus, WMA and the like) or one the app couldn't tell (they're sent as before; rekordbox may refuse them or not play them; DOGFOOD K13, 2026-10-10); Library tracks whose file rekordbox doesn't have while it has another file of the same track (they can be sent; a second rekordbox entry results); every crate or playlist that will arrive with entries left out, with its count; and a refused send, explained.
  - **An explicit confirm is needed** when a crate or playlist loses entries, and when the read couldn't store some of rekordbox's tracks (a Library track on such a track's file goes out as new, and a Yes in rekordbox overwrites that entry).
  - **Refused as a whole:** the writer's refusals above (same-name siblings, a nameless node) and **an incomplete export** (§5.3): it keeps rows from earlier reads, so some values would come from an old read. **An export saved before the last send isn't refused; the user is asked** (owner decision, 2026-10-10; from 2026-10-02 until then it was refused outright, DOGFOOD K16). Such an export can't show what that send put in rekordbox: if the send was imported, tracks rekordbox now has would go out as new and raise dialogs the review didn't count; if it never was, the export is as good as a new one, and the app can't tell which (it records nothing about imports). The export's modified time (what the checklist shows as "Export saved") is compared with the time the last send was recorded (the latest `last_exported_at` of any Library track or removed track, or `sent_at` of any sent crate); saved earlier, to the millisecond, the review asks whether a send has been imported into rekordbox since the export was saved. "No": the send goes ahead from that export, exactly as from a new export with the same content, and the review says it goes on that answer. "Yes": refused, with the ask to export again. No answer: the file can't be written. Saved at that millisecond or later, or with no send ever recorded: no question. The answer travels with that one go: it's kept only in that send job's own row, and no later send reads it, so the next send from the same export is asked again. A "no" lifts nothing else: a refusal still refuses, the explicit confirm is still needed where it was, and the write still rebuilds the reviewed send byte for byte. With no track to send and no crate or playlist, the send can't be started.
  - **One fixed file:** `tracklist-pro.xml`, directly in the app data folder, replaced on every send, so the app changes no bytes anywhere else (§6) and the user points rekordbox at it once. The checklist shows its path with "Copy path". There's no "show in folder": opening Explorer takes a process or shell call, which the write and network gates forbid.
  - **A failed send records nothing.** A failed write leaves the last send's file and every `sync_base` as they were. If recording the send fails once the file is in place, the last send's bytes are put back when that's possible; when there was no earlier file (or putting it back fails), the new file stays, unrecorded, and the send is reported as failed. A crash between the file going in place and the record leaves the same: an unrecorded file, which the next send replaces; nothing detects it at the next start. Before writing, the files a crashed send left behind are cleared (§5.6).
  - **What goes in:** every Library track (rule 4 decides which are written) and the crate tree: folders and hand-made crates, a crate's tracks in the order they were added. Smart crates wait for 3.3, and `Playlists` is written empty until Phase 3.
- **UI:** **Send to rekordbox** is the Overview's main button (1.13), shown once the Library holds tracks. It opens a guided checklist covering rekordbox's side: export the collection, review the preflight, write the file, point `rekordbox xml` at it (once), refresh, Import to Collection, Import Playlist, answer the dialogs (it says how many to expect). It warns not to play or edit in rekordbox from the export until the import is finished (the values sent come from the export). It is not a one-click sync.
- **Complexity:** M.

#### 1.10 Conflict detection & review · 1c
- **Three-way comparison per field** (app vs rekordbox vs `sync_base`):
  - only the app changed → send it
  - only rekordbox changed → pull it in
  - both changed → **conflict**
- **Conflicts queue:** **Review → Conflicts**, with side-by-side values. Keep the app's value, keep rekordbox's, or edit. Resolve in bulk ("take rekordbox for all ratings").
- **Analysis fields never conflict**, because the app doesn't edit them. The database enforces it (1aE-11). Conflicts are about tags, genre, comments, rating, color and playlist membership.
- **This is mandatory.** A rekordbox "Yes" is a full overwrite (§5.2), so any rekordbox-side edit not merged before sending is destroyed.
- **Second channel:** a changed file tag on a linked track is another sign of an edit made in rekordbox (§5.1).
- **How the engine applies it** (`conflicts`, 1cA-1 and 1cA-2, 2026-10-12; no screen and no change to the send yet, those are 1cB):
  - **The three sides are inputs.** rekordbox's value is its entry's attribute in the newest read, the base is `sync_base`, and the app's is whatever the app means to change (1.9 rule 1). Nothing stores the app's own values yet, so today every field of a track rekordbox knows follows rekordbox and nothing conflicts. A field the app holds no value of its own for takes rekordbox's.
  - **Both changed to the same value isn't a conflict.**
  - **A field is compared by its kind, never as plain text:** `Location` by its decoded key (1.9); `Rating` by stars, no rating being 0 stars; `Colour` by the color; dates and numbers by value (for `DiscNumber`, `TrackNumber` and `Year`, 0 is rekordbox's "none"; a `PlayCount` of 0 is a real count); everything else as text in NFC, where all-blank text is no value and **a difference in letter case or spacing is a change** (default, the owner may overrule). A value that doesn't parse as its kind is only equal to the same text. `Comments` is one field, My Tags block included.
  - **A field that was never sent has no base.** If the two sides hold the same value, nothing happens. If they differ, it's a conflict recorded without a base: nothing proves either side unchanged. The exception is when rekordbox still shows the very value the app's edit replaced (below): then only the app changed, and its value is sent.
  - **A send that was never imported isn't "rekordbox changed it back".** After a send the base is the app's value, and if the XML was never imported (or the dialog answered No) rekordbox still shows the old one, which by the rule above would be pulled in over the app's edit. When the value the app's edit replaced is known and rekordbox still shows it, the app's value stands and is sent again; nothing is taken in and no conflict is recorded (foreman default, 2026-10-12; the owner may make it a conflict instead: one constant, `NOT_TAKEN_UP`). **Known gap:** nothing records the replaced value yet (the edit store or the send must), and until then this case reads as "only rekordbox changed".
  - **A rekordbox row kept from an older, incomplete read decides nothing** (§5.3): no conflict is opened, updated or withdrawn from it. A track with no trusted rekordbox entry has nothing to merge, and **its open conflicts stay as they are** until the user resolves them or removes the Library track (they may hold the only copy of rekordbox's value); if its entry comes back in a later read, the same conflicts are decided again by these rules.
  - **Detecting isn't an operation.** It's derived from the three sides, so it isn't in the operation log and Undo never takes a detected conflict away. A rerun on unchanged sides writes nothing. An open conflict is updated in place when a side changes again, and withdrawn (deleted) when the sides no longer disagree: it only held copies of values that live on their sides.
  - **A settled conflict is never changed by detecting.** It's the user's decision and the only record of the value that lost. It stands while rekordbox's value and the base are the ones it was settled on and the app's value is the one it was settled on or the one chosen: the value chosen is what goes out, **whatever the comparison says** (a value given by hand that is the one last sent would otherwise read as "only rekordbox changed" once the app holds it, and the value the user turned down would come in). Once a side moves, the rules above apply again; if that's a conflict, it's a new one, and the settled one stays. Of a field's settled conflicts only the one settled last answers, by its id, not by the clock.
  - **Resolving** (keep the app's, keep rekordbox's, or a value given by hand, which must be a value of the field's kind; in bulk by field, by tracks, or both) is one operation per call, so one Undo takes back a whole bulk resolve and the conflicts are open again with all three values. A settled conflict is a new row replacing the open one (the table's checks tie status, time and value together, and the log writes one field at a time). Undo is refused if the field has a newer open conflict.
  - **Not compared yet: playlist membership.** Nothing records which tracks a send put in a playlist, so there's no base for it.
- **Complexity:** L.

#### 1.11 Track browser with stages and keyboard · 1c
- **What:** a virtualized table with full-text search, filters, columns and a preview player.
- **Stages:** Overview → Review → All music → Library → Crates; Inbox arrives in Phase 2 and Playlists in Phase 3.
- **Hidden filter** (in All music): lists everything the app isn't showing, each with its reason and a way back:
  - "Show again" for files you hid
  - "Use this file instead" for extra files (reopens the choice in Review)
  - "Put back in Inbox" for cleared Inbox files
- **What search covers** (owner decision, 2026-10-12; index 1cA-6): title, artist, album, remixer, label, genre, comment and file name. Capitals and accents don't matter, and a word matches from its start ("hou" finds "House"). Results keep the list's own order; there is no relevance ranking. A file name is searched without its extension ("mp3" finds nothing), and folder names aren't searched. Text with no letters or digits filters nothing. "N hidden files match" counts only hidden files whose track isn't already in the results (owner decisions, 2026-10-12).
- **Search finds hidden files too:** "N hidden files match · Show them".
- **Keyboard:** Ctrl+K palette; J/K to move, Space to preview, 1–0 to rate energy, T to tag; a bulk selection model.
- **Audio:** preview in the webview over a custom Tauri protocol, with AIFF and ALAC decoded to PCM/WAV in Rust. The page's content security policy allows that one local protocol as a media source and nothing else new; it serves only files the app has scanned, looked up by id, with no network (owner decision, 2026-10-13: a security setting, and the test that keeps the policy strict is changed on purpose for it). Waveforms are computed once in the job queue and cached as peak arrays.
- **Complexity:** L across all phases. **Reuse:** `TrackTable`, `ColumnEditor`, `useTrackRows`, `PlayerBar`.

#### 1.12 Review screen · 1b
- **What:** every decision the app can't make alone. Phase 1 covers duplicates, possible version mix-ups, quality problems, relink confirmations and "Better file available", with conflicts (1.10) in 1c. Later phases add tag suggestions (2.2) and unmapped genres (3.2).
- **Scope:** Library tracks, plus every relink match waiting to be confirmed, in the Library or not: a track whose only rekordbox match is probable can't be added until it is confirmed (1.8). Adding is instant, so there is no "being added" state (owner decision, 2026-10-12). Duplicates among tracks you never add aren't queued, though All music still shows them. The Missing list (1.3) stays its own section below the queue and isn't counted in "how many need a decision". Inside a group the oldest item comes first.
- **Layout:** a queue on the left and a comparison on the right. The queue is grouped by how much thought each item needs: "Needs a closer look" first, then "Clearly best file", then "Quality problems". It shows how many are left.
- **What each group holds in Phase 1** (owner decision, 2026-10-12):
  - "Needs a closer look": possible version mix-ups (pairs the rules send to review), duplicate sets held because their pairs disagree, two Library tracks that are the same recording (1.4), and relink matches to confirm (1.2).
  - "Clearly best file": a Library track whose linked file is gone while another copy of the track is on disk. Switching to that copy is the only choice "Accept all N" makes.
  - "Quality problems": a Library track whose linked file is flagged, cut off or broken, with no better copy; and "Better file available" items whose linked file is still there (so a flagged track that has a better copy is here too, as a better-file item).
- **"Better file available" in Phase 1** (owner decision, 2026-10-12): the item shows the better file and its reason (1.7) and can be dismissed; the dismissal is remembered. A dismissal covers that one file: a different, better file is a new item. Switching the linked file is offered only when the linked file is gone. For a track rekordbox knows the item says plainly what the switch means there: the XML can't change a known track's location, so rekordbox sees the copy as a new track without the old entry's cues and the old entry goes on the list to remove in rekordbox; relocating the file inside rekordbox keeps the cues. (That is the case where rekordbox's entry isn't trusted on the copy; the other case is in "How the actions work" below.) "Accept all N" for "Clearly best file" is this same switch. The Upgrade action arrives with managed copies (2.6).
- **"Accept all N"** for clear cases. It can be undone, and what it accepted stays viewable: afterwards a line at the top of Review says how many tracks were switched to the copy on disk, with View (each track with its old and new file) and Undo, and it stays until it is dismissed or the next "Accept all" (owner decision, 2026-10-13).
- **How the actions work** (1bE-5, 1bE-6, 1bE-7, 2026-10-13; `review::actions`). Each is one operation in the log, so Undo takes it back and the item is in the queue again; the queue is right as the action or the undo returns, without waiting for a background job (for up to 500 tracks in one step; beyond that when the roles job ends).
  - **Remembered by what the files hold** (`review_decision`, §2): a dismissal, "Keep anyway" and "Find a better file" name the audio the track is linked to (and a dismissal the better file's audio too), never a file or track id. They survive a rescan, a retag and a move; a file that is encoded again holds other audio, so it is a new item. A file that hasn't been hashed yet can't be decided about.
  - **Dismiss** is offered on "Better file available" items whose linked file is still there. A "File missing, copy found" item has no "don't switch" (owner decision, 2026-10-13).
  - **A dismissal hides only that one file** (foreman's default, 2026-10-13; the owner is asked). The track is then offered its best-ranked better file that isn't dismissed, a new item, as the dismissal returns; the stored best file and the ranking (1.7) don't change, only which file is offered (`file_roles::offered_better`, the one place that skips dismissed files; a dismissal made or taken back makes the track due for the roles job). When the linked file is flagged and every better copy is dismissed, the track is a single-file "Quality problem" item again ("Keep anyway" / "Find a better file"): dismissing a copy is no decision about the flagged file. A copy that is on disk while the linked file is gone is offered whatever was dismissed.
  - **What a switch means in rekordbox is the send's own answer** (1.9), asked for the track as it is linked now and as it would be linked to the copy; a switch never overrides relink's match (owner decision, 2026-10-13: cues are kept when rekordbox's entry is already matched with trust to the copy; otherwise a new track, and the old entry on the remove list). *rekordbox's entry stays the track's* where relink trusts that entry on the copy, or the user confirmed it there: the track is still sent as that entry, with rekordbox's old `Location`, so rekordbox keeps the track and its cues and shows the file as missing until it is relocated there. *A new track* where the entry is matched to the copy only probably, or to nothing: the track is sent from the copy as a new track without the old cues, and the old entry is recorded (`replaced_entry`, §2) and listed with the tracks to remove in rekordbox (1.9 rule 7), until a read no longer has it or the entry is trusted on the linked file after all. *Another entry* where rekordbox already holds the copy as a track of its own: the Library track is sent as that entry from then on, and the old one is listed to remove. A track rekordbox doesn't know has nothing to say. The item shows which case it is, and the switch works it out again when it is made: if it is another case by then, nothing is switched and the item is shown again.
  - **A switch is refused** when the linked file is on disk after all, when the file to switch to isn't a file of the track, is gone itself, or is linked by another Library track, and when a rekordbox entry matched to that file is the entry another Library track is sent as (that track would lose it, as the automatic follow holds there, 1.8).
  - **Undo of a switch after a send.** A switch that made the track a new one for rekordbox (a track rekordbox didn't know, or one whose entry isn't trusted on the copy) can't be undone once the track has been sent since: the copy's new entry is in rekordbox by then, and taking the switch back would leave it there with nothing to list it. It is refused with its own reason, as undoing the add of a track that was sent since is (the add's rule is stated in §2, `operation` / `change`, and in 1.9, "Rule 8"). A switch that kept the track rekordbox's own entry, or made it another entry rekordbox already held, adds nothing to rekordbox, so it can be undone after a send too. Before any send every switch can be undone (foreman's ruling, 2026-10-13).
  - **After a switch** the track's files are ranked again at once, for the new link. Nothing pulls the link back: the automatic follow only acts while a linked file is gone (1.8), and relink never writes a link (1.2).
  - **A "rekordbox match uncertain" item** has two buttons, "Confirm" and "Not the same" (1.2). Both answers are kept by the audio the file holds, so they outlive a move or a rename of the file and its music folder removed and added again, and removing a music folder is never refused because of them (foreman's ruling, 2026-10-14). "Not the same" waits until the file has been hashed. A match the user confirmed before, whose file holds other audio now, is the same item with "Confirm again", and says why it is back. There is no button to withdraw a confirmation yet (1cB, with the Details panel).
  - **"Accept all N" makes the switch only for the clear cases:** a track rekordbox doesn't know, and a track whose entry stays its own. A switch that arrives in rekordbox as another track than today (a new track without the old cues, or another entry) stays in the queue for a decision of its own: "Accept all" leaves out the switches that lose cues (owner decision, 2026-10-13; the rule is one function, `accept_all_covers`). N counts only what it will switch, and a click sends exactly the items the button counted: each is switched only if it is still that item, with the copy it showed, and still a clear case; an item that joined the queue since is never switched unseen (foreman's ruling, 2026-10-13). It is one operation, so one Undo restores every link, and it returns each switched track with its old and new file and each item it left with a reason code; the view of what it accepted (above) is built (1bE-3, 2026-10-16): the line sits at the top of Review whatever the queue holds, it is kept for the session, not only while Review is open, and `review_switched_view` gives the names the switch's ids stand for (the track, the old and the new file). Its Undo is the normal Undo of that one operation, offered while that operation is the next to undo, and the line goes once that operation is undone by any Undo (this button, the top bar's, Ctrl+Z); an Undo of some other step, a failed one or a refused one leaves it.
- **Comparison:** one row per file with the same columns, the suggestion first, and a plain-language reason. "More details" is a switch above the table, off at first and kept on from item to item while Review is open; it adds bitrate, sample rate and date added (the day the app first saw the file; rekordbox's entry has none). The Details panel shows each tag's value for the Library track and which file it comes from; that part arrives in 1c with the Details panel (owner decision, 2026-10-12).
- **Keyboard:** Enter keeps the suggestion, number keys pick another file, S splits the group ("these aren't the same"). The next item loads automatically. In detail (owner decisions, 2026-10-13): Enter only ever does the item's suggested action, so a habit of pressing it never hides anything: "these are the same track" on a possible version mix-up and on "Unclear duplicates", the switch to the suggested copy on "File missing, copy found", confirming on a relink match; on a quality problem and on "Better file available" it does nothing, and those keep their own buttons. Number keys pick by row (1 is the first row, up to 9) and act only where there is a choice of file today: "File missing, copy found" with more than one copy, where the digit switches to that copy. S on an item with two files says they aren't the same; with three or more it splits off the highlighted row, and does nothing while no row is highlighted. After a decision the next item in the same group loads, then the first item of the next group that has any, then the empty state; after an action the app refuses, the screen stays where it is. The number keys rate energy in the Library (1.11) and pick a file here. **Built** (1bE-2, 2026-10-16) except S and "these are the same track", which wait for the command that writes a grouping decision (the second PR of 1bE-2) and for the merge of two Library tracks (1bE-9): Enter confirms a relink match and switches to the suggested copy on "File missing, copy found", and does nothing on anything else; a number key switches to the copy in that row when the item has two or more copies on disk (the comparison carries the switch to each copy with its own effect, `copies`), and does nothing on the linked file's row, on a row with no copy or a refused switch, or on any other kind. The `.keys` line in the action row says which keys work on the item. The keys are read on the whole screen but never from a focused button, field, link or slider (the play buttons sit in every file row), with a modifier, while a dialog is open or when held down, and an action is never started twice. After a decision, by key or click, the next item in the same group, else the first item of the next group is selected; with nothing after it the selection wraps to the first item left, and the empty state shows only when nothing is left (foreman's ruling, 2026-10-16); after a refusal nothing moves.
- **Listening:** switching between files keeps the playback position. The preview engine (1.11, tasks 1cA-7 and 1cA-8) is built in 1b for this (owner decision, 2026-10-12). How it behaves (owner decisions, 2026-10-13): the position carries by time (1:30 stays 1:30; a file shorter than the position starts from its beginning); a file that was playing keeps playing and a paused one stays paused; there is one player for the whole app, with play/pause and a scrub bar, one file audible at a time, and no volume control or loudness matching in this version. A file that can't be played (missing, its drive not plugged in, not decodable) gets a short line in the player saying which, and no playback. An online-only OneDrive file is never downloaded to preview it: the player says it is online-only.
- **Single-file quality problems:** "Keep anyway", or "Find a better file" (→ wishlist, 2.3). In Phase 1 (owner decision, 2026-10-12) "Find a better file" stores a remembered mark that you want the track in better quality, which the wishlist picks up in Phase 2; either choice takes the item out of the queue for good. After "Keep anyway" the ranking (1.7) still treats the file as flagged: its audio hasn't changed. "For good" is for the audio that was flagged: linked to other audio later (the file encoded again), a flagged track is asked about again.
- **Complexity:** M.

#### 1.13 Overview · 1c
- **Header:** the Library's size, when it was last sent, and one main button: "Send N changes to rekordbox".
- **What to do next:** unsent changes, tracks made from low-quality files, missing tags or art, tracks rekordbox hasn't analyzed, and tracks in no crate or playlist. Each item disappears when fixed, and the empty list says "Nothing to fix". One progress line sits beside it ("This week: 4 tracks added, 12 decisions made").
- **Four numbers:** tracks and total length; how many are in crates or playlists; how many rekordbox has analyzed; when the Library was last sent.
- **Charts:** energy spread and genres as one-color bar charts.
- **What the numbers mean** (owner decision, 2026-10-12): a track's genre is rekordbox's for a track rekordbox knows, otherwise the file's tag. "Analyzed" means rekordbox supplied a BPM or a key. "Decisions made" counts the operations in the undo log; one that was undone doesn't count. "This week" is the last 7 days. A track rekordbox knows with a blank genre there has no genre (the file's tag isn't used instead). "Missing tags" is a Library track with no title, no artist or no genre; for a track rekordbox doesn't know, a title that is only the file name counts as no title (for a track rekordbox knows, its Name there is the title); "missing art" is one whose linked file has no embedded cover; they are two numbers. "Not analyzed" isn't shown until rekordbox has been read once. "In crates" counts the app's crates; playlists join when the app has them. Nothing stores energy yet, so the energy chart waits for the energy decision (1cC-1). What counts as an unsent change is defined with the Send button (1cB-11).
- **Outside your Library:** one strip with the All music count, how many need a decision, and links to Review, All music and Manage folders.
- **Every number is a door:** clicking opens the Library filtered, with a visible filter chip saying why.
- **No health score.**
- **Launch:** the app reopens where you left off, except on first run or when a scan or read finds something new; then it opens on Overview. During scans, numbers say "still counting".
- **Complexity:** S–M.

#### 1.14 Crates · 1c
- **What:** hand-made crates of Library tracks, built for finding what fits next.
- **Adding:** drag onto a crate in the sidebar, press C, or use "Add to crate or playlist", with multi-select. Adding a non-Library track adds it to the Library too. Every add has Undo.
- **Folders:** crates live in folders ("Friday residency › Warmup, Peak time") that carry over to rekordbox unchanged. The crate header shows the rekordbox path.
- **Notes and summary:** an optional notes line per crate, plus a summary line on every crate and playlist: BPM range, energy range, keys, top genres.
- **"Sort by fit after the selected track":** key compatibility (same key or Camelot neighbors) plus BPM closeness. The Fit column shows dots **with the reason** ("8A → 9A, +1.5 BPM"). Clicking a column header returns to normal sorting.
- **Membership:** the Details panel lists a track's crates, with "Remove from this crate" (undoable).
- **Tags vs crates:** tags (3.1) describe what a track is; crates collect tracks for a purpose. Smart crates (3.3) connect the two.
- **Basics since 1aG (2026-10-02):** a plain Crates screen and "Add to crate" on the Library screen make top-level static crates, rename and delete them, and add or remove Library tracks, each change an undoable operation. A name is trimmed and refused if empty or if it equals another top-level crate's as the XML writer compares siblings (1.9), so a crate the app lets you make never refuses a send. Folders, notes, summaries, the sidebar, drag and drop, the C key, multi-select, adding non-Library tracks, fit sorting and smart crates stay in 1c and Phase 3.
- **Complexity:** M. **Reuse:** the `playlists.rs` tree.

---

### Phase 2: Getting tracks in

#### 2.1 Inbox (watched folders)
- **What:** watch Downloads plus any pool or promo folders. Where new music lands is asked in onboarding, never assumed (4.2). New audio appears in the **Inbox**: identified (if opted in), fingerprinted and quality-checked. The goal is Inbox zero, and the sidebar badge counts what's left.
- **Labels, each with its reason, and filter chips:**
  - **New to you**
  - **Better file** ("FLAC vs your 320 MP3"). Its action is **Upgrade** (2.6), which is always asked, never automatic.
  - **Other cut** ("Radio Edit of your Extended"), in amber, because cuts are nearly interchangeable.
  - **Rework**, labeled neutrally, since it's new music.
  - **On your wishlist** (2.5).
  - **Already have:** in a collapsed group with one **Clear** button. These become extra files.
- **Actions:** **Add to Library** (prominent; Inbox files become managed copies by default, since Downloads is a fragile location, 2.6), **Not now**, and **Hide** (plain text, hard to hit by accident). All have Undo, and hidden files are recoverable (1.11).
- **Batches:** grouped the way files arrived (store order, pool pack, zip), with "Add all". Pool packs show one row per song, with its versions as checkboxes.
- **Previews start about a third of the way in**, only in the Inbox.
- **Read-only:** files stay where they landed.
- **How:** the `notify` crate. Wait until a file is stable, and ignore `.crdownload`, `.part` and `.tmp`. Zips are read in place, or extracted into the app's own staging area.
- **Complexity:** M.

#### 2.2 Identification & tag cleanup
- **Fields:** artist, title, version, remixer, label, year, catalog number, art.
- **Sources:** AcoustID → MusicBrainz and Discogs, each opt-in, plus existing tags. Every suggestion names its source ("MusicBrainz via AcoustID", "from a sibling file", "from the filename") and appears as a diff before anything is applied.
- **Offline:** it still works from tags, the filename/version parser, and sibling files.
- **Complexity:** L. **Reuse:** `enrich.rs`, the integration clients, credentials handling.

#### 2.3 Wishlists
- **Inputs, most reliable first:** a pasted tracklist → CSV (e.g. Exportify) → the Spotify API with your own client ID (opt-in). SoundCloud and Apple are CSV or paste only.
- **Matching:** normalized fuzzy matching, the version parser and ISRC. Results are *owned*, *owned, other cut* (counts as owned), *you have a rework* (a hint, not owned) or *missing*, each with its reason.
- **B2B:** partner tracks from a session (3.6) go to a wishlist in one step.
- **Complexity:** M. **Reuse:** the `integrations/spotify.rs` harness.

#### 2.4 Missing list + store links
- One-click search links to Beatport, Bandcamp, Traxsource and Juno (URL templates only; the app makes no network calls), a "bought" toggle, and notes. No prices.
- **Complexity:** S.

#### 2.5 Auto tick-off
- **What:** when an Inbox file matches a wishlist item, the item is marked acquired and the track goes straight into the Library.
- **"Done for you":** these are listed at the top of the Inbox, saying which wishlist item matched and how. Undo returns the file to the Inbox.
- **Optional:** add it to a crate named after the wishlist.
- **Complexity:** S.

#### 2.6 Managed copies & upgrades
Moved out of the MVP. Needs gate tests T2 and T4 (§7) first.
- **What:** a Library track can become a **managed copy**: the best file's audio → rules gate → convert to the Library format (never upconvert) → complete tags and art gathered from all the track's files (1.7) → named by the template → written into the Library folder through the write guard.
- **When:** on request, for Inbox files (2.1), when the user makes a fragile-location track safe (1.3), or on an accepted **Upgrade** (1.7).
- **First use** asks for the Library folder and format (§1.1).
- **A copy of a rekordbox-known track** is a new Location, which rekordbox treats as a new track. rekordbox's data, including cues and grid, is sent with it (1.9 rule 3, case B). The old rekordbox entry stays, and the app lists it for manual removal. How safely cues carry over, especially after a re-encode, is T2.
- **Library copy tags:** complete, matching the XML exactly, in a fixed frame order, ID3v2.3, and including `TKEY` once rekordbox has reported the key (§5.1; T4 decides the details).
- **Upgrades:** the copy is regenerated at the same path. Cues, grid and alignment survive a same-settings re-encode, **but only after Reload Tag** in rekordbox, so the app lists the affected tracks and walks the user through it.
- **Durability:** a copy survives its original disappearing ("original file missing").
- **Identity:** Library copies are identified by audio hash.
- **Complexity:** M. **Reuse:** `plan.rs`, `execute.rs`, `vet.rs`, `policy.rs`, plus the Phase 2 ports (0.2).

---

### Phase 3: Set prep

#### 3.1 Custom tag vocabulary
- **What:** tag groups you define (vibe, role, vocal, situation), colored, applied from the keyboard and filterable.
- **Import first, don't start empty:** the vocabulary starts as a one-to-one copy of your existing rekordbox My Tags.
- **Into rekordbox:** reading works (1.2). The app hides the `/* … */` block when showing comments and never writes inside it. Writing My Tags is untested (T5). The fallback is comment text plus per-tag playlists under the app's folder.
- **Complexity:** M.

#### 3.2 Genre taxonomy with aliases
- **What:** a genre tree you control, with aliases mapping the mess to canonical nodes. Unmapped genres go to a review queue, and each mapping says which alias rule applied.
- **Seed:** broad and genre-neutral, with real branches across electronic, hip-hop, R&B, pop, rock, Latin, throwbacks and more. The `Parent (Sub)` convention (e.g. `Drum & Bass (Neuro)`) is supported as a style, not assumed as the tree's shape. Files with no genre are the first review queue.
- **Genre belongs to the track, not the song.** A dubstep VIP of a house tune keeps "dubstep".
- **Complexity:** M.

#### 3.3 Smart crates
- **What:** rule-based crates, exported as static playlists refreshed on each send. A rule AST compiles to SQL.
- **Rules read like a sentence:** "energy is at least 7 and tag includes peak", built from dropdowns, with a live count. A text version is available, and smart crates get their own sidebar icon.
- **Always include / Never include** lists (`crate_entry.kind`). Dragging a track onto a smart crate always-includes it; removing one never-includes it.
- **Library tracks only.**
- **Complexity:** M.

#### 3.4 Harmonic "what's next"
- **Signals:** Camelot compatibility, BPM (including half/double time), energy delta in the direction you want, tags and genre, your played transitions (3.6), and optionally audio-model similarity (3.8).
- **Explained:** each ranked track shows its top reasons ("8A → 8B, same BPM, you've played this after X twice"). The weights are adjustable and visible.
- **Tolerance:** covers both tight same-genre blending and bigger BPM or genre jumps, as a user setting, genre-neutral by default.
- The simple Phase 1 version is "Sort by fit" (1.14).
- **Complexity:** M.

#### 3.5 Playlists with an energy arc
- **What:** ordered playlists, made in the app or imported from rekordbox with their order kept.
- **Timeline view:** duration; the energy curve vs a target shape you draw; key and BPM flow with clashes flagged; "fill the gap" suggestions, each saying what gap it fills (key bridge, energy step, BPM). There's no transition preview.
- **Playlists double as crates:** every crate tool works on a playlist directly. "Save as crate" and "Start a playlist from this crate" convert between them.
- **They look different:** a crate sorts by any column; a playlist shows a `#` column and drag-to-reorder.
- **Complexity:** L.

#### 3.6 Play history import
- **What:** plays from every source become `play_event`s, grouped into **sessions**, which gives a transition graph. Play counts and last played already arrive in Phase 1 (1.2).
- **Sources:**
  - **m3u8 from rekordbox's History** (right-click → "Export a playlist to a file"): paths and order, one session at a time.
  - **Gig sticks** (`export.pdb`): "HISTORY 001…", in order but undated, and only the plays from that stick. Sessions are dated on import and can be corrected.
- **B2B:** a session can combine several sources. Without timestamps, interleaving needs the user's help (drag to order). Partner tracks you don't own are kept as plays and offered to a wishlist.
- **De-duplication:** the same gig can arrive twice (stick, then rekordbox after importing the stick). Match by track sequence (T6).
- **Complexity:** M–L.

#### 3.7 Local estimates
- **What:** fast BPM, key and energy (1–10) for tracks rekordbox hasn't analyzed, clearly marked as **estimates**. Until this lands, those tracks show no BPM or key until rekordbox analyzes them.
- **How:** pure Rust with `rustfft` (+ `ebur128`).
  - **BPM (ready):** spectral-flux onsets → comb autocorrelation over 60–200 BPM → fine search → folded into the user's rekordbox BPM range preference.
  - **Key (not shown yet):** use file key tags first, then rekordbox. Show local key only once it reaches ≥80% exact on the reference set.
  - **Energy:** loudness, onset density, spectral flux and BPM, calibrated by percentile within your own library, then against your own ratings. Mixed In Key `Energy N` tags are a source too. The stated basis reads like "loud, dense, fast for your library".
- **Reference set:** the tracks rekordbox analyzed in E1 become a **local** regression test. They're private, so CI uses a synthetic set with known BPMs.
- **Complexity:** L (key is the hard part).

#### 3.8 Local audio ML
- **What:** on-device models, downloaded once. The download is opt-in and goes through the network gate. The models are never part of the installer (§1.1, Optional features).
  - "Sounds like" similarity search, and a similarity signal for 3.4.
  - Genre and vibe **suggestions** (review only, never applied automatically).
  - A better energy estimate.
- **Always labeled:** ML output is presented as the audio model's opinion ("Audio model: sounds similar", "Audio model: Dubstep, 84%"). It is never the only reason for a suggestion, and it can be switched off in settings. Nothing else depends on it.
- **How:** ONNX Runtime via `ort` on the CPU. Embeddings are computed in the job queue and stored in `embedding`. Brute-force search is fine at 100k × 512 floats.
- **Models:**
  - **EfficientAT `mn10_as`** (MIT) for every track: an embedding plus AudioSet genre and mood tags in one cheap pass.
  - **LAION-CLAP music** (Apache-2.0/CC0), run lazily: "sounds like" plus text search.
  - Later: small per-user heads trained on the user's own genres and tags.
- **Constraints:** GPL-compatible weights; pinned model versions; credit for CC-BY models. The training-data licensing (AudioSet/LAION) goes to legal review.
- **Complexity:** L–XL. It's the last Phase 3 item, so nothing else depends on it.

---

### Phase 4: Public v1 (Windows)

#### 4.1 Portability bundle
- Export the DB state, and optionally the Library files, into one bundle. Import it on another machine; paths re-link by volume identity plus relative path.
- **For your own machines only.** It's never described, tested or promoted as a way to hand music to someone else.
- **Complexity:** M.

#### 4.2 Onboarding
Polish on top of the Phase 1 first run (1.3):
- An **online-services step:** each service is off by default and says what it sends.
- An optional ML model download.
- **"Where do you put new music?"** The Inbox watch folder is asked for, never assumed.
- **OneDrive check:** if music folders are OneDrive-synced, explain the risk plainly and offer to guide "Always keep on this device" or moving the music.
- **rekordbox extras:**
  - Suggest turning on "Add My Tag to the comments".
  - Recommend closing rekordbox before reading a gig stick (§5.4).
- An **unsigned install guide** (README and download page).
- **Complexity:** M.

#### 4.3 Diagnostics
- An in-app log viewer and "Create diagnostics bundle". With no telemetry, this is the only debugging detail from other people's machines, so it must be good. It contains:
  - logs, version and OS
  - DB stats and schema version
  - recent job failures
  - a preview of exactly what's included, with paths and track names redacted by default
- **Complexity:** S–M.

#### 4.4 Docs & community
- A docs site, and issue templates for suggestions (version-parser patterns, pool naming, genre aliases). Code contributions aren't accepted yet; the README says so.
- A GPL notice plus a third-party license page generated from dependencies.
- A non-affiliation disclaimer (rekordbox, CDJ and store names in plain text, never logos), and a privacy statement listing what each opt-in service receives.
- **Complexity:** M.

---

### Later / backlog

| Feature | Why deferred |
|---|---|
| macOS release | Waits for a tester; CI keeps it building. Unsigned Mac apps are harder to install, so revisit the $0 decision then. |
| Linux release | rekordbox doesn't run there. |
| CLI / headless mode | The job system makes it cheap later. |
| Cue alignment for re-encoded copies | Only if T2 (before 2.6) shows cues drift. It needs audio alignment for MP3 encoder delay. |
| Disk cleanup report | Space freed by deleting extra files, leftover `.asd` files, empty and unreadable files. The app never deletes anything itself. |
| Plugin / extension API | Waits until the core APIs are stable. |
| Full history timeline UI | The operation log exists from day one. |
| Tracklist generation | Easy once play history exists. |
| Gig log (venue, tracks played) | Out of v1 scope. |
| Native audio engine (headphone output, transition preview) | Simple preview was chosen. |
| Cloud LLM features | Local ML only was chosen. |
| Serato / Traktor / Engine export | rekordbox only for now. The export layer stays pluggable; this is the path to DJs on other software. If built, it is an optional feature the user turns on (§1.1). |
| Store page lookups / prices | Ruled out (ToS). |
| Live multi-machine sync | The bundle comes first. |
| Writing gig USBs / OneLibrary | **Never.** There's no public spec, and a broken stick at a gig is the worst possible failure. |
| Stick check: "on your stick but not in your Library" | Cheap once sticks are read. |
| Rebuilding a B2B set's order from its recording | Fingerprint matching over `PIONEER REC` / `ALPHATHETA REC` recordings. |
| rekordbox 6 support | Revisit if users ask and the XML proves identical. |

---

## 5. rekordbox behavior and known traps

This section is the single home for these facts. Other sections link here.

### 5.1 rekordbox writes into audio files
- It writes tag edits made in rekordbox (MP3), and its detected key as `TKEY` into files made by our encoder. It did *not* do this for 22 key-less store MP3s; T4 finds the trigger.
- It writes the `/* My Tag */` comment (FLAC) when "Add My Tag to the comments" is on.
- It rewrites AIFFs at import.
- It sets a new modified time on every file it adds or analyzes.
- Setting cues doesn't touch files.
- **So:** never assume a rekordbox-managed file is unchanged; identify files by `audio_hash`; on an mtime-only change, check a partial hash before re-fingerprinting. After a tag rewrite (new size and mtime), skip the re-fingerprint when the hash stage is current and the `audio_hash` is unchanged (1aC-10).
- **Reload Tag is a full overwrite from the file.** It even blanks a comment the file lacks, so Library copy tags must be complete.

### 5.2 XML import behavior (rekordbox 7)
These are observed behaviors, not documented ones, and any 7.x update can change them. **Re-run the rekordbox behavior check** (`spikes/rb-kit/CHECK.md`, about an hour; it includes the T1/T3 gate tests, built in 0D) after every rekordbox update, before trusting sends again.

- A first import raises no dialog about the tracks (with auto-analysis on, rekordbox shows its analysis-settings popup on each add). Awkward paths resolve: accents, `# % + & '`, CJK, emoji.
- On first import, XML values win over the file's own tags. This covers title, artist, album, genre, comments, rating, color, composer, remixer, label, mix, year and track #.
- On a first import with auto-analysis on, BPM and key from the XML are ignored. **On a Yes re-import they are loaded** (`AverageBpm`, `Tonality`), even over rekordbox's own analysis, while its grid stays: the BPM shown can then disagree with the grid. Omitting them keeps rekordbox's values (T3).
- **Re-importing an existing track asks per track, Yes or No** ("don't ask again" exists). **Yes is a full overwrite from the XML**: it undid a rekordbox-side title edit even though the XML hadn't changed that value.
- **A missing attribute can reset data** (an omitted Rating became 0), and **a default can erase it** (`PlayCount="0"` wiped real plays). An omitted Colour was kept. That's inconsistent, so never rely on omission.
- Cues and grids the XML doesn't mention are left alone. My Tags and playlists made in rekordbox survive.
- Same-name playlists are **replaced**, contents and order. Playlists missing from the XML are left behind. Tracks missing from the XML stay in the collection.
- TrackIDs are stable between exports but **reassigned on import**, so match by Location.
- Nested playlist folders and their order survive.
- **Unverified:** whether two playlists in one folder whose names differ only in letter case, Unicode form (composed or decomposed) or trailing whitespace count as the same name (and so replace each other). Until the behavior check covers it, the writer treats them as the same and refuses the send (1.9).
- **Unverified:** whether a re-sent track is matched to rekordbox's own when its `Location` is a network path, a macOS path, or any spelling other than `file://localhost/C:/…`. The behavior check only re-sent drive paths. Until it covers the others, the writer sends such a `Location` back exactly as rekordbox wrote it (1.9).
- **A playlist entry only resolves against the same XML's COLLECTION** (T1): an entry for a track not in it is dropped silently, whether keyed by TrackID (`KeyType="0"`) or Location (`KeyType="1"`). rekordbox exports every playlist as `KeyType="0"`.
- The re-import dialog is titled with the track name and offers Yes / No / Don't ask me again; replacing a same-name playlist asks OK / Cancel. Their exact wording is in the author's private 2026-09-28 check results.
- Last verified: rekordbox 7.2.19, 2026-09-28 (all checks except T1 behave as listed).

### 5.3 Parsing rekordbox exports
- `Location` has raw `# ( ) , + !` and lowercase `%xx`. Decode it by hand; a URL parser cuts at `#`. Besides drive paths (`file://localhost/C:/…`) it can be a network path (`file://localhost//server/share/…`) or a macOS path (`file://localhost/Users/…`).
- An export that isn't well-formed XML (cut off, a broken attribute, playlist folders nested absurdly deep) is refused as a whole, never read as a shorter collection: missing tracks would look deleted. Bad values inside a well-formed file only clear that field. A COLLECTION whose track count disagrees with its `Entries` is read but marked incomplete, and its missing tracks are never treated as removed (1aA-8, 2026-09-28).
- `Grouping` holds rekordbox's color name, not user data.
- Exported `Size` and `BitRate` can be stale, so always read them from the file.
- `TotalTime` is whole seconds, truncated, not rounded (255.634 s exports as 255).
- `Tonality` may follow the user's key-notation setting.
- Skip:
  - rows marked deleted (`rb_local_deleted`)
  - built-in demo tracks and samples (`Music\rekordbox\Sampler`, `Music\PioneerDJ\Demo Tracks`), matched as path components under `Music` for any user or drive, ignoring case
  - the "CUE Analysis Playlist"
  - streaming entries (`soundcloud:`, `spotify:`, any non-file scheme), kept in the snapshot and shown as "streaming, no file", never matched to a file
- rekordbox's default export folder is Documents.

### 5.4 Gig sticks
- rekordbox rewrites a stick's databases the moment it's plugged in. Read sticks with rekordbox closed, and never trust file dates for "when was this gig".
- `export.pdb` history is numbered but undated, and holds only plays from that stick.
- Old gig-USB copies inside music folders (`Contents\` in rekordbox's `Artist\Album\` layout) are a major source of exact duplicates. Grouping handles that.

### 5.5 Scanning
- Skip `._*` (macOS AppleDouble) and `.DS_Store`.
- The walk doesn't follow symlinks, junctions or mount points inside a music folder; a folder reached that way is indexed only if it's itself inside a music folder (1aA-4, 2026-09-29).
- Read tags leniently. Broken APE/ID3 blocks or a bad `TDRC` must not stop the audio being indexed.
- Detect the format from the bytes (a "`.wav`" can be an MP3).
- Flag truly broken files, such as an M4A with no `moov` atom from an interrupted download.
- OneDrive "online-only" placeholders trigger a download when read, so skip them unless the user opts in.

### 5.6 Engineering traps
- **Unknown tag frames** (Serato cues, custom frames) must survive every write. Port those tests first.
- **Our tag writer (lofty) isn't deterministic:** sort frames before writing so identical input gives identical bytes.
- **Migrations are never edited after they've been applied** (enforced in CI).
- **Read-only is enforced in code:** only the write guard (`write_guard`, 0E-7) hands out write access, scoped to the app data folder (the Library folder joins in Phase 2). It resolves links and `..` before allowing a path, refuses writes through hard-linked files, opens SQLite only on guarded paths, and blocks `ATTACH`, `VACUUM INTO` and temp-file redirection at runtime. A source scan fails CI on any write API outside the guard, and a runtime test checks nothing else changes. A file that must never be seen half-written (the rekordbox XML) goes through `write_then_rename`: a new file beside the destination, flushed, renamed over it, and deleted on any failure (1aE-7). What a write cut short by a crash leaves behind is cleared by `remove_leftover_parts`: exactly the names `write_then_rename` can give its new file, in that folder, plain files only. Both refuse a root itself as the destination, since the file beside it would be outside the root (1aF-1). The WebView2 profile Tauri keeps under `%LOCALAPPDATA%\com.tracklistpro.desktop\` is written by the runtime, not by app code.
- **The SQLite DB never lives in a synced folder;** it's in `%APPDATA%`. Its write-ahead log file (`tracklist-pro.db-wal`) is kept small (DOGFOOD C1-13, 2026-10-10): SQLite cuts it back to 32 MiB whenever the log starts again, and it's folded into the database and emptied when the database is opened, when the last job in the queue ends, and when the app closes (the process ends without closing its connections, so SQLite never tidies up by itself). A checkpoint that meets a read in progress gives up at once and the next of those points tries again; while one large transaction is still being written the log can be bigger than the limit. The repo itself lives off OneDrive too (`C:\dev\tracklist-pro`, CLAUDE.md "Parallel work"), so `.git`, `target/` and `node_modules` never sync.
- **Open files through `\\?\` paths on Windows.** A plain path drops trailing dots and spaces from names, so `Q.X.Z.\a.mp3` can silently open a sibling `Q.X.Z\a.mp3`, and names like `CON` can open devices on Windows 10. The path model resolves to `\\?\E:\…` or `\\?\UNC\…` (found in the 0C-9 review, 2026-09-28).
- **WebView2 can't play AIFF or ALAC,** so decode them in Rust for preview.
- **Coexisting with musicmanager:** different bundle ID, data folder, updater endpoint and credential-store prefix. Test both installed side by side.
- **MusicBrainz allows 1 request per second.** Every request passes through the network gate.
- **No API secrets in the binary.** Users bring their own Spotify and Discogs keys; AcoustID uses its application-key model.
- **Licensing:** LAME (LGPL), Symphonia (MPL-2.0), ONNX Runtime (MIT) and the model weights must all be GPL-3.0-compatible, checked by `cargo-deny` in CI.

### 5.7 Online service terms
Read each service's terms before wiring it up, and recheck them when the project's situation changes.

| Service | Watch for |
|---|---|
| AcoustID | Free for non-commercial use. Recheck if the project ever takes donations or sponsorship. App key model. |
| MusicBrainz | 1 request per second, a descriptive User-Agent. Core data is CC0, some supplementary data isn't. |
| Cover Art Archive | Images belong to their owners. Embedding them in the user's own files is fine. |
| Discogs | Attribution and image-use rules. Users bring their own token. |
| Spotify | The strictest developer policy: check its limits on pairing Spotify data with other services (the wishlist → store links flow) and on ML use. Users bring their own client ID. Paste and CSV imports avoid the question. |
| Store links | Search-URL templates only, no scraping (already decided). If affiliate links are ever added, they need disclosure. |

---

## 6. Success criteria

| Phase | "Done" means |
|---|---|
| 0 | CI is green (full tests on Windows, builds on macOS and Linux). Ported tests pass. The architecture tests exist: network gate, migrations, write guard, reason strings. |
| 1a | A Library started from rekordbox, and separately one started fresh, arrive in rekordbox 7 via XML with BPM, key, play counts and cues intact. Starting from rekordbox adds no duplicate rekordbox entries, and re-sending duplicates nothing. T1 and T3 are answered and the writer follows the results. The app has changed nothing outside its data folder (rekordbox itself rewrites tags in audio files, §5.1; wording settled at the checkpoint, 2026-10-10). **Dogfooding:** the owner has done a real read → send → import cycle on an installed build and logged the friction (done 2026-10-10, `DOGFOOD.md`). The second cycle, around a practice gig, moved to the end of Phase 1 (owner decision, 2026-10-10): without search and full crates the app isn't gig-ready, so the gig test belongs to 1c. |
| 1b | Duplicates of Library tracks get a correct best-file suggestion with a reason. False merges of different versions are under 1% on the test corpus. Relink confirmations and "Better file available" work through Review. |
| 1c | An edit made on both sides shows up as a conflict and resolves cleanly. The 100k synthetic fixture stays responsive. Crates carry over to rekordbox with their folders. **The owner has prepped and played a real or practice gig with it**, starting fresh with a set-sized Library and sending again after preparing the tracks in rekordbox, **and has then brought their whole collection into the Library** (both moved here from 1a, 2026-10-10). |
| 2 | A Bandcamp zip dropped into Downloads is identified, deduplicated, ticked off the wishlist and added to the Library as a managed copy, without manual file handling. An upgrade keeps cues after the guided Reload Tag. |
| 3 | A real or practice gig is prepped using only crates, "what's next" and a playlist with an energy arc, then sent to rekordbox. Every suggestion along the way shows its reason. |
| 4 | Another DJ on Windows installs the unsigned build from the docs alone, opts into services, and moves their own Library to a second PC with a bundle. |

## 7. Open questions

**Gate tests (with the demo rekordbox)**
- **Before the XML writer (1a):**
  - ~~**T1: changed-tracks-only send.**~~ **Answered 2026-09-28 (7.2.19): no.** Playlist entries only resolve against the same XML's COLLECTION, so 1.9 rule 4 uses the full-send fallback.
  - ~~**T3: omitted analysis fields.**~~ **Answered 2026-09-28 (7.2.19): omitting them keeps rekordbox's analysis**, so 1.9 rule 3 (A) holds.
- **Before managed copies (2.6):**
  - **T2: a copy at a new Location.** Send a copy with cues, grid and play count in the XML, once as an unchanged MP3 and once as a WAV re-encoded to MP3. Check whether cues land exactly, whether play counts come through, and how far re-encoded cues drift.
  - **T4: the key-tag trigger.** Why does rekordbox add `TKEY` to files from our encoder but not to key-less store MP3s? Try ID3v2.3 vs v2.4, sorted frames, and a pre-filled TKEY.
- **Later:**
  - **T5: creating My Tags via XML** (e.g. comments with `/* Tag */`). Needed for 3.1.
  - **T6: does rekordbox clear a stick's history after importing it?** Needed for 3.6 de-duplication.

**Design, still open**
- **How energy reaches rekordbox.** Options include a Mixed In Key-style `Energy N` prefix in Comments or a file tag, one My Tag per energy level, a rating or color mapping, or not sending it. It affects 1.9 and MIK interop, so decide it before 1.9 is finalized (1c). Owner input (2026-09-30): no Mixed In Key and no energy ratings so far, so there's no existing habit or tag format to keep.
- **The UI word for linked vs copy** (Phase 2, after T2).
- **Phase 1 details:** the best-file ranking weights; the rule for choosing between files that disagree on a tag; which fields appear in conflict review by default.
- **Phase 2:** the default naming template.
- **Phase 3:** getting key estimation to ≥80% exact; the genre seed tree.

**Legal:** the license, trademark and privacy summary is in [NOTICE.md](NOTICE.md). The detailed legal notes are private.

## Status

- **Phase 0:** spikes done (2026-09-27). Stages 0A–0E done 2026-09-28: repo, CI (trimmed, with a changed-paths gate), schema, ports, fixtures, the rekordbox behavior check (T1/T3 answered), job queue, network gate, write guard, suggestions and undo. Phase 0 closed with Stage 0F.
- **Phase 1a:** Stages 1aA–1aF done by 2026-10-02: scan, hashing and fingerprints, the rekordbox XML reader, relink and attach, starting a Library, the XML writer, the guided send and the after-send lists. Stage 1aG (the owner's Phase 1a check) is open: end-to-end tests, crate basics and the installer are in; the owner's two real cycles are next. Stage 1bA (building blocks for duplicates and versions) started early. See [TASKS.md](TASKS.md).
