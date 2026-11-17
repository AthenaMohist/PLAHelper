//! D5: 出图助手 —— 用自然语言改出图方案。
//!
//! 一轮对话 = 把"当前方案 + 上次出图报告(放了哪些尺寸 / 孔组 / 警告)+ 预览图"连同用户的话发给 Claude;
//! Claude 用工具 `propose_drawing_changes` 给出**方案补丁**(只能改人本来就能改的字段), 或用 `remember_rule` 提议记一条长期规则。
//! 补丁 / 规则都先给用户看, 点「应用」才生效 —— 模型不直接改任何东西。应用过的补丁记进 `<base>.agent.jsonl`(D6 案例层的样本)。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::drawing::{Axes, Report, Sheet};
use pla_bridge::i18n::tr;
use pla_bridge::t;

/// 方案补丁: 全部可选, 没给的字段不动。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Patch {
    /// 一句话说明(给用户看)。
    #[serde(default)]
    pub summary: String,
    /// 一句话改多张图: 目标零件(名称 / 料号; ["all"] = 全部自制件); 空 = 只改当前这张。
    #[serde(default)]
    pub parts: Vec<String>,
    /// 主视朝向: flip(上下翻转)/ rotate(转 90°)/ back(看背面)/ auto(恢复自动)。
    #[serde(default)]
    pub orient: Option<String>,
    #[serde(default)]
    pub section: Option<bool>,
    /// 标孔位坐标的孔组: 孔组 id 列表; 字符串 "all" = 主视全部(默认)。
    #[serde(default)]
    pub ordinate: Option<Value>,
    /// 孔径 → 螺纹, 如 {"3.3": "M4"}; 值为空串 = 删掉这条。
    #[serde(default)]
    pub threads: BTreeMap<String, String>,
    #[serde(default)]
    pub density: Option<f64>,
    #[serde(default)]
    pub hide: Vec<String>,
    #[serde(default)]
    pub show: Vec<String>,
    #[serde(default)]
    pub key_add: Vec<String>,
    #[serde(default)]
    pub key_remove: Vec<String>,
    /// 尺寸名 → 显示文字; 空串 = 恢复自动。
    #[serde(default)]
    pub dim_text: BTreeMap<String, String>,
    /// 尺寸名 → 公差("h7" / "±0.05" / "+0.1/-0.05"); 空串 = 去掉。
    #[serde(default)]
    pub dim_tol: BTreeMap<String, String>,
    /// 技术要求全文(一行一条, 整体替换)。
    #[serde(default)]
    pub tech: Option<Vec<String>>,
    /// 标题栏字段(designer / reviewer / project / finish / hardness / version / pn / name / material); 空串 = 去掉本图的覆盖。
    #[serde(default)]
    pub title: BTreeMap<String, String>,
}

fn set_toggle(list: &mut Vec<String>, add: &[String], remove: &[String]) {
    for r in remove {
        list.retain(|x| x != r);
    }
    for a in add {
        if !list.contains(a) {
            list.push(a.clone());
        }
    }
}

fn merge_map(m: &mut BTreeMap<String, String>, upd: &BTreeMap<String, String>) {
    for (k, v) in upd {
        if v.trim().is_empty() {
            m.remove(k);
        } else {
            m.insert(k.clone(), v.trim().to_string());
        }
    }
}

/// 把补丁打到图上; 返回改了什么(给用户看)。尺寸名 / 孔组 id 不认识的报错(不悄悄忽略)。
pub fn apply(s: &mut Sheet, p: &Patch, rep: Option<&Report>) -> Result<Vec<String>, String> {
    let names: Vec<&str> = rep.map(|r| r.placed.iter().map(|x| x.name.as_str()).collect()).unwrap_or_default();
    let groups: Vec<&str> = rep.map(|r| r.hole_groups.iter().map(|g| g.id.as_str()).collect()).unwrap_or_default();
    let added: Vec<String> = s.dims_add.iter().map(|a| a.name.clone()).collect();
    let known = |n: &str| names.is_empty() || names.contains(&n) || added.iter().any(|a| a == n);
    for n in p.hide.iter().chain(&p.show).chain(&p.key_add).chain(&p.key_remove).chain(p.dim_text.keys()).chain(p.dim_tol.keys()) {
        if !known(n) {
            return Err(t!("没有叫 {n} 的尺寸", "No dimension named {n}"));
        }
    }
    let mut out = Vec::new();
    if let Some(o) = p.orient.as_deref() {
        let ax: Option<Axes> = rep.and_then(|r| r.axes);
        let (f, u) = match (o, ax) {
            ("auto", _) => (None, None),
            ("flip", Some(a)) => {
                let (f, u) = a.flipped();
                (Some(f), Some(u))
            }
            ("rotate", Some(a)) => {
                let (f, u) = a.rotated();
                (Some(f), Some(u))
            }
            ("back", Some(a)) => {
                let (f, u) = a.back();
                (Some(f), Some(u))
            }
            (_, None) => return Err(tr("要先出过一次图才能改朝向", "Draw once before changing the orientation").into()),
            (x, _) => return Err(t!("不认识的朝向操作 {x}", "Unknown orientation op {x}")),
        };
        s.front = f;
        s.up = u;
        out.push(t!("主视朝向: {o}", "Front view: {o}"));
    }
    if let Some(v) = p.section {
        s.section = v;
        out.push(if v { tr("剖视 A-A: 开", "Section A-A: on") } else { tr("剖视 A-A: 关", "Section A-A: off") }.into());
    }
    if let Some(o) = &p.ordinate {
        if o.as_str() == Some("all") {
            s.ordinate = None;
            out.push(tr("孔位: 主视全部孔组", "Hole positions: all groups in the front view").into());
        } else if let Some(a) = o.as_array() {
            let ids: Vec<String> = a.iter().filter_map(|x| x.as_str().map(String::from)).collect();
            if let Some(bad) = ids.iter().find(|i| !groups.is_empty() && !groups.contains(&i.as_str())) {
                return Err(t!("没有孔组 {bad}", "No hole group {bad}"));
            }
            out.push(t!("孔位只标: {}", "Hole positions only for: {}", if ids.is_empty() { "-".to_string() } else { ids.join(", ") }));
            s.ordinate = Some(ids);
        }
    }
    if !p.threads.is_empty() {
        merge_map(&mut s.threads, &p.threads);
        out.push(t!("孔径→螺纹: {}", "Hole → thread: {}", p.threads.iter().map(|(d, m)| format!("{d}={m}")).collect::<Vec<_>>().join(", ")));
    }
    if let Some(d) = p.density {
        s.density = Some(d);
        out.push(t!("密度 {d} g/cm³", "Density {d} g/cm³"));
    }
    if !p.hide.is_empty() || !p.show.is_empty() {
        set_toggle(&mut s.dims_hide, &p.hide, &p.show);
        if !p.hide.is_empty() {
            out.push(t!("不标: {}", "Hide: {}", p.hide.join(", ")));
        }
        if !p.show.is_empty() {
            out.push(t!("恢复标注: {}", "Show again: {}", p.show.join(", ")));
        }
    }
    if !p.key_add.is_empty() || !p.key_remove.is_empty() {
        set_toggle(&mut s.dims_key, &p.key_add, &p.key_remove);
        out.push(t!("关键尺寸 +{} −{}", "Key dims +{} −{}", p.key_add.join(","), p.key_remove.join(",")));
    }
    if !p.dim_text.is_empty() {
        merge_map(&mut s.dims_text, &p.dim_text);
        out.push(t!("改文字: {}", "Text: {}", p.dim_text.iter().map(|(k, v)| format!("{k}→{v}")).collect::<Vec<_>>().join(", ")));
    }
    if !p.dim_tol.is_empty() {
        merge_map(&mut s.dims_tol, &p.dim_tol);
        out.push(t!("公差: {}", "Tolerance: {}", p.dim_tol.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ")));
    }
    if let Some(tl) = &p.tech {
        s.tech = Some(tl.iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect());
        out.push(t!("技术要求改为 {} 条", "Technical requirements: {} lines", tl.len()));
    }
    if !p.title.is_empty() {
        merge_map(&mut s.title, &p.title);
        out.push(t!("标题栏: {}", "Title block: {}", p.title.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(", ")));
    }
    if out.is_empty() {
        return Err(tr("补丁里没有要改的东西", "The patch changes nothing").into());
    }
    Ok(out)
}

/// 工具定义(固定 → 可缓存)。
pub fn tools() -> Value {
    let strlist = json!({"type": "array", "items": {"type": "string"}});
    json!([
        {
            "name": "propose_drawing_changes",
            "description": "Propose a patch to the current part drawing's settings. The user sees the change list and decides whether to apply it; nothing changes until they do. Use dimension names and hole-group ids exactly as given in the context. Only include fields you want to change.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "summary": {"type": "string", "description": "One short sentence for the user, in their language"},
                    "parts": {"type": "array", "items": {"type": "string"}, "description": "To change several sheets at once: the target parts by name or part number from [All parts], or [\"all\"] for every made (not bought) part. Omit to change only the current sheet; when set, include the current part too if it should change. Dimension names differ between sheets - for other sheets prefer settings that don't name dimensions (title, tech, density, threads, section)."},
                    "orient": {"type": "string", "enum": ["flip", "rotate", "back", "auto"], "description": "Front view orientation: flip = upside down, rotate = 90°, back = view from the opposite side, auto = automatic"},
                    "section": {"type": "boolean", "description": "Automatic section view A-A on/off"},
                    "ordinate": {"description": "Hole groups that get coordinate (position) dimensions: array of group ids, or the string \"all\" for every group in the front view", "anyOf": [{"type": "array", "items": {"type": "string"}}, {"type": "string", "enum": ["all"]}]},
                    "threads": {"description": "Hole diameter (mm, as text) -> thread size for tapped holes without a hole record, e.g. {\"3.3\": \"M4\"}; empty value removes", "type": "object", "additionalProperties": {"type": "string"}},
                    "density": {"type": "number", "description": "Material density g/cm3 (weight = volume x density)"},
                    "hide": {"description": "Dimension names to hide", "type": "array", "items": {"type": "string"}},
                    "show": {"description": "Hidden dimension names to show again", "type": "array", "items": {"type": "string"}},
                    "key_add": {"description": "Dimension names to mark as key dimensions (*)", "type": "array", "items": {"type": "string"}},
                    "key_remove": strlist,
                    "dim_text": {"description": "Dimension name -> replacement text; empty value restores the automatic text", "type": "object", "additionalProperties": {"type": "string"}},
                    "dim_tol": {"description": "Dimension name -> tolerance: ISO 286 code like h7 / H7, or \"±0.05\", or \"+0.1/-0.05\"; empty value removes", "type": "object", "additionalProperties": {"type": "string"}},
                    "tech": {"description": "Full replacement of the technical requirements, one item per line", "type": "array", "items": {"type": "string"}},
                    "title": {"description": "Title block fields for this sheet (designer, reviewer, project, finish, hardness, version, pn, name, material); empty value removes this sheet's override", "type": "object", "additionalProperties": {"type": "string"}}
                },
                "required": ["summary"]
            }
        },
        {
            "name": "remember_rule",
            "description": "Propose a standing drawing rule to remember for all future drawings (e.g. 'always give bearing bores H7'). Use only when the user states a general preference, not a one-off fix. The user confirms before it is saved.",
            "input_schema": {
                "type": "object",
                "properties": {"rule": {"type": "string", "description": "The rule, one sentence, in the user's language"}},
                "required": ["rule"]
            }
        }
    ])
}

/// 系统提示: 角色 + 能改什么 + 用户记下的长期规则(规则少变 → 整段可缓存)。
pub fn system(rules: &[String]) -> String {
    let mut s = String::from(
        "You help a mechanical designer adjust automatically generated 2D part drawings (Chinese GB standard, first-angle projection, \
GB/T 1800 tolerances, GB/T 1804-f general tolerances). The drawing is produced by a compiler from a settings object; you change it \
only by proposing a patch with the propose_drawing_changes tool. The user reviews every patch and applies it themselves.\n\n\
What a patch can change: front-view orientation (flip / rotate 90° / view the back / auto), section A-A on or off, which hole groups get \
coordinate dimensions, hole diameter -> thread size, density, hide or show individual dimensions, key-dimension marks (*), dimension text, \
dimension tolerances, technical requirements, title block fields.\n\
What it cannot do: move individual dimensions or views to coordinates, add new dimensions (the user adds those by picking faces in \
Plasticity with the + button), change the model. If asked for that, say briefly what the user can do instead.\n\n\
Each user turn carries the current settings, the last compile report (placed dimensions with names and values, hole groups, warnings) and \
usually an image of the current sheet. Refer to dimensions by the names in the report. If a request is ambiguous (which hole? which face?), \
ask one short question instead of guessing. When the user asks for the same change on several sheets (\"all aluminium parts\", \"these three \
plates\"), propose one patch with `parts` listing them from [All parts]. When the user states a general preference that should hold for future \
drawings, also offer remember_rule. Answer in the user's language, briefly; no long explanations of drafting theory unless asked.",
    );
    if !rules.is_empty() {
        s.push_str("\n\nThe user's standing drawing rules (apply them unless the user says otherwise):\n");
        for r in rules {
            s.push_str("- ");
            s.push_str(r);
            s.push('\n');
        }
    }
    s
}

/// 本轮的上下文(当前方案 + 报告摘要), 放在用户消息里。
pub fn context(s: &Sheet, rep: Option<&Report>, bom_title: &BTreeMap<String, String>, uploaded_locked: bool) -> String {
    context_with(s, rep, bom_title, uploaded_locked, &[])
}

/// 同 `context`, 再附上整个 BOM 的零件清单(一句话改多张图时模型按名字 / 料号点名)。
pub fn context_with(s: &Sheet, rep: Option<&Report>, bom_title: &BTreeMap<String, String>, uploaded_locked: bool, all: &[crate::mcp_bridge::PartInfo]) -> String {
    let sheet = json!({
        "front": s.front, "up": s.up, "section": s.section, "ordinate": s.ordinate, "threads": s.threads, "density": s.density,
        "hidden": s.dims_hide, "key": s.dims_key, "dim_text": s.dims_text, "dim_tol": s.dims_tol, "tech": s.tech, "title": s.title,
        "added_dims": s.dims_add.iter().map(|a| &a.name).collect::<Vec<_>>(),
    });
    let report = rep.map(|r| {
        json!({
            "paper_mm": r.paper_mm,
            "dimensions": r.placed.iter().map(|p| json!({"name": p.name, "text": p.shown.clone().or(p.text.clone()), "value": p.value, "hidden": p.hidden})).collect::<Vec<_>>(),
            "hole_groups": r.hole_groups.iter().map(|g| json!({"id": g.id, "view": g.view, "text": g.text, "count": g.count, "coordinates": g.ordinate})).collect::<Vec<_>>(),
            "warnings": r.warnings, "tech_in_use": r.tech,
        })
    });
    let mut out = format!(
        "[Part] {}\n[Settings] {}\n[Last compile report] {}",
        serde_json::to_string(bom_title).unwrap_or_default(),
        sheet,
        report.map(|r| r.to_string()).unwrap_or_else(|| "none (not drawn yet)".into())
    );
    if uploaded_locked {
        out.push_str("\n[Note] This part is locked to a drawing the user uploaded; patches only take effect after they unlock it.");
    }
    if !all.is_empty() {
        out.push_str("\n[All parts] ");
        out.push_str(
            &all.iter()
                .map(|p| format!("{}{}{}{}", if p.pn.is_empty() { String::new() } else { format!("{} ", p.pn) }, p.name, if p.material.is_empty() { String::new() } else { format!(" ({})", p.material) }, if p.bought { " [bought]" } else { "" }))
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    out
}

/// 补丁的目标零件 key: parts 空 = 当前这张; "all" = 全部自制件; 否则按名称 / 料号找(找不到报错)。
pub fn targets(p: &Patch, current: &str, all: &[crate::mcp_bridge::PartInfo]) -> Result<Vec<String>, String> {
    if p.parts.is_empty() {
        return Ok(vec![current.to_string()]);
    }
    if p.parts.iter().any(|x| x.eq_ignore_ascii_case("all")) {
        return Ok(all.iter().filter(|x| !x.bought).map(|x| x.key.clone()).collect());
    }
    let mut out: Vec<String> = Vec::new();
    for q in &p.parts {
        let k = crate::mcp_bridge::resolve(all, q)?.key.clone();
        if !out.contains(&k) {
            out.push(k);
        }
    }
    Ok(out)
}

/// 等用户决定的提议。
#[derive(Debug, Clone)]
pub enum Proposal {
    Patch(Patch),
    Rule(String),
}

/// 对话里的一行(界面显示)。
#[derive(Debug, Clone)]
pub enum Line {
    User(String),
    Assistant(String),
    /// 提议 + 改动预览; decided = Some(应用了?)。
    Proposal { id: String, prop: Proposal, preview: Vec<String>, decided: Option<bool> },
    Error(String),
}

/// 一张图的对话(内存里; 换零件 / 换文档就清空)。
#[derive(Debug, Clone, Default)]
pub struct Chat {
    pub key: String,
    /// 发给 API 的原始消息(assistant 内容原样回传: thinking 块要保持不变)。
    pub history: Vec<Value>,
    pub lines: Vec<Line>,
    /// 还没回 tool_result 的 tool_use: id → 结果文字(决定之前 = "等待用户")。
    pub open_tools: Vec<(String, String)>,
    /// 上次发过的预览图(同一张不重复发)。
    pub last_image: String,
}

impl Chat {
    pub fn new(key: &str) -> Self {
        Chat { key: key.into(), ..Default::default() }
    }

    /// 用户对某个提议的决定 → 记下, 下一条用户消息里作为 tool_result 回给模型。
    pub fn decide(&mut self, id: &str, applied: bool, note: &str) {
        for l in &mut self.lines {
            if let Line::Proposal { id: i, decided, .. } = l {
                if i == id {
                    *decided = Some(applied);
                }
            }
        }
        let text = if applied { format!("The user applied it. {note}") } else { format!("The user rejected it. {note}") };
        for t in &mut self.open_tools {
            if t.0 == id {
                t.1 = text.trim().to_string();
            }
        }
    }

    /// 组一条用户消息: 先回全部没回的 tool_result, 再是上下文 + 用户的话(+ 预览图)。
    pub fn push_user(&mut self, text: &str, ctx: &str, image: Option<(&str, &[u8])>) {
        let mut content: Vec<Value> = self
            .open_tools
            .drain(..)
            .map(|(id, r)| json!({"type": "tool_result", "tool_use_id": id, "content": if r.is_empty() { "The user has not decided yet and moved on.".to_string() } else { r }}))
            .collect();
        if let Some((tag, png)) = image {
            if tag != self.last_image {
                content.push(json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": b64(png)}}));
                self.last_image = tag.to_string();
            }
        }
        content.push(json!({"type": "text", "text": format!("{ctx}\n\n[User] {text}")}));
        self.history.push(json!({"role": "user", "content": content}));
        self.lines.push(Line::User(text.to_string()));
    }
}

/// 一轮的结果(后台线程算完交回界面): 新增的 history 条目 + 显示行 + 待回的 tool_use。
#[derive(Debug, Clone, Default)]
pub struct TurnOut {
    pub history_add: Vec<Value>,
    pub lines: Vec<Line>,
    pub open_tools: Vec<(String, String)>,
}

/// 跑一轮(阻塞): 调 API; 工具输入解析失败 → 回 is_error 让模型改, 最多 3 次; 合法的提议停下来等用户。
/// `preview` 用来在提议时算改动预览(打到副本上, 不改原图)。
pub fn run_turn(cfg: &crate::llm::Config, rules: &[String], history: &[Value], preview: &dyn Fn(&Patch) -> Result<Vec<String>, String>) -> Result<TurnOut, String> {
    let mut out = TurnOut::default();
    let mut msgs = history.to_vec();
    for _ in 0..4 {
        let body = json!({
            "system": [{"type": "text", "text": system(rules), "cache_control": {"type": "ephemeral"}}],
            "tools": tools(),
            "messages": msgs,
        });
        let resp = crate::llm::messages(cfg, body)?;
        let content = resp["content"].clone();
        let stop = resp["stop_reason"].as_str().unwrap_or("");
        if stop == "refusal" {
            return Err(tr("模型拒绝了这个请求(安全分类器)", "The model declined this request (safety classifier)").into());
        }
        let assistant = json!({"role": "assistant", "content": content});
        msgs.push(assistant.clone());
        out.history_add.push(assistant);
        let text = crate::llm::text_of(&resp);
        if !text.trim().is_empty() {
            out.lines.push(Line::Assistant(text.trim().to_string()));
        }
        if stop == "max_tokens" {
            out.lines.push(Line::Error(tr("回答被截断(max_tokens)", "The answer was cut off (max_tokens)").into()));
        }
        let uses: Vec<&Value> = content.as_array().into_iter().flatten().filter(|b| b["type"] == "tool_use").collect();
        if uses.is_empty() {
            return Ok(out);
        }
        // 每个 tool_use 都要回 tool_result: 合法提议 → 留着等用户(下一条用户消息里回); 不合法 → 马上回错误再跑一次
        let mut errors: Vec<Value> = Vec::new();
        let mut parked: Vec<(String, String)> = Vec::new();
        for u in uses {
            let id = u["id"].as_str().unwrap_or("").to_string();
            let input = &u["input"];
            let r: Result<(Proposal, Vec<String>), String> = match u["name"].as_str() {
                Some("propose_drawing_changes") => serde_json::from_value::<Patch>(input.clone())
                    .map_err(|e| format!("invalid input: {e}"))
                    .and_then(|p| preview(&p).map(|v| (Proposal::Patch(p), v))),
                Some("remember_rule") => match input["rule"].as_str().map(str::trim).filter(|r| !r.is_empty()) {
                    Some(r) => Ok((Proposal::Rule(r.to_string()), vec![r.to_string()])),
                    None => Err("rule is empty".into()),
                },
                other => Err(format!("unknown tool {other:?}")),
            };
            match r {
                Ok((prop, pv)) => {
                    out.lines.push(Line::Proposal { id: id.clone(), prop, preview: pv, decided: None });
                    parked.push((id, String::new()));
                }
                Err(e) => errors.push(json!({"type": "tool_result", "tool_use_id": id, "is_error": true, "content": e})),
            }
        }
        if errors.is_empty() {
            out.open_tools = parked;
            return Ok(out);
        }
        // 有错误: 合法的那些先回"已展示给用户", 错的回错误, 让模型重来
        let mut content: Vec<Value> = parked.iter().map(|(id, _)| json!({"type": "tool_result", "tool_use_id": id, "content": "Shown to the user; waiting for their decision."})).collect();
        content.extend(errors);
        let user = json!({"role": "user", "content": content});
        msgs.push(user.clone());
        out.history_add.push(user);
    }
    Err(tr("模型多次给出无效的补丁", "The model kept producing invalid patches").into())
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

/// 应用过的补丁记一行到 `<base>.agent.jsonl`(D6 案例层样本: 用户说了什么 → 改了什么)。
pub fn log_applied(dir: &std::path::Path, s: &Sheet, request: &str, p: &Patch, changes: &[String]) {
    use std::io::Write;
    let line = json!({"date": crate::parts_lib::today(), "key": s.key, "request": request, "patch": p, "changes": changes});
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("{}.agent.jsonl", s.base))) {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::{HoleGroup, Placed};

    fn rep() -> Report {
        let mut r = Report::default();
        r.placed = vec![
            Placed { name: "LenW".into(), text: Some("120".into()), shown: None, value: Some(120.0), at: [0.0, 0.0], hidden: false },
            Placed { name: "DiaOuter".into(), text: Some("⌀20".into()), shown: None, value: Some(20.0), at: [0.0, 0.0], hidden: false },
        ];
        r.hole_groups = vec![HoleGroup { id: "H1".into(), view: "Front".into(), text: "4×⌀5.5".into(), count: 4, ordinate: true }];
        r.axes = Some(Axes { front: [0.0, 0.0, 1.0], up: [0.0, 1.0, 0.0], right: [1.0, 0.0, 0.0] });
        r
    }

    #[test]
    fn patch_apply_and_validation() {
        let r = rep();
        let mut s = Sheet::new("k", "P1", "m", "Plate");
        let p: Patch = serde_json::from_value(json!({
            "summary": "x", "orient": "flip", "section": false, "ordinate": ["H1"], "hide": ["LenW"],
            "dim_tol": {"DiaOuter": "h7"}, "threads": {"3.3": "M4"}, "title": {"finish": "阳极氧化"}
        }))
        .unwrap();
        let ch = apply(&mut s, &p, Some(&r)).unwrap();
        assert_eq!(ch.len(), 7, "{ch:?}");
        assert_eq!((s.front, s.up), (Some([0.0, 0.0, 1.0]), Some([0.0, -1.0, 0.0])));
        assert!(!s.section && s.ordinate == Some(vec!["H1".to_string()]) && s.dims_hide == vec!["LenW"]);
        assert_eq!(s.dims_tol["DiaOuter"], "h7");
        // 空串 = 去掉; "all" = 全部孔组
        let p2: Patch = serde_json::from_value(json!({"summary": "", "dim_tol": {"DiaOuter": ""}, "ordinate": "all", "show": ["LenW"]})).unwrap();
        apply(&mut s, &p2, Some(&r)).unwrap();
        assert!(s.dims_tol.is_empty() && s.ordinate.is_none() && s.dims_hide.is_empty());
        // 不认识的尺寸 / 孔组 / 空补丁 → 报错, 图不动
        let before = s.clone();
        for bad in [json!({"hide": ["Nope"]}), json!({"ordinate": ["H9"]}), json!({"summary": "nothing"})] {
            let p: Patch = serde_json::from_value(bad).unwrap();
            assert!(apply(&mut s.clone(), &p, Some(&r)).is_err());
        }
        assert_eq!(s, before);
        // 没出过图(没有报告)不能改朝向
        assert!(apply(&mut s.clone(), &serde_json::from_value(json!({"orient": "rotate"})).unwrap(), None).is_err());
    }

    #[test]
    fn multi_sheet_targets() {
        use crate::mcp_bridge::PartInfo;
        let all = vec![
            PartInfo { key: "a".into(), pn: "904002001".into(), name: "Base".into(), material: "6061".into(), ..Default::default() },
            PartInfo { key: "b".into(), name: "Cover".into(), ..Default::default() },
            PartInfo { key: "s".into(), name: "M4 screw".into(), bought: true, ..Default::default() },
        ];
        let p = |v: Value| serde_json::from_value::<Patch>(v).unwrap();
        assert_eq!(targets(&p(json!({"summary": ""})), "a", &all).unwrap(), vec!["a"]);
        assert_eq!(targets(&p(json!({"parts": ["all"]})), "a", &all).unwrap(), vec!["a", "b"], "all = 自制件");
        assert_eq!(targets(&p(json!({"parts": ["904002001", "cover", "Base"]})), "a", &all).unwrap(), vec!["a", "b"]);
        assert!(targets(&p(json!({"parts": ["nope"]})), "a", &all).is_err());
        let c = context_with(&Sheet::new("a", "B", "", ""), None, &BTreeMap::new(), false, &all);
        assert!(c.contains("[All parts] 904002001 Base (6061); Cover; M4 screw [bought]"), "{c}");
    }

    #[test]
    fn chat_tool_results_and_images() {
        let mut c = Chat::new("k");
        c.push_user("把外径标 h7", "[ctx]", Some(("img1", b"abc")));
        let m = &c.history[0]["content"];
        assert_eq!(m[0]["type"], "image");
        assert_eq!(m[0]["source"]["data"], "YWJj");
        // 模型提了一个补丁, 用户应用了 → 下一条用户消息先回 tool_result; 同一张图不重发
        c.open_tools.push(("toolu_1".into(), String::new()));
        c.lines.push(Line::Proposal { id: "toolu_1".into(), prop: Proposal::Rule("r".into()), preview: vec![], decided: None });
        c.decide("toolu_1", true, "");
        c.push_user("再看看", "[ctx]", Some(("img1", b"abc")));
        let m = &c.history[1]["content"];
        assert_eq!((m[0]["type"].as_str(), m[0]["tool_use_id"].as_str()), (Some("tool_result"), Some("toolu_1")));
        assert!(m[0]["content"].as_str().unwrap().starts_with("The user applied"));
        assert_eq!(m.as_array().unwrap().len(), 2);
        assert!(c.open_tools.is_empty());
        assert!(matches!(c.lines[1], Line::Proposal { decided: Some(true), .. }));
        assert_eq!(b64(b"ab"), "YWI=");
        // 工具定义 / 系统提示带上规则
        assert_eq!(tools().as_array().unwrap().len(), 2);
        assert!(system(&["轴承孔一律 H7".into()]).contains("轴承孔一律 H7"));
    }
}
