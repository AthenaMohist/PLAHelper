// [R5 探针] 码 23 SUBSCRIBE_ALL_1 订阅行为取证。
//
// 背景: 握手码表实锤 23 在支持表内且从未被探测; 需确认服务端是否推帧、
// 帧码布局、是否像分片/增量更新(决定 P0 分片拉取 PLA-004 TODO#9 方案)。
// 两种顺序各试一次: --mode direct(直接 23) / --mode after22(先 22 再 23)。
//
// 安全阀: 总收集上限 8MB 或 60s, 超限即停 — 防订阅洪泛拖垮 Plasticity。
//
// Run: cargo run -p pla-snapshot --example subscribe_probe -- \
//        --url ws://127.0.0.1:8980 --mode direct --secs 60
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

/// 收集总字节上限(防订阅洪泛)。
const MAX_COLLECT_BYTES: usize = 8 * 1024 * 1024;
/// 单帧读取等待上限。
const FRAME_WAIT: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() -> Result<()> {
    let mut url = "ws://127.0.0.1:8980".to_string();
    let mut mode = "direct".to_string();
    let mut secs: u64 = 60;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--url" => url = args.next().context("--url value")?,
            "--mode" => mode = args.next().context("--mode value")?,
            "--secs" => secs = args.next().context("--secs value")?.parse()?,
            other => bail!("unknown {other}"),
        }
    }
    if mode != "direct" && mode != "after22" {
        bail!("--mode must be direct|after22, got {mode}");
    }

    let (ws, _) = tokio_tungstenite::connect_async(&url).await?;
    let (mut sink, mut stream) = ws.split();

    // ---- 握手: 码100 mid=1, 无 payload ----
    sink.send(Message::Binary(vec![100, 0, 0, 0, 1, 0, 0, 0]))
        .await?;
    let hs = tokio::time::timeout(FRAME_WAIT, stream.next())
        .await
        .context("handshake timeout")?
        .ok_or_else(|| anyhow::anyhow!("handshake closed"))??;
    let hs_bytes = match hs {
        Message::Binary(b) => b,
        other => bail!("handshake non-binary: {other:?}"),
    };
    println!("=== SUBSCRIBE PROBE mode={mode} ===");
    println!("HANDSHAKE resp ({}B): {}", hs_bytes.len(), hex(&hs_bytes));
    // 解析支持码表验证链路(header: code=100, mid, count, n×u32)。
    if hs_bytes.len() >= 12 {
        let n = u32::from_le_bytes(hs_bytes[8..12].try_into().unwrap()) as usize;
        let codes: Vec<u32> = (0..n)
            .filter_map(|i| {
                let o = 12 + i * 4;
                if o + 4 <= hs_bytes.len() {
                    Some(u32::from_le_bytes(hs_bytes[o..o + 4].try_into().unwrap()))
                } else {
                    None
                }
            })
            .collect();
        println!("supported codes: {codes:?}");
    }

    let mut out = format!("=== SUBSCRIBE PROBE mode={mode} ===\n");
    out.push_str(&format!("HANDSHAKE resp ({}B)\n", hs_bytes.len()));

    // ---- 前置 22(仅 after22): 全树请求, 可能到 368KB, 给足 15s ----
    if mode == "after22" {
        sink.send(Message::Binary(frame(22, 50))).await?;
        match tokio::time::timeout(Duration::from_secs(15), stream.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                println!("PRE-22 resp: {}B (code={})", b.len(), le_u32(&b, 0));
                out.push_str(&format!("PRE-22 resp: {}B code={}\n", b.len(), le_u32(&b, 0)));
                // 落盘完整 22 响应(分片设计取证素材)。
                let pre22_path = "subscribe_pre22_dump.bin";
                std::fs::write(pre22_path, &b)?;
                println!("=== SAVED {pre22_path} ({} bytes) ===", b.len());
            }
            _ => {
                println!("PRE-22 resp: timeout/err");
                out.push_str("PRE-22 resp: TIMEOUT\n");
            }
        }
    }

    // ---- 发码 23 SUBSCRIBE_ALL_1(无 payload) ----
    sink.send(Message::Binary(frame(23, 60))).await?;
    println!("SENT code=23 mid=60, collecting pushed frames for {secs}s (cap 8MB)…");
    out.push_str(&format!("SENT code=23 mid=60, collecting {secs}s (cap 8MB)\n"));

    // ---- 收集循环: 截止 60s / 8MB, 任一超限即停 ----
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut total: usize = 0;
    let mut count: usize = 0;
    let mut dump: Vec<u8> = Vec::new();
    let dump_path = format!("subscribe_dump_{mode}.bin");
    let txt_path = format!("subscribe_probe_{mode}.txt");
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            println!("collection deadline reached");
            out.push_str("DEADLINE reached\n");
            break;
        }
        match tokio::time::timeout(remaining, stream.next()).await {
            Err(_) => {
                println!("collection deadline reached");
                out.push_str("DEADLINE reached\n");
                break;
            }
            Ok(None) => {
                println!("stream closed by server");
                out.push_str("STREAM CLOSED\n");
                break;
            }
            Ok(Some(Err(e))) => {
                println!("stream error: {e}");
                out.push_str(&format!("STREAM ERROR: {e}\n"));
                break;
            }
            Ok(Some(Ok(Message::Binary(b)))) => {
                count += 1;
                total += b.len();
                let code = le_u32(&b, 0);
                let mid = le_u32(&b, 4);
                let line = format!(
                    "frame#{count} code={code} mid={mid} len={}B head64={}\n",
                    b.len(),
                    hex(&b[..b.len().min(64)])
                );
                println!("{}", line.trim_end());
                out.push_str(&line);
                // dump 文件: [u32 len][frame bytes]… 便于逐帧回放解析。
                dump.extend_from_slice(&(b.len() as u32).to_le_bytes());
                dump.extend_from_slice(&b);
                if total >= MAX_COLLECT_BYTES {
                    println!("byte cap (8MB) reached — stopping to protect Plasticity");
                    out.push_str("BYTE CAP reached\n");
                    break;
                }
            }
            Ok(Some(Ok(other))) => {
                println!("non-binary frame: {other:?}");
                out.push_str(&format!("NON-BINARY: {other:?}\n"));
            }
        }
    }

    println!("TOTAL: {count} frames, {total}B");
    out.push_str(&format!("TOTAL: {count} frames, {total}B\n"));
    std::fs::write(&txt_path, &out)?;
    std::fs::write(&dump_path, &dump)?;
    println!("=== SAVED {txt_path} + {dump_path} (current dir) ===");
    Ok(())
}

fn frame(code: u32, mid: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(8);
    v.extend_from_slice(&code.to_le_bytes());
    v.extend_from_slice(&mid.to_le_bytes());
    v
}

/// 帧内偏移处的 LE u32(越界返回 0)。
fn le_u32(b: &[u8], off: usize) -> u32 {
    if off + 4 <= b.len() {
        u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
    } else {
        0
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
