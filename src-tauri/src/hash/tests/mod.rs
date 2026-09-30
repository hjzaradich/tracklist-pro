#![cfg(test)]
//! Tests for the hashes (1aB-5, 1aB-6).

mod command;
mod damaged;
mod definition;
mod fixtures;
#[cfg(windows)]
mod job;
mod one_pass;
mod partial;
mod real_tagger;
mod review;
