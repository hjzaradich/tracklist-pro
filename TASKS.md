# tracklist-pro Tasks

## How this list works
- Work is split into **stages**. Every task in a stage must be done, reviewed, merged and green in CI before the next stage starts.
- Each stage is split into **lanes**. **One instance takes one lane** and does its tasks **in order**. Different lanes run **in parallel**, because they don't depend on each other or edit the same code.
- **All of a stage's lanes run at once**, about 5–6 at most; small tasks are bundled so each lane is a real chunk of work (CLAUDE.md, Parallel work). A stage with more lanes than that runs in waves.
- **Before each lane merges**, a reviewer instance reads its diff against CLAUDE.md, the task and its tests, and every CI check must be green.
- **✅ = done and merged** (marked on `main` at merge time).
- **👤 = you.** These tasks need a person: running rekordbox, playing gigs, legal, or making the repo public. Claude instances skip them.
- Each task is sized at a few hours at most, and **ships with its tests**. If one turns out bigger, split it within its lane before starting.
- Section references point into [ROADMAP.md](ROADMAP.md). The parallel-work rules (worktrees off OneDrive, branches, merging, shared files) are in [CLAUDE.md](CLAUDE.md#parallel-work).
- Phases 0 and 1 are broken down fully. Phases 2–4 stay at feature level until Phase 1 is done.

---

## Phases 0 and 1a ✅

Every stage is done; their task lists, decisions and leftovers are in [TASKS-archive.md](TASKS-archive.md).

---

## Phase 1b: Order out of the mess

Stages 1bA to 1bE ✅: in [TASKS-archive.md](TASKS-archive.md).

### Stage 1bF: Phase 1b check 👤
- **Lane 1:** ✅ 1bF-1 **Checkpoint:** 1b success criteria (§6). Passed 2026-10-24: best-file suggestions with reasons the owner agreed with; 0 of 31 false merges on the corpus (re-run on the final code) and none made on the owner's Library; "Better file available" and moved files worked by hand, while "rekordbox match uncertain", "File missing, copy found", "Same track twice in your Library", "Accept all" and Undo of a merge are proven by the tests only (no items in this Library; they come up in cycle 2, 1cD-2); the copy gate at 0 of 534 proposed; the preview played all four formats. Findings in `DOGFOOD.md` (Phase 1b check), five marked Before 1c.
- **Lane 2, before 1c:** ✅ (PRs 103-106) 1bF-2 Split off takes the selected file's identical copies with it, a refused Split off shows its reason, and the filled button follows the evidence (B-4, B-5) → 1bF-3 An "Unclear duplicates" item says what holds it (B-3) → 1bF-4 Review layout (B-6, the Designer) → 1bF-5 Vite update (6 security alerts, development only)
- **Owner decisions at the check (2026-10-24), for the fixes before 1c:** (A) on a possible version mix-up whose own reason says the names point to different versions, the filled button (Enter) is "Keep separate", not "Same track": the filled button follows the evidence; (B) "Split off" takes the selected file together with its files of the very same audio, so on an item with two copies of one version and two of another, one Split off keeps the copies together and the versions apart. Also before 1c: a refused Split off shows its reason (today it does nothing visible); an "Unclear duplicates" item says why it is held; the Review layout (the Designer).

---

## Phase 1c: Round trip and daily use (MVP)

Owner decision (2026-10-17): Phase 1c runs in the order below, every screen before the gig (1cD-2); not split into a gig-ready part first. Each stage still gets its own plan and budget.

### Stage 1cA: Engines and components ✅
In [TASKS-archive.md](TASKS-archive.md).

### Stage 1cB: Screens ✅
In [TASKS-archive.md](TASKS-archive.md).

### Stage 1cC: Integration and performance ✅
In [TASKS-archive.md](TASKS-archive.md).

### Stage 1cD: Fixes and pre-public
- **Lanes 1…n:** 1cD-1 Performance fixes from 1cC-6/7, **one lane per issue**. **Plan approved 2026-10-31:** ✅ S1 send (PR 157, release at 100k: prepare 28 to 21 s, go 19 to 12 s, longest writer hold 19 to 6.5 s, named progress steps; left: a rekordbox read still queues a background relink during a send, about 6 s more on the writer, unmeasured), ✅ R1a stored low quality and shown energy for the doors (PR 156: lowQuality deep 1041 to 73 ms), ✅ R1b stored genres (PR 159: Overview at 100k 2.5 to 0.5 s; the genre bars are doors; index and table rows from nothing 73.5 s on a quiet machine, M1 had 59 s, background and once per library), ✅ G1 deciding (PR 161, release at 100k: one new file after a restart 58 to 4.3 s; a first run 198 to 216 s; a one-time fill of about 60 s after the upgrade), ✅ C1 crate add (PR 158: the outside add 77 to about 35 s, 45 to about 31 s with verdicts stored, measured with another build running; left: it still shows no progress, because it is a plain command and progress would need it turned into a job, frontend too), ✅ W1 waveforms for Library tracks only (PR 160), ✅ N1 security (PR 155); ranks 6 to 9 wait until after the gig. Measured 2026-10-31, release build, 100,000 tracks (33,000 of them in the Library for the Overview doors), ranked by what a user feels:
  1. Send: prepare 25 to 27 s, go 17 to 20 s, each with the same 11 to 15 s review as one writer job; progress only at the end (audit B6)
  2. Overview doors: 12 of 18 take 0.2 to 0.9 s a window, and about 3 times that with a 100,000-track Library; store low quality and shown energy in `track_row`, as missing tags and art already are
  3. Deciding job: 198 s on a first run, and 58 s at any restart that finds one new file, because the name index isn't stored
  4. Adding tracks from outside the Library to a crate: 77 s, 44 s with verdicts stored, no progress, blocks writes (audit B1)
  5. Overview open: 2.5 s, and again every 5 s while jobs count; 1.5 s of it is genres, so a stored genre fixes this and lets the genre bars become doors (audit B5, part of B9)
  6. Cold first scan: 295 s reading against 21 s warm; try more read threads once
  7. Search index from nothing: 49 s (10 s on 10-13), 59 s with the table rows
  8. Undo of a big operation: up to 362 MiB and 8 s (audit B3; high risk, last)
  9. Startup checks 16 s every launch with nothing due; roles pass 13 s after every rekordbox read
  10. Waveforms: about 5 to 11 hours of one thread for 100,000 files (extrapolated). Owner decision 2026-10-31: Library tracks only; any other track is drawn when it is played
  Dropped with numbers: audit B2 (a 100k add to a crate is 6 to 7.5 s), B4 (about 2.3 s), B7, B12, all Review queries. Not measured: B8 and B10 (need a browser profile), B11.
  **Trying 0.1.5, owner's findings 2026-11-01:** ✅ PR 163: a new track starts from the beginning and a play press always plays, only comparing on Review keeps the position; Play next leaves the selection alone. ✅ PR 164: energy can be set from the Details row and an Energy column (no Clear and no sort by energy yet). Installer 0.1.6 (PR 165) has them; the owner finished the 0.1.5 try-steps with nothing else strange.
  **Installer 0.1.5 made 2026-11-01 (PR 162) for the gig.** All seven 1cD-1 jobs are merged; ranks 6 to 9 stay for after the gig.
  **Extras (owner approved 2026-11-03, stop at weekly 93%, started at 81%):** ~~send skips the extra relink a read queues~~ dropped (PR 167 closed: that relink also runs follow, attach and the search and waveform refresh, which the prepare's matching doesn't); ✅ one rule for which rekordbox entry a track shows (PR 171, audit C12: the old rules disagreed in 3 of 7 cases; screens now show the send's entry, including a byte-identical copy's; left: the send's drive-dependent fallback isn't in the shared rule); ✅ the Missing list paged (PR 172, C13: 100 rows a group, then "Show more"); ✅ startup checks and the roles pass (PR 169: startup checks about 16 to 8 s at 100k; a read marks for roles only tracks it changed; left: the quality stage itself takes 5.7 s at startup, unexplained); ✅ win back the index build time (PR 170: a 64 MB cache and fewer checkpoints while the index job runs; 105 to 53 s at 100k on the same machine state); ✅ Clear energy and sort by energy (PR 168; migration 0046); ✅ housekeeping (1cC archived); the short-tool-list agent trial waits for an app restart (agent types load at startup). All extras merged 2026-11-04 at weekly 84%. They go into an installer after the gig.
- **Lane 👤, gig:** 1cD-2 Prep and play a real or practice gig with it; log the friction in `DOGFOOD.md`. This is also the second dogfooding cycle, moved here from 1aG-3 (2026-10-10). In order: (1) back up rekordbox and the app's data folder, clear old test entries out of rekordbox, install over the older build; (2) start fresh with a set-sized Library (50 to 150 tracks), send, prepare the tracks in rekordbox (analyze, cues), send again and check nothing prepared was lost; (3) the rekordbox checks cycle 1 couldn't run: paths with `;` `@` `=`, a network drive, the USB eject test, the unsigned-app warning, importing two playlists whose names differ only by letter case; (4) play the gig; (5) then bring the whole collection into the Library, after duplicates and versions have been reviewed
  Idea to check at the gig (owner, 2026-11-01; not planned work): the send lists tracks to remove from rekordbox by hand, and that is the chore XML can't do. A send could also write a `Crates › To remove` playlist, so the tracks can be selected and removed in one go. In rekordbox, check that such a playlist leaves the old entries unchanged and whether removing from it takes tracks out of the collection, and note how long the real list gets.
- **Lane 👤, legal:** ✅ 1cD-3 Pre-public checklist (done in 1aP; the repo is public)

### Stage 2A: Inbox and online lookups (pulled ahead of the 1c checkpoint, owner 2026-11-05)
- **Plan approved 2026-11-05, budget 12 weekly points, started at 2%:** ✅ N1 MusicBrainz, Discogs and Cover Art clients behind the network gate, every service pinned to its hosts, no caller yet (PR 176); ✅ I1 Inbox folders and intake: migration 0047, batches, the first complete walk as baseline, a file arrives once it holds still (PR 177); ✅ I2a labels worked out when read, with reasons (PR 178); ✅ I2b actions: Add to Library linking the file where it is until 2.6, Add all, Not now, Hide, Clear and Put back, each one Undo (PR 179; a Better file waits for Upgrade, 2.6); ✅ I3 the Inbox screen, plain (PR 180: batches, label chips, per-label actions, Already have collapsed with Clear all, Inbox folders, previews from a third in; about 55 proposed strings wait for the owner's wording pass); an installer.

### Stage 2B: Wishlists (owner 2026-11-05)
- **Plan approved 2026-11-05, budget 8 weekly points, started at 5%:** ✅ W1 wishlists and matching: migration 0048, paste and CSV import by header, owned / other cut / rework / missing with reasons, off by default (PR 182); ✅ W2 the Wishlists screen with copy-only store links (Beatport, Bandcamp, Traxsource, Juno search URLs to copy; the app opens nothing), bought and notes, an Optional features panel in Settings (PR 183; the link templates and all strings are proposed); ✅ W3 auto tick-off: an Inbox file that matches a wish exactly, with nothing ambiguous and the add rules allowing it, is added to the Library in one undoable operation and listed under Done for you; Undo returns it to the Inbox; optional crate named after the wishlist, reusing one with that name (PR 185; checked again after each Group job, since a file has no track before grouping); one 100k measurement of the new features.

### Run 2C + 3A + 3B (owner 2026-11-05: one approval, budget 20 weekly points, started at 6%)
- **2C** reading rekordbox's library directly (2.7): ✅ R1 the reader with its Settings row (PR 187; R2 folded in; owner hands-on test pending; follow-up: let the send use a database read). **3A** set prep basics: ✅ G genre tree with aliases (3.2; PR 184), ✅ T tag vocabulary from My Tags (3.1, no My Tags writing until T5; PR 186; tag filter has no chip yet), ✅ S smart crates (3.3; PR 189; follow-up: tag rules). **3B** ✅ H1 play history from rekordbox History m3u8 (3.6; PR 188; H2 gig-stick history later), ✅ N "what's next" (3.4; PR 190; follow-up: genre and tag signals). Barebones wording, all proposed.

### Run 3C + 4A ✅ (owner 2026-11-06: one approval, budget 30 weekly points, started at 16%; done 2026-11-07 at 29%: PRs 191-205)
- **Loose ends:** ✅ F tag rules in smart crates + a filter chip for tags (PR 191); ✅ N2 genre and tag signals in "what's next" (PR 192); ✅ D the send refuses when the newest read is the database's (PR 193; sending from the database waits for the owner's hands-on test: missing Grouping, Date Modified, Last Played, some kinds; follow-up: "Read again" or an offered export can still replace a newer database read with an older XML, at the user's request). **3.5** playlists with an energy arc: P1 ordered playlists (✅ P1a backend, PR 196; ✅ P1b the screen, PR 198), P2 timeline (energy, key and BPM flow, clashes) and P3 "fill the gap": ✅ backend (PR 199; also: estimated energy never counts anywhere), ✅ P2b the screens (PR 204). **3.7** local estimates: E0 library research, then the owner picks the audio-analysis library before any build (owner 2026-11-06); E1 BPM and energy (✅ E1a analysis job and BPM, PR 200; ✅ E1b energy suggestion backend, PR 202; ✅ E1c the suggestion beside the energy control and estimate marking, PR 205), ✅ E2 key behind an off switch with the owner's comparison (PR 201). **3.6** ✅ H2 play history from gig sticks (export.pdb, read-only; PR 197; follow-up: the stick's `Contents` folder itself isn't checked for being a link). **From Run 2C+3A+3B's 100k measure:** ✅ wishlist matching fast at 100k (PR 194: about 62 s to about 2 s, a quiet re-measure pending). ✅ Undo test fix for tick-offs (PR 195, test only). ✅ 4.3 diagnostics (PR 203: event log and a private report). Migrations reserved: P1 0052, E1 0053, H2 0054 (if needed). Barebones wording, all proposed.

### Run 4B + 3.8 research ✅ (owner 2026-11-07: one approval, budget 25 weekly points, started at 29%; PRs 206-212)
- **3.8** ✅ R0 audio-model research (owner picked EfficientAT mn10_as on tract; ROADMAP 3.8), no code: the owner picks the model before any build (same rule as the audio library).
- **4.1** portability bundle: ✅ B1a export and import (backend, PR 209), ✅ B1b "Find it…" for a moved music folder (PR 211), ✅ B2 the Save dialog and the screen (PR 212). Follow-up: a double failure during the swap's put-back can leave a stray -wal file; an undeletable marker is untested; "Find it…" for a folder gone from a connected drive.
- **4.2** ✅ O onboarding (PR 207), barebones: the online-services step, the Inbox folder asked for, the OneDrive check, rekordbox tips. The model download waits for 3.8.
- **4.4** ✅ W docs (PR 206): a user guide, issue templates, the non-affiliation and privacy notes, the GPL notice, the unsigned-install guide, and a third-party licence page generated from the dependencies.
- **Follow-ups:**
  - ✅ FB, backend (PR 208): any_stale moves to a read connection; the stick's Contents folder gets a link check; "Read again" or an offered export says so before an older XML replaces a newer database read; diagnostics gets a disk-space line.
  - ✅ FU, screens (PR 210): playlist sort by fit, start a playlist from a smart crate, autoscroll while dragging, the timeline's estimated key marked.
- All wording is proposed, for the owner's deep dive.

### Run 4C ✅ (owner 2026-11-07: "start the next run", budget 15 weekly points, started at 35%, ended at about 40%; PRs 214-216)
- **4.5** ✅ U (PR 216) update check (check and tell, through the gate) with the first-run question, the Settings switch and the installer's publisher name; ✅ R the release script (draft releases only; PR 214).
- **3.8** ✅ M1 (reviews/4C-M1-measure.md; owner then picked CED-tiny, shipped in the installer) first measurement, no app code: EfficientAT mn10_as and CED on `tract` (and `ort` for comparison), public test audio, a PyTorch reference for parity; speed and memory, label agreement on DJ genres, "sounds like" neighbours, a per-user head. The owner's own tracks only with a later OK.
- **Follow-ups:** ✅ F (PR 215) the bundle swap's double failure leaving a stray -wal; the Save dialog's gig-stick check refusing an ordinary folder that happens to hold a "Pioneer" folder; COM0 and superscript device names. Left: a stale set-aside folder can skip the fold before a swap, and a second failure overwrites the earlier `.stray` copy (review notes).
- Installer 0.1.9 built (PR 213).

### Run 5A ✅ (owner 2026-11-07: "go for it", budget about 20 weekly points, started at 39%, ended at about 43%; PRs 217-221)
- **3.8 part 1, the audio model (CED-tiny):**
  - ✅ A1 (PR 217) the model file: an export script that rebuilds it from the original (pinned versions), the file in the repo under `src-tauri/resources/models/ced-tiny/` with its Apache-2.0 licence and a notice (source, revision, sha256, our change: an embedding output), golden reference outputs from synthetic audio for the parity tests, the label names, bundled in the installer, the credit in About and on the third-party licence page. Foreman's call: the file lives in git (22 MB, plain, not LFS), because a release asset would be a public release and LFS bandwidth is limited.
  - ✅ A2a (PR 219) the listening step: the app's own 16 kHz / 64-mel front end, the model loaded with `tract` after a sha256 check, three 10-second windows per track, a fingerprint and label scores; parity tests against the golden outputs. No database, no job.
  - ✅ A2b (PR 220; Fable review + fix-check) the fingerprint job: migration 0055 (the `embedding` table and label scores), a background job that runs only when the audio model is switched on (off by default), progress and cancel, what a bundle carries.
  - ✅ A3 (PR 221; owner 2026-11-08: its own panel headed "Audio model") "Sounds like" and genre suggestions, labelled as the audio model's opinion, and the Settings switch.
- **Follow-ups:** ✅ F (PR 218; Fable review + fix-check: leftover pieces of an earlier failed swap are set aside as `-stray`, never folded over a Library that moved on) the bundle's stale set-aside folder skipping the fold before a swap; a second failure overwriting the earlier `.stray` copy; the update check: a test that the question closes before the check finishes, Semver equality agreeing with its order.

### Run 5B ✅ (owner 2026-11-08: "go for 5B first then build the installer afterwards", budget about 15 weekly points, started at 43%; PRs 222-225)
- **3.8 part 2, using the audio model** (owner decisions 2026-11-08 in ROADMAP 3.8):
  - ✅ B1 (PR 224; Fable review 7/7 mutations + fix-check) "Use" on a generic genre guess in the Audio model panel: sets the track's genre as an ordinary genre edit (one Undo; sent like any edit; an unknown name goes to Review's Unmapped genres).
  - ✅ B2 (PR 222; Sonnet review, 5/5 mutations) "sounds similar" as a What's next signal (default weight 1 while the audio model is on; never the only reason; listed as not used while the model is off or the track has no fingerprint).
  - ✅ B3a (PR 223; Sonnet review, 5/6 mutations, gaps closed) your own genre model, backend: a small model learned from the fingerprints of tracks you've given a genre (genres only; at least 10 tracks per genre), re-learned automatically after fingerprinting and when genres change; migration 0056.
  - ✅ B3b (PR 225; light review) "Your genres" in the Audio model panel, with "Use" (after B1 and B3a).
- **Follow-ups:** the panel's "Genre set to …" line can stay when switching tracks (the panel isn't keyed per track); the What's next reason percent can differ by 1 from the panel's (f32 vs f64); "Your genres" empty line can't say why (needs a reason in the reply); generic AudioSet names ("House music") need an alias to map to a tree node.
- Then installer 0.1.10 (owner asked: after 5B).

### Run 6A ✅ Identification and tag cleanup (2.2) (owner 2026-11-08: "Go for it", budget about 20 weekly points, started at 46%, ended at 54%; PRs 227-234)
- **2.2** (owner decisions 2026-11-08 in ROADMAP 2.2). Order: map, then K1 and S1 side by side, then S2, then U1. Migrations: S1 0057, S2 0058 if needed.
  - ✅ M code map (reviews/6A-map.md).
  - ✅ K1 (PR 227; Fable review 6/6 mutations) keys and switches, backend: Windows password store for the Discogs token (save, check, remove; never in the database, bundles, logs or diagnostics); the AcoustID service pinned to its host, its key a constant the owner fills in; switch commands for MusicBrainz, AcoustID, Discogs and Cover Art.
  - ✅ K2 (PR 229; light review) the Settings rows: each off, each saying what it sends; the Discogs token field.
  - ✅ S1 (PR 228; Fable review 4/7, gaps closed with tests + fix-check) suggestions, offline, and Use (one Undo; Year 4 digits; Name keeps a bracketed version, Mix offered on its own; case-only fixes offered; a merge carries a catalogue number to a survivor without one): per-field suggestions (title, artist, version, remixer, label, year, album, catalogue number, artwork) from tags, the filename and version parser and sibling files, each naming its source; the catalogue number kept by the app; migration 0057.
  - ✅ L1 (PR 231; Fable review 5/5) lists, search and sort show the app's own Name, Artist, Mix, Remixer, Label, Year and Album, as they do Genre (found in S1's review).
  - ✅ S2a (PR 230; Fable review 6/6; its five follow-ups moved into S2b: MBIDs checked as UUIDs, years 1000-9999, a stale cache after the file's audio changed, no AcoustID request while MusicBrainz is off, a typed offline reason). The online look-up job, split in two before it started: S2a AcoustID then MusicBrainz, cached per file (migration 0058), status per track; S2b the name-search fallback (MusicBrainz, Discogs), automatic look-ups of new Inbox files, the front cover. Whole job: AcoustID from the stored fingerprint, then MusicBrainz; MusicBrainz and Discogs text search; the front cover from Cover Art. Through the gate, at each service's speed limit, cancellable; one bad answer never stops a batch. On request, and automatically for new Inbox files while switched on. Fable review.
  - ✅ S2b (PR 233; Fable review 6/6, fix round + fix-check; migration 0059) the rest of the online job, plus S2a's follow-ups and a look-up by recording ids: name search, Discogs, automatic Inbox look-ups, covers in memory only.
  - ✅ U2 (PR 234; light review) Inbox rows show what a file was identified as; the cover in the panel; "Data provided by Discogs" where Discogs data shows; "Look up N tracks" switches to the recording-id request (S2b), so a large selection isn't limited to loaded rows.
  - ✅ U1 (PR 232; light review) the Look up panel in track details, with "Look up N tracks" for a selection (on Inbox rows: U2): in the track details and on Inbox rows, current value beside each suggestion with its source; "Use" per field and "Use all", one Undo each, sent to rekordbox like any edit; artwork and catalogue number shown only. Fable review.
- **Owner step:** ✅ the owner registered the AcoustID application key and put it in the code (PR 236); installer 0.1.11 carries it (PR 235 version).
- **Follow-ups:** re-check the Discogs switch right before the token is stored (tiny window after the check); the test-only mock credential store is chosen by `cfg!(test)`, which doesn't cover tests/ integration tests (none use it yet); tags expose no catalogue number (only file names like "[ABC123]"). From L1: no test proves a resolved conflict, a merge or the pre-send take-in on Name/Artist/Album/Remixer/Label marks the track due (only Genre's tests do); own Comments and Composer aren't indexed or shown as yours. From S2b: artist credits also split on "/", ";" and "+" ("AC/DC" could match a lone "DC"); OneDrive online-only files aren't looked up automatically (owner 2026-11-09: keep it that way, since reading one makes OneDrive download it).

### Run 6B Speed and loose ends (owner 2026-11-09: "go", overnight, budget about 10-12 weekly points, started at 54%)
- **Speed (1cD perf list, items 6, 7 and 9):** P1 the cold first scan (try more read threads); P2 the search index built from nothing; P3 the startup checks with nothing due, and the roles pass after every rekordbox read. Measured on synthetic data only, before and after; results must not change. Item 8 (Undo of a big operation, high risk) waits for a run while the owner is around.
- **F1** 6A backend loose ends: the Discogs switch re-checked as the token is stored; the test mock store for integration tests; catalogue number from file tags; credit split no longer on "/", ";", "+"; tests that a resolved conflict, a merge and the pre-send take-in mark the track due for Name/Artist/Album/Remixer/Label.
- **F2** own Comments and Composer indexed and shown as yours (like L1).
- **F3** 5B loose ends: the "Genre set to …" line keyed per track; one rounding for the What's next percent; "Your genres" empty line says why; generic audio-model names ("House music") map to a genre tree node.
- **F4** "Find it…" also for a folder gone from a drive that is connected (4.1).
- ✅ **T1** (PR 237; diff review) the theme control moves from the top bar into Settings (owner 2026-11-09).
- All new wording is proposed, for the owner's deep dive.

### Stage 1cE: Go public 👤
- **Owner decision 2026-11-05: the owner's review work moves later.** P1 (wording and formatting), installer 0.1.8, the 1c checkpoint (1cE-1) and the owner's remaining 1cD-2 checks wait until the owner takes a deep dive; Phase 2 goes ahead meanwhile with plain, barebones wording and interface (strings stay proposed; the Editor's rule stands: warnings, data loss and rekordbox steps go to the owner first).
- **Polish before the checkpoint. Plan approved 2026-11-05, budget 12 weekly points, started at 2%:** P1 wording and formatting from the owner's gig-computer use (the Designer and the Editor write it up; one or two builders); ✅ P2 a progress display when adding many tracks to a crate (PR 175: checking, then adding, with a percent after about a second; one add is still one Undo; left: two adds queued at once briefly show the first's numbers); ✅ P3 the quality startup check's 3.4 to 6.1 s, no code change: measured warm, the new order is 10 to 14% faster than the old, so PR 169's swap stays; the same check takes 2.5 s on the writer and 5 to 8 s on a read connection, cause unknown, not worth chasing at a real Library's size; installer 0.1.8 and a checkpoint walkthrough. Not planned: the send's fallback to the entry at a gone file's own Location stays out of the shown rule on purpose (PR 171 review: a value that flips when a drive is plugged in is worse than the known gap); M1 ranks 6-8 wait for Phase 2. Installer 0.1.7 (all 1cD extras) built 2026-11-05 (PR 173). The 1c copy pass merged (PR 174): all 409 proposed strings approved by the owner.
- **Owner, for the checkpoint:** the rest of 1cD-2: the rekordbox checks (paths with `;` `@` `=`, a network drive, the USB eject, two playlists differing only by letter case) and bringing the whole collection into the Library. The gig-computer use so far found only wording and formatting.
- **Lane 1:** 1cE-1 **Checkpoint:** 1c success criteria (§6). (1cE-2, making the repo public, moved to 1aP on 2026-09-30)

---

## Phases 2–4 (feature level: break down into stages and lanes when Phase 1 is done)
- **Phase 2:** Gate tests T2 + T4 👤 → Managed copies & upgrades (2.6; the Library folder joins the write guard as a second root, and the guard refuses writes through hard-linked files, so decide how Library copies treat a user's hard links before tag writing, with the tag-writing, transcode, rules and naming ports) · Online clients port (MusicBrainz, Discogs, Cover Art; pin each service to its hosts in `net`, strip auth headers on cross-host redirects, and check the User-Agent contact URL is public) · Inbox (2.1) · Identification & tag cleanup (2.2) · Wishlists (2.3) · Store links (2.4) · Auto tick-off (2.5) · Reading rekordbox's library directly (2.7; optional, off by default; read-only; one removable module; the user supplies the key)
- **Phase 3:** Tag vocabulary (3.1, needs T5) · Genre taxonomy (3.2) · Smart crates (3.3) · "What's next" (3.4) · Playlists with energy arc (3.5) · Play history (3.6, needs T6) · Local estimates (3.7) · Local ML (3.8, last)
- **Phase 4:** Set `bundle.publisher` for the installer (it shows "tracklistpro" today; 0F-1) · Updater + release script port · Portability bundle (4.1) · Onboarding (4.2) · Diagnostics (4.3) · Docs & community (4.4)
