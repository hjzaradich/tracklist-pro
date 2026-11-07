//! The report: one UTF-8 text built from an allow-list.
//!
//! Every line is written by name from a number, a fixed list or a version
//! string the app itself made. The report never dumps a table and never
//! copies text from a job, an error, a setting or a file, so it has no
//! track names, paths, `Location` values, keys or tokens in it, and no
//! setting's value except the on/off of the optional features. The
//! settings it reads are read through typed values (a reason code, a
//! count), never as text.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use rusqlite::Connection;

use super::{EventKind, EventLog, LogEvent, KEEP_REPORTS, REPORT_DIR};
use crate::db::{migrations, DbError, ReadPool};
use crate::jobs::JobKind;
use crate::net::Service;
use crate::rekordbox::source::{stored_source, ReadSource};
use crate::settings::{read_feature_enabled, Feature};
use crate::write_guard::{GuardError, WriteGuard};

/// How many of the newest events the report lists.
const REPORT_EVENTS: usize = 200;

/// The tables whose rows are counted, by the name the report gives them.
/// Fixed: no name comes from the database.
const COUNTED: [(&str, &str); 8] = [
    ("library_tracks", "library_track"),
    ("files", "file"),
    ("crates", "crate"),
    ("playlists", "playlist"),
    ("sessions", "session"),
    ("play_events", "play_event"),
    ("rekordbox_snapshot_tracks", "rekordbox_track"),
    ("wishlists", "wishlist"),
];

/// The report last previewed and not yet saved. Saving writes exactly this
/// text, so what the user read is what the file holds. Tauri state.
#[derive(Default)]
pub struct ReportDraft(Mutex<Option<String>>);

impl ReportDraft {
    pub fn set(&self, text: String) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(text);
    }

    pub fn take(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

/// Builds the report as of `now_ms`.
pub fn build(reads: &ReadPool, log: &EventLog, now_ms: i64) -> Result<String, DbError> {
    let events = log.events();
    let (stored, on) = reads.read(|c| Ok((Stored::read(c)?, Switches::read(c)?)))?;
    let mut out = String::new();
    let _ = write_report(&mut out, &stored, &on, &events, log.lost_writes(), now_ms);
    Ok(out)
}

/// What the report reads from the database.
struct Stored {
    schema: Option<u32>,
    migrations: usize,
    counts: Vec<(&'static str, Option<i64>)>,
    /// Jobs by kind and status, kinds this build knows only.
    jobs: BTreeMap<(&'static str, &'static str), i64>,
    xml_read: Option<String>,
    xml_failure: Option<super::EventReason>,
}

impl Stored {
    fn read(c: &Connection) -> rusqlite::Result<Stored> {
        let applied = migrations::applied(c)?;
        let counts = COUNTED
            .iter()
            .map(|(name, table)| (*name, count(c, table)))
            .collect();
        let mut jobs = BTreeMap::new();
        let mut stmt = c.prepare("SELECT kind, status, COUNT(*) FROM job GROUP BY kind, status")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (kind, status, n) = row?;
            let kind = JobKind::parse(&kind).map_or("other", JobKind::as_str);
            let status = ["queued", "running", "done", "failed", "cancelled"]
                .into_iter()
                .find(|s| *s == status)
                .unwrap_or("other");
            *jobs.entry((kind, status)).or_insert(0) += n;
        }
        let xml = stored_source(c)?;
        let xml_read = xml
            .last_read
            .filter(|r| r.source == ReadSource::Xml)
            .map(|r| {
                format!(
                    "{} tracks, {} streaming, complete: {}",
                    r.summary.tracks, r.summary.streaming, r.summary.complete
                )
            });
        Ok(Stored {
            schema: applied.last().map(|a| a.version),
            migrations: applied.len(),
            counts,
            jobs,
            xml_read,
            xml_failure: xml.last_failure.map(|f| f.reason.into()),
        })
    }
}

fn count(c: &Connection, table: &str) -> Option<i64> {
    c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .ok()
}

/// The optional features and services, on or off. Never their settings.
struct Switches {
    features: Vec<(&'static str, bool)>,
    services: Vec<(&'static str, bool)>,
}

impl Switches {
    fn read(c: &Connection) -> rusqlite::Result<Switches> {
        let features = [
            ("wishlist", Feature::Wishlist),
            ("rekordbox_database", Feature::RekordboxDatabase),
        ]
        .into_iter()
        .map(|(name, f)| Ok((name, read_feature_enabled(c, f)?)))
        .collect::<rusqlite::Result<_>>()?;
        let services = Service::ALL
            .into_iter()
            .map(|s| {
                let on = c
                    .query_row(
                        "SELECT enabled FROM service_optin WHERE service = ?1",
                        [s.id()],
                        |r| r.get::<_, i64>(0),
                    )
                    .map(|n| n == 1)
                    .or_else(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => Ok(false),
                        e => Err(e),
                    })?;
                Ok((s.id(), on))
            })
            .collect::<rusqlite::Result<_>>()?;
        Ok(Switches { features, services })
    }
}

fn write_report(
    out: &mut String,
    stored: &Stored,
    on: &Switches,
    events: &[LogEvent],
    lost_writes: u32,
    now_ms: i64,
) -> std::fmt::Result {
    writeln!(out, "Tracklist Pro diagnostics report")?;
    writeln!(out, "created: {}", time(now_ms))?;
    writeln!(
        out,
        "Counts, versions and reason codes only. No track names, paths, keys or free text."
    )?;
    writeln!(out)?;
    writeln!(out, "[app]")?;
    writeln!(out, "version: {}", env!("CARGO_PKG_VERSION"))?;
    writeln!(out, "os: {}", os())?;
    match stored.schema {
        Some(v) => writeln!(
            out,
            "schema: version {v}, {} migrations applied",
            stored.migrations
        )?,
        None => writeln!(out, "schema: none")?,
    }
    writeln!(out)?;
    writeln!(out, "[counts]")?;
    for (name, n) in &stored.counts {
        match n {
            Some(n) => writeln!(out, "{name}: {n}")?,
            None => writeln!(out, "{name}: unavailable")?,
        }
    }
    writeln!(out)?;
    writeln!(out, "[jobs]")?;
    for status in ["queued", "running", "failed"] {
        let list = stored
            .jobs
            .iter()
            .filter(|((_, s), _)| *s == status)
            .map(|((k, _), n)| format!("{k}={n}"))
            .collect::<Vec<_>>();
        writeln!(
            out,
            "{status}: {}",
            if list.is_empty() {
                "none".to_owned()
            } else {
                list.join(" ")
            }
        )?;
    }
    writeln!(out, "recent failures by kind and reason:")?;
    let mut failures: BTreeMap<(String, String), u32> = BTreeMap::new();
    for e in events.iter().filter(|e| {
        matches!(
            e.kind,
            EventKind::JobFailed | EventKind::RekordboxReadFailed | EventKind::DatabaseReadFailed
        )
    }) {
        let what = format!(
            "{} job={}",
            word(&e.kind),
            e.job.map_or("-".to_owned(), |j| j.to_string())
        );
        let reason = e.reason.map_or("-".to_owned(), |r| word(&r));
        *failures.entry((what, reason)).or_insert(0) += 1;
    }
    if failures.is_empty() {
        writeln!(out, "  none")?;
    }
    for ((what, reason), n) in &failures {
        writeln!(out, "  {what} {reason}: {n}")?;
    }
    writeln!(out)?;
    writeln!(out, "[rekordbox]")?;
    match &stored.xml_read {
        Some(read) => writeln!(out, "xml last read: ok, {read}")?,
        None => writeln!(out, "xml last read: none")?,
    }
    match stored.xml_failure {
        Some(r) => writeln!(out, "xml last failure: {}", word(&r))?,
        None => writeln!(out, "xml last failure: none")?,
    }
    let last_database = events.iter().rev().find(|e| {
        matches!(
            e.kind,
            EventKind::DatabaseReadOk | EventKind::DatabaseReadFailed
        )
    });
    match last_database {
        None => writeln!(out, "database last read: none this run or earlier")?,
        Some(e) if e.kind == EventKind::DatabaseReadOk => writeln!(out, "database last read: ok")?,
        Some(e) => writeln!(
            out,
            "database last read: failed, {}",
            e.reason.map_or("-".to_owned(), |r| word(&r))
        )?,
    }
    writeln!(out)?;
    writeln!(out, "[optional features]")?;
    for (name, enabled) in on.features.iter().chain(&on.services) {
        writeln!(out, "{name}: {}", if *enabled { "on" } else { "off" })?;
    }
    writeln!(out)?;
    writeln!(out, "[events]")?;
    writeln!(out, "events kept: {}", events.len())?;
    writeln!(out, "log writes lost this run: {lost_writes}")?;
    writeln!(out, "newest first, at most {REPORT_EVENTS}:")?;
    for e in events.iter().rev().take(REPORT_EVENTS) {
        writeln!(out, "{}", line(e))?;
    }
    Ok(())
}

/// One event as a line: time, kind, then the fields it has.
fn line(e: &LogEvent) -> String {
    let mut s = format!("{} {}", time(e.at_ms), word(&e.kind));
    if let Some(job) = e.job {
        let _ = write!(s, " job={job}");
    }
    if let Some(reason) = e.reason {
        let _ = write!(s, " reason={}", word(&reason));
    }
    if let Some(n) = e.count {
        let _ = write!(s, " count={n}");
    }
    if let Some(ms) = e.ms {
        let _ = write!(s, " ms={ms}");
    }
    s
}

/// The name of a fixed-list value, as the log's file spells it.
fn word<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// `ms` since the Unix epoch as `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC).
pub(super) fn time(ms: i64) -> String {
    let (days, in_day) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (h, m, s, milli) = (
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1_000 % 60,
        in_day % 1_000,
    );
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{milli:03}Z")
}

/// The operating system's name and version.
fn os() -> String {
    #[cfg(windows)]
    {
        let v = windows_version::OsVersion::current();
        // Windows 11 still reports major version 10.
        let name = if v.major == 10 && v.build >= 22_000 {
            "Windows 11"
        } else {
            "Windows"
        };
        format!("{name} ({}.{}.{})", v.major, v.minor, v.build)
    }
    #[cfg(not(windows))]
    {
        std::env::consts::OS.to_owned()
    }
}

/// The file name a report made at `now_ms` is saved under: sortable, and
/// to the millisecond so two never collide.
fn file_name(now_ms: i64) -> String {
    time(now_ms)
        .replace(['-', ':', '.'], "")
        .replace('T', "-")
        .replace('Z', ".txt")
}

/// Whether `name` is a file this module saved.
fn is_report(name: &str) -> bool {
    name.len() == 22
        && name.ends_with(".txt")
        && name[..name.len() - 4].chars().enumerate().all(|(i, c)| {
            if i == 8 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        })
}

/// Saves `text` as `diagnostics<time>.txt` in the app data folder,
/// through the write guard, and deletes the oldest reports beyond
/// [`KEEP_REPORTS`]. Only files this module named are ever deleted.
pub fn save(guard: &WriteGuard, text: &str, now_ms: i64) -> Result<PathBuf, GuardError> {
    guard.save_keeping_newest(
        &guard.app_data_dir().join(REPORT_DIR),
        &file_name(now_ms),
        text.as_bytes(),
        KEEP_REPORTS,
        is_report,
    )
}
