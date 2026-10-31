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
- **Lane 2, before 1c:** 1bF-2 Split off takes the selected file's identical copies with it, a refused Split off shows its reason, and the filled button follows the evidence (B-4, B-5) → 1bF-3 An "Unclear duplicates" item says what holds it (B-3) → 1bF-4 Review layout (B-6, the Designer) → 1bF-5 Vite update (6 security alerts, development only)
- **Owner decisions at the check (2026-10-24), for the fixes before 1c:** (A) on a possible version mix-up whose own reason says the names point to different versions, the filled button (Enter) is "Keep separate", not "Same track": the filled button follows the evidence; (B) "Split off" takes the selected file together with its files of the very same audio, so on an item with two copies of one version and two of another, one Split off keeps the copies together and the versions apart. Also before 1c: a refused Split off shows its reason (today it does nothing visible); an "Unclear duplicates" item says why it is held; the Review layout (the Designer).

---

## Phase 1c: Round trip and daily use (MVP)

Owner decision (2026-10-17): Phase 1c runs in the order below, every screen before the gig (1cD-2); not split into a gig-ready part first. Each stage still gets its own plan and budget.

### Stage 1cA: Engines and components ✅
In [TASKS-archive.md](TASKS-archive.md).

### Stage 1cB: Screens ✅
In [TASKS-archive.md](TASKS-archive.md).

### Stage 1cC: Integration and performance ✅
- **Plan (owner approved 2026-10-30): the dev process first.** ✅ D1 CI time and flaky tests (a ready test database copied per test instead of 40 migrations each time, the bindings check without a full rebuild, the known flaky tests; PR 141, 2026-10-30: the backend test step went from 32 to 13 minutes; consecutive finished `match` rows count as one in the chain tests until the "make way" rule is fixed, audit C2), then ✅ test sharding (PR 144, 2026-10-30: four shards plus a build part, the Windows part from about 14.5 to 8 minutes; a new top-level backend module or test binary goes in `.github/scripts/test-shards.mjs`, and CI fails until it does); ✅ D2 a lighter CI for frontend-only PRs (PR 143, 2026-10-30: the Windows Rust job runs only when a Rust input or a frontend file Rust reads changes, and a node test fails if Rust starts reading a frontend file that isn't on that list); ✅ D3 smaller, faster builds (debug info, optimised SQLite and hashing in dev, one crate type, a linker trial, the seroval lockfile bump that clears the 5 Dependabot alerts; PR 142, 2026-10-30: a build folder went from 35 GB to about 5 GB and a relink from about 311 s to 73 s. All builds now link with `rust-lld`, release included: if an installer misbehaves, first try `link.exe` again by removing `src-tauri/.cargo/config.toml`); D4 fewer PR collisions in `ipc.rs` and `bindings.ts` (dropped after a look: the collisions are mostly in generated types that no file layout avoids, tauri-specta needs one command list, and GitHub's merge queue is for organisation-owned repos only; the rebases come from the foreman's rule that a PR overlapping a newer merge gets a fresh CI run, which D1 makes shorter); D5 finished stages moved to [TASKS-archive.md](TASKS-archive.md). Then the lanes below, plus cleanup from a code audit: ✅ P3 (PR 145, 2026-10-30) the Library count instead of the whole list, the unused scanned-files store, the crate dropdown built once, one "reload after jobs" list, and the crate note ignoring Enter while an IME is composing; ✅ P4 (PR 151, 2026-10-31) backend hot paths with nothing written differently (the fingerprint "due" start point, re-used compiled queries in the Library check, the id-only crate check, the send merge's stale set worked out once, roles reading tags once); ✅ P5 dead code (PR 153, 2026-10-31: about 1,170 lines fewer; the seven job-trigger commands the UI never calls are left for an owner decision). 1cC-6 and 1cC-7 run as one measurement job after P3 and P4, in release and debug builds, and their list feeds 1cD-1; the slow spots earlier stages left for "the performance pass" are in TASKS-archive.md.
- **Lane 1 👤 + Claude:** ✅ 1cC-1 Decide how energy reaches rekordbox (§7; owner decision 2026-10-28: not sent by default; an opt-in setting sends it as the star rating; mapping, first switch-on and read-back settled the same day, ROADMAP §7) → ✅ 1cC-2 The opt-in setting and the XML writer to match (backend PR 146, the switch on a new small Settings screen PR 149, 2026-10-30; ROADMAP 1.9, 1.10, §7; its wording is with the Editor, 22 keys still draft)
- **Lane 2:** ✅ 1cC-3 Crates sent under the `Crates` folder with their trees (the send already wrote them, since 1aF; a check on 2026-10-30 found only end-to-end tests missing, added in PR 148: a nested tree read back from the XML, the empty and all-left-out cases, a track in two crates, and the depth limit through the real send)
- **Lane 3:** ✅ 1cC-4 "Every number is a door": filtered views with an explanatory chip (PRs 150 and 152, 2026-10-31; missing tags and missing art are flag bits in `track_row`, which took the Overview at 100k from about 17 s to 3 s in a debug build; the genre bars are not doors yet, they need a stored genre; the new filters want a 100k timing in the measurement job)
- **Lane 4:** ✅ 1cC-5 Launch behavior: reopen where you left off, or open on Overview when there's news (PR 147, 2026-10-30)
- **Lane 5:** ✅ 1cC-6 Performance pass on the 100k fixture: scan, table, search (produce a list of issues). Also the backend test suite: about 3,900 tests took 25-40 minutes on GitHub's Windows runners on 2026-10-28 (the job's limit was raised to 75 minutes, PR 126). And the send's prepare: about 200 s at 100,000 linked tracks in a debug build (measured on PR 127; the file-change check it added is 1.5 s of that). And setting energy on 100,000 selected tracks is one long operation outside the job queue, with no progress shown (PR 130), and adding 100,000 tracks to a crate takes about 34 s and its Undo 16 s in a debug build, with no progress shown (PR 133) **Done 2026-10-31 (PR 154, test code only): every number measured in a release build at 100,000 tracks; the issue list is under 1cD-1.**
- **Lane 6:** ✅ 1cC-7 Performance pass: Review and Overview queries (produce a list of issues) **Done 2026-10-31 in the same pass: Review needs nothing (counts 50 ms, pages 46 to 73 ms, opening an item 1 to 68 ms); the Overview is issue 5 under 1cD-1.**

### Stage 1cD: Fixes and pre-public
- **Lanes 1…n:** 1cD-1 Performance fixes from 1cC-6/7, **one lane per issue**. The owner approves the 1cD plan first. Measured 2026-10-31, release build, 100,000 tracks (33,000 of them in the Library for the Overview doors), ranked by what a user feels:
  1. Send: prepare 25 to 27 s, go 17 to 20 s, each with the same 11 to 15 s review as one writer job; progress only at the end (audit B6)
  2. Overview doors: 12 of 18 take 0.2 to 0.9 s a window, and about 3 times that with a 100,000-track Library; store low quality and shown energy in `track_row`, as missing tags and art already are
  3. Deciding job: 198 s on a first run, and 58 s at any restart that finds one new file, because the name index isn't stored
  4. Adding tracks from outside the Library to a crate: 77 s, 44 s with verdicts stored, no progress, blocks writes (audit B1)
  5. Overview open: 2.5 s, and again every 5 s while jobs count; 1.5 s of it is genres, so a stored genre fixes this and lets the genre bars become doors (audit B5, part of B9)
  6. Cold first scan: 295 s reading against 21 s warm; try more read threads once
  7. Search index from nothing: 49 s (10 s on 10-13), 59 s with the table rows
  8. Undo of a big operation: up to 362 MiB and 8 s (audit B3; high risk, last)
  9. Startup checks 16 s every launch with nothing due; roles pass 13 s after every rekordbox read
  10. Waveforms: about 5 to 11 hours of one thread for 100,000 files (extrapolated); an owner scope question
  Dropped with numbers: audit B2 (a 100k add to a crate is 6 to 7.5 s), B4 (about 2.3 s), B7, B12, all Review queries. Not measured: B8 and B10 (need a browser profile), B11.
- **Lane 👤, gig:** 1cD-2 Prep and play a real or practice gig with it; log the friction in `DOGFOOD.md`. This is also the second dogfooding cycle, moved here from 1aG-3 (2026-10-10). In order: (1) back up rekordbox and the app's data folder, clear old test entries out of rekordbox, install over the older build; (2) start fresh with a set-sized Library (50 to 150 tracks), send, prepare the tracks in rekordbox (analyze, cues), send again and check nothing prepared was lost; (3) the rekordbox checks cycle 1 couldn't run: paths with `;` `@` `=`, a network drive, the USB eject test, the unsigned-app warning, importing two playlists whose names differ only by letter case; (4) play the gig; (5) then bring the whole collection into the Library, after duplicates and versions have been reviewed
- **Lane 👤, legal:** 1cD-3 Pre-public checklist (moved to 1aP on 2026-09-30)

### Stage 1cE: Go public 👤
- **Lane 1:** 1cE-1 **Checkpoint:** 1c success criteria (§6). (1cE-2, making the repo public, moved to 1aP on 2026-09-30)

---

## Phases 2–4 (feature level: break down into stages and lanes when Phase 1 is done)
- **Phase 2:** Gate tests T2 + T4 👤 → Managed copies & upgrades (2.6; the Library folder joins the write guard as a second root, and the guard refuses writes through hard-linked files, so decide how Library copies treat a user's hard links before tag writing, with the tag-writing, transcode, rules and naming ports) · Online clients port (MusicBrainz, Discogs, Cover Art; pin each service to its hosts in `net`, strip auth headers on cross-host redirects, and check the User-Agent contact URL is public) · Inbox (2.1) · Identification & tag cleanup (2.2) · Wishlists (2.3) · Store links (2.4) · Auto tick-off (2.5) · Reading rekordbox's library directly (2.7; optional, off by default; read-only; one removable module; the user supplies the key)
- **Phase 3:** Tag vocabulary (3.1, needs T5) · Genre taxonomy (3.2) · Smart crates (3.3) · "What's next" (3.4) · Playlists with energy arc (3.5) · Play history (3.6, needs T6) · Local estimates (3.7) · Local ML (3.8, last)
- **Phase 4:** Set `bundle.publisher` for the installer (it shows "tracklistpro" today; 0F-1) · Updater + release script port · Portability bundle (4.1) · Onboarding (4.2) · Diagnostics (4.3) · Docs & community (4.4)
