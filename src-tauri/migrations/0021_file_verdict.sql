-- 0021: the stored quality verdict per file (ROADMAP 1.6; 1bC-5).
--
-- `quality::verdict::assess` turns what's known about a file (its codec,
-- bitrate and sample rate, the read stage's verdict, its `file_quality`
-- row) into a verdict and the findings behind it. This table keeps the
-- answer, so the grouping job and the best-file ranking read it instead of
-- working it out again.
--
-- It is a table of its own because `file.quality_verdict` is the READ
-- STAGE's verdict (`truncated` / `broken`, cleared by every read) and an
-- input to `assess`: writing the answer there would feed it back in.
--
-- Derived state: every row can be rebuilt from the file's own columns, so
-- nothing here is a fact. A row is current only while all of these hold,
-- and the quality module recomputes it the moment one doesn't:
--   * `rules_version` is the version of `assess`'s rules the build has;
--   * `inputs` is what the inputs read now (a key over `file`'s audio_hash,
--     codec, bitrate and sample_rate, the read stage's verdict, and the
--     `file_quality` row), so a new measurement, a re-read, a new audio
--     hash or a changed read-stage verdict each make it stale.
-- A file with no row, or a stale one, has no verdict yet; that is never
-- read as `ok`. A row whose `verdict` is NULL is a file that was assessed
-- and has nothing to say (not measured, nothing in its headers flags it).
--
-- Deleted with its file.
CREATE TABLE file_verdict (
    file_id       INTEGER PRIMARY KEY REFERENCES file (id) ON DELETE CASCADE,
    rules_version INTEGER NOT NULL CHECK (rules_version > 0),
    -- The key over the inputs `assess` read, as the quality module builds
    -- it (a JSON array of the quoted values).
    inputs        TEXT    NOT NULL,
    -- `clipped` is held back: nothing measures it.
    verdict       TEXT    CHECK (verdict IN ('ok', 'low_bitrate', 'suspect_transcode',
                                             'truncated', 'broken')),
    -- Every finding that applied, as a JSON array of {"code": ..., ...}.
    findings      TEXT    NOT NULL DEFAULT '[]'
                          CHECK (json_valid(findings) AND json_type(findings) = 'array'),
    assessed_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- No verdict, no findings: `assess` gives nothing for a file it can't
    -- say anything about.
    CHECK (verdict IS NOT NULL OR findings = '[]')
) STRICT;
