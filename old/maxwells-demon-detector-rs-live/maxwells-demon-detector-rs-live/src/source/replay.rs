use std::error::Error;
use std::fs::File;
use std::time::{Duration, Instant};

use csv::ReaderBuilder;
use serde::Deserialize;

use crate::event::PacketEvent;
use crate::source::EventSource;

#[derive(Debug, Deserialize)]
struct CsvEvent {
    ts_ms: Option<u64>,
    len: usize,
    #[serde(default)]
    proto: String,
    #[serde(default)]
    src: Option<String>,
    #[serde(default)]
    dst: Option<String>,
    #[serde(default)]
    sport: Option<u16>,
    #[serde(default)]
    dport: Option<u16>,
    #[serde(default)]
    dir: Option<String>,
}

pub struct ReplaySource {
    events: Vec<PacketEvent>,
    cursor: usize,
    started: Instant,
    speed: u64,
    base_ts: u64,
}

impl ReplaySource {
    pub fn from_path(path: &str, speed: u64) -> Result<Self, Box<dyn Error>> {
        let file = File::open(path)?;
        let mut rdr = ReaderBuilder::new().trim(csv::Trim::All).from_reader(file);
        let mut events = Vec::new();
        for row in rdr.deserialize() {
            let row: CsvEvent = row?;
            events.push(PacketEvent {
                ts_ms: row.ts_ms,
                len: row.len,
                proto: row.proto,
                src: row.src,
                dst: row.dst,
                sport: row.sport,
                dport: row.dport,
                dir: row.dir,
            });
        }
        let base_ts = events.iter().filter_map(|e| e.ts_ms).min().unwrap_or(0);
        Ok(Self {
            events,
            cursor: 0,
            started: Instant::now(),
            speed: speed.max(1),
            base_ts,
        })
    }
}

impl EventSource for ReplaySource {
    fn poll(&mut self, _budget: Duration) -> Result<Vec<PacketEvent>, Box<dyn Error>> {
        if self.events.is_empty() {
            return Ok(Vec::new());
        }
        let now_ms = self.started.elapsed().as_millis() as u64 * self.speed;
        let target = self.base_ts + now_ms;
        let mut out = Vec::new();

        while self.cursor < self.events.len() {
            let ev = &self.events[self.cursor];
            let ts = ev.ts_ms.unwrap_or(self.base_ts);
            if ts > target {
                break;
            }
            out.push(ev.clone());
            self.cursor += 1;
        }

        if self.cursor >= self.events.len() {
            self.cursor = 0;
            self.started = Instant::now();
        }

        Ok(out)
    }
}
