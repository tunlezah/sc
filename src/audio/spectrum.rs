use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::state::{AppStateHandle, SystemEvent};

const FFT_SIZE: usize = 2048;
const SAMPLE_RATE: f32 = 48000.0;
const NUM_BANDS: usize = 64;
const SMOOTHING_ALPHA: f32 = 0.35;
const MIN_DB: f32 = -80.0;

/// Spectrum analyzer that processes PCM audio and produces 64 frequency bands.
pub struct SpectrumAnalyzer {
    state: AppStateHandle,
    smoothed_bands: [f32; NUM_BANDS],
    fft: Arc<dyn rustfft::Fft<f32>>,
    window: [f32; FFT_SIZE],
}

impl SpectrumAnalyzer {
    pub fn new(state: AppStateHandle) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);

        // Hanning window
        let mut window = [0.0f32; FFT_SIZE];
        for (i, w) in window.iter_mut().enumerate() {
            *w =
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32).cos());
        }

        Self {
            state,
            smoothed_bands: [0.0; NUM_BANDS],
            fft,
            window,
        }
    }

    /// Run the spectrum analyzer, consuming audio frames from the capture broadcast.
    pub async fn run(mut self, mut audio_rx: broadcast::Receiver<Vec<f32>>) {
        info!("Spectrum analyzer started");
        let mut accumulator: Vec<f32> = Vec::with_capacity(FFT_SIZE);

        loop {
            match audio_rx.recv().await {
                Ok(samples) => {
                    // Convert stereo to mono by averaging channels
                    for chunk in samples.chunks(2) {
                        if chunk.len() == 2 {
                            accumulator.push((chunk[0] + chunk[1]) * 0.5);
                        } else if !chunk.is_empty() {
                            accumulator.push(chunk[0]);
                        }
                    }

                    // Process when we have enough samples
                    while accumulator.len() >= FFT_SIZE {
                        let frame: Vec<f32> = accumulator.drain(..FFT_SIZE).collect();
                        let bands = self.process_frame(&frame);
                        self.state.publish(SystemEvent::SpectrumData { bands });
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("Spectrum analyzer lagged, dropped {} frames", n);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    info!("Spectrum analyzer: audio channel closed");
                    break;
                }
            }
        }
    }

    fn process_frame(&mut self, samples: &[f32]) -> Vec<f32> {
        // Apply window and convert to complex
        let mut buffer: Vec<Complex<f32>> = samples
            .iter()
            .zip(self.window.iter())
            .map(|(s, w)| Complex::new(s * w, 0.0))
            .collect();

        // FFT
        self.fft.process(&mut buffer);

        // Convert to magnitude spectrum (only positive frequencies)
        let half = FFT_SIZE / 2;
        let magnitudes: Vec<f32> = buffer[..half]
            .iter()
            .map(|c| c.norm() / half as f32)
            .collect();

        // Map to logarithmically-spaced bands
        let bands = map_to_bands(&magnitudes, SAMPLE_RATE, NUM_BANDS);

        // Apply smoothing
        for (i, band) in bands.iter().enumerate() {
            self.smoothed_bands[i] =
                SMOOTHING_ALPHA * band + (1.0 - SMOOTHING_ALPHA) * self.smoothed_bands[i];
        }

        self.smoothed_bands.to_vec()
    }
}

/// Map FFT bins to logarithmically-spaced frequency bands (20 Hz to 20 kHz).
/// Output is normalized to 0.0-1.0 where 0.0 = -80 dBFS and 1.0 = 0 dBFS.
fn map_to_bands(magnitudes: &[f32], sample_rate: f32, num_bands: usize) -> Vec<f32> {
    let min_freq: f32 = 20.0;
    let max_freq: f32 = 20000.0;
    let log_min = min_freq.ln();
    let log_max = max_freq.ln();
    let half = magnitudes.len();
    let bin_width = sample_rate / (2.0 * half as f32);

    (0..num_bands)
        .map(|i| {
            let freq_low = (log_min + (log_max - log_min) * i as f32 / num_bands as f32).exp();
            let freq_high =
                (log_min + (log_max - log_min) * (i + 1) as f32 / num_bands as f32).exp();

            let bin_low = (freq_low / bin_width).floor() as usize;
            let bin_high = (freq_high / bin_width).ceil() as usize;
            let bin_low = bin_low.max(1).min(half - 1);
            let bin_high = bin_high.max(bin_low + 1).min(half);

            // Average magnitudes in this band
            let sum: f32 = magnitudes[bin_low..bin_high].iter().sum();
            let avg = sum / (bin_high - bin_low) as f32;

            // Convert to dB and normalize
            let db = if avg > 0.0 {
                20.0 * avg.log10()
            } else {
                MIN_DB
            };

            // Normalize: -80 dB → 0.0, 0 dB → 1.0
            ((db - MIN_DB) / -MIN_DB).clamp(0.0, 1.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_to_bands_count() {
        let magnitudes = vec![0.01; 1024];
        let bands = map_to_bands(&magnitudes, 48000.0, 64);
        assert_eq!(bands.len(), 64);
    }

    #[test]
    fn test_map_to_bands_range() {
        let magnitudes = vec![0.5; 1024];
        let bands = map_to_bands(&magnitudes, 48000.0, 64);
        for band in &bands {
            assert!(
                *band >= 0.0 && *band <= 1.0,
                "Band value out of range: {}",
                band
            );
        }
    }

    #[test]
    fn test_map_to_bands_silence() {
        let magnitudes = vec![0.0; 1024];
        let bands = map_to_bands(&magnitudes, 48000.0, 64);
        for band in &bands {
            assert_eq!(*band, 0.0);
        }
    }

    #[test]
    fn test_map_to_bands_full_scale() {
        let magnitudes = vec![1.0; 1024];
        let bands = map_to_bands(&magnitudes, 48000.0, 64);
        for band in &bands {
            assert_eq!(*band, 1.0);
        }
    }
}
