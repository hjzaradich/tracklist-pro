//! Picking the cover art.

use super::*;

fn with_art(id: i64, art: Art) -> FileFacts {
    with(file(id), |f| f.art = Some(art))
}

fn sized(bytes: u32, width: u32, height: u32) -> Art {
    Art {
        bytes,
        width: Some(width),
        height: Some(height),
        hash: None,
    }
}

fn hashed(bytes: u32, hash: &[u8]) -> Art {
    Art {
        bytes,
        width: None,
        height: None,
        hash: Some(hash.to_vec()),
    }
}

#[test]
fn no_art_in_any_file_picks_none() {
    assert_eq!(pick(&[file(1), file(2)]).art, None);
}

#[test]
fn art_only_one_file_has_comes_from_that_file() {
    let picks = pick(&[file(1), with_art(2, picture(5_000))]);
    let art = picks.art.unwrap();
    assert_eq!(art.from_file, 2);
    assert_eq!(art.reason, ArtReason::OnlyArt);
    assert!(art.disagreements.is_empty());
}

#[test]
fn the_same_size_in_bytes_is_the_same_picture() {
    let picks = pick(&[with_art(1, picture(5_000)), with_art(2, picture(5_000))]);
    let art = picks.art.unwrap();
    assert_eq!(art.reason, ArtReason::FilesAgree);
    assert!(art.disagreements.is_empty());
    // The better-ranked file's.
    assert_eq!(art.from_file, 1);
}

#[test]
fn a_different_size_in_bytes_is_a_disagreement() {
    let picks = pick(&[with_art(1, picture(5_000)), with_art(2, picture(9_000))]);
    let art = picks.art.unwrap();
    assert_eq!(art.reason, ArtReason::Largest);
    assert_eq!(art.from_file, 2);
    assert_eq!(art.disagreements.len(), 1);
    assert_eq!(art.disagreements[0].file_id, 1);
    assert_eq!(art.disagreements[0].art.bytes, 5_000);
}

#[test]
fn the_picture_with_more_pixels_wins_even_from_a_worse_ranked_file_and_with_fewer_bytes() {
    let best_file = with_art(1, sized(900_000, 300, 300));
    let worse_file = with(with_art(2, sized(80_000, 1_200, 1_200)), |f| {
        f.quality.verdict = Some(Verdict::LowBitrate)
    });
    let picks = pick(&[best_file, worse_file]);
    let art = picks.art.unwrap();
    assert_eq!(art.from_file, 2);
    assert_eq!(art.reason, ArtReason::Largest);
}

#[test]
fn where_pixels_are_unknown_the_larger_size_in_bytes_wins() {
    let small = with_art(1, sized(10_000, 500, 500));
    // Pixels unknown on this one: counted as none, so it loses to a file
    // whose pixels are known.
    let unknown = with_art(2, picture(2_000_000));
    let picks = pick(&[small, unknown]);
    assert_eq!(picks.art.unwrap().from_file, 1);

    let a = with_art(1, picture(10_000));
    let b = with_art(2, picture(30_000));
    let picks = pick(&[a, b]);
    assert_eq!(picks.art.unwrap().from_file, 2);
}

#[test]
fn equal_pictures_in_size_go_to_the_better_ranked_file() {
    let low = with(with_art(1, sized(5_000, 600, 600)), |f| {
        f.quality.verdict = Some(Verdict::LowBitrate)
    });
    let high = with_art(2, sized(5_000, 600, 600));
    for files in [[low.clone(), high.clone()], [high, low]] {
        assert_eq!(pick(&files).art.unwrap().from_file, 2);
    }
}

#[test]
fn with_hashes_the_hash_decides_not_the_size() {
    // The same size, different pictures.
    let a = with_art(1, hashed(5_000, b"aaaa"));
    let b = with_art(2, hashed(5_000, b"bbbb"));
    let art = pick(&[a, b]).art.unwrap();
    assert_eq!(art.disagreements.len(), 1);

    // Different sizes, the same picture (a re-encode of the same hash can't
    // happen, but a caller's hash is the caller's word).
    let c = with_art(1, hashed(5_000, b"cccc"));
    let d = with_art(2, hashed(5_001, b"cccc"));
    let art = pick(&[c, d]).art.unwrap();
    assert_eq!(art.reason, ArtReason::FilesAgree);
    assert!(art.disagreements.is_empty());
}

#[test]
fn the_art_pick_does_not_depend_on_input_order() {
    let a = with_art(1, sized(5_000, 600, 600));
    let b = with(with_art(2, sized(7_000, 600, 600)), |f| {
        f.quality.verdict = Some(Verdict::SuspectTranscode)
    });
    let c = file(3);
    let forward = pick(&[a.clone(), b.clone(), c.clone()]);
    let backward = pick(&[c, b, a]);
    assert_eq!(forward, backward);
}

#[test]
fn art_serializes_without_the_hash() {
    let picks = pick(&[with_art(1, hashed(10, b"xx"))]);
    let json = serde_json::to_value(picks.art.unwrap()).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "fromFile": 1,
            "art": { "bytes": 10, "width": null, "height": null },
            "reason": "onlyArt",
            "disagreements": [],
        })
    );
}
