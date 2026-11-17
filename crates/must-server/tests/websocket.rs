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
    node(Arc::new(Sim::default()))
        .max_line_length(1024)
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
        let mut stream = TcpStream::connect(node.local_addr()).await.unwrap();
        stream.write_all(UPGRADE.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        while !response.ends_with(b"\r\n\r\n") {
            response.push(stream.read_u8().await.unwrap());
        }
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 101 "), "{response}");
        assert!(response.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));
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
}

#[tokio::test]
async fn messages_in_frames() {
    let n = start();
    let mut c = WsClient::connect(&n).await;
    // one message per frame, with or without the line ending; replies have none
    assert!(c.request("*IDN?").await.starts_with("ISSE,SECoP,"));
    let reply = c.request("read t1:value\n").await;
    assert!(reply.starts_with("reply t1:value [295.0,"), "{reply}");
    // a message split over two frames
    c.send_frame(0x01, b"ping ").await;
    c.send_frame(0x80, b"42").await;
    assert!(c.message().await.starts_with("pong 42 [null,"));
    // updates are frames too
    c.send("activate t2").await;
    loop {
        let message = c.message().await;
        if message == "active t2" {
            break;
        }
        assert!(message.starts_with("update t2:"), "{message}");
    }
    // the same errors as over TCP, including the line-length limit
    let error = c.request("meas:volt?").await;
    assert!(error.starts_with("error_meas:volt? "), "{error}");
    let error = c
        .request(&format!("change t1:target {}", "1".repeat(2000)))
        .await;
    assert!(error.contains("message too long"), "{error}");
}

#[tokio::test]
async fn ping_and_close() {
    let n = start();
    let mut c = WsClient::connect(&n).await;
    c.send_frame(0x89, b"hi").await;
    assert_eq!(c.frame().await, (0xA, b"hi".to_vec()));
    c.send_frame(0x88, &1000u16.to_be_bytes()).await;
    assert_eq!(c.frame().await, (0x8, 1000u16.to_be_bytes().to_vec()));
    let mut rest = Vec::new();
    let closed = tokio::time::timeout(Duration::from_secs(5), c.stream.read_to_end(&mut rest));
    assert!(closed.await.is_ok(), "the node closes the connection");
}

#[tokio::test]
async fn unmasked_frame_closes_with_protocol_error() {
    let n = start();
    let mut c = WsClient::connect(&n).await;
    c.stream.write_all(&[0x81, 2, b'h', b'i']).await.unwrap();
    assert_eq!(c.frame().await, (0x8, 1002u16.to_be_bytes().to_vec()));
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
