-- 0040: a waveform per file (ROADMAP 1.11 "Audio"; 1cB-9).
--
-- The waveform job decodes each file once and keeps a short array of peaks
-- here, so the player can draw the whole track at once without opening the
-- audio again. Derived state: nothing here is the user's, and the whole
-- table can be dropped and worked out again.
--
-- One row per file, kept apart from `file` because a waveform belongs to the
-- AUDIO it was drawn from, not to the file's modified time (ROADMAP 5.1):
-- `audio_hash` is the audio it was computed from, and a file whose
-- audio_hash still equals it is never decoded again, however its tags or
-- mtime changed. `method_version` lets every row be computed again when the
-- method changes. Deleted with its file. Lives in the app database only;
-- nothing is written next to the music.
--
-- `peaks` holds `buckets` pairs of bytes, bucket after bucket: the loudest
-- sample in the bucket, then its RMS level, each 0 to 255 for 0 to full
-- scale (a track gets at most 1024 buckets, so at most 2 KiB).
-- A file whose content couldn't be decoded gets a row with `failure` and no
-- peaks, and isn't tried again until its audio changes.
CREATE TABLE file_waveform (
    file_id        INTEGER PRIMARY KEY REFERENCES file (id) ON DELETE CASCADE,
    -- The file's audio_hash (same 34-byte form) when it was computed.
    audio_hash     BLOB    NOT NULL CHECK (length(audio_hash) = 34),
    method_version INTEGER NOT NULL CHECK (method_version > 0),
    -- The audio actually decoded, in milliseconds.
    duration_ms    INTEGER CHECK (duration_ms >= 0),
    peaks          BLOB    CHECK (length(peaks) > 0 AND length(peaks) % 2 = 0),
    -- Why the content couldn't be decoded at all (the quality job's words).
    failure        TEXT    CHECK (failure IN ('unsupported_format', 'unsupported_codec',
                                              'no_audio', 'damaged', 'decoder_crashed')),
    computed_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Either the peaks, or the reason there are none.
    CHECK ((failure IS NULL) = (peaks IS NOT NULL)),
    CHECK (failure IS NULL OR duration_ms IS NULL),
    CHECK (failure IS NOT NULL OR duration_ms IS NOT NULL)
) STRICT;

-- Files that hold the very same audio share one waveform: the job looks up
-- another file's row by `audio_hash` before it decodes anything.
CREATE INDEX file_waveform_audio ON file_waveform (audio_hash, method_version);
