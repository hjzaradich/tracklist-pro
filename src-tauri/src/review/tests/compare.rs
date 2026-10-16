//! The comparison of one item: its files, the suggestion first, its reason.

use super::*;
use crate::quality::verdict::Verdict;
use crate::suggest::ReasonParam;

fn key_of(c: &ReviewComparison) -> String {
    c.suggestion.as_ref().unwrap().reasons()[0].key().to_owned()
}

#[test]
fn a_version_doubt_lists_the_files_of_both_tracks_the_better_one_first_with_its_reason() {
    let lib = Lib::new();
    let t1 = lib.track("Velo");
    let mp3 = lib.file(t1, "Velo.mp3", "mp3", true);
    let t2 = lib.track("Velo Dirty");
    let flac = lib.file(t2, "Velo Dirty.flac", "flac", true);
    lib.library(t1, mp3);
    lib.open_pair(mp3, flac);

    let c = lib
        .compare(id(ReviewKind::VersionDoubt, t1.min(t2), Some(t1.max(t2))))
        .unwrap();
    assert_eq!(c.group, ReviewGroup::NeedsCloserLook);
    let s = c.suggestion.as_ref().unwrap();
    assert_eq!(*s.what(), flac);
    assert_eq!(s.reasons()[0].key(), "review:reason.betterFormat");
    assert_eq!(
        s.reasons()[0].params().get("best"),
        Some(&ReasonParam::Text("FLAC".into()))
    );
    assert_eq!(
        c.rows.iter().map(|r| r.file_id).collect::<Vec<_>>(),
        [Some(flac), Some(mp3)]
    );
    // Each row says which track it is in; the Library's linked file is marked.
    assert_eq!(c.rows[0].recording_id, Some(t2));
    assert!(c.rows[1].linked && !c.rows[0].linked);
    // Why the pair is in doubt: the grouping's own codes, no text.
    let keys: Vec<_> = c.doubt.iter().map(|r| r.key().to_owned()).collect();
    assert_eq!(
        keys,
        [
            "review:doubt.sameAudioNamesDisagree",
            "review:doubt.namesDiffer"
        ]
    );
}

#[test]
fn every_row_has_the_same_columns_filled_from_the_file() {
    let lib = Lib::new();
    let t = lib.track("Velo");
    let f = lib.file(t, "Sub/Velo.mp3", "mp3", true);
    let g = lib.file(t, "Velo.flac", "flac", true);
    lib.library(t, f);
    lib.verdict(f, "low_bitrate");
    lib.better(t, g, "format");
    let c = lib.compare(id(ReviewKind::BetterFile, t, None)).unwrap();
    let row = c.rows.iter().find(|r| r.file_id == Some(f)).unwrap();
    assert_eq!(row.name, "Velo.mp3");
    assert_eq!(row.path.as_deref(), Some(r"E:\Music\Sub\Velo.mp3"));
    assert_eq!(row.format.as_deref(), Some("mp3"));
    assert_eq!(row.verdict, Some(Verdict::LowBitrate));
    assert_eq!(row.duration_ms, Some(215_000));
    assert_eq!(row.bitrate_kbps, Some(320));
    assert!(row.on_disk && row.drive_connected && row.linked);
    assert_eq!((row.tags_filled, row.tags_total), (0, 6));
}

#[test]
fn a_clear_best_file_suggests_the_copy_because_the_linked_file_is_gone() {
    let lib = Lib::new();
    let t = lib.track("Velo");
    let gone = lib.file(t, "Velo.mp3", "mp3", false);
    let copy = lib.file(t, "Velo copy.mp3", "mp3", true);
    lib.library(t, gone);
    lib.better(t, copy, "onDisk");
    let c = lib.compare(id(ReviewKind::ClearBestFile, t, None)).unwrap();
    assert_eq!(*c.suggestion.as_ref().unwrap().what(), copy);
    assert_eq!(key_of(&c), "review:reason.notOnDisk");
    assert_eq!(c.rows[0].file_id, Some(copy));
    assert!(!c.rows[0].linked);
    assert!(c.rows[1].linked && !c.rows[1].on_disk);
}

#[test]
fn a_better_file_item_suggests_the_better_file_with_the_ranking_reason() {
    let lib = Lib::new();
    let t = lib.track("Velo");
    let mp3 = lib.file(t, "Velo.mp3", "mp3", true);
    let flac = lib.file(t, "Velo.flac", "flac", true);
    lib.library(t, mp3);
    lib.better(t, flac, "format");
    let c = lib.compare(id(ReviewKind::BetterFile, t, None)).unwrap();
    assert_eq!(c.group, ReviewGroup::QualityProblems);
    assert_eq!(*c.suggestion.as_ref().unwrap().what(), flac);
    assert_eq!(key_of(&c), "review:reason.betterFormat");
}

#[test]
fn a_quality_problem_has_no_suggestion_only_the_files_verdict() {
    let lib = Lib::new();
    let (t, f) = lib.simple("Velo");
    lib.library(t, f);
    lib.verdict(f, "truncated");
    let c = lib
        .compare(id(ReviewKind::QualityProblem, t, None))
        .unwrap();
    assert!(c.suggestion.is_none());
    assert_eq!(c.rows.len(), 1);
    assert_eq!(c.rows[0].verdict, Some(Verdict::Truncated));
}

#[test]
fn a_relink_item_suggests_the_matched_file_beside_rekordbox_s_entry() {
    let lib = Lib::new();
    let (t, f) = lib.simple("Velo");
    let row = lib.rekordbox(Some(f), "filename_only", true);
    let c = lib.compare(id(ReviewKind::Relink, row, None)).unwrap();
    assert_eq!(*c.suggestion.as_ref().unwrap().what(), f);
    assert_eq!(key_of(&c), "review:relink.filenameOnly");
    assert_eq!(c.rows.len(), 2);
    assert_eq!(c.rows[0].file_id, Some(f));
    assert_eq!(c.rows[0].recording_id, Some(t));
    assert_eq!(c.rows[1].source, RowSource::RekordboxEntry);
    assert_eq!(c.rows[1].path.as_deref(), Some(r"E:\Old\old.mp3"));
    assert_eq!(c.rows[1].duration_ms, Some(215_000));
}

#[test]
fn a_held_set_lists_every_file_of_every_track() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (_, f2) = lib.simple("Velo b");
    let (_, f3) = lib.simple("Velo c");
    lib.library(t1, f1);
    lib.pair(f1, f2, "duplicate", "set_disagrees");
    lib.pair(f2, f3, "duplicate", "set_disagrees");
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    assert_eq!(c.rows.len(), 3);
    assert_eq!(c.tracks.len(), 3);
    assert!(c.suggestion.is_some());
    assert!(!c.doubt.is_empty());
}

#[test]
fn an_item_that_is_no_longer_queued_has_no_comparison() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (t2, f2) = lib.simple("Velo Dirty");
    lib.library(t1, f1);
    lib.open_pair(f1, f2);
    let item = id(ReviewKind::VersionDoubt, t1.min(t2), Some(t1.max(t2)));
    assert!(lib.compare(item).is_some());
    lib.exec("UPDATE grouping_pair SET state = 'decided'");
    assert!(lib.compare(item).is_none());
    assert!(lib.compare(id(ReviewKind::Relink, 99, None)).is_none());
    assert!(lib
        .compare(id(ReviewKind::QualityProblem, t1, None))
        .is_none());
}

#[test]
fn a_comparison_lists_at_most_fifty_files() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    lib.library(t1, f1);
    let t2 = lib.track("Velo b");
    let f2 = lib.file(t2, "b0.mp3", "mp3", true);
    for n in 1..60 {
        lib.file(t2, &format!("b{n}.mp3"), "mp3", true);
    }
    lib.open_pair(f1, f2);
    let c = lib
        .compare(id(ReviewKind::VersionDoubt, t1.min(t2), Some(t1.max(t2))))
        .unwrap();
    assert_eq!(c.rows.len(), MAX_FILES);
    assert_eq!(c.more_files, 61 - MAX_FILES as u32);
}

#[test]
fn a_relink_whose_file_is_in_no_track_yet_still_shows_that_file_as_the_suggestion() {
    let lib = Lib::new();
    // Scanned, not yet grouped: a file with no track.
    lib.exec(
        "INSERT INTO file (music_folder_id, rel_path, rel_path_key, sniffed_format, codec, bitrate)
         VALUES (1, 'new.mp3', 'new.mp3', 'mp3', 'mp3', 320)",
    );
    let file = lib.num("SELECT max(id) FROM file");
    let row = lib.rekordbox(Some(file), "filename_only", true);
    assert_eq!(lib.ids(ReviewGroup::NeedsCloserLook).len(), 1);

    let c = lib.compare(id(ReviewKind::Relink, row, None)).unwrap();
    assert_eq!(*c.suggestion.as_ref().unwrap().what(), file);
    assert_eq!(c.rows.len(), 2);
    assert_eq!(c.rows[0].file_id, Some(file));
    assert_eq!(c.rows[0].recording_id, None);
    assert_eq!(c.rows[0].name, "new.mp3");
    assert_eq!(c.rows[1].source, RowSource::RekordboxEntry);
    assert_eq!(c.tracks.len(), 1);
}

#[test]
fn a_rekordbox_length_that_is_not_a_number_shows_no_length() {
    let lib = Lib::new();
    let (_, f) = lib.simple("Velo");
    lib.exec(&format!(
        "INSERT INTO rekordbox_track
             (attributes, location_key, read_at, file_id, relink_method, relink_probable)
         VALUES (json_object('TrackID', '1', 'Location', 'file://localhost/E:/Old/old.mp3',
                             'TotalTime', 'n/a'),
                 'E:/Old/old.mp3', '2026-09-30T10:00:00.000Z', {f}, 'filename_only', 1)"
    ));
    let row = lib.num("SELECT max(id) FROM rekordbox_track");
    let c = lib.compare(id(ReviewKind::Relink, row, None)).unwrap();
    assert_eq!(c.rows[1].duration_ms, None);
}

#[test]
fn a_clear_best_file_offers_the_switch_to_every_copy_on_disk_for_the_number_keys() {
    let lib = Lib::new();
    let t = lib.track("Velo");
    let gone = lib.file(t, "Velo.mp3", "mp3", false);
    let copy = lib.file(t, "Velo copy.mp3", "mp3", true);
    let flac = lib.file(t, "Velo.flac", "flac", true);
    let lost = lib.file(t, "Velo lost.mp3", "mp3", false);
    lib.library(t, gone);
    lib.better(t, copy, "onDisk");
    let c = lib.compare(id(ReviewKind::ClearBestFile, t, None)).unwrap();
    let mut offered: Vec<i64> = c.copies.iter().map(|o| o.file_id).collect();
    offered.sort_unstable();
    // Both copies on disk; not the linked file, not the other gone file.
    assert_eq!(offered, vec![copy.min(flac), copy.max(flac)]);
    assert!(!offered.contains(&gone) && !offered.contains(&lost));
    assert!(c
        .copies
        .iter()
        .all(|o| o.effect.is_some() && o.refusal.is_none()));
    // The suggested copy's offer is the item's own.
    let suggested = c.switch.unwrap();
    assert_eq!(
        c.copies.iter().find(|o| o.file_id == copy),
        Some(&suggested)
    );
}

#[test]
fn only_a_clear_best_file_item_has_copies_to_switch_to() {
    let lib = Lib::new();
    let t = lib.track("Velo");
    let mp3 = lib.file(t, "Velo.mp3", "mp3", true);
    let flac = lib.file(t, "Velo.flac", "flac", true);
    lib.library(t, mp3);
    lib.better(t, flac, "format");
    let c = lib.compare(id(ReviewKind::BetterFile, t, None)).unwrap();
    assert!(c.copies.is_empty() && c.switch.is_none());
}

#[test]
fn the_view_of_a_switch_names_the_track_and_its_old_and_new_file() {
    use super::super::accepted::{switched_view, SwitchedRef};
    let lib = Lib::new();
    let t = lib.track("Velo");
    let gone = lib.file(t, "Velo.mp3", "mp3", false);
    let copy = lib.file(t, "Velo copy.mp3", "mp3", true);
    lib.library(t, gone);
    let shown = lib
        .writer
        .call(move |c| {
            switched_view(
                c,
                &Mount,
                &[SwitchedRef {
                    recording_id: t,
                    from_file_id: gone,
                    to_file_id: copy,
                }],
            )
        })
        .unwrap();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].recording_id, t);
    assert_eq!(shown[0].track.title.as_deref(), Some("Velo"));
    assert_eq!(shown[0].from_name, "Velo.mp3");
    assert_eq!(shown[0].to_name, "Velo copy.mp3");
}
