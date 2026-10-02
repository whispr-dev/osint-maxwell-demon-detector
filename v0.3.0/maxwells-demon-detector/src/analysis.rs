//! Information-theory stats and the per-lane analyser.
//!
//! For every lane we keep the last `window` observations (symbol + timestamp) and,
//! once per tick, ask four questions:
//!
//! 1. How mixed are the symbols? Shannon entropy H in bits (Miller-Madow corrected).
//! 2. Does the previous symbol predict the next one? Lag-1 mutual information,
//!    checked against shuffled copies of the same window (a permutation test),
//!    giving a z-score `dep.z`. Shuffling keeps the symbol mix but destroys order,
//!    so it measures order and nothing else, and it stays honest at small sample
//!    sizes where plug-in MI is badly biased.
//! 3. Is the timing metronomic? Robust spread (MAD / median) of the gaps between
//!    events, also at lag 2 and 3 so request/response beacons count.
//! 4. Did anything change versus this lane's own history? EWMA baselines with
//!    CUSUM (entropy-rate drop = "demon"), KL divergence (mix shift) and a
//!    log-load z-score (burst).

use std::collections::VecDeque;

use crate::event::{LaneSpec, ALPHABET};
use crate::rng::Rng;

/// Fewer observations than this and we refuse to call the structure.
pub const MIN_N: usize = 16;
/// Shuffles per permutation test.
pub const SHUFFLES: usize = 16;
/// Timestamps kept for rhythm detection (up to 48 gaps at lag 1, 16 at lag 3).
pub const RHYTHM_TS: usize = 49;
/// Observations older than this (stream time) fall out of a lane's window.
pub const MAX_AGE_US: u64 = 30_000_000;
/// Active ticks a detector needs before it is allowed to fire.
pub const WARM_TICKS: u32 = 30;
/// Ticks of load history needed before bursts can fire.
pub const WARM_LOAD_TICKS: u32 = 40;
/// A lane's entropy-rate baseline must span at least this many ticks (8 s at
/// the default 125 ms) before the demon detector may fire.
pub const WARM_SPAN_TICKS: u64 = 64;
/// Minimum ticks between two alerts of the same kind on the same lane.
pub const ALERT_COOLDOWN_TICKS: u64 = 80;

// ---------------------------------------------------------------------------
// Pure statistics
// ---------------------------------------------------------------------------

/// Plug-in Shannon entropy (bits) of a histogram.
pub fn entropy_bits(counts: &[u32]) -> f64 {
    let total: u64 = counts.iter().map(|&c| u64::from(c)).sum();
    if total == 0 {
        return 0.0;
    }
    let t = total as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = f64::from(c) / t;
            -p * p.log2()
        })
        .sum()
}

#[inline]
fn sym_index(s: u8) -> usize {
    usize::from(s).min(ALPHABET - 1)
}

/// Lag-1 mutual information I(X_t ; X_t+1) in bits (plug-in estimate).
pub fn lag1_mi(seq: &[u8]) -> f64 {
    if seq.len() < 2 {
        return 0.0;
    }
    let mut joint = [[0u32; ALPHABET]; ALPHABET];
    let mut first = [0u32; ALPHABET];
    let mut second = [0u32; ALPHABET];
    for w in seq.windows(2) {
        let (a, b) = (sym_index(w[0]), sym_index(w[1]));
        joint[a][b] += 1;
        first[a] += 1;
        second[b] += 1;
    }
    let t = (seq.len() - 1) as f64;
    let mut acc = 0.0;
    for (a, row) in joint.iter().enumerate() {
        if first[a] == 0 {
            continue;
        }
        for (b, &c) in row.iter().enumerate() {
            if c == 0 {
                continue;
            }
            let c = f64::from(c);
            acc += c * (c * t / (f64::from(first[a]) * f64::from(second[b]))).log2();
        }
    }
    (acc / t).max(0.0)
}

/// Result of the permutation test for sequential dependency.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Dependency {
    /// Observed lag-1 MI (bits).
    pub mi: f64,
    /// Mean lag-1 MI of shuffled copies (the small-sample bias floor).
    pub null_mean: f64,
    /// Standard deviation of the shuffled MI values.
    pub null_sd: f64,
    /// (mi - null_mean) / null_sd, clamped to +-999.
    pub z: f64,
}

/// Permutation test: is the order of `seq` more predictable than its shuffles?
pub fn dependency(seq: &[u8], shuffles: usize, rng: &mut Rng, scratch: &mut Vec<u8>) -> Dependency {
    let mi = lag1_mi(seq);
    if seq.len() < 3 || shuffles < 2 {
        return Dependency {
            mi,
            null_mean: mi,
            null_sd: 0.0,
            z: 0.0,
        };
    }
    scratch.clear();
    scratch.extend_from_slice(seq);
    let (mut sum, mut sumsq) = (0.0f64, 0.0f64);
    for _ in 0..shuffles {
        rng.shuffle(scratch);
        let m = lag1_mi(scratch);
        sum += m;
        sumsq += m * m;
    }
    let k = shuffles as f64;
    let null_mean = sum / k;
    let null_sd = ((sumsq - sum * sum / k) / (k - 1.0)).max(0.0).sqrt();
    let z = ((mi - null_mean) / null_sd.max(1e-3)).clamp(-999.0, 999.0);
    Dependency {
        mi,
        null_mean,
        null_sd,
        z,
    }
}

/// Timing regularity of an event train.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rhythm {
    /// Robust coefficient of variation (1.4826 x MAD / median) of the best lag's
    /// gaps: 0 = metronome, ~1 = random (Poisson) arrivals.
    pub cv: f64,
    /// Median gap at the best lag, i.e. the repeat period, in microseconds.
    pub period_us: f64,
    /// Which lag won (1 = every event, 2 = request/response pairs, 3 = triples).
    pub lag: usize,
}

/// Median of a slice (sorts it in place). Empty slice -> 0.
fn median_in_place(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_unstable_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

/// Gaps needed at a lag before its rhythm is judged.
const MIN_GAPS: usize = 12;

/// Rhythm over lags 1..=3. `ts` must be ascending (out-of-order stamps count as zero gaps).
///
/// Gaps at lag L are taken as non-overlapping blocks counted back from the newest
/// event, and spread is measured robustly (1.4826 x MAD / median): a few unrelated
/// packets interleaved with a beacon break a couple of gaps, not the lock. A lag
/// is rejected when its mean gap exceeds 4x its median (evenly spaced bursts with
/// long silences between them are not a rhythm).
/// Summing more gaps always shrinks spread (~1/sqrt(lag) for a jittery metronome),
/// so lags compete on `cv * sqrt(lag)` and a longer lag must at least halve it.
/// A 20 ms heartbeat reports 20 ms (lag 1); a request/response beacon, whose
/// single gaps alternate short/long, wins clearly at lag 2.
pub fn rhythm(ts: &[u64]) -> Option<Rhythm> {
    let mut best: Option<(f64, Rhythm)> = None;
    let mut gaps: Vec<f64> = Vec::with_capacity(ts.len());
    let mut dev: Vec<f64> = Vec::with_capacity(ts.len());
    for lag in 1..=3usize {
        if ts.len() < 1 + lag * MIN_GAPS {
            break;
        }
        gaps.clear();
        let mut i = ts.len() - 1;
        while i >= lag {
            gaps.push(ts[i].saturating_sub(ts[i - lag]) as f64);
            i -= lag;
        }
        let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
        let med = median_in_place(&mut gaps);
        // The regular gaps must cover most of the time: bursts of evenly spaced
        // packets separated by long silences are not a rhythm.
        if med <= 0.0 || mean > 4.0 * med {
            continue;
        }
        dev.clear();
        dev.extend(gaps.iter().map(|g| (g - med).abs()));
        let cv = 1.4826 * median_in_place(&mut dev) / med;
        let score = cv * (lag as f64).sqrt();
        if best.is_none_or(|(b, _)| score < 0.5 * b) {
            best = Some((
                score,
                Rhythm {
                    cv,
                    period_us: med,
                    lag,
                },
            ));
        }
    }
    best.map(|(_, r)| r)
}

/// KL(p || q) in bits between a smoothed histogram and a baseline distribution.
fn kl_bits(counts: &[u32; ALPHABET], n: usize, q: &[f64; ALPHABET]) -> f64 {
    const PSEUDO: f64 = 0.5;
    let denom = n as f64 + PSEUDO * ALPHABET as f64;
    let kl: f64 = counts
        .iter()
        .zip(q.iter())
        .map(|(&c, &qb)| {
            let p = (f64::from(c) + PSEUDO) / denom;
            let qi = 0.99 * qb + 0.01 / ALPHABET as f64;
            p * (p / qi).log2()
        })
        .sum();
    kl.max(0.0)
}

// ---------------------------------------------------------------------------
// Window metrics and classification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Metrics {
    /// Observations in the window.
    pub n: usize,
    /// Shannon entropy of the window's symbols (bits, 0..=4).
    pub h: f64,
    /// Share of the most common symbol.
    pub top_share: f64,
    /// Lag-1 dependency test.
    pub dep: Dependency,
    /// Excess MI as a fraction of H: how much of the uncertainty the previous symbol removes.
    pub rho: f64,
    /// Entropy-rate proxy: H minus excess MI (bits/symbol). Drops when order appears.
    pub h_rate: f64,
    /// Mean gap between this lane's events (us).
    pub mean_dt_us: f64,
    /// Timing regularity (None if too few events or timing not meaningful).
    pub rhythm: Option<Rhythm>,
    /// A rhythm candidate was rejected because its period is no slower than the
    /// stream's own event spacing (the pulse flag holds rather than flapping).
    pub rhythm_blocked: bool,
    pub hist: [u32; ALPHABET],
}

/// What the body (fill) of a landscape column looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Body {
    /// Nothing happened in this lane during the tick.
    #[default]
    Void,
    /// Too few observations to judge.
    Sparse,
    /// Mixed symbols, no sequential structure: random-looking.
    Noise,
    /// Statistically significant, loose sequential structure.
    Flow,
    /// Strong structure: the previous symbol largely predicts the next.
    Order,
    /// One symbol dominates.
    Mono,
}

impl Body {
    pub fn name(self) -> &'static str {
        match self {
            Body::Void => "idle",
            Body::Sparse => "sparse",
            Body::Noise => "noise",
            Body::Flow => "flow",
            Body::Order => "ORDER",
            Body::Mono => "MONO",
        }
    }
}

/// Map window metrics to a body class. Order of checks = priority.
pub fn classify(m: &Metrics) -> Body {
    if m.n == 0 {
        Body::Void
    } else if m.n < MIN_N {
        Body::Sparse
    } else if m.top_share >= 0.80 {
        Body::Mono
    } else if m.dep.z >= 5.0 && m.rho >= 0.25 {
        Body::Order
    } else if m.dep.z >= 3.0 {
        Body::Flow
    } else {
        Body::Noise
    }
}

/// Glyph drawn on top of a column (timing / volume).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cap {
    #[default]
    None,
    /// Metronomic timing (beacons, heartbeats, media frames).
    Pulse,
    /// Volume far above this lane's baseline.
    Burst,
}

/// Marker drawn above a column (detectors).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mark {
    #[default]
    None,
    /// The mix of symbols moved away from this lane's baseline.
    Shift,
    /// Entropy rate fell well below baseline: order is being injected.
    Demon,
}

/// One landscape column for one lane.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Col {
    /// Events in this tick.
    pub n: u32,
    /// Height as a fraction of the band, frozen when the column was made.
    pub level: f32,
    pub body: Body,
    pub cap: Cap,
    pub mark: Mark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    Demon,
    Shift,
    Burst,
    Pulse,
}

/// A rising-edge detector event from one lane.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneAlert {
    pub kind: AlertKind,
    pub msg: String,
}

// ---------------------------------------------------------------------------
// Detectors
// ---------------------------------------------------------------------------

/// Exponentially weighted mean/variance with a warm start (plain running
/// mean/variance until 1/alpha samples have been seen).
#[derive(Debug, Clone, Copy, Default)]
struct Ewma {
    mean: f64,
    var: f64,
    n: u32,
}

impl Ewma {
    fn update(&mut self, x: f64, alpha: f64) {
        if !x.is_finite() {
            return;
        }
        self.n = self.n.saturating_add(1);
        let a = alpha.max(1.0 / f64::from(self.n));
        let d = x - self.mean;
        self.mean += a * d;
        self.var = (1.0 - a) * (self.var + a * d * d);
    }

    fn sd(&self) -> f64 {
        self.var.max(0.0).sqrt()
    }
}

#[derive(Debug, Clone, Default)]
struct Detectors {
    load: Ewma,
    rate: Ewma,
    cusum: f64,
    q: [f64; ALPHABET],
    q_n: u32,
    shift_streak: u8,
    /// Baseline of this lane's own KL wandering (for the adaptive shift trigger).
    kl_base: Ewma,
    /// Tick of the first valid (non-sparse) window.
    first_valid: Option<u64>,
    /// Observations seen so far (fast lanes warm up by volume, not just time).
    obs_seen: u64,
    /// Consecutive active ticks with a clear rhythm.
    pulse_streak: u8,
    /// Lightly smoothed entropy rate (the demon detector's input).
    hr_smooth: Option<f64>,
    /// Tick counter and the tick of the last alert per kind (demon, shift, burst, pulse).
    tick: u64,
    last_alert: [Option<u64>; 4],
}

impl Detectors {
    /// Rate-limit alerts: flags still flip instantly, the alert list stays readable.
    fn alert(&mut self, alerts: &mut Vec<LaneAlert>, kind: AlertKind, msg: String) {
        let slot = match kind {
            AlertKind::Demon => 0,
            AlertKind::Shift => 1,
            AlertKind::Burst => 2,
            AlertKind::Pulse => 3,
        };
        let cooled = self.last_alert[slot]
            .is_none_or(|t| self.tick.saturating_sub(t) >= ALERT_COOLDOWN_TICKS);
        if cooled {
            self.last_alert[slot] = Some(self.tick);
            alerts.push(LaneAlert { kind, msg });
        }
    }
}

// ---------------------------------------------------------------------------
// Lane state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LaneState {
    pub spec: LaneSpec,
    ring: VecDeque<(u64, u8)>,
    window: usize,
    tick_n: u32,
    tick_bytes: u64,
    /// Events in the most recently finished tick.
    pub last_n: u32,
    /// Bytes in the most recently finished tick.
    pub last_bytes: u64,
    /// Metrics from the most recent tick that had events.
    pub metrics: Metrics,
    /// Body class of the newest column.
    pub body: Body,
    pub pulse: bool,
    pub burst: bool,
    pub shift: bool,
    pub demon: bool,
    /// Bias-corrected KL divergence vs baseline (bits), last computed.
    pub kl_excess: f64,
    det: Detectors,
    rng: Rng,
    seq: Vec<u8>,
    ts: Vec<u64>,
    scratch: Vec<u8>,
}

impl LaneState {
    pub fn new(spec: LaneSpec, window: usize, seed: u64) -> Self {
        let window = window.clamp(32, 4096);
        Self {
            spec,
            ring: VecDeque::with_capacity(window),
            window,
            tick_n: 0,
            tick_bytes: 0,
            last_n: 0,
            last_bytes: 0,
            metrics: Metrics::default(),
            body: Body::Void,
            pulse: false,
            burst: false,
            shift: false,
            demon: false,
            kl_excess: 0.0,
            det: Detectors::default(),
            rng: Rng::new(seed),
            seq: Vec::with_capacity(window),
            ts: Vec::with_capacity(window),
            scratch: Vec::with_capacity(window),
        }
    }

    pub fn push(&mut self, ts_us: u64, sym: u8, bytes: u32) {
        if self.ring.len() == self.window {
            self.ring.pop_front();
        }
        self.ring.push_back((ts_us, sym));
        self.tick_n = self.tick_n.saturating_add(1);
        self.tick_bytes = self.tick_bytes.saturating_add(u64::from(bytes));
    }

    /// Recompute window metrics (only called when the lane saw events this tick).
    fn compute_metrics(&mut self, all_mean_dt_us: Option<f64>) {
        let Some(&(newest, _)) = self.ring.back() else {
            self.metrics = Metrics::default();
            return;
        };
        let cutoff = newest.saturating_sub(MAX_AGE_US);
        let start = self
            .ring
            .iter()
            .position(|&(t, _)| t >= cutoff)
            .unwrap_or(self.ring.len());
        self.seq.clear();
        self.ts.clear();
        let mut hist = [0u32; ALPHABET];
        for &(t, s) in self.ring.iter().skip(start) {
            self.seq.push(s);
            self.ts.push(t);
            hist[sym_index(s)] += 1;
        }
        let n = self.seq.len();
        // Miller-Madow correction: plug-in entropy reads low on small windows,
        // which would make a lane's baseline creep upward while its window fills.
        let nonzero = hist.iter().filter(|&&c| c > 0).count();
        let h = if n > 0 {
            (entropy_bits(&hist)
                + nonzero.saturating_sub(1) as f64 / (2.0 * n as f64 * std::f64::consts::LN_2))
                .min(4.0)
        } else {
            0.0
        };
        let top = hist.iter().copied().max().unwrap_or(0);
        let top_share = if n > 0 {
            f64::from(top) / n as f64
        } else {
            0.0
        };
        let dep = if n >= MIN_N {
            dependency(&self.seq, SHUFFLES, &mut self.rng, &mut self.scratch)
        } else {
            Dependency {
                mi: lag1_mi(&self.seq),
                ..Dependency::default()
            }
        };
        let excess = (dep.mi - dep.null_mean).max(0.0);
        let rho = if h > 1e-9 { (excess / h).min(1.0) } else { 0.0 };
        let h_rate = (h - excess).max(0.0);
        let mean_dt_us = match (self.ts.first(), self.ts.last()) {
            (Some(&a), Some(&b)) if n >= 2 => b.saturating_sub(a) as f64 / (n - 1) as f64,
            _ => 0.0,
        };

        let mut rhythm_val = if self.spec.timing {
            let from = self.ts.len().saturating_sub(RHYTHM_TS);
            rhythm(&self.ts[from..])
        } else {
            None
        };
        // A beat must be slower than the stream's own event spacing. A "period"
        // shorter than 2x the overall gap is this lane riding the carrier (runs of
        // one byte class, a lane that is most of the stream), not a rhythm.
        let mut rhythm_blocked = false;
        if let (Some(r), Some(all)) = (rhythm_val, all_mean_dt_us) {
            if all > 0.0 && r.period_us < 2.0 * all {
                rhythm_val = None;
                rhythm_blocked = true;
            }
        }

        self.metrics = Metrics {
            n,
            h,
            top_share,
            dep,
            rho,
            h_rate,
            mean_dt_us,
            rhythm: rhythm_val,
            rhythm_blocked,
            hist,
        };
    }

    /// Close the current tick. Returns the column plus any rising-edge alerts.
    ///
    /// `all_mean_dt_us`: mean gap of the ALL lane (None when this *is* ALL).
    pub fn finish_tick(&mut self, all_mean_dt_us: Option<f64>, alerts: &mut Vec<LaneAlert>) -> Col {
        self.det.tick += 1;
        self.det.obs_seen = self.det.obs_seen.saturating_add(u64::from(self.tick_n));
        let n_tick = self.tick_n;
        let bytes_tick = self.tick_bytes;
        self.tick_n = 0;
        self.tick_bytes = 0;
        self.last_n = n_tick;
        self.last_bytes = bytes_tick;

        // --- volume baseline (every tick, including quiet ones) ---
        let x = (1.0 + f64::from(n_tick)).ln();
        let load_warm = self.det.load.n >= WARM_LOAD_TICKS;
        let load_z = (x - self.det.load.mean) / self.det.load.sd().max(0.25);
        let was_burst = self.burst;
        if self.burst {
            if load_z < 2.0 || n_tick < 4 {
                self.burst = false;
            }
        } else if load_warm && n_tick >= 8 && load_z >= 3.5 {
            self.burst = true;
        }
        let typical = self.det.load.mean.exp() - 1.0;
        self.det
            .load
            .update(x, if self.burst { 0.002 } else { 0.01 });
        if self.burst && !was_burst {
            self.det.alert(
                alerts,
                AlertKind::Burst,
                format!("^ burst: {n_tick} events in one tick (typical ~{typical:.1})"),
            );
        }

        if n_tick == 0 {
            self.body = Body::Void;
            return Col::default();
        }

        self.compute_metrics(all_mean_dt_us);
        let m = self.metrics;
        self.body = classify(&m);
        let valid = m.n >= MIN_N;

        // --- rhythm (cap) ---
        let was_pulse = self.pulse;
        match m.rhythm {
            // Lock only after 3 consecutive clear readings: kills lucky flukes.
            Some(r) if r.cv <= 0.15 => {
                self.det.pulse_streak = self.det.pulse_streak.saturating_add(1);
                if self.det.pulse_streak >= 3 {
                    self.pulse = true;
                }
            }
            Some(r) if r.cv > 0.35 => {
                self.pulse = false;
                self.det.pulse_streak = 0;
            }
            // Rejected as carrier-riding this tick: hold, don't flap.
            None if m.rhythm_blocked => {}
            None => {
                self.pulse = false;
                self.det.pulse_streak = 0;
            }
            // In the hysteresis band: keep the current state.
            _ => self.det.pulse_streak = 0,
        }
        if self.pulse && !was_pulse {
            if let Some(r) = m.rhythm {
                self.det.alert(
                    alerts,
                    AlertKind::Pulse,
                    format!(
                        "| rhythm locked: repeats every {} (cv {:.2})",
                        fmt_period(r.period_us),
                        r.cv
                    ),
                );
            }
        }

        // --- demon: entropy-rate drop vs baseline (one-sided CUSUM) ---
        // Fires only for a drop that is persistent (CUSUM) and bigger than this
        // lane's normal wobble: max(min(0.5 bit, half the baseline), 3 sd). Lanes
        // that swing between busy and quiet regimes learn a wide band and stay
        // calm; lanes that are already fairly ordered need a proportionally
        // smaller (but still clear) drop.
        if valid {
            // Light smoothing (~0.5 s): damps estimator jitter from small windows
            // and the shuffle test, so the gate reflects real regime swings while
            // sustained changes pass straight through.
            let y = match self.det.hr_smooth {
                Some(prev) => prev + 0.25 * (m.h_rate - prev),
                None => m.h_rate,
            };
            self.det.hr_smooth = Some(y);
            let rate = self.det.rate;
            let first = *self.det.first_valid.get_or_insert(self.det.tick);
            // Warm by time (slow lanes) or by volume: 32 full windows' worth of
            // observations is plenty for a fast, steady lane such as raw bytes.
            let warm = (rate.n >= WARM_TICKS && self.det.tick - first >= WARM_SPAN_TICKS)
                || (rate.n >= 16 && self.det.obs_seen >= 32 * self.window as u64);
            let sd = rate.sd().max(0.1);
            let gate = (3.0 * sd).max(0.5_f64.min(0.5 * rate.mean));
            let s = (rate.mean - y) / sd;
            self.det.cusum = (self.det.cusum + s - 1.0).clamp(0.0, 50.0);
            let was_demon = self.demon;
            if self.demon {
                if self.det.cusum < 2.0 || y > rate.mean - 0.5 * gate {
                    self.demon = false;
                }
            } else if warm && self.det.cusum >= 8.0 && y <= rate.mean - gate {
                self.demon = true;
            }
            if self.demon && !was_demon {
                let (mean, z) = (rate.mean, m.dep.z);
                self.det.alert(
                    alerts,
                    AlertKind::Demon,
                    format!("@ order appeared: entropy rate {mean:.2} -> {y:.2} bits/sym (dep z {z:.1})"),
                );
            }
            // Keep learning while flagged, at half speed: a lasting new regime is
            // absorbed in ~10-15 s instead of being flagged forever. Before the
            // baseline is warm, clip outliers so an early demon can't hide itself.
            let fed = if warm { y } else { y.max(rate.mean - gate) };
            self.det
                .rate
                .update(fed, if self.demon { 0.01 } else { 0.02 });
        }

        // --- shift: symbol mix vs baseline (KL divergence) ---
        // The trigger adapts to how far this lane's mix normally wanders (an EWMA
        // of its own KL values), so bursty mixed lanes don't cry wolf.
        if m.n >= 2 * MIN_N {
            let nonzero = m.hist.iter().filter(|&&c| c > 0).count();
            let bias =
                nonzero.saturating_sub(1) as f64 / (2.0 * m.n as f64 * std::f64::consts::LN_2);
            let q_warm = self.det.q_n >= WARM_TICKS;
            self.kl_excess = if q_warm {
                (kl_bits(&m.hist, m.n, &self.det.q) - bias).max(0.0)
            } else {
                0.0
            };
            let kb = self.det.kl_base;
            let on_level = (kb.mean + 4.0 * kb.sd()).max(0.8);
            let off_level = (kb.mean + 1.5 * kb.sd()).max(0.3);
            let was_shift = self.shift;
            if self.shift {
                if self.kl_excess < off_level {
                    self.shift = false;
                    self.det.shift_streak = 0;
                }
            } else if kb.n >= 20 && self.kl_excess >= on_level {
                self.det.shift_streak = self.det.shift_streak.saturating_add(1);
                if self.det.shift_streak >= 2 {
                    self.shift = true;
                }
            } else {
                self.det.shift_streak = 0;
            }
            if self.shift && !was_shift {
                let kl = self.kl_excess;
                self.det.alert(
                    alerts,
                    AlertKind::Shift,
                    format!("! mix shifted: KL {kl:.2} bits away from baseline"),
                );
            }
            if q_warm && !self.shift {
                // Winsorised: a big shift can't inflate its own trigger level.
                self.det.kl_base.update(self.kl_excess.min(on_level), 0.02);
            }
            // Mix baseline update (warm start, half speed while shifted).
            self.det.q_n = self.det.q_n.saturating_add(1);
            let a = if self.shift { 0.01_f64 } else { 0.02_f64 }.max(1.0 / f64::from(self.det.q_n));
            let total = m.n as f64;
            for (q, &c) in self.det.q.iter_mut().zip(m.hist.iter()) {
                *q += a * (f64::from(c) / total - *q);
            }
        }

        let cap = if self.burst {
            Cap::Burst
        } else if self.pulse {
            Cap::Pulse
        } else {
            Cap::None
        };
        let mark = if self.demon {
            Mark::Demon
        } else if self.shift {
            Mark::Shift
        } else {
            Mark::None
        };
        Col {
            n: n_tick,
            level: 0.0,
            body: self.body,
            cap,
            mark,
        }
    }
}

/// Human period: "20ms", "1.5s", "2m05s".
pub fn fmt_period(us: f64) -> String {
    if us < 1_000.0 {
        format!("{us:.0}us")
    } else if us < 1_000_000.0 {
        format!("{:.0}ms", us / 1_000.0)
    } else if us < 60_000_000.0 {
        format!("{:.1}s", us / 1_000_000.0)
    } else {
        let s = (us / 1_000_000.0).round() as u64;
        format!("{}m{:02}s", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn random_seq(rng: &mut Rng, n: usize, alphabet: u64) -> Vec<u8> {
        (0..n).map(|_| rng.range(0, alphabet) as u8).collect()
    }

    #[test]
    fn entropy_basics() {
        assert_eq!(entropy_bits(&[0, 0, 0]), 0.0);
        assert_eq!(entropy_bits(&[7, 0, 0]), 0.0);
        assert!((entropy_bits(&[5, 5, 5, 5]) - 2.0).abs() < 1e-12);
        assert!((entropy_bits(&[1; 16]) - 4.0).abs() < 1e-12);
    }

    #[test]
    fn mi_alternating_is_one_bit() {
        // Plug-in estimate on 399 pairs: 0.999994 bits, i.e. 1 bit up to sampling.
        let seq: Vec<u8> = (0..400).map(|i| (i % 2) as u8).collect();
        assert!((lag1_mi(&seq) - 1.0).abs() < 1e-4);
        assert_eq!(lag1_mi(&[3]), 0.0);
        assert_eq!(lag1_mi(&[]), 0.0);
    }

    #[test]
    fn permutation_test_separates_order_from_noise() {
        let mut rng = Rng::new(42);
        let mut scratch = Vec::new();
        // Alternation: overwhelming dependency.
        let alt: Vec<u8> = (0..256).map(|i| if i % 2 == 0 { 3 } else { 11 }).collect();
        let d = dependency(&alt, SHUFFLES, &mut rng, &mut scratch);
        assert!(d.z > 20.0, "alternation z = {}", d.z);

        // IID noise: false alarm rate at z >= 3 must be small.
        let mut false_alarms = 0;
        for _ in 0..200 {
            let seq = random_seq(&mut rng, 256, 16);
            let d = dependency(&seq, SHUFFLES, &mut rng, &mut scratch);
            if d.z >= 3.0 {
                false_alarms += 1;
            }
        }
        assert!(false_alarms <= 10, "false alarms: {false_alarms}/200");
    }

    #[test]
    fn sticky_markov_is_detected() {
        let mut rng = Rng::new(9);
        let mut scratch = Vec::new();
        let mut s = 0u8;
        let seq: Vec<u8> = (0..256)
            .map(|_| {
                if rng.chance(0.15) {
                    s = rng.range(0, 16) as u8;
                }
                s
            })
            .collect();
        let d = dependency(&seq, SHUFFLES, &mut rng, &mut scratch);
        assert!(d.z > 5.0, "sticky z = {}", d.z);
    }

    #[test]
    fn rhythm_metronome_vs_poisson() {
        let mut rng = Rng::new(5);
        // 20 ms period with ~1% jitter.
        let mut t = 0.0f64;
        let metro: Vec<u64> = (0..RHYTHM_TS)
            .map(|_| {
                t += 20_000.0 + rng.normal() * 200.0;
                t as u64
            })
            .collect();
        let r = rhythm(&metro).expect("enough samples");
        assert!(r.cv < 0.05 && r.lag == 1, "{r:?}");
        assert!((r.period_us - 20_000.0).abs() < 500.0);

        // Request/response beacon: 30 ms then 370 ms, repeating -> lag 2 wins.
        let mut t = 0u64;
        let beacon: Vec<u64> = (0..RHYTHM_TS)
            .map(|i| {
                t += if i % 2 == 0 { 30_000 } else { 370_000 };
                t
            })
            .collect();
        let r = rhythm(&beacon).expect("enough samples");
        assert!(r.cv < 0.01 && r.lag == 2, "{r:?}");
        assert!((r.period_us - 400_000.0).abs() < 1.0);

        // Evenly spaced bursts at random times (like mail exchanges): not a rhythm.
        // (Bursts at exactly periodic times *would* be a rhythm, correctly.)
        let mut brng = Rng::new(21);
        let mut t = 0u64;
        let mut bursts = Vec::new();
        for i in 0..RHYTHM_TS as u64 {
            t += if i % 4 == 0 {
                1_000_000 + brng.exp(4_000_000.0) as u64
            } else {
                40_000
            };
            bursts.push(t);
        }
        assert!(
            rhythm(&bursts).is_none_or(|r| r.cv > 0.35),
            "{:?}",
            rhythm(&bursts)
        );

        // A few unrelated packets interleaved: robust stats keep the lock.
        let mut noisy = beacon.clone();
        noisy.extend([1_234_567u64, 3_000_123, 5_555_555]);
        noisy.sort_unstable();
        let r = rhythm(&noisy).expect("enough samples");
        assert!(
            r.cv <= 0.2 && r.lag == 2,
            "interlopers broke the lock: {r:?}"
        );

        // Poisson arrivals stay well above the 0.2 pulse threshold.
        let mut low = 0;
        for seed in 0..100 {
            let mut rng = Rng::new(seed);
            let mut t = 0.0;
            let pois: Vec<u64> = (0..RHYTHM_TS)
                .map(|_| {
                    t += rng.exp(50_000.0);
                    t as u64
                })
                .collect();
            if rhythm(&pois).is_some_and(|r| r.cv <= 0.2) {
                low += 1;
            }
        }
        assert_eq!(low, 0, "poisson trains mistaken for rhythm: {low}/100");
        assert!(rhythm(&[1, 2, 3]).is_none());
    }

    #[test]
    fn classification() {
        let spec = LaneSpec {
            name: "T",
            timing: true,
        };
        let run = |seq: &[u8]| {
            let mut lane = LaneState::new(spec, 256, 1);
            for (i, &s) in seq.iter().enumerate() {
                lane.push(i as u64 * 1000, s, 1);
            }
            let mut alerts = Vec::new();
            lane.finish_tick(None, &mut alerts).body
        };
        let mut rng = Rng::new(77);
        assert_eq!(run(&[4; 200]), Body::Mono);
        let alt: Vec<u8> = (0..200).map(|i| if i % 2 == 0 { 2 } else { 9 }).collect();
        assert_eq!(run(&alt), Body::Order);
        assert_eq!(run(&random_seq(&mut rng, 256, 16)), Body::Noise);
        assert_eq!(run(&[1, 2, 3]), Body::Sparse);
    }

    #[test]
    fn demon_fires_when_order_is_injected() {
        let spec = LaneSpec {
            name: "T",
            timing: false,
        };
        let mut lane = LaneState::new(spec, 128, 3);
        let mut rng = Rng::new(11);
        let mut t = 0u64;
        let mut alerts = Vec::new();
        // 80 ticks of noise to learn the baseline (> WARM_SPAN_TICKS).
        for _ in 0..80 {
            for _ in 0..64 {
                t += 1000;
                lane.push(t, rng.range(0, 16) as u8, 1);
            }
            lane.finish_tick(None, &mut alerts);
        }
        assert!(
            alerts.iter().all(|a| a.kind != AlertKind::Demon),
            "no demon during noise: {alerts:?}"
        );
        // Then a rigid repeating pattern.
        let mut fired = false;
        for _ in 0..20 {
            for i in 0..64u64 {
                t += 1000;
                lane.push(t, (i % 4) as u8, 1);
            }
            lane.finish_tick(None, &mut alerts);
            fired |= lane.demon;
        }
        assert!(fired, "demon should fire; alerts: {alerts:?}");
        assert!(alerts.iter().any(|a| a.kind == AlertKind::Demon));
    }

    #[test]
    fn period_formatting() {
        assert_eq!(fmt_period(500.0), "500us");
        assert_eq!(fmt_period(20_000.0), "20ms");
        assert_eq!(fmt_period(1_500_000.0), "1.5s");
        assert_eq!(fmt_period(125_000_000.0), "2m05s");
    }
}
