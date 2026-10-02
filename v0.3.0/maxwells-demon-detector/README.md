# Maxwell's Demon Detector (`mdd`) v0.3.0

A realtime ASCII landscape of the **structure** flowing through a data stream:
network packets, event logs, or the raw bytes of any file.

Think of an SDR waterfall turned on its side. Time flows right to left (newest
column on the right). Each horizontal band is one **lane** of the stream. How tall
a column is tells you how busy that lane was; **which symbol it is built from tells
you what kind of structure the data had** at that moment: random, loosely
patterned, rigidly ordered, one-note, metronomic. When order suddenly appears
where there was none (the "demon" sorting molecules), a red `@` hovers over it.

Sample: the built-in demo at 18 s, while a DNS tunnel runs (real output of
`mdd --headless --fast --frames 144 --width 104 --height 30`):

```text
 MAXWELL'S DEMON DETECTOR | demo 00:18/1:00 DNS tunnel -> watch DNS | 349 ev/s 85.0KB/s | tick 125ms
                                                       ~~~~~~~~~~~~ ~~~~~~~~~~~ flow               349/s
            =                               =         ~~~~~~~~~~~~~~~~~~~~~~~~~ H 2.14 dep z    9.0
      ~~~~~ = ~~~~========  =  ==============~~~~~~~=~~~~~~~~~~~~~~~~~~~~~~~~~~ Hr 1.91  cv   -
      ~~~~~===~~~~========~==================~~~~~~~=~~~~~~~~~~~~~~~~~~~~~~~~~~ KL 0.22    85.0KB/s
      ~~~~~===~~~~========~==================~~~~~~~=~~~~~~~~~~~~~~~~~~~~~~~~~~
  ALL ~~~~~===~~~~========~==================~~~~~~~=~~~~~~~~~~~~~~~~~~~~~~~~~~
                                                                                idle                 2/s
  WEB _____;'______':'__::______'__:__________,__,____;_________:~__~_____~_~__ H 2.97 dep z    5.1
                               =            = =       =^^^^^^^^^^^@@@@@@@@@@@@@ ORDER @|           208/s
  DNS ==______=_=_=====_=______=_=_===_==__==_=__=___========================== H 1.28 dep z   55.6
                                                                         :      idle                 3/s
 MAIL __________________________________________________________________';:____ H 3.24 dep z   -0.7
                                               |                                ORDER |            100/s
MEDIA |||||||||||||||||||||||||||||||||||||||||=||||||||||||||||||||||||||||||| H 1.00 dep z  127.3
        ~~       ~    ~                  ~~  ~    ~   ~  ~~~    ~~       ~    ~ flow                14/s
 CTRL ~_~~~_~_~_~~__~_~~~~______~~_______~~~~~~__~~__~~~_~~~_~_~~~~_____~~~~~~~ H 1.85 dep z   22.2
         ~  ~  ~            ~       ~           ~  ~     ~     ~  ~   ~      ~~ flow                18/s
 BULK __~~__~_~~~~___~~_~~__~__~~___~~___~___~_~~__~_~_~~~__~~_~_~~_~~~~_~~~~~~ H 0.90 dep z   50.6
                                                                                noise                3/s
OTHER ___________.___..___.__________________.__.__._______.____;___:________'; H 3.11 dep z    0.1
      --------+-8s------------+-6s------------+-4s------------+-2s----------now
 _ idle  . sparse  :; noise  ~ flow  = order  # mono  | rhythm  ^ burst  ! shift  @ demon
 height = volume
 00:16 DNS   @ order appeared: entropy rate 1.26 -> 0.62 bits/sym (dep z 98.0)
 00:15 DNS   | rhythm locked: repeats every 10ms (cv 0.00)
 00:15 DNS   ^ burst: 24 events in one tick (typical ~0.7)
 00:00 MEDIA | rhythm locked: repeats every 20ms (cv 0.06)
```

## Quick start (Windows 11, PowerShell)

1. Install Rust from <https://rustup.rs> (run `rustup-init.exe`; let it install the
   Visual Studio C++ Build Tools when it asks). Rust 1.87 or newer.
2. Unzip this folder and open PowerShell in it (`maxwells-demon-detector`).
3. Run the demo:

   ```powershell
   cargo run --release
   ```

Use **Windows Terminal** (the default on Windows 11) for colour and smooth updates.
The finished program is `.\target\release\mdd.exe`; every command below uses it.

Keys: `space` pause, `+` / `-` scroll faster / slower, `l` legend, `c` colour,
`q` / `Esc` / `Ctrl+C` quit. Resizing the window is handled live.

## Reading the landscape

```text
   time flows <-- from right to left (newest column on the right)

         @      marker   @ demon: order appeared     ! shift: the mix changed
         |      cap      | rhythm: metronomic timing ^ burst: volume spike
         =      body     # mono   = order   ~ flow   :;', noise   . sparse
         =
   DNS ______   ground   _ nothing happened in this lane this tick
```

**Height** = volume relative to that lane's own typical level (a typical tick stands
at about 70% of the band; twice typical fills it). Absolute rates are in the side panel.

| Glyph | Name   | Exact rule (per lane, over its last 256 observations) |
|-------|--------|--------------------------------------------------------|
| `_`   | idle   | no events in this tick |
| `.`   | sparse | fewer than 16 observations: not enough to judge |
| `:;',`| noise  | symbols mixed, no sequential structure (dependency z < 3) |
| `~`   | flow   | real but loose sequential structure (dependency z >= 3) |
| `=`   | order  | the previous symbol strongly predicts the next (z >= 5 and >= 25% of the entropy explained) |
| `#`   | mono   | one symbol is >= 80% of the window |
| `\|`  | rhythm | gaps between events are regular (robust cv <= 0.15 for 3 ticks), at lag 1, 2 or 3 |
| `^`   | burst  | volume far above this lane's own baseline (z >= 3.5) |
| `!`   | shift  | the symbol mix moved away from this lane's baseline (KL divergence) |
| `@`   | demon  | entropy rate fell well below this lane's normal range: order appeared |

Side panel, per lane: class, active flags, events/s, then
`H` entropy (bits per symbol, 0 to 4), `dep z` (how far real order beats shuffled
copies; above 5 is strong), `Hr` entropy rate (the unpredictable part left after
using the previous symbol), `cv` timing spread (0 = metronome, about 1 = random),
`KL` distance of the current mix from the lane's baseline, and bytes/s.

**Lanes.** Packet modes: `ALL` (overview) plus `WEB DNS MAIL MEDIA CTRL BULK OTHER`,
chosen by port and protocol; each packet's symbol is its size bin (8 bins) times its
direction (in/out). Bytes mode: `ALL` plus `ZERO CTRL TEXT HIGH FF` byte classes;
class lanes use the byte's low nibble, `ALL` uses the high nibble.

## The demo (60-second loop)

| Time    | What happens | Where to look |
|---------|--------------|---------------|
| always  | a voice call (RTP, 50 packets/s each way) | `MEDIA`: `=` with a `\|` fence and **no** `@`. Order that is normal is not a demon |
| 0-15 s  | background only (web, DNS, mail, SSH, pings, a sync client) | detectors learn each lane's normal ("learning" in the title) |
| 15-25 s | DNS tunnel: fixed-size query/response ping-pong, 100 pairs/s | `DNS`: `^` burst, `=` order, `@` demon |
| 30-42 s | C2 beacon: request/response every 400 ms | `OTHER`: `\|` rhythm locks after a few seconds |
| 48-54 s | exfiltration: flood of identical 1280-byte packets | `BULK`: `#` mono, `^` burst, `!` shift, `@` demon |

`--seed N` gives a different (but repeatable) run of the same script.

## Feeding it your own data

```powershell
# Raw bytes of any file (the original Maxwell's-demon question): where is there order?
.\target\release\mdd.exe --mode bytes --input examples\demon_sandwich.bin --loop
.\target\release\mdd.exe --mode bytes --input C:\Windows\explorer.exe --rate 16384

# A packet capture (.pcap or .pcapng from Wireshark, tcpdump or pktmon)
.\target\release\mdd.exe --mode pcap-file --input C:\temp\cap.pcapng --speed 4

# An event log (CSV or JSONL, schema below)
.\target\release\mdd.exe --mode replay --input examples\events.csv --loop

# Record what you watch, replay it later
.\target\release\mdd.exe --record C:\temp\session.jsonl
.\target\release\mdd.exe --mode replay --input C:\temp\session.jsonl
```

`examples\demon_sandwich.bin` is random data with order patches injected: zeros,
0xFF, 0xAA, heavily biased bits, sticky bits, then random again. Watch `ALL` turn
from noise to `#`, then `=`, then back to noise, with a `@` the moment order appears.

Live JSONL on stdin (Linux or WSL shown; one event per line):

```bash
tail -f events.jsonl | ./target/release/mdd --mode stdin-jsonl
cat /dev/urandom | ./target/release/mdd --mode bytes --input -
```

Headless (no TUI: prints frames as plain text, handy for logs, CI and screenshots):

```powershell
.\target\release\mdd.exe --headless --fast --frames 480 --width 120 --height 36
```

prints the frame at 60 s plus the full alert log. `--print-every 8` prints a frame
every second of stream time; `--fast` skips real-time waiting (demo and files only).

### Capturing on Windows with nothing installed (pktmon)

Picture first: `pktmon` is built into Windows 10/11. It writes an `.etl` log, which
it can convert to `.pcapng` for `mdd`. It needs an **Administrator** PowerShell.

1. Open PowerShell as Administrator (Start menu, type `PowerShell`, right-click, *Run as administrator*).
2. Make a folder for the capture:
   ```powershell
   New-Item -ItemType Directory -Force C:\temp
   ```
3. Start capturing at the network cards only (`--comp nics` avoids each packet being
   logged once per stack layer), keeping whole packets (`--pkt-size 0`):
   ```powershell
   pktmon start --capture --comp nics --pkt-size 0 --file-name C:\temp\cap.etl
   ```
4. Use the machine normally for a minute or two, then stop:
   ```powershell
   pktmon stop
   ```
5. Convert to pcapng:
   ```powershell
   pktmon etl2pcap C:\temp\cap.etl --out C:\temp\cap.pcapng
   ```
6. Watch it (a normal PowerShell window is fine for this step):
   ```powershell
   .\target\release\mdd.exe --mode pcap-file --input C:\temp\cap.pcapng --speed 2
   ```

## Live capture (optional build)

Picture first: the default build needs no capture driver at all. Live capture uses
**Npcap** on Windows (the driver Wireshark installs) or **libpcap** on Linux, and has
to be switched on when building with `--features live`.

**Windows**

1. Download the Npcap installer from <https://npcap.com/#download> and run it. On the
   options page, tick **"Install Npcap in WinPcap API-compatible Mode"**.
2. From the same download page, get the **Npcap SDK** zip and extract it to
   `C:\npcap-sdk` (so that `C:\npcap-sdk\Lib\x64\wpcap.lib` exists).
3. In PowerShell, in this project folder, tell the build where the SDK is (this lasts
   for the current window only):
   ```powershell
   $env:LIBPCAP_LIBDIR = "C:\npcap-sdk\Lib\x64"
   ```
4. Build with live capture:
   ```powershell
   cargo build --release --features live
   ```
5. List interfaces; note the number in `[ ]` of the one you want:
   ```powershell
   .\target\release\mdd.exe --mode list-interfaces
   ```
6. Capture live (replace `3` with your number; the filter is optional BPF syntax):
   ```powershell
   .\target\release\mdd.exe --mode live-pcap --interface 3
   .\target\release\mdd.exe --mode live-pcap --interface 3 --filter "udp port 53"
   ```

If step 5 fails with *wpcap.dll was not found*: the compatible-mode box in step 1
was not ticked. Re-run the Npcap installer and tick it (or add
`C:\Windows\System32\Npcap` to your PATH). If step 6 opens but shows nothing, run
PowerShell as Administrator (Npcap may be set to admin-only).

**Linux (Lubuntu / DragonOS)**

```bash
sudo apt install libpcap-dev
cargo build --release --features live
sudo ./target/release/mdd --mode list-interfaces
sudo ./target/release/mdd --mode live-pcap --interface eth0
# or, instead of sudo each time:
sudo setcap cap_net_raw,cap_net_admin=eip target/release/mdd
```

## Event file formats (replay and stdin)

CSV with a header row, or JSONL with one object per line, same field names. Only
`len` is required.

```text
ts_ms,len,proto,src,dst,sport,dport,dir
0,92,dns,10.0.0.2,1.1.1.1,53111,53,out
{"ts_ms":25,"len":120,"proto":"tls","sport":51514,"dport":443,"dir":"out"}
```

`ts_us` (microseconds) may be used instead of `ts_ms`. Rows without a timestamp are
spaced 10 ms apart. `dir` is `in`, `out` or `local`. Malformed rows are counted
(`bad N` in the title) and skipped, never fatal. `--record` writes this same format.

## How it works

```text
 source --> symbolise --> per-lane window --> stats + detectors --> column --> frame --> screen
 (demo,     (lane +       (last 256 obs,      (every tick)          (height,   (layout)  (diff +
  files,     symbol 0-15)  30 s max age)                             texture,             synced
  pcap, live)                                                        cap, mark)           update)
```

Every tick (125 ms by default), each lane asks:

* **How mixed?** Shannon entropy, Miller-Madow bias-corrected.
* **Ordered?** Lag-1 mutual information compared against 16 shuffled copies of the
  same window. Shuffling keeps the mix but destroys order, so this measures order
  and nothing else, and stays honest on small samples where plain MI is badly biased.
* **Rhythmic?** Robust spread (MAD / median) of the gaps between events at lags 1-3.
  A beat must also be slower than the stream's own event spacing, cover most of the
  elapsed time, and hold for 3 ticks, which rules out runs, bursts and flukes.
* **Changed?** Exponentially weighted baselines per lane. Demon = CUSUM on drops of
  the entropy rate beyond max(min(0.5 bit, half the baseline), 3 standard deviations).
  Shift = KL divergence from the baseline mix, with a trigger that adapts to how much
  that lane normally wanders. Burst = z-score of log volume.

Detectors learn each lane's normal first: 8 seconds of history, or 32 windows' worth
of data for fast lanes such as raw bytes. The title shows `learning` meanwhile.

## Honest limits

* Every judgement is relative to the lane's own recent past. `@` means "more ordered
  than this lane has been", not "malicious". If new order persists it becomes the
  new normal and stops being flagged after 10-15 s.
* Packets are judged by size, direction and timing only. Payloads are never read,
  so encrypted traffic is handled exactly like plaintext.
* Lanes are by port and protocol. A beacon hidden inside a busy lane (say, WEB)
  shares that lane's statistics and can be masked; isolate it with `--filter` (live)
  or by filtering the capture first.
* In capture files, "us" (for in/out) means private addresses (10.x, 192.168.x, ...).
  Live capture uses the interface's own addresses.
* Built and tested on Linux (Rust 1.91): 45 tests, clippy clean with and without
  `--features live`. The crate has no Windows-specific code of its own; the
  Windows-specific parts are the widely used `crossterm` (terminal) and `pcap`
  (Npcap binding) crates.

## Development

```powershell
cargo test                                   # 40 unit + 5 end-to-end tests
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features live -- -D warnings   # needs the Npcap SDK / libpcap-dev
```

| File | Role |
|------|------|
| `src/source/*` | demo script, CSV/JSONL replay, stdin, raw bytes, pcap files, live capture |
| `src/pcapfile.rs`, `src/decode.rs` | pure-Rust pcap/pcapng reader and packet decoder (no libpcap needed) |
| `src/event.rs` | lanes and symbols |
| `src/analysis.rs` | statistics, classification, detectors |
| `src/app.rs` | scrolling history, heights, alerts |
| `src/render.rs`, `src/screen.rs` | layout and glyphs; flicker-free diff presenter |
| `src/run.rs` | TUI loop (keys, pause, resize, terminal restore) and headless mode |
| `tests/landscape.rs` | end-to-end checks that the right glyphs land in the right lanes |
