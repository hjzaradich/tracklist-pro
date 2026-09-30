//! 1aC-7: stages 1 and 2 of the scan against the ROADMAP 1.1 target,
//! "first results in under 5 s on 100k files, and a full index without
//! fingerprints in minutes".
//!
//! Windows only, like the walk's own tests: the walk and the read job ask
//! Windows about volumes and file ids.
//!
//! Two tests share one harness ([`run`]), which drives the real job queue
//! with the real [`Walker`] and [`Reader`] over a music folder on this PC's
//! own volumes, and measures:
//!
//! - the time to the first `ScannedFiles` batch (what the UI shows first),
//! - the whole stage-1 walk,
//! - the stage-2 read of every file,
//! - a rescan with nothing changed, and a re-read after it,
//! - the process's peak working set, and the database's size.
//!
//! **`hundred_thousand_files`** is ignored by default. It uses the tree in
//! the `TLP_PERF_ROOT` environment variable (point it at fixture-gen's
//! `music/` folder; the README in `tools/fixture-gen` says how to make a
//! 100k-file one) or, with the variable unset, a 100,000-file tree of tiny
//! WAVs it writes itself under the temp folder and deletes afterwards. Run
//! it with
//!
//! ```text
//! cargo test --release --test perf_100k -- --ignored --nocapture
//! ```
//!
//! It also times two floors for comparison, so a miss can be placed: a bare
//! `read_dir` walk of the same tree, and one open plus one 8 KiB read per
//! file. The stage-1 time above the first floor is the walker's own cost
//! (file ids, the database, batching); the stage-2 time above the second is
//! parsing.
//!
//! **`a_thousand_files_go_through_both_stages`** is the smoke test that
//! runs in CI: the same harness on 1,000 tiny WAVs, checking the results
//! rather than the times, so the benchmark can't rot.
//!
//! Nothing here writes outside the temp folder and the tree it's pointed
//! at is only read.
#![cfg(windows)]

use std::fs;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tracklist_pro_lib::db::{Writer, DB_FILE_NAME};
use tracklist_pro_lib::jobs::{self, JobId, JobKind, JobQueue, JobStatus};
use tracklist_pro_lib::paths::SystemVolumes;
use tracklist_pro_lib::read::{read_job, Reader, READ_VERSION};
use tracklist_pro_lib::scan::{folders, is_indexed, scan_job, MusicFolderRole, Walker};
use tracklist_pro_lib::scan_state::{self, Scope, Stage};
use tracklist_pro_lib::write_guard::WriteGuard;
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// ROADMAP 1.1: the first results within this.
const FIRST_RESULTS_TARGET: Duration = Duration::from_secs(5);
/// "In minutes": call it ten.
const FULL_INDEX_TARGET: Duration = Duration::from_secs(10 * 60);

/// The environment variable naming an existing music tree for the big run.
const ROOT_VAR: &str = "TLP_PERF_ROOT";

/// What one run measured.
#[derive(Debug)]
struct Report {
    files_on_disk: usize,
    /// `file` rows after the walk.
    rows: i64,
    /// Files and batches the sink heard during the first walk.
    sink_files: u64,
    sink_batches: u64,
    first_batch: Option<Duration>,
    walk: Duration,
    read: Duration,
    /// Files still due for the read stage after it ran, by reason.
    still_due: u64,
    read_done: i64,
    rescan: Duration,
    /// New rows the sink heard during the rescan (should be none).
    rescan_new: u64,
    reread: Duration,
    peak_working_set: usize,
    baseline_working_set: usize,
    db_bytes: u64,
}

/// Drives a walk, a read, a rescan and a re-read over `music`, one job at a
/// time, and measures each. `files_on_disk` is how many audio files the
/// caller knows are there.
fn run(music: &Path, files_on_disk: usize) -> Report {
    let app_data = tempfile::Builder::new()
        .prefix("tlp-perf-app-data-")
        .tempdir()
        .unwrap();
    let guard = WriteGuard::app_data(app_data.path()).unwrap();
    let db_path = guard.check(&app_data.path().join(DB_FILE_NAME)).unwrap();
    let writer = Writer::open(&db_path).unwrap();

    let volumes = SystemVolumes::scan();
    let folder = folders::add(&writer, &volumes, music, MusicFolderRole::Scan).unwrap();

    // What the frontend would hear: the first batch's time, and counts.
    let sink = Arc::new(Sink::default());
    let heard = sink.clone();
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(
            JobKind::Scan,
            Walker::new(SystemVolumes::scan, move |files| heard.batch(files.len())),
        )
        .handler(JobKind::Read, Reader::new(SystemVolumes::scan))
        .start()
        .unwrap();

    let baseline_working_set = working_set().1;

    // Stage 1.
    sink.start();
    let walk = timed(|| {
        let id = queue.enqueue(scan_job(Some(vec![folder.id]))).unwrap();
        assert_eq!(wait(&writer, id), JobStatus::Done, "the walk failed");
    });
    let first_batch = sink.first_batch();
    let (sink_files, sink_batches) = sink.counts();
    let rows = count(&writer, "SELECT COUNT(*) FROM file WHERE present = 1");

    // Stage 2.
    let read = timed(|| {
        let id = queue.enqueue(read_job(Some(vec![folder.id]))).unwrap();
        assert_eq!(wait(&writer, id), JobStatus::Done, "the read failed");
    });
    let still_due = writer
        .call(|c| scan_state::count_due(c, Stage::Read, READ_VERSION, &Scope::All))
        .unwrap();
    let read_done = count(
        &writer,
        "SELECT COUNT(*) FROM file_stage WHERE stage = 'read' AND status = 'done'",
    );

    // Nothing changed: walk and read again.
    sink.start();
    let rescan = timed(|| {
        let id = queue.enqueue(scan_job(Some(vec![folder.id]))).unwrap();
        assert_eq!(wait(&writer, id), JobStatus::Done, "the rescan failed");
    });
    let (rescan_new, _) = sink.counts();
    let reread = timed(|| {
        let id = queue.enqueue(read_job(Some(vec![folder.id]))).unwrap();
        assert_eq!(wait(&writer, id), JobStatus::Done, "the re-read failed");
    });

    let (peak_working_set, _) = working_set();
    queue.shutdown();
    // Checkpoint so the WAL is folded in and the size is the database's.
    writer
        .call(|c| c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)"))
        .unwrap();
    let db_bytes = fs::metadata(writer.path()).map(|m| m.len()).unwrap_or(0);
    drop(writer);

    Report {
        files_on_disk,
        rows,
        sink_files,
        sink_batches,
        first_batch,
        walk,
        read,
        still_due,
        read_done,
        rescan,
        rescan_new,
        reread,
        peak_working_set,
        baseline_working_set,
        db_bytes,
    }
}

/// The frontend's side of `ScannedFiles`: when the first batch of a run
/// arrived, and how much came through.
#[derive(Default)]
struct Sink {
    started: Mutex<Option<Instant>>,
    first: Mutex<Option<Instant>>,
    files: AtomicU64,
    batches: AtomicU64,
}

impl Sink {
    /// Starts a run's clock and forgets the last run's counts.
    fn start(&self) {
        *self.started.lock().unwrap() = Some(Instant::now());
        *self.first.lock().unwrap() = None;
        self.files.store(0, Ordering::SeqCst);
        self.batches.store(0, Ordering::SeqCst);
    }

    fn batch(&self, files: usize) {
        self.first.lock().unwrap().get_or_insert_with(Instant::now);
        self.files.fetch_add(files as u64, Ordering::SeqCst);
        self.batches.fetch_add(1, Ordering::SeqCst);
    }

    /// How long after the run started the first batch arrived.
    fn first_batch(&self) -> Option<Duration> {
        let started = (*self.started.lock().unwrap())?;
        let first = (*self.first.lock().unwrap())?;
        Some(first.duration_since(started))
    }

    fn counts(&self) -> (u64, u64) {
        (
            self.files.load(Ordering::SeqCst),
            self.batches.load(Ordering::SeqCst),
        )
    }
}

fn timed(work: impl FnOnce()) -> Duration {
    let start = Instant::now();
    work();
    start.elapsed()
}

fn count(writer: &Writer, sql: &'static str) -> i64 {
    writer
        .call(move |c| c.query_row(sql, [], |r| r.get(0)))
        .unwrap()
}

/// Waits for the job to finish. Polls, as the read job's tests do.
fn wait(writer: &Writer, id: JobId) -> JobStatus {
    let start = Instant::now();
    loop {
        let job = writer
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        if job.status.is_finished() {
            if let Some(error) = &job.error {
                eprintln!("job {id} ended with: {error}");
            }
            return job.status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(3 * 60 * 60),
            "the job never finished"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// This process's (peak, current) working set in bytes.
fn working_set() -> (usize, usize) {
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        PageFaultCount: 0,
        PeakWorkingSetSize: 0,
        WorkingSetSize: 0,
        QuotaPeakPagedPoolUsage: 0,
        QuotaPagedPoolUsage: 0,
        QuotaPeakNonPagedPoolUsage: 0,
        QuotaNonPagedPoolUsage: 0,
        PagefileUsage: 0,
        PeakPagefileUsage: 0,
    };
    // SAFETY: the current process's pseudo-handle is always valid, and the
    // struct and its size are what the call expects.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    assert_ne!(ok, 0, "GetProcessMemoryInfo failed");
    (counters.PeakWorkingSetSize, counters.WorkingSetSize)
}

fn mib(bytes: impl Into<u64>) -> String {
    format!("{:.0} MiB", bytes.into() as f64 / (1024.0 * 1024.0))
}

fn secs(d: Duration) -> String {
    format!("{:7.2} s", d.as_secs_f64())
}

/// Per-file rate, for the eye.
fn per_file(d: Duration, files: usize) -> String {
    if files == 0 {
        return String::new();
    }
    let us = d.as_secs_f64() * 1e6 / files as f64;
    format!("({us:.0} µs/file)")
}

fn verdict(d: Duration, target: Duration) -> &'static str {
    if d <= target {
        "PASS"
    } else {
        "MISS"
    }
}

fn print(report: &Report, music: &Path, source: &str) {
    let n = report.files_on_disk;
    println!();
    println!(
        "perf_100k: {n} audio files under {} ({source})",
        music.display()
    );
    println!(
        "machine: {} logical CPUs; one job worker; release build: {}",
        std::thread::available_parallelism().map_or(0, |p| p.get()),
        !cfg!(debug_assertions)
    );
    match report.first_batch {
        Some(d) => println!(
            "  first ScannedFiles batch   {}   target < {} s   {}",
            secs(d),
            FIRST_RESULTS_TARGET.as_secs(),
            verdict(d, FIRST_RESULTS_TARGET)
        ),
        None => println!("  first ScannedFiles batch   never arrived   MISS"),
    }
    println!(
        "  stage 1 walk, all rows     {}   {} rows, {} files in {} batches {}",
        secs(report.walk),
        report.rows,
        report.sink_files,
        report.sink_batches,
        per_file(report.walk, n)
    );
    println!(
        "  stage 2 read, all files    {}   {} done, {} still due {}",
        secs(report.read),
        report.read_done,
        report.still_due,
        per_file(report.read, n)
    );
    let full = report.walk + report.read;
    println!(
        "  walk + read                {}   target < {} min   {}",
        secs(full),
        FULL_INDEX_TARGET.as_secs() / 60,
        verdict(full, FULL_INDEX_TARGET)
    );
    println!(
        "  rescan, nothing changed    {}   {} new rows {}",
        secs(report.rescan),
        report.rescan_new,
        per_file(report.rescan, n)
    );
    println!("  re-read, nothing due       {}", secs(report.reread));
    println!(
        "  peak working set           {} (baseline {} before the walk)",
        mib(report.peak_working_set as u64),
        mib(report.baseline_working_set as u64)
    );
    println!("  database                   {}", mib(report.db_bytes));
}

/// A bare walk of `root`: what the file system alone costs to list every
/// entry and stat every audio file the walk would index. Returns (time,
/// files seen).
fn walk_floor(root: &Path) -> (Duration, usize) {
    let start = Instant::now();
    let mut files = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() && is_indexed(&entry.file_name().to_string_lossy()) {
                let _ = entry.metadata();
                files += 1;
            }
        }
    }
    (start.elapsed(), files)
}

/// One open and one 8 KiB read per file under `root`: what touching every
/// file costs before any parsing. Returns (time, files opened).
fn open_floor(root: &Path) -> (Duration, usize) {
    let mut paths = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() && is_indexed(&entry.file_name().to_string_lossy()) {
                paths.push(entry.path());
            }
        }
    }
    let start = Instant::now();
    let mut buf = vec![0u8; 8192];
    let mut opened = 0;
    for path in &paths {
        if let Ok(mut file) = fs::File::open(path) {
            let _ = file.read(&mut buf);
            opened += 1;
        }
    }
    (start.elapsed(), opened)
}

/// Writes `count` tiny, valid mono 16-bit WAVs under `root`, 25 to a
/// folder, three folders deep. Each differs in length, so no two are
/// byte-identical. Returns how many it wrote.
fn write_wav_tree(root: &Path, count: usize) -> usize {
    const PER_FOLDER: usize = 25;
    let mut dir = PathBuf::new();
    for i in 0..count {
        if i % PER_FOLDER == 0 {
            let folder = i / PER_FOLDER;
            dir = root
                .join(format!("Genre {}", folder / 100))
                .join(format!("Artist {}", folder / 10 % 10))
                .join(format!("Album {}", folder % 10));
            fs::create_dir_all(&dir).unwrap();
        }
        let samples = 441 + (i % 50) * 20;
        let mut file = fs::File::create(dir.join(format!("Track {i:06}.wav"))).unwrap();
        file.write_all(&wav(samples, i as u32)).unwrap();
    }
    count
}

/// A mono 16-bit 22.05 kHz WAV of `samples` samples of a quiet ramp seeded
/// by `seed`.
fn wav(samples: usize, seed: u32) -> Vec<u8> {
    let data = samples * 2;
    let mut bytes = Vec::with_capacity(44 + data);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&22_050u32.to_le_bytes());
    bytes.extend_from_slice(&(22_050u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data as u32).to_le_bytes());
    for n in 0..samples {
        let v = ((n as u32).wrapping_mul(37).wrapping_add(seed) % 2000) as i16 - 1000;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

/// The smoke test: the harness on a small tree, checking what it finds.
#[test]
fn a_thousand_files_go_through_both_stages() {
    const FILES: usize = 1_000;
    let tree = tempfile::Builder::new()
        .prefix("tlp-perf-smoke-")
        .tempdir()
        .unwrap();
    let music = tree.path().join("music");
    write_wav_tree(&music, FILES);

    let report = run(&music, FILES);
    print(&report, &music, "synthetic WAVs");

    assert_eq!(report.rows, FILES as i64, "every file gets a row");
    assert_eq!(
        report.sink_files, FILES as u64,
        "every new row reaches the sink"
    );
    assert!(report.sink_batches >= 1);
    assert!(report.first_batch.is_some(), "a batch arrived");
    assert_eq!(report.read_done, FILES as i64, "every file was read");
    assert_eq!(report.still_due, 0, "nothing is due after the read");
    assert_eq!(
        report.rescan_new, 0,
        "a rescan of an unchanged tree adds no rows"
    );
    assert!(report.peak_working_set >= report.baseline_working_set);
    assert!(report.db_bytes > 0);
}

/// The benchmark. See the module docs.
#[test]
#[ignore = "takes minutes and needs a 100k-file tree; see the module docs"]
fn hundred_thousand_files() {
    const FILES: usize = 100_000;
    let own_tree;
    let (music, files, source) = match std::env::var_os(ROOT_VAR) {
        Some(root) => {
            let root = PathBuf::from(root);
            assert!(
                root.is_dir(),
                "{ROOT_VAR} must name a folder: {}",
                root.display()
            );
            let (took, files) = walk_floor(&root);
            println!(
                "counted {files} files under {} in {}",
                root.display(),
                secs(took)
            );
            (root, files, format!("from {ROOT_VAR}"))
        }
        None => {
            println!("{ROOT_VAR} is unset: writing {FILES} tiny WAVs under the temp folder");
            own_tree = tempfile::Builder::new()
                .prefix("tlp-perf-100k-")
                .tempdir()
                .unwrap();
            let music = own_tree.path().join("music");
            let took = timed(|| {
                write_wav_tree(&music, FILES);
            });
            println!("wrote them in {}", secs(took));
            (music, FILES, "synthetic WAVs".to_owned())
        }
    };

    let report = run(&music, files);
    print(&report, &music, &source);

    // Floors, on a cache the runs above have warmed.
    let (walk, seen) = walk_floor(&music);
    println!(
        "  floor: bare read_dir walk  {}   {} files {}",
        secs(walk),
        seen,
        per_file(walk, seen)
    );
    let (open, opened) = open_floor(&music);
    println!(
        "  floor: open + 8 KiB read   {}   {} files {}",
        secs(open),
        opened,
        per_file(open, opened)
    );
    println!();

    assert_eq!(report.rows, files as i64, "every file gets a row");
    assert_eq!(
        report.rescan_new, 0,
        "a rescan of an unchanged tree adds no rows"
    );
}
