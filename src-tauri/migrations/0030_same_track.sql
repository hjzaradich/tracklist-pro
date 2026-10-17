-- 0030: "These are the same track" in Review (1bE-2; ROADMAP 1.4, 1.12, §2).
--
-- Enter on a possible version mix-up or on "Unclear duplicates" says the
-- tracks are one track: the files of the tracks outside the Library move
-- into the Library track, as one operation of the log, so Undo moves them
-- back. As with a merge of two Library tracks (0029), each track that lost
-- its files is kept, with none, while the decision stands, so Undo has a
-- track to move them back into: a `track_merge` row of kind 'same'.
--
-- `kind` says which action left the track: 'library', a Library track merged
-- into another ("Same track twice in your Library"; every row before this
-- one), or 'same', a track outside the Library that the user called the same
-- as a Library track. Only a 'library' merge makes the staying track keep its
-- own rekordbox BPM and key (attach); after 'same' the tracks are one as if
-- the grouping job had merged them.
ALTER TABLE track_merge ADD COLUMN kind TEXT NOT NULL DEFAULT 'library'
    CHECK (kind IN ('library', 'same'));
