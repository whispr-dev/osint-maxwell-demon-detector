use std::error::Error;
use std::io::{self, BufRead};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use crate::event::PacketEvent;
use crate::source::EventSource;

pub struct StdinJsonlSource {
    rx: Receiver<PacketEvent>,
}

impl StdinJsonlSource {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let (tx, rx) = mpsc::channel::<PacketEvent>();
        thread::spawn(move || {
            let stdin = io::stdin();
            let reader = io::BufReader::new(stdin.lock());
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => continue,
                };
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<PacketEvent>(trimmed) {
                    Ok(ev) => {
                        let _ = tx.send(ev);
                    }
                    Err(_) => continue,
                }
            }
        });
        Ok(Self { rx })
    }
}

impl EventSource for StdinJsonlSource {
    fn poll(&mut self, budget: Duration) -> Result<Vec<PacketEvent>, Box<dyn Error>> {
        let deadline = std::time::Instant::now() + budget;
        let mut out = Vec::new();
        while std::time::Instant::now() < deadline {
            match self.rx.recv_timeout(Duration::from_millis(2)) {
                Ok(ev) => out.push(ev),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        while let Ok(ev) = self.rx.try_recv() {
            out.push(ev);
        }
        Ok(out)
    }
}
