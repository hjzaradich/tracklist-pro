//! The tag filter (3A-T): the tracks that have a tag, by its id, which is a
//! bound value and never part of the SQL text.

use super::*;
use crate::vocab::{apply_on, create_group_on, create_tag_on};

fn rows(lib: &Lib, source: TableSource, id: i64) -> Vec<i64> {
    let page = run(lib, filtered(source, vec![TableFilter::Tag { id }]));
    let mut found = ids(&page);
    found.sort_unstable();
    assert_eq!(
        page.total.expect("the first window counts") as usize,
        found.len()
    );
    found
}

#[test]
fn the_filter_lists_the_tracks_that_have_the_tag_and_narrows_with_the_others() {
    let lib = Lib::new();
    let mut recordings = Vec::new();
    for (i, name) in ["a.mp3", "b.mp3", "c.mp3"].into_iter().enumerate() {
        let (recording, file) = track(&lib, &format!("T{i}"), "A", name);
        if i < 2 {
            lib.library(recording, file);
        }
        recordings.push(recording);
    }
    let (tag, other) = lib
        .writer
        .call(|c| {
            let group = create_group_on(c, "Vibe").unwrap().unwrap();
            let tag = create_tag_on(c, group, "Warm").unwrap().unwrap();
            let other = create_tag_on(c, group, "Quiet").unwrap().unwrap();
            Ok((tag, other))
        })
        .unwrap();
    let on = [recordings[0], recordings[2]];
    lib.writer
        .call(move |c| Ok(apply_on(c, tag, &on).unwrap().unwrap()))
        .unwrap();

    assert_eq!(
        rows(&lib, TableSource::AllMusic, tag.0),
        [recordings[0], recordings[2]]
    );
    assert_eq!(rows(&lib, TableSource::Library, tag.0), [recordings[0]]);
    // A tag nothing has, and one that doesn't exist, list nothing.
    assert!(rows(&lib, TableSource::AllMusic, other.0).is_empty());
    assert!(rows(&lib, TableSource::AllMusic, 9999).is_empty());
    // With another filter, both apply.
    let both = run(
        &lib,
        filtered(
            TableSource::AllMusic,
            vec![TableFilter::Tag { id: tag.0 }, TableFilter::InLibrary],
        ),
    );
    assert_eq!(ids(&both), [recordings[0]]);
}

#[test]
fn the_tag_id_is_bound_not_written_into_the_sql() {
    let mut params = Vec::new();
    let sql = filters::conditions(&[TableFilter::Tag { id: 7_654_321 }], None, &mut params);
    assert_eq!(sql.len(), 1);
    assert!(!sql[0].contains("7654321"), "{}", sql[0]);
    assert_eq!(params, [Value::Integer(7_654_321)]);
}
