//! Command-line interface.

use std::path::PathBuf;

use clap::{ArgAction, Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    /// Scripted synthetic traffic with injected demons (no input needed)
    Demo,
    /// Packet events from a .csv or .jsonl file
    Replay,
    /// Packet events as JSONL lines on stdin
    StdinJsonl,
    /// Raw bytes from any file (or '-' for stdin)
    Bytes,
    /// A .pcap or .pcapng capture file (Wireshark, tcpdump, pktmon)
    PcapFile,
    /// Live capture (build with --features live)
    LivePcap,
    /// List capture interfaces and exit (build with --features live)
    ListInterfaces,
}

fn parse_speed(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|_| format!("not a number: {s}"))?;
    if v.is_finite() && v > 0.0 && v <= 100_000.0 {
        Ok(v)
    } else {
        Err("speed must be > 0 and <= 100000".to_string())
    }
}

/// Maxwell's Demon Detector: watch structure flow through a data stream as an ASCII landscape.
///
/// Columns scroll right-to-left (newest on the right). Each band is one lane.
/// Height = volume. Texture = what kind of structure the lane has right now.
/// Caps and markers flag rhythm, bursts, shifts and demons (order appearing).
#[derive(Debug, Clone, Parser)]
#[command(name = "mdd", version, about, long_about = None)]
pub struct Cli {
    /// What to visualise
    #[arg(long, value_enum, default_value_t = Mode::Demo)]
    pub mode: Mode,

    /// Input file (replay: .csv/.jsonl; bytes: any file or '-'; pcap-file: .pcap/.pcapng)
    #[arg(long, short = 'i')]
    pub input: Option<PathBuf>,

    /// Milliseconds per landscape column (scroll speed). Adjust live with + and -
    #[arg(long, default_value_t = 125, value_parser = clap::value_parser!(u64).range(20..=5000))]
    pub tick_ms: u64,

    /// Playback speed multiplier for replay and pcap-file
    #[arg(long, default_value_t = 1.0, value_parser = parse_speed, alias = "replay-speed")]
    pub speed: f64,

    /// Bytes per second fed through the analyser in bytes mode
    #[arg(long, default_value_t = 4096, value_parser = clap::value_parser!(u64).range(1..=1_000_000_000))]
    pub rate: u64,

    /// Loop file inputs forever
    #[arg(long = "loop")]
    pub loop_input: bool,

    /// Observations per lane used for the structure statistics
    #[arg(long, default_value_t = 256, value_parser = clap::value_parser!(u32).range(32..=4096))]
    pub window: u32,

    /// Disable colour (the NO_COLOR environment variable also works)
    #[arg(long)]
    pub no_color: bool,

    /// Number of alert lines kept
    #[arg(long, default_value_t = 50)]
    pub max_alerts: usize,

    /// Record packet events to a JSONL file (replay later with --mode replay)
    #[arg(long)]
    pub record: Option<PathBuf>,

    /// Seed for the demo traffic (same seed = same landscape)
    #[arg(long, default_value_t = 0xD3A1_0A5)]
    pub seed: u64,

    /// No TUI: print frames as plain text (for pipes, logs, CI, screenshots)
    #[arg(long)]
    pub headless: bool,

    /// Headless: stop after this many ticks (default: at end of input)
    #[arg(long)]
    pub frames: Option<u64>,

    /// Headless: also print every Nth frame (0 = only the final frame)
    #[arg(long, default_value_t = 0)]
    pub print_every: u64,

    /// Headless: don't wait in real time (demo and file inputs only)
    #[arg(long)]
    pub fast: bool,

    /// Headless frame width in columns
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u16).range(20..=1000))]
    pub width: u16,

    /// Headless frame height in rows
    #[arg(long, default_value_t = 36, value_parser = clap::value_parser!(u16).range(8..=500))]
    pub height: u16,

    /// Live: interface name, description or index from --mode list-interfaces
    #[arg(long)]
    pub interface: Option<String>,

    /// Live: BPF capture filter, e.g. "tcp or udp"
    #[arg(long)]
    pub filter: Option<String>,

    /// Live: promiscuous mode
    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    pub promisc: bool,

    /// Live: bytes captured per packet
    #[arg(long, default_value_t = 65_535)]
    pub snaplen: u32,

    /// Live: kernel capture buffer in bytes
    #[arg(long, default_value_t = 4 * 1024 * 1024)]
    pub buffer_size: u32,

    /// Live: read timeout in milliseconds
    #[arg(long, default_value_t = 50)]
    pub pcap_timeout_ms: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn defaults_and_aliases() {
        let cli = Cli::parse_from(["mdd"]);
        assert_eq!(cli.mode, Mode::Demo);
        assert_eq!(cli.tick_ms, 125);
        assert!(cli.promisc);
        let cli = Cli::parse_from([
            "mdd",
            "--mode",
            "pcap-file",
            "-i",
            "x.pcapng",
            "--replay-speed",
            "4",
            "--loop",
        ]);
        assert_eq!(cli.mode, Mode::PcapFile);
        assert_eq!(cli.speed, 4.0);
        assert!(cli.loop_input);
        assert!(Cli::try_parse_from(["mdd", "--speed", "0"]).is_err());
        assert!(Cli::try_parse_from(["mdd", "--tick-ms", "5"]).is_err());
        let cli = Cli::parse_from(["mdd", "--promisc", "false", "--mode", "stdin-jsonl"]);
        assert!(!cli.promisc);
        assert_eq!(cli.mode, Mode::StdinJsonl);
    }
}
