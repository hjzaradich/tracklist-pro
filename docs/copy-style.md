# Copy style

How tracklist-pro's user-facing text reads. Every phrase or sentence the app shows is a proposal until the owner approves it (CLAUDE.md, "User-facing copy is the owner's call"). Draft in this style so proposals land close to the final wording. List new or changed phrases under "Copy for the owner" in your report.

## General
- **Short and plain.** "No crates", "Select a track", not full sentences.
- **A string is either a full sentence or a phrase.** Write full sentences in full. Make phrases (buttons, statuses, labels) as economical as possible, with no filler words: "Read export", not "Read the export".
- **Empty and idle states name exactly what's absent**, so they can't be read as a fault: "No background tasks", not "Nothing running".
- **Extra detail goes in parentheses**, not after a comma or a middle dot: "Scanning music folders (42%)", "Checking files (10%) (2 other tasks)".
- **Singular and plural are both written out** (i18next plurals): "1 other task", "2 other tasks".
- **Use the ROADMAP §1.2 words**, one word per concept (Library, All music, Details, Activity, crate, version, cut, rework, best file…).
- **Musical keys use plain `#` and `b`**, never ♯ or ♭.

## Suggestion reasons
Every suggestion shows a short reason (the "No black boxes" rule). Reasons are i18n keys with parameters, never text built in code.
- **Fragments, not sentences:** lowercase start, no final period, no "because".
- **Facts you could check in the app, never opinions or praise:** "same key", not "great match".
- **One fact per fragment, at most 3 fragments**, most decisive first. Fragments are joined with ", ".
- **A comparison or supporting detail goes in parentheses** after the fact it supports.
- **Numbers with sign and unit:** "+2 BPM", "320 kbps", "16 kHz".
- **Say the source when sources disagree:** "rekordbox key", "file tag".
- **No hedging** ("maybe", "probably"). The only uncertainty label is "(audio model)", added automatically; an audio-model reason never stands alone.
- **Counts:** "once" for 1, "N times" otherwise.

Examples:
- same key, +2 BPM, tagged Peak Time
- 320 kbps (the other file is 128 kbps)
- no 16 kHz cutoff
- same audio fingerprint
- title says Extended Mix
- follows this track in your history 4 times
- 3 tracks in this crate are tagged Warmup
- genre in 2 of 3 file tags
- energy 7 (crate average 7)
- sounds dark (audio model)

*Owner-approved wording, 2026-09-28.*

## Error messages
Errors are an error kind plus parameters (1aA-12), with the wording in the locale files.
- **Say what happened, then one thing to do,** in at most two short sentences: "Disk full. Free up some space and try again."
- **Feature errors can be one fragment with the detail in parentheses:** "Folder not found (D:\Music)", "Already inside a music folder (D:\Music)".
- **A refusal says what was refused first:** "Can't remove: tracks use files in this folder".
- **Only offer actions the app supports today.** No "report it" until there's a way to report; no "restore a backup" until backups exist.
- **Never show raw error text** from the database or the system.

Examples:
- The Library is busy. Try again in a moment.
- Can't save to the app's data folder (it may be read-only). Restart the app.
- Something went wrong. Try again or restart the app.
- Music folder not found

*Owner-approved wording, 2026-09-29.*
