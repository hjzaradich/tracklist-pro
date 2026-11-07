-- 0054: the key in the analyze job's ledger (ROADMAP 3.7 "Local estimates",
-- "Key"; 3C-E2).
--
-- The key libKeyFinder's method (translated into Rust) finds in the window
-- `file_estimate` was measured from: a pitch class (C = 0 to B = 11) and a
-- mode, with how far the best key stood above the next (0 to 1). 'silence'
-- when the window matched no key; all three NULL when the file's sample
-- rate is too low for the key, or the row was measured before there was a
-- key (an older method_version, measured again).
--
-- Kept always, shown only behind the owner's test switch: a track's
-- `'local'` analysis row holds the key, in Camelot, only while
-- `feature_local_key` is on. Derived state, like the rest of the table.
ALTER TABLE file_estimate ADD COLUMN key_pitch_class INTEGER
    CHECK (key_pitch_class BETWEEN 0 AND 11);

-- The checks use IS, so a missing value can't slip through as NULL.
ALTER TABLE file_estimate ADD COLUMN key_mode TEXT
    CHECK (((key_mode IS NULL OR key_mode IS 'silence') AND key_pitch_class IS NULL)
           OR ((key_mode IS 'major' OR key_mode IS 'minor') AND key_pitch_class IS NOT NULL))
    -- A skipped file has no key.
    CHECK (key_mode IS NULL OR skipped IS NULL);

-- A key comes with its confidence; silence and no key have none.
ALTER TABLE file_estimate ADD COLUMN key_confidence REAL
    CHECK ((key_confidence IS NULL) = (key_pitch_class IS NULL))
    CHECK (key_confidence IS NULL OR key_confidence BETWEEN 0 AND 1);
