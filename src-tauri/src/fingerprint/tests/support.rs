//! Test setup: a migrated database, a made-up volume mounted at a temp
//! folder, a music folder on it, the real walk to fill `file` rows, and a
//! queue running the fingerprinter.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::db::Writer;
use crate::fingerprint::{ledger, Fingerprint, Fingerprinter, FirstUp};
use crate::hash::Hasher;
use crate::jobs::{self, JobId, JobKind, JobQueue, JobRecord, JobUpdate};
use crate::paths::Volumes;
use crate::scan::folders::add;
use crate::scan::walk::{scan_job, ScannedFile, Walker};
use crate::scan::MusicFolderRole;
use crate::volume::{identity, IdentitySignals, Volume, VolumeId, VolumeKind};

/// Long enough that a passing test never hits it.
pub const PATIENCE: Duration = Duration::from_secs(300);

/// One volume "mounted" at a real temp folder; it can be unplugged.
#[derive(Clone)]
pub struct TempVolume {
    pub id: VolumeId,
    pub mount: PathBuf,
    online: Arc<AtomicBool>,
}

impl TempVolume {
    fn new(mount: &Path) -> TempVolume {
        TempVolume {
            id: identity(IdentitySignals {
                kind: VolumeKind::External,
                unc_share: None,
                serial: Some(0xF1F1),
                filesystem: "NTFS",
                guid: None,
            })
            .unwrap(),
            mount: fs::canonicalize(mount).unwrap(),
            online: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn set_online(&self, online: bool) {
        self.online.store(online, Ordering::SeqCst);
    }
}

impl Volumes for TempVolume {
    fn real_path(&self, path: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(path)
    }

    fn volume_for(&self, path: &Path) -> io::Result<Volume> {
        if !path.starts_with(&self.mount) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "not on the test volume",
            ));
        }
        Ok(Volume {
            id: self.id.clone(),
            label: "TEST".into(),
            mount_path: self.mount.clone(),
            kind: VolumeKind::External,
        })
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        (self.online.load(Ordering::SeqCst) && *id == self.id).then(|| self.mount.clone())
    }
}

/// A music folder with a database: the whole stage-1-to-3 setup.
pub struct Library {
    /// Holds the database under `data/` and the music under `music/`.
    pub base: PathBuf,
    pub music: PathBuf,
    pub data: PathBuf,
    pub writer: Writer,
    pub volume: TempVolume,
    pub first: FirstUp,
    _dir: tempfile::TempDir,
}

impl Library {
    pub fn new() -> Library {
        Library::with_data_in(None)
    }

    /// The database in `data`, e.g. inside a sandbox being watched.
    pub fn with_data_in(data: Option<&Path>) -> Library {
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let music = base.join("music");
        let data = data.map_or_else(|| base.join("data"), Path::to_path_buf);
        fs::create_dir_all(&music).unwrap();
        fs::create_dir_all(&data).unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(
            &data,
            crate::db::DB_FILE_NAME,
        ))
        .unwrap();
        let volume = TempVolume::new(&base);
        add(&writer, &volume, &music, MusicFolderRole::Scan).unwrap();
        Library {
            base,
            music,
            data,
            writer,
            volume,
            first: FirstUp::default(),
            _dir: dir,
        }
    }

    /// Writes `bytes` at `rel` (`/`-separated) in the music folder.
    pub fn put(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let mut path = self.music.clone();
        for part in rel.split('/') {
            path.push(part);
        }
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    /// A fingerprinter over this library's volume, sharing its line and
    /// thread budget, on one thread unless changed.
    pub fn fingerprinter(&self) -> Fingerprinter<TempVolume> {
        let volume = self.volume.clone();
        Fingerprinter::new(move || volume.clone(), self.first.clone()).threads(1)
    }

    /// A queue with one worker that walks and fingerprints.
    pub fn queue(&self, fingerprinter: Fingerprinter<TempVolume>) -> JobQueue {
        self.queue_with_updates(fingerprinter, |_| {})
    }

    pub fn queue_with_updates(
        &self,
        fingerprinter: Fingerprinter<TempVolume>,
        updates: impl Fn(&[JobUpdate]) + Send + 'static,
    ) -> JobQueue {
        self.queue_with(fingerprinter, 1, updates)
    }

    /// A queue with `workers` workers, sending job updates to `updates`.
    pub fn queue_with(
        &self,
        fingerprinter: Fingerprinter<TempVolume>,
        workers: usize,
        updates: impl Fn(&[JobUpdate]) + Send + 'static,
    ) -> JobQueue {
        let volume = self.volume.clone();
        let hash_volume = self.volume.clone();
        JobQueue::builder(self.writer.clone())
            .workers(workers)
            .on_updates(updates)
            .handler(
                JobKind::Scan,
                Walker::new(move || volume.clone(), |_: Vec<ScannedFile>| {}),
            )
            .handler(JobKind::Hash, Hasher::new(move || hash_volume.clone()))
            .handler(JobKind::Fingerprint, fingerprinter)
            .start()
            .unwrap()
    }

    /// Runs the real walk, so `file` rows are exactly what stage 1 makes.
    /// Returns each file's id by its path.
    pub fn walk(&self) -> BTreeMap<String, i64> {
        let queue = self.queue(self.fingerprinter());
        let id = queue.enqueue(scan_job(None)).unwrap();
        assert_eq!(wait(&self.writer, id).status, jobs::JobStatus::Done);
        queue.shutdown();
        self.ids()
    }

    /// Every file's id by its path.
    pub fn ids(&self) -> BTreeMap<String, i64> {
        self.writer
            .call(|c| {
                let mut s = c.prepare("SELECT rel_path, id FROM file")?;
                let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect()
            })
            .unwrap()
    }

    /// The stored fingerprint blob of file `id`.
    pub fn blob(&self, id: i64) -> Option<Vec<u8>> {
        self.writer
            .call(move |c| {
                c.query_row("SELECT fingerprint FROM file WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
            })
            .unwrap()
    }

    pub fn fingerprint(&self, id: i64) -> Option<Fingerprint> {
        self.blob(id)
            .map(|b| Fingerprint::from_blob(&b).expect("a stored fingerprint reads back"))
    }

    /// How the last try at file `id` ended.
    pub fn outcome(&self, id: i64) -> Option<crate::fingerprint::Outcome> {
        self.writer.call(move |c| ledger::outcome(c, id)).unwrap()
    }
}

/// Waits until job `id` has finished and returns it.
pub fn wait(writer: &Writer, id: JobId) -> JobRecord {
    let start = Instant::now();
    loop {
        let job = writer
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        if job.status.is_finished() {
            return job;
        }
        assert!(
            start.elapsed() < PATIENCE,
            "job {id} never finished: {job:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
