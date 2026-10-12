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

/// The version words of a file: its names' and the ones its folder adds.
fn all_words(file: &FileFacts) -> Vec<(MarkerKind, usize)> {
    let mut words = file.label_words.clone();
    for (kind, count) in &file.folder_words {
        *words.entry(*kind).or_default() += count;
    }
    words.into_iter().collect()
}

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
    assert_eq!(all_words(&clean), [(MarkerKind::Clean, 1)]);
    assert_eq!(all_words(&dirty), [(MarkerKind::Dirty, 1)]);
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
                    all_words(&inside),
                    all_words(&alone),
                    "{folder}/{file_name}"
                );
                assert!(inside.folder_words.is_empty(), "{folder}/{file_name}");
                let in_tag = FileFacts::in_folder(
                    Some(&file_name[..file_name.len() - 4]),
                    None,
                    "x.mp3",
                    Some(&folder),
                );
                assert_eq!(
                    all_words(&in_tag),
                    all_words(&FileFacts::new(
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
        all_words(&in_folder("Dirty Dirty", "Velo.mp3")),
        [(MarkerKind::Dirty, 1)]
    );
    assert_eq!(
        all_words(&in_folder("Dirty", "Velo (Clean).mp3")),
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
            all_words(&in_folder(folder, "Velo.mp3")),
            [(kind, 1)],
            "{folder}"
        );
    }
}

#[test]
fn a_folder_that_holds_no_label_word_adds_none() {
    for folder in ["Inbox", "Music", "2020", "Halden Rook", "Mixes", "Sets"] {
        assert_eq!(all_words(&in_folder(folder, "Velo.mp3")), [], "{folder}");
    }
}

#[test]
fn only_the_name_of_the_folder_counts_not_the_path_in_front_of_it() {
    assert_eq!(all_words(&in_folder("Clean/2020", "Velo.mp3")), []);
    assert_eq!(
        all_words(&in_folder("Music/Clean", "Velo.mp3")),
        [(MarkerKind::Clean, 1)]
    );
}

#[test]
fn a_folder_named_for_the_files_artist_is_no_version() {
    let by = |artist: Option<&str>| {
        FileFacts::in_folder(Some("Velo"), artist, "Velo.mp3", Some("Dub Kestrel"))
    };
    assert_eq!(all_words(&by(Some("Dub Kestrel"))), []);
    assert_eq!(all_words(&by(None)), [(MarkerKind::Dub, 1)]);
}

#[test]
fn a_file_with_no_folder_is_the_file_new_gives() {
    let a = FileFacts::in_folder(Some("Velo"), Some("Halden Rook"), "Velo (Dirty).mp3", None);
    let b = FileFacts::new(Some("Velo"), Some("Halden Rook"), "Velo (Dirty).mp3");
    assert_eq!(all_words(&a), all_words(&b));
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

#[test]
fn a_folders_word_and_a_file_names_word_that_swap_do_not_cancel() {
    // "Clean" is the folder's in A and the name's in B, and the other way
    // round for "Dirty": the files carry different words of their own.
    let a = in_folder("Clean", "Velo Dirty 128.mp3");
    let b = in_folder("Dirty", "Velo Clean 128.mp3");
    for (x, y) in [(&a, &b), (&b, &a)] {
        let got = decided(full(), x, y);
        assert_eq!(got.outcome, Outcome::NeedsReview);
        assert!(matches!(got.reasons[0], Reason::LabelWordsDiffer { .. }));
    }
}

#[test]
fn a_folders_word_never_makes_up_for_a_word_the_other_files_names_carry() {
    // Folder says X (or nothing), own name says Y (or nothing), on both
    // sides: two files merge only when their own words are equal and the
    // words their folders add are equal too.
    let labels = [
        None,
        Some("Clean"),
        Some("Dirty"),
        Some("Extended"),
        Some("Radio Edit"),
        Some("Dub"),
        Some("Instrumental"),
    ];
    let mut files = Vec::new();
    for folder in labels {
        for own in labels {
            let name = match own {
                Some(label) => format!("Velo {label} 128.mp3"),
                None => "Velo 128.mp3".to_string(),
            };
            let file = FileFacts::in_folder(None, None, &name, Some(folder.unwrap_or("Inbox")));
            // What the folder adds: its word, unless the names carry it.
            let added = folder.filter(|folder| Some(*folder) != own);
            files.push((folder, own, added, file));
        }
    }
    let mut merged = 0;
    for (folder_a, own_a, added_a, a) in &files {
        for (folder_b, own_b, added_b, b) in &files {
            let got = decided(full(), a, b);
            let may_merge = own_a == own_b && added_a == added_b;
            if got.outcome == Outcome::Duplicate {
                assert!(
                    may_merge,
                    "A: folder {folder_a:?}, name {own_a:?}; B: folder {folder_b:?}, name {own_b:?}"
                );
                merged += 1;
            }
        }
    }
    assert!(merged > 0, "equal words never merged");
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

// ---- Harmless brackets ---------------------------------------------------

/// `title` and `file_name` with `bracket` added at the end of the
/// title tag, the file name's title, or both.
#[derive(Debug, Clone, Copy)]
enum Where {
    Tag,
    FileName,
    Both,
}

fn with_bracket(title: &str, stem: &str, bracket: Option<&str>, at: Where) -> FileFacts {
    let tag = match (bracket, at) {
        (Some(b), Where::Tag | Where::Both) => format!("{title} {b}"),
        _ => title.to_string(),
    };
    let name = match (bracket, at) {
        (Some(b), Where::FileName | Where::Both) => format!("{stem} {b}.mp3"),
        _ => format!("{stem}.mp3"),
    };
    FileFacts::new(Some(&tag), Some("Halden Rook"), &name)
}

#[test]
fn adding_one_harmless_bracket_to_one_files_names_never_changes_the_outcome() {
    // (title, stem, whether the names end in a label). A bare number after
    // a label may be that label's number, so it is no harmless bracket there.
    let bases = [
        ("Velo", "Velo", false),
        ("Velo", "Halden Rook - Velo", false),
        ("Velo (Dirty)", "Velo (Dirty)", true),
        ("Velo (Halden Rook Remix)", "Velo - Radio Edit", true),
        ("Nightdrive", "Nightdrive", false),
    ];
    // One bracket of each kind, bare numbers included.
    let brackets = [
        ("(1)", true),
        ("(2011)", true),
        ("(124)", true),
        ("(8A)", false),
        ("[124 BPM]", false),
        ("[ABC123]", false),
    ];
    let mut compared = 0;
    for at in [Where::Tag, Where::FileName, Where::Both] {
        for (title_a, stem_a, label_a) in bases {
            let a = with_bracket(title_a, stem_a, None, at);
            for (title_b, stem_b, _) in bases {
                let b = with_bracket(title_b, stem_b, None, at);
                for (bracket, bare) in brackets {
                    if bare && label_a {
                        continue;
                    }
                    let bracketed = with_bracket(title_a, stem_a, Some(bracket), at);
                    for (name, audio) in every_audio() {
                        // The bracket on A, then the same pair the other way
                        // round: the bracket on the second file.
                        let before = decided(audio, &a, &b);
                        let after = decided(audio, &bracketed, &b);
                        assert_eq!(
                            after, before,
                            "{bracket} on {title_a} / {stem_a} ({at:?}) beside {title_b} / {stem_b}, {name}"
                        );
                        let before = decided(audio, &b, &a);
                        let after = decided(audio, &b, &bracketed);
                        assert_eq!(
                            after, before,
                            "{bracket} on the second file {title_a} / {stem_a} ({at:?}), {name}"
                        );
                        compared += 2;
                    }
                }
            }
        }
    }
    assert!(compared > 2000, "only {compared} comparisons ran");
}

#[test]
fn a_copy_counter_on_matched_audio_no_longer_makes_the_names_unclear() {
    for (a, b) in [
        ("Velo (1).mp3", "Velo.mp3"),
        ("Velo (2).mp3", "Velo (1).mp3"),
        ("Halden Rook - Velo (1).mp3", "Halden Rook - Velo.mp3"),
    ] {
        let got = decided(full(), &named(a), &named(b));
        assert_eq!(got.outcome, Outcome::Duplicate, "{a} / {b}");
        assert_eq!(codes(&got), ["sameAudio"], "{a} / {b}");
    }
}

#[test]
fn a_year_a_key_a_tempo_and_a_catalogue_number_on_matched_audio_no_longer_make_the_names_unclear() {
    for bracket in ["(2011)", "(8A)", "(124)", "[124 BPM]", "[ABC123]"] {
        let got = decided(
            full(),
            &named(&format!("Velo {bracket}.mp3")),
            &named("Velo.mp3"),
        );
        assert_eq!(got.outcome, Outcome::Duplicate, "{bracket}");
    }
}

#[test]
fn a_word_followed_by_digits_is_no_catalogue_number_and_still_goes_to_review() {
    for bracket in [
        "(Demo123)",
        "(Bonus123)",
        "(Rev123)",
        "(Mstr123)",
        "(Edt123)",
        "(abc123)",
        "(Abc123)",
        "(DEMO123)",
        "(REV123)",
    ] {
        let got = decided(
            full(),
            &named(&format!("Velo {bracket}.mp3")),
            &named("Velo.mp3"),
        );
        assert_eq!(got.outcome, Outcome::NeedsReview, "{bracket}");
    }
    // A code in capitals is harmless, with or without its hyphen.
    for bracket in ["(ABC123)", "[XYZ-001]"] {
        let got = decided(
            full(),
            &named(&format!("Velo {bracket}.mp3")),
            &named("Velo.mp3"),
        );
        assert_eq!(got.outcome, Outcome::Duplicate, "{bracket}");
    }
}

#[test]
fn a_bracket_that_could_be_a_version_still_sends_matched_audio_to_review() {
    for bracket in [
        "(Part 1)",
        "(Pt 2)",
        "(Take 2)",
        "(01)",
        "(808)",
        "(Dub 12)",
        "(2011 Version)",
        "(Acoustic)",
        "(ABC 123)",
    ] {
        let got = decided(
            full(),
            &named(&format!("Velo {bracket}.mp3")),
            &named("Velo.mp3"),
        );
        assert_eq!(got.outcome, Outcome::NeedsReview, "{bracket}");
    }
}

#[test]
fn a_number_after_a_label_is_not_a_harmless_bracket() {
    for (with, without) in [
        ("Velo (Dub) (2).mp3", "Velo (Dub).mp3"),
        ("Velo Dub (2).mp3", "Velo Dub.mp3"),
        ("Velo (Dirty) (1).mp3", "Velo (Dirty).mp3"),
        ("Velo (Remix) (2011).mp3", "Velo (Remix).mp3"),
    ] {
        let got = decided(full(), &named(with), &named(without));
        // A bracket nobody can settle: never a silent merge.
        assert_ne!(got.outcome, Outcome::Duplicate, "{with}");
    }
}

#[test]
fn two_files_whose_brackets_of_one_kind_disagree_still_go_to_review() {
    for (a, b) in [
        ("Velo (2011).mp3", "Velo (2012).mp3"),
        ("Velo (8A).mp3", "Velo (9A).mp3"),
        ("Velo [124 BPM].mp3", "Velo [126 BPM].mp3"),
        ("Velo [ABC123].mp3", "Velo [ABC124].mp3"),
    ] {
        let got = decided(full(), &named(a), &named(b));
        assert_eq!(got.outcome, Outcome::NeedsReview, "{a} / {b}");
    }
}

#[test]
fn a_year_beside_a_remaster_word_anywhere_in_the_files_names_is_a_version() {
    // A's file name has a year, its tag says remaster; B is the remaster
    // of another year. The year is the remaster's: not harmless.
    let a = FileFacts::new(Some("Velo (Remastered)"), None, "Velo (2011).mp3");
    let b = FileFacts::new(Some("Velo (Remastered 2015)"), None, "Velo.mp3");
    let got = decided(full(), &a, &b);
    assert_ne!(got.outcome, Outcome::Duplicate);
    // The same pair without the year in A merges: a remaster that gives
    // no year is the same as any (ROADMAP 1.5).
    let a = FileFacts::new(Some("Velo (Remastered)"), None, "Velo.mp3");
    assert_eq!(decided(full(), &a, &b).outcome, Outcome::Duplicate);
    // Any spelling that says "master" counts.
    for tag in ["Velo (Re-master)", "Velo remastered", "Velo (Mastered)"] {
        let a = FileFacts::new(Some(tag), None, "Velo (2011).mp3");
        let b = FileFacts::new(Some(tag), None, "Velo.mp3");
        assert_ne!(decided(full(), &a, &b).outcome, Outcome::Duplicate, "{tag}");
    }
    // A year with no remaster word anywhere is harmless.
    let a = FileFacts::new(Some("Velo"), None, "Velo (2011).mp3");
    let b = FileFacts::new(Some("Velo"), None, "Velo.mp3");
    assert_eq!(decided(full(), &a, &b).outcome, Outcome::Duplicate);
}

#[test]
fn a_harmless_bracket_beside_a_real_version_difference_changes_nothing() {
    let got = decided(
        full(),
        &named("Velo (Dirty) [124 BPM].mp3"),
        &named("Velo (Clean).mp3"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Clean, MarkerKind::Dirty])
    );
}

// ---- A file name whose second part is only a label ---------------------

#[test]
fn a_file_name_whose_second_part_is_a_label_is_that_label_of_the_tags_title() {
    // Tagged "Velo": "Velo - Radio Edit.mp3" is the Radio Edit of Velo.
    let radio = FileFacts::new(Some("Velo"), None, "Velo - Radio Edit.mp3");
    let plain = FileFacts::new(Some("Velo"), None, "Velo.mp3");
    let got = decided(full(), &radio, &plain);
    assert_eq!(got.outcome, cut_link(&[MarkerKind::RadioEdit]));
    assert_eq!(codes(&got), ["versionVeto"]);
    // The same version in two spellings is the same.
    let bracketed = FileFacts::new(Some("Velo"), None, "Velo (Radio Edit).mp3");
    assert_eq!(
        decided(full(), &radio, &bracketed).outcome,
        Outcome::Duplicate
    );
    // Without matched audio the names alone link them.
    let got = decided(Audio::Unknown, &radio, &plain);
    assert_eq!(got.outcome, cut_link(&[MarkerKind::RadioEdit]));
}

#[test]
fn a_file_name_whose_second_part_is_a_label_beside_a_label_in_the_tag_is_one_version() {
    let tagged = FileFacts::new(Some("Velo (Radio Edit)"), None, "Velo - Radio Edit.mp3");
    let bracketed = FileFacts::new(Some("Velo (Radio Edit)"), None, "Velo.mp3");
    assert_eq!(
        decided(full(), &tagged, &bracketed).outcome,
        Outcome::Duplicate
    );
}

#[test]
fn a_track_really_titled_like_a_label_stays_a_title() {
    // The tag says "Radio Edit" is the title and "Halden Rook" the artist.
    let titled =
        |file_name: &str| FileFacts::new(Some("Radio Edit"), Some("Halden Rook"), file_name);
    let (a, b) = (
        titled("Halden Rook - Radio Edit.mp3"),
        titled("Halden Rook - Radio Edit.flac"),
    );
    assert_eq!(decided(full(), &a, &b).outcome, Outcome::Duplicate);
    // ... and it is another track than Halden Rook's "Velo".
    let velo = FileFacts::new(Some("Velo"), Some("Halden Rook"), "Halden Rook - Velo.mp3");
    assert_eq!(decided(partial(), &a, &velo).outcome, Outcome::Different);
}

#[test]
fn without_a_title_tag_to_say_so_a_name_with_a_label_second_stays_artist_and_title() {
    let radio = named("Velo - Radio Edit.mp3");
    let plain = named("Velo.mp3");
    let got = decided(full(), &radio, &plain);
    // Still never a silent merge: the guard sees the label word.
    assert_ne!(got.outcome, Outcome::Duplicate);
    assert_eq!(label_words(&radio), [(MarkerKind::RadioEdit, 1)]);
}

#[test]
fn a_tag_that_is_neither_part_leaves_the_name_as_the_parser_reads_it() {
    let radio = FileFacts::new(Some("Nightdrive"), None, "Velo - Radio Edit.mp3");
    let plain = FileFacts::new(Some("Nightdrive"), None, "Nightdrive.mp3");
    assert_ne!(decided(full(), &radio, &plain).outcome, Outcome::Duplicate);
}

#[test]
fn a_label_added_as_the_second_part_of_a_file_name_never_comes_out_duplicate_on_a_full_match() {
    let labels = [
        "Clean",
        "Dirty",
        "Extended Mix",
        "Radio Edit",
        "Dub",
        "Instrumental",
        "Live",
        "Edit",
        "Remix",
        "Short Edit",
        "Intro",
        "VIP",
        "V.I.P.",
    ];
    let spare_tags = [Some("Velo"), None];
    for label in labels {
        for tag in spare_tags {
            for artist in [None, Some("Halden Rook")] {
                for stem in ["Velo", "Halden Rook - Velo"] {
                    let plain = FileFacts::new(tag, artist, &format!("{stem}.mp3"));
                    let labeled = FileFacts::new(tag, artist, &format!("{stem} - {label}.mp3"));
                    for (a, b) in [(&plain, &labeled), (&labeled, &plain)] {
                        let got = decided(full(), a, b);
                        assert_ne!(
                            got.outcome,
                            Outcome::Duplicate,
                            "{label} / {tag:?} / {artist:?} / {stem}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_harmless_bracket_after_a_second_part_label_does_not_hide_it() {
    let radio = FileFacts::new(Some("Velo"), None, "Velo - Radio Edit [124 BPM].mp3");
    let plain = FileFacts::new(Some("Velo"), None, "Velo.mp3");
    let got = decided(full(), &radio, &plain);
    assert_eq!(got.outcome, cut_link(&[MarkerKind::RadioEdit]));
}

// ---- Guards pinned after the PR review -------------------------------------

#[test]
fn the_title_tag_must_be_the_first_part_for_a_file_name_to_be_read_title_first() {
    let read = |tag: &str| {
        let by_tag = crate::versions::parse_title(tag);
        file_name_reading("Velo - Radio Edit.mp3", Some(&by_tag))
    };
    // The tag is the first part: "Velo" is the title, "Radio Edit" a label.
    let titled = read("Velo");
    assert_eq!(titled.base_title, "Velo");
    assert!(titled.credits.is_empty());
    assert_eq!(titled.markers[0].kind, MarkerKind::RadioEdit);
    // The tag is the second part, or neither: the parser's own reading,
    // an artist "Velo" and a title "Radio Edit".
    for tag in ["Radio Edit", "Nightdrive", "Velo Nightdrive"] {
        let default = read(tag);
        assert_eq!(default.base_title, "Radio Edit", "{tag}");
        assert_eq!(default.credits[0].name, "Velo", "{tag}");
        assert!(default.markers.is_empty(), "{tag}");
    }
    // No tag: the same.
    let untagged = file_name_reading("Velo - Radio Edit.mp3", None);
    assert_eq!(untagged.base_title, "Radio Edit");
}

#[test]
fn a_bracket_that_could_be_a_version_stops_the_harmless_ones_in_front_of_it() {
    // Nothing in front of a bracket that isn't harmless is an aside, so
    // two different counters there are still two unsettled brackets.
    for (a, b) in [
        ("Velo (1) (Acoustic).mp3", "Velo (2) (Acoustic).mp3"),
        ("Velo (1) (Part 2).mp3", "Velo (2) (Part 2).mp3"),
    ] {
        let got = decided(full(), &named(a), &named(b));
        assert_eq!(got.outcome, Outcome::NeedsReview, "{a} / {b}");
    }
    // A counter after a label is that label's number, whichever file has it.
    for (a, b) in [
        ("Velo (Dirty) (1).mp3", "Velo (1).mp3"),
        ("Velo (1).mp3", "Velo (Dirty) (1).mp3"),
        ("Velo (1) (Dirty).mp3", "Velo (1).mp3"),
    ] {
        let got = decided(full(), &named(a), &named(b));
        assert_ne!(got.outcome, Outcome::Duplicate, "{a} / {b}");
    }
}
