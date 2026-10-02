//! Small deterministic PRNG (SplitMix64).
//!
//! Used for the synthetic demo traffic and for the shuffles in the permutation
//! test. Not cryptographic, and it doesn't need to be: we want reproducible runs
//! (same seed -> same landscape) and zero extra dependencies.

#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform integer in [lo, hi). Returns `lo` when the range is empty.
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        lo + self.next_u64() % (hi - lo)
    }

    /// Uniform index in [0, n). Returns 0 when n == 0.
    pub fn index(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next_u64() % n as u64) as usize
    }

    pub fn chance(&mut self, p: f64) -> bool {
        self.f64() < p
    }

    /// Exponentially distributed value with the given mean (Poisson inter-arrival).
    pub fn exp(&mut self, mean: f64) -> f64 {
        // 1 - f64() lies in (0, 1], so ln() is finite.
        -mean * (1.0 - self.f64()).ln()
    }

    /// Standard normal via Box-Muller.
    pub fn normal(&mut self) -> f64 {
        let u1 = 1.0 - self.f64();
        let u2 = self.f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    /// In-place Fisher-Yates shuffle.
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.index(i + 1);
            v.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_in_range() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
            let x = a.f64();
            assert!((0.0..1.0).contains(&x));
            b.f64();
            let r = a.range(10, 20);
            assert!((10..20).contains(&r));
            b.range(10, 20);
        }
        assert_eq!(a.range(5, 5), 5);
        assert_eq!(a.index(0), 0);
    }

    #[test]
    fn shuffle_is_a_permutation() {
        let mut rng = Rng::new(1);
        let mut v: Vec<u32> = (0..100).collect();
        rng.shuffle(&mut v);
        let mut sorted = v.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..100).collect::<Vec<_>>());
        assert_ne!(v, sorted, "a 100-element shuffle should move something");
    }

    #[test]
    fn exp_mean_is_close() {
        let mut rng = Rng::new(3);
        let n = 20_000;
        let mean: f64 = (0..n).map(|_| rng.exp(2.0)).sum::<f64>() / n as f64;
        assert!((mean - 2.0).abs() < 0.1, "mean was {mean}");
    }
}
