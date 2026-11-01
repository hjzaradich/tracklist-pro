-- 0045: the keys the deciding run looks a file's names up by, stored (1cD-G1;
-- ROADMAP 1.4 "The name index").
--
-- The deciding run finds which files to compare through an index in memory:
-- each file's titles, a mashup's parts and its artists, as lookup keys. That
-- index was built by reading every present file's whole `raw_tags`, so a
-- restart that found one new file cost about a minute at 100,000 files
-- (1cC-M1). One row here is what that work gave for one file, so a restart
-- reads the rows instead.
--
--   rev            the `grouping_file.rev` the keys were worked out at. The
--                  triggers of 0022 change `rev` whenever a file's tags or
--                  its name change, and when its audio changes or it comes
--                  back, whichever code made the change. A row at another
--                  `rev` isn't read: the keys are worked out again.
--   rules_version  `grouping::RULES_VERSION` when they were worked out. The
--                  keys come from the version parser, so a row of other
--                  rules isn't read either.
--   entry          the keys: a JSON array of three arrays (titles, parts,
--                  artists), each sorted.
--
-- Derived data, never decided on: the index only says which files to compare,
-- and each decision reads the tags themselves. Nothing here is in the
-- operation log. A row goes with the file's record (`grouping_file`).
--
-- The table starts empty, and no trigger or job needs a row to exist: a file
-- with no current row has its keys worked out from `raw_tags` by the first
-- deciding run that needs them, one page at a time, as before this migration,
-- and they are kept from then on. An upgrade pays that once, in a run that
-- shows its progress and writes for a moment per page.
CREATE TABLE grouping_names (
    file_id       INTEGER NOT NULL PRIMARY KEY
                  REFERENCES grouping_file (file_id) ON DELETE CASCADE,
    rev           INTEGER NOT NULL,
    rules_version INTEGER NOT NULL,
    entry         TEXT    NOT NULL CHECK (json_valid(entry))
) STRICT, WITHOUT ROWID;
