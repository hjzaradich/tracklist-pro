-- 0020: a stamp that changes whenever a file's fingerprint does (1bA-18;
-- ROADMAP 1.4).
--
-- Matching keeps what it read of each fingerprint in memory between runs.
-- To know which ones to read again it needs a cheap answer to "has this
-- file's fingerprint changed since I read it?", which the blob itself can't
-- give without being read. This column is that answer: the stamp the file
-- had when the blob was read is the stamp it has now, or the blob changed.
--
-- The stamp is random, not a counter: only equality is asked of it, and
-- a row that reuses the id of a deleted one (a removed music folder's files
-- were the highest ids) can't carry a stamp the deleted row had. Rows that
-- exist when this migration runs start at 0, as does a row with no
-- fingerprint; the first run after the upgrade reads every fingerprint once.
--
-- Derived state, like `fingerprint_match`: nothing here is in the
-- operation log.
ALTER TABLE file ADD COLUMN fingerprint_rev INTEGER NOT NULL DEFAULT 0;

-- Every change of the stored bytes gets a new stamp, to or from NULL
-- included (`IS NOT` compares NULLs as values, as 0016's and 0019's
-- triggers do), whichever code made it. The same bytes written again, a
-- tag rewrite and a new modified time change nothing (ROADMAP 5.1).
CREATE TRIGGER file_fingerprint_rev_moves_with_its_fingerprint
AFTER UPDATE OF fingerprint ON file
WHEN old.fingerprint IS NOT new.fingerprint
BEGIN
    UPDATE file SET fingerprint_rev = random() | 1 WHERE id = new.id;
END;

-- A row inserted with a fingerprint already has one to stamp.
CREATE TRIGGER file_fingerprint_rev_starts_with_its_fingerprint
AFTER INSERT ON file
WHEN new.fingerprint IS NOT NULL
BEGIN
    UPDATE file SET fingerprint_rev = random() | 1 WHERE id = new.id;
END;
