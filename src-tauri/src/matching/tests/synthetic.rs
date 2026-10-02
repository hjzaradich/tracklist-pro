//! Made-up fingerprints: random items, with copies that differ the way a
//! re-encode does. No audio is involved, so thousands are cheap.

use crate::fingerprint::Fingerprint;

/// splitmix64: a seeded, platform-independent random source.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The items of made-up track `seed`, `len` items long. Different seeds
/// share nothing but chance.
pub fn track(seed: u64, len: usize) -> Vec<u32> {
    let mut rng = Rng(seed.wrapping_mul(0xD6E8_FEB8_6659_FD93) ^ 0x7AC4);
    (0..len).map(|_| rng.next() as u32).collect()
}

/// `items` as another encoding would give them: one item in `keep` stays
/// bit-identical and the rest get 1 to 4 bits flipped. With `keep` 7 that's
/// what a WAV against its MP3 measured (about 1 in 7 identical, score ~2).
pub fn reencoded(items: &[u32], seed: u64, keep: u64) -> Vec<u32> {
    let mut rng = Rng(seed ^ 0xE4C0);
    items
        .iter()
        .map(|&item| {
            if rng.below(keep) == 0 {
                return item;
            }
            let mut noisy = item;
            for _ in 0..=rng.below(4) {
                noisy ^= 1 << rng.below(32);
            }
            noisy
        })
        .collect()
}

pub fn fingerprint(items: Vec<u32>) -> Fingerprint {
    Fingerprint::new(items)
}
