//! Synthetic traffic with a script, so every glyph shows up within one minute.
//!
//! Background (always): web page loads, DNS lookups, a little mail, an SSH
//! session, pings, NTP, ARP, a voice call (RTP), occasional backups, misc UDP.
//! The voice call is ordered and rhythmic but *normal*: it shows `=` and `|`
//! without demon markers, which is the point.
//! Scripted "demons" on a 60 s cycle (the first 15 s let baselines settle):
//!   15-25 s  DNS tunnel   fixed-size query/response ping-pong  -> ORDER + demon on DNS
//!   30-42 s  C2 beacon    request/response every 400 ms        -> rhythm on OTHER
//!   48-54 s  exfil burst  flood of identical 1280-byte packets  -> MONO, burst, shift, demon on BULK

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::time::Duration;

use crate::event::PacketEvent;
use crate::rng::Rng;
use crate::source::{Sink, Source, SourceStatus};
use crate::Result;

const CYCLE_US: u64 = 60_000_000;
const SEC: f64 = 1_000_000.0;
/// Scripted windows within each 60 s cycle, in seconds.
const TUNNEL: (f64, f64) = (15.0, 25.0);
const BEACON: (f64, f64) = (30.0, 42.0);
const EXFIL: (f64, f64) = (48.0, 54.0);

struct Pending {
    ts: u64,
    seq: u64,
    ev: PacketEvent,
}

impl PartialEq for Pending {
    fn eq(&self, other: &Self) -> bool {
        (self.ts, self.seq) == (other.ts, other.seq)
    }
}
impl Eq for Pending {}
impl PartialOrd for Pending {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Pending {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.ts, self.seq).cmp(&(other.ts, other.seq))
    }
}

/// A recurring process: fires at `next`, reschedules itself.
#[derive(Debug, Clone, Copy)]
struct Proc {
    next: u64,
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    WebLoad,
    WebKeepalive,
    Dns,
    Mail,
    Rtp,
    Ssh,
    Ping,
    Ntp,
    Arp,
    BulkBackground,
    OtherUdp,
    DnsTunnel,
    Beacon,
    Exfil,
}

const KINDS: [Kind; 14] = [
    Kind::WebLoad,
    Kind::WebKeepalive,
    Kind::Dns,
    Kind::Mail,
    Kind::Rtp,
    Kind::Ssh,
    Kind::Ping,
    Kind::Ntp,
    Kind::Arp,
    Kind::BulkBackground,
    Kind::OtherUdp,
    Kind::DnsTunnel,
    Kind::Beacon,
    Kind::Exfil,
];

pub struct DemoSource {
    rng: Rng,
    procs: Vec<(Kind, Proc)>,
    queue: BinaryHeap<Reverse<Pending>>,
    seq: u64,
    now_us: u64,
    /// Everything up to here has been emitted; new events are never scheduled earlier.
    floor_us: u64,
}

fn cycle_s(t: u64) -> f64 {
    (t % CYCLE_US) as f64 / SEC
}

fn in_window(t: u64, from_s: f64, to_s: f64) -> bool {
    let c = cycle_s(t);
    c >= from_s && c < to_s
}

/// Next time (>= t) at which cycle time enters `from_s`.
fn next_window_start(t: u64, from_s: f64) -> u64 {
    let cycle_start = t - t % CYCLE_US;
    let start = cycle_start + (from_s * SEC) as u64;
    if start >= t {
        start
    } else {
        start + CYCLE_US
    }
}

impl DemoSource {
    pub fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let procs = KINDS
            .iter()
            .map(|&k| {
                let first = (rng.f64() * 0.5 * SEC) as u64;
                (k, Proc { next: first })
            })
            .collect();
        Self {
            rng,
            procs,
            queue: BinaryHeap::new(),
            seq: 0,
            now_us: 0,
            floor_us: 0,
        }
    }

    fn push(&mut self, ts: u64, proto: &str, sport: u16, dport: u16, len: usize, inbound: bool) {
        let (src, dst) = if inbound {
            ("203.0.113.40", "10.0.0.2")
        } else {
            ("10.0.0.2", "203.0.113.40")
        };
        let ev = PacketEvent {
            ts_ms: None,
            ts_us: None,
            len,
            proto: proto.to_string(),
            src: Some(src.to_string()),
            dst: Some(dst.to_string()),
            sport: (sport != 0).then_some(sport),
            dport: (dport != 0).then_some(dport),
            dir: Some(if inbound { "in" } else { "out" }.to_string()),
        };
        self.seq += 1;
        self.queue.push(Reverse(Pending {
            ts: ts.max(self.floor_us),
            seq: self.seq,
            ev,
        }));
    }

    fn poisson(&mut self, t: u64, per_sec: f64) -> u64 {
        t + (self.rng.exp(SEC / per_sec) as u64).max(1)
    }

    /// Fire one process at time `t`; return its next firing time.
    fn fire(&mut self, kind: Kind, t: u64) -> u64 {
        let r = &mut self.rng;
        match kind {
            Kind::WebLoad => {
                let k = r.range(30, 120);
                let dur = r.range(300_000, 1_200_000);
                let client_port = r.range(49_152, 65_000) as u16;
                for _ in 0..k {
                    let off = self.rng.range(0, dur);
                    let roll = self.rng.f64();
                    let (len, inbound) = if roll < 0.55 {
                        (self.rng.range(1_200, 1_501) as usize, true)
                    } else if roll < 0.80 {
                        (self.rng.range(54, 67) as usize, false)
                    } else {
                        (self.rng.range(200, 900) as usize, self.rng.chance(0.5))
                    };
                    self.push(t + off, "tls", client_port, 443, len, inbound);
                }
                self.poisson(t, 0.35)
            }
            Kind::WebKeepalive => {
                let len = r.range(54, 120) as usize;
                let inbound = r.chance(0.5);
                self.push(t, "tls", 50_001, 443, len, inbound);
                self.poisson(t, 2.0)
            }
            Kind::Dns => {
                let q = r.range(60, 110) as usize;
                let a = r.range(90, 320) as usize;
                let delay = r.range(4_000, 40_000);
                self.push(t, "dns", 53_111, 53, q, false);
                self.push(t + delay, "dns", 53, 53_111, a, true);
                self.poisson(t, 4.0)
            }
            Kind::Mail => {
                let n = r.range(2, 7);
                let mut ts = t;
                for i in 0..n {
                    let len = self.rng.range(80, 900) as usize;
                    self.push(ts, "imap", 50_200, 993, len, i % 2 == 1);
                    ts += self.rng.range(15_000, 90_000);
                }
                self.poisson(t, 0.2)
            }
            Kind::Rtp => {
                let j1 = (r.normal() * 800.0).clamp(-3_000.0, 3_000.0);
                let j2 = (r.normal() * 800.0).clamp(-3_000.0, 3_000.0);
                let l1 = r.range(172, 215) as usize;
                let l2 = r.range(172, 215) as usize;
                let t1 = (t as f64 + j1).max(0.0) as u64;
                let t2 = (t as f64 + 7_000.0 + j2).max(0.0) as u64;
                self.push(t1, "rtp", 5_004, 5_005, l1, false);
                self.push(t2, "rtp", 5_005, 5_004, l2, true);
                t + 20_000
            }
            Kind::Ssh => {
                let l = r.range(90, 140) as usize;
                let e = r.range(90, 140) as usize;
                let d = r.range(20_000, 60_000);
                self.push(t, "ssh", 51_022, 22, l, false);
                self.push(t + d, "ssh", 22, 51_022, e, true);
                self.poisson(t, 3.0)
            }
            Kind::Ping => {
                let d = r.range(10_000, 30_000);
                self.push(t, "icmp", 0, 0, 98, false);
                self.push(t + d, "icmp", 0, 0, 98, true);
                t + 1_000_000
            }
            Kind::Ntp => {
                self.push(t, "ntp", 123, 123, 90, false);
                self.push(t + 25_000, "ntp", 123, 123, 90, true);
                t + 16_000_000
            }
            Kind::Arp => {
                let inbound = r.chance(0.5);
                self.push(t, "arp", 0, 0, 60, inbound);
                self.poisson(t, 0.2)
            }
            Kind::BulkBackground => {
                // A sync client trickling data: Poisson packets, about half of them acked.
                let ack = r.chance(0.5);
                let delay = r.range(150, 450);
                self.push(t, "ftp", 20, 40_020, 1_448, false);
                if ack {
                    self.push(t + delay, "ftp", 40_020, 20, 66, true);
                }
                self.poisson(t, 8.0)
            }
            Kind::OtherUdp => {
                let len = r.range(60, 1_150) as usize;
                let sp = r.range(40_000, 60_000) as u16;
                let dp = r.range(40_000, 60_000) as u16;
                let inbound = r.chance(0.5);
                self.push(t, "udp", sp, dp, len, inbound);
                self.poisson(t, 0.7)
            }
            Kind::DnsTunnel => {
                if in_window(t, TUNNEL.0, TUNNEL.1) {
                    let d = r.range(3_000, 7_000);
                    self.push(t, "dns", 53_999, 53, 180, false);
                    self.push(t + d, "dns", 53, 53_999, 260, true);
                    t + 10_000
                } else {
                    next_window_start(t, TUNNEL.0)
                }
            }
            Kind::Beacon => {
                if in_window(t, BEACON.0, BEACON.1) {
                    let jitter = r.range(0, 6_000);
                    self.push(t, "tcp", 49_999, 8_531, 310, false);
                    self.push(t + 30_000, "tcp", 8_531, 49_999, 520, true);
                    t + 397_000 + jitter
                } else {
                    next_window_start(t, BEACON.0)
                }
            }
            Kind::Exfil => {
                if in_window(t, EXFIL.0, EXFIL.1) {
                    // Identical tunnel-MTU packets, ~1600/s with random spacing.
                    let gap = (r.exp(625.0) as u64).max(50);
                    self.push(t, "ftp", 20, 40_666, 1_280, false);
                    t + gap
                } else {
                    next_window_start(t, EXFIL.0)
                }
            }
        }
    }

    fn phase(&self) -> &'static str {
        let c = cycle_s(self.now_us);
        if (TUNNEL.0..TUNNEL.1).contains(&c) {
            "DNS tunnel -> watch DNS"
        } else if (BEACON.0..BEACON.1).contains(&c) {
            "C2 beacon -> watch OTHER"
        } else if (EXFIL.0..EXFIL.1).contains(&c) {
            "exfil burst -> watch BULK"
        } else {
            "background only"
        }
    }
}

impl Source for DemoSource {
    fn poll(&mut self, now: Duration, sink: &mut Sink) -> Result<()> {
        let target = u64::try_from(now.as_micros()).unwrap_or(u64::MAX);
        self.now_us = target;
        // Generate everything that starts up to `target` (plus a little lookahead
        // so responses scheduled just after `target` are queued in order).
        for i in 0..self.procs.len() {
            let (kind, mut p) = self.procs[i];
            let mut guard = 0u32;
            while p.next <= target && guard < 200_000 {
                p.next = self.fire(kind, p.next).max(p.next + 1);
                guard += 1;
            }
            self.procs[i] = (kind, p);
        }
        while let Some(Reverse(top)) = self.queue.peek() {
            if top.ts > target {
                break;
            }
            let Some(Reverse(p)) = self.queue.pop() else {
                break;
            };
            sink.packet(&p.ev, p.ts);
        }
        self.floor_us = self.floor_us.max(target);
        Ok(())
    }

    fn status(&self) -> SourceStatus {
        let c = cycle_s(self.now_us) as u64;
        SourceStatus {
            label: format!("demo {:02}:{:02}/1:00 {}", c / 60, c % 60, self.phase()),
            ..SourceStatus::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{LANE_BULK, LANE_DNS, LANE_MEDIA, LANE_OTHER};

    #[test]
    fn emits_ordered_events_and_scripted_demons() {
        let mut demo = DemoSource::new(1);
        let mut sink = Sink::new(None);
        let mut last = 0;
        let mut per_lane = [0u64; 8];
        let mut tunnel = 0;
        for step in 1..=600u64 {
            demo.poll(Duration::from_millis(step * 100), &mut sink)
                .unwrap();
            for o in sink.obs.drain(..) {
                assert!(o.ts_us >= last, "events must come out in time order");
                last = o.ts_us;
                per_lane[o.lane as usize] += 1;
                let c = cycle_s(o.ts_us);
                if o.lane == LANE_DNS && (15.5..24.5).contains(&c) {
                    tunnel += 1;
                }
            }
        }
        assert!(
            per_lane[LANE_MEDIA as usize] > 5_500,
            "RTP runs all minute at 100 pkt/s"
        );
        assert!(
            per_lane[LANE_BULK as usize] > 8_000,
            "exfil is ~1600 pkt/s for 6 s"
        );
        assert!(per_lane[LANE_OTHER as usize] > 60, "beacon + misc UDP");
        assert!(tunnel > 400, "tunnel adds ~60 pkt/s to DNS, got {tunnel}");
    }
}
