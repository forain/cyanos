//! Fixed-size bitmap for tracking free/used resources (pages, PIDs, etc.).

pub struct Bitmap<const BITS: usize>
where
    [(); (BITS + 63) / 64]:,
{
    words: [u64; (BITS + 63) / 64],
}

impl<const BITS: usize> Bitmap<BITS>
where
    [(); (BITS + 63) / 64]:,
{
    pub const fn new() -> Self { Self { words: [0; (BITS + 63) / 64] } }

    pub fn set(&mut self, bit: usize) {
        self.words[bit / 64] |= 1 << (bit % 64);
    }

    pub fn clear(&mut self, bit: usize) {
        self.words[bit / 64] &= !(1 << (bit % 64));
    }

    pub fn get(&self, bit: usize) -> bool {
        (self.words[bit / 64] >> (bit % 64)) & 1 != 0
    }

    /// Return the index of the first clear bit, or None if all set.
    pub fn first_clear(&self) -> Option<usize> {
        for (i, &w) in self.words.iter().enumerate() {
            if w != !0u64 {
                let bit = w.trailing_ones() as usize;
                let idx = i * 64 + bit;
                if idx < BITS { return Some(idx); }
            }
        }
        None
    }
}
