#!/usr/bin/env python3
import argparse
import os
import random
import hashlib
from pathlib import Path

def bits_to_bytes(bits):
    pad = (-len(bits)) % 8
    if pad:
        bits = bits + [0] * pad
    out = bytearray()
    for i in range(0, len(bits), 8):
        v = 0
        for j in range(8):
            v = (v << 1) | (bits[i + j] & 1)
        out.append(v)
    return bytes(out)

def gen_mt_bytes(nbytes, seed=123456789):
    rng = random.Random(seed)
    return bytes((rng.getrandbits(8) for _ in range(nbytes)))

def gen_alternating_bits(nbits, start=0):
    return [(start + i) & 1 for i in range(nbits)]

def gen_biased_bits(nbits, p1=0.10, seed=1):
    rng = random.Random(seed)
    return [1 if rng.random() < p1 else 0 for _ in range(nbits)]

def gen_markov_sticky_bits(nbits, stay_prob=0.97, seed=2):
    rng = random.Random(seed)
    x = rng.getrandbits(1)
    out = [x]
    for _ in range(nbits - 1):
        if rng.random() >= stay_prob:
            x ^= 1
        out.append(x)
    return out

def gen_lfsr_bits(nbits, seed=0xACE1):
    # 16-bit Fibonacci LFSR with taps 16,14,13,11 (common maximal-length choice).
    # Not crypto-secure; intended to look "random-ish" but be linear/structured.
    state = seed & 0xFFFF
    if state == 0:
        state = 1
    out = []
    for _ in range(nbits):
        lsb = state & 1
        out.append(lsb)
        # feedback = XOR of tapped bits (0-based from LSB): 0,2,3,5 corresponds to 16,14,13,11 in MSB notation
        feedback = (state ^ (state >> 2) ^ (state >> 3) ^ (state >> 5)) & 1
        state = (state >> 1) | (feedback << 15)
    return out

def write_bin(path: Path, data: bytes):
    path.write_bytes(data)
    sha = hashlib.sha256(data).hexdigest()
    return sha

def main():
    ap = argparse.ArgumentParser(description="Generate small .bin files as a demon-detector test suite.")
    ap.add_argument("--outdir", default="testbins", help="Output directory (default: testbins).")
    ap.add_argument("--size", type=int, default=65536, help="Default size in bytes for 64k samples (default: 65536).")
    ap.add_argument("--seed", type=int, default=123456789, help="Base seed for deterministic generators.")
    args = ap.parse_args()

    outdir = Path(args.outdir)
    outdir.mkdir(parents=True, exist_ok=True)

    manifest = []

    # 0) True random baseline
    data = os.urandom(args.size)
    manifest.append(("00_urandom_64k.bin", "OS urandom baseline", write_bin(outdir / "00_urandom_64k.bin", data)))

    # 1) MT19937-ish sample
    data = gen_mt_bytes(args.size, seed=args.seed)
    manifest.append(("01_mt19937_64k.bin", "Python random.getrandbits bytes", write_bin(outdir / "01_mt19937_64k.bin", data)))

    # 10) All zeros
    data = bytes([0x00]) * args.size
    manifest.append(("10_all_zeros_64k.bin", "All zeros", write_bin(outdir / "10_all_zeros_64k.bin", data)))

    # 11) Alternating bits 0101...
    bits = gen_alternating_bits(args.size * 8, start=0)
    data = bits_to_bytes(bits)
    manifest.append(("11_alternating_01_64k.bin", "0101... alternating bits", write_bin(outdir / "11_alternating_01_64k.bin", data)))

    # 12) Repeating byte 0xAA
    data = bytes([0xAA]) * args.size
    manifest.append(("12_repeating_AA_64k.bin", "0xAA repeat", write_bin(outdir / "12_repeating_AA_64k.bin", data)))

    # 13) Biased bits
    bits = gen_biased_bits(args.size * 8, p1=0.10, seed=args.seed + 13)
    data = bits_to_bytes(bits)
    manifest.append(("13_biased_p10_64k.bin", "Bits with p(1)=0.10", write_bin(outdir / "13_biased_p10_64k.bin", data)))

    # 14) Markov sticky bits (high dependence)
    bits = gen_markov_sticky_bits(args.size * 8, stay_prob=0.97, seed=args.seed + 14)
    data = bits_to_bytes(bits)
    manifest.append(("14_markov_sticky_64k.bin", "Markov sticky bits (stay_prob=0.97)", write_bin(outdir / "14_markov_sticky_64k.bin", data)))

    # 15) LFSR PRBS-like
    bits = gen_lfsr_bits(args.size * 8, seed=0xACE1)
    data = bits_to_bytes(bits)
    manifest.append(("15_lfsr_prbs_64k.bin", "16-bit LFSR stream", write_bin(outdir / "15_lfsr_prbs_64k.bin", data)))

    # 99) Demon sandwich: random + injected low-entropy patches aligned to typical window sizes
    chunk = 8192  # bytes (so 8192*8 bits)
    parts = []
    parts.append(os.urandom(chunk))                # random
    parts.append(bytes([0x00]) * chunk)            # extreme order
    parts.append(bytes([0xFF]) * chunk)            # extreme order
    parts.append(bytes([0xAA]) * chunk)            # periodic
    parts.append(bits_to_bytes(gen_biased_bits(chunk * 8, p1=0.02, seed=args.seed + 99)))  # strong bias
    parts.append(bits_to_bytes(gen_markov_sticky_bits(chunk * 8, stay_prob=0.995, seed=args.seed + 199)))  # dependence
    parts.append(os.urandom(chunk))                # random again
    data = b"".join(parts)
    manifest.append(("99_demon_sandwich_128k.bin", "Random with injected order patches", write_bin(outdir / "99_demon_sandwich_128k.bin", data)))

    # Write manifest
    man_path = outdir / "MANIFEST.txt"
    with man_path.open("w", encoding="utf-8", newline="\n") as f:
        for name, desc, sha in manifest:
            f.write(f"{name}\t{len((outdir/name).read_bytes())}\tsha256={sha}\t{desc}\n")

    print(f"Wrote {len(manifest)} files to: {outdir.resolve()}")
    print(f"Manifest: {man_path.resolve()}")

if __name__ == "__main__":
    main()
