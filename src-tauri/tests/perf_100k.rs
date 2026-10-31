//! 1aC-7: the scan on 100k files against the ROADMAP 1.1 target, "first
//! results in under 5 s on 100k files, and a full index without
//! fingerprints in minutes".
//!
//! Windows only, like the walk's own tests: the walk and the read job ask
//! Windows about volumes and file ids.
//!
//! Two tests share one harness ([`run`]), which drives the real job queue
//! with the real handlers, chained as the app chains them
//! (`scan::chain`: walk → read → hashes → grouping; fingerprints are a
//! no-op here), over a music folder on this PC's own volumes, and
//! measures:
//!
//! - the time to the first new rows in the database (what the UI can show
//!   first; polled from a second, read-only connection, see [`FirstRows`]),
//! - the whole stage-1 walk, and the chained stage-2 read: the target,
//! - the chained hashes and grouping, reported beside it,
//! - a rescan with nothing changed (the unchanged check, 1aC-1) and what
//!   the chain queues after it,
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
//! Two more variables shape the big run: `TLP_PERF_THREADS` pins the read
//! job's thread count (1 for a one-thread run to compare against), and
//! `TLP_PERF_EVICT=<GiB>` pushes the file cache out of memory right before
//! it ([`evict_file_cache`]), for a cold run without a reboot.
//!
//! **`stage_2_parts_per_format`** (also ignored) times the sniff and the
//! tag read on their own over a sample of each format, to place stage 2's
//! time. **`a_thousand_files_go_through_the_chain`** is the smoke test that runs
//! in CI: the same harness on 1,000 tiny WAVs, checking the results rather
//! than the times, so the benchmark can't rot.
//!
//! Nothing here writes outside the temp folder, and the tree it's pointed
//! at is only read.
#![cfg(windows)]

use std::fs;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tracklist_pro_lib::db::{Writer, DB_FILE_NAME};
use tracklist_pro_lib::jobs::{self, JobContext, JobId, JobKind, JobQueue, JobStatus};
use tracklist_pro_lib::paths::SystemVolumes;
use tracklist_pro_lib::read::{Reader, READ_VERSION};
use tracklist_pro_lib::scan::{chain, folders, is_indexed, scan_job, MusicFolderRole, Walker};
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
/// Pins how many threads the read job may use (its own included), so a
/// one-thread run can be compared with the app's default. Unset: the
/// app's default.
const THREADS_VAR: &str = "TLP_PERF_THREADS";
/// How many GiB to allocate and touch right before the big run, to push
/// the file cache out of memory ([`evict_file_cache`]). Unset: nothing.
/// The process's peak working set then includes that memory, so the
/// memory figures of such a run say nothing about the app.
const EVICT_VAR: &str = "TLP_PERF_EVICT";

/// What one run measured.
#[derive(Debug)]
struct Report {
    files_on_disk: usize,
    /// `file` rows after the walk.
    rows: i64,
    /// How long after the first walk started its first rows were readable.
    first_rows: Option<Duration>,
    walk: Duration,
    /// The chained read, from the walk's end to its end.
    read: Duration,
    /// Files still due for the read stage after it ran.
    still_due: u64,
    read_done: i64,
    /// The chained hashes, from the read's end to their end.
    hash: Duration,
    hash_done: i64,
    /// The chained grouping, from the hashes' end to its end.
    group: Duration,
    recordings: i64,
    /// The peak working set once stages 1 and 2 were done.
    peak_after_read: usize,
    rescan: Duration,
    /// Rows the rescan added (should be none).
    rescan_new: i64,
    /// The kinds of job the chain queued after the rescan, in order.
    after_rescan: Vec<String>,
    /// From the rescan's end until nothing was queued or running.
    settle: Duration,
    peak_working_set: usize,
    baseline_working_set: usize,
    db_bytes: u64,
}

/// Drives a walk and everything the chain queues after it, then a rescan,
/// one job at a time, and measures each. `files_on_disk` is how many
/// audio files the caller knows are there.
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

    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(
            JobKind::Scan,
            chain::after_walk(Walker::new(SystemVolumes::scan)),
        )
        .handler(
            JobKind::Read,
            chain::after_read(match read_threads() {
                Some(n) => Reader::new(SystemVolumes::scan).threads(n),
                None => Reader::new(SystemVolumes::scan),
            }),
        )
        .handler(
            JobKind::Hash,
            chain::after_hash(tracklist_pro_lib::hash::hasher()),
        )
        .handler(JobKind::Relink, tracklist_pro_lib::relink::relinker())
        .handler(
            JobKind::Group,
            chain::after_group(tracklist_pro_lib::grouping::Grouper::default()),
        )
        // Fingerprints are stage 3 and outside the target: accepted, not done.
        .handler(JobKind::Fingerprint, |_: &JobContext| Ok(()))
        .start()
        .unwrap();

    let baseline_working_set = working_set().1;

    // Stage 1, then what the chain queues: read, hashes, grouping.
    let first_rows = FirstRows::start(writer.path());
    let scan = queue.enqueue(scan_job(Some(vec![folder.id]))).unwrap();
    let walk = timed(|| assert_done(&writer, scan, "the walk"));
    let first_rows = first_rows.finish();
    let rows = count(&writer, "SELECT COUNT(*) FROM file WHERE present = 1");

    let read = timed(|| wait_next(&writer, JobKind::Read, scan, "the chained read"));
    let still_due = writer
        .call(|c| scan_state::count_due(c, Stage::Read, READ_VERSION, &Scope::All))
        .unwrap();
    let read_done = count(
        &writer,
        "SELECT COUNT(*) FROM file_stage WHERE stage = 'read' AND status = 'done'",
    );
    let peak_after_read = working_set().0;

    let hash = timed(|| wait_next(&writer, JobKind::Hash, scan, "the chained hashes"));
    let hash_done = count(
        &writer,
        "SELECT COUNT(*) FROM file_stage WHERE stage = 'hash' AND status = 'done'",
    );
    let group = timed(|| wait_next(&writer, JobKind::Group, scan, "the chained grouping"));
    let recordings = count(&writer, "SELECT COUNT(*) FROM recording");
    wait_idle(&writer);

    // Nothing changed: walk again, and see what the chain makes of it.
    let rows_before = count(&writer, "SELECT COUNT(*) FROM file");
    let rescan_job = queue.enqueue(scan_job(Some(vec![folder.id]))).unwrap();
    let rescan = timed(|| assert_done(&writer, rescan_job, "the rescan"));
    let rescan_new = count(&writer, "SELECT COUNT(*) FROM file") - rows_before;
    let settle = timed(|| wait_idle(&writer));
    let after_rescan = kinds_after(&writer, rescan_job);
    print_jobs(&writer);

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
        first_rows,
        walk,
        read,
        still_due,
        read_done,
        hash,
        hash_done,
        group,
        recordings,
        peak_after_read,
        rescan,
        rescan_new,
        after_rescan,
        settle,
        peak_working_set,
        baseline_working_set,
        db_bytes,
    }
}

/// When the first rows of a walk could be read from the database: the
/// moment the first batch's transaction commits, which is when the screens
/// can first list a new track. A second, read-only connection polls for a
/// `file` row while the walk runs, so the walk does the same work as in the
/// app and nothing is handed to it.
struct FirstRows {
    started: Instant,
    stop: Arc<AtomicBool>,
    poller: JoinHandle<Option<Instant>>,
}

impl FirstRows {
    /// How often the poller looks. Small beside the 5 s target.
    const EVERY: Duration = Duration::from_millis(2);

    /// Starts the clock and the poller on the database at `db`.
    fn start(db: &Path) -> FirstRows {
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, db) = (stop.clone(), db.to_owned());
        let started = Instant::now();
        let poller = std::thread::spawn(move || {
            let conn = rusqlite::Connection::open_with_flags(
                db,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            conn.busy_timeout(Duration::from_secs(5)).unwrap();
            let any = || {
                conn.query_row("SELECT EXISTS (SELECT 1 FROM file)", [], |r| r.get(0))
                    .unwrap_or(false)
            };
            loop {
                let stopped = flag.load(Ordering::SeqCst);
                if any() {
                    return Some(Instant::now());
                }
                if stopped {
                    return None;
                }
                std::thread::sleep(Self::EVERY);
            }
        });
        FirstRows {
            started,
            stop,
            poller,
        }
    }

    /// Stops the poller (after the walk ended) and says how long after
    /// [`FirstRows::start`] the first row could be read, if any could.
    fn finish(self) -> Option<Duration> {
        self.stop.store(true, Ordering::SeqCst);
        let first = self.poller.join().unwrap()?;
        Some(first.duration_since(self.started))
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

/// No job should take longer than this, even on a slow disk.
const JOB_LIMIT: Duration = Duration::from_secs(3 * 60 * 60);
const POLL: Duration = Duration::from_millis(5);

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
        assert!(start.elapsed() < JOB_LIMIT, "job {id} never finished");
        std::thread::sleep(POLL);
    }
}

fn assert_done(writer: &Writer, id: JobId, what: &str) {
    assert_eq!(wait(writer, id), JobStatus::Done, "{what} didn't finish");
}

/// Waits for the first job of `kind` queued after `after` to finish. The
/// chain queues the next stage before the stage before it is recorded as
/// done, so the job is there by the time the caller looks.
fn wait_next(writer: &Writer, kind: JobKind, after: JobId, what: &str) {
    let id: Option<i64> = writer
        .call(move |c| {
            c.query_row(
                "SELECT id FROM job WHERE kind = ?1 AND id > ?2 ORDER BY id LIMIT 1",
                (kind.as_str(), after.0),
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })
        })
        .unwrap();
    let id = id.unwrap_or_else(|| panic!("{what} was never queued"));
    assert_done(writer, JobId(id), what);
}

/// Waits until nothing is queued or running.
fn wait_idle(writer: &Writer) {
    let start = Instant::now();
    while count(
        writer,
        "SELECT COUNT(*) FROM job WHERE status IN ('queued', 'running')",
    ) > 0
    {
        assert!(start.elapsed() < JOB_LIMIT, "the queue never went idle");
        std::thread::sleep(POLL);
    }
}

/// Prints every job's kind, outcome and run time, from its own start and end
/// stamps (1cC-M1), so the time between two steps can be placed in a job.
fn print_jobs(writer: &Writer) {
    let rows: Vec<(i64, String, String, Option<String>, f64)> = writer
        .call(|c| {
            let mut stmt = c.prepare(
                "SELECT id, kind, status, error,
                        ifnull((julianday(finished_at) - julianday(started_at)) * 86400.0, 0.0)
                 FROM job ORDER BY id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?;
            rows.collect()
        })
        .unwrap();
    println!("jobs, in order (run time from the job's own stamps):");
    for (id, kind, status, error, took) in rows {
        println!(
            "  job {id:>3} {kind:<12} {status:<9} {took:>8.2} s{}",
            error.map_or(String::new(), |e| format!("   error: {e}"))
        );
    }
}

/// The kinds of the jobs queued after `after`, in order.
fn kinds_after(writer: &Writer, after: JobId) -> Vec<String> {
    writer
        .call(move |c| {
            let mut stmt = c.prepare("SELECT kind FROM job WHERE id > ?1 ORDER BY id")?;
            let rows = stmt.query_map([after.0], |r| r.get(0))?;
            rows.collect()
        })
        .unwrap()
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
        "machine: {} logical CPUs; one job worker; read threads: {}; release build: {}",
        std::thread::available_parallelism().map_or(0, |p| p.get()),
        read_threads().map_or("the app's default".to_owned(), |n| n.to_string()),
        !cfg!(debug_assertions)
    );
    println!("the target (ROADMAP 1.1):");
    match report.first_rows {
        Some(d) => println!(
            "  first rows in the database {}   target < {} s   {}",
            secs(d),
            FIRST_RESULTS_TARGET.as_secs(),
            verdict(d, FIRST_RESULTS_TARGET)
        ),
        None => println!("  first rows in the database never arrived   MISS"),
    }
    println!(
        "  stage 1 walk, all rows     {}   {} rows {}",
        secs(report.walk),
        report.rows,
        per_file(report.walk, n)
    );
    println!(
        "  stage 2 read, chained      {}   {} done, {} still due {}",
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
        "  peak working set           {} (baseline {} before the walk){}",
        mib(report.peak_after_read as u64),
        mib(report.baseline_working_set as u64),
        if evict_gib().is_some() {
            "; includes the memory TLP_PERF_EVICT touched, so see a run without it"
        } else {
            ""
        }
    );
    println!("the rest of the chain, and a rescan:");
    println!(
        "  stage 3 hashes, chained    {}   {} done {}",
        secs(report.hash),
        report.hash_done,
        per_file(report.hash, n)
    );
    println!(
        "  grouping, chained          {}   {} recordings",
        secs(report.group),
        report.recordings
    );
    println!(
        "  rescan, nothing changed    {}   {} new rows {}",
        secs(report.rescan),
        report.rescan_new,
        per_file(report.rescan, n)
    );
    println!(
        "  after the rescan           {}   queued: {}",
        secs(report.settle),
        if report.after_rescan.is_empty() {
            "nothing".to_owned()
        } else {
            report.after_rescan.join(", ")
        }
    );
    println!(
        "  peak working set, all      {}",
        mib(report.peak_working_set as u64)
    );
    println!("  database                   {}", mib(report.db_bytes));
}

/// The read thread count pinned by `TLP_PERF_THREADS`, if any.
fn read_threads() -> Option<usize> {
    std::env::var(THREADS_VAR)
        .ok()
        .map(|s| s.parse().expect("TLP_PERF_THREADS is a thread count"))
}

/// The GiB to touch before the run, from `TLP_PERF_EVICT`, if set.
fn evict_gib() -> Option<usize> {
    std::env::var(EVICT_VAR)
        .ok()
        .map(|s| s.parse().expect("TLP_PERF_EVICT is a number of GiB"))
}

/// Pushes the file cache out of memory the blunt way: allocates `gib`
/// GiB in 256 MiB pieces, touches one byte per 4 KiB page so every page
/// is really committed, holds it all at once, then frees it. Windows
/// gives the memory up from the standby list (the file cache) first, so
/// a run after this reads the files from disk, as a first index does.
///
/// Give it most of the machine's RAM (12 on a 16 GiB laptop) and run it
/// with nothing else open; other programs get paged out meanwhile and
/// come back slowly. For a genuinely first read, the files must also be
/// ones nothing has opened before (a fresh fixture), or Defender has
/// already judged them.
fn evict_file_cache(gib: usize) {
    const PIECE: usize = 256 << 20;
    const PAGE: usize = 4096;
    let mut held: Vec<Vec<u8>> = Vec::new();
    for _ in 0..(gib << 30) / PIECE {
        let mut piece = vec![0u8; PIECE];
        for page in (0..PIECE).step_by(PAGE) {
            piece[page] = 1;
        }
        held.push(piece);
    }
    // Read something back, so the touching can't be optimised away.
    let sum: u64 = held.iter().map(|p| u64::from(p[0])).sum();
    assert_eq!(sum, held.len() as u64);
    drop(held);
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

/// One open and one 8 KiB read per audio file under `root`: what touching
/// every file costs before any parsing. Returns (time, files opened).
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
/// folder, three folders deep. Each differs in length and content, so no
/// two share an audio hash. Returns how many it wrote.
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
fn a_thousand_files_go_through_the_chain() {
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
    assert!(report.first_rows.is_some(), "the first rows arrived");
    assert_eq!(report.read_done, FILES as i64, "every file was read");
    assert_eq!(report.still_due, 0, "nothing is due after the read");
    assert_eq!(report.hash_done, FILES as i64, "every file was hashed");
    assert_eq!(
        report.recordings, FILES as i64,
        "every distinct file became its own recording"
    );
    assert_eq!(
        report.rescan_new, 0,
        "a rescan of an unchanged tree adds no rows"
    );
    assert!(
        !report.after_rescan.iter().any(|k| k == "read"),
        "a rescan of an unchanged tree queues no read: {:?}",
        report.after_rescan
    );
    assert!(report.peak_working_set >= report.baseline_working_set);
    assert!(report.db_bytes > 0);
}

/// Where stage 2's time goes, by format, on a warm cache: the two public
/// parts of one file read (the sniff, and lofty's tags and properties)
/// timed on their own over a sample of each format under `TLP_PERF_ROOT`.
/// The rest of a read (codec headers, the second open, the database) is
/// what `hundred_thousand_files` reports above these. Seconds, not
/// minutes; it needs only the tree.
#[test]
#[ignore = "needs TLP_PERF_ROOT; see the module docs"]
fn stage_2_parts_per_format() {
    const SAMPLE: usize = 300;
    let root = PathBuf::from(std::env::var_os(ROOT_VAR).expect("TLP_PERF_ROOT names the tree"));
    let mut by_ext: std::collections::BTreeMap<String, Vec<PathBuf>> = Default::default();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if is_indexed(&entry.file_name().to_string_lossy()) {
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                let files = by_ext.entry(ext).or_default();
                if files.len() < SAMPLE {
                    files.push(path);
                }
            }
        }
    }
    println!();
    println!("stage 2 parts on a warm cache, up to {SAMPLE} files per format:");
    for (ext, files) in &by_ext {
        // Warm the cache for these files first, so both parts see the same.
        for path in files {
            let _ = fs::read(path);
        }
        let sniff = timed(|| {
            for path in files {
                let _ = tracklist_pro_lib::sniff::sniff_path(path);
            }
        });
        let tags = timed(|| {
            for path in files {
                let _ = tracklist_pro_lib::tags::read(path);
            }
        });
        println!(
            "  .{ext:<5} {:>4} files   sniff {}   tags + properties {}",
            files.len(),
            per_file(sniff, files.len()),
            per_file(tags, files.len())
        );
    }
    println!();
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

    if let Some(gib) = evict_gib() {
        let took = timed(|| evict_file_cache(gib));
        println!(
            "touched {gib} GiB to push the file cache out, in {}",
            secs(took)
        );
    }
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
