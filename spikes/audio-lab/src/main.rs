//! Phase 0 audio spike (see EXPERIMENTS.md). Throwaway code: it answers
//! questions, it isn't meant to be ported as-is.
//!
//!   audio-lab compare <a> <b>     fingerprint both files, print matched segments as JSON
//!   audio-lab bpmkey <file>       estimate BPM and key (Camelot), print JSON
//!   audio-lab cutoff <file>       estimate the spectral lowpass (fake-320 check), print JSON

use anyhow::{bail, Context, Result};
use rusty_chromaprint::{match_fingerprints, Configuration, Fingerprinter};
use rustfft::{num_complex::Complex, FftPlanner};
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

struct Audio {
    rate: u32,
    channels: usize,
    interleaved: Vec<i16>,
}

impl Audio {
    fn mono(&self) -> Vec<f32> {
        self.interleaved
            .chunks(self.channels)
            .map(|f| f.iter().map(|&s| s as f32).sum::<f32>() / (self.channels as f32 * 32768.0))
            .collect()
    }
}

/// Decodes up to `max_secs` seconds (all if None) into interleaved i16.
fn decode(path: &Path, max_secs: Option<f32>) -> Result<Audio> {
    let mss = MediaSourceStream::new(Box::new(std::fs::File::open(path)?), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .with_context(|| format!("probe {}", path.display()))?;
    let track = format.first_track(TrackType::Audio).context("no audio track")?;
    let track_id = track.id;
    let Some(CodecParameters::Audio(params)) = track.codec_params.clone() else { bail!("no codec params") };
    let mut decoder = symphonia::default::get_codecs().make_audio_decoder(&params, &AudioDecoderOptions::default())?;

    let (mut rate, mut channels) = (0u32, 0usize);
    let mut buf: Vec<i16> = Vec::new();
    let mut out: Vec<i16> = Vec::new();
    while let Some(packet) = format.next_packet()? {
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };
        if rate == 0 {
            rate = decoded.spec().rate();
            channels = decoded.spec().channels().count().max(1);
        }
        decoded.copy_to_vec_interleaved(&mut buf);
        out.extend_from_slice(&buf);
        if let Some(m) = max_secs {
            if out.len() >= (m * rate as f32) as usize * channels {
                break;
            }
        }
    }
    if rate == 0 {
        bail!("decoded no audio");
    }
    Ok(Audio { rate, channels, interleaved: out })
}

fn fingerprint(a: &Audio, cfg: &Configuration) -> Result<Vec<u32>> {
    let mut p = Fingerprinter::new(cfg);
    p.start(a.rate, a.channels as u32).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    p.consume(&a.interleaved);
    p.finish();
    Ok(p.fingerprint().to_vec())
}

fn compare(a: &Path, b: &Path) -> Result<()> {
    let cfg = Configuration::preset_test2();
    let (fa, fb) = (decode(a, None)?, decode(b, None)?);
    let (pa, pb) = (fingerprint(&fa, &cfg)?, fingerprint(&fb, &cfg)?);
    let segs = match_fingerprints(&pa, &pb, &cfg).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let item = cfg.item_duration_in_seconds();
    let (len_a, len_b) = (pa.len() as f32 * item, pb.len() as f32 * item);
    let matched: f32 = segs.iter().map(|s| s.duration(&cfg)).sum();
    let weighted = if matched > 0.0 {
        segs.iter().map(|s| s.score * s.duration(&cfg) as f64).sum::<f64>() / matched as f64
    } else {
        32.0
    };
    let json = serde_json::json!({
        "len_a": len_a, "len_b": len_b, "matched_s": matched,
        "cover_a": matched / len_a.max(1.0), "cover_b": matched / len_b.max(1.0),
        "score": weighted, "segments": segs.len(),
        "offsets": segs.iter().take(5).map(|s| (s.start1(&cfg), s.start2(&cfg), s.duration(&cfg), s.score)).collect::<Vec<_>>(),
    });
    println!("{json}");
    Ok(())
}

// ---------- BPM ----------

/// Spectral-flux onset envelope of mono audio, at `rate / hop` frames per second.
fn onset_envelope(x: &[f32], frame: usize, hop: usize) -> Vec<f32> {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(frame);
    let window: Vec<f32> = (0..frame).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / frame as f32).cos()).collect();
    let mut prev = vec![0f32; frame / 2];
    let mut env = Vec::new();
    let mut buf = vec![Complex::new(0f32, 0f32); frame];
    let mut i = 0;
    while i + frame <= x.len() {
        for (k, b) in buf.iter_mut().enumerate() {
            *b = Complex::new(x[i + k] * window[k], 0.0);
        }
        fft.process(&mut buf);
        let mut flux = 0f32;
        for k in 1..frame / 2 {
            let m = (1.0 + 1000.0 * buf[k].norm()).ln();
            flux += (m - prev[k]).max(0.0);
            prev[k] = m;
        }
        env.push(flux);
        i += hop;
    }
    // remove slow trend (local mean over ~1 s) and half-wave rectify
    let w = 40usize;
    let mut out = vec![0f32; env.len()];
    for t in 0..env.len() {
        let lo = t.saturating_sub(w);
        let hi = (t + w).min(env.len());
        let mean = env[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
        out[t] = (env[t] - mean).max(0.0);
    }
    out
}

/// Autocorrelation value at a fractional lag (linear interpolation).
fn acf_at(env: &[f32], lag: f32) -> f32 {
    let l0 = lag.floor() as usize;
    let frac = lag - l0 as f32;
    let n = env.len().saturating_sub(l0 + 1);
    let mut s = 0f32;
    for t in 0..n {
        let v = env[t + l0] * (1.0 - frac) + env[t + l0 + 1] * frac;
        s += env[t] * v;
    }
    s / n.max(1) as f32
}

fn estimate_bpm(mono: &[f32], rate: u32) -> f32 {
    // decimate to ~11 kHz by averaging
    let dec = (rate / 11025).max(1) as usize;
    let x: Vec<f32> = mono.chunks(dec).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    let sr = rate as f32 / dec as f32;
    let hop = 128usize;
    let env = onset_envelope(&x, 1024, hop);
    let fps = sr / hop as f32;
    // comb score: sum of ACF at 1..=8 beat multiples, searched finely over 60..200 BPM
    let score = |bpm: f32| -> f32 {
        let period = 60.0 * fps / bpm;
        (1..=8).map(|k| acf_at(&env, period * k as f32)).sum()
    };
    let mut best = (0f32, f32::MIN);
    let mut bpm = 60.0;
    while bpm <= 200.0 {
        let s = score(bpm);
        if s > best.1 {
            best = (bpm, s);
        }
        bpm += 0.5;
    }
    // refine ±0.5 in 0.02 steps
    let mut fine = best;
    let mut b = best.0 - 0.5;
    while b <= best.0 + 0.5 {
        let s = score(b);
        if s > fine.1 {
            fine = (b, s);
        }
        b += 0.02;
    }
    // fold into rekordbox's configured range 98..195 (the user's setting)
    let mut v = fine.0;
    while v < 98.0 {
        v *= 2.0;
    }
    while v >= 195.0 {
        v /= 2.0;
    }
    v
}

// ---------- Key ----------

// Krumhansl-Kessler (default) and Temperley (Kostka-Payne) key profiles; KEY_PROFILE picks one.
const KK_MAJOR: [f32; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const KK_MINOR: [f32; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];
const TP_MAJOR: [f32; 12] = [0.748, 0.060, 0.488, 0.082, 0.670, 0.460, 0.096, 0.715, 0.104, 0.366, 0.057, 0.400];
const TP_MINOR: [f32; 12] = [0.712, 0.084, 0.474, 0.618, 0.049, 0.460, 0.105, 0.747, 0.404, 0.067, 0.133, 0.330];
// Camelot for major keys C..B and minor keys Cm..Bm (pitch class 0 = C)
const CAMELOT_MAJOR: [&str; 12] = ["8B", "3B", "10B", "5B", "12B", "7B", "2B", "9B", "4B", "11B", "6B", "1B"];
const CAMELOT_MINOR: [&str; 12] = ["5A", "12A", "7A", "2A", "9A", "4A", "11A", "6A", "1A", "8A", "3A", "10A"];

fn corr(a: &[f32; 12], b: &[f32; 12]) -> f32 {
    let ma = a.iter().sum::<f32>() / 12.0;
    let mb = b.iter().sum::<f32>() / 12.0;
    let (mut n, mut da, mut db) = (0f32, 0f32, 0f32);
    for i in 0..12 {
        n += (a[i] - ma) * (b[i] - mb);
        da += (a[i] - ma).powi(2);
        db += (b[i] - mb).powi(2);
    }
    n / (da.sqrt() * db.sqrt()).max(1e-9)
}

fn estimate_key(mono: &[f32], rate: u32) -> (&'static str, f32) {
    let dec = (rate / 11025).max(1) as usize;
    let x: Vec<f32> = mono.chunks(dec).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    let sr = rate as f32 / dec as f32;
    let frame = 8192usize;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(frame);
    let window: Vec<f32> = (0..frame).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / frame as f32).cos()).collect();
    let mut chroma = [0f32; 12];
    let mut buf = vec![Complex::new(0f32, 0f32); frame];
    let mut i = 0;
    while i + frame <= x.len() {
        for (k, b) in buf.iter_mut().enumerate() {
            *b = Complex::new(x[i + k] * window[k], 0.0);
        }
        fft.process(&mut buf);
        let mut local = [0f32; 12];
        for k in 1..frame / 2 {
            let f = k as f32 * sr / frame as f32;
            if !(55.0..=2000.0).contains(&f) {
                continue;
            }
            let midi = 69.0 + 12.0 * (f / 440.0).log2();
            let pc = (midi.round() as i32).rem_euclid(12) as usize;
            local[pc] += buf[k].norm_sqr().sqrt();
        }
        let s: f32 = local.iter().sum::<f32>().max(1e-9);
        for p in 0..12 {
            chroma[p] += local[p] / s;
        }
        i += frame / 2;
    }
    let temperley = std::env::var("KEY_PROFILE").map(|v| v == "temperley").unwrap_or(false);
    let (major, minor) = if temperley { (TP_MAJOR, TP_MINOR) } else { (KK_MAJOR, KK_MINOR) };
    let minor_bias: f32 = std::env::var("MINOR_BIAS").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let mut best = ("?", f32::MIN);
    for tonic in 0..12 {
        let mut maj = [0f32; 12];
        let mut min = [0f32; 12];
        for p in 0..12 {
            maj[(p + tonic) % 12] = major[p];
            min[(p + tonic) % 12] = minor[p];
        }
        let (cm, cn) = (corr(&chroma, &maj), corr(&chroma, &min) + minor_bias);
        if cm > best.1 {
            best = (CAMELOT_MAJOR[tonic], cm);
        }
        if cn > best.1 {
            best = (CAMELOT_MINOR[tonic], cn);
        }
    }
    best
}

fn bpmkey(path: &Path) -> Result<()> {
    let a = decode(path, Some(150.0))?;
    let mono = a.mono();
    let start = (30.0 * a.rate as f32) as usize; // skip intros
    let seg = if mono.len() > start + a.rate as usize * 60 { &mono[start..] } else { &mono[..] };
    let t = std::time::Instant::now();
    let bpm = estimate_bpm(seg, a.rate);
    let (key, conf) = estimate_key(seg, a.rate);
    println!("{}", serde_json::json!({"bpm": bpm, "key": key, "key_conf": conf, "ms": t.elapsed().as_millis()}));
    Ok(())
}

// ---------- Fake-320 cutoff ----------

fn cutoff(path: &Path) -> Result<()> {
    let a = decode(path, Some(90.0))?;
    let mono = a.mono();
    let start = (30.0 * a.rate as f32) as usize;
    let x = if mono.len() > start + a.rate as usize * 20 { &mono[start..] } else { &mono[..] };
    let frame = 4096usize;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(frame);
    let window: Vec<f32> = (0..frame).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / frame as f32).cos()).collect();
    let mut power = vec![0f64; frame / 2];
    let mut buf = vec![Complex::new(0f32, 0f32); frame];
    let (mut i, mut n) = (0, 0);
    while i + frame <= x.len() {
        for (k, b) in buf.iter_mut().enumerate() {
            *b = Complex::new(x[i + k] * window[k], 0.0);
        }
        fft.process(&mut buf);
        for k in 0..frame / 2 {
            power[k] += buf[k].norm_sqr() as f64;
        }
        i += frame;
        n += 1;
    }
    let hz = |k: usize| k as f32 * a.rate as f32 / frame as f32;
    let db: Vec<f64> = power.iter().map(|p| 10.0 * (p / n.max(1) as f64 + 1e-20).log10()).collect();
    // reference level: median dB between 2 and 8 kHz
    let mut mid: Vec<f64> = (0..frame / 2).filter(|&k| (2000.0..8000.0).contains(&hz(k))).map(|k| db[k]).collect();
    mid.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let reference = mid.get(mid.len() / 2).copied().unwrap_or(-100.0);
    // highest frequency whose 250 Hz-smoothed level is within 50 dB of the reference
    let smooth = (250.0 / (a.rate as f32 / frame as f32)).ceil() as usize;
    let mut cut = 0f32;
    for k in (smooth..frame / 2 - smooth).rev() {
        let avg = db[k - smooth..k + smooth].iter().sum::<f64>() / (2 * smooth) as f64;
        if avg > reference - 50.0 {
            cut = hz(k);
            break;
        }
    }
    println!("{}", serde_json::json!({"cutoff_hz": cut, "rate": a.rate, "ref_db": reference}));
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["compare", a, b] => compare(Path::new(a), Path::new(b)),
        ["bpmkey", f] => bpmkey(Path::new(f)),
        ["cutoff", f] => cutoff(Path::new(f)),
        _ => bail!("usage: compare <a> <b> | bpmkey <file> | cutoff <file>"),
    }
}
