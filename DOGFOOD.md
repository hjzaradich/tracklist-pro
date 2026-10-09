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
| C1-1 | 3.1, first scan and fingerprinting | Fingerprinting 8,535 files in a OneDrive folder was slow: 29% after 6 minutes, then 60% after about 40 minutes, with the PC at 100% CPU and OneDrive and the antivirus busy (files carry OneDrive's reparse-point flag; the owner believed OneDrive was off, but its process was running). Reads ran at about 3 MB/s. After the owner paused OneDrive syncing, the rate rose to about 3% per 90 seconds. | A steady rate, or a hint in the app that something else on the PC is slowing the reads. |  |
| C1-2 | 3.1, Activity | The fingerprint progress bar dropped from 60% to 35%. The first fingerprint job had ended (about 50 minutes) and a second one, for files that became ready after the first listed its files, started its own 0 to 100%. No wait, nothing was lost, but it looks like going backwards. | One progress figure that only goes up, or a line saying a second pass has started. |  |
| C1-3 | 3.1, scan | The scan of 8,535 files took about 30 seconds, hashing the larger folders took 2 to 4 minutes each. | (timing note, nothing expected) |  |
| C1-4 | 3.2, rekordbox library | The rekordbox library on this PC is a leftover test library: of 184 entries, 170 point at files that no longer exist (folders from an earlier phase), 14 point at files that exist. The owner then added 7 tracks from the OneDrive folders (FLAC, M4A, an MP3 with `$` and a non-English letter in its path, an MP3 with `!!` in its path, 3 plain MP3s) with cues, grids and play counts, and exported again. | (setup note) |  |

## Cycle 2 (real library, practice gig)

| # | Step | What happened | What was expected | Mark |
|---|---|---|---|---|
