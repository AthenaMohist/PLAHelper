//! doc_key -> session registry. Every open document (identified by the
//! filename carried in bridge events) gets its own session entry with an
//! isolated event channel and pending-request map.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use crate::protocol::{Frame, FramePayload, MessageType};
use crate::request_id::PendingMap;

/// Capacity of a per-session event channel.
const SESSION_EVENT_CAPACITY: usize = 128;

/// One open document's session.
#[derive(Debug)]
pub struct Session {
    doc_key: String,
    event_tx: broadcast::Sender<Frame>,
    pending: PendingMap,
}

impl Session {
    pub fn new(doc_key: impl Into<String>) -> Self {
        let (event_tx, _) = broadcast::channel(SESSION_EVENT_CAPACITY);
        Self {
            doc_key: doc_key.into(),
            event_tx,
            pending: PendingMap::new(),
        }
    }

    pub fn doc_key(&self) -> &str {
        &self.doc_key
    }

    /// New subscriber for this document's events.
    pub fn events(&self) -> broadcast::Receiver<Frame> {
        self.event_tx.subscribe()
    }

    /// Per-session pending-request map (isolated from other documents).
    pub fn pending(&self) -> &PendingMap {
        &self.pending
    }
}

/// Registry of open-document sessions.
#[derive(Debug, Default)]
pub struct SessionTable {
    inner: Mutex<HashMap<String, Arc<Session>>>,
}

impl SessionTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create the session for `doc_key`.
    pub fn get_or_create(&self, doc_key: &str) -> Arc<Session> {
        let mut map = self.lock();
        map.entry(doc_key.to_string())
            .or_insert_with(|| Arc::new(Session::new(doc_key)))
            .clone()
    }

    /// Get an existing session, if the document is open.
    pub fn get(&self, doc_key: &str) -> Option<Arc<Session>> {
        self.lock().get(doc_key).cloned()
    }

    /// Remove a session (document closed).
    pub fn remove(&self, doc_key: &str) -> Option<Arc<Session>> {
        self.lock().remove(doc_key)
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Route a bridge event to the right session. Returns true when the
    /// event was routed to a session (which may have zero subscribers —
    /// a normal state for a freshly created session).
    ///
    /// Binary protocol: the event's document is identified by the filename
    /// carried inside the frame payload when decodable; frames whose payload
    /// carries no filename are global and not routed (returns false).
    /// NEW_FILE events create the session; other events need it to exist.
    pub fn dispatch(&self, frame: &Frame) -> bool {
        let FramePayload::Event { body } = &frame.payload else {
            return false;
        };
        let Some(doc_key) = event_filename(body) else {
            return false;
        };
        let is_new_file = frame.msg_type == MessageType::NewFile;
        let session = if is_new_file {
            self.get_or_create(doc_key)
        } else {
            match self.get(doc_key) {
                Some(s) => s,
                None => return false,
            }
        };
        let _ = session.event_tx.send(frame.clone());
        true
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Session>>> {
        // Poisoning is not recoverable here; a fresh empty map keeps the API
        // infallible. This only happens after a panic while holding the lock.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Convenience alias used by `BridgeClient::session`.
pub type SharedSession = Arc<Session>;

/// Extract the document filename from an event body. Layout of
/// NEW_VERSION/NEW_FILE bodies is pending G4 real-device sampling (ARCH §8.1);
/// the filename is length-prefixed at the body start per the reference
/// implementation. Bodies that don't parse yield None (global event).
fn event_filename(body: &[u8]) -> Option<&str> {
    if body.len() < 4 {
        return None;
    }
    let len = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as usize;
    if len == 0 || 4 + len > body.len() {
        return None;
    }
    std::str::from_utf8(&body[4..4 + len]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_frame(msg_type: MessageType, body: &[u8]) -> Frame {
        Frame {
            msg_type,
            message_id: 1,
            payload: FramePayload::Event { body: body.to_vec() },
        }
    }

    fn filename_body(name: &str) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&(name.len() as u32).to_le_bytes());
        b.extend_from_slice(name.as_bytes());
        b
    }

    #[test]
    fn new_file_creates_and_routes() {
        let table = SessionTable::new();
        let f = event_frame(MessageType::NewFile, &filename_body("a.plasticity"));
        assert!(table.dispatch(&f));
        let f2 = event_frame(MessageType::NewFile, &filename_body("b.plasticity"));
        assert!(table.dispatch(&f2));
        assert_eq!(table.len(), 2);
    }

    #[tokio::test]
    async fn events_route_to_right_session_only() {
        let table = SessionTable::new();
        let a = table.get_or_create("a.plasticity");
        let b = table.get_or_create("b.plasticity");
        let mut a_rx = a.events();
        let mut b_rx = b.events();

        let ev = event_frame(MessageType::NewVersion, &filename_body("a.plasticity"));
        assert!(table.dispatch(&ev));
        let ev2 = event_frame(MessageType::NewVersion, &filename_body("b.plasticity"));
        assert!(table.dispatch(&ev2));

        let got_a = a_rx.recv().await.unwrap_or_else(|e| panic!("{e}"));
        let got_b = b_rx.recv().await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(got_a.msg_type, MessageType::NewVersion);
        assert_eq!(got_b.msg_type, MessageType::NewVersion);
        // b's channel must not have received a's event.
        assert!(b_rx.try_recv().is_err());
        assert!(a_rx.try_recv().is_err());
    }

    #[test]
    fn event_for_unknown_doc_is_ignored() {
        let table = SessionTable::new();
        let f = event_frame(MessageType::NewVersion, &filename_body("ghost.plasticity"));
        assert!(!table.dispatch(&f));
    }

    #[test]
    fn global_event_not_routed() {
        let table = SessionTable::new();
        // Unparseable body -> no filename -> global.
        let f = event_frame(MessageType::NewVersion, &[0xff, 0xff, 0xff, 0xff, 0x01]);
        assert!(!table.dispatch(&f));
    }

    #[test]
    fn non_event_frame_not_routed() {
        let table = SessionTable::new();
        let f = Frame {
            msg_type: MessageType::ListAll,
            message_id: 1,
            payload: FramePayload::Empty,
        };
        assert!(!table.dispatch(&f));
    }

    #[test]
    fn remove_closes_session() {
        let table = SessionTable::new();
        let _ = table.get_or_create("x.plasticity");
        assert!(table.remove("x.plasticity").is_some());
        assert!(table.get("x.plasticity").is_none());
        let f = event_frame(MessageType::NewVersion, &filename_body("x.plasticity"));
        assert!(!table.dispatch(&f));
    }
}
