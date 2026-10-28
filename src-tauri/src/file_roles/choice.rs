//! The files the user chose (`file_choice`, migration 0037; ROADMAP 1.11
//! "Use this file instead"), read into the ranking as
//! [`crate::best_file::FileFacts::chosen`].
//!
//! # What a choice covers
//!
//! A choice names audio, not a file or a track: the `audio_hash` of the file
//! chosen (its `blake3` when it has none), and its `blake3` when it had one.
//! Of the files of a track that are on disk:
//!
//! - those holding that audio are the candidates; a file that is encoded
//!   again holds other audio, so it is not covered;
//! - when the choice kept a `blake3` and a candidate still has these exact
//!   bytes, that file alone is chosen (two copies that differ only in tags
//!   share the audio);
//! - otherwise every candidate is chosen (the file was retagged, or the
//!   choice kept no whole-file hash), and the ranking tells them apart.
//!
//! A track in the Library has no choice at all (owner decision, 2026-10-28:
//! this is for tracks outside it): [`super::facts`] marks nothing for it,
//! so the ranking and Review's rules work as if there were none. The row
//! stays, and applies again if the track leaves the Library.
//!
//! A retag of a chosen file moves the choice to the file's new bytes (a
//! trigger, migration 0037), so the choice keeps naming that file even when
//! another copy of the track holds the same audio. The "every candidate"
//! case above is what is left when no file has the bytes any more (the file
//! was replaced by another one, or hashed for the first time).
//!
//! A file that is not on disk is never chosen: **when the chosen file is
//! gone, the ranking decides again**, and the choice stays, so it takes
//! effect again if the file comes back (or a file with the same audio is
//! there). Files holding the very same bytes can't be told apart; the
//! ranking decides among them.

use std::collections::HashMap;

use rusqlite::Connection;

/// What the ranking needs of a file to tell whether a choice covers it.
pub(crate) struct Held {
    pub on_disk: bool,
    /// The file's `audio_hash`, else its `blake3` (`grouping::CONTENT_KEY`).
    pub key: Option<Vec<u8>>,
    pub blake3: Option<Vec<u8>>,
}

/// The key a file's choices are kept under: its `audio_hash`, else its
/// `blake3`.
pub(crate) fn content_key(
    audio_hash: Option<Vec<u8>>,
    blake3: Option<&Vec<u8>>,
) -> Option<Vec<u8>> {
    audio_hash
        .filter(|h| !h.is_empty())
        .or_else(|| blake3.cloned())
}

/// Every choice, by the key it is kept under: the `blake3` each kept (empty
/// when it kept none). A choice is made by hand, so there are few.
pub(crate) struct Choices(HashMap<Vec<u8>, Vec<Vec<u8>>>);

impl Choices {
    pub fn load(conn: &Connection) -> rusqlite::Result<Choices> {
        let mut by_key: HashMap<Vec<u8>, Vec<Vec<u8>>> = HashMap::new();
        let mut stmt = conn.prepare_cached("SELECT content_key, blake3 FROM file_choice")?;
        for row in stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))? {
            let (key, blake3) = row?;
            by_key.entry(key).or_default().push(blake3);
        }
        Ok(Choices(by_key))
    }

    /// Whether there is no choice at all.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Which of one track's `files` the choices cover.
    pub fn chosen(&self, files: &[Held]) -> Vec<bool> {
        let mut chosen = vec![false; files.len()];
        if self.0.is_empty() {
            return chosen;
        }
        let mut keys: Vec<&Vec<u8>> = files.iter().filter_map(|f| f.key.as_ref()).collect();
        keys.sort_unstable();
        keys.dedup();
        for key in keys {
            let Some(kept) = self.0.get(key) else {
                continue;
            };
            let candidates: Vec<usize> = (0..files.len())
                .filter(|&i| files[i].on_disk && files[i].key.as_ref() == Some(key))
                .collect();
            for blake3 in kept {
                let exact = candidates
                    .iter()
                    .copied()
                    .filter(|&i| !blake3.is_empty() && files[i].blake3.as_ref() == Some(blake3));
                let exact: Vec<usize> = exact.collect();
                for &i in if exact.is_empty() {
                    &candidates
                } else {
                    &exact
                } {
                    chosen[i] = true;
                }
            }
        }
        chosen
    }
}
