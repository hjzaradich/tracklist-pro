-- 0058: what an online look-up found for a file (ROADMAP 2.2, owner decisions
-- 2026-11-08; Run 6A, S2a).
--
-- The look-up job asks AcoustID (by the file's chromaprint fingerprint) and
-- MusicBrainz (for the recordings and releases AcoustID names) about a file,
-- and keeps the answer here so a screen never waits on the network. The
-- answer belongs to the FILE (its fingerprint and tags are the file's), not
-- to a Library track; the track shows the answer for its linked file.
--
-- A cache: nothing here is the user's, no action changes it through the
-- operation log, and the whole table can be emptied and fetched again. It
-- does not travel in bundles (the other PC asks again). Deleted with its file.
--
-- One row per file. A new look-up replaces the row only when it completes
-- (`found` or `nothing`); one that fails or is skipped leaves a good older
-- row alone and notes itself in `attempt_*`, and replaces a row that is
-- itself only a failure or a skip.
CREATE TABLE identify_lookup (
    file_id    INTEGER PRIMARY KEY REFERENCES file (id) ON DELETE CASCADE,
    -- The services asked, in order, e.g. 'acoustid+musicbrainz'.
    path       TEXT    NOT NULL CHECK (path <> ''),
    fetched_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    outcome    TEXT    NOT NULL CHECK (outcome IN ('found', 'nothing', 'failed', 'skipped')),
    -- Why, as a short code the screen words ('offline', 'key_missing', ...).
    reason     TEXT    CHECK (reason IS NULL OR reason <> ''),
    -- [{"field": "Label", "value": "...", "service": "MusicBrainz", "via": "AcoustID"}]
    candidates TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(candidates)),
    -- The MusicBrainz release ids the candidates came from (covers, later).
    releases   TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(releases)),
    -- A later attempt that failed or was skipped while this row stood.
    attempt_reason TEXT CHECK (attempt_reason IS NULL OR attempt_reason <> ''),
    attempt_at     TEXT,
    -- A failure or a skip always says why.
    CHECK (outcome IN ('found', 'nothing') OR reason IS NOT NULL),
    CHECK ((attempt_reason IS NULL) = (attempt_at IS NULL))
) STRICT;
