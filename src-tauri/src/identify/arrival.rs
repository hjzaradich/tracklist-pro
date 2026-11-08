//! Automatic look-ups for files that arrive in the Inbox (ROADMAP 2.2, owner
//! decision of 2026-11-08; Run 6A, S2b).
//!
//! Inbox intake calls [`queue`] once, with the files it just made Inbox items
//! for (the end of a hash job, which is one arrival walk's batch). When
//! AcoustID or MusicBrainz is switched on, those files are queued as one
//! look-up job, at the job's low priority ([`super::job::PRIORITY`]). With
//! both off nothing is queued, whatever Discogs and Cover Art say: the
//! automatic look-up is for the two services that identify a file. The
//! services' own switches are checked again when the job runs, so a service
//! switched off meanwhile is not asked.
//!
//! A failure to queue is logged and never touches intake or the hash job.

use rusqlite::Connection;

use super::job::lookup_job;
use crate::jobs::{JobContext, NewJob};
use crate::net::{read_opt_in, Service};

/// The look-up job for `files` that just arrived, or none when there are no
/// files or neither AcoustID nor MusicBrainz is on.
pub fn arrivals_job(conn: &Connection, files: &[i64]) -> rusqlite::Result<Option<NewJob>> {
    if files.is_empty() {
        return Ok(None);
    }
    if !read_opt_in(conn, Service::AcoustId)? && !read_opt_in(conn, Service::MusicBrainz)? {
        return Ok(None);
    }
    Ok(Some(lookup_job(files)))
}

/// Queues the automatic look-up for `files`, the Inbox files just made items
/// (module docs). Best effort.
pub fn queue(job: &JobContext, files: Vec<i64>) {
    let queued = job
        .writer()
        .call(move |conn| arrivals_job(conn, &files))
        .map_err(|e| e.to_string())
        .and_then(|new| match new {
            Some(new) => job.enqueue(new).map(|_| ()).map_err(|e| e.to_string()),
            None => Ok(()),
        });
    if let Err(e) = queued {
        eprintln!("the look-up of arriving Inbox files wasn't queued: {e}");
    }
}
