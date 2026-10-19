//! CDP 探针(R7-13): 连 Plasticity renderer 的 DevTools 端口, 执行 JS 并回显。
//!
//! 前置: Plasticity 需以打过 PLA-CDP 补丁的方式启动(removeSwitch 已被中和,
//! 端口 9333 由注入的 appendSwitch 打开)。
//! 用法:
//!   cargo run -p pla-bridge --example cdp_probe -- --list
//!   cargo run -p pla-bridge --example cdp_probe -- --port 9333 --eval "<js>"
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut port = 9333u16;
    let mut eval: Option<String> = None;
    let mut list = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().unwrap().parse()?,
            "--eval" => eval = Some(args.next().unwrap()),
            "--list" => list = true,
            other => anyhow::bail!("unknown arg: {other}"),
        }
    }

    let targets = http_get_json(port, "/json/list").await?;
    if list {
        println!("{targets}");
        return Ok(());
    }
    let v: serde_json::Value = serde_json::from_str(&targets)?;
    let page = v
        .as_array()
        .and_then(|a| {
            a.iter().find(|t| {
                t["type"] == "page" && t["title"].as_str().unwrap_or("").contains("Plasticity")
            })
        })
        .or_else(|| v.as_array().and_then(|a| a.iter().find(|t| t["type"] == "page")))
        .ok_or_else(|| anyhow::anyhow!("no page target"))?;
    let ws_url = page["webSocketDebuggerUrl"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no ws url"))?
        .to_string();
    println!("[probe] target: {}", page["title"].as_str().unwrap_or("?"));

    let eval = match eval {
        Some(e) => e,
        None => anyhow::bail!("pass --eval \"<js>\" or --list"),
    };
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await?;
    ws.send(Message::Text(
        serde_json::json!({"id":1,"method":"Runtime.enable"}).to_string(),
    ))
    .await?;
    ws.send(Message::Text(
        serde_json::json!({
            "id": 2,
            "method": "Runtime.evaluate",
            "params": {
                "expression": eval,
                "returnByValue": true,
                "awaitPromise": true,
                "userGesture": true
            }
        })
        .to_string(),
    ))
    .await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let msg = tokio::time::timeout_at(deadline, ws.next())
            .await
            .map_err(|_| anyhow::anyhow!("eval timeout (20s)"))?
            .ok_or_else(|| anyhow::anyhow!("ws stream ended"))?
            .map_err(|e| anyhow::anyhow!("ws read: {e}"))?;
        match msg {
            Message::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t)?;
                if v["id"] == 2 {
                    let r = &v["result"];
                    if let Some(exc) = r["exceptionDetails"]["text"].as_str() {
                        println!("EXCEPTION: {exc}");
                        if let Some(d) =
                            r["exceptionDetails"]["exception"]["description"].as_str()
                        {
                            println!("{d}");
                        }
                    } else {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&r["result"]["value"]).unwrap_or_default()
                        );
                    }
                    break;
                }
            }
            Message::Close(_) => anyhow::bail!("ws closed"),
            _ => {}
        }
    }
    Ok(())
}

async fn http_get_json(port: u16, path: &str) -> anyhow::Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
    s.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await?;
    let text = String::from_utf8_lossy(&buf);
    let body = text.split("\r\n\r\n").nth(1).unwrap_or("");
    Ok(body.to_string())
}
