//! Spike helper for the rekordbox session kit (E1) (see EXPERIMENTS.md).
//!
//!   kit-helper info  <file>                      -> one line of JSON
//!   kit-helper mp3   <src> <dst> <kbps>          -> encode, copying basic tags
//!   kit-helper aiff  <src> <dst>                 -> 16-bit PCM AIFF, copying basic tags
//!   kit-helper retag <file> <title> <comment>    -> rewrite title + comment only
//!
//! `transcode.rs` is musicmanager's encoder, copied verbatim (minus tests).
//! Throwaway code: it exists to make the kit, not to be ported.

mod transcode;

use anyhow::{bail, Context, Result};
use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{Tag, TagType};
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["info", file] => info(Path::new(file)),
        ["mp3", src, dst, kbps] => {
            let report = transcode::to_mp3(Path::new(src), Path::new(dst), kbps.parse()?)?;
            copy_tags(Path::new(src), Path::new(dst))?;
            eprintln!("encoded {} kbps, {} frames, {} damaged", report.bitrate_kbps, report.frames, report.damaged_packets);
            Ok(())
        }
        ["aiff", src, dst] => {
            to_aiff(Path::new(src), Path::new(dst))?;
            copy_tags(Path::new(src), Path::new(dst))
        }
        ["retag", file, title, comment] => retag(Path::new(file), title, comment),
        _ => bail!("usage: info <f> | mp3 <src> <dst> <kbps> | aiff <src> <dst> | retag <f> <title> <comment>"),
    }
}

fn info(path: &Path) -> Result<()> {
    let tagged = Probe::open(path)?.read().with_context(|| format!("reading {}", path.display()))?;
    let props = tagged.properties();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let get = |f: fn(&Tag) -> Option<std::borrow::Cow<'_, str>>| tag.and_then(|t| f(t).map(|s| s.to_string()));
    let json = serde_json::json!({
        "path": path.display().to_string(),
        "size": std::fs::metadata(path)?.len(),
        "duration_ms": props.duration().as_millis() as u64,
        "bitrate": props.audio_bitrate(),
        "sample_rate": props.sample_rate(),
        "title": get(|t| t.title()),
        "artist": get(|t| t.artist()),
        "album": get(|t| t.album()),
        "genre": get(|t| t.genre()),
    });
    println!("{json}");
    Ok(())
}

fn copy_tags(src: &Path, dst: &Path) -> Result<()> {
    let source = Probe::open(src)?.read()?;
    let Some(from) = source.primary_tag().or_else(|| source.first_tag()) else {
        return Ok(()); // untagged source: the kit's XML supplies metadata anyway
    };
    let mut tag = Tag::new(TagType::Id3v2);
    if let Some(v) = from.title() { tag.set_title(v.to_string()); }
    if let Some(v) = from.artist() { tag.set_artist(v.to_string()); }
    if let Some(v) = from.album() { tag.set_album(v.to_string()); }
    if let Some(v) = from.genre() { tag.set_genre(v.to_string()); }
    tag.save_to_path(dst, WriteOptions::default())
        .with_context(|| format!("tagging {}", dst.display()))
}

fn retag(path: &Path, title: &str, comment: &str) -> Result<()> {
    let mut tagged = Probe::open(path)?.read()?;
    if tagged.primary_tag().is_none() {
        tagged.insert_tag(Tag::new(tagged.primary_tag_type()));
    }
    let tag = tagged.primary_tag_mut().expect("inserted above");
    tag.set_title(title.to_string());
    tag.set_comment(comment.to_string());
    tag.save_to_path(path, WriteOptions::default())
        .with_context(|| format!("retagging {}", path.display()))
}

/// Decodes anything symphonia reads and writes a 16-bit big-endian PCM AIFF.
fn to_aiff(src: &Path, dst: &Path) -> Result<()> {
    let mss = MediaSourceStream::new(Box::new(std::fs::File::open(src)?), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = src.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe().probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())?;
    let track = format.first_track(TrackType::Audio).context("no audio track")?;
    let track_id = track.id;
    let Some(CodecParameters::Audio(params)) = track.codec_params.clone() else {
        bail!("no audio codec parameters")
    };
    let mut decoder = symphonia::default::get_codecs().make_audio_decoder(&params, &AudioDecoderOptions::default())?;

    let (mut rate, mut channels) = (0u32, 0u16);
    let mut pcm: Vec<i16> = Vec::new();
    let mut all: Vec<i16> = Vec::new();
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
            channels = decoded.spec().channels().count() as u16;
        }
        decoded.copy_to_vec_interleaved(&mut pcm);
        all.extend_from_slice(&pcm);
    }
    if channels == 0 {
        bail!("decoded no audio");
    }

    let frames = (all.len() / channels as usize) as u32;
    let data_len = all.len() as u32 * 2;
    let mut out = Vec::with_capacity(data_len as usize + 64);
    out.extend_from_slice(b"FORM");
    out.extend_from_slice(&(4 + 8 + 18 + 8 + 8 + data_len).to_be_bytes());
    out.extend_from_slice(b"AIFF");
    out.extend_from_slice(b"COMM");
    out.extend_from_slice(&18u32.to_be_bytes());
    out.extend_from_slice(&channels.to_be_bytes());
    out.extend_from_slice(&frames.to_be_bytes());
    out.extend_from_slice(&16u16.to_be_bytes());
    out.extend_from_slice(&extended80(rate as f64));
    out.extend_from_slice(b"SSND");
    out.extend_from_slice(&(8 + data_len).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes()); // offset
    out.extend_from_slice(&0u32.to_be_bytes()); // block size
    for s in all {
        out.extend_from_slice(&s.to_be_bytes());
    }
    std::fs::write(dst, out)?;
    Ok(())
}

/// IEEE 754 80-bit extended float, as AIFF's COMM chunk wants the sample rate.
fn extended80(value: f64) -> [u8; 10] {
    let mut out = [0u8; 10];
    if value == 0.0 {
        return out;
    }
    let exp = value.log2().floor() as i32;
    let mantissa = (value / 2f64.powi(exp) * 2f64.powi(63)) as u64;
    out[..2].copy_from_slice(&((exp + 16383) as u16).to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}
