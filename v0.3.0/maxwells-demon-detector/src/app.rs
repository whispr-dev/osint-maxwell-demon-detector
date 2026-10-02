//! The landscape model: lanes, scrolling history, alerts, rates.

use std::collections::VecDeque;

use crate::analysis::{AlertKind, Col, LaneAlert, LaneState};
use crate::event::{LaneSet, Obs};

/// Columns of history kept per lane (wider terminals show more of it).
pub const HISTORY: usize = 2048;
/// EWMA weight for each lane's typical events-per-tick (~12 s memory at 125 ms).
const TYPICAL_ALPHA: f64 = 0.01;
/// Heights are never measured against fewer than this many events per tick.
const REF_FLOOR: f64 = 4.0;

/// Metadata shared by all lanes for one column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColMeta {
    /// Monotone column id; used to keep noise grain stable as columns scroll.
    pub id: u64,
    /// Seconds since start (virtual clock) when the column was closed.
    pub t: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub t: f64,
    pub lane: &'static str,
    pub kind: AlertKind,
    pub msg: String,
}

pub struct App {
    pub lane_set: LaneSet,
    pub lanes: Vec<LaneState>,
    pub history: Vec<VecDeque<Col>>,
    pub cols: VecDeque<ColMeta>,
    pub alerts: VecDeque<Alert>,
    pub max_alerts: usize,
    pub ticks: u64,
    pub total_events: u64,
    pub total_bytes: u64,
    /// Smoothed events per second.
    pub ev_rate: f64,
    /// Smoothed bytes per second.
    pub byte_rate: f64,
    /// Smoothed (events/s, bytes/s) per lane.
    pub lane_rates: Vec<(f64, f64)>,
    last_t: f64,
    /// Per-lane typical events per tick (EWMA) that heights are measured against.
    typical: Vec<f64>,
    scratch_alerts: Vec<LaneAlert>,
}

impl App {
    pub fn new(lane_set: LaneSet, window: usize, max_alerts: usize) -> Self {
        let lanes: Vec<LaneState> = lane_set
            .specs()
            .into_iter()
            .enumerate()
            .map(|(i, spec)| LaneState::new(spec, window, 0x5EED_0000 + i as u64))
            .collect();
        let history = lanes
            .iter()
            .map(|_| VecDeque::with_capacity(HISTORY))
            .collect();
        let lane_rates = vec![(0.0, 0.0); lanes.len()];
        let typical = vec![0.0; lanes.len()];
        Self {
            lane_set,
            lanes,
            history,
            cols: VecDeque::with_capacity(HISTORY),
            alerts: VecDeque::new(),
            max_alerts: max_alerts.max(1),
            ticks: 0,
            total_events: 0,
            total_bytes: 0,
            ev_rate: 0.0,
            byte_rate: 0.0,
            lane_rates,
            last_t: 0.0,
            typical,
            scratch_alerts: Vec::new(),
        }
    }

    /// Feed one observation into its lane and into ALL (lane 0).
    pub fn ingest(&mut self, o: &Obs) {
        self.total_events += 1;
        self.total_bytes += u64::from(o.bytes);
        if let Some(all) = self.lanes.first_mut() {
            all.push(o.ts_us, o.sym_all, o.bytes);
        }
        let idx = usize::from(o.lane);
        if idx >= 1 && idx < self.lanes.len() {
            self.lanes[idx].push(o.ts_us, o.sym, o.bytes);
        }
    }

    /// True while detectors are still learning what "normal" looks like.
    pub fn warming_up(&self) -> bool {
        self.ticks < u64::from(crate::analysis::WARM_LOAD_TICKS)
    }

    /// Close the current tick: every lane gets one new column.
    /// `t` = seconds since start on the virtual clock.
    pub fn finish_tick(&mut self, t: f64) {
        let dt = (t - self.last_t).max(1e-3);
        self.last_t = t;
        let id = self.ticks;
        self.ticks += 1;

        let mut cols: Vec<Col> = Vec::with_capacity(self.lanes.len());
        let mut all_mean_dt = None;
        for (i, lane) in self.lanes.iter_mut().enumerate() {
            self.scratch_alerts.clear();
            let hint = if i == 0 { None } else { all_mean_dt };
            let col = lane.finish_tick(hint, &mut self.scratch_alerts);
            if i == 0 {
                let m = lane.metrics.mean_dt_us;
                all_mean_dt = (m > 0.0).then_some(m);
            }
            for a in self.scratch_alerts.drain(..) {
                self.alerts.push_front(Alert {
                    t,
                    lane: lane.spec.name,
                    kind: a.kind,
                    msg: a.msg,
                });
            }
            cols.push(col);
        }
        while self.alerts.len() > self.max_alerts {
            self.alerts.pop_back();
        }

        // Heights: each lane against its own typical volume, square-root scaled:
        // a typical tick stands at ~70% of the band and twice typical fills it.
        // Every skyline shows its own dynamics; the side panel has absolute rates.
        let a = TYPICAL_ALPHA.max(1.0 / self.ticks as f64);
        for (col, typical) in cols.iter_mut().zip(self.typical.iter_mut()) {
            let n = f64::from(col.n);
            *typical += a * (n - *typical);
            let reference = (2.0 * *typical).max(REF_FLOOR);
            col.level = if col.n == 0 {
                0.0
            } else {
                (n / reference).sqrt().clamp(0.0, 1.0) as f32
            };
        }
        for (hist, col) in self.history.iter_mut().zip(cols) {
            if hist.len() == HISTORY {
                hist.pop_front();
            }
            hist.push_back(col);
        }
        if self.cols.len() == HISTORY {
            self.cols.pop_front();
        }
        self.cols.push_back(ColMeta { id, t });

        // Smoothed throughput (title bar and side panel).
        let k = if self.ticks <= 1 { 1.0 } else { 0.2 };
        for (rate, lane) in self.lane_rates.iter_mut().zip(self.lanes.iter()) {
            rate.0 += k * (f64::from(lane.last_n) / dt - rate.0);
            rate.1 += k * (lane.last_bytes as f64 / dt - rate.1);
        }
        if let Some(&(ev, b)) = self.lane_rates.first() {
            self.ev_rate = ev;
            self.byte_rate = b;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::byte_obs;

    #[test]
    fn ticks_make_columns_for_every_lane() {
        let mut app = App::new(LaneSet::Bytes, 256, 10);
        for tick in 0..5u64 {
            for i in 0..100u64 {
                app.ingest(&byte_obs((i % 256) as u8, tick * 100_000 + i));
            }
            app.finish_tick(tick as f64 * 0.1 + 0.1);
        }
        assert_eq!(app.cols.len(), 5);
        assert!(app.history.iter().all(|h| h.len() == 5));
        assert_eq!(app.total_events, 500);
        let all = app.history[0].back().unwrap();
        assert_eq!(all.n, 100);
        assert!(all.level > 0.0 && all.level <= 1.0);
    }

    #[test]
    fn history_is_bounded() {
        let mut app = App::new(LaneSet::Packets, 64, 3);
        for t in 0..(HISTORY + 10) {
            app.finish_tick(t as f64);
        }
        assert_eq!(app.cols.len(), HISTORY);
        assert!(app.history.iter().all(|h| h.len() == HISTORY));
        assert_eq!(app.cols.back().unwrap().id, (HISTORY + 9) as u64);
    }
}
