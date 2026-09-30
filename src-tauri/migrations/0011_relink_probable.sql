-- 0011: a relink match that isn't trusted yet (ROADMAP 1.2, §2).
--
-- rekordbox_track.relink_probable = 1: the row has a file, but the match
-- is only probable (a filename-only match, or a unique duration no title
-- tag agrees with). No rekordbox data is attached to the file's track
-- until the user confirms it in Review; a confirmation is a `relink` row,
-- which each relink re-applies with relink_probable = 0. A probable
-- match's file still counts as taken, so no other rekordbox track gets it.
ALTER TABLE rekordbox_track ADD COLUMN relink_probable INTEGER NOT NULL DEFAULT 0
    CHECK (relink_probable IN (0, 1) AND (relink_probable = 0 OR file_id IS NOT NULL));
