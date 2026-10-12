-- 0024: the full-text search index (1cA-6; ROADMAP 1.11 "What search covers").
--
-- Derived state: every row can be worked out again from the files, their
-- tags and rekordbox's snapshot (src/search), so nothing here is a fact and
-- nothing is in the operation log.
--
-- `search_track` has one row per track, its rowid the track's id. Its eight
-- columns hold the text the lists show for the track (title and artist as
-- the lists compute them, the other fields from the same source) and the
-- names of its files. It is contentless: the text is not kept, only what
-- MATCH needs, so a row can be found and deleted but not read back. The
-- tokenizer ignores capitals and accents, and a search word matches from
-- the start of a word (the query is built in src/search, never passed in
-- raw).
--
-- `search_due` lists the tracks whose row is missing or may be out of date.
-- The triggers below only write down WHICH track (they parse nothing); the
-- search job works the rest out in Rust, the way the lists do, and removes
-- the track from `search_due` in the same transaction that writes its row.
--   -1  stands for "every Library track": a rekordbox read or relink can
--       change the title a Library track shows (a track that is not in the
--       Library never takes its text from rekordbox).
-- Nothing is hidden in the index: whether a file is hidden is read from
-- `file` and `recording_file` when a search runs.

CREATE VIRTUAL TABLE search_track USING fts5(
    title, artist, album, remixer, label, genre, comment, file_name,
    content = '',
    contentless_delete = 1,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TABLE search_due (
    recording_id INTEGER NOT NULL PRIMARY KEY
) STRICT, WITHOUT ROWID;

-- Finding the hidden files of the tracks a search matched: the few extra
-- copies and the few files the user hid, without a walk over every file.
CREATE INDEX recording_file_extras ON recording_file (file_id) WHERE role = 'extra';
CREATE INDEX file_hidden ON file (hidden_reason) WHERE hidden_reason IS NOT NULL;

-- After an upgrade, every track is due.
INSERT INTO search_due (recording_id) SELECT id FROM recording;

-- A file's tags or name changed, or it went missing or came back.
CREATE TRIGGER search_file_text_changed
AFTER UPDATE OF raw_tags, rel_path, present ON file
WHEN old.raw_tags IS NOT new.raw_tags
  OR old.rel_path IS NOT new.rel_path
  OR old.present IS NOT new.present
BEGIN
    INSERT OR IGNORE INTO search_due (recording_id)
    SELECT recording_id FROM recording_file WHERE file_id = new.id;
END;

-- A file joins, leaves or moves between tracks, or changes role (the best
-- file's tags are read first).
CREATE TRIGGER search_file_joins_a_track
AFTER INSERT ON recording_file
BEGIN
    INSERT OR IGNORE INTO search_due (recording_id) VALUES (new.recording_id);
END;

-- INSERT OR REPLACE over a file's row drops the old row without a delete
-- trigger: its track is due too.
CREATE TRIGGER search_file_replaces_a_row
BEFORE INSERT ON recording_file
BEGIN
    INSERT OR IGNORE INTO search_due (recording_id)
    SELECT recording_id FROM recording_file WHERE file_id = new.file_id;
END;

CREATE TRIGGER search_file_leaves_a_track
AFTER DELETE ON recording_file
BEGIN
    INSERT OR IGNORE INTO search_due (recording_id) VALUES (old.recording_id);
END;

CREATE TRIGGER search_file_moves_or_changes_role
AFTER UPDATE OF recording_id, role ON recording_file
BEGIN
    INSERT OR IGNORE INTO search_due (recording_id) VALUES (old.recording_id);
    INSERT OR IGNORE INTO search_due (recording_id) VALUES (new.recording_id);
END;

-- A track appears, goes, or gets a title or artist of its own.
CREATE TRIGGER search_track_appears
AFTER INSERT ON recording
BEGIN
    INSERT OR IGNORE INTO search_due (recording_id) VALUES (new.id);
END;

CREATE TRIGGER search_track_goes
AFTER DELETE ON recording
BEGIN
    INSERT OR IGNORE INTO search_due (recording_id) VALUES (old.id);
END;

CREATE TRIGGER search_track_is_named
AFTER UPDATE OF title, artist ON recording
WHEN old.title IS NOT new.title OR old.artist IS NOT new.artist
BEGIN
    INSERT OR IGNORE INTO search_due (recording_id) VALUES (new.id);
END;

-- A track joins or leaves the Library, or is linked to another file: where
-- its title comes from changes.
CREATE TRIGGER search_library_track_added
AFTER INSERT ON library_track
BEGIN
    INSERT OR IGNORE INTO search_due (recording_id) VALUES (new.recording_id);
END;

CREATE TRIGGER search_library_track_removed
AFTER DELETE ON library_track
BEGIN
    INSERT OR IGNORE INTO search_due (recording_id) VALUES (old.recording_id);
END;

CREATE TRIGGER search_library_track_changes
AFTER UPDATE OF recording_id, linked_file_id ON library_track
BEGIN
    INSERT OR IGNORE INTO search_due (recording_id) VALUES (old.recording_id);
    INSERT OR IGNORE INTO search_due (recording_id) VALUES (new.recording_id);
END;

-- A rekordbox read replaces the snapshot, and a relink changes which file a
-- row belongs to: every Library track may show another title.
CREATE TRIGGER search_rekordbox_row_added
AFTER INSERT ON rekordbox_track
BEGIN
    INSERT OR IGNORE INTO search_due (recording_id) VALUES (-1);
END;

CREATE TRIGGER search_rekordbox_row_removed
AFTER DELETE ON rekordbox_track
BEGIN
    INSERT OR IGNORE INTO search_due (recording_id) VALUES (-1);
END;

CREATE TRIGGER search_rekordbox_row_matched
AFTER UPDATE OF file_id, relink_probable ON rekordbox_track
WHEN old.file_id IS NOT new.file_id OR old.relink_probable IS NOT new.relink_probable
BEGIN
    INSERT OR IGNORE INTO search_due (recording_id) VALUES (-1);
END;
