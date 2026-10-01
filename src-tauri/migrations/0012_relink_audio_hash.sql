-- 0012: the audio a relink match was made to (ROADMAP 1.2, §2).
--
-- A match that carries over between relink runs (a gig stick's) and a
-- confirmed relink both name a file. If that file is
-- later replaced by other audio under the same path, the match is no
-- longer about the audio it was made to. So each keeps the matched file's
-- audio_hash as it was at match time:
--
-- - rekordbox_track.relink_audio_hash: the matched file's audio_hash when
--   the match was made. A carried match whose file now has another
--   audio_hash is dropped and decided again.
-- - relink.audio_hash: the file's audio_hash when the user confirmed it. A
--   confirmation whose file now has another audio_hash is kept, but applied
--   as probable until the user confirms again.
--
-- NULL means no evidence yet (the file wasn't hashed, or the row is older
-- than this migration): it's filled in by the next relink run that finds
-- the file's hash current, and never counts as a change.
ALTER TABLE rekordbox_track ADD COLUMN relink_audio_hash BLOB;
ALTER TABLE relink ADD COLUMN audio_hash BLOB;
