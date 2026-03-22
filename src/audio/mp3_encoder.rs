use mp3lame_encoder::{Builder, Encoder, FlushNoGap, InterleavedPcm};
use tracing::warn;

/// Wraps the `mp3lame-encoder` crate for 48 kHz stereo at 192 kbps CBR.
///
/// Each call to `encode_frame` expects interleaved f32 PCM samples
/// (960 samples per channel = 1920 values total for a 20 ms frame at 48 kHz).
/// Returns encoded MP3 bytes suitable for HTTP streaming to Chromecast devices.
pub struct Mp3Encoder {
    encoder: Encoder,
    mp3_buf: Vec<u8>,
}

impl Mp3Encoder {
    /// Create a new MP3 encoder configured for Chromecast audio streaming.
    pub fn new() -> Result<Self, String> {
        let mut builder = Builder::new().ok_or("Failed to create MP3 encoder builder")?;
        builder
            .set_sample_rate(48_000)
            .map_err(|e| format!("MP3 set sample rate error: {:?}", e))?;
        builder
            .set_num_channels(2)
            .map_err(|e| format!("MP3 set channels error: {:?}", e))?;
        builder
            .set_brate(mp3lame_encoder::Bitrate::Kbps192)
            .map_err(|e| format!("MP3 set bitrate error: {:?}", e))?;
        builder
            .set_quality(mp3lame_encoder::Quality::Best)
            .map_err(|e| format!("MP3 set quality error: {:?}", e))?;

        let encoder = builder
            .build()
            .map_err(|e| format!("MP3 encoder build error: {:?}", e))?;

        // MP3 buffer used by encode_to_vec/flush_to_vec
        let mp3_buf = Vec::with_capacity(8192);

        Ok(Self { encoder, mp3_buf })
    }

    /// Encode interleaved f32 PCM samples to MP3 bytes.
    ///
    /// `pcm` should contain interleaved stereo f32 samples (e.g. 1920 values for 20ms at 48kHz).
    /// Returns the encoded MP3 bytes, or `None` if encoding failed or produced zero bytes.
    pub fn encode_frame(&mut self, pcm: &[f32]) -> Option<Vec<u8>> {
        // Convert f32 samples to i16 for LAME (which expects integer PCM)
        let pcm_i16: Vec<i16> = pcm
            .iter()
            .map(|&s| {
                let clamped = s.clamp(-1.0, 1.0);
                (clamped * 32767.0) as i16
            })
            .collect();

        let input = InterleavedPcm(&pcm_i16);
        self.mp3_buf.clear();

        match self.encoder.encode_to_vec(input, &mut self.mp3_buf) {
            Ok(len) if len > 0 => Some(self.mp3_buf[..len].to_vec()),
            Ok(_) => None,
            Err(e) => {
                warn!("MP3 encode error: {:?}", e);
                None
            }
        }
    }

    /// Flush any remaining MP3 data from the encoder.
    /// Call this when the stream is ending to get the final MP3 bytes.
    pub fn flush(&mut self) -> Option<Vec<u8>> {
        self.mp3_buf.clear();
        match self.encoder.flush_to_vec::<FlushNoGap>(&mut self.mp3_buf) {
            Ok(len) if len > 0 => Some(self.mp3_buf[..len].to_vec()),
            Ok(_) => None,
            Err(e) => {
                warn!("MP3 flush error: {:?}", e);
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
        let encoder = Mp3Encoder::new();
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_encode_silence() {
        let mut encoder = Mp3Encoder::new().unwrap();
        let silence = vec![0.0f32; 1920]; // 20ms of stereo silence at 48kHz
        let result = encoder.encode_frame(&silence);
        // MP3 encoder may buffer initial frames, so first call may return None
        // Encode a few frames to ensure output
        let mut got_output = result.is_some();
        for _ in 0..10 {
            if encoder.encode_frame(&silence).is_some() {
                got_output = true;
            }
        }
        assert!(got_output);
    }

    #[test]
    fn test_encode_tone() {
        let mut encoder = Mp3Encoder::new().unwrap();
        // Generate a 1kHz sine wave at 48kHz, stereo interleaved
        let mut samples = Vec::with_capacity(1920);
        for i in 0..960 {
            let t = i as f32 / 48000.0;
            let sample = (2.0 * std::f32::consts::PI * 1000.0 * t).sin() * 0.5;
            samples.push(sample); // left
            samples.push(sample); // right
        }
        let mut got_output = false;
        for _ in 0..10 {
            if encoder.encode_frame(&samples).is_some() {
                got_output = true;
            }
        }
        assert!(got_output);
    }

    #[test]
    fn test_flush() {
        let mut encoder = Mp3Encoder::new().unwrap();
        let silence = vec![0.0f32; 1920];
        for _ in 0..5 {
            encoder.encode_frame(&silence);
        }
        // Flush should not panic; may or may not produce output
        let _ = encoder.flush();
    }
}
