-- 0033: a quality verdict no longer makes a track due for the track table
-- (1cA-4).
--
-- The Quality problem filter reads Review's definition when the query runs,
-- so a made, changed or dropped verdict changes no stored row. Bit 8 of
-- `track_row.flags` ("its file has a quality finding") is unused. Rows
-- written before this had it set are cleared here.
DROP TRIGGER IF EXISTS search_verdict_added;
DROP TRIGGER IF EXISTS search_verdict_changed;
DROP TRIGGER IF EXISTS search_verdict_removed;

UPDATE track_row SET flags = flags & ~8 WHERE flags & 8 != 0;
