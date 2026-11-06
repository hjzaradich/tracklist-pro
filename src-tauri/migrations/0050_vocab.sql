-- 0050: the custom tag vocabulary (ROADMAP 3.1): `tag_group`, `tag`,
-- `tag_applied`.
--
-- A group is a named heading ("vibe", "role"); a tag belongs to one group and
-- is applied to tracks. A tag is applied to a track (`recording`), not to a
-- Library track, so it follows the track into and out of the Library.
--
-- `tag.rekordbox_name` links a tag to the rekordbox My Tag it was imported
-- from. A My Tag that later disappears from rekordbox stays here, flagged
-- (`gone_from_rekordbox`): the app never deletes a tag by itself.
--
-- Only the operation log inserts, changes and deletes these rows, so ids may
-- be reused: history is linear for them. `tag_applied.recording_id` is
-- RESTRICT, like every table that names a track; the grouping sweep leaves a
-- tagged track alone.
--
-- Colours are `#rrggbb`, or none (the screen draws a plain default).

CREATE TABLE tag_group (
    id       INTEGER PRIMARY KEY,
    name     TEXT    NOT NULL UNIQUE CHECK (name <> ''),
    colour   TEXT    CHECK (colour IS NULL OR (length(colour) = 7 AND colour GLOB '#[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]')),
    position INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE TABLE tag (
    id                  INTEGER PRIMARY KEY,
    group_id            INTEGER NOT NULL REFERENCES tag_group (id) ON DELETE RESTRICT,
    name                TEXT    NOT NULL UNIQUE CHECK (name <> ''),
    colour              TEXT    CHECK (colour IS NULL OR (length(colour) = 7 AND colour GLOB '#[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]')),
    position            INTEGER NOT NULL DEFAULT 0,
    -- The My Tag this tag was copied from, exactly as rekordbox spells it.
    rekordbox_name      TEXT    UNIQUE CHECK (rekordbox_name IS NULL OR rekordbox_name <> ''),
    -- The last import did not find the My Tag in rekordbox.
    gone_from_rekordbox INTEGER NOT NULL DEFAULT 0 CHECK (gone_from_rekordbox IN (0, 1)),
    CHECK (gone_from_rekordbox = 0 OR rekordbox_name IS NOT NULL)
) STRICT;

CREATE INDEX tag_by_group ON tag (group_id);

CREATE TABLE tag_applied (
    id           INTEGER PRIMARY KEY,
    recording_id INTEGER NOT NULL REFERENCES recording (id) ON DELETE RESTRICT,
    tag_id       INTEGER NOT NULL REFERENCES tag (id) ON DELETE RESTRICT,
    applied_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (recording_id, tag_id)
) STRICT;

CREATE INDEX tag_applied_by_tag ON tag_applied (tag_id);
