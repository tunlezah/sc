use tracing::warn;

/// Wraps the `opus` crate encoder for 48 kHz stereo at 128 kbps.
///
/// Each call to `encode_frame` expects a 20 ms frame of interleaved f32
/// PCM (960 samples per channel = 1920 values total). This matches the
/// frame size used by `AudioCapture`.
pub struct OpusEncoder {
    encoder: opus::Encoder,
    encode_buf: Vec<u8>,
}

impl OpusEncoder {
    /// Create a new Opus encoder configured for WebRTC audio streaming.
    pub fn new() -> Result<Self, opus::Error> {
        let mut encoder =
            opus::Encoder::new(48000, opus::Channels::Stereo, opus::Application::Audio)?;
        encoder.set_bitrate(opus::Bitrate::Bits(128_000))?;
        Ok(Self {
            encoder,
            // 4000 bytes is well above the max possible Opus frame size (~1275 bytes)
            encode_buf: vec![0u8; 4000],
        })
    }

    /// Encode a 20 ms frame of interleaved f32 PCM.
    ///
    /// `pcm` must contain exactly 1920 samples (960 per channel, interleaved).
    /// Returns the encoded Opus bytes, or `None` if encoding failed or produced
    /// zero bytes.
    pub fn encode_frame(&mut self, pcm: &[f32]) -> Option<Vec<u8>> {
        match self.encoder.encode_float(pcm, &mut self.encode_buf) {
            Ok(len) if len > 0 => Some(self.encode_buf[..len].to_vec()),
            Ok(_) => None,
            Err(e) => {
                warn!("Opus encode error: {}", e);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_encoder() {
        let encoder = OpusEncoder::new();
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_encode_silence() {
        let mut encoder = OpusEncoder::new().unwrap();
        let silence = vec![0.0f32; 1920]; // 20ms of stereo silence
        let result = encoder.encode_frame(&silence);
        // Opus should produce output even for silence (comfort noise)
        assert!(result.is_some());
        let data = result.unwrap();
        assert!(!data.is_empty());
        assert!(data.len() < 4000); // Should be much smaller than buffer
    }

    #[test]
    fn test_encode_tone() {
        let mut encoder = OpusEncoder::new().unwrap();
        // Generate a 1kHz sine wave at 48kHz, stereo interleaved
        let mut samples = Vec::with_capacity(1920);
        for i in 0..960 {
            let t = i as f32 / 48000.0;
            let sample = (2.0 * std::f32::consts::PI * 1000.0 * t).sin() * 0.5;
            samples.push(sample); // left
            samples.push(sample); // right
        }
        let result = encoder.encode_frame(&samples);
        assert!(result.is_some());
    }
}
