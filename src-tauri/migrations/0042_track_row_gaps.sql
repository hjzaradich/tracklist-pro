-- 0042: two more flags in `track_row.flags`, for the Overview's "tracks with
-- missing tags" and "tracks with missing art" (1cC-U1b; ROADMAP 1.13 "Every
-- number is a door").
--      64  the Library track lacks a title, an artist or a genre
--     128  the Library track's linked file holds tags and no cover picture
-- Both are worked out by the Overview's own definition (`overview::gaps_of`)
-- in Rust, by the index job, in the same transaction as the rest of the row,
-- because the definition reads tag data per track (the title and artist rules
-- of a send, the genre of rekordbox's entry or the file's tag) and cannot be
-- a cheap table filter. The Overview then counts the bits instead of reading
-- every track's tags, and the Library's two new filters read them, so a
-- number and the rows behind it are one definition.
--
-- Derived data like the rest of the row: nothing is stored that the files,
-- their tags and rekordbox's snapshot don't give, and nothing is in the
-- operation log. No column changes; `flags` is already an integer.
--
-- After an upgrade the bits of every Library track are unknown. Every
-- Library track is due, so the index job (queued at startup, as for any due
-- track) fills them, and the Overview says "still counting" until it has.
-- (A row whose bits come out as none is the row already stored, so most are
-- not even written.)
INSERT OR IGNORE INTO search_due (recording_id) SELECT recording_id FROM library_track;

-- What a track's missing tags read besides what the triggers of 0024 and
-- 0032 already watch (its tags, files, links, rekordbox's snapshot): the
-- merge that decides which of rekordbox's entries a track may take its genre
-- from (`attach::PICKABLE`).
CREATE TRIGGER search_track_merge_added
AFTER INSERT ON track_merge
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT new.into_id WHERE NOT EXISTS (SELECT 1 FROM search_due WHERE recording_id = new.into_id);
END;

CREATE TRIGGER search_track_merge_removed
AFTER DELETE ON track_merge
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT old.into_id WHERE NOT EXISTS (SELECT 1 FROM search_due WHERE recording_id = old.into_id);
END;
