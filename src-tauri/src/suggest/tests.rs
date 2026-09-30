use super::*;
use serde_json::json;

fn fact(key: &str) -> Reason {
    Reason::fact(key).unwrap()
}

fn audio(key: &str) -> Reason {
    Reason::audio_model(key).unwrap()
}

// --- A suggestion needs a reason ---------------------------------------

#[test]
fn a_suggestion_without_a_reason_is_refused() {
    assert_eq!(
        Suggestion::new("track 12", vec![]),
        Err(SuggestError::NoReason)
    );
}

#[test]
fn a_suggestion_with_a_fact_reason_is_made() {
    let s = Suggestion::new("track 12", vec![fact("crates:reason.sameKey")]).unwrap();
    assert_eq!(s.what(), &"track 12");
    assert_eq!(s.reasons().len(), 1);
}

#[test]
fn a_reasonless_suggestion_is_refused_when_deserialized() {
    let err =
        serde_json::from_value::<Suggestion<i32>>(json!({ "what": 1, "reasons": [] })).unwrap_err();
    assert!(err.to_string().contains("at least one reason"), "{err}");
}

#[test]
fn a_suggestion_missing_its_reasons_field_is_refused_when_deserialized() {
    assert!(serde_json::from_value::<Suggestion<i32>>(json!({ "what": 1 })).is_err());
}

#[test]
fn the_check_run_at_serialization_refuses_an_empty_reason_list() {
    // `Serialize` runs this same check; a suggestion made by `new` can't be
    // empty, so the check itself is what's tested here.
    assert_eq!(check_reasons(&[]), Err(SuggestError::NoReason));
}

#[test]
fn a_serialized_suggestion_always_has_its_reasons() {
    let s = Suggestion::new(7, vec![fact("crates:reason.sameKey")]).unwrap();
    let value = serde_json::to_value(&s).unwrap();
    assert_eq!(
        value,
        json!({
            "what": 7,
            "reasons": [{ "key": "crates:reason.sameKey", "params": {}, "source": "fact" }],
        })
    );
}

// --- The audio model is never the only reason --------------------------

#[test]
fn a_suggestion_whose_only_reason_is_the_audio_model_is_refused() {
    assert_eq!(
        Suggestion::new(1, vec![audio("crates:reason.soundsDark")]),
        Err(SuggestError::OnlyAudioModel)
    );
    assert_eq!(
        Suggestion::new(
            1,
            vec![
                audio("crates:reason.soundsDark"),
                audio("crates:reason.soundsFast")
            ]
        ),
        Err(SuggestError::OnlyAudioModel)
    );
}

#[test]
fn the_audio_model_can_add_to_a_fact() {
    let s = Suggestion::new(
        1,
        vec![
            audio("crates:reason.soundsDark"),
            fact("crates:reason.sameKey"),
        ],
    )
    .unwrap();
    assert_eq!(s.reasons().len(), 2);
}

#[test]
fn an_audio_model_only_suggestion_is_refused_when_deserialized() {
    let err = serde_json::from_value::<Suggestion<i32>>(json!({
        "what": 1,
        "reasons": [{ "key": "crates:reason.soundsDark", "params": {}, "source": "audioModel" }],
    }))
    .unwrap_err();
    assert!(err.to_string().contains("only reason"), "{err}");
}

#[test]
fn an_audio_model_reason_is_marked_as_such_when_serialized() {
    let s = Suggestion::new(
        1,
        vec![
            fact("crates:reason.sameKey"),
            audio("crates:reason.soundsDark"),
        ],
    )
    .unwrap();
    let value = serde_json::to_value(&s).unwrap();
    assert_eq!(value["reasons"][0]["source"], "fact");
    assert_eq!(value["reasons"][1]["source"], "audioModel");
}

#[test]
fn switching_the_audio_model_off_leaves_a_suggestion_with_a_reason() {
    let s = Suggestion::new(
        1,
        vec![
            audio("crates:reason.soundsDark"),
            fact("crates:reason.sameKey"),
            audio("crates:reason.soundsFast"),
        ],
    )
    .unwrap()
    .without_audio_model();
    assert_eq!(s.reasons().len(), 1);
    assert_eq!(s.reasons()[0].key(), "crates:reason.sameKey");
    assert_eq!(check_reasons(s.reasons()), Ok(()));
}

// --- Reasons are translatable: a key plus parameters, not English ------

#[test]
fn english_text_is_refused_as_a_reason_key() {
    for text in [
        "same key, +2 BPM, tagged Peak Time",
        "Same key",
        "crates:same key",
        "crates:reason.same key",
    ] {
        assert_eq!(
            Reason::fact(text),
            Err(SuggestError::BadKey(text.to_owned())),
            "{text:?}"
        );
    }
}

#[test]
fn a_key_needs_a_namespace_and_a_path() {
    for bad in [
        "",
        "sameKey",
        ":sameKey",
        "crates:",
        "crates:reason.",
        "crates:.sameKey",
        "crates:reason..sameKey",
        "crates:1st",
        "1crates:sameKey",
        "crates:reason:sameKey",
    ] {
        assert!(Reason::fact(bad).is_err(), "{bad:?} was accepted");
    }
    for good in [
        "crates:sameKey",
        "crates:reason.sameKey",
        "all-music:reason.best_file.higherBitrate",
    ] {
        assert!(Reason::fact(good).is_ok(), "{good:?} was refused");
    }
}

#[test]
fn a_bad_key_is_refused_when_deserialized() {
    let err = serde_json::from_value::<Reason>(json!({
        "key": "same key, +2 BPM",
        "params": {},
        "source": "fact",
    }))
    .unwrap_err();
    assert!(err.to_string().contains("is not an i18n key"), "{err}");
}

#[test]
fn parameters_are_typed_values_not_formatted_text() {
    let reason = fact("crates:reason.nearBpm")
        .with_number("bpm", 2.0)
        .unwrap()
        .with_text("tag", "Peak Time")
        .unwrap();
    assert_eq!(
        serde_json::to_value(&reason).unwrap(),
        json!({
            "key": "crates:reason.nearBpm",
            "params": { "bpm": 2.0, "tag": "Peak Time" },
            "source": "fact",
        })
    );
    let back: Reason = serde_json::from_value(serde_json::to_value(&reason).unwrap()).unwrap();
    assert_eq!(back, reason);
}

#[test]
fn a_parameter_name_must_be_an_identifier() {
    for bad in ["", "two words", "bpm!", "{{bpm}}", "1bpm"] {
        assert_eq!(
            fact("crates:reason.nearBpm").with_number(bad, 2.0),
            Err(SuggestError::BadParamName(bad.to_owned())),
            "{bad:?}"
        );
    }
}

#[test]
fn a_number_parameter_must_be_finite() {
    for n in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            fact("crates:reason.nearBpm").with_number("bpm", n),
            Err(SuggestError::NotFinite("bpm".to_owned()))
        );
    }
}

#[test]
fn a_bad_parameter_name_is_refused_when_deserialized() {
    let result = serde_json::from_value::<Reason>(json!({
        "key": "crates:reason.nearBpm",
        "params": { "two words": 2 },
        "source": "fact",
    }));
    assert!(result.is_err());
}

#[test]
fn an_unknown_reason_source_is_refused() {
    let result = serde_json::from_value::<Reason>(json!({
        "key": "crates:reason.sameKey",
        "source": "vibes",
    }));
    assert!(result.is_err());
}

// --- The frontend gets the same shape ----------------------------------

#[test]
fn bindings_declare_the_suggestion_and_reason_types_for_the_frontend() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bindings.ts");
    crate::ipc::export_bindings(&path).unwrap();
    let ts = std::fs::read_to_string(&path).unwrap();
    for expected in [
        "export type Suggestion<T> = {",
        "reasons: Reason[],",
        "export type Reason = {",
        "source: ReasonSource,",
        "\"audioModel\"",
    ] {
        assert!(ts.contains(expected), "missing `{expected}` in:\n{ts}");
    }
}
