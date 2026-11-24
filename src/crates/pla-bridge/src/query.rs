//! A1: 几何查询 / 文档状态(只读; 设计见 docs/PRD/PRD-ai-modeling-v2.md §2)。
//!
//! `state` = 一次批量调用: 撤销深度 + 当前选择 + 几何查询(`geo.query`), 合成一份给大模型读的 JSON。
//! 全部只读, 不改文档、不改选择、不动视角。

use std::time::Duration;

use serde_json::{json, Value};

use crate::cdp::{connect_page, eval_roundtrip, WsStream};
use crate::shim::{self, api};

/// 文档状态: `{units, history, selection, geometry}`。`geo_args` = `geo.query` 的参数(detail / of / units / 过滤)。
/// 某一项失败不影响其它项: 失败项写成 `{"err": "..."}`。
pub async fn state(ws: &mut WsStream, mid: &mut u64, geo_args: &Value) -> anyhow::Result<Value> {
    let calls = [(api::HIST_STATE, Value::Null), (api::SEL_SNAPSHOT, Value::Null), (api::GEO_QUERY, geo_args.clone())];
    let raw = eval_roundtrip(ws, mid, &shim::batch_expr(&calls)).await?;
    let names: Vec<&str> = calls.iter().map(|c| c.0).collect();
    let mut parts = shim::parse_batch(&raw, &names).into_iter().map(|(_, r)| match r {
        Ok(v) => v,
        Err(e) => json!({ "err": e.to_string() }),
    });
    let (history, selection, geometry) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
    Ok(json!({
        "units": geometry.get("units").cloned().unwrap_or(Value::Null),
        "history": history,
        "selection": selection,
        "geometry": geometry,
    }))
}

/// CLI 用: 连指定窗口(`hwnd` 0 = 任意) → 注入垫片 → 读状态。
pub async fn state_once(cdp_port: u16, hwnd: isize, geo_args: &Value) -> anyhow::Result<Value> {
    let mut ws = tokio::time::timeout(Duration::from_secs(8), connect_page(cdp_port, hwnd))
        .await
        .map_err(|_| anyhow::anyhow!("connect timeout (8s)"))??;
    let mut mid: u64 = 100;
    eval_roundtrip(&mut ws, &mut mid, &shim::install_expr()).await?;
    state(&mut ws, &mut mid, geo_args).await
}

/// CLI 用: 调一个垫片接口(名字必须在 `shim::api::ALL` 里; 异步接口同样等结果)。
pub async fn call_once(cdp_port: u16, hwnd: isize, name: &str, args: &Value) -> anyhow::Result<Value> {
    if !api::ALL.contains(&name) {
        anyhow::bail!("unknown shim api {name} (known: {})", api::ALL.join(", "));
    }
    let mut ws = tokio::time::timeout(Duration::from_secs(8), connect_page(cdp_port, hwnd))
        .await
        .map_err(|_| anyhow::anyhow!("connect timeout (8s)"))??;
    let mut mid: u64 = 100;
    eval_roundtrip(&mut ws, &mut mid, &shim::install_expr()).await?;
    let raw = eval_roundtrip(&mut ws, &mut mid, &shim::call_expr(name, args)).await?;
    let first = shim::parse_reply(&raw);
    // H5: 要用几何工厂的接口(hole.make 等)第一次用 / 快照失效 → 挂 / 重拍后重试一次(以前只有 model_exec 会)
    if let Some(force) = crate::exec::factory_retry(&first) {
        crate::exec::heal_factories(&mut ws, &mut mid, force).await?;
        let raw = eval_roundtrip(&mut ws, &mut mid, &shim::call_expr(name, args)).await?;
        return Ok(shim::parse_reply(&raw)?);
    }
    Ok(first?)
}

/// D CLI 用: 导出 STEP(全流程见 `export.rs`)。ids = 稳定 id(空 = 全部可见)。
pub async fn export_step_once(cdp_port: u16, hwnd: isize, path: &str, ids: &[String], format: Option<&str>) -> anyhow::Result<Value> {
    let mut ws = tokio::time::timeout(Duration::from_secs(8), connect_page(cdp_port, hwnd))
        .await
        .map_err(|_| anyhow::anyhow!("connect timeout (8s)"))??;
    let mut mid: u64 = 100;
    eval_roundtrip(&mut ws, &mut mid, &shim::install_expr()).await?;
    crate::cdp::export_step(&mut ws, &mut mid, hwnd, path, ids, format, crate::export::inspect_port_for(cdp_port)).await.map_err(|e| anyhow::anyhow!(e))
}

/// N5b MCP 用: 建一个 asset pack(全流程见 cdp::build_pack)。
pub async fn build_pack_once(cdp_port: u16, hwnd: isize, items: &[Value], path: &str) -> anyhow::Result<Value> {
    let mut ws = tokio::time::timeout(Duration::from_secs(8), connect_page(cdp_port, hwnd))
        .await
        .map_err(|_| anyhow::anyhow!("connect timeout (8s)"))??;
    let mut mid: u64 = 100;
    eval_roundtrip(&mut ws, &mut mid, &shim::install_expr()).await?;
    crate::cdp::build_pack(&mut ws, &mut mid, hwnd, items, path, crate::export::inspect_port_for(cdp_port)).await.map_err(|e| anyhow::anyhow!(e))
}
/// A5 CLI 用: 指定视角截图 → (PNG 字节, 回包去掉 data 后的其余字段)。
/// 分三步(实测 2026-11-17, 窗口可见时): 摆相机(cam.view shot:false)→ 隔几帧再单独截(cam.thumbnail)→ 还原(cam.viewRestore)。
/// 同一个同步调用里摆完立刻截, 画面还是旧相机。还原一定做(截图失败也还原)。
pub async fn view_png_once(cdp_port: u16, hwnd: isize, args: &Value) -> anyhow::Result<(Vec<u8>, Value)> {
    let mut ws = tokio::time::timeout(Duration::from_secs(8), connect_page(cdp_port, hwnd))
        .await
        .map_err(|_| anyhow::anyhow!("connect timeout (8s)"))??;
    let mut mid: u64 = 100;
    eval_roundtrip(&mut ws, &mut mid, &shim::install_expr()).await?;
    // H10: isolate = 先用 Plasticity 自己的隔离只留 of 里的实体; 截完一定退出隔离 + 恢复选择(截图失败也退)
    let iso = if args["isolate"].as_bool() == Some(true) {
        if args["of"].is_null() {
            anyhow::bail!("isolate needs of (the solids to show)");
        }
        let raw = eval_roundtrip(&mut ws, &mut mid, &shim::call_expr(api::VIEW_ISOLATE, &json!({ "of": args["of"] }))).await?;
        Some(shim::parse_reply(&raw)?)
    } else {
        None
    };
    let shot = view_png_inner(&mut ws, &mut mid, args).await;
    if let Some(iso) = iso {
        let back = json!({ "to": iso["start"], "prev": iso["prev"] });
        if let Err(e) = eval_roundtrip(&mut ws, &mut mid, &shim::call_expr(api::VIEW_UNISOLATE, &back)).await {
            eprintln!("[pla-view] unisolate failed: {e:#}");
        }
    }
    shot
}

async fn view_png_inner(ws: &mut WsStream, mid: &mut u64, args: &Value) -> anyhow::Result<(Vec<u8>, Value)> {
    let (mut ws, mut mid) = (ws, mid);
    let mut a = args.clone();
    a.as_object_mut().map(|o| o.remove("isolate"));
    a["shot"] = json!(false);
    let raw = eval_roundtrip(&mut ws, &mut mid, &shim::call_expr(api::CAM_VIEW, &a)).await?;
    let mut v = shim::parse_reply(&raw)?;
    let width = args.get("maxWidth").cloned().unwrap_or(json!(960));
    // 截两次取第二张: 垫片刚注入 / 刚切正交后的头一张偶尔还是旧画面(实测 2026-11-18, 同参数第二次总是对的)
    let mut shot = Err(anyhow::anyhow!("no capture"));
    for _ in 0..2 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        shot = match eval_roundtrip(&mut ws, &mut mid, &shim::call_expr(api::CAM_THUMBNAIL, &json!({ "maxWidth": width }))).await {
            Ok(raw) => shim::parse_reply(&raw).map_err(|e| anyhow::anyhow!(e)),
            Err(e) => Err(e),
        };
    }
    let restore = json!({ "prev": v["prev"].clone(), "toggled": v["toggled"].clone() });
    let restored = eval_roundtrip(&mut ws, &mut mid, &shim::call_expr(api::CAM_VIEW_RESTORE, &restore)).await.is_ok();
    let shot = shot?;
    let b64 = shot["data"].as_str().ok_or_else(|| anyhow::anyhow!("cam.thumbnail: no image data"))?.to_string();
    if let Some(o) = v.as_object_mut() {
        o.remove("prev");
        o.remove("toggled");
        o.insert("width".into(), shot["width"].clone());
        o.insert("height".into(), shot["height"].clone());
        o.insert("restored".into(), json!(restored));
    }
    Ok((crate::cdp::base64_decode(&b64)?, v))
}

/// 由 CLI 参数拼 `geo.query` 的参数: detail / of(逗号分隔的物体 id) / units。
pub fn geo_args(detail: Option<&str>, of: Option<&str>, units: Option<&str>) -> Value {
    let mut a = serde_json::Map::new();
    if let Some(d) = detail {
        a.insert("detail".into(), json!(d));
    }
    if let Some(o) = of {
        let ids: Vec<Value> = o
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<i64>().map(Value::from).unwrap_or_else(|_| json!({ "name": s })))
            .collect();
        a.insert("of".into(), Value::Array(ids));
    }
    if let Some(u) = units {
        a.insert("units".into(), json!(u));
    }
    Value::Object(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geo_args_from_cli() {
        assert_eq!(geo_args(None, None, None), json!({}));
        assert_eq!(
            geo_args(Some("faces"), Some("34, Box.007"), Some("meter")),
            json!({ "detail": "faces", "of": [34, { "name": "Box.007" }], "units": "meter" }),
            "数字按物体 id, 其余按大纲名"
        );
    }
}
