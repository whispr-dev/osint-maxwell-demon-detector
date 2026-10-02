//! Events, lanes and symbolisation.
//!
//! Every source ends up producing [`Obs`] values: "at time t, lane L saw symbol S".
//! A lane is one horizontal band of the landscape. A symbol is a small token
//! (0..16) that the information-theory stats work on.
//!
//! Packets: lane = traffic class (WEB, DNS, ...), symbol = size bin x direction.
//! Bytes:   lane = byte class (ZERO, CTRL, TEXT, HIGH, FF), symbol = a nibble.
//! Lane 0 is always ALL: every observation also lands there (the overview skyline).

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

/// Number of distinct symbols. Every symbol is in `0..ALPHABET`.
pub const ALPHABET: usize = 16;

/// One packet-ish event. This is also the JSONL/CSV schema for replay, stdin and `--record`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PacketEvent {
    /// Timestamp in milliseconds (any epoch; only differences matter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts_ms: Option<u64>,
    /// Optional finer timestamp in microseconds. Wins over `ts_ms` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts_us: Option<u64>,
    /// Packet length on the wire, in bytes.
    pub len: usize,
    /// Protocol hint, e.g. "tls", "dns", "rtp". Free text; matched by substring.
    #[serde(default)]
    pub proto: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sport: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dport: Option<u16>,
    /// "in", "out" or "local" (anything else counts as unknown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

impl PacketEvent {
    /// Timestamp in microseconds, if the event carries one.
    pub fn ts_us(&self) -> Option<u64> {
        self.ts_us
            .or_else(|| self.ts_ms.map(|ms| ms.saturating_mul(1000)))
    }

    pub fn is_inbound(&self) -> bool {
        self.dir
            .as_deref()
            .is_some_and(|d| d.eq_ignore_ascii_case("in"))
    }
}

/// One symbolised observation, ready for the analyser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Obs {
    /// Stream time in microseconds (monotone per source).
    pub ts_us: u64,
    /// Lane index (>= 1; lane 0 is ALL and is fed automatically).
    pub lane: u8,
    /// Symbol for the observation's own lane.
    pub sym: u8,
    /// Symbol used for the ALL lane.
    pub sym_all: u8,
    /// Bytes this observation represents (packet length, or 1 for a raw byte).
    pub bytes: u32,
}

/// Static description of one lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneSpec {
    pub name: &'static str,
    /// Whether inter-arrival rhythm is meaningful for this lane.
    pub timing: bool,
}

/// Which lane layout a mode uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneSet {
    Packets,
    Bytes,
}

impl LaneSet {
    pub fn specs(self) -> Vec<LaneSpec> {
        match self {
            // Rhythm is a per-lane property: ALL is the sum of every lane, so its
            // "rhythm" would only echo whichever lane happens to dominate.
            LaneSet::Packets => [
                "ALL", "WEB", "DNS", "MAIL", "MEDIA", "CTRL", "BULK", "OTHER",
            ]
            .iter()
            .map(|&name| LaneSpec {
                name,
                timing: name != "ALL",
            })
            .collect(),
            LaneSet::Bytes => {
                // For raw bytes the ALL lane receives one observation per byte, so its
                // "rhythm" would be a metronome by construction. Timing is off there.
                let mut v = vec![LaneSpec {
                    name: "ALL",
                    timing: false,
                }];
                v.extend(
                    ["ZERO", "CTRL", "TEXT", "HIGH", "FF"]
                        .iter()
                        .map(|&name| LaneSpec { name, timing: true }),
                );
                v
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Packets
// ---------------------------------------------------------------------------

pub const LANE_WEB: u8 = 1;
pub const LANE_DNS: u8 = 2;
pub const LANE_MAIL: u8 = 3;
pub const LANE_MEDIA: u8 = 4;
pub const LANE_CTRL: u8 = 5;
pub const LANE_BULK: u8 = 6;
pub const LANE_OTHER: u8 = 7;

/// Classify a packet into a traffic lane (1..=7) using ports first, then the proto hint.
pub fn packet_lane(ev: &PacketEvent) -> u8 {
    let proto = ev.proto.to_ascii_lowercase();
    let has_port = |p: u16| ev.sport == Some(p) || ev.dport == Some(p);
    let any_port = |ps: &[u16]| ps.iter().any(|&p| has_port(p));
    let any_name = |names: &[&str]| names.iter().any(|n| proto.contains(n));

    if any_port(&[53, 853, 5353, 5355]) || any_name(&["dns", "mdns", "llmnr"]) {
        return LANE_DNS;
    }
    if any_port(&[25, 110, 143, 465, 587, 993, 995]) || any_name(&["smtp", "imap", "pop3", "mail"])
    {
        return LANE_MAIL;
    }
    if any_port(&[554, 1935, 3478, 3479, 5004, 5005, 5060, 5061, 5349])
        || any_name(&[
            "rtp", "rtcp", "sip", "rtsp", "rtmp", "webrtc", "stun", "turn", "video", "audio",
        ])
    {
        return LANE_MEDIA;
    }
    if any_port(&[80, 443, 8000, 8080, 8443, 8888])
        || any_name(&["http", "tls", "quic", "ws", "wss", "web"])
    {
        return LANE_WEB;
    }
    if any_port(&[
        22, 23, 67, 68, 88, 123, 137, 138, 139, 161, 162, 389, 445, 546, 547, 636, 1900, 3389,
    ]) || any_name(&[
        "ssh", "telnet", "icmp", "igmp", "ntp", "snmp", "rdp", "smb", "dhcp", "arp", "lldp",
        "eapol", "ssdp", "ldap", "kerberos", "netbios", "control", "ctrl",
    ]) {
        return LANE_CTRL;
    }
    if any_port(&[20, 21, 873, 989, 990, 2049])
        || any_name(&["ftp", "rsync", "nfs", "bulk", "file", "backup"])
        || ev.len >= 1200
    {
        return LANE_BULK;
    }
    LANE_OTHER
}

/// Log-ish size bins, with extra resolution near the MTU where most bulk data lives.
pub fn size_bin(len: usize) -> u8 {
    match len {
        0..=63 => 0,
        64..=127 => 1,
        128..=255 => 2,
        256..=511 => 3,
        512..=767 => 4,
        768..=1023 => 5,
        1024..=1399 => 6,
        _ => 7,
    }
}

/// Packet symbol: size bin (0..8) plus 8 for inbound traffic.
pub fn packet_symbol(ev: &PacketEvent) -> u8 {
    size_bin(ev.len) | if ev.is_inbound() { 8 } else { 0 }
}

pub fn packet_obs(ev: &PacketEvent, ts_us: u64) -> Obs {
    let sym = packet_symbol(ev);
    Obs {
        ts_us,
        lane: packet_lane(ev),
        sym,
        sym_all: sym,
        bytes: u32::try_from(ev.len).unwrap_or(u32::MAX),
    }
}

/// Direction of a packet relative to "us".
///
/// Uses the capture device's own addresses when known; otherwise falls back to
/// "private address = us" (right for most home and lab captures).
pub fn infer_dir(src: &IpAddr, dst: &IpAddr, local: &[IpAddr]) -> Option<&'static str> {
    let (src_local, dst_local) = if local.is_empty() {
        (is_private(src), is_private(dst))
    } else {
        (local.contains(src), local.contains(dst))
    };
    match (src_local, dst_local) {
        (true, false) => Some("out"),
        (false, true) => Some("in"),
        (true, true) => Some("local"),
        (false, false) => None,
    }
}

fn is_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            let seg0 = v6.segments()[0];
            v6.is_loopback() || (seg0 & 0xfe00) == 0xfc00 || (seg0 & 0xffc0) == 0xfe80
        }
    }
}

// ---------------------------------------------------------------------------
// Raw bytes
// ---------------------------------------------------------------------------

/// Byte class lane (1..=5): ZERO, CTRL, TEXT, HIGH, FF.
pub fn byte_lane(b: u8) -> u8 {
    match b {
        0x00 => 1,
        0x01..=0x1F | 0x7F => 2,
        0x20..=0x7E => 3,
        0x80..=0xFE => 4,
        0xFF => 5,
    }
}

/// Observation for one raw byte. The class lanes look at the low nibble (variety
/// inside a class); the ALL lane looks at the high nibble (which region of the
/// byte space we are in), so patterns across classes show up there.
pub fn byte_obs(b: u8, ts_us: u64) -> Obs {
    Obs {
        ts_us,
        lane: byte_lane(b),
        sym: b & 0x0F,
        sym_all: b >> 4,
        bytes: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(proto: &str, sport: Option<u16>, dport: Option<u16>, len: usize) -> PacketEvent {
        PacketEvent {
            len,
            proto: proto.to_string(),
            sport,
            dport,
            ..Default::default()
        }
    }

    #[test]
    fn lanes_by_port_and_name() {
        assert_eq!(
            packet_lane(&pkt("udp", Some(51000), Some(53), 80)),
            LANE_DNS
        );
        assert_eq!(
            packet_lane(&pkt("tls", Some(51000), Some(443), 1400)),
            LANE_WEB
        );
        assert_eq!(packet_lane(&pkt("quic", None, None, 1200)), LANE_WEB);
        assert_eq!(
            packet_lane(&pkt("imap", Some(993), Some(50000), 300)),
            LANE_MAIL
        );
        assert_eq!(
            packet_lane(&pkt("rtp", Some(5004), Some(5005), 200)),
            LANE_MEDIA
        );
        assert_eq!(packet_lane(&pkt("icmp", None, None, 98)), LANE_CTRL);
        assert_eq!(packet_lane(&pkt("arp", None, None, 60)), LANE_CTRL);
        assert_eq!(
            packet_lane(&pkt("ftp", Some(20), Some(40000), 1448)),
            LANE_BULK
        );
        assert_eq!(
            packet_lane(&pkt("udp", Some(40000), Some(41000), 1300)),
            LANE_BULK
        );
        assert_eq!(
            packet_lane(&pkt("tcp", Some(40000), Some(8531), 300)),
            LANE_OTHER
        );
    }

    #[test]
    fn symbols_cover_alphabet() {
        let mut ev = pkt("tls", None, None, 0);
        for (len, bin) in [
            (0, 0),
            (64, 1),
            (200, 2),
            (300, 3),
            (600, 4),
            (900, 5),
            (1300, 6),
            (1500, 7),
        ] {
            ev.len = len;
            ev.dir = Some("out".into());
            assert_eq!(packet_symbol(&ev), bin);
            ev.dir = Some("IN".into());
            assert_eq!(packet_symbol(&ev), bin + 8);
        }
        for b in 0..=255u8 {
            let o = byte_obs(b, 0);
            assert!((o.sym as usize) < ALPHABET && (o.sym_all as usize) < ALPHABET);
            assert!((1..=5).contains(&o.lane));
        }
    }

    #[test]
    fn ts_prefers_micros() {
        let ev = PacketEvent {
            ts_ms: Some(5),
            ts_us: Some(5_123),
            ..Default::default()
        };
        assert_eq!(ev.ts_us(), Some(5_123));
        let ev = PacketEvent {
            ts_ms: Some(5),
            ..Default::default()
        };
        assert_eq!(ev.ts_us(), Some(5_000));
    }

    #[test]
    fn direction_heuristic() {
        let lan: IpAddr = "192.168.1.10".parse().unwrap();
        let wan: IpAddr = "93.184.216.34".parse().unwrap();
        assert_eq!(infer_dir(&lan, &wan, &[]), Some("out"));
        assert_eq!(infer_dir(&wan, &lan, &[]), Some("in"));
        assert_eq!(infer_dir(&wan, &wan, &[]), None);
        assert_eq!(infer_dir(&wan, &lan, &[wan]), Some("out"));
    }
}
