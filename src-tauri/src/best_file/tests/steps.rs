//! One test per step of the order. A step is proven by two files that are
//! equal at every step before it, one good and one bad at it, the good one
//! bad at *every* later step and the bad one good at every later step: the
//! good one is still ahead, and the reason names the step.

use super::*;

/// The order, written out here on purpose: a test that read it from the
/// code would pass for any order.
const ORDER: [Step; 11] = [
    Step::OnDisk,
    Step::Verdict,
    Step::Playable,
    Step::Cutoff,
    Step::DecodeErrors,
    Step::Format,
    Step::Tags,
    Step::Art,
    Step::Rekordbox,
    Step::Place,
    Step::Tied,
];

fn make_good(file: &mut FileFacts, step: Step) {
    match step {
        Step::OnDisk => file.on_disk = true,
        Step::Verdict => file.quality.verdict = Some(Verdict::Ok),
        Step::Playable => file.plays_in_dj_app = true,
        Step::Cutoff => file.quality.cutoff_hz = Some(20_000),
        Step::DecodeErrors => file.quality.decode_errors = 0,
        Step::Format => {
            file.format = Format {
                codec: Some(Codec::Flac),
                kbps: None,
            }
        }
        Step::Tags => file.tags = full_tags(),
        Step::Art => file.art = Some(picture(40_000)),
        Step::Rekordbox => file.rekordbox = RekordboxLink::Trusted,
        Step::Place => file.place.volume_online = true,
        Step::Tied => file.file_id = 1,
    }
}

fn make_bad(file: &mut FileFacts, step: Step) {
    match step {
        Step::OnDisk => file.on_disk = false,
        Step::Verdict => file.quality.verdict = Some(Verdict::LowBitrate),
        Step::Playable => file.plays_in_dj_app = false,
        Step::Cutoff => file.quality.cutoff_hz = Some(15_000),
        Step::DecodeErrors => file.quality.decode_errors = 4,
        Step::Format => {
            file.format = Format {
                codec: Some(Codec::Mp3),
                kbps: Some(128),
            }
        }
        Step::Tags => file.tags = TagFields::default(),
        Step::Art => file.art = None,
        Step::Rekordbox => file.rekordbox = RekordboxLink::Unknown,
        Step::Place => file.place.volume_online = false,
        Step::Tied => file.file_id = 2,
    }
}

/// `good` is good at `step` and bad at every later step; `bad` is the
/// reverse. They are equal before `step`.
fn pair_at(step: Step) -> (FileFacts, FileFacts) {
    let at = ORDER.iter().position(|s| *s == step).unwrap();
    // Ids: the tie-break must favor `bad` unless it is what's tested.
    let mut good = file(2);
    let mut bad = file(1);
    make_good(&mut good, step);
    make_bad(&mut bad, step);
    for later in &ORDER[at + 1..] {
        make_bad(&mut good, *later);
        make_good(&mut bad, *later);
    }
    (good, bad)
}

fn proves(step: Step) {
    let (good, bad) = pair_at(step);
    let why =
        why_ahead(&good, &bad).unwrap_or_else(|| panic!("{step:?}: the good file isn't ahead"));
    assert_eq!(why.step(), Some(step), "{why:?}");
    assert_eq!(why_ahead(&bad, &good), None, "{step:?}");
    // Through the ranking, in either input order.
    for files in [[good.clone(), bad.clone()], [bad.clone(), good.clone()]] {
        let ranking = rank(&files);
        assert_eq!(ranking.best(), Some(good.file_id), "{step:?}");
        assert_eq!(ranking.order[0].reason.step(), Some(step));
        assert_eq!(ranking.order[1].reason.step(), Some(step));
    }
}

macro_rules! step_test {
    ($name:ident, $step:expr) => {
        #[test]
        fn $name() {
            proves($step);
        }
    };
}

step_test!(
    a_file_on_disk_is_ahead_of_a_file_that_is_gone_whatever_it_has_later,
    Step::OnDisk
);
step_test!(
    a_better_verdict_is_ahead_whatever_the_file_has_after_it,
    Step::Verdict
);
step_test!(
    a_file_that_plays_in_the_dj_app_is_ahead_whatever_it_has_after_that,
    Step::Playable
);
step_test!(
    a_higher_cutoff_is_ahead_whatever_the_file_has_after_it,
    Step::Cutoff
);
step_test!(
    fewer_decode_errors_are_ahead_whatever_the_file_has_after_them,
    Step::DecodeErrors
);
step_test!(
    a_better_format_is_ahead_whatever_the_file_has_after_it,
    Step::Format
);
step_test!(
    more_tags_filled_is_ahead_whatever_the_file_has_after_them,
    Step::Tags
);
step_test!(
    embedded_art_is_ahead_whatever_the_file_has_after_it,
    Step::Art
);
step_test!(
    a_file_rekordbox_knows_is_ahead_whatever_the_file_has_after_it,
    Step::Rekordbox
);
step_test!(
    a_better_place_is_ahead_when_nothing_before_it_differs,
    Step::Place
);
step_test!(
    the_lower_file_id_is_ahead_when_nothing_else_differs,
    Step::Tied
);
