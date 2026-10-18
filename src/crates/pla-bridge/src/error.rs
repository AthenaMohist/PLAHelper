//! Error type for the bridge crate.

use thiserror::Error;

/// Errors surfaced by the bridge client.
#[derive(Debug, Error, Clone)]
pub enum BridgeError {
    /// Underlying WebSocket / transport failure.
    #[error("websocket error: {0}")]
    Ws(String),
    /// Connection was closed (or is not established yet).
    #[error("connection closed")]
    ConnectionClosed,
    /// Request timed out waiting for a correlated response.
    #[error("request timeout after {ms}ms (id {id})")]
    Timeout { ms: u64, id: u64 },
    /// Server rejected the request (ok=false response).
    #[error("server error: {0}")]
    Server(String),
    /// Server returned a non-200 embedded result code (official binary protocol).
    #[error("server code {code} on {op}")]
    ServerCode { op: &'static str, code: u32 },
    /// Wire message could not be (de)serialized.
    #[error("codec error: {0}")]
    Codec(String),
    /// A duplicate request id was registered.
    #[error("duplicate request id {0}")]
    DuplicateId(u64),
    /// Internal invariant violation (e.g. poisoned lock).
    #[error("internal error: {0}")]
    Internal(String),
}
