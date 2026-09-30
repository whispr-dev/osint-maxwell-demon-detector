# Maxwell's Demon Detector (Rust MVP)

This is the **actual realtime mapper foundation**, not the earlier offline math-only helper crate.

It renders a live ASCII traffic landscape in the terminal. Each column is one time bucket. Each row is a traffic lane. Each cell blends:

- packet load in that bucket
- packet-size Shannon entropy in that bucket
- a simple rolling anomaly score

`!` marks an anomalous bucket.

## What this build is

- a realtime terminal mapper
- an animated ASCII landscape
- a lawful stream consumer
- a foundation for later integrated capture adapters

## What this build is not yet

- a raw packet sniffer by itself
- a full Wireshark replacement
- a GUI app
- deep protocol reconstruction

## Modes

### 1. Demo

Synthetic live traffic with periodic bursts and anomalies.

```bash
cargo run --release -- --mode demo
```

### 2. Replay

Replay a CSV event stream as if it were live.

```bash
cargo run --release -- --mode replay --input examples/events.csv --replay-speed 40
```

### 3. STDIN JSONL

Consume live event objects from standard input.

```bash
cat examples/events.jsonl | cargo run --release -- --mode stdin-jsonl
```

Each line must be a JSON object like:

```json
{"ts_ms":0,"len":512,"proto":"tls","src":"10.0.0.2","dst":"93.184.216.34","sport":51514,"dport":443,"dir":"out"}
```

## Why STDIN JSONL matters

This keeps the mapper platform-agnostic and lightweight.

You can later feed it from:
- a Rust capture adapter
- tshark output transformed into JSONL
- Zeek or Suricata logs converted to events
- a replay file
- a custom lawful packet observer

So this is already the **real map program**, with capture intentionally decoupled.

## Event schema

| field | type | meaning |
|---|---:|---|
| `ts_ms` | integer? | replay timestamp in milliseconds |
| `len` | integer | packet length |
| `proto` | string | protocol hint such as `tls`, `dns`, `rtp`, `smtp`, `ssh`, `icmp` |
| `src` | string? | source address |
| `dst` | string? | destination address |
| `sport` | integer? | source port |
| `dport` | integer? | destination port |
| `dir` | string? | `in` or `out` |

## Traffic lanes

Rows are classified into:
- `WEB`
- `DNS`
- `MAIL`
- `MEDIA`
- `CTRL`
- `BULK`
- `OTHER`

Classification uses ports, protocol hints, and packet size.

## Glyph semantics

A cell is **not** a single packet.

A cell is one time bucket for one lane. Its glyph is computed from:
- load intensity
- packet-size entropy
- rolling z-score anomaly state

This is the practical version of the original idea: still precise, but usable at real rates.

## Controls

- `q` or `Esc` to quit
- `Ctrl+C` also works

## Suggested next step

Keep this mapper as the front-end truth, and add a separate adapter crate later for:
- Linux packet capture
- Windows capture
- file replay
- structured log ingestion

That avoids turning one clean program into two messy platform-specific ones.
