//! Native-rate window stretch to the model's 16 kHz.

use rubato::{FftFixedInOut, Resampler};

use super::mel::{RATE, WINDOW_SAMPLES};
use super::window::MARGIN_SECONDS;

/// A stretch (a window and the margin each side, at `rate`) to exactly the
/// window's 160 000 samples at 16 kHz. `None` when the resampler can't be
/// built for this rate.
pub fn to_window(stretch: &[f32], rate: u32) -> Option<Vec<f32>> {
    let margin = (MARGIN_SECONDS * RATE as f64).round() as usize;
    if rate as usize == RATE {
        // Nothing to convert; just drop the margins.
        return stretch
            .get(margin..margin + WINDOW_SAMPLES)
            .map(<[f32]>::to_vec);
    }
    let mut resampler = FftFixedInOut::<f32>::new(rate as usize, RATE, 1024, 1).ok()?;
    let delay = resampler.output_delay();
    let need = ((10.0 + 2.0 * MARGIN_SECONDS) * RATE as f64).round() as usize + delay;
    let mut out: Vec<f32> = Vec::with_capacity(need + 4096);
    let mut chunk: Vec<f32> = Vec::new();
    let mut pos = 0;
    while out.len() < need {
        let n = resampler.input_frames_next();
        chunk.clear();
        chunk.resize(n, 0.0);
        let take = n.min(stretch.len().saturating_sub(pos));
        chunk[..take].copy_from_slice(&stretch[pos..pos + take]);
        pos += n;
        let done = resampler.process(&[&chunk[..]], None).ok()?;
        out.extend_from_slice(&done[0]);
    }
    out.get(delay + margin..delay + margin + WINDOW_SAMPLES)
        .map(<[f32]>::to_vec)
}
