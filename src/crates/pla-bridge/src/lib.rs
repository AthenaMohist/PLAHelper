//! pla-bridge: WebSocket 桥接层（连接、协议、关联、重连、会话表）。

pub mod connection;
pub mod error;
pub mod protocol;
pub mod reconnect;
pub mod request_id;
pub mod session_table;

pub use connection::{BridgeClient, BridgeConfig};
pub use error::BridgeError;

/// 占位：桥接层 crate 已就位。
pub fn bridge_ready() -> bool {
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_bridge_scaffold() {
        assert!(super::bridge_ready());
    }
}
