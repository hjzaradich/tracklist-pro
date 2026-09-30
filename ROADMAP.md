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
| AI / ML | Local audio ML only; labeled and optional | No cloud LLM. ML signals can be switched off, and nothing depends on them (3.8). |
| In-app audio | Simple preview in the webview | No native engine and no headphone routing. AIFF and ALAC are decoded in Rust. |
| musicmanager | **A blueprint only; never part of the owner's workflow** (owner, 2026-09-30) | Nobody relies on it, so there's no freeze to time. Separate bundle IDs, data folders, updater endpoints and credential prefixes, so both can be installed side by side. No shared DB and no migration. Ported modules are copies (0.2); nothing is ported back. |
| Frontend | React | |
| Language | English, translation-ready | All UI strings go through `i18next`. |
| Theme | Dark first, light available | Design tokens; both themes are tested. |
| Layout | Sidebar · table · Details panel · player | The sidebar lists stages in workflow order with count badges, then crates and playlists. The top bar holds search/palette and Activity (background work). |
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
| **Cut** | A version with the same production but a different length or lyrics: Extended, Radio Edit, Club Edit, Clean/Dirty, Intro/Outro, Short. Nearly interchangeable | `version_link.kind = cut` | edit |
| **Rework** | A different production: remix, VIP, flip, bootleg, rework, dub, cover, live, alternate arrangement, mashup. Never a stand-in | `version_link.kind = rework` | remix (as a catch-all) |
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
| `volume` | identity (serial-first; never a drive letter), label, guid, last_mount_path, kind (internal / external / network) | Portable path identity. |
| `music_folder` | volume_id, rel_path, watch (bool), role (scan / inbox), unreadable_folders, unreadable_files, walked_at | The counts are from the last finished walk (1aB-14). |
| `file` | music_folder_id, rel_path (on-disk form), rel_path_key (NFC, for matching), size, mtime, file_id (volume serial + NTFS file index, as text), blake3, **audio_hash**, partial_hash, fingerprint, fingerprint_audio_hash, **sniffed_format**, codec, bitrate, sample_rate, duration, **cutoff_hz**, quality_verdict, raw_tags (JSON: one array of {key, value} per tag block type, e.g. id3v2, ape; `{}` when untagged, NULL when the tags couldn't be read), present (bool), online_only (bool: a OneDrive placeholder, never opened unless the user opts in), hidden_reason (null / user / inbox_cleared) | One row per physical file, never written to by the app. `audio_hash` covers only the audio frames, because rekordbox rewrites tags. `partial_hash` is the cheap check for an mtime-only change (1.1). `fingerprint_audio_hash` is the `audio_hash` the fingerprint was made from, set only when the hash stage was current at that stat (1aC-10). `sniffed_format` comes from the bytes. |
| `file_stage` | file_id, stage (read / hash / fingerprint), version, size, mtime, status (done / failed / skipped), reason, done_at | What each scan stage last did to a file, and at which size, mtime and stage version, so a stage redoes only changed files (1aB, 2026-09-29). **Failed** means the content couldn't be processed and waits until the file changes; a file that couldn't be reached is never failed, only skipped or left unrecorded, so it's retried. Deleted with its file. |
| `recording` | canonical title / artist / version / remixers / label / year, genre_id, per-field provenance; display bpm / key / energy | A **track**. The display bpm/key/energy are **derived from `analysis`** and never edited directly. |
| `analysis` | recording_id, source (rekordbox / tag / MIK / user / local / ml), bpm, key, energy, confidence | **The source of truth** for these values: several per track. Precedence: BPM and key go rekordbox > file tag > MIK > local; energy goes user > MIK > local > ML (the audio model is optional and never the only reason, so it ranks last). Disagreements are flagged. The track's shown values come from the `recording_display` view, so they can't drift or be edited directly. |
| `recording_file` | recording_id, file_id, role (best / undecided / extra), match_confidence | A track's duplicates. "Extra" is purely a DB state. |
| `version_link` | recording_a, recording_b, kind (cut / rework), label (extended, radio, clean/dirty, remix, VIP, flip, bootleg, cover, live, arrangement, mashup-contains…), source (parser / fingerprint / user), confirmed | Nothing is inherited across a link. |
| `library_track` | recording_id, kind (copy / linked), managed rel_path (copy), linked file_id (linked), format, source_file_id (nullable), source_status (ok / missing), **last_sent_location**, last_exported_at | Deleted only by an explicit in-app action. The rekordbox TrackID isn't stored, because it's reassigned on import (§5.2). |
| `rekordbox_track` | TrackID, location, location_key, matched file_id and recording_id, relink method, confidence and **relink_probable**, bpm, key, beatgrid, cues, play_count, last_played, rating, color, comments, My Tags, playlists, read_at | A read-only snapshot, refreshed on every read. The TrackID is valid within this read only. `relink_probable` marks a match that isn't trusted yet (filename only, or a unique duration no title tag agrees with): its file is taken, but no rekordbox data is attached until the user confirms it, and a confirmation (a `relink` row) re-applies it as trusted (1aC-5, 2026-09-30). |
| `relink` | location_key (rekordbox Location decoded by hand, NFC), file_id, method, confidence, confirmed_at | A confirmed relink, kept apart from the `rekordbox_track` snapshot so it survives every fresh read and is re-applied by Location (0D-1 review, 2026-09-28). |
| `sync_base` | library_track_id, field, value_at_last_sync | Tells "app changed", "rekordbox changed" and "both changed" apart. |
| `conflict` | library_track_id, field, app_value, rekordbox_value, base_value, status | The review queue (1.10). |
| `tag` / `tag_group` | name, group (vibe / role / situation / vocal…), color, rekordbox mapping | |
| `genre` / `genre_alias` | tree (parent_id), aliases | |
| `crate` | kind (static / smart), rules JSON, folder tree, notes | Exported inside the `Crates` folder. Smart crates are exported as static playlists. |
| `crate_entry` | crate_id, library_track_id, kind (member / always_include / never_include) | Smart crates use the always/never lists (3.3). |
| `playlist` / `playlist_entry` | order, target energy, notes, origin (app / rekordbox) | Exported inside the `Playlists` folder. |
| `play_source` | kind (rekordbox_db / history_m3u8 / usb_pdb / xml_playcount), label, device, imported_at, dated (bool) | USB sessions are numbered but undated (§5.4). |
| `play_event` | recording_id (nullable), raw artist/title/path, play_source_id, position, played_at (nullable), prev_play_event_id | A null `recording_id` means a partner's track you don't own. |
| `session` | name, date, venue?, notes; many play_sources | The UI calls it "gig" or "set". |
| `wishlist` / `wish_item` | source (Spotify / CSV / paste / session), raw artist/title, matched recording_id, source_play_event_id (nullable), status | |
| `inbox_batch` | kind (store order / pool pack / zip / loose), label, arrived_at | 2.1 grouping and "Add all". |
| `inbox_item` | file_id, batch_id, status (new / not_now / hidden / added), label (new / better_file / other_cut / rework / on_wishlist / already_have), reason, arrived_at | |
| `job` | kind, target, priority, status, progress, error, attempts, timestamps | Persisted so jobs resume after a crash (0.1). |
| `setting` / `service_optin` | key/value; service, enabled, enabled_at | The network gate checks `service_optin`. |
| `operation` / `change` | carried over from musicmanager; `change` records an action (set / insert / delete) so a real NULL value is never mistaken for a created or removed row | Undo and history. `operation` stores a kind plus details, never UI text. |
| `embedding` | recording_id, model_id, vector | 3.8. |

**When tables arrive:** each migration is added in the phase that first needs it (§1 principles).
- **Phase 0–1:** `volume`, `music_folder`, `file`, `recording`, `recording_file`, `version_link`, `analysis`, `library_track`, `rekordbox_track`, `relink`, `sync_base`, `conflict`, `crate`, `crate_entry`, `operation`/`change`, `job`, `setting`, `service_optin`.
- **Phase 2:** `inbox_batch`, `inbox_item`, `wishlist`, `wish_item`.
- **Phase 3:** `tag`/`tag_group`, `genre`/`genre_alias` (until then, genre is a plain canonical tag), `playlist`/`playlist_entry`, `play_source`, `play_event`, `session`, `embedding`.

**Provenance:** every field records where its value came from and how confident that is. Review can then show "Key 8A (rekordbox) vs 9A (file tag)" instead of silently picking one, and every suggestion can state its reason.

---

## 3. Phases

| Phase | Theme | Outcome |
|---|---|---|
| **0** | Foundations | Private repo, CI (Windows tests; macOS/Linux builds), the architecture pieces that are cheap now and costly later, and the ports Phase 1 needs. Spikes are done (0.4). |
| **1a** | Prove the loop | Scan music folders, read rekordbox with relink, start a Library of linked tracks, send to rekordbox, and use it for real or practice gigs. The gate tests T1 and T3 (§7) come first. The MVP never writes an audio file. |
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
- **Job queue:** typed (scan, read, hash, fingerprint, read_rekordbox, analyze, embed, convert, export), with priorities, progress events and cancellation, persisted in the `job` table. Crash resume is added in 1c. Every long operation goes through it.
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
| Operation log / undo | `ops/mod.rs` | Phase 0 | Multi-step undo added in 1b, when "Accept all" needs it. |
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
- **Scanning rules:** see §5.5 (skip AppleDouble files, read tags leniently, sniff the format, flag broken files).
- **OneDrive placeholders:** reading one triggers a download. Detect the placeholder attributes and skip unless the user opts in. Onboarding explains the risk (4.2).
- **Target:** first results in under 5 s on 100k files, and a full index without fingerprints in minutes.
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
  3. A unique duration within the candidate set: the files directly in the folder the Location's folder still names, plus the folders its neighbours (tracks from the same rekordbox folder, matched by path or filename + duration, or confirmed) were matched into; at most 50 files, and none of unknown duration. Accepted only when a title tag agrees with rekordbox's Name (featuring credits, case, punctuation and spacing folded; bracket contents kept); otherwise **probable**, like step 5 (1aC-5, 2026-09-30).
  4. The acoustic fingerprint (1.4).
  5. Filename only. Marked **probable** and queued in Review as "Confirm relink" before any rekordbox data is attached.
  6. A connected gig stick as a recovery source.
  
  **Never match on size:** rekordbox rewrites tags, which changes the size, and the XML's `Size` can be stale (§5.3). Use `audio_hash` as a confirming check where available. Whatever is left is listed as "missing" with its last known path.
- **Skip what isn't a track:** deleted rows, rekordbox's demo tracks and samples, the "CUE Analysis Playlist", and streaming entries, which are shown as "streaming, no file" (§5.3). Streaming tracks are out of scope beyond that: they're never linked, sent or checked in the behavior check (owner, 2026-09-29).
- **Parsing:** see §5.3.
- **Complexity:** M, plus relink (M).

#### 1.3 Start your Library · 1a
- **First run:** point the app at your music folders, then choose:
  - **Start from rekordbox (main):** your whole collection or chosen playlists become **linked** Library tracks (1.8). There are no new rekordbox entries, no re-encoding and no file writes, and the Library looks like what you already play. Their duplicates and problems queue in Review (1b).
  - **Start fresh:** an empty Library, for new DJs, DJs switching software, or a clean start. Tracks you add are linked to their files where they are.
- **Fragile locations:** a Library track whose file lives in Downloads, a temp folder or an external drive gets a visible warning with its reason ("this file is in Downloads; if you clear Downloads, the track goes missing"). Making a safe copy arrives in Phase 2 (2.6).
- **Tracks whose file is missing** when you start from rekordbox don't become Library tracks yet. They wait in Review's **Missing** list (1aD-5) with their rekordbox data intact (BPM, key, cues, play counts, playlists), until a file is found and married to them by relink; then they join the Library as linked tracks. From Phase 2 they can also go on the wishlist. Decided 2026-09-28.
- The Library folder and format are chosen when managed copies arrive (2.6). Importing playlists with their order comes in 3.5.
- **Complexity:** S–M.

#### 1.4 Acoustic fingerprinting + duplicate grouping · 1b
- **How:** exact blake3 or audio-hash matches are the fast path. Otherwise, index fingerprints by blocks to find candidate pairs, compare them, and group.
- **Decision rule:**
  - **Duplicate:** matched coverage ≥ 90% on *both* files and a difference score ≤ 4.
  - **The version check (1.5) can veto** a merge, because Clean vs Dirty fingerprint as 100% identical.
  - **One-sided coverage** (a Radio Edit found inside an Extended mix) means a **cut**: link it, don't merge.
  - **Low or no coverage:** names decide whether it's a rework link.
- **Speed-up:** a short-window fingerprint (e.g. the first 120 s) may be used **only to find candidates**. Duplicate and cut decisions always use full-track fingerprints: one E3 pair only diverges after about 2 minutes.
- **Cost:** measured in 1aB-7 (release build, one core, decode included): about 0.3 s (WAV), 0.4 s (FLAC) and 0.6 s (MP3 320) per minute of audio, so 1.5–3 s for a five-minute track. A large real library (thousands of tracks) takes 1–2 hours of background time on 4 threads. Runs in the background at below-normal priority on a shared budget of at most 4 threads, visible tracks first. The short-window candidate pass is the lever if big libraries need it.
- **Risk:** false merges. Mitigated by version awareness and by always letting the user split a group.
- **Complexity:** L.

#### 1.5 Version awareness · 1b
- **What:** never merge different versions. Link them instead, as one of two kinds (§1.2):
  - **Cuts:** Original ↔ Extended / Radio Edit / Club Edit / Short, Clean ↔ Dirty, Intro / Outro / Intro-Outro, Quick Hit.
  - **Reworks:** remix (per remixer), VIP, flip, bootleg, rework/refix/reboot, dub, cover (labeled "cover"), live, alternate arrangement, and mashups (`A x B`, linked to each source they contain).
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
- **Verdicts:** `ok` / `low bitrate` / `suspect transcode` / `clipped` / `truncated` / `broken`.
- **How:** FFT spectral cutoff (the highest frequency within 50 dB of the 2–8 kHz level, from a 60 s sample), decoded vs header duration, and decode errors.
- **Thresholds:**
  - MP3 at ≥256 kbps with a cutoff below 17 kHz → `suspect transcode` ("this 320 was made from a ~128 kbps source").
  - 17–19 kHz → shown as "possibly lower quality", not flagged.
  - Lossless with a cutoff below 19.5 kHz → `suspect transcode` ("this FLAC/WAV was made from an MP3").
- Suspect files are reviewed in Review (1.12): "Keep anyway" handles genuinely rolled-off masters, and "Find a better file" sends the track to the wishlist.
- **Complexity:** M.

#### 1.7 Best-file picker · 1b
- **Ranking:** real quality (the cutoff, not the header bitrate) → format → tag completeness → embedded art → whether rekordbox knows the file → location preference. Every suggestion states its reason.
- **Audio and tags are picked separately.** The best file decides only the audio. Each tag and the cover art come from whichever of the track's files has the best value, including extra files. When files disagree, the app picks one by rule and Review shows the disagreement.
- **Linked tracks:** their audio *is* the file rekordbox uses, so in the MVP a better duplicate shows as **"Better file available"**, with its reason ("FLAC vs your 320 MP3"). The **Upgrade** action arrives with managed copies (2.6). Chosen tags reach rekordbox only via the XML, because the file is read-only.
- **Resolving changes DB state only.** Nothing on disk changes.
- **Complexity:** M.

#### 1.8 Add to Library · 1a
- **In the MVP every Library track is linked:** it points at an existing file. For tracks rekordbox knows, that's the file rekordbox already uses, so nothing changes in rekordbox and cues and history stay safe. For tracks rekordbox doesn't know, the next send adds them at their current Location.
- **Read-only:** the file stays where it is and is never written. Tag fixes reach rekordbox only via the XML (on first import, XML values win over file tags, §5.2).
- **Carried over:** rekordbox's data (1.2) and the best tags gathered from all the track's files (1.7), sent through the XML.
- **Fragile locations** are warned about (1.3).
- **Durability:** "file missing" when the file disappears. An in-app delete asks for confirmation, can be undone, and drops the track from the next send.
- Managed copies (converting, clean tags written into a new file, upgrades) are Phase 2 (2.6).
- **Complexity:** S.

#### 1.9 Send to rekordbox (XML) · 1a
- **What:** write the Library and its folders as rekordbox XML:
  - **COLLECTION:** tracks, per rule 4.
  - **PLAYLISTS:** two top-level folders, `Crates` and `Playlists`, holding their trees as NODEs.
  
  The user then imports it through rekordbox's `rekordbox xml` sidebar.
- **XML writer rules** (the behavior behind them is in §5.2):
  1. **Every attribute of every track sent, every time.** Fill each with rekordbox's current value from a fresh read, unless the app means to change it. Never omit, never default. The only exception is the analysis fields in rule 3.
  2. **Read → merge (1.10) → send in one step.** Tell the user not to play in rekordbox between Send and the import.
  3. **Analysis fields** (`TEMPO`, `POSITION_MARK`, `Tonality`, `AverageBpm`) depend on the case:
     - **(A) The track is already in rekordbox at this Location:** omit them.
     - **(B) A managed copy at a new Location** (Phase 2, 2.6): send them deliberately, carried over from rekordbox's entry for the old file.
     
     **Confirmed by gate test T3** (2026-09-28, rekordbox 7.2.19): omitting them keeps rekordbox's key, BPM, grid and cues. **Never send the app's own BPM or key estimates for a track rekordbox already has**: a Yes re-import loads any `Tonality`/`AverageBpm` the XML carries, over rekordbox's analysis and out of step with its grid.
  4. **COLLECTION holds every track that a sent playlist or crate references, plus new tracks, each with every attribute (rule 1).** Gate test T1 failed (2026-09-28, rekordbox 7.2.19): a playlist entry only resolves if its track is in the same XML's COLLECTION; other entries are dropped silently, whether keyed by TrackID or by Location. So playlists-only sends are impossible. To keep a send small, it may carry only the playlists and crates that changed plus their tracks. Every track already in rekordbox raises the Yes/No dialog (or the user ticks "Don't ask me again"), so the checklist states how many dialogs to expect.
  5. Write `Location` fully percent-encoded.
  6. **App-owned playlists live only under `Crates` and `Playlists`.** When a crate is renamed or deleted, the next send lists the stale rekordbox playlists to delete by hand.
  7. **Removing a Library track can't reach rekordbox via XML.** The app says so and lists what to remove.
  8. **Record `sync_base`** for every field sent.
- **UI:** **Send to rekordbox** is the Overview's main button (1.13). It opens a guided checklist covering rekordbox's side: switch the XML file, Import to Collection, answer the dialog. It is not a one-click sync.
- **Complexity:** M.

#### 1.10 Conflict detection & review · 1c
- **Three-way comparison per field** (app vs rekordbox vs `sync_base`):
  - only the app changed → send it
  - only rekordbox changed → pull it in
  - both changed → **conflict**
- **Conflicts queue:** **Review → Conflicts**, with side-by-side values. Keep the app's value, keep rekordbox's, or edit. Resolve in bulk ("take rekordbox for all ratings").
- **Analysis fields never conflict**, because the app doesn't edit them. Conflicts are about tags, genre, comments, rating, color and playlist membership.
- **This is mandatory.** A rekordbox "Yes" is a full overwrite (§5.2), so any rekordbox-side edit not merged before sending is destroyed.
- **Second channel:** a changed file tag on a linked track is another sign of an edit made in rekordbox (§5.1).
- **Complexity:** L.

#### 1.11 Track browser with stages and keyboard · 1c
- **What:** a virtualized table with full-text search, filters, columns and a preview player.
- **Stages:** Overview → Review → All music → Library → Crates; Inbox arrives in Phase 2 and Playlists in Phase 3.
- **Hidden filter** (in All music): lists everything the app isn't showing, each with its reason and a way back:
  - "Show again" for files you hid
  - "Use this file instead" for extra files (reopens the choice in Review)
  - "Put back in Inbox" for cleared Inbox files
- **Search finds hidden files too:** "N hidden files match · Show them".
- **Keyboard:** Ctrl+K palette; J/K to move, Space to preview, 1–0 to rate energy, T to tag; a bulk selection model.
- **Audio:** preview in the webview over a custom Tauri protocol, with AIFF and ALAC decoded to PCM/WAV in Rust. Waveforms are computed once in the job queue and cached as peak arrays.
- **Complexity:** L across all phases. **Reuse:** `TrackTable`, `ColumnEditor`, `useTrackRows`, `PlayerBar`.

#### 1.12 Review screen · 1b
- **What:** every decision the app can't make alone. Phase 1 covers duplicates, possible version mix-ups, quality problems, relink confirmations and "Better file available", with conflicts (1.10) in 1c. Later phases add tag suggestions (2.2) and unmapped genres (3.2).
- **Scope:** Library tracks and tracks being added. Duplicates among tracks you never add aren't queued, though All music still shows them.
- **Layout:** a queue on the left and a comparison on the right. The queue is grouped by how much thought each item needs: "Needs a closer look" first, then "Clear best file", then "Quality problems". It shows how many are left.
- **"Accept all N"** for clear cases. It can be undone, and what it accepted stays viewable.
- **Comparison:** one row per file with the same columns, the suggestion first, and a plain-language reason. "More details" shows bitrate, sample rate and date added. The Details panel shows each tag's value for the Library track and which file it comes from.
- **Keyboard:** Enter keeps the suggestion, number keys pick another file, S splits the group ("these aren't the same"). The next item loads automatically.
- **Listening:** switching between files keeps the playback position.
- **Single-file quality problems:** "Keep anyway", or "Find a better file" (→ wishlist, 2.3).
- **Complexity:** M.

#### 1.13 Overview · 1c
- **Header:** the Library's size, when it was last sent, and one main button: "Send N changes to rekordbox".
- **What to do next:** unsent changes, tracks made from low-quality files, missing tags or art, tracks rekordbox hasn't analyzed, and tracks in no crate or playlist. Each item disappears when fixed, and the empty list says "Nothing to fix". One progress line sits beside it ("This week: 4 tracks added, 12 decisions made").
- **Four numbers:** tracks and total length; how many are in crates or playlists; how many rekordbox has analyzed; when the Library was last sent.
- **Charts:** energy spread and genres as one-color bar charts.
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
- **What:** on-device models, downloaded once. The download is opt-in and goes through the network gate.
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
| Serato / Traktor / Engine export | rekordbox only for now. The export layer stays pluggable; this is the path to DJs on other software. |
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
- **Read-only is enforced in code:** only the write guard (`write_guard`, 0E-7) hands out write access, scoped to the app data folder (the Library folder joins in Phase 2). It resolves links and `..` before allowing a path, refuses writes through hard-linked files, opens SQLite only on guarded paths, and blocks `ATTACH`, `VACUUM INTO` and temp-file redirection at runtime. A source scan fails CI on any write API outside the guard, and a runtime test checks nothing else changes. The WebView2 profile Tauri keeps under `%LOCALAPPDATA%\com.tracklistpro.desktop\` is written by the runtime, not by app code.
- **The SQLite DB never lives in a synced folder;** it's in `%APPDATA%`. The repo itself lives off OneDrive too (`C:\dev\tracklist-pro`, CLAUDE.md "Parallel work"), so `.git`, `target/` and `node_modules` never sync.
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
| 1a | A Library started from rekordbox, and separately one started fresh, arrive in rekordbox 7 via XML with BPM, key, play counts and cues intact. Starting from rekordbox adds no duplicate rekordbox entries, and re-sending duplicates nothing. T1 and T3 are answered and the writer follows the results. Nothing outside the app data folder has changed on disk. **Dogfooding:** the owner has done at least two real read → send → import cycles around real gigs and logged the friction. **musicmanager is frozen.** |
| 1b | Duplicates of Library tracks get a correct best-file suggestion with a reason. False merges of different versions are under 1% on the test corpus. Relink confirmations and "Better file available" work through Review. |
| 1c | An edit made on both sides shows up as a conflict and resolves cleanly. The 100k synthetic fixture stays responsive. Crates carry over to rekordbox with their folders. **The owner has prepped and played a real or practice gig with it.** |
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

- **Phase 0:** spikes done (2026-09-27). Stages 0A–0E done 2026-09-28: repo, CI (trimmed, with a changed-paths gate), schema, ports, fixtures, the rekordbox behavior check (T1/T3 answered), job queue, network gate, write guard, suggestions and undo. Next: Stage 0F (the owner's Phase 0 check). See [TASKS.md](TASKS.md).
