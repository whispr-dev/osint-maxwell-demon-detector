use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use pcap::{Linktype, PacketHeader};

use crate::event::PacketEvent;

pub fn packet_to_event(
    linktype: Linktype,
    header: &PacketHeader,
    data: &[u8],
    local_addrs: &HashSet<IpAddr>,
) -> Option<PacketEvent> {
    let ts_ms = Some(timeval_to_ms(header));
    let wire_len = usize::try_from(header.len).unwrap_or(data.len());

    match linktype {
        lt if lt == Linktype::ETHERNET => parse_ethernet(data, wire_len, ts_ms, local_addrs),
        lt if lt == Linktype::LINUX_SLL => parse_linux_sll(data, wire_len, ts_ms, local_addrs),
        lt if lt == Linktype::RAW => parse_ip_guess(data, wire_len, ts_ms, local_addrs),
        lt if lt == Linktype::NULL || lt == Linktype::LOOP => {
            parse_null_loopback(data, wire_len, ts_ms, local_addrs)
        }
        _ => Some(PacketEvent {
            ts_ms,
            len: wire_len,
            proto: "other".to_string(),
            src: None,
            dst: None,
            sport: None,
            dport: None,
            dir: None,
        }),
    }
}

fn parse_ethernet(
    data: &[u8],
    wire_len: usize,
    ts_ms: Option<u64>,
    local_addrs: &HashSet<IpAddr>,
) -> Option<PacketEvent> {
    if data.len() < 14 {
        return None;
    }
    let mut offset = 14usize;
    let mut ethertype = be_u16(data, 12)?;

    if matches!(ethertype, 0x8100 | 0x88a8 | 0x9100) {
        if data.len() < 18 {
            return None;
        }
        ethertype = be_u16(data, 16)?;
        offset = 18;
    }

    parse_network_from_ethertype(ethertype, &data[offset..], wire_len, ts_ms, local_addrs)
}

fn parse_linux_sll(
    data: &[u8],
    wire_len: usize,
    ts_ms: Option<u64>,
    local_addrs: &HashSet<IpAddr>,
) -> Option<PacketEvent> {
    if data.len() < 16 {
        return None;
    }
    let proto = be_u16(data, 14)?;
    parse_network_from_ethertype(proto, &data[16..], wire_len, ts_ms, local_addrs)
}

fn parse_null_loopback(
    data: &[u8],
    wire_len: usize,
    ts_ms: Option<u64>,
    local_addrs: &HashSet<IpAddr>,
) -> Option<PacketEvent> {
    if data.len() < 4 {
        return None;
    }

    let fam_le = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let fam_be = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let payload = &data[4..];

    let family = if is_ipv4_family(fam_le) || is_ipv6_family(fam_le) {
        fam_le
    } else {
        fam_be
    };

    if is_ipv4_family(family) {
        parse_ipv4(payload, wire_len, ts_ms, local_addrs)
    } else if is_ipv6_family(family) {
        parse_ipv6(payload, wire_len, ts_ms, local_addrs)
    } else {
        parse_ip_guess(payload, wire_len, ts_ms, local_addrs)
    }
}

fn parse_ip_guess(
    data: &[u8],
    wire_len: usize,
    ts_ms: Option<u64>,
    local_addrs: &HashSet<IpAddr>,
) -> Option<PacketEvent> {
    let version = data.first().map(|b| b >> 4)?;
    match version {
        4 => parse_ipv4(data, wire_len, ts_ms, local_addrs),
        6 => parse_ipv6(data, wire_len, ts_ms, local_addrs),
        _ => Some(PacketEvent {
            ts_ms,
            len: wire_len,
            proto: "other".to_string(),
            src: None,
            dst: None,
            sport: None,
            dport: None,
            dir: None,
        }),
    }
}

fn parse_network_from_ethertype(
    ethertype: u16,
    payload: &[u8],
    wire_len: usize,
    ts_ms: Option<u64>,
    local_addrs: &HashSet<IpAddr>,
) -> Option<PacketEvent> {
    match ethertype {
        0x0800 => parse_ipv4(payload, wire_len, ts_ms, local_addrs),
        0x86dd => parse_ipv6(payload, wire_len, ts_ms, local_addrs),
        0x0806 => Some(PacketEvent {
            ts_ms,
            len: wire_len,
            proto: "arp".to_string(),
            src: None,
            dst: None,
            sport: None,
            dport: None,
            dir: None,
        }),
        _ => Some(PacketEvent {
            ts_ms,
            len: wire_len,
            proto: "other".to_string(),
            src: None,
            dst: None,
            sport: None,
            dport: None,
            dir: None,
        }),
    }
}

fn parse_ipv4(
    data: &[u8],
    wire_len: usize,
    ts_ms: Option<u64>,
    local_addrs: &HashSet<IpAddr>,
) -> Option<PacketEvent> {
    if data.len() < 20 || (data[0] >> 4) != 4 {
        return None;
    }
    let ihl = ((data[0] & 0x0f) as usize) * 4;
    if ihl < 20 || data.len() < ihl {
        return None;
    }

    let src = IpAddr::V4(Ipv4Addr::new(data[12], data[13], data[14], data[15]));
    let dst = IpAddr::V4(Ipv4Addr::new(data[16], data[17], data[18], data[19]));
    let proto_num = data[9];
    let frag = be_u16(data, 6)?;
    let fragmented = (frag & 0x1fff) != 0 || (frag & 0x2000) != 0;
    let payload = &data[ihl..];

    parse_transport(proto_num, payload, fragmented, wire_len, ts_ms, src, dst, local_addrs)
}

fn parse_ipv6(
    data: &[u8],
    wire_len: usize,
    ts_ms: Option<u64>,
    local_addrs: &HashSet<IpAddr>,
) -> Option<PacketEvent> {
    if data.len() < 40 || (data[0] >> 4) != 6 {
        return None;
    }

    let src = IpAddr::V6(Ipv6Addr::from([
        data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15], data[16],
        data[17], data[18], data[19], data[20], data[21], data[22], data[23],
    ]));
    let dst = IpAddr::V6(Ipv6Addr::from([
        data[24], data[25], data[26], data[27], data[28], data[29], data[30], data[31],
        data[32], data[33], data[34], data[35], data[36], data[37], data[38], data[39],
    ]));

    let mut next = data[6];
    let mut offset = 40usize;
    let mut fragmented = false;

    for _ in 0..8 {
        match next {
            0 | 43 | 60 => {
                if data.len() < offset + 8 {
                    return None;
                }
                next = data[offset];
                let hdr_len = (data[offset + 1] as usize + 1) * 8;
                offset = offset.saturating_add(hdr_len);
            }
            44 => {
                if data.len() < offset + 8 {
                    return None;
                }
                next = data[offset];
                fragmented = true;
                offset = offset.saturating_add(8);
            }
            51 => {
                if data.len() < offset + 2 {
                    return None;
                }
                next = data[offset];
                let hdr_len = (data[offset + 1] as usize + 2) * 4;
                offset = offset.saturating_add(hdr_len);
            }
            50 => {
                return Some(build_event(
                    ts_ms,
                    wire_len,
                    "esp".to_string(),
                    Some(src),
                    Some(dst),
                    None,
                    None,
                    local_addrs,
                ));
            }
            _ => break,
        }

        if data.len() < offset {
            return None;
        }
    }

    parse_transport(next, &data[offset..], fragmented, wire_len, ts_ms, src, dst, local_addrs)
}

fn parse_transport(
    proto_num: u8,
    payload: &[u8],
    fragmented: bool,
    wire_len: usize,
    ts_ms: Option<u64>,
    src: IpAddr,
    dst: IpAddr,
    local_addrs: &HashSet<IpAddr>,
) -> Option<PacketEvent> {
    if fragmented {
        return Some(build_event(
            ts_ms,
            wire_len,
            protocol_name(proto_num).to_string(),
            Some(src),
            Some(dst),
            None,
            None,
            local_addrs,
        ));
    }

    match proto_num {
        6 => parse_tcp(payload, wire_len, ts_ms, src, dst, local_addrs),
        17 => parse_udp(payload, wire_len, ts_ms, src, dst, local_addrs),
        1 => Some(build_event(
            ts_ms,
            wire_len,
            "icmp".to_string(),
            Some(src),
            Some(dst),
            None,
            None,
            local_addrs,
        )),
        58 => Some(build_event(
            ts_ms,
            wire_len,
            "icmpv6".to_string(),
            Some(src),
            Some(dst),
            None,
            None,
            local_addrs,
        )),
        _ => Some(build_event(
            ts_ms,
            wire_len,
            protocol_name(proto_num).to_string(),
            Some(src),
            Some(dst),
            None,
            None,
            local_addrs,
        )),
    }
}

fn parse_tcp(
    data: &[u8],
    wire_len: usize,
    ts_ms: Option<u64>,
    src: IpAddr,
    dst: IpAddr,
    local_addrs: &HashSet<IpAddr>,
) -> Option<PacketEvent> {
    if data.len() < 20 {
        return Some(build_event(
            ts_ms,
            wire_len,
            "tcp".to_string(),
            Some(src),
            Some(dst),
            None,
            None,
            local_addrs,
        ));
    }

    let sport = be_u16(data, 0);
    let dport = be_u16(data, 2);
    let data_offset = ((data[12] >> 4) as usize) * 4;
    let payload = if data_offset >= 20 && data.len() >= data_offset {
        &data[data_offset..]
    } else {
        &[][..]
    };
    let proto = guess_app_proto_tcp(sport, dport, payload);

    Some(build_event(
        ts_ms,
        wire_len,
        proto,
        Some(src),
        Some(dst),
        sport,
        dport,
        local_addrs,
    ))
}

fn parse_udp(
    data: &[u8],
    wire_len: usize,
    ts_ms: Option<u64>,
    src: IpAddr,
    dst: IpAddr,
    local_addrs: &HashSet<IpAddr>,
) -> Option<PacketEvent> {
    if data.len() < 8 {
        return Some(build_event(
            ts_ms,
            wire_len,
            "udp".to_string(),
            Some(src),
            Some(dst),
            None,
            None,
            local_addrs,
        ));
    }

    let sport = be_u16(data, 0);
    let dport = be_u16(data, 2);
    let payload = &data[8..];
    let proto = guess_app_proto_udp(sport, dport, payload);

    Some(build_event(
        ts_ms,
        wire_len,
        proto,
        Some(src),
        Some(dst),
        sport,
        dport,
        local_addrs,
    ))
}

fn build_event(
    ts_ms: Option<u64>,
    wire_len: usize,
    proto: String,
    src: Option<IpAddr>,
    dst: Option<IpAddr>,
    sport: Option<u16>,
    dport: Option<u16>,
    local_addrs: &HashSet<IpAddr>,
) -> PacketEvent {
    let dir = infer_direction(src.as_ref(), dst.as_ref(), local_addrs);
    PacketEvent {
        ts_ms,
        len: wire_len,
        proto,
        src: src.map(|v| v.to_string()),
        dst: dst.map(|v| v.to_string()),
        sport,
        dport,
        dir,
    }
}

fn infer_direction(
    src: Option<&IpAddr>,
    dst: Option<&IpAddr>,
    local_addrs: &HashSet<IpAddr>,
) -> Option<String> {
    let src = src?;
    let dst = dst?;
    let src_local = local_addrs.contains(src);
    let dst_local = local_addrs.contains(dst);
    match (src_local, dst_local) {
        (true, false) => Some("out".to_string()),
        (false, true) => Some("in".to_string()),
        (true, true) => Some("local".to_string()),
        _ => None,
    }
}

fn guess_app_proto_tcp(sport: Option<u16>, dport: Option<u16>, payload: &[u8]) -> String {
    let ports = [sport, dport];
    let has_port = |p| ports.iter().flatten().any(|v| *v == p);

    if has_port(22) || payload.starts_with(b"SSH-") {
        return "ssh".to_string();
    }
    if has_port(53) {
        return "dns".to_string();
    }
    if has_port(25) || has_port(465) || has_port(587) {
        return "smtp".to_string();
    }
    if has_port(110) || has_port(995) {
        return "pop3".to_string();
    }
    if has_port(143) || has_port(993) {
        return "imap".to_string();
    }
    if has_port(20) || has_port(21) {
        return "ftp".to_string();
    }
    if has_port(445) {
        return "smb".to_string();
    }
    if has_port(3389) {
        return "rdp".to_string();
    }
    if has_port(80) || has_port(8080) {
        if looks_like_http(payload) {
            return "http".to_string();
        }
        return "http".to_string();
    }
    if has_port(443) || has_port(8443) {
        if looks_like_tls(payload) {
            return "tls".to_string();
        }
        if looks_like_http(payload) {
            return "https".to_string();
        }
        return "tls".to_string();
    }
    if looks_like_http(payload) {
        return "http".to_string();
    }
    if looks_like_tls(payload) {
        return "tls".to_string();
    }

    "tcp".to_string()
}

fn guess_app_proto_udp(sport: Option<u16>, dport: Option<u16>, payload: &[u8]) -> String {
    let ports = [sport, dport];
    let has_port = |p| ports.iter().flatten().any(|v| *v == p);

    if has_port(53) {
        return "dns".to_string();
    }
    if has_port(67) || has_port(68) {
        return "dhcp".to_string();
    }
    if has_port(123) {
        return "ntp".to_string();
    }
    if has_port(161) || has_port(162) {
        return "snmp".to_string();
    }
    if has_port(3478) || has_port(3479) {
        return "stun".to_string();
    }
    if has_port(443) {
        return "quic".to_string();
    }
    if has_port(5004) || has_port(5005) {
        return "rtp".to_string();
    }
    if has_port(5060) || has_port(5061) {
        return "sip".to_string();
    }
    if looks_like_dns(payload) {
        return "dns".to_string();
    }

    "udp".to_string()
}

fn looks_like_http(payload: &[u8]) -> bool {
    const PREFIXES: [&[u8]; 8] = [
        b"GET ",
        b"POST ",
        b"PUT ",
        b"HEAD ",
        b"HTTP/",
        b"DELETE ",
        b"OPTIONS ",
        b"PATCH ",
    ];
    PREFIXES.iter().any(|p| payload.starts_with(p))
}

fn looks_like_tls(payload: &[u8]) -> bool {
    payload.len() >= 5
        && matches!(payload[0], 20 | 21 | 22 | 23)
        && payload[1] == 0x03
}

fn looks_like_dns(payload: &[u8]) -> bool {
    payload.len() >= 12 && (payload.len() >= 14 || !payload.is_empty())
}

fn protocol_name(proto_num: u8) -> &'static str {
    match proto_num {
        1 => "icmp",
        2 => "igmp",
        6 => "tcp",
        17 => "udp",
        41 => "ipv6",
        47 => "gre",
        50 => "esp",
        51 => "ah",
        58 => "icmpv6",
        89 => "ospf",
        132 => "sctp",
        _ => "other",
    }
}

fn timeval_to_ms(header: &PacketHeader) -> u64 {
    let secs = if header.ts.tv_sec < 0 {
        0
    } else {
        header.ts.tv_sec as u64
    };
    let micros = if header.ts.tv_usec < 0 {
        0
    } else {
        header.ts.tv_usec as u64
    };
    secs.saturating_mul(1000)
        .saturating_add(micros / 1000)
}

fn be_u16(data: &[u8], offset: usize) -> Option<u16> {
    let slice = data.get(offset..offset + 2)?;
    Some(u16::from_be_bytes([slice[0], slice[1]]))
}

fn is_ipv4_family(v: u32) -> bool {
    matches!(v, 2)
}

fn is_ipv6_family(v: u32) -> bool {
    matches!(v, 24 | 28 | 30)
}
