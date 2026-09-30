# rekordbox behavior check

Checks that rekordbox still behaves the way [ROADMAP §5.2](../../ROADMAP.md#52-xml-import-behavior-rekordbox-7) says, and answers gate tests T1 and T3 (§7). **Run it again after every rekordbox update**, before trusting sends.

**Time:** about an hour. **Where:** a demo or empty rekordbox is best; the backup in step 0 undoes everything at the end.

**Before you start**, build the kit (or ask Claude to). The music folder is only read:

```
python C:\dev\tracklist-pro\spikes\rb-kit\rb_check.py build "<a music folder>"
```

- The kit is `%USERPROFILE%\tlp-rb-check\`. It holds *copies* of 12 tracks with deliberately awkward names, 2 made-up test tones (tracks 13 and 14), and `1-first.xml`.
- Save every export into the kit's `exports` folder, with exactly the name given.
- Don't open, move or edit the kit's files yourself.
- If a menu or dialog doesn't match these steps, write down what you see.

## 0. Start (10 min)

1. **Help → About rekordbox**: note the version.
2. **File → Library → Backup Library**: save it outside OneDrive, e.g. `%USERPROFILE%\rekordbox-backups\`. Step 5 restores it.
3. **Preferences → Analysis**: make sure tracks are analyzed automatically when added.
4. **Preferences → View → Layout**: tick **rekordbox xml** if it isn't already.
5. **Preferences → View → Key display format**: pick **Classic** (not Alphanumeric). The test tones check how rekordbox spells keys.

## 1. First import (10 min)

1. Drag the kit's `drag-me` folder into **Collection** (2 tracks).
2. **Preferences → Advanced → Database → rekordbox xml → Imported Library → Browse** → the kit's `1-first.xml`.
3. **rekordbox xml → All Tracks**: select all 11 → right-click → **Import to Collection**.
   - If a dialog says a track **already exists**, stop: an earlier run wasn't undone. Tell Claude.
4. **rekordbox xml → Playlists**: right-click `TLP Check` → **Import Playlist**.
5. Wait until all 13 tracks are analyzed (they show a BPM and key).
6. Look at the **Key** of `TLPCHK K2B Key 2B [v1]` (a tone in F# major) and `TLPCHK K6B Key 6B [v1]` (Bb major). Write down exactly how each is spelled, e.g. `F#` or `Gb`, `Bb` or `A#`. The analyzer reads it too; your note is the backup.
7. **File → Export Collection in xml format** → `01-after-first.xml`.

## 2. Two edits in rekordbox (5 min)

1. Track `TLPCHK 01 Edit me [v1]`: change its title to `EDITED IN RB`.
2. Track `TLPCHK 04 T3 both [v1]`: load it on a deck and set **hot cue A** anywhere.
3. Export → `02-before-resend.xml`.
4. Run this. It writes three more XML files into the kit:
   ```
   python C:\dev\tracklist-pro\spikes\rb-kit\rb_check.py next
   ```
   If it names a problem, fix it, export `02-before-resend.xml` again, and rerun. If it warns that the keys are in Camelot, do step 0.5, export 02 again and rerun.

## 3. Resend on top (15 min)

> [!WARNING]
> **Don't skip step 3.4 (Import Playlist).** Step 3.3 imports the tracks but not the playlists, and rekordbox gives no hint that anything is left. Do 3.4 before you export `03-after-resend.xml`. If it's skipped, the analyzer only finds out at the end, and the whole check has to be run again.

1. **Preferences → Analysis**: turn automatic analysis **off**. (Otherwise rekordbox could quietly re-analyze and hide a lost BPM or key.)
2. **Imported Library** → `2-resend.xml`. If the sidebar still shows the old tracks, restart rekordbox.
3. **rekordbox xml → All Tracks**: select all 11 → **Import to Collection**.
   - A dialog asks about each track ("This track already exists…"). Click **Yes** every time, 11 times. Don't tick "don't ask me again".
4. ⚠️ **rekordbox xml → Playlists**: right-click `TLP Check` → **Import Playlist**. If it asks to replace lists with the same name, click **OK**.
5. Export → `03-after-resend.xml`.
6. Look at the **Key** of the two test tones again: the resend swapped their keys, so K2B should now be Bb major and K6B F# major. Write down what each shows (a spelling like `Bb`, or `6B` as sent, or unchanged).

## 4. T1: playlists pointing at tracks rekordbox already has (10 min)

1. **Imported Library** → `3-t1a-playlists-only.xml`. All Tracks should be empty.
2. Right-click `TLP T1a` → **Import Playlist**. Write down any dialog or error.
3. Export → `04-after-t1a.xml`.
4. **Imported Library** → `4-t1b-mixed.xml`. All Tracks shows one track, `TLPCHK 10 New in T1b [v1]`: **Import to Collection**.
5. Right-click `TLP T1b` → **Import Playlist**.
6. Export → `05-after-t1b.xml`.

## 5. Results, then undo (10 min)

1. Run this, or tell Claude you're done. It prints PASS or FAIL for each behavior and saves `exports\report.txt`:
   ```
   python C:\dev\tracklist-pro\spikes\rb-kit\rb_check.py analyze
   ```
   If it says **step 3.4 was skipped**, the playlist and T1 results don't count: finish steps 5.2–5.3, rebuild the kit with `--force`, and run the check again from step 0.
2. **Preferences → Analysis**: turn automatic analysis back **on**.
3. **File → Library → Restore Library** → the backup from step 0.
4. Keep the kit until the results have been read. To run the check again later, rebuild the kit with `--force`.

**Notes** (version, dialogs, the tones' keys from steps 1.6 and 3.6, anything unexpected): ______

---

## What each part tests (for Claude)

The analyzer reads the exports; matching is by Location, decoded by hand (§5.3). Every track title starts `TLPCHK`, and every re-imported track's comment becomes `TLPCHK v2 <n>`, which shows the "Yes" landed.

| # | File (kit) | First import | Step 2 | Resend (`next` builds it from export 02) |
|---|---|---|---|---|
| 01 | `' & []` | Rating 153, PlayCount 5 | retitled | old title sent back; Rating and PlayCount sent as they are (control) |
| 02 | accents | Rating 204, red | — | Rating omitted → expect 0 |
| 03 | `# %` | PlayCount 5 | — | `PlayCount="0"` → expect 0 |
| 04 | `+ ()` | | hot cue | **T3:** Tonality and AverageBpm omitted |
| 05 | CJK | | | **T3:** Tonality omitted |
| 06 | `,` emoji | | | **T3:** AverageBpm omitted |
| 07 | nested folders | | | **T3 control:** both sent |
| 08 | | made-up BPM/key (ignored?) | | made-up BPM/key again (info) |
| 09 | | | | left out → stays; T1 target |
| 10 | | | | (T1b's new track) |
| D1 | `# & ' + , ()` emoji | dragged in | | sent with our encoding → no duplicate |
| D2 | | dragged in | | T1 target |
| K2B | tone, F# major | analyzed by rekordbox (no key sent) | | `Tonality="6B"` (swapped, Camelot) |
| K6B | tone, Bb major | analyzed by rekordbox (no key sent) | | `Tonality="2B"` (swapped, Camelot) |

- Every resent track carries every attribute with rekordbox's value from export 02 (rule 1), a fully encoded Location (rule 5), and no TEMPO or POSITION_MARK (rule 3, case A). Only the field under test differs.
- **T3** passes if 04–06 keep key, BPM, beatgrid and cue while their comment shows the resend landed. Automatic analysis is off during the resend, so a lost value can't be re-created and hidden. 07 must also keep everything; if it doesn't, the result is inconclusive.
- **T1a** sends an empty COLLECTION and two playlists over existing tracks: one keyed by rekordbox's TrackIDs (KeyType 0), one by Location (KeyType 1). **T1b** does the same with one new track in COLLECTION (the ongoing case for rule 4). Each passes if the playlist holds exactly the right tracks in order, no track is duplicated, and the existing tracks are otherwise untouched.
- Playlists: `pl-replace` is resent with a new order (expect replaced, not merged or duplicated), `pl-left` isn't resent (expect it to stay), and `nested/pl-deep` checks that folders and order survive. If `pl-replace` is unchanged between exports 02 and 03, step 3.4 was skipped (or rekordbox ignored it): the analyzer fails "Step 3.4 done" and marks the two playlist checks and the T1 rows inconclusive rather than reporting a behavior change. It's only found after step 4, and re-exporting 03 then would capture T1's changes, so the fix is a full rerun.
- **Key spellings:** K2B and K6B are generated tones (a I–IV–I–V cadence at 120 BPM, `tones.py`), not copies. The analyzer reads the key rekordbox's analysis wrote for them in export 02 and compares it with the app's rekordbox key names, `REKORDBOX` in [key.rs](../../src-tauri/src/tags/key.rs) (`F#` for 2B, `Bb` for 6B): PASS if they match, FAIL if rekordbox spells the same key differently (update those names), INCONCLUSIVE if it heard another key or wrote Camelot (key display not Classic). It also lists every key spelling seen in export 02, and says whether the swapped Camelot key on the resend was converted, kept as a string, or ignored.
- This check replaces the one-off E1 session (2026-09-27) that first established §5.2; that session's script and steps aren't published.
- Tests: `python -m unittest discover -s spikes/rb-kit -p "test_*.py"` from the repo root (or `python -m unittest -v test_rb_check` in this folder). `E1ExportsLocal` runs only where the private E1 exports exist.
