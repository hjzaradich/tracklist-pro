"""Writes the golden outputs the Rust parity tests compare against.

    python -I golden.py --out DIR --download-dir DIR [--onnx PATH]

Synthetic signals only, each defined by a formula that is simple to rebuild exactly
(see the README next to the fixtures): sums of sines, a linear chirp, and noise from
an integer LCG. The reference is PyTorch with CED's own feature extractor, the same
model export.py exports.

  (a) a 16 kHz, 10 s signal: CED's mel (64 x 1001), emb (192) and logits (527);
  (b) a 44.1 kHz, 40 s signal through the whole-track path: three 10 s windows at
      25/50/75 % (start = clamp(p * duration - 5 s, 0, duration - 10 s)), resampled to
      16 kHz with soxr "HQ" (what librosa 1.0.0 uses by default), fingerprint = mean of
      the three window embeddings, label scores = mean of the three windows' logits.
"""
import argparse
import hashlib
import json
import math
import os
import sys
from importlib import metadata

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np
import torch

import export

TWO_PI = 2.0 * math.pi
WINDOW_SECONDS = 10.0
POSITIONS = (0.25, 0.50, 0.75)

# sines are (amplitude, frequency in Hz, phase in radians)
SIGNALS = {
    "parity_16k_10s": {
        "sample_rate": 16000,
        "seconds": 10,
        "sines": [(0.30, 220.0, 0.0), (0.20, 880.0, 0.5), (0.15, 3000.0, 0.0)],
        "chirp": {"amplitude": 0.20, "f0": 200.0, "f1": 6000.0},
        "noise": {"amplitude": 0.05, "seed": 12345},
    },
    "whole_track_44k_40s": {
        "sample_rate": 44100,
        "seconds": 40,
        "sines": [(0.30, 110.0, 0.0), (0.20, 440.0, 1.0), (0.10, 1760.0, 0.0), (0.10, 12000.0, 0.0)],
        "chirp": {"amplitude": 0.15, "f0": 300.0, "f1": 7000.0},
        "noise": {"amplitude": 0.05, "seed": 54321},
    },
}


def lcg_noise(count, seed):
    """Uniform in [-1, 1): state = (state * 1664525 + 1013904223) mod 2^32; sample i uses the
    state after i + 1 steps, its top 24 bits u (an integer): 2 * u / 2^24 - 1. Exact in f64."""
    out = np.empty(count, dtype=np.float64)
    state = seed
    for i in range(count):
        state = (state * 1664525 + 1013904223) & 0xFFFFFFFF
        out[i] = 2.0 * (state >> 8) / 16777216.0 - 1.0
    return out


def make_signal(spec):
    """The float32 signal. All arithmetic in f64, left to right, rounded to f32 once at the end:
    x[i] = a1*sin(2*pi*f1*t + p1) + ... + ac*sin(2*pi*(f0*t + (k/2)*t*t)) + an*noise[i], t = i / rate,
    k = (f1 - f0) / seconds."""
    rate, seconds = spec["sample_rate"], spec["seconds"]
    t = np.arange(rate * seconds, dtype=np.float64) / rate
    x = np.zeros_like(t)
    for amplitude, freq, phase in spec["sines"]:
        x = x + amplitude * np.sin(TWO_PI * freq * t + phase)
    c = spec["chirp"]
    k = (c["f1"] - c["f0"]) / seconds
    x = x + c["amplitude"] * np.sin(TWO_PI * (c["f0"] * t + (k / 2.0) * t * t))
    n = spec["noise"]
    x = x + n["amplitude"] * lcg_noise(len(t), n["seed"])
    return x.astype(np.float32)


def window_start(position, duration):
    return max(0.0, min(position * duration - WINDOW_SECONDS / 2.0, duration - WINDOW_SECONDS))


def run_model(fe, model, wave16):
    """(mel (64, 1001), emb (192,), logits (527,)) for exactly 10 s of 16 kHz float32."""
    assert wave16.dtype == np.float32 and len(wave16) == 160000
    with torch.no_grad():
        mel = fe(wave16, sampling_rate=16000, return_tensors="pt")["input_values"]
        logits, emb = model(mel)
    return mel[0].numpy(), emb[0].numpy(), logits[0].numpy()


def resample_soxr(wave, rate):
    import soxr

    return soxr.resample(wave, rate, 16000, quality="HQ").astype(np.float32)


def resample_other(wave, rate):
    """A different resampler (polyphase FIR), only to show how far the result moves with the resampler."""
    from scipy.signal import resample_poly

    g = math.gcd(rate, 16000)
    return resample_poly(wave.astype(np.float64), 16000 // g, rate // g).astype(np.float32)


def whole_track(fe, model, wave, rate, resample):
    y = resample(wave, rate)
    duration = len(y) / 16000.0
    starts, embs, logits = [], [], []
    for p in POSITIONS:
        start = window_start(p, duration)
        s = int(round(start * 16000))
        seg = np.zeros(160000, dtype=np.float32)
        piece = y[s:s + 160000]
        seg[:len(piece)] = piece
        _, e, lg = run_model(fe, model, seg)
        starts.append(start)
        embs.append(e)
        logits.append(lg)
    return starts, np.stack(embs), np.stack(logits)


def cosine(a, b):
    return float(np.dot(a, b) / (np.linalg.norm(a) * np.linalg.norm(b)))


def write_f32(path, array):
    np.ascontiguousarray(array, dtype="<f4").tofile(path)


def sha256_of(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def probes(x, indices):
    return [{"index": i, "value": float(x[i])} for i in indices]


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", required=True)
    ap.add_argument("--download-dir", required=True, help="where export.py's download goes (reused if already there)")
    ap.add_argument("--onnx", help="also check this ced-tiny.onnx against the reference (onnxruntime)")
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    torch.set_num_threads(2)

    model_dir = export.download(args.download_dir)
    fe, model = export.load(model_dir)
    session = None
    if args.onnx:
        import onnxruntime as ort

        session = ort.InferenceSession(args.onnx, providers=["CPUExecutionProvider"])

    def check_onnx(label, mel, emb, logits):
        if session is None:
            return
        lg, em = session.run(["logits", "emb"], {"mel": mel[None]})
        print(f"{label}: onnxruntime vs torch, max abs difference: logits {np.abs(lg[0] - logits).max():.2e}, emb {np.abs(em[0] - emb).max():.2e}")

    manifest = {
        "model": {"repo": export.REPO_ID, "revision": export.REVISION},
        "built_with": {p: metadata.version(p) for p in ("torch", "torchaudio", "transformers", "numpy", "soxr", "scipy", "onnxruntime")},
        "lcg": "state = (state * 1664525 + 1013904223) mod 2^32; sample i uses the state after i + 1 steps; noise = 2 * (state >> 8) / 2^24 - 1",
    }

    # (a)
    spec = SIGNALS["parity_16k_10s"]
    x = make_signal(spec)
    mel, emb, logits = run_model(fe, model, x)
    check_onnx("parity_16k_10s", mel, emb, logits)
    write_f32(os.path.join(args.out, "parity_16k_10s.mel.f32"), mel)
    write_f32(os.path.join(args.out, "parity_16k_10s.emb.f32"), emb)
    write_f32(os.path.join(args.out, "parity_16k_10s.logits.f32"), logits)
    manifest["parity_16k_10s"] = dict(
        spec,
        probes=probes(x, [0, 1, 2, 159, 12345, 80000, 159999]),
        signal_sha256_info_only=hashlib.sha256(x.astype("<f4").tobytes()).hexdigest(),
    )

    # (b)
    spec = SIGNALS["whole_track_44k_40s"]
    x44 = make_signal(spec)
    starts, embs, logs = whole_track(fe, model, x44, spec["sample_rate"], resample_soxr)
    mean_emb, mean_logits = embs.mean(0), logs.mean(0)
    write_f32(os.path.join(args.out, "whole_track_44k_40s.emb.f32"), mean_emb)
    write_f32(os.path.join(args.out, "whole_track_44k_40s.logits.f32"), mean_logits)
    write_f32(os.path.join(args.out, "whole_track_44k_40s.windows_emb.f32"), embs)
    write_f32(os.path.join(args.out, "whole_track_44k_40s.windows_logits.f32"), logs)
    manifest["whole_track_44k_40s"] = dict(
        spec,
        window_starts_seconds=starts,
        resampled_length=40 * 16000,
        probes=probes(x44, [0, 1, 2, 441, 44100, 882000, 1763999]),
        signal_sha256_info_only=hashlib.sha256(x44.astype("<f4").tobytes()).hexdigest(),
    )

    # How far do the outputs move when only the resampler changes? (for the test's tolerance)
    _, embs2, logs2 = whole_track(fe, model, x44, spec["sample_rate"], resample_other)
    me2, ml2 = embs2.mean(0), logs2.mean(0)
    manifest["resampler_sensitivity"] = {
        "compared": "soxr HQ (the fixture) versus scipy resample_poly (polyphase FIR)",
        "fingerprint_cosine": cosine(mean_emb, me2),
        "fingerprint_max_abs": float(np.abs(mean_emb - me2).max()),
        "logits_max_abs": float(np.abs(mean_logits - ml2).max()),
    }
    print("resampler sensitivity:", manifest["resampler_sensitivity"])

    manifest["files"] = {}
    for name in sorted(os.listdir(args.out)):
        if name.endswith(".f32"):
            path = os.path.join(args.out, name)
            manifest["files"][name] = {"bytes": os.path.getsize(path), "sha256": sha256_of(path)}
    with open(os.path.join(args.out, "manifest.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")
    print("window starts:", starts, "top logits (a):", np.argsort(-logits)[:3].tolist())


if __name__ == "__main__":
    main()
