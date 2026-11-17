//! pla-mcp: PLAhelper 的 MCP 服务器(stdio, JSON-RPC 2.0, 一行一条)。给 Claude Code 等外部 agent 用:
//! 看零件清单 / 某张图的设置 + 报告 + 预览图, 改出图设置(可一次改多张), 出图, 记长期规则, 在 pla-ui 里打开某个零件。
//!
//! 接入 Claude Code:  claude mcp add plahelper -- "<repo>\src\target\release\pla-mcp.exe"
//! 读直接读文件; 改 / 出图交给正在跑的 pla-ui(见 pla_ui::mcp_bridge)。

use std::io::{BufRead, Write};
use std::time::Duration;

use pla_ui::draw_agent;
use pla_ui::drawing;
use pla_ui::mcp_bridge::{self as bridge, PartInfo};
use serde_json::{json, Value};

const SUPPORTED: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

fn patch_schema() -> Value {
    let mut s = draw_agent::tools()[0]["input_schema"].clone();
    s["properties"]["parts"] = json!({"type": "array", "items": {"type": "string"}, "description": "Parts to change: key, part number or name (one sentence can change many sheets)"});
    s["properties"]["redraw"] = json!({"type": "boolean", "description": "Recompile the changed sheets afterwards (default false)"});
    s["required"] = json!(["parts", "summary"]);
    s
}

fn tools() -> Value {
    let part = json!({"type": "object", "properties": {"part": {"type": "string", "description": "Part key, part number or name"}}, "required": ["part"]});
    json!([
        {"name": "status", "description": "PLAhelper status: is pla-ui running, current Plasticity document, selected part, drawing job.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "list_parts", "description": "Parts in the current BOM scope with part number, name, quantity, material, supplier and drawing status (drawn / uploaded+locked / model changed / not drawn).", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "get_drawing", "description": "One part's drawing: current settings, the last compile report (dimension names and values, hole groups, warnings) and the preview image of the sheet. Use the dimension names from here in apply_drawing_changes.", "inputSchema": part},
        {"name": "apply_drawing_changes", "description": "Apply a settings patch to one or more part drawings (orientation, section, hole positions, threads, density, hide/show dims, key dims, dim text, tolerances, technical requirements, title block). Applies immediately - confirm with the user first when the request was ambiguous. Locked parts (uploaded drawing) are skipped.", "inputSchema": patch_schema()},
        {"name": "draw", "description": "Compile drawings in PLAhelper (re-exports STEP from Plasticity when needed; ~20-90 s per sheet). parts = list, or [\"undrawn\"] for every part not drawn yet or changed since. Optionally waits.", "inputSchema": {"type": "object", "properties": {
            "parts": {"type": "array", "items": {"type": "string"}},
            "export": {"type": "boolean", "description": "Force STEP re-export (after the model changed)"},
            "wait_seconds": {"type": "integer", "description": "Wait up to this long for the sheets to finish (0 = return at once, max 600)"}
        }, "required": ["parts"]}},
        {"name": "list_rules", "description": "The user's standing drawing rules (applied by the in-app assistant to every drawing).", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "remember_rule", "description": "Save a standing drawing rule for all future drawings (only when the user states a general preference).", "inputSchema": {"type": "object", "properties": {"rule": {"type": "string"}}, "required": ["rule"]}},
        {"name": "show_part", "description": "Open the part in PLAhelper's drawing tab so the user sees its sheet.", "inputSchema": part}
    ])
}

fn text(t: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}]})
}

fn err(t: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}], "isError": true})
}

/// pla-ui 的状态; 零件清单空(还没扫过 BOM)→ 让它扫一次(只读), 最多等 30 s。
fn state() -> Result<bridge::State, String> {
    let st = bridge::read_state()?;
    if !st.parts.is_empty() {
        return Ok(st);
    }
    bridge::call(&json!({"op": "scan"}), Duration::from_secs(10))?;
    let t0 = std::time::Instant::now();
    while t0.elapsed() < Duration::from_secs(30) {
        std::thread::sleep(Duration::from_millis(500));
        let s = bridge::read_state()?;
        if !s.parts.is_empty() {
            return Ok(s);
        }
    }
    Ok(bridge::read_state()?)
}

fn sheets(st: &bridge::State) -> drawing::Sheets {
    drawing::load_in(std::path::Path::new(&st.drawings_dir))
}

fn status_of(sh: &drawing::Sheets, key: &str) -> String {
    match sh.get(key) {
        Some(s) if s.locked() => "uploaded (locked)".into(),
        Some(s) if s.model_changed => "model changed, redraw".into(),
        Some(s) if !s.compiled.is_empty() => format!("drawn {}", s.compiled),
        _ => "not drawn".into(),
    }
}

fn resolve_many(st: &bridge::State, qs: &[String]) -> Result<Vec<PartInfo>, String> {
    qs.iter().map(|q| bridge::resolve(&st.parts, q).cloned()).collect()
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

fn call_tool(name: &str, a: &Value) -> Value {
    let st = match state() {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let ask = |req: Value, secs: u64| bridge::call(&req, Duration::from_secs(secs));
    match name {
        "status" => text(
            json!({"document": st.doc_key, "parts": st.parts.len(), "selected": st.selected, "drawing_now": st.drawing, "queued": st.queued, "drawings_dir": st.drawings_dir}).to_string(),
        ),
        "list_parts" => {
            let sh = sheets(&st);
            let rows: Vec<Value> = st
                .parts
                .iter()
                .map(|p| json!({"key": p.key, "pn": p.pn, "name": p.name, "qty": p.qty, "material": p.material, "supplier": p.supplier, "bought_or_standard": p.bought, "drawing": status_of(&sh, &p.key)}))
                .collect();
            text(serde_json::to_string_pretty(&rows).unwrap_or_default())
        }
        "get_drawing" | "show_part" => {
            let p = match bridge::resolve(&st.parts, a["part"].as_str().unwrap_or("")) {
                Ok(p) => p.clone(),
                Err(e) => return err(e),
            };
            if name == "show_part" {
                return match ask(json!({"op": "show", "key": p.key}), 10) {
                    Ok(v) if v["ok"] == true => text(format!("Opened {} {} in PLAhelper.", p.pn, p.name)),
                    Ok(v) => err(v["error"].as_str().unwrap_or("failed").to_string()),
                    Err(e) => err(e),
                };
            }
            let sh = sheets(&st);
            let dir = std::path::Path::new(&st.drawings_dir);
            let Some(s) = sh.get(&p.key) else {
                return text(format!("{} {}: no drawing yet. Use draw to create it.", p.pn, p.name));
            };
            let rep = drawing::load_report(dir, s);
            let mut title = std::collections::BTreeMap::new();
            for (k, v) in [("pn", &p.pn), ("name", &p.name), ("material", &p.material)] {
                if !v.is_empty() {
                    title.insert(k.to_string(), v.clone());
                }
            }
            title.insert("qty".into(), p.qty.to_string());
            let mut out = draw_agent::context(s, rep.as_ref(), &title, s.locked());
            out.push_str(&format!("\n[Status] {}\n[PDF] {}", status_of(&sh, &p.key), drawing::file(dir, s, "pdf").display()));
            if let Some(r) = &rep {
                let stale = drawing::stale_refs(s, r);
                if !stale.is_empty() {
                    out.push_str(&format!("\n[Stale corrections - names not in the current report] {}", stale.join(", ")));
                }
            }
            let mut content = vec![json!({"type": "text", "text": out})];
            let png = s
                .uploaded
                .as_ref()
                .filter(|u| u.locked && !u.pages.is_empty())
                .map(|u| dir.join(&u.pages[0].png))
                .unwrap_or_else(|| drawing::file(dir, s, "png"));
            if let Ok(b) = std::fs::read(&png) {
                content.push(json!({"type": "image", "data": b64(&b), "mimeType": "image/png"}));
            }
            json!({"content": content})
        }
        "apply_drawing_changes" => {
            let qs: Vec<String> = serde_json::from_value(a["parts"].clone()).unwrap_or_default();
            let parts = match resolve_many(&st, &qs) {
                Ok(p) if !p.is_empty() => p,
                Ok(_) => return err("parts is empty"),
                Err(e) => return err(e),
            };
            let mut patch = a.clone();
            if let Some(o) = patch.as_object_mut() {
                o.remove("parts");
                o.remove("redraw");
            }
            if let Err(e) = serde_json::from_value::<draw_agent::Patch>(patch.clone()) {
                return err(format!("invalid patch: {e}"));
            }
            match ask(json!({"op": "apply", "keys": parts.iter().map(|p| &p.key).collect::<Vec<_>>(), "patch": patch, "redraw": a["redraw"].as_bool().unwrap_or(false)}), 20) {
                Ok(v) => text(serde_json::to_string_pretty(&v["results"]).unwrap_or_default()),
                Err(e) => err(e),
            }
        }
        "draw" => {
            let qs: Vec<String> = serde_json::from_value(a["parts"].clone()).unwrap_or_default();
            let sh = sheets(&st);
            let keys: Vec<String> = if qs.iter().any(|q| q == "undrawn") {
                st.parts.iter().filter(|p| !p.bought && sh.get(&p.key).map_or(true, |s| !s.locked() && (s.compiled.is_empty() || s.model_changed))).map(|p| p.key.clone()).collect()
            } else {
                match resolve_many(&st, &qs) {
                    Ok(p) => p.into_iter().map(|p| p.key).collect(),
                    Err(e) => return err(e),
                }
            };
            if keys.is_empty() {
                return text("Nothing to draw.");
            }
            let dir = std::path::PathBuf::from(&st.drawings_dir);
            let before: Vec<Option<std::time::SystemTime>> = keys.iter().map(|k| report_mtime(&dir, &sh, k)).collect();
            let r = match ask(json!({"op": "draw", "keys": keys, "export": a["export"].as_bool().unwrap_or(false)}), 20) {
                Ok(v) => v,
                Err(e) => return err(e),
            };
            if r["ok"] != true {
                return err(r["error"].as_str().unwrap_or("failed").to_string());
            }
            let wait = a["wait_seconds"].as_u64().unwrap_or(0).min(600);
            let t0 = std::time::Instant::now();
            let done = loop {
                let sh2 = drawing::load_in(&dir);
                let done: Vec<String> = keys.iter().zip(&before).filter(|(k, b)| report_mtime(&dir, &sh2, k) != **b).map(|(k, _)| k.clone()).collect();
                if done.len() == keys.len() || t0.elapsed().as_secs() >= wait {
                    break done;
                }
                std::thread::sleep(Duration::from_secs(2));
            };
            text(format!(
                "Queued {} sheet(s); {} finished{}. Skipped: {}",
                keys.len(),
                done.len(),
                if done.len() < keys.len() { " (the rest are still compiling; call get_drawing later)" } else { "" },
                r["skipped"]
            ))
        }
        "list_rules" => {
            let p = drawing::Prefs::load();
            text(if p.agent_rules.is_empty() { "(no rules yet)".to_string() } else { p.agent_rules.iter().map(|r| format!("- {r}")).collect::<Vec<_>>().join("\n") })
        }
        "remember_rule" => match ask(json!({"op": "rule", "rule": a["rule"]}), 10) {
            Ok(v) if v["ok"] == true => text("Saved."),
            Ok(v) => err(v["error"].as_str().unwrap_or("failed").to_string()),
            Err(e) => err(e),
        },
        _ => err(format!("unknown tool {name}")),
    }
}

fn report_mtime(dir: &std::path::Path, sh: &drawing::Sheets, key: &str) -> Option<std::time::SystemTime> {
    let s = sh.get(key)?;
    std::fs::metadata(dir.join(format!("{}.report.json", s.base))).and_then(|m| m.modified()).ok()
}

fn handle(msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg["method"].as_str().unwrap_or("");
    let result = match method {
        "initialize" => {
            let want = msg["params"]["protocolVersion"].as_str().unwrap_or("");
            let v = if SUPPORTED.contains(&want) { want } else { SUPPORTED[0] };
            json!({"protocolVersion": v, "capabilities": {"tools": {}}, "serverInfo": {"name": "plahelper", "version": env!("CARGO_PKG_VERSION")},
                   "instructions": "PLAhelper drawing tools for the user's Plasticity model (Chinese GB part drawings). Start with list_parts / get_drawing; use dimension names from get_drawing when changing settings. pla-ui must be running."})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => call_tool(msg["params"]["name"].as_str().unwrap_or(""), &msg["params"]["arguments"]),
        _ if id.is_none() => return None, // 通知(notifications/initialized 等)不回
        _ => return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("method not found: {method}")}})),
    };
    id.map(|id| json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(line.trim_start_matches('\u{feff}')) {
            Ok(m) => handle(&m),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}})),
        };
        if let Some(r) = reply {
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
}
