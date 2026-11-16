//! D: 从 Plasticity 导出 STEP(出图用)。
//!
//! Plasticity 的导出入口 `Editor.export()` 没有参数: 渲染进程经 IPC `show-save-dialog` 让主进程弹系统"另存为",
//! 再按扩展名弹站内对话框(STEP: 格式 AP203/214/242、解析面转 NURBS、OK), 最后由它自带的写出器写文件
//! (导出范围 = 当前选择, 没选 = 全部可见)。我们不碰写出器, 只把"另存为"换成一次性自动应答:
//!
//! 1. 经主进程 inspector(9334)给 `dialog.showSaveDialog` 装一次性包装 —— 只应答绑定窗口的下一次调用,
//!    用一次或超时就自己还原, 其它窗口 / 之后的调用照常弹系统对话框;
//! 2. 垫片 `file.exportStep`: 记下用户选择 → 按稳定 id 选上零件 → `Editor.export()` → 站内对话框选格式、点 OK;
//! 3. 等文件落盘(写文件期间动选择会取消导出命令);
//! 4. 垫片 `file.exportRestore` 还原用户选择。
//!
//! 武装失败就不调导出 —— 否则会给用户弹出一个真的"另存为"。
use serde_json::Value;
use std::time::Duration;

/// Plasticity 主进程 inspector 端口(补丁版以 `--inspect=127.0.0.1:9334` 启动)。
pub const INSPECT_PORT: u16 = 9334;
/// 一次性应答的有效期: 超时没被用掉就还原。
pub const ARM_TIMEOUT_MS: u64 = 20_000;
/// 等文件落盘的上限。
pub const WRITE_WAIT: Duration = Duration::from_secs(60);

/// 主进程里执行的武装脚本: 下一次(且只一次)绑定窗口的 `dialog.showSaveDialog` 直接回 `{canceled:false, filePath}`。
/// `hwnd` = 绑定窗口(十进制 HWND, 0 = 不限窗口)。重复武装先还原上一次。
pub fn arm_save_dialog_js(hwnd: isize, path: &str, timeout_ms: u64) -> String {
    let want = if hwnd == 0 { String::new() } else { hwnd.to_string() };
    format!(
        r#"(() => {{
  const {{ dialog }} = process.mainModule.require('electron');
  const want = {want}, filePath = {path}, ms = {ms};
  const hwndOf = (w) => {{ const b = w.getNativeWindowHandle(); return (b.length >= 8 ? b.readBigUInt64LE(0) : BigInt(b.readUInt32LE(0))).toString(); }};
  const G = global.__PLA_SAVE__ || (global.__PLA_SAVE__ = {{}});
  if (typeof G.restore === 'function') G.restore();
  const orig = dialog.showSaveDialog;
  const wrap = function (win, opts) {{
    const isWin = win && typeof win.getNativeWindowHandle === 'function';
    if (want && (!isWin || hwndOf(win) !== want)) return orig.apply(this, arguments);
    restore();
    G.last = {{ at: Date.now(), title: (isWin ? opts : win) && (isWin ? opts : win).title }};
    return Promise.resolve({{ canceled: false, filePath }});
  }};
  const restore = () => {{ if (dialog.showSaveDialog === wrap) dialog.showSaveDialog = orig; clearTimeout(G.timer); G.restore = null; }};
  dialog.showSaveDialog = wrap;
  G.restore = restore;
  G.timer = setTimeout(restore, ms);
  return 'armed';
}})()"#,
        want = serde_json::to_string(&want).unwrap(),
        path = serde_json::to_string(path).unwrap(),
        ms = timeout_ms
    )
}

/// 撤销武装(导出失败时调用, 免得之后用户自己的"另存为"被吞掉)。
pub const DISARM_JS: &str = "(() => { const G = global.__PLA_SAVE__; if (G && typeof G.restore === 'function') { G.restore(); return 'disarmed'; } return 'idle'; })()";

/// 在 Plasticity 主进程里执行一个表达式(一次性连接 inspector 的 node 目标), 返回值(returnByValue)。
pub async fn main_eval_once(port: u16, expr: &str) -> anyhow::Result<Value> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let body = crate::cdp::http_get(port, "/json/list").await?;
    let v: Value = serde_json::from_str(&body)?;
    let node = v
        .as_array()
        .and_then(|a| a.iter().find(|t| t["type"] == "node"))
        .ok_or_else(|| anyhow::anyhow!("no node target on inspector port {port}"))?;
    let ws_url = node["webSocketDebuggerUrl"].as_str().ok_or_else(|| anyhow::anyhow!("no ws url"))?.to_string();
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await?;
    let req = serde_json::json!({"id": 1, "method": "Runtime.evaluate",
        "params": {"expression": expr, "returnByValue": true, "awaitPromise": true}});
    ws.send(Message::Text(req.to_string())).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let msg = tokio::time::timeout_at(deadline, ws.next())
            .await
            .map_err(|_| anyhow::anyhow!("main eval timeout"))?
            .ok_or_else(|| anyhow::anyhow!("ws closed"))?
            .map_err(|e| anyhow::anyhow!("ws: {e}"))?;
        if let Message::Text(t) = msg {
            let r: Value = serde_json::from_str(&t)?;
            if r["id"].as_u64() == Some(1) {
                let _ = ws.close(None).await;
                if let Some(exc) = r["result"]["exceptionDetails"]["exception"]["description"].as_str() {
                    return Err(anyhow::anyhow!("main js: {exc}"));
                }
                if let Some(t) = r["result"]["exceptionDetails"]["text"].as_str() {
                    return Err(anyhow::anyhow!("main js: {t}"));
                }
                return Ok(r["result"]["result"]["value"].clone());
            }
        }
    }
}

/// 导出参数: 路径必须是 .step / .stp(Plasticity 按扩展名决定格式与站内对话框)。
pub fn check_path(path: &str) -> Result<(), String> {
    let low = path.to_lowercase();
    if !(low.ends_with(".step") || low.ends_with(".stp")) {
        return Err(format!("export path must end with .step / .stp: {path}"));
    }
    if !std::path::Path::new(path).is_absolute() {
        return Err(format!("export path must be absolute: {path}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm_js_escapes_and_scopes() {
        let js = arm_save_dialog_js(3150394, r"C:\a b\零件 1.step", 20000);
        assert!(js.contains(r#"want = "3150394""#), "{js}");
        assert!(js.contains(r#"filePath = "C:\\a b\\零件 1.step""#), "路径按 JSON 转义: {js}");
        assert!(js.contains("ms = 20000"));
        assert!(js.contains("orig.apply(this, arguments)"), "别的窗口照常弹");
        let any = arm_save_dialog_js(0, r"C:\x.step", 1);
        assert!(any.contains(r#"want = """#), "0 = 不限窗口");
    }

    #[test]
    fn path_rules() {
        assert!(check_path(r"C:\out\a.step").is_ok());
        assert!(check_path(r"C:\out\a.STP").is_ok());
        assert!(check_path(r"C:\out\a.obj").is_err());
        assert!(check_path(r"out\a.step").is_err());
    }
}
