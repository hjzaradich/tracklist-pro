-- 0019: which files a finished look for shared audio has covered (1bA-14;
-- ROADMAP 1.4).
--
-- One row per file whose current fingerprint has been through matching
-- (src/matching): its candidates were found and every result stored. The
-- row is written in the same transaction as the last of those results, so
-- a run cut short leaves "not done", never "done without results". A
-- present file with a fingerprint and no row of the current version is
-- what makes matching due.
--
-- Derived state, like `fingerprint_match`: nothing here is in the
-- operation log, and a file row that goes takes its row with it.
CREATE TABLE fingerprint_matched (
    file_id    INTEGER NOT NULL PRIMARY KEY REFERENCES file (id) ON DELETE CASCADE,
    -- The comparison's definition (matching::VERSION). A row of another
    -- version counts as no row: a new version makes every file due without
    -- anything being deleted.
    version    INTEGER NOT NULL CHECK (version >= 1),
    -- The file whose results stand for this one: itself, or the first of
    -- the files holding the very same fingerprint, which holds the results
    -- for all of them. When that file goes or changes, this one is due.
    stands     INTEGER NOT NULL REFERENCES file (id) ON DELETE CASCADE,
    matched_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT, WITHOUT ROWID;

-- Deleting the file that stands for others looks their rows up by it.
CREATE INDEX fingerprint_matched_stands ON fingerprint_matched (stands);

-- A row stands for the fingerprint it was made from, exactly as a result
-- does (0016): when a file's fingerprint changes or is taken away, its row
-- goes, whichever code changed it. A tag rewrite, a new modified time or
-- the same bytes written again changes nothing (ROADMAP 5.1).
CREATE TRIGGER fingerprint_matched_goes_with_its_fingerprint
AFTER UPDATE OF fingerprint ON file
WHEN old.fingerprint IS NOT new.fingerprint
BEGIN
    DELETE FROM fingerprint_matched WHERE file_id = old.id;
END;

-- A file that goes missing keeps its row, as it keeps its results. When it
-- comes back, files that arrived meanwhile were never compared with it, so
-- it's due again: one look in the index, and only pairs with no result
-- yet are compared. (An unplugged drive's files stay present; this is a
-- file that was gone from a folder the walk could list.)
CREATE TRIGGER fingerprint_matched_goes_when_a_missing_file_comes_back
AFTER UPDATE OF present ON file
WHEN old.present = 0 AND new.present = 1
BEGIN
    DELETE FROM fingerprint_matched WHERE file_id = old.id;
END;
