//! CDP 会话（R7-14）：连 Plasticity renderer 的 DevTools 端口，实时读选择/实体树。
//!
//! 前置：Plasticity 以 PLA-CDP 补丁版启动（resources/app/.webpack/main/index.js
//! 注入补丁会打开 9333；见 docs/know-how/editor-cdp-facts.md）。
//! 架构与 bridge_thread.rs 同型：常驻线程 + current_thread runtime +
//! mpsc 命令/事件通道，UI 每帧非阻塞 poll。
//!
//! 读通道 = Runtime.evaluate 轮询（500ms）：
//!   * selection 快照：editor.selection.selected 各类型集合（体/片体/面/边/
//!     线段/曲线/曲线顶点/组/基准面）；
//!   * 实体树：BFS OutlinerSection vnodes（name/ctor/indent/selected）——
//!     包含 bridge 不下发的 curves 与 instances。
//! 写通道 = evaluate `Selection.addById`（应用选择集，v1 仅数值 id）。

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use serde_json::Value;

use crate::shim;

/// UI → CDP 线程命令。
#[derive(Debug)]
pub enum CdpCmd {
    /// 应用选择集: 按成员类型回放(solid/sheet=addById(数值); 其余=add<Kind>({id,versionId,...}))。
    ApplySel(Vec<ApplyEntry>),
    /// R8-1: 采集视口状态(相机/缩放/剖面/参考面/显示模式) → VpStateResult。
    VpStateSave,
    /// R8-1: 恢复视口状态(state = VpStateSave 采集的 JSON)。
    VpStateApply(String),
    /// R8-1: 视口裁剪截图(Page.captureScreenshot clip=视口 rect)。
    /// 参数 = 输出 PNG 路径; 结果经 Status 上报。
    CaptureViewport(String),
    /// R7-28: 派发一个按键(经 CDP Input.dispatchKeyEvent)。
    /// 注意: CDP 输入要求 Plasticity 窗口持有 OS 焦点, 否则事件被丢弃。
    Key(KeySpec),
    /// 立即拉一次实体树。
    RefreshTree,
    /// F2: 立即重新探测能力(结果无变化也上报, 供面板"重新探测")。
    ProbeCaps,
    /// F1b: 调一个垫片接口(名字来自 `shim::api`), 结果经 `CdpEvent::CallResult` 回报。
    /// 取代 R7-16 的通用 Eval(界面不再拼 JS)。
    Call { name: &'static str, args: Value },
    Shutdown,
}

/// 一次按键派发的参数(R7-28)。
#[derive(Debug, Clone)]
pub struct KeySpec {
    /// `KeyboardEvent.key`, 如 "d" / "Escape"。
    pub key: String,
    /// `KeyboardEvent.code`, 如 "KeyD" / "Escape"。
    pub code: String,
    /// Windows 虚拟键码(如 68 / 27)。
    pub vk: u32,
    /// 修饰键位掩码: 8=Shift, 2=Ctrl, 4=Alt。
    pub modifiers: u32,
}

/// 一条待回放的选择成员。
#[derive(Debug, Clone)]
pub struct ApplyEntry {
    pub kind: String,
    pub id: String,
    /// 保存时采集的原始 id 元素 JSON(对象形); None = 纯数值 id。
    pub key: Option<String>,
    /// v6(R7-31l): 保存时刻的相机 JSON({px,py,pz,qx,qy,qz,qw})。
    /// Some 时应用流程 = 还原保存视角 → 点击 → 恢复用户视角(按图索骥,
    /// 保存视角下目标面必然正对可见)。None = 旧选择集, 走当前视角扫描。
    pub camera: Option<String>,
}

/// CDP 线程 → UI 事件。
#[derive(Debug, Clone)]
pub enum CdpEvent {
    /// R25: 连接状态(结构化; 界面按变体显示文本与颜色, 不再字符串匹配)。
    Conn(ConnState),
    /// R25: 一次操作的结果(选择集应用、视口恢复、截图失败…); ok 决定界面用绿色还是红色。
    /// 以前和连接状态共用一个 Status(String), 结果文本把"已连接"冲掉、连接指示变红。
    Report { ok: bool, msg: String },
    /// 实时选择快照（st=ok 时有意义）。
    Selection(SelectionSnapshot),
    /// 实体树（OutlinerSection 平铺, 含 curves/instances）。
    Tree(Vec<TreeNode>),
    /// `CdpCmd::Call` 的回包: 接口名 + v 或错误说明。
    CallResult { name: &'static str, result: Result<Value, String> },
    /// R8-1: 视口状态采集回包(JSON 字符串; err 字段非空表示失败)。
    VpStateResult(String),
    /// R8-1: 视口截图回包(PNG 字节数)。
    ViewportPng(usize),
    /// R7-28: 请 UI 把前台焦点交还 Plasticity(武装按键丢失时重试前置)。
    RequestFocus,
    /// F2: 能力探测结果(变化时才发)。None = CDP 断开, 能力未知。
    Caps(Option<shim::CapsReport>),
}

/// R25: CDP 连接状态。
#[derive(Debug, Clone, PartialEq)]
pub enum ConnState {
    /// 启动后尚未有结果。
    Connecting,
    Connected,
    /// 连不上, 等待重试。refused = 9333 无人监听(Plasticity 未以 CDP 模式运行 / 代理没起)。
    Waiting { detail: String, refused: bool },
    /// 连接尝试超时(典型: Plasticity 刚启动, renderer 未就绪)。
    Timeout,
    /// 会话中途断开(原因)。
    Disconnected(String),
    /// CDP 线程起不来。
    Failed(String),
}

impl ConnState {
    pub fn is_connected(&self) -> bool {
        matches!(self, ConnState::Connected)
    }

    /// 当前界面语言下的文本。
    pub fn text(&self) -> String {
        use crate::i18n::tr;
        match self {
            ConnState::Connecting => tr("CDP 连接中…", "CDP connecting…").into(),
            ConnState::Connected => tr("CDP 已连接", "CDP connected").into(),
            ConnState::Waiting { detail, refused: true } => crate::t!(
                "CDP 等待中: 9333 无监听 — Plasticity 未以 CDP 模式运行, 或代理未启动({detail})",
                "CDP waiting: nothing on 9333 — Plasticity not in CDP mode, or proxy not running ({detail})"
            ),
            ConnState::Waiting { detail, .. } => crate::t!("CDP 等待中({detail})", "CDP waiting ({detail})"),
            ConnState::Timeout => tr("CDP 连接超时, 重试中…", "CDP connection timed out, retrying…").into(),
            ConnState::Disconnected(e) => crate::t!("CDP 断开: {e}", "CDP disconnected: {e}"),
            ConnState::Failed(e) => crate::t!("CDP 线程启动失败: {e}", "CDP thread failed to start: {e}"),
        }
    }
}

/// 一个选中条目。
/// key = id 元素的原始 JSON(对象形 id 如 face/edge 的 {id,versionId,...});
/// None = 纯数值 id(solid/sheet 的 kernel item id)。应用时原样回放。
#[derive(Debug, Clone)]
pub struct SelItem {
    /// 成员类型(solid/sheet/face/edge/…), 取值表唯一来源 = 垫片 `SEL_KINDS`。
    pub kind: String,
    pub id: String,
    /// 自身名字(实体/片体/曲线等物体; 面/边没有名字)。
    pub name: String,
    pub ctor: String,
    pub key: Option<String>,
    /// R24: 所属物体名(面/边; 大纲里的名字)。空 = 读不到。
    pub owner: String,
    /// R24: 曲面类型原名(面: Plane / Cylinder / Sphere / Torus / BlendSurf …)。空 = 非面或读不到。
    pub surface: String,
}

#[derive(Debug, Clone, Default)]
pub struct SelectionSnapshot {
    pub ok: bool,
    pub state: String,
    pub size: i64,
    pub items: Vec<SelItem>,
    /// undo 栈深度 = 已执行操作步数(自动快照"定步数"计数源)。
    pub undo_len: i64,
    /// 几何数据库版本号(编辑即增)。
    pub geo_ver: i64,
    /// R7-27: Plasticity 侧最近一次命令报错(原文 + 堆栈), 供 UI 显示。
    pub error: Option<(String, String)>,
    /// v6(R7-31l): 当前相机 JSON 字符串({px..qw}); UI 保存选择集时随集存储,
    /// 应用时还原(按图索骥)。None = 读取失败。
    pub camera: Option<String>,
}

/// 实体树节点（平铺, indent 表层级）。
#[derive(Debug, Clone)]
pub struct TreeNode {
    pub name: String,
    pub ctor: String,
    pub indent: i64,
    pub selected: bool,
}

/// CDP 线程句柄。
pub struct CdpHandle {
    tx: std::sync::mpsc::Sender<CdpCmd>,
    rx: Receiver<CdpEvent>,
    pub port: u16,
    /// R19: 要连接的 Plasticity 窗口 HWND(0 = 任意/第一个)。多窗口时每个助手只连自己那扇窗口。
    target: std::sync::Arc<std::sync::atomic::AtomicIsize>,
}

impl CdpHandle {
    /// 启动 CDP 常驻线程（连接失败不致命, 线程内自动重试）。
    pub fn spawn(port: u16) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<CdpCmd>();
        let (ev_tx, ev_rx) = std::sync::mpsc::channel::<CdpEvent>();
        let target = std::sync::Arc::new(std::sync::atomic::AtomicIsize::new(0));
        let t2 = target.clone();
        std::thread::Builder::new()
            .name("pla-cdp".into())
            .spawn(move || cdp_thread_main(port, t2, rx, ev_tx))
            .expect("spawn pla-cdp thread");
        Self { tx, rx: ev_rx, port, target }
    }

    /// R19: 设置要连接的窗口; 与当前连接的不同时, CDP 线程会断开并重连到新窗口。
    pub fn set_target(&self, hwnd: Option<isize>) {
        self.target.store(hwnd.unwrap_or(0), std::sync::atomic::Ordering::Relaxed);
    }

    pub fn send(&self, cmd: CdpCmd) {
        let _ = self.tx.send(cmd);
    }

    /// UI 每帧非阻塞收取事件。
    pub fn poll(&self) -> Option<CdpEvent> {
        match self.rx.try_recv() {
            Ok(ev) => Some(ev),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }
}

fn cdp_thread_main(
    port: u16,
    target: std::sync::Arc<std::sync::atomic::AtomicIsize>,
    cmd_rx: Receiver<CdpCmd>,
    ev_tx: std::sync::mpsc::Sender<CdpEvent>,
) {
    use std::sync::atomic::Ordering;
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            let _ = ev_tx.send(CdpEvent::Conn(ConnState::Failed(e.to_string())));
            return;
        }
    };
    rt.block_on(async move {
        loop {
            // 连接阶段: 反复重试(应用未启动/未打补丁时静默等待)。
            // R7-19: 整个连接尝试套 5s 超时 — Plasticity 刚启动几秒内 renderer
            // 未就绪时 /json/list 或 WS 握手可能不响应, http_get/connect_async
            // 本身无超时, 线程会永久卡死且一个事件都发不出(UI 永远停在
            // 「CDP 连接中…」)。超时丢弃本次尝试, 下轮重试即恢复。
            let want = target.load(Ordering::Relaxed);
            match tokio::time::timeout(Duration::from_secs(5), connect_page(port, want)).await {
                Ok(Ok(mut ws)) => {
                    let _ = ev_tx.send(CdpEvent::Conn(ConnState::Connected));
                    if let Err(e) = session_loop(&mut ws, &cmd_rx, &ev_tx, &target, want).await {
                        eprintln!("[pla-cdp] session err: {e:#}");
                        let _ = ev_tx.send(CdpEvent::Conn(ConnState::Disconnected(e.to_string())));
                    }
                    // F2: 断开后能力未知(面板回到"未连接", 不保留过期的能力表)。
                    let _ = ev_tx.send(CdpEvent::Caps(None));
                }
                Ok(Err(e)) => {
                    eprintln!("[pla-cdp] connect err: {e:#}");
                    // R7-19: 10061(拒连)=9333 无监听 — 补丁版 index.js 任意启动都会
                    // appendSwitch 9333, 拒连只可能是: Plasticity 未启动, 或运行中的
                    // 实例是补丁部署前启动的旧实例(index.js 是启动时加载的)。
                    // 给出可执行指引, 不再让用户对着「积极拒绝」猜。
                    let detail = format!("{e}");
                    let refused = detail.contains("10061");
                    let _ = ev_tx.send(CdpEvent::Conn(ConnState::Waiting { detail, refused }));
                }
                Err(_) => {
                    eprintln!("[pla-cdp] connect timeout(5s)");
                    // 5s 超时: 连接挂起(典型为 Plasticity 启动初期 renderer 未就绪),
                    // 丢弃重试 — 状态照常上报, UI 不会永远停在「连接中」。
                    let _ = ev_tx.send(CdpEvent::Conn(ConnState::Timeout));
                }
            }
            // 断开冷却 + 处理积压命令
            if let Ok(CdpCmd::Shutdown) = cmd_rx.try_recv() {
                return;
            }
            // R19: 绑定窗口刚变 → 立即连新窗口, 不等冷却。
            if target.load(Ordering::Relaxed) != want {
                continue;
            }
            tokio::time::sleep(Duration::from_millis(2000)).await;
        }
    });
}

pub(crate) type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

/// R19: 从 /json/list 里选目标。`want` = 绑定窗口 HWND(0 = 任意)。
/// 代理(R19 起)给每个 Plasticity 窗口列一个 page, 带 `hwnd` 字段; 旧代理没有该字段 → 退回第一个。
pub(crate) fn pick_page(list: &Value, want: isize) -> anyhow::Result<&Value> {
    let pages: Vec<&Value> = list
        .as_array()
        .map(|a| a.iter().filter(|t| t["type"] == "page").collect())
        .unwrap_or_default();
    let first = || {
        pages
            .iter()
            .copied()
            .find(|t| t["title"].as_str().unwrap_or("").contains("Plasticity"))
            .or_else(|| pages.first().copied())
            .ok_or_else(|| anyhow::anyhow!("no page target"))
    };
    if want == 0 || !pages.iter().any(|t| t.get("hwnd").is_some()) {
        return first();
    }
    let w = want.to_string();
    pages
        .iter()
        .copied()
        .find(|t| t["hwnd"].as_str() == Some(w.as_str()))
        .ok_or_else(|| anyhow::anyhow!("绑定的 Plasticity 窗口(HWND {want})不在 CDP 目标列表里"))
}

/// 连 page target(`want` = 绑定窗口 HWND, 0 = 任意)。
pub(crate) async fn connect_page(port: u16, want: isize) -> anyhow::Result<WsStream> {
    let body = http_get(port, "/json/list").await?;
    let v: Value = serde_json::from_str(&body)?;
    let page = pick_page(&v, want)?;
    let ws_url = page["webSocketDebuggerUrl"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no ws url"))?
        .to_string();
    // R7-19: Chromium 按 Host 头生成 ws url — Host 缺端口时返回
    // ws://127.0.0.1/...(连 80 被拒)。Host 已带端口, 这里再兜底补一次。
    let ws_url = match ws_url.strip_prefix("ws://127.0.0.1/") {
        Some(rest) => format!("ws://127.0.0.1:{port}/{rest}"),
        None => ws_url,
    };
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await?;
    // 握手: Runtime.enable(垫片由 session_loop 注入并读回结果)
    ws_send(&mut ws, serde_json::json!({"id":1,"method":"Runtime.enable"})).await?;
    Ok(ws)
}

async fn session_loop(
    ws: &mut WsStream,
    cmd_rx: &Receiver<CdpCmd>,
    ev_tx: &std::sync::mpsc::Sender<CdpEvent>,
    target: &std::sync::atomic::AtomicIsize,
    connected_to: isize,
) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    let mut mid: u64 = 10;
    let mut last_snapshot = String::new();
    let mut last_tree = String::new();
    let mut sel_tick: u32 = 0;

    // F1: 注入垫片(错误捕获钩子由垫片按 editor 代际自动安装); F2: 上报能力。
    let mut last_caps: Option<shim::CapsReport> = None;
    let mut last_probe = String::new();
    let caps = install_shim(ws, &mut mid).await;
    publish_caps(ev_tx, &mut last_caps, caps);

    loop {
        // R19: 助手换绑了窗口 → 断开, 由外层立即重连到新窗口。
        if target.load(std::sync::atomic::Ordering::Relaxed) != connected_to {
            return Err(anyhow::anyhow!("绑定窗口已变更, 重新连接"));
        }
        // --- UI 命令(非阻塞) ---
        match cmd_rx.try_recv() {
            Ok(CdpCmd::Shutdown) => return Err(anyhow::anyhow!("shutdown")),
            Ok(CdpCmd::Call { name, args }) => {
                let result = shim_call(ws, &mut mid, name, args).await.map_err(|e| e.to_string());
                let _ = ev_tx.send(CdpEvent::CallResult { name, result });
            }
            Ok(CdpCmd::ProbeCaps) => {
                last_caps = None; // 强制上报
                let caps = probe_caps(ws, &mut mid).await;
                publish_caps(ev_tx, &mut last_caps, caps);
            }
            Ok(CdpCmd::RefreshTree) => {
                let tree_raw = eval_roundtrip(ws, &mut mid, &shim::call_expr(shim::api::TREE_LIST, &Value::Null)).await?;
                if tree_raw != last_tree {
                    last_tree = tree_raw.clone();
                    let nodes: Vec<Value> = match shim::parse_reply(&tree_raw) {
                        Ok(Value::Array(a)) => a,
                        Ok(_) => Vec::new(),
                        Err(e) => {
                            eprintln!("[pla-cdp] 实体树不可用: {e}");
                            Vec::new()
                        }
                    };
                    let _ = ev_tx.send(CdpEvent::Tree(
                        nodes
                            .into_iter()
                            .map(|n| TreeNode {
                                name: n["name"].as_str().unwrap_or("").to_string(),
                                ctor: n["ctor"].as_str().unwrap_or("").to_string(),
                                indent: n["indent"].as_i64().unwrap_or(0),
                                selected: n["selected"].as_bool().unwrap_or(false),
                            })
                            .collect(),
                    ));
                }
            }
            Ok(CdpCmd::ApplySel(entries)) => {
                // R22: 按数据恢复选择 — 垫片 sel.apply 按 id 查对象, 在一个选择命令里逐项走
                // changeSelection.onClick(与手点同一条链路, 一个撤销步), 不碰屏幕。
                // 取代 R7-R21 的悬停扫网格 + 真实点击 + 遮挡补救 + 下钻补点(漏面、受遮挡/视角/后台节流影响)。
                // 见 docs/know-how/selection-by-command.md。
                if entries.is_empty() {
                    let _ = ev_tx.send(CdpEvent::Report { ok: false, msg: crate::i18n::tr("选择集应用: 成员为空", "Apply selection set: no members").into() });
                    continue;
                }
                let items: Vec<Value> = entries
                    .iter()
                    .map(|e| serde_json::json!({ "kind": e.kind, "id": e.id }))
                    .collect();
                let args = serde_json::json!({ "items": items, "replace": true });
                let (ok, msg) = match shim_call(ws, &mut mid, shim::api::SEL_APPLY, args).await {
                    Ok(v) => sel_apply_message(entries.len(), &v),
                    Err(e) => (false, crate::t!("选择集应用失败: {e}", "Apply selection set failed: {e}")),
                };
                let _ = ev_tx.send(CdpEvent::Report { ok, msg });
            }
            // ---- R8-1: 视口状态采集/恢复 + 视口裁剪截图 ----
            Ok(CdpCmd::VpStateSave) => {
                // 回包格式与 R8-1 存档一致(err 字段非空 = 失败)。
                let s = match shim_call(ws, &mut mid, shim::api::VP_SAVE, Value::Null).await {
                    Ok(v) => v.to_string(),
                    Err(e) => serde_json::json!({ "err": e.to_string() }).to_string(),
                };
                let _ = ev_tx.send(CdpEvent::VpStateResult(s));
            }
            Ok(CdpCmd::VpStateApply(state)) => {
                let (ok, msg) = match serde_json::from_str::<Value>(&state) {
                    Err(e) => (false, crate::t!("视口状态恢复失败: 存档无法解析({e})", "Restore viewport state failed: unreadable save ({e})")),
                    Ok(s) => match shim_call(ws, &mut mid, shim::api::VP_RESTORE, s).await {
                        Ok(r) => fmt_vpstate_apply_result(&r.to_string()),
                        Err(e) => (false, crate::t!("视口状态恢复失败: {e}", "Restore viewport state failed: {e}")),
                    },
                };
                let _ = ev_tx.send(CdpEvent::Report { ok, msg });
            }
            Ok(CdpCmd::CaptureViewport(out_path)) => {
                // R33d: 优先用垫片在页面里渲染并读 canvas(Page.captureScreenshot 会让整个窗口闪一下黑, 用户实测);
                // 拿不到再退回下面的 CDP 截图。
                if let Ok(v) = shim_call(ws, &mut mid, shim::api::CAM_THUMBNAIL, serde_json::json!({ "maxWidth": 960 })).await {
                    if let Some(b64) = v["data"].as_str() {
                        if let Ok(bytes) = base64_decode(b64) {
                            if !bytes.is_empty() && std::fs::write(&out_path, &bytes).is_ok() {
                                let _ = ev_tx.send(CdpEvent::ViewportPng(bytes.len()));
                                continue;
                            }
                        }
                    }
                }
                // 视口 DOM rect(CSS px) → clip(逻辑 px); captureScreenshot
                // 的 clip 以 CSS 像素为单位(与 viewport 缩放无关, 按 dpr 放大输出)。
                let clip: Value = match shim_call(ws, &mut mid, shim::api::CAM_VP_RECT, Value::Null).await {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = ev_tx.send(CdpEvent::Report { ok: false, msg: crate::t!("视口截图失败: 读不到视口区域({e}), 仍将保存, 无缩略图", "Viewport screenshot failed: cannot read viewport area ({e}); saving without thumbnail") });
                        let _ = ev_tx.send(CdpEvent::ViewportPng(0));
                        continue;
                    }
                };
                // 先强制渲染一帧(按需渲染模式下 canvas 可能是旧帧)
                shim_fire(ws, &mut mid, shim::api::CAM_RENDER, Value::Null).await;
                tokio::time::sleep(Duration::from_millis(120)).await;
                let shot = serde_json::json!({
                    "id": mid,
                    "method": "Page.captureScreenshot",
                    "params": {
                        "format": "png",
                        "clip": clip,
                        "captureBeyondViewport": false
                    }
                });
                mid += 1;
                if let Err(e) = ws_send(ws, shot).await {
                    let _ = ev_tx.send(CdpEvent::Report { ok: false, msg: crate::t!("视口截图失败: {e}", "Viewport screenshot failed: {e}") });
                    continue;
                }
                // 等回包(带 data 字段的 base64)
                let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                let mut png_len = 0usize;
                let shot_id = mid - 1;
                'shot: loop {
                    let msg = match tokio::time::timeout_at(deadline, ws.next()).await {
                        Ok(Some(Ok(m))) => m,
                        _ => break 'shot,
                    };
                    if let tokio_tungstenite::tungstenite::Message::Text(t) = msg {
                        if let Ok(v) = serde_json::from_str::<Value>(&t) {
                            // 错误回包(同 id)立即终止, 不空等超时
                            if v.get("id").and_then(|i| i.as_u64()) == Some(shot_id)
                                && v.get("error").is_some()
                            {
                                eprintln!(
                                    "[pla-cdp] captureScreenshot 错误: {}",
                                    v["error"]["message"].as_str().unwrap_or("?")
                                );
                                break 'shot;
                            }
                            if v.get("id").is_some() && v.get("result").map(|r| r.get("data").is_some()).unwrap_or(false) {
                                if let Some(b64) = v["result"]["data"].as_str() {
                                    match base64_decode(b64) {
                                        Ok(bytes) => {
                                            png_len = bytes.len();
                                            if let Err(e) = std::fs::write(&out_path, bytes) {
                                                let _ = ev_tx.send(CdpEvent::Report { ok: false, msg: crate::t!("视口截图写盘失败: {e}", "Writing viewport screenshot failed: {e}") });
                                                continue;
                                            }
                                        }
                                        Err(e) => {
                                            let _ = ev_tx.send(CdpEvent::Report { ok: false, msg: crate::t!("视口截图解码失败: {e}", "Decoding viewport screenshot failed: {e}") });
                                            continue;
                                        }
                                    }
                                }
                                break 'shot;
                            }
                        }
                    }
                }
                if png_len > 0 {
                    let _ = ev_tx.send(CdpEvent::ViewportPng(png_len));
                } else {
                    let _ = ev_tx.send(CdpEvent::Report { ok: false, msg: crate::i18n::tr("视口截图失败: 未收到截图数据(仍将保存, 无缩略图)", "Viewport screenshot failed: no image data received (saving without thumbnail)").into() });
                    // R8b: 失败也发 ViewportPng(0) — UI 的 pending 保存流程
                    // (选择集/视口状态)靠该事件落盘, 不发会永远挂起无法保存。
                    let _ = ev_tx.send(CdpEvent::ViewportPng(0));
                }
            }
            // (R7-28「重新武装」ApplySelRearm 已于 R7-32 移除 — 方案被用户否决;
            //  v4+ 真实点击管线自身建立活跃实体上下文, 无需武装。)
            Ok(CdpCmd::Key(spec)) => {
                if let Err(e) = dispatch_key(ws, &mut mid, &spec).await {
                    let _ = ev_tx.send(CdpEvent::Report { ok: false, msg: crate::t!("按键派发失败: {e}", "Key dispatch failed: {e}") });
                }
            }
            Err(TryRecvError::Disconnected) => return Err(anyhow::anyhow!("ui gone")),
            Err(_) => {}
        }

        // --- 轮询选择快照(每 2 个 tick ≈ 1s; 500ms 底噪) ---
        sel_tick += 1;
        if sel_tick % 2 == 0 {
            // R7-25: renderer reload(切文档/崩溃恢复)会清掉页面上的垫片。
            // 探活: stale = 垫片缺失或接口版本不符 → 重注; noed = 垫片在但 editor 丢了
            // (重新定位 editor 是 F4 的事, 这里如实上报 no-editor)。
            // F2: 重注后、ok↔noed 变化时、以及每 ~30s(视口可能后建)重新探测能力。
            match eval_roundtrip(ws, &mut mid, &shim::probe_expr()).await {
                Ok(p) if p.trim() == "stale" => {
                    eprintln!("[pla-cdp] 垫片缺失(页面已重载?), 重新注入");
                    let caps = install_shim(ws, &mut mid).await;
                    publish_caps(ev_tx, &mut last_caps, caps);
                    last_probe = p.trim().to_string();
                }
                Ok(p) => {
                    let p = p.trim().to_string();
                    if p != last_probe || sel_tick % 60 == 0 {
                        let caps = probe_caps(ws, &mut mid).await;
                        publish_caps(ev_tx, &mut last_caps, caps);
                    }
                    last_probe = p;
                }
                Err(e) => eprintln!("[pla-cdp] 垫片探活失败: {e:#}"),
            }
            // (Alt+←/→ 滚转命令的注册/补注册由垫片常驻维护, 随每次调用节流执行。)
            let raw = eval_roundtrip(ws, &mut mid, &shim::batch_expr(&poll_calls())).await?;
            if raw != last_snapshot {
                last_snapshot = raw.clone();
                let _ = ev_tx.send(CdpEvent::Selection(selection_from_batch(&raw)));
            }
        }

        // --- 500ms 心跳 + ws 事件抽取 ---
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            msg = ws.next() => {
                match msg {
                    Some(Ok(_)) => {} // 事件帧忽略
                    Some(Err(e)) => return Err(anyhow::anyhow!("ws: {e}")),
                    None => return Err(anyhow::anyhow!("ws closed")),
                }
            }
        }
    }
}

fn json_eval(mid: u64, expr: &str) -> Value {
    serde_json::json!({
        "id": mid,
        "method": "Runtime.evaluate",
        "params": {"expression": expr, "returnByValue": true, "awaitPromise": true, "userGesture": true}
    })
}

/// F1: 注入垫片, 随后做一次能力探测。失败不致命: 下轮探活会重试。
async fn install_shim(ws: &mut WsStream, mid: &mut u64) -> Option<shim::CapsReport> {
    match eval_roundtrip(ws, mid, &shim::install_expr()).await {
        Ok(r) => eprintln!("[pla-cdp] 垫片注入: {r}"),
        Err(e) => {
            eprintln!("[pla-cdp] 垫片注入失败: {e:#}");
            return None;
        }
    }
    probe_caps(ws, mid).await
}

/// 调一个垫片接口, 返回其 v(失败带能力 / 异常说明)。
pub(crate) async fn shim_call(ws: &mut WsStream, mid: &mut u64, name: &str, args: Value) -> anyhow::Result<Value> {
    let raw = eval_roundtrip(ws, mid, &shim::call_expr(name, &args)).await?;
    Ok(shim::parse_reply(&raw)?)
}

/// `sel.apply` 回包 → 状态栏文字(如实列出没选上 / 不支持的成员)。
/// 返回 (是否完全成功, 文本)。
fn sel_apply_message(total: usize, v: &Value) -> (bool, String) {
    let list = |k: &str| -> Vec<String> {
        v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
    };
    let missing = list("missing");
    let unsupported = list("unsupported");
    let extra = list("extra");
    let ok = total.saturating_sub(missing.len() + unsupported.len());
    if missing.is_empty() && unsupported.is_empty() && extra.is_empty() {
        return (true, crate::t!("选择集已应用: {ok} 项全部选中", "Selection set applied: all {ok} items selected"));
    }
    let mut parts = Vec::new();
    if !missing.is_empty() {
        let head: Vec<&str> = missing.iter().take(3).map(String::as_str).collect();
        let more = if missing.len() > 3 { "…" } else { "" };
        let (n, h) = (missing.len(), head.join(", "));
        parts.push(crate::t!("{n} 项找不到或没选上({h}{more})", "{n} not found or not selected ({h}{more})"));
    }
    if !unsupported.is_empty() {
        let n = unsupported.len();
        parts.push(crate::t!("{n} 项类型暂不支持", "{n} of an unsupported type"));
    }
    if !extra.is_empty() {
        let n = extra.len();
        parts.push(crate::t!("多选了 {n} 项", "{n} extra selected"));
    }
    let detail = parts.join("; ");
    (false, crate::t!("选择集应用: {ok}/{total} 项选中; {detail}", "Selection set: {ok}/{total} selected; {detail}"))
}

/// 不关心结果的垫片调用(收尾/唤醒类): 失败只记日志。
async fn shim_fire(ws: &mut WsStream, mid: &mut u64, name: &str, args: Value) {
    if let Err(e) = shim_call(ws, mid, name, args).await {
        eprintln!("[pla-cdp] {name} 失败: {e:#}");
    }
}

/// F2: 能力探测(只读)。
async fn probe_caps(ws: &mut WsStream, mid: &mut u64) -> Option<shim::CapsReport> {
    let raw = match eval_roundtrip(ws, mid, &shim::call_expr(shim::api::CORE_PROBE, &Value::Null)).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[pla-cdp] 能力探测失败: {e:#}");
            return None;
        }
    };
    match shim::parse_reply(&raw) {
        Ok(p) => Some(shim::CapsReport::from_probe(&p)),
        Err(e) => {
            eprintln!("[pla-cdp] 能力探测失败: {e}");
            None
        }
    }
}

/// F2: 探测结果有变化才上报 UI(并打日志: 哪项能力失效、走了哪些备选路径)。
fn publish_caps(
    ev_tx: &std::sync::mpsc::Sender<CdpEvent>,
    last: &mut Option<shim::CapsReport>,
    new: Option<shim::CapsReport>,
) {
    let Some(new) = new else { return };
    if last.as_ref() == Some(&new) {
        return;
    }
    let bad: Vec<String> = new
        .caps
        .iter()
        .filter(|(_, s)| !s.ok && s.todo.is_none())
        .map(|(k, s)| format!("{k}{:?}", s.missing))
        .collect();
    eprintln!(
        "[pla-cdp] 能力探测: editor={} finder={:?} 失效={bad:?} 备选路径={:?}",
        new.editor, new.finder, new.alt_paths
    );
    *last = Some(new.clone());
    let _ = ev_tx.send(CdpEvent::Caps(Some(new)));
}

/// 选择轮询一次取回的接口(一次 evaluate, 每项独立成败)。
fn poll_calls() -> [(&'static str, Value); 4] {
    [
        (shim::api::SEL_SNAPSHOT, Value::Null),
        (shim::api::HIST_STATE, Value::Null),
        (shim::api::CAM_GET, Value::Null),
        (shim::api::ERR_LAST, Value::Null),
    ]
}

/// 把轮询批量回包转成 UI 用的快照。选择读取失败 = 整体不可用(state 说明原因);
/// 撤销栈/相机/报错各自失败只置空对应字段(能力降级, 不拖垮选择)。
fn selection_from_batch(raw: &str) -> SelectionSnapshot {
    let names: Vec<&str> = poll_calls().iter().map(|(n, _)| *n).collect();
    let mut res: std::collections::HashMap<String, Result<Value, shim::ShimErr>> =
        shim::parse_batch(raw, &names).into_iter().collect();
    let mut take = |n: &str| res.remove(n).unwrap_or(Err(shim::ShimErr::Bad(n.into())));
    let sel = take(shim::api::SEL_SNAPSHOT);
    let hist = take(shim::api::HIST_STATE).ok();
    let cam = take(shim::api::CAM_GET).ok();
    let err = take(shim::api::ERR_LAST).ok();
    let sel = match sel {
        Ok(v) => v,
        Err(e) => {
            return SelectionSnapshot {
                ok: false,
                state: match e {
                    shim::ShimErr::NoEditor => "no-editor".into(),
                    shim::ShimErr::NoShim => "no-shim".into(),
                    other => other.to_string(),
                },
                size: -1,
                undo_len: -1,
                geo_ver: -1,
                ..Default::default()
            }
        }
    };
    // R7-21: 元素分型 — 纯数值(solid/sheet kernel id)→key=None;
    // 对象形(face/edge {id,versionId,...})→key=原始 JSON 原样保存/回放。
    let items = sel["items"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .map(|it| {
            let kind = it["kind"].as_str().unwrap_or("").to_string();
            let (id, key) = match &it["e"] {
                Value::String(s) => (s.clone(), None),
                Value::Number(n) => (n.to_string(), None),
                Value::Object(o) => {
                    let id = o
                        .get("id")
                        .map(|x| match x {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .or_else(|| o.get("__str").and_then(|s| s.as_str()).map(String::from))
                        .unwrap_or_default();
                    (id, Some(Value::Object(o.clone()).to_string()))
                }
                _ => (String::new(), None),
            };
            let info = &it["info"];
            let s = |k: &str| info[k].as_str().unwrap_or("").to_string();
            SelItem { ctor: kind.clone(), kind, id, name: s("name"), key, owner: s("ownerName"), surface: s("surface") }
        })
        .collect();
    SelectionSnapshot {
        ok: true,
        state: "ok".into(),
        size: sel["size"].as_i64().unwrap_or(-1),
        items,
        undo_len: hist.as_ref().and_then(|h| h["undo"].as_i64()).unwrap_or(-1),
        geo_ver: hist.as_ref().and_then(|h| h["geoVer"].as_i64()).unwrap_or(-1),
        error: err.as_ref().and_then(|e| {
            e["msg"].as_str().map(|m| (m.to_string(), e["stack"].as_str().unwrap_or("").to_string()))
        }),
        camera: cam.filter(|c| c.is_object()).map(|c| c.to_string()),
    }
}



/// R10: 把 VPSTATE_RESTORE_JS_TMPL 的回包翻成人话。**不再谎报成功**: 只要
/// failed 非空就明确说"部分失败", missing(存档里本来就没有)单独列出。
///
/// 回包形状: `{"applied":[..],"missing":[..],"failed":[..]}`;
/// 老格式(`ok:cam,orbit`)或任何解析失败都原样透出, 保证不丢信息。
///
/// @param raw CDP 回包字符串
/// @return 给面板显示的中文摘要
/// 返回 (是否完全成功, 文本)。
fn fmt_vpstate_apply_result(raw: &str) -> (bool, String) {
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return (true, crate::t!("视口状态已恢复: {raw}", "Viewport state restored: {raw}"));
    };
    /// 把 JSON 数组转成可读列表。
    fn list(v: &Value, key: &str) -> Vec<String> {
        v.get(key)
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|x| x.as_str().unwrap_or_default().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
    let applied = list(&v, "applied");
    let missing = list(&v, "missing");
    let failed = list(&v, "failed");

    let nothing = applied.is_empty() && failed.is_empty();
    let mut msg = if nothing {
        crate::i18n::tr("视口状态恢复: 没有任何项被应用", "Restore viewport state: nothing was applied").to_string()
    } else {
        let a = applied.join("/");
        crate::t!("视口状态已恢复: {a}", "Viewport state restored: {a}")
    };
    if !failed.is_empty() {
        let f = failed.join(", ");
        msg.push_str(&crate::t!(" | 部分失败: {f}", " | partly failed: {f}"));
    }
    if !missing.is_empty() {
        let m = missing.join(", ");
        msg.push_str(&crate::t!(" | 存档未含: {m}", " | not in save: {m}"));
    }
    (!nothing && failed.is_empty(), msg)
}

/// R10: 比较两个 CAM_SNAPSHOT_JS 回包的差异(相对位移 + 四元数夹角, 无量纲)。
///
/// 用途: 「切到保存视角」之后回读一次相机, 确认真的切过去了。相机在 Plasticity
/// 里由 orbitControls 驱动, 只写 camera.position/quaternion 会在下一帧被
/// orbit.update() 拉回原位 —— 那种情况下扫描会在**错误的视角**里进行, 表现
/// 只是「未扫到」, 真实故障被掩盖。这里给调用方一个可判定的是/否。
///
/// @param target_json 期望视角(CAM_SNAPSHOT_JS 输出)
/// @param actual_json 实际回读(CAM_SNAPSHOT_JS 输出)
/// @return 差值: 相对位移 + 2*acos(|dot(q,q')|); 解析失败时返回 Err(不撒谎说 0)。
pub(crate) fn view_delta(target_json: &str, actual_json: &str) -> anyhow::Result<f64> {
    /// 取一个数字字段; 缺失或类型不对时**报错**, 不静默当 0。
    fn num(value: &Value, key: &str) -> anyhow::Result<f64> {
        value
            .get(key)
            .and_then(|v| v.as_f64())
            .ok_or_else(|| anyhow::anyhow!("视角回包缺少字段 {key}"))
    }
    let a: Value = serde_json::from_str(target_json)?;
    let b: Value = serde_json::from_str(actual_json)?;
    let dx = num(&a, "px")? - num(&b, "px")?;
    let dy = num(&a, "py")? - num(&b, "py")?;
    let dz = num(&a, "pz")? - num(&b, "pz")?;
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    let scale = ((num(&a, "px")?).powi(2) + (num(&a, "py")?).powi(2) + (num(&a, "pz")?).powi(2))
        .sqrt()
        .max(1.0);
    let dot = (num(&a, "qx")? * num(&b, "qx")?
        + num(&a, "qy")? * num(&b, "qy")?
        + num(&a, "qz")? * num(&b, "qz")?
        + num(&a, "qw")? * num(&b, "qw")?)
    .abs()
    .min(1.0);
    Ok(dist / scale + 2.0 * dot.acos())
}

async fn ws_send(ws: &mut WsStream, v: Value) -> anyhow::Result<()> {
    use futures_util::SinkExt;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(v.to_string()))
        .await
        .map_err(|e| anyhow::anyhow!("ws send: {e}"))
}

/// R8-1: 标准 base64 解码(captureScreenshot 的 data 字段)。
/// 非法字符跳过(容错), 输出字节流。
fn base64_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    let mut lookup = [255u8; 256];
    for (i, c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        .iter()
        .enumerate()
    {
        lookup[*c as usize] = i as u8;
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &b in s.as_bytes() {
        if b == b'=' {
            break;
        }
        let v = lookup[b as usize];
        if v == 255 {
            continue; // 空白/非法字符跳过
        }
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}

/// R7-28: 经 CDP Input 派发一次完整按键(修饰键先下后起)。
/// **前置条件**: Plasticity 窗口必须持有 OS 焦点, 否则 Chromium 丢弃该事件
/// (实测: 前台是别的窗口时, 按键完全无声; 助手在应用完成后本就会把焦点交还)。
async fn dispatch_key(ws: &mut WsStream, mid: &mut u64, spec: &KeySpec) -> anyhow::Result<()> {
    let m = *mid;
    *mid += 1;
    // 修饰键先按下(Shift 需要 rawKeyDown, 否则 text 为空会被应用忽略)
    if spec.modifiers & 8 != 0 {
        ws_send(
            ws,
            serde_json::json!({"id": m, "method": "Input.dispatchKeyEvent", "params": {
                "type": "rawKeyDown", "key": "Shift", "code": "ShiftLeft",
                "windowsVirtualKeyCode": 16, "nativeVirtualKeyCode": 16, "modifiers": 8
            }}),
        )
        .await?;
    }
    let mut params = serde_json::json!({
        "type": "keyDown",
        "key": spec.key, "code": spec.code,
        "text": spec.key, "unmodifiedText": spec.key,
        "modifiers": spec.modifiers,
        "windowsVirtualKeyCode": spec.vk, "nativeVirtualKeyCode": spec.vk,
    });
    if let Some(o) = params.as_object_mut() {
        o.remove("text");
        if spec.key.chars().count() == 1 {
            o.insert("text".into(), serde_json::json!(spec.key));
        }
    }
    ws_send(
        ws,
        serde_json::json!({"id": m, "method": "Input.dispatchKeyEvent", "params": params}),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(80)).await;
    ws_send(
        ws,
        serde_json::json!({"id": m, "method": "Input.dispatchKeyEvent", "params": {
            "type": "keyUp",
            "key": spec.key, "code": spec.code,
            "modifiers": spec.modifiers,
            "windowsVirtualKeyCode": spec.vk, "nativeVirtualKeyCode": spec.vk,
        }}),
    )
    .await?;
    if spec.modifiers & 8 != 0 {
        tokio::time::sleep(Duration::from_millis(30)).await;
        ws_send(
            ws,
            serde_json::json!({"id": m, "method": "Input.dispatchKeyEvent", "params": {
                "type": "keyUp", "key": "Shift", "code": "ShiftLeft",
                "windowsVirtualKeyCode": 16, "nativeVirtualKeyCode": 16, "modifiers": 0
            }}),
        )
        .await?;
    }
    Ok(())
}

/// 发 evaluate 并收对应 id 的回包(跳过事件帧)。
pub(crate) async fn eval_roundtrip(
    ws: &mut WsStream,
    mid: &mut u64,
    expr: &str,
) -> anyhow::Result<String> {
    use futures_util::StreamExt;
    *mid += 1;
    let want = *mid;
    ws_send(ws, json_eval(want, expr)).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let msg = tokio::time::timeout_at(deadline, ws.next())
            .await
            .map_err(|_| anyhow::anyhow!("eval timeout"))?
            .ok_or_else(|| anyhow::anyhow!("ws closed"))?
            .map_err(|e| anyhow::anyhow!("ws read: {e}"))?;
        if let tokio_tungstenite::tungstenite::Message::Text(t) = msg {
            let v: Value = serde_json::from_str(&t)?;
            if v["id"].as_u64() == Some(want) {
                if let Some(exc) = v["result"]["exceptionDetails"]["exception"]["description"]
                    .as_str()
                {
                    return Err(anyhow::anyhow!("js: {exc}"));
                }
                return Ok(v["result"]["result"]["value"]
                    .as_str()
                    .unwrap_or("")
                    .to_string());
            }
        }
    }
}

pub(crate) async fn http_get(port: u16, path: &str) -> anyhow::Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
    s.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    )
    .await?;
    // R7-19: 不能 read_to_end(等 EOF) — 沙箱代理/启动初期的 Chromium 都可能
    // 不关连接, 读会永久挂起。改为按 header 分帧: 读到 \r\n\r\n 解析
    // Content-Length, 收满即停; EOF 作兜底。
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut tmp = [0u8; 8192];
    let body: Vec<u8> = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_ascii_lowercase();
            let clen: Option<usize> = head.lines().find_map(|l| {
                l.strip_prefix("content-length:")
                    .map(|v| v.trim().parse::<usize>().ok())?
            });
            let head_end = pos + 4;
            if let Some(n) = clen {
                while buf.len() < head_end + n {
                    let got = s.read(&mut tmp).await?;
                    if got == 0 { break; }
                    buf.extend_from_slice(&tmp[..got]);
                }
                break buf[head_end..(head_end + n).min(buf.len())].to_vec();
            }
            // 无 Content-Length: 退回读到 EOF(有外层 5s 超时兜底)
            loop {
                let got = s.read(&mut tmp).await?;
                if got == 0 { break; }
                buf.extend_from_slice(&tmp[..got]);
            }
            break buf[head_end..].to_vec();
        }
        let got = s.read(&mut tmp).await?;
        if got == 0 {
            // EOF 且没找到 header 结束 — 异常响应
            anyhow::bail!("http: EOF before headers, got {} bytes", buf.len());
        }
        buf.extend_from_slice(&tmp[..got]);
    };
    Ok(String::from_utf8_lossy(&body).to_string())
}

/// 一次性 CDP evaluate(R7-17, CLI 调试用): 连 page → Runtime.evaluate → 回包。
pub async fn cdp_eval_once(port: u16, expr: &str) -> anyhow::Result<String> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let body = http_get(port, "/json/list").await?;
    let v: Value = serde_json::from_str(&body)?;
    let page = v
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|t| t["type"] == "page" && t["title"].as_str().unwrap_or("").contains("Plasticity"))
        })
        .or_else(|| v.as_array().and_then(|a| a.iter().find(|t| t["type"] == "page")))
        .ok_or_else(|| anyhow::anyhow!("no page target"))?;
    let ws_url = page["webSocketDebuggerUrl"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no ws url"))?
        .to_string();
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await?;
    ws.send(Message::Text(
        serde_json::json!({"id":1,"method":"Runtime.enable"}).to_string(),
    ))
    .await?;
    ws.send(Message::Text(json_eval(2, expr).to_string())).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let msg = tokio::time::timeout_at(deadline, ws.next())
            .await
            .map_err(|_| anyhow::anyhow!("timeout"))?
            .ok_or_else(|| anyhow::anyhow!("ws closed"))?
            .map_err(|e| anyhow::anyhow!("ws: {e}"))?;
        if let tokio_tungstenite::tungstenite::Message::Text(t) = msg {
            let v: Value = serde_json::from_str(&t)?;
            if v["id"].as_u64() == Some(2) {
                if let Some(exc) =
                    v["result"]["exceptionDetails"]["exception"]["description"].as_str()
                {
                    return Err(anyhow::anyhow!("js: {exc}"));
                }
                return Ok(
                    serde_json::to_string_pretty(&v["result"]["result"]["value"]).unwrap_or_default(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_page_follows_bound_window() {
        let multi = serde_json::json!([
            {"type":"page","title":"a.plasticity - Plasticity","hwnd":"111","webSocketDebuggerUrl":"ws://x/hwnd/111"},
            {"type":"page","title":"b.plasticity - Plasticity","hwnd":"222","webSocketDebuggerUrl":"ws://x/hwnd/222"}
        ]);
        assert_eq!(pick_page(&multi, 222).unwrap()["hwnd"], "222");
        assert_eq!(pick_page(&multi, 0).unwrap()["hwnd"], "111", "未绑定 → 第一个");
        assert!(pick_page(&multi, 333).is_err(), "绑定窗口不在列表里必须报错, 不能连错窗口");
        // 旧代理(无 hwnd 字段): 退回第一个
        let old = serde_json::json!([{"type":"page","title":"Plasticity","webSocketDebuggerUrl":"ws://x/"}]);
        assert_eq!(pick_page(&old, 222).unwrap()["title"], "Plasticity");
    }

    #[test]
    fn selection_from_batch_degrades_per_item() {
        // 完整回包
        let raw = r#"{"sel.snapshot":{"ok":true,"v":{"size":2,"items":[{"kind":"solid","e":7,"info":{"name":"Body7"}},{"kind":"face","e":{"__ctor":"FaceId","id":5,"versionId":"14f2025","__str":"F5"},"info":{"ownerName":"plate","surface":"Plane"}}]}},
            "hist.state":{"ok":true,"v":{"undo":3,"geoVer":9}},
            "cam.get":{"ok":true,"v":{"px":1,"py":2,"pz":3,"qx":0,"qy":0,"qz":0,"qw":1}},
            "err.last":{"ok":true,"v":{"msg":"boom","stack":"at x"}}}"#;
        let s = selection_from_batch(raw);
        assert!(s.ok);
        assert_eq!((s.size, s.undo_len, s.geo_ver), (2, 3, 9));
        assert_eq!(s.items[0].kind, "solid");
        assert_eq!(s.items[0].id, "7");
        assert!(s.items[0].key.is_none());
        assert_eq!(s.items[1].id, "5");
        assert_eq!(s.items[0].name, "Body7");
        assert_eq!((s.items[1].owner.as_str(), s.items[1].surface.as_str()), ("plate", "Plane"));
        assert!(s.items[1].key.as_ref().unwrap().contains("14f2025"));
        assert_eq!(s.error, Some(("boom".into(), "at x".into())));
        assert!(s.camera.unwrap().contains("\"qw\":1"));
        // 撤销栈能力失效: 选择照常, undo 置 -1(降级而非整体失败)
        let raw = r#"{"sel.snapshot":{"ok":true,"v":{"size":0,"items":[]}},
            "hist.state":{"ok":false,"err":"cap:hist.read","missing":["ed.undoStack"]},
            "cam.get":{"ok":true,"v":null},"err.last":{"ok":true,"v":null}}"#;
        let s = selection_from_batch(raw);
        assert!(s.ok && s.undo_len == -1 && s.camera.is_none() && s.error.is_none());
        // 没有 editor / 没有垫片
        let s = selection_from_batch(r#"{"sel.snapshot":{"ok":false,"err":"cap:editor","missing":["editor"]}}"#);
        assert_eq!((s.ok, s.state.as_str()), (false, "no-editor"));
        assert_eq!(selection_from_batch(r#"{"__noShim":true}"#).state, "no-shim");
    }

    #[test]
    fn view_delta_detects_identity_and_mismatch() {
        let a = r#"{"px":10,"py":20,"pz":30,"qx":0,"qy":0,"qz":0,"qw":1}"#;
        // 完全一致 → 0
        assert!(view_delta(a, a).unwrap() < 1e-9);
        // 四元数取反表示同一朝向, 仍应判为一致
        let flipped = r#"{"px":10,"py":20,"pz":30,"qx":0,"qy":0,"qz":0,"qw":-1}"#;
        assert!(view_delta(a, flipped).unwrap() < 1e-9);
        // 位置平移 → 明显非 0(>1e-3 阈值)
        let moved = r#"{"px":110,"py":20,"pz":30,"qx":0,"qy":0,"qz":0,"qw":1}"#;
        assert!(view_delta(a, moved).unwrap() > 1e-3);
        // 缺字段 → 报错, 不静默当 0(R10: 不撒谎)
        assert!(view_delta(a, r#"{"px":1}"#).is_err());
        assert!(view_delta("not-json", a).is_err());
    }

    #[test]
    fn fmt_vpstate_apply_result_reports_failures() {
        // 全部应用成功
        let ok = r#"{"applied":["cam","orbit","shading"],"missing":[],"failed":[]}"#;
        let (good, s) = fmt_vpstate_apply_result(ok);
        assert!(good, "全部应用 = 成功");
        assert!(s.contains("cam"), "{s}");
        assert!(!s.contains("失败"), "{s}");
        // 部分失败必须如实带出来(旧实现会谎报"已恢复")
        let bad = r#"{"applied":["cam"],"missing":["grid"],"failed":["shading:cannot read"]}"#;
        let (good, s) = fmt_vpstate_apply_result(bad);
        assert!(!good, "部分失败 = 不算成功(界面显示红色)");
        assert!(s.contains("shading:cannot read"), "{s}");
        assert!(s.contains("存档未含"), "{s}");
        // 什么都没应用
        let none = r#"{"applied":[],"missing":["cam"],"failed":[]}"#;
        assert!(fmt_vpstate_apply_result(none).1.contains("没有任何项被应用"));
        assert!(!fmt_vpstate_apply_result(none).0);
        // 老格式 / 坏数据原样透出, 不丢信息
        assert_eq!(fmt_vpstate_apply_result("ok:cam,orbit").1, "视口状态已恢复: ok:cam,orbit");
    }

}
