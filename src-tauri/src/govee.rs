//! Govee LAN API: devices with "LAN Control" on answer a multicast scan.

use serde_json::{json, Value};
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const SCAN: &[u8] = br#"{"msg":{"cmd":"scan","data":{"account_topic":"reserve"}}}"#;

/// Sends one scan and collects replies for 1.5 s. Each reply: {ip, device, sku, ...}.
pub fn scan() -> Result<Vec<Value>, String> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 4002))
        .map_err(|e| format!("Port 4002 is busy (is the Govee desktop app open?): {e}"))?;
    sock.set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|e| e.to_string())?;
    let _ = sock.send_to(SCAN, (Ipv4Addr::new(239, 255, 255, 250), 4001));
    // Many routers drop the multicast scan, so also ask every address on our subnet directly.
    // shortcut: assumes a /24 home network; read the real prefix if someone has a bigger LAN.
    if let Some(me) = local_v4() {
        let [a, b, c, _] = me.octets();
        for pass in 0..2 {
            for i in 1..255 {
                let _ = sock.send_to(SCAN, (Ipv4Addr::new(a, b, c, i), 4001));
            }
            if pass == 0 {
                std::thread::sleep(Duration::from_millis(300)); // let ARP resolve, then resend
            }
        }
    }

    let mut found: Vec<Value> = Vec::new();
    let mut buf = [0u8; 2048];
    let end = Instant::now() + Duration::from_millis(1700);
    while Instant::now() < end {
        // Errors include Windows' ConnectionReset from hosts that aren't lights: skip them.
        let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
        if let Some(d) = parse(&buf[..n]) {
            if !found.iter().any(|f| f["device"] == d["device"]) {
                found.push(d);
            }
        }
    }
    Ok(found)
}

/// Our LAN address: the one Windows would use to reach the internet (no packet is sent).
fn local_v4() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    s.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;
    match s.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_link_local() => Some(ip),
        _ => None,
    }
}

fn sock() -> Result<&'static UdpSocket, String> {
    static SOCK: OnceLock<UdpSocket> = OnceLock::new();
    if let Some(s) = SOCK.get() {
        return Ok(s);
    }
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| e.to_string())?;
    Ok(SOCK.get_or_init(|| s))
}

fn send(ip: &str, msg: Value) -> Result<(), String> {
    let ip: Ipv4Addr = ip.parse().map_err(|_| format!("Bad light address: {ip}"))?;
    sock()?.send_to(msg.to_string().as_bytes(), (ip, 4003)).map(|_| ()).map_err(|e| e.to_string())
}

pub fn turn(ip: &str, on: bool) -> Result<(), String> {
    send(ip, json!({"msg":{"cmd":"turn","data":{"value": on as u8}}}))
}

/// Real-time mode (what DreamView/Razer sync uses): the light shows only what we stream.
pub fn stream_mode(ip: &str, on: bool) -> Result<(), String> {
    send(ip, razer(&[0xBB, 0x00, 0x01, 0xB1, on as u8]))
}

/// One frame of per-segment colours, `rgb` = r,g,b per segment (max 84 segments).
pub fn frame(ip: &str, rgb: &[u8]) -> Result<(), String> {
    let n = (rgb.len() / 3).min(84);
    let mut p = vec![0xBB, 0x00, (2 + 3 * n) as u8, 0xB0, 0x00, n as u8];
    p.extend_from_slice(&rgb[..n * 3]);
    send(ip, razer(&p))
}

/// Appends the XOR checksum and wraps the bytes as a base64 "razer" command.
fn razer(bytes: &[u8]) -> Value {
    let mut p = bytes.to_vec();
    p.push(p.iter().fold(0, |a, b| a ^ b));
    json!({"msg":{"cmd":"razer","data":{"pt": b64(&p)}}})
}

pub fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            s.push(if i <= c.len() { T[(n >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
        }
    }
    s
}

static BLINKING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Starts or stops blinking a light so you can tell which one it is. Stopping turns it back on.
pub fn identify(ip: &str, on: bool) -> Result<(), String> {
    let mut set = BLINKING.lock().unwrap();
    let was = set.iter().any(|x| x == ip);
    set.retain(|x| x != ip);
    if !on {
        return turn(ip, true);
    }
    set.push(ip.to_string());
    if !was {
        let ip = ip.to_string();
        std::thread::spawn(move || {
            let ms = Duration::from_millis(400);
            while BLINKING.lock().unwrap().contains(&ip) {
                let _ = turn(&ip, false);
                std::thread::sleep(ms);
                let _ = turn(&ip, true);
                std::thread::sleep(ms);
            }
        });
    }
    Ok(())
}

fn parse(reply: &[u8]) -> Option<Value> {
    let v: Value = serde_json::from_slice(reply).ok()?;
    let d = v["msg"]["data"].clone();
    d["device"].is_string().then_some(d)
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_scan_reply() {
        let r = br#"{"msg":{"cmd":"scan","data":{"ip":"192.168.1.23","device":"1F:80:C5:32","sku":"H619A"}}}"#;
        assert_eq!(super::parse(r).unwrap()["sku"], "H619A");
        assert!(super::parse(b"junk").is_none());
        assert!(super::parse(br#"{"msg":{"cmd":"scan","data":{}}}"#).is_none());
    }

    #[test]
    #[ignore] // needs real lights, and a firewall rule for the test exe: cargo test live_scan -- --ignored --nocapture
    fn live_scan() {
        let found = super::scan().unwrap();
        println!("{found:?}");
        assert!(!found.is_empty());
    }

    #[test]
    fn razer_packets() {
        // Known "enable real-time mode" packet used by DreamView clients.
        assert_eq!(super::razer(&[0xBB, 0x00, 0x01, 0xB1, 0x01])["msg"]["data"]["pt"], "uwABsQEK");
        assert_eq!(super::b64(b"ab"), "YWI=");
    }
}
