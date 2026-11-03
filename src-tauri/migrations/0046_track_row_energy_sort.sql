-- 0046: an index to sort the track table by the energy a track shows (1cD-XE;
-- ROADMAP 1.11 "The track table").
--
-- The shown energy already sits in four bits of `track_row.flags`
-- (`search::flag::ENERGY`, 3840; 0 is none). Sorting by a bit expression with
-- no index would read and sort every row, so this index holds the same two
-- expressions the sort asks for, in the same form as the other columns'
-- indexes: "has no energy" first (so the tracks with none come last, whichever
-- way the column goes), then the energy, then the id. `NULLIF(.., 0)` makes
-- none a blank like the other columns' NULL. The expression must stay
-- identical to `SortColumn::Energy` in `track_table`; a test reads its plan.
--
-- No column or trigger changes: nothing here marks a track due.
CREATE INDEX track_row_energy ON track_row (
    (NULLIF(flags & 3840, 0) IS NULL),
    NULLIF(flags & 3840, 0),
    recording_id,
    flags,
    format
);
