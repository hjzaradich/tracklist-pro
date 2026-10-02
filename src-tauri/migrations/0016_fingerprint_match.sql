-- 0016: what comparing two files' full-track fingerprints found (1bA-2;
-- ROADMAP 1.4).
--
-- One row per compared pair, the lower file id first. Derived state: the
-- rows can always be worked out again from `file.fingerprint`
-- (src/matching), so nothing here is in the operation log, and a file row
-- that goes takes its rows with it.
--
-- A pair that was compared and didn't match is kept too (coverage 0,
-- score 32, no segments), so it isn't compared again.
CREATE TABLE fingerprint_match (
    file_a      INTEGER NOT NULL REFERENCES file (id) ON DELETE CASCADE,
    file_b      INTEGER NOT NULL REFERENCES file (id) ON DELETE CASCADE,
    -- The comparison's definition (matching::VERSION). Rows of another
    -- version are worked out again.
    version     INTEGER NOT NULL CHECK (version >= 1),
    -- Each fingerprint's length, in items (~0.124 s each).
    items_a     INTEGER NOT NULL CHECK (items_a >= 0),
    items_b     INTEGER NOT NULL CHECK (items_b >= 0),
    -- The share of each file the main alignment matched, and the average
    -- number of differing bits (of 32) per item over the matched part:
    -- what ROADMAP 1.4's duplicate rule is applied to.
    coverage_a  REAL    NOT NULL CHECK (coverage_a BETWEEN 0 AND 1),
    coverage_b  REAL    NOT NULL CHECK (coverage_b BETWEEN 0 AND 1),
    score       REAL    NOT NULL CHECK (score BETWEEN 0 AND 32),
    -- Every matched stretch: a JSON array of
    -- [offset_a, offset_b, items, score, alignment], offsets in items.
    segments    TEXT    NOT NULL CHECK (json_valid(segments) AND json_type(segments) = 'array'),
    compared_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (file_a, file_b),
    CHECK (file_a < file_b)
) STRICT, WITHOUT ROWID;

-- A file's rows where it's the second of the pair.
CREATE INDEX fingerprint_match_file_b ON fingerprint_match (file_b);

-- A result stands for the fingerprints it was made from. When a file's
-- fingerprint changes (other audio, or none any more), its results go,
-- whichever code changed it. A tag rewrite, a new modified time or a
-- fingerprint carried forward (0010) doesn't change these bytes, so the
-- results stay (ROADMAP 5.1).
CREATE TRIGGER fingerprint_match_goes_with_its_fingerprint
AFTER UPDATE OF fingerprint ON file
WHEN old.fingerprint IS NOT new.fingerprint
BEGIN
    DELETE FROM fingerprint_match WHERE file_a = old.id OR file_b = old.id;
END;
