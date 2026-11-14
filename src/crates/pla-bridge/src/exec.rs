//! N2: 命令派发(设计与实测见 docs/know-how/command-dispatch.md)。
//!
//! 脚本 = 垫片 `cmd.exec` 的参数 `{ops: [{op, id?, <参数>…, bool?, targets?}], units?}`, 每步一个撤销步。
//! 几何工厂类只在 Plasticity 的闭包里: 第一次用时垫片报 [`shim::NEED_FACTORIES_ERR`] →
//! 让代理拍一次堆快照把工厂类挂到页面上([`shim::PROXY_EXPOSE_CLOSURES`]) → 重试。
//! 挂上的类在页面重载前一直有效, 之后的调用不再拍快照。

use std::time::Duration;

use serde_json::Value;

use crate::cdp::{connect_page, eval_roundtrip, eval_roundtrip_timeout, ws_send, WsStream};
use crate::shim::{self, api, ShimErr};

/// 代理拍快照 + 复活的超时(实测 62 MB 快照 ~5 s; 大文档留足余量)。
const EXPOSE_TIMEOUT: Duration = Duration::from_secs(120);

/// 每步的超时上限(垫片里单步 15 s)再加一点余量。
const PER_STEP: Duration = Duration::from_secs(16);

/// 发一个原始 CDP 方法并等对应 id 的回包; 返回 result(带 error 时报错)。
pub(crate) async fn cdp_call(
    ws: &mut WsStream,
    mid: &mut u64,
    method: &str,
    params: Value,
    timeout: Duration,
) -> anyhow::Result<Value> {
    use futures_util::StreamExt;
    *mid += 1;
    let want = *mid;
    ws_send(ws, serde_json::json!({ "id": want, "method": method, "params": params })).await?;
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let msg = tokio::time::timeout_at(deadline, ws.next())
            .await
            .map_err(|_| anyhow::anyhow!("{method}: timeout"))?
            .ok_or_else(|| anyhow::anyhow!("ws closed"))?
            .map_err(|e| anyhow::anyhow!("ws read: {e}"))?;
        if let tokio_tungstenite::tungstenite::Message::Text(t) = msg {
            let v: Value = serde_json::from_str(&t)?;
            if v["id"].as_u64() == Some(want) {
                if let Some(m) = v["error"]["message"].as_str() {
                    anyhow::bail!("{method}: {m}");
                }
                if let Some(m) = v["result"]["__pla_err"].as_str() {
                    anyhow::bail!("{method}: {m}");
                }
                return Ok(v["result"].clone());
            }
        }
    }
}

/// 让代理把几何工厂类挂到页面上(已挂过且 `force=false` 时直接返回 cached)。
/// 回包 `{cached, count, found?, bad?, ms, bytes?}`。旧代理不认这个方法 → 报错并提示重启代理。
pub async fn ensure_factories(ws: &mut WsStream, mid: &mut u64, force: bool) -> anyhow::Result<Value> {
    let r = cdp_call(ws, mid, shim::PROXY_EXPOSE_CLOSURES, shim::expose_closures_params(force), EXPOSE_TIMEOUT).await?;
    // 旧代理把未知方法转发给 renderer, 回一个空/错误结果, 没有 count
    if r["count"].as_u64().is_none() {
        anyhow::bail!(
            "{}",
            crate::t!(
                "CDP 代理不支持 {m}(代理版本过旧, 请在 pla-ui 设置里重启 CDP 代理): {r}",
                "CDP proxy does not support {m} (proxy too old; restart it from pla-ui settings): {r}",
                m = shim::PROXY_EXPOSE_CLOSURES
            )
        );
    }
    if r["count"].as_u64() == Some(0) {
        anyhow::bail!("{}", crate::t!("堆快照里没找到几何工厂类: {r}", "No geometry factory classes found in heap snapshot: {r}"));
    }
    Ok(r)
}

async fn call_exec(ws: &mut WsStream, mid: &mut u64, script: &Value) -> Result<Value, ShimErr> {
    let steps = script["ops"].as_array().map(|a| a.len()).unwrap_or(1).max(1) as u32;
    let raw = eval_roundtrip_timeout(ws, mid, &shim::call_expr(api::CMD_EXEC, script), PER_STEP * steps + Duration::from_secs(5))
        .await
        .map_err(|e| ShimErr::Bad(format!("{e:#}")))?;
    shim::parse_reply(&raw)
}

/// 执行脚本; 工厂类还没挂上就先让代理挂一次再重试。返回 `cmd.exec` 的 v(某步失败时 failed 非空, 仍是 Ok)。
pub async fn run(ws: &mut WsStream, mid: &mut u64, script: &Value) -> Result<Value, String> {
    match call_exec(ws, mid, script).await {
        Err(ShimErr::Exc(m)) if m == shim::NEED_FACTORIES_ERR => {
            let r = ensure_factories(ws, mid, false).await.map_err(|e| format!("{e:#}"))?;
            eprintln!("[pla-exec] 工厂类已挂上: {r}");
            call_exec(ws, mid, script).await.map_err(|e| e.to_string())
        }
        other => other.map_err(|e| e.to_string()),
    }
}

/// CLI 用: 连指定窗口(`hwnd` 0 = 任意) → 注入垫片 → 执行脚本。
pub async fn run_once(cdp_port: u16, hwnd: isize, script: &Value) -> anyhow::Result<Value> {
    let mut ws = tokio::time::timeout(Duration::from_secs(8), connect_page(cdp_port, hwnd))
        .await
        .map_err(|_| anyhow::anyhow!("connect timeout (8s)"))??;
    let mut mid: u64 = 100;
    eval_roundtrip(&mut ws, &mut mid, &shim::install_expr()).await?;
    run(&mut ws, &mut mid, script).await.map_err(|e| anyhow::anyhow!(e))
}

/// `cmd.exec` 回包 → (是否全部完成, 状态栏文字)。
pub fn summary(v: &Value) -> (bool, String) {
    let done = v["results"].as_array().map(|a| a.len()).unwrap_or(0);
    let made: usize = v["results"]
        .as_array()
        .map(|a| a.iter().map(|r| r["items"].as_array().map(|i| i.len()).unwrap_or(0)).sum())
        .unwrap_or(0);
    match v.get("failed").filter(|f| !f.is_null()) {
        None => (true, crate::t!("命令派发: {done} 步全部完成, 产出 {made} 个物体", "Command dispatch: all {done} steps done, {made} objects produced")),
        Some(f) => {
            let i = f["index"].as_u64().unwrap_or(0) + 1;
            let op = f["op"].as_str().unwrap_or("?");
            let err = f["err"].as_str().unwrap_or("");
            (false, crate::t!(
                "命令派发: 完成 {done} 步, 第 {i} 步({op})失败: {err}",
                "Command dispatch: {done} steps done, step {i} ({op}) failed: {err}"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summary_reports_partial_failure_as_is() {
        let all = json!({"units":"millimeter","results":[{"index":0,"op":"box","items":[{"kind":"solid","id":3}]},{"index":1,"op":"box","items":[{"kind":"solid","id":4}]}],"failed":null});
        let (ok, msg) = summary(&all);
        assert!(ok);
        assert!(msg.contains('2'), "{msg}");
        let part = json!({"results":[{"index":0,"op":"box","items":[{}]}],"failed":{"index":1,"op":"fillet","err":"edge near [1,2,3]: none within 0.5"}});
        let (ok, msg) = summary(&part);
        assert!(!ok);
        assert!(msg.contains("fillet") && msg.contains("none within"), "{msg}");
        assert!(msg.contains('2'), "第 2 步(从 1 数): {msg}");
    }

    #[test]
    fn expose_params_come_from_shim_constants() {
        let p = shim::expose_closures_params(false);
        assert_eq!(p["global"], shim::FACTORIES_GLOBAL);
        assert_eq!(p["pattern"], shim::FACTORY_CLASS_PATTERN);
        assert_eq!(p["force"], false);
        assert_eq!(shim::config_json()["factoriesGlobal"], shim::FACTORIES_GLOBAL, "垫片与代理用同一个全局名");
        assert_eq!(shim::config_json()["needFactoriesErr"], shim::NEED_FACTORIES_ERR);
    }
}
