//! Integration tests: mock bridge server over loopback WebSocket (same
//! pattern as pla-bridge / pla-selection). Verifies the M4 dim flow:
//! replace write-back via PUT_SOME, missing-member report, topology hard
//! reject, and the degraded read-only path. Payload shapes are TODO-PROBE
//! assumptions.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::accept_async;

use pla_bridge::reconnect::Backoff;
use pla_bridge::{BridgeClient, BridgeConfig};
use pla_dim::{dim_update, DimDir, DimStore};
use pla_selection::SelectionStore;


fn test_cfg() -> BridgeConfig {
    BridgeConfig {
        request_timeout: Duration::from_secs(2),
        backoff: Backoff::new(Duration::from_millis(50), 2.0, Duration::from_millis(400)),
    }
}

/// Mock bridge with a fixed LIST_ALL entity payload (two translating
/// solids + one fillet) and switchable PUT_SOME availability.
async fn mock_server(listener: TcpListener, put_some_ok: Arc<AtomicBool>) {
    loop {
        let Ok((sock, _)) = listener.accept().await else { return };
        let flag = Arc::clone(&put_some_ok);
        let ok0 = flag.load(Ordering::SeqCst);
        tokio::spawn(async move {
            let ws = match accept_async(sock).await {
                Ok(ws) => ws,
                Err(_) => return,
            };
                        let (mut sink, mut stream) = ws.split();
            // [FIX 2026-10-18 OC 直修] 二进制协议 mock(原 JSON mock 对不上二进制客户端 → E2E 全 timeout):
            // 握手: 客户端首帧 [100][mid], 回 REAL_HANDSHAKE_RESP(mid 重戳);
            // LIST_ALL(20) → 2 实体表; PUT_SOME(31) → code=200 + counter++; 其余回 code=200 空表。
            let hs_resp: Vec<u8> = [
                0x64,0,0,0, 0,0,0,0, 0x0e,0,0,0, 0,0,0,0, 1,0,0,0, 2,0,0,0, 3,0,0,0,
                0x0a,0,0,0, 0x0b,0,0,0, 0x14,0,0,0, 0x16,0,0,0, 0x17,0,0,0, 0x18,0,0,0,
                0x19,0,0,0, 0x1a,0,0,0, 0x1f,0,0,0, 0x64,0,0,0,
            ].to_vec();
            fn obj_record(id: u32, kind: u32, name: &str) -> Vec<u8> {
                // [真机布局 DPL-218 逆向]: flags|kind_raw|type_code|index|0|0|parent|tag6|len|name+pad
                let kind_raw = 4u32;                                   // 实体 kind_raw=4(Extruded 实锤)
                let type_code = match kind { 2 => 2u32, _ => 5 };      // 5=solid, 2=sheet
                let mut v = Vec::new();
                v.extend_from_slice(&1u32.to_le_bytes());
                v.extend_from_slice(&kind_raw.to_le_bytes());
                v.extend_from_slice(&type_code.to_le_bytes());
                v.extend_from_slice(&id.to_le_bytes());               // index = 对外 id
                v.extend_from_slice(&0u32.to_le_bytes());
                v.extend_from_slice(&0u32.to_le_bytes());
                v.extend_from_slice(&0xffffffffu32.to_le_bytes());    // parent=root
                v.extend_from_slice(&6u32.to_le_bytes());             // tag=6 name
                v.extend_from_slice(&(name.len() as u32).to_le_bytes());
                v.extend_from_slice(name.as_bytes());
                let pad = (4 - (name.len() % 4)) % 4;
                v.extend_from_slice(&vec![0u8; pad]);
                // facets 段(dim 顶点平移测试用, mock 语义; 真机块布局待探针):
                v.extend_from_slice(&1u32.to_le_bytes());
                v.extend_from_slice(&2u32.to_le_bytes());
                for pt in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0]] {
                    for c in pt { v.extend_from_slice(&c.to_le_bytes()); }
                }
                v
            }
            fn list_all_resp(mid: u32) -> Vec<u8> {
                let mut v = Vec::new();
                v.extend_from_slice(&20u32.to_le_bytes());
                v.extend_from_slice(&mid.to_le_bytes());
                v.extend_from_slice(&200u32.to_le_bytes()); // code
                let fname = b"model1.plasticity";
                v.extend_from_slice(&(fname.len() as u32).to_le_bytes());
                v.extend_from_slice(fname);
                // 4 字节对齐 padding(protocol::Reader::string 读后对齐, 真机帧 17B+3pad 实锤)
                let pad = (4 - (fname.len() % 4)) % 4;
                v.extend_from_slice(&vec![0u8; pad]);
                v.extend_from_slice(&0u32.to_le_bytes()); // version
                let recs = vec![obj_record(1, 1, "Body"), obj_record(2, 1, "Arm"), {
            // fillet 实体: type_code=9(TODO-PROBE 真机码), kind_raw=4
            let mut v = obj_record(3, 1, "Blend");
            v[8..12].copy_from_slice(&9u32.to_le_bytes());
            v
        }];
                v.extend_from_slice(&(recs.len() as u32).to_le_bytes());
                for r in recs { v.extend_from_slice(&(r.len() as u32).to_le_bytes()); v.extend_from_slice(&r); }
                v
            }
            fn code_resp_code(msg_type: u32, mid: u32, code: u32) -> Vec<u8> {
                let mut v = Vec::new();
                v.extend_from_slice(&msg_type.to_le_bytes());
                v.extend_from_slice(&mid.to_le_bytes());
                v.extend_from_slice(&code.to_le_bytes());
                v.extend_from_slice(&0u32.to_le_bytes());
                v
            }
            fn code_resp(msg_type: u32, mid: u32) -> Vec<u8> {
                let mut v = Vec::new();
                v.extend_from_slice(&msg_type.to_le_bytes());
                v.extend_from_slice(&mid.to_le_bytes());
                v.extend_from_slice(&200u32.to_le_bytes());
                v.extend_from_slice(&0u32.to_le_bytes()); // empty result table
                v
            }
            // 握手阶段: 等客户端首帧(type=100)后回握手响应
            while let Some(Ok(msg)) = stream.next().await {
                let Message::Binary(b) = msg else { continue };
                if b.len() >= 8 {
                    let mt = u32::from_le_bytes([b[0],b[1],b[2],b[3]]);
                    let mid = u32::from_le_bytes([b[4],b[5],b[6],b[7]]);
                    let out = match mt {
                        100 => hs_resp.clone(),
                        23 => continue, // SUBSCRIBE_ALL: 无专响应(probe 语义)
                        20 => list_all_resp(mid),
                        31 => {
                            if ok0 { flag.store(true, Ordering::SeqCst); code_resp(31, mid) } else { code_resp_code(31, mid, 500) }
                        }
                        other => code_resp(other, mid),
                    };
                    if mt == 100 { let mut r = out; r[4..8].copy_from_slice(&mid.to_le_bytes()); if sink.send(Message::Binary(r)).await.is_err() { break; } continue; }
                    if sink.send(Message::Binary(out)).await.is_err() { break; }
                }
            }
        });
    }
}

async fn spawn_mock(put_some_ok: bool) -> BridgeClient {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let flag = Arc::new(AtomicBool::new(put_some_ok));
    tokio::spawn(mock_server(listener, flag));
    let client = BridgeClient::connect_with_config(&url, test_cfg()).unwrap();
    client.wait_alive(Duration::from_secs(2)).await.unwrap();
    client
}

fn temp_base(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "pla-dim-ws-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&base).unwrap();
    base
}

#[tokio::test]
async fn dim_update_translates_and_persists_new_value() {
    let client = spawn_mock(true).await;
    let base = temp_base("update-ok");
    let selections = SelectionStore::new(&base);
    let dims = DimStore::new(&base);
    selections
        .save("doc.plasticity", "arms", vec!["1".into(), "2".into()])
        .unwrap();
    dims.define("doc.plasticity", "length", "arms", DimDir::unit_x(), 10.0)
        .unwrap();

    let report = dim_update(&client, &selections, &dims, "doc.plasticity", "length", 12.0)
        .await
        .unwrap();
    assert!(report.ok);
    assert_eq!(report.moved, 2);
    assert!(report.missing.is_empty());
    // New current value persisted (delta next time is relative to it).
    assert_eq!(dims.load("doc.plasticity", "length").unwrap().value, 12.0);
    std::fs::remove_dir_all(&base).ok();
    client.shutdown();
}

#[tokio::test]
async fn dim_update_missing_members_reported() {
    let client = spawn_mock(true).await;
    let base = temp_base("update-missing");
    let selections = SelectionStore::new(&base);
    let dims = DimStore::new(&base);
    selections
        .save(
            "doc.plasticity",
            "stale",
            vec!["1".into(), "gone".into()],
        )
        .unwrap();
    dims.define("doc.plasticity", "length", "stale", DimDir::unit_x(), 10.0)
        .unwrap();

    let report = dim_update(&client, &selections, &dims, "doc.plasticity", "length", 11.0)
        .await
        .unwrap();
    assert!(!report.ok);
    assert_eq!(report.missing, vec!["gone".to_string()]);
    assert_eq!(report.moved, 0);
    // Value unchanged.
    assert_eq!(dims.load("doc.plasticity", "length").unwrap().value, 10.0);
    std::fs::remove_dir_all(&base).ok();
    client.shutdown();
}

#[tokio::test]
async fn dim_update_hard_rejects_topology_members() {
    let client = spawn_mock(true).await;
    let base = temp_base("update-topo");
    let selections = SelectionStore::new(&base);
    let dims = DimStore::new(&base);
    selections
        .save("doc.plasticity", "blend", vec!["3".into()])
        .unwrap();
    dims.define("doc.plasticity", "radius", "blend", DimDir::unit_x(), 3.0)
        .unwrap();

    let err = dim_update(&client, &selections, &dims, "doc.plasticity", "radius", 4.0)
        .await
        .unwrap_err();
    match err {
        pla_dim::DimError::TopologyUnsupported { member, kind } => {
            assert_eq!(member, "3");
            assert_eq!(kind, "fillet");
        }
        other => panic!("expected TopologyUnsupported, got {other:?}"),
    }
    std::fs::remove_dir_all(&base).ok();
    client.shutdown();
}

#[tokio::test]
async fn dim_update_degrades_to_read_only_when_put_some_unavailable() {
    let client = spawn_mock(false).await;
    let base = temp_base("update-degraded");
    let selections = SelectionStore::new(&base);
    let dims = DimStore::new(&base);
    selections
        .save("doc.plasticity", "arms", vec!["1".into()])
        .unwrap();
    dims.define("doc.plasticity", "length", "arms", DimDir::unit_x(), 10.0)
        .unwrap();

    let report = dim_update(&client, &selections, &dims, "doc.plasticity", "length", 15.0)
        .await
        .unwrap();
    assert!(!report.ok);
    assert_eq!(report.moved, 0);
    assert!(report.message.contains("read-only"));
    // Value unchanged on the degraded path.
    assert_eq!(dims.load("doc.plasticity", "length").unwrap().value, 10.0);
    std::fs::remove_dir_all(&base).ok();
    client.shutdown();
}
