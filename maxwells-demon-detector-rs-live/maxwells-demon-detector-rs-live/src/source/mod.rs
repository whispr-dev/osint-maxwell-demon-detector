use std::error::Error;
use std::io;
use std::time::Duration;

use crate::cli::{Cli, Mode};
use crate::event::PacketEvent;

mod demo;
mod live_pcap;
mod packet_decode;
mod replay;
mod stdin_jsonl;

pub trait EventSource {
    fn poll(&mut self, budget: Duration) -> Result<Vec<PacketEvent>, Box<dyn Error>>;
}

pub fn build_source(cli: &Cli) -> Result<Box<dyn EventSource>, Box<dyn Error>> {
    match cli.mode {
        Mode::Demo => Ok(Box::new(demo::DemoSource::new(cli.tick_ms))),
        Mode::Replay => {
            let path = cli
                .input
                .clone()
                .ok_or_else(|| io::Error::other("--input is required in replay mode"))?;
            Ok(Box::new(replay::ReplaySource::from_path(&path, cli.replay_speed)?))
        }
        Mode::StdinJsonl => Ok(Box::new(stdin_jsonl::StdinJsonlSource::new()?)),
        Mode::LivePcap => Ok(Box::new(live_pcap::LivePcapSource::new(cli)?)),
        Mode::ListInterfaces => Err(io::Error::other("list-interfaces mode does not stream events").into()),
    }
}

pub fn print_interfaces() -> Result<(), Box<dyn Error>> {
    live_pcap::print_interfaces()
}
