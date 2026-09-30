//! Reading and changing the user's opt-in for each service
//! (`service_optin`, migration 0005).
//!
//! The app is offline by default: a service with no row, or with
//! `enabled = 0`, is not opted in.

use rusqlite::{Connection, OptionalExtension};

use super::Service;
use crate::db::{DbError, ReadPool, Writer};

/// Whether the user has opted in to `service`, read fresh from the
/// database.
pub fn is_opted_in(reads: &ReadPool, service: Service) -> Result<bool, DbError> {
    reads.read(|conn| read_opt_in(conn, service))
}

fn read_opt_in(conn: &Connection, service: Service) -> rusqlite::Result<bool> {
    let enabled: Option<i64> = conn
        .query_row(
            "SELECT enabled FROM service_optin WHERE service = ?1",
            [service.id()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(enabled == Some(1))
}

/// Opts in to `service`, or out of it. Takes effect for the next request,
/// including one already waiting its turn.
pub fn set_opt_in(writer: &Writer, service: Service, enabled: bool) -> Result<(), DbError> {
    writer.call(move |conn| {
        conn.execute(
            "INSERT INTO service_optin (service, enabled, enabled_at)
             VALUES (?1, ?2, CASE WHEN ?2 THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END)
             ON CONFLICT (service) DO UPDATE SET
                 enabled = excluded.enabled,
                 enabled_at = CASE
                     WHEN excluded.enabled AND service_optin.enabled
                         THEN service_optin.enabled_at
                     ELSE excluded.enabled_at
                 END",
            rusqlite::params![service.id(), enabled],
        )
        .map(|_| ())
    })
}
