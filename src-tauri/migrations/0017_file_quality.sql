-- 0017: quality measurements per file (ROADMAP 1.6, 1bA-10 and 1bA-11).
--
-- The quality job measures each file's spectral cutoff, the duration it
-- actually decodes against the one its header claims, and its decode
-- errors. These are measurements only; verdicts come later (1bB-6).
--
-- One row per file, kept apart from `file` because a measurement belongs to
-- the AUDIO it was made from, not to the file's modified time (ROADMAP
-- 5.1): `audio_hash` is the audio it was measured from, and a file whose
-- audio_hash still equals it is never measured again, however its tags or
-- mtime changed. `method_version` lets every row be recomputed when the
-- method changes. Deleted with its file.
CREATE TABLE file_quality (
    file_id        INTEGER PRIMARY KEY REFERENCES file (id) ON DELETE CASCADE,
    -- The file's audio_hash (same 34-byte form) when it was measured.
    audio_hash     BLOB    NOT NULL CHECK (length(audio_hash) = 34),
    method_version INTEGER NOT NULL CHECK (method_version > 0),
    -- The highest frequency within 50 dB of the 2-8 kHz level, in Hz.
    -- NULL when it can't be measured; `cutoff_gap` then says why.
    cutoff_hz      INTEGER CHECK (cutoff_hz > 0),
    cutoff_gap     TEXT    CHECK (cutoff_gap IN ('too_short', 'silent', 'low_rate')),
    -- The duration actually decoded, and the one the header claims (NULL if
    -- the header gives none), in milliseconds.
    decoded_ms     INTEGER CHECK (decoded_ms >= 0),
    header_ms      INTEGER CHECK (header_ms >= 0),
    -- Packets that failed to decode, in all, and by kind as a JSON object
    -- (e.g. {"decode": 3, "container": 1}); NULL when there were none.
    decode_errors  INTEGER NOT NULL DEFAULT 0 CHECK (decode_errors >= 0),
    error_kinds    TEXT    CHECK (error_kinds IS NULL
                                  OR (json_valid(error_kinds) AND json_type(error_kinds) = 'object')),
    -- How the stream ended: to its end, cut off mid-stream (the file
    -- stopped early), or abandoned after too many errors in a row.
    ended          TEXT    CHECK (ended IN ('complete', 'cut_short', 'gave_up')),
    -- Why the content couldn't be decoded at all. Every measurement is
    -- NULL then, and the file isn't tried again until its audio changes.
    failure        TEXT    CHECK (failure IN ('unsupported_format', 'unsupported_codec',
                                              'no_audio', 'damaged', 'decoder_crashed')),
    measured_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- A cutoff gap is a reason for a missing cutoff, and a failed file has
    -- no measurement of any kind.
    CHECK (cutoff_gap IS NULL OR cutoff_hz IS NULL),
    CHECK (failure IS NULL
           OR (cutoff_hz IS NULL AND cutoff_gap IS NULL AND decoded_ms IS NULL
               AND header_ms IS NULL AND decode_errors = 0 AND error_kinds IS NULL
               AND ended IS NULL)),
    -- A decoded file has its durations and ending, and a cutoff or the
    -- reason it has none.
    CHECK (failure IS NOT NULL
           OR (decoded_ms IS NOT NULL AND ended IS NOT NULL
               AND (cutoff_hz IS NOT NULL OR cutoff_gap IS NOT NULL)))
) STRICT;

-- Nothing reads or writes `file.cutoff_hz`; the cutoff lives in
-- `file_quality`, the one home for the measurement. (`quality_verdict`
-- stays: 1bB-6 fills it.)
ALTER TABLE file DROP COLUMN cutoff_hz;
