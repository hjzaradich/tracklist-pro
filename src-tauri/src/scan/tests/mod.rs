#![cfg(test)]
//! Tests for music folders and the walk.

#[cfg(windows)]
mod chain;
#[cfg(windows)]
mod cheap_checks;
#[cfg(windows)]
mod file_id;
mod folders;
#[cfg(windows)]
mod inbox;
#[cfg(windows)]
mod ipc;
mod onedrive;
#[cfg(windows)]
mod online_only;
#[cfg(windows)]
mod relocate;
mod schema;
#[cfg(windows)]
mod startup;
mod support;
#[cfg(windows)]
mod unchanged;
#[cfg(windows)]
mod unreadable;
#[cfg(windows)]
mod vanished;
mod volumes;
#[cfg(windows)]
mod walk;
#[cfg(windows)]
mod watch;
