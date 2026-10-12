-- 0026: indexes for the Review queue (1bD-1; ROADMAP 1.12).
--
-- Review counts and lists what is waiting for a decision without reading
-- the whole library: it looks only at the few rows that wait. Each index
-- below is partial, and its WHERE is the very text the read model puts in
-- its own queries (src/review: PENDING_PAIRS, FLAGGED, PROBABLE_RELINK; a
-- test holds the two together), so SQLite can use it. No table changes.

-- The grouping pairs that wait for the user: a version doubt ('open'), a
-- held duplicate set ('set_disagrees') and two Library tracks that are the
-- same recording ('both_in_library'). Almost every pair is something else.
CREATE INDEX grouping_pair_pending ON grouping_pair (state)
    WHERE state IN ('open', 'set_disagrees', 'both_in_library');

-- The files whose stored verdict flags them.
CREATE INDEX file_verdict_flagged ON file_verdict (verdict)
    WHERE verdict IN ('low_bitrate', 'suspect_transcode', 'truncated', 'broken');

-- rekordbox entries matched to a file only probably.
CREATE INDEX rekordbox_track_probable ON rekordbox_track (file_id)
    WHERE relink_probable = 1 AND file_id IS NOT NULL;
