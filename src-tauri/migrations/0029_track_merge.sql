-- 0029: two Library tracks that are the same recording, merged into one by
-- the user in Review (1bE-9; ROADMAP 1.4, 1.12, 1.9 rule 7, §2).
--
-- The merge moves the files of the track that leaves into the track that
-- stays, re-points or drops its crate entries and deletes its Library track,
-- all as one operation of the log, so Undo puts everything back. The track
-- that leaves is not deleted: its row is kept, with no files, while the
-- merge stands, so Undo has a track to move its files back into (with its
-- own analysis rows and links, which the merge never touches). A track with
-- no files is shown nowhere (All music, the Library and search list tracks
-- through their files).
--
-- `recording_id` is the track that left: a foreign key, so nothing deletes
-- it while the merge stands (grouping's sweep reads this table too).
-- `into_id` is the track it was merged into, not a foreign key: it is what
-- the merge did, and whatever later becomes of that track (removed from the
-- Library, merged again by grouping) the record stays as it was.
--
-- Made in the merge's own operation and deleted by its Undo, nowhere else,
-- so its ids are only ever made through the log.
CREATE TABLE track_merge (
    id           INTEGER PRIMARY KEY,
    recording_id INTEGER NOT NULL UNIQUE REFERENCES recording (id) ON DELETE RESTRICT,
    into_id      INTEGER NOT NULL,
    merged_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (into_id <> recording_id)
) STRICT;

-- What a track something was merged into reads (attach: its rekordbox BPM
-- and key come from the entry it is sent as only).
CREATE INDEX track_merge_into ON track_merge (into_id);

-- Why a rekordbox entry was left behind (0027): 'switch', a Library track's
-- gone linked file was switched to a copy; 'merge', the Library track it
-- was sent as was merged into another. Every row before this one is a
-- switch's.
ALTER TABLE replaced_entry ADD COLUMN reason TEXT NOT NULL DEFAULT 'switch'
    CHECK (reason IN ('switch', 'merge'));
