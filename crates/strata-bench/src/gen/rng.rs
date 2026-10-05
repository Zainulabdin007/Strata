//! Deterministic PRNG for synthetic generators (no external rand dep).

/// SplitMix64 / xorshift-style generator. Identical across platforms for a seed.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Creates a generator from `seed` (zero is remapped).
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    /// Next u64.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (n > 0).
    pub fn gen_range(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0);
        self.next_u64() % n
    }

    /// Pick from a non-empty slice.
    pub fn choose<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.gen_range(items.len() as u64) as usize]
    }
}
