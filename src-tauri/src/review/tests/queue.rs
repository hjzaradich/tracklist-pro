//! Which items are queued, in which group, and how they are counted.

use super::*;

const NEEDS: ReviewGroup = ReviewGroup::NeedsCloserLook;
const CLEAR: ReviewGroup = ReviewGroup::ClearBestFile;
const QUALITY: ReviewGroup = ReviewGroup::QualityProblems;

#[test]
fn an_empty_library_has_an_empty_queue() {
    let lib = Lib::new();
    assert_eq!(
        lib.counts(),
        ReviewCounts {
            needs_closer_look: 0,
            clear_best_file: 0,
            quality_problems: 0,
            total: 0
        }
    );
    for group in ReviewGroup::ALL {
        assert!(lib.page(group, 0, 10).items.is_empty());
    }
}

#[test]
fn a_version_doubt_with_a_library_track_lands_in_needs_a_closer_look() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (t2, f2) = lib.simple("Velo Dirty");
    lib.library(t1, f1);
    lib.open_pair(f1, f2);
    assert_eq!(
        lib.ids(NEEDS),
        vec![(ReviewKind::VersionDoubt, t1.min(t2), Some(t1.max(t2)))]
    );
    assert!(lib.ids(CLEAR).is_empty() && lib.ids(QUALITY).is_empty());
}

#[test]
fn a_pair_outside_the_library_is_not_queued() {
    let lib = Lib::new();
    let (_, f1) = lib.simple("Velo");
    let (_, f2) = lib.simple("Velo Dirty");
    lib.open_pair(f1, f2);
    // Held sets and the same-recording pairs follow the same scope.
    let (_, f3) = lib.simple("Tune");
    lib.pair(f1, f3, "duplicate", "set_disagrees");
    assert_eq!(lib.counts().total, 0);
}

#[test]
fn many_file_pairs_of_the_same_two_tracks_are_one_item() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (t2, f2) = lib.simple("Velo Dirty");
    let f1b = lib.file(t1, "Velo copy.mp3", "mp3", true);
    let f2b = lib.file(t2, "Velo Dirty copy.mp3", "mp3", true);
    lib.library(t1, f1);
    for (x, y) in [(f1, f2), (f1, f2b), (f1b, f2), (f1b, f2b)] {
        lib.open_pair(x, y);
    }
    assert_eq!(lib.ids(NEEDS).len(), 1);
    assert_eq!(lib.counts().needs_closer_look, 1);
}

#[test]
fn a_held_duplicate_set_is_one_item_for_all_its_tracks() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (_, f2) = lib.simple("Velo b");
    let (_, f3) = lib.simple("Velo c");
    lib.library(t1, f1);
    lib.pair(f1, f2, "duplicate", "set_disagrees");
    lib.pair(f2, f3, "duplicate", "set_disagrees");
    assert_eq!(lib.ids(NEEDS), vec![(ReviewKind::HeldSet, t1, None)]);
    let page = lib.page(NEEDS, 0, 10);
    assert_eq!(page.items[0].tracks.len(), 3);
    assert_eq!(page.items[0].more_tracks, 0);
}

#[test]
fn a_held_set_with_no_library_track_is_not_queued() {
    let lib = Lib::new();
    let (_, f1) = lib.simple("Velo");
    let (_, f2) = lib.simple("Velo b");
    lib.pair(f1, f2, "duplicate", "set_disagrees");
    assert!(lib.ids(NEEDS).is_empty());
}

#[test]
fn two_library_tracks_that_are_the_same_recording_are_one_item_and_a_third_copy_joins_it() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (t2, f2) = lib.simple("Velo again");
    let (_, f3) = lib.simple("Velo third");
    lib.library(t1, f1);
    lib.library(t2, f2);
    lib.pair(f1, f2, "duplicate", "both_in_library");
    lib.pair(f1, f3, "duplicate", "set_disagrees");
    lib.pair(f2, f3, "duplicate", "set_disagrees");
    assert_eq!(lib.ids(NEEDS), vec![(ReviewKind::SameRecording, t1, None)]);
    assert_eq!(lib.page(NEEDS, 0, 10).items[0].tracks.len(), 3);
}

#[test]
fn only_a_probable_relink_with_a_file_is_queued_library_or_not() {
    let lib = Lib::new();
    let (_, f1) = lib.simple("Velo");
    let (_, f2) = lib.simple("Tune");
    let (_, f3) = lib.simple("Other");
    let probable = lib.rekordbox(Some(f1), "filename_only", true);
    let probable_too = lib.rekordbox(Some(f2), "unique_duration", true);
    lib.rekordbox(Some(f3), "path", false);
    lib.rekordbox(None, "path", false);
    assert_eq!(
        lib.ids(NEEDS),
        vec![
            (ReviewKind::Relink, probable, None),
            (ReviewKind::Relink, probable_too, None)
        ]
    );
    // The Missing list (an entry with no file) and a trusted match are not
    // "how many need a decision".
    assert_eq!(lib.counts().total, 2);
    assert_eq!(lib.counts().needs_closer_look, 2);
    // A relink item is named by rekordbox's own entry.
    let page = lib.page(NEEDS, 0, 10);
    assert_eq!(page.items[0].tracks[0].recording_id, None);
    assert_eq!(page.items[0].tracks[0].title.as_deref(), Some("Old name"));
}

#[test]
fn a_library_track_whose_linked_file_is_gone_beside_a_copy_is_a_clear_best_file() {
    let lib = Lib::new();
    let t = lib.track("Velo");
    let gone = lib.file(t, "Velo.mp3", "mp3", false);
    let copy = lib.file(t, "Velo copy.mp3", "mp3", true);
    lib.library(t, gone);
    lib.better(t, copy, "onDisk");
    assert_eq!(lib.ids(CLEAR), vec![(ReviewKind::ClearBestFile, t, None)]);
    assert!(lib.ids(QUALITY).is_empty() && lib.ids(NEEDS).is_empty());
}

#[test]
fn a_flagged_linked_file_with_no_better_copy_is_a_quality_problem() {
    let lib = Lib::new();
    let (t, f) = lib.simple("Velo");
    lib.library(t, f);
    lib.verdict(f, "low_bitrate");
    assert_eq!(
        lib.ids(QUALITY),
        vec![(ReviewKind::QualityProblem, t, None)]
    );
}

#[test]
fn a_verdict_that_is_ok_stale_or_of_a_file_not_linked_or_gone_is_not_a_quality_problem() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Fine");
    lib.library(t1, f1);
    lib.verdict(f1, "ok");
    // Stale: an input of the verdict changed after it was stored.
    let (t2, f2) = lib.simple("Stale");
    lib.library(t2, f2);
    lib.verdict(f2, "broken");
    lib.exec(&format!("UPDATE file SET bitrate = 128 WHERE id = {f2}"));
    // Not in the Library: a flagged file of a track that isn't.
    let (_, f3) = lib.simple("Not in Library");
    lib.verdict(f3, "truncated");
    // Gone: the linked file isn't there.
    let t4 = lib.track("Gone");
    let f4 = lib.file(t4, "Gone.mp3", "mp3", false);
    lib.library(t4, f4);
    lib.verdict(f4, "broken");
    assert_eq!(lib.counts().quality_problems, 0);
}

#[test]
fn a_better_copy_beside_a_linked_file_that_is_there_is_a_better_file_item_in_quality_problems() {
    let lib = Lib::new();
    let t = lib.track("Velo");
    let linked = lib.file(t, "Velo.mp3", "mp3", true);
    let flac = lib.file(t, "Velo.flac", "flac", true);
    lib.library(t, linked);
    lib.better(t, flac, "format");
    // Flagged as well: it is the better-file item, not a second item.
    lib.verdict(linked, "suspect_transcode");
    assert_eq!(lib.ids(QUALITY), vec![(ReviewKind::BetterFile, t, None)]);
    assert_eq!(lib.counts().quality_problems, 1);
}

#[test]
fn counts_equal_the_totals_of_the_lists() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("One");
    let (_, f2) = lib.simple("Two");
    lib.library(t1, f1);
    lib.open_pair(f1, f2);
    let (t3, f3) = lib.simple("Three");
    lib.library(t3, f3);
    lib.verdict(f3, "broken");
    let t4 = lib.track("Four");
    let gone = lib.file(t4, "Four.mp3", "mp3", false);
    let copy = lib.file(t4, "Four copy.mp3", "mp3", true);
    lib.library(t4, gone);
    lib.better(t4, copy, "onDisk");
    lib.rekordbox(Some(f2), "filename_only", true);

    let counts = lib.counts();
    for (group, count) in [
        (NEEDS, counts.needs_closer_look),
        (CLEAR, counts.clear_best_file),
        (QUALITY, counts.quality_problems),
    ] {
        assert_eq!(lib.page(group, 0, 1000).total, count, "{group:?}");
        assert_eq!(lib.ids(group).len() as u32, count, "{group:?}");
    }
    assert_eq!(counts.total, 4);
}

#[test]
fn pages_cut_the_group_in_its_order_whatever_the_page_size() {
    let lib = Lib::new();
    let (t0, f0) = lib.simple("Anchor");
    lib.library(t0, f0);
    // Three doubts, a held set, a relink: five items in one group.
    for n in 0..3 {
        let (_, f) = lib.simple(&format!("Doubt {n}"));
        lib.open_pair(f0, f);
    }
    let (_, h1) = lib.simple("Held 1");
    let (_, h2) = lib.simple("Held 2");
    lib.pair(f0, h1, "duplicate", "set_disagrees");
    lib.pair(h1, h2, "duplicate", "set_disagrees");
    lib.rekordbox(Some(h2), "fingerprint", true);

    let all = lib.ids(NEEDS);
    assert_eq!(all.len(), 5);
    let kinds: Vec<ReviewKind> = all.iter().map(|i| i.0).collect();
    assert_eq!(
        kinds,
        [
            ReviewKind::VersionDoubt,
            ReviewKind::VersionDoubt,
            ReviewKind::VersionDoubt,
            ReviewKind::HeldSet,
            ReviewKind::Relink
        ]
    );
    for size in 1..=5u32 {
        let mut paged = Vec::new();
        let mut offset = 0;
        while offset < 5 {
            let page = lib.page(NEEDS, offset, size);
            assert_eq!(page.total, 5);
            paged.extend(page.items.iter().map(|i| (i.id.kind, i.id.a, i.id.b)));
            offset += size;
        }
        assert_eq!(paged, all, "pages of {size}");
    }
}

#[test]
fn an_item_names_its_tracks_by_title() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (_, f2) = lib.simple("Velo Dirty");
    lib.library(t1, f1);
    lib.open_pair(f1, f2);
    let item = lib.page(NEEDS, 0, 10).items.remove(0);
    let titles: Vec<_> = item
        .tracks
        .iter()
        .map(|t| t.title.clone().unwrap())
        .collect();
    assert_eq!(titles, ["Velo", "Velo Dirty"]);
}

#[test]
fn the_group_of_each_kind_is_the_one_the_roadmap_names() {
    let (needs, clear, quality) = (NEEDS, CLEAR, QUALITY);
    let groups: Vec<_> = ReviewKind::ALL.iter().map(|k| k.group()).collect();
    assert_eq!(
        groups,
        [needs, needs, needs, needs, clear, quality, quality]
    );
    // Every kind is in exactly one group, and the groups' kinds are all of them.
    let listed: usize = ReviewGroup::ALL.iter().map(|g| g.kinds().count()).sum();
    assert_eq!(listed, ReviewKind::ALL.len());
}

/// The ids of `group`'s items of one kind, in the order listed.
fn ids_of(lib: &Lib, group: ReviewGroup, kind: ReviewKind) -> Vec<(i64, Option<i64>)> {
    lib.ids(group)
        .into_iter()
        .filter(|i| i.0 == kind)
        .map(|i| (i.1, i.2))
        .collect()
}

#[test]
fn inside_a_kind_the_oldest_item_comes_first() {
    let lib = Lib::new();
    // Three version doubts, made in this order; a Library track is in each.
    let mut doubts = Vec::new();
    for n in 0..3 {
        let (t1, f1) = lib.simple(&format!("Doubt {n}"));
        let (t2, f2) = lib.simple(&format!("Doubt {n} b"));
        lib.library(t1, f1);
        lib.open_pair(f1, f2);
        doubts.push((t1, Some(t2)));
    }
    // Two held sets, two same-recording sets.
    let (mut held, mut same) = (Vec::new(), Vec::new());
    for n in 0..2 {
        let (t1, f1) = lib.simple(&format!("Held {n}"));
        let (_, f2) = lib.simple(&format!("Held {n} b"));
        lib.library(t1, f1);
        lib.pair(f1, f2, "duplicate", "set_disagrees");
        held.push((t1, None));
        let (t3, f3) = lib.simple(&format!("Same {n}"));
        let (t4, f4) = lib.simple(&format!("Same {n} b"));
        lib.library(t3, f3);
        lib.library(t4, f4);
        lib.pair(f3, f4, "duplicate", "both_in_library");
        same.push((t3, None));
    }
    // Two relinks, two clear best files, two quality problems, two better files.
    let mut relinks = Vec::new();
    let (mut clear, mut quality, mut better) = (Vec::new(), Vec::new(), Vec::new());
    for n in 0..2 {
        let (_, f) = lib.simple(&format!("Relink {n}"));
        relinks.push((lib.rekordbox(Some(f), "filename_only", true), None));
        let t = lib.track(&format!("Clear {n}"));
        let gone = lib.file(t, &format!("Clear {n}.mp3"), "mp3", false);
        let copy = lib.file(t, &format!("Clear {n} copy.mp3"), "mp3", true);
        lib.library(t, gone);
        lib.better(t, copy, "onDisk");
        clear.push((t, None));
        let (t, f) = lib.simple(&format!("Quality {n}"));
        lib.library(t, f);
        lib.verdict(f, "broken");
        quality.push((t, None));
        let t = lib.track(&format!("Better {n}"));
        let mp3 = lib.file(t, &format!("Better {n}.mp3"), "mp3", true);
        let flac = lib.file(t, &format!("Better {n}.flac"), "flac", true);
        lib.library(t, mp3);
        lib.better(t, flac, "format");
        better.push((t, None));
    }
    // Made first, listed first, in every kind.
    assert_eq!(ids_of(&lib, NEEDS, ReviewKind::VersionDoubt), doubts);
    assert_eq!(ids_of(&lib, NEEDS, ReviewKind::HeldSet), held);
    assert_eq!(ids_of(&lib, NEEDS, ReviewKind::SameRecording), same);
    assert_eq!(ids_of(&lib, NEEDS, ReviewKind::Relink), relinks);
    assert_eq!(ids_of(&lib, CLEAR, ReviewKind::ClearBestFile), clear);
    assert_eq!(ids_of(&lib, QUALITY, ReviewKind::QualityProblem), quality);
    assert_eq!(ids_of(&lib, QUALITY, ReviewKind::BetterFile), better);
}

#[test]
fn counting_and_listing_leave_no_transaction_open() {
    let lib = Lib::new();
    lib.writer
        .call(|c| {
            counts(c)?;
            items(c, &Mount, ReviewGroup::NeedsCloserLook, 0, 10)?;
            comparison(c, &Mount, id(ReviewKind::Relink, 1, None))?;
            assert!(c.is_autocommit());
            Ok(())
        })
        .unwrap();
}
