// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP over WebSockets (SECoP transport, RFC 6455), server side: a connection whose
//! first bytes are `GET /` is upgraded with an HTTP handshake, and from then on every
//! SECoP message travels in a frame. [`tungstenite`](tokio_tungstenite::tungstenite)
//! checks the request, computes the accept key and handles the frames; this module only
//! tells the two transports apart and reads the request, so that it can be answered with
//! `400 Bad Request` when it is no valid upgrade.

use std::io::Cursor;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, Chain, Join};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::machine::TryParse;
use tokio_tungstenite::tungstenite::handshake::server::{Request, create_response, write_response};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Error, error::ProtocolError};

/// Environment variable that turns WebSockets off with `0`.
pub const WEBSOCKETS_VARIABLE: &str = "MUST_WEBSOCKETS";

/// The first bytes of a connection that asks for a WebSocket.
const PREFIX: &[u8] = b"GET /";

/// The longest HTTP request the handshake reads.
const MAX_REQUEST: usize = 8 * 1024;

/// A WebSocket connection after the handshake: the bytes the client sent after its
/// request come first, then the rest of the TCP stream.
pub(crate) type Socket =
    WebSocketStream<Join<Chain<Cursor<Vec<u8>>, OwnedReadHalf>, OwnedWriteHalf>>;

/// Whether `MUST_WEBSOCKETS` leaves WebSockets on.
pub(crate) fn enabled_by_environment() -> bool {
    std::env::var(WEBSOCKETS_VARIABLE).map_or(true, |v| v.trim() != "0")
}

/// What the first bytes of a connection say about it.
#[derive(Debug, PartialEq, Eq)]
enum Sniff {
    /// Too few bytes to tell.
    Undecided,
    /// A WebSocket handshake.
    WebSocket,
    /// SECoP over raw TCP.
    Tcp,
}

fn sniff(first: &[u8]) -> Sniff {
    if first.starts_with(PREFIX) {
        Sniff::WebSocket
    } else if PREFIX.starts_with(first) {
        Sniff::Undecided
    } else {
        Sniff::Tcp
    }
}

/// A connection, once its first bytes showed how it carries SECoP messages.
pub(crate) enum Opened {
    /// Raw TCP, and the bytes already read from it.
    Tcp(TcpStream, Vec<u8>),
    /// A WebSocket, after a successful handshake.
    WebSocket(Box<Socket>),
}

/// Reads the first bytes of `stream` and, if they start a WebSocket handshake, answers
/// it. `Err` says why the connection ends: it closed, or its handshake was refused (and
/// answered with `400 Bad Request`). Cancel safe, as the connection ends when cancelled.
pub(crate) async fn open(mut stream: TcpStream, max_line_length: usize) -> Result<Opened, String> {
    let mut first = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match sniff(&first) {
            Sniff::Tcp => return Ok(Opened::Tcp(stream, first)),
            Sniff::WebSocket => break,
            Sniff::Undecided => read_more(&mut stream, &mut buf, &mut first).await?,
        }
    }
    let (used, request) = loop {
        match Request::try_parse(&first) {
            Ok(Some(parsed)) => break parsed,
            Ok(None) if first.len() < MAX_REQUEST => {
                read_more(&mut stream, &mut buf, &mut first).await?;
            }
            Ok(None) => return Err(refuse(stream, "request too long").await),
            Err(e) => return Err(refuse(stream, &e.to_string()).await),
        }
    };
    let response = match create_response(&request) {
        Ok(response) => response,
        Err(e) => return Err(refuse(stream, &e.to_string()).await),
    };
    let mut bytes = Vec::new();
    write_response(&mut bytes, &response).map_err(|e| e.to_string())?;
    stream
        .write_all(&bytes)
        .await
        .map_err(|e| format!("write error: {e}"))?;
    // the line ending may follow the longest message
    let limit = Some(max_line_length.saturating_add(1));
    let config = WebSocketConfig::default()
        .max_message_size(limit)
        .max_frame_size(limit);
    let (reader, writer) = stream.into_split();
    let tail = Cursor::new(first.split_off(used));
    let socket = tokio::io::join(tail.chain(reader), writer);
    let socket = WebSocketStream::from_raw_socket(socket, Role::Server, Some(config)).await;
    Ok(Opened::WebSocket(Box::new(socket)))
}

/// Appends the next bytes of `stream` to `first`; `Err` if there are none.
async fn read_more(
    stream: &mut (impl AsyncRead + Unpin),
    buf: &mut [u8],
    first: &mut Vec<u8>,
) -> Result<(), String> {
    match stream.read(buf).await {
        Ok(0) => Err("closed by client".into()),
        Ok(n) => {
            first.extend_from_slice(&buf[..n]);
            Ok(())
        }
        Err(e) => Err(format!("read error: {e}")),
    }
}

/// Answers a request that is no valid upgrade with `400 Bad Request` and closes; returns
/// why the connection ends.
async fn refuse(mut stream: TcpStream, reason: &str) -> String {
    let response = format!(
        "HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Type: text/plain\r\n\
         Content-Length: {}\r\n\r\n{reason}",
        reason.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
    format!("WebSocket handshake refused: {reason}")
}

/// A close frame with `code` and `reason` (at most 123 bytes).
pub(crate) fn close_frame(code: CloseCode, reason: &'static str) -> CloseFrame {
    CloseFrame {
        code,
        reason: reason.into(),
    }
}

/// Why reading a WebSocket failed, and the close frame the node sends for it.
pub(crate) fn failure(error: &Error) -> (String, Option<CloseFrame>) {
    match error {
        Error::ConnectionClosed
        | Error::AlreadyClosed
        | Error::Protocol(ProtocolError::ResetWithoutClosingHandshake) => {
            ("closed by client".into(), None)
        }
        Error::Io(e) => (format!("read error: {e}"), None),
        Error::Capacity(_) => (
            "message too long".into(),
            Some(close_frame(CloseCode::Size, "message too long")),
        ),
        Error::Utf8(_) => (
            "WebSocket message is not UTF-8".into(),
            Some(close_frame(CloseCode::Invalid, "invalid UTF-8")),
        ),
        e => (
            format!("WebSocket protocol error: {e}"),
            Some(close_frame(CloseCode::Protocol, "protocol error")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffing() {
        assert_eq!(sniff(b""), Sniff::Undecided);
        assert_eq!(sniff(b"G"), Sniff::Undecided);
        assert_eq!(sniff(b"GET "), Sniff::Undecided);
        assert_eq!(sniff(b"GET /"), Sniff::WebSocket);
        assert_eq!(sniff(b"GET /secop HTTP/1.1"), Sniff::WebSocket);
        assert_eq!(sniff(b"*IDN?\n"), Sniff::Tcp);
        assert_eq!(sniff(b"GE\n"), Sniff::Tcp);
        assert_eq!(sniff(b"GET x"), Sniff::Tcp);
    }
}
