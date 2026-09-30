-- 0008: scan follow-ups (1aB-8, 1aB-14).
--
-- file.online_only: the walk found the file "online only" (a OneDrive
-- placeholder: RECALL_ON_DATA_ACCESS, RECALL_ON_OPEN or OFFLINE in the
-- directory listing). Reading its bytes would download it, so no later
-- stage reads it unless the user opts in (ROADMAP 1.1, §5.5).
ALTER TABLE file ADD COLUMN online_only INTEGER NOT NULL DEFAULT 0
    CHECK (online_only IN (0, 1));

-- What the last finished walk of a music folder couldn't read: folders it
-- couldn't list, and audio files it couldn't look at or whose names can't
-- be stored. NULL until the folder has been walked to the end.
ALTER TABLE music_folder ADD COLUMN walked_at TEXT;
ALTER TABLE music_folder ADD COLUMN unreadable_folders INTEGER
    CHECK (unreadable_folders >= 0);
ALTER TABLE music_folder ADD COLUMN unreadable_files INTEGER
    CHECK (unreadable_files >= 0);
