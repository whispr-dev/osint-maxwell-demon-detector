# Maxwell's Demon Detector (Rust, live-capture build)

This is the realtime ASCII mapper with an embedded libpcap/Npcap capture adapter.

It can now:
- list capture interfaces
- capture live packets via **libpcap** on Linux and **Npcap** on Windows
- replay CSV event streams
- ingest live JSONL events on stdin
- render a rolling ASCII traffic landscape in realtime
- compute per-lane packet-load / packet-size entropy / anomaly scores

## What this build is

- a realtime terminal mapper
- a lawful packet-stream visualizer
- a single Rust codebase for Linux and Windows
- a proper front-end for the original live-map concept

## What this build is not

- a Wireshark replacement
- a full application-layer reassembler
- a DPI / TLS decryption suite
- a GUI

## Build prerequisites

### Linux

Install the libpcap development package before building. On Debian-family systems that is typically:

```bash
sudo apt-get install libpcap-dev
```

### Windows

Install **Npcap** and the **Npcap SDK**, then add the SDK `Lib` directory to your `LIB` environment variable before building.

## Commands

### 1. List capture interfaces

```bash
cargo run --release -- --mode list-interfaces
```

### 2. Live packet capture

Use the default capture device:

```bash
cargo run --release -- --mode live-pcap
```

Use a named interface:

```bash
cargo run --release -- --mode live-pcap --interface eth0
```

Use a listed numeric interface index:

```bash
cargo run --release -- --mode live-pcap --interface 0
```

Use a BPF filter:

```bash
cargo run --release -- --mode live-pcap --interface eth0 --filter "tcp or udp"
```

Example narrow capture:

```bash
cargo run --release -- --mode live-pcap --interface eth0 --filter "port 53 or port 80 or port 443"
```

### 3. Demo mode

```bash
cargo run --release -- --mode demo
```

### 4. Replay mode

```bash
cargo run --release -- --mode replay --input examples/events.csv --replay-speed 40
```

### 5. STDIN JSONL mode

```bash
cat examples/events.jsonl | cargo run --release -- --mode stdin-jsonl
```

## Live capture notes

- The capture adapter uses `pcap::Device::lookup()` when no interface is specified.
- Interface listing uses `pcap::Device::list()`.
- A configurable BPF filter is applied with `pcap::Capture::filter()` after the interface is opened.
- The capture thread uses a non-zero timeout to avoid UI starvation and to allow a clean shutdown.
- The decoder currently supports the most common link and network combinations needed for practical traffic mapping:
  - Ethernet II
  - Linux cooked capture (SLL)
  - RAW IP
  - NULL / LOOP loopback headers
  - IPv4 / IPv6
  - TCP / UDP / ICMP / ICMPv6

## Traffic lanes

Rows are classified into:
- `WEB`
- `DNS`
- `MAIL`
- `MEDIA`
- `CTRL`
- `BULK`
- `OTHER`

Classification uses decoded transport ports, lightweight protocol guessing, and packet size.

## Glyph semantics

A cell is **not** a single packet.

A cell is one time bucket for one lane. Its glyph blends:
- packet load
- packet-size Shannon entropy
- rolling anomaly state

`!` marks an anomalous bucket.

## Controls

- `q` or `Esc` quits
- `Ctrl+C` also works

## Useful options

```text
--tick-ms <ms>            redraw interval
--width <cols>            map width override
--max-alerts <n>          alert history depth
--snaplen <bytes>         capture snap length
--buffer-size <bytes>     libpcap/Npcap kernel buffer target
--pcap-timeout-ms <ms>    capture timeout used by the capture thread
--promisc <true|false>    promiscuous capture on/off
```

## Example event schema for replay/stdin modes

```json
{"ts_ms":0,"len":512,"proto":"tls","src":"10.0.0.2","dst":"93.184.216.34","sport":51514,"dport":443,"dir":"out"}
```

## Packaging and licensing note

This project links against libpcap on Linux and Npcap on Windows. If you distribute a Windows build, review the current Npcap licensing and redistribution terms carefully.
