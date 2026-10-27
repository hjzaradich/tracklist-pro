-- 0034: settings for how the app looks, not for what it holds (1cA-5).
--
-- The first is which columns each track table shows, and in what order. They
-- are the user's taste, not Library data: nothing here goes in the operation
-- log or can be undone, and nothing clears it (not "start fresh" either).
-- Each value is JSON; a value this version doesn't understand reads as the
-- default.
CREATE TABLE ui_setting (
    key        TEXT NOT NULL PRIMARY KEY CHECK (key <> ''),
    value      TEXT NOT NULL CHECK (json_valid(value)),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT, WITHOUT ROWID;
