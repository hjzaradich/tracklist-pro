//! The per-root watcher (1aC-6, ROADMAP 1.1 "an optional watcher per
//! root", §2 `music_folder.watch`).
//!
//! A music folder with `watch` on is watched with `notify` (Windows:
//! `ReadDirectoryChangesW` on the folder, recursively). Changes under it
//! are gathered into bursts ([`Debounce`]): once a burst has been quiet
//! for [`QUIET`], or its longest wait after it began if it never goes
//! quiet, the folder is rescanned, and the scan chain ([`super::chain`])
//! reads, hashes and fingerprints whatever the walk found new or changed.
//!
//! - **The rescan is a background scan job of the whole root**, the same
//!   walk as a manual scan, at background priority. The walk is stat-only
//!   and its end-of-walk bookkeeping (marking unseen files missing, the
//!   unreadable counts, `walked_at`) is per music folder; a walk of just
//!   the changed subfolders needs a scoped version of that bookkeeping
//!   inside the walk, which this stage's walk lane owns, so it's left for
//!   a later task. One walk per burst keeps the cost bounded, and a root
//!   already being walked is never walked twice at once: the request
//!   folds into one more walk after the current one
//!   ([`super::chain::queue_once`]).
//! - **Sustained churn backs off.** A burst that never goes quiet (a long
//!   copy, a sync) is due [`AT_MOST`] after it began; each such burst in a
//!   row doubles the next one's longest wait, up to [`AT_MOST_CAP`], and a
//!   burst that does go quiet resets it.
//! - **Only changes the walk could index count** ([`matters`]): audio
//!   files by the walk's own rule ([`super::is_indexed`]), and folders
//!   coming, going or being renamed. Files the walk skips (`._*`,
//!   `.DS_Store`, notes, artwork, a download still in progress) start no
//!   burst; a download finished by a rename to an audio name does. A path
//!   that's gone (removed, or renamed away) counts only if the index has
//!   it, or something under it: an autosave's temp file renamed over a
//!   project file, or a download that ends up with a non-audio name, was
//!   never indexed and costs no walk. Nothing under a folder Windows marks
//!   hidden and system counts. A folder's own modification (its times
//!   changed) doesn't either: a child's own event says the same, and NTFS
//!   reports one after a walk lists the folder.
//! - **Every online music folder is rechecked** (the same background
//!   scan job, and the chain) at app start and whenever its drive comes
//!   back, watched or not: nothing could see what changed while the app
//!   was closed or the drive away, and a walk that skips unchanged files
//!   without opening them (1aC-1) is cheap. Turning a folder's watch on
//!   rescans it too. Each trigger scans a folder once (owner's decision,
//!   2026-09-29). Those scans retry files a stage found unreachable; a
//!   watcher's own rescan is marked ([`super::chain::RESCAN_KEY`]) and
//!   doesn't, so one locked file can't cost every stage per burst.
//! - **Watch adds live updates only while the app runs,** and is off by
//!   default (`music_folder.watch = 0`).
//! - **An offline root has no watcher.** [`Watchers::refresh`] (called
//!   after drives come or go, and after a music folder changes) stops the
//!   watcher of a root that no longer resolves, and starts one for a root
//!   that does again, wherever its drive is mounted now.
//! - **Ejecting a drive works.** A watcher holds a handle on its root,
//!   which would make Windows refuse "Eject" ("device in use"). Each root
//!   is registered for removal messages
//!   ([`crate::volume::devices::watch_handles`]): when Windows asks, the
//!   root's watcher stops and its handles close before the answer; if the
//!   removal is then refused elsewhere, the watcher starts again with a
//!   catch-up rescan; if it goes through, the drive is offline as usual.
//!   While the answer is pending, nothing reopens the root: a refresh
//!   leaves it alone until the removal fails or the drive goes, or until
//!   it has waited [`EJECT_GRACE`] with the drive still here.
//! - **Nothing here reads a file.** The watcher only holds listing and
//!   query-only handles on each root; events name paths, whose attributes
//!   are looked at, and the walk they lead to looks at listings and
//!   attributes only, so a OneDrive placeholder is never downloaded by
//!   being watched (the later stages keep their [`super::ReadGate`]). The
//!   only writes go to the database, through the job queue.
//!
//! One thread (`music-watch`) owns the `notify` watcher, the set of roots
//! watched and the bursts; the app talks to it through [`Watchers`].

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
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
use super::online_only::attributes;
use super::walk::scan_job;
use super::{is_indexed, is_skipped};
use crate::db::{DbError, Writer};
use crate::ipc::IpcError;
use crate::jobs::{JobId, JobQueue, NewJob, Priority};
use crate::paths::Volumes;

/// How long a burst of changes must stay quiet before its root is
/// rescanned.
pub const QUIET: Duration = Duration::from_secs(3);
/// How long a burst that never goes quiet (a long copy) can hold the
/// rescan back, the first time.
pub const AT_MOST: Duration = Duration::from_secs(60);
/// The longest wait a run of such bursts can back off to.
pub const AT_MOST_CAP: Duration = Duration::from_secs(600);
/// How long Windows' question "may this drive go?" waits for the watch
/// thread before the answer is yes anyway.
#[cfg(windows)]
const EJECT_ANSWER: Duration = Duration::from_secs(5);
/// How long a root stopped for an eject waits, with its drive still
/// here and no word of a refusal, before a refresh watches it again.
pub const EJECT_GRACE: Duration = Duration::from_secs(30);

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
    /// Windows asks whether the drive behind a root's registration may go:
    /// stop watching it, then answer.
    QueryRemove(isize, mpsc::Sender<bool>),
    /// The removal was refused elsewhere: watch the root again.
    RemoveFailed(isize),
    /// Asks what's watched right now (tests and diagnostics).
    Status(mpsc::Sender<Status>),
    /// Asks for the eject window (tests send it messages).
    #[cfg(all(test, windows))]
    Window(mpsc::Sender<Option<crate::volume::devices::DeviceWatch>>),
    /// Replaces [`EJECT_GRACE`] (tests).
    #[cfg(test)]
    EjectGrace(Duration),
    Stop,
}

/// What the watch thread holds right now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    /// The music folders watched, in id order.
    pub watched: Vec<MusicFolderId>,
    /// The music folders whose watcher is stopped for an eject in
    /// progress, in id order.
    pub suspended: Vec<MusicFolderId>,
    /// How many query-only root handles are open.
    pub handles_open: usize,
    /// Each root's removal registration, by music folder.
    pub registrations: Vec<(MusicFolderId, isize)>,
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

    /// [`Watchers::start`] with the burst times given (`at_most` backs
    /// off up to [`AT_MOST_CAP`], or to ten times itself if that's less).
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
        let eject = eject::EjectWatch::start(tx.clone());
        let mut supervisor = Supervisor {
            writer,
            volumes: Box::new(volumes),
            enqueue,
            watcher,
            eject,
            roots: HashMap::new(),
            online: None,
            debounce: Debounce::new(quiet, at_most, AT_MOST_CAP.min(at_most * 10)),
            windows_own: HashMap::new(),
            eject_grace: EJECT_GRACE,
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

    /// What's watched right now. Answered after everything sent before.
    pub fn status(&self) -> Status {
        let (reply, answer) = mpsc::channel();
        if self.inner.tx.send(Msg::Status(reply)).is_err() {
            return Status::default();
        }
        answer.recv().unwrap_or_default()
    }

    /// The music folders watched right now, in id order.
    pub fn watched(&self) -> Vec<MusicFolderId> {
        self.status().watched
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

/// A watched root.
struct Root {
    path: PathBuf,
    /// Its removal registration, with the query-only handle Windows asks
    /// about. `None` where that couldn't be set up; then an eject fails
    /// while the root is watched.
    registration: Option<eject::Registration>,
    /// Stopped for an eject Windows asked about, since then, until the
    /// removal fails or the drive goes.
    suspended: Option<Instant>,
}

/// Why a folder is scanned, which decides what the chain retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Because {
    /// It's just online, or just watched: catch up on everything, files a
    /// stage found unreachable included.
    Online,
    /// Files changed under it while watched.
    Changes,
}

/// The watch thread's state.
struct Supervisor<V> {
    writer: Writer,
    volumes: Box<dyn Fn() -> V + Send>,
    enqueue: Enqueue,
    watcher: RecommendedWatcher,
    eject: Option<eject::EjectWatch>,
    roots: HashMap<MusicFolderId, Root>,
    /// The music folders that were online at the last refresh; `None`
    /// before the first, when every online folder counts as just back.
    online: Option<BTreeSet<MusicFolderId>>,
    debounce: Debounce,
    /// Folders looked at for the hidden-and-system mark, and the answer.
    windows_own: HashMap<PathBuf, bool>,
    eject_grace: Duration,
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
                // `notify` reported trouble (not on Windows, where it drops
                // a root silently instead; `refresh` looks for that).
                Ok(Msg::Event(Err(_))) => {
                    if self.refresh() == Refreshed::WriterGone {
                        break;
                    }
                }
                Ok(Msg::QueryRemove(registration, reply)) => {
                    self.suspend(registration);
                    let _ = reply.send(true);
                }
                Ok(Msg::RemoveFailed(registration)) => self.resume(registration),
                Ok(Msg::Status(reply)) => {
                    let _ = reply.send(self.status());
                }
                #[cfg(all(test, windows))]
                Ok(Msg::Window(reply)) => {
                    let _ = reply.send(self.eject.as_ref().map(|e| *e.window()));
                }
                #[cfg(test)]
                Ok(Msg::EjectGrace(grace)) => self.eject_grace = grace,
                Err(RecvTimeoutError::Timeout) => {}
            }
            for id in self.debounce.take_due(Instant::now()) {
                self.rescan(id, Because::Changes);
            }
        }
        for (_, root) in self.roots.drain() {
            let _ = self.watcher.unwatch(&root.path);
        }
    }

    fn status(&self) -> Status {
        let mut watched: Vec<_> = self
            .roots
            .iter()
            .filter(|(_, r)| r.suspended.is_none())
            .map(|(id, _)| *id)
            .collect();
        watched.sort();
        let mut suspended: Vec<_> = self
            .roots
            .iter()
            .filter(|(_, r)| r.suspended.is_some())
            .map(|(id, _)| *id)
            .collect();
        suspended.sort();
        let mut registrations: Vec<_> = self
            .roots
            .iter()
            .filter_map(|(id, r)| Some((*id, r.registration.as_ref()?.id())))
            .collect();
        registrations.sort();
        Status {
            watched,
            suspended,
            handles_open: self
                .roots
                .values()
                .filter(|r| r.registration.as_ref().is_some_and(|reg| reg.is_open()))
                .count(),
            registrations,
        }
    }

    /// Brings the watched roots in step with the music folders and the
    /// drives: watched and online → watched at its root now; anything
    /// else → not watched. A folder that just came online (the first
    /// refresh, or its drive back) and a root whose watcher just started
    /// are rescanned, once each.
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
        let online: BTreeMap<MusicFolderId, PathBuf> = folders
            .iter()
            .filter_map(|f| Some((f.id, f.stored_path().resolve(&volumes).ok()?)))
            .collect();
        let wanted: BTreeMap<MusicFolderId, PathBuf> = folders
            .iter()
            .filter(|f| f.watch)
            .filter_map(|f| Some((f.id, online.get(&f.id)?.clone())))
            .collect();
        // Folders just back online: every online one the first time.
        let mut to_scan: BTreeSet<MusicFolderId> = online
            .keys()
            .filter(|id| self.online.as_ref().is_none_or(|was| !was.contains(id)))
            .copied()
            .collect();
        self.online = Some(online.keys().copied().collect());
        // Out: turned off, gone with its drive, or a root folder that's
        // gone from under the watcher (Windows drops that watch silently).
        let stale: Vec<MusicFolderId> = self
            .roots
            .iter()
            .filter(|(id, root)| {
                wanted.get(*id) != Some(&root.path)
                    || std::fs::symlink_metadata(&root.path).is_err()
            })
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            self.forget(id);
        }
        // A root stopped for an eject stays stopped: watching it again
        // would make Windows refuse the eject as "in use". Only one that
        // has waited out the grace with its drive still here (no refusal
        // ever heard) is watched again.
        let grace = self.eject_grace;
        let stuck: Vec<MusicFolderId> = self
            .roots
            .iter()
            .filter(|(_, root)| root.suspended.is_some_and(|since| since.elapsed() >= grace))
            .map(|(id, _)| *id)
            .collect();
        for id in stuck {
            self.forget(id);
        }
        for (id, path) in wanted {
            if self.roots.contains_key(&id) {
                continue;
            }
            if self.watch(id, path) {
                // Nobody was watching until now: catch up.
                to_scan.insert(id);
            }
        }
        for id in to_scan {
            self.rescan(id, Because::Online);
        }
        Refreshed::Ok
    }

    /// Starts watching `path` as music folder `id`'s root. False if the
    /// watch couldn't start (it's tried again at the next refresh).
    fn watch(&mut self, id: MusicFolderId, path: PathBuf) -> bool {
        if let Err(e) = self.watcher.watch(&path, RecursiveMode::Recursive) {
            eprintln!("music watch: can't watch {}: {e}", path.display());
            return false;
        }
        let registration = self.eject.as_ref().and_then(|eject| {
            eject
                .register(&path)
                .map_err(|e| {
                    eprintln!(
                        "music watch: {} is watched, but an eject of its drive will fail: {e}",
                        path.display()
                    )
                })
                .ok()
        });
        self.roots.insert(
            id,
            Root {
                path,
                registration,
                suspended: None,
            },
        );
        true
    }

    /// Stops watching music folder `id` and forgets it, its handles
    /// closed and its burst dropped.
    fn forget(&mut self, id: MusicFolderId) {
        if let Some(root) = self.roots.remove(&id) {
            if root.suspended.is_none() {
                let _ = self.watcher.unwatch(&root.path);
            }
            // The registration drops here: handle closed, registration ended.
        }
        // A burst for a root nobody watches any more isn't a rescan.
        self.debounce.forget(id);
    }

    /// Windows asks whether the drive behind `registration` may go: the
    /// root's watcher stops and its handle closes, now.
    fn suspend(&mut self, registration: isize) {
        let ids: Vec<MusicFolderId> = self
            .roots
            .iter()
            .filter(|(_, r)| {
                r.registration
                    .as_ref()
                    .is_some_and(|reg| reg.id() == registration)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            let Some(root) = self.roots.get_mut(&id) else {
                continue;
            };
            if root.suspended.is_some() {
                continue;
            }
            let _ = self.watcher.unwatch(&root.path);
            if let Some(reg) = &mut root.registration {
                reg.close();
            }
            root.suspended = Some(Instant::now());
            self.debounce.forget(id);
        }
    }

    /// The removal behind `registration` was refused: the drive stays, so
    /// the root's watcher starts again, with a catch-up for what it missed.
    fn resume(&mut self, registration: isize) {
        let ids: Vec<MusicFolderId> = self
            .roots
            .iter()
            .filter(|(_, r)| {
                r.suspended.is_some()
                    && r.registration
                        .as_ref()
                        .is_some_and(|reg| reg.id() == registration)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            let Some(root) = self.roots.remove(&id) else {
                continue;
            };
            if self.watch(id, root.path) {
                self.rescan(id, Because::Changes);
            }
        }
    }

    /// Notes a change under a watched root as part of that root's burst.
    fn note(&mut self, event: &Event, now: Instant) {
        for path in &event.paths {
            let root = self
                .roots
                .iter()
                .find(|(_, root)| root.suspended.is_none() && path.starts_with(&root.path))
                .map(|(id, root)| (*id, root.path.clone()));
            let Some((id, root)) = root else {
                continue;
            };
            let counts = match matters(&event.kind, path, &root, &mut self.windows_own) {
                Matters::Yes => true,
                Matters::No => false,
                Matters::IfIndexed => self.indexed(id, path, &root),
            };
            if counts {
                self.debounce.note(id, now);
            }
        }
        if self.windows_own.len() > 4096 {
            self.windows_own.clear();
        }
    }

    /// Whether the index has a present file at `path` under music folder
    /// `id`'s `root`, or under it (then it was a folder). If the database
    /// can't say, it counts.
    fn indexed(&self, id: MusicFolderId, path: &Path, root: &Path) -> bool {
        let Ok(rel) = path.strip_prefix(root) else {
            return true;
        };
        let rel: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        if rel.is_empty() {
            // The root itself.
            return true;
        }
        let rel = rel.join("/");
        // `%` and `_` are LIKE's wildcards.
        let under = rel
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
            + "/%";
        self.writer
            .call(move |c| {
                c.prepare_cached(
                    "SELECT EXISTS (SELECT 1 FROM file
                     WHERE music_folder_id = ?1 AND present = 1
                       AND (rel_path = ?2 OR rel_path LIKE ?3 ESCAPE '\\'))",
                )?
                .query_row((id.0, rel, under), |r| r.get::<_, bool>(0))
            })
            .unwrap_or(true)
    }

    /// Queues a background scan of music folder `id`, unless one is
    /// already waiting (a running one is asked to run once more).
    fn rescan(&self, id: MusicFolderId, because: Because) {
        let mut scan = scan_job(Some(vec![id])).priority(Priority::BACKGROUND);
        if because == Because::Changes {
            if let Some(serde_json::Value::Object(target)) = &mut scan.target {
                target.insert(chain::RESCAN_KEY.into(), serde_json::Value::Bool(true));
            }
        }
        if let Err(e) = chain::queue_once(&self.writer, scan, |j| (self.enqueue)(j)) {
            eprintln!(
                "music watch: couldn't queue a rescan of music folder {}: {e}",
                id.0
            );
        }
    }
}

/// Whether a change is worth a rescan (see [`matters`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Matters {
    Yes,
    No,
    /// The path is gone, so a file can't be told from a folder: it
    /// matters if the index has it, or something under it.
    IfIndexed,
}

/// Whether a change at `path` (under the watched `root`) is one the walk
/// could index, so it's worth a rescan (see the module docs). Looks at
/// attributes only; nothing is opened. `windows_own` remembers which
/// folders are hidden and system, so a burst asks about each once.
pub(crate) fn matters(
    kind: &EventKind,
    path: &Path,
    root: &Path,
    windows_own: &mut HashMap<PathBuf, bool>,
) -> Matters {
    // Reads change nothing.
    if matches!(kind, EventKind::Access(_)) {
        return Matters::No;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            // A folder's own modification says nothing its children's
            // events don't; coming, going and renames do.
            if matches!(
                kind,
                EventKind::Modify(ModifyKind::Any | ModifyKind::Data(_) | ModifyKind::Metadata(_))
            ) {
                return Matters::No;
            }
            yes_or_no(!under_windows_own(path, root, windows_own))
        }
        Ok(_) => yes_or_no(
            is_indexed(&name)
                && !under_windows_own(path.parent().unwrap_or(root), root, windows_own),
        ),
        // Gone: a removed (or renamed-away) file or folder. A name the walk
        // never indexes can be dropped without looking; the rest depends
        // on whether the index knew it.
        Err(_) if is_skipped(&name) => Matters::No,
        Err(_) => Matters::IfIndexed,
    }
}

fn yes_or_no(yes: bool) -> Matters {
    if yes {
        Matters::Yes
    } else {
        Matters::No
    }
}

/// Whether `folder` (a folder under `root`, or `root` itself) or any
/// folder between it and `root` is marked hidden and system: Windows' own,
/// which the walk never enters.
fn under_windows_own(folder: &Path, root: &Path, cache: &mut HashMap<PathBuf, bool>) -> bool {
    const HIDDEN_AND_SYSTEM: u32 = 0x2 | 0x4; // FILE_ATTRIBUTE_HIDDEN | _SYSTEM
    let mut at = folder;
    while at != root && at.starts_with(root) {
        let own = match cache.get(at) {
            Some(own) => *own,
            None => {
                let own = std::fs::symlink_metadata(at)
                    .is_ok_and(|m| attributes(&m) & HIDDEN_AND_SYSTEM == HIDDEN_AND_SYSTEM);
                cache.insert(at.to_path_buf(), own);
                own
            }
        };
        if own {
            return true;
        }
        match at.parent() {
            Some(parent) => at = parent,
            None => break,
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refreshed {
    Ok,
    /// The database is gone: the app is closing.
    WriterGone,
}

/// Bursts of changes, one per music folder. A burst is due once it has
/// been quiet for `quiet`, or its longest wait after it began; that wait
/// starts at `at_most` and doubles after every burst that never went
/// quiet, up to `cap`, and goes back to `at_most` after one that did.
#[derive(Debug)]
pub(crate) struct Debounce {
    quiet: Duration,
    at_most: Duration,
    cap: Duration,
    bursts: BTreeMap<MusicFolderId, Burst>,
    /// Each folder's longest wait for its next burst, once it has backed
    /// off.
    backoff: BTreeMap<MusicFolderId, Duration>,
}

#[derive(Debug, Clone, Copy)]
struct Burst {
    began: Instant,
    last: Instant,
    at_most: Duration,
}

impl Burst {
    fn due_at(&self, quiet: Duration) -> Instant {
        (self.last + quiet).min(self.began + self.at_most)
    }
}

impl Debounce {
    pub(crate) fn new(quiet: Duration, at_most: Duration, cap: Duration) -> Debounce {
        Debounce {
            quiet,
            at_most,
            cap: cap.max(at_most),
            bursts: BTreeMap::new(),
            backoff: BTreeMap::new(),
        }
    }

    /// A change under `folder` at `now`: starts its burst, or extends it.
    pub(crate) fn note(&mut self, folder: MusicFolderId, now: Instant) {
        let at_most = self.backoff.get(&folder).copied().unwrap_or(self.at_most);
        self.bursts
            .entry(folder)
            .and_modify(|b| b.last = now)
            .or_insert(Burst {
                began: now,
                last: now,
                at_most,
            });
    }

    /// Drops `folder`'s burst and backoff, if any.
    pub(crate) fn forget(&mut self, folder: MusicFolderId) {
        self.bursts.remove(&folder);
        self.backoff.remove(&folder);
    }

    /// The folders whose bursts are due at `now`, taken out, in id order.
    pub(crate) fn take_due(&mut self, now: Instant) -> Vec<MusicFolderId> {
        let due: Vec<(MusicFolderId, Burst)> = self
            .bursts
            .iter()
            .filter(|(_, b)| b.due_at(self.quiet) <= now)
            .map(|(id, b)| (*id, *b))
            .collect();
        for (id, burst) in &due {
            self.bursts.remove(id);
            if now >= burst.last + self.quiet {
                // It went quiet: no churn to back off from.
                self.backoff.remove(id);
            } else {
                let next = (burst.at_most * 2).min(self.cap);
                self.backoff.insert(*id, next);
            }
        }
        due.into_iter().map(|(id, _)| id).collect()
    }

    /// How long until the next burst is due, from `now`; `None` with no
    /// burst pending.
    pub(crate) fn next_due(&self, now: Instant) -> Option<Duration> {
        self.bursts
            .values()
            .map(|b| b.due_at(self.quiet).saturating_duration_since(now))
            .min()
    }

    /// The longest wait `folder`'s next burst gets.
    #[cfg(test)]
    pub(crate) fn longest_wait(&self, folder: MusicFolderId) -> Duration {
        self.backoff.get(&folder).copied().unwrap_or(self.at_most)
    }
}

// --- ejecting ---

/// The removal messages for watched roots (see the module docs).
#[cfg(windows)]
mod eject {
    use std::io;
    use std::path::Path;
    use std::sync::mpsc;

    use super::{Msg, EJECT_ANSWER};
    use crate::volume::devices::{self, DeviceWatch, HandleEvents, HandleRegistration};

    /// The window that hears the messages, relayed to the watch thread.
    pub(super) struct EjectWatch {
        window: DeviceWatch,
    }

    pub(super) type Registration = HandleRegistration;

    impl EjectWatch {
        /// `None` if the window couldn't be made: then roots are watched
        /// without it, and an eject of their drive fails.
        pub(super) fn start(tx: mpsc::Sender<Msg>) -> Option<EjectWatch> {
            match devices::watch_handles(Relay { tx }) {
                Ok(window) => Some(EjectWatch { window }),
                Err(e) => {
                    eprintln!("music watch: no eject handling: {e}");
                    None
                }
            }
        }

        pub(super) fn register(&self, root: &Path) -> io::Result<Registration> {
            self.window.register_directory(root)
        }

        /// The hidden window, for tests that send it messages.
        #[cfg(test)]
        pub(super) fn window(&self) -> &DeviceWatch {
            &self.window
        }
    }

    /// Passes each message to the watch thread and, for a question, waits
    /// for its answer.
    struct Relay {
        tx: mpsc::Sender<Msg>,
    }

    impl HandleEvents for Relay {
        fn query_remove(&self, registration: isize) -> bool {
            let (reply, answer) = mpsc::channel();
            if self.tx.send(Msg::QueryRemove(registration, reply)).is_err() {
                return true;
            }
            // A watch thread that doesn't answer in time doesn't hold the
            // drive up (its handle would, but then something is wrong
            // anyway).
            answer.recv_timeout(EJECT_ANSWER).unwrap_or(true)
        }

        fn remove_failed(&self, registration: isize) {
            let _ = self.tx.send(Msg::RemoveFailed(registration));
        }
    }
}

/// No ejecting outside Windows (unsupported, ROADMAP §1.1).
#[cfg(not(windows))]
mod eject {
    use std::io;
    use std::path::Path;
    use std::sync::mpsc;

    use super::Msg;

    pub(super) struct EjectWatch;

    #[derive(Debug)]
    pub(super) struct Registration;

    impl EjectWatch {
        pub(super) fn start(_tx: mpsc::Sender<Msg>) -> Option<EjectWatch> {
            None
        }

        pub(super) fn register(&self, _root: &Path) -> io::Result<Registration> {
            Ok(Registration)
        }
    }

    impl Registration {
        pub(super) fn id(&self) -> isize {
            0
        }

        pub(super) fn is_open(&self) -> bool {
            false
        }

        pub(super) fn close(&mut self) {}
    }
}

impl Watchers {
    /// Replaces [`EJECT_GRACE`] for the tests that wait it out.
    #[cfg(test)]
    pub(crate) fn set_eject_grace(&self, grace: Duration) {
        let _ = self.inner.tx.send(Msg::EjectGrace(grace));
    }

    /// The hidden window that hears the eject messages, for tests that
    /// send it some.
    #[cfg(all(test, windows))]
    pub(crate) fn eject_window(&self) -> Option<crate::volume::devices::DeviceWatch> {
        let (reply, answer) = mpsc::channel();
        self.inner.tx.send(Msg::Window(reply)).ok()?;
        answer.recv().ok().flatten()
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
/// now and whenever files change under it while the app runs. Off (the
/// default): it's still rechecked at app start and when its drive comes
/// back, and otherwise scanned when asked.
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
        let mut d = Debounce::new(3 * S, 60 * S, 600 * S);
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
        let mut d = Debounce::new(3 * S, 10 * S, 100 * S);
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
    fn bursts_that_keep_hitting_their_longest_wait_back_off_and_a_quiet_one_resets_it() {
        let t0 = Instant::now();
        let mut d = Debounce::new(3 * S, 60 * S, 600 * S);
        assert_eq!(d.longest_wait(id(1)), 60 * S);
        // Churn: a change every second, for ever.
        let mut t = t0;
        let mut waits = Vec::new();
        for _ in 0..6 {
            let began = t;
            let mut steps = 0;
            loop {
                d.note(id(1), t);
                t += S;
                let due = d.take_due(t);
                if !due.is_empty() {
                    break;
                }
                steps += 1;
                assert!(steps < 10_000, "the longest wait never came");
            }
            waits.push(t - began);
        }
        // 60 s, then doubling, capped at 10 min.
        assert_eq!(waits, [60 * S, 120 * S, 240 * S, 480 * S, 600 * S, 600 * S]);
        // A burst that goes quiet ends the churn: back to the start.
        d.note(id(1), t);
        assert_eq!(d.take_due(t + 3 * S), vec![id(1)]);
        assert_eq!(d.longest_wait(id(1)), 60 * S);
        // Another folder's churn is its own.
        assert_eq!(d.longest_wait(id(2)), 60 * S);
    }

    #[test]
    fn each_folder_has_its_own_burst_and_a_forgotten_one_is_never_due() {
        let t0 = Instant::now();
        let mut d = Debounce::new(3 * S, 60 * S, 600 * S);
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
        let mut d = Debounce::new(S, 60 * S, 600 * S);
        d.note(id(1), t0);
        assert_eq!(d.next_due(t0 + 10 * S), Some(Duration::ZERO));
    }

    #[test]
    fn only_changes_the_walk_could_index_matter() {
        use notify::event::{CreateKind, RemoveKind, RenameMode};
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let file = |name: &str| {
            let path = root.join(name);
            std::fs::write(&path, b"x").unwrap();
            path
        };
        let folder = root.join("Album");
        std::fs::create_dir(&folder).unwrap();
        let mut cache = HashMap::new();
        let create = EventKind::Create(CreateKind::Any);
        let modify = EventKind::Modify(ModifyKind::Any);
        let renamed_to = EventKind::Modify(ModifyKind::Name(RenameMode::To));
        let remove = EventKind::Remove(RemoveKind::Any);
        let mut check = |kind: &EventKind, path: &Path| matters(kind, path, &root, &mut cache);
        use Matters::*;

        // Audio files, by the walk's own rule, in any letter case.
        assert_eq!(check(&create, &file("track.mp3")), Yes);
        assert_eq!(check(&modify, &file("Track 2.FLAC")), Yes);
        // What the walk skips or never indexes.
        assert_eq!(check(&create, &file("notes.txt")), No);
        assert_eq!(check(&create, &file("cover.jpg")), No);
        assert_eq!(check(&create, &file("._track.mp3")), No);
        assert_eq!(check(&create, &file(".DS_Store")), No);
        assert_eq!(check(&modify, &file("track.mp3.crdownload")), No);
        assert_eq!(check(&modify, &file("track.part")), No);
        // A download finished by a rename to an audio name.
        assert_eq!(check(&renamed_to, &file("finished.mp3")), Yes);
        // Folders: coming, going and renames matter; their own touch not.
        assert_eq!(check(&create, &folder), Yes);
        assert_eq!(check(&renamed_to, &folder), Yes);
        assert_eq!(check(&modify, &folder), No);
        // Gone: a file can't be told from a folder any more, so it's up to
        // the index; a name the walk never indexes needs no look.
        assert_eq!(check(&remove, &root.join("gone.mp3")), IfIndexed);
        assert_eq!(check(&remove, &root.join("Gone Album")), IfIndexed);
        assert_eq!(check(&remove, &root.join("autosave.tmp")), IfIndexed);
        assert_eq!(check(&remove, &root.join("._gone.mp3")), No);
        assert_eq!(check(&remove, &root.join(".DS_Store")), No);
        // Reads never matter.
        assert_eq!(
            check(
                &EventKind::Access(notify::event::AccessKind::Any),
                &root.join("track.mp3")
            ),
            No
        );
    }

    #[cfg(windows)]
    #[test]
    fn nothing_under_a_hidden_and_system_folder_matters() {
        use notify::event::CreateKind;
        use std::iter;
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM,
        };
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let own = root.join("$RECYCLE.BIN");
        std::fs::create_dir_all(own.join("S-1-5-21")).unwrap();
        let wide: Vec<u16> = own.as_os_str().encode_wide().chain(iter::once(0)).collect();
        assert_ne!(
            unsafe {
                SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM)
            },
            0
        );
        let hidden_only = root.join("Hidden");
        std::fs::create_dir(&hidden_only).unwrap();
        let wide: Vec<u16> = hidden_only
            .as_os_str()
            .encode_wide()
            .chain(iter::once(0))
            .collect();
        assert_ne!(
            unsafe { SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_HIDDEN) },
            0
        );
        let deleted = own.join("S-1-5-21").join("$R1234.mp3");
        std::fs::write(&deleted, b"x").unwrap();
        let users = hidden_only.join("track.mp3");
        std::fs::write(&users, b"x").unwrap();

        let mut cache = HashMap::new();
        let create = EventKind::Create(CreateKind::Any);
        assert_eq!(matters(&create, &deleted, &root, &mut cache), Matters::No);
        assert_eq!(
            matters(&create, &own.join("S-1-5-21"), &root, &mut cache),
            Matters::No
        );
        // Only hidden: the user's, and walked.
        assert_eq!(matters(&create, &users, &root, &mut cache), Matters::Yes);
        // The answer is remembered per folder.
        assert_eq!(cache.get(&own), Some(&true));
        assert_eq!(cache.get(&hidden_only), Some(&false));
    }
}
