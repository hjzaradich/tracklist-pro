# E3: Duplicate and version labels (2026-09-27)

43 pairs of files from a real collection, labeled one at a time by the author. The pair list and labels are the author's own and aren't published; this is the write-up of what they showed. Track names are left out.

## How the pairs were chosen
- From names, tags, durations and sizes only (**no fingerprints**), so the labels can grade the fingerprinter without having been chosen by it.
- **One pair per song**, to avoid repeats. An early draft had the same few songs five times each, because an old copy of a gig USB sat in the collection, so many songs existed 3–4 times.
- Ten kinds of case: exact copy, format pair, bitrate pair, extended vs radio, flip/edit, VIP, clean/dirty, mashup, near-duration same artist, same title different artist.

## Results
- **24 version · 14 same · 5 different** (one cover pair resolved as a version).
- **Length can't separate "same" from "version":**
  - "same" pairs differ by **up to 4 s** (a different release or rip of one mix)
  - **5 "version" pairs are within 5 s**: a Clean vs Dirty pair (0 s), a bootleg edit (1 s), two remixes (1 s and 2 s) and a pair of remixes (4 s)
- **Name-based guesses are unreliable.** The category that picked each pair was wrong in several cases:
  - 2 of 5 "format pairs" were remixes
  - 1 "same title, different artist" was a remix credited only to its remixer
  - 1 "flip" pair was two different songs

  → **Acoustic fingerprinting is required.** Names and length can only nominate candidates.

## Hard cases the version logic must handle
| Case | Lesson |
|---|---|
| An original and a bootleg edit, 1 s apart | A tiny length difference can still be a different version |
| One mix on two releases, 4 s apart | A few seconds of difference can still be the same recording |
| A remix credited only to the remixer | Different artist ≠ different song (remixer-only credits) |
| Two remixes with the same title, of different original songs; neither filename names the original artist | **Same title, different original songs** |
| A mashup whose part names match unrelated tracks | A mashup's part names can match unrelated tracks |
| A track called an "Edit" that is really a mashup | Don't design around a one-off joke name, but don't trust "Edit" to mean "not a mashup" either |
| A title that ends in "(dirty)", next to a file whose title ends in "(dirty) (dirty)" | **Version words can be part of the real title.** Never strip them blindly |
| Clean vs Dirty of the same length | Clean/Dirty needs the label or tags (the fingerprints will be nearly identical) |
| A store filename with an ID prefix and HTML-escaped parentheses | Unescape HTML entities, and strip store ID prefixes |
| A cover of the same song by a different artist | **Covers: rule = version, shown with a "cover" label** |

## Rules from the author
- Remix, flip, bootleg, VIP, extended/radio/club edit, rework, live, alternate arrangement, clean/dirty, mashup containing it: all **version**.
- **Cover = version, with its own "cover" label** so it's never confused with a remix.
- Rips of the same recording at different bitrates, and FLAC vs MP3 of the same master: **same**.

## Clarification: "version" means related, not interchangeable
"Many of the cases where I called things versions are not substitutable for each other in any way. A VIP or remix could change the genre or entire vibe of a track." Resulting model (confirmed, recorded in ROADMAP §1.2):
- **Only "same" merges files** (duplicates → one track, best file chosen).
- **Versions are separate tracks, linked but never merged.** No field is inherited across versions: genre, BPM, key, energy, tags, cues and history are all per track.
- The author called the group **"Versions"**, with two kinds:
  - **Cuts** (same production, different length or lyrics): Extended, Radio/Club Edit, Clean/Dirty, Intro/Outro, Short. Nearly interchangeable. A wishlist item counts as "owned, other cut".
  - **Reworks** (a different production): remix, VIP, flip, bootleg, rework, cover, live, arrangement, mashup. Never a stand-in. A wishlist item isn't owned; "you have a rework" is only a hint.
- Of the "version" labels, three were cuts (extended vs original, radio edit vs original, clean vs dirty). The rest were reworks.

## Side findings
- An old gig-USB copy (rekordbox's `Artist\Album\` layout) was a major source of exact duplicates.
- Stream rips appeared at **129–138 kbps** next to 320 kbps rips of the same songs. These are fake-320 / quality test candidates.

## Fingerprint grading (audio-lab + rusty-chromaprint 0.3)
- **Same (13):** all have ≥98% coverage both ways, score ≤2.5 (exact copies score 0.00). **13/13.**
- **One pair** (a single-tagged FLAC vs an album-tagged MP3) was first labeled *same* but matched only 62%/62%: aligned for about 115 s with weak scores, then nothing. **After listening, the author relabeled it *version*:** one file was a demo or live recording that sounded much worse. **The fingerprint was right and the tags were misleading**: same title and artist, different recording. Final tally: **14 same → 13; 24 versions → 25.**
- **Different (5):** coverage 0–1%. No false matches.
- **Versions (24):**
  - most reworks 0–25%
  - a VIP: 48%/36% (shares sections)
  - cuts: an extended mix 50% / its original 72%; an original 85% / its radio edit 47%
  - **Clean vs Dirty: 100%/100%, score 0.23**, indistinguishable by audio
- Rule: duplicates = ≥90% both ways and score ≤4, with the name-based version check able to veto; one-sided coverage = cut link.
- Speed: 43 pairs (86 full tracks, decoded twice) in 25 s on 4 threads, about 1 s per track per core.
