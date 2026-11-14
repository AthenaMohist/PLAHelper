//! F3: CDP 契约自检(`pla-app cdp-selftest`)。
//!
//! 一次 CDP 会话里依次跑:
//!   传输 → 垫片 → 能力 → 只读 → 回环 → 守护
//! 原则: **绝不改动用户文档**。回环只做"读出来原样写回"类操作; 最后对比撤销栈长度、
//! 几何版本、选择、相机, 任何一项变了都判失败。Plasticity 升级后先跑它再切换(F5)。
//!
//! 检查项全部由垫片接口与能力表驱动: 能力表里新增的能力自动出现在「能力」段,
//! 不在这里另写一份清单(cdp-shim-design.md 规则 R2/R3)。

use serde_json::{json, Value};

use crate::cdp::{connect_page, eval_roundtrip, http_get, pick_page, view_delta, WsStream};
use crate::shim::{self, api, CapsReport};

/// 一项检查的结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Pass,
    /// 可用但有问题(降级、走了备选路径、环境告警)。
    Warn,
    Fail,
    /// 未执行(路线图预留 / 依赖的接口尚未迁入垫片)。
    Skip,
}

impl Level {
    pub fn tag(self) -> &'static str {
        match self {
            Level::Pass => "PASS",
            Level::Warn => "WARN",
            Level::Fail => "FAIL",
            Level::Skip => "SKIP",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Check {
    pub stage: &'static str,
    pub name: String,
    pub level: Level,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    /// 自检目标(窗口标题 + HWND)。
    pub target: Option<String>,
    pub checks: Vec<Check>,
    /// 传输层失败, 后续阶段未执行。
    pub aborted: bool,
}

impl Report {
    fn add(&mut self, stage: &'static str, name: impl Into<String>, level: Level, detail: impl Into<String>) {
        self.checks.push(Check { stage, name: name.into(), level, detail: detail.into() });
    }

    pub fn count(&self, l: Level) -> usize {
        self.checks.iter().filter(|c| c.level == l).count()
    }

    /// 进程退出码: 0 = 无失败; 1 = 有失败; 2 = 连不上(传输层失败)。
    pub fn exit_code(&self) -> i32 {
        if self.aborted {
            2
        } else if self.count(Level::Fail) > 0 {
            1
        } else {
            0
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "target": self.target,
            "aborted": self.aborted,
            "summary": {
                "pass": self.count(Level::Pass), "warn": self.count(Level::Warn),
                "fail": self.count(Level::Fail), "skip": self.count(Level::Skip),
            },
            "checks": self.checks.iter().map(|c| json!({
                "stage": c.stage, "name": c.name, "level": c.level.tag(), "detail": c.detail,
            })).collect::<Vec<_>>(),
        })
    }
}

const S_TRANSPORT: &str = "传输";
const S_SHIM: &str = "垫片";
const S_CAPS: &str = "能力";
const S_READ: &str = "只读";
const S_ROUNDTRIP: &str = "回环";
const S_GUARD: &str = "守护";

/// 相机回读偏差阈值(无量纲, 见 `view_delta`)。
const CAM_EPS: f64 = 1e-6;

async fn call(ws: &mut WsStream, mid: &mut u64, name: &str, args: &Value) -> Result<Value, String> {
    let raw = eval_roundtrip(ws, mid, &shim::call_expr(name, args)).await.map_err(|e| format!("{e:#}"))?;
    shim::parse_reply(&raw).map_err(|e| e.to_string())
}

/// 文档状态指纹: (撤销栈长度, 几何版本, 选择快照 JSON)。
async fn fingerprint(ws: &mut WsStream, mid: &mut u64) -> Option<(i64, i64, String)> {
    let h = call(ws, mid, api::HIST_STATE, &Value::Null).await.ok()?;
    let s = call(ws, mid, api::SEL_SNAPSHOT, &Value::Null).await.ok()?;
    Some((h["undo"].as_i64()?, h["geoVer"].as_i64()?, selection_key(&s).join(",")))
}

/// 选择快照 → 排好序的 "kind:id" 列表(与顺序无关地比较选择)。
fn selection_key(snapshot: &Value) -> Vec<String> {
    let mut v: Vec<String> = selection_items(snapshot)
        .iter()
        .map(|i| format!("{}:{}", i["kind"].as_str().unwrap_or(""), i["id"].as_str().unwrap_or("")))
        .collect();
    v.sort();
    v
}

/// 选择快照 → sel.apply 的成员列表 [{kind, id}](元素为数值/字符串原样, 对象形取 id 或 versionId)。
fn selection_items(snapshot: &Value) -> Vec<Value> {
    snapshot["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|it| {
                    let e = &it["e"];
                    let id = match e {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Object(o) => o
                            .get("versionId")
                            .or_else(|| o.get("id"))
                            .map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string()))
                            .unwrap_or_default(),
                        _ => String::new(),
                    };
                    json!({ "kind": it["kind"], "id": id })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 去掉与状态无关、每次都可能变的字段(视口屏幕矩形、DPR、失败段说明)。
/// 视口状态回环的相对容差(见 vp.restore 回环处的说明)。
const VP_EPS: f64 = 1e-4;

fn strip_volatile(v: &Value) -> Value {
    let mut v = v.clone();
    if let Some(o) = v.as_object_mut() {
        for k in ["vpRect", "dpr", "failed", "err"] {
            o.remove(k);
        }
    }
    v
}

/// JSON 结构相等, 数值允许相对误差 eps(相机/轨道量经阻尼更新后有浮点抖动)。
fn json_close(a: &Value, b: &Value, eps: f64) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap_or(0.0), y.as_f64().unwrap_or(0.0));
            (x - y).abs() <= eps * (1.0 + x.abs().max(y.abs()))
        }
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| json_close(p, q, eps)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, p)| y.get(k).is_some_and(|q| json_close(p, q, eps)))
        }
        _ => a == b,
    }
}

/// 列出前 `max` 处差异路径(同 [`json_close`] 的容差), 供"不一致"时直接说明差在哪。
fn json_diff_paths(a: &Value, b: &Value, eps: f64, path: &str, out: &mut Vec<String>, max: usize) {
    if out.len() >= max {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                json_diff_paths(x.get(k).unwrap_or(&Value::Null), y.get(k).unwrap_or(&Value::Null), eps, &format!("{path}.{k}"), out, max);
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                json_diff_paths(p, q, eps, &format!("{path}[{i}]"), out, max);
            }
        }
        _ => {
            if !json_close(a, b, eps) {
                let s = |v: &Value| { let t = v.to_string(); if t.len() > 40 { format!("{}…", &t[..40]) } else { t } };
                out.push(format!("{path}: {} → {}", s(a), s(b)));
            }
        }
    }
}

fn has_numbers(v: &Value, keys: &[&str]) -> Result<(), String> {
    let missing: Vec<&str> = keys.iter().copied().filter(|k| !v[*k].is_number()).collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("缺数值字段 {missing:?}"))
    }
}

/// 跑完整自检。`hwnd` = 指定 Plasticity 窗口; None = 第一个。
pub async fn run(cdp_port: u16, inspector_port: u16, hwnd: Option<isize>) -> Report {
    let mut r = Report::default();

    // ================= 传输 =================
    match http_get(inspector_port, "/json/version").await {
        Ok(_) => r.add(S_TRANSPORT, format!("调试端口 {inspector_port}"), Level::Pass, "Plasticity 以 CDP 模式运行"),
        Err(e) => {
            r.add(S_TRANSPORT, format!("调试端口 {inspector_port}"), Level::Fail,
                format!("无响应({e}) — Plasticity 未带 --inspect 启动, 或未启动"));
            r.aborted = true;
            return r;
        }
    }
    let list: Value = match http_get(cdp_port, "/json/list").await.map(|b| serde_json::from_str(&b)) {
        Ok(Ok(v)) => v,
        other => {
            r.add(S_TRANSPORT, format!("代理 {cdp_port}"), Level::Fail,
                format!("读不到目标列表({other:?}) — 代理未运行(pla-ui 会自动拉起)"));
            r.aborted = true;
            return r;
        }
    };
    let titles: Vec<String> = list
        .as_array()
        .map(|a| a.iter().map(|t| format!("{}[{}]", t["title"].as_str().unwrap_or("?"), t["hwnd"].as_str().unwrap_or("-"))).collect())
        .unwrap_or_default();
    r.add(S_TRANSPORT, format!("代理 {cdp_port}"), Level::Pass, format!("{} 个窗口: {}", titles.len(), titles.join(", ")));
    let want = hwnd.unwrap_or(0);
    match pick_page(&list, want) {
        Ok(p) => {
            let t = format!("{} (HWND {})", p["title"].as_str().unwrap_or("?"), p["hwnd"].as_str().unwrap_or("-"));
            r.target = Some(t.clone());
            r.add(S_TRANSPORT, "目标窗口", Level::Pass, t);
        }
        Err(e) => {
            r.add(S_TRANSPORT, "目标窗口", Level::Fail, e.to_string());
            r.aborted = true;
            return r;
        }
    }
    let mut ws = match tokio::time::timeout(std::time::Duration::from_secs(8), connect_page(cdp_port, want)).await {
        Ok(Ok(ws)) => {
            r.add(S_TRANSPORT, "CDP 会话", Level::Pass, "已连接");
            ws
        }
        other => {
            let why = match other {
                Ok(Err(e)) => e.to_string(),
                _ => "连接超时(8s)".into(),
            };
            r.add(S_TRANSPORT, "CDP 会话", Level::Fail, why);
            r.aborted = true;
            return r;
        }
    };
    let ws = &mut ws;
    let mut mid: u64 = 100;
    let mid = &mut mid;

    // 窗口可见性: 不可见时(被遮挡/最小化/显示器关)帧率被节流, 相机阻尼收敛不完 → 相机类回环只作参考(WARN)。
    let hidden = eval_roundtrip(ws, mid, "document.visibilityState").await.map(|s| s.trim() == "hidden").unwrap_or(false);
    if hidden {
        r.add(S_TRANSPORT, "窗口可见性", Level::Warn, "hidden — 相机 / 视口回环的偏差只作参考(阻尼在后台收敛很慢)");
    }
    // 不可见时相机类回环的 FAIL 降为 WARN
    let cam_fail = if hidden { Level::Warn } else { Level::Fail };

    // ================= 垫片 =================
    match eval_roundtrip(ws, mid, &shim::install_expr()).await {
        Ok(s) => {
            let v: Value = serde_json::from_str(&s).unwrap_or(Value::Null);
            let how = if v["same"] == true { "页面已有同版本垫片" } else { "已注入" };
            r.add(S_SHIM, "注入", Level::Pass, how);
        }
        Err(e) => r.add(S_SHIM, "注入", Level::Fail, format!("{e:#}")),
    }
    match call(ws, mid, api::CORE_HEALTH, &Value::Null).await {
        Ok(h) => {
            let api_ok = h["api"].as_u64() == Some(shim::API_VERSION as u64);
            let rev = h["rev"].as_str().unwrap_or("");
            let rev_ok = rev == shim::rev_str();
            r.add(S_SHIM, "接口版本", if api_ok { Level::Pass } else { Level::Fail },
                format!("页面 api {} / 本程序 api {}", h["api"], shim::API_VERSION));
            r.add(S_SHIM, "垫片 rev", if rev_ok { Level::Pass } else { Level::Warn },
                if rev_ok { rev.to_string() } else { format!("页面 {rev} ≠ 本程序 {}(另一个构建的助手注入的; 接口版本一致即可用)", shim::rev_str()) });
        }
        Err(e) => r.add(S_SHIM, "接口版本", Level::Fail, e),
    }
    match call(ws, mid, api::CORE_APIS, &Value::Null).await {
        Ok(v) => {
            let page: Vec<&str> = v.as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
            let missing: Vec<&str> = api::ALL.iter().copied().filter(|n| !page.contains(n)).collect();
            r.add(S_SHIM, "接口清单", if missing.is_empty() { Level::Pass } else { Level::Fail },
                if missing.is_empty() { format!("{} 个接口齐全", api::ALL.len()) } else { format!("页面缺 {missing:?}") });
        }
        Err(e) => r.add(S_SHIM, "接口清单", Level::Fail, e),
    }

    // ================= 能力 =================
    let caps = match call(ws, mid, api::CORE_PROBE, &Value::Null).await {
        Ok(p) => CapsReport::from_probe(&p),
        Err(e) => {
            r.add(S_CAPS, "能力探测", Level::Fail, e);
            CapsReport::default()
        }
    };
    r.add(S_CAPS, "编辑器", if caps.editor { Level::Pass } else { Level::Fail },
        if caps.editor { format!("已找到(经 {})", caps.finder.as_deref().unwrap_or("?")) } else { "未找到 — 后续读写全部不可用".into() });
    for (name, s) in &caps.caps {
        if name == shim::cap::EDITOR {
            continue;
        }
        let (level, detail) = if let Some(t) = &s.todo {
            (Level::Skip, format!("路线图预留({t})"))
        } else if !s.ok {
            (Level::Fail, format!("缺 {}", s.missing.join(", ")))
        } else if !s.optional_missing.is_empty() {
            (Level::Warn, format!("缺可选 {}", s.optional_missing.join(", ")))
        } else {
            (Level::Pass, String::new())
        };
        r.add(S_CAPS, name.clone(), level, detail);
    }
    for (k, p) in &caps.alt_paths {
        r.add(S_CAPS, format!("备选路径 {k}"), Level::Warn, format!("命中 {p}(Plasticity 改过名, 垫片已兜住)"));
    }
    if let Some(b) = &caps.bridge_server {
        if b.closed_while_enabled() {
            r.add(S_CAPS, "Bridge 服务端", Level::Warn,
                "设置开着但服务端已关闭: 端口被占用(另一个 Plasticity 窗口占着, 或上次没退干净); \
                 单窗口时到偏好设置 → Network 关掉再打开 Server");
        } else if b.state == Some(0) {
            r.add(S_CAPS, "Bridge 服务端", Level::Pass, format!("运行中, 端口 {}", b.port.unwrap_or(0)));
        }
    }

    // ================= 只读 =================
    let reads = [api::SEL_SNAPSHOT, api::HIST_STATE, api::CAM_GET, api::ERR_LAST, api::TREE_LIST];
    let calls: Vec<(&str, Value)> = reads.iter().map(|n| (*n, Value::Null)).collect();
    match eval_roundtrip(ws, mid, &shim::batch_expr(&calls)).await {
        Ok(raw) => {
            for (name, res) in shim::parse_batch(&raw, &reads) {
                let verdict: Result<String, String> = match res {
                    Err(e) => Err(e.to_string()),
                    Ok(v) => match name.as_str() {
                        api::SEL_SNAPSHOT => {
                            if v["size"].is_number() && v["items"].is_array() {
                                Ok(format!("选中 {} 项", v["size"]))
                            } else {
                                Err("结构不符".into())
                            }
                        }
                        api::HIST_STATE => has_numbers(&v, &["undo", "geoVer"])
                            .map(|_| format!("撤销栈 {} 步, 几何版本 {}", v["undo"], v["geoVer"])),
                        api::CAM_GET => has_numbers(&v, &["px", "py", "pz", "qx", "qy", "qz", "qw"])
                            .map(|_| if v["orbit"].is_object() { "相机 + 轨道控制器".into() } else { "相机(无轨道控制器)".into() }),
                        api::TREE_LIST => match v.as_array() {
                            Some(a) => Ok(format!("{} 个节点", a.len())),
                            None => Err("不是数组".into()),
                        },
                        _ => Ok(String::new()),
                    },
                };
                match verdict {
                    Ok(d) => r.add(S_READ, name, Level::Pass, d),
                    Err(d) => r.add(S_READ, name, Level::Fail, d),
                }
            }
        }
        Err(e) => r.add(S_READ, "批量读取", Level::Fail, format!("{e:#}")),
    }

    // ================= 回环(读出来原样写回, 不改文档) =================
    let before = fingerprint(ws, mid).await;
    let cam_before = call(ws, mid, api::CAM_GET, &Value::Null).await.ok().map(|v| v.to_string());
    match &cam_before {
        Some(cam) => {
            let view: Value = serde_json::from_str(cam).unwrap_or(Value::Null);
            match call(ws, mid, api::CAM_SET, &view).await {
                Ok(_) => match call(ws, mid, api::CAM_GET, &Value::Null).await {
                    Ok(now) => match view_delta(cam, &now.to_string()) {
                        Ok(d) if d < CAM_EPS => r.add(S_ROUNDTRIP, "cam.set ← cam.get", Level::Pass, format!("回读偏差 {d:.1e}")),
                        Ok(d) => r.add(S_ROUNDTRIP, "cam.set ← cam.get", cam_fail, format!("回读偏差 {d:.3e} — 写入未生效或被拉回")),
                        Err(e) => r.add(S_ROUNDTRIP, "cam.set ← cam.get", Level::Fail, e.to_string()),
                    },
                    Err(e) => r.add(S_ROUNDTRIP, "cam.set ← cam.get", Level::Fail, e),
                },
                Err(e) => r.add(S_ROUNDTRIP, "cam.set ← cam.get", Level::Fail, e),
            }
        }
        None => r.add(S_ROUNDTRIP, "cam.set ← cam.get", Level::Skip, "读不到相机"),
    }
    match call(ws, mid, api::CMD_PING, &Value::Null).await {
        Ok(v) if v["fired"] == true => r.add(S_ROUNDTRIP, "cmd.ping 注册→派发→回调", Level::Pass, ""),
        Ok(v) => r.add(S_ROUNDTRIP, "cmd.ping 注册→派发→回调", Level::Fail, format!("回调未执行({v})")),
        Err(e) => r.add(S_ROUNDTRIP, "cmd.ping 注册→派发→回调", Level::Fail, e),
    }
    // 视口状态: 存一份 → 原样恢复 → 再存一份, 两份应一致(着色模式相同时不会派发切换)。
    match call(ws, mid, api::VP_SAVE, &Value::Null).await {
        Ok(s0) => match call(ws, mid, api::VP_RESTORE, &s0).await {
            Ok(res) => {
                let failed = res["failed"].as_array().map(|a| a.len()).unwrap_or(0);
                let applied: Vec<&str> = res["applied"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
                // 恢复后轨道控制器被标记需要更新, 会按阻尼收敛一两帧 — 等它稳定再回读(后台窗口帧率被节流, 留足余量)。
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                match call(ws, mid, api::VP_SAVE, &Value::Null).await {
                    Ok(s1) => {
                        let (a, b) = (strip_volatile(&s0), strip_volatile(&s1));
                        // 容差 1e-4: 缩放 / 轨道半径是阻尼量, 窗口在后台帧率被节流时 1.5 s 内收敛不完(实测漂移 ~2e-5);
                        // 真正的恢复失败(视角被重置等)差距是数量级的, 照样抓得到。
                        let same = json_close(&a, &b, VP_EPS);
                        let level = if failed > 0 { Level::Fail } else if !same { cam_fail } else { Level::Pass };
                        r.add(S_ROUNDTRIP, "vp.restore ← vp.save", level, if same && failed == 0 {
                            format!("前后一致; 已应用 {}", applied.join(","))
                        } else {
                            let mut d = Vec::new();
                            json_diff_paths(&a, &b, VP_EPS, "", &mut d, 4);
                            format!("前后{}; 失败段 {}; 差异 {}", if same { "一致" } else { "不一致" }, res["failed"], d.join(" | "))
                        });
                    }
                    Err(e) => r.add(S_ROUNDTRIP, "vp.restore ← vp.save", Level::Fail, e),
                }
            }
            Err(e) => r.add(S_ROUNDTRIP, "vp.restore ← vp.save", Level::Fail, e),
        },
        Err(e) => r.add(S_ROUNDTRIP, "vp.restore ← vp.save", Level::Fail, e),
    }
    // R34 材质状态: 存一份 → 原样恢复 → 应识别为"没变化"(materials(same)), 不碰 memento、不改文档。
    match call(ws, mid, api::MAT_SAVE, &Value::Null).await {
        Ok(s0) => {
            let n = (s0["materials"].as_array().map(|a| a.len()).unwrap_or(0), s0["assign"].as_array().map(|a| a.len()).unwrap_or(0));
            match call(ws, mid, api::MAT_RESTORE, &s0).await {
                Ok(res) if res["applied"] == serde_json::json!(["materials(same)"]) => {
                    r.add(S_ROUNDTRIP, "mat.restore ← mat.save", Level::Pass, format!("材质 {} / 指派 {}; 原样识别为无变化", n.0, n.1))
                }
                Ok(res) => r.add(S_ROUNDTRIP, "mat.restore ← mat.save", Level::Fail, format!("原样恢复却被当成改动: {res}")),
                Err(e) => r.add(S_ROUNDTRIP, "mat.restore ← mat.save", Level::Fail, e),
            }
        }
        Err(e) => r.add(S_ROUNDTRIP, "mat.restore ← mat.save", Level::Fail, e),
    }
    // 选择回环: 把当前选择经 sel.apply 原样恢复(选择命令进撤销历史: 撤销栈 +1 属预期, 守护段扣除)。
    let mut selection_commands = 0i64;
    match call(ws, mid, api::SEL_SNAPSHOT, &Value::Null).await {
        Ok(s0) => {
            let items = selection_items(&s0);
            if items.is_empty() {
                r.add(S_ROUNDTRIP, "sel.apply ← sel.snapshot", Level::Skip, "当前没有选择(选中几个面后再跑可覆盖这一项)");
            } else {
                match call(ws, mid, api::SEL_APPLY, &json!({ "items": items, "replace": true })).await {
                    Ok(res) => {
                        selection_commands += 1;
                        let after = call(ws, mid, api::SEL_SNAPSHOT, &Value::Null).await.unwrap_or(Value::Null);
                        let same = selection_key(&s0) == selection_key(&after);
                        let unsupported = res["unsupported"].as_array().map(|a| a.len()).unwrap_or(0);
                        let level = if same { Level::Pass } else if unsupported > 0 { Level::Warn } else { Level::Fail };
                        r.add(S_ROUNDTRIP, "sel.apply ← sel.snapshot", level, format!(
                            "{} 项{}; 下钻 {} 次; 缺 {}; 不支持 {}",
                            items.len(), if same { "原样恢复" } else { "恢复后不一致" },
                            res["drills"], res["missing"], res["unsupported"]
                        ));
                    }
                    Err(e) => r.add(S_ROUNDTRIP, "sel.apply ← sel.snapshot", Level::Fail, e),
                }
            }
        }
        Err(e) => r.add(S_ROUNDTRIP, "sel.apply ← sel.snapshot", Level::Fail, e),
    }

    // ================= 守护: 文档绝不能被改动 =================
    let after = fingerprint(ws, mid).await;
    match (before, after) {
        (Some((u0, g0, s0)), Some((u1, g1, s1))) => {
            // 几何版本必须不变; 撤销栈只允许多出本次执行的选择命令(选择进撤销历史, 不改几何)。
            // 选择原样恢复时 Plasticity 可能不记撤销步(实测 2026-11-15: 21→21) → 允许 +0..+N。
            let doc_same = g0 == g1 && u1 >= u0 && u1 <= u0 + selection_commands;
            r.add(S_GUARD, "撤销栈 / 几何版本", if doc_same { Level::Pass } else { Level::Fail },
                if doc_same {
                    format!("几何未变(版本 {g0}); 撤销栈 {u0}→{u1}(含 {selection_commands} 个选择命令)")
                } else {
                    format!("变了: {u0}→{u1} 步(预期 +0..+{selection_commands}), 版本 {g0}→{g1} — 自检改动了文档!")
                });
            r.add(S_GUARD, "选择", if s0 == s1 { Level::Pass } else { Level::Fail },
                if s0 == s1 { "未变".to_string() } else { "选择被改变".to_string() });
        }
        _ => r.add(S_GUARD, "文档指纹", Level::Fail, "读不到撤销栈/选择, 无法确认文档未被改动"),
    }
    if let (Some(c0), Ok(c1)) = (cam_before, call(ws, mid, api::CAM_GET, &Value::Null).await) {
        match view_delta(&c0, &c1.to_string()) {
            Ok(d) if d < CAM_EPS => r.add(S_GUARD, "相机", Level::Pass, "与自检前一致"),
            Ok(d) => r.add(S_GUARD, "相机", Level::Warn, format!("与自检前偏差 {d:.3e}(自检期间转动过视角?)")),
            Err(e) => r.add(S_GUARD, "相机", Level::Warn, e.to_string()),
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_close_tolerates_float_jitter_only() {
        let a = json!({"cam": {"px": 1.0, "q": [0.0, 1.0]}, "mode": "Shaded", "vpRect": [0, 0, 1, 1]});
        let b = json!({"cam": {"px": 1.0 + 1e-9, "q": [0.0, 1.0]}, "mode": "Shaded", "vpRect": [5, 5, 9, 9]});
        assert!(json_close(&strip_volatile(&a), &strip_volatile(&b), 1e-6));
        let c = json!({"cam": {"px": 1.1, "q": [0.0, 1.0]}, "mode": "Shaded"});
        assert!(!json_close(&strip_volatile(&a), &c, 1e-6));
        let d = json!({"cam": {"px": 1.0, "q": [0.0, 1.0]}, "mode": "Matcap"});
        assert!(!json_close(&strip_volatile(&a), &d, 1e-6));
    }

    #[test]
    fn report_exit_codes_and_json() {
        let mut r = Report::default();
        r.add(S_CAPS, "a", Level::Pass, "");
        r.add(S_CAPS, "b", Level::Warn, "x");
        r.add(S_CAPS, "c", Level::Skip, "");
        assert_eq!(r.exit_code(), 0, "WARN/SKIP 不算失败");
        r.add(S_GUARD, "d", Level::Fail, "y");
        assert_eq!(r.exit_code(), 1);
        r.aborted = true;
        assert_eq!(r.exit_code(), 2);
        let j = r.to_json();
        assert_eq!(j["summary"]["fail"], 1);
        assert_eq!(j["checks"][1]["level"], "WARN");
    }
}
