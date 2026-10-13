-- 0028: what the user said about a relink match, as rows the operation log
-- can record (1bE-6; ROADMAP 1.2, 1.12, §2).
--
-- A probable match of a rekordbox entry to a file waits in Review. The user
-- confirms it or says the file is not the entry's ("Not the same"), and can
-- take either back with Undo. The log names a row by its id, and `relink`
-- was a WITHOUT ROWID table keyed by the Location, so it is rebuilt here as
-- a rowid table, as 0013 rebuilt `crate_entry` and `sync_base`:
--
-- - the same columns, checks and foreign key, every row kept;
-- - a row id for the log, never reused (AUTOINCREMENT, as `sync_base` got in
--   0018): the relink job writes this table outside the log (it fills in a
--   missing `audio_hash`), so nothing may ever take the id of a row an
--   operation deleted and an Undo could put back;
-- - `verdict`: 'same' is a confirmation, as every row was until now;
--   'not_same' is the user's "this file is not that entry", which relink
--   then never matches the two on again (steps 2 to 5).
--
-- Both are kept by the entry's Location key and the file, with the file's
-- `audio_hash` when the user decided, as a confirmation always was. The
-- file's row can't go while a row here names it (RESTRICT), so its id is
-- never another file's. A file that holds other audio later is another
-- question: a confirmation then asks again, and a "not the same" no longer
-- stands in the way.
CREATE TABLE relink_new (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    location_key TEXT    NOT NULL CHECK (location_key <> ''),
    file_id      INTEGER NOT NULL REFERENCES file (id) ON DELETE RESTRICT,
    verdict      TEXT    NOT NULL DEFAULT 'same' CHECK (verdict IN ('same', 'not_same')),
    -- How the match the user decided about was found.
    method       TEXT    NOT NULL
        CHECK (method IN ('path', 'filename_duration', 'unique_duration',
                          'fingerprint', 'filename_only', 'gig_stick', 'user')),
    confidence   REAL    CHECK (confidence BETWEEN 0 AND 1),
    audio_hash   BLOB,
    confirmed_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

INSERT INTO relink_new (location_key, file_id, method, confidence, audio_hash, confirmed_at)
SELECT location_key, file_id, method, confidence, audio_hash, confirmed_at FROM relink
ORDER BY location_key;

DROP TABLE relink;
ALTER TABLE relink_new RENAME TO relink;

CREATE INDEX relink_by_file ON relink (file_id);

-- A Location has one confirmed file, as it always had.
CREATE UNIQUE INDEX relink_one_confirmation ON relink (location_key) WHERE verdict = 'same';

-- And says "not the same" about a file once.
CREATE UNIQUE INDEX relink_one_rejection ON relink (location_key, file_id)
    WHERE verdict = 'not_same';
