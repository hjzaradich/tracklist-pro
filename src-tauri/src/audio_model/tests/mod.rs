//! The listening step against CED's own reference outputs
//! (`tests/fixtures/audio_model/`, see its README). The two signals are
//! rebuilt here from the README's formulas, so no audio is committed.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::mel::{Frontend, N_FRAMES, N_MELS, WINDOW_SAMPLES};
use super::window::{stretch_of, window_starts};
use super::*;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio_model")
        .join(name)
}

fn model_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/models/ced-tiny/ced-tiny.onnx")
}

fn floats(name: &str) -> Vec<f32> {
    let bytes = std::fs::read(fixture(name)).expect("fixture");
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// One model for the whole test run: loading checks 22 MB and optimises it.
fn model() -> &'static AudioModel {
    static MODEL: OnceLock<AudioModel> = OnceLock::new();
    MODEL.get_or_init(|| AudioModel::load(&model_file()).expect("the bundled model loads"))
}

/// The README's signals: sines, a linear chirp and LCG noise, f64 left to
/// right, rounded to f32 once.
fn signal(
    rate: u32,
    seconds: u32,
    sines: &[(f64, f64, f64)],
    chirp: (f64, f64, f64),
    noise: (f64, u32),
) -> Vec<f32> {
    let n = (rate * seconds) as usize;
    let (ac, f0, f1) = chirp;
    let k = (f1 - f0) / f64::from(seconds);
    let two_pi = 2.0 * std::f64::consts::PI;
    let mut state = noise.1;
    (0..n)
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            let mut x = 0.0;
            for &(a, f, p) in sines {
                x += a * (two_pi * f * t + p).sin();
            }
            x += ac * (two_pi * (f0 * t + (k / 2.0) * t * t)).sin();
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            x += noise.0 * (2.0 * f64::from(state >> 8) / 16_777_216.0 - 1.0);
            x as f32
        })
        .collect()
}

fn signal_a() -> Vec<f32> {
    signal(
        16_000,
        10,
        &[(0.30, 220.0, 0.0), (0.20, 880.0, 0.5), (0.15, 3000.0, 0.0)],
        (0.20, 200.0, 6000.0),
        (0.05, 12345),
    )
}

fn signal_b() -> Vec<f32> {
    signal(
        44_100,
        40,
        &[
            (0.30, 110.0, 0.0),
            (0.20, 440.0, 1.0),
            (0.10, 1760.0, 0.0),
            (0.10, 12_000.0, 0.0),
        ],
        (0.15, 300.0, 7000.0),
        (0.05, 54321),
    )
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    let norm = |v: &[f32]| v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    dot / (norm(a) * norm(b))
}

fn largest_difference(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

fn top_n(scores: &[f32], n: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    order.truncate(n);
    order
}

fn top5(scores: &[f32]) -> Vec<usize> {
    top_n(scores, 5)
}

/// A mono 32-bit float WAV, written by hand so the samples survive exactly.
fn write_wav(path: &Path, samples: &[f32], rate: u32) {
    let data = (samples.len() * 4) as u32;
    let mut b = Vec::with_capacity(samples.len() * 4 + 56);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(48 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(b"fact");
    b.extend_from_slice(&4u32.to_le_bytes());
    b.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, b).expect("temp wav");
}

#[test]
fn the_model_can_be_shared_between_threads() {
    fn shareable<T: Send + Sync>() {}
    shareable::<AudioModel>();
}

#[test]
fn windows_of_a_long_track_are_centred_on_a_quarter_a_half_and_three_quarters() {
    assert_eq!(window_starts(300.0), [70.0, 145.0, 220.0]);
    assert_eq!(window_starts(40.0), [5.0, 15.0, 25.0]);
}

#[test]
fn a_track_of_exactly_ten_seconds_listens_to_all_of_it_three_times() {
    assert_eq!(window_starts(10.0), [0.0, 0.0, 0.0]);
}

#[test]
fn windows_of_a_short_track_are_held_inside_it() {
    assert_eq!(window_starts(4.0), [0.0, 0.0, 0.0]);
    assert_eq!(window_starts(11.0), [0.0, 0.5, 1.0]);
}

#[test]
fn a_short_track_is_padded_with_silence() {
    let tone = vec![0.5f32; 4 * 16_000];
    let stretch = stretch_of(&tone, 16_000, 0.0);
    assert_eq!(stretch.len(), 10 * 16_000 + 3200);
    // The margin before the start, then the track, then silence.
    assert!(stretch[..1600].iter().all(|&v| v == 0.0));
    assert!(stretch[1600..1600 + 4 * 16_000].iter().all(|&v| v == 0.5));
    assert!(stretch[1600 + 4 * 16_000..].iter().all(|&v| v == 0.0));
}

#[test]
fn a_short_track_gets_a_fingerprint_and_scores() {
    let short = &signal_a()[..4 * 16_000];
    let heard = model().listen_samples(short, 16_000).expect("listens");
    assert_eq!(heard.fingerprint.len(), EMBEDDING_LEN);
    assert_eq!(heard.scores.len(), LABEL_COUNT);
    assert_eq!(heard.window_starts, [0.0, 0.0, 0.0]);
    assert!(heard
        .fingerprint
        .iter()
        .chain(&heard.scores)
        .all(|v| v.is_finite()));
}

#[test]
fn the_rebuilt_signals_match_the_references_probe_samples() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture("manifest.json")).unwrap()).unwrap();
    for (key, samples) in [
        ("parity_16k_10s", signal_a()),
        ("whole_track_44k_40s", signal_b()),
    ] {
        for probe in manifest[key]["probes"].as_array().unwrap() {
            let (i, want) = (
                probe["index"].as_u64().unwrap() as usize,
                probe["value"].as_f64().unwrap(),
            );
            assert!(
                (f64::from(samples[i]) - want).abs() < 1e-5,
                "{key} sample {i}"
            );
        }
    }
}

#[test]
fn the_mel_features_of_signal_a_match_ced_within_a_twentieth_of_a_decibel() {
    let mel = Frontend::new().mel(&signal_a());
    let want = floats("parity_16k_10s.mel.f32");
    assert_eq!(mel.len(), N_MELS * N_FRAMES);
    let worst = largest_difference(&mel, &want);
    assert!(worst <= 0.05, "largest mel difference {worst} dB");
}

#[test]
fn the_model_alone_reproduces_the_reference_on_the_reference_mel() {
    let heard = model().run(floats("parity_16k_10s.mel.f32")).expect("runs");
    assert!(cosine(&heard.emb, &floats("parity_16k_10s.emb.f32")) >= 0.99999);
    assert!(largest_difference(&heard.logits, &floats("parity_16k_10s.logits.f32")) <= 1e-3);
}

#[test]
fn signal_a_through_the_front_end_and_model_matches_the_reference() {
    let wave = signal_a();
    assert_eq!(wave.len(), WINDOW_SAMPLES);
    let heard = model().hear_window(&wave).expect("listens");
    let (emb, logits) = (
        floats("parity_16k_10s.emb.f32"),
        floats("parity_16k_10s.logits.f32"),
    );
    let similarity = cosine(&heard.emb, &emb);
    let worst = largest_difference(&heard.logits, &logits);
    assert!(similarity >= 0.99999, "embedding cosine {similarity}");
    assert!(worst <= 1e-3, "largest logit difference {worst}");
    assert_eq!(top5(&heard.logits), top5(&logits));
}

#[test]
fn signal_b_through_resampling_windows_and_model_matches_the_reference() {
    let (heard, starts) = model().hear_samples(&signal_b(), 44_100).expect("listens");
    assert_eq!(starts, [5.0, 15.0, 25.0]);
    let (embs, logits) = (
        floats("whole_track_44k_40s.windows_emb.f32"),
        floats("whole_track_44k_40s.windows_logits.f32"),
    );
    for (w, h) in heard.iter().enumerate() {
        let similarity = cosine(&h.emb, &embs[w * EMBEDDING_LEN..(w + 1) * EMBEDDING_LEN]);
        assert!(
            similarity >= 0.999,
            "window {w}: embedding cosine {similarity}"
        );
        let scores = cosine(&h.logits, &logits[w * LABEL_COUNT..(w + 1) * LABEL_COUNT]);
        assert!(scores >= 0.999, "window {w}: score cosine {scores}");
    }
    let mean = average(&heard, starts);
    let similarity = cosine(&mean.fingerprint, &floats("whole_track_44k_40s.emb.f32"));
    assert!(similarity >= 0.999, "fingerprint cosine {similarity}");
    // The fifth and sixth labels of the reference are 0.003 apart, so a
    // different resampler from the reference's can swap them: the first four
    // must agree in order, and the fifth must be among our first six.
    let reference = floats("whole_track_44k_40s.logits.f32");
    let (ours, theirs) = (top_n(&mean.scores, 6), top_n(&reference, 5));
    assert_eq!(ours[..4], theirs[..4], "the four best labels");
    assert!(
        ours.contains(&theirs[4]),
        "the reference's fifth label {} is not in our first six {ours:?}",
        theirs[4]
    );
}

#[test]
fn a_wav_on_disk_gives_the_same_result_as_the_samples_it_holds() {
    let samples = signal_b();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("signal.wav");
    write_wav(&path, &samples, 44_100);
    let from_file = model()
        .listen(&path, &|| true)
        .expect("decodes and listens");
    let from_samples = model().listen_samples(&samples, 44_100).expect("listens");
    assert_eq!(from_file.window_starts, [5.0, 15.0, 25.0]);
    assert_eq!(from_file.window_starts, from_samples.window_starts);
    assert!(cosine(&from_file.fingerprint, &from_samples.fingerprint) >= 0.999_999);
    assert!(largest_difference(&from_file.scores, &from_samples.scores) <= 1e-3);
}

#[test]
fn listening_stops_when_asked_to() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("short.wav");
    write_wav(&path, &signal_a(), 16_000);
    assert!(matches!(
        model().listen(&path, &|| false),
        Err(ListenError::Cancelled)
    ));
}

#[test]
fn a_file_that_is_not_audio_is_refused_not_panicked_on() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, b"not audio at all, just some words").unwrap();
    assert!(model().listen(&path, &|| true).is_err());
    assert!(model()
        .listen(&dir.path().join("gone.wav"), &|| true)
        .is_err());
}

#[test]
fn a_tampered_model_file_is_refused() {
    let mut bytes = std::fs::read(model_file()).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 1;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ced-tiny.onnx");
    std::fs::write(&path, bytes).unwrap();
    let Err(error) = AudioModel::load(&path) else {
        panic!("a changed model must not load")
    };
    assert!(matches!(error, ModelError::NotTheModel { .. }), "{error}");
}

#[test]
fn a_missing_model_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let Err(error) = AudioModel::load(&dir.path().join("ced-tiny.onnx")) else {
        panic!("no file, no model")
    };
    assert!(matches!(error, ModelError::Missing(_)), "{error}");
}

#[test]
fn the_app_without_the_bundle_reports_the_model_missing() {
    let app = tauri::test::mock_app();
    assert!(AudioModel::load_bundled(app.handle()).is_err());
}
