-- 0004: the Library and keeping it in step with rekordbox (ROADMAP 1.8–1.10,
-- §2, §5.2).

-- A track the user added to the Library. Removed only by an explicit,
-- confirmed, undoable in-app action: nothing here deletes one as a side
-- effect, and a Library track with send history or conflicts can't be
-- deleted until those are dealt with.
--
-- The rekordbox TrackID is not stored: rekordbox reassigns it on import
-- (§5.2). Tracks are matched by Location.
CREATE TABLE library_track (
    id                 INTEGER PRIMARY KEY,
    -- One Library track per track, and deleting the track can't take the
    -- Library track with it.
    recording_id       INTEGER NOT NULL UNIQUE REFERENCES recording (id) ON DELETE RESTRICT,
    -- linked: points at an existing file, which is never written.
    -- copy: a managed copy in the Library folder (Phase 2, 2.6).
    kind               TEXT    NOT NULL DEFAULT 'linked' CHECK (kind IN ('copy', 'linked')),
    -- The file a linked track plays. NULL only while it's missing, e.g. a
    -- rekordbox track whose file was already gone when the Library started.
    linked_file_id     INTEGER REFERENCES file (id) ON DELETE RESTRICT,
    -- A copy's path inside the Library folder.
    managed_rel_path   TEXT
        CHECK (managed_rel_path IS NULL OR (
            managed_rel_path <> '' AND instr(managed_rel_path, '\') = 0
            AND instr(managed_rel_path, ':') = 0
            AND instr('/' || managed_rel_path || '/', '//') = 0
            AND instr('/' || managed_rel_path || '/', '/./') = 0
            AND instr('/' || managed_rel_path || '/', '/../') = 0)),
    format             TEXT,
    -- The file a copy was made from.
    source_file_id     INTEGER REFERENCES file (id) ON DELETE RESTRICT,
    -- missing: the file (linked) or the original (copy) has disappeared.
    source_status      TEXT    NOT NULL DEFAULT 'ok' CHECK (source_status IN ('ok', 'missing')),
    -- The Location this track was last sent to rekordbox at.
    last_sent_location TEXT,
    last_exported_at   TEXT,
    added_at           TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (
        (kind = 'linked' AND managed_rel_path IS NULL
         AND (linked_file_id IS NOT NULL OR source_status = 'missing'))
        OR (kind = 'copy' AND managed_rel_path IS NOT NULL AND linked_file_id IS NULL)
    )
) STRICT;

CREATE INDEX library_track_by_linked_file ON library_track (linked_file_id);
CREATE INDEX library_track_by_source_file ON library_track (source_file_id);

-- The MVP never writes an audio file, so every Library track is linked
-- (CLAUDE.md). Managed copies arrive in Phase 2 (2.6), whose migration
-- drops these two triggers.
CREATE TRIGGER library_track_linked_only_in_phase_1_insert
BEFORE INSERT ON library_track
WHEN new.kind <> 'linked'
BEGIN
    SELECT RAISE(ABORT, 'every Library track is linked until Phase 2');
END;

CREATE TRIGGER library_track_linked_only_in_phase_1_update
BEFORE UPDATE OF kind ON library_track
WHEN new.kind <> 'linked'
BEGIN
    SELECT RAISE(ABORT, 'every Library track is linked until Phase 2');
END;

-- rekordbox's collection as of the last read: a read-only snapshot,
-- replaced on every read. Every TRACK attribute is kept verbatim in
-- `attributes`, because a send fills every attribute with rekordbox's
-- current value (1.9 rule 1); the typed columns are derived from it, so
-- they can't disagree with it.
CREATE TABLE rekordbox_track (
    id                INTEGER PRIMARY KEY,
    -- Every attribute of the TRACK element, as a JSON object of strings,
    -- e.g. {"TrackID": "12", "Location": "file://localhost/E:/a.mp3", …}.
    attributes        TEXT    NOT NULL
        CHECK (json_valid(attributes) AND json_type(attributes) = 'object'),
    -- Valid within this read only: rekordbox reassigns it on import.
    track_id          INTEGER NOT NULL
        GENERATED ALWAYS AS (CAST(json_extract(attributes, '$.TrackID') AS INTEGER)) VIRTUAL,
    -- Raw, as exported (decode it by hand, §5.3).
    location          TEXT    NOT NULL
        GENERATED ALWAYS AS (json_extract(attributes, '$.Location')) VIRTUAL,
    -- The Location decoded by hand and in NFC, as the reader computes it:
    -- what tracks are matched on, and what a confirmed relink is keyed by.
    location_key      TEXT    NOT NULL CHECK (location_key <> ''),
    bpm               REAL
        GENERATED ALWAYS AS (CAST(json_extract(attributes, '$.AverageBpm') AS REAL)) VIRTUAL,
    tonality          TEXT
        GENERATED ALWAYS AS (json_extract(attributes, '$.Tonality')) VIRTUAL,
    play_count        INTEGER
        GENERATED ALWAYS AS (CAST(json_extract(attributes, '$.PlayCount') AS INTEGER)) VIRTUAL,
    rating            INTEGER
        GENERATED ALWAYS AS (CAST(json_extract(attributes, '$.Rating') AS INTEGER)) VIRTUAL,
    colour            TEXT
        GENERATED ALWAYS AS (json_extract(attributes, '$.Colour')) VIRTUAL,
    comments          TEXT
        GENERATED ALWAYS AS (json_extract(attributes, '$.Comments')) VIRTUAL,
    -- The TEMPO (beatgrid) and POSITION_MARK (cues) children, each a JSON
    -- array of attribute objects.
    tempo             TEXT    NOT NULL DEFAULT '[]'
        CHECK (json_valid(tempo) AND json_type(tempo) = 'array'),
    position_marks    TEXT    NOT NULL DEFAULT '[]'
        CHECK (json_valid(position_marks) AND json_type(position_marks) = 'array'),
    -- My Tag names, and the paths of the rekordbox playlists holding it.
    my_tags           TEXT    NOT NULL DEFAULT '[]'
        CHECK (json_valid(my_tags) AND json_type(my_tags) = 'array'),
    playlists         TEXT    NOT NULL DEFAULT '[]'
        CHECK (json_valid(playlists) AND json_type(playlists) = 'array'),
    -- The match to a file and track made for this read (1.2 relink), or
    -- re-applied from a confirmed `relink`. NULL = missing.
    file_id           INTEGER REFERENCES file (id) ON DELETE RESTRICT,
    recording_id      INTEGER REFERENCES recording (id) ON DELETE RESTRICT,
    relink_method     TEXT
        CHECK (relink_method IN ('path', 'filename_duration', 'unique_duration',
                                 'fingerprint', 'filename_only', 'gig_stick', 'user')),
    relink_confidence REAL    CHECK (relink_confidence BETWEEN 0 AND 1),
    read_at           TEXT    NOT NULL,
    -- The TrackID is a whole number as rekordbox wrote it, not a guess.
    CHECK (CAST(track_id AS TEXT) = json_extract(attributes, '$.TrackID')),
    -- A match always says how it was made.
    CHECK ((file_id IS NULL) = (relink_method IS NULL))
) STRICT;

CREATE UNIQUE INDEX rekordbox_track_by_track_id ON rekordbox_track (track_id);
CREATE INDEX rekordbox_track_by_location ON rekordbox_track (location);
CREATE INDEX rekordbox_track_by_location_key ON rekordbox_track (location_key);
CREATE INDEX rekordbox_track_by_file ON rekordbox_track (file_id);
CREATE INDEX rekordbox_track_by_recording ON rekordbox_track (recording_id);

-- rekordbox's values are only ever replaced by a fresh read, never edited:
-- the app's own edits live on the track. The match columns can change
-- (a relink confirmed or corrected).
CREATE TRIGGER rekordbox_track_values_are_read_only
BEFORE UPDATE OF attributes, location_key, tempo, position_marks, my_tags, playlists, read_at
ON rekordbox_track
BEGIN
    SELECT RAISE(ABORT, 'the rekordbox snapshot is read-only; replace it with a fresh read');
END;

-- A relink the user confirmed (1.2: a filename-only match is probable until
-- confirmed). The snapshot is replaced on every read, so confirmations live
-- here, keyed by the Location match key, and each fresh read re-applies
-- them to the rows with that key.
CREATE TABLE relink (
    location_key TEXT    NOT NULL PRIMARY KEY CHECK (location_key <> ''),
    file_id      INTEGER NOT NULL REFERENCES file (id) ON DELETE RESTRICT,
    method       TEXT    NOT NULL
        CHECK (method IN ('path', 'filename_duration', 'unique_duration',
                          'fingerprint', 'filename_only', 'gig_stick', 'user')),
    confidence   REAL    CHECK (confidence BETWEEN 0 AND 1),
    confirmed_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT, WITHOUT ROWID;

CREATE INDEX relink_by_file ON relink (file_id);

-- Each field's value as last sent to rekordbox (1.9 rule 8), for telling
-- "app changed", "rekordbox changed" and "both changed" apart (1.10).
CREATE TABLE sync_base (
    library_track_id INTEGER NOT NULL REFERENCES library_track (id) ON DELETE RESTRICT,
    -- The field as sent, e.g. an XML attribute name such as `Rating`.
    field            TEXT    NOT NULL CHECK (field <> ''),
    value            TEXT    NOT NULL,
    synced_at        TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- One base value per track and field.
    PRIMARY KEY (library_track_id, field)
) STRICT, WITHOUT ROWID;

-- A field both sides changed since the last send: the review queue (1.10).
-- Never settled silently.
CREATE TABLE conflict (
    id               INTEGER PRIMARY KEY,
    library_track_id INTEGER NOT NULL REFERENCES library_track (id) ON DELETE RESTRICT,
    field            TEXT    NOT NULL CHECK (field <> ''),
    app_value        TEXT,
    rekordbox_value  TEXT,
    -- The sync_base value both sides moved away from.
    base_value       TEXT,
    status           TEXT    NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'kept_app', 'kept_rekordbox', 'edited')),
    -- The value chosen when status is `edited`.
    resolved_value   TEXT,
    detected_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    resolved_at      TEXT,
    -- Both sides changed, to different values; otherwise it isn't a
    -- conflict (1.10).
    CHECK (app_value IS NOT base_value AND rekordbox_value IS NOT base_value
           AND app_value IS NOT rekordbox_value),
    CHECK ((status = 'open') = (resolved_at IS NULL)),
    CHECK ((status = 'edited') = (resolved_value IS NOT NULL))
) STRICT;

CREATE INDEX conflict_by_library_track ON conflict (library_track_id);
-- One open conflict per track and field.
CREATE UNIQUE INDEX conflict_one_open_per_field ON conflict (library_track_id, field)
    WHERE status = 'open';
