//! MCP 桥(R3(b) / A6 出图部分): `pla-mcp`(Claude Code 等外部 agent 启动的 stdio MCP 服务器)和正在跑的 pla-ui 之间的文件队列。
//!
//! - pla-ui 定期写 `<数据目录>/mcp/state.json`: 当前文档、出图目录、零件清单(BOM 行)、选中的零件、出图任务、心跳时间。
//! - 读: pla-mcp 直接读文件(sheets.json / 报告 / 预览图), 不打扰 pla-ui。
//! - 写 / 出图: pla-mcp 写 `req-<id>.json`, pla-ui 下一帧处理、写 `resp-<id>.json` —— 改动都经过 pla-ui, 它内存里的设置不会被外面覆盖。

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub fn dir() -> PathBuf {
    crate::prefs::data_dir().join("mcp")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PartInfo {
    pub key: String,
    #[serde(default)]
    pub pn: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub qty: u64,
    #[serde(default)]
    pub material: String,
    /// 外发归哪家(供应商 id / "buy" / 空)。
    #[serde(default)]
    pub supplier: String,
    /// 外购 / 标准件(批量不出图)。
    #[serde(default)]
    pub bought: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct State {
    pub doc_key: String,
    pub drawings_dir: String,
    #[serde(default)]
    pub parts: Vec<PartInfo>,
    #[serde(default)]
    pub selected: Option<String>,
    /// 正在出的图(零件 key)+ 排队数。
    #[serde(default)]
    pub drawing: Option<String>,
    #[serde(default)]
    pub queued: usize,
    /// pla-ui 吸附着的 Plasticity 窗口(建模工具连同一个窗口; 0 = 没有)。
    #[serde(default)]
    pub hwnd: isize,
    /// 心跳(unix 秒)。
    #[serde(default)]
    pub ts: u64,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn write_atomic(p: &std::path::Path, data: &[u8]) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, p).map_err(|e| e.to_string())
}

pub fn write_state(s: &State) -> Result<(), String> {
    write_atomic(&dir().join("state.json"), &serde_json::to_vec(s).map_err(|e| e.to_string())?)
}

/// pla-ui 的状态; 心跳超过 15 秒 = pla-ui 没在跑(或卡住)。
pub fn read_state() -> Result<State, String> {
    let s: State = std::fs::read_to_string(dir().join("state.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or("PLAhelper (pla-ui) has not reported any state yet - is it running?")?;
    if now().saturating_sub(s.ts) > 15 {
        return Err("PLAhelper (pla-ui) is not running (no heartbeat for 15 s). Start it, open the document in Plasticity, then retry.".into());
    }
    Ok(s)
}

/// 交一个请求给 pla-ui, 等它回(超时报错)。
pub fn call(req: &Value, timeout: Duration) -> Result<Value, String> {
    let id = format!("{}-{}", now(), std::process::id() as u64 * 1000 + (SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_millis()).unwrap_or(0) as u64));
    let d = dir();
    write_atomic(&d.join(format!("req-{id}.json")), &serde_json::to_vec(req).map_err(|e| e.to_string())?)?;
    let resp = d.join(format!("resp-{id}.json"));
    let t0 = std::time::Instant::now();
    while t0.elapsed() < timeout {
        if let Ok(t) = std::fs::read_to_string(&resp) {
            if let Ok(v) = serde_json::from_str::<Value>(&t) {
                let _ = std::fs::remove_file(&resp);
                return Ok(v);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = std::fs::remove_file(d.join(format!("req-{id}.json")));
    Err("PLAhelper did not answer in time (is its window frozen?)".into())
}

/// pla-ui 这边: 取走全部待处理请求(读完就删)。
pub fn take_requests() -> Vec<(String, Value)> {
    let Ok(rd) = std::fs::read_dir(dir()) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(id) = name.strip_prefix("req-").and_then(|n| n.strip_suffix(".json")) else { continue };
        if let Ok(t) = std::fs::read_to_string(e.path()) {
            if let Ok(v) = serde_json::from_str::<Value>(&t) {
                let _ = std::fs::remove_file(e.path());
                out.push((id.to_string(), v));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn respond(id: &str, v: &Value) {
    let _ = write_atomic(&dir().join(format!("resp-{id}.json")), &serde_json::to_vec(v).unwrap_or_default());
}

/// 按 key / 料号 / 名称 / 图名(不分大小写)找零件。
pub fn resolve<'a>(parts: &'a [PartInfo], q: &str) -> Result<&'a PartInfo, String> {
    let q = q.trim();
    let ql = q.to_lowercase();
    if let Some(p) = parts.iter().find(|p| p.key == q) {
        return Ok(p);
    }
    let hits: Vec<&PartInfo> = parts.iter().filter(|p| (!p.pn.is_empty() && p.pn.to_lowercase() == ql) || p.name.to_lowercase() == ql).collect();
    match hits.len() {
        1 => Ok(hits[0]),
        0 => {
            let near: Vec<&PartInfo> = parts.iter().filter(|p| p.name.to_lowercase().contains(&ql) || p.pn.to_lowercase().contains(&ql)).collect();
            if near.len() == 1 {
                Ok(near[0])
            } else if near.is_empty() {
                Err(format!("no part matches \"{q}\" (use list_parts)"))
            } else {
                Err(format!("\"{q}\" matches {} parts: {} - be more specific or use the key", near.len(), near.iter().take(8).map(|p| format!("{} {}", p.pn, p.name).trim().to_string()).collect::<Vec<_>>().join("; ")))
            }
        }
        n => Err(format!("\"{q}\" matches {n} parts - use the key")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_parts() {
        let parts = vec![
            PartInfo { key: "k1".into(), pn: "904002001".into(), name: "Base plate".into(), ..Default::default() },
            PartInfo { key: "k2".into(), pn: "".into(), name: "Cover".into(), ..Default::default() },
            PartInfo { key: "k3".into(), pn: "".into(), name: "Cover clip".into(), ..Default::default() },
        ];
        assert_eq!(resolve(&parts, "k2").unwrap().key, "k2");
        assert_eq!(resolve(&parts, "904002001").unwrap().key, "k1");
        assert_eq!(resolve(&parts, "cover").unwrap().key, "k2", "名字完全相同优先");
        assert_eq!(resolve(&parts, "base").unwrap().key, "k1", "唯一的部分匹配");
        assert!(resolve(&parts, "co").unwrap_err().contains("2 parts"));
        assert!(resolve(&parts, "zzz").is_err());
    }
}
