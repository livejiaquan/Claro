"""
Standalone logic test for the voice-to-text audio pipeline.

Tests without microphone/hotkey/GUI:
  1. RMS calculation on silence, noise, speech
  2. Audio concatenation
  3. Audio length / energy rejection thresholds
  4. Full simulation of _do_stop_and_transcribe flow
"""

import io
import sys
import time

import numpy as np

# Reuse functions from main
sys.path.insert(0, ".")
from main import _rms, SAMPLE_RATE

PASS = 0
FAIL = 0


def check(name: str, ok: bool, detail: str = ""):
    global PASS, FAIL
    if ok:
        PASS += 1
        print(f"  ✅ {name}")
    else:
        FAIL += 1
        print(f"  ❌ {name}  {detail}")


# ─── 1. RMS basic values ──────────────────────────────────────────────────

print("\n=== RMS function ===")

silence = np.zeros(SAMPLE_RATE, dtype=np.float32)
check("silence RMS == 0", _rms(silence) == 0.0)

quiet_noise = np.random.default_rng(42).normal(0, 0.001, SAMPLE_RATE).astype(np.float32)
rms_q = _rms(quiet_noise)
check(f"quiet noise RMS ({rms_q:.6f}) < 0.02", rms_q < 0.02)

loud_noise = np.random.default_rng(42).normal(0, 0.1, SAMPLE_RATE).astype(np.float32)
rms_l = _rms(loud_noise)
check(f"loud noise RMS ({rms_l:.4f}) > 0.02", rms_l > 0.02)

speech = np.random.default_rng(42).normal(0, 0.08, SAMPLE_RATE).astype(np.float32)
rms_s = _rms(speech)
check(f"speech-like RMS ({rms_s:.4f}) > 0.02", rms_s > 0.02)

# ─── 2. Audio concatenation ───────────────────────────────────────────────

print("\n=== Audio concatenation ===")

chunk1 = np.ones(SAMPLE_RATE // 2, dtype=np.float32)
chunk2 = np.ones(SAMPLE_RATE // 2, dtype=np.float32) * 0.5
concat = np.concatenate([chunk1, chunk2])
check("concat length == 1 sec", len(concat) == SAMPLE_RATE)
check("concat first half all 1.0", np.allclose(concat[:SAMPLE_RATE // 2], 1.0))
check("concat second half all 0.5", np.allclose(concat[SAMPLE_RATE // 2:], 0.5))

# ─── 3. Length and RMS thresholds ─────────────────────────────────────────

print("\n=== Threshold logic ===")

short_audio = np.zeros(int(SAMPLE_RATE * 0.2), dtype=np.float32)
check("0.2s < 0.3s too short", len(short_audio) < SAMPLE_RATE * 0.3)

long_audio = np.zeros(int(SAMPLE_RATE * 0.5), dtype=np.float32)
check("0.5s >= 0.3s OK length", len(long_audio) >= SAMPLE_RATE * 0.3)

# Audio that should be rejected by RMS
noise_silent = (np.random.default_rng(42).normal(0, 0.005, SAMPLE_RATE * 2).astype(np.float32))
rms_ns = _rms(noise_silent)
check(f"near-silence RMS ({rms_ns:.4f}) < 0.02 → reject", rms_ns < 0.02)

# Audio that should pass RMS
noise_loud = (np.random.default_rng(42).normal(0, 0.05, SAMPLE_RATE * 2).astype(np.float32))
rms_nl = _rms(noise_loud)
check(f"loud-noise RMS ({rms_nl:.4f}) >= 0.02 → accept", rms_nl >= 0.02)

# ─── 4. Simulation of full pipeline ───────────────────────────────────────

print("\n=== Full pipeline simulation ===")

# Set a debug flag on main module
import main as M

audio_good = (np.random.default_rng(42).normal(0, 0.1, SAMPLE_RATE * 2).astype(np.float32))
audio_bad = (np.random.default_rng(42).normal(0, 0.001, SAMPLE_RATE * 2).astype(np.float32))
audio_short = np.zeros(int(SAMPLE_RATE * 0.2), dtype=np.float32)

# Simulate the checks in _do_stop_and_transcribe
def simulate(audio):
    if len(audio) == 0:
        return "empty"
    if len(audio) < SAMPLE_RATE * 0.3:
        return "too_short"
    if _rms(audio) < 0.02:
        return "too_quiet"
    return "pass"

check("good audio → pass", simulate(audio_good) == "pass")
check("bad/silent audio → too_quiet", simulate(audio_bad) == "too_quiet")
check("short audio → too_short", simulate(audio_short) == "too_short")

# ─── Summary ──────────────────────────────────────────────────────────────

total = PASS + FAIL
print(f"\n{'='*40}")
print(f"  {PASS}/{total} passed", flush=True)

if FAIL > 0:
    print(f"  {FAIL} FAILURES", flush=True)
    sys.exit(1)
else:
    print("  All checks passed.", flush=True)
