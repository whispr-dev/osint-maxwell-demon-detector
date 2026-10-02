//! Live capture via libpcap (Linux/macOS) or Npcap (Windows). Feature `live`.
//!
//! A capture thread decodes packets and hands them over a bounded channel.
//! Capture never blocks on the UI: if the analyser falls behind, packets are
//! dropped and counted (shown as `drop` in the title bar).

use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pcap::{Capture, Device};

use crate::cli::Cli;
use crate::decode::Decoder;
use crate::event::PacketEvent;
use crate::source::{Sink, Source, SourceStatus};
use crate::Result;

const QUEUE: usize = 131_072;

pub struct LiveSource {
    rx: Receiver<(PacketEvent, u64)>,
    stop: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    error: Arc<Mutex<Option<String>>>,
    handle: Option<JoinHandle<()>>,
    label: String,
    first_ts: Option<u64>,
    last_ts: u64,
    ended: bool,
}

struct Settings {
    promisc: bool,
    snaplen: i32,
    buffer_size: i32,
    timeout_ms: i32,
    filter: Option<String>,
}

impl LiveSource {
    pub fn start(cli: &Cli, keep_addrs: bool) -> Result<Self> {
        let device = resolve_device(cli.interface.as_deref())?;
        let local: Vec<IpAddr> = device.addresses.iter().map(|a| a.addr).collect();
        let label = format!(
            "live {}{}",
            device.desc.as_deref().unwrap_or(&device.name),
            cli.filter
                .as_deref()
                .map(|f| format!(" [{f}]"))
                .unwrap_or_default()
        );
        let settings = Settings {
            promisc: cli.promisc,
            snaplen: i32::try_from(cli.snaplen).map_err(|_| "--snaplen is too large")?,
            buffer_size: i32::try_from(cli.buffer_size)
                .map_err(|_| "--buffer-size is too large")?,
            timeout_ms: i32::try_from(cli.pcap_timeout_ms)
                .map_err(|_| "--pcap-timeout-ms is too large")?
                .max(1),
            filter: cli.filter.clone(),
        };

        // Open in the calling thread so permission/filter errors surface immediately.
        let mut cap = Capture::from_device(device)?
            .promisc(settings.promisc)
            .snaplen(settings.snaplen)
            .buffer_size(settings.buffer_size)
            .timeout(settings.timeout_ms)
            .immediate_mode(true)
            .open()
            .map_err(|e| format!("cannot open capture ({e}). On Linux run with sudo or grant CAP_NET_RAW; on Windows check Npcap is installed"))?;
        if let Some(f) = settings.filter.as_deref().filter(|f| !f.trim().is_empty()) {
            cap.filter(f, true)
                .map_err(|e| format!("bad capture filter {f:?}: {e}"))?;
        }
        let linktype = u32::try_from(cap.get_datalink().0).unwrap_or(u32::MAX);

        let (tx, rx) = mpsc::sync_channel::<(PacketEvent, u64)>(QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU64::new(0));
        let error = Arc::new(Mutex::new(None));
        let (stop2, dropped2, error2) =
            (Arc::clone(&stop), Arc::clone(&dropped), Arc::clone(&error));
        let decoder = Decoder { local, keep_addrs };

        let handle = thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                match cap.next_packet() {
                    Ok(p) => {
                        let secs = u64::try_from(p.header.ts.tv_sec).unwrap_or(0);
                        let micros = u64::try_from(p.header.ts.tv_usec).unwrap_or(0);
                        let ts = secs.saturating_mul(1_000_000).saturating_add(micros);
                        let wire = usize::try_from(p.header.len).unwrap_or(p.data.len());
                        let ev = decoder.decode(linktype, p.data, wire);
                        match tx.try_send((ev, ts)) {
                            Ok(()) => {}
                            Err(TrySendError::Full(_)) => {
                                dropped2.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(TrySendError::Disconnected(_)) => break,
                        }
                    }
                    Err(pcap::Error::TimeoutExpired) => {}
                    Err(pcap::Error::NoMorePackets) => break,
                    Err(e) => {
                        if let Ok(mut slot) = error2.lock() {
                            *slot = Some(e.to_string());
                        }
                        break;
                    }
                }
            }
        });

        Ok(Self {
            rx,
            stop,
            dropped,
            error,
            handle: Some(handle),
            label,
            first_ts: None,
            last_ts: 0,
            ended: false,
        })
    }
}

impl Source for LiveSource {
    fn poll(&mut self, _now: Duration, sink: &mut Sink) -> Result<()> {
        if let Some(msg) = self.error.lock().ok().and_then(|mut s| s.take()) {
            return Err(format!("capture failed: {msg}").into());
        }
        // Bounded drain per poll keeps the UI responsive under floods.
        for _ in 0..200_000 {
            match self.rx.try_recv() {
                Ok((ev, ts)) => {
                    let rel = ts.saturating_sub(*self.first_ts.get_or_insert(ts));
                    self.last_ts = self.last_ts.max(rel);
                    sink.packet(&ev, self.last_ts);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.ended = true;
                    break;
                }
            }
        }
        Ok(())
    }

    fn status(&self) -> SourceStatus {
        SourceStatus {
            label: if self.ended {
                format!("{}: capture stopped", self.label)
            } else {
                self.label.clone()
            },
            ended: self.ended,
            dropped: self.dropped.load(Ordering::Relaxed),
            bad: 0,
        }
    }

    fn realtime(&self) -> bool {
        true
    }
}

impl Drop for LiveSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn resolve_device(interface: Option<&str>) -> Result<Device> {
    let devices = Device::list()?;
    if let Some(token) = interface {
        if let Ok(idx) = token.parse::<usize>() {
            return devices.get(idx).cloned().ok_or_else(|| {
                format!("no interface at index {idx} (see --mode list-interfaces)").into()
            });
        }
        return devices
            .into_iter()
            .find(|d| d.name == token || d.desc.as_deref() == Some(token))
            .ok_or_else(|| {
                format!("interface not found: {token} (see --mode list-interfaces)").into()
            });
    }
    Device::lookup()?.ok_or_else(|| {
        "no default capture device; pass --interface (see --mode list-interfaces)".into()
    })
}

pub fn print_interfaces() -> Result<()> {
    let devices = Device::list()?;
    if devices.is_empty() {
        println!(
            "No capture interfaces found (on Linux, try sudo; on Windows, is Npcap installed?)."
        );
        return Ok(());
    }
    for (idx, dev) in devices.iter().enumerate() {
        println!("[{idx}] {}", dev.name);
        println!("    desc:  {}", dev.desc.as_deref().unwrap_or("(none)"));
        let addrs: Vec<String> = dev.addresses.iter().map(|a| a.addr.to_string()).collect();
        println!(
            "    addrs: {}",
            if addrs.is_empty() {
                "(none)".to_string()
            } else {
                addrs.join(", ")
            }
        );
    }
    println!("\nUse:  mdd --mode live-pcap --interface <index or name>");
    Ok(())
}
