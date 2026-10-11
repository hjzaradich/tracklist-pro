//! What holds for every decision, and how a decision is serialized.

use serde_json::json;

use super::*;

/// Files whose names relate in every way the table tells apart.
fn every_file() -> Vec<FileFacts> {
    vec![
        titled("Paper Harbor"),
        by("Kestrel Nine", "Paper Harbor"),
        by("The Marrow Choir", "Paper Harbor"),
        titled("Paper Harbor (Extended Mix)"),
        titled("Paper Harbor (Clean)"),
        titled("Paper Harbor (Dirty)"),
        by("Kestrel Nine", "Paper Harbor (Quill Ashby Remix)"),
        by("Quill Ashby", "Paper Harbor (Quill Ashby Remix)"),
        titled("Paper Harbor (Remix)"),
        titled("Paper Harbor (Dub)"),
        titled("Paper Harbor (Skyline Session)"),
        titled("Paper Harbor Extended Mix"),
        titled("Paper Harbor x Neon Moth (Mashup)"),
        titled("Paper Harbor x Korvex (Mashup)"),
        titled("Neon Moth"),
        titled(""),
        named("Kestrel Nine - Paper Harbor (Dirty).mp3"),
        named("Paper Harbor.flac"),
        tagged_and_named("Paper Harbor", "Kestrel Nine - Paper Harbor (Clean).mp3"),
        tagged_and_named("Paper Harbor", "Paper Harbor (Quill Ashby Remix).mp3"),
        tagged_and_named("Neon Moth", "Paper Harbor (Skyline Session).mp3"),
    ]
}

/// Every decision over [`every_file`] and [`every_audio`], with the audio's
/// name. [`decided`] checks each one both ways round.
fn every_decision() -> Vec<(&'static str, Decision)> {
    let files = every_file();
    let mut decisions = Vec::new();
    for a in &files {
        for b in &files {
            for (what, audio) in every_audio() {
                decisions.push((what, decided(audio, a, b)));
            }
        }
    }
    decisions
}

/// The outcome each deciding reason always comes with.
fn outcome_of(code: &str) -> &'static str {
    match code {
        "identicalAudio" | "sameAudio" => "duplicate",
        "versionVeto" | "audioInsideOther" | "namesSayCut" => "cutLink",
        "namesSayRework" | "mashupContains" => "reworkLink",
        "sameAudioNamesDisagree"
        | "audioInsideOtherNamesDisagree"
        | "sameTagsPartialAudio"
        | "partialAudioNamesUnclear"
        | "notCompared"
        | "noFingerprint" => "needsReview",
        "fingerprintVeto"
        | "sameTitleDifferentArtists"
        | "mashupPart"
        | "namesUnclear"
        | "titlesDiffer" => "different",
        other => panic!("{other} is not a deciding reason"),
    }
}

fn kind(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Duplicate => "duplicate",
        Outcome::CutLink { .. } => "cutLink",
        Outcome::ReworkLink { .. } => "reworkLink",
        Outcome::Different => "different",
        Outcome::NeedsReview => "needsReview",
    }
}

#[test]
fn every_decision_has_a_deciding_reason_first_and_it_always_comes_with_the_same_outcome() {
    for (what, decision) in every_decision() {
        let deciding = decision.reasons[0].code();
        assert_eq!(
            outcome_of(deciding),
            kind(&decision.outcome),
            "{what}: {decision:?}"
        );
    }
}

#[test]
fn every_outcome_and_every_deciding_reason_is_reached() {
    let decisions = every_decision();
    for outcome in [
        "duplicate",
        "cutLink",
        "reworkLink",
        "different",
        "needsReview",
    ] {
        assert!(
            decisions.iter().any(|(_, d)| kind(&d.outcome) == outcome),
            "{outcome}"
        );
    }
    for code in [
        "identicalAudio",
        "sameAudio",
        "versionVeto",
        "sameAudioNamesDisagree",
        "audioInsideOther",
        "audioInsideOtherNamesDisagree",
        "sameTagsPartialAudio",
        "partialAudioNamesUnclear",
        "namesSayCut",
        "namesSayRework",
        "mashupContains",
        "fingerprintVeto",
        "notCompared",
        "noFingerprint",
        "sameTitleDifferentArtists",
        "mashupPart",
        "namesUnclear",
        "titlesDiffer",
    ] {
        assert!(
            decisions.iter().any(|(_, d)| d.reasons[0].code() == code),
            "{code}"
        );
    }
}

#[test]
fn nothing_is_a_duplicate_without_identical_or_full_audio() {
    for (what, decision) in every_decision() {
        if decision.outcome == Outcome::Duplicate {
            assert!(matches!(what, "identical" | "full"), "{what}: {decision:?}");
        }
    }
}

#[test]
fn versions_never_merge_names_that_give_two_versions_are_a_duplicate_only_on_identical_audio() {
    let versions = [
        ("Paper Harbor (Clean)", "Paper Harbor (Dirty)"),
        ("Paper Harbor (Extended Mix)", "Paper Harbor"),
        ("Paper Harbor (Quill Ashby Remix)", "Paper Harbor"),
        ("Paper Harbor (Dub)", "Paper Harbor (Instrumental)"),
    ];
    for (a, b) in versions {
        for (what, audio) in every_audio() {
            let got = decided(audio, &titled(a), &titled(b));
            assert_eq!(
                got.outcome == Outcome::Duplicate,
                what == "identical",
                "{a} / {b}, {what}"
            );
        }
    }
}

#[test]
fn a_links_labels_are_of_its_own_class_sorted_and_a_rework_link_always_has_one() {
    use crate::versions::VersionClass;
    for (what, decision) in every_decision() {
        let (labels, class) = match &decision.outcome {
            Outcome::CutLink { labels } => (labels, VersionClass::Cut),
            Outcome::ReworkLink { labels } => {
                assert!(!labels.is_empty(), "{what}: {decision:?}");
                (labels, VersionClass::Rework)
            }
            _ => continue,
        };
        assert!(
            labels.iter().all(|label| label.class() == class),
            "{what}: {decision:?}"
        );
        assert!(
            labels.windows(2).all(|pair| pair[0] < pair[1]),
            "{what}: {decision:?}"
        );
    }
}

#[test]
fn the_same_pair_gets_the_same_decision_every_time() {
    assert_eq!(every_decision(), every_decision());
}

#[test]
fn a_reasons_code_is_what_it_serializes_as_in_camel_case() {
    let mut seen = std::collections::BTreeSet::new();
    for (_, decision) in every_decision() {
        for reason in decision.reasons {
            let code = reason.code();
            let json = serde_json::to_value(&reason).expect("a reason serializes");
            assert_eq!(json["code"], code);
            assert!(code.chars().all(|c| c.is_ascii_alphabetic()), "{code}");
            assert!(code.starts_with(|c: char| c.is_ascii_lowercase()), "{code}");
            seen.insert(code);
        }
    }
    // Every reason there is came by.
    assert_eq!(seen.len(), 21);
}

#[test]
fn a_reasons_parameters_serialize_in_camel_case_with_kinds_as_their_names() {
    let veto = Reason::VersionVeto {
        only_a: vec![MarkerKind::Clean],
        only_b: vec![MarkerKind::RadioEdit, MarkerKind::Dirty],
    };
    assert_eq!(
        serde_json::to_value(&veto).unwrap(),
        json!({ "code": "versionVeto", "onlyA": ["clean"], "onlyB": ["radio_edit", "dirty"] })
    );
    let rework = Reason::NamesSayRework {
        only_a: vec![Rework {
            kind: MarkerKind::Remix,
            whose: Some("Quill Ashby".to_string()),
        }],
        only_b: vec![Rework {
            kind: MarkerKind::Vip,
            whose: None,
        }],
    };
    assert_eq!(
        serde_json::to_value(&rework).unwrap(),
        json!({
            "code": "namesSayRework",
            "onlyA": [{ "kind": "remix", "whose": "Quill Ashby" }],
            "onlyB": [{ "kind": "vip", "whose": null }],
        })
    );
    let inside = Reason::AudioInsideOther {
        inner: Side::B,
        found_a: 0.5,
        found_b: 1.0,
    };
    assert_eq!(
        serde_json::to_value(&inside).unwrap(),
        json!({ "code": "audioInsideOther", "inner": "b", "foundA": 0.5, "foundB": 1.0 })
    );
    let same = Reason::SameAudio {
        coverage_a: 1.0,
        coverage_b: 0.5,
        score: 1.5,
    };
    assert_eq!(
        serde_json::to_value(&same).unwrap(),
        json!({ "code": "sameAudio", "coverageA": 1.0, "coverageB": 0.5, "score": 1.5 })
    );
    let mashup = Reason::MashupContains {
        mashup: Side::A,
        part: "Duskline".to_string(),
    };
    assert_eq!(
        serde_json::to_value(&mashup).unwrap(),
        json!({ "code": "mashupContains", "mashup": "a", "part": "Duskline" })
    );
    let unclear = Reason::NamesUnclear {
        doubts: vec!["unrecognized_differs"],
    };
    assert_eq!(
        serde_json::to_value(&unclear).unwrap(),
        json!({ "code": "namesUnclear", "doubts": ["unrecognized_differs"] })
    );
}

#[test]
fn a_decision_serializes_its_outcome_by_kind_with_a_links_labels() {
    let decision = decided(full(), &titled("Velo (Clean)"), &titled("Velo (Dirty)"));
    assert_eq!(
        serde_json::to_value(&decision).unwrap(),
        json!({
            "outcome": { "kind": "cutLink", "labels": ["clean", "dirty"] },
            "reasons": [{ "code": "versionVeto", "onlyA": ["clean"], "onlyB": ["dirty"] }],
            "reading": "tags",
        })
    );
    for (outcome, kind) in [
        (Outcome::Duplicate, "duplicate"),
        (Outcome::Different, "different"),
        (Outcome::NeedsReview, "needsReview"),
    ] {
        assert_eq!(
            serde_json::to_value(&outcome).unwrap(),
            json!({ "kind": kind })
        );
    }
    assert_eq!(
        serde_json::to_value(rework_link(&[MarkerKind::Remix, MarkerKind::Dub])).unwrap(),
        json!({ "kind": "reworkLink", "labels": ["remix", "dub"] })
    );
}

#[test]
fn swapping_a_reason_swaps_its_sides_and_swapping_back_restores_it() {
    for (_, decision) in every_decision() {
        assert_eq!(decision.clone().swapped().swapped(), decision);
    }
    assert_eq!(
        Reason::CreditedToRemixer {
            sides: vec![Side::A],
        }
        .swapped(),
        Reason::CreditedToRemixer {
            sides: vec![Side::B],
        }
    );
    assert_eq!(
        Reason::CreditedToRemixer {
            sides: vec![Side::A, Side::B],
        }
        .swapped(),
        Reason::CreditedToRemixer {
            sides: vec![Side::A, Side::B],
        }
    );
    assert_eq!(
        Reason::NamesSayCut {
            only_a: vec![MarkerKind::Clean],
            only_b: vec![],
        }
        .swapped(),
        Reason::NamesSayCut {
            only_a: vec![],
            only_b: vec![MarkerKind::Clean],
        }
    );
    assert_eq!(Side::A.other(), Side::B);
    assert_eq!(Side::B.other(), Side::A);
}
