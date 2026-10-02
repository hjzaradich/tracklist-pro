//! Phase 1a, end to end (1aG-1, 1aG-2; ROADMAP §6, the 1a row).
//!
//! Every test here runs the app the way a user does: the real startup, the
//! real commands over IPC, the real job queue with the app's own handlers,
//! in the order a user reaches them. Nothing calls an inner function to
//! skip a step, and no database row is set up by hand.
//!
//! These drive the backend, not the screens: the React frontend isn't
//! loaded, and choices that are only a screen ("Start fresh", "Start from
//! rekordbox") appear here as the commands those screens call.
//!
//! What the two runs prove:
//!
//! - **Start from rekordbox** ([`start_from_rekordbox`]): a generated music
//!   folder and a generated rekordbox export are scanned, read, matched and
//!   added to the Library; a send is prepared and written; and the file is
//!   read back with the app's own reader. Every known track goes out as
//!   rekordbox's own entry, with its TrackID and without the analysis
//!   fields, so BPM, key, play counts and cues stay rekordbox's.
//! - **Start fresh** ([`start_fresh`]): the same folder, an empty Library,
//!   tracks added from All music, a crate, a send. New tracks arrive with
//!   their tag values and a TrackID above rekordbox's highest.
//!
//! Both also prove the 1a row's last sentence, on every test: when a
//! [`harness::World`] ends, nothing under the music folder or beside
//! rekordbox's export has been added, removed or changed by the app, and
//! everything the app wrote is under its data folder.
//!
//! rekordbox itself isn't here. Where a test needs rekordbox's side of an
//! import, [`rekordbox_side::Rekordbox`] stands in, doing only what
//! ROADMAP §5.2 records rekordbox 7 as doing. What that proves is that the
//! file is right by those rules; the real import is the dogfooding lane's.
//!
//! Windows only, like the scan's own tests: the walk and the path model
//! ask Windows about volumes and file ids. Everything is generated; no
//! real track, artist or library is involved.
#![cfg(windows)]

mod fixtures;
mod harness;
mod interrupted;
mod rekordbox_side;
mod start_fresh;
mod start_from_rekordbox;
