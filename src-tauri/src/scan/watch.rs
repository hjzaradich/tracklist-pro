//! The per-root watcher (1aC-6, ROADMAP 1.1 "an optional watcher per
//! root", §2 `music_folder.watch`).
//!
//! A music folder with `watch` on is watched with `notify` (Windows:
//! `ReadDirectoryChangesW` on the folder, recursively). Changes under it
//! are gathered into bursts ([`Debounce`]): once a burst has been quiet
//! for [`QUIET`], or [`AT_MOST`] after it began if it never goes quiet,
//! the folder is rescanned, and the scan chain ([`super::chain`]) reads,
//! hashes and fingerprints whatever the walk found new or changed.
//!
//! - **The rescan is a background scan job of the whole root**, the same
//!   walk as a manual scan, at background priority. The walk is stat-only
//!   and its end-of-walk bookkeeping (marking unseen files missing, the
//!   unreadable counts, `walked_at`) is per music folder; a walk of just
//!   the changed subfolders needs a scoped version of that bookkeeping
//!   inside the walk, which this stage's walk lane owns, so it's left for
//!   a later task. One walk per burst keeps the cost bounded.
//! - **A root is rescanned whenever its watcher starts:** at app start,
//!   when its watch is turned on, and when its drive comes back. Nobody
//!   was watching it until then, so it catches up on what changed
//!   meanwhile. A root whose watch is off is never scanned on its own.
//! - **An offline root has no watcher.** [`Watchers::refresh`] (called
//!   after drives come or go, and after a music folder changes) stops the
//!   watcher of a root that no longer resolves, and starts one for a root
//!   that does again, wherever its drive is mounted now.
//! - **Nothing here reads a file.** The watcher only holds a listing
//!   handle on each root; events name paths, and the walk they lead to
//!   looks at listings and attributes only, so a OneDrive placeholder is
//!   never downloaded by being watched (the later stages keep their
//!   [`super::ReadGate`]). The only writes go to the database, through
//!   the job queue.
//!
//! One thread (`music-watch`) owns the `notify` watcher, the set of roots
//! watched and the bursts; the app talks to it through [`Watchers`].

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::event::ModifyKind;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::Connection;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_specta::Event as _;

use super::chain;
use super::folders::{self, MusicFolderError, MusicFolderId};
use super::walk::scan_job;
use crate::db::{DbError, Writer};
use crate::ipc::IpcError;
use crate::jobs::{JobId, JobQueue, NewJob, Priority};
use crate::paths::Volumes;

/// How long a burst of changes must stay quiet before its root is
/// rescanned.
pub const QUIET: Duration = Duration::from_secs(3);
/// How long a burst that never goes quiet (a long copy) can hold the
/// rescan back.
pub const AT_MOST: Duration = Duration::from_secs(60);

/// How the watcher queues a job: the app's job queue, or a test's.
pub type Enqueue = Arc<dyn Fn(NewJob) -> Result<JobId, DbError> + Send + Sync>;

/// The watchers of every music folder with `watch` on. Tauri state; reach
/// it as `State<Watchers>`. Cheap to clone; clones share the one thread.
#[derive(Clone)]
pub struct Watchers {
    inner: Arc<Inner>,
}

struct Inner {
    tx: mpsc::Sender<Msg>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

/// What the app tells the watch thread.
enum Msg {
    /// The music folders or the drives changed: look at the roots again.
    Refresh,
    /// The `notify` watcher saw something.
    Event(notify::Result<Event>),
    /// Asks which roots are watched right now (tests and diagnostics).
    Watched(mpsc::Sender<Vec<MusicFolderId>>),
    Stop,
}

impl Watchers {
    /// Starts the watch thread with the default times. It reads the music
    /// folders through `writer`, resolves them with `volumes` (asked
    /// afresh at every refresh) and queues scans with `enqueue`.
    pub fn start<V: Volumes + 'static>(
        writer: Writer,
        volumes: impl Fn() -> V + Send + 'static,
        enqueue: Enqueue,
    ) -> io::Result<Watchers> {
        Watchers::start_with(QUIET, AT_MOST, writer, volumes, enqueue)
    }

    /// [`Watchers::start`] with the burst times given.
    pub fn start_with<V: Volumes + 'static>(
        quiet: Duration,
        at_most: Duration,
        writer: Writer,
        volumes: impl Fn() -> V + Send + 'static,
        enqueue: Enqueue,
    ) -> io::Result<Watchers> {
        let (tx, rx) = mpsc::channel();
        let events = tx.clone();
        let watcher = RecommendedWatcher::new(
            move |event: notify::Result<Event>| {
                // Only fails once the thread is gone.
                let _ = events.send(Msg::Event(event));
            },
            notify::Config::default(),
        )
        .map_err(|e| io::Error::other(format!("can't start the folder watcher: {e}")))?;
        let mut supervisor = Supervisor {
            writer,
            volumes: Box::new(volumes),
            enqueue,
            watcher,
            watched: HashMap::new(),
            debounce: Debounce::new(quiet, at_most),
        };
        let thread = thread::Builder::new()
            .name("music-watch".into())
            .spawn(move || supervisor.run(&rx))?;
        // Look at the roots right away.
        let _ = tx.send(Msg::Refresh);
        Ok(Watchers {
            inner: Arc::new(Inner {
                tx,
                thread: Mutex::new(Some(thread)),
            }),
        })
    }

    /// Watchers that watch nothing and never will: the stand-in when the
    /// watch thread couldn't start, so the app's state is still whole.
    pub fn inert() -> Watchers {
        let (tx, _gone) = mpsc::channel();
        Watchers {
            inner: Arc::new(Inner {
                tx,
                thread: Mutex::new(None),
            }),
        }
    }

    /// Looks at the music folders and the drives again: starts watching
    /// roots that are watched and online now, stops watching the rest.
    pub fn refresh(&self) {
        let _ = self.inner.tx.send(Msg::Refresh);
    }

    /// The music folders watched right now, in id order.
    pub fn watched(&self) -> Vec<MusicFolderId> {
        let (reply, answer) = mpsc::channel();
        if self.inner.tx.send(Msg::Watched(reply)).is_err() {
            return Vec::new();
        }
        answer.recv().unwrap_or_default()
    }

    /// Stops every watcher and the thread. Only the first call waits.
    pub fn shutdown(&self) {
        let thread = self
            .inner
            .thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(thread) = thread {
            let _ = self.inner.tx.send(Msg::Stop);
            let _ = thread.join();
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        // Never joined here: the last clone may be dropped on the watch
        // thread's own callers' side; `shutdown` is the waiting call.
    }
}

/// The watch thread's state.
struct Supervisor<V> {
    writer: Writer,
    volumes: Box<dyn Fn() -> V + Send>,
    enqueue: Enqueue,
    watcher: RecommendedWatcher,
    /// Each watched music folder and the root it's watched at.
    watched: HashMap<MusicFolderId, PathBuf>,
    debounce: Debounce,
}

impl<V: Volumes> Supervisor<V> {
    fn run(&mut self, rx: &mpsc::Receiver<Msg>) {
        loop {
            let now = Instant::now();
            let msg = match self.debounce.next_due(now) {
                Some(wait) => rx.recv_timeout(wait),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match msg {
                Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(Msg::Refresh) => {
                    if self.refresh() == Refreshed::WriterGone {
                        break;
                    }
                }
                Ok(Msg::Event(Ok(event))) => self.note(&event, Instant::now()),
                // `notify` couldn't keep up or lost a root: look again.
                Ok(Msg::Event(Err(_))) => {
                    if self.refresh() == Refreshed::WriterGone {
                        break;
                    }
                }
                Ok(Msg::Watched(reply)) => {
                    let mut ids: Vec<_> = self.watched.keys().copied().collect();
                    ids.sort();
                    let _ = reply.send(ids);
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            for id in self.debounce.take_due(Instant::now()) {
                self.rescan(id);
            }
        }
        for (_, root) in self.watched.drain() {
            let _ = self.watcher.unwatch(&root);
        }
    }

    /// Brings the watched roots in step with the music folders and the
    /// drives: watched and online → watched at its root now; anything
    /// else → not watched. A root whose watcher just started is rescanned.
    fn refresh(&mut self) -> Refreshed {
        let folders = match self.writer.call(|c| folders::stored(c)) {
            Ok(folders) => folders,
            Err(DbError::WriterGone) => return Refreshed::WriterGone,
            Err(e) => {
                eprintln!("music watch: couldn't read the music folders: {e}");
                return Refreshed::Ok;
            }
        };
        let volumes = (self.volumes)();
        let wanted: BTreeMap<MusicFolderId, PathBuf> = folders
            .iter()
            .filter(|f| f.watch)
            .filter_map(|f| Some((f.id, f.stored_path().resolve(&volumes).ok()?)))
            .collect();
        let stale: Vec<MusicFolderId> = self
            .watched
            .iter()
            .filter(|(id, root)| wanted.get(*id) != Some(root))
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            if let Some(root) = self.watched.remove(&id) {
                // Gone with its drive, or turned off: either way it's out.
                let _ = self.watcher.unwatch(&root);
            }
            // A burst for a root nobody watches any more isn't a rescan.
            self.debounce.forget(id);
        }
        for (id, root) in wanted {
            if self.watched.contains_key(&id) {
                continue;
            }
            match self.watcher.watch(&root, RecursiveMode::Recursive) {
                Ok(()) => {
                    self.watched.insert(id, root);
                    // Nobody was watching until now: catch up.
                    self.rescan(id);
                }
                Err(e) => {
                    eprintln!("music watch: can't watch {}: {e}", root.display());
                }
            }
        }
        Refreshed::Ok
    }

    /// Notes a change under a watched root as part of that root's burst.
    fn note(&mut self, event: &Event, now: Instant) {
        // Reads change nothing; everything else may.
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        for path in &event.paths {
            if is_folder_touch(event, path) {
                continue;
            }
            let root = self
                .watched
                .iter()
                .find(|(_, root)| path.starts_with(root))
                .map(|(id, _)| *id);
            if let Some(id) = root {
                self.debounce.note(id, now);
            }
        }
    }

    /// Queues a background scan of music folder `id`, unless one is
    /// already waiting.
    fn rescan(&self, id: MusicFolderId) {
        let scan = scan_job(Some(vec![id])).priority(Priority::BACKGROUND);
        if let Err(e) = chain::unless_queued(&self.writer, scan, |j| (self.enqueue)(j)) {
            eprintln!(
                "music watch: couldn't queue a rescan of music folder {}: {e}",
                id.0
            );
        }
    }
}

/// A folder's own modification (its times or attributes changed), which
/// says nothing a child's own event doesn't: a file added, removed or
/// renamed in it has its own event, and a renamed folder is a rename, not
/// this. Listing a folder can also make NTFS report one, after the fact,
/// so a walk that took this as a change would walk once more for nothing.
/// Asks for the path's attributes only; nothing is opened.
fn is_folder_touch(event: &Event, path: &std::path::Path) -> bool {
    matches!(
        event.kind,
        EventKind::Modify(ModifyKind::Any | ModifyKind::Data(_) | ModifyKind::Metadata(_))
    ) && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refreshed {
    Ok,
    /// The database is gone: the app is closing.
    WriterGone,
}

/// Bursts of changes, one per music folder. A burst is due once it has
/// been quiet for `quiet`, or `at_most` after it began.
#[derive(Debug)]
pub(crate) struct Debounce {
    quiet: Duration,
    at_most: Duration,
    bursts: BTreeMap<MusicFolderId, Burst>,
}

#[derive(Debug, Clone, Copy)]
struct Burst {
    began: Instant,
    last: Instant,
}

impl Burst {
    fn due_at(&self, quiet: Duration, at_most: Duration) -> Instant {
        (self.last + quiet).min(self.began + at_most)
    }
}

impl Debounce {
    pub(crate) fn new(quiet: Duration, at_most: Duration) -> Debounce {
        Debounce {
            quiet,
            at_most,
            bursts: BTreeMap::new(),
        }
    }

    /// A change under `folder` at `now`: starts its burst, or extends it.
    pub(crate) fn note(&mut self, folder: MusicFolderId, now: Instant) {
        self.bursts
            .entry(folder)
            .and_modify(|b| b.last = now)
            .or_insert(Burst {
                began: now,
                last: now,
            });
    }

    /// Drops `folder`'s burst, if any.
    pub(crate) fn forget(&mut self, folder: MusicFolderId) {
        self.bursts.remove(&folder);
    }

    /// The folders whose bursts are due at `now`, taken out, in id order.
    pub(crate) fn take_due(&mut self, now: Instant) -> Vec<MusicFolderId> {
        let due: Vec<MusicFolderId> = self
            .bursts
            .iter()
            .filter(|(_, b)| b.due_at(self.quiet, self.at_most) <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in &due {
            self.bursts.remove(id);
        }
        due
    }

    /// How long until the next burst is due, from `now`; `None` with no
    /// burst pending.
    pub(crate) fn next_due(&self, now: Instant) -> Option<Duration> {
        self.bursts
            .values()
            .map(|b| {
                b.due_at(self.quiet, self.at_most)
                    .saturating_duration_since(now)
            })
            .min()
    }
}

// --- the app ---

/// Starts the app's watchers: they scan through the app's job queue,
/// resolve folders on Windows' volumes, and look again whenever drives
/// come or go ([`super::VolumesChanged`]). Call it after the job queue is
/// in the app's state.
///
/// Nothing here can stop the app starting: if the watcher can't start,
/// the folders are simply not watched, and manual scans still work.
pub fn start<R: Runtime>(app: &AppHandle<R>, writer: Writer) -> Watchers {
    let handle = app.clone();
    let enqueue: Enqueue = Arc::new(move |job| {
        handle
            .try_state::<JobQueue>()
            .ok_or(DbError::WriterGone)?
            .enqueue(job)
    });
    let watchers = match Watchers::start(writer, super::system_volumes, enqueue) {
        Ok(watchers) => watchers,
        Err(e) => {
            eprintln!("music watch: not watching any folder: {e}");
            Watchers::inert()
        }
    };
    let on_drives = watchers.clone();
    super::VolumesChanged::listen_any(app, move |_| on_drives.refresh());
    watchers
}

/// Sets a music folder's `watch`. Returns whether a row changed.
pub(crate) fn set_watch(
    conn: &Connection,
    id: MusicFolderId,
    watch: bool,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE music_folder SET watch = ?2 WHERE id = ?1",
        (id.0, watch),
    )?;
    Ok(changed == 1)
}

/// Turns a music folder's watcher on or off. On: the folder is rescanned
/// now and whenever files change under it. Off: it's scanned only when
/// asked.
#[tauri::command]
#[specta::specta]
pub async fn set_music_folder_watch(
    writer: State<'_, Writer>,
    watchers: State<'_, Watchers>,
    id: MusicFolderId,
    watch: bool,
) -> Result<(), IpcError> {
    let changed = writer.call(move |c| set_watch(c, id, watch))?;
    if !changed {
        return Err(MusicFolderError::NotFound.into());
    }
    watchers.refresh();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    fn id(n: i64) -> MusicFolderId {
        MusicFolderId(n)
    }

    #[test]
    fn a_burst_is_due_once_it_has_been_quiet_for_the_quiet_time() {
        let t0 = Instant::now();
        let mut d = Debounce::new(3 * S, 60 * S);
        d.note(id(1), t0);
        d.note(id(1), t0 + S);
        d.note(id(1), t0 + 2 * S);
        // Quiet since t0 + 2 s: nothing yet…
        assert_eq!(d.take_due(t0 + 4 * S), Vec::<MusicFolderId>::new());
        assert_eq!(d.next_due(t0 + 4 * S), Some(S));
        // …then due exactly once.
        assert_eq!(d.take_due(t0 + 5 * S), vec![id(1)]);
        assert_eq!(d.take_due(t0 + 5 * S), Vec::<MusicFolderId>::new());
        assert_eq!(d.next_due(t0 + 5 * S), None);
    }

    #[test]
    fn a_burst_that_never_goes_quiet_is_due_at_most_after_it_began() {
        let t0 = Instant::now();
        let mut d = Debounce::new(3 * S, 10 * S);
        for n in 0..20 {
            d.note(id(1), t0 + n * S);
        }
        // The last change was at t0 + 19 s, but the burst began at t0.
        assert_eq!(d.take_due(t0 + 10 * S), vec![id(1)]);
        // A change after that starts a new burst.
        d.note(id(1), t0 + 11 * S);
        assert_eq!(d.next_due(t0 + 11 * S), Some(3 * S));
    }

    #[test]
    fn each_folder_has_its_own_burst_and_a_forgotten_one_is_never_due() {
        let t0 = Instant::now();
        let mut d = Debounce::new(3 * S, 60 * S);
        d.note(id(2), t0);
        d.note(id(1), t0 + S);
        d.note(id(3), t0 + S);
        d.forget(id(3));
        assert_eq!(d.next_due(t0 + S), Some(2 * S));
        assert_eq!(d.take_due(t0 + 3 * S), vec![id(2)]);
        assert_eq!(d.take_due(t0 + 4 * S), vec![id(1)]);
        assert_eq!(d.take_due(t0 + 100 * S), Vec::<MusicFolderId>::new());
    }

    #[test]
    fn a_wait_never_goes_negative_when_a_burst_is_overdue() {
        let t0 = Instant::now();
        let mut d = Debounce::new(S, 60 * S);
        d.note(id(1), t0);
        assert_eq!(d.next_due(t0 + 10 * S), Some(Duration::ZERO));
    }
}
