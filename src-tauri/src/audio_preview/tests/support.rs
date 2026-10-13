//! Test setup: a migrated database, a made-up volume mounted at a temp
//! folder with a music folder on it, and a way to ask the preview for a
//! file the way the webview does.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::http::{header, Request};

use crate::audio_preview::serve::{answer, Reply};
use crate::db::{ReadPool, Writer};
use crate::paths::Volumes;
use crate::scan::folders::add;
use crate::scan::MusicFolderRole;
use crate::volume::{identity, IdentitySignals, Volume, VolumeId, VolumeKind};

/// One volume "mounted" at a real temp folder; it can be unplugged.
#[derive(Clone)]
pub struct TempVolume {
    id: VolumeId,
    mount: PathBuf,
    online: Arc<AtomicBool>,
}

impl TempVolume {
    fn new(mount: &Path) -> TempVolume {
        TempVolume {
            id: identity(IdentitySignals {
                kind: VolumeKind::External,
                unc_share: None,
                serial: Some(0xA11A),
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

/// A music folder with a database, and the preview over it.
pub struct Lib {
    pub music: PathBuf,
    /// A folder beside the music folder: on the volume, but in no music
    /// folder.
    pub outside: PathBuf,
    pub writer: Writer,
    pub reads: ReadPool,
    pub volume: TempVolume,
    folder: i64,
    _dir: tempfile::TempDir,
}

/// What the webview gets back.
#[derive(Debug)]
pub struct Got {
    pub status: u16,
    pub body: Vec<u8>,
    reply: Reply,
}

impl Got {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.reply.headers().get(name)?.to_str().ok()
    }

    /// The refusal code a refused request names.
    pub fn refusal(&self) -> Option<&str> {
        self.header(crate::audio_preview::serve::REFUSAL_HEADER)
    }
}

impl Lib {
    pub fn new() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let (music, outside, data) = (base.join("music"), base.join("outside"), base.join("data"));
        for folder in [&music, &outside, &data] {
            fs::create_dir_all(folder).unwrap();
        }
        let writer = Writer::open(&crate::write_guard::test_path(
            &data,
            crate::db::DB_FILE_NAME,
        ))
        .unwrap();
        let reads = ReadPool::open(writer.guarded_path()).unwrap();
        let volume = TempVolume::new(&base);
        let folder = add(&writer, &volume, &music, MusicFolderRole::Scan)
            .unwrap()
            .id
            .0;
        Lib {
            music,
            outside,
            writer,
            reads,
            volume,
            folder,
            _dir: dir,
        }
    }

    /// Writes `bytes` at `rel` (`/`-separated) in the music folder and
    /// gives it a `file` row, as a scan would. Returns the file id.
    pub fn put(&self, rel: &str, bytes: &[u8]) -> i64 {
        let path = self.path_of(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        self.row(rel, true, false)
    }

    /// Where `rel` is on disk.
    pub fn path_of(&self, rel: &str) -> PathBuf {
        rel.split('/')
            .fold(self.music.clone(), |p, part| p.join(part))
    }

    /// A `file` row with nothing on disk behind it.
    pub fn row(&self, rel: &str, present: bool, online_only: bool) -> i64 {
        let (folder, rel) = (self.folder, rel.to_owned());
        self.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present, online_only)
                     VALUES (?1, ?2, ?2, ?3, ?4)",
                    (folder, rel, present, online_only),
                )?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    /// Asks for `/<path>` the way the webview does, with a `Range` if given.
    pub fn ask(&self, path: &str, range: Option<&str>) -> Got {
        let mut request = Request::builder()
            .method("GET")
            .uri(format!("http://audio-preview.localhost{path}"));
        if let Some(range) = range {
            request = request.header(header::RANGE, range);
        }
        let reply = answer(
            &self.reads,
            &self.volume,
            &request.body(Vec::new()).unwrap(),
        );
        Got {
            status: reply.status().as_u16(),
            body: reply.body().to_vec(),
            reply,
        }
    }

    /// Asks for file `id`.
    pub fn get(&self, id: i64, range: Option<&str>) -> Got {
        self.ask(&format!("/{id}"), range)
    }

    /// Asks for bytes `from..=to` of file `id`.
    pub fn bytes(&self, id: i64, from: u64, to: u64) -> Got {
        self.get(id, Some(&format!("bytes={from}-{to}")))
    }
}
