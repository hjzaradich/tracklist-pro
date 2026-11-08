//! Identification offline, and "Use" (ROADMAP 2.2; 6A-S1). Every name and
//! value is invented. No test goes online: nothing here can.

use super::*;
use crate::conflicts::app_values;
use crate::ops::{self, UndoOutcome};
use crate::search::tests::support::{Lib, Mount};

/// A Library track with its linked file `name`.
struct Track {
    lib: Lib,
    recording: i64,
    library: LibraryTrackId,
    file: i64,
}

fn track(name: &str) -> Track {
    let lib = Lib::new();
    let (recording, file) = lib.track(None, None, name);
    let library = LibraryTrackId(lib.library(recording, file));
    Track {
        lib,
        recording,
        library,
        file,
    }
}

/// `raw_tags` with these ID3v2 text frames.
fn frames(items: &[(&str, &str)]) -> String {
    let items: Vec<_> = items
        .iter()
        .map(|(key, text)| serde_json::json!({"key": key, "value": {"type": "text", "text": text}}))
        .collect();
    serde_json::json!({ "id3v2": items }).to_string()
}

impl Track {
    fn raw_tags(&self, file: i64, raw: String) {
        self.lib
            .insert("UPDATE file SET raw_tags = ?1 WHERE id = ?2", (raw, file));
    }

    fn sibling(&self, name: &str) -> i64 {
        self.lib.file(self.recording, name, "extra")
    }

    /// rekordbox has the track, with no attribute but its genre.
    fn rekordbox_has_it(&self) {
        self.lib.trusted_entry(self.file, "House");
    }

    fn suggestions_with(&self, extra: Vec<Candidate>) -> IdentifySuggestions {
        let library = self.library;
        self.lib
            .writer
            .call(move |c| suggestions_with(c, &Mount, library, extra))
            .unwrap()
            .expect("a Library track")
    }

    fn suggestions(&self) -> IdentifySuggestions {
        self.suggestions_with(Vec::new())
    }

    /// The field's current value and its suggestions as (value, sources).
    fn field(&self, field: IdentifyField) -> (Option<String>, Vec<(String, Vec<IdentifySource>)>) {
        let all = self.suggestions();
        let f = if field == IdentifyField::CatalogueNumber {
            all.catalogue_number
        } else {
            all.fields.into_iter().find(|f| f.field == field).unwrap()
        };
        (
            f.current,
            f.suggestions
                .into_iter()
                .map(|s| (s.value, s.sources))
                .collect(),
        )
    }

    fn values(&self, field: IdentifyField) -> Vec<String> {
        self.field(field).1.into_iter().map(|(v, _)| v).collect()
    }

    fn named(&self, id: i64, name: &str) -> NamedFile {
        NamedFile {
            id,
            name: name.to_owned(),
        }
    }

    fn use_these(&self, choices: &[(&str, &str)]) -> Result<Option<i64>, UseError> {
        let choices: Vec<UseChoice> = choices
            .iter()
            .map(|(field, value)| UseChoice {
                field: (*field).to_owned(),
                value: (*value).to_owned(),
            })
            .collect();
        let library = self.library;
        self.lib
            .writer
            .call(move |c| Ok(use_on(c, &Mount, library, &choices)))
            .unwrap()
    }

    fn undo(&self) -> UndoOutcome {
        self.lib
            .writer
            .call(|c| Ok(ops::undo_for_user(c, None)))
            .unwrap()
            .unwrap()
    }

    fn own(&self, field: &'static str) -> Option<String> {
        let library = self.library.0;
        self.lib
            .writer
            .call(move |c| {
                c.prepare("SELECT value FROM app_value WHERE library_track_id = ?1 AND field = ?2")?
                    .query_map((library, field), |r| r.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()
            })
            .unwrap()
            .pop()
    }

    fn catalogue(&self) -> Option<String> {
        let library = self.library;
        self.lib
            .writer
            .call(move |c| catalogue_number(c, library))
            .unwrap()
    }

    fn operations(&self) -> Vec<String> {
        self.lib
            .writer
            .call(|c| {
                c.prepare("SELECT kind FROM operation ORDER BY id")?
                    .query_map([], |r| r.get(0))?
                    .collect()
            })
            .unwrap()
    }

    /// What the next send writes for the track: the merge before a send,
    /// then the send's input built from it.
    fn sent(&self) -> Vec<(String, String)> {
        let library = self.library;
        self.lib
            .writer
            .call(move |c| {
                let merged = crate::conflicts::merge_before_send(c, &Mount, &[library])?;
                let input = crate::rekordbox_write::gather_merged(c, merged, vec![], vec![])?;
                Ok(input)
            })
            .unwrap()
            .tracks
            .into_iter()
            .find(|t| t.library_track == library)
            .and_then(|t| match t.values {
                crate::rekordbox_write::Values::Ready { attributes, .. } => Some(attributes),
                _ => None,
            })
            .expect("the track is sent")
    }
}

fn get<'a>(attributes: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

// ---- Suggestions ---------------------------------------------------------------

#[test]
fn a_file_name_alone_suggests_title_artist_mix_and_remixer_naming_the_file() {
    let t = track("Halden Rook - Velo (Kit Remix).mp3");
    let file = IdentifySource::FileName {
        file: t.named(t.file, "Halden Rook - Velo (Kit Remix).mp3"),
    };
    for (field, value) in [
        (IdentifyField::Name, "Velo"),
        (IdentifyField::Artist, "Halden Rook"),
        (IdentifyField::Mix, "Kit Remix"),
        (IdentifyField::Remixer, "Kit"),
    ] {
        assert_eq!(
            t.field(field),
            (None, vec![(value.to_owned(), vec![file.clone()])]),
            "{field:?}"
        );
    }
}

#[test]
fn the_linked_files_tags_suggest_values_and_the_version_their_title_names() {
    let t = track("a.mp3");
    t.raw_tags(
        t.file,
        frames(&[
            ("TIT2", "Velo (Extended Mix)"),
            ("TPUB", "Rook Records"),
            ("TALB", "Night Drive"),
            ("TDRC", "2019-05-01"),
        ]),
    );
    t.rekordbox_has_it();
    let tags = vec![IdentifySource::Tags {
        file: t.named(t.file, "a.mp3"),
    }];
    let (current, label) = t.field(IdentifyField::Label);
    assert_eq!(current, None, "rekordbox's entry has no label");
    assert_eq!(label, vec![("Rook Records".to_owned(), tags.clone())]);
    assert_eq!(t.field(IdentifyField::Mix).1, vec![("Extended Mix".to_owned(), tags.clone())]);
    assert_eq!(t.field(IdentifyField::Album).1, vec![("Night Drive".to_owned(), tags.clone())]);
    assert_eq!(t.field(IdentifyField::Year).1, vec![("2019".to_owned(), tags.clone())]);
    // The tag's title comes before the file name's reading of "a.mp3".
    assert_eq!(t.values(IdentifyField::Name), ["Velo (Extended Mix)", "a"]);
}

#[test]
fn a_sibling_file_suggests_the_value_it_holds_when_it_differs_from_the_linked_files() {
    let t = track("a.mp3");
    t.lib.tag(t.file, &[("Label", "Rook Records")]);
    let other = t.sibling("b.flac");
    t.lib.tag(other, &[("Label", "Kit Music")]);

    let (current, label) = t.field(IdentifyField::Label);
    assert_eq!(current.as_deref(), Some("Rook Records"), "the send carries the linked file's");
    assert_eq!(
        label,
        vec![(
            "Kit Music".to_owned(),
            vec![IdentifySource::Sibling {
                file: t.named(other, "b.flac")
            }]
        )]
    );
}

#[test]
fn the_same_value_from_two_sources_is_one_suggestion_naming_both_in_its_best_spelling() {
    let t = track("HALDEN ROOK - Velo.mp3");
    t.lib.tag(t.file, &[("Artist", "Halden  Rook ")]);
    t.rekordbox_has_it();

    let (_, artist) = t.field(IdentifyField::Artist);
    assert_eq!(
        artist,
        vec![(
            "Halden Rook".to_owned(),
            vec![
                IdentifySource::Tags {
                    file: t.named(t.file, "HALDEN ROOK - Velo.mp3")
                },
                IdentifySource::FileName {
                    file: t.named(t.file, "HALDEN ROOK - Velo.mp3")
                },
            ]
        )]
    );
}

#[test]
fn a_value_with_more_sources_ranks_first_then_the_stronger_source() {
    let t = track("Kit - Velo.mp3");
    t.lib.tag(t.file, &[("Artist", "Halden Rook")]);
    let other = t.sibling("Kit - Velo.flac");
    t.lib.tag(other, &[("Artist", "Kit")]);
    t.rekordbox_has_it();
    // "Kit": the sibling's tags and both names; "Halden Rook": one tag.
    assert_eq!(t.values(IdentifyField::Artist), ["Kit", "Halden Rook"]);

    let u = track("x.mp3");
    u.lib.tag(u.file, &[("Label", "Rook Records")]);
    let other = u.sibling("y.mp3");
    u.lib.tag(other, &[("Label", "Kit Music")]);
    u.rekordbox_has_it();
    // One source each: the linked file's tags beat a sibling's.
    assert_eq!(u.values(IdentifyField::Label), ["Rook Records", "Kit Music"]);
    // The same order every time.
    assert_eq!(u.suggestions(), u.suggestions());
}

#[test]
fn a_value_equal_to_the_current_one_is_not_suggested_whatever_its_case_or_spacing() {
    let t = track("Kit - VELO.mp3");
    t.lib.tag(t.file, &[("Name", "Velo")]);
    let (current, names) = t.field(IdentifyField::Name);
    assert_eq!(current.as_deref(), Some("Velo"));
    assert!(names.is_empty(), "{names:?}");
    assert_eq!(t.values(IdentifyField::Artist), ["Kit"]);

    // The app's own value is the current one when it has one.
    t.lib
        .writer
        .call({
            let library = t.library;
            move |c| Ok(app_values::set(c, &Mount, library, "Artist", "kit"))
        })
        .unwrap()
        .unwrap();
    assert_eq!(t.field(IdentifyField::Artist), (Some("kit".to_owned()), vec![]));
}

#[test]
fn a_file_whose_tags_cant_be_read_gives_nothing_and_the_others_still_count() {
    let t = track("Kit - Velo.mp3");
    t.raw_tags(t.file, r#"{"id3v2": 7, "vorbis_comments": [{"key": 3}]}"#.to_owned());
    let other = t.sibling("b.flac");
    t.raw_tags(other, r#"{"id3v2": [{"key": "TPUB"}]}"#.to_owned());
    let third = t.sibling("c.flac");
    t.lib.tag(third, &[("Album", "Night Drive")]);
    t.rekordbox_has_it();

    assert_eq!(t.values(IdentifyField::Artist), ["Kit"]);
    assert_eq!(t.values(IdentifyField::Album), ["Night Drive"]);
}

#[test]
fn a_track_with_nothing_to_suggest_answers_empty_lists() {
    let t = track("Velo.mp3");
    t.lib.tag(t.file, &[("Name", "Velo")]);
    let all = t.suggestions();
    assert_eq!(all.fields.len(), 7);
    assert!(all.fields.iter().all(|f| f.suggestions.is_empty()), "{all:?}");
    assert!(all.catalogue_number.suggestions.is_empty());
    assert_eq!(all.catalogue_number.current, None);
}

#[test]
fn a_track_that_is_not_in_the_library_has_no_suggestions() {
    let t = track("Velo.mp3");
    let none = t
        .lib
        .writer
        .call(|c| suggestions(c, &Mount, LibraryTrackId(999)))
        .unwrap();
    assert_eq!(none, None);
}

#[test]
fn a_catalogue_number_in_the_file_name_is_suggested_and_none_comes_from_tags() {
    let t = track("Kit - Velo [ROOK001].mp3");
    assert_eq!(
        t.field(IdentifyField::CatalogueNumber),
        (
            None,
            vec![(
                "ROOK001".to_owned(),
                vec![IdentifySource::FileName {
                    file: t.named(t.file, "Kit - Velo [ROOK001].mp3")
                }]
            )]
        )
    );
}

#[test]
fn online_candidates_merge_and_rank_with_the_offline_ones() {
    let t = track("Kit - Velo.mp3");
    let online = IdentifySource::Online {
        service: "musicbrainz".to_owned(),
        via: Some("acoustid".to_owned()),
    };
    let all = t.suggestions_with(vec![
        Candidate {
            field: IdentifyField::Artist,
            value: "kit".to_owned(),
            source: online.clone(),
        },
        Candidate {
            field: IdentifyField::Year,
            value: "not a year".to_owned(),
            source: online.clone(),
        },
    ]);
    let artist = &all.fields[1];
    assert_eq!(artist.field, IdentifyField::Artist);
    assert_eq!(artist.suggestions.len(), 1);
    assert_eq!(artist.suggestions[0].value, "Kit");
    assert_eq!(artist.suggestions[0].sources[1], online);
    assert!(all.fields[5].suggestions.is_empty(), "a value Use would refuse");
}

// ---- Use ------------------------------------------------------------------------

#[test]
fn use_of_two_fields_is_one_operation_and_one_undo_puts_both_back() {
    let t = track("Kit - Velo.mp3");
    t.lib
        .writer
        .call({
            let library = t.library;
            move |c| Ok(app_values::set(c, &Mount, library, "Name", "Old Title"))
        })
        .unwrap()
        .unwrap();
    let before = t.operations().len();

    let op = t
        .use_these(&[("Name", "Velo"), ("Label", " Rook  Records ")])
        .unwrap();

    assert!(op.is_some());
    assert_eq!(t.operations().len(), before + 1);
    assert_eq!(t.operations().last().unwrap(), USE_OPERATION);
    assert_eq!(t.own("Name").as_deref(), Some("Velo"));
    assert_eq!(t.own("Label").as_deref(), Some("Rook Records"));

    assert!(matches!(t.undo(), UndoOutcome::Undone { .. }));
    assert_eq!(t.own("Name").as_deref(), Some("Old Title"));
    assert_eq!(t.own("Label"), None);
}

#[test]
fn after_use_the_next_send_carries_the_new_values_and_never_the_catalogue_number() {
    let t = track("Kit - Velo.mp3");
    t.lib.tag(t.file, &[("Name", "Velo"), ("Artist", "Kit")]);
    t.use_these(&[
        ("Name", "Velo (Extended Mix)"),
        ("Mix", "Extended Mix"),
        ("Year", "2019"),
        ("CatalogueNumber", "ROOK001"),
    ])
    .unwrap();

    let sent = t.sent();
    assert_eq!(get(&sent, "Name"), Some("Velo (Extended Mix)"));
    assert_eq!(get(&sent, "Mix"), Some("Extended Mix"));
    assert_eq!(get(&sent, "Year"), Some("2019"));
    assert_eq!(get(&sent, "Artist"), Some("Kit"));
    assert!(
        sent.iter()
            .all(|(n, v)| !v.contains("ROOK001") && !n.to_lowercase().contains("catalog")),
        "{sent:?}"
    );
}

#[test]
fn the_catalogue_number_is_kept_shown_as_current_changed_by_use_and_put_back_by_undo() {
    let t = track("Kit - Velo [ROOK001].mp3");
    t.use_these(&[("CatalogueNumber", "rook001")]).unwrap();
    assert_eq!(t.catalogue().as_deref(), Some("rook001"));
    // Equal but for case: no longer suggested.
    assert_eq!(t.field(IdentifyField::CatalogueNumber), (Some("rook001".to_owned()), vec![]));

    t.use_these(&[("CatalogueNumber", "ROOK001")]).unwrap();
    assert_eq!(t.catalogue().as_deref(), Some("ROOK001"));
    t.undo();
    assert_eq!(t.catalogue().as_deref(), Some("rook001"));
    t.undo();
    assert_eq!(t.catalogue(), None);
}

#[test]
fn removing_a_library_track_with_a_catalogue_number_works_and_undo_brings_it_back() {
    let t = track("Velo.mp3");
    t.use_these(&[("CatalogueNumber", "ROOK001")]).unwrap();
    crate::library::remove(&t.lib.writer, t.library).unwrap();
    assert_eq!(t.catalogue(), None);
    t.undo();
    assert_eq!(t.catalogue().as_deref(), Some("ROOK001"));
}

#[test]
fn use_refuses_what_it_cant_set_and_changes_nothing() {
    let t = track("Velo.mp3");
    let refused: [(&[(&str, &str)], fn(&UseError) -> bool); 7] = [
        (&[("Name", "Velo"), ("Genre", "House")], |e| {
            matches!(e, UseError::UnknownField(f) if f == "Genre")
        }),
        (&[("ISRC", "GBABC1900001")], |e| matches!(e, UseError::UnknownField(_))),
        (&[("name", "Velo")], |e| matches!(e, UseError::UnknownField(_))),
        (&[("Label", "  ")], |e| {
            matches!(e, UseError::ValueEmpty(IdentifyField::Label))
        }),
        (&[("Year", "next year")], |e| {
            matches!(e, UseError::NotSendable(IdentifyField::Year))
        }),
        (&[("Name", "Velo\u{1}")], |e| {
            matches!(e, UseError::NotSendable(IdentifyField::Name))
        }),
        (&[("CatalogueNumber", "ROOK\u{1}")], |e| {
            matches!(e, UseError::NotSendable(IdentifyField::CatalogueNumber))
        }),
    ];
    for (choices, is) in refused {
        let e = t.use_these(choices).unwrap_err();
        assert!(is(&e), "{choices:?}: {e:?}");
    }
    let library = LibraryTrackId(999);
    let e = t
        .lib
        .writer
        .call(move |c| {
            Ok(use_on(
                c,
                &Mount,
                library,
                &[UseChoice {
                    field: "Name".into(),
                    value: "Velo".into(),
                }],
            ))
        })
        .unwrap()
        .unwrap_err();
    assert!(matches!(e, UseError::NoTrack(_)), "{e:?}");
    assert!(t.operations().is_empty());
    assert_eq!((t.own("Name"), t.catalogue()), (None, None));
}

#[test]
fn every_refusal_of_use_reaches_the_screen_as_its_own_kind() {
    let kind = |e: UseError| IpcError::from(e).kind();
    assert_eq!(
        kind(UseError::UnknownField("Genre".into())),
        ErrorKind::IdentifyFieldUnknown
    );
    assert_eq!(
        kind(UseError::ValueEmpty(IdentifyField::Name)),
        ErrorKind::IdentifyValueEmpty
    );
    assert_eq!(
        kind(UseError::NotSendable(IdentifyField::Name)),
        ErrorKind::IdentifyValueNotSendable
    );
    assert_eq!(
        kind(UseError::NoTrack(LibraryTrackId(1))),
        ErrorKind::LibraryTrackNotFound
    );
}
