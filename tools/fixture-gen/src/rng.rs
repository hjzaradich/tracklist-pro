//! A small seeded random number generator (SplitMix64).
//!
//! Our own rather than a crate's, so a dependency update can never change
//! what a seed produces.

#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    /// An independent stream for one named purpose, so adding a draw in one
    /// place doesn't shift every value drawn after it elsewhere.
    pub fn derive(seed: u64, label: &str) -> Self {
        // FNV-1a over the label, mixed with the seed.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in label.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        let mut rng = Rng(seed ^ h);
        rng.next_u64();
        rng
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A value in `0..n`. `n` must be above 0.
    pub fn below(&mut self, n: u64) -> u64 {
        // The tiny modulo bias doesn't matter for test data.
        self.next_u64() % n
    }

    /// A value in `lo..=hi`.
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }

    /// A value in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_gives_the_same_stream() {
        let a: Vec<u64> = (0..5).map(|_| Rng::new(7).next_u64()).collect();
        assert!(a.windows(2).all(|w| w[0] == w[1]));
        let mut x = Rng::derive(7, "songs");
        let mut y = Rng::derive(7, "songs");
        for _ in 0..100 {
            assert_eq!(x.next_u64(), y.next_u64());
        }
    }

    #[test]
    fn different_labels_give_different_streams() {
        assert_ne!(
            Rng::derive(7, "songs").next_u64(),
            Rng::derive(7, "names").next_u64()
        );
    }
}
