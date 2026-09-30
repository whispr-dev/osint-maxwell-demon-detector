use std::error::Error;
use std::time::Duration;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::event::PacketEvent;
use crate::source::EventSource;

pub struct DemoSource {
    rng: StdRng,
    phase: u64,
    tick_ms: u64,
}

impl DemoSource {
    pub fn new(tick_ms: u64) -> Self {
        Self {
            rng: StdRng::seed_from_u64(0xD3A10A5u64),
            phase: 0,
            tick_ms,
        }
    }

    fn proto_for(&mut self, burst: bool) -> (&'static str, u16, u16, usize) {
        let r = self.rng.gen_range(0..100);
        if burst {
            if r < 45 {
                return ("tls", 51432, 443, self.rng.gen_range(900..1500));
            }
            if r < 70 {
                return ("dns", 53111, 53, self.rng.gen_range(72..180));
            }
            if r < 88 {
                return ("rtp", 5004, 5005, self.rng.gen_range(160..900));
            }
            return ("ssh", 55221, 22, self.rng.gen_range(64..200));
        }

        if r < 35 {
            ("tls", 51432, 443, self.rng.gen_range(90..1400))
        } else if r < 48 {
            ("dns", 53111, 53, self.rng.gen_range(72..180))
        } else if r < 58 {
            ("smtp", 49322, 587, self.rng.gen_range(200..900))
        } else if r < 72 {
            ("rtp", 5004, 5005, self.rng.gen_range(120..1100))
        } else if r < 84 {
            ("ssh", 55221, 22, self.rng.gen_range(64..220))
        } else if r < 92 {
            ("ftp", 52000, 21, self.rng.gen_range(1100..1500))
        } else {
            ("icmp", 0, 0, self.rng.gen_range(56..128))
        }
    }
}

impl EventSource for DemoSource {
    fn poll(&mut self, budget: Duration) -> Result<Vec<PacketEvent>, Box<dyn Error>> {
        self.phase = self.phase.wrapping_add(1);
        let burst = (self.phase / 30) % 5 == 2 || (self.phase / 45) % 7 == 3;
        let base = ((budget.as_millis() as u64).max(self.tick_ms) / self.tick_ms).max(1) as usize;
        let count = if burst {
            self.rng.gen_range(70..160) * base
        } else {
            self.rng.gen_range(8..55) * base
        };

        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let (proto, sport, dport, len) = self.proto_for(burst);
            out.push(PacketEvent {
                ts_ms: None,
                len,
                proto: proto.to_string(),
                src: Some("10.0.0.2".into()),
                dst: Some("198.51.100.10".into()),
                sport: if sport == 0 { None } else { Some(sport) },
                dport: if dport == 0 { None } else { Some(dport) },
                dir: Some(if self.rng.gen_bool(0.62) { "out" } else { "in" }.into()),
            });
        }
        Ok(out)
    }
}
