//! A seeded Gaussian noise source: the same seed gives the same flight.

/// SplitMix64 with Box-Muller: small, fast, and reproducible.
#[derive(Debug, Clone)]
pub struct Noise {
    state: u64,
    spare: Option<f64>,
}

impl Noise {
    /// A source from `seed`.
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed,
            spare: None,
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in (0, 1].
    fn uniform(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 1.0) / (1u64 << 53) as f64
    }

    /// A normal sample with mean 0 and standard deviation `sigma`.
    pub fn gaussian(&mut self, sigma: f64) -> f64 {
        if let Some(spare) = self.spare.take() {
            return spare * sigma;
        }
        let radius = (-2.0 * self.uniform().ln()).sqrt();
        let angle = std::f64::consts::TAU * self.uniform();
        self.spare = Some(radius * angle.sin());
        radius * angle.cos() * sigma
    }

    /// Three independent normal samples.
    pub fn gaussian3(&mut self, sigma: f64) -> [f64; 3] {
        [
            self.gaussian(sigma),
            self.gaussian(sigma),
            self.gaussian(sigma),
        ]
    }
}
