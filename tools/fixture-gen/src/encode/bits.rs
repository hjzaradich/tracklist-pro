//! A most-significant-bit-first bit writer, for FLAC and ALAC frames.

pub struct BitWriter {
    bytes: Vec<u8>,
    acc: u64,
    nbits: u32,
}

impl BitWriter {
    pub fn new() -> BitWriter {
        BitWriter {
            bytes: Vec::new(),
            acc: 0,
            nbits: 0,
        }
    }

    /// Writes the low `n` bits of `value` (n ≤ 32).
    pub fn put(&mut self, n: u32, value: u64) {
        debug_assert!(n <= 32);
        if n == 0 {
            return;
        }
        self.acc = (self.acc << n) | (value & ((1u64 << n) - 1));
        self.nbits += n;
        while self.nbits >= 8 {
            self.nbits -= 8;
            self.bytes.push((self.acc >> self.nbits) as u8);
        }
        self.acc &= (1u64 << self.nbits) - 1;
    }

    /// Writes a signed value as `n`-bit two's complement.
    pub fn put_signed(&mut self, n: u32, value: i64) {
        self.put(n, value as u64);
    }

    /// `q` zero bits and a one.
    pub fn unary(&mut self, mut q: u32) {
        while q >= 32 {
            self.put(32, 0);
            q -= 32;
        }
        self.put(q + 1, 1);
    }

    /// Pads with zero bits to the next byte boundary.
    pub fn align(&mut self) {
        if self.nbits > 0 {
            self.put(8 - self.nbits, 0);
        }
    }

    pub fn into_bytes(mut self) -> Vec<u8> {
        self.align();
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_bits_most_significant_first() {
        let mut w = BitWriter::new();
        w.put(3, 0b101);
        w.put(5, 0b10011);
        w.put(4, 0xF);
        w.unary(2);
        assert_eq!(w.into_bytes(), vec![0b1011_0011, 0b1111_0010]);
    }
}
