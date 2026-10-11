//! Picking each tag: the best-ranked file with a value wins; every other
//! file's differing value comes back with the pick.

use super::*;

fn titled(id: i64, title: &str) -> FileFacts {
    with(file(id), |f| f.tags.title = Some(title.into()))
}

fn field(picks: &Picks, field: TagField) -> &FieldPick {
    picks
        .fields
        .iter()
        .find(|p| p.field == field)
        .unwrap_or_else(|| panic!("{field:?} was not picked"))
}

fn text(value: &str) -> FieldValue {
    FieldValue::Text { text: value.into() }
}

#[test]
fn a_track_with_no_files_picks_nothing() {
    let picks = pick(&[]);
    assert!(picks.fields.is_empty());
    assert_eq!(picks.art, None);
}

#[test]
fn a_field_no_file_has_a_value_for_is_not_picked() {
    let picks = pick(&[titled(1, "Velo"), titled(2, "Velo")]);
    assert_eq!(picks.fields.len(), 1);
    assert_eq!(picks.fields[0].field, TagField::Title);
}

#[test]
fn a_value_only_one_file_has_comes_from_that_file() {
    let best = file(1);
    let other = with(file(2), |f| f.tags.genre = Some("Deep House".into()));
    let picks = pick(&[best, other]);
    let genre = field(&picks, TagField::Genre);
    assert_eq!(genre.value, text("Deep House"));
    assert_eq!(genre.from_file, 2);
    assert_eq!(genre.reason, PickReason::OnlyValue);
    assert!(genre.disagreements.is_empty());
}

#[test]
fn files_that_agree_give_the_value_with_no_disagreement() {
    let picks = pick(&[titled(1, "Velo"), titled(2, "Velo")]);
    let title = field(&picks, TagField::Title);
    assert_eq!(title.value, text("Velo"));
    assert_eq!(title.from_file, 1);
    assert_eq!(title.reason, PickReason::FilesAgree);
    assert_eq!(picks.disagreeing_fields().count(), 0);
}

#[test]
fn values_that_differ_only_in_case_spacing_or_unicode_form_agree() {
    let a = titled(1, "  Café   Velo ");
    // NFD ("e" + combining acute), other case, one space.
    let b = titled(2, "cafe\u{301} VELO");
    let picks = pick(&[a, b]);
    let title = field(&picks, TagField::Title);
    assert_eq!(title.reason, PickReason::FilesAgree);
    // The spelling is the picked file's, trimmed.
    assert_eq!(title.value, text("Café   Velo"));
}

#[test]
fn when_files_differ_the_best_file_wins_and_the_other_values_come_back() {
    let best = with(titled(1, "Velo"), |f| f.format.codec = Some(Codec::Flac));
    let other = titled(2, "Velo (Dirty)");
    let third = titled(3, "Velo!");
    let picks = pick(&[third, other, best]);
    let title = field(&picks, TagField::Title);
    assert_eq!(title.value, text("Velo"));
    assert_eq!(title.from_file, 1);
    assert_eq!(title.reason, PickReason::FromBestFile);
    assert_eq!(
        title.disagreements,
        vec![
            Disagreement {
                file_id: 2,
                value: text("Velo (Dirty)")
            },
            Disagreement {
                file_id: 3,
                value: text("Velo!")
            },
        ]
    );
    assert_eq!(picks.disagreeing_fields().count(), 1);
}

#[test]
fn there_is_no_majority_vote_two_lesser_files_that_agree_do_not_outvote_the_best() {
    let best = titled(1, "Velo (Original Mix)");
    let b = with(titled(2, "Velo"), |f| {
        f.quality.verdict = Some(Verdict::LowBitrate)
    });
    let c = with(titled(3, "Velo"), |f| {
        f.quality.verdict = Some(Verdict::LowBitrate)
    });
    let picks = pick(&[b, c, best]);
    let title = field(&picks, TagField::Title);
    assert_eq!(title.value, text("Velo (Original Mix)"));
    assert_eq!(title.from_file, 1);
    assert_eq!(title.disagreements.len(), 2);
}

#[test]
fn a_better_ranked_file_wins_whatever_order_the_files_are_given_in() {
    let low = with(titled(1, "Velo (rip)"), |f| {
        f.quality.verdict = Some(Verdict::LowBitrate)
    });
    let high = titled(2, "Velo");
    for files in [[low.clone(), high.clone()], [high, low]] {
        let picks = pick(&files);
        assert_eq!(field(&picks, TagField::Title).from_file, 2);
    }
}

#[test]
fn when_the_best_file_has_no_value_the_next_ranked_file_with_one_supplies_it() {
    let best = file(1);
    let second = with(titled(2, "Velo"), |f| {
        f.quality.verdict = Some(Verdict::LowBitrate)
    });
    let third = with(titled(3, "Velo II"), |f| {
        f.quality.verdict = Some(Verdict::Truncated)
    });
    let picks = pick(&[third, second, best]);
    let title = field(&picks, TagField::Title);
    assert_eq!(title.value, text("Velo"));
    assert_eq!(title.from_file, 2);
    assert_eq!(title.reason, PickReason::BestFileBlank);
    assert_eq!(title.disagreements.len(), 1);
    assert_eq!(title.disagreements[0].file_id, 3);
}

#[test]
fn a_blank_value_is_not_a_value_and_does_not_disagree() {
    let blank = with(file(1), |f| f.tags.artist = Some("   ".into()));
    let real = with(file(2), |f| f.tags.artist = Some("Halden Rook".into()));
    let picks = pick(&[blank, real]);
    let artist = field(&picks, TagField::Artist);
    assert_eq!(artist.from_file, 2);
    assert_eq!(artist.reason, PickReason::OnlyValue);
}

#[test]
fn numbers_are_picked_like_text_and_a_year_of_zero_is_no_year() {
    let best = with(file(1), |f| f.tags.year = Some(2019));
    let other = with(file(2), |f| f.tags.year = Some(2020));
    let zero = with(file(3), |f| f.tags.year = Some(0));
    let picks = pick(&[best, other, zero]);
    let year = field(&picks, TagField::Year);
    assert_eq!(year.value, FieldValue::Number { number: 2019 });
    assert_eq!(year.reason, PickReason::FromBestFile);
    assert_eq!(
        year.disagreements,
        vec![Disagreement {
            file_id: 2,
            value: FieldValue::Number { number: 2020 }
        }]
    );
}

#[test]
fn every_tag_field_is_picked_independently_each_from_its_own_best_file() {
    let best = with(file(1), |f| f.tags.title = Some("Velo".into()));
    let second = with(file(2), |f| {
        f.quality.verdict = Some(Verdict::LowBitrate);
        f.tags.title = Some("Velo (rip)".into());
        f.tags.label = Some("Kestrel Audio".into());
        f.tags.isrc = Some("QZABC2100001".into());
    });
    let picks = pick(&[best, second]);
    assert_eq!(field(&picks, TagField::Title).from_file, 1);
    assert_eq!(field(&picks, TagField::Label).from_file, 2);
    assert_eq!(field(&picks, TagField::Isrc).from_file, 2);
}

#[test]
fn bpm_key_and_energy_are_not_picked() {
    // Analysis is their source of truth (ROADMAP §2).
    let names: Vec<String> = TagField::ALL
        .iter()
        .map(|f| {
            serde_json::to_value(f)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    for name in ["bpm", "key", "energy"] {
        assert!(!names.iter().any(|n| n == name), "{name}");
    }
    assert_eq!(names.len(), 13);
}

#[test]
fn fields_come_back_in_the_order_of_the_field_list() {
    let all = with(file(1), |f| {
        f.tags = TagFields {
            title: Some("Velo".into()),
            artist: Some("Halden Rook".into()),
            album: Some("Nightdrive".into()),
            album_artist: Some("Halden Rook".into()),
            genre: Some("House".into()),
            comment: Some("n".into()),
            composer: Some("A. Fenn".into()),
            remixer: Some("Dub Kestrel".into()),
            label: Some("Kestrel Audio".into()),
            isrc: Some("QZABC2100001".into()),
            year: Some(2019),
            track_no: Some(2),
            disc_no: Some(1),
        }
    });
    let picks = pick(&[all]);
    let listed: Vec<TagField> = picks.fields.iter().map(|p| p.field).collect();
    assert_eq!(listed, TagField::ALL);
}

#[test]
fn a_pick_serializes_with_camel_case_names() {
    let picks = pick(&[titled(1, "Velo"), titled(2, "Velo II")]);
    let json = serde_json::to_value(&picks).unwrap();
    assert_eq!(
        json["fields"][0],
        serde_json::json!({
            "field": "title",
            "value": { "type": "text", "text": "Velo" },
            "fromFile": 1,
            "reason": "fromBestFile",
            "disagreements": [
                { "fileId": 2, "value": { "type": "text", "text": "Velo II" } }
            ],
        })
    );
}
