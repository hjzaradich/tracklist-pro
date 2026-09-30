//! 1aB-7: fingerprinting a two-hour mix keeps memory flat. The decoder
//! streams packets into chromaprint; nothing holds the whole track.
//!
//! Its own test binary, so the counting allocator sees only this test. The
//! mix is never written to disk: a reader makes up the WAV's bytes as
//! they're read.

use std::alloc::{GlobalAlloc, Layout, System};
use std::f64::consts::TAU;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicUsize, Ordering};

use symphonia::core::io::MediaSource;
use tracklist_pro_lib::fingerprint::decode::fingerprint;

/// Counts live heap bytes and their peak.
struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(by: usize) {
    let now = CURRENT.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            grew(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            if new_size >= layout.size() {
                grew(new_size - layout.size());
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// chromaprint's own rate: no resampling, so the test spends its time on
/// the part that could grow, not on filtering.
const RATE: u32 = 11_025;

/// A mono 16-bit WAV of `seconds`, whose audio is a 30-second phrase
/// repeated, made up as it's read.
struct EndlessMix {
    header: Vec<u8>,
    phrase: Vec<u8>,
    len: u64,
    pos: u64,
}

impl EndlessMix {
    fn new(seconds: u64) -> EndlessMix {
        let data = seconds * u64::from(RATE) * 2;
        let mut header = b"RIFF".to_vec();
        header.extend_from_slice(&(36 + data as u32).to_le_bytes());
        header.extend_from_slice(b"WAVEfmt ");
        for (v, n) in [
            (16u32, 4),
            (1, 2),
            (1, 2),
            (RATE, 4),
            (RATE * 2, 4),
            (2, 2),
            (16, 2),
        ] {
            header.extend_from_slice(&v.to_le_bytes()[..n]);
        }
        header.extend_from_slice(b"data");
        header.extend_from_slice(&(data as u32).to_le_bytes());
        // A new pair of notes every half second.
        let mut seed = 0x1234_5678u64;
        let mut phrase = Vec::new();
        for beat in 0..60 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let (a, b) = (
                110.0 + (seed >> 40) as f64 % 800.0,
                220.0 + (seed >> 20) as f64 % 900.0,
            );
            for i in 0..RATE / 2 {
                let t = f64::from(beat * RATE / 2 + i) / f64::from(RATE);
                let x = 0.4 * (TAU * a * t).sin() + 0.3 * (TAU * b * t).sin();
                phrase.extend_from_slice(&((x * 30_000.0) as i16).to_le_bytes());
            }
        }
        EndlessMix {
            len: header.len() as u64 + data,
            header,
            phrase,
            pos: 0,
        }
    }
}

impl Read for EndlessMix {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let at = self.pos as usize;
        let left = (self.len - self.pos.min(self.len)) as usize;
        let from = if at < self.header.len() {
            &self.header[at..]
        } else {
            &self.phrase[(at - self.header.len()) % self.phrase.len()..]
        };
        let n = buf.len().min(from.len()).min(left);
        buf[..n].copy_from_slice(&from[..n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for EndlessMix {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::End(d) => self.len as i64 + d,
            SeekFrom::Current(d) => self.pos as i64 + d,
        };
        self.pos = u64::try_from(pos).map_err(|_| io::ErrorKind::InvalidInput)?;
        Ok(self.pos)
    }
}

impl MediaSource for EndlessMix {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

/// Fingerprints `seconds` of audio; returns the fingerprint's length and
/// the peak heap growth while it ran.
fn peak_while_fingerprinting(seconds: u64) -> (f32, usize) {
    let mix = EndlessMix::new(seconds);
    let base = CURRENT.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let fp = fingerprint(Box::new(mix), &mut |_| true).expect("the mix decodes");
    (fp.seconds(), PEAK.load(Ordering::SeqCst) - base)
}

#[test]
fn a_two_hour_mix_is_fingerprinted_in_a_few_megabytes_of_memory() {
    let two_hours = 2 * 60 * 60;
    let (short_len, short_peak) = peak_while_fingerprinting(10 * 60);
    let (long_len, long_peak) = peak_while_fingerprinting(two_hours);
    println!(
        "10 min: peak {} KB; 2 h: peak {} KB (the audio alone would be {} MB)",
        short_peak / 1024,
        long_peak / 1024,
        two_hours * u64::from(RATE) * 2 / 1_000_000
    );
    assert!(
        long_len > two_hours as f32 - 5.0,
        "the whole mix: {long_len} s"
    );
    assert!(short_len > 590.0);
    // The fingerprint itself is about 32 bytes a second (230 KB for two
    // hours); everything else is fixed buffers.
    assert!(long_peak < 8 * 1024 * 1024, "peak {long_peak} bytes");
    let growth = long_peak.saturating_sub(short_peak);
    assert!(
        growth < 1024 * 1024,
        "grew {growth} bytes from 10 min to 2 h"
    );
}
