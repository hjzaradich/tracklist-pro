//! The genre commands. Edits run on the writer and are undoable operations;
//! reads go through the read pool.

use tauri::State;

use super::edit::{self, GenreError};
use super::unmapped::{self, GenreTree, Spelling, UnmappedGenres};
use super::AliasKind;
use crate::db::{ReadPool, Writer};
use crate::ipc::{ErrorKind, ErrorParam, IpcError};

impl From<GenreError> for IpcError {
    fn from(e: GenreError) -> Self {
        match e {
            GenreError::NotFound => IpcError::new(ErrorKind::GenreNotFound),
            GenreError::NameEmpty => IpcError::new(ErrorKind::GenreNameEmpty),
            GenreError::NameTaken { existing } => IpcError::with(
                ErrorKind::GenreNameTaken,
                [("name", ErrorParam::from(existing))],
            ),
            GenreError::MoveIntoItself => IpcError::new(ErrorKind::GenreMoveIntoItself),
            GenreError::AliasTaken { genre } => IpcError::with(
                ErrorKind::GenreAliasTaken,
                [("name", ErrorParam::from(genre))],
            ),
            GenreError::NameIsAlias { genre } => IpcError::with(
                ErrorKind::GenreAliasTaken,
                [("name", ErrorParam::from(genre))],
            ),
            GenreError::TextTooLong => IpcError::new(ErrorKind::GenreTextTooLong),
            GenreError::Ops(e) => IpcError::from(e),
        }
    }
}

/// The genre tree with its aliases and the Library tracks per genre.
#[tauri::command]
#[specta::specta]
pub async fn genre_tree(reads: State<'_, ReadPool>) -> Result<GenreTree, IpcError> {
    Ok(reads.read(unmapped::tree)?)
}

/// Every genre spelling the Library uses, with the genre and the rule it
/// resolves by (none: unmapped).
#[tauri::command]
#[specta::specta]
pub async fn genre_spellings(reads: State<'_, ReadPool>) -> Result<Vec<Spelling>, IpcError> {
    Ok(reads.read(unmapped::spellings)?)
}

/// Review's group: tracks with no genre, then unmapped spellings.
#[tauri::command]
#[specta::specta]
pub async fn genre_unmapped(reads: State<'_, ReadPool>) -> Result<UnmappedGenres, IpcError> {
    Ok(reads.read(unmapped::unmapped)?)
}

/// Adds a genre under `parent`, or as a branch of its own.
#[tauri::command]
#[specta::specta]
pub async fn genre_add(
    writer: State<'_, Writer>,
    name: String,
    parent: Option<i64>,
) -> Result<i64, IpcError> {
    Ok(writer.call(move |conn| Ok(edit::add_node(conn, parent, &name)))???)
}

/// Renames a genre. Answers the operation recorded, none if nothing changed.
#[tauri::command]
#[specta::specta]
pub async fn genre_rename(
    writer: State<'_, Writer>,
    id: i64,
    name: String,
) -> Result<Option<i64>, IpcError> {
    Ok(writer.call(move |conn| Ok(edit::rename_node(conn, id, &name)))???)
}

/// Moves a genre and everything below it under `parent` (none: the top).
#[tauri::command]
#[specta::specta]
pub async fn genre_move(
    writer: State<'_, Writer>,
    id: i64,
    parent: Option<i64>,
) -> Result<Option<i64>, IpcError> {
    Ok(writer.call(move |conn| Ok(edit::move_node(conn, id, parent)))???)
}

/// Makes a spelling an alias of a genre.
#[tauri::command]
#[specta::specta]
pub async fn genre_add_alias(
    writer: State<'_, Writer>,
    genre_id: i64,
    alias: String,
    kind: AliasKind,
) -> Result<i64, IpcError> {
    Ok(writer.call(move |conn| Ok(edit::add_alias(conn, genre_id, &alias, kind)))???)
}

/// Removes an alias.
#[tauri::command]
#[specta::specta]
pub async fn genre_remove_alias(
    writer: State<'_, Writer>,
    id: i64,
) -> Result<Option<i64>, IpcError> {
    Ok(writer.call(move |conn| Ok(edit::remove_alias(conn, id)))???)
}

/// Adds the proposed starter genres that aren't there yet; answers how many.
#[tauri::command]
#[specta::specta]
pub async fn genre_load_seed(writer: State<'_, Writer>) -> Result<u32, IpcError> {
    Ok(writer.call(move |conn| Ok(edit::load_seed(conn)))???)
}
