# Changelog

## 0.3.0 (2026-09-30)

Rebuilt from the v0.2 ASCII mapper as a new crate (binary `mdd`).

### Fixed
- The default build no longer needs the Npcap SDK or libpcap: live capture moved behind `--features live`.
- Rows no longer overflow the terminal width (the old display wrapped into garbage).
- No staircase output in raw mode on Linux/WSL and no full-screen clear per frame (flicker): frames are diffed and drawn with cursor addressing inside synchronized updates.
- Windows key-release events are ignored (keys no longer fire twice); Ctrl+C quits; the terminal is restored on errors and panics.
- The DNS lane no longer swallows every UDP payload of 12+ bytes; ARP counts as control traffic.

### New
- A real landscape: height = volume, texture = structure class (sparse, noise, flow, order, mono), caps for rhythm and bursts, markers for mix shifts and demons.
- Statistics: bias-corrected entropy, permutation-tested lag-1 dependency, robust multi-lag rhythm detection, per-lane EWMA/CUSUM/KL detectors with warm-up.
- Sources: scripted demo, raw bytes (file or stdin), pcap/pcapng files (pure Rust), CSV/JSONL replay, stdin JSONL, live capture; `--record` to JSONL.
- Headless mode (`--headless --fast`) with an alert log, for CI and screenshots.
- 45 tests (40 unit, 5 end-to-end).
