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

## Phase 0: Foundations

### Stage 0A: Bootstrap ✅
- **Lane 1, scaffold:** ✅ 0A-1 Set up the repo at `C:\dev\tracklist-pro` (off OneDrive; private files stay on OneDrive behind junctions, see [CLAUDE.md](CLAUDE.md#parallel-work)), scrub personal paths from `spikes/rb-kit/SESSION.md` and `spikes/bpmkey_grade.py`, `git init` and check `git status` for anything personal before the first commit (pre-public checklist) → ✅ 0A-2 Tauri 2 + Rust + React/TS scaffold that opens an empty window → ✅ 0A-3 Set the product name, bundle ID `com.tracklistpro.desktop` and the `%APPDATA%` data folder (§1.1)
- **Lane 2 👤:** ✅ 0A-4 Create `C:\dev` and the private GitHub repo; set the git author to the GitHub noreply address

### Stage 0B: Tooling and base modules ✅
Three lanes, because on a fresh scaffold most of these tasks touch the same few files.
- **Lane 1, CI:** ✅ 0B-1 CI: Windows workflow (build + Rust and frontend tests) on every PR → ✅ 0B-2 CI: macOS and Linux build + platform-neutral tests, on pushes to `main` only (saves Actions minutes) → ✅ 0B-3 `cargo-deny` config (GPL-compatible licenses) + CI step
- **Lane 2, backend base:** ✅ 0B-4 `tauri-specta`: bindings for one sample command end to end → ✅ 0B-5 SQLite: open the DB in the data folder with a single writer connection → ✅ 0B-10 Volume detection: UUID/serial, label, mount path
- **Lane 3, frontend base:** ✅ 0B-6 Router, TanStack Query, Zustand providers → ✅ 0B-7 Vitest + ESLint baseline → ✅ 0B-8 `i18next` setup, **one namespace file per feature** → ✅ 0B-9 Theme tokens (dark + light) + theme switch

### Stage 0C: Core plumbing ✅
Five lanes, all at once. Merge order: lane 3 before lane 4 (the shell builds on the i18n typing).
- **Lane 1, CI:** ✅ 0C-1 CI check that the generated bindings are current (`cargo run --example export_bindings`, then a clean tree) → ✅ 0C-11 `cargo fmt --check` + `clippy -D warnings` in CI → ✅ 0C-12 Dependabot for GitHub Actions versions → ✅ 0C-13 PR check that existing migrations and their `checksums.txt` lines are never edited, renamed or removed (only additions)
- **Lane 2, DB:** ✅ 0C-2 WAL mode + read-connection pool → ✅ 0C-3 Migration runner with the first (empty) migration → ✅ 0C-4 Append-only migration test
- **Lane 3, frontend rules:** ✅ 0C-5 ESLint rule forbidding raw JSX strings, plus compile-time checking of i18n keys (without a shared registry file that every feature lane edits) → ✅ 0C-6 Theme test: a sample screen rendered in both themes; move the warning color away from the amber accent so the two can't be confused
- **Lane 4, shell:** ✅ 0C-7 Layout shell: sidebar, table, Details panel, player zones → ✅ 0C-8 Stage routes (Overview, Review, All music, Library, Crates) as empty screens
- **Lane 5, platform:** ✅ 0C-9 Path model: volume + relative path, NFC-normalized, resolved back to absolute → ✅ 0C-10 Content Security Policy: replace the scaffold's `csp: null` with a strict policy (self only, plus what Tauri IPC and the dev server need), with a test that fails if it's null or allows remote origins

### Stage 0D: Phase 1 schema, read-side ports, fixtures and the rekordbox check kit ✅
Four lanes, all at once. The check kit is pulled forward from 0E so the owner can run the T1/T3 gate tests (1aA lane 1) while 0E is built.
- **Lane 1, schema** ✅ (Phase 1 tables only, §2; one migration per task, numbered 0002 onward in this order): 0D-1 `volume`, `music_folder`, `file` (store `rel_path` on-disk plus an NFC match key, §0.3; add a `VolumeId` constructor for ids loaded from the DB, e.g. `VolumeId::from_stored`; volume identity is serial-first, so detect two mounted volumes sharing a serial, e.g. a cloned backup drive, and tell them apart by label/GUID) → 0D-2 `recording`, `recording_file`, `version_link`, `analysis` → 0D-3 `library_track`, `rekordbox_track`, `sync_base`, `conflict` → 0D-4 `crate`, `crate_entry`, `operation`, `change`, `job`, `setting`, `service_optin` → 0D-8 Wrap the migration authorizer in a drop guard so no early return can leave it on the writer connection (`db/migrations/mod.rs` `apply()`)
- **Lane 2, tags:** ✅ 0D-5 Port tag **reading** (`tags/mod.rs`), lenient (§5.5) → ✅ 0D-6 Port Camelot key normalization (`tags/key.rs`)
- **Lane 3, fixtures:** ✅ 0D-7 Synthetic fixture generator: folder tree + small valid audio files, scalable to 100k → ✅ 0D-9 Small hardening: Tauri `security.freezePrototype`; `build.assetsInlineLimit: 0` in vite.config.ts (strict CSP blocks inlined `data:` assets)
- **Lane 4, rekordbox check kit** (from 0E): ✅ 0E-11 Extend `spikes/rb-kit` with the T1 and T3 cases (§7) → ✅ 0E-12 Turn it into a re-runnable **rekordbox behavior check** with a short step list, for gate tests now and after every rekordbox update (§5.2)

### Stage 0E: App services ✅
- **Lane 1, jobs:** ✅ 0E-1 Job model, persistence and enqueue API → ✅ 0E-2 Worker pool with priorities → ✅ 0E-3 Progress events + top-bar Activity status → ✅ 0E-4 Cancellation
- **Lane 2, network:** ✅ 0E-5 `net` module checking `service_optin` → ✅ 0E-6 Test: no HTTP client outside `net`
- **Lane 3, write guard:** ✅ 0E-7 Writer handle API scoped to the app data folder (the Library folder joins in Phase 2) → ✅ 0E-8 Test: no writes anywhere else (exempt `cfg(test)` code: test helpers such as `tags/test_audio.rs` write fixtures into temp dirs)
- **Lane 4, suggestions and undo:** ✅ 0E-9 Suggestion type with a required reason string + test → ✅ 0E-10 Port the operation log (`ops/mod.rs`), single-step undo
- **Lane 5, follow-ups:** ✅ 0E-13 Trim CI minutes (target: at least 40% fewer weighted minutes, every check still meaningful) → ✅ 0E-14 Key notation options (§1.1: Camelot, Musical standard / rekordbox / sharps / flats), using rekordbox's spellings from the behavior check (the 0D schema differs from musicmanager: `operation` stores kind + a details JSON, no UI text; `change` stores entity + entity_id)

### Stage 0F: Phase 0 check 👤 ✅
Runs alongside Stage 1aA (owner OK 2026-09-28): nothing in 1aA depends on it.
- **Lane 1, 👤 + guide session:** ✅ 0F-1 Side-by-side install check with musicmanager: build the Windows installer (`tauri build`), install it next to musicmanager, confirm separate data folders (`%APPDATA%\com.tracklistpro.desktop` vs `com.musicmanager.desktop`) and that both run (passed 2026-09-29 against musicmanager's dev build; steps in docs/install-check.md) → ✅ 0F-2 **Checkpoint:** Phase 0 success criteria (§6), checked by the foreman (passed 2026-09-29: Windows checks green on every merge; macOS/Linux last green at a4b7455, re-run when Actions minutes are back; architecture tests exist for the network gate, migrations, write guard and reason strings)

---

## Phase 1a: Prove the loop

### Stage 1aA: Gate tests, plus work that doesn't depend on them ✅
Four Claude lanes, all at once, alongside Stage 0F. Lane 4 opens 1aA-12 (shared IPC error type) as its own early PR, because lane 1's new commands use it; lane 1 rebases onto it once merged. Lane 1 owns the frontend `src/activity/` files, so the Activity-sync half of 1aA-13 moved there.
- **Gate tests 👤** ✅ (done 2026-09-28 on rekordbox 7.2.19: T1 no, T3 yes; see ROADMAP 1.9 and §7) (demo rekordbox, using the check kit from 0E-12): 1aA-1 T1: playlist referencing tracks absent from COLLECTION → 1aA-2 T3: re-import with `Tonality`/`AverageBpm` omitted
- **Lane 1, walker:** ✅ 1aA-3 Music folder commands: add, remove, list, with a role → ✅ 1aA-4 Stage-1 walker: stat-only walk into `file` rows, streamed to the UI (store the music folder once, then build each file as `folder.join(rel)` from the listing; don't call `from_absolute` per file, it canonicalizes and queries the volume) → ✅ 1aA-5 Skip rules: `._*`, `.DS_Store` → ✅ 1aA-11 Activity popover: list running and queued jobs, with Cancel (the backend and `cancel_job` exist since 0E-4; its copy goes to the owner); also, from 1aA-13, Activity sync retries a failed snapshot, and its pending/seen maps stay bounded
- **Lane 2, rekordbox XML:** ✅ 1aA-7 rekordbox `Location` decoder by hand, with tests (§5.3) → ✅ 1aA-8 COLLECTION tracks, with all attributes, cues and grid → ✅ 1aA-9 The PLAYLISTS tree → ✅ 1aA-10 Skip rules: deleted rows, demo tracks, CUE Analysis Playlist, streaming entries
- **Lane 3, sniffer:** ✅ 1aA-6 Format sniffer from bytes, with a fake-`.wav` test → ✅ 1aA-14 Write-guard scan: a regression test that a `windows_sys` module alias (`use …::FileSystem as fs; fs::DeleteFileW`) stays caught
- **Lane 5, CI runner** (added 2026-09-29: Actions minutes ran out): ✅ 1aA-16 Let CI run on the owner's self-hosted Windows runner (label `tlp-windows`), switched by a repository variable so it can flip back to GitHub's runners; every job keeps checking the same things
- **Lane 4, platform follow-ups:** ✅ 1aA-12 A shared IPC error type (a kind plus an i18n key and parameters, never raw DbError text), used by every command that can fail; switch the 0E commands (`key_notation`, jobs, ops) to it. Error wording is copy for the owner → ✅ 1aA-15 The undo outcome carries operation and row ids again (bigints now cross IPC as numbers; they were skipped in 0E-10) → ✅ 1aA-13 Job queue polish (from the 0E-1 review): Reentrant guard in shutdown/drop; a time limit on joining workers at exit; monotonic progress via an atomic swap (the frontend Activity-sync items moved to lane 1)

### Stage 1aB: Reading files and rekordbox ✅
Six lanes, all at once (started 2026-09-29, while 1aA-16 finished). Each scan-stage job stands alone with a command to start it; chaining walk → stage 2 → stage 3 is 1aC-8. A lane that needs a schema change asks the foreman first (migrations are numbered in merge order).
- **Gate write-up 👤** ✅ 1aB-1 Write up T1 and T3; update ROADMAP 1.9 and §7 with the results (done 2026-09-28)
- **Lane 1, reading files:** ✅ 1aB-2 Lenient tag reading job → ✅ 1aB-3 Audio properties (codec, bitrate, sample rate, duration) → ✅ 1aB-4 Broken-file detection (the 1aA-6 sniffer already flags an M4A with no `moov` as `NoMoov`, and zero-byte/truncated files; reuse or replace)
- **Lane 2, hashes:** ✅ 1aB-5 Stage-3 job: `blake3` → ✅ 1aB-6 Stage-3 job: `audio_hash` (audio frames only; definition 1 is documented in hash/audio.rs)
- **Lane 3, fingerprint:** ✅ 1aB-7 Stage-3 job: `rusty-chromaprint` fingerprint, visible tracks first
- **Lane 4, scan follow-ups:** ✅ 1aB-8 OneDrive placeholder detection and skipping, with opt-in → ✅ 1aB-14 Scan errors: count the folders and files the walk couldn't read (1aA-4 skips them silently today) and show the count (copy for the owner) → ✅ 1aB-9 Offline volumes greyed out (their files stay present); files missing from a folder the walk could list set to `present = false`; rescan volumes on device change (WM_DEVICECHANGE), since `SystemVolumes` is a snapshot and a swapped drive on the same letter would otherwise resolve stale paths; also wire 0D-1's clone rule (`volume::tell_clones_apart`, two mounted volumes sharing a serial) into `SystemVolumes::scan`, using the remembered `volume` rows; also let `mount_path()` accept a lone volume whose serial and GUID match a clone's `serial=…+guid=` id
- **Lane 5, rekordbox in:** ✅ 1aB-10 Write the `rekordbox_track` snapshot from the parsed XML (if `RekordboxXml::is_complete()` is false, don't mark missing tracks as removed) → ✅ 1aB-11 XML source: file picker + watch on the Documents export location (the watch never opens an online-only file; it polls only while Overview is open)
- **Lane 6, check kit:** ✅ 1aB-13 Check-kit polish: make CHECK.md step 3.4 (Import Playlist before export 03) stand out, set the time estimate to about an hour, and add 2B/6B keys to the kit so the rekordbox Classic spellings for F# and Bb major get verified (the owner runs it later; then record the result in ROADMAP §1.1/§5.2 and update the "unverified" note on REKORDBOX in src-tauri/src/tags/key.rs)
- **Dropped for now (owner, 2026-09-29):** 1aB-12 `export.pdb` reader for track paths; revisit at 1aD-3 (gig stick recovery)

### Stage 1aC: Tracks and first relink steps ✅
- **Lane 1:** ✅ 1aC-1 Unchanged check (size, mtime, file-id) + partial hash on mtime-only changes (the 1aA-4 walker overwrites size/mtime/file_id in place, so compare inside the walker before its upsert)
- **Lane 2:** ✅ 1aC-2 Provisional grouping: one file → one track, except exact `audio_hash` matches (replaced in 1b)
- **Lane 3, relink:** ✅ 1aC-3 Step 1: path still valid → ✅ 1aC-4 Step 2: filename + duration (±0.5 s) → ✅ 1aC-5 Step 3: unique duration within the candidate set
- **Lane 4:** ✅ 1aC-8 Chain the scan stages: after a walk, queue stage 2 (tags, properties), then stage 3 (hashes, fingerprint) for new or changed files; also decide whether a drive coming back queues a scan of its folders (1aB-9 only refreshes the volume rows); don't loop on `scan_state::count_due`, which never reaches 0 while online-only files are skipped → ✅ 1aC-6 Per-root watcher toggle (`notify`) → incremental re-scan
- **Lane 5:** ✅ 1aC-7 Performance check: stages 1–2 on the 100k fixture meet the 1.1 target
- **Lane 6:** ✅ 1aC-9 Make timing-sensitive tests robust on a busy PC (lanes and the self-hosted runner share one laptop): `net::tests::musicbrainz_requests_reach_the_server_at_least_a_second_apart` and `a_request_held_up_after_its_turn…` fail now and then under load; `src/i18n/noRawJsxStrings.test.ts` ("flags plain text children") times out on ESLint's cold start. Keep what each test proves; remove the dependence on wall-clock speed
- **Lane 5, third PR:** ✅ 1aC-12 Read files in parallel (the shared hash/fingerprint thread budget, 4 or fewer at below-normal priority) and re-measure: 1aC-7 found the cold first index of 100k files takes 13.8 min, almost all first-open disk reads and Defender scanning done one file at a time (parsing is 0.15–0.35 ms per file)
- **Lane 5, second PR:** ✅ 1aC-11 Frontend tests robust under load: no elapsed-time assertions (count work instead), one global vitest timeout of 20 s, fake timers where real ones are waited on. CI on the shared laptop failed 7 unrelated tests in XmlSourcePanel, activityStore, AppShell and one more file (run 36663347701)
- **Follow-up (after lanes 1 and 4 merge):** ✅ 1aC-10 The fingerprint stage skips re-decoding a file whose `audio_hash` is unchanged since it was fingerprinted. rekordbox writes tags, which changes size, mtime and the partial hash but not the audio (§5.1), so 1aC-1 alone would still re-fingerprint those files

### Stage 1aP: Go public ✅ (published 2026-09-30)
Added 2026-09-30 and run right after 1aC: the owner won't pay for CI, and public repos get GitHub's runners free. Decided by the owner the same day:
- A **fresh public repo** with one clean starting commit. This repo is renamed `tracklist-pro-archive` and stays private, with the full history, PRs, reviews and CI logs; GitHub keeps old PR pages and their code even after a history rewrite.
- The legal notes and private research notes stay private. The public repo gets a short NOTICE instead.

- **Lane 1, cleanup** (docs only; runs alongside 1aC):
  - ✅ 1aP-1 Scrub every tracked file for publication:
    - library details (folder names, track counts) and personal paths;
    - private notes and research that aren't meant for publication;
    - links to untracked private files.

    Move the legal notes to the private side and add `NOTICE.md` (license, non-affiliation, privacy).
  - ✅ 1aP-2 README:
    - what it is and its status;
    - GPL-3.0;
    - a non-affiliation disclaimer;
    - privacy: no telemetry, and what each opt-in service receives;
    - "Issues and ideas welcome; I'm not accepting code contributions right now".

    The owner approves the wording.
  - ✅ 1aP-3 A publication manifest: every file that goes public and every file that stays private, with the reason.
- **Research (foreman):** ✅ 1aP-4 Read the current rekordbox EULA and summarize its terms for the owner (done 2026-09-30).
- **Owner 👤:** ✅ 1aP-5 A trademark search on the name (done 2026-09-30; the name stays) → ✅ 1aP-6 Approve the README and NOTICE, then give the final go/no-go.
- **Publish (foreman, once 1aC is merged and no PR is open):**
  - ✅ 1aP-7 Set up the repos:
    - rename this repo to `tracklist-pro-archive` (private);
    - create the public `tracklist-pro` from one clean commit of the scrubbed main (owner's name as author, noreply email);
    - private docs move to the git-ignored `private/` folder, like `spikes/results`.
  - Configure the public repo:
    - CI on GitHub-hosted runners (delete `CI_RUNNER`);
    - unregister the self-hosted runner, and never attach it to a public repo;
    - branch protection requiring **CI result**;
    - secret scanning with push protection, and Dependabot alerts;
    - Actions set to require approval for outside contributors' workflows.
  - Repoint the local checkout, the lane worktrees and the fallback gate at the new remote.
  - ✅ 1aP-8 Update CLAUDE.md and playbook references, and confirm that the first CI run on the public repo is green.

### Stage 1aD: Finish relink, build Library tracks
- **Lane 1, relink:** 1aD-1 Step 4: fingerprint (also queue relink after the fingerprint job; 1aC-8 queues it only after the read job. Relink carries later-step matches over between runs (1aC-3): record the evidence at match time, e.g. the file's audio_hash, and drop a carried match when its file's audio changes; this applies to 1aD-2 and user matches too) → 1aD-2 Step 5: filename-only, stored as probable and unconfirmed → 1aD-3 Step 6: gig stick recovery (if 1aB-12 was done)
- **Lane 2:** 1aD-4 Attach rekordbox data to tracks with provenance; write BPM and key into `analysis`; parse My Tags out of Comments (rekordbox's "Add My Tag to the comments" setting, in the format the E2 spike recorded) into `rekordbox_track.my_tags`, which 1aB-10 leaves empty
- **Lane 3:** 1aD-5 Missing list with last known paths + an offer to add folders: shown in Review, holding rekordbox tracks whose file is missing with their rekordbox data intact until relink marries them to a file (§1.3)
- **Lane 4:** 1aD-6 Linked Library tracks: create, list, store (1.8)
- **Lane 5:** 1aD-7 Fragile-location detection (Downloads, temp, external and network drives), with the warning and its reason (1.3). Known limit from 0B-10: NVMe drives in Thunderbolt enclosures report as internal
- **Lane 6:** 1aD-8 Per-track send values: rekordbox data plus the best tags from the track's files

### Stage 1aE: Starting a Library, and the XML writer
- **Lane 1, first run:** 1aE-1 Pick music folders → 1aE-2 Start from rekordbox: whole collection → linked (tracks whose file is missing go to the Missing list, not the Library, §1.3) → 1aE-3 Start from rekordbox: chosen playlists → 1aE-4 Start fresh, adding linked tracks from All music
- **Lane 2:** 1aE-5 "File missing" flag, updated on each scan
- **Lane 3:** 1aE-6 In-app delete: confirmation, undo, dropped from the next send
- **Lane 4, XML writer:** 1aE-7 Every attribute from a fresh read (rule 1; the reader keeps values byte-exact, so escape tab, CR and LF as character references on write) → 1aE-8 Analysis fields omitted for tracks already in rekordbox (rule 3, case A) → 1aE-9 COLLECTION = every track the sent playlists and crates reference, plus new tracks, with every attribute (rule 4; T1 failed, so there's no playlists-only send) → 1aE-10 Percent-encoded `Location` + `Crates` / `Playlists` folder nodes
- **Lane 5:** 1aE-11 Record `sync_base` for every field sent (field names = the rekordbox XML attribute names; then enforce in the DB that analysis fields never become conflicts)

### Stage 1aF: Send flow
- **Lane 1:** 1aF-1 Guided send checklist UI + the "don't play in between" warning
- **Lane 2:** 1aF-2 Stale-playlist and manual-removal lists after each send

### Stage 1aG: Phase 1a check 👤
- **Lane 1:** 1aG-1 End to end: start from rekordbox with the reference library → send → read back → 1aG-2 End to end: start fresh → add → send → read back
- **Lane 2, dogfooding:** 1aG-3 Two real read → send → import cycles around real or practice gigs (none booked as of 2026-09-30; the owner stages practice ones), with the friction logged in `DOGFOOD.md`. 1aG-4 (freeze musicmanager) dropped: it was never part of the owner's workflow
- After both lanes: 1aG-5 **Checkpoint:** 1a success criteria (§6), including no bytes changed outside the app data folder

---

## Phase 1b: Order out of the mess

### Stage 1bA: Building blocks
- **Lane 1, fingerprint matching:** 1bA-1 Block index for candidate pairs → 1bA-2 Full-track comparison: two-way coverage + difference score
- **Lane 2, test corpus:** 1bA-3 Local-only corpus from the 43 private E3 pairs (including #42) → 1bA-4 Synthetic CI corpus covering the same patterns (start from `tools/fixture-gen`'s duplicate/version ground truth, 0D-7; check with real chromaprint that its reworks and cuts land in E3's coverage ranges, since the generator models them with correlation)
- **Lane 3, version parser:** 1bA-5 Bracket/segment tokenizer → 1bA-6 Store and rip junk stripper → 1bA-7 Cut labels + pool conventions → 1bA-8 Rework labels + bootleg patterns → 1bA-9 Mashups (`A x B`)
- **Lane 4:** 1bA-10 Spectral cutoff analyzer (ported from the spike)
- **Lane 5:** 1bA-11 Decoded vs header duration + decode-error detection
- **Lane 6:** 1bA-12 Multi-step undo (needed by "Accept all")

### Stage 1bB: Decisions
- **Lane 1, grouping rules:** 1bB-1 Duplicate rule (≥90% both ways, score ≤4) → 1bB-2 One-sided → cut link; none → name-based rework link → 1bB-3 Version veto + fingerprint veto → 1bB-4 Credits signal (remixer-only credits) → 1bB-5 "Needs review" on disagreement, with a reason
- **Lane 2:** 1bB-6 Quality verdicts and thresholds, with reasons (1.6)

### Stage 1bC: Grouping and best file
- **Lane 1:** 1bC-1 Grouping job replacing provisional grouping; re-point `library_track` rows safely (the merge function must refuse moving a file between linked versions by delete-then-insert, which the schema's triggers can't see)
- **Lane 2, best file:** 1bC-2 Ranking with reasons → 1bC-3 Per-field tag and art picking + recorded disagreements
- **Lane 3:** 1bC-4 "Better file available" for linked tracks, with its reason (1.7)

### Stage 1bD: Review scaffold and corpus check
- **Lane 1, Review scaffold:** 1bD-1 Queue list, grouped with counts → 1bD-2 Comparison pane: one row per file, suggestion first, with its reason
- **Lane 2:** 1bD-3 Corpus check: false merges <1%, pair #42 kept apart

### Stage 1bE: Review features
- **Lane 1:** 1bE-1 "More details" toggle + tag sources in the Details panel
- **Lane 2:** 1bE-2 Keyboard: Enter, number keys, S to split, auto-advance
- **Lane 3:** 1bE-3 "Accept all N" with undo and a view of what it accepted
- **Lane 4:** 1bE-4 Playback that keeps position when switching files
- **Lane 5:** 1bE-5 Single-file quality actions: Keep anyway / Find a better file (flagged for the Phase 2 wishlist)
- **Lane 6:** 1bE-6 Relink confirmation items (withdrawing a confirmation returns the track to recomputation, per 1aC-3; a confirmed match survives audio changes only if the user re-confirms)
- **Lane 7:** 1bE-7 "Better file available" items

### Stage 1bF: Phase 1b check 👤
- **Lane 1:** 1bF-1 **Checkpoint:** 1b success criteria (§6)

---

## Phase 1c: Round trip and daily use (MVP)

### Stage 1cA: Engines and components
- **Lane 1, conflicts:** 1cA-1 Three-way diff engine per field → 1cA-2 Conflict records + bulk-resolve operations
- **Lane 2, table:** 1cA-3 Port `TrackTable` with virtualization → 1cA-4 Filter model + filter chips → 1cA-5 Port `ColumnEditor`
- **Lane 3:** 1cA-6 Full-text search index (SQLite FTS5)
- **Lane 4, preview:** 1cA-7 Preview over a custom Tauri protocol → 1cA-8 AIFF/ALAC decode for preview
- **Lane 5, crates:** 1cA-9 Port the playlist tree as the crate folder tree → 1cA-10 Crate commands: create, rename, delete, add, remove, with undo (first decide how `crate_entry`, a WITHOUT ROWID table, becomes undoable: a migration giving it a rowid, or change rows keyed by a composite key; the 0E-10 op log only records rowid tables, and doesn't record ON DELETE CASCADE children, so delete children explicitly) → 1cA-11 Crate folders in the sidebar
- **Lane 6:** 1cA-12 Overview data queries
- **Lane 7:** 1cA-13 Job queue: resume unfinished jobs on startup

### Stage 1cB: Screens
- **Lane 1, conflicts UI:** 1cB-1 Side-by-side values: keep app / keep rekordbox / edit → 1cB-2 Bulk resolve ("take rekordbox for all ratings")
- **Lane 2:** 1cB-3 Merge conflicts before every send
- **Lane 3:** 1cB-4 Treat a changed file tag on a linked track as a rekordbox edit
- **Lane 4, hidden:** 1cB-5 Hidden filter with reasons and "way back" actions → 1cB-6 Search shows "N hidden files match · Show them"
- **Lane 5, keyboard:** 1cB-7 Command palette (Ctrl+K) → 1cB-8 Hotkeys J/K, Space, 1–0, T + bulk selection
- **Lane 6, player:** 1cB-9 Waveform peak job + cached display → 1cB-10 Port `PlayerBar`
- **Lane 7, Overview:** 1cB-11 Header with the Send button + to-do list + progress line → 1cB-12 Four numbers, energy and genre bars, "Outside your Library" strip
- **Lane 8, adding to crates:** 1cB-13 Drag onto the sidebar, press C, "Add to crate or playlist" dialog → 1cB-14 Crate membership in the Details panel + "Remove from this crate"
- **Lane 9, crate view:** 1cB-15 Notes line + summary line (BPM, energy, keys, genres) → 1cB-16 "Sort by fit" with reasons in the Fit column

### Stage 1cC: Integration and performance
- **Lane 1 👤 + Claude:** 1cC-1 Decide how energy reaches rekordbox (§7) → 1cC-2 Update the XML writer to match
- **Lane 2:** 1cC-3 Crates sent under the `Crates` folder with their trees
- **Lane 3:** 1cC-4 "Every number is a door": filtered views with an explanatory chip
- **Lane 4:** 1cC-5 Launch behavior: reopen where you left off, or open on Overview when there's news
- **Lane 5:** 1cC-6 Performance pass on the 100k fixture: scan, table, search (produce a list of issues)
- **Lane 6:** 1cC-7 Performance pass: Review and Overview queries (produce a list of issues)

### Stage 1cD: Fixes and pre-public
- **Lanes 1…n:** 1cD-1 Performance fixes from 1cC-6/7, **one lane per issue**
- **Lane 👤, gig:** 1cD-2 Prep and play a real or practice gig with it; log the friction in `DOGFOOD.md`
- **Lane 👤, legal:** 1cD-3 Pre-public checklist (moved to 1aP on 2026-09-30)

### Stage 1cE: Go public 👤
- **Lane 1:** 1cE-1 **Checkpoint:** 1c success criteria (§6). (1cE-2, making the repo public, moved to 1aP on 2026-09-30)

---

## Phases 2–4 (feature level: break down into stages and lanes when Phase 1 is done)
- **Phase 2:** Gate tests T2 + T4 👤 → Managed copies & upgrades (2.6; the Library folder joins the write guard as a second root, and the guard refuses writes through hard-linked files, so decide how Library copies treat a user's hard links before tag writing, with the tag-writing, transcode, rules and naming ports) · Online clients port (MusicBrainz, Discogs, Cover Art; pin each service to its hosts in `net`, strip auth headers on cross-host redirects, and check the User-Agent contact URL is public) · Inbox (2.1) · Identification & tag cleanup (2.2) · Wishlists (2.3) · Store links (2.4) · Auto tick-off (2.5)
- **Phase 3:** Tag vocabulary (3.1, needs T5) · Genre taxonomy (3.2) · Smart crates (3.3) · "What's next" (3.4) · Playlists with energy arc (3.5) · Play history (3.6, needs T6) · Local estimates (3.7) · Local ML (3.8, last)
- **Phase 4:** Set `bundle.publisher` for the installer (it shows "tracklistpro" today; 0F-1) · Updater + release script port · Portability bundle (4.1) · Onboarding (4.2) · Diagnostics (4.3) · Docs & community (4.4)
