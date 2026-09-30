"""Synthetic test tones for the rekordbox behavior check. Standard library only.

A tone is a WAV file that plays a I-IV-I-V cadence in one major key at 120 BPM,
with the root in the bass and a click on every beat, so rekordbox's analysis has
an unambiguous key and a clear beat. No real music is involved.
"""

import array
import math
import random
import sys
import wave
from functools import lru_cache
from pathlib import Path

RATE = 44100
BPM = 120
BEAT = RATE * 60 // BPM  # 22050 samples: whole, so every beat is identical
NOTE_NAMES = ("C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B")

# Semitones above the tonic: (chord tones, bass note). I-IV-I-V keeps the tonic on half the beats.
_CHORDS = {"I": ((0, 4, 7), 0), "IV": ((5, 9, 12), 5), "V": ((7, 11, 14), 7)}
_BAR = ("I", "IV", "I", "V")


def _freq(midi: float) -> float:
    return 440.0 * 2 ** ((midi - 69) / 12)


def _beat(tonic_midi: int, chord: str, click) -> list:
    tones, bass = _CHORDS[chord]
    parts = [(_freq(tonic_midi + t), 0.18) for t in tones]
    parts += [(_freq(tonic_midi + t + 12), 0.05) for t in tones]  # a little brightness
    parts.append((_freq(tonic_midi + bass - 24), 0.30))
    out = []
    for n in range(BEAT):
        env = min(1.0, n / 220) * math.exp(-2.2 * n / BEAT)
        s = env * sum(a * math.sin(2 * math.pi * f * n / RATE) for f, a in parts)
        if n < len(click):
            s += click[n]
        out.append(s)
    return out


@lru_cache(maxsize=None)
def _bar(tonic_pc: int) -> bytes:
    """One bar of 16-bit PCM; rendered once per key."""
    rng = random.Random(tonic_pc)
    click = [0.25 * (rng.random() * 2 - 1) * math.exp(-n / 60) for n in range(440)]
    tonic = 60 + tonic_pc  # the octave above middle C
    beats = {c: _beat(tonic, c, click) for c in _CHORDS}
    peak = max(abs(x) for b in beats.values() for x in b)
    pcm = {c: array.array("h", (int(x / peak * 0.8 * 32767) for x in b)) for c, b in beats.items()}
    if sys.byteorder == "big":
        for a in pcm.values():
            a.byteswap()
    return b"".join(pcm[c].tobytes() for c in _BAR)


def write_tone(dest: Path, tonic_pc: int, seconds: int = 48) -> None:
    """Writes a mono 16-bit WAV in the major key whose tonic is pitch class `tonic_pc` (C = 0).
    The output is deterministic: same arguments, same bytes."""
    bar = _bar(tonic_pc % 12)
    bars = seconds * BPM // 60 // len(_BAR)
    dest = Path(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(dest), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(bar * bars)
