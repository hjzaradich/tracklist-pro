//! Version words the guard learned to see in 1bD-4: dotted spellings, an
//! artist who is called like a label, a version given only by a folder,
//! and a label that is the whole second part of a file name. Also the
//! harmless brackets that must not send same-audio pairs to review.

use super::*;

/// The version words of a file, as `kind × count`.
fn label_words(file: &FileFacts) -> Vec<(MarkerKind, usize)> {
    file.label_words.iter().map(|(k, n)| (*k, *n)).collect()
}

fn differ(only_a: &[MarkerKind], only_b: &[MarkerKind]) -> Reason {
    Reason::LabelWordsDiffer {
        only_a: only_a.to_vec(),
        only_b: only_b.to_vec(),
    }
}

// ---- Dotted spellings --------------------------------------------------

#[test]
fn a_dotted_vip_in_the_middle_of_a_file_name_is_a_version_word() {
    for name in [
        "Velo V.I.P. 128.mp3",
        "Velo V.I.P 128.mp3",
        "Velo v.i.p. 128.mp3",
        "Velo (V.I.P.) 128.mp3",
        "Velo.V.I.P.128.mp3",
        "Velo_V.I.P._128.mp3",
    ] {
        assert_eq!(label_words(&named(name)), [(MarkerKind::Vip, 1)], "{name}");
    }
}

#[test]
fn full_audio_never_merges_a_dotted_vip_with_the_plain_file() {
    for dotted in [
        "Velo V.I.P. 128.mp3",
        "Velo 128 v.i.p. copy.mp3",
        "V.I.P. Velo 128.mp3",
    ] {
        let (a, b) = (named(dotted), named("Velo 128.mp3"));
        for (x, y, only_a, only_b) in [
            (&a, &b, vec![MarkerKind::Vip], vec![]),
            (&b, &a, vec![], vec![MarkerKind::Vip]),
        ] {
            let got = decided(full(), x, y);
            assert_eq!(got.outcome, Outcome::NeedsReview, "{dotted}");
            assert_eq!(got.reasons[0], differ(&only_a, &only_b), "{dotted}");
        }
    }
}

#[test]
fn a_dotted_vip_in_a_tag_is_seen_too() {
    let got = decided(
        full(),
        &FileFacts::new(Some("Velo V.I.P. 128"), None, "nightdrive promo.mp3"),
        &FileFacts::new(Some("Velo 128"), None, "other promo.mp3"),
    );
    assert_ne!(got.outcome, Outcome::Duplicate);
}

#[test]
fn the_dotted_and_the_plain_spelling_of_one_label_are_the_same_version_word() {
    let got = decided(
        full(),
        &named("Velo V.I.P. 128.mp3"),
        &named("Velo VIP 128.mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
}

#[test]
fn other_dotted_forms_of_known_label_words_are_read_as_the_label() {
    for (name, kind) in [
        ("Velo R.M.X. 128.mp3", MarkerKind::Remix),
        ("Velo D.U.B. 128.mp3", MarkerKind::Dub),
        ("Velo A.L.T. 128.mp3", MarkerKind::Alternate),
        ("Velo Q.H. 128.mp3", MarkerKind::QuickHit),
    ] {
        assert_eq!(label_words(&named(name)), [(kind, 1)], "{name}");
    }
}

#[test]
fn dotted_letters_that_spell_no_label_stay_ordinary_words() {
    for name in [
        "Velo A.B. 128.mp3",
        "Velo U.S.A. 128.mp3",
        "Velo Mr. X 128.mp3",
    ] {
        assert_eq!(label_words(&named(name)), [], "{name}");
    }
    // Words that are no label add no version word: the same audio under
    // names that differ only by them still merges.
    for name in ["Velo A.B. 128.mp3", "Velo U.S.A. 128.mp3"] {
        let got = decided(full(), &named(name), &named("Velo 128.mp3"));
        assert_eq!(got.outcome, Outcome::Duplicate, "{name}");
    }
}

#[test]
fn a_dotted_stretch_inside_a_longer_run_of_letters_is_still_found() {
    // "X.V.I.P.": the title's letter is no part of the label.
    assert_eq!(
        label_words(&named("X.V.I.P.128.mp3")),
        [(MarkerKind::Vip, 1)]
    );
}

#[test]
fn an_artist_called_with_a_dotted_label_is_set_aside_where_the_name_spells_it_out() {
    let by_vip = |file_name: &str| {
        label_words(&FileFacts::new(
            Some("Velo"),
            Some("V.I.P. Kestrel"),
            file_name,
        ))
    };
    assert_eq!(by_vip("V.I.P. Kestrel - Velo.mp3"), []);
    assert_eq!(by_vip("Velo V.I.P. 128.mp3"), [(MarkerKind::Vip, 1)]);
}

// ---- An artist called like a label -------------------------------------

/// A file tagged "Velo" by `artist`, named `file_name`.
fn by_artist(artist: &str, file_name: &str) -> FileFacts {
    FileFacts::new(Some("Velo"), Some(artist), file_name)
}

#[test]
fn an_artist_called_like_a_label_does_not_hide_a_real_version_word() {
    for (artist, label, kind) in [
        ("Live", "Live", MarkerKind::Live),
        ("Edit", "Edit", MarkerKind::Edit),
        ("Dub", "Dub", MarkerKind::Dub),
    ] {
        let file_name = format!("Velo {label} 128.mp3");
        assert_eq!(
            label_words(&by_artist(artist, &file_name)),
            [(kind, 1)],
            "{file_name}"
        );
        let got = decided(
            full(),
            &by_artist(artist, &file_name),
            &by_artist(artist, "Velo 128.mp3"),
        );
        assert_eq!(got.outcome, Outcome::NeedsReview, "{file_name}");
        assert_ne!(got.outcome, Outcome::Duplicate, "{file_name}");
    }
}

#[test]
fn an_artist_called_like_a_label_is_set_aside_where_the_file_names_artist_part_names_it() {
    for (artist, other) in [("Live", "Edit"), ("Edit", "Live"), ("Dub", "Clean")] {
        for file_name in [
            format!("{artist} - Velo 128.mp3"),
            format!("{artist} & Halden Rook - Velo 128.mp3"),
            format!("01 {artist} - Velo 128.mp3"),
        ] {
            let tag = format!("{artist} & Halden Rook");
            let file = FileFacts::new(Some("Velo"), Some(&tag), &file_name);
            assert_eq!(label_words(&file), [], "{file_name}");
            let plain = FileFacts::new(Some("Velo"), Some(&tag), "Velo 128.mp3");
            let got = decided(full(), &file, &plain);
            assert_eq!(got.outcome, Outcome::Duplicate, "{file_name}");
        }
        // Another label word beside the artist is still the file's.
        let file = by_artist(artist, &format!("{artist} - Velo {other} 128.mp3"));
        assert_eq!(label_words(&file).len(), 1, "{artist} - Velo {other}");
    }
}

#[test]
fn an_artist_called_like_a_label_creates_no_version_word_when_credited_with_feat() {
    // "feat. Live" credits an artist called Live: no live version.
    for (title, file_name) in [
        ("Velo (feat. Live)", "nightdrive promo.mp3"),
        ("Velo", "Velo feat. Live.mp3"),
    ] {
        let file = FileFacts::new(Some(title), Some("Halden Rook & Live"), file_name);
        assert_eq!(label_words(&file), [], "{title} / {file_name}");
    }
    // ... and the credit hides no other "Live" in the same name.
    let file = FileFacts::new(Some("Velo Live (feat. Live)"), None, "nightdrive promo.mp3");
    assert_eq!(label_words(&file), [(MarkerKind::Live, 1)]);
}

#[test]
fn an_artist_called_like_a_label_still_counts_when_the_file_name_names_no_artist_part() {
    // "Live Velo.mp3" by "Live": nothing says "Live" is the artist here.
    // Known and cautious: review, never a wrong merge.
    let file = by_artist("Live", "Live Velo.mp3");
    assert_eq!(label_words(&file), [(MarkerKind::Live, 1)]);
}

#[test]
fn two_files_by_an_artist_called_like_a_label_with_the_artist_part_in_front_still_merge() {
    let got = decided(
        full(),
        &by_artist("Live", "Live - Velo.mp3"),
        &by_artist("Live", "Live - Velo.flac"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
}

#[test]
fn an_artist_called_like_a_label_with_a_label_in_the_artist_part_of_the_name_is_not_hidden_in_the_title(
) {
    // The second "Live" is a version: the artist is set aside once.
    let file = by_artist("Live", "Live - Velo Live.mp3");
    assert_eq!(label_words(&file), [(MarkerKind::Live, 1)]);
}
