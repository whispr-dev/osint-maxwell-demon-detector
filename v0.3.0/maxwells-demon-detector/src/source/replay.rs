//! Replay packet events from a CSV or JSONL file, paced by their timestamps.
//!
//! Streams the file (constant memory, any size). Rows without a timestamp are
//! spaced 10 ms apart. Malformed rows are counted and skipped, never fatal.

use std::fs::File;
use std::io::{BufRead, BufReader, Lines};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::event::PacketEvent;
use crate::source::{file_name, Pacer, Sink, Source, SourceStatus};
use crate::Result;

const MISSING_TS_STEP_US: u64 = 10_000;

enum Reader {
    Csv(csv::DeserializeRecordsIntoIter<File, PacketEvent>),
    Jsonl(Lines<BufReader<File>>),
}

pub struct ReplaySource {
    path: PathBuf,
    jsonl: bool,
    looping: bool,
    reader: Option<Reader>,
    next: Option<(u64, PacketEvent)>,
    last_read_ts: Option<u64>,
    pacer: Pacer,
    events_this_pass: u64,
    ended: bool,
    bad: u64,
}

fn open_reader(path: &Path, jsonl: bool) -> Result<Reader> {
    let file = File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    Ok(if jsonl {
        Reader::Jsonl(BufReader::new(file).lines())
    } else {
        let rdr = csv::ReaderBuilder::new()
            .trim(csv::Trim::All)
            .flexible(true)
            .from_reader(file);
        Reader::Csv(rdr.into_deserialize())
    })
}

impl ReplaySource {
    pub fn open(path: &Path, speed: f64, looping: bool) -> Result<Self> {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let jsonl = matches!(ext.as_str(), "jsonl" | "ndjson" | "json");
        let reader = open_reader(path, jsonl)?;
        Ok(Self {
            path: path.to_path_buf(),
            jsonl,
            looping,
            reader: Some(reader),
            next: None,
            last_read_ts: None,
            pacer: Pacer::new(speed),
            events_this_pass: 0,
            ended: false,
            bad: 0,
        })
    }

    /// Read the next valid event and resolve its timestamp. `Ok(None)` at EOF.
    fn read_next(&mut self) -> Result<Option<(u64, PacketEvent)>> {
        let Some(reader) = self.reader.as_mut() else {
            return Ok(None);
        };
        loop {
            let ev = match reader {
                Reader::Csv(it) => match it.next() {
                    None => return Ok(None),
                    Some(Ok(ev)) => ev,
                    Some(Err(e)) => {
                        if e.is_io_error() {
                            return Err(format!("reading {}: {e}", self.path.display()).into());
                        }
                        self.bad += 1;
                        continue;
                    }
                },
                Reader::Jsonl(lines) => match lines.next() {
                    None => return Ok(None),
                    Some(Err(e)) => {
                        return Err(format!("reading {}: {e}", self.path.display()).into())
                    }
                    Some(Ok(line)) => {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') {
                            continue;
                        }
                        match serde_json::from_str::<PacketEvent>(line) {
                            Ok(ev) => ev,
                            Err(_) => {
                                self.bad += 1;
                                continue;
                            }
                        }
                    }
                },
            };
            let ts = match (ev.ts_us(), self.last_read_ts) {
                (Some(t), _) => t,
                (None, Some(prev)) => prev + MISSING_TS_STEP_US,
                (None, None) => 0,
            };
            self.last_read_ts = Some(ts);
            return Ok(Some((ts, ev)));
        }
    }
}

impl Source for ReplaySource {
    fn poll(&mut self, now: Duration, sink: &mut Sink) -> Result<()> {
        if self.ended {
            return Ok(());
        }
        let now_us = u64::try_from(now.as_micros()).unwrap_or(u64::MAX);
        let mut emitted = 0u32;
        loop {
            if self.next.is_none() {
                self.next = self.read_next()?;
            }
            let Some((raw_ts, _)) = self.next.as_ref() else {
                // End of this pass.
                if self.looping && self.events_this_pass > 0 {
                    self.reader = Some(open_reader(&self.path, self.jsonl)?);
                    self.last_read_ts = None;
                    self.events_this_pass = 0;
                    self.pacer.new_pass(now_us);
                    continue;
                }
                self.ended = true;
                self.reader = None;
                break;
            };
            let raw_ts = *raw_ts;
            let (due, stream_ts) = self.pacer.schedule(raw_ts);
            if due > now_us {
                break;
            }
            if let Some((_, ev)) = self.next.take() {
                sink.packet(&ev, stream_ts);
            }
            self.pacer.commit(raw_ts, stream_ts);
            self.events_this_pass += 1;
            emitted += 1;
            // Keep a single poll bounded even at absurd speeds.
            if emitted >= 200_000 {
                break;
            }
        }
        Ok(())
    }

    fn status(&self) -> SourceStatus {
        let name = file_name(&self.path);
        let label = if self.ended {
            format!("replay {name}: end of file")
        } else if self.pacer.passes > 1 {
            format!("replay {name} (loop {})", self.pacer.passes)
        } else {
            format!("replay {name}")
        };
        SourceStatus {
            label,
            ended: self.ended,
            dropped: 0,
            bad: self.bad,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, contents: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mdd-replay-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, contents).unwrap();
        p
    }

    #[test]
    fn csv_paced_and_bad_rows_skipped() {
        let p = temp_file(
            "e.csv",
            "ts_ms,len,proto,src,dst,sport,dport,dir\n0,92,dns,a,b,5,53,out\nnot,a,row\n1000,1400,tls,a,b,5,443,in\n",
        );
        let mut src = ReplaySource::open(&p, 1.0, false).unwrap();
        let mut sink = Sink::new(None);
        src.poll(Duration::from_millis(10), &mut sink).unwrap();
        assert_eq!(sink.obs.len(), 1, "only the t=0 row is due at 10 ms");
        src.poll(Duration::from_millis(1_000), &mut sink).unwrap();
        assert_eq!(sink.obs.len(), 2);
        assert_eq!(sink.obs[1].ts_us, 1_000_000);
        src.poll(Duration::from_millis(1_100), &mut sink).unwrap();
        let st = src.status();
        assert!(st.ended);
        assert_eq!(st.bad, 1);
    }

    #[test]
    fn jsonl_loops_with_increasing_stream_time() {
        let p = temp_file(
            "e.jsonl",
            "{\"ts_ms\":0,\"len\":100,\"proto\":\"dns\"}\n# comment\n{\"len\":200,\"proto\":\"tls\"}\n{broken\n",
        );
        let mut src = ReplaySource::open(&p, 1.0, true).unwrap();
        let mut sink = Sink::new(None);
        for ms in (0..=500).step_by(50) {
            src.poll(Duration::from_millis(ms), &mut sink).unwrap();
        }
        assert!(
            sink.obs.len() >= 6,
            "several passes, got {}",
            sink.obs.len()
        );
        for w in sink.obs.windows(2) {
            assert!(
                w[1].ts_us >= w[0].ts_us,
                "stream time must not go backwards"
            );
        }
        assert!(!src.status().ended);
        assert!(src.status().bad >= 1);
    }
}
