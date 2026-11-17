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
    .as_array()
    .cloned()
    .unwrap_or_default()
    .into_iter()
    .chain(model_tools().as_array().cloned().unwrap_or_default())
    .collect::<Vec<_>>()
    .into()
}

// ---------------------------------------------------------------- A6 建模工具(直接经 CDP 连 Plasticity, 不经 pla-ui)

const CDP_PORT: u16 = 9333;

fn model_tools() -> Value {
    let refs = "Topology ids look like '34f9439' (face) / '34e912' (edge) / '34v12' (vertex) - '34' is the solid's version id, which changes after every edit of that solid, so re-query after changes. Solids can also be given as {\"name\": \"<outliner name>\"}.";
    let ro = json!({"readOnlyHint": true});
    json!([
        {"name": "model_state", "annotations": ro, "description": format!("Read the open Plasticity model: undo depth, current selection, and geometry. detail: solids (default, overview) / faces / edges / all (+ vertices). Filter with of (solid ids or {{name}}), surface (Plane/Cylinder/Cone/Torus/…), curve (Line/Circle/…), normal [x,y,z] with angleTol (deg), near [x,y,z] with tol. Lengths in document units. Start with the overview, then query faces/edges of one solid. {refs}"),
         "inputSchema": {"type": "object", "properties": {
            "detail": {"type": "string", "enum": ["solids", "faces", "edges", "all"]},
            "of": {"type": "array", "items": {}}, "surface": {"type": "string"}, "curve": {"type": "string"},
            "normal": {"type": "array", "items": {"type": "number"}}, "angleTol": {"type": "number"},
            "near": {"type": "array", "items": {"type": "number"}}, "tol": {"type": "number"},
            "units": {"type": "string"}, "limit": {"type": "integer"}}}},
        {"name": "model_snaps", "annotations": ro, "description": "Snap points Plasticity itself uses (also internal ones): face centers + normals, edge midpoints + tangents, circle centers + axes, points on arcs, vertices, bbox corners. kinds filters (faceCenter|edgeMid|circleCenter|circleQuarter|vertex|bboxCorner); near/tol/of/limit as in model_state.",
         "inputSchema": {"type": "object", "properties": {"of": {"type": "array", "items": {}}, "kinds": {"type": "array", "items": {"type": "string"}}, "near": {"type": "array", "items": {"type": "number"}}, "tol": {"type": "number"}, "units": {"type": "string"}, "limit": {"type": "integer"}}}},
        {"name": "model_measure", "annotations": ro, "description": format!("Measure. op=distance: a, b = [x,y,z] | face id | edge id | vertex id → distance, delta, and normalDistance (wall thickness) when both are parallel planes. op=angle: a, b = faces (normals), line edges (directions) or vectors → degrees. op=mass: of (solids) and/or faces → volume, area, centroid (display mesh, curved surfaces ~0.3% low). {refs}"),
         "inputSchema": {"type": "object", "properties": {"op": {"type": "string", "enum": ["distance", "angle", "mass"]}, "a": {}, "b": {}, "of": {"type": "array", "items": {}}, "faces": {"type": "array", "items": {"type": "string"}}, "units": {"type": "string"}}, "required": ["op"]}},
        {"name": "model_view", "annotations": ro, "description": "Screenshot the model from a standard view (front/back/left/right/top/bottom/iso), framed on 'of' solids (default: everything). The user's camera is restored afterwards. Use it to check results visually.",
         "inputSchema": {"type": "object", "properties": {"view": {"type": "string", "enum": ["front", "back", "left", "right", "top", "bottom", "iso"]}, "of": {"type": "array", "items": {}}, "width": {"type": "integer", "description": "max width px (default 960)"}}}},
        {"name": "model_exec", "annotations": {"destructiveHint": true},
         "description": format!("CHANGES THE USER'S MODEL - only when the user asked for this modeling work. Runs geometry steps through Plasticity's own factories (each step = one undo step, same as manual work). atomic (default true): if a step fails, every step of this call is undone. Cancels any command the user has in progress; the Plasticity window must be visible.\n\
ops (lengths in document units unless units is given; points [x,y,z]):\n\
- box {{p1, p2 (opposite corners of the base), height}} (+ bool, targets)\n\
- cylinder {{center (base center), axis [x,y,z] (grow direction), radius, height}} (+ bool, targets) - a hole = cylinder with bool 'subtract' and targets [the solid]\n\
- fillet {{edges, distance}} / chamfer {{edges, distance}} (45°)\n\
- offsetFace {{faces, distance}} (+ = grow outward along the face normal; on a hole wall + makes the hole smaller)\n\
- refillet {{faces (fillet faces), distance (= radius change)}}\n\
- moveFaces {{faces, move [dx,dy,dz]}} (move a hole / boss / pocket)\n\
- revolve {{origin, axis, profile [[r, z], ...], degrees?=360}} (+ bool, targets) - solid of revolution: r = distance from the axis (>= 0, never crossing it), z = position along axis from origin; the profile closes itself; points with r = 0 lie on the axis. Screws, shafts, steps, cones, grooves (subtract).\n\
- prism {{center, normal?=[0,0,1] (extrude direction), sides, radius (circumscribed) | flats (across flats), height (negative = opposite), rotate? (deg about normal)}} (+ bool, targets) - regular polygon prism: hex socket (subtract, normal pointing into the head), hex head, nut.\n\
bool: union | subtract | intersect with targets [solid refs]. Give a step id: \"base\" and later steps can use \"$base\" (its result). Edges / faces: topology ids, or {{\"near\": [x,y,z], \"of\": solid ref, \"tol\"?}} = the item whose bbox center is nearest (errors instead of guessing when ambiguous). {refs} After running, verify with model_state / model_measure / model_view."),
         "inputSchema": {"type": "object", "properties": {
            "ops": {"type": "array", "items": {"type": "object"}},
            "atomic": {"type": "boolean"}, "units": {"type": "string"}}, "required": ["ops"]}},
        {"name": "model_hole", "annotations": {"destructiveHint": true},
         "description": "CHANGES THE USER'S MODEL. Hole wizard in one atomic step: face (plane face id where the hole starts), type simple|counterbore|countersink, d (diameter), depth (omit = through), cbD/cbDepth (counterbore), cskD (countersink, 90°), position: at [x,y,z] or offset [u,v] from the face center (u/v axes are returned), or points (sketch points). stack = further solid ids for a hole through several parts (tapD = diameter in the last one). Returns the created faces per hole.",
         "inputSchema": {"type": "object", "properties": {
            "face": {"type": "string"}, "type": {"type": "string", "enum": ["simple", "counterbore", "countersink"]}, "d": {"type": "number"},
            "depth": {"type": "number"}, "cbD": {"type": "number"}, "cbDepth": {"type": "number"}, "cskD": {"type": "number"}, "tapD": {"type": "number"},
            "at": {"type": "array", "items": {"type": "number"}}, "offset": {"type": "array", "items": {"type": "number"}},
            "points": {"type": "array", "items": {"type": "object"}}, "stack": {"type": "array", "items": {"type": "string"}}, "units": {"type": "string"}},
          "required": ["face", "d"]}},
        {"name": "model_part", "annotations": {"destructiveHint": true},
         "description": format!("CHANGES THE USER'S MODEL. Build a GB standard part in one atomic call (A10 macros; threads not modelled). kind: screw-cap (GB/T 70.1 socket head cap) / screw-csk (GB/T 70.3 countersunk socket, length includes head) / screw-hex (GB/T 5783 hex bolt) / screw-set (GB/T 77 socket set screw, flat point) / nut (GB/T 6170) / washer (GB/T 97.1) / pin (GB/T 119.2 dowel, size = diameter) / shaft (steps [[diameter, length], …], chamfer). Placement: at + axis. Screws: at = centre of the head's bearing face (countersunk: of the head top), axis = from head toward the tip (into the hole). Set screw: at = socket end. Nut / washer / pin / shaft: at = one end face centre, axis = thickness / length direction. Returns the steps' results and the spec key PLAhelper's BOM recognizer will report. Sizes: {}", pla_ui::fasteners::sizes()),
         "inputSchema": {"type": "object", "properties": {
            "kind": {"type": "string", "enum": ["screw-cap", "screw-csk", "screw-hex", "screw-set", "nut", "washer", "pin", "shaft"]},
            "size": {"description": "\"M4\" (or 4); pin = diameter"}, "length": {"type": "number"},
            "steps": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}}, "chamfer": {"type": "number"},
            "at": {"type": "array", "items": {"type": "number"}}, "axis": {"type": "array", "items": {"type": "number"}}},
          "required": ["kind"]}},
        {"name": "model_transaction", "annotations": {"destructiveHint": true},
         "description": "Group several model_exec / model_hole calls so they can be undone together. action=begin (remember the undo depth) → work → commit (keep) or rollback (undo everything since begin, including anything the user did in between; refused if the undo stack below begin was changed). action=undo with steps = undo the last N steps (like Ctrl+Z). action=state = undo / redo depth. The transaction survives shim updates (kept on the page).",
         "inputSchema": {"type": "object", "properties": {"action": {"type": "string", "enum": ["begin", "commit", "rollback", "undo", "state"]}, "steps": {"type": "integer", "description": "for undo: how many steps (default 1)"}}, "required": ["action"]}}
    ])
}

fn rt() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio"))
}

/// 连哪个 Plasticity 窗口: pla-ui 在跑就用它吸附的那个, 否则 0(代理挑唯一的那个)。
fn hwnd() -> isize {
    bridge::read_state().map(|s| s.hwnd).unwrap_or(0)
}

/// 连不上才提示 CDP 模式; 垫片报的参数 / 几何错误原样给。
fn cdp_err(e: impl std::fmt::Display) -> Value {
    let m = format!("{e:#}");
    let conn = ["connect", "timeout", "refused", "json/list", "no page", "no ws url"].iter().any(|k| m.to_lowercase().contains(k));
    if conn {
        err(format!("Cannot reach Plasticity over CDP (port {CDP_PORT}): {m}. Is Plasticity running in CDP mode (PLAhelper settings → CDP MODE) with a document open?"))
    } else {
        err(m)
    }
}

/// 参数对象里只留这些键(不认识的直接丢, 免得原样传进垫片)。
fn pick(a: &Value, keys: &[&str]) -> Value {
    let mut m = serde_json::Map::new();
    for k in keys {
        if let Some(v) = a.get(*k).filter(|v| !v.is_null()) {
            m.insert(k.to_string(), v.clone());
        }
    }
    Value::Object(m)
}

fn call_model_tool(name: &str, a: &Value) -> Option<Value> {
    use pla_bridge::shim::api;
    let h = hwnd();
    let json_text = |v: &Value| text(serde_json::to_string_pretty(v).unwrap_or_default());
    let call = |api_name: &str, args: Value| match rt().block_on(pla_bridge::query::call_once(CDP_PORT, h, api_name, &args)) {
        Ok(v) if v.get("err").is_some() => err(v["err"].to_string()),
        Ok(v) => json_text(&v),
        Err(e) => cdp_err(e),
    };
    Some(match name {
        "model_state" => {
            let args = pick(a, &["detail", "of", "surface", "curve", "normal", "angleTol", "near", "tol", "units", "limit"]);
            match rt().block_on(pla_bridge::query::state_once(CDP_PORT, h, &args)) {
                Ok(v) => json_text(&v),
                Err(e) => cdp_err(e),
            }
        }
        "model_snaps" => call(api::GEO_SNAPS, pick(a, &["of", "kinds", "near", "tol", "units", "limit"])),
        "model_measure" => call(api::GEO_MEASURE, pick(a, &["op", "a", "b", "of", "faces", "units"])),
        "model_view" => {
            let mut args = pick(a, &["view", "of"]);
            if let Some(w) = a["width"].as_u64() {
                args["maxWidth"] = json!(w);
            }
            match rt().block_on(pla_bridge::query::view_png_once(CDP_PORT, h, &args)) {
                Ok((png, meta)) => json!({"content": [{"type": "image", "data": b64(&png), "mimeType": "image/png"}, {"type": "text", "text": meta.to_string()}]}),
                Err(e) => cdp_err(e),
            }
        }
        "model_exec" => {
            let mut script = pick(a, &["ops", "units"]);
            script["atomic"] = json!(a["atomic"].as_bool().unwrap_or(true));
            match rt().block_on(pla_bridge::exec::run_once(CDP_PORT, h, &script)) {
                Ok(v) => {
                    let (ok, msg) = pla_bridge::exec::summary(&v);
                    let mut r = json_text(&v);
                    r["content"].as_array_mut().map(|c| c.insert(0, json!({"type": "text", "text": msg})));
                    if !ok {
                        r["isError"] = json!(true);
                    }
                    r
                }
                Err(e) => cdp_err(e),
            }
        }
        "model_part" => {
            let q: pla_ui::fasteners::PartReq = match serde_json::from_value(a.clone()) {
                Ok(q) => q,
                Err(e) => return Some(err(format!("bad arguments: {e}"))),
            };
            let x = match pla_ui::fasteners::expand(&q) {
                Ok(x) => x,
                Err(e) => return Some(err(e)),
            };
            let script = json!({"ops": x.ops, "atomic": true});
            match rt().block_on(pla_bridge::exec::run_once(CDP_PORT, h, &script)) {
                Ok(v) => {
                    let (ok, msg) = pla_bridge::exec::summary(&v);
                    let part = v["results"].as_array().and_then(|r| r.last()).map(|r| r["items"].clone()).unwrap_or(Value::Null);
                    let mut r = json_text(&json!({"part": part, "spec": x.spec, "standard": x.text, "steps": v}));
                    r["content"].as_array_mut().map(|c| c.insert(0, json!({"type": "text", "text": format!("{} — {msg}", x.text)})));
                    if !ok {
                        r["isError"] = json!(true);
                    }
                    r
                }
                Err(e) => cdp_err(e),
            }
        }
        "model_hole" => call(api::HOLE_MAKE, pick(a, &["face", "type", "d", "depth", "cbD", "cbDepth", "cskD", "tapD", "at", "offset", "points", "stack", "units"])),
        "model_transaction" => match a["action"].as_str().unwrap_or("") {
            "begin" => call(api::HIST_BEGIN, Value::Null),
            "commit" => call(api::HIST_COMMIT, Value::Null),
            "rollback" => call(api::HIST_ROLLBACK, Value::Null),
            "undo" => call(api::HIST_UNDO, json!({"steps": a["steps"].as_u64().unwrap_or(1)})),
            "state" => call(api::HIST_STATE, Value::Null),
            x => err(format!("unknown action {x}")),
        },
        _ => return None,
    })
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
    // 建模工具不需要 pla-ui(直接连 Plasticity)
    if let Some(r) = call_model_tool(name, a) {
        return r;
    }
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
