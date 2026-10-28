-- 0037: the file the user chose to show for a track outside the Library
-- ("Use this file instead", ROADMAP 1.11 "Hidden filter" and 1.7; 1cB-22).
--
-- A choice is kept by what the file holds, not by file or track id (as
-- `review_decision`, 0027, and `grouping_decision`, 0022, are): `content_key`
-- is the file's `audio_hash`, or its `blake3` when it has none. So a choice
-- survives a rescan, a retag, a move and a music folder removed and added
-- again. A file that is encoded again holds other audio: it has another
-- key, and the choice doesn't cover it (the ranking decides again).
--
-- `blake3` is the whole file's hash when the file had one: two copies of one
-- recording that differ only in tags share a `content_key`, and this tells
-- the chosen one from the other. A track's files that share the key and not
-- the bytes are told apart by it; if no file of the track still has these
-- bytes (it was retagged), the choice covers the files with the key. Empty
-- when the file had no whole-file hash.
--
-- Read by the ranking (`file_roles`) as one of the facts about a file:
-- `best_file::FileFacts::chosen`, the first thing the ranking looks at. A
-- chosen file that isn't on disk has no say.
--
-- A rowid table with a unique key, so the operation log can record it:
-- every row is made by a user action and taken away by its Undo. Choosing
-- another file of the same track takes the earlier choice away in the same
-- operation.
CREATE TABLE file_choice (
    id          INTEGER PRIMARY KEY,
    content_key BLOB    NOT NULL CHECK (length(content_key) > 0),
    blake3      BLOB    NOT NULL DEFAULT x'' CHECK (blake3 = x'' OR length(blake3) = 32),
    chosen_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (content_key, blake3)
) STRICT;

-- A choice made or taken back changes the roles of the tracks that hold a
-- file of that audio, whichever code changes the row: they are ranked
-- again. The same audio is looked up through the hash indexes of `file`.
CREATE TRIGGER best_file_due_choice_made
AFTER INSERT ON file_choice
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT rf.recording_id FROM file f JOIN recording_file rf ON rf.file_id = f.id
    WHERE f.audio_hash = new.content_key OR f.blake3 = new.content_key
    ON CONFLICT DO NOTHING;
END;

CREATE TRIGGER best_file_due_choice_taken_back
AFTER DELETE ON file_choice
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT rf.recording_id FROM file f JOIN recording_file rf ON rf.file_id = f.id
    WHERE f.audio_hash = old.content_key OR f.blake3 = old.content_key
    ON CONFLICT DO NOTHING;
END;

-- A file's whole-file hash changed (a retag, a first hash): a choice may
-- now cover it or no longer. Only while there is a choice, so a first scan
-- of a library with none pays one empty lookup per file.
CREATE TRIGGER best_file_due_hash_changed_under_a_choice
AFTER UPDATE OF blake3 ON file
WHEN old.blake3 IS NOT new.blake3 AND EXISTS (SELECT 1 FROM file_choice)
BEGIN
    INSERT INTO best_file_due (recording_id)
    SELECT recording_id FROM recording_file WHERE file_id = new.id
    ON CONFLICT DO NOTHING;
END;

-- A chosen file that is retagged keeps its choice: its bytes change and its
-- audio doesn't, and the choice names the bytes to tell it from another
-- copy of the same audio. So the bytes follow the file, as the file is
-- hashed again. Only for a choice kept by `audio_hash` (a file with none is
-- kept by its `blake3`, which a retag changes: that choice is lost with it,
-- as `review_decision`'s is). This is done outside the operation log, so
-- `file_choice.blake3` is a derived column (`ops::DERIVED_FIELDS`): the
-- Undo of the choice doesn't count it as "changed since". An update that
-- would make two choices the same is left out (`OR IGNORE`).
CREATE TRIGGER file_choice_follows_a_retag
AFTER UPDATE OF blake3 ON file
WHEN old.blake3 IS NOT NULL AND new.blake3 IS NOT NULL AND old.blake3 <> new.blake3
 AND length(coalesce(old.audio_hash, x'')) > 0
BEGIN
    UPDATE OR IGNORE file_choice SET blake3 = new.blake3
    WHERE blake3 = old.blake3 AND content_key = old.audio_hash;
END;
