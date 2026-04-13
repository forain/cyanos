//! Fixed-size bitmap for tracking free/used resources (pages, PIDs, etc.).
//!
//! `N` is the number of u64 words, so total capacity = N * 64 bits.
//! Example: `Bitmap::<16>` tracks 1024 resources.

pub struct Bitmap<const N: usize> {
    words: [u64; N],
}

impl<const N: usize> Bitmap<N> {
    pub const fn new() -> Self { Self { words: [0; N] } }

    pub fn set(&mut self, bit: usize) {
        self.words[bit / 64] |= 1 << (bit % 64);
    }

    pub fn clear(&mut self, bit: usize) {
        self.words[bit / 64] &= !(1 << (bit % 64));
    }

    pub fn get(&self, bit: usize) -> bool {
        (self.words[bit / 64] >> (bit % 64)) & 1 != 0
    }

    /// Return the index of the first clear bit, or None if all are set.
    pub fn first_clear(&self) -> Option<usize> {
        for (i, &w) in self.words.iter().enumerate() {
            if w != !0u64 {
                return Some(i * 64 + w.trailing_ones() as usize);
            }
        }
        None
    }

    pub const fn capacity(&self) -> usize { N * 64 }
}
