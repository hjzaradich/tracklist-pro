#![cfg(test)]
//! Tests for music folders and the walk.

#[cfg(windows)]
mod chain;
#[cfg(windows)]
mod file_id;
mod folders;
#[cfg(windows)]
mod ipc;
#[cfg(windows)]
mod online_only;
mod schema;
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
