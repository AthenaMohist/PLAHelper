//! Request id allocation and pending-request correlation.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::Value;
use tokio::sync::oneshot;

use crate::error::BridgeError;

/// Allocates monotonically unique request ids.
#[derive(Debug, Default)]
pub struct IdAllocator {
    next: AtomicU64,
}

impl IdAllocator {
    pub fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }

    pub fn allocate(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }
}

/// Result delivered to a pending request's waiter.
pub type ReplyResult = Result<Value, BridgeError>;

/// Map of pending request id -> oneshot responder.
#[derive(Debug, Default)]
pub struct PendingMap {
    inner: Mutex<HashMap<u64, oneshot::Sender<ReplyResult>>>,
}

impl PendingMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a responder for `id`, returning its receiver.
    pub fn register(
        &self,
        id: u64,
    ) -> Result<oneshot::Receiver<ReplyResult>, BridgeError> {
        let (tx, rx) = oneshot::channel();
        let mut map = self
            .inner
            .lock()
            .map_err(|_| BridgeError::Internal("pending map poisoned".into()))?;
        if map.insert(id, tx).is_some() {
            return Err(BridgeError::DuplicateId(id));
        }
        Ok(rx)
    }

    /// Completes a pending request; returns false when nobody waits on `id`.
    pub fn complete(&self, id: u64, result: ReplyResult) -> bool {
        let sender = match self.inner.lock() {
            Ok(mut map) => map.remove(&id),
            Err(_) => None,
        };
        match sender {
            Some(tx) => tx.send(result).is_ok(),
            None => false,
        }
    }

    /// Fails every pending request (used on connection loss).
    pub fn fail_all(&self, err: BridgeError) {
        let senders = match self.inner.lock() {
            Ok(mut map) => map.drain().collect::<Vec<_>>(),
            Err(_) => return,
        };
        for (_, tx) in senders {
            let _ = tx.send(Err(err.clone()));
        }
    }

    /// Number of currently pending requests.
    pub fn len(&self) -> usize {
        self.inner.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn allocate_is_unique_and_monotonic() {
        let alloc = IdAllocator::new();
        let a = alloc.allocate();
        let b = alloc.allocate();
        assert_eq!(a + 1, b);
    }

    #[tokio::test]
    async fn register_complete_routes_result() {
        let map = PendingMap::new();
        let rx = map.register(1).unwrap_or_else(|e| panic!("{e}"));
        assert!(map.complete(1, Ok(Value::Null)));
        let got = rx.await.unwrap_or_else(|e| panic!("{e}"));
        assert!(got.is_ok());
    }

    #[tokio::test]
    async fn fail_all_fails_pending() {
        let map = PendingMap::new();
        let rx = map.register(2).unwrap_or_else(|e| panic!("{e}"));
        map.fail_all(BridgeError::ConnectionClosed);
        assert!(rx.await.unwrap_or_else(|e| panic!("{e}")).is_err());
    }
}
