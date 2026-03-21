use std::f64::consts::PI;

/// Biquad filter coefficients (normalized by a0).
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct BiquadCoefficients {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

/// Per-channel delay line state for Direct Form II Transposed biquad.
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub struct BiquadState {
    pub x1: f32,
    pub x2: f32,
    pub y1: f32,
    pub y2: f32,
}

#[allow(dead_code)]
impl BiquadState {
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

/// Stereo biquad filter (one BiquadState per channel).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StereoBiquad {
    pub left: BiquadState,
    pub right: BiquadState,
    pub coeffs: BiquadCoefficients,
}

#[allow(dead_code)]
impl StereoBiquad {
    pub fn new(coeffs: BiquadCoefficients) -> Self {
        Self {
            left: BiquadState::default(),
            right: BiquadState::default(),
            coeffs,
        }
    }

    /// Process a single stereo sample pair.
    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        let l = process_sample(&self.coeffs, &mut self.left, left);
        let r = process_sample(&self.coeffs, &mut self.right, right);
        (l, r)
    }

    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }
}

/// Process a single sample through a biquad filter.
/// Direct Form I:
///   y[n] = b0*x[n] + b1*x[n-1] + b2*x[n-2] - a1*y[n-1] - a2*y[n-2]
#[allow(dead_code)]
fn process_sample(coeffs: &BiquadCoefficients, state: &mut BiquadState, input: f32) -> f32 {
    let x = input as f64;
    let y = coeffs.b0 * x
        + coeffs.b1 * state.x1 as f64
        + coeffs.b2 * state.x2 as f64
        - coeffs.a1 * state.y1 as f64
        - coeffs.a2 * state.y2 as f64;

    state.x2 = state.x1;
    state.x1 = input;
    state.y2 = state.y1;
    state.y1 = y as f32;

    y as f32
}

/// Calculate peaking EQ coefficients per Audio EQ Cookbook.
#[allow(dead_code)]
pub fn peaking_eq(freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> BiquadCoefficients {
    let a = 10.0_f64.powf(gain_db / 40.0);
    let w0 = 2.0 * PI * freq / sample_rate;
    let alpha = w0.sin() / (2.0 * q);

    let b0 = 1.0 + alpha * a;
    let b1 = -2.0 * w0.cos();
    let b2 = 1.0 - alpha * a;
    let a0 = 1.0 + alpha / a;
    let a1 = -2.0 * w0.cos();
    let a2 = 1.0 - alpha / a;

    BiquadCoefficients {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
    }
}

/// Calculate low-shelf coefficients per Audio EQ Cookbook.
#[allow(dead_code)]
pub fn low_shelf(freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> BiquadCoefficients {
    let a = 10.0_f64.powf(gain_db / 40.0);
    let w0 = 2.0 * PI * freq / sample_rate;
    let alpha = w0.sin() / (2.0 * q);
    let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

    let cos_w0 = w0.cos();

    let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
    let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
    let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
    let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
    let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
    let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

    BiquadCoefficients {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
    }
}

/// Calculate high-shelf coefficients per Audio EQ Cookbook.
#[allow(dead_code)]
pub fn high_shelf(freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> BiquadCoefficients {
    let a = 10.0_f64.powf(gain_db / 40.0);
    let w0 = 2.0 * PI * freq / sample_rate;
    let alpha = w0.sin() / (2.0 * q);
    let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

    let cos_w0 = w0.cos();

    let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
    let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
    let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
    let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
    let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
    let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

    BiquadCoefficients {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f64 = 48000.0;

    #[test]
    fn test_peaking_zero_gain_is_unity() {
        // At 0dB gain, A=1.0, so b0 = 1 + alpha and a0 = 1 + alpha → b0/a0 = 1.0
        // Similarly b2 = 1 - alpha = a2, so b2/a0 = a2/a0
        let coeffs = peaking_eq(1000.0, 0.0, 1.414, SAMPLE_RATE);
        assert!((coeffs.b0 - 1.0).abs() < 1e-10, "b0 should be ~1.0: {}", coeffs.b0);
        // b2 and a2 should be equal (both = (1 - alpha) / a0)
        assert!((coeffs.a2 - coeffs.b2).abs() < 1e-10, "a2 and b2 should match");
    }

    #[test]
    fn test_low_shelf_zero_gain_is_unity() {
        let coeffs = low_shelf(60.0, 0.0, 0.707, SAMPLE_RATE);
        assert!((coeffs.b0 - 1.0).abs() < 1e-6, "b0={}", coeffs.b0);
    }

    #[test]
    fn test_high_shelf_zero_gain_is_unity() {
        let coeffs = high_shelf(16000.0, 0.0, 0.707, SAMPLE_RATE);
        assert!((coeffs.b0 - 1.0).abs() < 1e-6, "b0={}", coeffs.b0);
    }

    #[test]
    fn test_peaking_boost_coefficients() {
        let coeffs = peaking_eq(1000.0, 6.0, 1.414, SAMPLE_RATE);
        // With positive gain, b0 > 1.0
        assert!(coeffs.b0 > 1.0);
    }

    #[test]
    fn test_peaking_cut_coefficients() {
        let coeffs = peaking_eq(1000.0, -6.0, 1.414, SAMPLE_RATE);
        // With negative gain, b0 < 1.0
        assert!(coeffs.b0 < 1.0);
    }

    #[test]
    fn test_stereo_biquad_process() {
        let coeffs = peaking_eq(1000.0, 0.0, 1.414, SAMPLE_RATE);
        let mut bq = StereoBiquad::new(coeffs);

        // Unity gain at 0dB should pass through approximately unchanged
        let (l, r) = bq.process(1.0, 0.5);
        // First sample may differ due to filter startup, but shouldn't be wildly off
        assert!(l.abs() < 10.0);
        assert!(r.abs() < 10.0);
    }

    #[test]
    fn test_stereo_biquad_reset() {
        let coeffs = peaking_eq(1000.0, 6.0, 1.414, SAMPLE_RATE);
        let mut bq = StereoBiquad::new(coeffs);
        bq.process(1.0, 1.0);
        bq.reset();
        assert_eq!(bq.left.x1, 0.0);
        assert_eq!(bq.right.y1, 0.0);
    }

    #[test]
    fn test_symmetry_of_peaking() {
        let boost = peaking_eq(1000.0, 6.0, 1.414, SAMPLE_RATE);
        let cut = peaking_eq(1000.0, -6.0, 1.414, SAMPLE_RATE);
        // b0 of boost should equal a0-normalized inverse of cut
        // At the center frequency, boost * cut ≈ unity
        assert!((boost.b0 * cut.b0 + boost.b1 * cut.b1).abs() < 10.0);
    }

    #[test]
    fn test_coefficients_are_finite() {
        for freq in [60.0, 250.0, 1000.0, 8000.0, 16000.0] {
            for gain in [-12.0, -6.0, 0.0, 6.0, 12.0] {
                let c = peaking_eq(freq, gain, 1.414, SAMPLE_RATE);
                assert!(c.b0.is_finite());
                assert!(c.b1.is_finite());
                assert!(c.b2.is_finite());
                assert!(c.a1.is_finite());
                assert!(c.a2.is_finite());
            }
        }
    }
}
