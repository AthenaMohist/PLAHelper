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

/// UI → CDP 线程命令。
#[derive(Debug)]
pub enum CdpCmd {
    /// 应用选择集: 按成员类型回放(solid/sheet=addById(数值); 其余=add<Kind>({id,versionId,...}))。
    ApplySel(Vec<ApplyEntry>),
    /// R7-28: 应用选择集 + 先「重新武装」应用侧选择状态。
    /// 先发 shift+D(duplicate, 跑一个完整命令周期)再 ESC 取消 gizmo, 然后才恢复 —
    /// 实测这样恢复后 G/E 等 gizmo 命令立即可用, 高亮也出现。
    /// 副作用: shift+D 会复制当前选择(用户可 ctrl+Z 撤销, 撤销后需再次应用)。
    ApplySelRearm(Vec<ApplyEntry>),
    /// R7-28: 派发一个按键(经 CDP Input.dispatchKeyEvent)。
    /// 注意: CDP 输入要求 Plasticity 窗口持有 OS 焦点, 否则事件被丢弃。
    Key(KeySpec),
    /// 立即拉一次实体树。
    RefreshTree,
    /// 通用 evaluate(R7-16): 尺寸驱动命令派发等自动化尝试。
    Eval(String),
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
    /// 连接状态。
    Status(String),
    /// 实时选择快照（st=ok 时有意义）。
    Selection(SelectionSnapshot),
    /// 实体树（OutlinerSection 平铺, 含 curves/instances）。
    Tree(Vec<TreeNode>),
    /// Eval 回包。
    EvalResult(String),
    /// R7-28: 请 UI 把前台焦点交还 Plasticity(武装按键丢失时重试前置)。
    RequestFocus,
}

/// 一个选中条目。
/// key = id 元素的原始 JSON(对象形 id 如 face/edge 的 {id,versionId,...});
/// None = 纯数值 id(solid/sheet 的 kernel item id)。应用时原样回放。
#[derive(Debug, Clone)]
pub struct SelItem {
    pub kind: &'static str,
    pub id: String,
    pub name: String,
    pub ctor: String,
    pub key: Option<String>,
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
}

impl CdpHandle {
    /// 启动 CDP 常驻线程（连接失败不致命, 线程内自动重试）。
    pub fn spawn(port: u16) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<CdpCmd>();
        let (ev_tx, ev_rx) = std::sync::mpsc::channel::<CdpEvent>();
        std::thread::Builder::new()
            .name("pla-cdp".into())
            .spawn(move || cdp_thread_main(port, rx, ev_tx))
            .expect("spawn pla-cdp thread");
        Self { tx, rx: ev_rx, port }
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

const SEL_SNAPSHOT_JS: &str = r#"(function(){
  var ed = window.__PLA_EDITOR__;
  if (!ed || !ed.selection) {
    var f = window.__PLA_HOOK__; if (f) ed = f.findEditor();
    if (ed) window.__PLA_EDITOR__ = ed;
  }
  if (!ed || !ed.selection) return JSON.stringify({st:'no-editor'});
  var s = ed.selection.selected;
  if (!s) return JSON.stringify({st:'no-sel'});
  // R7-21: id 集合元素保真采集 — 纯数值原样; 对象形(face/edge 的 {id,versionId,...})
  // 抽 own 可枚举属性 + __ctor + __str(String 化), 应用时按原结构回放。
  function idset(n){ try{ var v=s[n]; if(v&&typeof v.forEach==='function'){ var o=[]; v.forEach(function(x){
      if(x===null||x===undefined){ o.push(null) }
      else if(typeof x==='object'){
        var c={__ctor:(x.constructor&&x.constructor.name)||''};
        try{ Object.keys(x).forEach(function(k){ var val=x[k]; if(val===null||typeof val!=='object') c[k]=val }) }catch(e){}
        try{ c.__str=String(x) }catch(e2){}
        o.push(c)
      } else { o.push(x) }
  }); return o } return [] }catch(e){ return [] } }
  var undo=-1, geoVer=-1;
  try{ undo = ed.history._undoStack.length }catch(e){}
  try{ geoVer = ed.geo._version!==undefined? ed.geo._version : ed.geo.version }catch(e){}
  var cam=null;
  try{ var vp2=null; ed.viewports.forEach(function(v){if(!vp2)vp2=v});
    if(vp2&&vp2.camera){ var c2=vp2.camera;
      cam={px:c2.position.x,py:c2.position.y,pz:c2.position.z,
           qx:c2.quaternion.x,qy:c2.quaternion.y,qz:c2.quaternion.z,qw:c2.quaternion.w}; } }catch(e){}
  var lastErr = null;
  try{ var le = window.__PLA_LAST_ERROR__; if(le && le.msg) lastErr = { msg: String(le.msg).slice(0,200), stack: String(le.stack||'').slice(0,600), ts: le.ts }; }catch(e){}
  return JSON.stringify({ st:'ok', size: (typeof s.size==='number')? s.size : -1,
    undo: undo, geoVer: geoVer, err: lastErr, cam: cam,
    ids: idset('ids'),
    solidIds: idset('solidIds'), sheetIds: idset('sheetIds'),
    faceIds: idset('faceIds'), edgeIds: idset('edgeIds'),
    segmentIds: idset('segmentIds'), curveIds: idset('curveIds'),
    curveVertexIds: idset('curveVertexIds'), groupIds: idset('groupIds'),
    planeIds: idset('planeIds') });
})()"#;

/// 探活: hook 与 editor 引用是否还在页面里(页面重载会清掉)。
/// 返回 "1" = 健在, "0" = 需要重新注入。
const HOOK_PROBE_JS: &str = r#"(function(){
  return (window.__PLA_HOOK__ && window.__PLA_EDITOR__) ? "1" : "0";
})()"#;

const TREE_JS: &str = r#"(function(){
  var root = document.getElementById('root');
  if (!root || !root.__k) return JSON.stringify([]);
  var out = []; var q = [root.__k]; var g = 0; var seen = [];
  function seenHas(v){ for (var i=0;i<seen.length;i++) if (seen[i]===v) return true; return false }
  while(q.length && g++ < 80000){
    var v = q.shift();
    if(!v || typeof v!=='object' || seenHas(v)) continue;
    seen.push(v);
    if(typeof v.type==='function' && (v.type.name==='OutlinerSection'||v.type.name==='Memoed') && v.props && v.props.object){
      var o = v.props.object;
      out.push({ name: String(o.name||''), ctor: (o.constructor&&o.constructor.name)||'',
                 indent: v.props.indent||0, selected: !!v.props.selected });
    }
    var k = v.__k;
    if(Array.isArray(k)) q.push.apply(q, k);
    else if(k && typeof k==='object') q.push(k);
  }
  return JSON.stringify(out);
})()"#;

const INJECT_JS: &str = r#"(function(){
  if (window.__PLA_HOOK__) return 'already';
  window.__PLA_HOOK__ = { findEditor: function(){
    try{
      var root = document.getElementById('root');
      if(!root || !root.__k) return null;
      var q=[root.__k], g=0, seen=[];
      function sh(v){ for(var i=0;i<seen.length;i++) if(seen[i]===v) return true; return false }
      while(q.length && g++<80000){
        var v=q.shift();
        if(!v||typeof v!=='object'||sh(v))continue;
        seen.push(v);
        if(typeof v.type==='function'&&v.type.name==='Outliner'&&v.props&&v.props.model&&v.props.model.editor) return v.props.model.editor;
        var k=v.__k;
        if(Array.isArray(k)) q.push.apply(q,k);
        else if(k&&typeof k==='object') q.push(k);
      }
    }catch(e){}
    return null;
  }};
  return 'ok';
})()"#;

/// R7-27: 错误捕获注入 — 页面重载后钩子会丢, 故与 INJECT_JS 合并一次注入。
/// Plasticity 把命令期异常经 `ed.signals.environmentChanged` 派发(载荷是 Error 实例,
/// 带 stack); 助手把它抓下来存进 window.__PLA_LAST_ERROR__, 由 SEL_SNAPSHOT_JS
/// 带回 Rust 侧显示 —— 用户在助手面板就能看到 "Move Item / ...reading 'active'" 这类
/// 报错原文, 不必只盯着 Plasticity 的弹窗。
const ERR_HOOK_JS: &str = r#"(function(){
  if (window.__PLA_ERR_HOOK__) return 'already';
  var ed = null;
  try{ var f = window.__PLA_HOOK__; if (f) ed = f.findEditor(); }catch(e){}
  if (!ed || !ed.signals || !ed.signals.environmentChanged) return 'no-editor';
  window.__PLA_ERR_HOOK__ = true;
  window.__PLA_LAST_ERROR__ = null;
  try {
    ed.signals.environmentChanged.add(function(p){
      try {
        if (!p) return;
        var isErr = (p instanceof Error) || (p && p.message && p.stack);
        if (!isErr) return;
        window.__PLA_LAST_ERROR__ = { msg: String(p.message || p).slice(0,200),
                                      stack: String(p.stack || '').slice(0,800),
                                      ts: Date.now() };
      } catch(e){}
    });
  } catch(e){ return 'hook-fail'; }
  return 'ok';
})()"#;

fn cdp_thread_main(port: u16, cmd_rx: Receiver<CdpCmd>, ev_tx: std::sync::mpsc::Sender<CdpEvent>) {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            let _ = ev_tx.send(CdpEvent::Status(format!("CDP 线程启动失败: {e}")));
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
            match tokio::time::timeout(Duration::from_secs(5), connect_page(port)).await {
                Ok(Ok(mut ws)) => {
                    let _ = ev_tx.send(CdpEvent::Status("CDP 已连接".into()));
                    if let Err(e) = session_loop(&mut ws, &cmd_rx, &ev_tx).await {
                        eprintln!("[pla-cdp] session err: {e:#}");
                        let _ = ev_tx.send(CdpEvent::Status(format!("CDP 断开: {e}")));
                    }
                }
                Ok(Err(e)) => {
                    eprintln!("[pla-cdp] connect err: {e:#}");
                    // R7-19: 10061(拒连)=9333 无监听 — 补丁版 index.js 任意启动都会
                    // appendSwitch 9333, 拒连只可能是: Plasticity 未启动, 或运行中的
                    // 实例是补丁部署前启动的旧实例(index.js 是启动时加载的)。
                    // 给出可执行指引, 不再让用户对着「积极拒绝」猜。
                    let msg = format!("{e}");
                    let hint = if msg.contains("10061") {
                        " — 9333 无监听: Plasticity 未启动, 或是补丁部署前启动的旧实例。保存文件并重启 Plasticity 即可(补丁已装, 普通启动即生效)"
                    } else {
                        ""
                    };
                    let _ = ev_tx.send(CdpEvent::Status(format!("CDP 等待中({e}){hint}")));
                }
                Err(_) => {
                    eprintln!("[pla-cdp] connect timeout(5s)");
                    // 5s 超时: 连接挂起(典型为 Plasticity 启动初期 renderer 未就绪),
                    // 丢弃重试 — 状态照常上报, UI 不会永远停在「连接中」。
                    let _ = ev_tx.send(CdpEvent::Status("CDP 连接超时, 重试中…".into()));
                }
            }
            // 断开冷却 + 处理积压命令
            if let Ok(CdpCmd::Shutdown) = cmd_rx.try_recv() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(2000)).await;
        }
    });
}

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

/// 连 page target。
async fn connect_page(port: u16) -> anyhow::Result<WsStream> {
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
    // R7-19: Chromium 按 Host 头生成 ws url — Host 缺端口时返回
    // ws://127.0.0.1/...(连 80 被拒)。Host 已带端口, 这里再兜底补一次。
    let ws_url = match ws_url.strip_prefix("ws://127.0.0.1/") {
        Some(rest) => format!("ws://127.0.0.1:{port}/{rest}"),
        None => ws_url,
    };
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await?;
    // 双发握手: Runtime.enable + 注入 hook
    ws_send(&mut ws, serde_json::json!({"id":1,"method":"Runtime.enable"})).await?;
    ws_send(&mut ws, serde_json::json!({"id":2,"method":"Runtime.evaluate","params":{"expression":INJECT_JS,"returnByValue":true}})).await?;
    Ok(ws)
}

async fn session_loop(
    ws: &mut WsStream,
    cmd_rx: &Receiver<CdpCmd>,
    ev_tx: &std::sync::mpsc::Sender<CdpEvent>,
) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    let mut mid: u64 = 10;
    let mut last_snapshot = String::new();
    let mut last_tree = String::new();
    let mut sel_tick: u32 = 0;
    // R7-27: 错误钩子补注节流计数(每 5 轮 ≈ 2.5s 探一次)。
    let mut hook_tick: u32 = 0;

    // 注入 hook(读取结果丢弃)
    ws_send(ws, json_eval(mid, INJECT_JS)).await?;
    mid += 1;

    loop {
        // --- UI 命令(非阻塞) ---
        match cmd_rx.try_recv() {
            Ok(CdpCmd::Shutdown) => return Err(anyhow::anyhow!("shutdown")),
            Ok(CdpCmd::Eval(expr)) => {
                match eval_roundtrip(ws, &mut mid, &expr).await {
                    Ok(r) => {
                        let _ = ev_tx.send(CdpEvent::EvalResult(r));
                    }
                    Err(e) => {
                        let _ = ev_tx.send(CdpEvent::EvalResult(format!("ERR: {e}")));
                    }
                }
            }
            Ok(CdpCmd::RefreshTree) => {
                let tree_raw = eval_roundtrip(ws, &mut mid, TREE_JS).await?;
                if tree_raw != last_tree {
                    last_tree = tree_raw.clone();
                    let nodes: Vec<Value> = serde_json::from_str(&tree_raw).unwrap_or_default();
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
                // v5(替换 v4 网格扫描): 累积式 hover spy + 零 per-point eval
                // 快速网格 + 加密网格兜底 + CDP 真实点击(不变)。
                // v4→v5 动因: v4 每点一次 eval 读 ed.selection.hits — 已实测该
                // 数据源是 stale 点击会话(合成 onClick 也会写它), 禁用; 且 54 次
                // eval 往返拖慢扫描(2~3s)。v5 spy 在页面侧自读 vp.lastPointerEvent
                // 坐标累积进 window.__PLA_MAP__, Rust 只发 mouseMoved, 实测 1.7s 内。
                // 点击管线保持: 真实 CDP 点击(ClickChangeSelectionCommand 完整
                // 周期), 合成 onClick/memento 路线已证伪(MoveCommand 移交时清选择,
                // 缺活跃实体上下文)。视角无关: hover 命中穿透(一射线多面), 目标面
                // 转到背面也能被射线扫到时即可找到。
                if entries.is_empty() {
                    let _ = ev_tx.send(CdpEvent::Status("选择集应用: 成员为空".into()));
                    continue;
                }
                // v6(R7-31l) Phase 0: 还原保存视角(按图索骥)。
                // 保存相机随集存储; 有则先快照用户相机 → 切到保存相机,
                // 结束时(无论成败)恢复。旧集无相机 → 跳过(走当前视角)。
                let saved_cam: Option<String> = entries
                    .iter()
                    .find_map(|e| e.camera.clone())
                    .filter(|c| !c.is_empty());
                let mut user_cam: Option<String> = None;
                if let Some(cam_json) = saved_cam.as_ref() {
                    match eval_roundtrip(ws, &mut mid, CAM_SNAPSHOT_JS).await {
                        Ok(s) if !s.is_empty() => user_cam = Some(s),
                        Ok(_) => eprintln!("[pla-cdp] v6: 用户相机快照为空(仍继续)"),
                        Err(e) => eprintln!("[pla-cdp] v6: 用户相机快照失败(仍继续): {e:#}"),
                    }
                    let enter_js = format!(
                        "(function(){{try{{var o={cam_json};var ed=window.__PLA_EDITOR__;if(!ed){{var f=window.__PLA_HOOK__;if(f)ed=f.findEditor();}}if(!ed)return 'no-ed';var vp=null;ed.viewports.forEach(function(v){{if(!vp)vp=v}});if(!vp||!vp.camera)return 'no-cam';var cam=vp.camera;cam.position.set(o.px,o.py,o.pz);cam.quaternion.set(o.qx,o.qy,o.qz,o.qw);try{{vp.setNeedsRender()}}catch(e2){{}}return 'ok'}}catch(e){{return 'EXC'}}}})()"
                    );
                    match eval_roundtrip(ws, &mut mid, &enter_js).await {
                        Ok(r) if r.contains("ok") => {
                            eprintln!("[pla-cdp] v6: 已切换到保存视角");
                        }
                        other => eprintln!("[pla-cdp] v6: 保存视角切换异常: {other:?}"),
                    }
                    // 相机切换后渲染/命中管线需要一拍稳定
                    tokio::time::sleep(Duration::from_millis(350)).await;
                }
                let js = match apply_sel_js(&entries) {
                    Ok(j) => j,
                    Err(e) => {
                        let _ = ev_tx.send(CdpEvent::Status(format!("选择集应用失败: {e}")));
                        continue;
                    }
                };
                // Phase A: 准备 — 取消活动命令(hover 管线在命令期间暂停!),
                // 清选择, 装扫描 spy, 取视口矩形。返回 [x,y,w,h]
                let prep = match eval_roundtrip(ws, &mut mid, &js).await {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = ev_tx.send(CdpEvent::Status(format!(
                            "选择集应用失败(准备阶段): {e}"
                        )));
                        continue;
                    }
                };
                let rect: (i64, i64, i64, i64) = match serde_json::from_str::<(i64,i64,i64,i64)>(&prep) {
                    Ok(r) => r,
                    Err(_) => {
                        // prep 是错误 JSON {err:...}
                        let (ok, fail, errs) = parse_apply_result(&prep);
                        let _ = ev_tx.send(CdpEvent::Status(format!(
                            "选择集应用: {ok} 项成功, {fail} 项失败 {errs}"
                        )));
                        continue;
                    }
                };
                // Phase B(v5): 快速网格扫描 — spy 在页面侧自读坐标并累积进
                // __PLA_MAP__, Rust 只发 mouseMoved(零 per-point eval), 每扫完
                // 一行(9 点)一次进度 eval, 目标全中即提前退。54 点实测 1.7s 内。
                let target_ids: std::collections::HashSet<String> =
                    entries.iter().map(|e| e.id.clone()).collect();
                let mut pos_map: std::collections::HashMap<String, (i64, i64)> =
                    std::collections::HashMap::new();
                let missing = |pm: &std::collections::HashMap<String, (i64, i64)>| -> usize {
                    target_ids.iter().filter(|t| !pm.contains_key(*t)).count()
                };
                'scan: for gy in 0..SCAN_ROWS_COARSE {
                    for gx in 0..SCAN_COLS_COARSE {
                        let x = rect.0 + (rect.2 as f64 * (gx as f64 + 0.5)
                            / SCAN_COLS_COARSE as f64) as i64;
                        let y = rect.1 + (rect.3 as f64 * (gy as f64 + 0.5)
                            / SCAN_ROWS_COARSE as f64) as i64;
                        let _ = dispatch_mouse_move(ws, &mut mid, x, y).await;
                        tokio::time::sleep(Duration::from_millis(SCAN_STEP_MS)).await;
                    }
                    match eval_roundtrip(ws, &mut mid, MAP_READ_JS).await {
                        Ok(s) => absorb_map(&s, &mut pos_map),
                        Err(e) => {
                            eprintln!("[pla-cdp] map read(粗网格行级) 传输失败: {e:#}");
                        }
                    }
                    if missing(&pos_map) == 0 {
                        break 'scan;
                    }
                }
                // 兜底: 仍缺的 id 再扫一轮 12x18 加密网格(只补缺失)。
                let coarse_missing = missing(&pos_map);
                if coarse_missing > 0 {
                    'fine: for gy in 0..SCAN_ROWS_FINE {
                        for gx in 0..SCAN_COLS_FINE {
                            if missing(&pos_map) == 0 {
                                break 'fine;
                            }
                            let x = rect.0 + (rect.2 as f64 * (gx as f64 + 0.5)
                                / SCAN_COLS_FINE as f64) as i64;
                            let y = rect.1 + (rect.3 as f64 * (gy as f64 + 0.5)
                                / SCAN_ROWS_FINE as f64) as i64;
                            let _ = dispatch_mouse_move(ws, &mut mid, x, y).await;
                            tokio::time::sleep(Duration::from_millis(SCAN_STEP_MS)).await;
                        }
                        match eval_roundtrip(ws, &mut mid, MAP_READ_JS).await {
                            Ok(s) => absorb_map(&s, &mut pos_map),
                            Err(e) => {
                                eprintln!("[pla-cdp] map read(细网格行级) 传输失败: {e:#}");
                            }
                        }
                    }
                }
                let rescued = coarse_missing.saturating_sub(missing(&pos_map));
                eprintln!(
                    "[pla-cdp] scan(v5): {} 个 id 定位 / 目标 {} / 兜底补中 {rescued}",
                    pos_map.len(),
                    target_ids.len()
                );
                // Phase C(v5.1): 遮挡感知点击分组 — hover 穿透把同射线前后遮挡面
                // 记到同一坐标, 但 shift+click 不穿透(只命中最前面的面)。同坐标
                // 组只点一次(无论组内几个目标 id), 谁被选中交给点击后校验判定。
                let mut located: Vec<(String, i64, i64)> = Vec::new();
                let mut misses: Vec<String> = Vec::new();
                for e in &entries {
                    match pos_map.get(&e.id) {
                        Some(&(px, py)) => located.push((e.id.clone(), px, py)),
                        None => misses.push(e.id.clone()),
                    }
                }
                if located.is_empty() {
                    let _ = ev_tx.send(CdpEvent::Status(format!(
                        "选择集应用失败: 当前视角扫描未命中目标面({} 个; 可旋转视角后重试)",
                        misses.len()
                    )));
                    continue;
                }
                let click_points = plan_clicks(&located);
                eprintln!(
                    "[pla-cdp] click(v5.1): {} 个目标 → {} 次点击(同坐标组去重)",
                    located.len(),
                    click_points.len()
                );
                // 发送成功的组目标参与 Phase D 校验; 发送失败的整组直接计失败 —
                // 点击丢失时其目标必然不在 faceIds, 若混进校验会被误标为遮挡。
                let mut clicked: Vec<(String, i64, i64)> = Vec::new();
                let mut click_failed: Vec<String> = Vec::new();
                for (i, (px, py)) in click_points.iter().enumerate() {
                    let shift = i > 0;
                    let sent = dispatch_mouse_click(ws, &mut mid, *px, *py, shift).await;
                    let group = click_group(&located, *px, *py);
                    match sent {
                        Ok(()) => {
                            for (id, lx, ly) in group {
                                clicked.push((id.clone(), *lx, *ly));
                            }
                        }
                        Err(e) => {
                            eprintln!("[pla-cdp] click 发送失败 @({px},{py}): {e:#}");
                            for (id, _lx, _ly) in group {
                                click_failed.push(id.clone());
                            }
                        }
                    }
                }
                // Phase D(v5.1): 校验 + 唤醒渲染。verify 回读 faceIds, 对每个
                // 已定位目标 id 求交集 — 没进 faceIds 的即同坐标组里被遮挡的,
                // 如实计失败(旋转视角分离是标准解法, 不尝试自动分离)。
                let verify = r#"(function(){
  var ed=window.__PLA_EDITOR__;
  if(!ed||!ed.selection){ var f=window.__PLA_HOOK__; if(f) ed=f.findEditor(); if(ed) window.__PLA_EDITOR__=ed; }
  if(!ed||!ed.selection) return JSON.stringify({ok:0,fail:0,errs:['no-editor']});
  var s=ed.selection.selected;
  var vp=null; ed.viewports.forEach(function(v){if(!vp)vp=v});
  try{ if(vp&&vp.setNeedsRender) vp.setNeedsRender() }catch(e){}
  delete window.__PLA_SCAN__;
  return JSON.stringify({selN:(typeof s.size==='number')?s.size:-1, faces:(function(){var a=[];try{s.faceIds.forEach(function(x){a.push(String(x&&x.versionId!==undefined?x.versionId:x))})}catch(e){}return a})()});})()"#;
                let verify_raw = eval_roundtrip(ws, &mut mid, verify).await;
                if let Err(ref e) = verify_raw {
                    eprintln!("[pla-cdp] verify eval 失败: {e:#}");
                }
                let faces_opt = match verify_raw {
                    Ok(ref s) => extract_face_ids(s),
                    Err(_) => None,
                };
                // 校验读取失败(通道异常/回包无法解析)时如实上报, 不谎报成败 —
                // faceIds 区分不了"没选中"与"没读到", 不能当作遮挡计数。
                let face_ids = match faces_opt {
                    Some(f) => f,
                    None => {
                        let _ = ev_tx.send(CdpEvent::Status(format!(
                            "选择集应用: {} 项已点击, 但校验读取失败(可查看选择状态确认)",
                            clicked.len()
                        )));
                        let _ = dispatch_mouse_move(
                            ws,
                            &mut mid,
                            rect.0 + rect.2 / 2,
                            rect.1 + rect.3 / 2,
                        )
                        .await;
                        let _ = eval_roundtrip(ws, &mut mid, CLEAR_MAP_JS).await;
                        continue;
                    }
                };
                let mut tally = tally_occlusion(&clicked, &face_ids);
                let mut misses = misses;
                // Phase E(v5.2): 遮挡自动补救 — 快照相机 → addById 临时选中
                // 被遮挡面 → navigateToSelection() 转到该面正视 → 扫描定位 →
                // shift+click 补选 → 处理完恢复用户原相机。用户全程无感知。
                if tally.occluded > 0 || !misses.is_empty() {
                    let mut rescued_occ: Vec<String> = Vec::new();
                    // 补救对象 = 被遮挡面 + 扫描未命中面(小面/背对面:
                    // 当前视角射线扫不到 — 自动转到该面正视即可见, v5.4)
                    let mut occluded_ids: Vec<String> = clicked
                        .iter()
                        .filter(|(id, _, _)| !face_ids.contains(id))
                        .map(|(id, _, _)| id.clone())
                        .collect();
                    occluded_ids.extend(misses.iter().cloned());
                    // 相机快照(position/quaternion 直写直读已验证; 不碰投影矩阵)
                    let snap = eval_roundtrip(ws, &mut mid, CAM_SNAPSHOT_JS).await;
                    let cam_saved = snap.unwrap_or_else(|e| {
                        eprintln!("[pla-cdp] cam snapshot 失败(将跳过补救): {e:#}");
                        String::new()
                    });
                    if !cam_saved.is_empty() {
                        for occ_id in &occluded_ids {
                            // v6.1: 若该成员自带保存相机, 先直达该视角(比
                            // navigateToSelection 的单面取景更忠实于保存环境)
                            if let Some(mc) = entries
                                .iter()
                                .find(|e| e.id == *occ_id)
                                .and_then(|e| e.camera.clone())
                            {
                                let enter_js = format!(
                                    "(function(){{try{{var o={mc};var ed=window.__PLA_EDITOR__;if(!ed){{var f=window.__PLA_HOOK__;if(f)ed=f.findEditor();}}if(!ed)return 'no-ed';var vp=null;ed.viewports.forEach(function(v){{if(!vp)vp=v}});var cam=vp.camera;cam.position.set(o.px,o.py,o.pz);cam.quaternion.set(o.qx,o.qy,o.qz,o.qw);try{{vp.setNeedsRender()}}catch(e2){{}}return 'ok'}}catch(e){{return 'EXC'}}}})()"
                                );
                                let _ = eval_roundtrip(ws, &mut mid, &enter_js).await;
                                tokio::time::sleep(Duration::from_millis(350)).await;
                            }
                            // 1) addById 临时选中该面(navigateToSelection 只读选择,
                            //    不需要点击管线的活跃实体上下文)
                            let select_js = format!(
                                "(function(){{try{{var ed=window.__PLA_EDITOR__;if(!ed||!ed.selection){{var f=window.__PLA_HOOK__;if(f)ed=f.findEditor();}}if(!ed)return 'no-ed';ed.click.addById('2:{}');return 'ok'}}catch(e){{return 'EXC'}}}})()",
                                occ_id
                            );
                            if let Err(e) = eval_roundtrip(ws, &mut mid, &select_js).await {
                                eprintln!("[pla-cdp] 补救选择 {occ_id} 失败: {e:#}");
                                continue;
                            }
                            // 2) 转到该面正视
                            if let Err(e) = eval_roundtrip(ws, &mut mid, NAV_TO_SELECTION_JS).await {
                                eprintln!("[pla-cdp] navigateToSelection 失败: {e:#}");
                                continue;
                            }
                            // 导航是动画, 等待相机稳定
                            tokio::time::sleep(Duration::from_millis(700)).await;
                            // 3) 重扫定位该面(粗网格即可, 正视下必在前排)
                            let mut occ_pos: Option<(i64, i64)> = None;
                            'occ_scan: for gy in 0..SCAN_ROWS_COARSE {
                                for gx in 0..SCAN_COLS_COARSE {
                                    let x = rect.0 + (rect.2 as f64 * (gx as f64 + 0.5) / SCAN_COLS_COARSE as f64) as i64;
                                    let y = rect.1 + (rect.3 as f64 * (gy as f64 + 0.5) / SCAN_ROWS_COARSE as f64) as i64;
                                    let _ = dispatch_mouse_move(ws, &mut mid, x, y).await;
                                    tokio::time::sleep(Duration::from_millis(25)).await;
                                }
                                if let Ok(s) = eval_roundtrip(ws, &mut mid, MAP_READ_JS).await {
                                    let mut m2 = std::collections::HashMap::new();
                                    absorb_map(&s, &mut m2);
                                    if let Some(&(px, py)) = m2.get(occ_id) {
                                        occ_pos = Some((px, py));
                                        break 'occ_scan;
                                    }
                                }
                            }
                            // 4) shift+click 补选 + 取消可能的自动命令
                            if let Some((px, py)) = occ_pos {
                                let _ = eval_roundtrip(ws, &mut mid, CLEAR_MAP_JS).await;
                                if dispatch_mouse_click(ws, &mut mid, px, py, true).await.is_ok() {
                                    let _ = eval_roundtrip(ws, &mut mid, CANCEL_CMD_JS).await;
                                    tokio::time::sleep(Duration::from_millis(250)).await;
                                    // 复核: 进 faceIds 才算救回
                                    if let Ok(v) = eval_roundtrip(ws, &mut mid, VERIFY_FACES_JS).await {
                                        if let Some(fs) = extract_face_ids(&v) {
                                            if fs.contains(occ_id) {
                                                rescued_occ.push(occ_id.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // 5) 恢复用户相机
                        let restore_js = format!(
                            "(function(){{try{{var o={cam_saved};var ed=window.__PLA_EDITOR__;if(!ed){{var f=window.__PLA_HOOK__;if(f)ed=f.findEditor();}}if(!ed)return 'no-ed';var vp=null;ed.viewports.forEach(function(v){{if(!vp)vp=v}});var cam=vp.camera;cam.position.set(o.px,o.py,o.pz);cam.quaternion.set(o.qx,o.qy,o.qz,o.qw);try{{vp.setNeedsRender()}}catch(e2){{}}return 'ok'}}catch(e){{return 'EXC'}}}})()"
                        );
                        if let Err(e) = eval_roundtrip(ws, &mut mid, &restore_js).await {
                            eprintln!("[pla-cdp] 相机恢复失败: {e:#}");
                        }
                        let _ = dispatch_mouse_move(ws, &mut mid, rect.0 + rect.2 / 2, rect.1 + rect.3 / 2).await;
                        // 重算 tally: 救回的计入成功(从遮挡/缺失两侧移除)
                        if !rescued_occ.is_empty() {
                            // 归因: 救回目标原本来自"被遮挡"还是"未扫到"
                            let occl_set: std::collections::HashSet<&String> = clicked
                                .iter()
                                .filter(|(id, _, _)| !face_ids.contains(id))
                                .map(|(id, _, _)| id)
                                .collect();
                            let rescued_from_occ =
                                rescued_occ.iter().filter(|r| occl_set.contains(*r)).count();
                            let rescued_from_miss = rescued_occ.len().saturating_sub(rescued_from_occ);
                            tally.confirmed += rescued_occ.len();
                            tally.occluded = tally.occluded.saturating_sub(rescued_from_occ);
                            misses.retain(|m| !rescued_occ.contains(m));
                            let _ = ev_tx.send(CdpEvent::Status(format!(
                                "自动转视角补救: {}/{} 项补选成功(遮挡 {} + 未扫到 {})",
                                rescued_occ.len(),
                                occluded_ids.len(),
                                rescued_from_occ,
                                rescued_from_miss
                            )));
                        }
                    }
                }
                let ok = tally.confirmed;
                let fail = misses.len() + tally.occluded + click_failed.len();
                let _ = dispatch_mouse_move(ws, &mut mid, rect.0 + rect.2 / 2, rect.1 + rect.3 / 2).await;
                // 唤醒 hover 后清累积表: 点击完成后的正常 hover 又会写入新命中,
                // 留着既无用途也会无限增长(甚至干扰下次应用), 统一在收尾清零。
                let _ = eval_roundtrip(ws, &mut mid, CLEAR_MAP_JS).await;
                let msg = if fail > 0 {
                    let mut parts: Vec<String> = Vec::new();
                    if tally.occluded > 0 {
                        parts.push(format!("{} 项被遮挡", tally.occluded));
                    }
                    if misses.len() > 0 {
                        parts.push(format!("{} 项未扫到", misses.len()));
                    }
                    if !click_failed.is_empty() {
                        parts.push(format!("{} 项点击发送失败", click_failed.len()));
                    }
                    let detail = parts.join(", ");
                    format!("选择集应用: {ok} 项成功, {fail} 项失败({detail}, 请旋转视角后重试)")
                } else if rescued > 0 {
                    format!("选择集已应用: {ok} 项成功({rescued} 项经加密扫描)")
                } else {
                    format!("选择集已应用: {ok} 项成功(真实点击管线)")
                };
                // v6: 恢复用户相机(保存视角仅用于点击, 用完即还)
                if let Some(uc) = user_cam.as_ref() {
                    let restore_js = format!(
                        "(function(){{try{{var o={uc};var ed=window.__PLA_EDITOR__;if(!ed){{var f=window.__PLA_HOOK__;if(f)ed=f.findEditor();}}if(!ed)return 'no-ed';var vp=null;ed.viewports.forEach(function(v){{if(!vp)vp=v}});var cam=vp.camera;cam.position.set(o.px,o.py,o.pz);cam.quaternion.set(o.qx,o.qy,o.qz,o.qw);try{{vp.setNeedsRender()}}catch(e2){{}}return 'ok'}}catch(e){{return 'EXC'}}}})()"
                    );
                    if let Err(e) = eval_roundtrip(ws, &mut mid, &restore_js).await {
                        eprintln!("[pla-cdp] v6: 用户相机恢复失败: {e:#}");
                    }
                }
                let _ = ev_tx.send(CdpEvent::Status(msg));
            }
            // R7-28: 先「重新武装」再恢复 — 见 CdpCmd::ApplySelRearm 注释。
            Ok(CdpCmd::ApplySelRearm(entries)) => {
                if entries.is_empty() {
                    let _ = ev_tx.send(CdpEvent::Status("选择集应用: 成员为空".into()));
                    continue;
                }
                // 1) shift+D — duplicate 跑一个完整命令周期, 把应用侧选择状态
                //    重新武装(实测: 不武装时 G 静默无反应, 武装后立即可用)。
                //    CDP 按键偶发不被应用处理(焦点被夺/事件丢失), 故用 undo 深度
                //    验证并重试, 最多 3 轮 — 否则会静默退回"未武装"的老问题。
                let undo_before = eval_roundtrip(ws, &mut mid, UNDO_DEPTH_JS)
                    .await
                    .ok()
                    .and_then(|s| s.trim().parse::<i64>().ok())
                    .unwrap_or(-1);
                let mut armed = false;
                for attempt in 1..=3 {
                    let _ = dispatch_key(
                        ws,
                        &mut mid,
                        &KeySpec { key: "d".into(), code: "KeyD".into(), vk: 68, modifiers: 8 },
                    )
                    .await;
                    tokio::time::sleep(Duration::from_millis(1300)).await;
                    let undo_after = eval_roundtrip(ws, &mut mid, UNDO_DEPTH_JS)
                        .await
                        .ok()
                        .and_then(|s| s.trim().parse::<i64>().ok())
                        .unwrap_or(-1);
                    if undo_after > undo_before {
                        armed = true;
                        break;
                    }
                    eprintln!(
                        "[pla-cdp] rearm: shift+D 未生效(undo {undo_before}→{undo_after}), 第 {attempt}/3 次重试"
                    );
                    // 重试前把焦点抢回 Plasticity(按键丢失的常见原因)
                    let _ = ev_tx.send(CdpEvent::RequestFocus);
                    tokio::time::sleep(Duration::from_millis(400)).await;
                }
                if !armed {
                    let _ = ev_tx.send(CdpEvent::Status(
                        "重新武装失败: shift+D 未被 Plasticity 接收（请确认 Plasticity 窗口在前台）, 已直接恢复".into(),
                    ));
                }
                // 2) ESC — 取消 duplicate 留下的移动 gizmo
                let _ = dispatch_key(
                    ws,
                    &mut mid,
                    &KeySpec { key: "Escape".into(), code: "Escape".into(), vk: 27, modifiers: 0 },
                )
                .await;
                tokio::time::sleep(Duration::from_millis(1000)).await;
                // 3) 恢复
                let js = match apply_sel_js(&entries) {
                    Ok(j) => j,
                    Err(e) => {
                        let _ = ev_tx.send(CdpEvent::Status(format!("选择集应用失败: {e}")));
                        continue;
                    }
                };
                let res = match eval_roundtrip(ws, &mut mid, &js).await {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = ev_tx.send(CdpEvent::Status(format!(
                            "选择集应用失败: {e}（CDP 通道异常, 非选择数据问题）"
                        )));
                        continue;
                    }
                };
                eprintln!("[pla-cdp] ApplySel(rearm) raw result: {res}");
                let (ok, fail, errs) = parse_apply_result(&res);
                let msg = if fail > 0 || !errs.is_empty() {
                    format!("选择集应用: {ok} 项成功, {fail} 项失败 {errs}")
                } else if ok == 0 {
                    "选择集应用: 成员为空, 未发送任何项".to_string()
                } else {
                    format!(
                        "选择集已应用: {ok} 项成功（已重新武装, G/E 等命令可直接用）"
                    )
                };
                let _ = ev_tx.send(CdpEvent::Status(msg));
            }
            Ok(CdpCmd::Key(spec)) => {
                if let Err(e) = dispatch_key(ws, &mut mid, &spec).await {
                    let _ = ev_tx.send(CdpEvent::Status(format!("按键派发失败: {e}")));
                }
            }
            Err(TryRecvError::Disconnected) => return Err(anyhow::anyhow!("ui gone")),
            Err(_) => {}
        }

        // --- 轮询选择快照(每 2 个 tick ≈ 1s; 500ms 底噪) ---
        sel_tick += 1;
        if sel_tick % 2 == 0 {
            // R7-25: 页面上下文可能被重置 — Plasticity renderer reload(切文档/
            // 崩溃恢复)会产生**新的 CDP target**(实测 target id 由 66CCF320… 变为
            // 97D532B6…), 旧 target 上注入的 window.__PLA_HOOK__ / __PLA_EDITOR__
            // 随之蒸发。此时所有 JS 都拿不到 editor → 选择快照恒空、ApplySel 报
            // no-editor。轮询前先探活, 缺失就重新注入(INJECT_JS 幂等)。
            match eval_roundtrip(ws, &mut mid, HOOK_PROBE_JS).await {
                Ok(p) if p.trim() == "1" => {}
                Ok(_) => {
                    eprintln!("[pla-cdp] hook 丢失(页面已重载?), 重新注入");
                    let _ = eval_roundtrip(ws, &mut mid, INJECT_JS).await;
                }
                Err(e) => {
                    eprintln!("[pla-cdp] hook 探活失败: {e:#}");
                }
            }
            // R7-27: 错误捕获钩子随页面重载一起丢, 缺了就补注(否则助手看不到
            // Plasticity 侧的命令报错, 例如 G 移动的 "reading 'active'")。
            if hook_tick % 5 == 0 {
                let _ = eval_roundtrip(ws, &mut mid, ERR_HOOK_JS).await;
            }
            hook_tick += 1;
            let raw = eval_roundtrip(ws, &mut mid, SEL_SNAPSHOT_JS).await?;
            if raw != last_snapshot {
                last_snapshot = raw.clone();
                let v: Value = serde_json::from_str(&raw)?;
                if v["st"] == "ok" {
                    let mut items = Vec::new();
                    // R7-21: 元素分型 — 纯数值(solid/sheet kernel id)→key=None;
                    // 对象形(face/edge {id,versionId,...})→key=原始 JSON 原样保存/回放。
                    for (kind, key) in [
                        ("solid", "solidIds"),
                        ("sheet", "sheetIds"),
                        ("face", "faceIds"),
                        ("edge", "edgeIds"),
                        ("segment", "segmentIds"),
                        ("curve", "curveIds"),
                        ("curveVertex", "curveVertexIds"),
                        ("group", "groupIds"),
                        ("plane", "planeIds"),
                    ] {
                        if let Some(arr) = v[key].as_array() {
                            for it in arr {
                                let (id, raw) = match it {
                                    Value::String(sv) => (sv.clone(), None),
                                    Value::Number(nv) => (nv.to_string(), None),
                                    Value::Object(ov) => {
                                        let raw = Value::Object(ov.clone()).to_string();
                                        let id = ov
                                            .get("id")
                                            .map(|x| match x {
                                                Value::String(s) => s.clone(),
                                                other => other.to_string(),
                                            })
                                            .or_else(|| {
                                                ov.get("__str").and_then(|s| s.as_str()).map(String::from)
                                            })
                                            .unwrap_or_default();
                                        (id, Some(raw))
                                    }
                                    _ => (String::new(), None),
                                };
                                items.push(SelItem {
                                    kind,
                                    id,
                                    name: String::new(),
                                    ctor: kind.to_string(),
                                    key: raw,
                                });
                            }
                        }
                    }
                    let _ = ev_tx.send(CdpEvent::Selection(SelectionSnapshot {
                        ok: true,
                        state: "ok".into(),
                        size: v["size"].as_i64().unwrap_or(-1),
                        items,
                        undo_len: v["undo"].as_i64().unwrap_or(-1),
                        geo_ver: v["geoVer"].as_i64().unwrap_or(-1),
                        error: v["err"]["msg"].as_str().map(|m| {
                            (
                                m.to_string(),
                                v["err"]["stack"].as_str().unwrap_or("").to_string(),
                            )
                        }),
                        camera: v["cam"].as_object().map(|c| Value::Object(c.clone()).to_string()),
                    }));
                } else {
                    let _ = ev_tx.send(CdpEvent::Selection(SelectionSnapshot {
                        ok: false,
                        state: v["st"].as_str().unwrap_or("unknown").to_string(),
                        size: -1,
                        items: Vec::new(),
                        undo_len: -1,
                        geo_ver: -1,
                        error: None,
                        camera: None,
                    }));
                }
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

/// v5: ApplySel Phase A — 鼠标管线的准备脚本(累积式 hover spy 版)。
/// 取消活动命令(hover 管线在命令期间暂停) + 清选择 + 清 gizmo/command body
/// 属性 + 装累积 spy + 清零累积表 + 返回视口 rect "[x,y,w,h]" 或错误 JSON。
///
/// 累积式 spy(v5 提速核心, 已实测): 包装 ed.changeSelection.onHover 信号,
/// 回调内**自读** vp.lastPointerEvent.clientX/Y(=页面坐标, CDP Input 同一
/// 坐标系; onHover 自带的 x/y 参数恒 0 不可用), 把本次 hits 里每个
/// topologyItem||object 的 versionId 累积进 window.__PLA_MAP__(id→{x,y},
/// **只记首次出现** — 后续命中不覆盖, 保证坐标来自确定扫过的点)。
/// Rust 侧扫描只发 mouseMoved, 零 per-point eval, 行级读一次表即可。
///
/// 幂等: window.__PLA_SPY5B__ 标记已装; 已装时仅清空 __PLA_MAP__ 复用。
/// (调试期页面里可能残留旧版 spy — 只要 __PLA_MAP__ 由本 spy 写入即兼容。)
///
/// 数据源铁律: 本管线只信 spy 累积表; ed.selection.hits.hits 是 stale 点击
/// 会话(合成 onClick 也会写它), 绝不可作扫描数据源(v4 的教训)。
fn apply_sel_js(entries: &[ApplyEntry]) -> anyhow::Result<String> {
    // Phase A 只需 entries 用于去重校验(空校验在 Rust 侧), 这里生成准备脚本。
    let _ = entries;
    Ok(r#"(function(){
  var ed=window.__PLA_EDITOR__;
  if(!ed||!ed.selection){ var f=window.__PLA_HOOK__; if(f) ed=f.findEditor(); if(ed) window.__PLA_EDITOR__=ed; }
  if(!ed||!ed.selection) return JSON.stringify({ok:0,fail:0,errs:['no-editor']});
  try{ ed.executor.cancelActiveCommand() }catch(e){}
  try{ ed.click.clear() }catch(e){}
  document.body.removeAttribute('gizmo');
  document.body.removeAttribute('command');
  var c=document.querySelector('plasticity-viewport canvas');
  if(!c) return JSON.stringify({ok:0,fail:0,errs:['no-canvas']});
  var cs = ed.changeSelection;
  if(cs && cs.onHover && window.__PLA_SPY_VER__ !== 2){
    var orig = cs.onHover;
    var wrapped = function(hits){
      try{
        var edDyn = window.__PLA_HOOK__ ? window.__PLA_HOOK__.findEditor() : null;
        if(!edDyn) edDyn = ed;
        var vp=null; edDyn.viewports.forEach(function(v){if(!vp)vp=v});
        var pe = vp && vp.lastPointerEvent;
        var px = pe && typeof pe.clientX==='number' ? pe.clientX : null;
        var py = pe && typeof pe.clientY==='number' ? pe.clientY : null;
        if(px!==null && py!==null && hits && hits.length){
          var m = window.__PLA_MAP__;
          if(!m){ m={}; window.__PLA_MAP__=m }
          for(var i=0;i<hits.length;i++){
            var t = hits[i] && (hits[i].topologyItem || hits[i].object);
            var vid = t && t.versionId!==undefined ? String(t.versionId) : null;
            if(vid && !m[vid]) m[vid]={x:px,y:py};
          }
        }
      }catch(e2){}
      if(typeof orig==='function'){ try{ return orig.apply(cs, arguments) }catch(e3){} }
      return undefined;
    };
    try{ Object.defineProperty(wrapped,'name',{value:'__plaSpy5b'}) }catch(e4){}
    cs.onHover = wrapped;
    window.__PLA_SPY5B__ = true;
    window.__PLA_SPY_VER__ = 2;
  }
  window.__PLA_MAP__ = {};
  var r=c.getBoundingClientRect();
  return JSON.stringify([Math.round(r.x),Math.round(r.y),Math.round(r.width),Math.round(r.height)]);
})()"#
    .to_string())
}

/// v5: 收尾清累积表 — 点击完成后的正常 hover 又会写入新命中, 表留着既无用
/// 又会无限增长(甚至污染下次应用), Phase D 唤醒 hover 后统一清零。
const CLEAR_MAP_JS: &str = r#"(function(){
  try{ if(window.__PLA_MAP__) window.__PLA_MAP__={} }catch(e){}
  return '1';})()"#;

/// v5: 读取累积表 — 返回 spy 在扫描中累积的全部 id→[x,y](JSON 对象, 值为
/// 数组形, 与 Rust 侧 (i64,i64) 元组反序列化匹配)。表空/未装 spy → "{}"。
const MAP_READ_JS: &str = r#"(function(){
  var m = window.__PLA_MAP__;
  if(!m) return '{}';
  var o={}; var n=0;
  for(var k in m){
    if(!Object.prototype.hasOwnProperty.call(m,k)) continue;
    var v=m[k];
    if(v && typeof v.x==='number' && typeof v.y==='number'){
      o[k]=[Math.round(v.x),Math.round(v.y)];
      if(++n>=512) break;
    }
  }
  return JSON.stringify(o);})()"#;

/// v5.2: 相机快照 — 返回 {px,py,pz,qx,qy,qz,qw}(JSON 字符串)。
/// position/quaternion 直读; 绝不读/写投影参数(fov/near/zoom — 正交
/// ProxyCamera 上 updateProjectionMatrix 有破坏性副作用, R7-31h 教训)。
const CAM_SNAPSHOT_JS: &str = r#"(function(){
  var ed=window.__PLA_EDITOR__;
  if(!ed||!ed.selection){ var f=window.__PLA_HOOK__; if(f) ed=f.findEditor(); if(ed) window.__PLA_EDITOR__=ed; }
  if(!ed||!ed.selection) return '';
  var vp=null; ed.viewports.forEach(function(v){if(!vp)vp=v});
  if(!vp||!vp.camera) return '';
  var cam=vp.camera;
  return JSON.stringify({px:cam.position.x,py:cam.position.y,pz:cam.position.z,
    qx:cam.quaternion.x,qy:cam.quaternion.y,qz:cam.quaternion.z,qw:cam.quaternion.w});})()"#;

/// v5.2: 转到当前选择正视(navigateToSelection, 实测相机立即响应;
/// 动画驱动, 调用方需等待 ~700ms 让相机稳定)。
const NAV_TO_SELECTION_JS: &str = r#"(function(){
  var ed=window.__PLA_EDITOR__;
  if(!ed||!ed.selection){ var f=window.__PLA_HOOK__; if(f) ed=f.findEditor(); if(ed) window.__PLA_EDITOR__=ed; }
  if(!ed||!ed.selection) return 'no-ed';
  var vp=null; ed.viewports.forEach(function(v){if(!vp)vp=v});
  if(!vp||typeof vp.navigateToSelection!=='function') return 'no-nav';
  try{ vp.navigateToSelection(); return 'ok' }catch(e){ return 'EXC' }})()"#;

/// v5.2: 取消活动命令 + 清 body 的 gizmo/command 属性(补选后收尾)。
const CANCEL_CMD_JS: &str = r#"(function(){
  var ed=window.__PLA_EDITOR__;
  if(!ed){ var f=window.__PLA_HOOK__; if(f) ed=f.findEditor(); }
  if(!ed) return 'no-ed';
  try{ ed.executor.cancelActiveCommand() }catch(e){}
  document.body.removeAttribute('gizmo');
  document.body.removeAttribute('command');
  return 'ok'})()"#;

/// v5.2: 复核当前 faceIds(补救后判定该面是否真的进了选择)。
const VERIFY_FACES_JS: &str = r#"(function(){
  var ed=window.__PLA_EDITOR__;
  if(!ed||!ed.selection){ var f=window.__PLA_HOOK__; if(f) ed=f.findEditor(); if(ed) window.__PLA_EDITOR__=ed; }
  if(!ed||!ed.selection) return JSON.stringify({selN:-1, faces:[]});
  var s=ed.selection.selected;
  return JSON.stringify({selN:(typeof s.size==='number')?s.size:-1,
    faces:(function(){var a=[];try{s.faceIds.forEach(function(x){a.push(String(x&&x.versionId!==undefined?x.versionId:x))})}catch(e){}return a})()});})()"#;

/// v5: 吸收 MAP_READ_JS 回包(JSON 对象字符串)进 pos_map。
/// 回包异常时不吞错误 — 打日志供诊断, 已定位的不受影响。
fn absorb_map(raw: &str, pos_map: &mut std::collections::HashMap<String, (i64, i64)>) {
    match serde_json::from_str::<std::collections::HashMap<String, (i64, i64)>>(raw) {
        Ok(m) => {
            for (fid, xy) in m {
                pos_map.entry(fid).or_insert(xy);
            }
        }
        Err(e) => eprintln!("[pla-cdp] map read 解析失败: {e} (raw: {raw})"),
    }
}

/// v5: 粗网格行×列(6x9=54 点, 实测 1.7s 内扫完)。
const SCAN_ROWS_COARSE: i64 = 6;
const SCAN_COLS_COARSE: i64 = 9;
/// v5: 兜底加密网格行×列(12x18=216 点, 仅粗网格后仍缺目标时触发)。
const SCAN_ROWS_FINE: i64 = 12;
const SCAN_COLS_FINE: i64 = 18;
/// v5: 扫描点间隔 — mouseMoved 后等 onHover 回调把命中写入累积表。
const SCAN_STEP_MS: u64 = 25;

/// v5.1: 点击计划 — 对已定位目标按 (px,py) 完全相同分组去重。
/// hover 穿透把同射线前后遮挡面记到同一坐标, 而 shift+click 不穿透(只命中
/// 最前面的面), 同坐标组点多次既无效又浪费一次完整点击周期(≤300ms/次)。
/// 返回去重后的点击坐标序列(保持首次出现顺序, 首坐标=普通点击替换选择)。
fn plan_clicks(located: &[(String, i64, i64)]) -> Vec<(i64, i64)> {
    let mut seen: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
    let mut out: Vec<(i64, i64)> = Vec::new();
    for (_id, px, py) in located {
        if seen.insert((*px, *py)) {
            out.push((*px, *py));
        }
    }
    out
}

/// v5.1(O2): 取点击计划中某坐标组的全部成员 — 发送失败时整组计失败,
/// 发送成功时整组参与 Phase D 校验; 组划分保证每个 located 目标恰好落组一次。
fn click_group<'a>(
    located: &'a [(String, i64, i64)],
    px: i64,
    py: i64,
) -> Vec<&'a (String, i64, i64)> {
    located
        .iter()
        .filter(|(_, lx, ly)| (*lx, *ly) == (px, py))
        .collect()
}

/// v5.1: 点击后校验结果。
#[derive(Debug, Clone, PartialEq, Eq)]
struct OcclusionTally {
    /// 确认进入 faceIds 的目标数。
    confirmed: usize,
    /// 已点击但未进入 faceIds 的目标数(同坐标组里被遮挡的)。
    occluded: usize,
}

/// v5.1: 遮挡判定 — 把每个已定位目标 id 与点击后 faceIds 求交集。
/// faceIds 无法区分“确实没选中”与“校验读取失败”, 故调用方在读取失败时
/// 直接走校验异常分支, 不会把通道故障误报成遮挡。
fn tally_occlusion(located: &[(String, i64, i64)], face_ids: &[String]) -> OcclusionTally {
    let sel: std::collections::HashSet<&str> =
        face_ids.iter().map(|s| s.as_str()).collect();
    let mut confirmed = 0usize;
    let mut occluded = 0usize;
    for (id, _px, _py) in located {
        if sel.contains(id.as_str()) {
            confirmed += 1;
        } else {
            occluded += 1;
        }
    }
    OcclusionTally { confirmed, occluded }
}

/// v5.1: 解析 Phase D verify 回包 → faceIds 列表。
/// 回包应为 {selN, faces:[id,...]} JSON; 解析失败时如实返回 None(不吞错误),
/// 由调用方走校验异常分支。
fn extract_face_ids(raw: &str) -> Option<Vec<String>> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let arr = v["faces"].as_array()?;
    Some(
        arr.iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
    )
}

/// R7-28: 探活 — 取 undo 栈深度(用于验证按键是否真被应用处理)。
/// CDP 按键要求 Plasticity 持有 OS 焦点, 偶发丢失; 用 undo 深度变化判定成败。
const UNDO_DEPTH_JS: &str = r#"(function(){
  var ed = window.__PLA_EDITOR__;
  if (!ed || !ed.selection) { var f = window.__PLA_HOOK__; if (f) ed = f.findEditor(); if (ed) window.__PLA_EDITOR__ = ed; }
  if (!ed) return "-1";
  try { return String(ed.history._undoStack.length); } catch(e){ return "-1"; }
})()"#;

/// R7-28: 解析 ApplySel JS 的 JSON 回包 → (ok, fail, errs 串)。
fn parse_apply_result(res: &str) -> (i64, i64, String) {
    let v: Value = serde_json::from_str(res).unwrap_or(Value::Null);
    let ok = v["ok"].as_i64().unwrap_or(0);
    let fail = v["fail"].as_i64().unwrap_or(0);
    let errs = v["errs"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .unwrap_or_default();
    (ok, fail, errs)
}

async fn ws_send(ws: &mut WsStream, v: Value) -> anyhow::Result<()> {
    use futures_util::SinkExt;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(v.to_string()))
        .await
        .map_err(|e| anyhow::anyhow!("ws send: {e}"))
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

/// R7-31d: 经 CDP Input 派发一次鼠标点击(mouseMoved→mousePressed→mouseReleased)。
/// shift=true 时全部事件带 modifiers:8(实测变体2姿势, 无需前置 shift keyDown)。
/// 这是**浏览器级输入注入**, 走 Plasticity 完整命中管线(ClickChangeSelectionCommand
/// 真实命令周期 + 活跃实体上下文), 与用户真实点击等价 — 合成 onClick/memento 路线
/// 已证伪(MoveCommand 移交时清选择, 因缺活跃实体上下文)。
async fn dispatch_mouse_click(
    ws: &mut WsStream,
    mid: &mut u64,
    x: i64,
    y: i64,
    shift: bool,
) -> anyhow::Result<()> {
    let m = *mid;
    *mid += 2;
    let mo = if shift { 8 } else { 0 };
    ws_send(
        ws,
        serde_json::json!({"id": m, "method": "Input.dispatchMouseEvent", "params": {
            "type": "mouseMoved", "x": x, "y": y, "button": "none", "buttons": 0,
            "modifiers": mo, "pointerType": "mouse"
        }}),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(120)).await;
    ws_send(
        ws,
        serde_json::json!({"id": m + 1, "method": "Input.dispatchMouseEvent", "params": {
            "type": "mousePressed", "x": x, "y": y, "button": "left", "buttons": 1,
            "clickCount": 1, "modifiers": mo, "pointerType": "mouse"
        }}),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(70)).await;
    ws_send(
        ws,
        serde_json::json!({"id": m + 1, "method": "Input.dispatchMouseEvent", "params": {
            "type": "mouseReleased", "x": x, "y": y, "button": "left", "buttons": 0,
            "clickCount": 1, "modifiers": mo, "pointerType": "mouse"
        }}),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(320)).await;
    Ok(())
}

/// R7-31d: 单次 mouseMoved(扫描/唤醒渲染用)。
async fn dispatch_mouse_move(
    ws: &mut WsStream,
    mid: &mut u64,
    x: i64,
    y: i64,
) -> anyhow::Result<()> {
    let m = *mid;
    *mid += 1;
    ws_send(
        ws,
        serde_json::json!({"id": m, "method": "Input.dispatchMouseEvent", "params": {
            "type": "mouseMoved", "x": x, "y": y, "button": "none", "buttons": 0,
            "pointerType": "mouse"
        }}),
    )
    .await?;
    Ok(())
}

/// 发 evaluate 并收对应 id 的回包(跳过事件帧)。
async fn eval_roundtrip(
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

async fn http_get(port: u16, path: &str) -> anyhow::Result<String> {
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
    fn dump_apply_sel_js() {
        let entries = vec![
            ApplyEntry { kind: "face".into(), id: "14f2025".into(), key: None, camera: None },
            ApplyEntry { kind: "edge".into(), id: "14e1984".into(), key: None, camera: None },
            ApplyEntry { kind: "solid".into(), id: "7".into(), key: None, camera: None },
        ];
        let js = apply_sel_js(&entries).unwrap();
        eprintln!("===GENERATED-JS-BEGIN===");
        eprintln!("{js}");
        eprintln!("===GENERATED-JS-END===");
        // 基本健全性: 不得含 // 注释痕迹, 必须完整收尾
        assert!(!js.contains("//"), "JS 内含 // 注释会吞掉后续代码");
        assert!(js.ends_with("})()"), "JS 未完整收尾");
        // v5: Phase A 关键步骤存在
        assert!(js.contains("cancelActiveCommand"), "缺少取消活动命令");
        assert!(js.contains("getBoundingClientRect"), "缺少视口矩形获取");
        // v5: 累积式 hover spy 装配
        assert!(js.contains("__PLA_SPY5B__"), "缺少 spy 幂等标记");
        assert!(js.contains("__PLA_MAP__"), "缺少累积表");
        assert!(js.contains("lastPointerEvent"), "缺少页面侧自读坐标");
        assert!(js.contains("changeSelection"), "缺少 onHover 信号包装");
        // v5 铁律: 不得读 stale 点击会话
        assert!(!js.contains("selection.hits"), "Phase A 禁用 stale hits 数据源");
    }

    #[test]
    fn map_read_js_sane() {
        assert!(!MAP_READ_JS.contains("//"), "JS 内含 // 注释");
        assert!(MAP_READ_JS.ends_with("})()"), "MAP_READ_JS 未完整收尾");
        assert!(MAP_READ_JS.contains("__PLA_MAP__"));
    }

    #[test]
    fn absorb_map_merges_and_keeps_first() {
        let mut m = std::collections::HashMap::new();
        absorb_map(r#"{"a":[10,20],"b":[30,40]}"#, &mut m);
        assert_eq!(m.get("a"), Some(&(10, 20)));
        assert_eq!(m.get("b"), Some(&(30, 40)));
        // 首次坐标优先(or_insert): 后到的不覆盖
        absorb_map(r#"{"a":[99,99],"c":[1,2]}"#, &mut m);
        assert_eq!(m.get("a"), Some(&(10, 20)), "首次记录不可被覆盖");
        assert_eq!(m.get("c"), Some(&(1, 2)));
        // 坏回包不 panic、不清已有数据
        absorb_map("not-json", &mut m);
        absorb_map("{}", &mut m);
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn scan_grid_geometry_covers_viewport() {
        // 网格几何: 任意 rect 下, 粗/细网格列数行数均为正, 点位落在 rect 内
        assert!(SCAN_COLS_COARSE > 0 && SCAN_ROWS_COARSE > 0);
        assert!(SCAN_COLS_FINE > SCAN_COLS_COARSE && SCAN_ROWS_FINE > SCAN_ROWS_COARSE);
        let rect = (100, 50, 900, 600);
        for gx in 0..SCAN_COLS_COARSE {
            let x = rect.0 + (rect.2 as f64 * (gx as f64 + 0.5) / SCAN_COLS_COARSE as f64) as i64;
            assert!(x >= rect.0 && x < rect.0 + rect.2, "粗网格 x 越界: {x}");
        }
        for gy in 0..SCAN_ROWS_FINE {
            let y = rect.1 + (rect.3 as f64 * (gy as f64 + 0.5) / SCAN_ROWS_FINE as f64) as i64;
            assert!(y >= rect.1 && y < rect.1 + rect.3, "细网格 y 越界: {y}");
        }
    }

    #[test]
    fn plan_clicks_dedupes_same_coord_group() {
        // 真机场景复刻: 4f718 与 17f3640 同坐标 (983,193)(同射线前后遮挡),
        // 5f758 屏幕分离独立坐标 — 3 目标只应产生 2 次点击。
        let located = vec![
            ("4f718".to_string(), 983, 193),
            ("17f3640".to_string(), 983, 193),
            ("5f758".to_string(), 412, 507),
        ];
        let clicks = plan_clicks(&located);
        assert_eq!(clicks, vec![(983, 193), (412, 507)], "同坐标组必须去重为一次点击");
        // 首坐标 = 普通点击(替换选择), 后续 = shift 追加 — 顺序保持首现次序
        assert_eq!(clicks.len(), located.len() - 1);
        // 全独立坐标: 不去重
        let solo = vec![
            ("a".to_string(), 1, 1),
            ("b".to_string(), 2, 2),
        ];
        assert_eq!(plan_clicks(&solo).len(), 2);
        // 三面同坐标: 仍只点一次
        let triple = vec![
            ("a".to_string(), 5, 5),
            ("b".to_string(), 5, 5),
            ("c".to_string(), 5, 5),
        ];
        assert_eq!(plan_clicks(&triple).len(), 1);
    }

    #[test]
    fn tally_occlusion_counts_confirmed_and_occluded() {
        let located = vec![
            ("4f718".to_string(), 983, 193),
            ("17f3640".to_string(), 983, 193),
            ("5f758".to_string(), 412, 507),
        ];
        // 点击后 faceIds 只剩最前面的 4f718 + 独立面 → 17f3640 被遮挡计失败
        let faces = vec!["4f718".to_string(), "5f758".to_string()];
        let t = tally_occlusion(&located, &faces);
        assert_eq!(t, OcclusionTally { confirmed: 2, occluded: 1 });
        // 全中
        let all = vec!["4f718".to_string(), "17f3640".to_string(), "5f758".to_string()];
        assert_eq!(
            tally_occlusion(&located, &all),
            OcclusionTally { confirmed: 3, occluded: 0 }
        );
        // 全被遮挡(faceIds 空)
        assert_eq!(
            tally_occlusion(&located, &[]),
            OcclusionTally { confirmed: 0, occluded: 3 }
        );
    }

    #[test]
    fn extract_face_ids_parses_verify_payload() {
        let raw = r#"{"selN":2,"faces":["4f718","5f758"]}"#;
        assert_eq!(
            extract_face_ids(raw),
            Some(vec!["4f718".to_string(), "5f758".to_string()])
        );
        // faces 缺失 / 非 JSON → None(走校验异常分支, 不吞错误)
        assert_eq!(extract_face_ids(r#"{"selN":-1}"#), None);
        assert_eq!(extract_face_ids("not-json"), None);
    }

    #[test]
    fn click_groups_partition_targets_exactly_once() {
        // O2: 点击计划与坐标分组必须恰好划分全部 located 目标(无遗漏/无重复) —
        // 这是 ok+fail=总数 恒成立的根。
        let located = vec![
            ("4f718".to_string(), 983, 193),
            ("17f3640".to_string(), 983, 193),
            ("5f758".to_string(), 412, 507),
        ];
        let plan = plan_clicks(&located);
        let mut seen: Vec<&String> = Vec::new();
        for (px, py) in &plan {
            for (id, _lx, _ly) in click_group(&located, *px, *py) {
                assert!(!seen.contains(&&id), "目标 {id} 被划入多个组");
                seen.push(id);
            }
            // 组内成员坐标与组坐标一致
            for (_id, lx, ly) in click_group(&located, *px, *py) {
                assert_eq!((*lx, *ly), (*px, *py));
            }
        }
        assert_eq!(seen.len(), located.len(), "每个目标必须恰好落组一次");
        // 模拟一组发送失败: fail = misses + occluded + click_failed 保持总数恒等
        let clicked: Vec<(String, i64, i64)> = click_group(&located, plan[0].0, plan[0].1)
            .into_iter()
            .cloned()
            .collect();
        let click_failed: Vec<String> = click_group(&located, plan[1].0, plan[1].1)
            .into_iter()
            .map(|(id, _, _)| id.clone())
            .collect();
        let misses: Vec<String> = Vec::new();
        let faces = vec!["4f718".to_string()]; // 只有最前面那面真被选中
        let tally = tally_occlusion(&clicked, &faces);
        let ok = tally.confirmed;
        let fail = misses.len() + tally.occluded + click_failed.len();
        assert_eq!(ok + fail, located.len() + misses.len());
        // 分组语义: 同坐标组(983,193) 2 员, 独立点(412,507) 1 员
        assert_eq!(click_group(&located, 983, 193).len(), 2);
        assert_eq!(click_group(&located, 412, 507).len(), 1);
        assert_eq!(click_group(&located, 1, 1).len(), 0);
    }
}
