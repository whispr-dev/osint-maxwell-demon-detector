use std::collections::VecDeque;

use crate::cli::Mode;
use crate::event::{Lane, PacketEvent};
use crate::stats::{mean, normalized_entropy, stddev};

const SIZE_BINS: usize = 6;
const GLYPHS: &[u8] = b" .,:;ox%#@";
const Z_WINDOW: usize = 24;

#[derive(Debug, Clone)]
pub struct Cell {
    pub count: u64,
    pub bytes: u64,
    pub entropy: f64,
    pub score: f64,
    pub anomaly: bool,
}

impl Cell {
    pub fn empty() -> Self {
        Self {
            count: 0,
            bytes: 0,
            entropy: 0.0,
            score: 0.0,
            anomaly: false,
        }
    }

    pub fn glyph(&self) -> char {
        if self.anomaly && self.count > 0 {
            return '!';
        }
        let idx = (self.score.clamp(0.0, 0.999) * (GLYPHS.len() as f64)) as usize;
        GLYPHS[idx.min(GLYPHS.len() - 1)] as char
    }
}

#[derive(Debug, Clone)]
struct Bucket {
    count: u64,
    bytes: u64,
    size_bins: [u64; SIZE_BINS],
}

impl Bucket {
    fn new() -> Self {
        Self {
            count: 0,
            bytes: 0,
            size_bins: [0; SIZE_BINS],
        }
    }

    fn add(&mut self, ev: &PacketEvent) {
        self.count += 1;
        self.bytes += ev.len as u64;
        let idx = match ev.len {
            0..=63 => 0,
            64..=127 => 1,
            128..=255 => 2,
            256..=511 => 3,
            512..=1023 => 4,
            _ => 5,
        };
        self.size_bins[idx] += 1;
    }

    fn entropy(&self) -> f64 {
        normalized_entropy(&self.size_bins)
    }
}

#[derive(Debug)]
pub struct AppState {
    pub width: usize,
    pub tick_ms: u64,
    pub mode: Mode,
    pub started_at: std::time::Instant,
    pub total_events: u64,
    pub total_bytes: u64,
    pub lanes: Vec<Lane>,
    pub history: Vec<VecDeque<Cell>>,
    buckets: Vec<Bucket>,
    score_history: Vec<VecDeque<f64>>,
    pub alerts: VecDeque<String>,
    pub max_alerts: usize,
    pub last_tick_count: u64,
}

impl AppState {
    pub fn new(width: usize, tick_ms: u64, mode: Mode, max_alerts: usize) -> Self {
        let lanes = Lane::ALL.to_vec();
        let history = lanes
            .iter()
            .map(|_| {
                let mut row = VecDeque::with_capacity(width);
                for _ in 0..width {
                    row.push_back(Cell::empty());
                }
                row
            })
            .collect();
        let buckets = lanes.iter().map(|_| Bucket::new()).collect();
        let score_history = lanes.iter().map(|_| VecDeque::with_capacity(Z_WINDOW)).collect();
        Self {
            width,
            tick_ms,
            mode,
            started_at: std::time::Instant::now(),
            total_events: 0,
            total_bytes: 0,
            lanes,
            history,
            buckets,
            score_history,
            alerts: VecDeque::new(),
            max_alerts: max_alerts.max(1),
            last_tick_count: 0,
        }
    }

    pub fn ingest(&mut self, ev: PacketEvent) {
        let lane = ev.lane();
        let idx = self
            .lanes
            .iter()
            .position(|l| *l == lane)
            .unwrap_or(self.lanes.len() - 1);
        self.buckets[idx].add(&ev);
        self.total_events += 1;
        self.total_bytes += ev.len as u64;
        self.last_tick_count += 1;
    }

    pub fn finalize_tick(&mut self) {
        for i in 0..self.lanes.len() {
            let bucket = std::mem::replace(&mut self.buckets[i], Bucket::new());
            let entropy = bucket.entropy();
            let load_term = ((bucket.count as f64 + 1.0).ln() / 4.0).clamp(0.0, 1.0);
            let score = (0.55 * load_term) + (0.45 * entropy);

            let baseline: Vec<f64> = self.score_history[i].iter().copied().collect();
            let mu = mean(&baseline);
            let sd = stddev(&baseline, mu).max(0.08);
            let z = if baseline.len() >= 6 { (score - mu) / sd } else { 0.0 };
            let anomaly = bucket.count >= 8 && z >= 2.6;

            let cell = Cell {
                count: bucket.count,
                bytes: bucket.bytes,
                entropy,
                score: score.clamp(0.0, 1.0),
                anomaly,
            };
            let row = &mut self.history[i];
            if row.len() >= self.width {
                row.pop_front();
            }
            row.push_back(cell.clone());

            let sh = &mut self.score_history[i];
            if sh.len() >= Z_WINDOW {
                sh.pop_front();
            }
            sh.push_back(score);

            if anomaly {
                let line = format!(
                    "{} spike: count={} bytes={} entropy={:.2} z={:.2}",
                    self.lanes[i].label(),
                    cell.count,
                    cell.bytes,
                    cell.entropy,
                    z,
                );
                self.alerts.push_front(line);
                while self.alerts.len() > self.max_alerts {
                    self.alerts.pop_back();
                }
            }
        }
        self.last_tick_count = 0;
    }

    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    pub fn current_cell(&self, idx: usize) -> Option<&Cell> {
        self.history.get(idx).and_then(|row| row.back())
    }
}
