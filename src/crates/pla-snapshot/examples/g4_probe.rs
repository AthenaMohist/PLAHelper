//! G4 probe: real-device PUT_SOME group-create semantics (降级项 3 复跑).
//! Run: cargo run -p pla-snapshot --example g4_probe -- --url ws://host:8980 --doc model1.plasticity

use std::time::Duration;

use anyhow::{bail, Context, Result};

use pla_bridge::reconnect::Backoff;
use pla_bridge::{BridgeClient, BridgeConfig};

#[tokio::main]
async fn main() -> Result<()> {
    let mut url = "ws://127.0.0.1:8980".to_string();
    let mut doc = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => url = args.next().context("--url requires a value")?,
            "--doc" => doc = args.next().context("--doc requires a value")?,
            other => bail!("unknown argument: {other}"),
        }
    }
    if doc.is_empty() {
        bail!("--doc required");
    }

    let cfg = BridgeConfig {
        request_timeout: Duration::from_secs(5),
        backoff: Backoff::new(Duration::from_millis(100), 2.0, Duration::from_secs(2)),
    };
    let client = BridgeClient::connect_with_config(&url, cfg)
        .with_context(|| format!("connect to {url}"))?;
    client.wait_alive(Duration::from_secs(5)).await?;

    // Probe A: empty-table PUT_SOME (G2 wire shape) — expect code=200.
    let r = client.put_some(&doc, serde_json::Value::Null).await;
    println!("put_some(empty) -> {r:?}");

    // Probe B: re-LIST_ALL to check the device state is unchanged (no
    // phantom objects created by the empty probe).
    let list = client.list_all(&doc).await?;
    println!("post-put list: {}", serde_json::to_string(&list)?);
    Ok(())
}
