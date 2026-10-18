//! Protocol smoke: connect a BridgeClient, subscribe_all + list_all, print
//! pretty JSON. Verifies the bridge protocol end-to-end against a live
//! Plasticity instance without touching snapshot state.
//!
//! Run:
//!   cargo run -p pla-snapshot --example smoke -- \
//!       --url ws://127.0.0.1:8980 --doc mydoc.plasticity

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use pla_bridge::reconnect::Backoff;
use pla_bridge::{BridgeClient, BridgeConfig};

fn cfg() -> BridgeConfig {
    BridgeConfig {
        request_timeout: Duration::from_secs(5),
        backoff: Backoff::new(Duration::from_millis(100), 2.0, Duration::from_secs(2)),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut url = "ws://127.0.0.1:8980".to_string();
    let mut doc = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => {
                url = args.next().context("--url requires a value")?;
            }
            "--doc" => {
                doc = args.next().context("--doc requires a value")?;
            }
            other => bail!("unknown argument: {other} (expected --url/--doc)"),
        }
    }
    if doc.is_empty() {
        bail!("--doc <doc_key> is required (e.g. --doc mydoc.plasticity)");
    }

    let client = BridgeClient::connect_with_config(&url, cfg())
        .with_context(|| format!("connect to {url}"))?;
    client
        .wait_alive(Duration::from_secs(5))
        .await
        .context("link did not come alive within 5s")?;
    eprintln!("connected: {url}");

    client
        .subscribe_all()
        .await
        .context("subscribe_all")?;
    println!("subscribe_all: ok");

    let list: Value = client.list_all(&doc).await.context("list_all")?;
    println!("list_all({doc}):\n{}", serde_json::to_string_pretty(&list)?);

    client.shutdown();
    Ok(())
}
