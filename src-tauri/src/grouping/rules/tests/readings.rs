//! A pair's titles are read twice, by title tag and by file name, and one
//! reading decides (module docs). The labels are the files' own, whichever
//! reading is taken: see `one_file`.

use super::*;

/// A file with a title tag and a file name.
fn file(title: &str, file_name: &str) -> FileFacts {
    tagged_and_named(title, file_name)
}

// ---- "Another title" never hides what the other reading says -----------

#[test]
fn full_audio_is_a_duplicate_with_nothing_said_of_the_names_when_either_reading_says_the_same() {
    for (a, b) in [
        (
            file("Velo", "Halden Rook - Velo.mp3"),
            file("Velo", "nightdrive promo.mp3"),
        ),
        (
            file("Velo", "Halden Rook - Velo.mp3"),
            file("nightdrive promo", "Halden Rook - Velo.flac"),
        ),
    ] {
        let got = decided(full(), &a, &b);
        assert_eq!(got.outcome, Outcome::Duplicate);
        assert_eq!(codes(&got), ["sameAudio"]);
    }
}

#[test]
fn full_audio_under_another_title_on_both_readings_is_a_duplicate_and_says_the_names_differ() {
    let got = decided(
        full(),
        &file("Velo", "Halden Rook - Velo.mp3"),
        &file("Nightdrive", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio", "namesDiffer"]);
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

#[test]
fn one_sided_coverage_is_a_cut_link_when_one_reading_says_the_same_and_the_other_another_title() {
    let got = decided(
        a_inside_b(),
        &file("Velo", "Halden Rook - Velo.mp3"),
        &file("Velo", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(codes(&got), ["audioInsideOther"]);
}

#[test]
fn the_same_names_on_one_reading_count_as_the_same_names_when_the_other_says_another_title() {
    let pairs = [
        // The tags are the same, the file names unrelated.
        (
            file("Velo", "Velo.mp3"),
            file("Velo", "nightdrive promo.mp3"),
        ),
        // The file names are the same, the tags unrelated.
        (
            file("Velo", "Halden Rook - Velo.mp3"),
            file("nightdrive promo", "Halden Rook - Velo.flac"),
        ),
    ];
    for (a, b) in pairs {
        let part = decided(partial(), &a, &b);
        assert_eq!(part.outcome, Outcome::NeedsReview);
        assert_eq!(codes(&part), ["sameTagsPartialAudio"]);
        let none = decided(nothing_shared(), &a, &b);
        assert_eq!(codes(&none), ["fingerprintVeto"]);
        let not_compared = decided(Audio::NotCompared, &a, &b);
        assert_eq!(not_compared.outcome, Outcome::NeedsReview);
        assert_eq!(codes(&not_compared), ["notCompared"]);
        let unknown = decided(Audio::Unknown, &a, &b);
        assert_eq!(codes(&unknown), ["noFingerprint"]);
    }
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

// ---- Label words a title keeps: one reading can't tell, the other can --

/// Tagged with bare label words, named plainly; beside a plain file.
fn bare_words_in_the_tag() -> (FileFacts, FileFacts) {
    (
        file("Paper Harbor Extended Mix", "Paper Harbor.mp3"),
        file("Paper Harbor", "Paper Harbor.flac"),
    )
}

#[test]
fn full_audio_goes_to_review_when_one_reading_cant_be_told_though_the_other_says_the_same() {
    let (a, b) = bare_words_in_the_tag();
    let got = decided(full(), &a, &b);
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameAudioNamesDisagree", "namesUnclear"]);
}

#[test]
fn full_audio_goes_to_review_when_one_reading_cant_be_told_and_the_other_says_another_title() {
    let got = decided(
        full(),
        &file("Paper Harbor Extended Mix", "other promo.mp3"),
        &file("Paper Harbor", "fourth promo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameAudioNamesDisagree", "namesUnclear"]);
}

#[test]
fn with_low_or_no_coverage_the_reading_that_says_the_same_comes_before_one_that_cant_be_told() {
    let (a, b) = bare_words_in_the_tag();
    let part = decided(partial(), &a, &b);
    assert_eq!(part.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&part), ["sameTagsPartialAudio"]);
    let unknown = decided(Audio::Unknown, &a, &b);
    assert_eq!(unknown.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&unknown), ["noFingerprint"]);
}

#[test]
fn a_cut_on_one_reading_comes_before_a_reading_that_cant_be_told() {
    // The tag keeps "Extended Mix" in the title (can't be told beside the
    // plain title); the file name brackets it (a cut).
    let (a, b) = (
        file(
            "Paper Harbor Extended Mix",
            "Paper Harbor (Extended Mix).mp3",
        ),
        file("Paper Harbor", "Paper Harbor.mp3"),
    );
    let same_audio = decided(full(), &a, &b);
    assert_eq!(same_audio.outcome, cut_link(&[MarkerKind::Extended]));
    assert_eq!(codes(&same_audio), ["versionVeto"]);
    let no_audio = decided(nothing_shared(), &a, &b);
    assert_eq!(no_audio.outcome, cut_link(&[MarkerKind::Extended]));
    assert_eq!(codes(&no_audio), ["namesSayCut"]);
}

// ---- A mashup part on one reading --------------------------------------

/// Tagged as a mashup by its "x" alone, named as the plain title; beside
/// the plain title.
fn mashup_by_tag_only() -> (FileFacts, FileFacts) {
    (
        file("Duskline x Emberlow", "Duskline.mp3"),
        file("Duskline", "Duskline.flac"),
    )
}

#[test]
fn full_audio_goes_to_review_when_one_reading_names_a_mashup_part_though_the_other_says_the_same() {
    let (a, b) = mashup_by_tag_only();
    let got = decided(full(), &a, &b);
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameAudioNamesDisagree", "mashupPart"]);
}

#[test]
fn with_low_or_no_coverage_the_reading_that_says_the_same_comes_before_a_mashup_part() {
    let (a, b) = mashup_by_tag_only();
    let got = decided(partial(), &a, &b);
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameTagsPartialAudio"]);
}

#[test]
fn a_mashup_part_on_one_reading_makes_the_link_when_the_other_says_another_title() {
    for audio in [partial(), b_inside_a()] {
        let got = decided(
            audio,
            &file("Duskline x Emberlow (Mashup)", "other promo.mp3"),
            &file("Duskline", "fourth promo.mp3"),
        );
        assert_eq!(got.outcome, rework_link(&[MarkerKind::Mashup]));
        assert_eq!(codes(&got), ["mashupContains"]);
    }
}

/// One reading names a mashup part, the other can't be told (the same
/// title, with a bracket nobody understands on one file).
fn mashup_part_or_unclear() -> (FileFacts, FileFacts) {
    (
        file("Duskline x Emberlow", "Duskline (Skyline Session).mp3"),
        file("Duskline", "Duskline.flac"),
    )
}

#[test]
fn a_mashup_part_on_one_reading_comes_before_a_reading_that_cant_be_told() {
    let (a, b) = mashup_part_or_unclear();
    // One inside the other: the mashup link, not the cut link "can't be
    // told" would give.
    let inside = decided(b_inside_a(), &a, &b);
    assert_eq!(inside.outcome, rework_link(&[MarkerKind::Mashup]));
    assert_eq!(codes(&inside), ["mashupContains"]);
    let part = decided(partial(), &a, &b);
    assert_eq!(part.outcome, rework_link(&[MarkerKind::Mashup]));
    assert_eq!(codes(&part), ["mashupContains"]);
}

// ---- Which names there are to read -------------------------------------

#[test]
fn two_files_with_no_title_tag_are_read_by_file_name_alone() {
    let got = decided(
        full(),
        &named("Halden Rook - Velo (Clean).mp3"),
        &named("Halden Rook - Velo (Dirty).mp3"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Clean, MarkerKind::Dirty])
    );
    assert_eq!(codes(&got), ["versionVeto"]);
}

#[test]
fn a_file_with_no_title_tag_stands_in_with_its_file_name_beside_a_tagged_one() {
    // The untagged file's name and the other's tag are the same title;
    // the other's file name is unrelated.
    let got = decided(
        partial(),
        &named("Halden Rook - Velo.mp3"),
        &file("Velo", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameTagsPartialAudio"]);
}

#[test]
fn a_blank_title_tag_counts_as_no_title_tag() {
    for blank in ["", "   "] {
        // Read as a tag, a blank title is no title at all, and the pair
        // would go to review; read by file name it is the same name.
        let got = decided(
            full(),
            &FileFacts::new(Some(blank), None, "Halden Rook - Velo.mp3"),
            &FileFacts::new(Some(blank), None, "Halden Rook - Velo.flac"),
        );
        assert_eq!(got.outcome, Outcome::Duplicate);
        assert_eq!(codes(&got), ["sameAudio"]);
    }
}

#[test]
fn the_artist_tag_counts_for_both_readings() {
    // The file names give no artist; the tags' artists differ, so a link
    // the file names alone would make isn't made.
    let got = decided(
        nothing_shared(),
        &FileFacts::new(Some("other promo"), Some("Kestrel Nine"), "Velo (Dub).mp3"),
        &FileFacts::new(Some("fourth promo"), Some("The Marrow Choir"), "Velo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::Different);
    assert_eq!(got.reasons, [Reason::SameTitleDifferentArtists]);
}
