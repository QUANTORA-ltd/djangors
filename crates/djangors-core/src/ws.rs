//! RFC 6455 WebSocket upgrade handshake, frame protocol, and broadcast channels.
//!
//! Available when the `ws` feature is enabled (on by default). Provides zero-heavy-dependency
//! RFC 6455 WebSocket framing (`Text`, `Binary`, `Ping`, `Pong`, `Close`) over
//! [`hyper::upgrade::Upgraded`], plus [`Router::ws`](crate::Router::ws) for ergonomic
//! route registration.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use base64::Engine;
use hyper::http::StatusCode;
use hyper::upgrade::OnUpgrade;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::DjangorsError;
use crate::path_params::PathParams;
use crate::request::Request;
use crate::response::Response;

const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const OP_CONTINUATION: u8 = 0x0;
const OP_TEXT: u8 = 0x1;
const OP_BINARY: u8 = 0x2;
const OP_CLOSE: u8 = 0x8;
const OP_PING: u8 = 0x9;
const OP_PONG: u8 = 0xA;

/// Extension attached to [`Request`] containing Hyper's pending HTTP upgrade handle.
#[derive(Clone)]
pub(crate) struct UpgradeSlot(pub Arc<Mutex<Option<OnUpgrade>>>);

/// A single RFC 6455 WebSocket message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsMessage {
    /// A UTF-8 text frame (`opcode = 0x1`).
    Text(String),
    /// A binary payload frame (`opcode = 0x2`).
    Binary(Vec<u8>),
    /// A ping control frame (`opcode = 0x9`).
    Ping(Vec<u8>),
    /// A pong control frame (`opcode = 0xA`).
    Pong(Vec<u8>),
    /// A connection close control frame (`opcode = 0x8`).
    Close,
}

/// An upgraded full-duplex RFC 6455 WebSocket connection.
pub struct WebSocket<IO = hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>> {
    io: IO,
}

impl<IO> WebSocket<IO>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    /// Wraps an already-upgraded async stream as a [`WebSocket`].
    pub fn from_raw(io: IO) -> Self {
        Self { io }
    }

    /// Reads the next [`WsMessage`] from the peer.
    ///
    /// Reassembles fragmented messages, automatically responds to incoming `Ping`
    /// frames with a matching `Pong` frame, and returns `Ok(None)` when the peer
    /// closes the connection cleanly or sends a `Close` frame. Protocol violations
    /// (unmasked client frames, oversized or fragmented control frames, unknown
    /// opcodes, stray continuation frames) close the connection with an error.
    pub async fn recv(&mut self) -> Result<Option<WsMessage>, DjangorsError> {
        // (opcode of the first fragment, accumulated payload)
        let mut partial: Option<(u8, Vec<u8>)> = None;
        loop {
            let frame = match read_frame(&mut self.io).await? {
                None => return Ok(None),
                Some(frame) => frame,
            };
            if !frame.masked {
                return self.fail("client frames must be masked").await;
            }
            match frame.opcode {
                OP_CLOSE | OP_PING | OP_PONG => {
                    if !frame.fin || frame.payload.len() > 125 {
                        return self.fail("invalid control frame").await;
                    }
                    match frame.opcode {
                        OP_CLOSE => {
                            let _ = write_frame(&mut self.io, OP_CLOSE, &[]).await;
                            return Ok(None);
                        }
                        OP_PING => {
                            write_frame(&mut self.io, OP_PONG, &frame.payload).await?;
                            if partial.is_none() {
                                return Ok(Some(WsMessage::Ping(frame.payload)));
                            }
                        }
                        _ => {
                            if partial.is_none() {
                                return Ok(Some(WsMessage::Pong(frame.payload)));
                            }
                        }
                    }
                }
                OP_TEXT | OP_BINARY => {
                    if partial.is_some() {
                        return self
                            .fail("new data frame before previous message finished")
                            .await;
                    }
                    if frame.fin {
                        return finish_message(frame.opcode, frame.payload).map(Some);
                    }
                    partial = Some((frame.opcode, frame.payload));
                }
                OP_CONTINUATION => {
                    let Some((opcode, mut buf)) = partial.take() else {
                        return self.fail("continuation frame without a message").await;
                    };
                    if buf.len() + frame.payload.len() > MAX_MESSAGE_BYTES {
                        return self.fail("websocket message exceeds 16 MiB limit").await;
                    }
                    buf.extend_from_slice(&frame.payload);
                    if frame.fin {
                        return finish_message(opcode, buf).map(Some);
                    }
                    partial = Some((opcode, buf));
                }
                _ => return self.fail("unknown websocket opcode").await,
            }
        }
    }

    async fn fail<T>(&mut self, reason: &str) -> Result<T, DjangorsError> {
        // 1002 = protocol error.
        let _ = write_frame(&mut self.io, OP_CLOSE, &1002u16.to_be_bytes()).await;
        Err(DjangorsError::BadRequest(format!(
            "websocket protocol error: {reason}"
        )))
    }

    /// Sends a [`WsMessage`] to the connected client.
    pub async fn send(&mut self, msg: WsMessage) -> Result<(), DjangorsError> {
        match msg {
            WsMessage::Text(text) => write_frame(&mut self.io, 0x1, text.as_bytes()).await,
            WsMessage::Binary(bin) => write_frame(&mut self.io, 0x2, &bin).await,
            WsMessage::Ping(data) => write_frame(&mut self.io, 0x9, &data).await,
            WsMessage::Pong(data) => write_frame(&mut self.io, 0xA, &data).await,
            WsMessage::Close => write_frame(&mut self.io, 0x8, &[]).await,
        }
    }

    /// Sends a UTF-8 text message (`WsMessage::Text`) to the connected client.
    pub async fn send_text(&mut self, text: impl AsRef<str>) -> Result<(), DjangorsError> {
        write_frame(&mut self.io, 0x1, text.as_ref().as_bytes()).await
    }

    /// Sends a binary message (`WsMessage::Binary`) to the connected client.
    pub async fn send_binary(&mut self, bytes: &[u8]) -> Result<(), DjangorsError> {
        write_frame(&mut self.io, 0x2, bytes).await
    }

    /// Sends a close frame and flushes the stream.
    pub async fn close(&mut self) -> Result<(), DjangorsError> {
        write_frame(&mut self.io, 0x8, &[]).await
    }
}

/// Trait for async functions that handle an upgraded [`WebSocket`] connection.
pub trait WsHandler: Send + Sync {
    /// Invoked once the HTTP `101 Switching Protocols` handshake completes.
    fn call(
        &self,
        ws: WebSocket,
        req: Request,
        params: PathParams,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

impl<F, Fut> WsHandler for F
where
    F: Fn(WebSocket, Request, PathParams) -> Fut + Send + Sync,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn call(
        &self,
        ws: WebSocket,
        req: Request,
        params: PathParams,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(self(ws, req, params))
    }
}

/// Which browser origins may open a WebSocket.
///
/// Browsers send cookies on cross-site WebSocket handshakes and CSRF middleware does not
/// run on `GET`, so accepting any origin lets another site act as the logged-in user
/// (cross-site WebSocket hijacking). Handshakes without an `Origin` header (non-browser
/// clients) are allowed under every policy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum OriginPolicy {
    /// The `Origin` must match the request `Host` (the default).
    #[default]
    SameOrigin,
    /// Same origin, or one of these exact origins (e.g. `"https://app.example.com"`).
    Allow(Vec<String>),
    /// Accept any origin. Only for sockets that expose nothing user-specific.
    Any,
}

impl OriginPolicy {
    fn permits(&self, origin: &str, host: &str) -> bool {
        match self {
            OriginPolicy::Any => true,
            OriginPolicy::SameOrigin => origin_matches_host(origin, host),
            OriginPolicy::Allow(list) => {
                origin_matches_host(origin, host)
                    || list
                        .iter()
                        .any(|o| o.trim_end_matches('/').eq_ignore_ascii_case(origin))
            }
        }
    }
}

/// Validates an incoming WebSocket handshake request under [`OriginPolicy::SameOrigin`],
/// spawns `on_upgrade` once the `101 Switching Protocols` response is sent, and returns
/// the handshake [`Response`].
pub fn upgrade_websocket<F, Fut>(
    req: &Request,
    params: PathParams,
    on_upgrade: F,
) -> Result<Response, DjangorsError>
where
    F: FnOnce(WebSocket, Request, PathParams) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    upgrade_websocket_with_policy(req, params, &OriginPolicy::SameOrigin, on_upgrade)
}

/// Like [`upgrade_websocket`], with an explicit [`OriginPolicy`].
pub fn upgrade_websocket_with_policy<F, Fut>(
    req: &Request,
    params: PathParams,
    policy: &OriginPolicy,
    on_upgrade: F,
) -> Result<Response, DjangorsError>
where
    F: FnOnce(WebSocket, Request, PathParams) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let upgrade_hdr = req
        .header("upgrade")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !upgrade_hdr.eq_ignore_ascii_case("websocket") {
        return Err(DjangorsError::BadRequest(
            "missing or invalid Upgrade: websocket header".to_string(),
        ));
    }

    let version = req
        .header("sec-websocket-version")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if version.trim() != "13" {
        return Err(DjangorsError::BadRequest(
            "unsupported Sec-WebSocket-Version (expected 13)".to_string(),
        ));
    }

    if let Some(origin) = req.header("origin").and_then(|v| v.to_str().ok()) {
        let host = req
            .header("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !policy.permits(origin, host) {
            return Err(DjangorsError::Forbidden(
                "cross-origin WebSocket handshake rejected".to_string(),
            ));
        }
    }

    let ws_key = req
        .header("sec-websocket-key")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| DjangorsError::BadRequest("missing Sec-WebSocket-Key header".to_string()))?;

    let accept_val = compute_accept_key(ws_key);

    let on_upgrade_fut = req
        .ext::<UpgradeSlot>()
        .and_then(|slot| slot.0.lock().ok().and_then(|mut g| g.take()));

    // Without hyper's upgrade handle (for example a request dispatched in-process rather
    // than over an HTTP/1.1 connection) a 101 would leave the client waiting forever.
    let Some(upgrade_handle) = on_upgrade_fut else {
        return Err(DjangorsError::BadRequest(
            "this connection cannot be upgraded to a WebSocket".to_string(),
        ));
    };
    let req_clone = req.clone();
    tokio::spawn(async move {
        if let Ok(upgraded) = upgrade_handle.await {
            let io = hyper_util::rt::TokioIo::new(upgraded);
            let ws = WebSocket::from_raw(io);
            on_upgrade(ws, req_clone, params).await;
        }
    });

    let resp = Response::new(StatusCode::SWITCHING_PROTOCOLS)
        .header("upgrade", "websocket")
        .header("connection", "Upgrade")
        .header("sec-websocket-accept", &accept_val);
    Ok(resp)
}

/// Whether a handshake `Origin` (`scheme://host[:port]`) names the same host and port
/// as the request `Host` header.
fn origin_matches_host(origin: &str, host: &str) -> bool {
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    let origin_authority = rest.split('/').next().unwrap_or("");
    let default_port = match scheme.to_ascii_lowercase().as_str() {
        "http" | "ws" => "80",
        "https" | "wss" => "443",
        _ => return false,
    };
    let normalize = |authority: &str| -> String {
        let authority = authority.trim().to_ascii_lowercase();
        let has_port = match authority.rfind(':') {
            Some(i) => {
                !authority.ends_with(']') && authority[i + 1..].chars().all(|c| c.is_ascii_digit())
            }
            None => false,
        };
        if has_port {
            authority
        } else {
            format!("{authority}:{default_port}")
        }
    };
    !host.is_empty() && normalize(origin_authority) == normalize(host)
}

/// Computes the RFC 6455 `Sec-WebSocket-Accept` header value for a client `Sec-WebSocket-Key`.
pub fn compute_accept_key(client_key: &str) -> String {
    let mut input = Vec::with_capacity(client_key.trim().len() + WS_GUID.len());
    input.extend_from_slice(client_key.trim().as_bytes());
    input.extend_from_slice(WS_GUID);
    let digest = sha1_digest(&input);
    base64::engine::general_purpose::STANDARD.encode(digest)
}

/// Minimal, self-contained FIPS 180-4 SHA-1 implementation used strictly for RFC 6455
/// `Sec-WebSocket-Accept` handshake key derivation.
// `unknown_lints`: this lint only exists in newer clippy; CI pins an older toolchain.
#[allow(unknown_lints, clippy::chunks_exact_to_as_chunks)]
fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;

        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

struct Frame {
    fin: bool,
    masked: bool,
    opcode: u8,
    payload: Vec<u8>,
}

fn finish_message(opcode: u8, payload: Vec<u8>) -> Result<WsMessage, DjangorsError> {
    if opcode == OP_TEXT {
        String::from_utf8(payload)
            .map(WsMessage::Text)
            .map_err(|e| {
                DjangorsError::BadRequest(format!("invalid UTF-8 in websocket text frame: {e}"))
            })
    } else {
        Ok(WsMessage::Binary(payload))
    }
}

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Option<Frame>, DjangorsError> {
    let mut hdr = [0u8; 2];
    match reader.read_exact(&mut hdr).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => {
            return Err(DjangorsError::Internal(format!(
                "websocket read error: {e}"
            )))
        }
    }

    let fin = (hdr[0] & 0x80) != 0;
    if (hdr[0] & 0x70) != 0 {
        return Err(DjangorsError::BadRequest(
            "websocket frame uses reserved bits without a negotiated extension".to_string(),
        ));
    }
    let opcode = hdr[0] & 0x0F;
    let masked = (hdr[1] & 0x80) != 0;
    let mut len = u64::from(hdr[1] & 0x7F);

    if len == 126 {
        let mut ext = [0u8; 2];
        reader
            .read_exact(&mut ext)
            .await
            .map_err(|e| DjangorsError::Internal(format!("websocket length error: {e}")))?;
        len = u64::from(u16::from_be_bytes(ext));
    } else if len == 127 {
        let mut ext = [0u8; 8];
        reader
            .read_exact(&mut ext)
            .await
            .map_err(|e| DjangorsError::Internal(format!("websocket length error: {e}")))?;
        len = u64::from_be_bytes(ext);
    }

    if len > MAX_MESSAGE_BYTES as u64 {
        return Err(DjangorsError::BadRequest(
            "websocket frame exceeds 16 MiB limit".to_string(),
        ));
    }
    let len = len as usize;

    let mut mask_key = [0u8; 4];
    if masked {
        reader
            .read_exact(&mut mask_key)
            .await
            .map_err(|e| DjangorsError::Internal(format!("websocket mask error: {e}")))?;
    }

    let mut payload = vec![0u8; len];
    if len > 0 {
        reader
            .read_exact(&mut payload)
            .await
            .map_err(|e| DjangorsError::Internal(format!("websocket payload error: {e}")))?;
    }

    if masked {
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask_key[i & 3];
        }
    }

    Ok(Some(Frame {
        fin,
        masked,
        opcode,
        payload,
    }))
}

async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    opcode: u8,
    payload: &[u8],
) -> Result<(), DjangorsError> {
    let mut header = Vec::with_capacity(10);
    header.push(0x80 | (opcode & 0x0F)); // FIN = 1, server frames are unmasked

    let len = payload.len();
    if len < 126 {
        header.push(len as u8);
    } else if len <= u16::MAX as usize {
        header.push(126);
        header.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        header.push(127);
        header.extend_from_slice(&(len as u64).to_be_bytes());
    }

    writer
        .write_all(&header)
        .await
        .map_err(|e| DjangorsError::Internal(format!("websocket write header error: {e}")))?;
    if !payload.is_empty() {
        writer
            .write_all(payload)
            .await
            .map_err(|e| DjangorsError::Internal(format!("websocket write payload error: {e}")))?;
    }
    writer
        .flush()
        .await
        .map_err(|e| DjangorsError::Internal(format!("websocket flush error: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masked_frame(first_byte: u8, payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() < 126);
        let mask = [0x12, 0x34, 0x56, 0x78];
        let mut frame = vec![first_byte, 0x80 | payload.len() as u8];
        frame.extend_from_slice(&mask);
        for (i, &b) in payload.iter().enumerate() {
            frame.push(b ^ mask[i & 3]);
        }
        frame
    }

    #[test]
    fn rfc6455_accept_key_matches_spec_vector() {
        // RFC 6455 Section 1.3 test vector
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        assert_eq!(compute_accept_key(key), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn origin_must_match_host() {
        assert!(origin_matches_host(
            "http://localhost:8000",
            "localhost:8000"
        ));
        assert!(origin_matches_host("https://example.com", "example.com"));
        assert!(origin_matches_host(
            "https://Example.com",
            "example.com:443"
        ));
        assert!(!origin_matches_host("https://evil.com", "example.com"));
        assert!(!origin_matches_host(
            "http://example.com",
            "example.com:8000"
        ));
        assert!(!origin_matches_host("null", "example.com"));
        assert!(!origin_matches_host("http://example.com", ""));
    }

    #[tokio::test]
    async fn websocket_framing_round_trip_over_duplex_stream() {
        let (client_io, server_io) = tokio::io::duplex(4096);
        let mut server_ws = WebSocket::from_raw(server_io);
        let mut client_io = client_io;

        server_ws.send_text("hello live reload").await.unwrap();
        let frame = read_frame(&mut client_io).await.unwrap().unwrap();
        assert!(frame.fin && !frame.masked && frame.opcode == OP_TEXT);
        assert_eq!(frame.payload, b"hello live reload");

        client_io
            .write_all(&masked_frame(0x81, b"ping!"))
            .await
            .unwrap();
        let received = server_ws.recv().await.unwrap();
        assert_eq!(received, Some(WsMessage::Text("ping!".to_string())));
    }

    #[tokio::test]
    async fn fragmented_message_with_interleaved_ping_is_reassembled() {
        let (mut client_io, server_io) = tokio::io::duplex(4096);
        let mut server_ws = WebSocket::from_raw(server_io);

        client_io
            .write_all(&masked_frame(0x01, b"hel"))
            .await
            .unwrap(); // text, FIN=0
        client_io
            .write_all(&masked_frame(0x89, b"p"))
            .await
            .unwrap(); // ping
        client_io
            .write_all(&masked_frame(0x80, b"lo"))
            .await
            .unwrap(); // continuation, FIN=1

        assert_eq!(
            server_ws.recv().await.unwrap(),
            Some(WsMessage::Text("hello".to_string()))
        );
        let pong = read_frame(&mut client_io).await.unwrap().unwrap();
        assert_eq!((pong.opcode, pong.payload.as_slice()), (OP_PONG, &b"p"[..]));
    }

    #[tokio::test]
    async fn unmasked_client_frame_is_a_protocol_error() {
        let (mut client_io, server_io) = tokio::io::duplex(4096);
        let mut server_ws = WebSocket::from_raw(server_io);
        client_io
            .write_all(&[0x81, 0x02, b'h', b'i'])
            .await
            .unwrap();
        assert!(server_ws.recv().await.is_err());
        let close = read_frame(&mut client_io).await.unwrap().unwrap();
        assert_eq!(close.opcode, OP_CLOSE);
        assert_eq!(close.payload, 1002u16.to_be_bytes());
    }

    #[tokio::test]
    async fn stray_continuation_frame_is_a_protocol_error() {
        let (mut client_io, server_io) = tokio::io::duplex(4096);
        let mut server_ws = WebSocket::from_raw(server_io);
        client_io
            .write_all(&masked_frame(0x80, b"x"))
            .await
            .unwrap();
        assert!(server_ws.recv().await.is_err());
    }

    /// Full handshake over a real TCP connection served by `Djangors::serve`.
    #[tokio::test]
    async fn router_ws_end_to_end_handshake_echo_and_origin_check() {
        use tokio::io::AsyncBufReadExt;

        let router = crate::Router::new().ws(
            "/echo",
            |mut ws: WebSocket, _req: Request, _params: PathParams| async move {
                while let Ok(Some(msg)) = ws.recv().await {
                    if let WsMessage::Text(t) = msg {
                        if ws.send_text(format!("echo:{t}")).await.is_err() {
                            break;
                        }
                    }
                }
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = crate::Djangors::new(crate::DjangorsSettings::default(), router)
                .serve(listener)
                .await;
        });

        async fn handshake(
            addr: std::net::SocketAddr,
            origin: &str,
        ) -> (String, tokio::net::TcpStream) {
            let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
            let host = addr.to_string();
            let req = format!(
                "GET /echo HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
                 Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\
                 Origin: {origin}\r\n\r\n"
            );
            stream.write_all(req.as_bytes()).await.unwrap();
            let mut reader = tokio::io::BufReader::new(stream);
            let mut head = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap() == 0 || line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            (head, reader.into_inner())
        }

        let (head, _) = handshake(addr, "http://evil.example").await;
        assert!(head.starts_with("HTTP/1.1 403"), "{head}");

        let (head, mut stream) = handshake(addr, &format!("http://{addr}")).await;
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        assert!(head
            .to_ascii_lowercase()
            .contains("sec-websocket-accept: s3pplmbitxaq9kygzzhzrbk+xoo="));

        stream.write_all(&masked_frame(0x81, b"hi")).await.unwrap();
        let reply =
            tokio::time::timeout(std::time::Duration::from_secs(5), read_frame(&mut stream))
                .await
                .expect("echo reply")
                .unwrap()
                .unwrap();
        assert_eq!(reply.payload, b"echo:hi");
    }
}
