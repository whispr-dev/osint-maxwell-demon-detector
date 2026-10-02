//! Packet events as JSONL on stdin, e.g. `some-tool | mdd --mode stdin-jsonl`.
//!
//! A reader thread parses lines and hands them over a bounded channel. If the
//! producer is faster than the analyser, events are dropped (and counted) rather
//! than letting memory grow or the UI stall.

use std::io::{self, BufRead};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::event::PacketEvent;
use crate::source::{Sink, Source, SourceStatus};
use crate::Result;

const QUEUE: usize = 65_536;

enum Msg {
    Event(PacketEvent, u64),
    Eof(Option<String>),
}

pub struct StdinSource {
    rx: Receiver<Msg>,
    dropped: Arc<AtomicU64>,
    bad: Arc<AtomicU64>,
    ended: bool,
    first_ts: Option<u64>,
    last_ts: u64,
    error: Option<String>,
}

impl StdinSource {
    pub fn start() -> Self {
        let (tx, rx) = mpsc::sync_channel::<Msg>(QUEUE);
        let dropped = Arc::new(AtomicU64::new(0));
        let bad = Arc::new(AtomicU64::new(0));
        let (d2, b2) = (Arc::clone(&dropped), Arc::clone(&bad));
        let start = Instant::now();
        thread::spawn(move || {
            let stdin = io::stdin();
            let mut lock = stdin.lock();
            let mut line = String::new();
            let err = loop {
                line.clear();
                match lock.read_line(&mut line) {
                    Ok(0) => break None,
                    Ok(_) => {}
                    Err(e) => break Some(e.to_string()),
                }
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                match serde_json::from_str::<PacketEvent>(trimmed) {
                    Ok(ev) => {
                        let arrival =
                            u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);
                        match tx.try_send(Msg::Event(ev, arrival)) {
                            Ok(()) => {}
                            Err(TrySendError::Full(_)) => {
                                d2.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(TrySendError::Disconnected(_)) => return,
                        }
                    }
                    Err(_) => {
                        b2.fetch_add(1, Ordering::Relaxed);
                    }
                }
            };
            let _ = tx.send(Msg::Eof(err));
        });
        Self {
            rx,
            dropped,
            bad,
            ended: false,
            first_ts: None,
            last_ts: 0,
            error: None,
        }
    }
}

impl Source for StdinSource {
    fn poll(&mut self, _now: Duration, sink: &mut Sink) -> Result<()> {
        loop {
            match self.rx.try_recv() {
                Ok(Msg::Event(ev, arrival)) => {
                    // Prefer the event's own clock (relative to the first stamped
                    // event); fall back to arrival time. Never go backwards.
                    let ts = match ev.ts_us() {
                        Some(t) => t.saturating_sub(*self.first_ts.get_or_insert(t)),
                        None => arrival,
                    };
                    self.last_ts = self.last_ts.max(ts);
                    sink.packet(&ev, self.last_ts);
                }
                Ok(Msg::Eof(err)) => {
                    self.ended = true;
                    self.error = err;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.ended = true;
                    break;
                }
            }
        }
        Ok(())
    }

    fn status(&self) -> SourceStatus {
        let label = match (&self.error, self.ended) {
            (Some(e), _) => format!("stdin-jsonl: read error: {e}"),
            (None, true) => "stdin-jsonl: end of input".to_string(),
            (None, false) => "stdin-jsonl".to_string(),
        };
        SourceStatus {
            label,
            ended: self.ended,
            dropped: self.dropped.load(Ordering::Relaxed),
            bad: self.bad.load(Ordering::Relaxed),
        }
    }

    fn realtime(&self) -> bool {
        true
    }
}
