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

// ---- A version given by the folder -------------------------------------

/// A file with no tags, named `file_name`, in a folder called `folder`.
fn in_folder(folder: &str, file_name: &str) -> FileFacts {
    FileFacts::in_folder(None, None, file_name, Some(folder))
}

#[test]
fn a_clean_folder_and_a_dirty_folder_holding_the_same_file_name_never_merge_on_full_audio() {
    let (clean, dirty) = (
        in_folder("Clean", "Velo.mp3"),
        in_folder("Dirty", "Velo.mp3"),
    );
    assert_eq!(label_words(&clean), [(MarkerKind::Clean, 1)]);
    assert_eq!(label_words(&dirty), [(MarkerKind::Dirty, 1)]);
    let got = decided(full(), &clean, &dirty);
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons[0],
        differ(&[MarkerKind::Clean], &[MarkerKind::Dirty])
    );
}

#[test]
fn a_folder_label_the_file_names_already_carry_adds_nothing_so_the_file_still_merges_elsewhere() {
    let got = decided(
        full(),
        &in_folder("Clean", "Velo (Clean).mp3"),
        &in_folder("Inbox", "Velo (Clean).mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn a_remixes_folder_holding_a_remix_still_merges_with_the_same_file_elsewhere() {
    let got = decided(
        full(),
        &in_folder("Remixes", "Velo (Halden Rook Remix).mp3"),
        &in_folder("Inbox", "Velo (Halden Rook Remix).mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
}

#[test]
fn an_edits_folder_beside_an_inbox_file_of_the_same_name_goes_to_review() {
    // Accepted: the folder may be the only place the version is written.
    let (edits, inbox) = (
        in_folder("Edits", "Velo.mp3"),
        in_folder("Inbox", "Velo.mp3"),
    );
    let got = decided(full(), &edits, &inbox);
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(got.reasons[0], differ(&[MarkerKind::Edit], &[]));
}

#[test]
fn a_folder_label_the_names_already_carry_changes_nothing_however_the_names_write_it() {
    let written = |label: &str| {
        [
            format!("Velo ({label}).mp3"),
            format!("Velo [{label}].mp3"),
            format!("Velo {label}.mp3"),
            format!("Velo - {label}.mp3"),
            format!("Velo {label} 128.mp3"),
            format!("{label} Velo.mp3"),
        ]
    };
    let plural = |label: &str| match label.ends_with('x') {
        true => format!("{label}es"),
        false => format!("{label}s"),
    };
    for label in [
        "Clean",
        "Dirty",
        "Extended",
        "Radio Edit",
        "Dub",
        "Instrumental",
        "Remix",
    ] {
        for file_name in written(label) {
            for folder in [label.to_string(), plural(label), label.to_uppercase()] {
                let alone = FileFacts::new(None, None, &file_name);
                let inside = in_folder(&folder, &file_name);
                assert_eq!(
                    label_words(&inside),
                    label_words(&alone),
                    "{folder}/{file_name}"
                );
                let in_tag = FileFacts::in_folder(
                    Some(&file_name[..file_name.len() - 4]),
                    None,
                    "x.mp3",
                    Some(&folder),
                );
                assert_eq!(
                    label_words(&in_tag),
                    label_words(&FileFacts::new(
                        Some(&file_name[..file_name.len() - 4]),
                        None,
                        "x.mp3"
                    )),
                    "{folder} / tag {file_name}"
                );
            }
        }
    }
}

#[test]
fn a_folders_label_word_counts_once_and_beside_the_names_other_labels() {
    assert_eq!(
        label_words(&in_folder("Dirty Dirty", "Velo.mp3")),
        [(MarkerKind::Dirty, 1)]
    );
    assert_eq!(
        label_words(&in_folder("Dirty", "Velo (Clean).mp3")),
        [(MarkerKind::Clean, 1), (MarkerKind::Dirty, 1)]
    );
}

#[test]
fn a_folders_plural_and_two_word_labels_are_read() {
    for (folder, kind) in [
        ("Remixes", MarkerKind::Remix),
        ("Radio Edits", MarkerKind::RadioEdit),
        ("Instrumentals", MarkerKind::Instrumental),
        ("Acapellas", MarkerKind::Acapella),
        ("Extended Mixes", MarkerKind::Extended),
        ("V.I.P.", MarkerKind::Vip),
    ] {
        assert_eq!(
            label_words(&in_folder(folder, "Velo.mp3")),
            [(kind, 1)],
            "{folder}"
        );
    }
}

#[test]
fn a_folder_that_holds_no_label_word_adds_none() {
    for folder in ["Inbox", "Music", "2020", "Halden Rook", "Mixes", "Sets"] {
        assert_eq!(label_words(&in_folder(folder, "Velo.mp3")), [], "{folder}");
    }
}

#[test]
fn only_the_name_of_the_folder_counts_not_the_path_in_front_of_it() {
    assert_eq!(label_words(&in_folder("Clean/2020", "Velo.mp3")), []);
    assert_eq!(
        label_words(&in_folder("Music/Clean", "Velo.mp3")),
        [(MarkerKind::Clean, 1)]
    );
}

#[test]
fn a_folder_named_for_the_files_artist_is_no_version() {
    let by = |artist: Option<&str>| {
        FileFacts::in_folder(Some("Velo"), artist, "Velo.mp3", Some("Dub Kestrel"))
    };
    assert_eq!(label_words(&by(Some("Dub Kestrel"))), []);
    assert_eq!(label_words(&by(None)), [(MarkerKind::Dub, 1)]);
}

#[test]
fn a_file_with_no_folder_is_the_file_new_gives() {
    let a = FileFacts::in_folder(Some("Velo"), Some("Halden Rook"), "Velo (Dirty).mp3", None);
    let b = FileFacts::new(Some("Velo"), Some("Halden Rook"), "Velo (Dirty).mp3");
    assert_eq!(label_words(&a), label_words(&b));
}

#[test]
fn a_version_given_only_by_a_folder_never_comes_out_duplicate_on_a_full_match() {
    let labels = [
        "Clean",
        "Dirty",
        "Extended",
        "Radio Edit",
        "Dub",
        "Instrumental",
        "Remix",
        "Live",
    ];
    for label in labels {
        for plain in ["Velo.mp3", "Velo 128.mp3", "Halden Rook - Velo.mp3"] {
            let (versioned, elsewhere) = (in_folder(label, plain), in_folder("Inbox", plain));
            for (a, b) in [(&versioned, &elsewhere), (&elsewhere, &versioned)] {
                let got = decided(full(), a, b);
                assert_ne!(got.outcome, Outcome::Duplicate, "{label}/{plain}");
            }
        }
    }
}

// ---- Pinned after the 1bB-1 mutation review ----------------------------

#[test]
fn version_words_are_counted_per_file_as_a_multiset_not_a_set() {
    assert_eq!(
        label_words(&named("Velo Dub Dub.mp3")),
        [(MarkerKind::Dub, 2)]
    );
    assert_eq!(label_words(&named("Velo Dub.mp3")), [(MarkerKind::Dub, 1)]);
    for (twice, once) in [
        ("Velo Dub Dub.mp3", "Velo Dub.mp3"),
        ("Velo (Dub) Dub.mp3", "Velo (Dub).mp3"),
        ("Velo Dirty Dirty 128.mp3", "Velo Dirty 128.mp3"),
    ] {
        for (a, b) in [(named(twice), named(once)), (named(once), named(twice))] {
            let got = decided(full(), &a, &b);
            assert_ne!(got.outcome, Outcome::Duplicate, "{twice} / {once}");
        }
    }
    // The extra count is what the guard names.
    let got = decided(
        full(),
        &named("Velo Dub Dub 128.mp3"),
        &named("Velo Dub 128.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(got.reasons[0], differ(&[MarkerKind::Dub], &[]));
    // The same words the same number of times merge.
    let got = decided(
        full(),
        &named("Velo Dub Dub 128.mp3"),
        &named("Velo Dub Dub 128.flac"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
}

#[test]
fn a_label_written_in_both_of_a_files_names_counts_as_often_as_the_name_that_writes_it_most() {
    // Not summed over the two names ...
    let once = FileFacts::new(Some("Velo Dub 128"), None, "Velo Dub 128.mp3");
    assert_eq!(label_words(&once), [(MarkerKind::Dub, 1)]);
    // ... and not lost either: the most wins.
    let twice = FileFacts::new(Some("Velo Dub Dub 128"), None, "Velo Dub 128.mp3");
    assert_eq!(label_words(&twice), [(MarkerKind::Dub, 2)]);
    let got = decided(full(), &once, &twice);
    assert_ne!(got.outcome, Outcome::Duplicate);
}

#[test]
fn the_artist_is_set_aside_only_where_a_name_spells_the_whole_artist_out() {
    let dub = [(MarkerKind::Dub, 1)];
    let by = |artist: &str, file_name: &str| {
        label_words(&FileFacts::new(Some("Velo"), Some(artist), file_name))
    };
    // The whole artist, as words in a row: set aside.
    assert_eq!(by("Dub Kestrel", "Dub Kestrel - Velo.mp3"), []);
    assert_eq!(by("Dub Kestrel", "Velo - Dub Kestrel.mp3"), []);
    assert_eq!(by("Dub Kestrel", "Velo (Dub Kestrel).mp3"), []);
    // Only part of the artist, or the artist's words apart or reordered:
    // the label word stays.
    assert_eq!(by("Dub Kestrel", "Dub Velo.mp3"), dub);
    assert_eq!(by("Dub Kestrel", "Velo Dub.mp3"), dub);
    assert_eq!(by("Dub Kestrel", "Dub Velo Kestrel.mp3"), dub);
    assert_eq!(by("Dub Kestrel", "Kestrel Dub Velo.mp3"), dub);
    assert_eq!(by("Dub Kestrel", "Dub Halden Kestrel Velo.mp3"), dub);
    // The artist's other word alone is no label and sets nothing aside.
    assert_eq!(by("Dub Kestrel", "Velo Kestrel Dub.mp3"), dub);
    // The artist is taken out once: a second "Dub" is the file's own.
    assert_eq!(by("Dub Kestrel", "Dub Kestrel Velo Dub.mp3"), dub);
    assert_eq!(by("Dub Kestrel", "Dub Kestrel Dub Kestrel Velo.mp3"), dub);
    // Two artists in the tag: each is set aside whole.
    assert_eq!(
        by("Dub Kestrel & Radio Wren", "Dub Kestrel - Velo Radio.mp3"),
        [(MarkerKind::RadioEdit, 1)]
    );
    assert_eq!(by("Dub Kestrel & Radio Wren", "Radio Wren - Velo.mp3"), []);
}

#[test]
fn an_artist_the_name_does_not_spell_out_whole_never_lets_full_audio_merge_over_a_label_word() {
    let plain = FileFacts::new(Some("Velo"), Some("Dub Kestrel"), "Velo 128.mp3");
    for file_name in [
        "Velo Dub 128.mp3",
        "Dub Velo 128.mp3",
        "Velo Kestrel Dub 128.mp3",
    ] {
        let dubbed = FileFacts::new(Some("Velo"), Some("Dub Kestrel"), file_name);
        for (a, b) in [(&dubbed, &plain), (&plain, &dubbed)] {
            let got = decided(full(), a, b);
            assert_ne!(got.outcome, Outcome::Duplicate, "{file_name}");
        }
    }
    let spelled = FileFacts::new(
        Some("Velo"),
        Some("Dub Kestrel"),
        "Dub Kestrel - Velo 128.mp3",
    );
    assert_eq!(
        decided(full(), &spelled, &plain).outcome,
        Outcome::Duplicate
    );
}
