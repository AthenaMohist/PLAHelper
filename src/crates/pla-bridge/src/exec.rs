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
    // H7: 每次调用一个 token; 回包丢了(CDP 报错 / 解析不了)→ 按 token 取回页面上记下的实际结果, 不让调用方以为没做而重做一遍
    let token = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let mut s = script.clone();
    s["token"] = serde_json::json!(token.to_string());
    let first = match eval_roundtrip_timeout(ws, mid, &shim::call_expr(api::CMD_EXEC, &s), PER_STEP * steps + Duration::from_secs(5)).await {
        Ok(raw) => shim::parse_reply(&raw),
        Err(e) => Err(ShimErr::Bad(format!("{e:#}"))),
    };
    match first {
        Err(ShimErr::Bad(why)) => match recover(ws, mid, &token.to_string()).await {
            Some(mut v) => {
                v["recovered"] = serde_json::json!(why);
                Ok(v)
            }
            None => Err(ShimErr::Bad(format!(
                "{why} — {}",
                crate::t!(
                    "回包丢了, 操作可能已经生效: 先用 model_state / model_outline 核实, 不要直接重试",
                    "reply lost; the operation may have been applied: check with model_state / model_outline before retrying"
                )
            ))),
        },
        other => other,
    }
}

/// H7: 取回页面上记下的最近一次 cmd.exec 结果(token 对得上才算)。
async fn recover(ws: &mut WsStream, mid: &mut u64, token: &str) -> Option<Value> {
    let raw = eval_roundtrip(ws, mid, &shim::call_expr(api::CMD_LAST_RESULT, &Value::Null)).await.ok()?;
    let last = shim::parse_reply(&raw).ok()?;
    (last["token"].as_str() == Some(token)).then(|| last["result"].clone())
}

/// H5: 工厂快照失效的症状(2026-11-21 外部反馈, 撤销之后出现): 建模命令构造时读 `xxx.Viewport` 拿到 undefined。
/// 页面上的 `__PLA_FACTORIES__` 还在, 所以 `ensure_factories(false)` 会直接返回缓存 —— 必须 `force = true` 重拍。
pub fn stale_factories(msg: &str) -> bool {
    msg.contains("reading 'Viewport'") || msg.contains("reading \"Viewport\"")
}

/// H5: 这个结果要不要先挂 / 重拍几何工厂再重试一次? `Some(false)` = 还没挂; `Some(true)` = 快照失效, 强制重拍。
/// 只在"什么都没留下"时重试: 抛异常(hole.make 失败时已整体撤回), 或 `cmd.exec` 回包里第一步就失败 / 已撤回。
/// 部分步骤已生效且没撤回的不重试(重试会把已完成的步骤再做一遍)。
pub fn factory_retry(r: &Result<Value, ShimErr>) -> Option<bool> {
    match r {
        Err(ShimErr::Exc(m)) if m == shim::NEED_FACTORIES_ERR => Some(false),
        Err(ShimErr::Exc(m)) if stale_factories(m) => Some(true),
        Ok(v) => {
            let err = v["failed"]["err"].as_str().unwrap_or("");
            let nothing_left = v["results"].as_array().map(|a| a.is_empty()).unwrap_or(true) || v.get("rolledBack").map(|x| !x.is_null()).unwrap_or(false);
            (stale_factories(err) && nothing_left).then_some(true)
        }
        _ => None,
    }
}

/// 同上, 给拿到 `anyhow::Error` 的调用方(CDP 会话)用。
pub fn factory_retry_any(r: &anyhow::Result<Value>) -> Option<bool> {
    match r {
        Ok(v) => factory_retry(&Ok(v.clone())),
        Err(e) => match e.downcast_ref::<ShimErr>() {
            Some(ShimErr::Exc(m)) => factory_retry(&Err(ShimErr::Exc(m.clone()))),
            _ => None,
        },
    }
}

/// 挂 / 重拍工厂并记日志(force = 快照失效后的自愈)。
pub async fn heal_factories(ws: &mut WsStream, mid: &mut u64, force: bool) -> anyhow::Result<Value> {
    let r = ensure_factories(ws, mid, force).await?;
    eprintln!("[pla-exec] 工厂类{}: {r}", if force { "已重拍(快照失效自愈)" } else { "已挂上" });
    Ok(r)
}

/// 执行脚本; 工厂类还没挂上就先让代理挂一次, 快照失效就强制重拍, 再重试一次。返回 `cmd.exec` 的 v(某步失败时 failed 非空, 仍是 Ok)。
pub async fn run(ws: &mut WsStream, mid: &mut u64, script: &Value) -> Result<Value, String> {
    let first = call_exec(ws, mid, script).await;
    match factory_retry(&first) {
        Some(force) => {
            heal_factories(ws, mid, force).await.map_err(|e| format!("{e:#}"))?;
            call_exec(ws, mid, script).await.map_err(|e| e.to_string())
        }
        None => first.map_err(|e| e.to_string()),
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
            // atomic: 已完成的步骤被撤回(或撤回失败)
            if let Some(n) = v["rolledBack"].as_u64() {
                return (false, crate::t!(
                    "命令派发: 第 {i} 步({op})失败: {err}; 已撤回本次全部 {n} 步, 文档回到执行前",
                    "Command dispatch: step {i} ({op}) failed: {err}; rolled back all {n} steps, document as before"
                ));
            }
            if let Some(re) = v["rollbackErr"].as_str() {
                return (false, crate::t!(
                    "命令派发: 第 {i} 步({op})失败: {err}; 撤回也失败({re}), 已完成的 {done} 步还在文档里",
                    "Command dispatch: step {i} ({op}) failed: {err}; rollback failed too ({re}), {done} completed steps remain"
                ));
            }
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
        let rb = json!({"results":[],"failed":{"index":2,"op":"fillet","err":"PK"},"rolledBack":2});
        let (ok, msg) = summary(&rb);
        assert!(!ok && msg.contains("撤回") && msg.contains('2'), "{msg}");
        let rbe = json!({"results":[{},{}],"failed":{"index":2,"op":"fillet","err":"PK"},"rollbackErr":"window hidden"});
        assert!(summary(&rbe).1.contains("window hidden"));
    }

    #[test]
    fn factory_retry_rules() {
        let exc = |m: &str| Err(ShimErr::Exc(m.to_string()));
        assert_eq!(factory_retry(&exc(shim::NEED_FACTORIES_ERR)), Some(false));
        assert_eq!(factory_retry(&exc("step 0 (cylinder): Cannot read properties of undefined (reading 'Viewport') — rolled back")), Some(true));
        assert_eq!(factory_retry(&exc("solid not found: 12")), None);
        // cmd.exec 回包: 第一步就失败 / 已撤回 → 重拍重试; 部分步骤已生效且没撤回 → 不重试
        let v = |results: Value, rolled: Value| Ok(json!({"results": results, "failed": {"index": 0, "op": "cylinder", "err": "Cannot read properties of undefined (reading 'Viewport')"}, "rolledBack": rolled}));
        assert_eq!(factory_retry(&v(json!([]), Value::Null)), Some(true));
        assert_eq!(factory_retry(&v(json!([{"index": 0}]), json!(1))), Some(true));
        assert_eq!(factory_retry(&v(json!([{"index": 0}]), Value::Null)), None, "非原子、已有步骤生效: 不重试");
        assert_eq!(factory_retry(&Ok(json!({"results": [], "failed": null}))), None);
        let any: anyhow::Result<Value> = Err(anyhow::Error::new(ShimErr::Exc("x (reading 'Viewport')".into())));
        assert_eq!(factory_retry_any(&any), Some(true));
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
