//! Reads just the three windows of a track: seek to each, decode the stretch,
//! average the channels. A track is opened read-only and never written.

use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;

use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::{Error as SymphoniaError, SeekErrorKind};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{TimeBase, Timestamp};

use super::window::{first_frame, stretch_len, window_starts};
use super::ListenError;

/// Damaged packets in a row before the rest of a stretch is given up on
/// (it stays silent).
const MAX_ERRORS_IN_A_ROW: u32 = 64;

/// What was read: the three stretches (see [`super::window`]) at the file's
/// own rate, and the track's duration in seconds.
pub struct Stretches {
    pub rate: u32,
    pub duration: f64,
    pub stretches: Vec<Vec<f32>>,
}

struct Reader {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    rate: u32,
}

/// Opens `path` and reads its three windows' stretches. `keep_going` is asked
/// before every packet; `false` stops with [`ListenError::Cancelled`].
pub fn read_stretches(
    path: &Path,
    keep_going: &dyn Fn() -> bool,
) -> Result<Stretches, ListenError> {
    let file = File::open(path).map_err(|e| ListenError::Unreadable(e.to_string()))?;
    let (mut reader, header_frames) = open(file)?;
    let rate = reader.rate;
    // The length is in most headers; if it isn't, one pass counts it.
    let frames = match header_frames {
        Some(n) => n,
        None => reader.count_frames(keep_going)?,
    };
    if frames == 0 {
        return Err(ListenError::NoAudio);
    }
    let duration = frames as f64 / f64::from(rate);
    let mut stretches = Vec::with_capacity(3);
    for start in window_starts(duration) {
        stretches.push(reader.stretch(first_frame(start, rate), stretch_len(rate), keep_going)?);
    }
    Ok(Stretches {
        rate,
        duration,
        stretches,
    })
}

fn open(file: File) -> Result<(Reader, Option<u64>), ListenError> {
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    // The format comes from the bytes, never the extension.
    let format = symphonia::default::get_probe()
        .probe(
            &Hint::new(),
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| failure(&e))?;
    let track = format
        .first_track(TrackType::Audio)
        .ok_or(ListenError::NoAudio)?;
    let Some(CodecParameters::Audio(params)) = track.codec_params.clone() else {
        return Err(ListenError::NoAudio);
    };
    let rate = params
        .sample_rate
        .filter(|&r| r > 0)
        .ok_or(ListenError::NoAudio)?;
    // A header can claim any rate; buffers are sized from it.
    if rate > super::MAX_RATE {
        return Err(ListenError::ImpossibleRate(rate));
    }
    let decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| failure(&e))?;
    let (frames, track_id, time_base) = (
        track.num_frames.filter(|&n| n > 0),
        track.id,
        track.time_base,
    );
    Ok((
        Reader {
            format,
            decoder,
            track_id,
            time_base,
            rate,
        },
        frames,
    ))
}

fn failure(e: &SymphoniaError) -> ListenError {
    match e {
        SymphoniaError::Unsupported(_) => ListenError::Unsupported,
        SymphoniaError::SeekError(SeekErrorKind::Unseekable | SeekErrorKind::ForwardOnly) => {
            ListenError::Unsupported
        }
        SymphoniaError::IoError(e) if e.kind() != ErrorKind::UnexpectedEof => {
            ListenError::Unreadable(e.to_string())
        }
        _ => ListenError::Damaged,
    }
}

/// A failed seek: `Ok` only when the frame is past the end of the data (the
/// stretch is then silence); every other kind is a real error, so a track
/// that can't be seeked is never given a silent fingerprint.
pub(crate) fn past_the_end(e: &SymphoniaError) -> Result<(), ListenError> {
    match e {
        SymphoniaError::SeekError(SeekErrorKind::OutOfRange) => Ok(()),
        e => Err(failure(e)),
    }
}

impl Reader {
    /// `ticks` of the track's time base as frames.
    fn frames_of(&self, ticks: i64) -> i64 {
        let ticks = i128::from(ticks.max(0));
        match self.time_base {
            None => ticks as i64,
            Some(tb) => {
                (ticks * i128::from(tb.numer.get()) * i128::from(self.rate)
                    / i128::from(tb.denom.get())) as i64
            }
        }
    }

    /// The time-base ticks at frame `frame`.
    fn ticks_of(&self, frame: i64) -> i64 {
        match self.time_base {
            None => frame,
            Some(tb) => {
                (i128::from(frame) * i128::from(tb.denom.get())
                    / (i128::from(tb.numer.get()) * i128::from(self.rate))) as i64
            }
        }
    }

    /// The next decoded block (in packet order), or `None` at the end or
    /// after too much damage in a row.
    fn next_block(
        &mut self,
        errors: &mut u32,
        keep_going: &dyn Fn() -> bool,
    ) -> Result<Option<GenericAudioBufferRef<'_>>, ListenError> {
        loop {
            if !keep_going() {
                return Err(ListenError::Cancelled);
            }
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) | Err(SymphoniaError::ResetRequired) => return Ok(None),
                Err(SymphoniaError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => {
                    return Ok(None)
                }
                Err(SymphoniaError::IoError(e)) => {
                    return Err(ListenError::Unreadable(e.to_string()))
                }
                Err(_) => {
                    *errors += 1;
                    if *errors >= MAX_ERRORS_IN_A_ROW {
                        return Ok(None);
                    }
                    continue;
                }
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let decoded = self.decoder.decode(&packet).map(|_| ());
            match decoded {
                Ok(()) => {
                    *errors = 0;
                    return Ok(Some(self.decoder.last_decoded()));
                }
                Err(SymphoniaError::ResetRequired) => self.decoder.reset(),
                Err(_) => {
                    *errors += 1;
                    if *errors >= MAX_ERRORS_IN_A_ROW {
                        return Ok(None);
                    }
                }
            }
        }
    }

    /// The track's length in frames, by decoding it through once.
    fn count_frames(&mut self, keep_going: &dyn Fn() -> bool) -> Result<u64, ListenError> {
        let mut errors = 0;
        let mut total = 0u64;
        while let Some(block) = self.next_block(&mut errors, keep_going)? {
            total += block.frames() as u64;
        }
        // Back to the start for the seeks that follow.
        self.seek(0)?;
        Ok(total)
    }

    /// Positions the reader at (or a little before) frame `frame`; returns
    /// the frame it is really at, or `None` when the frame is past the end.
    fn seek(&mut self, frame: i64) -> Result<Option<i64>, ListenError> {
        let to = SeekTo::Timestamp {
            ts: Timestamp::new(self.ticks_of(frame.max(0))),
            track_id: self.track_id,
        };
        match self.format.seek(SeekMode::Accurate, to) {
            Ok(seeked) => {
                self.decoder.reset();
                Ok(Some(self.frames_of(seeked.actual_ts.get())))
            }
            Err(e) => past_the_end(&e).map(|()| None),
        }
    }

    /// `len` mono frames from frame `first` (negative: silence comes first).
    /// Frames the file doesn't hold, or damage swallowed, are silence.
    fn stretch(
        &mut self,
        first: i64,
        len: usize,
        keep_going: &dyn Fn() -> bool,
    ) -> Result<Vec<f32>, ListenError> {
        let mut out = vec![0f32; len];
        let end = first + len as i64;
        let Some(mut pos) = self.seek(first)? else {
            return Ok(out);
        };
        let mut errors = 0;
        let mut inter: Vec<f32> = Vec::new();
        while pos < end {
            let Some(block) = self.next_block(&mut errors, keep_going)? else {
                break;
            };
            let n = block.frames() as i64;
            if pos + n > first {
                let channels = block.spec().channels().count().max(1);
                block.copy_to_vec_interleaved(&mut inter);
                for (k, frame) in inter.chunks_exact(channels).enumerate() {
                    let at = pos + k as i64;
                    if at >= first && at < end {
                        out[(at - first) as usize] = frame.iter().sum::<f32>() / channels as f32;
                    }
                }
            }
            pos += n;
        }
        Ok(out)
    }
}
