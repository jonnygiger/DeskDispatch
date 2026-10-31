use macaddr::MacAddr6;
use std::net::SocketAddr;
use tokio::net::UdpSocket;

/// Parses a single hex ASCII digit into its numeric value `0..=15`.
#[inline]
fn parse_hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Parses two hexadecimal ASCII characters into a single byte.
#[inline]
fn parse_hex_pair(hi: u8, lo: u8) -> Option<u8> {
    let hi = parse_hex_digit(hi)?;
    let lo = parse_hex_digit(lo)?;
    Some((hi << 4) | lo)
}

/// Parses a MAC address string into a 6-byte array without heap allocations.
/// Supports formats: `XX:XX:XX:XX:XX:XX`, `XX-XX-XX-XX-XX-XX`, or `XXXXXXXXXXXX`.
pub fn parse_mac_address(mac_str: &str) -> Result<[u8; 6], String> {
    let s = mac_str.trim().as_bytes();
    let mut mac = [0u8; 6];

    match s.len() {
        // Delimited format: 17 ASCII characters (e.g. "00:11:22:33:44:55" or "00-11-22-33-44-55")
        17 => {
            let delim = s[2];
            if delim != b':' && delim != b'-' {
                return Err(format!("Invalid MAC address '{mac_str}'"));
            }
            for (i, slot) in mac.iter_mut().enumerate() {
                let offset = i * 3;
                if i > 0 && s[offset - 1] != delim {
                    return Err(format!("Invalid MAC address '{mac_str}'"));
                }
                match parse_hex_pair(s[offset], s[offset + 1]) {
                    Some(byte) => *slot = byte,
                    None => return Err(format!("Invalid MAC address '{mac_str}'")),
                }
            }
            Ok(mac)
        }
        // Raw hex format: 12 ASCII characters (e.g. "001122334455")
        12 => {
            for (i, slot) in mac.iter_mut().enumerate() {
                let offset = i * 2;
                match parse_hex_pair(s[offset], s[offset + 1]) {
                    Some(byte) => *slot = byte,
                    None => return Err(format!("Invalid MAC address '{mac_str}'")),
                }
            }
            Ok(mac)
        }
        _ => Err(format!("Invalid MAC address '{mac_str}'")),
    }
}

/// Constructs a 102-byte Wake-on-LAN Magic Packet for the given 6-byte MAC address.
///
/// The magic packet consists of 6 bytes of 0xFF followed by 16 repetitions of the target MAC address.
pub fn create_magic_packet(mac: &[u8; 6]) -> [u8; 102] {
    let mut packet = [0xFFu8; 102];
    let (chunks, _) = packet[6..].as_chunks_mut::<6>();
    for chunk in chunks {
        chunk.copy_from_slice(mac);
    }
    packet
}

/// Sends a Wake-on-LAN magic packet via UDP broadcast (`255.255.255.255:9`).
pub async fn send_wol_packet(mac_str: &str) -> Result<(), String> {
    let mac = parse_mac_address(mac_str)?;
    let packet = create_magic_packet(&mac);

    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| format!("Failed to bind UDP socket for WOL: {e}"))?;

    socket
        .set_broadcast(true)
        .map_err(|e| format!("Failed to set UDP broadcast option for WOL: {e}"))?;

    let dest: SocketAddr = "255.255.255.255:9"
        .parse()
        .map_err(|e| format!("Invalid broadcast address for WOL: {e}"))?;

    socket
        .send_to(&packet, dest)
        .await
        .map_err(|e| format!("Failed to send WOL packet to {mac_str}: {e}"))?;

    tracing::info!("Sent Wake-on-LAN magic packet to MAC {mac_str}");
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mac_address_colon_format() {
        let mac = parse_mac_address("00:11:22:33:44:55").expect("Valid MAC address");
        assert_eq!(mac, [0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);
    }

    #[test]
    fn test_parse_mac_address_hyphen_format() {
        let mac = parse_mac_address("AA-BB-CC-DD-EE-FF").expect("valid mac");
        assert_eq!(mac, [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn test_parse_mac_address_raw_hex() {
        let mac = parse_mac_address("0123456789ab").expect("Valid MAC address");
        assert_eq!(mac, [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB]);
    }

    #[test]
    fn test_parse_mac_address_invalid() {
        assert!(parse_mac_address("invalid").is_err());
        assert!(parse_mac_address("00:11:22:33:44").is_err());
        assert!(parse_mac_address("00:11:22:33:44:55:66").is_err());
        assert!(parse_mac_address("00:11:22:33:44:ZZ").is_err());
        assert!(parse_mac_address("café12345678").is_err());
    }

    #[test]
    fn test_create_magic_packet() {
        let mac = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
        let packet = create_magic_packet(&mac);

        assert_eq!(packet.len(), 102);
        assert_eq!(&packet[..6], &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);

        for i in 0..16 {
            let start = 6 + i * 6;
            assert_eq!(&packet[start..start + 6], &mac);
        }
    }
}
