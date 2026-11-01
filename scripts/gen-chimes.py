#!/usr/bin/env python3
"""Synthesize the two alert chimes shipped by the `alerts` plugin.

Standard library only (wave, math, struct). Writes mono, 22050 Hz, 16-bit
WAVs next to the plugin so they can be embedded with include_bytes!:

    python3 scripts/gen-chimes.py            # writes plugins/alerts/assets/
    python3 scripts/gen-chimes.py OUT_DIR    # or somewhere else

needs-input: two rising notes, E5 (659.26 Hz) then A5 (880 Hz), 140 ms apart:
  a "hey, look" sound.
done:        one soft lower note, C5 (523.25 Hz) with a quiet G5 (783.99 Hz)
  fifth overtone: a calm "finished" sound.

Each note is a sine with a faint 2nd partial, a 5 ms raised-cosine attack and
an exponential decay (a bell), faded to zero at the end so nothing clicks.
Each file is scaled to a peak of -6 dBFS (half of full scale).
"""
import math
import struct
import sys
import wave
from pathlib import Path

RATE = 22050
PEAK = 0.5  # -6.02 dBFS
ATTACK = 0.005  # seconds
TAIL_FADE = 0.030  # seconds, raised-cosine to zero at the very end

# Exact equal-temperament frequencies (A4 = 440 Hz).
C5, E5, G5, A5 = 523.2511, 659.2551, 783.9909, 880.0


def note(freq, length, decay, partials=((1.0, 1.0), (2.0, 0.18))):
    """One bell-like note: `length` seconds, exponential decay time `decay`."""
    n = int(RATE * length)
    out = []
    for i in range(n):
        t = i / RATE
        tone = sum(a * math.sin(2 * math.pi * freq * m * t) for m, a in partials)
        env = math.exp(-t / decay)
        if t < ATTACK:
            env *= 0.5 - 0.5 * math.cos(math.pi * t / ATTACK)
        left = length - t
        if left < TAIL_FADE:
            env *= 0.5 - 0.5 * math.cos(math.pi * left / TAIL_FADE)
        out.append(tone * env)
    return out


def mix(parts, total):
    """Sum notes placed at (start seconds, samples) into `total` seconds."""
    buf = [0.0] * int(RATE * total)
    for start, samples in parts:
        off = int(RATE * start)
        for i, s in enumerate(samples):
            if off + i < len(buf):
                buf[off + i] += s
    return buf


def write(path, samples):
    peak = max(abs(s) for s in samples)
    scale = PEAK * 32767 / peak
    pcm = b"".join(struct.pack("<h", round(s * scale)) for s in samples)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(pcm)


def needs_input():
    return mix([(0.0, note(E5, 0.26, 0.07)), (0.14, note(A5, 0.26, 0.08))], 0.40)


def done():
    return mix(
        [
            (0.0, note(C5, 0.46, 0.13)),
            (0.0, [0.3 * s for s in note(G5, 0.46, 0.10, partials=((1.0, 1.0),))]),
        ],
        0.46,
    )


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent / "plugins/alerts/assets"
    out.mkdir(parents=True, exist_ok=True)
    write(out / "chime-needs-input.wav", needs_input())
    write(out / "chime-done.wav", done())


if __name__ == "__main__":
    main()
