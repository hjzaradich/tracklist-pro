-- 0039: an `analysis` row id is never used twice, so the user's energy can
-- go through the operation log (ROADMAP §2 `analysis`, `operation` /
-- `change`; 1cB-8).
--
-- Keys 1 to 0 set a track's energy (ROADMAP 1.11). The value is the user's
-- own: an `analysis` row with source `user`, which the display already
-- ranks first for energy. Setting it is one undoable operation, so the
-- log records the row it inserts or changes. The scan, the rekordbox
-- attach and later the local estimate insert `analysis` rows outside the
-- log, and without AUTOINCREMENT a new row takes the highest id plus one,
-- which can be the id of a row an operation deleted (the user's energy,
-- taken back by Undo). Undoing an older operation would then meet another
-- source's row under the id it expects and be refused for no reason the
-- user could see. With AUTOINCREMENT a new row's id is higher than any the
-- table has ever held. The same change `sync_base` (0018), `conflict`
-- (0025) and `file` and `recording` (0031) got.
--
-- No operation has recorded an `analysis` row before, so there are no ids
-- in the log to count past.
--
-- The table is rebuilt with every row under its own id, the same columns,
-- checks and unique key, and what depended on it put back: the view
-- `recording_display` goes first (renaming the new table fails while it
-- names `analysis`), the three triggers of 0032 go with the old table.
-- Foreign keys are off while migrations run, and nothing points at
-- `analysis`. One more change: a row that gives no BPM and no key (the
-- user's energy, or another source's) is no reason to index the track
-- again, because search and the track table read BPM and key only.
DROP VIEW recording_display;

CREATE TABLE analysis_new (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    -- RESTRICT: merging tracks moves or drops their analysis on purpose.
    recording_id INTEGER NOT NULL REFERENCES recording (id) ON DELETE RESTRICT,
    -- mik = Mixed In Key; local = the app's own estimate (3.7); ml = the
    -- audio model (3.8).
    source       TEXT    NOT NULL
        CHECK (source IN ('rekordbox', 'tag', 'mik', 'user', 'local', 'ml')),
    bpm          REAL    CHECK (bpm > 0 AND bpm < 1000),
    -- Camelot, normalized before it's stored (tags/key.rs).
    key          TEXT    CHECK (key IN (
        '1A', '2A', '3A', '4A', '5A', '6A', '7A', '8A', '9A', '10A', '11A', '12A',
        '1B', '2B', '3B', '4B', '5B', '6B', '7B', '8B', '9B', '10B', '11B', '12B')),
    energy       INTEGER CHECK (energy BETWEEN 1 AND 10),
    confidence   REAL    CHECK (confidence BETWEEN 0 AND 1),
    analyzed_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- A row says something.
    CHECK (bpm IS NOT NULL OR key IS NOT NULL OR energy IS NOT NULL),
    -- Every value a source can give has a place in the precedence below:
    -- BPM and key come from rekordbox, tags, MIK or the local estimate;
    -- energy from the user, MIK, the local estimate or the audio model.
    CHECK (source NOT IN ('user', 'ml') OR (bpm IS NULL AND key IS NULL)),
    CHECK (source NOT IN ('rekordbox', 'tag') OR energy IS NULL),
    UNIQUE (recording_id, source)
) STRICT;

INSERT INTO analysis_new (id, recording_id, source, bpm, key, energy, confidence, analyzed_at)
SELECT id, recording_id, source, bpm, key, energy, confidence, analyzed_at FROM analysis
ORDER BY id;

DROP TABLE analysis;
ALTER TABLE analysis_new RENAME TO analysis;

-- trigger on `analysis`: a track's BPM or key (rekordbox's, or another
-- source's) was added.
CREATE TRIGGER search_analysis_added
AFTER INSERT ON analysis
WHEN new.bpm IS NOT NULL OR new.key IS NOT NULL
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT new.recording_id WHERE NOT EXISTS (SELECT 1 FROM search_due WHERE recording_id = new.recording_id);
END;

-- trigger on `analysis`
CREATE TRIGGER search_analysis_removed
AFTER DELETE ON analysis
WHEN old.bpm IS NOT NULL OR old.key IS NOT NULL
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT old.recording_id WHERE NOT EXISTS (SELECT 1 FROM search_due WHERE recording_id = old.recording_id);
END;

-- trigger on `analysis`
CREATE TRIGGER search_analysis_changed
AFTER UPDATE OF recording_id, source, bpm, key ON analysis
WHEN old.recording_id IS NOT new.recording_id OR old.source IS NOT new.source
  OR old.bpm IS NOT new.bpm OR old.key IS NOT new.key
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT old.recording_id WHERE NOT EXISTS (SELECT 1 FROM search_due WHERE recording_id = old.recording_id);
    INSERT INTO search_due (recording_id)
    SELECT new.recording_id WHERE NOT EXISTS (SELECT 1 FROM search_due WHERE recording_id = new.recording_id);
END;

-- view on `analysis`, as 0031 declared it
CREATE VIEW recording_display AS
SELECT
    r.id AS recording_id,
    b.bpm, b.source AS bpm_source,
    k.key, k.source AS key_source,
    e.energy, e.source AS energy_source
FROM recording r
LEFT JOIN analysis b ON b.id = (
    SELECT a.id FROM analysis a
    WHERE a.recording_id = r.id AND a.bpm IS NOT NULL
    ORDER BY CASE a.source
        WHEN 'rekordbox' THEN 1 WHEN 'tag' THEN 2 WHEN 'mik' THEN 3 WHEN 'local' THEN 4
    END
    LIMIT 1)
LEFT JOIN analysis k ON k.id = (
    SELECT a.id FROM analysis a
    WHERE a.recording_id = r.id AND a.key IS NOT NULL
    ORDER BY CASE a.source
        WHEN 'rekordbox' THEN 1 WHEN 'tag' THEN 2 WHEN 'mik' THEN 3 WHEN 'local' THEN 4
    END
    LIMIT 1)
LEFT JOIN analysis e ON e.id = (
    SELECT a.id FROM analysis a
    WHERE a.recording_id = r.id AND a.energy IS NOT NULL
    ORDER BY CASE a.source
        WHEN 'user' THEN 1 WHEN 'mik' THEN 2 WHEN 'local' THEN 3 WHEN 'ml' THEN 4
    END
    LIMIT 1);
