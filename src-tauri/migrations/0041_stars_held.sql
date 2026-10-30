-- 0041: a Library track whose stars the user chose to keep rekordbox's
-- (ROADMAP 1.10, "Star change while energy is sent as stars"; 1cC-2).
--
-- While the user has energy sent as the star rating, keeping rekordbox's
-- 0 stars in a conflict clears the user's own energy. A track can still show
-- another source's energy (Mixed In Key's, the estimate, the model's); with
-- nothing else said, that energy would bring the stars back and the conflict
-- with them. This marker remembers the choice: a marked track's stars stay
-- rekordbox's, whatever energy it shows. It ends when the user sets an
-- energy on the track.
--
-- It is Library data (one column of the track, written through the operation
-- log in the resolve's own operation and cleared in the energy edit's), so
-- Undo takes it back with the change that set it, and a track removed from
-- the Library takes it along. The default is not marked.
ALTER TABLE library_track
    ADD COLUMN stars_held INTEGER NOT NULL DEFAULT 0
        CHECK (stars_held IN (0, 1));
