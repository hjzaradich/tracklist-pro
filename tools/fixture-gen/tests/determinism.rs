//! A seed fully decides the output: the same seed gives a byte-identical
//! tree and manifest, a different seed gives different files.

mod common;

use common::{core_len, generate_into, walk};
use fixture_gen::{plan, render};

#[test]
fn the_same_seed_writes_a_byte_identical_tree_and_manifest() {
    let files = core_len() + 40;
    let a = generate_into("seed-42-a", 42, files);
    let b = generate_into("seed-42-b", 42, files);

    let (ta, tb) = (walk(&a.root), walk(&b.root));
    assert_eq!(ta.len(), files + 1, "every file plus the manifest");
    assert_eq!(ta.len(), tb.len());
    for ((pa, ba), (pb, bb)) in ta.iter().zip(&tb) {
        assert_eq!(pa, pb, "names differ");
        assert!(ba == bb, "{pa:?}: bytes differ");
    }
}

#[test]
fn a_different_seed_writes_different_audio_and_names() {
    let a = plan::plan(1, core_len() + 5, 500);
    let b = plan::plan(2, core_len() + 5, 500);

    // Core cases keep their names (they're the ground truth) but not their audio.
    let (fa, fb) = (&a.files[0], &b.files[0]);
    assert_eq!(fa.path, fb.path);
    assert_ne!(render(&fa.body), render(&fb.body));
    // Bulk files get different names too.
    let bulk = |p: &plan::Plan| -> Vec<String> {
        p.files
            .iter()
            .filter(|f| f.entry.cases.iter().any(|c| c == "bulk"))
            .map(|f| f.entry.path.clone())
            .collect()
    };
    assert_ne!(bulk(&a), bulk(&b));
}
