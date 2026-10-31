-- 0043: two more answers in `track_row.flags`, for the Overview's doors
-- "from a low-quality file", "No energy" and "Energy 1" to "Energy 10"
-- (1cD-R1a; ROADMAP 1.13 "Every number is a door").
--       8  the Library track's linked file has a current verdict that flags
--          it (free since 0033)
--  256..  bits 8 to 11 hold the energy the Library track shows, 1 to 10, or
--          0 for none
-- Both are worked out by the Overview's own definitions (`overview::
-- door_answers_of`) in the index job, in the same transaction as the rest of
-- the row, and are set on Library tracks' rows only, as the two gap flags of
-- 0042 are. The energy is in the flags, and not in a column of its own,
-- because `flags` is in every index of the table: a door's window, its count
-- and a track's position are read from an index alone, and no index is built
-- again by this migration.
--
-- Derived data like the rest of the row: nothing is in the operation log, no
-- column changes, and every other reader of a track's energy (the crate
-- summary, energy as stars) goes on reading `analysis`.
--
-- After an upgrade the new bits of every Library track are unknown. Every
-- Library track is due, so the index job fills them, and the Overview says
-- "still counting" until it has.
INSERT OR IGNORE INTO search_due (recording_id) SELECT recording_id FROM library_track;

-- Why the verdict triggers of 0032 can come back, after 0033 dropped them.
-- 0033 dropped them because their flag had no reader left: the Quality
-- problem chip reads Review's definition when the query runs, and a stored
-- bit could not follow that definition (a Keep anyway, a better copy, a
-- verdict gone stale with no row written). They also marked a track for
-- every verdict the quality job wrote. Here:
--   - the stored answer has one definition, a current flagged verdict, so
--     the triggers below watch all it reads: the verdict, and everything that
--     can make a verdict stale (`quality::store::INPUTS`), which is the
--     file's facts and its measurement. The one input no row holds is
--     `RULES_VERSION`: `quality::due` asks for the quality job after a new
--     version, and the job's new verdicts mark the tracks here.
--   - only a verdict that may flag marks anything. The answer is 1 only with
--     such a verdict, so on a file with none (`ok`, none yet, or no row) the
--     answer is 0 before and after: a healthy library's quality job, and the
--     read and hash stages, mark nothing. "May flag" is any verdict but `ok`
--     and none: a wider test than the Overview's list, so it can't miss one.
--   - only a Library track's row holds the answer, so only its linked file
--     marks anything.

-- A file's verdict was made, changed or dropped.
CREATE TRIGGER search_low_quality_verdict_added
AFTER INSERT ON file_verdict
WHEN coalesce(new.verdict, 'ok') <> 'ok'
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.linked_file_id = new.file_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;

CREATE TRIGGER search_low_quality_verdict_changed
AFTER UPDATE OF verdict, rules_version, inputs ON file_verdict
WHEN coalesce(old.verdict, 'ok') <> 'ok' OR coalesce(new.verdict, 'ok') <> 'ok'
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.linked_file_id = new.file_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;

CREATE TRIGGER search_low_quality_verdict_removed
AFTER DELETE ON file_verdict
WHEN coalesce(old.verdict, 'ok') <> 'ok'
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.linked_file_id = old.file_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;

-- A fact a verdict was made from changed, so the verdict is stale.
CREATE TRIGGER search_low_quality_file_changed
AFTER UPDATE OF audio_hash, codec, bitrate, sample_rate, quality_verdict ON file
WHEN (old.audio_hash IS NOT new.audio_hash OR old.codec IS NOT new.codec
      OR old.bitrate IS NOT new.bitrate OR old.sample_rate IS NOT new.sample_rate
      OR old.quality_verdict IS NOT new.quality_verdict)
  AND EXISTS (SELECT 1 FROM file_verdict v
              WHERE v.file_id = new.id AND coalesce(v.verdict, 'ok') <> 'ok')
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.linked_file_id = new.id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;

-- The measurement a verdict was made from was taken, replaced or dropped.
CREATE TRIGGER search_low_quality_measured
AFTER INSERT ON file_quality
WHEN EXISTS (SELECT 1 FROM file_verdict v
             WHERE v.file_id = new.file_id AND coalesce(v.verdict, 'ok') <> 'ok')
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.linked_file_id = new.file_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;

CREATE TRIGGER search_low_quality_measured_again
AFTER UPDATE ON file_quality
WHEN EXISTS (SELECT 1 FROM file_verdict v
             WHERE v.file_id = new.file_id AND coalesce(v.verdict, 'ok') <> 'ok')
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.linked_file_id = new.file_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;

CREATE TRIGGER search_low_quality_measurement_removed
AFTER DELETE ON file_quality
WHEN EXISTS (SELECT 1 FROM file_verdict v
             WHERE v.file_id = old.file_id AND coalesce(v.verdict, 'ok') <> 'ok')
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.linked_file_id = old.file_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;

-- A Library track's energy was given, changed or taken away. (A row that
-- moves to another track or source is `search_analysis_changed` of 0039.)
-- A track outside the Library has no stored energy, so it marks nothing;
-- joining the Library marks it.
CREATE TRIGGER search_analysis_energy_added
AFTER INSERT ON analysis
WHEN new.energy IS NOT NULL
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.recording_id = new.recording_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;

CREATE TRIGGER search_analysis_energy_removed
AFTER DELETE ON analysis
WHEN old.energy IS NOT NULL
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.recording_id = old.recording_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;

CREATE TRIGGER search_analysis_energy_changed
AFTER UPDATE OF energy ON analysis
WHEN old.energy IS NOT new.energy
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.recording_id = new.recording_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;
