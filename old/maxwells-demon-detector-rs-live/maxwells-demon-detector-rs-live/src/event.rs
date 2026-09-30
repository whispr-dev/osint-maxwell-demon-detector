use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct PacketEvent {
    #[serde(default)]
    pub ts_ms: Option<u64>,
    pub len: usize,
    #[serde(default)]
    pub proto: String,
    #[serde(default)]
    pub src: Option<String>,
    #[serde(default)]
    pub dst: Option<String>,
    #[serde(default)]
    pub sport: Option<u16>,
    #[serde(default)]
    pub dport: Option<u16>,
    #[serde(default)]
    pub dir: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lane {
    Web,
    Dns,
    Mail,
    Media,
    Control,
    Bulk,
    Other,
}

impl Lane {
    pub const ALL: [Lane; 7] = [
        Lane::Web,
        Lane::Dns,
        Lane::Mail,
        Lane::Media,
        Lane::Control,
        Lane::Bulk,
        Lane::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Lane::Web => "WEB",
            Lane::Dns => "DNS",
            Lane::Mail => "MAIL",
            Lane::Media => "MEDIA",
            Lane::Control => "CTRL",
            Lane::Bulk => "BULK",
            Lane::Other => "OTHER",
        }
    }
}

impl PacketEvent {
    pub fn lane(&self) -> Lane {
        let proto = self.proto.to_ascii_lowercase();
        let ports = [self.sport, self.dport];

        let has_port = |needle: u16| ports.iter().flatten().any(|p| *p == needle);
        let has_any = |needles: &[u16]| needles.iter().any(|n| has_port(*n));

        if has_port(53) || proto.contains("dns") {
            return Lane::Dns;
        }
        if has_any(&[80, 443, 8080, 8443])
            || ["http", "https", "tls", "quic", "ws", "wss"].iter().any(|s| proto.contains(s))
        {
            return Lane::Web;
        }
        if has_any(&[25, 110, 143, 465, 587, 993, 995])
            || ["smtp", "imap", "pop3", "mail"].iter().any(|s| proto.contains(s))
        {
            return Lane::Mail;
        }
        if has_any(&[3478, 3479, 5004, 5005, 554, 1935, 5060, 5061])
            || ["rtp", "rtcp", "sip", "rtsp", "webrtc", "video", "audio", "stun"].iter().any(|s| proto.contains(s))
        {
            return Lane::Media;
        }
        if has_any(&[22, 23, 123, 161, 162, 445, 3389])
            || ["ssh", "icmp", "ntp", "snmp", "rdp", "smb", "control"].iter().any(|s| proto.contains(s))
        {
            return Lane::Control;
        }
        if self.len >= 1200
            || has_any(&[20, 21, 989, 990])
            || ["ftp", "bulk", "file", "arp"].iter().any(|s| proto.contains(s))
        {
            return Lane::Bulk;
        }
        Lane::Other
    }
}
