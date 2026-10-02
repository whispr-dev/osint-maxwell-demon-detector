//! Event sources. Every source is pulled with a virtual clock `now` and pushes
//! everything that is due into a [`Sink`].

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cli::{Cli, Mode};
use crate::event::{byte_obs, packet_obs, LaneSet, Obs, PacketEvent};
use crate::Result;

mod bytes;
mod demo;
#[cfg(feature = "live")]
mod live;
mod pcap;
mod replay;
mod stdin;

pub trait Source {
    /// Emit everything due up to virtual time `now` (time since the run started).
    fn poll(&mut self, now: Duration, sink: &mut Sink) -> Result<()>;
    /// One-line description for the title bar, plus counters.
    fn status(&self) -> SourceStatus;
    /// True if the source is driven by the outside world (can't be fast-forwarded).
    fn realtime(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Default)]
pub struct SourceStatus {
    pub label: String,
    pub ended: bool,
    /// Events dropped because the analyser couldn't keep up.
    pub dropped: u64,
    /// Malformed input lines/records skipped.
    pub bad: u64,
}

/// Collects observations for the analyser and (optionally) records packets to JSONL.
pub struct Sink {
    pub obs: Vec<Obs>,
    recorder: Option<Recorder>,
    pub record_error: Option<String>,
}

impl Sink {
    pub fn new(recorder: Option<Recorder>) -> Self {
        Self {
            obs: Vec::with_capacity(4096),
            recorder,
            record_error: None,
        }
    }

    pub fn packet(&mut self, ev: &PacketEvent, ts_us: u64) {
        self.obs.push(packet_obs(ev, ts_us));
        if let Some(rec) = self.recorder.as_mut() {
            if let Err(e) = rec.write(ev, ts_us) {
                self.record_error = Some(format!("recording stopped: {e}"));
                self.recorder = None;
            }
        }
    }

    pub fn byte(&mut self, b: u8, ts_us: u64) {
        self.obs.push(byte_obs(b, ts_us));
    }

    pub fn recording(&self) -> bool {
        self.recorder.is_some()
    }

    /// Flush the recording (also happens on drop).
    pub fn flush(&mut self) -> io::Result<()> {
        match self.recorder.as_mut() {
            Some(r) => r.out.flush(),
            None => Ok(()),
        }
    }
}

/// Writes packet events as JSONL, replayable with `--mode replay`.
pub struct Recorder {
    out: BufWriter<File>,
}

impl Recorder {
    pub fn create(path: &Path) -> io::Result<Self> {
        Ok(Self {
            out: BufWriter::new(File::create(path)?),
        })
    }

    fn write(&mut self, ev: &PacketEvent, ts_us: u64) -> io::Result<()> {
        let mut rec = ev.clone();
        rec.ts_us = Some(ts_us);
        rec.ts_ms = Some(ts_us / 1_000);
        serde_json::to_writer(&mut self.out, &rec).map_err(io::Error::other)?;
        self.out.write_all(b"\n")
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.out.flush();
    }
}

/// Turns recorded timestamps into "due" times on the virtual clock, with a speed
/// multiplier and seamless looping (stream time keeps increasing across passes).
#[derive(Debug, Clone)]
pub(crate) struct Pacer {
    speed: f64,
    base_ts: Option<u64>,
    pass_start_us: u64,
    offset_us: u64,
    last_raw_ts: u64,
    last_stream_ts: u64,
    pub passes: u64,
}

impl Pacer {
    pub(crate) fn new(speed: f64) -> Self {
        Self {
            speed: if speed.is_finite() && speed > 0.0 {
                speed
            } else {
                1.0
            },
            base_ts: None,
            pass_start_us: 0,
            offset_us: 0,
            last_raw_ts: 0,
            last_stream_ts: 0,
            passes: 1,
        }
    }

    /// (due time on the virtual clock, stream timestamp) for a raw event timestamp.
    /// Idempotent until [`Pacer::commit`] is called.
    pub(crate) fn schedule(&mut self, raw_ts: u64) -> (u64, u64) {
        let base = *self.base_ts.get_or_insert(raw_ts);
        let raw = raw_ts.max(self.last_raw_ts).max(base);
        let rel = raw - base;
        let due = self
            .pass_start_us
            .saturating_add((rel as f64 / self.speed) as u64);
        (due, self.offset_us.saturating_add(rel))
    }

    pub(crate) fn commit(&mut self, raw_ts: u64, stream_ts: u64) {
        self.last_raw_ts = self.last_raw_ts.max(raw_ts);
        self.last_stream_ts = stream_ts;
    }

    /// Start the next loop pass: 100 ms of stream time after `now_us` (scaled by speed),
    /// so even a file whose events share one timestamp can't spin.
    pub(crate) fn new_pass(&mut self, now_us: u64) {
        self.offset_us = self.last_stream_ts.saturating_add(100_000);
        self.base_ts = None;
        self.last_raw_ts = 0;
        self.pass_start_us = now_us.saturating_add((100_000.0 / self.speed) as u64);
        self.passes += 1;
    }
}

pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn require_input(cli: &Cli, what: &str) -> Result<PathBuf> {
    cli.input
        .clone()
        .ok_or_else(|| format!("--input is required for --mode {what}").into())
}

/// Which lane layout a mode draws.
pub fn lane_set(mode: Mode) -> LaneSet {
    match mode {
        Mode::Bytes => LaneSet::Bytes,
        _ => LaneSet::Packets,
    }
}

/// Build the source for the selected mode.
pub fn build(cli: &Cli) -> Result<Box<dyn Source>> {
    let keep_addrs = cli.record.is_some();
    match cli.mode {
        Mode::Demo => Ok(Box::new(demo::DemoSource::new(cli.seed))),
        Mode::Replay => {
            let path = require_input(cli, "replay")?;
            Ok(Box::new(replay::ReplaySource::open(
                &path,
                cli.speed,
                cli.loop_input,
            )?))
        }
        Mode::StdinJsonl => Ok(Box::new(stdin::StdinSource::start())),
        Mode::Bytes => {
            let path = require_input(cli, "bytes")?;
            Ok(Box::new(bytes::BytesSource::open(
                &path,
                cli.rate,
                cli.loop_input,
            )?))
        }
        Mode::PcapFile => {
            let path = require_input(cli, "pcap-file")?;
            Ok(Box::new(pcap::PcapFileSource::open(
                &path,
                cli.speed,
                cli.loop_input,
                keep_addrs,
            )?))
        }
        Mode::LivePcap => build_live(cli, keep_addrs),
        Mode::ListInterfaces => Err("list-interfaces does not stream; it just prints".into()),
    }
}

#[cfg(feature = "live")]
fn build_live(cli: &Cli, keep_addrs: bool) -> Result<Box<dyn Source>> {
    Ok(Box::new(live::LiveSource::start(cli, keep_addrs)?))
}

#[cfg(not(feature = "live"))]
fn build_live(_cli: &Cli, _keep_addrs: bool) -> Result<Box<dyn Source>> {
    Err(NO_LIVE.into())
}

#[cfg(not(feature = "live"))]
const NO_LIVE: &str = "this build has no live capture. Rebuild with:\n    cargo build --release --features live\n(needs libpcap-dev on Linux, or the Npcap runtime + SDK on Windows; see README).\nNo-install alternative: capture with Wireshark/tcpdump/pktmon and use --mode pcap-file";

/// Print capture interfaces (live builds only).
pub fn print_interfaces() -> Result<()> {
    #[cfg(feature = "live")]
    {
        live::print_interfaces()
    }
    #[cfg(not(feature = "live"))]
    {
        Err(NO_LIVE.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacer_schedules_speed_and_loops() {
        let mut p = Pacer::new(2.0);
        let (due, st) = p.schedule(1_000_000);
        assert_eq!((due, st), (0, 0));
        p.commit(1_000_000, st);
        let (due, st) = p.schedule(3_000_000);
        assert_eq!((due, st), (1_000_000, 2_000_000));
        // Idempotent before commit.
        assert_eq!(p.schedule(3_000_000), (1_000_000, 2_000_000));
        p.commit(3_000_000, st);
        // Out-of-order timestamps are clamped, never going backwards.
        assert_eq!(p.schedule(2_000_000), (1_000_000, 2_000_000));
        p.new_pass(5_000_000);
        let (due, st) = p.schedule(1_000_000);
        assert_eq!(due, 5_050_000, "100 ms gap at 2x speed = 50 ms");
        assert_eq!(st, 2_100_000, "stream time continues after the loop gap");
        assert_eq!(p.passes, 2);
    }

    #[test]
    fn recorder_roundtrip() {
        let dir = std::env::temp_dir().join(format!("mdd-rec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rec.jsonl");
        {
            let mut sink = Sink::new(Some(Recorder::create(&path).unwrap()));
            let ev = PacketEvent {
                len: 120,
                proto: "dns".into(),
                dport: Some(53),
                dir: Some("out".into()),
                ..Default::default()
            };
            sink.packet(&ev, 1_234_567);
            assert_eq!(sink.obs.len(), 1);
            sink.flush().unwrap();
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let back: PacketEvent = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(back.ts_us, Some(1_234_567));
        assert_eq!(back.ts_ms, Some(1_234));
        assert_eq!(back.len, 120);
        std::fs::remove_dir_all(&dir).ok();
    }
}
