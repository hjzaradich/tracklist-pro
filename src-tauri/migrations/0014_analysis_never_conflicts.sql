-- 0014: analysis fields never become conflicts (ROADMAP 1.9 rule 3, 1.10).
--
-- BPM, key, the beat grid and cues are rekordbox's: the app never edits
-- them and never sends its own, so there's no app-side change for a
-- conflict to hold. `field` is a rekordbox XML name (as in `sync_base`):
-- the `AverageBpm` and `Tonality` attributes, and the `TEMPO` and
-- `POSITION_MARK` elements.
--
-- A migration that rebuilds `conflict` (create new, copy, drop, rename)
-- drops these triggers with the old table and must create them again.

-- No code has made one; any that exists is dropped so the rule holds for
-- every row.
DELETE FROM conflict
WHERE field IN ('AverageBpm', 'Tonality', 'TEMPO', 'POSITION_MARK');

CREATE TRIGGER conflict_never_on_an_analysis_field_insert
BEFORE INSERT ON conflict
WHEN new.field IN ('AverageBpm', 'Tonality', 'TEMPO', 'POSITION_MARK')
BEGIN
    SELECT RAISE(ABORT, 'analysis fields never become conflicts');
END;

CREATE TRIGGER conflict_never_on_an_analysis_field_update
BEFORE UPDATE OF field ON conflict
WHEN new.field IN ('AverageBpm', 'Tonality', 'TEMPO', 'POSITION_MARK')
BEGIN
    SELECT RAISE(ABORT, 'analysis fields never become conflicts');
END;
