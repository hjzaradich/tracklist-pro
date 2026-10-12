-- 0023: which file of a track is its best, kept up to date (ROADMAP 1.7;
-- 1bC-4).
--
-- `recording_file.role` is the ranking's answer (`best_file::rank`): the
-- best-ranked file is `best`, every other file `extra`. It is derived
-- state, worked out in the background for the tracks that need it. This
-- migration adds the two tables that job needs and the triggers that tell
-- it which tracks to look at.

-- The tracks whose files must be ranked again. A row is added by a trigger
-- below whenever something the ranking reads changes, and removed by the
-- job in the same transaction as the roles it writes. No foreign key: the
-- id of a track that has since been deleted is still cleared away by the
-- job. Nothing here fires on `recording_file.role`, so the job's own
-- writes never make a track due.
CREATE TABLE best_file_due (
    recording_id INTEGER PRIMARY KEY
) STRICT;

-- "A better file is available" (ROADMAP 1.7): for a track in the Library,
-- the file that is ahead of the linked file in what the audio is (or in
-- being there at all), and the step of the ranking that put it ahead.
-- Only those tracks have a row. Written by the same job; never touches
-- `library_track.linked_file_id`, which is pinned. A more complete tag
-- block, art, a rekordbox link or a better place do not make a file better
-- (tags and art are picked per field), so those steps are not allowed here.
CREATE TABLE better_file (
    recording_id INTEGER PRIMARY KEY REFERENCES recording (id) ON DELETE CASCADE,
    file_id      INTEGER NOT NULL REFERENCES file (id) ON DELETE CASCADE,
    step         TEXT    NOT NULL
        CHECK (step IN ('onDisk', 'verdict', 'playable', 'cutoff', 'decodeErrors', 'format'))
) STRICT;

-- Deleting a file looks its row up here.
CREATE INDEX better_file_by_file ON better_file (file_id);

-- ---- what makes a track due ------------------------------------------------

-- Every trigger adds a track with `ON CONFLICT DO NOTHING` (a track is due
-- once, however many things changed), not `INSERT OR IGNORE`: SQLite lets
-- the outer statement's own conflict clause override a trigger's `OR
-- IGNORE`, and the statements that fire these (an upsert of a file's
-- track) have one.
--
-- `recursive_triggers` is off, so an `INSERT OR REPLACE` into
-- `recording_file`, `rekordbox_track`, `file_quality`, `file_verdict` or
-- `library_track` would not fire the DELETE triggers for the row it
-- replaces. Don't use REPLACE on these tables (nothing does).

-- Files joined, left or moved between tracks (a grouping pass).
CREATE TRIGGER best_file_due_file_placed
AFTER INSERT ON recording_file
BEGIN
    INSERT INTO best_file_due (recording_id) VALUES (new.recording_id) ON CONFLICT DO NOTHING;
END;

CREATE TRIGGER best_file_due_file_released
AFTER DELETE ON recording_file
BEGIN
    INSERT INTO best_file_due (recording_id) VALUES (old.recording_id) ON CONFLICT DO NOTHING;
END;

CREATE TRIGGER best_file_due_file_moved
AFTER UPDATE OF recording_id ON recording_file
WHEN old.recording_id IS NOT new.recording_id
BEGIN
    INSERT INTO best_file_due (recording_id) VALUES (old.recording_id) ON CONFLICT DO NOTHING;
    INSERT INTO best_file_due (recording_id) VALUES (new.recording_id) ON CONFLICT DO NOTHING;
END;

-- A file left or came back (`present`), was read again (format, tags,
-- art), got other audio (`audio_hash`, which the stored cutoff and verdict
-- are valid for), or moved to another music folder.
CREATE TRIGGER best_file_due_file_changed
AFTER UPDATE OF present, sniffed_format, codec, bitrate, sample_rate, audio_hash,
                quality_verdict, raw_tags, music_folder_id ON file
WHEN old.present IS NOT new.present
  OR old.sniffed_format IS NOT new.sniffed_format
  OR old.codec IS NOT new.codec
  OR old.bitrate IS NOT new.bitrate
  OR old.sample_rate IS NOT new.sample_rate
  OR old.audio_hash IS NOT new.audio_hash
  OR old.quality_verdict IS NOT new.quality_verdict
  OR old.raw_tags IS NOT new.raw_tags
  OR old.music_folder_id IS NOT new.music_folder_id
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT recording_id FROM recording_file WHERE file_id = new.id
    ON CONFLICT DO NOTHING;
END;

-- A measurement landed, changed or went (the cutoff, and what the quality
-- verdict is made from).
CREATE TRIGGER best_file_due_quality_measured
AFTER INSERT ON file_quality
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT recording_id FROM recording_file WHERE file_id = new.file_id
    ON CONFLICT DO NOTHING;
END;

CREATE TRIGGER best_file_due_quality_dropped
AFTER DELETE ON file_quality
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT recording_id FROM recording_file WHERE file_id = old.file_id
    ON CONFLICT DO NOTHING;
END;

CREATE TRIGGER best_file_due_quality_changed
AFTER UPDATE OF audio_hash, method_version, cutoff_hz, cutoff_gap, decoded_ms, header_ms,
                decode_errors, error_kinds, ended, failure ON file_quality
WHEN old.audio_hash IS NOT new.audio_hash
  OR old.method_version IS NOT new.method_version
  OR old.cutoff_hz IS NOT new.cutoff_hz
  OR old.cutoff_gap IS NOT new.cutoff_gap
  OR old.decoded_ms IS NOT new.decoded_ms
  OR old.header_ms IS NOT new.header_ms
  OR old.decode_errors IS NOT new.decode_errors
  OR old.error_kinds IS NOT new.error_kinds
  OR old.ended IS NOT new.ended
  OR old.failure IS NOT new.failure
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT recording_id FROM recording_file WHERE file_id = new.file_id
    ON CONFLICT DO NOTHING;
END;

-- A stored verdict settled to something else. A row with no verdict reads
-- the same as no row, so only a row that says something counts.
CREATE TRIGGER best_file_due_verdict_stored
AFTER INSERT ON file_verdict
WHEN new.verdict IS NOT NULL
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT recording_id FROM recording_file WHERE file_id = new.file_id
    ON CONFLICT DO NOTHING;
END;

CREATE TRIGGER best_file_due_verdict_dropped
AFTER DELETE ON file_verdict
WHEN old.verdict IS NOT NULL
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT recording_id FROM recording_file WHERE file_id = old.file_id
    ON CONFLICT DO NOTHING;
END;

CREATE TRIGGER best_file_due_verdict_changed
AFTER UPDATE OF verdict, findings ON file_verdict
WHEN old.verdict IS NOT new.verdict OR old.findings IS NOT new.findings
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT recording_id FROM recording_file WHERE file_id = new.file_id
    ON CONFLICT DO NOTHING;
END;

-- rekordbox's match to a file was made, dropped, or turned trusted or
-- probable. A fresh read of rekordbox replaces the rows, so every matched
-- track is ranked again then (the result is the same; nothing is written).
CREATE TRIGGER best_file_due_match_made
AFTER INSERT ON rekordbox_track
WHEN new.file_id IS NOT NULL
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT recording_id FROM recording_file WHERE file_id = new.file_id
    ON CONFLICT DO NOTHING;
END;

CREATE TRIGGER best_file_due_match_dropped
AFTER DELETE ON rekordbox_track
WHEN old.file_id IS NOT NULL
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT recording_id FROM recording_file WHERE file_id = old.file_id
    ON CONFLICT DO NOTHING;
END;

CREATE TRIGGER best_file_due_match_changed
AFTER UPDATE OF file_id, relink_probable ON rekordbox_track
WHEN old.file_id IS NOT new.file_id OR old.relink_probable IS NOT new.relink_probable
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT recording_id FROM recording_file
    WHERE file_id = old.file_id OR file_id = new.file_id
    ON CONFLICT DO NOTHING;
END;

-- A track entered the Library, left it, or was linked to another file: its
-- "better file" is worked out again.
CREATE TRIGGER best_file_due_library_added
AFTER INSERT ON library_track
BEGIN
    INSERT INTO best_file_due (recording_id) VALUES (new.recording_id) ON CONFLICT DO NOTHING;
END;

CREATE TRIGGER best_file_due_library_removed
AFTER DELETE ON library_track
BEGIN
    INSERT INTO best_file_due (recording_id) VALUES (old.recording_id) ON CONFLICT DO NOTHING;
END;

CREATE TRIGGER best_file_due_library_relinked
AFTER UPDATE OF linked_file_id, recording_id ON library_track
WHEN old.linked_file_id IS NOT new.linked_file_id OR old.recording_id IS NOT new.recording_id
BEGIN
    INSERT INTO best_file_due (recording_id) VALUES (old.recording_id) ON CONFLICT DO NOTHING;
    INSERT INTO best_file_due (recording_id) VALUES (new.recording_id) ON CONFLICT DO NOTHING;
END;

-- A music folder changed role or volume, or a volume changed kind: the
-- "place" of every file in it.
CREATE TRIGGER best_file_due_folder_changed
AFTER UPDATE OF role, volume_id ON music_folder
WHEN old.role IS NOT new.role OR old.volume_id IS NOT new.volume_id
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT rf.recording_id
    FROM file f JOIN recording_file rf ON rf.file_id = f.id
    WHERE f.music_folder_id = new.id
    ON CONFLICT DO NOTHING;
END;

CREATE TRIGGER best_file_due_volume_changed
AFTER UPDATE OF kind ON volume
WHEN old.kind IS NOT new.kind
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT rf.recording_id
    FROM music_folder mf
    JOIN file f ON f.music_folder_id = mf.id
    JOIN recording_file rf ON rf.file_id = f.id
    WHERE mf.volume_id = new.id
    ON CONFLICT DO NOTHING;
END;

-- Every track that exists is ranked once.
INSERT OR IGNORE INTO best_file_due (recording_id) SELECT id FROM recording;
