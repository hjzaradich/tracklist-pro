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
- **Phrases are trimmed; a refusal's reason is not.** Buttons, labels and statuses drop filler words; the reason after "Can't X:" keeps its articles: "Can't undo: the Library has changed since".
- **Periods:** a single line standing alone has no final period, even if it is a sentence. Two or more sentences together each get one.
- **"your Library"** for the user's tracks and actions; **"the Library"** when it is the thing that failed or changed.
- **Help text opens with what to do.** When it describes a choice instead, it opens with what the user gets. The reason or reassurance comes second.
- **The subject of a line is the user's things** (tracks, folders, the export), never "the app" and never an opening "You". "The app" may be named as an object ("Restart the app"); "you" and "your" may appear inside an instruction.
- **A button is a verb.** Add the object only when the screen offers more than one thing to act on ("Add folder"); never a pronoun ("Add it").
- **In a confirmation,** the action button restates the verb of the question ("Remove"), and the other button is "Go back". Never "OK", "Yes", "No" or "Cancel" in a confirmation.
- **An action that isn't available is greyed out,** not answered with a message after the click. If the reason isn't obvious, it is already stated on the screen near the control.
- **The same situation uses the same words on every screen, and each verb has one meaning:** Export (rekordbox makes its collection file), Read (the app takes in that file), Write (the app makes the file for rekordbox), Import (rekordbox takes in that file), Send (the whole flow); Choose (never "Pick"); Remove (out of a list, the thing still exists); Delete (gone).
- **A track is written "Artist - Title".**
- **When the count is the point of a line, the number comes first, as a digit** ("3 tracks left out"). Zero is written "No …", never "0 …".
- **Don't reassure about routine actions.** Reassure only when an action is complex or infrequent and the user has a real reason to worry about their music files, the state of rekordbox, or their computer (an action that takes a while or uses a lot of processing power). Then give one reassurance, not several.

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
- **Say what happened, then one thing to do,** in at most two short sentences: "Disk full. Free up some space and try again." A third is allowed only when it tells the user not to do something harmful.
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

## Warnings and consequences
- **State a consequence plainly,** with no idiom or softening: "will be lost", not "goes missing". Use the present for what is true now and the future for what will happen. "Lost" is only for what can't be recovered; when an action can be undone, say so.
- **A warning says what the user stands to lose and what triggers it,** in the user's terms. Leave out how the app or rekordbox does it, unless it is a step the user performs.
- **Use only the technical terms the user absolutely needs to know.**

*Owner-approved rules, 2026-10-01.*
