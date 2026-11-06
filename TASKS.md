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
- **Plan approved 2026-11-05, budget 8 weekly points, started at 5%:** ✅ W1 wishlists and matching: migration 0048, paste and CSV import by header, owned / other cut / rework / missing with reasons, off by default (PR 182); W2 the Wishlists screen with copy-only store links, bought and notes, the switch in Settings; W3 auto tick-off: Done for you in the Inbox, Undo returns the file, optional crate named after the wishlist; one 100k measurement of the new features.

### Run 2C + 3A + 3B (owner 2026-11-05: one approval, budget 20 weekly points, started at 6%)
- **2C** reading rekordbox's library directly (2.7): R1 the reader, R2 its Settings switch. **3A** set prep basics: G genre tree with aliases (3.2), T tag vocabulary from My Tags (3.1, no My Tags writing until T5), S smart crates (3.3). **3B** H play history import (3.6), N "what's next" (3.4). Barebones wording, all proposed.

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
