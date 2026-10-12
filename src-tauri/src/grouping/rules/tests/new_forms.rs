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
