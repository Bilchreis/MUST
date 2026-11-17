// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP over WebSockets (SECoP transport, RFC 6455), server side: a connection whose
//! first bytes are `GET /` is upgraded with an HTTP handshake, and from then on every
//! SECoP message travels in a frame. The frames' payload goes to the same line splitter
//! as raw TCP, so the line-length limit applies to frames as well.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

/// The first bytes of a connection that asks for a WebSocket.
pub(crate) const PREFIX: &[u8] = b"GET /";

/// The longest HTTP request the handshake reads.
const MAX_REQUEST: usize = 8 * 1024;

/// What the first bytes of a connection say about it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Sniff {
    /// Too few bytes to tell.
    Undecided,
    /// A WebSocket handshake.
    WebSocket,
    /// SECoP over raw TCP.
    Tcp,
}

pub(crate) fn sniff(first: &[u8]) -> Sniff {
    if first.starts_with(PREFIX) {
        Sniff::WebSocket
    } else if PREFIX.starts_with(first) {
        Sniff::Undecided
    } else {
        Sniff::Tcp
    }
}

/// The outcome of reading the HTTP request of the handshake.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Handshake {
    /// The request is not complete yet.
    Incomplete,
    /// The request was accepted: the response to send, and how many bytes it took (the
    /// rest are frames).
    Accepted { response: Vec<u8>, used: usize },
    /// The request was refused: the response to send before closing, and why.
    Refused {
        response: Vec<u8>,
        reason: &'static str,
    },
}

/// Reads the HTTP request of the handshake from the bytes received so far.
pub(crate) fn handshake(request: &[u8]) -> Handshake {
    let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
        return if request.len() > MAX_REQUEST {
            refuse("the HTTP request is too long")
        } else {
            Handshake::Incomplete
        };
    };
    let Ok(head) = std::str::from_utf8(&request[..end]) else {
        return refuse("the HTTP request is not UTF-8");
    };
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    if !request_line.ends_with(" HTTP/1.1") {
        return refuse("not an HTTP/1.1 GET request");
    }
    let mut key = None;
    let (mut upgrade, mut version) = (false, false);
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return refuse("malformed HTTP header");
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "upgrade" => upgrade = value.eq_ignore_ascii_case("websocket"),
            "sec-websocket-version" => version = value == "13",
            "sec-websocket-key" => key = Some(value),
            _ => {}
        }
    }
    let Some(key) = key.filter(|_| upgrade && version) else {
        return refuse("not a WebSocket upgrade (version 13)");
    };
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\r\n",
        accept_key(key)
    );
    Handshake::Accepted {
        response: response.into_bytes(),
        used: end + 4,
    }
}

fn refuse(reason: &'static str) -> Handshake {
    Handshake::Refused {
        response: b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
            .to_vec(),
        reason,
    }
}

/// `Sec-WebSocket-Accept` for a `Sec-WebSocket-Key`.
fn accept_key(key: &str) -> String {
    let mut input = key.as_bytes().to_vec();
    input.extend_from_slice(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    BASE64.encode(sha1(&input))
}

/// SHA-1, needed only for the handshake; the names are those of FIPS 180-4.
#[allow(clippy::many_single_char_names)]
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..20 => ((b & c) | (!b & d), 0x5A82_7999),
                20..40 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..60 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            (e, d, c, b, a) = (d, c, b.rotate_left(30), a, t);
        }
        for (hi, v) in h.iter_mut().zip([a, b, c, d, e]) {
            *hi = hi.wrapping_add(v);
        }
    }
    let mut out = [0u8; 20];
    for (chunk, v) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        chunk.copy_from_slice(&v.to_be_bytes());
    }
    out
}

const CONTINUATION: u8 = 0x0;
const TEXT: u8 = 0x1;
const BINARY: u8 = 0x2;
const CLOSE: u8 = 0x8;
const PING: u8 = 0x9;
const PONG: u8 = 0xA;

/// A server frame (unmasked) with one payload.
fn frame(opcode: u8, payload: &[u8], out: &mut Vec<u8>) {
    out.push(0x80 | opcode);
    match payload.len() {
        n @ 0..=125 => out.push(u8::try_from(n).unwrap_or_default()),
        n @ 126..=0xFFFF => {
            out.push(126);
            out.extend_from_slice(&u16::try_from(n).unwrap_or_default().to_be_bytes());
        }
        n => {
            out.push(127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(payload);
}

/// A TEXT frame holding one SECoP message, written to `out` (cleared first).
pub(crate) fn text_frame<'b>(out: &'b mut Vec<u8>, message: &str) -> &'b [u8] {
    out.clear();
    frame(TEXT, message.as_bytes(), out);
    out
}

/// A close frame with a status code.
pub(crate) fn close_frame(code: u16) -> Vec<u8> {
    let mut out = Vec::new();
    frame(CLOSE, &code.to_be_bytes(), &mut out);
    out
}

/// Close codes.
pub(crate) const NORMAL: u16 = 1000;
pub(crate) const PROTOCOL_ERROR: u16 = 1002;

/// What the decoder found in the client's bytes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// Payload bytes of a TEXT or BINARY message, unmasked.
    Data(Vec<u8>),
    /// The end of a message.
    EndOfMessage,
    /// A frame to send back at once (pong).
    Reply(Vec<u8>),
    /// The client closed the connection; the close frame to answer with.
    Closed(Vec<u8>),
    /// The client broke the protocol; the close frame to send.
    Failed(&'static str, Vec<u8>),
}

/// The frame being read.
struct Frame {
    fin: bool,
    opcode: u8,
    mask: [u8; 4],
    remaining: u64,
    /// Payload bytes unmasked so far, for the mask's offset.
    offset: usize,
    /// The payload of a control frame (at most 125 bytes).
    control: Vec<u8>,
}

/// Decodes client frames as they arrive, passing message payload through without
/// keeping it.
#[derive(Default)]
pub(crate) struct Decoder {
    header: Vec<u8>,
    frame: Option<Frame>,
    /// Inside a fragmented message.
    in_message: bool,
    /// After a close or a protocol error, nothing more is read.
    done: bool,
}

impl Decoder {
    /// Decodes `data`, appending what it found to `events`.
    pub fn feed(&mut self, mut data: &[u8], events: &mut Vec<Event>) {
        while !self.done && !data.is_empty() {
            if self.frame.is_none() {
                let taken = self.read_header(data, events);
                data = &data[taken..];
                continue;
            }
            data = self.read_payload(data, events);
        }
    }

    fn fail(&mut self, reason: &'static str, events: &mut Vec<Event>) {
        self.done = true;
        events.push(Event::Failed(reason, close_frame(PROTOCOL_ERROR)));
    }

    /// Takes header bytes; returns how many of `data` it used.
    fn read_header(&mut self, data: &[u8], events: &mut Vec<Event>) -> usize {
        let before = self.header.len();
        self.header
            .extend_from_slice(&data[..data.len().min(14 - before)]);
        let h = &self.header;
        if h.len() < 2 {
            return data.len();
        }
        // without a mask the header has no mask key to wait for
        if h[1] & 0x80 == 0 {
            self.header.clear();
            self.fail("a client frame is not masked", events);
            return data.len();
        }
        let short = h[1] & 0x7F;
        let length_bytes = match short {
            126 => 2,
            127 => 8,
            _ => 0,
        };
        let needed = 2 + length_bytes + 4;
        if h.len() < needed {
            return data.len().min(14 - before);
        }
        let (fin, opcode) = (h[0] & 0x80 != 0, h[0] & 0x0F);
        let remaining = match length_bytes {
            0 => u64::from(short),
            2 => u64::from(u16::from_be_bytes([h[2], h[3]])),
            _ => u64::from_be_bytes(h[2..10].try_into().unwrap_or_default()),
        };
        let mask = h[needed - 4..needed].try_into().unwrap_or_default();
        let reserved = h[0] & 0x70 != 0;
        self.header.clear();
        let used = needed - before;
        let control = opcode & 0x8 != 0;
        let problem = if reserved {
            Some("reserved bits are set")
        } else if control && (!fin || remaining > 125) {
            Some("a control frame is fragmented or too long")
        } else {
            match opcode {
                CONTINUATION if !self.in_message => Some("a continuation without a message"),
                TEXT | BINARY if self.in_message => Some("a new message inside a fragmented one"),
                CONTINUATION | TEXT | BINARY | CLOSE | PING | PONG => None,
                _ => Some("an unknown opcode"),
            }
        };
        if let Some(problem) = problem {
            self.fail(problem, events);
            return used;
        }
        if !control {
            self.in_message = !fin;
        }
        self.frame = Some(Frame {
            fin,
            opcode,
            mask,
            remaining,
            offset: 0,
            control: Vec::new(),
        });
        self.finish_if_complete(events);
        used
    }

    /// Takes payload bytes of the current frame; returns the rest of `data`.
    fn read_payload<'d>(&mut self, data: &'d [u8], events: &mut Vec<Event>) -> &'d [u8] {
        let Some(frame) = &mut self.frame else {
            return data;
        };
        let n = usize::try_from(frame.remaining).map_or(data.len(), |r| r.min(data.len()));
        let mut payload = data[..n].to_vec();
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= frame.mask[(frame.offset + i) % 4];
        }
        frame.offset = (frame.offset + n) % 4;
        frame.remaining -= n as u64;
        if frame.opcode & 0x8 != 0 {
            frame.control.extend_from_slice(&payload);
        } else if !payload.is_empty() {
            events.push(Event::Data(payload));
        }
        self.finish_if_complete(events);
        &data[n..]
    }

    fn finish_if_complete(&mut self, events: &mut Vec<Event>) {
        if self.frame.as_ref().is_none_or(|f| f.remaining > 0) {
            return;
        }
        let Some(frame) = self.frame.take() else {
            return;
        };
        match frame.opcode {
            PING => {
                let mut pong = Vec::new();
                self::frame(PONG, &frame.control, &mut pong);
                events.push(Event::Reply(pong));
            }
            PONG => {}
            CLOSE => {
                self.done = true;
                events.push(Event::Closed(close_frame(NORMAL)));
            }
            _ if frame.fin => events.push(Event::EndOfMessage),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    /// A masked client frame.
    fn client_frame(first: u8, payload: &[u8]) -> Vec<u8> {
        let mask = [0x37, 0xfa, 0x21, 0x3d];
        let mut out = vec![first];
        match payload.len() {
            n @ 0..=125 => out.push(0x80 | u8::try_from(n).unwrap()),
            n @ 126..=0xFFFF => {
                out.push(0xFE);
                out.extend_from_slice(&u16::try_from(n).unwrap().to_be_bytes());
            }
            n => {
                out.push(0xFF);
                out.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        out.extend_from_slice(&mask);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        out
    }

    fn decode_bytewise(bytes: &[u8]) -> Vec<Event> {
        let mut decoder = Decoder::default();
        let mut events = Vec::new();
        for b in bytes {
            decoder.feed(std::slice::from_ref(b), &mut events);
        }
        events
    }

    fn data(events: &[Event]) -> Vec<u8> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Data(d) => Some(d.as_slice()),
                _ => None,
            })
            .flatten()
            .copied()
            .collect()
    }

    #[test]
    fn sha1_vectors() {
        let hex = |d: [u8; 20]| {
            d.iter().fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            })
        };
        assert_eq!(hex(sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(hex(sha1(long)), "84983e441c3bd26ebaae4aa1f95129e5e54670f1");
    }

    /// The example of RFC 6455, section 1.3.
    #[test]
    fn accept_key_of_the_rfc() {
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn sniffing() {
        assert_eq!(sniff(b"GE"), Sniff::Undecided);
        assert_eq!(sniff(b"GET / HTTP/1.1"), Sniff::WebSocket);
        assert_eq!(sniff(b"*IDN?"), Sniff::Tcp);
        assert_eq!(sniff(b"GEX"), Sniff::Tcp);
    }

    #[test]
    fn handshakes() {
        let good = b"GET /ws HTTP/1.1\r\nHost: x\r\nUpgrade: WebSocket\r\nConnection: Upgrade\r\n\
            Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\nrest";
        let Handshake::Accepted { response, used } = handshake(good) else {
            panic!("refused");
        };
        assert_eq!(&good[used..], b"rest");
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 101 "), "{response}");
        assert!(response.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"));
        assert_eq!(handshake(&good[..40]), Handshake::Incomplete);

        let plain = b"GET / HTTP/1.1\r\nHost: x\r\n\r\n";
        assert!(matches!(handshake(plain), Handshake::Refused { .. }));
        let long = [b"GET / HTTP/1.1\r\nX: ".as_slice(), &[b'a'; MAX_REQUEST]].concat();
        assert!(matches!(handshake(&long), Handshake::Refused { .. }));
    }

    #[test]
    fn messages_in_frames() {
        let mut bytes = client_frame(0x81, b"read t1:value");
        // fragmented, with a ping in between
        bytes.extend(client_frame(0x01, b"des"));
        bytes.extend(client_frame(0x89, b"hi"));
        bytes.extend(client_frame(0x80, b"cribe"));
        let events = decode_bytewise(&bytes);
        assert_eq!(data(&events), b"read t1:valuedescribe");
        let ends = events.iter().filter(|e| **e == Event::EndOfMessage).count();
        assert_eq!(ends, 2);
        assert!(events.contains(&Event::Reply(vec![0x8A, 2, b'h', b'i'])));
    }

    #[test]
    fn long_payload_lengths() {
        for n in [126, 70_000] {
            let payload = vec![b'x'; n];
            let events = decode_bytewise(&client_frame(0x81, &payload));
            assert_eq!(data(&events), payload);
        }
        let mut out = Vec::new();
        assert_eq!(
            text_frame(&mut out, &"y".repeat(300))[..4],
            [0x81, 126, 1, 44]
        );
    }

    #[test]
    fn close_and_protocol_errors() {
        let events = decode_bytewise(&client_frame(0x88, &1000u16.to_be_bytes()));
        assert_eq!(events, [Event::Closed(close_frame(NORMAL))]);

        let unmasked = [0x81, 2, b'h', b'i'];
        let bad = [
            unmasked.to_vec(),
            client_frame(0x80, b"x"),      // continuation without a message
            client_frame(0x83, b"x"),      // unknown opcode
            client_frame(0xC1, b"x"),      // reserved bit
            client_frame(0x09, b"x"),      // fragmented ping
            client_frame(0x89, &[0; 126]), // long ping
        ];
        for bytes in bad {
            let events = decode_bytewise(&bytes);
            assert!(
                matches!(events.last(), Some(Event::Failed(..))),
                "{bytes:?}: {events:?}"
            );
        }
    }
}
