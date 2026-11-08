//! Which 10 seconds of a track the model listens to, and the stretch of
//! native-rate audio each window is cut from.

/// Length of one window, in seconds (CED's own clip length).
pub const WINDOW_SECONDS: f64 = 10.0;
/// Where in the track the windows sit, as fractions of its duration. Each
/// window is centred on its position.
pub const POSITIONS: [f64; 3] = [0.25, 0.50, 0.75];
/// Extra audio each side of a window, in seconds, so the resampler has
/// settled by the time the window starts.
pub const MARGIN_SECONDS: f64 = 0.1;

/// Where the three windows start, in seconds: `p x duration - 5`, held
/// between 0 and `duration - 10`. A track shorter than 10 seconds starts all
/// three at 0 (and is zero-padded).
pub fn window_starts(duration: f64) -> [f64; 3] {
    POSITIONS.map(|p| {
        (p * duration - WINDOW_SECONDS / 2.0)
            .min(duration - WINDOW_SECONDS)
            .max(0.0)
    })
}

/// The first native-rate frame a window's stretch holds (negative when the
/// window starts within the margin of the track's start: zeros come first).
pub fn first_frame(start: f64, rate: u32) -> i64 {
    ((start - MARGIN_SECONDS) * f64::from(rate)).round_ties_even() as i64
}

/// How many native-rate frames a window's stretch holds: the window and a
/// margin each side.
pub fn stretch_len(rate: u32) -> usize {
    ((WINDOW_SECONDS + 2.0 * MARGIN_SECONDS) * f64::from(rate)).round() as usize
}

/// The stretch for the window at `start` from audio already in memory;
/// frames the audio doesn't hold are zeros.
pub fn stretch_of(mono: &[f32], rate: u32, start: f64) -> Vec<f32> {
    let first = first_frame(start, rate);
    (first..first + stretch_len(rate) as i64)
        .map(|i| {
            usize::try_from(i)
                .ok()
                .and_then(|i| mono.get(i))
                .copied()
                .unwrap_or(0.0)
        })
        .collect()
}
