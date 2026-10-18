//! Integration tests: mock bridge server over loopback WebSocket (same
//! pattern as crates/pla-bridge/tests/mock_ws.rs). Verifies the M3
//! selection flow end-to-end: cache fetch via LIST_ALL, apply with
//! PUT_SOME, and the missing-members report (no silent drop, D3).
//! All payload shapes are TODO-PROBE assumptions.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::accept_async;

use pla_bridge::reconnect::Backoff;
use pla_bridge::{BridgeClient, BridgeConfig};
use pla_selection::{ApplyReport, EntityCache, SelectionStore};


fn test_cfg() -> BridgeConfig {
    BridgeConfig {
        request_timeout: Duration::from_secs(2),
        backoff: Backoff::new(Duration::from_millis(50), 2.0, Duration::from_millis(400)),
    }
}

/// Mock bridge: LIST_ALL answers with a fixed entity payload; PUT_SOME
/// answers ok and increments `put_some_count`.
async fn mock_server(listener: TcpListener, put_some_count: Arc<AtomicUsize>) {
    loop {
        let Ok((sock, _)) = listener.accept().await else { return };
        let counter = Arc::clone(&put_some_count);
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
                let recs = vec![obj_record(1, 1, "Body"), obj_record(2, 1, "Arm")];
                v.extend_from_slice(&(recs.len() as u32).to_le_bytes());
                for r in recs { v.extend_from_slice(&(r.len() as u32).to_le_bytes()); v.extend_from_slice(&r); }
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
                        31 => { counter.fetch_add(1, Ordering::SeqCst); code_resp(31, mid) }
                        other => code_resp(other, mid),
                    };
                    if mt == 100 { let mut r = out; r[4..8].copy_from_slice(&mid.to_le_bytes()); if sink.send(Message::Binary(r)).await.is_err() { break; } continue; }
                    if sink.send(Message::Binary(out)).await.is_err() { break; }
                }
            }
        });
    }
}

async fn spawn_mock() -> (String, BridgeClient, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let counter = Arc::new(AtomicUsize::new(0));
    tokio::spawn(mock_server(listener, Arc::clone(&counter)));
    let client = BridgeClient::connect_with_config(&url, test_cfg()).unwrap();
    client.wait_alive(Duration::from_secs(2)).await.unwrap();
    (url, client, counter)
}

fn temp_base(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "pla-sel-ws-{}-{}-{}",
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
async fn fetch_caches_tree_per_doc_key() {
    let (_url, client, _counter) = spawn_mock().await;
    let cache = EntityCache::new();
    let tree = cache.fetch(&client, "doc.plasticity").await.unwrap();
    assert_eq!(tree.doc_key, "doc.plasticity");
    assert_eq!(tree.collect_ids(), vec!["1", "2"]);
    assert_eq!(tree.version, 1);
    // Second fetch is served from cache (same generation stamp).
    let again = cache.fetch(&client, "doc.plasticity").await.unwrap();
    assert_eq!(again.version, 1);
    // Invalidation forces a refetch with a newer stamp.
    cache.invalidate("doc.plasticity");
    let refetched = cache.fetch(&client, "doc.plasticity").await.unwrap();
    assert_eq!(refetched.version, 2);
    // Docs are isolated (D5).
    let other = cache.fetch(&client, "other.plasticity").await.unwrap();
    assert_eq!(other.collect_ids(), vec!["1", "2"]);
    assert_eq!(other.doc_key, "other.plasticity");
    client.shutdown();
}

#[tokio::test]
async fn apply_puts_group_when_all_members_exist() {
    let (_url, client, counter) = spawn_mock().await;
    let base = temp_base("apply-ok");
    let store = SelectionStore::new(&base);
    store
        .save("doc.plasticity", "arms", vec!["1".into(), "2".into()])
        .unwrap();
    let cache = EntityCache::new();
    let tree = cache.fetch(&client, "doc.plasticity").await.unwrap();
    let report = store
        .apply(&client, "doc.plasticity", &tree, "arms")
        .await
        .unwrap();
    assert_eq!(
        report,
        ApplyReport {
            ok: true,
            missing: vec![]
        }
    );
    assert_eq!(counter.load(Ordering::SeqCst), 1, "exactly one PUT_SOME");
    std::fs::remove_dir_all(&base).ok();
    client.shutdown();
}

#[tokio::test]
async fn apply_reports_missing_without_put_some() {
    let (_url, client, counter) = spawn_mock().await;
    let base = temp_base("apply-missing");
    let store = SelectionStore::new(&base);
    store
        .save(
            "doc.plasticity",
            "stale",
            vec!["1".into(), "deleted-entity".into()],
        )
        .unwrap();
    let cache = EntityCache::new();
    let tree = cache.fetch(&client, "doc.plasticity").await.unwrap();
    let report = store
        .apply(&client, "doc.plasticity", &tree, "stale")
        .await
        .unwrap();
    // D3: missing members surfaced for UI yellow-marking; nothing written.
    assert_eq!(
        report,
        ApplyReport {
            ok: false,
            missing: vec!["deleted-entity".to_string()]
        }
    );
    assert_eq!(counter.load(Ordering::SeqCst), 0, "no PUT_SOME on missing");
    std::fs::remove_dir_all(&base).ok();
    client.shutdown();
}
