-- 0053: the app's own measurements of each file's audio (ROADMAP 3.7 "Local
-- estimates"; 3C-E1a).
--
-- The analyze job decodes a window from the middle of each Library track's
-- file and keeps what it measured here: the ledger of estimates. Derived
-- state: nothing here is the user's, and the whole table can be dropped and
-- worked out again.
--
-- One row per file, keyed by the AUDIO it was measured from, like the
-- waveforms (0040): `audio_hash` is the audio it came from, and a file whose
-- audio_hash still equals it is never decoded again, however its tags or
-- mtime changed. `method_version` lets every row be measured again when the
-- method changes. Deleted with its file. Lives in the app database only;
-- nothing is written next to the music.
--
-- What is shown comes from here, never the other way round: a track's
-- `'local'` analysis row holds the BPM from this ledger, folded into the
-- user's BPM range, and loses to every other source (rekordbox, tags, Mixed
-- In Key, the user). Energy never goes into `analysis` from here (owner,
-- 2026-11-06): the energy features and the suggestion stay in this table,
-- shown only as a suggestion where the user sets energy.
--
-- Columns E1b fills are here already, NULL until then: the energy features
-- (loudness, onset density, spectral flux) and the suggestion computed from
-- them across the Library.
--
-- A file that couldn't be measured gets a row with `skipped` and no
-- measurements, and isn't tried again until its audio or the method changes.
-- A file with audio but no beat the detector trusts has a row with no BPM.
CREATE TABLE file_estimate (
    file_id           INTEGER PRIMARY KEY REFERENCES file (id) ON DELETE CASCADE,
    -- The file's audio_hash (same 34-byte form) when it was measured.
    audio_hash        BLOB    NOT NULL CHECK (length(audio_hash) = 34),
    method_version    INTEGER NOT NULL CHECK (method_version > 0),
    -- The audio actually measured (the window), in milliseconds.
    analysed_ms       INTEGER CHECK (analysed_ms >= 0),
    -- The detector's tempo before any folding into the user's range, and
    -- how sure it is, 0 to 1.
    bpm               REAL    CHECK (bpm > 0 AND bpm < 1000),
    bpm_confidence    REAL    CHECK (bpm_confidence BETWEEN 0 AND 1),
    -- E1b: integrated loudness (LUFS), onsets per second, mean spectral flux.
    loudness_lufs     REAL    CHECK (loudness_lufs <= 0),
    onset_density     REAL    CHECK (onset_density >= 0),
    spectral_flux     REAL    CHECK (spectral_flux >= 0),
    -- E1b: the energy the app would suggest, 1 to 10. A suggestion only.
    energy_suggestion INTEGER CHECK (energy_suggestion BETWEEN 1 AND 10),
    -- Why there are no measurements: the content couldn't be decoded (the
    -- quality job's words), too little audio, or nothing but silence.
    skipped           TEXT    CHECK (skipped IN ('unsupported_format', 'unsupported_codec',
                                                 'no_audio', 'damaged', 'decoder_crashed',
                                                 'too_short', 'silent')),
    analysed_at       TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- A BPM comes with its confidence.
    CHECK ((bpm IS NULL) = (bpm_confidence IS NULL)),
    -- Either a window was measured, or the reason none was.
    CHECK ((skipped IS NULL) = (analysed_ms IS NOT NULL)),
    CHECK (skipped IS NULL OR (bpm IS NULL AND loudness_lufs IS NULL AND onset_density IS NULL
                               AND spectral_flux IS NULL AND energy_suggestion IS NULL))
) STRICT;

-- Files that hold the very same audio share one measurement: the job looks
-- up another file's row by `audio_hash` before it decodes anything.
CREATE INDEX file_estimate_audio ON file_estimate (audio_hash, method_version);
