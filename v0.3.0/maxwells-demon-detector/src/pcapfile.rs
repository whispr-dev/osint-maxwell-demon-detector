//! Minimal streaming reader for `.pcap` and `.pcapng` capture files.
//!
//! No libpcap needed, so Windows users can visualise captures from Wireshark
//! (pcapng), tcpdump (pcap) or `pktmon etl2pcap` without the Npcap SDK.
//!
//! Supported: classic pcap (microsecond and nanosecond magic, either byte order);
//! pcapng sections in either byte order with Interface Description, Enhanced
//! Packet, Simple Packet and (obsolete) Packet blocks, honouring `if_tsresol`.
//! A truncated final record (a capture that was killed mid-write) ends the stream
//! cleanly instead of erroring.

use std::fmt;
use std::io::{self, Read};

/// Hard cap on a single record/block, to reject corrupt lengths early.
const MAX_RECORD: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    /// Timestamp in microseconds since the Unix epoch (or whatever the capture used).
    pub ts_us: u64,
    pub linktype: u32,
    /// Original on-the-wire length.
    pub orig_len: usize,
    /// Captured bytes (may be shorter than `orig_len`).
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub enum PcapError {
    Io(io::Error),
    NotPcap,
    Corrupt(String),
}

impl fmt::Display for PcapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PcapError::Io(e) => write!(f, "I/O error reading capture: {e}"),
            PcapError::NotPcap => {
                write!(f, "not a pcap or pcapng file (unrecognised magic number)")
            }
            PcapError::Corrupt(why) => write!(f, "corrupt capture file: {why}"),
        }
    }
}

impl std::error::Error for PcapError {}

impl From<io::Error> for PcapError {
    fn from(e: io::Error) -> Self {
        PcapError::Io(e)
    }
}

#[derive(Debug, Clone, Copy)]
struct Iface {
    linktype: u32,
    snaplen: u32,
    /// Timestamp units per second (1e6 by default).
    units_per_sec: u64,
}

#[derive(Debug)]
enum Format {
    Classic {
        le: bool,
        nanos: bool,
        linktype: u32,
    },
    Ng {
        le: bool,
        ifaces: Vec<Iface>,
        last_ts_us: u64,
    },
}

pub struct PcapReader<R: Read> {
    inner: R,
    format: Format,
}

enum Fill {
    Full,
    /// Stream ended before the buffer was full (clean end or truncated tail).
    Eof,
}

fn fill<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<Fill> {
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..]) {
            Ok(0) => return Ok(Fill::Eof),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Fill::Full)
}

fn u16_at(b: &[u8], off: usize, le: bool) -> u16 {
    let x = [b[off], b[off + 1]];
    if le {
        u16::from_le_bytes(x)
    } else {
        u16::from_be_bytes(x)
    }
}

fn u32_at(b: &[u8], off: usize, le: bool) -> u32 {
    let x = [b[off], b[off + 1], b[off + 2], b[off + 3]];
    if le {
        u32::from_le_bytes(x)
    } else {
        u32::from_be_bytes(x)
    }
}

const SHB: u32 = 0x0A0D_0D0A;

impl<R: Read> PcapReader<R> {
    pub fn new(mut inner: R) -> Result<Self, PcapError> {
        let mut magic = [0u8; 4];
        if let Fill::Eof = fill(&mut inner, &mut magic)? {
            return Err(PcapError::NotPcap);
        }
        let classic = match magic {
            [0xD4, 0xC3, 0xB2, 0xA1] => Some((true, false)),
            [0xA1, 0xB2, 0xC3, 0xD4] => Some((false, false)),
            [0x4D, 0x3C, 0xB2, 0xA1] => Some((true, true)),
            [0xA1, 0xB2, 0x3C, 0x4D] => Some((false, true)),
            _ => None,
        };
        if let Some((le, nanos)) = classic {
            let mut rest = [0u8; 20];
            if let Fill::Eof = fill(&mut inner, &mut rest)? {
                return Err(PcapError::Corrupt("file header is truncated".into()));
            }
            // Lower 16 bits are the link type; the upper bits carry FCS info.
            let linktype = u32_at(&rest, 16, le) & 0xFFFF;
            return Ok(Self {
                inner,
                format: Format::Classic {
                    le,
                    nanos,
                    linktype,
                },
            });
        }
        if magic == [0x0A, 0x0D, 0x0D, 0x0A] {
            let mut reader = Self {
                inner,
                format: Format::Ng {
                    le: true,
                    ifaces: Vec::new(),
                    last_ts_us: 0,
                },
            };
            reader.read_shb_rest()?;
            return Ok(reader);
        }
        Err(PcapError::NotPcap)
    }

    /// Called right after the 4-byte SHB block type has been consumed.
    fn read_shb_rest(&mut self) -> Result<(), PcapError> {
        let mut head = [0u8; 8];
        if let Fill::Eof = fill(&mut self.inner, &mut head)? {
            return Err(PcapError::Corrupt("section header is truncated".into()));
        }
        let le = match head[4..8] {
            [0x4D, 0x3C, 0x2B, 0x1A] => true,
            [0x1A, 0x2B, 0x3C, 0x4D] => false,
            _ => return Err(PcapError::Corrupt("bad pcapng byte-order magic".into())),
        };
        let total = u32_at(&head, 0, le) as usize;
        if total < 28 || !total.is_multiple_of(4) || total > MAX_RECORD {
            return Err(PcapError::Corrupt(format!(
                "bad section header length {total}"
            )));
        }
        let mut rest = vec![0u8; total - 12];
        if let Fill::Eof = fill(&mut self.inner, &mut rest)? {
            return Err(PcapError::Corrupt("section header is truncated".into()));
        }
        self.format = Format::Ng {
            le,
            ifaces: Vec::new(),
            last_ts_us: 0,
        };
        Ok(())
    }

    /// Next packet, or `None` at end of file.
    pub fn next_packet(&mut self) -> Result<Option<Packet>, PcapError> {
        match self.format {
            Format::Classic {
                le,
                nanos,
                linktype,
            } => self.next_classic(le, nanos, linktype),
            Format::Ng { .. } => self.next_ng(),
        }
    }

    fn next_classic(
        &mut self,
        le: bool,
        nanos: bool,
        linktype: u32,
    ) -> Result<Option<Packet>, PcapError> {
        let mut hdr = [0u8; 16];
        if let Fill::Eof = fill(&mut self.inner, &mut hdr)? {
            return Ok(None);
        }
        let sec = u64::from(u32_at(&hdr, 0, le));
        let frac = u64::from(u32_at(&hdr, 4, le));
        let incl = u32_at(&hdr, 8, le) as usize;
        let orig = u32_at(&hdr, 12, le) as usize;
        if incl > MAX_RECORD {
            return Err(PcapError::Corrupt(format!(
                "record length {incl} is implausible"
            )));
        }
        let mut data = vec![0u8; incl];
        if let Fill::Eof = fill(&mut self.inner, &mut data)? {
            return Ok(None);
        }
        let micros = if nanos { frac / 1_000 } else { frac };
        Ok(Some(Packet {
            ts_us: sec.saturating_mul(1_000_000).saturating_add(micros),
            linktype,
            orig_len: orig.max(incl),
            data,
        }))
    }

    fn next_ng(&mut self) -> Result<Option<Packet>, PcapError> {
        loop {
            let mut head = [0u8; 8];
            if let Fill::Eof = fill(&mut self.inner, &mut head)? {
                return Ok(None);
            }
            // The SHB type is a byte palindrome, so it can be read before we know the order.
            if u32::from_le_bytes([head[0], head[1], head[2], head[3]]) == SHB {
                // Re-feed the length+magic we already consumed.
                let mut rest8 = [0u8; 4];
                if let Fill::Eof = fill(&mut self.inner, &mut rest8)? {
                    return Ok(None);
                }
                let le = match rest8 {
                    [0x4D, 0x3C, 0x2B, 0x1A] => true,
                    [0x1A, 0x2B, 0x3C, 0x4D] => false,
                    _ => return Err(PcapError::Corrupt("bad pcapng byte-order magic".into())),
                };
                let total = u32_at(&head, 4, le) as usize;
                if total < 28 || !total.is_multiple_of(4) || total > MAX_RECORD {
                    return Err(PcapError::Corrupt(format!(
                        "bad section header length {total}"
                    )));
                }
                let mut rest = vec![0u8; total - 12];
                if let Fill::Eof = fill(&mut self.inner, &mut rest)? {
                    return Ok(None);
                }
                self.format = Format::Ng {
                    le,
                    ifaces: Vec::new(),
                    last_ts_us: 0,
                };
                continue;
            }

            let le = match self.format {
                Format::Ng { le, .. } => le,
                Format::Classic { .. } => return Ok(None),
            };
            let btype = u32_at(&head, 0, le);
            let total = u32_at(&head, 4, le) as usize;
            if total < 12 || !total.is_multiple_of(4) || total > MAX_RECORD {
                return Err(PcapError::Corrupt(format!("bad block length {total}")));
            }
            // Body plus the trailing copy of the length.
            let mut body = vec![0u8; total - 8];
            if let Fill::Eof = fill(&mut self.inner, &mut body)? {
                return Ok(None);
            }
            let body_len = body.len() - 4;
            let body = &body[..body_len];

            let Format::Ng {
                ifaces, last_ts_us, ..
            } = &mut self.format
            else {
                return Ok(None);
            };
            match btype {
                // Interface Description Block.
                1 => {
                    if body.len() < 8 {
                        return Err(PcapError::Corrupt("interface block too short".into()));
                    }
                    let linktype = u32::from(u16_at(body, 0, le));
                    let snaplen = u32_at(body, 4, le);
                    let mut units_per_sec = 1_000_000u64;
                    let mut off = 8;
                    while off + 4 <= body.len() {
                        let code = u16_at(body, off, le);
                        let len = usize::from(u16_at(body, off + 2, le));
                        let val_start = off + 4;
                        if code == 0 || val_start + len > body.len() {
                            break;
                        }
                        if code == 9 && len >= 1 {
                            let v = body[val_start];
                            units_per_sec = if v & 0x80 == 0 {
                                10u64.checked_pow(u32::from(v)).unwrap_or(1_000_000)
                            } else {
                                1u64.checked_shl(u32::from(v & 0x7F)).unwrap_or(1_000_000)
                            };
                            if units_per_sec == 0 {
                                units_per_sec = 1_000_000;
                            }
                        }
                        off = val_start + len.div_ceil(4) * 4;
                    }
                    ifaces.push(Iface {
                        linktype,
                        snaplen,
                        units_per_sec,
                    });
                }
                // Enhanced Packet Block (6) and obsolete Packet Block (2).
                2 | 6 => {
                    if body.len() < 20 {
                        return Err(PcapError::Corrupt("packet block too short".into()));
                    }
                    let if_id = if btype == 6 {
                        u32_at(body, 0, le) as usize
                    } else {
                        usize::from(u16_at(body, 0, le))
                    };
                    let ts =
                        (u64::from(u32_at(body, 4, le)) << 32) | u64::from(u32_at(body, 8, le));
                    let caplen = u32_at(body, 12, le) as usize;
                    let orig = u32_at(body, 16, le) as usize;
                    if 20 + caplen > body.len() {
                        return Err(PcapError::Corrupt("packet data overruns its block".into()));
                    }
                    let iface = ifaces.get(if_id).copied().unwrap_or(Iface {
                        linktype: 1,
                        snaplen: 0,
                        units_per_sec: 1_000_000,
                    });
                    let ts_us =
                        u64::try_from(u128::from(ts) * 1_000_000 / u128::from(iface.units_per_sec))
                            .unwrap_or(u64::MAX);
                    *last_ts_us = ts_us;
                    return Ok(Some(Packet {
                        ts_us,
                        linktype: iface.linktype,
                        orig_len: orig.max(caplen),
                        data: body[20..20 + caplen].to_vec(),
                    }));
                }
                // Simple Packet Block: no timestamp, interface 0.
                3 => {
                    if body.len() < 4 {
                        return Err(PcapError::Corrupt("simple packet block too short".into()));
                    }
                    let orig = u32_at(body, 0, le) as usize;
                    let iface = ifaces.first().copied().unwrap_or(Iface {
                        linktype: 1,
                        snaplen: 0,
                        units_per_sec: 1_000_000,
                    });
                    let mut caplen = orig.min(body.len() - 4);
                    if iface.snaplen > 0 {
                        caplen = caplen.min(iface.snaplen as usize);
                    }
                    return Ok(Some(Packet {
                        ts_us: *last_ts_us,
                        linktype: iface.linktype,
                        orig_len: orig.max(caplen),
                        data: body[4..4 + caplen].to_vec(),
                    }));
                }
                // Name resolution, statistics, custom blocks, ...: skip.
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::tests::eth_ipv4_tcp;

    fn put_u16(v: &mut Vec<u8>, x: u16, le: bool) {
        v.extend_from_slice(&if le { x.to_le_bytes() } else { x.to_be_bytes() });
    }
    fn put_u32(v: &mut Vec<u8>, x: u32, le: bool) {
        v.extend_from_slice(&if le { x.to_le_bytes() } else { x.to_be_bytes() });
    }

    fn classic(le: bool, nanos: bool, pkts: &[(u32, u32, Vec<u8>)]) -> Vec<u8> {
        let mut v = Vec::new();
        put_u32(&mut v, if nanos { 0xA1B2_3C4D } else { 0xA1B2_C3D4 }, le);
        put_u16(&mut v, 2, le);
        put_u16(&mut v, 4, le);
        put_u32(&mut v, 0, le);
        put_u32(&mut v, 0, le);
        put_u32(&mut v, 65535, le);
        put_u32(&mut v, 1, le);
        for (sec, frac, data) in pkts {
            put_u32(&mut v, *sec, le);
            put_u32(&mut v, *frac, le);
            put_u32(&mut v, data.len() as u32, le);
            put_u32(&mut v, data.len() as u32, le);
            v.extend_from_slice(data);
        }
        v
    }

    fn read_all(bytes: &[u8]) -> Vec<Packet> {
        let mut r = PcapReader::new(bytes).expect("header");
        let mut out = Vec::new();
        while let Some(p) = r.next_packet().expect("packet") {
            out.push(p);
        }
        out
    }

    #[test]
    fn classic_all_variants() {
        let frame = eth_ipv4_tcp([10, 0, 0, 1], [1, 1, 1, 1], 5000, 443, b"hi");
        for le in [true, false] {
            for nanos in [false, true] {
                let frac = if nanos { 250_000_000 } else { 250_000 };
                let file = classic(
                    le,
                    nanos,
                    &[(10, frac, frame.clone()), (11, 0, frame.clone())],
                );
                let pkts = read_all(&file);
                assert_eq!(pkts.len(), 2, "le={le} nanos={nanos}");
                assert_eq!(pkts[0].ts_us, 10_250_000);
                assert_eq!(pkts[1].ts_us, 11_000_000);
                assert_eq!(pkts[0].linktype, 1);
                assert_eq!(pkts[0].data, frame);
            }
        }
    }

    #[test]
    fn classic_truncated_tail_ends_cleanly() {
        let frame = eth_ipv4_tcp([10, 0, 0, 1], [1, 1, 1, 1], 5000, 443, b"");
        let mut file = classic(true, false, &[(1, 0, frame.clone()), (2, 0, frame)]);
        file.truncate(file.len() - 7);
        assert_eq!(read_all(&file).len(), 1);
    }

    fn ng_block(v: &mut Vec<u8>, btype: u32, body: &[u8], le: bool) {
        let padded = body.len().div_ceil(4) * 4;
        let total = (12 + padded) as u32;
        put_u32(v, btype, le);
        put_u32(v, total, le);
        v.extend_from_slice(body);
        v.resize(v.len() + padded - body.len(), 0);
        put_u32(v, total, le);
    }

    fn pcapng(le: bool) -> (Vec<u8>, Vec<u8>) {
        let frame = eth_ipv4_tcp([10, 0, 0, 1], [8, 8, 8, 8], 4000, 53, b"q");
        let mut v = Vec::new();
        // Section Header Block.
        let mut shb = Vec::new();
        put_u32(&mut shb, 0x1A2B_3C4D, le);
        put_u16(&mut shb, 1, le);
        put_u16(&mut shb, 0, le);
        shb.extend_from_slice(&[0xFF; 8]); // section length: unknown
        ng_block(&mut v, SHB, &shb, le);
        // Interface Description Block with if_tsresol = 9 (nanoseconds).
        let mut idb = Vec::new();
        put_u16(&mut idb, 1, le);
        put_u16(&mut idb, 0, le);
        put_u32(&mut idb, 262_144, le);
        put_u16(&mut idb, 9, le);
        put_u16(&mut idb, 1, le);
        idb.extend_from_slice(&[9, 0, 0, 0]);
        put_u16(&mut idb, 0, le);
        put_u16(&mut idb, 0, le);
        ng_block(&mut v, 1, &idb, le);
        // A statistics block to skip.
        ng_block(&mut v, 5, &[0u8; 12], le);
        // Enhanced Packet Block: ts = 3.5 s in ns.
        let ts: u64 = 3_500_000_000;
        let mut epb = Vec::new();
        put_u32(&mut epb, 0, le);
        put_u32(&mut epb, (ts >> 32) as u32, le);
        put_u32(&mut epb, ts as u32, le);
        put_u32(&mut epb, frame.len() as u32, le);
        put_u32(&mut epb, frame.len() as u32, le);
        epb.extend_from_slice(&frame);
        ng_block(&mut v, 6, &epb, le);
        // Simple Packet Block.
        let mut spb = Vec::new();
        put_u32(&mut spb, frame.len() as u32, le);
        spb.extend_from_slice(&frame);
        ng_block(&mut v, 3, &spb, le);
        (v, frame)
    }

    #[test]
    fn pcapng_both_byte_orders() {
        for le in [true, false] {
            let (file, frame) = pcapng(le);
            let pkts = read_all(&file);
            assert_eq!(pkts.len(), 2, "le={le}");
            assert_eq!(pkts[0].ts_us, 3_500_000);
            assert_eq!(pkts[0].data, frame);
            assert_eq!(pkts[1].ts_us, 3_500_000, "SPB inherits the last timestamp");
            assert_eq!(pkts[1].data, frame);
        }
    }

    #[test]
    fn rejects_non_pcap_and_survives_corruption() {
        assert!(matches!(
            PcapReader::new(&b"hello world"[..]),
            Err(PcapError::NotPcap)
        ));
        assert!(matches!(PcapReader::new(&b""[..]), Err(PcapError::NotPcap)));
        let (mut file, _) = pcapng(true);
        // Corrupt the IDB length field: must error, not panic.
        let idb_len_at = 28 + 4;
        file[idb_len_at] = 0x03;
        let mut r = PcapReader::new(&file[..]).expect("SHB is intact");
        let mut n = 0;
        while let Ok(Some(_)) = r.next_packet() {
            n += 1;
            assert!(n < 10);
        }
    }
}
