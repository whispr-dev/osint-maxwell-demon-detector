use std::collections::HashSet;
use std::error::Error;
use std::io;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use pcap::{Capture, Device, Error as PcapError};

use crate::cli::Cli;
use crate::event::PacketEvent;
use crate::source::packet_decode::packet_to_event;
use crate::source::EventSource;

pub struct LivePcapSource {
    rx: Receiver<PacketEvent>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    thread_error: Arc<Mutex<Option<String>>>,
}

impl LivePcapSource {
    pub fn new(cli: &Cli) -> Result<Self, Box<dyn Error>> {
        let device = resolve_device(cli.interface.as_deref())?;
        let local_addrs: HashSet<IpAddr> = device.addresses.iter().map(|a| a.addr).collect();
        let device_name = device.name.clone();
        let filter = cli.filter.clone();
        let promisc = cli.promisc;
        let snaplen = i32::try_from(cli.snaplen)
            .map_err(|_| io::Error::other("--snaplen is too large for libpcap/Npcap"))?;
        let buffer_size = i32::try_from(cli.buffer_size)
            .map_err(|_| io::Error::other("--buffer-size is too large for libpcap/Npcap"))?;
        let timeout_ms = i32::try_from(cli.pcap_timeout_ms)
            .map_err(|_| io::Error::other("--pcap-timeout-ms is too large for libpcap/Npcap"))?;

        let (tx, rx) = mpsc::channel::<PacketEvent>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let thread_error = Arc::new(Mutex::new(None));
        let thread_error_clone = Arc::clone(&thread_error);

        let handle = thread::spawn(move || {
            let result = run_capture_thread(
                device,
                device_name,
                local_addrs,
                filter,
                promisc,
                snaplen,
                buffer_size,
                timeout_ms,
                stop_thread,
                tx,
            );

            if let Err(err) = result {
                if let Ok(mut slot) = thread_error_clone.lock() {
                    *slot = Some(err.to_string());
                }
            }
        });

        Ok(Self {
            rx,
            stop,
            handle: Some(handle),
            thread_error,
        })
    }
}

impl EventSource for LivePcapSource {
    fn poll(&mut self, budget: Duration) -> Result<Vec<PacketEvent>, Box<dyn Error>> {
        if let Some(msg) = self
            .thread_error
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
        {
            return Err(io::Error::other(msg).into());
        }

        let start = Instant::now();
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(ev) => out.push(ev),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
            if start.elapsed() >= budget {
                break;
            }
        }
        Ok(out)
    }
}

impl Drop for LivePcapSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run_capture_thread(
    device: Device,
    device_name: String,
    local_addrs: HashSet<IpAddr>,
    filter: Option<String>,
    promisc: bool,
    snaplen: i32,
    buffer_size: i32,
    timeout_ms: i32,
    stop: Arc<AtomicBool>,
    tx: mpsc::Sender<PacketEvent>,
) -> Result<(), Box<dyn Error>> {
    let mut cap = Capture::from_device(device)?
        .promisc(promisc)
        .snaplen(snaplen)
        .buffer_size(buffer_size)
        .timeout(timeout_ms.max(1))
        .open()?;

    if let Some(program) = filter.as_deref() {
        if !program.trim().is_empty() {
            cap.filter(program, true)?;
        }
    }

    let linktype = cap.get_datalink();

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        match cap.next_packet() {
            Ok(packet) => {
                if let Some(ev) = packet_to_event(linktype, packet.header, packet.data, &local_addrs) {
                    if tx.send(ev).is_err() {
                        break;
                    }
                }
            }
            Err(PcapError::TimeoutExpired) => continue,
            Err(PcapError::NoMorePackets) => break,
            Err(err) => {
                return Err(io::Error::other(format!(
                    "pcap capture error on {}: {}",
                    device_name, err
                ))
                .into())
            }
        }
    }

    Ok(())
}

fn resolve_device(interface: Option<&str>) -> Result<Device, Box<dyn Error>> {
    let devices = Device::list()?;
    if let Some(token) = interface {
        if let Ok(idx) = token.parse::<usize>() {
            return devices
                .get(idx)
                .cloned()
                .ok_or_else(|| io::Error::other(format!("no interface at index {}", idx)).into());
        }

        return devices
            .into_iter()
            .find(|d| d.name == token)
            .ok_or_else(|| io::Error::other(format!("interface not found: {}", token)).into());
    }

    Device::lookup()?
        .ok_or_else(|| io::Error::other("pcap could not find a default capture device").into())
}

pub fn print_interfaces() -> Result<(), Box<dyn Error>> {
    let devices = Device::list()?;
    if devices.is_empty() {
        println!("No pcap interfaces were found.");
        return Ok(());
    }

    for (idx, dev) in devices.iter().enumerate() {
        let desc = dev.desc.as_deref().unwrap_or("(no description)");
        println!("[{}] {}", idx, dev.name);
        println!("    desc: {}", desc);
        if dev.addresses.is_empty() {
            println!("    addrs: (none)");
        } else {
            let addrs = dev
                .addresses
                .iter()
                .map(|a| a.addr.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            println!("    addrs: {}", addrs);
        }
        println!();
    }

    Ok(())
}
