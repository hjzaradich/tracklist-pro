-- 0055: what the app's own audio model heard in each file (ROADMAP 3.8,
-- 5A-A2b).
--
-- The embed job listens to three 10 s windows of each Library track's file
-- (`audio_model`) and keeps the result here: a fingerprint of 192 numbers to
-- compare tracks by ("Sounds like"), and the scores of the AudioSet music
-- genre and mood labels. Derived state: nothing here is the user's, and the
-- whole table can be dropped and worked out again.
--
-- Keyed like `file_estimate` (0053), not by track: a track can have several
-- files, and the result follows the AUDIO it was made from. `audio_hash` is
-- the audio listened to; a file whose audio_hash still equals it is never
-- listened to again, however its tags or mtime changed, and a file whose
-- audio changed no longer matches (the new audio is listened to, and the
-- stale row replaced). `model_id` names the model, its sha256 prefix and the
-- version of the label list (for example 'ced-tiny@a455633f/labels1'), so a
-- new model or list listens again. Deleted with its file.
--
-- `fingerprint` is 192 f32, little endian (768 bytes). `label_scores` is the
-- kept labels' scores, as the model gives them (before any sigmoid), f32
-- little endian, in the order of the list that `model_id` names.
--
-- The table is carried in a portability bundle: it is computed from the
-- user's own audio and takes hours to make again. A switch in `setting`
-- (`feature_audio_model`, off by default) says whether the job runs; the
-- rows are kept when it is turned off.
CREATE TABLE embedding (
    file_id      INTEGER PRIMARY KEY REFERENCES file (id) ON DELETE CASCADE,
    audio_hash   BLOB    NOT NULL CHECK (length(audio_hash) = 34),
    model_id     TEXT    NOT NULL CHECK (model_id <> ''),
    fingerprint  BLOB    NOT NULL CHECK (length(fingerprint) = 768),
    label_scores BLOB    NOT NULL CHECK (length(label_scores) % 4 = 0),
    embedded_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Files that hold the very same audio can share a result.
CREATE INDEX embedding_audio ON embedding (audio_hash, model_id);

-- A file the model couldn't listen to, so it isn't tried again until its
-- audio (or the model) changes. Per PC: it does not travel in a bundle,
-- since the other PC's decoder may do better.
CREATE TABLE embedding_failure (
    file_id    INTEGER PRIMARY KEY REFERENCES file (id) ON DELETE CASCADE,
    audio_hash BLOB    NOT NULL CHECK (length(audio_hash) = 34),
    model_id   TEXT    NOT NULL CHECK (model_id <> ''),
    reason     TEXT    NOT NULL CHECK (reason IN ('unreadable', 'no_audio', 'unsupported',
                                                  'damaged', 'decoder_crashed', 'model_failed')),
    failed_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
