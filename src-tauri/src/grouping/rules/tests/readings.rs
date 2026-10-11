//! A pair's names are read twice, by title tag and by file name, and one
//! reading decides (module docs, "Two readings of the names").

use super::*;

/// A file with a title tag and a file name.
fn file(title: &str, file_name: &str) -> FileFacts {
    tagged_and_named(title, file_name)
}

// ---- Full audio: the most cautious reading wins ------------------------

#[test]
fn full_audio_with_tags_that_say_clean_and_dirty_is_a_cut_link_though_the_file_names_say_another_title(
) {
    let got = decided(
        full(),
        &file("Velo (Clean)", "Halden Rook - Velo radio.mp3"),
        &file("Velo (Dirty)", "nightdrive promo side b.mp3"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Clean, MarkerKind::Dirty])
    );
    assert_eq!(
        got.reasons,
        [Reason::VersionVeto {
            only_a: vec![MarkerKind::Clean],
            only_b: vec![MarkerKind::Dirty],
        }]
    );
    assert_eq!(got.reading, Reading::Tags);
}

#[test]
fn full_audio_with_file_names_that_say_clean_and_dirty_is_a_cut_link_though_the_tags_say_another_title(
) {
    let got = decided(
        full(),
        &file("Velo radio", "Halden Rook - Velo (Clean).mp3"),
        &file("nightdrive promo side b", "Halden Rook - Velo (Dirty).mp3"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Clean, MarkerKind::Dirty])
    );
    assert_eq!(codes(&got), ["versionVeto"]);
    assert_eq!(got.reading, Reading::FileNames);
}

#[test]
fn full_audio_with_file_names_that_say_clean_and_dirty_is_a_cut_link_though_the_tags_say_the_same()
{
    let got = decided(
        full(),
        &file("Velo", "Halden Rook - Velo (Clean).mp3"),
        &file("Velo", "Halden Rook - Velo (Dirty).mp3"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Clean, MarkerKind::Dirty])
    );
    assert_eq!(got.reading, Reading::FileNames);
}

#[test]
fn full_audio_where_one_reading_says_a_cut_and_the_other_a_rework_is_a_cut_link() {
    let got = decided(
        full(),
        &file("Velo (Quill Ashby Remix)", "Velo (Clean).mp3"),
        &file("Velo", "Velo (Dirty).mp3"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Clean, MarkerKind::Dirty])
    );
    assert_eq!(got.reading, Reading::FileNames);
}

#[test]
fn full_audio_where_one_reading_says_a_rework_goes_to_review_whatever_the_other_says() {
    for (what, title_a, title_b) in [
        ("tags say the same", "Velo", "Velo"),
        ("tags say another title", "Velo", "Nightdrive"),
    ] {
        let got = decided(
            full(),
            &file(title_a, "Velo (Quill Ashby Remix).mp3"),
            &file(title_b, "Velo.mp3"),
        );
        assert_eq!(got.outcome, Outcome::NeedsReview, "{what}");
        assert_eq!(
            codes(&got),
            ["sameAudioNamesDisagree", "namesSayRework"],
            "{what}"
        );
        assert_eq!(got.reading, Reading::FileNames, "{what}");
    }
}

#[test]
fn full_audio_where_one_reading_cant_be_told_goes_to_review_whatever_the_other_says() {
    for (what, name_a, name_b) in [
        ("file names say the same", "Velo.mp3", "Velo.flac"),
        (
            "file names say another title",
            "Velo.mp3",
            "nightdrive promo.mp3",
        ),
    ] {
        let got = decided(
            full(),
            &file("Velo (Skyline Session)", name_a),
            &file("Velo", name_b),
        );
        assert_eq!(got.outcome, Outcome::NeedsReview, "{what}");
        assert_eq!(
            codes(&got),
            ["sameAudioNamesDisagree", "namesUnclear"],
            "{what}"
        );
        assert_eq!(got.reading, Reading::Tags, "{what}");
    }
}

#[test]
fn full_audio_merges_only_when_no_reading_says_a_cut_a_rework_or_cant_tell() {
    // The same on one reading, another title on the other: a duplicate,
    // and the names agree where it matters, so nothing is said of them.
    for (a, b, reading) in [
        (
            file("Velo", "Halden Rook - Velo.mp3"),
            file("Velo", "nightdrive promo.mp3"),
            Reading::Tags,
        ),
        (
            file("Velo", "Halden Rook - Velo.mp3"),
            file("nightdrive promo", "Halden Rook - Velo.flac"),
            Reading::FileNames,
        ),
    ] {
        let got = decided(full(), &a, &b);
        assert_eq!(got.outcome, Outcome::Duplicate);
        assert_eq!(codes(&got), ["sameAudio"]);
        assert_eq!(got.reading, reading);
    }
    // Another title on both: still a duplicate, and it says so.
    let got = decided(
        full(),
        &file("Velo", "Halden Rook - Velo.mp3"),
        &file("Nightdrive", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio", "namesDiffer"]);
}

// ---- Identical audio: always a duplicate, the cautious reading shown ---

#[test]
fn identical_audio_says_the_names_differ_when_either_reading_gives_two_versions() {
    let got = decided(
        Audio::Identical,
        &file("Velo", "Halden Rook - Velo (Clean).mp3"),
        &file("Velo", "Halden Rook - Velo (Dirty).mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(got.reasons, [Reason::IdenticalAudio, Reason::NamesDiffer]);
    assert_eq!(got.reading, Reading::FileNames);
}

#[test]
fn identical_audio_says_nothing_of_the_names_when_one_reading_says_the_same_and_the_other_another_title(
) {
    let got = decided(
        Audio::Identical,
        &file("Velo", "Halden Rook - Velo.mp3"),
        &file("Velo", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(got.reasons, [Reason::IdenticalAudio]);
}

// ---- One file inside the other -----------------------------------------

#[test]
fn one_sided_coverage_takes_its_labels_from_whichever_reading_gives_a_cut() {
    let got = decided(
        a_inside_b(),
        &file("Velo", "Velo (Radio Edit).mp3"),
        &file("Velo", "Velo (Extended Mix).mp3"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Extended, MarkerKind::RadioEdit])
    );
    assert_eq!(codes(&got), ["audioInsideOther", "namesSayCut"]);
    assert_eq!(got.reading, Reading::FileNames);
}

#[test]
fn one_sided_coverage_where_one_reading_says_a_rework_goes_to_review_though_the_other_says_the_same(
) {
    let got = decided(
        a_inside_b(),
        &file("Velo", "Velo.mp3"),
        &file("Velo", "Velo (Quill Ashby Remix).mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        codes(&got),
        ["audioInsideOtherNamesDisagree", "namesSayRework"]
    );
}

#[test]
fn one_sided_coverage_is_a_cut_link_when_one_reading_says_the_same_and_the_other_another_title() {
    let got = decided(
        a_inside_b(),
        &file("Velo", "Halden Rook - Velo.mp3"),
        &file("Velo", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(codes(&got), ["audioInsideOther"]);
    assert_eq!(got.reading, Reading::Tags);
}

// ---- Little or no audio: the reading that tells the files apart wins ---

#[test]
fn with_low_or_no_coverage_a_version_label_only_the_file_names_carry_still_makes_the_link() {
    for audio in [
        partial(),
        nothing_shared(),
        Audio::NotCompared,
        Audio::Unknown,
    ] {
        let cut = decided(
            audio,
            &file("Velo", "Velo (Extended Mix).mp3"),
            &file("Velo", "Velo.mp3"),
        );
        assert_eq!(cut.outcome, cut_link(&[MarkerKind::Extended]));
        assert_eq!(codes(&cut), ["namesSayCut"]);
        assert_eq!(cut.reading, Reading::FileNames);

        let rework = decided(
            audio,
            &file("Velo", "Velo [Vey Sun Flip].mp3"),
            &file("Velo", "Velo.mp3"),
        );
        assert_eq!(rework.outcome, rework_link(&[MarkerKind::Flip]));
        assert_eq!(codes(&rework), ["namesSayRework"]);
        assert_eq!(rework.reading, Reading::FileNames);
    }
}

#[test]
fn with_low_or_no_coverage_a_rework_on_one_reading_comes_before_a_cut_on_the_other() {
    let got = decided(
        nothing_shared(),
        &file("Velo (Quill Ashby Remix)", "Velo (Extended Mix).mp3"),
        &file("Velo", "Velo.mp3"),
    );
    assert_eq!(got.outcome, rework_link(&[MarkerKind::Remix]));
    assert_eq!(got.reading, Reading::Tags);
}

#[test]
fn the_same_names_on_one_reading_count_as_the_same_names_when_the_other_says_less() {
    // The tags are the same; the file names can't be told, or say another
    // title. Neither hides that the tags agree.
    for name_b in ["Velo (Skyline Session).mp3", "nightdrive promo.mp3"] {
        let (a, b) = (file("Velo", "Velo.mp3"), file("Velo", name_b));
        let part = decided(partial(), &a, &b);
        assert_eq!(part.outcome, Outcome::NeedsReview, "{name_b}");
        assert_eq!(codes(&part), ["sameTagsPartialAudio"], "{name_b}");
        let none = decided(nothing_shared(), &a, &b);
        assert_eq!(codes(&none), ["fingerprintVeto"], "{name_b}");
        let not_compared = decided(Audio::NotCompared, &a, &b);
        assert_eq!(not_compared.outcome, Outcome::NeedsReview, "{name_b}");
        assert_eq!(codes(&not_compared), ["notCompared"], "{name_b}");
        let unknown = decided(Audio::Unknown, &a, &b);
        assert_eq!(codes(&unknown), ["noFingerprint"], "{name_b}");
    }
}

#[test]
fn the_same_file_names_under_tags_that_say_another_title_count_as_the_same_names() {
    let (a, b) = (
        file("Velo", "Halden Rook - Velo.mp3"),
        file("nightdrive promo", "Halden Rook - Velo.flac"),
    );
    let got = decided(partial(), &a, &b);
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameTagsPartialAudio"]);
    assert_eq!(got.reading, Reading::FileNames);
}

#[test]
fn another_title_on_both_readings_is_another_title() {
    let got = decided(
        partial(),
        &file("Velo", "Halden Rook - Velo.mp3"),
        &file("Nightdrive", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::Different);
    assert_eq!(got.reasons, [Reason::TitlesDiffer]);
}

// ---- Which names there are to read -------------------------------------

#[test]
fn of_two_readings_that_say_the_same_kind_of_thing_the_tags_decide() {
    let got = decided(
        full(),
        &file("Velo (Clean)", "Velo (Intro).mp3"),
        &file("Velo (Dirty)", "Velo.mp3"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Clean, MarkerKind::Dirty])
    );
    assert_eq!(got.reading, Reading::Tags);
}

#[test]
fn two_files_with_no_title_tag_are_read_by_file_name_alone() {
    let got = decided(
        full(),
        &named("Halden Rook - Velo (Clean).mp3"),
        &named("Halden Rook - Velo (Dirty).mp3"),
    );
    assert_eq!(codes(&got), ["versionVeto"]);
    assert_eq!(got.reading, Reading::FileNames);
}

#[test]
fn a_file_with_no_title_tag_stands_in_with_its_file_name_beside_a_tagged_one() {
    let got = decided(
        full(),
        &named("Halden Rook - Velo (Clean).mp3"),
        &file("Velo (Dirty)", "nightdrive promo.mp3"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Clean, MarkerKind::Dirty])
    );
    assert_eq!(got.reading, Reading::Tags);
}

#[test]
fn a_blank_title_tag_counts_as_no_title_tag() {
    for blank in ["", "   "] {
        let got = decided(
            full(),
            &FileFacts::new(Some(blank), None, "Halden Rook - Velo (Clean).mp3"),
            &FileFacts::new(Some(blank), None, "Halden Rook - Velo (Dirty).mp3"),
        );
        assert_eq!(codes(&got), ["versionVeto"]);
        assert_eq!(got.reading, Reading::FileNames);
    }
}

#[test]
fn the_artist_tag_counts_for_both_readings() {
    // The file names give no artist; the tags' artists differ, so a link
    // the file names alone would make isn't made.
    let got = decided(
        nothing_shared(),
        &FileFacts::new(Some("Velo"), Some("Kestrel Nine"), "Velo (Dub).mp3"),
        &FileFacts::new(Some("Velo"), Some("The Marrow Choir"), "Velo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::Different);
    assert_eq!(got.reasons, [Reason::SameTitleDifferentArtists]);
    assert_eq!(got.reading, Reading::FileNames);
}

// ---- The order among the rarer readings --------------------------------

#[test]
fn one_sided_coverage_where_one_reading_says_a_rework_goes_to_review_though_the_other_cant_be_told()
{
    let got = decided(
        a_inside_b(),
        &file("Velo (Skyline Session)", "Velo.mp3"),
        &file("Velo", "Velo (Quill Ashby Remix).mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        codes(&got),
        ["audioInsideOtherNamesDisagree", "namesSayRework"]
    );
    assert_eq!(got.reading, Reading::FileNames);
}

#[test]
fn full_audio_where_one_reading_names_the_other_file_as_a_mashups_part_goes_to_review() {
    let got = decided(
        full(),
        &file("Duskline", "Duskline x Emberlow (Mashup).mp3"),
        &file("Duskline", "Duskline.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameAudioNamesDisagree", "mashupPart"]);
    assert_eq!(got.reading, Reading::FileNames);
}

#[test]
fn a_mashup_part_on_one_reading_makes_the_link_when_the_other_cant_be_told_or_says_another_title() {
    for name_b in [
        "Duskline x Emberlow (Skyline Session).mp3",
        "nightdrive.mp3",
    ] {
        let got = decided(
            partial(),
            &file("Duskline x Emberlow (Mashup)", "Duskline x Emberlow.mp3"),
            &file("Duskline", name_b),
        );
        assert_eq!(got.outcome, rework_link(&[MarkerKind::Mashup]), "{name_b}");
        assert_eq!(codes(&got), ["mashupContains"], "{name_b}");
        assert_eq!(got.reading, Reading::Tags, "{name_b}");
    }
}

#[test]
fn the_same_names_on_one_reading_come_before_a_mashup_part_on_the_other() {
    let got = decided(
        partial(),
        &file("Duskline", "Duskline x Emberlow (Mashup).mp3"),
        &file("Duskline", "Duskline.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameTagsPartialAudio"]);
    assert_eq!(got.reading, Reading::Tags);
}
