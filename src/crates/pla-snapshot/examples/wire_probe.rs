// [DPL-218 探针] 遍历握手声明码, 落盘原始响应 hex 供逆向。
// Run: cargo run -p pla-snapshot --example wire_probe -- --url ws://host:8980
use std::time::Duration;
use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

#[tokio::main]
async fn main() -> Result<()> {
    let mut url = "ws://127.0.0.1:8980".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--url" { url = args.next().context("--url value")?; }
        else { bail!("unknown {a}"); }
    }
    let (ws, _) = tokio_tungstenite::connect_async(&url).await?;
    let (mut sink, mut stream) = ws.split();
    // handshake
    sink.send(Message::Binary(vec![100,0,0,0, 1,0,0,0])).await?;
    let hs = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .context("hs timeout")?
        .ok_or_else(|| anyhow::anyhow!("hs closed"))??;
    let hs_bytes = match hs { Message::Binary(b) => b, other => bail!("hs non-binary: {other:?}") };
    println!("=== WIRE PROBE START ===");
    println!("HANDSHAKE resp hex ({}/{}B): {}", hs_bytes.len()/2 - 1, hs_bytes.len(), hex(&hs_bytes));
    // 遍历码: 每码 header-only 请求 mid=10+i
    let codes = [10u32, 11, 16, 22, 24, 25, 26, 27, 28, 29, 30];
    let mut out = String::new();
    for (i, code) in codes.iter().enumerate() {
        let mid = 100 + i as u32;
        sink.send(Message::Binary(frame(*code, mid))).await?;
        match tokio::time::timeout(Duration::from_secs(2), stream.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                let line = format!("code={code} mid={mid} len={} hex={}\n", b.len(), hex(&b));
                // 大响应截断 hex 只存前 4096B + 后 256B
                println!("{}", &line[..line.len().min(300)]);
                out.push_str(&line);
                if *code == 22 {
                    std::fs::write("wire_dump.bin", &b).expect("write wire_dump.bin");
                    println!("=== SAVED wire_dump.bin ({} bytes, current dir) ===", b.len());
                }
            }
            _ => { println!("code={code}: err/timeout/non-binary"); out.push_str(&format!("code={code}: ERR\n")); }
        }
    }
    std::fs::write("wire_probe_out.txt", &out)?;
    println!("=== SAVED wire_probe_out.txt (current dir) ===");
    Ok(())
}

fn frame(code: u32, mid: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(8);
    v.extend_from_slice(&code.to_le_bytes());
    v.extend_from_slice(&mid.to_le_bytes());
    v
}

fn hex(b: &[u8]) -> String {
    if b.len() > 4200 {
        let mut s = hex(&b[..4096]);
        s.push_str("...TRUNC...");
        s.push_str(&hex(&b[b.len()-256..]));
        s
    } else {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
}
