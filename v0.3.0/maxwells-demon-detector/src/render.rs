//! Turns the model into a [`Frame`]: title bar, landscape bands, side panel,
//! time axis, legend and alerts.
//!
//! Glyph grammar (every symbol means exactly one thing):
//!
//! ```text
//!   @ demon (order appeared)   ! shift (mix changed)      <- marker above a column
//!   ^ burst (volume spike)     | rhythm (metronomic)      <- cap on top of a column
//!   # mono   = order   ~ flow   :;', noise   . sparse     <- body texture
//!   _ idle ground                                          height = volume (log)
//! ```

use crate::analysis::{AlertKind, Body, Cap, Col, Mark};
use crate::app::App;
use crate::screen::{Frame, Style, Tone};
use crate::source::SourceStatus;

pub const LABEL_W: u16 = 6;
pub const PANEL_W: u16 = 24;
pub const MIN_W: u16 = 40;
const MAX_BAND: u16 = 6;

/// Everything the renderer needs besides the model.
pub struct View<'a> {
    pub status: &'a SourceStatus,
    pub tick_ms: u64,
    pub paused: bool,
    pub show_legend: bool,
    pub recording: bool,
    pub record_error: Option<&'a str>,
    /// Show key hints (TUI only).
    pub hints: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// (top row, height) per lane; lane 0 (ALL) first.
    pub bands: Vec<(u16, u16)>,
    pub land_x: u16,
    pub land_w: u16,
    pub panel_x: Option<u16>,
    pub axis_y: u16,
    pub legend_y: u16,
    pub legend_rows: u16,
    pub alerts_y: u16,
    pub alerts_rows: u16,
}

/// Compute the screen layout, or `None` if the terminal is too small.
pub fn layout(w: u16, h: u16, lanes: usize, show_legend: bool) -> Option<Layout> {
    let lanes_u = u16::try_from(lanes).ok().filter(|&n| n >= 1)?;
    let weight = lanes_u + 1; // ALL counts double
    if w < MIN_W || h < 2 + weight {
        return None;
    }
    let panel = w >= 96;
    let land_x = LABEL_W;
    let land_w = w - LABEL_W - if panel { PANEL_W + 1 } else { 0 };
    let avail = h - 2; // title + axis
    let mut legend_rows: u16 = match (show_legend, w >= 112) {
        (false, _) => 0,
        (true, true) => 1,
        (true, false) => 2,
    };
    let mut alerts_min: u16 = 2;
    while avail < weight + legend_rows + alerts_min {
        if alerts_min > 0 {
            alerts_min -= 1;
        } else if legend_rows > 0 {
            legend_rows -= 1;
        } else {
            break;
        }
    }
    let budget = avail - legend_rows - alerts_min;
    let base = (budget / weight).clamp(1, MAX_BAND);
    let leftover = budget.saturating_sub(base * weight);
    let all_h = 2 * base + leftover.min(base);

    let mut bands = Vec::with_capacity(lanes);
    let mut y = 1u16;
    bands.push((y, all_h));
    y += all_h;
    for _ in 1..lanes {
        bands.push((y, base));
        y += base;
    }
    let axis_y = y;
    let legend_y = axis_y + 1;
    let alerts_y = legend_y + legend_rows;
    Some(Layout {
        bands,
        land_x,
        land_w,
        panel_x: panel.then_some(land_x + land_w + 1),
        axis_y,
        legend_y,
        legend_rows,
        alerts_y,
        alerts_rows: h.saturating_sub(alerts_y),
    })
}

pub fn body_style(b: Body) -> Style {
    match b {
        Body::Void | Body::Sparse => Style::fg(Tone::Dim),
        Body::Noise => Style::fg(Tone::Grey),
        Body::Flow => Style::fg(Tone::Cyan),
        Body::Order => Style::fg(Tone::Green),
        Body::Mono => Style::bold(Tone::Blue),
    }
}

fn kind_tone(k: AlertKind) -> Tone {
    match k {
        AlertKind::Demon => Tone::Red,
        AlertKind::Shift => Tone::Yellow,
        AlertKind::Burst => Tone::White,
        AlertKind::Pulse => Tone::Magenta,
    }
}

/// Stable per-cell grain for noise, so texture scrolls with the terrain instead of boiling.
fn noise_glyph(id: u64, lane: usize, row: u16) -> u8 {
    const SET: &[u8] = b":;:',;";
    let mut z = id.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (lane as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93)
        ^ u64::from(row).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z ^= z >> 29;
    z = z.wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 32;
    SET[(z % SET.len() as u64) as usize]
}

fn draw_column(f: &mut Frame, x: u16, bottom: u16, height: u16, col: &Col, id: u64, lane: usize) {
    if col.n == 0 || col.body == Body::Void {
        f.put(x, bottom, b'_', Style::fg(Tone::Dim));
        return;
    }
    let rows = ((col.level * f32::from(height)).round() as u16).clamp(1, height);
    let style = body_style(col.body);
    for r in 0..rows {
        let g = match col.body {
            Body::Noise => noise_glyph(id, lane, r),
            Body::Sparse => b'.',
            Body::Flow => b'~',
            Body::Order => b'=',
            Body::Mono => b'#',
            Body::Void => b'_',
        };
        f.put(x, bottom - r, g, style);
    }
    let top = bottom + 1 - rows;
    match col.cap {
        Cap::Burst => f.put(x, top, b'^', Style::bold(Tone::White)),
        Cap::Pulse => f.put(x, top, b'|', Style::bold(Tone::Magenta)),
        Cap::None => {}
    }
    let mark = match col.mark {
        Mark::Demon => Some((b'@', Style::bold(Tone::Red))),
        Mark::Shift => Some((b'!', Style::bold(Tone::Yellow))),
        Mark::None => None,
    };
    if let Some((g, s)) = mark {
        let y = if rows < height { top - 1 } else { top };
        f.put(x, y, g, s);
    }
}

pub fn fmt_count(x: f64) -> String {
    if !x.is_finite() || x < 0.0 {
        "0".to_string()
    } else if x < 1_000.0 {
        format!("{x:.0}")
    } else if x < 1_000_000.0 {
        format!("{:.1}k", x / 1_000.0)
    } else {
        format!("{:.1}M", x / 1_000_000.0)
    }
}

pub fn fmt_bytes(x: f64) -> String {
    if !x.is_finite() || x < 0.0 {
        "0B".to_string()
    } else if x < 1_024.0 {
        format!("{x:.0}B")
    } else if x < 1_048_576.0 {
        format!("{:.1}KB", x / 1_024.0)
    } else {
        format!("{:.1}MB", x / 1_048_576.0)
    }
}

fn draw_title(f: &mut Frame, app: &App, view: &View) {
    f.fill_row(0, Style::title(false));
    let mut left = format!(
        " MAXWELL'S DEMON DETECTOR | {} | {} ev/s {}/s | tick {}ms",
        view.status.label,
        fmt_count(app.ev_rate),
        fmt_bytes(app.byte_rate),
        view.tick_ms
    );
    if view.paused {
        left.push_str(" | PAUSED");
    }
    if app.warming_up() {
        left.push_str(" | learning");
    }
    if view.status.dropped > 0 {
        left.push_str(&format!(" | drop {}", view.status.dropped));
    }
    if view.status.bad > 0 {
        left.push_str(&format!(" | bad {}", view.status.bad));
    }
    if view.record_error.is_some() {
        left.push_str(" | REC FAILED");
    } else if view.recording {
        left.push_str(" | REC");
    }
    let right = " spc:pause +/-:speed l:legend c:colour q:quit ";
    let w = f.w;
    let used = f.text(0, 0, &left, Style::title(true), w);
    let rlen = right.len() as u16;
    if view.hints && used + rlen < w {
        f.text(w - rlen, 0, right, Style::title(false), rlen);
    }
}

fn draw_band(f: &mut Frame, app: &App, lay: &Layout, lane: usize) {
    let (top, height) = lay.bands[lane];
    if height == 0 {
        return;
    }
    let bottom = top + height - 1;
    let name = app.lanes[lane].spec.name;
    let label_style = if lane == 0 {
        Style::bold(Tone::White)
    } else {
        Style::fg(Tone::Grey)
    };
    f.text(0, bottom, &format!("{name:>5} "), label_style, LABEL_W);

    let hist = &app.history[lane];
    let n = hist.len().min(app.cols.len());
    let hist_off = hist.len() - n;
    let cols_off = app.cols.len() - n;
    for sx in 0..lay.land_w {
        let back = usize::from(lay.land_w - 1 - sx);
        if back >= n {
            continue;
        }
        let i = n - 1 - back;
        let col = &hist[hist_off + i];
        let id = app.cols[cols_off + i].id;
        draw_column(f, lay.land_x + sx, bottom, height, col, id, lane);
    }

    if let Some(px) = lay.panel_x {
        draw_panel(f, app, lane, top, height, px);
    }
}

fn draw_panel(f: &mut Frame, app: &App, lane: usize, top: u16, height: u16, x0: u16) {
    let st = &app.lanes[lane];
    let body = app.history[lane].back().map_or(Body::Void, |c| c.body);
    let (ev_rate, byte_rate) = app.lane_rates.get(lane).copied().unwrap_or((0.0, 0.0));

    let mut x = x0;
    x += f.text(x, top, &format!("{:<6}", body.name()), body_style(body), 6);
    let flags = [
        (st.demon, b'@', Style::bold(Tone::Red)),
        (st.shift, b'!', Style::bold(Tone::Yellow)),
        (st.burst, b'^', Style::bold(Tone::White)),
        (st.pulse, b'|', Style::bold(Tone::Magenta)),
    ];
    for (on, g, s) in flags {
        if on {
            f.put(x, top, g, s);
            x += 1;
        }
    }
    let rate = format!("{}/s", fmt_count(ev_rate));
    let rlen = rate.len() as u16;
    if x + 1 + rlen <= x0 + PANEL_W {
        f.text(x0 + PANEL_W - rlen, top, &rate, Style::fg(Tone::Grey), rlen);
    }

    let m = &st.metrics;
    let lines: Vec<String> = if m.n == 0 {
        vec!["no data yet".to_string()]
    } else {
        let cv = m
            .rhythm
            .map_or_else(|| "  -".to_string(), |r| format!("{:.2}", r.cv));
        vec![
            format!("H{:5.2} dep z{:>7.1}", m.h, m.dep.z),
            format!("Hr{:5.2}  cv {cv}", m.h_rate),
            format!(
                "KL{:5.2} {:>11}",
                st.kl_excess,
                format!("{}/s", fmt_bytes(byte_rate))
            ),
        ]
    };
    for (k, line) in lines.iter().enumerate() {
        let y = top + 1 + k as u16;
        if y > top + height - 1 {
            break;
        }
        f.text(x0, y, line, Style::fg(Tone::Dim), PANEL_W);
    }
}

fn step_label(secs: u64) -> String {
    if secs >= 60 && secs.is_multiple_of(60) {
        format!("-{}m", secs / 60)
    } else {
        format!("-{secs}s")
    }
}

fn draw_axis(f: &mut Frame, app: &App, lay: &Layout, tick_ms: u64) {
    let y = lay.axis_y;
    let n = app.cols.len();
    if n == 0 || lay.land_w < 8 {
        return;
    }
    let t_new = app.cols[n - 1].t;
    let span = n.min(16);
    let spc = if span >= 2 {
        ((t_new - app.cols[n - span].t) / (span - 1) as f64).max(1e-3)
    } else {
        tick_ms as f64 / 1_000.0
    };
    const STEPS: [u64; 12] = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1_800, 3_600];
    let step = STEPS
        .iter()
        .copied()
        .find(|&s| s as f64 / spc >= 14.0)
        .unwrap_or(3_600);

    let x_end = lay.land_x + lay.land_w; // exclusive
    for sx in 0..lay.land_w {
        let back = usize::from(lay.land_w - 1 - sx);
        if back < n {
            f.put(lay.land_x + sx, y, b'-', Style::fg(Tone::Dim));
        }
    }
    let now_x = x_end - 3;
    f.text(now_x, y, "now", Style::fg(Tone::Grey), 3);

    let mut limit = now_x; // labels must end before this x
    let mut k = 1u64;
    for sx in (0..lay.land_w).rev() {
        let back = usize::from(lay.land_w - 1 - sx);
        if back >= n {
            break;
        }
        let age = t_new - app.cols[n - 1 - back].t;
        if age + 1e-9 < (k * step) as f64 {
            continue;
        }
        let x = lay.land_x + sx;
        let label = step_label(k * step);
        let need = 1 + label.len() as u16;
        if x + need < limit {
            f.put(x, y, b'+', Style::fg(Tone::Grey));
            f.text(x + 1, y, &label, Style::fg(Tone::Grey), need - 1);
            limit = x;
        }
        while age + 1e-9 >= (k * step) as f64 {
            k += 1;
        }
    }
}

const LEGEND: [(&str, Style, &str); 10] = [
    ("_", Style::fg(Tone::Dim), "idle"),
    (".", Style::fg(Tone::Dim), "sparse"),
    (":;", Style::fg(Tone::Grey), "noise"),
    ("~", Style::fg(Tone::Cyan), "flow"),
    ("=", Style::fg(Tone::Green), "order"),
    ("#", Style::bold(Tone::Blue), "mono"),
    ("|", Style::bold(Tone::Magenta), "rhythm"),
    ("^", Style::bold(Tone::White), "burst"),
    ("!", Style::bold(Tone::Yellow), "shift"),
    ("@", Style::bold(Tone::Red), "demon"),
];

fn draw_legend(f: &mut Frame, lay: &Layout) {
    if lay.legend_rows == 0 {
        return;
    }
    let mut row = 0u16;
    let mut x = 1u16;
    let items = LEGEND
        .iter()
        .map(|&(g, s, l)| (g, s, l))
        .chain(std::iter::once((
            "height",
            Style::fg(Tone::Grey),
            "= volume",
        )));
    for (g, s, label) in items {
        let width = (g.len() + 1 + label.len() + 2) as u16;
        if x + width > f.w {
            row += 1;
            x = 1;
            if row >= lay.legend_rows {
                return;
            }
        }
        let y = lay.legend_y + row;
        x += f.text(x, y, g, s, f.w - x);
        x += 1;
        x += f.text(x, y, label, Style::fg(Tone::Dim), f.w.saturating_sub(x));
        x += 2;
    }
}

fn draw_alerts(f: &mut Frame, app: &App, lay: &Layout, view: &View) {
    if lay.alerts_rows == 0 {
        return;
    }
    let mut y = lay.alerts_y;
    let end = lay.alerts_y + lay.alerts_rows;
    if let Some(err) = view.record_error {
        f.text(1, y, err, Style::bold(Tone::Red), f.w.saturating_sub(1));
        y += 1;
    }
    if app.alerts.is_empty() && y < end {
        let hint = if app.warming_up() {
            "learning what normal looks like for each lane..."
        } else {
            "no alerts: every lane looks like its own recent past"
        };
        f.text(1, y, hint, Style::fg(Tone::Dim), f.w.saturating_sub(1));
        return;
    }
    for a in app.alerts.iter() {
        if y >= end {
            break;
        }
        let secs = a.t.max(0.0) as u64;
        let tone = kind_tone(a.kind);
        let mut x = 1;
        x += f.text(
            x,
            y,
            &format!("{:02}:{:02} ", secs / 60, secs % 60),
            Style::fg(Tone::Dim),
            6,
        );
        x += f.text(x, y, &format!("{:<6}", a.lane), Style::bold(tone), 6);
        f.text(x, y, &a.msg, Style::fg(tone), f.w.saturating_sub(x));
        y += 1;
    }
}

/// Build the whole frame for a `w` x `h` terminal.
pub fn render(app: &App, view: &View, w: u16, h: u16) -> Frame {
    let mut f = Frame::new(w, h);
    if w == 0 || h == 0 {
        return f;
    }
    let Some(lay) = layout(w, h, app.lanes.len(), view.show_legend) else {
        let need_h = app.lanes.len() as u16 + 3;
        let msg = format!("terminal too small: need {MIN_W}x{need_h}, have {w}x{h}");
        let y = h / 2;
        let x = w.saturating_sub(msg.len() as u16) / 2;
        f.text(x, y, &msg, Style::bold(Tone::Yellow), w);
        return f;
    };
    draw_title(&mut f, app, view);
    for lane in 0..app.lanes.len() {
        draw_band(&mut f, app, &lay, lane);
    }
    draw_axis(&mut f, app, &lay, view.tick_ms);
    draw_legend(&mut f, &lay);
    draw_alerts(&mut f, app, &lay, view);
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{byte_obs, LaneSet};

    fn status() -> SourceStatus {
        SourceStatus {
            label: "test".into(),
            ..SourceStatus::default()
        }
    }

    #[test]
    fn layout_fits_every_size() {
        for lanes in [6usize, 8] {
            for w in (0..=260).step_by(7) {
                for h in 0..=90 {
                    let Some(l) = layout(w, h, lanes, true) else {
                        continue;
                    };
                    assert_eq!(l.bands.len(), lanes);
                    assert!(l.bands.iter().all(|&(_, bh)| bh >= 1));
                    assert!(l.axis_y < h, "axis inside screen w={w} h={h}");
                    assert!(l.alerts_y + l.alerts_rows <= h);
                    assert!(l.land_x + l.land_w <= w);
                    if let Some(px) = l.panel_x {
                        assert!(px + PANEL_W <= w);
                    }
                }
            }
        }
    }

    #[test]
    fn render_any_size_is_ascii_and_never_panics() {
        let mut app = App::new(LaneSet::Bytes, 64, 20);
        let mut t = 0u64;
        for tick in 0..300u64 {
            for i in 0..80u64 {
                t += 250;
                let b = if tick % 50 < 25 {
                    (i * 37 % 251) as u8
                } else {
                    0
                };
                app.ingest(&byte_obs(b, t));
            }
            app.finish_tick(tick as f64 * 0.125);
        }
        let st = status();
        for (w, h) in [
            (0, 0),
            (1, 1),
            (39, 20),
            (40, 9),
            (40, 10),
            (80, 24),
            (95, 30),
            (96, 30),
            (120, 36),
            (300, 100),
        ] {
            for legend in [true, false] {
                let view = View {
                    status: &st,
                    tick_ms: 125,
                    paused: false,
                    show_legend: legend,
                    recording: false,
                    record_error: None,
                    hints: true,
                };
                let frame = render(&app, &view, w, h);
                let text = frame.to_text();
                assert!(text.is_ascii());
                assert_eq!(frame.w, w);
                assert_eq!(text.lines().count(), usize::from(h));
            }
        }
    }

    #[test]
    fn column_drawing_rules() {
        let mut f = Frame::new(10, 6);
        let col = Col {
            n: 50,
            level: 0.5,
            body: Body::Order,
            cap: Cap::Pulse,
            mark: Mark::Demon,
        };
        draw_column(&mut f, 2, 4, 4, &col, 7, 1);
        // rows = round(0.5*4) = 2: body at y=4, cap at y=3, marker hovering at y=2.
        assert_eq!(f.cell(2, 4).ch, b'=');
        assert_eq!(f.cell(2, 3).ch, b'|');
        assert_eq!(f.cell(2, 2).ch, b'@');
        assert_eq!(f.cell(2, 1).ch, b' ');
        let idle = Col::default();
        draw_column(&mut f, 3, 4, 4, &idle, 8, 1);
        assert_eq!(f.cell(3, 4).ch, b'_');
    }

    #[test]
    fn formatting() {
        assert_eq!(fmt_count(12.4), "12");
        assert_eq!(fmt_count(1_540.0), "1.5k");
        assert_eq!(fmt_bytes(2_048.0), "2.0KB");
        assert_eq!(fmt_bytes(f64::NAN), "0B");
        assert_eq!(step_label(30), "-30s");
        assert_eq!(step_label(120), "-2m");
    }
}
