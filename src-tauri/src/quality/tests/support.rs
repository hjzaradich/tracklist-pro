//! Test setup: a migrated database, a made-up volume mounted at a temp
//! folder, a music folder on it, and a queue that runs the real walk, the
//! real hash stage and the quality job.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::db::Writer;
use crate::fingerprint::FirstUp;
use crate::hash::{hash_job, Hasher};
use crate::jobs::{self, JobId, JobKind, JobQueue, JobRecord, JobStatus};
use crate::paths::Volumes;
use crate::quality::ledger::{self, Stored};
use crate::quality::Qualifier;
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

/// A music folder with a database.
pub struct Library {
    pub music: PathBuf,
    pub writer: Writer,
    pub volume: TempVolume,
    pub first: FirstUp,
    _dir: tempfile::TempDir,
}

impl Library {
    pub fn new() -> Library {
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let music = base.join("music");
        let data = base.join("data");
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
            music,
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

    /// A quality handler over this library's volume, on one thread.
    pub fn qualifier(&self) -> Qualifier<TempVolume> {
        let volume = self.volume.clone();
        Qualifier::new(move || volume.clone(), self.first.clone()).threads(1)
    }

    /// A queue with one worker that walks, hashes and measures quality.
    pub fn queue(&self, qualifier: Qualifier<TempVolume>) -> JobQueue {
        let volume = self.volume.clone();
        let hash_volume = self.volume.clone();
        JobQueue::builder(self.writer.clone())
            .workers(1)
            .handler(
                JobKind::Scan,
                Walker::new(move || volume.clone(), |_: Vec<ScannedFile>| {}),
            )
            .handler(JobKind::Hash, Hasher::new(move || hash_volume.clone()))
            .handler(JobKind::Quality, qualifier)
            .start()
            .unwrap()
    }

    /// Runs the real walk and the real hash stage, so `file` rows are
    /// exactly what stages 1 and 3 make. Returns each file's id by its path.
    pub fn walk_and_hash(&self) -> BTreeMap<String, i64> {
        let queue = self.queue(self.qualifier());
        for job in [scan_job(None), hash_job(None)] {
            let id = queue.enqueue(job).unwrap();
            assert_eq!(wait(&self.writer, id).status, JobStatus::Done);
        }
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

    /// The `audio_hash` of file `id`.
    pub fn audio_hash(&self, id: i64) -> Vec<u8> {
        self.writer
            .call(move |c| {
                c.query_row("SELECT audio_hash FROM file WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
            })
            .unwrap()
    }

    /// What the quality job stored for file `id`.
    pub fn stored(&self, id: i64) -> Option<Stored> {
        self.writer.call(move |c| ledger::stored(c, id)).unwrap()
    }

    /// How many files have a stored measurement.
    pub fn measured_files(&self) -> i64 {
        self.writer
            .call(|c| c.query_row("SELECT count(*) FROM file_quality", [], |r| r.get(0)))
            .unwrap()
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
