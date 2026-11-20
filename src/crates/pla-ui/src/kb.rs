//! K5 知识库接口(2026-11-19 用户: helper 不内置知识库, 但要留接口让用户接自己的; 范例库 mech-kb 纯本地 + git)。
//!
//! 知识源配置: `<数据目录>/knowledge.json` = `{"sources": [{name, kind, …}]}`
//! - `local`   {path}: 本地目录, 格式见 mech-kb/SCHEMA.md(cards/*.md、resources/*.json、tools/<id>/tool.json + tool.js + tests.json、proposals/)
//! - `command` {command, args}: 别的知识库包一层脚本: stdin 一行 JSON 请求(search / get / run), stdout 一行 JSON 回答
//! - `ima`     腾讯 ima(只读, 只能按标题搜; 见 `knowledge`)
//!
//! agent 的用法写在 MCP 工具描述里: 先 kb_search → 有工具就 kb_run → 没有就手算并注明出处, 做完后可 kb_propose 把这类计算固化成工具草稿(用户审核后才进 tools/)。

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(20);

pub fn config_path() -> PathBuf {
    crate::prefs::data_dir().join("knowledge.json")
}

pub fn sources() -> Vec<Value> {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["sources"].as_array().cloned())
        .unwrap_or_default()
}

fn save_sources(list: &[Value]) -> Result<(), String> {
    let p = config_path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(&json!({"sources": list})).unwrap()).map_err(|e| format!("{}: {e}", p.display()))
}

fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 40 && n.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// kb_sources: list / add / remove。add 时检查: local 目录里要有 kb.json; command 不检查(用户自己的程序)。
pub fn sources_tool(a: &Value) -> Result<Value, String> {
    let mut list = sources();
    match a["action"].as_str().unwrap_or("list") {
        "list" => Ok(json!({"config": config_path().to_string_lossy(), "sources": list})),
        "add" => {
            let name = a["name"].as_str().unwrap_or("").trim().to_string();
            if !valid_name(&name) {
                return Err("name: letters, digits, - or _ (≤ 40)".into());
            }
            if list.iter().any(|s| s["name"].as_str() == Some(&name)) {
                return Err(format!("source {name} already exists (remove it first)"));
            }
            let kind = a["kind"].as_str().unwrap_or("local");
            let entry = match kind {
                "local" => {
                    let p = a["path"].as_str().ok_or("local needs path")?;
                    let kbj = Path::new(p).join("kb.json");
                    if !kbj.is_file() {
                        return Err(format!("{} not found: a local knowledge base needs kb.json (see mech-kb/SCHEMA.md)", kbj.display()));
                    }
                    json!({"name": name, "kind": "local", "path": p})
                }
                "command" => json!({"name": name, "kind": "command", "command": a["command"].as_str().ok_or("command needs command")?, "args": a["args"].clone()}),
                "ima" => json!({"name": name, "kind": "ima"}),
                k => return Err(format!("unknown kind {k} (local / command / ima)")),
            };
            list.push(entry.clone());
            save_sources(&list)?;
            Ok(json!({"added": entry}))
        }
        "remove" => {
            let name = a["name"].as_str().unwrap_or("");
            let n0 = list.len();
            list.retain(|s| s["name"].as_str() != Some(name));
            if list.len() == n0 {
                return Err(format!("no source named {name}"));
            }
            save_sources(&list)?;
            Ok(json!({"removed": name}))
        }
        x => Err(format!("unknown action {x}")),
    }
}

fn pick_sources(source: Option<&str>) -> Result<Vec<Value>, String> {
    let all = sources();
    if all.is_empty() {
        return Err(format!("no knowledge source configured: add one with kb_sources action=add (e.g. kind local, path = a mech-kb folder); config file {}", config_path().display()));
    }
    let picked = match source {
        None | Some("") => all,
        Some(n) => all.into_iter().find(|s| s["name"].as_str() == Some(n)).map(|s| vec![s]).ok_or_else(|| format!("no knowledge source named {n}"))?,
    };
    Ok(picked.into_iter().map(via_rpc).collect())
}

/// 本地库的 kb.json 写了 `"rpc": ["runner/kb.js", "rpc"]` → 交给库自己的运行器(检索 / 目录结构 / 格式由库决定, 体量上来换检索后端也不用改 helper);
/// 没写的(schema 1 的老库)按内置的本地读法。
fn via_rpc(s: Value) -> Value {
    if s["kind"].as_str() != Some("local") {
        return s;
    }
    let root = PathBuf::from(s["path"].as_str().unwrap_or(""));
    let kbj: Value = read(&root.join("kb.json")).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let Some(rpc) = kbj["rpc"].as_array().filter(|a| !a.is_empty()) else { return s };
    let mut args: Vec<Value> = vec![json!(root.join(rpc[0].as_str().unwrap_or("runner/kb.js")).to_string_lossy())];
    args.extend(rpc[1..].iter().cloned());
    json!({"name": s["name"], "kind": "command", "command": node_for(&root), "args": args, "path": s["path"], "via": "rpc"})
}

/// 知识库自带运行时(发布包里的 runtime/node.exe, 用户不用装 Node)优先, 否则用 PATH 上的 node。
fn node_for(root: &Path) -> String {
    let bundled = root.join("runtime").join("node.exe");
    if bundled.is_file() {
        bundled.to_string_lossy().into_owned()
    } else {
        "node".into()
    }
}

// ---------------------------------------------------------------- 本地库: 读取与检索

/// 卡片头信息: `---` 之间每行 `键: 值`, 值 `[a, b]` 是列表。→ (头, 正文)
pub fn front_matter(text: &str) -> (serde_json::Map<String, Value>, String) {
    let mut m = serde_json::Map::new();
    let t = text.trim_start_matches('\u{feff}');
    let mut lines = t.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (m, t.to_string());
    }
    let mut body = String::new();
    let mut in_head = true;
    for l in lines {
        if in_head {
            if l.trim() == "---" {
                in_head = false;
                continue;
            }
            if let Some((k, v)) = l.split_once(':') {
                let v = v.trim();
                let val = if v.starts_with('[') && v.ends_with(']') {
                    Value::Array(v[1..v.len() - 1].split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).map(|x| json!(x)).collect())
                } else {
                    json!(v)
                };
                m.insert(k.trim().to_string(), val);
            }
        } else {
            body.push_str(l);
            body.push('\n');
        }
    }
    (m, body)
}

/// 本地库的条目: (kind, id, title, 检索用文本(标题 / 标签 / 正文), 摘要)
struct Item {
    kind: &'static str,
    id: String,
    title: String,
    tags: String,
    text: String,
    summary: String,
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim_start_matches('\u{feff}').to_string())
}

fn local_items(root: &Path) -> Vec<Item> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(root.join("cards")) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("md") {
                continue;
            }
            if let Some(t) = read(&p) {
                let (h, body) = front_matter(&t);
                let id = h.get("id").and_then(|v| v.as_str()).map(str::to_string).unwrap_or_else(|| p.file_stem().unwrap().to_string_lossy().to_string());
                let tags = h.get("tags").map(|v| v.to_string()).unwrap_or_default();
                let summary = body.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).unwrap_or("").chars().take(120).collect();
                out.push(Item { kind: "card", id, title: h.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(), tags, text: body, summary });
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(root.join("tools")) {
        for e in rd.flatten() {
            if let Some(v) = read(&e.path().join("tool.json")).and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
                out.push(Item {
                    kind: "tool",
                    id: v["id"].as_str().unwrap_or("").to_string(),
                    title: v["title"].as_str().unwrap_or("").to_string(),
                    tags: format!("{} {}", v["tags"], v["standards"]),
                    text: format!("{} {} {}", v["summary"], v["inputs"], v["refs"]),
                    summary: v["summary"].as_str().unwrap_or("").to_string(),
                });
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(root.join("resources")) {
        for e in rd.flatten() {
            if let Some(arr) = read(&e.path()).and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|v| v.as_array().cloned()) {
                for r in arr {
                    out.push(Item {
                        kind: "resource",
                        id: r["id"].as_str().unwrap_or("").to_string(),
                        title: r["title"].as_str().unwrap_or("").to_string(),
                        tags: format!("{} {}", r["topics"], r["kind"]),
                        text: format!("{} {}", r["look_for"], r["cites"]),
                        summary: r["look_for"].as_str().unwrap_or("").to_string(),
                    });
                }
            }
        }
    }
    out
}

/// 查询拆词(空白 / 标点); 中文长词另外按两字切片做模糊匹配。
pub fn query_terms(q: &str) -> Vec<String> {
    q.split(|c: char| c.is_whitespace() || ",，、;；。:：/()（）".contains(c)).map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}

fn score_field(field: &str, term: &str, w: f64) -> f64 {
    let f = field.to_lowercase();
    if f.contains(term) {
        return w;
    }
    // 中文: "联轴器孔径" 这种连写 → 按两字片段的命中比例给分(打折)
    let cs: Vec<char> = term.chars().collect();
    if cs.len() >= 3 && cs.iter().all(|c| is_cjk(*c)) {
        let grams: Vec<String> = cs.windows(2).map(|w| w.iter().collect()).collect();
        let hit = grams.iter().filter(|g| f.contains(g.as_str())).count();
        return w * 0.5 * hit as f64 / grams.len() as f64;
    }
    0.0
}

pub fn local_search(root: &Path, query: &str, kinds: &[String], limit: usize) -> Vec<Value> {
    let terms = query_terms(query);
    let mut hits: Vec<(f64, Value)> = local_items(root)
        .into_iter()
        .filter(|it| kinds.is_empty() || kinds.iter().any(|k| k == it.kind))
        .filter_map(|it| {
            let s: f64 = terms.iter().map(|t| score_field(&it.title, t, 3.0) + score_field(&it.tags, t, 2.0) + score_field(&it.id, t, 2.0) + score_field(&it.text, t, 1.0)).sum();
            // 工具优先(有现成工具就该用工具算)
            let s = if it.kind == "tool" { s * 1.2 } else { s };
            (s > 0.0).then(|| (s, json!({"kind": it.kind, "id": it.id, "title": it.title, "summary": it.summary, "score": (s * 100.0).round() / 100.0})))
        })
        .collect();
    hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    hits.into_iter().take(limit).map(|h| h.1).collect()
}

fn safe_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 60 || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') || id.starts_with('-') {
        return Err(format!("bad id {id:?}: lower-case letters, digits, '-'"));
    }
    Ok(())
}

pub fn local_get(root: &Path, kind: &str, id: &str) -> Result<Value, String> {
    match kind {
        "card" => {
            for it in std::fs::read_dir(root.join("cards")).map_err(|e| e.to_string())?.flatten() {
                if let Some(t) = read(&it.path()) {
                    let (h, body) = front_matter(&t);
                    let cid = h.get("id").and_then(|v| v.as_str()).map(str::to_string).unwrap_or_else(|| it.path().file_stem().unwrap().to_string_lossy().to_string());
                    if cid == id {
                        return Ok(json!({"kind": "card", "id": id, "meta": h, "text": body, "file": it.path().to_string_lossy()}));
                    }
                }
            }
            Err(format!("no card {id}"))
        }
        "tool" => {
            safe_id(id)?;
            let d = root.join("tools").join(id);
            let spec: Value = read(&d.join("tool.json")).and_then(|s| serde_json::from_str(&s).ok()).ok_or_else(|| format!("no tool {id}"))?;
            let tests = read(&d.join("tests.json")).and_then(|s| serde_json::from_str::<Value>(&s).ok()).map(|v| v.as_array().map(|a| a.len()).unwrap_or(0)).unwrap_or(0);
            Ok(json!({"kind": "tool", "id": id, "spec": spec, "test_cases": tests, "dir": d.to_string_lossy()}))
        }
        "resource" => {
            for e in std::fs::read_dir(root.join("resources")).map_err(|e| e.to_string())?.flatten() {
                if let Some(arr) = read(&e.path()).and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|v| v.as_array().cloned()) {
                    if let Some(r) = arr.into_iter().find(|r| r["id"].as_str() == Some(id)) {
                        return Ok(json!({"kind": "resource", "id": id, "entry": r}));
                    }
                }
            }
            Err(format!("no resource {id}"))
        }
        k => Err(format!("kind must be card / tool / resource, not {k}")),
    }
}

/// 起一个进程: stdin 写 input, 读 stdout 一行 JSON; 超时杀掉。
fn run_json(mut cmd: Command, input: &str) -> Result<Value, String> {
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("start: {e}"))?;
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(input.as_bytes());
    }
    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    let ho = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = so.read_to_string(&mut s);
        s
    });
    let he = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = se.read_to_string(&mut s);
        s
    });
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if t0.elapsed() > RUN_TIMEOUT => {
                let _ = child.kill();
                return Err(format!("timed out after {} s", RUN_TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let out = ho.join().unwrap_or_default();
    let err = he.join().unwrap_or_default();
    let line = out.lines().rev().find(|l| l.trim_start().starts_with('{') || l.trim_start().starts_with('[')).unwrap_or("");
    serde_json::from_str(line).map_err(|_| format!("no JSON answer (stdout {:?}, stderr {:?})", out.chars().take(300).collect::<String>(), err.chars().take(300).collect::<String>()))
}

pub fn local_run(root: &Path, tool: &str, inputs: &Value) -> Result<Value, String> {
    safe_id(tool)?;
    let kbj: Value = read(&root.join("kb.json")).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
    let runner = root.join(kbj["runner"].as_str().unwrap_or("runner/kb.js"));
    if !runner.is_file() {
        return Err(format!("runner {} not found", runner.display()));
    }
    let mut cmd = Command::new(node_for(root));
    cmd.arg(&runner).arg("run").arg(tool).arg("-").current_dir(root);
    let v = run_json(cmd, &inputs.to_string()).map_err(|e| format!("node: {e} (Node.js ≥ 18 must be on PATH)"))?;
    if let Some(e) = v["error"].as_str() {
        return Err(e.to_string());
    }
    Ok(v)
}

/// kb_propose: 草稿只写进 proposals/(不碰 tools/ 与 cards/)。tool: files = {tool.json, tool.js, tests.json}; card: text。
pub fn local_propose(root: &Path, a: &Value) -> Result<Value, String> {
    let id = a["id"].as_str().unwrap_or("");
    safe_id(id)?;
    let overwrite = a["overwrite"].as_bool().unwrap_or(false);
    match a["kind"].as_str().unwrap_or("") {
        "tool" => {
            let d = root.join("proposals").join("tools").join(id);
            if d.exists() && !overwrite {
                return Err(format!("proposal {id} exists (overwrite: true to replace)"));
            }
            if root.join("tools").join(id).exists() {
                return Err(format!("a verified tool {id} already exists: propose a new id (e.g. {id}-v2)"));
            }
            let spec = a["spec"].clone();
            if !spec.is_object() || spec["inputs"].as_array().is_none() || spec["outputs"].as_array().is_none() || spec["refs"].as_array().map(|r| r.is_empty()).unwrap_or(true) {
                return Err("spec needs inputs, outputs and non-empty refs (where the formulas come from)".into());
            }
            let code = a["code"].as_str().ok_or("code (tool.js source) is required")?;
            let tests = a["tests"].as_array().filter(|t| !t.is_empty()).ok_or("tests (reference cases with independent expected values) are required")?;
            if tests.iter().any(|t| t["ref"].as_str().map(str::trim).unwrap_or("").is_empty()) {
                return Err("every test needs ref: where the expected value comes from (standard table / handbook example / hand calculation)".into());
            }
            let mut spec = spec;
            spec["id"] = json!(id);
            spec["status"] = json!("draft");
            std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
            std::fs::write(d.join("tool.json"), serde_json::to_string_pretty(&spec).unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(d.join("tool.js"), code).map_err(|e| e.to_string())?;
            std::fs::write(d.join("tests.json"), serde_json::to_string_pretty(&Value::Array(tests.clone())).unwrap()).map_err(|e| e.to_string())?;
            Ok(json!({"proposed": "tool", "id": id, "dir": d.to_string_lossy(), "next": "tell the user: review proposals/tools/<id> (formulas, refs, test expectations; node runner/kb.js test-proposal <id>), then move it to tools/, set status verified, run node runner/kb.js test <id>, commit"}))
        }
        "card" => {
            let text = a["text"].as_str().ok_or("text (markdown with front matter) is required")?;
            let (h, _) = front_matter(text);
            if h.get("title").is_none() {
                return Err("card text must start with front matter (--- id / title / tags / standards ---)".into());
            }
            let p = root.join("proposals").join("cards").join(format!("{id}.md"));
            if p.exists() && !overwrite {
                return Err(format!("proposal {id} exists (overwrite: true to replace)"));
            }
            std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&p, text).map_err(|e| e.to_string())?;
            Ok(json!({"proposed": "card", "id": id, "file": p.to_string_lossy()}))
        }
        k => Err(format!("kind must be tool or card, not {k:?}")),
    }
}

// ---------------------------------------------------------------- 分发到各知识源

fn command_call(src: &Value, req: Value) -> Result<Value, String> {
    let exe = src["command"].as_str().ok_or("command source without command")?;
    let mut cmd = Command::new(exe);
    for a in src["args"].as_array().into_iter().flatten() {
        if let Some(s) = a.as_str() {
            cmd.arg(s);
        }
    }
    let v = run_json(cmd, &format!("{req}\n"))?;
    if let Some(e) = v["error"].as_str() {
        return Err(e.to_string());
    }
    Ok(v)
}

pub fn search(query: &str, kinds: &[String], source: Option<&str>, limit: usize) -> Result<Value, String> {
    if query_terms(query).is_empty() {
        return Err("query is empty".into());
    }
    let mut results = vec![];
    let mut errors = vec![];
    for s in pick_sources(source)? {
        let name = s["name"].as_str().unwrap_or("").to_string();
        let r = match s["kind"].as_str().unwrap_or("") {
            "local" => Ok(json!(local_search(Path::new(s["path"].as_str().unwrap_or("")), query, kinds, limit))),
            "command" => command_call(&s, json!({"op": "search", "query": query, "kinds": kinds, "limit": limit})).map(|v| v["hits"].clone()),
            "ima" => {
                if !kinds.is_empty() && !kinds.iter().any(|k| k == "resource") {
                    continue;
                }
                crate::knowledge::search(query, &[], limit).map(|v| {
                    Value::Array(v["hits"].as_array().cloned().unwrap_or_default().into_iter()
                        .map(|h| json!({"kind": "resource", "id": h["media_id"], "title": h["title"], "summary": format!("ima 知识库「{}」(只有标题; 订阅库原文 API 拿不到)", h["kb"].as_str().unwrap_or("")), "score": h["score"]}))
                        .collect())
                })
            }
            k => Err(format!("unknown kind {k}")),
        };
        match r {
            Ok(h) => results.push(json!({"source": name, "hits": h})),
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    Ok(json!({"results": results, "errors": errors}))
}

fn one_source(source: Option<&str>) -> Result<Value, String> {
    let s = pick_sources(source)?;
    if s.len() > 1 {
        return Err(format!("several knowledge sources: give source ({})", s.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", ")));
    }
    Ok(s.into_iter().next().unwrap())
}

pub fn get(source: Option<&str>, kind: &str, id: &str) -> Result<Value, String> {
    let s = one_source(source)?;
    match s["kind"].as_str().unwrap_or("") {
        "local" => local_get(Path::new(s["path"].as_str().unwrap_or("")), kind, id),
        "command" => command_call(&s, json!({"op": "get", "kind": kind, "id": id})),
        "ima" => crate::knowledge::get(id),
        k => Err(format!("unknown kind {k}")),
    }
}

pub fn run(source: Option<&str>, tool: &str, inputs: &Value) -> Result<Value, String> {
    let s = one_source(source)?;
    match s["kind"].as_str().unwrap_or("") {
        "local" => local_run(Path::new(s["path"].as_str().unwrap_or("")), tool, inputs),
        "command" => command_call(&s, json!({"op": "run", "tool": tool, "inputs": inputs})),
        _ => Err("this knowledge source has no calculation tools".into()),
    }
}

pub fn propose(source: Option<&str>, a: &Value) -> Result<Value, String> {
    let s = one_source(source)?;
    match s["kind"].as_str().unwrap_or("") {
        "local" => local_propose(Path::new(s["path"].as_str().unwrap_or("")), a),
        "command" => {
            let mut req = a.clone();
            if let Some(o) = req.as_object_mut() {
                o.remove("source");
                o.insert("op".into(), json!("propose"));
            }
            command_call(&s, req)
        }
        _ => Err("this knowledge source does not take proposals".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_and_search() {
        let (h, body) = front_matter("---\nid: fit\ntitle: 配合的选择\ntags: [配合, 公差]\n---\n正文 H7/g6\n");
        assert_eq!(h["id"], json!("fit"));
        assert_eq!(h["tags"], json!(["配合", "公差"]));
        assert!(body.contains("H7/g6"));
        assert!(score_field("联轴器的选型", "联轴器", 1.0) == 1.0);
        let fuzzy = score_field("联轴器的选型与孔径", "联轴器孔径", 2.0);
        assert!(fuzzy > 0.0 && fuzzy < 2.0, "连写中文按两字片段打折命中: {fuzzy}");
        assert_eq!(query_terms("配合 公差，H7"), vec!["配合", "公差", "h7"]);
        assert!(safe_id("fit-limits").is_ok() && safe_id("../x").is_err() && safe_id("Fit").is_err());
    }

    #[test]
    fn local_kb_roundtrip() {
        let dir = std::env::temp_dir().join(format!("pla-kb-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cards")).unwrap();
        std::fs::create_dir_all(dir.join("tools").join("demo")).unwrap();
        std::fs::create_dir_all(dir.join("resources")).unwrap();
        std::fs::write(dir.join("kb.json"), r#"{"schema":1,"name":"t","runner":"runner/kb.js"}"#).unwrap();
        std::fs::write(dir.join("cards").join("a.md"), "---\nid: a\ntitle: 螺栓预紧\ntags: [螺栓]\n---\n正文\n").unwrap();
        std::fs::write(dir.join("tools").join("demo").join("tool.json"), r#"{"id":"demo","title":"螺栓力矩","summary":"s","tags":["螺栓"],"inputs":[],"outputs":[],"refs":["x"]}"#).unwrap();
        std::fs::write(dir.join("resources").join("r.json"), r#"[{"id":"r1","title":"链接","topics":["螺栓"],"look_for":"查"}]"#).unwrap();
        let hits = local_search(&dir, "螺栓", &[], 10);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0]["kind"], json!("tool"), "工具排前面");
        assert_eq!(local_search(&dir, "螺栓", &["card".into()], 10).len(), 1);
        assert_eq!(local_get(&dir, "card", "a").unwrap()["meta"]["title"], json!("螺栓预紧"));
        assert!(local_get(&dir, "tool", "../demo").is_err());
        // 草稿: 缺出处 / 缺用例来源 → 拒绝; 合格 → 只写 proposals/
        let bad = json!({"kind": "tool", "id": "x1", "spec": {"inputs": [], "outputs": [], "refs": []}, "code": "", "tests": [{"inputs": {}}]});
        assert!(local_propose(&dir, &bad).is_err());
        let ok = json!({"kind": "tool", "id": "x1", "spec": {"title": "t", "inputs": [], "outputs": [], "refs": ["教科书 式 1"]}, "code": "module.exports={run(){return{outputs:{}}}}",
                        "tests": [{"name": "c", "inputs": {}, "expect": {}, "ref": "手算"}]});
        let r = local_propose(&dir, &ok).unwrap();
        assert!(dir.join("proposals").join("tools").join("x1").join("tool.json").is_file(), "{r}");
        assert!(!dir.join("tools").join("x1").exists(), "不碰 tools/");
        assert!(local_propose(&dir, &json!({"kind": "tool", "id": "demo", "spec": ok["spec"], "code": "x", "tests": ok["tests"]})).is_err(), "不覆盖已审核的工具");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_kb_with_rpc_runner() {
        let dir = std::env::temp_dir().join(format!("pla-kb-rpc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("runner")).unwrap();
        // 没写 rpc 的老库 → 原样(内置读法)
        std::fs::write(dir.join("kb.json"), r#"{"schema":1,"name":"t"}"#).unwrap();
        let src = json!({"name": "t", "kind": "local", "path": dir.to_string_lossy()});
        assert_eq!(via_rpc(src.clone())["kind"], json!("local"));
        // 写了 rpc → 交给库自己的运行器
        std::fs::write(dir.join("kb.json"), r#"{"schema":2,"name":"t","rpc":["runner/kb.js","rpc"]}"#).unwrap();
        std::fs::write(dir.join("runner").join("kb.js"),
            "let s='';process.stdin.on('data',d=>s+=d).on('end',()=>{const r=JSON.parse(s);console.log(JSON.stringify(r.op==='search'?{hits:[{kind:'page',id:'x',q:r.query}]}:r.op==='propose'?{proposed:r.kind,id:r.id,hasSource:'source' in r}:{error:'unsupported'}))});").unwrap();
        let s = via_rpc(src.clone());
        assert_eq!(s["kind"], json!("command"));
        assert_eq!(s["command"], json!("node"));
        // 库里带了运行时 → 用它(用户没装 Node 也能用)
        std::fs::create_dir_all(dir.join("runtime")).unwrap();
        std::fs::write(dir.join("runtime").join("node.exe"), b"").unwrap();
        assert!(via_rpc(src).as_object().unwrap()["command"].as_str().unwrap().ends_with("node.exe"));
        std::fs::remove_dir_all(dir.join("runtime")).unwrap();
        assert_eq!(s["args"][1], json!("rpc"));
        assert!(s["args"][0].as_str().unwrap().ends_with("kb.js"));
        if Command::new("node").arg("--version").output().is_ok() {
            let r = command_call(&s, json!({"op": "search", "query": "联轴器"})).unwrap();
            assert_eq!(r["hits"][0]["q"], json!("联轴器"));
            assert_eq!(command_call(&s, json!({"op": "nope"})).unwrap_err(), "unsupported");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
