//! 1aB-5: one bounded pass over the file gives both hashes, stops promptly
//! when asked, and is fast.

use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::time::Instant;

use super::fixtures::*;
use crate::hash::{hash_reader, AudioFormat, BUFFER};

/// Counts the bytes read through it.
struct Counting<R> {
    inner: R,
    read: u64,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        Ok(n)
    }
}

impl<R: Seek> Seek for Counting<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// A tagged WAVE with `mb` MiB of samples.
fn big_wav(mb: usize) -> Vec<u8> {
    let samples: Vec<u8> = (0..mb << 20).map(|i| (i * 13 + (i >> 9)) as u8).collect();
    riff(
        b"RIFF",
        &[
            chunk(b"fmt ", &wav_fmt(), false),
            list_info("Night Drive"),
            chunk(b"data", &samples, false),
            chunk(b"id3 ", &vec![0; 4096], false),
        ],
    )
}

#[test]
fn both_hashes_come_from_a_single_read_of_the_file() {
    let bytes = big_wav(16);
    let len = bytes.len() as u64;
    let mut file = Counting {
        inner: Cursor::new(bytes),
        read: 0,
    };
    let hashed = hash_reader(&mut file, &mut vec![0; BUFFER], &mut |_| false)
        .unwrap()
        .unwrap();
    assert!(hashed.audio.is_ok());
    // The file once, plus the few small header reads that find the audio.
    let planning = file.read - len;
    assert!(
        planning < 16 * 1024,
        "{planning} bytes read on top of the file"
    );
}

#[test]
fn cancelling_stops_before_the_next_buffer_even_mid_file() {
    let bytes = big_wav(32);
    let stop_at = 5 << 20;
    let mut file = Counting {
        inner: Cursor::new(bytes),
        read: 0,
    };
    let mut asked = 0;
    let result = hash_reader(&mut file, &mut vec![0; BUFFER], &mut |offset| {
        asked += 1;
        offset >= stop_at
    })
    .unwrap();
    assert!(result.is_none(), "it finished instead of stopping");
    // Stopped at the first check past 5 MiB: nothing read after it.
    assert!(
        file.read <= stop_at + BUFFER as u64 + 16 * 1024,
        "read {} bytes",
        file.read
    );
    assert!(asked > 5, "asked only {asked} times");
}

#[test]
fn throughput_of_one_pass_hashing_in_memory() {
    const MB: usize = 64;
    let bytes = big_wav(MB);
    let mut buf = vec![0; BUFFER];
    let start = Instant::now();
    let hashed = hash_reader(&mut Cursor::new(&bytes[..]), &mut buf, &mut |_| false)
        .unwrap()
        .unwrap();
    let elapsed = start.elapsed();
    assert_eq!(hashed.audio.map(|a| a.format), Ok(AudioFormat::Wave));
    let rate = MB as f64 / elapsed.as_secs_f64();
    println!("blake3 + audio_hash in memory: {MB} MiB in {elapsed:?} = {rate:.0} MiB/s");
    // A smoke check, not a benchmark: even an unoptimized test build on a
    // loaded runner hashes faster than a hard disk reads.
    assert!(rate > 20.0, "{rate:.0} MiB/s");
}
