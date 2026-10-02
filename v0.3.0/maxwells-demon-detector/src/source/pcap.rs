//! Replay a `.pcap` / `.pcapng` capture file (no libpcap needed).

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::decode::Decoder;
use crate::pcapfile::{Packet, PcapReader};
use crate::source::{file_name, Pacer, Sink, Source, SourceStatus};
use crate::Result;

pub struct PcapFileSource {
    path: PathBuf,
    reader: Option<PcapReader<BufReader<File>>>,
    decoder: Decoder,
    looping: bool,
    next: Option<Packet>,
    pacer: Pacer,
    packets_this_pass: u64,
    ended: bool,
}

fn open(path: &Path) -> Result<PcapReader<BufReader<File>>> {
    let file = File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    PcapReader::new(BufReader::with_capacity(256 * 1024, file))
        .map_err(|e| format!("{}: {e}", path.display()).into())
}

impl PcapFileSource {
    pub fn open(path: &Path, speed: f64, looping: bool, keep_addrs: bool) -> Result<Self> {
        let reader = open(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            reader: Some(reader),
            decoder: Decoder {
                local: Vec::new(),
                keep_addrs,
            },
            looping,
            next: None,
            pacer: Pacer::new(speed),
            packets_this_pass: 0,
            ended: false,
        })
    }
}

impl Source for PcapFileSource {
    fn poll(&mut self, now: Duration, sink: &mut Sink) -> Result<()> {
        if self.ended {
            return Ok(());
        }
        let now_us = u64::try_from(now.as_micros()).unwrap_or(u64::MAX);
        let mut emitted = 0u32;
        loop {
            if self.next.is_none() {
                self.next = match self.reader.as_mut() {
                    Some(r) => r
                        .next_packet()
                        .map_err(|e| format!("{}: {e}", self.path.display()))?,
                    None => None,
                };
            }
            let Some(pkt) = self.next.as_ref() else {
                if self.looping && self.packets_this_pass > 0 {
                    self.reader = Some(open(&self.path)?);
                    self.packets_this_pass = 0;
                    self.pacer.new_pass(now_us);
                    continue;
                }
                self.ended = true;
                self.reader = None;
                break;
            };
            let (due, stream_ts) = self.pacer.schedule(pkt.ts_us);
            if due > now_us {
                break;
            }
            let raw_ts = pkt.ts_us;
            if let Some(pkt) = self.next.take() {
                let ev = self.decoder.decode(pkt.linktype, &pkt.data, pkt.orig_len);
                sink.packet(&ev, stream_ts);
            }
            self.pacer.commit(raw_ts, stream_ts);
            self.packets_this_pass += 1;
            emitted += 1;
            if emitted >= 200_000 {
                break;
            }
        }
        Ok(())
    }

    fn status(&self) -> SourceStatus {
        let name = file_name(&self.path);
        let label = if self.ended {
            format!("pcap {name}: end of file")
        } else if self.pacer.passes > 1 {
            format!("pcap {name} (loop {})", self.pacer.passes)
        } else {
            format!("pcap {name}")
        };
        SourceStatus {
            label,
            ended: self.ended,
            ..SourceStatus::default()
        }
    }
}
