"""Hone voice gate, reference implementation (spec 2f layer 5, option 2: mute speech blocks).

Input: one 5 s frame at 32 kHz (float32, -1..1).
Output: boolean mask per 100 ms block (True = speech, mute it) and whether to drop the frame.

Method (no third-party VAD):
  1. Low-pass to ~1 kHz and decimate to 8 kHz. Human voice pitch (70-500 Hz) and its first
     harmonics live here; most birdsong is above 1 kHz and is removed.
     A voiced frame must also carry most of its energy below 1 kHz (LOW_SHARE_DB), which
     speech does and high-pitched birdsong does not.
  2. 32 ms analysis frames every 10 ms. A frame is voiced when its normalized autocorrelation
     peak over lags for 70-500 Hz is >= VOICED_R and its energy is above both an absolute floor
     and the window's own noise floor + FLOOR_DB.
  3. A 100 ms block is speech when it has >= MIN_VOICED voiced frames.
  4. Speech blocks are widened by MARGIN_BLOCKS on each side (covers unvoiced consonants and
     onsets/offsets), then muted.
  5. If more than DROP_FRACTION of the blocks are speech, the whole frame is dropped.
"""
import numpy as np

SR = 32000
BLOCK = 3200            # 100 ms at 32 kHz
DEC = 4                 # 32 kHz -> 8 kHz
SR8 = SR // DEC
WIN = 256               # 32 ms at 8 kHz
HOP = 80                # 10 ms at 8 kHz
LAG_MIN = SR8 // 500    # 16  -> 500 Hz
LAG_MAX = SR8 // 70     # 114 -> 70 Hz
VOICED_R = 0.65
ABS_FLOOR_DB = -70.0
FLOOR_DB = 6.0
LOW_SHARE_DB = -8.0     # voiced frame's <1 kHz energy must be within 8 dB of its full-band energy
LOUD_DB = -50.0         # loud voiced frames count even with no quieter pause in the window
MIN_VOICED = 3
MARGIN_BLOCKS = 3
DROP_FRACTION = 0.5
NFFT = 512


def _lowpass_taps(n=63, cutoff_hz=1000.0, sr=SR):
    m = np.arange(n) - (n - 1) / 2
    fc = cutoff_hz / sr
    h = 2 * fc * np.sinc(2 * fc * m) * np.hamming(n)
    return (h / h.sum()).astype(np.float64)


TAPS = _lowpass_taps()


def voiced_frames(x):
    """Per 10 ms hop: (voiced bool, autocorr peak r, energy dB)."""
    y = np.convolve(x.astype(np.float64), TAPS, mode="same")[::DEC]
    n = 1 + (len(y) - WIN) // HOP
    idx = np.arange(WIN)[None, :] + HOP * np.arange(n)[:, None]
    f = y[idx]
    f = f - f.mean(axis=1, keepdims=True)
    f = f * np.hanning(WIN)[None, :]
    e = (f ** 2).sum(axis=1)
    edb = 10 * np.log10(e / WIN + 1e-20)
    spec = np.fft.rfft(f, NFFT, axis=1)
    ac = np.fft.irfft(np.abs(spec) ** 2, NFFT, axis=1)[:, : LAG_MAX + 1]
    # unbiased-ish normalization so longer lags are not penalized by the window overlap
    norm = ac[:, :1] + 1e-20
    lags = np.arange(LAG_MAX + 1)
    unbias = WIN / np.maximum(WIN - lags, 1)
    r = (ac / norm) * unbias[None, :]
    peak = r[:, LAG_MIN : LAG_MAX + 1].max(axis=1)
    floor = np.percentile(edb, 10)
    # full-band energy over the same 32 ms span, at 32 kHz
    xf = x.astype(np.float64)
    fidx = np.arange(WIN * DEC)[None, :] + HOP * DEC * np.arange(n)[:, None]
    fidx = np.minimum(fidx, len(xf) - 1)
    full_db = 10 * np.log10((xf[fidx] ** 2).mean(axis=1) + 1e-20)
    share = edb - full_db
    voiced = (peak >= VOICED_R) & (edb >= ABS_FLOOR_DB) & ((edb >= floor + FLOOR_DB) | (edb >= LOUD_DB)) & (share >= LOW_SHARE_DB)
    return voiced, peak, edb


def speech_mask(x):
    """Returns (mask per 100 ms block, drop_frame)."""
    nblocks = len(x) // BLOCK
    voiced, _, _ = voiced_frames(x)
    hops_per_block = BLOCK // DEC // HOP  # 10
    counts = np.zeros(nblocks, dtype=int)
    for b in range(nblocks):
        counts[b] = voiced[b * hops_per_block : (b + 1) * hops_per_block].sum()
    core = counts >= MIN_VOICED
    mask = core.copy()
    for b in np.flatnonzero(core):
        mask[max(0, b - MARGIN_BLOCKS) : b + MARGIN_BLOCKS + 1] = True
    return mask, bool(mask.mean() > DROP_FRACTION)


def apply(x, mask):
    y = x.copy()
    for b in np.flatnonzero(mask):
        y[b * BLOCK : (b + 1) * BLOCK] = 0.0
    return y
