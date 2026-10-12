//! The conflict engine's tests. Every name here is invented.
//!
//! - `table`: one test per row of the decision table (module docs), the
//!   order of the rows, and what is done about each verdict.
//! - `compare`: the comparison of each kind of field, and what "no value"
//!   is for each.
//! - `sides`: reading the three sides from the database.
//! - `detect`: keeping the `conflict` table in step, and what never
//!   changes.
//! - `resolve`: settling one conflict or many, and undoing it.
//! - `never_lost`: any sequence of edits, detects, resolves and undos
//!   keeps every value a conflict holds.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use super::*;
use crate::db::Writer;
use crate::library::LibraryTrackId;
use crate::ops::{self, Recorded, UndoOutcome};
use crate::paths::Volumes;
use crate::send_values::send_values;
use crate::volume::{identity, IdentitySignals, Volume, VolumeId, VolumeKind};

mod compare;
mod detect;
mod never_lost;
mod resolve;
mod sides;
mod table;

/// A field sent before: the app's value, rekordbox's, and the base. What
/// the app's value replaced isn't known.
pub(super) fn sides<'a>(app: &'a str, rekordbox: &'a str, base: &'a str) -> Sides<'a> {
    Sides {
        app: Some(app),
        rekordbox: Rekordbox::Value(Some(rekordbox)),
        base: Some(base),
        replaced: None,
    }
}

/// A field that was never sent.
pub(super) fn never_sent<'a>(app: &'a str, rekordbox: &'a str) -> Sides<'a> {
    Sides {
        app: Some(app),
        rekordbox: Rekordbox::Value(Some(rekordbox)),
        base: None,
        replaced: None,
    }
}

/// The same sides, with the value the app's own replaced known.
pub(super) fn replacing<'a>(sides: Sides<'a>, replaced: &'a str) -> Sides<'a> {
    Sides {
        replaced: Some(replaced),
        ..sides
    }
}

// ---- A database -----------------------------------------------------------

fn volume_id() -> VolumeId {
    identity(IdentitySignals {
        kind: VolumeKind::External,
        unc_share: None,
        serial: Some(0x1A2B_3C4D),
        filesystem: "NTFS",
        guid: None,
    })
    .unwrap()
}

/// The test volume, mounted at `E:\`.
struct Mount;

impl Volumes for Mount {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!(
            "{} isn't looked up in these tests",
            path.display()
        )))
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        (*id == volume_id()).then(|| PathBuf::from(r"E:\"))
    }
}

/// The read every rekordbox row belongs to unless a test says otherwise.
pub(super) const THIS_READ: &str = "2026-10-12T09:00:00.000Z";
/// An earlier read, whose rows an incomplete export kept.
pub(super) const EARLIER_READ: &str = "2026-10-05T09:00:00.000Z";

/// A migrated database in a temp dir, opened as the app opens it.
pub(super) struct Lib {
    _dir: tempfile::TempDir,
    pub writer: Writer,
    next_track_id: Cell<i64>,
    next_file: Cell<i64>,
}

impl Lib {
    pub fn new() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(
            dir.path(),
            crate::db::DB_FILE_NAME,
        ))
        .unwrap();
        let identity = volume_id().as_str().to_owned();
        writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO volume (identity, kind, last_mount_path)
                     VALUES (?1, 'external', 'E:\\')",
                    [identity],
                )?;
                c.execute_batch(
                    "INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                     VALUES (1, 'Music', 'Music');",
                )
            })
            .unwrap();
        Lib {
            _dir: dir,
            writer,
            next_track_id: Cell::new(1),
            next_file: Cell::new(1),
        }
    }

    fn insert(&self, sql: &'static str, params: impl rusqlite::Params + Send + 'static) -> i64 {
        self.writer
            .call(move |c| {
                c.execute(sql, params)?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    pub fn run(&self, sql: &str) {
        let sql = sql.to_owned();
        self.writer.call(move |c| c.execute_batch(&sql)).unwrap();
    }

    /// A Library track and nothing else: no file, nothing in rekordbox. For
    /// tests that give the sides by hand.
    pub fn bare(&self) -> LibraryTrackId {
        let recording = self.insert("INSERT INTO recording DEFAULT VALUES", ());
        LibraryTrackId(self.insert(
            "INSERT INTO library_track (recording_id, kind, source_status)
             VALUES (?1, 'linked', 'missing')",
            [recording],
        ))
    }

    /// A Library track linked to a file on disk. Returns it and its file.
    pub fn linked(&self) -> (LibraryTrackId, i64) {
        let n = self.next_file.get();
        self.next_file.set(n + 1);
        let recording = self.insert("INSERT INTO recording DEFAULT VALUES", ());
        let file = self.insert(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present)
             VALUES (1, ?1, ?1, 1)",
            [format!("file {n}.mp3")],
        );
        self.insert(
            "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, 'best')",
            [recording, file],
        );
        let track = self.insert(
            "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
             VALUES (?1, 'linked', ?2, 'ok')",
            [recording, file],
        );
        (LibraryTrackId(track), file)
    }

    /// rekordbox's entry for `file`, with these attributes in this order,
    /// from the read at `read_at`; a trusted match unless `probable`.
    pub fn entry(&self, file: i64, read_at: &str, probable: bool, attributes: &[(&str, &str)]) {
        let track_id = self.next_track_id.get();
        self.next_track_id.set(track_id + 1);
        // Built by hand, in this order: a JSON map would sort the names.
        let mut parts = vec![format!("\"TrackID\":\"{track_id}\"")];
        let mut all: Vec<(String, String)> = attributes
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        if !all.iter().any(|(k, _)| k == "Location") {
            all.push((
                "Location".into(),
                format!("file://localhost/E:/Music/{track_id}.mp3"),
            ));
        }
        for (k, v) in &all {
            parts.push(format!(
                "{}:{}",
                serde_json::to_string(k).unwrap(),
                serde_json::to_string(v).unwrap()
            ));
        }
        self.insert(
            "INSERT INTO rekordbox_track
                 (attributes, location_key, read_at, file_id, relink_method, relink_probable)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                format!("{{{}}}", parts.join(",")),
                format!("E:/MUSIC/{track_id}.MP3"),
                read_at.to_owned(),
                file,
                if probable { "filename_only" } else { "path" },
                probable,
            ),
        );
    }

    /// A Library track rekordbox knows, by a trusted match in this read,
    /// with these attributes.
    pub fn known(&self, attributes: &[(&str, &str)]) -> LibraryTrackId {
        let (track, file) = self.linked();
        self.entry(file, THIS_READ, false, attributes);
        track
    }

    /// `count` Library tracks rekordbox knows, made in one go: each a
    /// `Genre` of Tech House where House was sent, and a `Name` nobody
    /// changed.
    pub fn many_known(&self, count: i64) -> Vec<LibraryTrackId> {
        let first = self.next_file.get();
        self.next_file.set(first + count);
        let first_track_id = self.next_track_id.get();
        self.next_track_id.set(first_track_id + count);
        self.writer
            .call(move |c| {
                let tx = c.transaction()?;
                let mut ids = Vec::new();
                for n in 0..count {
                    tx.execute("INSERT INTO recording DEFAULT VALUES", [])?;
                    let recording = tx.last_insert_rowid();
                    tx.execute(
                        "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present)
                         VALUES (1, ?1, ?1, 1)",
                        [format!("file {}.mp3", first + n)],
                    )?;
                    let file = tx.last_insert_rowid();
                    tx.execute(
                        "INSERT INTO recording_file (recording_id, file_id, role)
                         VALUES (?1, ?2, 'best')",
                        [recording, file],
                    )?;
                    tx.execute(
                        "INSERT INTO library_track
                             (recording_id, kind, linked_file_id, source_status)
                         VALUES (?1, 'linked', ?2, 'ok')",
                        [recording, file],
                    )?;
                    let track = tx.last_insert_rowid();
                    let track_id = first_track_id + n;
                    tx.execute(
                        "INSERT INTO rekordbox_track
                             (attributes, location_key, read_at, file_id, relink_method,
                              relink_probable)
                         VALUES (?1, ?2, ?3, ?4, 'path', 0)",
                        (
                            serde_json::json!({
                                "TrackID": track_id.to_string(),
                                "Name": format!("Track {track_id}"),
                                "Genre": "Tech House",
                                "Location":
                                    format!("file://localhost/E:/Music/{track_id}.mp3"),
                            })
                            .to_string(),
                            format!("E:/MUSIC/{track_id}.MP3"),
                            THIS_READ,
                            file,
                        ),
                    )?;
                    for (field, value) in [
                        ("Name", format!("Track {track_id}")),
                        ("Genre", "House".into()),
                    ] {
                        tx.execute(
                            "INSERT INTO sync_base (library_track_id, field, value)
                             VALUES (?1, ?2, ?3)",
                            (track, field, value),
                        )?;
                    }
                    ids.push(LibraryTrackId(track));
                }
                tx.commit()?;
                Ok(ids)
            })
            .unwrap()
    }

    /// Records bases for a track, as a send that wrote these fields does.
    pub fn sent(&self, track: LibraryTrackId, fields: &[(&str, &str)]) {
        for (field, value) in fields {
            self.insert(
                "INSERT INTO sync_base (library_track_id, field, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT (library_track_id, field) DO UPDATE SET value = excluded.value",
                (track.0, (*field).to_owned(), (*value).to_owned()),
            );
        }
    }

    /// The sides of these tracks as the app reads them.
    pub fn sides(&self, ids: &[LibraryTrackId], app: &AppValues) -> Vec<TrackSides> {
        let (ids, app) = (ids.to_vec(), app.clone());
        self.writer
            .call(move |c| {
                let sent = send_values(c, &Mount, &ids)?;
                load_sides(c, &sent, &app)
            })
            .unwrap()
    }

    pub fn try_detect(&self, tracks: &[TrackSides]) -> Result<Detected, crate::db::DbError> {
        let tracks = tracks.to_vec();
        self.writer.call(move |c| detect(c, &tracks))
    }

    pub fn detect(&self, tracks: &[TrackSides]) -> Detected {
        self.try_detect(tracks).unwrap()
    }

    pub fn detect_for(&self, ids: &[LibraryTrackId], app: &AppValues) -> Detected {
        let (ids, app) = (ids.to_vec(), app.clone());
        self.writer
            .call(move |c| detect_for(c, &Mount, &ids, &app))
            .unwrap()
    }

    /// Every `conflict` row, oldest id first.
    pub fn rows(&self) -> Vec<Conflict> {
        self.writer
            .call(|c| {
                c.prepare(&format!(
                    "SELECT {} FROM conflict ORDER BY id",
                    store::CONFLICT_COLUMNS
                ))?
                .query_map([], store::conflict_from)?
                .collect()
            })
            .unwrap()
    }

    /// The settled conflicts, oldest id first.
    pub fn settled(&self) -> Vec<Conflict> {
        let mut rows = self.rows();
        rows.retain(|c| c.status != Status::Open);
        rows
    }

    pub fn open(&self, filter: &Filter) -> Vec<Conflict> {
        let filter = filter.clone();
        self.writer
            .call(move |c| open_conflicts(c, &filter))
            .unwrap()
    }

    /// Settles an open conflict in the table itself, as a resolve leaves
    /// one: its status, and for an edit the value given by hand.
    pub fn settle_by_hand(&self, id: ConflictId, status: Status, edited: Option<&str>) {
        self.settle_by_hand_at(id, status, edited, "2026-10-12T10:00:00.000Z");
    }

    /// [`Lib::settle_by_hand`], settled at `resolved_at`.
    pub fn settle_by_hand_at(
        &self,
        id: ConflictId,
        status: Status,
        edited: Option<&str>,
        resolved_at: &str,
    ) {
        let (edited, resolved_at) = (edited.map(str::to_owned), resolved_at.to_owned());
        let changed = self
            .writer
            .call(move |c| {
                c.execute(
                    "UPDATE conflict SET status = ?2, resolved_value = ?3, resolved_at = ?4
                     WHERE id = ?1 AND status = 'open'",
                    (id.0, status.stored(), edited, resolved_at),
                )
            })
            .unwrap();
        assert_eq!(changed, 1, "conflict {id:?} isn't open");
    }

    /// The one open conflict.
    pub fn only_open(&self) -> Conflict {
        let mut open = self.open(&Filter::default());
        assert_eq!(open.len(), 1, "{open:?}");
        open.remove(0)
    }

    pub fn resolve(
        &self,
        id: ConflictId,
        choice: Choice,
    ) -> Result<Recorded<Resolved>, ResolveError> {
        self.writer
            .call(move |c| Ok(resolve(c, id, &choice)))
            .unwrap()
    }

    pub fn resolve_all(
        &self,
        filter: &Filter,
        choice: Choice,
    ) -> Result<Recorded<Vec<Resolved>>, ResolveError> {
        let filter = filter.clone();
        self.writer
            .call(move |c| Ok(resolve_all(c, &filter, &choice)))
            .unwrap()
    }

    pub fn undo(&self) -> UndoOutcome {
        ops::undo_last_via(&self.writer).unwrap()
    }

    /// Undoes the newest operation, which must work; returns its kind.
    pub fn undone(&self) -> String {
        match self.undo() {
            UndoOutcome::Undone { operation } => operation.kind,
            other => panic!("expected an undo, got {other:?}"),
        }
    }

    /// Every operation in the log: its kind, its details, and whether it
    /// was undone.
    pub fn operations(&self) -> Vec<(String, serde_json::Value, bool)> {
        self.writer
            .call(|c| {
                c.prepare("SELECT kind, details, undone_at IS NOT NULL FROM operation ORDER BY id")?
                    .query_map([], |r| {
                        let details: String = r.get(1)?;
                        Ok((
                            r.get(0)?,
                            serde_json::from_str(&details).unwrap(),
                            r.get(2)?,
                        ))
                    })?
                    .collect()
            })
            .unwrap()
    }

    /// Runs `job` and returns its result with the name of every table a
    /// row was inserted into, updated in or deleted from while it ran.
    pub fn written_by<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> (T, Vec<String>) {
        self.writer
            .call(move |c| {
                let written = Arc::new(Mutex::new(Vec::new()));
                let seen = Arc::clone(&written);
                c.update_hook(Some(
                    move |_: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
                        seen.lock().unwrap().push(table.to_owned());
                    },
                ))?;
                let result = job(c);
                c.update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>)?;
                let written = written.lock().unwrap().clone();
                Ok((result?, written))
            })
            .unwrap()
    }
}

/// One field's sides, by hand: the app's own value, rekordbox's, the base.
pub(super) fn field(
    name: &str,
    app: Option<&str>,
    rekordbox: Option<&str>,
    base: Option<&str>,
) -> FieldSides {
    FieldSides {
        field: name.to_owned(),
        app: app.map(str::to_owned),
        rekordbox: rekordbox.map(str::to_owned),
        base: base.map(str::to_owned),
        replaced: None,
    }
}

/// A field all three sides hold a different value for: a conflict.
pub(super) fn apart(name: &str, app: &str, rekordbox: &str, base: &str) -> FieldSides {
    field(name, Some(app), Some(rekordbox), Some(base))
}

/// A track's sides, with rekordbox's entry from the newest read.
pub(super) fn track(id: LibraryTrackId, fields: Vec<FieldSides>) -> TrackSides {
    TrackSides {
        library_track: id,
        entry: RekordboxEntry::Current,
        fields,
    }
}

/// The three values a conflict holds.
pub(super) fn values(conflict: &Conflict) -> (Option<&str>, Option<&str>, Option<&str>) {
    (
        conflict.app_value.as_deref(),
        conflict.rekordbox_value.as_deref(),
        conflict.base_value.as_deref(),
    )
}
