-- 0044: the genre the Overview counts, stored in `track_row` (1cD-R1b; ROADMAP
-- 1.13 "What the numbers mean", "Every number is a door").
--
-- The Overview's genre bars were worked out per track when the screen opened:
-- the genre frames of every Library track's tags, read out of `raw_tags` JSON
-- and parsed with the send's rules, about 1.5 s of the Overview's 2.5 s at
-- 100,000 tracks. The index job now works each Library track's genre out
-- once (`overview::door_answers_of`, the one place that says what the genre
-- is: rekordbox's for a track rekordbox knows, else the linked file's tag) and
-- keeps it in the row:
--   genre        the genre as the track spells it, without outer blanks
--   genre_group  the same in lower case (Rust's, so the SQL needs no Unicode
--                tables): capitals don't make a genre of its own, so this is
--                what a bar counts and what its door filters by
-- Both are NULL for a track with no genre and for a track outside the
-- Library. `genre_key`, the table's sort key, can't stand in: it is the
-- search index's genre (the best file's tags, accents folded), a different
-- definition on edge cases, and it keeps no spelling to show.
--
-- One index, for the Overview's count (it groups by `genre_group` and
-- `genre`, in the order of the index) and for the genre door: a door's rows
-- are the index's entries for one `genre_group`, and the keys a window sorts
-- them by (title, artist, id), the flags and the format are in the entries,
-- like the table's other indexes, so no row of the table is read for it.
--
-- Derived data like the rest of the row: nothing is in the operation log.
-- Every input of the genre (a rekordbox read or relink, a file's tags, the
-- linked file, a merge) already marks the Library track due, because the
-- missing-tags answer of 0042 reads the same inputs, so this migration adds
-- no trigger (`search/tests/genre.rs` proves it, writer by writer).
--
-- After an upgrade the genre of every Library track is unknown. Every Library
-- track is due, so the index job fills it, and the Overview says "still
-- counting" for the genre bars until it has.
ALTER TABLE track_row ADD COLUMN genre TEXT;
ALTER TABLE track_row ADD COLUMN genre_group TEXT;

CREATE INDEX track_row_in_genre
    ON track_row (genre_group, genre, title_key, artist_key, recording_id, flags, format);

INSERT OR IGNORE INTO search_due (recording_id) SELECT recording_id FROM library_track;
