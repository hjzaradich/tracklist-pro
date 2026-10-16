-- 0028: what the user said about a relink match, as rows the operation log
-- can record (1bE-6; ROADMAP 1.2, 1.12, §2).
--
-- A probable match of a rekordbox entry to a file waits in Review. The user
-- confirms it or says the file is not the entry's ("Not the same"), and can
-- take either back with Undo. The log names a row by its id, and `relink`
-- was a WITHOUT ROWID table keyed by the Location, so it is rebuilt here as
-- a rowid table, as 0013 rebuilt `crate_entry` and `sync_base`:
--
-- - every row kept, as a confirmation;
-- - a row id for the log, never reused (AUTOINCREMENT, as `sync_base` got in
--   0018): the relink job writes this table outside the log (it fills in a
--   missing `audio_hash` or `file_key`), so nothing may ever take the id of
--   a row an operation deleted and an Undo could put back;
-- - `verdict`: 'same' is a confirmation, as every row was until now;
--   'not_same' is the user's "this file is not that entry".
--
-- **Kept by what the file holds** (as `review_decision` is, 0027): the
-- entry's Location key and `file_key`, the file's `audio_hash`, or its
-- `blake3` when it has none. So a decision survives a move, a rename, a
-- rescan, and the file's music folder removed and added again: a file is a
-- new row then, with the same audio. "Not the same" stands in the way of
-- every file that holds that audio, for that Location; a file encoded again
-- holds other audio, and is another question. (A `blake3` doesn't survive a
-- tag rewrite: a decision about a file with no `audio_hash` is lost with
-- one.)
--
-- `file_id` is only a pointer to the file the user looked at, kept while
-- that row exists: what a confirmation is applied to, and what tells that
-- the confirmed file holds other audio now (`audio_hash`). It never holds
-- the file in place: removing a music folder lets go of it (SET NULL), and
-- a confirmation without its file is applied to a file with its audio.
CREATE TABLE relink_new (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    location_key TEXT    NOT NULL CHECK (location_key <> ''),
    -- What the file held when the user decided; NULL only for a
    -- confirmation made before its file was hashed (the relink job fills it
    -- in once the file is).
    file_key     BLOB    CHECK (file_key IS NULL OR length(file_key) > 0),
    file_id      INTEGER REFERENCES file (id) ON DELETE SET NULL,
    verdict      TEXT    NOT NULL DEFAULT 'same' CHECK (verdict IN ('same', 'not_same')),
    -- How the match the user decided about was found.
    method       TEXT    NOT NULL
        CHECK (method IN ('path', 'filename_duration', 'unique_duration',
                          'fingerprint', 'filename_only', 'gig_stick', 'user')),
    confidence   REAL    CHECK (confidence BETWEEN 0 AND 1),
    -- A confirmation: the file's `audio_hash` when the user confirmed it,
    -- only if that was current for the file.
    audio_hash   BLOB,
    confirmed_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- "Not the same" is always about known audio.
    CHECK (verdict = 'same' OR file_key IS NOT NULL)
) STRICT;

INSERT INTO relink_new
    (location_key, file_key, file_id, method, confidence, audio_hash, confirmed_at)
SELECT r.location_key,
       (SELECT coalesce(nullif(f.audio_hash, x''), f.blake3) FROM file f WHERE f.id = r.file_id),
       r.file_id, r.method, r.confidence, r.audio_hash, r.confirmed_at
FROM relink r
ORDER BY r.location_key;

DROP TABLE relink;
ALTER TABLE relink_new RENAME TO relink;

CREATE INDEX relink_by_file ON relink (file_id);

-- A Location has one confirmation, as it always had.
CREATE UNIQUE INDEX relink_one_confirmation ON relink (location_key) WHERE verdict = 'same';

-- And says "not the same" about a piece of audio once.
CREATE UNIQUE INDEX relink_one_rejection ON relink (location_key, file_key)
    WHERE verdict = 'not_same';
