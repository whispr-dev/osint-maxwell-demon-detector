//! Raw bytes from a file or stdin, fed through at a fixed rate.
//!
//! This is the original Maxwell's-demon question, made visual: where in this
//! stream does order appear that a fair-coin model would not produce?
//! Every byte is one observation; stream time advances 1/rate per byte.

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use crate::source::{file_name, Sink, Source, SourceStatus};
use crate::Result;

const CHUNK: usize = 64 * 1024;

enum Input {
    File {
        reader: BufReader<File>,
        size: u64,
    },
    Stdin {
        rx: Receiver<io::Result<Vec<u8>>>,
        pending: Vec<u8>,
        pos: usize,
    },
}

pub struct BytesSource {
    input: Input,
    /// File path (None when reading stdin).
    path: Option<PathBuf>,
    rate: u64,
    looping: bool,
    /// Bytes emitted in total (drives stream time).
    emitted: u64,
    /// Bytes emitted in the current pass over a file.
    pass_pos: u64,
    ended: bool,
    passes: u64,
    buf: Vec<u8>,
}

fn open_file(path: &Path) -> Result<(BufReader<File>, u64)> {
    let file = File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    Ok((BufReader::with_capacity(CHUNK, file), size))
}

impl BytesSource {
    pub fn open(path: &Path, rate: u64, looping: bool) -> Result<Self> {
        let is_stdin = path.as_os_str() == "-";
        let input = if is_stdin {
            let (tx, rx) = mpsc::sync_channel::<io::Result<Vec<u8>>>(16);
            thread::spawn(move || {
                let stdin = io::stdin();
                let mut lock = stdin.lock();
                loop {
                    let mut chunk = vec![0u8; CHUNK];
                    match lock.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            chunk.truncate(n);
                            // Blocking send: backpressure instead of unbounded memory.
                            if tx.send(Ok(chunk)).is_err() {
                                return;
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(e) => {
                            let _ = tx.send(Err(e));
                            return;
                        }
                    }
                }
            });
            Input::Stdin {
                rx,
                pending: Vec::new(),
                pos: 0,
            }
        } else {
            let (reader, size) = open_file(path)?;
            if size == 0 {
                return Err(format!("{} is empty", path.display()).into());
            }
            Input::File { reader, size }
        };
        Ok(Self {
            input,
            path: (!is_stdin).then(|| path.to_path_buf()),
            rate: rate.max(1),
            looping,
            emitted: 0,
            pass_pos: 0,
            ended: false,
            passes: 1,
            buf: vec![0u8; CHUNK],
        })
    }
}

impl Source for BytesSource {
    fn poll(&mut self, now: Duration, sink: &mut Sink) -> Result<()> {
        if self.ended {
            return Ok(());
        }
        let target =
            u64::try_from(now.as_micros() * u128::from(self.rate) / 1_000_000).unwrap_or(u64::MAX);
        // Never try to catch up more than ~2 s of data in one poll (e.g. after a stall).
        let mut due = target
            .saturating_sub(self.emitted)
            .min(self.rate.saturating_mul(2).max(1));
        while due > 0 {
            let want = usize::try_from(due).unwrap_or(usize::MAX).min(CHUNK);
            let got = match &mut self.input {
                Input::File { reader, .. } => {
                    let n = loop {
                        match reader.read(&mut self.buf[..want]) {
                            Ok(n) => break n,
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                            Err(e) => return Err(format!("reading input: {e}").into()),
                        }
                    };
                    if n == 0 {
                        if let (true, Some(path)) = (self.looping, self.path.as_ref()) {
                            let (reader, size) = open_file(path)?;
                            self.input = Input::File { reader, size };
                            self.passes += 1;
                            self.pass_pos = 0;
                            continue;
                        }
                        self.ended = true;
                        break;
                    }
                    n
                }
                Input::Stdin { rx, pending, pos } => {
                    if *pos >= pending.len() {
                        match rx.try_recv() {
                            Ok(Ok(chunk)) => {
                                *pending = chunk;
                                *pos = 0;
                            }
                            Ok(Err(e)) => return Err(format!("reading stdin: {e}").into()),
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                self.ended = true;
                                break;
                            }
                        }
                    }
                    let n = want.min(pending.len() - *pos);
                    self.buf[..n].copy_from_slice(&pending[*pos..*pos + n]);
                    *pos += n;
                    n
                }
            };
            for &b in &self.buf[..got] {
                // Stream time of byte k is k / rate seconds.
                let ts =
                    u64::try_from(u128::from(self.emitted) * 1_000_000 / u128::from(self.rate))
                        .unwrap_or(u64::MAX);
                sink.byte(b, ts);
                self.emitted += 1;
            }
            self.pass_pos += got as u64;
            due -= got as u64;
        }
        Ok(())
    }

    fn status(&self) -> SourceStatus {
        let name = self.path.as_deref().map(file_name).unwrap_or_default();
        let label = match &self.input {
            Input::File { size, .. } => {
                let pct = if *size > 0 {
                    (self.pass_pos.min(*size) * 100) / size
                } else {
                    0
                };
                let lp = if self.passes > 1 {
                    format!(" loop {}", self.passes)
                } else {
                    String::new()
                };
                if self.ended {
                    format!("bytes {name}: end of file")
                } else {
                    format!("bytes {name} {pct}%{lp} @ {} B/s", self.rate)
                }
            }
            Input::Stdin { .. } => {
                if self.ended {
                    "bytes stdin: end of input".to_string()
                } else {
                    format!("bytes stdin @ {} B/s", self.rate)
                }
            }
        };
        SourceStatus {
            label,
            ended: self.ended,
            ..SourceStatus::default()
        }
    }

    fn realtime(&self) -> bool {
        matches!(self.input, Input::Stdin { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_and_loop() {
        let dir = std::env::temp_dir().join(format!("mdd-bytes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("b.bin");
        std::fs::write(&p, vec![0xAAu8; 1000]).unwrap();

        let mut src = BytesSource::open(&p, 1000, false).unwrap();
        let mut sink = Sink::new(None);
        src.poll(Duration::from_millis(250), &mut sink).unwrap();
        assert_eq!(sink.obs.len(), 250);
        assert_eq!(sink.obs[249].ts_us, 249_000);
        src.poll(Duration::from_millis(2_000), &mut sink).unwrap();
        assert_eq!(sink.obs.len(), 1000, "file exhausted");
        assert!(src.status().ended);

        let mut src = BytesSource::open(&p, 1000, true).unwrap();
        let mut sink = Sink::new(None);
        for ms in (100..=2_500).step_by(100) {
            src.poll(Duration::from_millis(ms), &mut sink).unwrap();
        }
        assert_eq!(sink.obs.len(), 2_500);
        assert!(!src.status().ended);
        assert!(src.status().label.contains("loop"));

        assert!(BytesSource::open(&dir.join("missing.bin"), 10, false).is_err());
        std::fs::write(dir.join("empty.bin"), b"").unwrap();
        assert!(BytesSource::open(&dir.join("empty.bin"), 10, false).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
