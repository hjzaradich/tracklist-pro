-- 0009: a partial hash of each file's content, for the unchanged check
-- (1aC-1; ROADMAP 1.1, §5.1).
--
-- rekordbox bumps the modified time of files it adds or analyzes, so a new
-- mtime alone doesn't mean new audio. The hash stage stores this while it
-- reads the whole file; a later walk that finds only the mtime changed
-- reads it again and compares (hash/partial.rs).
--
-- 33 bytes: a definition byte, then a 32-byte digest of the file's length,
-- all its metadata (every byte outside the audio) and the first and last
-- 64 KiB of the audio. NULL until the hash stage has run, and for a file
-- with no shortcut (unknown format, Ogg, more than 4 MiB of metadata): a
-- touched file with none counts as changed.
ALTER TABLE file ADD COLUMN partial_hash BLOB
    CHECK (partial_hash IS NULL OR length(partial_hash) = 33);
