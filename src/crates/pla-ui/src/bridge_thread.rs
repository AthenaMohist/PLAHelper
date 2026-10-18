//! [PLA-000] bridge 调用桥 — UI 线程同步拉取(短连接, 5s 超时), 后续换后台线程+通道。
use std::time::Duration;

/// 拉实体列表(Vec<serde_json::Value>)。同步阻塞 ≤5s。
pub fn fetch_entities_blocking(doc_key: &str) -> Result<Vec<serde_json::Value>, String> {
    let url = bridge_url();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async move {
        // connect() 是同步 TCP 连接; 超时靠整体 timeout 包住后续 async 流程。
        let client = pla_bridge::BridgeClient::connect(&url).map_err(|e| format!("连接失败: {e} — Plasticity bridge server 未开?"))?;
        let v = tokio::time::timeout(
            Duration::from_secs(5),
            client.list_visible(doc_key),
        )
        .await
        .map_err(|_| "查询超时(5s)".to_string())?
        .map_err(|e| e.to_string())?;
        client.shutdown();
        Ok(v.get("entities")
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default())
    })
}

fn bridge_url() -> String {
    std::env::var("PLA_BRIDGE_URL").unwrap_or_else(|_| "ws://127.0.0.1:8980".into())
}
