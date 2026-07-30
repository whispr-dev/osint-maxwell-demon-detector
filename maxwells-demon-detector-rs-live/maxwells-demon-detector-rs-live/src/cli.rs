use clap::{ArgAction, Parser, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum Mode {
    Demo,
    Replay,
    StdinJsonl,
    ListInterfaces,
    LivePcap,
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

    #[arg(long)]
    pub interface: Option<String>,

    #[arg(long)]
    pub filter: Option<String>,

    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    pub promisc: bool,

    #[arg(long, default_value_t = 65_535)]
    pub snaplen: u32,

    #[arg(long, default_value_t = 1_048_576)]
    pub buffer_size: u32,

    #[arg(long, default_value_t = 50)]
    pub pcap_timeout_ms: u32,
}
