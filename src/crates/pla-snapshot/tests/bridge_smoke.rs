//! Integration test: full manual-snapshot loop against the mock WS server
//! (minimal replication of pla-bridge's mock, per task note).
//!
//! Flow: mock server -> BridgeClient::connect -> wait_alive -> fake source
//! file -> snapshot_create_from_bridge -> list -> delete -> empty.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};

use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{accept_async, WebSocketStream};

use pla_bridge::reconnect::Backoff;
use pla_bridge::{BridgeClient, BridgeConfig};
use pla_snapshot::{SnapshotStore, TriggerKind};

fn test_cfg() -> BridgeConfig {
    BridgeConfig {
        request_timeout: Duration::from_secs(2),
        backoff: Backoff::new(Duration::from_millis(50), 2.0, Duration::from_millis(400)),
    }
}

/// Mock server: LIST_ALL returns a 3-entity payload; SUBSCRIBE_ALL pongs
/// with null; HELLO ignored.
async fn mock_server(listener: TcpListener) {
    loop {
        let Ok((sock, _)) = listener.accept().await else { return };
        let ws = match accept_async(sock).await {
            Ok(ws) => ws,
            Err(_) => continue,
        };
        tokio::spawn(mock_conn(ws));
    }
}

async fn mock_conn(ws: WebSocketStream<TcpStream>) {
    let (mut sink, mut stream) = ws.split();
    // [FIX 2026-10-18 OC 直修] 二进制协议 mock(原 JSON mock 对不上二进制客户端):
    let hs_resp: Vec<u8> = vec![
        0x64,0,0,0, 0,0,0,0, 0x0e,0,0,0, 0,0,0,0, 1,0,0,0, 2,0,0,0, 3,0,0,0,
        0x0a,0,0,0, 0x0b,0,0,0, 0x14,0,0,0, 0x16,0,0,0, 0x17,0,0,0, 0x18,0,0,0,
        0x19,0,0,0, 0x1a,0,0,0, 0x1f,0,0,0, 0x64,0,0,0,
    ];
    fn list_all_resp(mid: u32) -> Vec<u8> {
        fn obj(id: u32, kind: u32, name: &str) -> Vec<u8> {
        // [真机布局 DPL-218 逆向]: flags|kind_raw|type_code|index|0|0|parent|tag6|len|name+pad
        let kind_raw = 4u32;
        let type_code = match kind { 2 => 2u32, _ => 5 };
        let mut v = Vec::new();
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&kind_raw.to_le_bytes());
        v.extend_from_slice(&type_code.to_le_bytes());
        v.extend_from_slice(&id.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0xffffffffu32.to_le_bytes());
        v.extend_from_slice(&6u32.to_le_bytes());
        v.extend_from_slice(&(name.len() as u32).to_le_bytes());
        v.extend_from_slice(name.as_bytes());
        let pad = (4 - (name.len() % 4)) % 4;
        v.extend_from_slice(&vec![0u8; pad]);
        v
    }
        let mut v = 20u32.to_le_bytes().to_vec();
        v.extend_from_slice(&mid.to_le_bytes());
        v.extend_from_slice(&200u32.to_le_bytes());
        let fname = b"model1.plasticity";
        v.extend_from_slice(&(fname.len() as u32).to_le_bytes());
        v.extend_from_slice(fname);
                // 4 字节对齐 padding(protocol::Reader::string 读后对齐, 真机帧 17B+3pad 实锤)
                let pad = (4 - (fname.len() % 4)) % 4;
                v.extend_from_slice(&vec![0u8; pad]);
        v.extend_from_slice(&0u32.to_le_bytes());
        let recs = vec![obj(1, 1, "Body"), obj(2, 1, "Arm"), obj(3, 1, "Leg")];
        v.extend_from_slice(&(recs.len() as u32).to_le_bytes());
        for r in recs { v.extend_from_slice(&(r.len() as u32).to_le_bytes()); v.extend_from_slice(&r); }
        v
    }
    while let Some(Ok(msg)) = stream.next().await {
        let Message::Binary(b) = msg else { continue };
        if b.len() < 8 { continue; }
        let mt = u32::from_le_bytes([b[0],b[1],b[2],b[3]]);
        let mid = u32::from_le_bytes([b[4],b[5],b[6],b[7]]);
        let out = match mt {
            100 => { let mut r = hs_resp.clone(); r[4..8].copy_from_slice(&mid.to_le_bytes()); r }
            23 => continue,
            20 => list_all_resp(mid),
            other => { let mut v = other.to_le_bytes().to_vec(); v.extend_from_slice(&mid.to_le_bytes()); v.extend_from_slice(&200u32.to_le_bytes()); v.extend_from_slice(&0u32.to_le_bytes()); v }
        };
        if sink.send(Message::Binary(out)).await.is_err() { break; }
    }
}

#[tokio::test]
async fn full_manual_snapshot_loop_over_mock_bridge() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    tokio::spawn(mock_server(listener));

    let client = BridgeClient::connect_with_config(&url, test_cfg()).unwrap();
    client.wait_alive(Duration::from_secs(2)).await.unwrap();

    // Fake source file + temp base dir.
    let base = std::env::temp_dir().join(format!(
        "pla-snapshot-smoke-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&base).unwrap();
    let src = base.join("incoming").join("doc.plasticity");
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, b"plasticity-smoke-bytes").unwrap();

    let store = SnapshotStore::new(&base);
    let meta = store
        .snapshot_create_from_bridge(&client, "doc.plasticity", &src, "manual smoke")
        .await
        .unwrap();
    assert_eq!(meta.doc_filename, "doc.plasticity");
    assert_eq!(meta.trigger, TriggerKind::Manual);
    assert_eq!(meta.entity_count, 3, "entity_count from mock LIST_ALL");
    assert_eq!(meta.label, "manual-smoke");

    // One entry; both files exist on disk.
    let list = store.snapshot_list("doc.plasticity");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].entity_count, 3);
    let dir = base.join("doc.plasticity").join("snapshots");
    let snap = dir
        .join(format!("{}.plasticity", stem_of(&list[0], &dir)));
    assert!(snap.exists(), "snapshot file missing: {}", snap.display());
    assert!(snap.with_extension("json").exists(), "meta json missing");
    assert_eq!(
        std::fs::read_to_string(&snap).unwrap(),
        "plasticity-smoke-bytes"
    );

    // Delete removes both; list empty.
    let stem = stem_of(&list[0], &dir);
    store.snapshot_delete("doc.plasticity", &stem).unwrap();
    assert!(!snap.exists());
    assert!(!snap.with_extension("json").exists());
    assert!(store.snapshot_list("doc.plasticity").is_empty());

    client.shutdown();
    std::fs::remove_dir_all(&base).ok();
}

/// Recover the on-disk stem for a meta (ts component + "-" + label prefix).
fn stem_of(meta: &pla_snapshot::SnapshotMeta, dir: &std::path::Path) -> String {
    let ts: String = meta
        .ts
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '-' || *c == 'T')
        .collect();
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_str().unwrap_or("").to_string())
        .find(|n| n.starts_with(&ts) && n.ends_with(".plasticity"))
        .map(|n| n.trim_end_matches(".plasticity").to_string())
        .expect("stem")
}
