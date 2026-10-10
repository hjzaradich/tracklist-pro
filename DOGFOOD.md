# Dogfooding log

Friction found while using tracklist-pro for real read → send → import cycles (TASKS Stage 1aG). This file is public: describe tracks by what they are ("an MP3 with an accent in its folder name"), never by title or artist.

Each entry gets one of three marks, set at the checkpoint:
- **Fixed in 1aG:** it gave wrong data or stopped a cycle.
- **Before 1b:** fix before the next phase starts.
- **Later:** kept as an idea.

## Known before cycle 1

Found by a read-through of the Phase 1a flow and by reviews on 2026-10-02, and left as they are for now. They are listed here so the cycles don't report them again. Each gets a mark at the checkpoint like any other entry.

| # | Where | What to expect | Mark |
|---|---|---|---|
| K1 | Overview, "Start from rekordbox" | The offer stays hidden until scanning, grouping and relinking are done and there is something to add. Until then nothing says it is coming. | |
| K2 | Overview, "Start from rekordbox" | The count read from the export is rekordbox entries; the count added is tracks. Exact duplicates collapse into one track, and no line explains the difference. | |
| K3 | Library | Building a crate from a large Library is slow going: no search, no paging, and a crate picker on every row. | |
| K4 | Send checklist | Closing the checklist while a step is running loses the wait. Reopened, it shows the state from before the step ended until it is refreshed. | |
| K5 | After a send | The "remove in rekordbox" and stale-playlist lists are only inside the Send checklist, which can't be opened with an empty Library. | |
| K6 | A file moved on disk | Its Library track shows "File missing" and is left out of sends, even after relinking finds the file. Removing the track and adding it again fixes it. By design until matches can be confirmed (1bE-6). | |
| K7 | Relinked tracks | They are sent with rekordbox's old Location, so rekordbox still can't find the file, and the preflight doesn't say so. | |
| K8 | Scans | Cancelling a scan stage, or one failing, stops the chain. "Scan again" on the folder restarts it. A scan that is already running can't be moved ahead of other work, and the rescan queued behind it runs at background priority. | |
| K9 | Fingerprinting | "Visible tracks first" is never asked for by the screens, so fingerprints run in scan order. | |
| K10 | rekordbox BPM and key | They are stored when an export is read, but no screen shows them in 1a. | |
| K11 | Placeholders | The Overview's subtitle, the search box with "Ctrl K", "Select a track" and "Nothing playing" look like features and do nothing yet. | |
| K12 | Memory | Every scanned file is kept in memory for the session and never shown. | |
| K13 | Formats rekordbox may not play | New tracks in Ogg, Opus, WMA and similar are sent without a Kind and without a warning. | |
| K14 | All music | A row can show a title taken from the file's tags, which the list doesn't sort or search by. Sorting and search use the stored title, artist and file name. | |
| K15 | Library | The list at 10,000 tracks takes about 0.4 s to load, up from about 0.1 s, since it shows the titles a send would write. | |
| K16 | Sending | One send per export: after a send, a second send from the same export is refused until a new export is saved, whether or not the first was imported. | |
| K17 | A drive with coarse file times (FAT, exFAT) | An export saved within 2 s after a send could be refused as older than the send. | |

## Cycle 1 (backup copy of the rekordbox library)

| # | Step | What happened | What was expected | Mark |
|---|---|---|---|---|
| C1-1 | 3.1, first scan and fingerprinting | Fingerprinting about 9,900 files (8,535 when first counted, in OneDrive folders) took about 1.5 hours awake: 29% after 6 minutes, 60% after about 40 minutes, and a second pass of about 40 minutes (the PC slept for an hour in between). While it ran the PC was at 100% CPU with OneDrive and the antivirus busy (files carry OneDrive's reparse-point flag; the owner believed OneDrive was off, but its process was running); reads ran at about 3 MB/s. The owner paused OneDrive syncing partway through; whether it sped things up isn't proven. | A steady rate, or a hint in the app that something else on the PC is slowing the reads. |  |
| C1-2 | 3.1, Activity | The fingerprint progress bar dropped from 60% to 35%. The first fingerprint job had ended (about 50 minutes) and a second one, for files that became ready after the first listed its files, started its own 0 to 100%. No wait, nothing was lost, but it looks like going backwards. | One progress figure that only goes up, or a line saying a second pass has started. |  |
| C1-3 | 3.1, scan | The scan of 8,535 files took about 30 seconds, hashing the larger folders took 2 to 4 minutes each. | (timing note, nothing expected) |  |
| C1-4 | 3.2, rekordbox library | The rekordbox library on this PC is a leftover test library: of 184 entries, 170 point at files that no longer exist (folders from an earlier phase), 14 point at files that exist. The owner then added 7 tracks from the OneDrive folders (a FLAC with 2 cues, an M4A with a cue, an MP3 with `$` and a non-English letter in its path and a cue, an MP3 with `!!` in its path and a cue, 3 plain MP3s with a grid and no cues; play counts on 3 of them) and exported again. | (setup note) |  |
| C1-5 | 3.3, reading the export | The export holds 191 entries; the app stored 177 and said "164 rekordbox tracks aren't in your library". The 14 left out are rekordbox's own built-in sampler sounds, skipped on purpose (`rekordbox/skip.rs`), but nothing on screen says some entries were skipped or why. 157 of the 164 were paired by file name and length only (confidence 0.9, which the app treats as a trusted match, not a probable one) with copies of the old test tracks in another folder, while rekordbox points at folders that no longer exist. | A line saying how many entries were skipped and why; matches by name and length shown as probable. |  |
| C1-6 | 3.5, Library | The Library screen shows no count of its tracks, so the owner can't tell it holds 164 without counting; Overview says 177 (rekordbox tracks read) and All music 5,899. | A count on the Library, and a line on the Overview about how 177 relates to the 164 added. |  |
| C1-7 | 3.5, Missing | The 13 Missing rows show only a title and a path, with no reason and nothing to do next. | Why each is missing and what the user can do (relink, remove). |  |
| C1-8 | 4, crate | A Missing track can't be put in a crate: it isn't in All music, so it never reaches the Library. A known track with a missing file, which ROADMAP 1.9 says is still sent, can't be tried this way. | A way to include a Missing track in a crate, or a line saying why not. |  |
| C1-9 | 5, review | The review listed 16 known and 2 new and nothing else. Nine of the 16 (paired only by name and length with a copy elsewhere) were sent with rekordbox's old path, and rekordbox then showed "file not found" on them, visible only once a track is selected. Ten known tracks had their path spelling changed in escaping only (`( ) , ! $` written as `%xx`). The review says nothing about either. (The first is K7.) | A line in the review for tracks that will still point at a missing file, and one for re-spelled paths. |  |
| C1-10 | 7, playlist names | rekordbox let the owner make two playlists named "House" and "house" with no complaint, while the app refuses to send two crates whose names differ only in letter case (ROADMAP 5.2 listed this as unverified). Importing such names was not tried. | Either the app's refusal is confirmed necessary by an import test, or it is relaxed. |  |
| C1-11 | 8.3, Undo | After removing a Library track and deleting a crate (both prompts said "This action can be undone"), the owner opened the Send checklist and came back: no Undo button was anywhere. Undo exists only as a button on the screen right after the action and is gone once the screen is left; the one later attempt took back the newest removal instead of the crate. The crate can only be rebuilt by hand. | Undo that stays available after leaving the screen (a visible history of recent actions), or a prompt that doesn't promise it. |  |
| C1-12 | 3.2 to 7, files changed on disk | rekordbox rewrote 8 audio files in place (same size, 1 FLAC, 1 M4A, 6 MP3: the tracks the user added to rekordbox by hand, plus one the app had sent as new) when the user set cues or played them, between 11:26 and 11:34 and once at 01:40 the next night; the app did not write them (the write guard allows only the app data folder, and the times match the user's rekordbox actions). The app's rescan saw 7 of them and kept each on its own track: no new track, no duplicate, none missing, the same 5,899 tracks. For the 8th, not yet rescanned, every text frame the app stored equals the file's now. | (note: the checkpoint rule "nothing changed outside the app's data folder" has to be read as "nothing changed by the app") |  |
| C1-13 | 10, database size | After the cycle the database file was 85 MB and the write-ahead log file beside it 365 MB (about 9,900 files, 5,899 tracks). Nothing limits the log's size or shrinks it while the app runs. | A log file that is folded back into the database and stays small. |  |

## Cycle 2 (real library, practice gig)

| # | Step | What happened | What was expected | Mark |
|---|---|---|---|---|
