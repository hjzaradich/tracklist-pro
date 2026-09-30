-- 0006: a file's id on its volume, the third part of the unchanged check
-- (size, mtime, file id; ROADMAP 1.1, §2). Filled by the walk (1aA-4).
--
-- Text: `<volume serial, 16 hex digits>-<128-bit file id, 32 hex digits>`,
-- lowercase, since it doesn't fit a 64-bit integer. NULL where the
-- filesystem reports no id (some FAT and network drives).
ALTER TABLE file ADD COLUMN file_id TEXT
    CHECK (
        file_id IS NULL OR (
            length(file_id) = 49
            AND substr(file_id, 17, 1) = '-'
            AND NOT ((substr(file_id, 1, 16) || substr(file_id, 18)) GLOB '*[^0-9a-f]*')
        )
    );
