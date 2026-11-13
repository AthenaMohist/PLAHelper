//! F1: 页面内垫片 `window.__PLA__` 的 Rust 侧（设计见 docs/know-how/cdp-shim-design.md）。
//!
//! 单一来源（R3/R4）：
//!   * 接口名只在 [`api`] 里定义一次，与 `js/pla-shim.js` 的 `def()` 一一对应（契约测试保证）；
//!   * 全局名、编辑器特征、DOM 选择器等常量只在这里定义，经配置注入垫片，其他 crate 引用这里。
//!   * 传参一律 JSON 序列化，不做字符串替换拼代码。

use serde_json::{json, Value};

/// 垫片源码（整个文件是一个函数表达式 `(function (CFG) {...})`）。
pub const SHIM_JS: &str = include_str!("../js/pla-shim.js");

/// 接口契约版本：必须与 `pla-shim.js` 里的 `API_VERSION` 相等（契约测试强制）。
pub const API_VERSION: u32 = 1;

/// 垫片内容哈希（FNV-1a 64，构建期计算）：页面里 rev 不同即热替换。
pub const REV: u64 = fnv1a(SHIM_JS.as_bytes());

/// 代理堆快照定位到 Editor 后挂在页面上的全局名（代理与垫片共用的交接点）。
pub const EDITOR_GLOBAL: &str = "__PLA_EDITOR__";

/// Editor 特征：同时拥有这些属性的对象即 Editor。垫片校验与堆快照搜索共用这一份。
pub const EDITOR_SIGNATURE: &[&str] = &["selection", "shading", "viewports"];

/// 视口自定义元素（`orbit:rotate:*` / `viewport:*` 命令的监听器挂在它上面；keymap 也用它做选择器）。
pub const VIEWPORT_SELECTOR: &str = "plasticity-viewport";

/// 自检用空命令名（`cmd.ping` 注册并派发它，验证"注册 → 派发 → 回调"整条链）。
pub const PING_COMMAND: &str = "pla:shim:ping";

/// 滚转命令名(垫片在视口元素上注册; pla-ui keymap 把 Alt+←/→ 绑到它们)。
pub const ROLL_LEFT_COMMAND: &str = "pla:roll:left";
pub const ROLL_RIGHT_COMMAND: &str = "pla:roll:right";

/// 接口名（与 `pla-shim.js` 的 `def()` 一一对应；新增接口两边同时加，契约测试会检查）。
pub mod api {
    pub const CORE_PROBE: &str = "core.probe";
    pub const CORE_HEALTH: &str = "core.health";
    pub const CORE_APIS: &str = "core.apis";
    pub const SEL_SNAPSHOT: &str = "sel.snapshot";
    pub const HIST_STATE: &str = "hist.state";
    pub const CAM_GET: &str = "cam.get";
    pub const CAM_SET: &str = "cam.set";
    pub const CMD_DISPATCH: &str = "cmd.dispatch";
    pub const CMD_PING: &str = "cmd.ping";
    pub const ERR_LAST: &str = "err.last";
    pub const TREE_LIST: &str = "tree.list";
    pub const SEL_CANCEL_COMMAND: &str = "sel.cancelCommand";
    pub const SEL_CLEAR: &str = "sel.clear";
    pub const SEL_ADD_FACE_BY_ID: &str = "sel.addFaceById";
    pub const SEL_FACE_IDS: &str = "sel.faceIds";
    pub const SCAN_BEGIN: &str = "scan.begin";
    pub const SCAN_READ: &str = "scan.read";
    pub const SCAN_CLEAR: &str = "scan.clear";
    pub const SCAN_END: &str = "scan.end";
    pub const CAM_NAVIGATE: &str = "cam.navigateToSelection";
    pub const CAM_RENDER: &str = "cam.render";
    pub const CAM_VP_RECT: &str = "cam.vpRect";
    pub const CAM_SET_ORBIT_STEP: &str = "cam.setOrbitStep";
    pub const CAM_ROLL: &str = "cam.roll";
    pub const KEYMAP_SET_BINDINGS: &str = "keymap.setBindings";
    pub const VP_SAVE: &str = "vp.save";
    pub const VP_RESTORE: &str = "vp.restore";

    /// 全部接口名（契约测试用）。
    pub const ALL: &[&str] = &[
        CORE_PROBE, CORE_HEALTH, CORE_APIS, SEL_SNAPSHOT, HIST_STATE, CAM_GET, CAM_SET, CMD_DISPATCH,
        CMD_PING, ERR_LAST, TREE_LIST, SEL_CANCEL_COMMAND, SEL_CLEAR, SEL_ADD_FACE_BY_ID, SEL_FACE_IDS,
        SCAN_BEGIN, SCAN_READ, SCAN_CLEAR, SCAN_END, CAM_NAVIGATE, CAM_RENDER, CAM_VP_RECT,
        CAM_SET_ORBIT_STEP, CAM_ROLL, KEYMAP_SET_BINDINGS, VP_SAVE, VP_RESTORE,
    ];
}

/// 能力名（与 `pla-shim.js` 的 `CAPS` 键一一对应；契约测试检查）。功能 → 能力的映射在 pla-ui `features.rs`。
pub mod cap {
    pub const EDITOR: &str = "editor";
    pub const SEL_READ: &str = "sel.read";
    pub const SEL_WRITE: &str = "sel.write";
    pub const HIST_READ: &str = "hist.read";
    pub const CAM_READ: &str = "cam.read";
    pub const CAM_WRITE: &str = "cam.write";
    pub const CAM_NAV: &str = "cam.nav";
    pub const CAM_STEP: &str = "cam.step";
    pub const CAM_ROLL: &str = "cam.roll";
    pub const SCAN: &str = "scan";
    pub const VP_STATE: &str = "vp.state";
    pub const KEYMAP: &str = "keymap";
    pub const CMD_DISPATCH: &str = "cmd.dispatch";
    pub const ERR_HOOK: &str = "err.hook";
    pub const TREE: &str = "tree";
    pub const MAT_SET: &str = "mat.set";
    pub const CMD_EXEC: &str = "cmd.exec";
    pub const BOOL_RUN: &str = "bool.run";
    pub const ASSET_PLACE: &str = "asset.place";

    /// 全部能力名（契约测试用）。
    pub const ALL: &[&str] = &[
        EDITOR, SEL_READ, SEL_WRITE, HIST_READ, CAM_READ, CAM_WRITE, CAM_NAV, CAM_STEP, CAM_ROLL, SCAN,
        VP_STATE, KEYMAP, CMD_DISPATCH, ERR_HOOK, TREE, MAT_SET, CMD_EXEC, BOOL_RUN, ASSET_PLACE,
    ];
}

/// 一项能力的探测结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CapState {
    pub ok: bool,
    /// 缺失的必需内部结构（垫片 LOC 表键）。
    pub missing: Vec<String>,
    /// 缺失的可选内部结构（能力降级但可用）。
    pub optional_missing: Vec<String>,
    /// 路线图预留、尚未实现（值 = 待办编号，如 "N1"）。
    pub todo: Option<String>,
}

/// F2: 一次能力探测的结果（`core.probe` 回包的结构化形式）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CapsReport {
    /// 是否找到 Editor。
    pub editor: bool,
    /// 找到 Editor 的途径（global / preact）。
    pub finder: Option<String>,
    /// 垫片 rev。
    pub rev: String,
    pub caps: std::collections::BTreeMap<String, CapState>,
    /// 走了备选路径的表键 → 实际命中的路径（Plasticity 改过名的痕迹）。
    pub alt_paths: std::collections::BTreeMap<String, String>,
    /// 官方 Bridge 服务端状态（环境诊断，不是能力）。
    pub bridge_server: Option<BridgeServer>,
}

/// Plasticity 内置 Bridge（WebSocket 服务端）状态。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BridgeServer {
    /// ws 库 `_state`：0 运行中 / 1 关闭中 / 2 已关闭；None = 读不到。
    pub state: Option<i64>,
    pub port: Option<i64>,
    /// 偏好设置里 Server 开关。
    pub enabled: bool,
}

impl BridgeServer {
    /// 设置里开着、服务端却已关闭（典型：启动时端口被占用，监听失败）。
    pub fn closed_while_enabled(&self) -> bool {
        self.enabled && matches!(self.state, Some(1) | Some(2))
    }
}

fn str_list(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

impl CapsReport {
    /// 由 `core.probe` 的 v 构造。
    pub fn from_probe(v: &Value) -> Self {
        let caps = v["caps"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, c)| {
                        (
                            k.clone(),
                            CapState {
                                ok: c["ok"].as_bool() == Some(true),
                                missing: str_list(&c["missing"]),
                                optional_missing: str_list(&c["optionalMissing"]),
                                todo: c["todo"].as_str().map(String::from),
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let alt_paths = v["altPaths"]
            .as_object()
            .map(|m| m.iter().map(|(k, p)| (k.clone(), p.as_str().unwrap_or("").to_string())).collect())
            .unwrap_or_default();
        CapsReport {
            editor: v["editor"]["ok"].as_bool() == Some(true),
            finder: v["editor"]["finder"].as_str().map(String::from),
            rev: v["rev"].as_str().unwrap_or("").to_string(),
            caps,
            alt_paths,
            bridge_server: v["env"]["bridge"].as_object().map(|b| BridgeServer {
                state: b.get("state").and_then(|x| x.as_i64()),
                port: b.get("port").and_then(|x| x.as_i64()),
                enabled: b.get("enabled").and_then(|x| x.as_bool()) == Some(true),
            }),
        }
    }

    /// 给定一组能力，返回不可用的那些：(能力名, 缺失的内部结构)。
    /// 报告里没有的能力名视为不可用（垫片版本不含该能力），缺失列表写 `未知能力`。
    pub fn blocked(&self, caps: &[&str]) -> Vec<(String, Vec<String>)> {
        caps.iter()
            .filter_map(|c| match self.caps.get(*c) {
                Some(s) if s.ok => None,
                Some(s) if s.todo.is_some() => Some((c.to_string(), vec![format!("尚未实现({})", s.todo.as_deref().unwrap_or(""))])),
                Some(s) => Some((c.to_string(), s.missing.clone())),
                None => Some((c.to_string(), vec!["未知能力".into()])),
            })
            .collect()
    }

    /// 已实现能力里失效的数量（todo 不计）。
    pub fn failed_count(&self) -> usize {
        self.caps.values().filter(|s| !s.ok && s.todo.is_none()).count()
    }
}

const fn fnv1a(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < b.len() {
        h ^= b[i] as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    h
}

/// rev 的字符串形（JS 数字装不下 u64，按十六进制字符串传）。
pub fn rev_str() -> String {
    format!("{REV:016x}")
}

/// 注入配置（R4：常量从这里流向垫片）。
pub fn config_json() -> Value {
    json!({
        "rev": rev_str(),
        "editorGlobal": EDITOR_GLOBAL,
        "editorSignature": EDITOR_SIGNATURE,
        "viewportSelector": VIEWPORT_SELECTOR,
        "pingCommand": PING_COMMAND,
        "rollCommands": { "left": ROLL_LEFT_COMMAND, "right": ROLL_RIGHT_COMMAND },
    })
}

/// 安装表达式：`<垫片>(<配置>)`，返回 JSON 字符串 `{installed, api, rev, editor}` 或 `{installed:false, same:true}`。
pub fn install_expr() -> String {
    format!("{SHIM_JS}({})", config_json())
}

/// 探活表达式：`ok` = 垫片在且找得到 editor；`noed` = 垫片在但没有 editor；`stale` = 无垫片或接口版本不符。
pub fn probe_expr() -> String {
    format!(
        "(function(){{var p=window.__PLA__;if(!p||p.api!=={API_VERSION})return 'stale';return p.hasEditor()?'ok':'noed'}})()"
    )
}

/// 单次调用表达式。
pub fn call_expr(name: &str, args: &Value) -> String {
    format!(
        "(function(){{var p=window.__PLA__;return p?p.call({},{}):JSON.stringify({{ok:false,err:'no-shim'}})}})()",
        Value::String(name.into()),
        args
    )
}

/// 批量调用表达式：每项独立成败，回包是以接口名为键的对象。
pub fn batch_expr(calls: &[(&str, Value)]) -> String {
    let list: Vec<Value> = calls.iter().map(|(n, a)| json!([n, a])).collect();
    format!(
        "(function(){{var p=window.__PLA__;return p?p.batch({}):JSON.stringify({{__noShim:true}})}})()",
        Value::Array(list)
    )
}

/// 垫片调用失败的分类（面板据此显示"哪项能力失效、缺什么内部结构"）。
#[derive(Debug, Clone, PartialEq)]
pub enum ShimErr {
    /// 页面里没有垫片（刚重载 / 未注入）。
    NoShim,
    /// 找不到 Editor。
    NoEditor,
    /// 能力失效：cap = 能力名，missing = 缺失的内部结构表键。
    Cap { cap: String, missing: Vec<String> },
    /// 垫片内抛异常。
    Exc(String),
    /// 回包格式不对 / 未知接口。
    Bad(String),
}

impl std::fmt::Display for ShimErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShimErr::NoShim => write!(f, "垫片未注入"),
            ShimErr::NoEditor => write!(f, "未找到 Plasticity 编辑器"),
            ShimErr::Cap { cap, missing } => write!(f, "能力 {cap} 不可用(缺 {})", missing.join(", ")),
            ShimErr::Exc(m) => write!(f, "垫片异常: {m}"),
            ShimErr::Bad(m) => write!(f, "垫片回包异常: {m}"),
        }
    }
}

impl std::error::Error for ShimErr {}

/// 解析一项 `{ok, v | err, missing}`。
pub fn parse_one(v: &Value) -> Result<Value, ShimErr> {
    if v["ok"].as_bool() == Some(true) {
        return Ok(v.get("v").cloned().unwrap_or(Value::Null));
    }
    let err = v["err"].as_str().unwrap_or("");
    if err == "no-shim" {
        return Err(ShimErr::NoShim);
    }
    if err == "cap:editor" {
        return Err(ShimErr::NoEditor);
    }
    if let Some(cap) = err.strip_prefix("cap:") {
        let missing = v["missing"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        return Err(ShimErr::Cap { cap: cap.to_string(), missing });
    }
    if let Some(m) = err.strip_prefix("exc:") {
        return Err(ShimErr::Exc(m.to_string()));
    }
    Err(ShimErr::Bad(if err.is_empty() { v.to_string() } else { err.to_string() }))
}

/// 解析单次调用回包（JSON 字符串）。
pub fn parse_reply(raw: &str) -> Result<Value, ShimErr> {
    let v: Value = serde_json::from_str(raw).map_err(|e| ShimErr::Bad(format!("{e}: {raw:.120}")))?;
    parse_one(&v)
}

/// 解析批量回包：返回 (接口名 → 结果)。整包缺垫片 → 每项都是 NoShim。
pub fn parse_batch(raw: &str, names: &[&str]) -> Vec<(String, Result<Value, ShimErr>)> {
    let v: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => {
            return names
                .iter()
                .map(|n| (n.to_string(), Err(ShimErr::Bad(format!("{e}")))))
                .collect()
        }
    };
    names
        .iter()
        .map(|n| {
            let r = if v["__noShim"].as_bool() == Some(true) {
                Err(ShimErr::NoShim)
            } else {
                match v.get(*n) {
                    Some(item) => parse_one(item),
                    None => Err(ShimErr::Bad(format!("缺少 {n} 的回包"))),
                }
            };
            (n.to_string(), r)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shim_source_is_function_expression() {
        let s = SHIM_JS.trim_start_matches('\u{feff}');
        let body: String = s.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        assert!(body.trim_start().starts_with("(function (CFG)"), "垫片必须是函数表达式");
        assert!(body.trim_end().ends_with("})"), "垫片必须以 }}) 结尾, 便于拼接 (配置)");
        assert!(SHIM_JS.contains(&format!("var API_VERSION = {API_VERSION};")), "JS 与 Rust 的 API_VERSION 不一致");
    }

    #[test]
    fn exprs_are_json_safe() {
        let e = call_expr("cam.get", &json!({"a": "x'y\"z"}));
        assert!(e.contains(r#"p.call("cam.get",{"a":"x'y\"z"})"#), "{e}");
        let b = batch_expr(&[(api::SEL_SNAPSHOT, json!({})), (api::CAM_GET, json!(null))]);
        assert!(b.contains(r#"[["sel.snapshot",{}],["cam.get",null]]"#), "{b}");
        assert!(install_expr().ends_with(&format!("({})", config_json())));
    }

    #[test]
    fn parse_errors_are_classified() {
        assert_eq!(parse_reply(r#"{"ok":true,"v":3}"#), Ok(json!(3)));
        assert_eq!(parse_reply(r#"{"ok":false,"err":"no-shim"}"#), Err(ShimErr::NoShim));
        assert_eq!(parse_reply(r#"{"ok":false,"err":"cap:editor","missing":["editor"]}"#), Err(ShimErr::NoEditor));
        assert_eq!(
            parse_reply(r#"{"ok":false,"err":"cap:hist.read","missing":["ed.undoStack"]}"#),
            Err(ShimErr::Cap { cap: "hist.read".into(), missing: vec!["ed.undoStack".into()] })
        );
        assert_eq!(parse_reply(r#"{"ok":false,"err":"exc:boom"}"#), Err(ShimErr::Exc("boom".into())));
        assert!(matches!(parse_reply("nope"), Err(ShimErr::Bad(_))));
    }

    #[test]
    fn parse_batch_isolates_items() {
        let raw = r#"{"sel.snapshot":{"ok":true,"v":{"size":0,"items":[]}},"hist.state":{"ok":false,"err":"cap:hist.read","missing":["ed.undoStack"]}}"#;
        let r = parse_batch(raw, &[api::SEL_SNAPSHOT, api::HIST_STATE, api::CAM_GET]);
        assert!(r[0].1.is_ok());
        assert!(matches!(r[1].1, Err(ShimErr::Cap { .. })));
        assert!(matches!(r[2].1, Err(ShimErr::Bad(_))));
        let none = parse_batch(r#"{"__noShim":true}"#, &[api::CAM_GET]);
        assert_eq!(none[0].1, Err(ShimErr::NoShim));
    }

    #[test]
    fn caps_report_from_probe() {
        let v = json!({
            "rev": "ab", "editor": {"ok": true, "finder": "preact"},
            "caps": {
                "sel.read": {"ok": true, "missing": []},
                "sel.write": {"ok": false, "missing": ["ed.click.addById"]},
                "vp.state": {"ok": true, "missing": [], "optionalMissing": ["vp.settings.grid"]},
                "mat.set": {"ok": false, "missing": [], "todo": "N1"}
            },
            "altPaths": {"ed.undoStack": "history.undoStack"},
            "env": {"bridge": {"state": 2, "port": 8980, "enabled": true}}
        });
        let r = CapsReport::from_probe(&v);
        let b = r.bridge_server.clone().unwrap();
        assert!(b.closed_while_enabled(), "设置开着但服务端已关闭");
        assert_eq!(b.port, Some(8980));
        assert!(r.editor);
        assert_eq!(r.finder.as_deref(), Some("preact"));
        assert_eq!(r.failed_count(), 1, "todo 不计入失效");
        assert_eq!(r.caps["vp.state"].optional_missing, vec!["vp.settings.grid".to_string()]);
        assert_eq!(r.alt_paths["ed.undoStack"], "history.undoStack");
        assert!(r.blocked(&[cap::SEL_READ, cap::VP_STATE]).is_empty());
        let b = r.blocked(&[cap::SEL_READ, cap::SEL_WRITE, cap::MAT_SET, "nope"]);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0], ("sel.write".into(), vec!["ed.click.addById".into()]));
        assert!(b[1].1[0].contains("N1"));
        assert_eq!(b[2].1, vec!["未知能力".to_string()]);
    }

    /// 契约测试（R3）：用 node 在模拟页面里执行 Rust 生成的安装表达式，逐项校验接口与能力探测。
    /// 本机无 node 时跳过（打印提示，不算失败）。
    #[test]
    fn shim_contract_node() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("js");
        let tmp = std::env::temp_dir().join(format!("pla-shim-install-{}.js", std::process::id()));
        std::fs::write(&tmp, install_expr()).unwrap();
        let out = std::process::Command::new("node")
            .arg(dir.join("test").join("shim-contract.test.js"))
            .arg(&tmp)
            .arg(serde_json::to_string(api::ALL).unwrap())
            .arg(API_VERSION.to_string())
            .arg(serde_json::to_string(cap::ALL).unwrap())
            .output();
        let _ = std::fs::remove_file(&tmp);
        match out {
            Err(e) => eprintln!("[skip] node 不可用, 跳过垫片契约测试: {e}"),
            Ok(o) => {
                let so = String::from_utf8_lossy(&o.stdout);
                let se = String::from_utf8_lossy(&o.stderr);
                eprintln!("{so}{se}");
                assert!(o.status.success(), "垫片契约测试失败:\n{so}\n{se}");
            }
        }
    }
}
