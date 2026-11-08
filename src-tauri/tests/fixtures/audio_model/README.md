# Audio model: golden outputs

Reference outputs of PyTorch with CED's own code (Hugging Face `mispeech/ced-tiny`, revision in `manifest.json`) for two synthetic signals. The Rust parity tests rebuild each signal from the formulas below, run the app's front end and model, and compare. Made by `tools/ml/ced_tiny/golden.py` (deterministic; rerun it to check). `manifest.json` has the parameters, the versions used, probe samples, and the size and sha256 of every `.f32` file.

All `.f32` files are raw little-endian float32, no header, row-major.

| File | Shape | What |
|---|---|---|
| `parity_16k_10s.mel.f32` | 64 x 1001 | CED's mel features of signal A: mel band (row) by frame (column) |
| `parity_16k_10s.emb.f32` | 192 | the embedding of signal A |
| `parity_16k_10s.logits.f32` | 527 | the label scores of signal A (before any sigmoid) |
| `whole_track_44k_40s.emb.f32` | 192 | signal B's fingerprint: mean of its 3 window embeddings |
| `whole_track_44k_40s.logits.f32` | 527 | signal B's label scores: mean of its 3 windows' logits |
| `whole_track_44k_40s.windows_emb.f32` | 3 x 192 | the 3 window embeddings (to tell a window mistake from the averaging) |
| `whole_track_44k_40s.windows_logits.f32` | 3 x 527 | the 3 windows' logits |

The window starts of B are in `manifest.json` (`window_starts_seconds`): 5, 15 and 25 seconds.

## The signals

Both are built the same way. With `t = i / rate` (f64), for sample `i` from 0, evaluate left to right in f64, add the terms in this order, and round to f32 once at the end:

```
x[i] = a1 * sin(2*pi*f1*t + p1) + a2 * sin(2*pi*f2*t + p2) + ...      (the sines, in the order listed)
     + ac * sin(2*pi*(f0*t + (k/2)*t*t))                               (the linear chirp, k = (f1 - f0) / seconds)
     + an * noise[i]
```

`2*pi*f*t` is `((2*pi)*f)*t`. The noise is an integer LCG, so every language gets the same numbers:

```
state = seed (u32)
for each sample i:  state = (state * 1664525 + 1013904223) mod 2^32
                    noise[i] = 2 * (state >> 8) / 16777216 - 1        (in [-1, 1), exact in f64)
```

**Signal A** (`parity_16k_10s`): 16 000 Hz, 10 s = 160 000 samples.

| Part | Amplitude | Frequency | Phase |
|---|---|---|---|
| sine | 0.30 | 220 Hz | 0 |
| sine | 0.20 | 880 Hz | 0.5 |
| sine | 0.15 | 3000 Hz | 0 |
| chirp | 0.20 | 200 Hz up to 6000 Hz over the 10 s (k = 580) | |
| noise | 0.05 | LCG seed 12345 | |

**Signal B** (`whole_track_44k_40s`): 44 100 Hz, 40 s = 1 764 000 samples.

| Part | Amplitude | Frequency | Phase |
|---|---|---|---|
| sine | 0.30 | 110 Hz | 0 |
| sine | 0.20 | 440 Hz | 1.0 |
| sine | 0.10 | 1760 Hz | 0 |
| sine | 0.10 | 12000 Hz (above the model's 8 kHz: the resampler must remove it) | 0 |
| chirp | 0.15 | 300 Hz up to 7000 Hz over the 40 s (k = 167.5) | |
| noise | 0.05 | LCG seed 54321 | |

Peak amplitude is at most 0.9 in both. `manifest.json` lists a few sample values (`probes`) to check a Rust reimplementation against (agreement to about 1e-6 is enough; the last bit of a sine depends on the maths library). `signal_sha256_info_only` is the hash of the f32 bytes on the machine that made the fixtures; a different libm can differ in a last bit, so don't make a test depend on it.

## The model's input (what the mel file is)

CED's own `feature_extraction_ced.py` (torchaudio): 16 kHz mono in; STFT with a 512-point periodic Hann window, 512-point FFT, hop 160, centred (reflect padding of 256 each side, so 160 000 samples give 1001 frames); power spectrum (magnitude squared); 64 HTK mel bands from 0 to 8000 Hz, no normalisation; then decibels, `10 * log10(max(x, 1e-10))`, with a floor 120 dB below the loudest value of the clip. That is the reference for the app's front end; the mel file is what it should reproduce. (An independent f64 implementation of exactly this description agrees with the file to 0.0015 dB at most; the values run from about -34 to +35 dB.)

## Whole-track path (signal B)

Resample the 44.1 kHz signal to 16 kHz (the fixture used soxr "HQ", which is librosa 1.0.0's default; exactly 640 000 samples), so `duration = 40 s`. Three 10 s windows at 25, 50 and 75 %: `start = clamp(p * duration - 5, 0, duration - 10)` seconds, sample offset `round(start * 16000)` (round half away from zero; the starts here are 5, 15 and 25 s, exact multiples of 16000, so no tie arises, and real tracks should use the same rule), 160 000 samples each (zero-padded if the track is shorter). Each window goes through the front end and the model; the fingerprint is the mean of the 3 `emb` outputs and the label scores are the mean of the 3 `logits` outputs.

## Tolerance guidance

- onnxruntime versus PyTorch on the same mel differs by about 1.3e-5 on the logits (their values reach tens) and 2e-6 on the embedding. A different engine (`tract`) will differ at that scale; the logits are the larger number, so compare them relatively.
- The resampler is the biggest uncertainty in B. Swapping soxr "HQ" for a plain polyphase FIR (scipy `resample_poly`) moved the 40 s fingerprint to cosine 0.99988 (largest single embedding value moved by 0.058) and the label scores by up to 0.26, so test B on cosine similarity and on the label ranking, and test A (no resampling) tightly. The window starts are exact.
