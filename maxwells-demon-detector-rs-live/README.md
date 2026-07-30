<!-- repo-convergence:readme-header:start -->
<!-- repo-convergence:language=FILL_ME -->
# maxwell-demon-detector

<p align="center">
  <a href="https://github.com/whisprer/maxwell-demon-detector/releases">
    <img src="https://img.shields.io/github/v/release/whisprer/maxwell-demon-detector?color=4CAF50&label=release" alt="Release Version">
  </a>
  <a href="https://github.com/whisprer/maxwell-demon-detector/blob/main/LICENSE">
    <img src="https://img.shields.io/badge/license-Hybrid-green.svg" alt="License">
  </a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey.svg" alt="Platform">
  <a href="https://github.com/whisprer/maxwell-demon-detector/actions">
    <img src="https://img.shields.io/badge/build-workflow%20not%20set-lightgrey.svg" alt="Build Status">
  </a>
</p>

[![GitHub](https://img.shields.io/badge/GitHub-whisprer%2Fmaxwell-demon-detector-blue?logo=github&style=flat-square)](https://github.com/whisprer/maxwell-demon-detector)
![Commits](https://img.shields.io/github/commit-activity/m/whisprer/maxwell-demon-detector?label=commits)
![Last Commit](https://img.shields.io/github/last-commit/whisprer/maxwell-demon-detector)
![Issues](https://img.shields.io/github/issues/whisprer/maxwell-demon-detector)
[![Version](https://img.shields.io/badge/version-3.1.1-blue.svg)](https://github.com/whisprer/maxwell-demon-detector)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11-lightgrey.svg)](https://www.microsoft.com/windows)
[![Language](https://img.shields.io/badge/language-FILL_ME-blue.svg)](#)
[![Status](https://img.shields.io/badge/Status-Alpha%20Release-orange?style=flat-square)](#)

<p align="center">
  <img src="/assets/maxwell-demon-detector-banner.png" width="850" alt="maxwell-demon-detector Banner">
</p>
<!-- repo-convergence:readme-header:end -->

# entropy-forge

`entropy-forge` is a research-oriented Rust foundation for the first real software phase of an entropy-centric project.

It deliberately implements the parts we can validate **in software right now**:

- discrete Shannon / Rényi / Miller–Madow entropy
- first-order Markov entropy-rate estimation
- Lempel–Ziv complexity based entropy-rate approximation
- Sample Entropy (SampEn) for continuous time series
- Gaussian minimum-joint-entropy time-delay estimation (TDE) with cross-correlation baseline
- graph entropy and entropy-centrality for industrial sensor-network style structural analysis

It does **not** pretend to implement thermodynamic hardware. Instead, it builds the software math stack that can later feed a hardware/SPU path.

## Why this scope

Your literature bundle points to three immediately buildable software lanes:

1. **Entropy estimation**: parametric and non-parametric estimators are the mathematical base.
2. **Entropy-based TDE**: minimum joint entropy is the practical signal-processing win over pure correlation in noisy settings.
3. **Entropy centrality**: structural graph weighting is the cleanest anomaly-detection building block before any future GNN work.

That makes this crate the right Phase 1 foundation.

## Build

```bash
cargo build --release
```

## CLI examples

### 1) Discrete Shannon entropy from a CSV column

```bash
cargo run --release -- discrete --input examples/discrete_symbols.csv --column symbol
```

### 2) First-order Markov entropy rate

```bash
cargo run --release -- markov-rate --input examples/discrete_symbols.csv --column symbol
```

### 3) Lempel–Ziv entropy-rate approximation

```bash
cargo run --release -- lz-rate --input examples/discrete_symbols.csv --column symbol
```

### 4) Sample entropy for a numeric series

```bash
cargo run --release -- sample-entropy --input examples/sensor_pairs.csv --column ref --m 2 --r-ratio 0.2
```

### 5) Time-delay estimation

Positive lag means the target occurs **later** than the reference.

```bash
cargo run --release -- tde \
  --input examples/sensor_pairs.csv \
  --reference ref \
  --target target \
  --max-lag 8
```

### 6) Entropy centrality from a weighted directed graph

```bash
cargo run --release -- entropy-centrality \
  --input examples/graph_edges.csv \
  --src-col src \
  --dst-col dst \
  --weight-col weight
```

## Project layout

```text
entropy-forge-rs/
├── Cargo.toml
├── README.md
├── examples/
│   ├── discrete_symbols.csv
│   ├── graph_edges.csv
│   └── sensor_pairs.csv
└── src/
    ├── error.rs
    ├── io.rs
    ├── lib.rs
    ├── main.rs
    ├── stats.rs
    ├── entropy/
    │   ├── discrete.rs
    │   ├── lz.rs
    │   ├── markov.rs
    │   ├── mod.rs
    │   └── sample_entropy.rs
    ├── graph/
    │   ├── centrality.rs
    │   └── mod.rs
    └── signal/
        ├── mod.rs
        └── tde.rs
```

## Design notes

- The estimators use `base = 2` by default so the outputs are in **bits**.
- The TDE implementation uses the Gaussian minimum-joint-entropy formulation based on covariance log-determinant. It is the right first software implementation because it is mathematically tractable and directly comparable with normalized cross-correlation.
- The graph module focuses on **entropy centrality** and **system anomaly score** rather than a full GNN. That is intentional: it keeps the first build compact, testable, and extensible.
- For very large datasets, the non-parametric estimators here are a correctness-first baseline. The next optimization step would be streaming counters, SIMD kernels, batched covariance, and optional GPU / distributed execution.

## Next obvious extensions

- nearest-neighbor differential entropy estimation
- permutation entropy and transfer entropy
- multichannel TDE with joint lag search
- CSV + Parquet backends
- ndarray / BLAS acceleration path
- graph-learning layer for entropy-weighted message passing
