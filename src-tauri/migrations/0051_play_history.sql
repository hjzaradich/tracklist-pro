-- 0051: play history (ROADMAP 3.6, §2): `session`, `play_event`.
--
-- A session is one set as a source gave it: for now one m3u8 file exported
-- from rekordbox's History. `played_on` is the day it was played: a date in
-- the file's name if it has one, otherwise the day it was imported; the
-- user can correct it. 'stick' is allowed for the gig-stick history that
-- comes later (H2); nothing writes it yet.
--
-- A play keeps what the source said (path, artist, title) as read, and the
-- track it was matched to by path, if any. A play of a file the user
-- doesn't have (a partner's track in a B2B) stays a play with no track.
-- `position` is its place in the session, from 0, never renumbered.
--
-- AUTOINCREMENT on both: every write goes through the operation log, and a
-- row Undo deleted must never have its id taken by another.
--
-- The play counts and last played that rekordbox gives (1.2) are left as
-- they are: these tables are a record of sets, not a counter.

CREATE TABLE session (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    source      TEXT    NOT NULL CHECK (source IN ('m3u8', 'stick')),
    name        TEXT    NOT NULL CHECK (trim(name) <> ''),
    played_on   TEXT    NOT NULL CHECK (played_on IS date(played_on)),
    imported_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE play_event (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id   INTEGER NOT NULL REFERENCES session (id) ON DELETE CASCADE,
    position     INTEGER NOT NULL CHECK (position >= 0),
    recording_id INTEGER REFERENCES recording (id) ON DELETE SET NULL,
    raw_path     TEXT    NOT NULL CHECK (raw_path <> ''),
    raw_artist   TEXT    CHECK (raw_artist IS NULL OR trim(raw_artist) <> ''),
    raw_title    TEXT    CHECK (raw_title IS NULL OR trim(raw_title) <> ''),
    UNIQUE (session_id, position)
) STRICT;

CREATE INDEX play_event_recording ON play_event (recording_id)
    WHERE recording_id IS NOT NULL;
