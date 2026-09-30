-- 0010: the audio_hash a fingerprint was computed from (1aC-10).
--
-- rekordbox and taggers rewrite tags, which changes a file's size and mtime
-- but not its audio. When the hash stage is current for a file and its
-- audio_hash still equals this, the stored fingerprint still stands, so the
-- fingerprint stage carries it forward instead of decoding the file again
-- (fingerprint/ledger.rs).
--
-- The same 34-byte form as file.audio_hash. NULL when the fingerprint was
-- made while the hash stage wasn't current for the file (its audio_hash
-- couldn't be vouched for), when the file has no audio_hash, and when the
-- file has no fingerprint. NULL never allows a skip.
ALTER TABLE file ADD COLUMN fingerprint_audio_hash BLOB
    CHECK (fingerprint_audio_hash IS NULL OR length(fingerprint_audio_hash) = 34);
