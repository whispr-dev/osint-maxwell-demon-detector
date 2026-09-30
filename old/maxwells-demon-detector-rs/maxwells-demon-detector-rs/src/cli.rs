use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Mode {
    Demo,
    Replay,
    StdinJsonl,
}

#[derive(Debug, Clone, Parser)]
#[command(name = "maxwells-demon-detector")]
#[command(about = "Realtime ASCII network-stream mapper with rolling entropy and anomaly detection")]
pub struct Cli {
    #[arg(long, value_enum, default_value_t = Mode::Demo)]
    pub mode: Mode,

    #[arg(long)]
    pub input: Option<String>,

    #[arg(long, default_value_t = 200)]
    pub tick_ms: u64,

    #[arg(long)]
    pub width: Option<usize>,

    #[arg(long, default_value_t = 32)]
    pub replay_speed: u64,

    #[arg(long, default_value_t = 8)]
    pub max_alerts: usize,
}
