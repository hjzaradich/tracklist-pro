-- 0003: tracks (ROADMAP §1.2, §2). A track (`recording`) is one piece of
-- music in one version, however many files hold it. Duplicates merge;
-- versions never merge, they're only linked. BPM, key and energy live in
-- `analysis`, and the values a track shows are derived from it by the
-- `recording_display` view, so they can't be edited directly.

-- A track.
CREATE TABLE recording (
    id         INTEGER PRIMARY KEY,
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

-- A track's files: its duplicates. A file holds exactly one track.
CREATE TABLE recording_file (
    recording_id     INTEGER NOT NULL REFERENCES recording (id) ON DELETE RESTRICT,
    file_id          INTEGER NOT NULL PRIMARY KEY REFERENCES file (id) ON DELETE RESTRICT,
    -- best: the file whose audio is used; extra: hidden, never touched on
    -- disk; undecided: not yet reviewed.
    role             TEXT    NOT NULL DEFAULT 'undecided'
        CHECK (role IN ('best', 'undecided', 'extra')),
    match_confidence REAL    CHECK (match_confidence BETWEEN 0 AND 1)
) STRICT;

CREATE INDEX recording_file_by_recording ON recording_file (recording_id);
-- One best file per track.
CREATE UNIQUE INDEX recording_file_one_best ON recording_file (recording_id)
    WHERE role = 'best';

-- Two versions of one song: separate tracks, linked, never merged, and
-- nothing is inherited across the link. `recording_a` → `recording_b`
-- keeps a direction for labels that have one (a mashup contains a source).
CREATE TABLE version_link (
    id          INTEGER PRIMARY KEY,
    recording_a INTEGER NOT NULL REFERENCES recording (id) ON DELETE RESTRICT,
    recording_b INTEGER NOT NULL REFERENCES recording (id) ON DELETE RESTRICT,
    -- cut: same production, different length or lyrics; rework: a
    -- different production (§1.2).
    kind        TEXT    NOT NULL CHECK (kind IN ('cut', 'rework')),
    -- extended, radio, clean/dirty, remix, vip, flip, bootleg, cover, live,
    -- arrangement, mashup-contains… (1.5). Open-ended, so not a CHECK.
    label       TEXT,
    source      TEXT    NOT NULL CHECK (source IN ('parser', 'fingerprint', 'user')),
    confirmed   INTEGER NOT NULL DEFAULT 0 CHECK (confirmed IN (0, 1)),
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (recording_a <> recording_b)
) STRICT;

-- One link per pair, whichever way round it was made.
CREATE UNIQUE INDEX version_link_one_per_pair
    ON version_link (min(recording_a, recording_b), max(recording_a, recording_b));
-- Foreign-key lookups: deleting a track checks both sides.
CREATE INDEX version_link_by_a ON version_link (recording_a);
CREATE INDEX version_link_by_b ON version_link (recording_b);

-- Merging two linked versions is refused: a file can't be moved from one
-- to the other, by an UPDATE (or an upsert, which runs one) or by an
-- INSERT OR REPLACE over the file's row. (Deleting either track is refused
-- by the foreign keys, so the link has to be removed on purpose first.)
CREATE TRIGGER recording_file_versions_never_merge
BEFORE UPDATE OF recording_id ON recording_file
WHEN EXISTS (
    SELECT 1 FROM version_link
    WHERE (recording_a = old.recording_id AND recording_b = new.recording_id)
       OR (recording_a = new.recording_id AND recording_b = old.recording_id)
)
BEGIN
    SELECT RAISE(ABORT, 'versions never merge: these tracks are linked versions');
END;

-- A BEFORE INSERT trigger still sees the row a REPLACE is about to remove.
CREATE TRIGGER recording_file_versions_never_merge_replace
BEFORE INSERT ON recording_file
WHEN EXISTS (
    SELECT 1 FROM recording_file f JOIN version_link l
      ON (l.recording_a = f.recording_id AND l.recording_b = new.recording_id)
      OR (l.recording_a = new.recording_id AND l.recording_b = f.recording_id)
    WHERE f.file_id = new.file_id
)
BEGIN
    SELECT RAISE(ABORT, 'versions never merge: these tracks are linked versions');
END;

-- BPM, key and energy, one row per track per source. The source of truth
-- for these values (CLAUDE.md).
CREATE TABLE analysis (
    id           INTEGER PRIMARY KEY,
    -- RESTRICT: merging tracks moves or drops their analysis on purpose.
    recording_id INTEGER NOT NULL REFERENCES recording (id) ON DELETE RESTRICT,
    -- mik = Mixed In Key; local = the app's own estimate (3.7); ml = the
    -- audio model (3.8).
    source       TEXT    NOT NULL
        CHECK (source IN ('rekordbox', 'tag', 'mik', 'user', 'local', 'ml')),
    bpm          REAL    CHECK (bpm > 0 AND bpm < 1000),
    -- Camelot, normalized before it's stored (tags/key.rs).
    key          TEXT    CHECK (key IN (
        '1A', '2A', '3A', '4A', '5A', '6A', '7A', '8A', '9A', '10A', '11A', '12A',
        '1B', '2B', '3B', '4B', '5B', '6B', '7B', '8B', '9B', '10B', '11B', '12B')),
    energy       INTEGER CHECK (energy BETWEEN 1 AND 10),
    confidence   REAL    CHECK (confidence BETWEEN 0 AND 1),
    analyzed_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- A row says something.
    CHECK (bpm IS NOT NULL OR key IS NOT NULL OR energy IS NOT NULL),
    -- Every value a source can give has a place in the precedence below:
    -- BPM and key come from rekordbox, tags, MIK or the local estimate;
    -- energy from the user, MIK, the local estimate or the audio model.
    CHECK (source NOT IN ('user', 'ml') OR (bpm IS NULL AND key IS NULL)),
    CHECK (source NOT IN ('rekordbox', 'tag') OR energy IS NULL),
    UNIQUE (recording_id, source)
) STRICT;

-- The BPM, key and energy a track shows, derived from `analysis` and never
-- stored. Precedence (ROADMAP §2): BPM and key go rekordbox > file tag >
-- MIK > local; energy goes user > MIK > local > audio model. Each value
-- comes with the source it came from, so the UI can say why.
-- A view can't be written to, so there is nothing to edit directly.
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
