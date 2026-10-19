//! Async bridge client — official binary wire protocol.
//!
//! Per PLA-004 G1 probe (docs/protocol-probe/capture-20261018/): the server
//! speaks binary frames only; the client initiates the HANDSHAKE; SUBSCRIBE_ALL
//! is a subscription (not a ping) and there is no application-layer heartbeat —
//! dead-link detection relies on the TCP layer + supervisor backoff reconnect.
//! Text frames are logged and dropped (reference-implementation semantics).

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, watch};
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::error::BridgeError;
use crate::protocol::{
    self, Frame, FramePayload, MessageType,
};
use crate::reconnect::Backoff;
use crate::request_id::{IdAllocator, PendingMap, ReplyResult};
use crate::session_table::{SessionTable, SharedSession};

/// Default bridge port on the device (probe-confirmed).
pub const DEFAULT_BRIDGE_PORT: u16 = 8980;

/// tungstenite max message size floor: real LIST_ALL responses hit 368KB;
/// keep a 4MB ceiling headroom (ARCH §8.3, reference MAX_SIZE=2^32-1 style).
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// Client tunables.
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// Per-request correlation timeout.
    pub request_timeout: Duration,
    /// Reconnect backoff policy (reset on successful handshake).
    pub backoff: Backoff,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(10),
            backoff: Backoff::default(),
        }
    }
}

/// State shared between the client handle and the connection/supervisor tasks.
struct Shared {
    cfg: BridgeConfig,
    ids: IdAllocator,
    pending: PendingMap,
    event_tx: broadcast::Sender<Frame>,
    /// Encoded binary frames produced by `send_request`; consumed by the
    /// active connection's writer arm.
    out_tx: broadcast::Sender<Vec<u8>>,
    alive_tx: watch::Sender<bool>,
    /// Permanent receiver kept in `Shared` so the watch channel never closes
    /// (same contract as the pre-rewrite design).
    alive_rx: watch::Receiver<bool>,
    sessions: SessionTable,
}

impl Shared {
    fn mark_alive(&self) {
        let _ = self.alive_tx.send(true);
    }

    fn mark_dead(&self) {
        let _ = self.alive_tx.send(false);
    }
}

/// Plasticity bridge WebSocket client (official binary protocol).
#[derive(Clone)]
pub struct BridgeClient {
    shared: Arc<Shared>,
    supervisor: Arc<tokio::task::JoinHandle<()>>,
}

impl BridgeClient {
    /// Connect to `ws_url` and spawn the supervisor (reconnect + handshake).
    /// The first connection attempt happens inside the supervisor; use
    /// `wait_alive` in callers that need to know when the link came up.
    pub fn connect(ws_url: &str) -> Result<Self, BridgeError> {
        Self::connect_with_config(ws_url, BridgeConfig::default())
    }

    pub fn connect_with_config(ws_url: &str, cfg: BridgeConfig) -> Result<Self, BridgeError> {
        let (event_tx, _) = broadcast::channel(1024);
        let (out_tx, out_rx) = broadcast::channel(256);
        let (alive_tx, alive_rx) = watch::channel(false);
        let shared = Arc::new(Shared {
            cfg,
            ids: IdAllocator::new(),
            pending: PendingMap::new(),
            event_tx,
            out_tx,
            alive_tx,
            alive_rx,
            sessions: SessionTable::new(),
        });
        let url = ws_url.to_string();
        let sup_shared = Arc::clone(&shared);
        let supervisor = tokio::spawn(supervisor_loop(sup_shared, url, out_rx));
        Ok(Self {
            shared,
            supervisor: Arc::new(supervisor),
        })
    }

    /// Wait until the link is alive (handshake completed), or fail after `within`.
    pub async fn wait_alive(&self, within: Duration) -> Result<(), BridgeError> {
        let mut rx = self.shared.alive_tx.subscribe();
        if *rx.borrow() {
            return Ok(());
        }
        tokio::time::timeout(within, async {
            loop {
                if rx.changed().await.is_err() {
                    return;
                }
                if *rx.borrow_and_update() {
                    return;
                }
            }
        })
        .await
        .map(|_| ())
        .map_err(|_| BridgeError::Timeout { ms: within.as_millis() as u64, id: 0 })
    }

    /// Current alive flag (handshake completed on the active connection).
    pub fn is_alive(&self) -> bool {
        *self.shared.alive_rx.borrow()
    }

    /// Broadcast receiver for every server-pushed event frame.
    pub fn events(&self) -> broadcast::Receiver<Frame> {
        self.shared.event_tx.subscribe()
    }

    /// Session for one open document (created on demand).
    pub fn session(&self, doc_key: &str) -> SharedSession {
        self.shared.sessions.get_or_create(doc_key)
    }

    /// Send SUBSCRIBE_ALL_1 now; resolves when the connection loop next
    /// observes the subscription is registered (no dedicated response frame —
    /// probe-confirmed semantics). Resolves Ok once the frame is on the wire.
    pub async fn subscribe_all(&self) -> Result<(), BridgeError> {
        let id = self.allocate_mid();
        let frame = protocol::request(MessageType::SubscribeAll, id);
        let bytes = protocol::encode(&frame).map_err(BridgeError::from)?;
        self.shared
            .out_tx
            .send(bytes)
            .map(|_| ())
            .map_err(|_| BridgeError::ConnectionClosed)
    }

    /// LIST_ALL for one document; resolves with the decoded JSON mirror.
    pub async fn list_all(&self, _doc_key: &str) -> Result<serde_json::Value, BridgeError> {
        // LIST_ALL_1 request is header-only on the wire (probe: 8 bytes).
        let frame = self.send_request(MessageType::ListAll).await?;
        protocol::list_response_to_value(&frame)
    }

    /// LIST_VISIBLE_1 (code 22): full-scene tree dump. Header-only request.
    /// Wire-verified 2026-10-20: response carries nested object records
    /// (tag=6 name blocks); single frame capped ≈368KB by the server —
    /// oversized scenes are TRUNCATED silently by the device.
    pub async fn list_visible(&self, _doc_key: &str) -> Result<serde_json::Value, BridgeError> {
        let frame = self.send_request(MessageType::ListVisible).await?;
        protocol::list_response_to_value(&frame)
    }

    /// PUT_SOME for one document.
    ///
    /// G2 scope: wire payload = filename + empty group/item tables — the
    /// exact shape the real device answered code=200 in the probe
    /// (frame-putsome-resp-0.bin). A non-empty `data` update body requires
    /// the full group/item encoder, which is G4 real-device scope (ARCH §7
    /// 卡 3); passing one here logs a warning and sends the empty probe
    /// shape rather than inventing an unverified layout.
    pub async fn put_some(
        &self,
        doc_key: &str,
        data: serde_json::Value,
    ) -> Result<serde_json::Value, BridgeError> {
        let non_empty = data
            .as_object()
            .map(|m| !m.is_empty())
            .unwrap_or(!data.is_null());
        if non_empty {
            tracing::warn!(
                "PUT_SOME update body deferred to G4 (empty-table probe shape sent)"
            );
        }
        let mut payload = Vec::with_capacity(64);
        protocol::put_string(&mut payload, doc_key);
        protocol::put_u32(&mut payload, 0); // group_count
        protocol::put_u32(&mut payload, 0); // item_count
        let frame = self.send_request_with(MessageType::PutSome, payload).await?;
        let code = protocol::response_code(&frame).unwrap_or(0);
        if code != 200 {
            return Err(BridgeError::ServerCode { op: "PUT_SOME", code });
        }
        Ok(serde_json::json!({ "code": code }))
    }

    fn allocate_mid(&self) -> u32 {
        // Protocol ids are u32; the allocator is u64 — truncate (collision
        // window only after 2^32 in-flight allocations, unreachable here).
        self.shared.ids.allocate() as u32
    }

    async fn send_request(&self, msg_type: MessageType) -> Result<Frame, BridgeError> {
        self.send_request_with(msg_type, Vec::new()).await
    }

    async fn send_request_with(
        &self,
        msg_type: MessageType,
        payload: Vec<u8>,
    ) -> Result<Frame, BridgeError> {
        let id = self.allocate_mid();
        let rx = self.shared.pending.register(u64::from(id))?;
        let mut frame = protocol::request(msg_type, id);
        frame.payload = if payload.is_empty() {
            FramePayload::Empty
        } else {
            FramePayload::Raw(payload)
        };
        let bytes = protocol::encode(&frame).map_err(BridgeError::from)?;
        self.shared
            .out_tx
            .send(bytes)
            .map_err(|_| BridgeError::ConnectionClosed)?;
        self.await_reply(id, rx).await
    }

    async fn await_reply(
        &self,
        id: u32,
        rx: tokio::sync::oneshot::Receiver<ReplyResult>,
    ) -> Result<Frame, BridgeError> {
        let timeout_ms = self.shared.cfg.request_timeout.as_millis() as u64;
        match tokio::time::timeout(self.shared.cfg.request_timeout, rx).await {
            Ok(Ok(result)) => result.map(|v| decode_reply(v, id)),
            Ok(Err(_)) => Err(BridgeError::ConnectionClosed), // responder dropped
            Err(_) => {
                self.shared.pending.complete(
                    u64::from(id),
                    Err(BridgeError::Timeout { ms: timeout_ms, id: u64::from(id) }),
                );
                Err(BridgeError::Timeout { ms: timeout_ms, id: u64::from(id) })
            }
        }
    }

    /// Abort the supervisor and tear the client down.
    pub fn shutdown(&self) {
        self.shared.mark_dead();
        self.shared.pending.fail_all(BridgeError::ConnectionClosed);
        self.supervisor.abort();
    }
}

/// Rehydrate a Frame from the Value stashed by the reader arm. The pending
/// map carries Value (ReplyResult contract shared with session_table); binary
/// frames are round-tripped through `encode` so this is lossless for
/// protocol-produced values.
fn decode_reply(v: serde_json::Value, id: u32) -> Frame {
    let bytes = v
        .get("__bytes")
        .and_then(|b| b.as_str())
        .and_then(|s| base64ish_decode(s))
        .unwrap_or_default();
    protocol::decode(&bytes).unwrap_or_else(|_| Frame {
        msg_type: MessageType::Unknown(u32::MAX),
        message_id: id,
        payload: FramePayload::Raw(Vec::new()),
    })
}

/// Hex decode (no external base64 dep; frames are small in the reply path).
fn base64ish_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Outcome of one connection run.
enum RunOutcome {
    /// Handshake completed (backoff resets after this).
    HandshakeOk,
    /// Never got to a working handshake.
    HandshakeFailed,
}

/// Supervisor: connect, handshake, run, backoff, repeat.
async fn supervisor_loop(shared: Arc<Shared>, url: String, mut out_rx: broadcast::Receiver<Vec<u8>>) {
    let mut backoff = shared.cfg.backoff.clone();
    loop {
        let outcome = run_connection(&shared, &url, &mut out_rx).await;
        let _ = shared.alive_tx.send(false);
        shared.pending.fail_all(BridgeError::ConnectionClosed);
        match outcome {
            RunOutcome::HandshakeOk => backoff.reset(),
            RunOutcome::HandshakeFailed => {}
        }
        let delay = backoff.next_delay();
        tracing::warn!(
            attempt = backoff.attempts(),
            delay_ms = delay.as_millis() as u64,
            "bridge connection lost; reconnecting"
        );
        sleep(delay).await;
    }
}

/// One connection lifetime: WS connect, client-initiated HANDSHAKE, then a
/// select loop over incoming binary frames and outgoing encoded frames.
async fn run_connection(
    shared: &Arc<Shared>,
    url: &str,
    out_rx: &mut broadcast::Receiver<Vec<u8>>,
) -> RunOutcome {
    // ARCH §8.3 / est §1.3: real LIST_ALL responses hit 368KB; make the WS
    // message ceiling explicit (≥4MB) instead of relying on the library
    // default — MAX_FRAME_BYTES is the single source of truth.
    let ws = match tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(url) {
        Ok(req) => req,
        Err(e) => {
            tracing::warn!(error = %e, "bridge request build failed");
            return RunOutcome::HandshakeFailed;
        }
    };
    let ws = match tokio_tungstenite::connect_async_with_config(
        ws,
        Some(WebSocketConfig {
            max_message_size: Some(MAX_FRAME_BYTES),
            ..WebSocketConfig::default()
        }),
        false,
    )
    .await
    {
        Ok((ws, _)) => ws,
        Err(e) => {
            tracing::warn!(error = %e, "bridge connect failed");
            return RunOutcome::HandshakeFailed;
        }
    };
    let (mut sink, mut stream) = ws.split();

    // Client speaks first: HANDSHAKE_1 with no payload (probe: server stays
    // silent until the client's 8-byte handshake arrives).
    let hs = protocol::request(MessageType::Handshake, 1);
    let hs_bytes = match protocol::encode(&hs) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = ?e, "HANDSHAKE encode failed");
            return RunOutcome::HandshakeFailed;
        }
    };
    if sink.send(Message::Binary(hs_bytes)).await.is_err() {
        return RunOutcome::HandshakeFailed;
    }

    // Await the HANDSHAKE response (mid echo + supported table).
    match stream.next().await {
        Some(Ok(Message::Binary(bytes))) => {
            match protocol::decode(&bytes) {
                Ok(Frame {
                    msg_type: MessageType::Handshake,
                    payload: FramePayload::Handshake { .. },
                    ..
                }) => {}
                Ok(other) => {
                    tracing::warn!(frame = ?other.msg_type, "expected HANDSHAKE response");
                    return RunOutcome::HandshakeFailed;
                }
                Err(e) => {
                    tracing::error!(error = ?e, "HANDSHAKE decode failed");
                    return RunOutcome::HandshakeFailed;
                }
            }
        }
        _ => return RunOutcome::HandshakeFailed,
    }

    // Handshake OK: mark alive and (re-)subscribe after reconnect.
    shared.mark_alive();
    let sub = protocol::request(MessageType::SubscribeAll, 2);
    if let Ok(b) = protocol::encode(&sub) {
        let _ = sink.send(Message::Binary(b)).await;
    }

    loop {
        tokio::select! {
            maybe_msg = stream.next() => {
                match maybe_msg {
                    Some(Ok(Message::Binary(bytes))) => {
                        if !handle_binary(shared, &bytes) {
                            break;
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        // Binary-protocol server; text frames are not part of
                        // the wire contract — log and drop.
                        tracing::debug!(len = text.len(), "dropping unexpected text frame");
                    }
                    Some(Ok(_)) => {} // ping/pong handled by tungstenite
                    Some(Err(e)) => {
                        tracing::debug!(error = %e, "bridge stream error");
                        break; // dead link; supervisor reconnects
                    }
                    None => break,
                }
            }
            maybe_out = out_rx.recv() => {
                match maybe_out {
                    Ok(bytes) => {
                        if sink.send(Message::Binary(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    RunOutcome::HandshakeOk
}

/// Dispatch one incoming binary frame. Returns false when the link must drop.
fn handle_binary(shared: &Arc<Shared>, bytes: &[u8]) -> bool {
    let frame = match protocol::decode(bytes) {
        Ok(f) => f,
        Err(e) => {
            tracing::debug!(error = ?e, "undecodable bridge frame");
            // [R5 返工] fail-loud 传播: 截断响应帧的 8 字节头(code+mid)仍
            // 完好 — 按帧头 mid 把 pending 等待方以 Codec 错误立即完成,
            // 否则 oneshot 永不解析, 用户只会看到笼统的 30s 超时而不是
            // 「声明/实收」截断详情(安全网失效)。mid 无主时 complete 是
            // 无害 no-op(PendingMap::complete 语义), 事件类坏帧不受影响。
            if bytes.len() >= 8 {
                let mid =
                    u64::from(u32::from_le_bytes(bytes[4..8].try_into().expect("8-byte slice")));
                shared.pending.complete(mid, Err(BridgeError::from(e)));
            }
            return true; // tolerate and continue; not a link-level failure
        }
    };
    match &frame.payload {
        // Server-pushed events (NEW_VERSION / NEW_FILE / ...): route to the
        // per-document session table, then fan out globally.
        FramePayload::Event { .. } => {
            shared.sessions.dispatch(&frame);
            let _ = shared.event_tx.send(frame);
            true
        }
        // Correlated responses complete pending waiters. The raw frame bytes
        // ride along (hex) so the waiter-side decode is lossless.
        _ => {
            let mid = u64::from(frame.message_id);
            let carried = serde_json::json!({
                "__bytes": hex_encode(bytes),
            });
            shared.pending.complete(mid, Ok(carried));
            true
        }
    }
}

// Keep the WebSocketStream type name in scope for documentation clarity.
type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
#[allow(dead_code)]
fn _ws_stream_type_check(_: Option<WsStream>) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal Shared for dispatch-level tests (no real connection involved).
    fn test_shared() -> Arc<Shared> {
        let (event_tx, _) = broadcast::channel(16);
        let (out_tx, _out_rx) = broadcast::channel(16);
        let (alive_tx, alive_rx) = watch::channel(false);
        Arc::new(Shared {
            cfg: BridgeConfig::default(),
            ids: IdAllocator::new(),
            pending: PendingMap::new(),
            event_tx,
            out_tx,
            alive_tx,
            alive_rx,
            sessions: SessionTable::new(),
        })
    }

    /// [R5 返工] 截断响应帧(声明 record len > 实际字节): 等待方必须立刻
    /// 收到 Codec 截断错误(含声明数字), 而不是饿死成 30s 超时。
    #[test]
    fn truncated_response_completes_pending_with_codec_error() {
        let shared = test_shared();
        let mut rx = shared.pending.register(7).expect("register waiter");

        let mut bytes = Vec::new();
        protocol::put_u32(&mut bytes, MessageType::ListVisible.code());
        protocol::put_u32(&mut bytes, 7); // mid — waiter correlation id
        protocol::put_u32(&mut bytes, 200);
        protocol::put_string(&mut bytes, "model1.plasticity");
        protocol::put_u32(&mut bytes, 0); // version
        protocol::put_u32(&mut bytes, 1); // object count
        protocol::put_u32(&mut bytes, 1000); // declared record length
        bytes.extend_from_slice(&[0u8; 8]); // far short of 1000 → truncated

        assert!(handle_binary(&shared, &bytes));

        let result = rx
            .try_recv()
            .expect("oneshot must resolve immediately on truncation");
        match result {
            Err(BridgeError::Codec(msg)) => {
                assert!(msg.contains("截断"), "got: {msg}");
                assert!(msg.contains("声明 1000B"), "got: {msg}");
            }
            other => panic!("expected Codec truncation error, got: {other:?}"),
        }
    }

    /// 无主 mid 的坏帧: complete 走 no-op, 链路不掉线、pending 不受污染。
    #[test]
    fn undecodable_frame_with_unknown_mid_is_tolerated() {
        let shared = test_shared();
        let mut bytes = Vec::new();
        protocol::put_u32(&mut bytes, MessageType::ListVisible.code());
        protocol::put_u32(&mut bytes, 9999); // nobody waits on this mid
        bytes.extend_from_slice(&[0xff; 16]);
        assert!(handle_binary(&shared, &bytes));
        assert!(shared.pending.is_empty());
    }
}
