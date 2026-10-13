//! Behavior tests for the preview protocol (1cA-7, 1cA-8). The audio is
//! made in code (small files; the long one is never in memory or on disk),
//! never real music.

mod aiff;
mod decoding;
#[cfg(windows)]
mod files;
#[cfg(windows)]
mod refusals;
#[cfg(windows)]
mod support;
mod threads;
