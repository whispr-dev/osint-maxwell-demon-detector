//! Link / IP / transport decoding into [`PacketEvent`].
//!
//! Pure Rust and bounds-checked everywhere: garbage in gives an event with
//! `proto = "other"`, never a panic. Shared by the pcap-file reader and live capture.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::event::{infer_dir, PacketEvent};

/// Link types (pcap file LINKTYPE_* values, plus the platform DLT_RAW aliases
/// that live captures report).
pub mod linktype {
    pub const NULL: u32 = 0;
    pub const ETHERNET: u32 = 1;
    pub const RAW_DLT: u32 = 12;
    pub const RAW_DLT_OPENBSD: u32 = 14;
    pub const RAW: u32 = 101;
    pub const LOOP: u32 = 108;
    pub const LINUX_SLL: u32 = 113;
    pub const IPV4: u32 = 228;
    pub const IPV6: u32 = 229;
    pub const LINUX_SLL2: u32 = 276;
}

/// Decoder configuration.
#[derive(Debug, Clone, Default)]
pub struct Decoder {
    /// Addresses that count as "us" for in/out direction. Empty = private-range heuristic.
    pub local: Vec<IpAddr>,
    /// Keep src/dst address strings (only needed for `--record`; costs an allocation each).
    pub keep_addrs: bool,
}

impl Decoder {
    /// Decode one frame. `wire_len` is the original on-the-wire length.
    pub fn decode(&self, lt: u32, data: &[u8], wire_len: usize) -> PacketEvent {
        let mut ev = PacketEvent {
            len: wire_len.max(data.len()),
            proto: "other".to_string(),
            ..PacketEvent::default()
        };
        match lt {
            linktype::ETHERNET => self.ethernet(data, &mut ev),
            linktype::NULL | linktype::LOOP => self.null_loop(lt, data, &mut ev),
            linktype::RAW_DLT | linktype::RAW_DLT_OPENBSD | linktype::RAW => {
                self.ip_any(data, &mut ev)
            }
            linktype::IPV4 => self.ipv4(data, &mut ev),
            linktype::IPV6 => self.ipv6(data, &mut ev),
            linktype::LINUX_SLL => {
                if data.len() >= 16 {
                    self.ethertype(be16(data, 14), &data[16..], &mut ev);
                }
            }
            linktype::LINUX_SLL2 => {
                if data.len() >= 20 {
                    self.ethertype(be16(data, 0), &data[20..], &mut ev);
                }
            }
            _ => {}
        }
        ev
    }

    fn ethernet(&self, d: &[u8], ev: &mut PacketEvent) {
        if d.len() < 14 {
            return;
        }
        let mut et = be16(d, 12);
        let mut off = 14;
        // Up to two stacked VLAN tags (802.1Q / QinQ).
        for _ in 0..2 {
            if matches!(et, 0x8100 | 0x88A8 | 0x9100) {
                if d.len() < off + 4 {
                    return;
                }
                et = be16(d, off + 2);
                off += 4;
            } else {
                break;
            }
        }
        self.ethertype(et, &d[off..], ev);
    }

    fn ethertype(&self, et: u16, payload: &[u8], ev: &mut PacketEvent) {
        match et {
            0x0800 => self.ipv4(payload, ev),
            0x86DD => self.ipv6(payload, ev),
            0x0806 | 0x8035 => ev.proto = "arp".to_string(),
            0x88CC => ev.proto = "lldp".to_string(),
            0x888E => ev.proto = "eapol".to_string(),
            _ => {}
        }
    }

    fn null_loop(&self, lt: u32, d: &[u8], ev: &mut PacketEvent) {
        if d.len() < 4 {
            return;
        }
        let raw = [d[0], d[1], d[2], d[3]];
        let is_family = |f: u32| matches!(f, 2 | 10 | 23 | 24 | 28 | 30);
        // LOOP is network byte order; NULL is the capturing host's order (usually LE).
        let family = if lt == linktype::LOOP {
            u32::from_be_bytes(raw)
        } else {
            let le = u32::from_le_bytes(raw);
            if is_family(le) {
                le
            } else {
                u32::from_be_bytes(raw)
            }
        };
        let payload = &d[4..];
        match family {
            2 => self.ipv4(payload, ev),
            10 | 23 | 24 | 28 | 30 => self.ipv6(payload, ev),
            _ => self.ip_any(payload, ev),
        }
    }

    fn ip_any(&self, d: &[u8], ev: &mut PacketEvent) {
        match d.first().map(|b| b >> 4) {
            Some(4) => self.ipv4(d, ev),
            Some(6) => self.ipv6(d, ev),
            _ => {}
        }
    }

    fn set_addrs(&self, src: IpAddr, dst: IpAddr, ev: &mut PacketEvent) {
        ev.dir = infer_dir(&src, &dst, &self.local).map(str::to_string);
        if self.keep_addrs {
            ev.src = Some(src.to_string());
            ev.dst = Some(dst.to_string());
        }
    }

    fn ipv4(&self, d: &[u8], ev: &mut PacketEvent) {
        if d.len() < 20 || d[0] >> 4 != 4 {
            return;
        }
        let ihl = usize::from(d[0] & 0x0F) * 4;
        if ihl < 20 || d.len() < ihl {
            return;
        }
        let src = IpAddr::V4(Ipv4Addr::new(d[12], d[13], d[14], d[15]));
        let dst = IpAddr::V4(Ipv4Addr::new(d[16], d[17], d[18], d[19]));
        self.set_addrs(src, dst, ev);
        let frag_offset = be16(d, 6) & 0x1FFF;
        let total = usize::from(be16(d, 2));
        // Trim link-layer padding using the IP total length when it is sane.
        let end = if total >= ihl && total <= d.len() {
            total
        } else {
            d.len()
        };
        transport(d[9], &d[ihl..end], frag_offset == 0, ev);
    }

    fn ipv6(&self, d: &[u8], ev: &mut PacketEvent) {
        if d.len() < 40 || d[0] >> 4 != 6 {
            return;
        }
        let mut s = [0u8; 16];
        let mut t = [0u8; 16];
        s.copy_from_slice(&d[8..24]);
        t.copy_from_slice(&d[24..40]);
        self.set_addrs(
            IpAddr::V6(Ipv6Addr::from(s)),
            IpAddr::V6(Ipv6Addr::from(t)),
            ev,
        );

        let plen = usize::from(be16(d, 4));
        let end = if plen == 0 {
            d.len()
        } else {
            (40 + plen).min(d.len())
        };
        let mut next = d[6];
        let mut off = 40usize;
        let mut first_fragment = true;
        for _ in 0..10 {
            match next {
                // Hop-by-hop, routing, destination options, mobility, HIP, shim6.
                0 | 43 | 60 | 135 | 139 | 140 => {
                    if end < off + 8 {
                        ev.proto = "ipv6".to_string();
                        return;
                    }
                    next = d[off];
                    off += (usize::from(d[off + 1]) + 1) * 8;
                }
                // Fragment header.
                44 => {
                    if end < off + 8 {
                        ev.proto = "ipv6".to_string();
                        return;
                    }
                    first_fragment = (be16(d, off + 2) >> 3) == 0;
                    next = d[off];
                    off += 8;
                }
                // Authentication header (length in 4-byte units, minus 2).
                51 => {
                    if end < off + 2 {
                        ev.proto = "ah".to_string();
                        return;
                    }
                    next = d[off];
                    off += (usize::from(d[off + 1]) + 2) * 4;
                }
                _ => break,
            }
            if off > end {
                ev.proto = "ipv6".to_string();
                return;
            }
        }
        transport(next, &d[off..end], first_fragment, ev);
    }
}

fn transport(proto: u8, p: &[u8], has_header: bool, ev: &mut PacketEvent) {
    let name: &str = match proto {
        6 => {
            if has_header && p.len() >= 4 {
                ev.sport = Some(be16(p, 0));
                ev.dport = Some(be16(p, 2));
            }
            let payload = if has_header && p.len() >= 20 {
                let off = usize::from(p[12] >> 4) * 4;
                if off >= 20 && off <= p.len() {
                    &p[off..]
                } else {
                    &[][..]
                }
            } else {
                &[][..]
            };
            guess_tcp(ev.sport, ev.dport, payload)
        }
        17 => {
            if has_header && p.len() >= 4 {
                ev.sport = Some(be16(p, 0));
                ev.dport = Some(be16(p, 2));
            }
            guess_udp(ev.sport, ev.dport)
        }
        132 => {
            if has_header && p.len() >= 4 {
                ev.sport = Some(be16(p, 0));
                ev.dport = Some(be16(p, 2));
            }
            "sctp"
        }
        1 => "icmp",
        2 => "igmp",
        4 | 41 => "ip-in-ip",
        47 => "gre",
        50 => "esp",
        51 => "ah",
        58 => "icmpv6",
        89 => "ospf",
        _ => "ip",
    };
    ev.proto = name.to_string();
}

fn guess_tcp(sport: Option<u16>, dport: Option<u16>, payload: &[u8]) -> &'static str {
    let has = |p: u16| sport == Some(p) || dport == Some(p);
    if has(22) || payload.starts_with(b"SSH-") {
        return "ssh";
    }
    if has(53) || has(853) {
        return "dns";
    }
    if has(25) || has(465) || has(587) {
        return "smtp";
    }
    if has(110) || has(995) {
        return "pop3";
    }
    if has(143) || has(993) {
        return "imap";
    }
    if has(20) || has(21) {
        return "ftp";
    }
    if has(445) {
        return "smb";
    }
    if has(3389) {
        return "rdp";
    }
    if has(554) {
        return "rtsp";
    }
    if has(1935) {
        return "rtmp";
    }
    if looks_like_tls(payload) || has(443) || has(8443) {
        return "tls";
    }
    if looks_like_http(payload) || has(80) || has(8080) || has(8000) {
        return "http";
    }
    "tcp"
}

fn guess_udp(sport: Option<u16>, dport: Option<u16>) -> &'static str {
    let has = |p: u16| sport == Some(p) || dport == Some(p);
    if has(53) || has(5353) || has(5355) {
        "dns"
    } else if has(67) || has(68) || has(546) || has(547) {
        "dhcp"
    } else if has(123) {
        "ntp"
    } else if has(137) || has(138) {
        "netbios"
    } else if has(161) || has(162) {
        "snmp"
    } else if has(1900) {
        "ssdp"
    } else if has(3478) || has(3479) || has(5349) {
        "stun"
    } else if has(5004) || has(5005) {
        "rtp"
    } else if has(5060) || has(5061) {
        "sip"
    } else if has(443) {
        "quic"
    } else {
        "udp"
    }
}

fn looks_like_http(p: &[u8]) -> bool {
    const PREFIXES: [&[u8]; 9] = [
        b"GET ",
        b"POST ",
        b"PUT ",
        b"HEAD ",
        b"HTTP/",
        b"DELETE ",
        b"OPTIONS ",
        b"PATCH ",
        b"CONNECT ",
    ];
    PREFIXES.iter().any(|x| p.starts_with(x))
}

fn looks_like_tls(p: &[u8]) -> bool {
    p.len() >= 5 && matches!(p[0], 20..=23) && p[1] == 0x03 && p[2] <= 0x04
}

/// Big-endian u16 at `off`, or 0 if out of bounds.
fn be16(d: &[u8], off: usize) -> u16 {
    match d.get(off..off + 2) {
        Some(b) => u16::from_be_bytes([b[0], b[1]]),
        None => 0,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::rng::Rng;

    /// Ethernet + IPv4 + TCP frame builder (used by pcapfile tests too).
    pub(crate) fn eth_ipv4_tcp(
        src: [u8; 4],
        dst: [u8; 4],
        sport: u16,
        dport: u16,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut f = vec![0u8; 12];
        f.extend_from_slice(&[0x08, 0x00]);
        let total = (20 + 20 + payload.len()) as u16;
        let mut ip = vec![0x45, 0, 0, 0, 0, 0, 0x40, 0, 64, 6, 0, 0];
        ip[2..4].copy_from_slice(&total.to_be_bytes());
        ip.extend_from_slice(&src);
        ip.extend_from_slice(&dst);
        f.extend_from_slice(&ip);
        let mut tcp = vec![0u8; 20];
        tcp[0..2].copy_from_slice(&sport.to_be_bytes());
        tcp[2..4].copy_from_slice(&dport.to_be_bytes());
        tcp[12] = 5 << 4;
        f.extend_from_slice(&tcp);
        f.extend_from_slice(payload);
        f
    }

    #[test]
    fn ethernet_ipv4_tls() {
        let frame = eth_ipv4_tcp(
            [192, 168, 1, 5],
            [93, 184, 216, 34],
            51514,
            443,
            &[22, 3, 1, 0, 5],
        );
        let dec = Decoder {
            keep_addrs: true,
            ..Decoder::default()
        };
        let ev = dec.decode(linktype::ETHERNET, &frame, frame.len());
        assert_eq!(ev.proto, "tls");
        assert_eq!((ev.sport, ev.dport), (Some(51514), Some(443)));
        assert_eq!(ev.dir.as_deref(), Some("out"));
        assert_eq!(ev.src.as_deref(), Some("192.168.1.5"));
        assert_eq!(ev.len, frame.len());
    }

    #[test]
    fn vlan_ipv6_udp_dns() {
        let mut f = vec![0u8; 12];
        f.extend_from_slice(&[0x81, 0x00, 0x00, 0x0A, 0x86, 0xDD]);
        let mut ip6 = vec![0x60, 0, 0, 0, 0, 12, 17, 64];
        let src: Ipv6Addr = "2001:db8::1".parse().unwrap();
        let dst: Ipv6Addr = "fd00::53".parse().unwrap();
        ip6.extend_from_slice(&src.octets());
        ip6.extend_from_slice(&dst.octets());
        f.extend_from_slice(&ip6);
        f.extend_from_slice(&[0xC3, 0x50, 0x00, 0x35, 0x00, 0x0C, 0, 0, 1, 2, 3, 4]);
        let ev = Decoder::default().decode(linktype::ETHERNET, &f, f.len());
        assert_eq!(ev.proto, "dns");
        assert_eq!(ev.dport, Some(53));
        assert_eq!(ev.dir.as_deref(), Some("in"));
    }

    #[test]
    fn sll_and_null_loopback() {
        // Linux cooked capture carrying IPv4/UDP to port 5004 (RTP).
        let mut f = vec![0u8; 14];
        f.extend_from_slice(&[0x08, 0x00]);
        let mut ip = vec![
            0x45, 0, 0, 28, 0, 0, 0, 0, 64, 17, 0, 0, 10, 0, 0, 2, 10, 0, 0, 3,
        ];
        ip.extend_from_slice(&[0x13, 0x8C, 0x13, 0x8D, 0, 8, 0, 0]);
        f.extend_from_slice(&ip);
        let ev = Decoder::default().decode(linktype::LINUX_SLL, &f, f.len());
        assert_eq!(ev.proto, "rtp");

        // NULL loopback, little-endian AF_INET, ICMP.
        let mut n = vec![2, 0, 0, 0];
        n.extend_from_slice(&[
            0x45, 0, 0, 20, 0, 0, 0, 0, 64, 1, 0, 0, 127, 0, 0, 1, 127, 0, 0, 1,
        ]);
        let ev = Decoder::default().decode(linktype::NULL, &n, n.len());
        assert_eq!(ev.proto, "icmp");
        assert_eq!(ev.dir.as_deref(), Some("local"));
    }

    #[test]
    fn later_fragments_have_no_ports() {
        let mut frame = eth_ipv4_tcp([10, 0, 0, 1], [10, 0, 0, 2], 1234, 443, &[]);
        // Set fragment offset = 185 (8-byte units) on the IP header (bytes 6..8 after the 14-byte Ethernet header).
        frame[14 + 6] = 0x00;
        frame[14 + 7] = 185;
        let ev = Decoder::default().decode(linktype::ETHERNET, &frame, frame.len());
        assert_eq!(ev.sport, None);
        assert_eq!(ev.proto, "tcp");
    }

    #[test]
    fn garbage_never_panics() {
        let dec = Decoder {
            keep_addrs: true,
            ..Decoder::default()
        };
        let mut rng = Rng::new(0xBAD);
        let lts = [0, 1, 12, 14, 101, 108, 113, 228, 229, 276, 9999];
        for i in 0..30_000 {
            let len = rng.index(120);
            let mut buf: Vec<u8> = (0..len).map(|_| rng.next_u64() as u8).collect();
            // Bias some buffers towards valid-looking headers to reach deeper code.
            if i % 3 == 0 && buf.len() > 20 {
                buf[0] = 0x45;
            }
            if i % 3 == 1 && buf.len() > 20 {
                buf[0] = 0x60;
            }
            if i % 5 == 0 && buf.len() > 14 {
                buf[12] = 0x08;
                buf[13] = 0x00;
                if buf.len() > 15 {
                    buf[14] = 0x45;
                }
            }
            let lt = lts[i % lts.len()];
            let ev = dec.decode(lt, &buf, buf.len());
            assert!(!ev.proto.is_empty());
        }
    }
}
