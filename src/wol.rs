use std::net::SocketAddr;
use tokio::net::UdpSocket;

/// Parses a MAC address string into a 6-byte array.
/// Supports formats: `XX:XX:XX:XX:XX:XX`, `XX-XX-XX-XX-XX-XX`, or `XXXXXXXXXXXX`.
pub fn parse_mac_address(mac_str: &str) -> Result<[u8; 6], String> {
    let clean_str: String = mac_str.chars().filter(|c| c.is_ascii_hexdigit()).collect();

    if clean_str.len() != 12 {
        return Err(format!(
            "Invalid MAC address '{}': expected 12 hexadecimal digits",
            mac_str
        ));
    }

    let mut bytes = [0u8; 6];
    for i in 0..6 {
        bytes[i] = u8::from_str_radix(&clean_str[i * 2..i * 2 + 2], 16)
            .map_err(|e| format!("Invalid hex byte in MAC address '{}': {}", mac_str, e))?;
    }

    Ok(bytes)
}

/// Constructs a 102-byte Wake-on-LAN Magic Packet for the given 6-byte MAC address.
///
/// The magic packet consists of 6 bytes of 0xFF followed by 16 repetitions of the target MAC address.
pub fn create_magic_packet(mac: &[u8; 6]) -> [u8; 102] {
    let mut packet = [0u8; 102];
    // First 6 bytes are 0xFF
    packet[..6].fill(0xFF);
    // Next 96 bytes are 16 repetitions of the MAC address
    for i in 0..16 {
        let start = 6 + i * 6;
        packet[start..start + 6].copy_from_slice(mac);
    }
    packet
}

/// Sends a Wake-on-LAN magic packet via UDP broadcast (`255.255.255.255:9`).
pub async fn send_wol_packet(mac_str: &str) -> Result<(), String> {
    let mac = parse_mac_address(mac_str)?;
    let packet = create_magic_packet(&mac);

    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| format!("Failed to bind UDP socket for WOL: {}", e))?;

    socket
        .set_broadcast(true)
        .map_err(|e| format!("Failed to set UDP broadcast option for WOL: {}", e))?;

    let dest: SocketAddr = "255.255.255.255:9"
        .parse()
        .map_err(|e| format!("Invalid broadcast address for WOL: {}", e))?;

    socket
        .send_to(&packet, dest)
        .await
        .map_err(|e| format!("Failed to send WOL packet to {}: {}", mac_str, e))?;

    tracing::info!("Sent Wake-on-LAN magic packet to MAC {}", mac_str);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mac_address_colon_format() {
        let mac = parse_mac_address("00:11:22:33:44:55").expect("valid mac");
        assert_eq!(mac, [0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);
    }

    #[test]
    fn test_parse_mac_address_hyphen_format() {
        let mac = parse_mac_address("AA-BB-CC-DD-EE-FF").expect("valid mac");
        assert_eq!(mac, [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn test_parse_mac_address_raw_hex() {
        let mac = parse_mac_address("0123456789ab").expect("valid mac");
        assert_eq!(mac, [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB]);
    }

    #[test]
    fn test_parse_mac_address_invalid() {
        assert!(parse_mac_address("invalid").is_err());
        assert!(parse_mac_address("00:11:22:33:44").is_err());
        assert!(parse_mac_address("00:11:22:33:44:55:66").is_err());
        assert!(parse_mac_address("00:11:22:33:44:ZZ").is_err());
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
