// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP over WebSockets (transport, RFC 107), over loopback TCP with a minimal client.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{Client, Sim, node};
use must_server::RunningNode;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::runtime::Handle;

fn start() -> RunningNode {
    start_with(true)
}

fn start_with(websockets: bool) -> RunningNode {
    node(Arc::new(Sim::default()))
        .max_line_length(1024)
        .websockets(websockets)
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap()
}

const UPGRADE: &str = "GET /secop HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
    Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
    Sec-WebSocket-Version: 13\r\n\r\n";

/// A WebSocket client: masked frames out, server frames in.
struct WsClient {
    stream: TcpStream,
}

impl WsClient {
    async fn connect(node: &RunningNode) -> Self {
        Self::connect_sending(node, b"").await
    }

    /// Connects, sending `after` right behind the upgrade request.
    async fn connect_sending(node: &RunningNode, after: &[u8]) -> Self {
        let mut stream = TcpStream::connect(node.local_addr()).await.unwrap();
        let mut request = UPGRADE.as_bytes().to_vec();
        request.extend_from_slice(after);
        stream.write_all(&request).await.unwrap();
        let mut response = Vec::new();
        while !response.ends_with(b"\r\n\r\n") {
            response.push(stream.read_u8().await.unwrap());
        }
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 101 "), "{response}");
        // header names are case-insensitive
        let accept = "sec-websocket-accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";
        assert!(
            response
                .to_ascii_lowercase()
                .contains(&accept.to_ascii_lowercase())
        );
        Self { stream }
    }

    async fn send_frame(&mut self, first: u8, payload: &[u8]) {
        let mask = [1, 2, 3, 4];
        let mut frame = vec![first];
        match payload.len() {
            n @ 0..=125 => frame.push(0x80 | u8::try_from(n).unwrap()),
            n => {
                frame.push(0xFE);
                frame.extend_from_slice(&u16::try_from(n).unwrap().to_be_bytes());
            }
        }
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.stream.write_all(&frame).await.unwrap();
    }

    async fn send(&mut self, message: &str) {
        self.send_frame(0x81, message.as_bytes()).await;
    }

    /// The next frame: opcode and payload; panics after 5 s.
    async fn frame(&mut self) -> (u8, Vec<u8>) {
        tokio::time::timeout(Duration::from_secs(5), async {
            let first = self.stream.read_u8().await.unwrap();
            let len = match self.stream.read_u8().await.unwrap() {
                126 => usize::from(self.stream.read_u16().await.unwrap()),
                127 => usize::try_from(self.stream.read_u64().await.unwrap()).unwrap(),
                n => usize::from(n),
            };
            let mut payload = vec![0; len];
            self.stream.read_exact(&mut payload).await.unwrap();
            assert_eq!(first & 0x80, 0x80, "unfragmented");
            (first & 0x0F, payload)
        })
        .await
        .expect("no frame within 5 s")
    }

    /// The next SECoP message, from a TEXT frame.
    async fn message(&mut self) -> String {
        let (opcode, payload) = self.frame().await;
        assert_eq!(opcode, 1, "TEXT frame");
        String::from_utf8(payload).unwrap()
    }

    async fn request(&mut self, message: &str) -> String {
        self.send(message).await;
        self.message().await
    }

    /// The next frame, which must be a close frame: its status code.
    async fn close_code(&mut self) -> u16 {
        let (opcode, payload) = self.frame().await;
        assert_eq!(opcode, 8, "CLOSE frame");
        u16::from_be_bytes([payload[0], payload[1]])
    }
}

/// A masked client frame, as [`WsClient::send_frame`] sends it.
fn masked(first: u8, payload: &[u8]) -> Vec<u8> {
    let mask = [1, 2, 3, 4];
    let mut frame = vec![first, 0x80 | u8::try_from(payload.len()).unwrap()];
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    frame
}

#[tokio::test]
async fn messages_in_frames() {
    let n = start();
    let mut c = WsClient::connect(&n).await;
    // one message per frame, with or without the line ending; replies have none
    assert!(c.request("*IDN?").await.starts_with("ISSE,SECoP,"));
    let reply = c.request("read t1:value\n").await;
    assert!(reply.starts_with("reply t1:value [295.0,"), "{reply}");
    // a message split over two frames, and a BINARY message
    c.send_frame(0x01, b"ping ").await;
    c.send_frame(0x80, b"42").await;
    assert!(c.message().await.starts_with("pong 42 [null,"));
    c.send_frame(0x82, b"ping 43").await;
    assert!(c.message().await.starts_with("pong 43 [null,"));
    // updates are frames too
    c.send("activate t2").await;
    loop {
        let message = c.message().await;
        if message == "active t2" {
            break;
        }
        assert!(message.starts_with("update t2:"), "{message}");
    }
    // the same errors as over TCP
    let error = c.request("meas:volt?").await;
    assert!(error.starts_with("error_meas:volt? "), "{error}");
}

/// The line-length limit applies to messages; a longer one closes the WebSocket.
#[tokio::test]
async fn too_long_message_closes_with_1009() {
    let n = start();
    let mut c = WsClient::connect(&n).await;
    // within the limit, a trailing line ending included
    let change = format!("change t1:target {}", "1".repeat(1000));
    let reply = c.request(&format!("{change:<1024}\n")).await;
    assert!(reply.starts_with("error_change t1:target "), "{reply}");
    c.send(&format!("change t1:target {}", "1".repeat(2000)))
        .await;
    assert_eq!(c.close_code().await, 1009);
}

/// A client may send its first frame right behind the request.
#[tokio::test]
async fn frame_right_after_the_request() {
    let n = start();
    let mut c = WsClient::connect_sending(&n, &masked(0x81, b"*IDN?")).await;
    assert!(c.message().await.starts_with("ISSE,SECoP,"));
}

#[tokio::test]
async fn node_stop_closes_with_1001() {
    let n = start();
    let mut c = WsClient::connect(&n).await;
    assert!(c.request("*IDN?").await.starts_with("ISSE,SECoP,"));
    n.stop().await;
    assert_eq!(c.close_code().await, 1001);
}

/// Turned off, `GET /` is an unknown action, as without WebSocket support.
#[tokio::test]
async fn turned_off() {
    let n = start_with(false);
    let mut c = Client::connect(n.local_addr()).await;
    c.send_raw(UPGRADE.as_bytes()).await;
    let error = c.line().await;
    assert!(error.starts_with("error_GET "), "{error}");
}

#[tokio::test]
async fn ping_and_close() {
    let n = start();
    let mut c = WsClient::connect(&n).await;
    c.send_frame(0x89, b"hi").await;
    assert_eq!(c.frame().await, (0xA, b"hi".to_vec()));
    c.send_frame(0x88, &1000u16.to_be_bytes()).await;
    assert_eq!(c.close_code().await, 1000);
    let mut rest = Vec::new();
    let closed = tokio::time::timeout(Duration::from_secs(5), c.stream.read_to_end(&mut rest));
    assert!(closed.await.is_ok(), "the node closes the connection");
}

#[tokio::test]
async fn unmasked_frame_closes_with_protocol_error() {
    let n = start();
    let mut c = WsClient::connect(&n).await;
    c.stream.write_all(&[0x81, 2, b'h', b'i']).await.unwrap();
    assert_eq!(c.close_code().await, 1002);
}

#[tokio::test]
async fn plain_http_get_is_refused() {
    let n = start();
    let mut stream = TcpStream::connect(n.local_addr()).await.unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.starts_with("HTTP/1.1 400 "), "{response}");
}

/// A raw TCP client is not affected, also when its first bytes arrive one at a time.
#[tokio::test]
async fn raw_tcp_still_works() {
    let n = start();
    let mut c = Client::connect(n.local_addr()).await;
    for b in b"*IDN?\n" {
        c.send_raw(std::slice::from_ref(b)).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(c.line().await.starts_with("ISSE,SECoP,"));
}
