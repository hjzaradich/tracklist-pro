-- 0022: what the grouping job decided, and what the user decided for it
-- (1bC-1; ROADMAP 1.4, 1.5).
--
-- Three tables. `grouping_pair` and `grouping_file` are derived state: the
-- job can work them out again from the files, their fingerprints and their
-- names (src/grouping), so nothing in them is in the operation log.
-- `grouping_decision` is the user's: it's never worked out again.

-- What the grouping rules said about two files, the lower file id first.
-- Every pair the job decided that isn't "different", plus the "different"
-- pairs it had to decide inside a set of duplicates before merging it.
CREATE TABLE grouping_pair (
    file_a      INTEGER NOT NULL REFERENCES file (id) ON DELETE CASCADE,
    file_b      INTEGER NOT NULL REFERENCES file (id) ON DELETE CASCADE,
    -- duplicate: the same track. cut / rework: two versions, linked.
    -- review: the signals disagree, the user decides. different: nothing
    -- to each other.
    outcome     TEXT    NOT NULL
        CHECK (outcome IN ('duplicate', 'cut', 'rework', 'review', 'different')),
    -- The rules' reasons, the deciding one first: a JSON array of codes
    -- with their parameters (`grouping::rules::Reason`), never text. Sides
    -- A and B are file_a and file_b.
    reasons     TEXT    NOT NULL CHECK (json_valid(reasons) AND json_type(reasons) = 'array'),
    -- For a cut or a rework, the link it asks for: its label, what made it
    -- (the names alone, or the audio too), and which file's track comes
    -- first where the label has a direction (a mashup contains its part).
    link_label  TEXT,
    link_source TEXT    CHECK (link_source IN ('parser', 'fingerprint')),
    link_first  TEXT    CHECK (link_first IN ('a', 'b')),
    -- Where the pair stands, set by every grouping run:
    --   waiting          a duplicate whose set isn't decided in full yet
    --   merged           a duplicate, and the two files share a track
    --   both_in_library  a duplicate held apart: both tracks are in the
    --                    Library (the user decides, 1bE-9)
    --   set_disagrees    a duplicate held apart: a pair in its set isn't
    --                    one, or the set is too big to check
    --   linked           a cut or a rework: a version link stands for it
    --   open             a review pair, the files in two tracks
    --   in_one_track     the files share a track, and the rules no longer
    --                    call them duplicates (a merge is never undone by
    --                    the job)
    --   decided          the user settled it: a remembered decision, or a
    --                    version link of their own
    --   apart            different: nothing to do
    state       TEXT    NOT NULL DEFAULT 'waiting'
        CHECK (state IN ('waiting', 'merged', 'both_in_library', 'set_disagrees', 'linked',
                         'open', 'in_one_track', 'decided', 'apart')),
    decided_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (file_a, file_b),
    CHECK (file_a < file_b),
    CHECK ((outcome IN ('cut', 'rework')) = (link_source IS NOT NULL))
) STRICT, WITHOUT ROWID;

-- A file's rows where it's the second of the pair.
CREATE INDEX grouping_pair_file_b ON grouping_pair (file_b);

-- What grouping knows about one file.
CREATE TABLE grouping_file (
    file_id         INTEGER NOT NULL PRIMARY KEY REFERENCES file (id) ON DELETE CASCADE,
    -- The audio the file held when it was put in its track: its
    -- `audio_hash`, or its `blake3` when it has none. NULL when it had
    -- neither. A file that holds other audio now leaves its track.
    placed_key      BLOB,
    -- The version of the rules and of the comparison every one of the
    -- file's pairs was decided under (`grouping::decided_as`). NULL: due.
    -- Set only once all of the file's pairs are written.
    decided_as      INTEGER,
    -- Changes whenever something a decision reads does, so a run that
    -- read the file before the change can't mark it decided.
    rev             INTEGER NOT NULL DEFAULT 0,
    -- The quality verdict the file was decided at. Reserved: nothing reads
    -- or writes it yet (a cut-off copy inside a same-named file is a worse
    -- duplicate, not a cut; TASKS 1bC-1).
    quality_verdict TEXT
) STRICT, WITHOUT ROWID;

-- A decision the user made about two tracks, kept by what the files hold,
-- not by file or track id: each side is a file's `audio_hash`, or its
-- `blake3` when it has none, the lower first. So it survives a retag, a
-- move, and a music folder removed and added again, and it covers every
-- pair of files between the two tracks that hold those two pieces of audio.
--   apart: never merge them, and don't ask again.
--   same:  they are duplicates, whatever the rules say.
-- A rowid table with a unique pair, so the operation log can record it.
CREATE TABLE grouping_decision (
    id         INTEGER PRIMARY KEY,
    key_a      BLOB    NOT NULL CHECK (length(key_a) > 0),
    key_b      BLOB    NOT NULL CHECK (length(key_b) > 0),
    decision   TEXT    NOT NULL CHECK (decision IN ('apart', 'same')),
    decided_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (key_a, key_b),
    -- Equal keys: two tracks that hold the very same audio (two Library
    -- tracks the user keeps both of).
    CHECK (key_a <= key_b)
) STRICT;

-- A pair stands for the audio it was decided on. When a file's fingerprint
-- changes, or the audio it holds (its `audio_hash`, or its `blake3` when it
-- has none), its pairs go and it's due again, whichever code changed it.
-- A tag rewrite changes `blake3` but not the audio (ROADMAP 5.1), so a file
-- with an `audio_hash` keeps its pairs through one.
CREATE TRIGGER grouping_pair_goes_with_its_audio
AFTER UPDATE OF fingerprint, audio_hash, blake3 ON file
WHEN old.fingerprint IS NOT new.fingerprint
  OR coalesce(nullif(old.audio_hash, x''), old.blake3)
     IS NOT coalesce(nullif(new.audio_hash, x''), new.blake3)
BEGIN
    DELETE FROM grouping_pair WHERE file_a = old.id OR file_b = old.id;
    UPDATE grouping_file SET decided_as = NULL, rev = random() WHERE file_id = old.id;
END;

-- New names (the tags, or the file's name) can change what the rules say:
-- the file is due again. Its pairs stay until they're decided again, so a
-- version link doesn't come and go with every tag rewrite.
CREATE TRIGGER grouping_file_is_due_when_its_names_change
AFTER UPDATE OF raw_tags, rel_path ON file
WHEN old.raw_tags IS NOT new.raw_tags OR old.rel_path IS NOT new.rel_path
BEGIN
    UPDATE grouping_file SET decided_as = NULL, rev = random() WHERE file_id = old.id;
END;

-- A file that comes back was never decided against the files that arrived
-- while it was gone.
CREATE TRIGGER grouping_file_is_due_when_it_comes_back
AFTER UPDATE OF present ON file
WHEN old.present = 0 AND new.present = 1
BEGIN
    UPDATE grouping_file SET decided_as = NULL, rev = random() WHERE file_id = old.id;
END;
