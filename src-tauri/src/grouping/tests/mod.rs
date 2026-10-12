//! Grouping (1bC-1): files into tracks, versions linked, pairs for the
//! user kept apart. The exact-audio behavior of provisional grouping
//! (1aC-2) is still here, unchanged.

// The matching tests' made-up fingerprints, taken as they are: they're
// private to those tests, and this lane doesn't edit their module.
#[path = "../../matching/tests/synthetic.rs"]
#[allow(dead_code, clippy::duplicate_mod)]
mod synthetic;

mod attached;
mod behavior;
mod current;
mod deciding;
mod decisions;
mod folders;
mod job;
mod plan;
mod removals;
mod steps;
pub(crate) mod support;
