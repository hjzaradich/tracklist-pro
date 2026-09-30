-- 0002: where files are (ROADMAP §0.3, §2): volumes, the music folders on
-- them, and every audio file found in those folders.
--
-- Conventions for every Phase 1 table:
-- - STRICT, so a value of the wrong type is refused instead of stored.
-- - Enums are lowercase TEXT with a CHECK listing every allowed value.
-- - Booleans are INTEGER 0/1 with a CHECK.
-- - Timestamps are UTC ISO-8601 TEXT, e.g. 2026-09-28T14:03:07.123Z.
-- - A relative path uses `/` only, never starts or ends with `/`, and has
--   no empty, `.` or `..` component, no `\` and no `:`, so it can never
--   climb out of its folder or name a drive (paths::RelPath).

-- A disk or share, identified in a way that survives drive-letter changes.
CREATE TABLE volume (
    id              INTEGER PRIMARY KEY,
    -- volume::VolumeId: `serial=…`, `guid={…}`, `unc=\\server\share`, or
    -- `serial=…+guid={…}` for a clone. Never a drive letter. Read it back
    -- with VolumeId::from_stored, which checks the full form.
    identity        TEXT    NOT NULL UNIQUE
        CHECK (
            (identity GLOB 'serial=?*' OR identity GLOB 'guid={?*}'
             OR identity GLOB 'unc=\\?*' OR identity GLOB 'dev=?*')
            AND instr(identity, ':') = 0
        ),
    label           TEXT    NOT NULL DEFAULT '',
    -- The volume GUID last seen for it on this PC. With `label`, it decides
    -- which of two clones keeps the identity (volume::tell_clones_apart).
    guid            TEXT
        CHECK (guid IS NULL OR guid GLOB '{?*}'),
    kind            TEXT    NOT NULL
        CHECK (kind IN ('internal', 'external', 'network')),
    -- Where it was mounted last time, e.g. `E:\`. Shown for an offline
    -- volume; never used to find files.
    last_mount_path TEXT,
    last_seen_at    TEXT
) STRICT;

-- A folder the user pointed the app at.
CREATE TABLE music_folder (
    id           INTEGER PRIMARY KEY,
    volume_id    INTEGER NOT NULL REFERENCES volume (id) ON DELETE RESTRICT,
    -- From the volume's mount point. '' is the root of the volume.
    rel_path     TEXT    NOT NULL
        CHECK (
            instr(rel_path, '\') = 0 AND instr(rel_path, ':') = 0
            AND (rel_path = '' OR (
                instr('/' || rel_path || '/', '//') = 0
                AND instr('/' || rel_path || '/', '/./') = 0
                AND instr('/' || rel_path || '/', '/../') = 0))
        ),
    -- rel_path in NFC, for matching (§0.3).
    rel_path_key TEXT    NOT NULL
        CHECK (
            instr(rel_path_key, '\') = 0 AND instr(rel_path_key, ':') = 0
            AND (rel_path_key = '' OR (
                instr('/' || rel_path_key || '/', '//') = 0
                AND instr('/' || rel_path_key || '/', '/./') = 0
                AND instr('/' || rel_path_key || '/', '/../') = 0))
        ),
    watch        INTEGER NOT NULL DEFAULT 0 CHECK (watch IN (0, 1)),
    role         TEXT    NOT NULL DEFAULT 'scan' CHECK (role IN ('scan', 'inbox')),
    added_at     TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- NFC and NFD spellings are two folders on NTFS, so uniqueness is on
    -- the on-disk spelling.
    UNIQUE (volume_id, rel_path)
) STRICT;

-- One audio file on disk. The app never writes to it (CLAUDE.md), and a
-- file that vanishes is marked present = 0, never dropped (ROADMAP 1.1).
CREATE TABLE file (
    id              INTEGER PRIMARY KEY,
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
    cutoff_hz       INTEGER CHECK (cutoff_hz > 0),
    quality_verdict TEXT
        CHECK (quality_verdict IN
            ('ok', 'low_bitrate', 'suspect_transcode', 'clipped', 'truncated', 'broken')),
    -- Every tag as read, as a JSON object.
    raw_tags        TEXT
        CHECK (raw_tags IS NULL OR (json_valid(raw_tags) AND json_type(raw_tags) = 'object')),
    present         INTEGER NOT NULL DEFAULT 1 CHECK (present IN (0, 1)),
    hidden_reason   TEXT    CHECK (hidden_reason IN ('user', 'inbox_cleared')),
    first_seen_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_seen_at    TEXT,
    UNIQUE (music_folder_id, rel_path)
) STRICT;

CREATE INDEX file_by_rel_path_key ON file (rel_path_key);
CREATE INDEX file_by_blake3 ON file (blake3) WHERE blake3 IS NOT NULL;
CREATE INDEX file_by_audio_hash ON file (audio_hash) WHERE audio_hash IS NOT NULL;
