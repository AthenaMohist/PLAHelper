//! [PLA-000] bridge 常驻后台线程。
//!
//! 架构(Bug 1 修复): UI 线程不再每次点击新建 tokio runtime + 短连接(会卡 UI ≤5s),
//! 而是 spawn 一条常驻线程: 线程内建 current_thread runtime, 持有常驻 `BridgeClient`
//! (connect 即 spawn supervisor, 断线自动重连)。UI ↔ 线程用 std::sync::mpsc 通道,
//! UI 每帧 `poll()` 非阻塞收取事件。
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use serde_json::Value;

/// UI → bridge 线程命令。
#[derive(Debug)]
pub enum BridgeCmd {
    /// 拉取实体树(LIST_VISIBLE)。
    RefreshEntities,
    /// 退出线程(断开连接, abort supervisor)。
    Shutdown,
}

/// bridge 线程 → UI 事件。
#[derive(Debug)]
pub enum BridgeEvent {
    /// LIST_VISIBLE 拉到的实体列表。
    Entities(Vec<Value>),
    /// 连接状态: "已连接" / "连接中…" / "断线:<原因>"。
    Status(String),
    /// 单次操作失败(不改变连接状态)。
    Error(String),
}

/// bridge 线程句柄: UI 持有, 每帧 try_recv 非阻塞轮询。
pub struct BridgeHandle {
    tx: std::sync::mpsc::Sender<BridgeCmd>,
    rx: Receiver<BridgeEvent>,
    /// 实际使用的 ws 地址(设置面板展示)。
    pub url: String,
}

impl BridgeHandle {
    /// 解析 bridge 地址并启动常驻线程。
    pub fn spawn() -> Self {
        let url = bridge_url();
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<BridgeCmd>();
        let (ev_tx, ev_rx) = std::sync::mpsc::channel::<BridgeEvent>();
        let thread_url = url.clone();
        let ev_tx_thread = ev_tx.clone();
        let spawned = std::thread::Builder::new()
            .name("pla-bridge".into())
            .spawn(move || bridge_thread_main(thread_url, cmd_rx, ev_tx_thread));
        if spawned.is_err() {
            // 线程启动失败: 不 panic, UI 显示断线原因。
            let _ = ev_tx.send(BridgeEvent::Status("断线:bridge 线程启动失败".into()));
        }
        Self {
            tx: cmd_tx,
            rx: ev_rx,
            url,
        }
    }

    /// 发送命令(通道断开 = 线程已退出, 静默忽略)。
    pub fn send(&self, cmd: BridgeCmd) {
        let _ = self.tx.send(cmd);
    }

    /// 非阻塞收取一个事件(UI 每帧调用)。
    pub fn poll(&self) -> Option<BridgeEvent> {
        match self.rx.try_recv() {
            Ok(ev) => Some(ev),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }
}

/// bridge 地址: 默认 ws://127.0.0.1:8980, PLA_BRIDGE_URL 环境变量可覆写。
pub fn bridge_url() -> String {
    std::env::var("PLA_BRIDGE_URL").unwrap_or_else(|_| "ws://127.0.0.1:8980".into())
}

/// 常驻线程主体: 建桥、上报状态、循环处理命令 + 上报存活翻转。
fn bridge_thread_main(
    url: String,
    cmd_rx: Receiver<BridgeCmd>,
    ev_tx: std::sync::mpsc::Sender<BridgeEvent>,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let _ = ev_tx.send(BridgeEvent::Status(format!("断线:runtime 初始化失败({e})")));
            return;
        }
    };
    // doc_key 与旧实现保持一致(暂固定)。
    let doc_key = "model1";
    rt.block_on(async move {
        // connect() 内部 spawn supervisor(重连退避内建), 首连也在 supervisor 内完成。
        let client = match pla_bridge::BridgeClient::connect(&url) {
            Ok(c) => c,
            Err(e) => {
                let _ = ev_tx.send(BridgeEvent::Status(format!("断线:连接失败({e})")));
                return;
            }
        };
        let _ = ev_tx.send(BridgeEvent::Status("连接中…".into()));
        // 首次握手等待(超时不致命, supervisor 会继续重试; 但必须回报状态,
        // 否则 UI 永远停在「连接中…」— QA Bug B)。
        if client.wait_alive(Duration::from_secs(10)).await.is_ok() {
            let _ = ev_tx.send(BridgeEvent::Status("已连接".into()));
        } else {
            let _ = ev_tx.send(BridgeEvent::Status(
                "断线:连接超时(10s) — bridge 服务未启动?".into(),
            ));
        }

        let mut was_alive = client.is_alive();
        loop {
            // ---- 命令处理(非阻塞) ----
            match cmd_rx.try_recv() {
                Ok(BridgeCmd::Shutdown) => {
                    client.shutdown();
                    break;
                }
                Ok(BridgeCmd::RefreshEntities) => {
                    if client.is_alive() {
                        match tokio::time::timeout(
                            Duration::from_secs(5),
                            client.list_visible(doc_key),
                        )
                        .await
                        {
                            Ok(Ok(v)) => {
                                let list = v
                                    .get("entities")
                                    .and_then(|e| e.as_array())
                                    .cloned()
                                    .unwrap_or_default();
                                let _ = ev_tx.send(BridgeEvent::Entities(list));
                            }
                            Ok(Err(e)) => {
                                let _ = ev_tx.send(BridgeEvent::Error(format!("拉取实体失败: {e}")));
                            }
                            Err(_) => {
                                let _ = ev_tx.send(BridgeEvent::Error("拉取实体超时(5s)".into()));
                            }
                        }
                    } else {
                        let _ = ev_tx.send(BridgeEvent::Error("bridge 未连接, 无法拉取实体".into()));
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    // UI 侧已 drop 命令发送端: 显式退出(语义修复, QA 建议)。
                    client.shutdown();
                    break;
                }
                Err(_) => {} // 无命令
            }
            // ---- 存活翻转上报(supervisor 重连后 is_alive 会翻回 true) ----
            let alive = client.is_alive();
            if alive != was_alive {
                was_alive = alive;
                let _ = ev_tx.send(BridgeEvent::Status(if alive {
                    "已连接".into()
                } else {
                    "断线:等待重连…".into()
                }));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    });
}
