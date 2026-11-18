//! The small subset of MQTT 3.1.1 this client needs: CONNECT, SUBSCRIBE,
//! PINGREQ and DISCONNECT out; CONNACK, SUBACK, PUBLISH and PINGRESP in.
//! There is deliberately no PUBLISH encoder: the client never publishes.

/// Largest packet accepted from the broker. The broker truncates messages at
/// 4096 bytes (protocols.md section 11.4), so this is generous.
pub const MAX_PACKET: usize = 256 * 1024;

fn put_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(&(s.len() as u16).to_be_bytes());
    buf.extend_from_slice(s.as_bytes());
}

fn frame(header: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![header];
    let mut len = body.len();
    loop {
        let mut b = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            b |= 0x80;
        }
        out.push(b);
        if len == 0 {
            break;
        }
    }
    out.extend_from_slice(body);
    out
}

/// CONNECT with clean session, username and password, and no will.
pub fn connect(client_id: &str, username: &str, password: &str, keep_alive: u16) -> Vec<u8> {
    let mut b = Vec::new();
    put_str(&mut b, "MQTT");
    b.push(4); // protocol level 3.1.1
    b.push(0x80 | 0x40 | 0x02); // username, password, clean session
    b.extend_from_slice(&keep_alive.to_be_bytes());
    put_str(&mut b, client_id);
    put_str(&mut b, username);
    put_str(&mut b, password);
    frame(0x10, &b)
}

/// SUBSCRIBE to the given topics, all with QoS 0.
pub fn subscribe(packet_id: u16, topics: &[String]) -> Vec<u8> {
    let mut b = packet_id.to_be_bytes().to_vec();
    for t in topics {
        put_str(&mut b, t);
        b.push(0);
    }
    frame(0x82, &b)
}

pub const PINGREQ: [u8; 2] = [0xc0, 0x00];
pub const DISCONNECT: [u8; 2] = [0xe0, 0x00];

/// A packet received from the broker.
#[derive(Debug, PartialEq, Eq)]
pub enum Incoming {
    ConnAck { code: u8 },
    SubAck { packet_id: u16, granted: Vec<u8> },
    Publish { topic: String, payload: Vec<u8> },
    PingResp,
    Other(u8),
}

/// Error from [`take_packet`].
#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    TooLarge(usize),
    Malformed(&'static str),
}

/// Removes one complete packet from the front of `buf`. Returns `Ok(None)`
/// when more bytes are needed.
pub fn take_packet(buf: &mut Vec<u8>) -> Result<Option<Incoming>, FrameError> {
    if buf.is_empty() {
        return Ok(None);
    }
    let mut len = 0usize;
    let mut i = 1;
    loop {
        let Some(&b) = buf.get(i) else { return Ok(None) };
        len |= ((b & 0x7f) as usize) << (7 * (i - 1));
        i += 1;
        if b & 0x80 == 0 {
            break;
        }
        if i > 4 {
            return Err(FrameError::Malformed("remaining length too long"));
        }
    }
    if len > MAX_PACKET {
        return Err(FrameError::TooLarge(len));
    }
    if buf.len() < i + len {
        return Ok(None);
    }
    let header = buf[0];
    let body: Vec<u8> = buf[i..i + len].to_vec();
    buf.drain(..i + len);
    Ok(Some(parse(header, body)?))
}

fn parse(header: u8, body: Vec<u8>) -> Result<Incoming, FrameError> {
    Ok(match header >> 4 {
        2 => {
            let code = *body.get(1).ok_or(FrameError::Malformed("short CONNACK"))?;
            Incoming::ConnAck { code }
        }
        9 => {
            if body.len() < 2 {
                return Err(FrameError::Malformed("short SUBACK"));
            }
            Incoming::SubAck { packet_id: u16::from_be_bytes([body[0], body[1]]), granted: body[2..].to_vec() }
        }
        3 => {
            if body.len() < 2 {
                return Err(FrameError::Malformed("short PUBLISH"));
            }
            let tl = u16::from_be_bytes([body[0], body[1]]) as usize;
            let qos = (header >> 1) & 3;
            let off = 2 + tl + if qos > 0 { 2 } else { 0 };
            if body.len() < off {
                return Err(FrameError::Malformed("short PUBLISH"));
            }
            let topic = String::from_utf8_lossy(&body[2..2 + tl]).into_owned();
            Incoming::Publish { topic, payload: body[off..].to_vec() }
        }
        13 => Incoming::PingResp,
        _ => Incoming::Other(header),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_layout() {
        let p = connect("p_0123456789abcdef", "u", "t", 60);
        assert_eq!(p[0], 0x10);
        // "MQTT", level 4, flags, keep-alive 60.
        assert_eq!(&p[2..12], &[0, 4, b'M', b'Q', b'T', b'T', 4, 0xc2, 0, 60]);
    }

    #[test]
    fn long_remaining_length_round_trip() {
        let mut body = vec![0, 3, b'a', b'/', b'b'];
        body.extend_from_slice(&[b'x'; 300]);
        let mut buf = frame(0x30, &body);
        assert_eq!(buf[1..3], [0xb1, 0x02]); // 305 = 0x31 + 2 * 128
        buf.extend_from_slice(&[0xd0, 0x00, 0x20]);
        let p = take_packet(&mut buf).unwrap().unwrap();
        assert!(matches!(p, Incoming::Publish { ref topic, ref payload } if topic == "a/b" && payload.len() == 300));
        assert_eq!(take_packet(&mut buf).unwrap(), Some(Incoming::PingResp));
        // A partial CONNACK waits for more bytes.
        assert_eq!(take_packet(&mut buf).unwrap(), None);
        buf.extend_from_slice(&[0x02, 0x00, 0x05]);
        assert_eq!(take_packet(&mut buf).unwrap(), Some(Incoming::ConnAck { code: 5 }));
        assert!(buf.is_empty());
    }

    #[test]
    fn suback_and_qos1_publish() {
        let mut buf = vec![0x90, 0x03, 0x00, 0x01, 0x00];
        assert_eq!(take_packet(&mut buf).unwrap(), Some(Incoming::SubAck { packet_id: 1, granted: vec![0] }));
        let mut buf = vec![0x32, 0x06, 0x00, 0x01, b't', 0x00, 0x07, b'{'];
        assert_eq!(take_packet(&mut buf).unwrap(), Some(Incoming::Publish { topic: "t".into(), payload: b"{".to_vec() }));
    }

    #[test]
    fn oversized_packet_is_rejected() {
        let mut buf = vec![0x30, 0xff, 0xff, 0xff, 0x7f];
        assert!(matches!(take_packet(&mut buf), Err(FrameError::TooLarge(_))));
    }
}
