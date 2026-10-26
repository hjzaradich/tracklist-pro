-- 0032: the track table's sort and filter rows (1cA-3, 1cA-4; ROADMAP 1.11
-- "The track table").
--
-- Derived state, like the search index it is kept beside: every row can be
-- worked out again from the files, their tags, the analysis and rekordbox's
-- snapshot (src/search/table_row.rs), so nothing here is a fact and nothing
-- is in the operation log. The table has one row per track that has a file
-- or a Library track. It holds what the track table sorts and filters by,
-- so a window of a list of 100,000 tracks is one indexed query that reads
-- the index alone, and never a pass over the tracks in Rust. What the table
-- SHOWS is worked out again for the rows on screen, so a row is never wrong
-- for being a moment behind: only its place in the order can be.
--
-- Text is kept as a sort key (accents and capitals folded, `sort_key`), and
-- a blank value is NULL, so that the table can put blanks last whichever
-- way it sorts. `key_order` is the position on the Camelot wheel (1A = 2,
-- 1B = 3, 2A = 4 ...), so keys sort round the wheel whatever notation is
-- shown.
--
-- `flags` packs what the filters ask, so an index holds them and a filter
-- never leaves the index:
--      1  in the Library            2  has a file (it is in All music)
--      4  its file is missing       8  its file has a quality finding
--     16  rekordbox has it         32  it is in a crate
-- `format` is the shown file's `sniffed_format`.
--
-- The index job (`search_due`) writes the rows, in the same transaction as
-- the search index's. The triggers below add what the search index did not
-- need: a change of BPM, key, length, format, quality verdict, crate or the
-- Library's missing mark.
CREATE TABLE track_row (
    recording_id INTEGER NOT NULL PRIMARY KEY,
    flags        INTEGER NOT NULL DEFAULT 0,
    format       TEXT,
    -- The file the track shows: its Library track's linked file, else its
    -- best file, else its lowest file id.
    file_id      INTEGER,
    title_key    TEXT    NOT NULL,
    artist_key   TEXT,
    genre_key    TEXT,
    file_key     TEXT,
    folder_key   TEXT,
    bpm          REAL,
    key_order    INTEGER,
    time_ms      INTEGER,
    -- Stars, 0 to 5; none for a track rekordbox does not know.
    rating       INTEGER,
    -- When it joined the Library, else when the scan first found it.
    added_at     TEXT
) STRICT;

-- One index per column and direction, each holding the flags and the format
-- after the id, so a page, a filtered page and a count are read from the
-- index alone. A blank sorts last both ways, so the sort is on "is blank"
-- first; ties fall back to the track's id (and, for the text a DJ sorts a
-- list by, to the title first), the same whichever way the column goes.
CREATE INDEX track_row_title ON track_row (title_key, artist_key, recording_id, flags, format);
CREATE INDEX track_row_title_desc ON track_row (title_key DESC, artist_key, recording_id, flags, format);
CREATE INDEX track_row_artist ON track_row ((artist_key IS NULL), artist_key, title_key, recording_id, flags, format);
CREATE INDEX track_row_artist_desc ON track_row ((artist_key IS NULL), artist_key DESC, title_key, recording_id, flags, format);
CREATE INDEX track_row_genre ON track_row ((genre_key IS NULL), genre_key, title_key, recording_id, flags, format);
CREATE INDEX track_row_genre_desc ON track_row ((genre_key IS NULL), genre_key DESC, title_key, recording_id, flags, format);
CREATE INDEX track_row_bpm ON track_row ((bpm IS NULL), bpm, recording_id, flags, format);
CREATE INDEX track_row_bpm_desc ON track_row ((bpm IS NULL), bpm DESC, recording_id, flags, format);
CREATE INDEX track_row_key ON track_row ((key_order IS NULL), key_order, recording_id, flags, format);
CREATE INDEX track_row_key_desc ON track_row ((key_order IS NULL), key_order DESC, recording_id, flags, format);
CREATE INDEX track_row_time ON track_row ((time_ms IS NULL), time_ms, recording_id, flags, format);
CREATE INDEX track_row_time_desc ON track_row ((time_ms IS NULL), time_ms DESC, recording_id, flags, format);
CREATE INDEX track_row_rating ON track_row ((rating IS NULL), rating, recording_id, flags, format);
CREATE INDEX track_row_rating_desc ON track_row ((rating IS NULL), rating DESC, recording_id, flags, format);
CREATE INDEX track_row_added ON track_row ((added_at IS NULL), added_at, recording_id, flags, format);
CREATE INDEX track_row_added_desc ON track_row ((added_at IS NULL), added_at DESC, recording_id, flags, format);
CREATE INDEX track_row_file ON track_row ((file_key IS NULL), file_key, recording_id, flags, format);
CREATE INDEX track_row_file_desc ON track_row ((file_key IS NULL), file_key DESC, recording_id, flags, format);
CREATE INDEX track_row_folder ON track_row ((folder_key IS NULL), folder_key, recording_id, flags, format);
CREATE INDEX track_row_folder_desc ON track_row ((folder_key IS NULL), folder_key DESC, recording_id, flags, format);

-- A count of the rows a filter leaves reads this narrow index instead of a
-- wide sort index.
CREATE INDEX track_row_flags ON track_row (flags, format);

-- After an upgrade, every track is due.
INSERT OR IGNORE INTO search_due (recording_id) SELECT id FROM recording;

-- A track's BPM or key (rekordbox's, or another source's) changed.
CREATE TRIGGER search_analysis_added
AFTER INSERT ON analysis
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT new.recording_id WHERE NOT EXISTS (SELECT 1 FROM search_due WHERE recording_id = new.recording_id);
END;

CREATE TRIGGER search_analysis_removed
AFTER DELETE ON analysis
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT old.recording_id WHERE NOT EXISTS (SELECT 1 FROM search_due WHERE recording_id = old.recording_id);
END;

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

-- A file's length or format was worked out or changed.
CREATE TRIGGER search_file_facts_changed
AFTER UPDATE OF duration_ms, sniffed_format ON file
WHEN old.duration_ms IS NOT new.duration_ms OR old.sniffed_format IS NOT new.sniffed_format
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT recording_id FROM recording_file
    WHERE file_id = new.id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = recording_file.recording_id);
END;

-- A file's stored quality verdict was made, changed or dropped.
CREATE TRIGGER search_verdict_added
AFTER INSERT ON file_verdict
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT recording_id FROM recording_file
    WHERE file_id = new.file_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = recording_file.recording_id);
END;

CREATE TRIGGER search_verdict_changed
AFTER UPDATE OF verdict ON file_verdict
WHEN old.verdict IS NOT new.verdict
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT recording_id FROM recording_file
    WHERE file_id = new.file_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = recording_file.recording_id);
END;

CREATE TRIGGER search_verdict_removed
AFTER DELETE ON file_verdict
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT recording_id FROM recording_file
    WHERE file_id = old.file_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = recording_file.recording_id);
END;

-- The scan marked a Library track's file missing, or found it again.
CREATE TRIGGER search_library_status_changed
AFTER UPDATE OF source_status ON library_track
WHEN old.source_status IS NOT new.source_status
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT new.recording_id WHERE NOT EXISTS (SELECT 1 FROM search_due WHERE recording_id = new.recording_id);
END;

-- A Library track joins a crate, leaves it, or moves to another.
CREATE TRIGGER search_crate_entry_added
AFTER INSERT ON crate_entry
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.id = new.library_track_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;

CREATE TRIGGER search_crate_entry_removed
AFTER DELETE ON crate_entry
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.id = old.library_track_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;

CREATE TRIGGER search_crate_entry_changed
AFTER UPDATE OF library_track_id, kind ON crate_entry
WHEN old.library_track_id IS NOT new.library_track_id OR old.kind IS NOT new.kind
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.id IN (old.library_track_id, new.library_track_id)
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;
