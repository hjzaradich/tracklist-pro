use anyhow::{bail, Context, Result};
use std::fs::File;
use std::num::NonZeroU32;
use std::path::Path;

use mp3lame_encoder::{Bitrate, Builder, FlushNoGap, InterleavedPcm, MonoPcm, Quality};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// The only bitrates MP3 allows, ascending.
const LAME_BITRATES: &[(u32, Bitrate)] = &[
    (8, Bitrate::Kbps8),
    (16, Bitrate::Kbps16),
    (24, Bitrate::Kbps24),
    (32, Bitrate::Kbps32),
    (40, Bitrate::Kbps40),
    (48, Bitrate::Kbps48),
    (64, Bitrate::Kbps64),
    (80, Bitrate::Kbps80),
    (96, Bitrate::Kbps96),
    (112, Bitrate::Kbps112),
    (128, Bitrate::Kbps128),
    (160, Bitrate::Kbps160),
    (192, Bitrate::Kbps192),
    (224, Bitrate::Kbps224),
    (256, Bitrate::Kbps256),
    (320, Bitrate::Kbps320),
];

/// Sample rates LAME can emit. It resamples internally, so a 96 kHz source does
/// not need a resampler of our own.
const LAME_RATES: &[u32] = &[8_000, 11_025, 12_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000];

/// Picks the highest legal MP3 bitrate **at or below** the request.
///
/// Snapping down rather than to the nearest is the never-upconvert rule
/// (DESIGN.md §9.4) reaching the encoder: a 137 kbps VBR source capped to its
/// own rate must land on 128, not 160. Rounding up here would quietly reinstate
/// the inflation the policy layer went to trouble to prevent.
pub fn snap_bitrate(requested: u32) -> Bitrate {
    LAME_BITRATES
        .iter()
        .rev()
        .find(|(kbps, _)| *kbps <= requested)
        .map(|(_, bitrate)| *bitrate)
        // Nothing legal is lower than 8 kbps; a request under it is nonsense,
        // and refusing to encode at all would be worse than encoding badly.
        .unwrap_or(Bitrate::Kbps8)
}

/// Numeric form of [`snap_bitrate`], for reporting what was actually used.
pub fn snapped_kbps(requested: u32) -> u32 {
    LAME_BITRATES
        .iter()
        .rev()
        .find(|(kbps, _)| *kbps <= requested)
        .map(|(kbps, _)| *kbps)
        .unwrap_or(8)
}

/// Chooses the output sample rate for a given input rate.
///
/// High-resolution sources are halved so the result stays in the same family —
/// 88.2 kHz becomes 44.1 rather than 48, which avoids a resample between
/// incompatible clock families and the artefacts that come with it.
pub fn output_rate_for(input: u32) -> u32 {
    if LAME_RATES.contains(&input) {
        return input;
    }

    let mut rate = input;
    while rate > 48_000 {
        rate /= 2;
        if LAME_RATES.contains(&rate) {
            return rate;
        }
    }

    // An unusual rate that is already in range: take the nearest legal rate at
    // or below it, so we never claim more resolution than the source had.
    *LAME_RATES
        .iter()
        .rev()
        .find(|r| **r <= input)
        .unwrap_or(&8_000)
}

#[derive(Debug, Clone)]
pub struct TranscodeReport {
    pub source_sample_rate: u32,
    pub output_sample_rate: u32,
    pub channels: u16,
    /// What was actually encoded, after snapping to a legal MP3 bitrate.
    pub bitrate_kbps: u32,
    pub frames: u64,
    pub bytes_written: u64,
    /// Packets the decoder rejected but the stream recovered from. Non-zero
    /// means the output is missing audio, which the caller should surface
    /// rather than treat as a clean conversion.
    pub damaged_packets: u32,
}

/// Decodes `source` and writes an MP3 to `dest`.
///
/// The destination's parent directories are created. Nothing else on disk is
/// touched — the caller owns the temp-file-then-rename dance that makes an
/// import atomic.
pub fn to_mp3(source: &Path, dest: &Path, requested_kbps: u32) -> Result<TranscodeReport> {
    let file = File::open(source)
        .with_context(|| format!("could not open {} for decoding", source.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    // The extension is a hint only: probing still inspects the bytes, so a
    // mislabelled file is decoded as whatever it actually is.
    let mut hint = Hint::new();
    if let Some(extension) = source.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .with_context(|| format!("could not identify the format of {}", source.display()))?;

    let track = format
        .first_track(TrackType::Audio)
        .context("file contains no audio track")?;
    let track_id = track.id;

    let params = match track.codec_params.as_ref() {
        Some(CodecParameters::Audio(audio)) => audio.clone(),
        _ => bail!("audio track has no usable codec parameters"),
    };

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .context(
            "no decoder for this codec — it should have been refused at plan time \
             by policy::is_decodable",
        )?;

    let mut encoder: Option<mp3lame_encoder::Encoder> = None;
    let mut source_rate = 0_u32;
    let mut output_rate = 0_u32;
    let mut channels = 0_u16;
    let mut bitrate_kbps = snapped_kbps(requested_kbps);

    let mut pcm: Vec<i16> = Vec::new();
    let mut mp3: Vec<u8> = Vec::new();
    let mut frames = 0_u64;
    let mut damaged_packets = 0_u32;

    while let Some(packet) = format
        .next_packet()
        .with_context(|| format!("failed reading {}", source.display()))?
    {
        if packet.track_id != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // A corrupt packet is survivable and the stream continues. Counting
            // rather than aborting matches the per-file isolation principle
            // (DESIGN.md §4.3) one level down: a damaged frame should not cost
            // the user the whole track.
            Err(SymphoniaError::DecodeError(_)) => {
                damaged_packets += 1;
                continue;
            }
            Err(SymphoniaError::ResetRequired) => {
                bail!("the audio stream changes format part-way through, which cannot be encoded as one MP3")
            }
            Err(err) => return Err(err).context("decoding failed"),
        };

        if encoder.is_none() {
            let spec = decoded.spec();
            source_rate = spec.rate();
            let count = spec.channels().count();

            if count == 0 || count > 2 {
                bail!(
                    "{count}-channel audio cannot be encoded to MP3 without a downmix, \
                     which would silently discard channels"
                );
            }

            channels = count as u16;
            output_rate = output_rate_for(source_rate);

            let mut builder = Builder::new().context("could not create the LAME encoder")?;
            builder
                .set_num_channels(channels as u8)
                .map_err(|e| anyhow::anyhow!("channel count rejected by LAME: {e:?}"))?;
            builder
                .set_sample_rate(source_rate)
                .map_err(|e| anyhow::anyhow!("sample rate rejected by LAME: {e:?}"))?;
            if output_rate != source_rate {
                builder
                    .set_output_sample_rate(NonZeroU32::new(output_rate))
                    .map_err(|e| anyhow::anyhow!("output sample rate rejected: {e:?}"))?;
            }
            builder
                .set_brate(snap_bitrate(requested_kbps))
                .map_err(|e| anyhow::anyhow!("bitrate rejected by LAME: {e:?}"))?;
            builder
                .set_quality(Quality::Best)
                .map_err(|e| anyhow::anyhow!("quality rejected by LAME: {e:?}"))?;

            bitrate_kbps = snapped_kbps(requested_kbps);
            encoder = Some(builder.build().map_err(|e| {
                anyhow::anyhow!("could not initialise the LAME encoder: {e:?}")
            })?);
        }

        let encoder = encoder.as_mut().expect("encoder built above");

        decoded.copy_to_vec_interleaved(&mut pcm);
        if pcm.is_empty() {
            continue;
        }
        frames += (pcm.len() / channels as usize) as u64;

        // Reserving is not optional. `encode_to_vec` hands LAME whatever
        // `spare_capacity_mut()` happens to be, and LAME reads a zero-length
        // output buffer as "no bounds checking" — so without this it writes
        // past the end of the Vec and corrupts the heap. Over-reserving costs
        // nothing; `pcm.len()` is already generous for stereo, where LAME sizes
        // its estimate per channel.
        mp3.reserve(mp3lame_encoder::max_required_buffer_size(pcm.len()));

        if channels == 1 {
            encoder
                .encode_to_vec(MonoPcm(&pcm), &mut mp3)
                .map_err(|e| anyhow::anyhow!("encoding failed: {e:?}"))?;
        } else {
            encoder
                .encode_to_vec(InterleavedPcm(&pcm), &mut mp3)
                .map_err(|e| anyhow::anyhow!("encoding failed: {e:?}"))?;
        }
    }

    let mut encoder = encoder.context("the file decoded to no audio at all")?;
    // Same reservation rule as above: the final frame needs room, and LAME
    // documents 7200 bytes as the worst case for a flush.
    mp3.reserve(7_200);
    encoder
        .flush_to_vec::<FlushNoGap>(&mut mp3)
        .map_err(|e| anyhow::anyhow!("flushing the encoder failed: {e:?}"))?;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    std::fs::write(dest, &mp3)
        .with_context(|| format!("could not write {}", dest.display()))?;

    Ok(TranscodeReport {
        source_sample_rate: source_rate,
        output_sample_rate: output_rate,
        channels,
        bitrate_kbps,
        frames,
        bytes_written: mp3.len() as u64,
        damaged_packets,
    })
}

