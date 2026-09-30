#!/usr/bin/env python3

# Maxwell Demon Detector
# Detect 'Maxwell Monster' style structure in binary datasets via entropy/MI/compressibility scans.
#
# Run on a raw binary file (bits taken MSB-first per byte): python maxwell_monster_detector.py --file data.bin --window 8192 --step 2048 --maxlag 8 --csv out.csv
# How to use + what “flags” mean
# (Shannon entropy is computed as an uncertainty measure, so consistently low values indicate bias/structure).
# The output CSV has one row per window, with columns:
# A “flagged” window usually means either
# (a) bit bias/regularity reduced entropy, or
# (b) dependence raised mutual information,
# both of which are inconsistent with an IID fair-coin model of bits.
#
# Adjust --maxlag to capture longer-range dependencies if desired.
# Adjust --window and --step to tune resolution/speed tradeoff.
# Adjust --z and --cratio thresholds as desired to tune sensitivity.
# Dependencies: Python 3.6+ (standard library only).
#

import argparse
import csv
import math
import statistics
import sys
import zlib
from typing import List, Tuple, Optional


def bytes_to_bits(data: bytes) -> List[int]:
    out = []
    for b in data:
        for i in range(7, -1, -1):
            out.append((b >> i) & 1)
    return out


def bits_to_bytes(bits: List[int]) -> bytes:
    if not bits:
        return b""
    pad = (-len(bits)) % 8
    bits2 = bits + [0] * pad
    out = bytearray()
    for i in range(0, len(bits2), 8):
        v = 0
        for j in range(8):
            v = (v << 1) | (bits2[i + j] & 1)
        out.append(v)
    return bytes(out)


def binary_shannon_entropy(bits: List[int]) -> Tuple[float, float]:
    n = len(bits)
    if n == 0:
        return 0.0, 0.0
    ones = sum(bits)
    p1 = ones / n
    p0 = 1.0 - p1
    h = 0.0
    if p0 > 0.0:
        h -= p0 * math.log2(p0)
    if p1 > 0.0:
        h -= p1 * math.log2(p1)
    return h, p1


def mutual_information_lag(bits: List[int], lag: int) -> float:
    n = len(bits)
    if lag <= 0 or n <= lag:
        return 0.0

    c00 = c01 = c10 = c11 = 0
    for i in range(n - lag):
        a = bits[i]
        b = bits[i + lag]
        if a == 0 and b == 0:
            c00 += 1
        elif a == 0 and b == 1:
            c01 += 1
        elif a == 1 and b == 0:
            c10 += 1
        else:
            c11 += 1

    m = n - lag
    p00 = c00 / m
    p01 = c01 / m
    p10 = c10 / m
    p11 = c11 / m

    pa0 = (c00 + c01) / m
    pa1 = (c10 + c11) / m
    pb0 = (c00 + c10) / m
    pb1 = (c01 + c11) / m

    def term(pxy: float, px: float, py: float) -> float:
        if pxy <= 0.0 or px <= 0.0 or py <= 0.0:
            return 0.0
        return pxy * math.log2(pxy / (px * py))

    mi = 0.0
    mi += term(p00, pa0, pb0)
    mi += term(p01, pa0, pb1)
    mi += term(p10, pa1, pb0)
    mi += term(p11, pa1, pb1)
    return mi


def compress_ratio_bytes(payload: bytes, level: int = 9) -> float:
    if not payload:
        return 1.0
    comp = zlib.compress(payload, level)
    return len(comp) / len(payload)


def windows(bits: List[int], win: int, step: int):
    n = len(bits)
    if win <= 0 or step <= 0:
        return
    for start in range(0, max(0, n - win + 1), step):
        yield start, bits[start:start + win]


def parse_args():
    ap = argparse.ArgumentParser(
        description="Detect 'Maxwell Monster' style structure in binary datasets via entropy/MI/compressibility scans."
    )
    src = ap.add_mutually_exclusive_group(required=True)
    src.add_argument("--file", help="Path to a binary file (bytes will be expanded to bits, MSB-first).")
    src.add_argument("--bits", help="Bitstring like 010011... (whitespace allowed).")
    ap.add_argument("--window", type=int, default=8192, help="Window size in bits (default: 8192).")
    ap.add_argument("--step", type=int, default=2048, help="Step size in bits (default: 2048).")
    ap.add_argument("--maxlag", type=int, default=8, help="Compute MI for lags 1..maxlag (default: 8).")
    ap.add_argument("--z", type=float, default=3.0, help="Flag if entropy z-score <= -z (default: 3.0).")
    ap.add_argument("--cratio", type=float, default=0.98,
                    help="Also flag if compression_ratio <= cratio (default: 0.98). Lower means more compressible.")
    ap.add_argument("--csv", default="-", help="Output CSV path, or '-' for stdout (default).")
    return ap.parse_args()


def load_bits_from_args(args) -> List[int]:
    if args.file:
        with open(args.file, "rb") as f:
            data = f.read()
        return bytes_to_bits(data)

    s = "".join(ch for ch in args.bits if ch in "01")
    return [1 if ch == "1" else 0 for ch in s]


def main():
    args = parse_args()
    bits = load_bits_from_args(args)

    if len(bits) < args.window:
        raise SystemExit(f"Need at least {args.window} bits, got {len(bits)}.")

    # First pass: gather metrics per window
    rows = []
    entropies = []
    for start, wbits in windows(bits, args.window, args.step):
        h, p1 = binary_shannon_entropy(wbits)
        entropies.append(h)

        mi = [mutual_information_lag(wbits, k) for k in range(1, args.maxlag + 1)]
        payload = bits_to_bytes(wbits)
        cr = compress_ratio_bytes(payload)

        rows.append((start, start + args.window, h, p1, mi, cr))

    mu = statistics.mean(entropies)
    sd = statistics.pstdev(entropies)  # population stddev for stability
    if sd == 0.0:
        sd = 1e-12

    # Output
    out_f = sys.stdout if args.csv == "-" else open(args.csv, "w", newline="")
    try:
        w = csv.writer(out_f)
        header = ["start_bit", "end_bit", "entropy_bits_per_bit", "p1", "entropy_zscore"]
        header += [f"mi_lag{k}" for k in range(1, args.maxlag + 1)]
        header += ["compression_ratio", "flagged"]
        w.writerow(header)

        for (start, end, h, p1, mi, cr) in rows:
            zscore = (h - mu) / sd
            flagged = (zscore <= -abs(args.z)) or (cr <= args.cratio)
            w.writerow([start, end, f"{h:.6f}", f"{p1:.6f}", f"{zscore:.3f}"]
                       + [f"{x:.6f}" for x in mi]
                       + [f"{cr:.6f}", int(flagged)])
    finally:
        if out_f is not sys.stdout:
            out_f.close()


if __name__ == "__main__":
    main()

