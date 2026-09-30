use std::error::Error;
use std::io::Write;

use crate::cli::Mode;
use crate::model::AppState;

pub fn draw<W: Write>(out: &mut W, state: &AppState) -> Result<(), Box<dyn Error>> {
    write!(out, "\x1b[2J\x1b[H")?;

    let mode = match state.mode {
        Mode::Demo => "demo",
        Mode::Replay => "replay",
        Mode::StdinJsonl => "stdin-jsonl",
        Mode::ListInterfaces => "list-interfaces",
        Mode::LivePcap => "live-pcap",
    };

    let bytes_per_sec = if state.uptime_secs() > 0 {
        state.total_bytes / state.uptime_secs()
    } else {
        0
    };
    let events_per_sec = if state.uptime_secs() > 0 {
        state.total_events / state.uptime_secs()
    } else {
        0
    };

    writeln!(
        out,
        "Maxwell's Demon Detector  |  mode={}  |  tick={}ms  |  q/esc quits",
        mode, state.tick_ms
    )?;
    writeln!(
        out,
        "events={}  bytes={}  approx_eps={}  approx_Bps={}  uptime={}s",
        state.total_events,
        state.total_bytes,
        events_per_sec,
        bytes_per_sec,
        state.uptime_secs()
    )?;
    writeln!(
        out,
        "Each cell is one time bucket. Glyph intensity blends load and packet-size entropy. ! = anomalous bucket."
    )?;
    writeln!(out)?;

    for (i, lane) in state.lanes.iter().enumerate() {
        let row = &state.history[i];
        let mut line = String::with_capacity(row.len());
        for cell in row {
            line.push(cell.glyph());
        }
        let tail = state
            .current_cell(i)
            .map(|c| format!(" count={:<4} bytes={:<6} H={:.2}", c.count, c.bytes, c.entropy))
            .unwrap_or_default();
        writeln!(out, "{:<6} {}{}", lane.label(), line, tail)?;
    }

    writeln!(out)?;
    writeln!(out, "Legend: ' ' quiet  .,:; medium  o x % # @ heavy/structured  ! anomaly")?;
    writeln!(out)?;
    writeln!(out, "Recent alerts:")?;
    if state.alerts.is_empty() {
        writeln!(out, "  (none yet)")?;
    } else {
        for line in state.alerts.iter().take(state.max_alerts) {
            writeln!(out, "  - {}", line)?;
        }
    }

    Ok(())
}
