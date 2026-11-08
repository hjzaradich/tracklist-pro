-- 0056: the genre model learned from the user's own tracks (ROADMAP 3.8,
-- 5B-B3a).
--
-- While the audio model is on, the app learns, from the 192-number
-- fingerprints in `embedding` (0055) and the canonical genre nodes in
-- `track_row.genre_node` (0049), what each of the user's genres sounds like:
-- one logistic regression per genre node that has at least 10 such tracks
-- (`audio_model::genre_model`). Derived state: nothing here is the user's
-- own data, and the tables can be emptied and learned again from the rows
-- they come from.
--
-- `genre_model` holds at most one row, the state of the last learning:
-- - `model_id` is the embedding model whose fingerprints were learned from
--   (as in `embedding.model_id`); weights of another model are never used.
-- - `signature` names exactly what was learned from (the model, the version
--   of the method, and every track and genre node used with the identity of
--   its fingerprint). The model is learned again only when it changes.
-- - `mean` and `scale` are the preprocessing: each fingerprint number has
--   `mean` taken off and is multiplied by `scale` (one over its standard
--   deviation) before the weights apply. 192 f32, little endian (768 bytes),
--   or empty when there was too little to learn from.
-- - `genres` and `tracks` count what was learned from. A row with no genres
--   is the answer "too little to learn from" (fewer than two genres with 10
--   tracks), kept so that answer is not worked out again until the signature
--   changes.
--
-- `genre_model_node` holds one row per genre node learned: `weights` are
-- 192 f32, little endian (768 bytes), in the preprocessed space, with the
-- `bias` beside them; the score is a logit (the app shows its sigmoid).
-- `trained_on` is the number of tracks of that genre learned from. Deleted
-- with its genre node.
--
-- Both tables travel in a portability bundle, like `embedding`; a model
-- from another embedding model, or whose signature no longer matches, is
-- learned again on the other PC.
CREATE TABLE genre_model (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    model_id   TEXT    NOT NULL CHECK (model_id <> ''),
    signature  TEXT    NOT NULL CHECK (signature <> ''),
    mean       BLOB    NOT NULL CHECK (length(mean) IN (0, 768)),
    scale      BLOB    NOT NULL CHECK (length(scale) IN (0, 768)),
    genres     INTEGER NOT NULL CHECK (genres >= 0),
    tracks     INTEGER NOT NULL CHECK (tracks >= 0),
    learned_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE genre_model_node (
    genre_id   INTEGER PRIMARY KEY REFERENCES genre (id) ON DELETE CASCADE,
    weights    BLOB    NOT NULL CHECK (length(weights) = 768),
    bias       REAL    NOT NULL,
    trained_on INTEGER NOT NULL CHECK (trained_on >= 1),
    model_id   TEXT    NOT NULL CHECK (model_id <> ''),
    learned_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
