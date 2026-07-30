[README.md]

<p align="center">
  <a href="https://github.com/whispr-dev/maxwell-demon-detector">
    <img src="https://img.shields.io/github/stars/whispr-dev/maxwell-demon-detector?style=for-the-badge" alt="GitHub stars" />
  </a>
  <a href="https://github.com/whispr-dev/maxwell-demon-detector/issues">
    <img src="https://img.shields.io/github/issues/whispr-dev/maxwell-demon-detector?style=for-the-badge" alt="GitHub issues" />
  </a>
  <a href="https://github.com/whispr-dev/maxwell-demon-detector/fork">
    <img src="https://img.shields.io/github/forks/whispr-dev/maxwell-demon-detector?style=for-the-badge" alt="GitHub forks" />
  </a>
</p>

# maxwell-demon-detector
is my entropy accidentally non-random or sus?




Maxwell Demon Detector
This project is a small playground for poking at binary entropy, “suspicious” structure, and Maxwell’s demon / “Maxwell Monster” vibes in bitstreams. It provides:

A detector that scans .bin files for low-entropy / highly structured windows

A generator that spits out a test suite of binaries ranging from genuinely random to “cartoon demon” structure

The whole thing is aimed at exploring how information-theoretic ideas (Shannon entropy, mutual information, compressibility) show up in actual binary data.
​

1. Maxwell Demon Detector
maxwell_monster_detector.py takes a binary file, cracks it into bits (MSB-first per byte), slides a window over it, and computes:

Shannon entropy per bit in each window (
H
(
p
)
H(p) via the binary entropy function).
​

Mutual information at small lags 
I
(
X
t
;
X
t
+
k
)
I(X 
t
 ;X 
t+k
 ) to detect dependency/feedback in the bitstream.
​

Compression ratio using zlib as a cheap “structure density” proxy.
​

Each window gets a z-score relative to the file’s entropy distribution and is flagged if:

Entropy z-score is below a configurable cutoff (e.g. ≤ −3), or

Compression ratio is below a threshold (i.e. compresses “too well”).

Flagged windows are good candidates for “someone/something is acting like a demon here” — injecting order, bias, or dependency into what might otherwise be random-looking data.
​

1.1 Usage
Basic scan:

bash
python maxwell_monster_detector.py \
  --file path/to/data.bin \
  --window 8192 \
  --step 2048 \
  --maxlag 8 \
  --csv out.csv
Key options:

--file: input .bin (raw bytes; bits decoded MSB-first per byte)

--window: window size in bits (default 8192)

--step: step size in bits between windows (default 2048)

--maxlag: compute mutual information for lags 1..maxlag (default 8)
​

--z: entropy z-score cutoff (default 3.0)

--cratio: compression-ratio cutoff (default 0.98; lower = more compressible)

--csv: output CSV (or - for stdout)

The CSV contains one row per window with fields like:

start_bit, end_bit

entropy_bits_per_bit, p1

entropy_zscore

mi_lag1 … mi_lagN

compression_ratio

flagged (0/1)

Interpretation:

Low entropy, strong bias (p1 far from 0.5), small compression_ratio → strong structure / “ordered” region.
​

High mutual information for small lags → bits depend on previous bits, hinting at feedback or a non-trivial generator (Markov, LFSR, etc.).
​

2. Test Binary Suite Generator
gen_testbins.py builds a small zoo of .bin files to exercise the detector: from OS randomness to extremely low-entropy patterns and structured pseudorandom streams. It uses os.urandom() for crypto-grade noise and various generators for “sus” patterns.
​

2.1 Generated files
By default (64 KiB for most samples), it creates in testbins/:

00_urandom_64k.bin

Baseline randomness from the OS entropy source (should be hardest to flag).
​

01_mt19937_64k.bin

Bytes from Python’s random.getrandbits() (Mersenne Twister PRNG).
​

10_all_zeros_64k.bin

Obvious low-entropy, max-order sample (ideal sanity check for the scanner).

11_alternating_01_64k.bin

Bit pattern 0101… across the file; strongly periodic.

12_repeating_AA_64k.bin

Repeating 0xAA (10101010); similar periodicity at byte level.

13_biased_p10_64k.bin

Bits with 
P
(
1
)
=
0.10
P(1)=0.10; reduced Shannon entropy compared to unbiased bits.
​

14_markov_sticky_64k.bin

A simple Markov chain: bit tends to repeat previous value (“sticky” state).

High mutual information at lag 1, low entropy per bit.
​

15_lfsr_prbs_64k.bin

16-bit LFSR-based pseudorandom sequence; looks fairly noisy but fully linear and predictable with state reconstruction.
​

99_demon_sandwich_128k.bin

Larger file built as: random chunk → low-entropy patches (zeros, ones, 0xAA, heavy bias, sticky Markov) → random again.

Designed to produce a nice pattern of “boring” windows with clusters of “demon found here” flags.

A MANIFEST.txt is also written with size, SHA-256, and a short description for each file.

2.2 Usage
Generate suite:

bash
python gen_testbins.py --outdir ./testbins
Optional parameters:

--size: base size in bytes for most samples (default 65536 = 64 KiB)

--seed: base seed for deterministic generators (MT, biased bits, Markov)

Listing files (PowerShell example):

powershell
python .\gen_testbins.py --outdir .\testbins
Get-ChildItem .\testbins\*.bin
Get-Content .\testbins\MANIFEST.txt
3. Typical Workflow
Generate test data:

bash
python gen_testbins.py --outdir ./testbins
Scan everything:

bash
for f in testbins/*.bin; do
    python maxwell_monster_detector.py \
        --file "$f" \
        --window 8192 \
        --step 2048 \
        --maxlag 8 \
        --csv "${f%.bin}.csv"
done
Inspect flagged windows in the CSVs and verify that:

Pure urandom / MT look mostly unflagged or only weakly structured.
​

All-zero / alternating / biased / Markov / LFSR / “demon sandwich” sections stand out strongly according to entropy and mutual information.
​

4. Conceptual Notes
Shannon entropy here is the standard information-theoretic measure of uncertainty per bit; for unbiased IID bits it’s 1, dropping as the distribution skews or becomes more predictable.
​

Mutual information between bit positions is used as a simple proxy for “feedback” or structure that a Maxwell demon / controller would exploit.
​

Compressibility via zlib is a practical shortcut: strongly structured data compresses more than high-entropy noise, which is why compression is widely used as an empirical entropy heuristic.
​

In other words, the detector is looking for windows where some “Maxwell Monster” could in principle be extracting work or compressing the data more than a generic random model would allow, given the observed structure in the bitstream.
​