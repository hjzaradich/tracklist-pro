-- 0031: a file's or a track's row id is never used twice (ROADMAP §2
-- `operation` / `change`, `file`, `recording`; 1bE).
--
-- The operation log names rows by id, and some of the values it records
-- are ids of `file` and `recording` rows: a Library track's linked and
-- source file, the files a merge, a split or "same track" moved
-- (`recording_file` is named by its file's id), a relink decision's file,
-- the track a removal, a merge, a split or a replaced entry is about.
-- Neither table is written through the log: every scan makes `file` rows,
-- removing a music folder deletes them, and the grouping job (and a split)
-- makes `recording` rows, which the grouping job sweeps. Without
-- AUTOINCREMENT a new row takes the highest id plus one, which can be the
-- id of a row deleted since an operation recorded it. Undoing that
-- operation then acts on the new file or track, believing nothing changed
-- (a switch's Undo links the track to an unrelated file; a merge's Undo
-- moves a new file out of the track that stayed). With AUTOINCREMENT a new
-- row's id is higher than any the table has ever held, so a recorded id
-- names its own row or none, and an Undo whose row is gone is refused.
-- The same change `sync_base` (0018), `conflict` (0025) and `relink`
-- (0028) got, for the ids the log records as values.
--
-- Both tables are rebuilt with every row under its own id, the same
-- columns, checks and foreign keys, and every index, trigger and view put
-- back as it was declared. Foreign keys are off while migrations run, so
-- dropping the old tables deletes nothing that points at them, and the
-- runner checks every reference before this commits. It is one
-- transaction: if any step fails, the database stays as it was.
--
-- The copy rewrites the whole `file` table once (about 20 s for 100,000
-- files); the space the old table held stays in the database file and is
-- reused.
--
-- "Ever held" includes rows deleted before this migration ran: an older
-- operation may still name them, waiting for an undo. So the count SQLite
-- keeps for each table (`sqlite_sequence`) is raised past every id of it
-- the log names (step 5).

-- 1. What names the two tables from elsewhere goes for now: with it in
--    place, renaming the new tables fails ("no such table: main.file").
DROP VIEW recording_display;
DROP TRIGGER library_track_status_follows_new_link_update;
DROP TRIGGER library_track_status_follows_new_link_insert;
DROP TRIGGER best_file_due_folder_changed;
DROP TRIGGER best_file_due_volume_changed;
DROP TRIGGER best_file_due_dismissal_made;
DROP TRIGGER best_file_due_dismissal_taken_back;

-- 2. `file`, rebuilt: its definition exactly as SQLite holds it now
--    (columns added by later migrations included), with AUTOINCREMENT.
CREATE TABLE file_new (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    music_folder_id INTEGER NOT NULL REFERENCES music_folder (id) ON DELETE RESTRICT,
    -- From the music folder, in the exact on-disk spelling (used to open
    -- the file). Never empty: that would be the folder itself.
    rel_path        TEXT    NOT NULL
        CHECK (
            rel_path <> '' AND instr(rel_path, '\') = 0 AND instr(rel_path, ':') = 0
            AND instr('/' || rel_path || '/', '//') = 0
            AND instr('/' || rel_path || '/', '/./') = 0
            AND instr('/' || rel_path || '/', '/../') = 0
        ),
    -- rel_path in NFC, for comparing and matching rekordbox Locations (§0.3).
    rel_path_key    TEXT    NOT NULL
        CHECK (
            rel_path_key <> '' AND instr(rel_path_key, '\') = 0 AND instr(rel_path_key, ':') = 0
            AND instr('/' || rel_path_key || '/', '//') = 0
            AND instr('/' || rel_path_key || '/', '/./') = 0
            AND instr('/' || rel_path_key || '/', '/../') = 0
        ),
    -- Stage 1 (stat). Size is for the unchanged check only: never match or
    -- relink on it (§5.3).
    size            INTEGER CHECK (size >= 0),
    -- Nanoseconds since the Unix epoch, as the filesystem reports it.
    mtime           INTEGER,
    -- Stage 3 (hashes). blake3 covers the whole file; audio_hash only the
    -- audio frames, since rekordbox rewrites tags (§5.1).
    blake3          BLOB    CHECK (blake3 IS NULL OR length(blake3) = 32),
    audio_hash      BLOB,
    fingerprint     BLOB,
    -- Stage 2 (tags and audio properties). The format comes from the bytes,
    -- never the extension.
    sniffed_format  TEXT,
    codec           TEXT,
    bitrate         INTEGER CHECK (bitrate > 0),
    sample_rate     INTEGER CHECK (sample_rate > 0),
    duration_ms     INTEGER CHECK (duration_ms >= 0),
    quality_verdict TEXT
        CHECK (quality_verdict IN
            ('ok', 'low_bitrate', 'suspect_transcode', 'clipped', 'truncated', 'broken')),
    -- Every tag as read, as a JSON object.
    raw_tags        TEXT
        CHECK (raw_tags IS NULL OR (json_valid(raw_tags) AND json_type(raw_tags) = 'object')),
    present         INTEGER NOT NULL DEFAULT 1 CHECK (present IN (0, 1)),
    hidden_reason   TEXT    CHECK (hidden_reason IN ('user', 'inbox_cleared')),
    first_seen_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_seen_at    TEXT, file_id TEXT
    CHECK (
        file_id IS NULL OR (
            length(file_id) = 49
            AND substr(file_id, 17, 1) = '-'
            AND NOT ((substr(file_id, 1, 16) || substr(file_id, 18)) GLOB '*[^0-9a-f]*')
        )
    ), online_only INTEGER NOT NULL DEFAULT 0
    CHECK (online_only IN (0, 1)), partial_hash BLOB
    CHECK (partial_hash IS NULL OR length(partial_hash) = 33), fingerprint_audio_hash BLOB
    CHECK (fingerprint_audio_hash IS NULL OR length(fingerprint_audio_hash) = 34), fingerprint_rev INTEGER NOT NULL DEFAULT 0,
    UNIQUE (music_folder_id, rel_path)
) STRICT;

INSERT INTO file_new (
    id, music_folder_id, rel_path, rel_path_key, size, mtime,
    blake3, audio_hash, fingerprint, sniffed_format, codec, bitrate,
    sample_rate, duration_ms, quality_verdict, raw_tags, present, hidden_reason,
    first_seen_at, last_seen_at, file_id, online_only, partial_hash, fingerprint_audio_hash,
    fingerprint_rev)
SELECT
    id, music_folder_id, rel_path, rel_path_key, size, mtime,
    blake3, audio_hash, fingerprint, sniffed_format, codec, bitrate,
    sample_rate, duration_ms, quality_verdict, raw_tags, present, hidden_reason,
    first_seen_at, last_seen_at, file_id, online_only, partial_hash, fingerprint_audio_hash,
    fingerprint_rev
FROM file
ORDER BY id;

DROP TABLE file;
ALTER TABLE file_new RENAME TO file;

-- 3. `recording`, rebuilt: its definition exactly as SQLite holds it now
--    (columns added by later migrations included), with AUTOINCREMENT.
CREATE TABLE recording_new (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    -- Canonical tags: the best value gathered from the track's files and
    -- rekordbox (1.7).
    title      TEXT,
    artist     TEXT,
    version    TEXT,
    remixers   TEXT,
    label      TEXT,
    year       INTEGER CHECK (year BETWEEN 1000 AND 9999),
    -- A plain canonical tag until the genre tree arrives in Phase 3 (§2).
    genre      TEXT,
    -- Where each canonical value came from and how sure that is, as a JSON
    -- object keyed by field, e.g.
    -- {"title": {"source": "rekordbox", "confidence": 1.0}}.
    provenance TEXT    NOT NULL DEFAULT '{}'
        CHECK (json_valid(provenance) AND json_type(provenance) = 'object'),
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    -- No bpm, key or energy columns: see recording_display.
) STRICT;

INSERT INTO recording_new (
    id, title, artist, version, remixers, label,
    year, genre, provenance, created_at)
SELECT
    id, title, artist, version, remixers, label,
    year, genre, provenance, created_at
FROM recording
ORDER BY id;

DROP TABLE recording;
ALTER TABLE recording_new RENAME TO recording;

-- 4. Everything that went with the old tables, and what step 1 dropped,
--    exactly as it was declared.
-- index on `file`
CREATE INDEX file_by_rel_path_key ON file (rel_path_key);

-- index on `file`
CREATE INDEX file_by_blake3 ON file (blake3) WHERE blake3 IS NOT NULL;

-- index on `file`
CREATE INDEX file_by_audio_hash ON file (audio_hash) WHERE audio_hash IS NOT NULL;

-- trigger on `file`
CREATE TRIGGER library_track_status_follows_file
AFTER UPDATE OF present ON file
BEGIN
    UPDATE library_track
    SET source_status = CASE WHEN new.present = 1 THEN 'ok' ELSE 'missing' END
    WHERE kind = 'linked' AND linked_file_id = new.id
      AND source_status <> CASE WHEN new.present = 1 THEN 'ok' ELSE 'missing' END;
END;

-- trigger on `file`
CREATE TRIGGER fingerprint_match_goes_with_its_fingerprint
AFTER UPDATE OF fingerprint ON file
WHEN old.fingerprint IS NOT new.fingerprint
BEGIN
    DELETE FROM fingerprint_match WHERE file_a = old.id OR file_b = old.id;
END;

-- trigger on `file`
CREATE TRIGGER fingerprint_matched_goes_with_its_fingerprint
AFTER UPDATE OF fingerprint ON file
WHEN old.fingerprint IS NOT new.fingerprint
BEGIN
    DELETE FROM fingerprint_matched WHERE file_id = old.id;
END;

-- trigger on `file`
CREATE TRIGGER fingerprint_matched_goes_when_a_missing_file_comes_back
AFTER UPDATE OF present ON file
WHEN old.present = 0 AND new.present = 1
BEGIN
    DELETE FROM fingerprint_matched WHERE file_id = old.id;
END;

-- trigger on `file`
CREATE TRIGGER file_fingerprint_rev_moves_with_its_fingerprint
AFTER UPDATE OF fingerprint ON file
WHEN old.fingerprint IS NOT new.fingerprint
BEGIN
    UPDATE file SET fingerprint_rev = random() | 1 WHERE id = new.id;
END;

-- trigger on `file`
CREATE TRIGGER file_fingerprint_rev_starts_with_its_fingerprint
AFTER INSERT ON file
WHEN new.fingerprint IS NOT NULL
BEGIN
    UPDATE file SET fingerprint_rev = random() | 1 WHERE id = new.id;
END;

-- trigger on `file`
CREATE TRIGGER grouping_pair_goes_with_its_audio
AFTER UPDATE OF fingerprint, audio_hash, blake3 ON file
WHEN old.fingerprint IS NOT new.fingerprint
  OR coalesce(nullif(old.audio_hash, x''), old.blake3)
     IS NOT coalesce(nullif(new.audio_hash, x''), new.blake3)
BEGIN
    DELETE FROM grouping_pair WHERE file_a = old.id OR file_b = old.id;
    UPDATE grouping_file SET decided_as = NULL, rev = random() WHERE file_id = old.id;
END;

-- trigger on `file`
CREATE TRIGGER grouping_file_is_due_when_its_names_change
AFTER UPDATE OF raw_tags, rel_path ON file
WHEN old.raw_tags IS NOT new.raw_tags OR old.rel_path IS NOT new.rel_path
BEGIN
    UPDATE grouping_file SET decided_as = NULL, rev = random() WHERE file_id = old.id;
END;

-- trigger on `file`
CREATE TRIGGER grouping_file_is_due_when_it_comes_back
AFTER UPDATE OF present ON file
WHEN old.present = 0 AND new.present = 1
BEGIN
    UPDATE grouping_file SET decided_as = NULL, rev = random() WHERE file_id = old.id;
END;

-- trigger on `file`
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

-- index on `file`
CREATE INDEX file_hidden ON file (hidden_reason) WHERE hidden_reason IS NOT NULL;

-- trigger on `file`
CREATE TRIGGER search_file_text_changed
AFTER UPDATE OF raw_tags, rel_path, present ON file
WHEN old.raw_tags IS NOT new.raw_tags
  OR old.rel_path IS NOT new.rel_path
  OR old.present IS NOT new.present
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT recording_id FROM recording_file
    WHERE file_id = new.id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = recording_file.recording_id);
END;

-- trigger on `recording`
CREATE TRIGGER search_track_appears
AFTER INSERT ON recording
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT new.id WHERE NOT EXISTS (SELECT 1 FROM search_due WHERE recording_id = new.id);
END;

-- trigger on `recording`
CREATE TRIGGER search_track_goes
AFTER DELETE ON recording
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT old.id WHERE NOT EXISTS (SELECT 1 FROM search_due WHERE recording_id = old.id);
END;

-- trigger on `recording`
CREATE TRIGGER search_track_is_named
AFTER UPDATE OF title, artist ON recording
WHEN old.title IS NOT new.title OR old.artist IS NOT new.artist
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT new.id WHERE NOT EXISTS (SELECT 1 FROM search_due WHERE recording_id = new.id);
END;

-- view on `recording_display`
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

-- trigger on `library_track`
CREATE TRIGGER library_track_status_follows_new_link_update
AFTER UPDATE OF linked_file_id ON library_track
WHEN new.kind = 'linked' AND EXISTS (SELECT 1 FROM file WHERE id = new.linked_file_id)
BEGIN
    UPDATE library_track
    SET source_status = coalesce((SELECT CASE WHEN present = 1 THEN 'ok' ELSE 'missing' END
                                  FROM file WHERE id = new.linked_file_id), source_status)
    WHERE id = new.id;
END;

-- trigger on `library_track`
CREATE TRIGGER library_track_status_follows_new_link_insert
AFTER INSERT ON library_track
WHEN new.kind = 'linked' AND EXISTS (SELECT 1 FROM file WHERE id = new.linked_file_id)
BEGIN
    UPDATE library_track
    SET source_status = coalesce((SELECT CASE WHEN present = 1 THEN 'ok' ELSE 'missing' END
                                  FROM file WHERE id = new.linked_file_id), source_status)
    WHERE id = new.id;
END;

-- trigger on `music_folder`
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

-- trigger on `volume`
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

-- trigger on `review_decision`
CREATE TRIGGER best_file_due_dismissal_made
AFTER INSERT ON review_decision
WHEN new.kind = 'dismissed_better'
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT lt.recording_id FROM file f JOIN library_track lt ON lt.linked_file_id = f.id
    WHERE f.audio_hash = new.linked_key OR f.blake3 = new.linked_key
    ON CONFLICT DO NOTHING;
END;

-- trigger on `review_decision`
CREATE TRIGGER best_file_due_dismissal_taken_back
AFTER DELETE ON review_decision
WHEN old.kind = 'dismissed_better'
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT lt.recording_id FROM file f JOIN library_track lt ON lt.linked_file_id = f.id
    WHERE f.audio_hash = old.linked_key OR f.blake3 = old.linked_key
    ON CONFLICT DO NOTHING;
END;

-- 5. The count SQLite keeps for each table: past the highest id it holds
--    (the copy set that, if it copied a row) and past every id of it the
--    operation log names, as a recorded value or an operation's detail.
INSERT INTO sqlite_sequence (name, seq)
SELECT name, 0 FROM (SELECT 'file' AS name UNION ALL SELECT 'recording')
WHERE name NOT IN (SELECT name FROM sqlite_sequence);

UPDATE sqlite_sequence
SET seq = max(seq, (
    SELECT ifnull(max(id), 0) FROM (
        -- `recording_file` is named by its file's id.
        SELECT entity_id AS id FROM change WHERE entity = 'recording_file'
        UNION ALL
        SELECT CAST(v.value AS INTEGER)
        FROM change c, json_each(json_array(c.before, c.after)) v
        WHERE (c.entity, c.field) IN (VALUES ('library_track', 'linked_file_id'),
                                             ('library_track', 'source_file_id'),
                                             ('relink', 'file_id'))
        UNION ALL
        -- A detail that names a file, at any depth (a split's `moved`
        -- files). `from` is a file in a switch and a follow, a track in a
        -- split: counted for both tables, which only skips some ids.
        SELECT d.value FROM operation o, json_tree(o.details) d
        WHERE d.key IN ('file', 'fileId', 'from', 'to', 'keepLinked')
          AND d.type = 'integer')))
WHERE name = 'file';

UPDATE sqlite_sequence
SET seq = max(seq, (
    SELECT ifnull(max(id), 0) FROM (
        SELECT CAST(v.value AS INTEGER) AS id
        FROM change c, json_each(json_array(c.before, c.after)) v
        WHERE (c.entity, c.field) IN (VALUES ('library_track', 'recording_id'),
                                             ('library_removal', 'recording_id'),
                                             ('replaced_entry', 'recording_id'),
                                             ('track_merge', 'recording_id'),
                                             ('track_merge', 'into_id'),
                                             ('recording_file', 'recording_id'))
        UNION ALL
        SELECT d.value FROM operation o, json_tree(o.details) d
        WHERE (d.key IN ('recordingId', 'keep', 'into', 'from') OR d.path = '$.recordingIds')
          AND d.type = 'integer')))
WHERE name = 'recording';
