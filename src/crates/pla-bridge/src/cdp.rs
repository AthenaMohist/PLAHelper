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
    /// R8-1: 视口状态采集回包(JSON 字符串; err 字段非空表示失败)。
    VpStateResult(String),
    /// R8-1: 视口截图回包(PNG 字节数)。
    ViewportPng(usize),
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

/// R14: 注册 `pla:roll:left` / `pla:roll:right` — 绕屏幕法线(视线)轴按步进角滚转。
///
/// Plasticity 自带 `viewport:roll:*` 固定 90°、不读步进角; `orbit:rotate:*` 只有上下左右。
/// 实测(26.2.3): 把轨道控制器的目标朝向 `quaternionEnd` 右乘绕本地 Z 轴的旋转即为滚转 —
/// 视线方向不变、相机不平移, 阻尼动画自然过渡, 角度精确(15° 实测)。
/// 步进角每次现读 `orbitControls.rotateDiscreteIncrement`, 与方向键一致、随设置实时变化。
/// left = 画面逆时针(相机绕本地 Z 负转), right = 画面顺时针。
/// 监听器挂在 `<plasticity-viewport>` 元素上(与 orbit:rotate:* 同处), 按元素标记幂等;
/// 视口元素重建或页面重载后由周期钩子补注册。keymap 绑定见 pla-ui `plasticity_config`。
const ROLL_HOOK_JS: &str = r#"(function(){
  var ed = window.__PLA_EDITOR__;
  if (!ed || !ed.registry || !ed.viewports) return 'no-editor';
  var reg = ed.registry;
  // 幂等判断看注册表本身(不用元素标记: 标记可能先于注册失败而被置上)。
  function registered(el){
    var m = reg.inlineListenersByCommandName;
    var w = m && (m instanceof Map ? m.get('pla:roll:left') : m['pla:roll:left']);
    return !!(w && w.has && w.has(el));
  }
  var els = document.querySelectorAll('plasticity-viewport');
  var n = 0;
  for (var i = 0; i < els.length; i++) {
    var el = els[i];
    if (registered(el)) continue;
    (function(el){
      function vpOf(){
        var best = null;
        ed.viewports.forEach(function(v){
          if (!best && v && v.orbitControls && v.orbitControls.domElement && el.contains(v.orbitControls.domElement)) best = v;
        });
        return best || ed._activeViewport || ed.viewports.values().next().value;
      }
      function roll(sign){
        var v = vpOf(); if (!v || !v.orbitControls || !v.camera) return;
        var oc = v.orbitControls;
        var Q = oc.quaternionEnd.constructor, V = v.camera.position.constructor;
        var step = oc.rotateDiscreteIncrement || (Math.PI / 12);
        oc.quaternionEnd.multiply(new Q().setFromAxisAngle(new V(0, 0, 1), sign * step));
        oc.needsUpdate = true;
        try { oc.setNeedsUpdate(); } catch (e) {}
      }
      // Plasticity 的 CommandRegistry.add 只有对象形式: add(target, {命令名: 回调})(arity 2)。
      reg.add(el, {
        'pla:roll:left': function(){ roll(-1); },
        'pla:roll:right': function(){ roll(1); }
      });
    })(el);
    if (registered(el)) n++;
  }
  return 'ok:' + n;
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
                // R8(成功率): 按成员相机分组应用 — 多视角选择集(A 面视角1选中、
                // B 面视角2选中)每组切到自己的保存视角再扫描点击, 避免单一视角
                // 下大量面被遮挡/漏扫。分组键 = camera JSON 字符串(None 归一组,
                // 走当前视角)。组顺序 = 首次出现顺序(稳定)。
                let mut cam_groups: Vec<(Option<String>, Vec<ApplyEntry>)> = Vec::new();
                for e in entries {
                    let key = e.camera.clone().filter(|c| !c.is_empty());
                    match cam_groups.iter_mut().find(|(k, _)| *k == key) {
                        Some((_, v)) => v.push(e.clone()),
                        None => cam_groups.push((key, vec![e.clone()])),
                    }
                }
                let n_groups = cam_groups.len();
                eprintln!(
                    "[pla-cdp] R8: {n_groups} 个相机组, 成员分布 {:?}",
                    cam_groups
                        .iter()
                        .map(|(_, v)| v.len())
                        .collect::<Vec<_>>()
                );
                // 全流程汇总
                let mut total_ok = 0usize;
                let mut total_fail = 0usize;
                let mut fail_parts: Vec<String> = Vec::new();
                let mut total_rescued = 0usize;
                // 用户相机只快照一次, 全部组结束后恢复
                let mut user_cam: Option<String> = None;
                let mut user_cam_taken = false;
                'groups: for (gi, (group_cam, group_entries)) in
                    cam_groups.into_iter().enumerate()
                {
                    let entries = group_entries;
                    // v6(R7-31l) Phase 0: 还原该组保存视角(按图索骥)。
                    if let Some(cam_json) = group_cam.as_ref() {
                        if !user_cam_taken {
                            user_cam_taken = true;
                            match eval_roundtrip(ws, &mut mid, CAM_SNAPSHOT_JS).await {
                                Ok(s) if !s.is_empty() => user_cam = Some(s),
                                Ok(_) => eprintln!("[pla-cdp] v6: 用户相机快照为空(仍继续)"),
                                Err(e) => {
                                    eprintln!("[pla-cdp] v6: 用户相机快照失败(仍继续): {e:#}")
                                }
                            }
                        }
                        let enter_js = PLA_VIEW_APPLY_JS_TMPL.replace("__VIEW__", cam_json);
                        match eval_roundtrip(ws, &mut mid, &enter_js).await {
                            Ok(r) if r.starts_with("ok") => {
                                eprintln!("[pla-cdp] R8: 组 {gi} 已切换到保存视角: {r}");
                            }
                            other => {
                                eprintln!("[pla-cdp] R8: 组 {gi} 视角切换异常: {other:?}")
                            }
                        }
                        // 相机切换后渲染/命中管线需要一拍稳定
                        tokio::time::sleep(Duration::from_millis(350)).await;
                        // R10: 回读校验 —— 只写 camera 而 orbitControls 仍在旧
                        // target/radius 上时, 下一帧会把相机拉回原位。以前这是
                        // 静默失败(表现为"未扫到"), 现在如实报警。
                        if let Ok(now) = eval_roundtrip(ws, &mut mid, CAM_SNAPSHOT_JS).await {
                            match view_delta(cam_json, &now) {
                                Ok(d) if d > 1e-3 => {
                                    eprintln!(
                                        "[pla-cdp] R8: 组 {gi} 视角切换未生效(回读偏差 {d:.4})"
                                    );
                                    let _ = ev_tx.send(CdpEvent::Status(format!(
                                        "警告: 视角组 {gi} 切视角未生效(偏差 {d:.4}) — 该组目标大概率扫不到"
                                    )));
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    eprintln!("[pla-cdp] R8: 组 {gi} 视角回读校验失败: {e}")
                                }
                            }
                        }
                    }
                let js = match apply_sel_js(&entries, gi > 0) {
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
                        continue 'groups;
                    }
                };
                let rect: (i64, i64, i64, i64) = match serde_json::from_str::<(i64,i64,i64,i64)>(&prep) {
                    Ok(r) => r,
                    Err(_) => {
                        // prep 是错误 JSON {err:...}
                        let (ok, fail, errs) = parse_apply_result(&prep);
                        total_fail += fail as usize;
                        fail_parts.push(errs);
                        continue 'groups;
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
                    // R8: 相机绑定语义下"未扫到"= 该面在保存视角下不可见(极小面/
                    // 边), 提示如实但不引导"旋转视角"(用户旋转会破坏组视角)。
                    fail_parts.push(format!(
                        "{} 项未扫到(保存视角下不可见)",
                        misses.len()
                    ));
                    total_fail += misses.len();
                    continue 'groups;
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
                    // R8: 首组第一击=普通点击(替换语义); 其余一律 shift 追加
                    // (组内第 2+ 击 + 多相机组的全部后续组)。
                    let shift = i > 0 || gi > 0;
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
                        total_ok += clicked.len();
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
                        continue 'groups;
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
                                let enter_js = PLA_VIEW_APPLY_JS_TMPL.replace("__VIEW__", &mc);
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
                        let restore_js = PLA_VIEW_APPLY_JS_TMPL.replace("__VIEW__", &cam_saved);
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
                // R8: 组结果并入汇总(不再逐组发 Status, 统一收尾一条)
                total_ok += ok;
                total_fail += fail;
                total_rescued += rescued;
                if fail > 0 {
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
                    fail_parts.push(parts.join(", "));
                }
                } // ---- 'groups 循环结束 ----
                // 全部组完成: 恢复用户相机 + 统一上报
                if let Some(uc) = user_cam.as_ref() {
                    let restore_js = PLA_VIEW_APPLY_JS_TMPL.replace("__VIEW__", uc);
                    if let Err(e) = eval_roundtrip(ws, &mut mid, &restore_js).await {
                        eprintln!("[pla-cdp] v6: 用户相机恢复失败: {e:#}");
                    }
                }
                let group_note = if n_groups > 1 {
                    format!("({n_groups} 个视角组)")
                } else {
                    String::new()
                };
                let msg = if total_fail > 0 {
                    let detail = fail_parts.join("; ");
                    format!(
                        "选择集应用: {total_ok} 项成功, {total_fail} 项失败({detail}){group_note}"
                    )
                } else if total_rescued > 0 {
                    format!(
                        "选择集已应用: {total_ok} 项成功({total_rescued} 项经加密扫描){group_note}"
                    )
                } else {
                    format!("选择集已应用: {total_ok} 项成功(真实点击管线){group_note}")
                };
                let _ = ev_tx.send(CdpEvent::Status(msg));
            }
            // ---- R8-1: 视口状态采集/恢复 + 视口裁剪截图 ----
            Ok(CdpCmd::VpStateSave) => {
                let r = eval_roundtrip(ws, &mut mid, VPSTATE_SNAPSHOT_JS).await;
                match r {
                    Ok(s) => {
                        let _ = ev_tx.send(CdpEvent::VpStateResult(s));
                    }
                    Err(e) => {
                        let _ = ev_tx.send(CdpEvent::VpStateResult(format!(
                            "{{\"err\":\"{e}\"}}"
                        )));
                    }
                }
            }
            Ok(CdpCmd::VpStateApply(state)) => {
                let js = VPSTATE_RESTORE_JS_TMPL.replace("__STATE__", &state);
                match eval_roundtrip(ws, &mut mid, &js).await {
                    Ok(r) => {
                        let _ = ev_tx.send(CdpEvent::Status(fmt_vpstate_apply_result(&r)));
                    }
                    Err(e) => {
                        let _ = ev_tx
                            .send(CdpEvent::Status(format!("视口状态恢复失败: {e}")));
                    }
                }
            }
            Ok(CdpCmd::CaptureViewport(out_path)) => {
                // 视口 DOM rect(CSS px) → clip(逻辑 px); captureScreenshot
                // 的 clip 以 CSS 像素为单位(与 viewport 缩放无关, 按 dpr 放大输出)。
                let rect_js = r#"(function(){
  var ed=window.__PLA_EDITOR__;
  if(!ed||!ed.selection){ var f=window.__PLA_HOOK__; if(f) ed=f.findEditor(); if(ed) window.__PLA_EDITOR__=ed; }
  if(!ed) return '';
  var vps=[]; ed.viewports.forEach(function(v){vps.push(v)});
  if(!vps.length) return '';
  try{ var r=vps[0].container.getBoundingClientRect();
    return JSON.stringify({x:r.x,y:r.y,width:r.width,height:r.height,scale:1}) }catch(e){ return '' }})()"#;
                let rect_raw = match eval_roundtrip(ws, &mut mid, rect_js).await {
                    Ok(s) if !s.is_empty() => s,
                    _ => {
                        let _ = ev_tx.send(CdpEvent::Status(
                            "视口截图失败: 读不到视口区域(仍将保存, 无缩略图)".into(),
                        ));
                        let _ = ev_tx.send(CdpEvent::ViewportPng(0));
                        continue;
                    }
                };
                let clip: Value = match serde_json::from_str(&rect_raw) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = ev_tx.send(CdpEvent::Status(format!(
                            "视口截图失败: rect 解析 {e}(仍将保存, 无缩略图)"
                        )));
                        let _ = ev_tx.send(CdpEvent::ViewportPng(0));
                        continue;
                    }
                };
                // 先强制渲染一帧(按需渲染模式下 canvas 可能是旧帧)
                let force = r#"(function(){
  var ed=window.__PLA_EDITOR__;
  if(!ed){ var f=window.__PLA_HOOK__; if(f) ed=f.findEditor(); }
  if(!ed) return 'no-ed';
  var vps=[]; ed.viewports.forEach(function(v){vps.push(v)});
  if(!vps.length) return 'no-vp';
  try{ vps[0].setNeedsRender() }catch(e){}
  return 'ok'})()"#;
                let _ = eval_roundtrip(ws, &mut mid, force).await;
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
                    let _ = ev_tx.send(CdpEvent::Status(format!("视口截图失败: {e}")));
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
                                                let _ = ev_tx.send(CdpEvent::Status(format!(
                                                    "视口截图写盘失败: {e}"
                                                )));
                                                continue;
                                            }
                                        }
                                        Err(e) => {
                                            let _ = ev_tx.send(CdpEvent::Status(format!(
                                                "视口截图解码失败: {e}"
                                            )));
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
                    let _ = ev_tx.send(CdpEvent::Status(
                        "视口截图失败: 未收到截图数据(仍将保存, 无缩略图)".into(),
                    ));
                    // R8b: 失败也发 ViewportPng(0) — UI 的 pending 保存流程
                    // (选择集/视口状态)靠该事件落盘, 不发会永远挂起无法保存。
                    let _ = ev_tx.send(CdpEvent::ViewportPng(0));
                }
            }
            // (R7-28「重新武装」ApplySelRearm 已于 R7-32 移除 — 方案被用户否决;
            //  v4+ 真实点击管线自身建立活跃实体上下文, 无需武装。)
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
                // R14: Alt+←/→ 步进滚转命令(视口元素重建/页面重载后自动补注册)。
                let _ = eval_roundtrip(ws, &mut mid, ROLL_HOOK_JS).await;
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
fn apply_sel_js(entries: &[ApplyEntry], keep_sel: bool) -> anyhow::Result<String> {
    // Phase A 只需 entries 用于去重校验(空校验在 Rust 侧), 这里生成准备脚本。
    // R8: keep_sel=true(多相机组的第 2+ 组)跳过 click.clear — 保住前面组
    // 已点好的选择; 首组 keep_sel=false 维持"替换语义"。
    let _ = entries;
    let clear_js = if keep_sel { "" } else { "try{ ed.click.clear() }catch(e){}" };
    let js = r#"(function(){
  var ed=window.__PLA_EDITOR__;
  if(!ed||!ed.selection){ var f=window.__PLA_HOOK__; if(f) ed=f.findEditor(); if(ed) window.__PLA_EDITOR__=ed; }
  if(!ed||!ed.selection) return JSON.stringify({ok:0,fail:0,errs:['no-editor']});
  try{ ed.executor.cancelActiveCommand() }catch(e){}
  __CLEAR_SEL__
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
})()"#;
    Ok(js.replace("__CLEAR_SEL__", clear_js))
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
///
/// R10: 额外尽力附带 orbit(target/focal/radius/zoom) 与 zoom/fov。原因:
/// Plasticity 的相机由 orbitControls 驱动, 只写 camera.position/quaternion
/// 会在下一帧被 orbit.update() 覆盖 —— 这是「选择集多视角组还原失败」的头号
/// 嫌疑(单组=当前视角不受影响, 多组必然失败)。老存档只有 7 个数字, 消费方
/// 必须容忍 orbit/zoom 缺失(见 `PLA_VIEW_APPLY_JS_TMPL`)。
const CAM_SNAPSHOT_JS: &str = r#"(function(){
  var ed=window.__PLA_EDITOR__;
  if(!ed||!ed.selection){ var f=window.__PLA_HOOK__; if(f) ed=f.findEditor(); if(ed) window.__PLA_EDITOR__=ed; }
  if(!ed||!ed.selection) return '';
  var vp=null; ed.viewports.forEach(function(v){if(!vp)vp=v});
  if(!vp||!vp.camera) return '';
  var cam=vp.camera;
  var o={px:cam.position.x,py:cam.position.y,pz:cam.position.z,
    qx:cam.quaternion.x,qy:cam.quaternion.y,qz:cam.quaternion.z,qw:cam.quaternion.w};
  try{ if(typeof cam.zoom==='number') o.zoom=cam.zoom }catch(e){}
  try{ if(typeof cam.fov==='number') o.fov=cam.fov }catch(e){}
  try{
    var oc=vp.orbitControls;
    if(oc){
      var orb={};
      if(oc.target&&oc.target.x!==undefined) orb.target=[oc.target.x,oc.target.y,oc.target.z];
      if(oc.targetEnd&&oc.targetEnd.x!==undefined) orb.targetEnd=[oc.targetEnd.x,oc.targetEnd.y,oc.targetEnd.z];
      if(oc.focalOffset&&oc.focalOffset.x!==undefined) orb.focal=[oc.focalOffset.x,oc.focalOffset.y,oc.focalOffset.z];
      if(oc.focalOffsetEnd&&oc.focalOffsetEnd.x!==undefined) orb.focalEnd=[oc.focalOffsetEnd.x,oc.focalOffsetEnd.y,oc.focalOffsetEnd.z];
      if(typeof oc.radius==='number') orb.radius=oc.radius;
      if(typeof oc.radiusEnd==='number') orb.radiusEnd=oc.radiusEnd;
      if(typeof oc.zoom==='number') orb.zoom=oc.zoom;
      if(typeof oc.zoomEnd==='number') orb.zoomEnd=oc.zoomEnd;
      o.orbit=orb;
    }
  }catch(e2){}
  return JSON.stringify(o);})()"#;

/// R10: 统一的「切到某个已存视角」脚本(把原先复制三份的内联 JS 收口)。
/// 入参 __VIEW__ = CAM_SNAPSHOT_JS 的 JSON(可缺 orbit/zoom/fov — 老存档兼容)。
///
/// 顺序要点: 先写 camera, 再写 orbitControls(否则下一帧 orbit.update() 用
/// 旧 target/radius 把相机拉回去), 最后 setNeedsRender。投影参数只在**透视**
/// 相机上写(正交 ProxyCamera 的 updateProjectionMatrix 有破坏性副作用)。
/// 返回 'ok:cam' / 'ok:orbit' / 'no-ed' / 'no-cam' / 'EXC:<msg>'。
const PLA_VIEW_APPLY_JS_TMPL: &str = r#"(function(){
  try{
    var o=__VIEW__;
    var ed=window.__PLA_EDITOR__;
    if(!ed||!ed.selection){ var f=window.__PLA_HOOK__; if(f) ed=f.findEditor(); if(ed) window.__PLA_EDITOR__=ed; }
    if(!ed||!ed.selection) return 'no-ed';
    var vp=null; ed.viewports.forEach(function(v){if(!vp)vp=v});
    if(!vp||!vp.camera) return 'no-cam';
    var cam=vp.camera;
    cam.position.set(o.px,o.py,o.pz);
    cam.quaternion.set(o.qx,o.qy,o.qz,o.qw);
    var persp = (cam.isPerspectiveCamera===true);
    var proj=false;
    if(persp){
      try{ if(typeof o.zoom==='number' && typeof cam.zoom==='number'){ cam.zoom=o.zoom; proj=true } }catch(e1){}
      try{ if(typeof o.fov==='number' && typeof cam.fov==='number'){ cam.fov=o.fov; proj=true } }catch(e2){}
    }
    var orbit=false;
    try{
      var oc=vp.orbitControls;
      if(o.orbit&&oc){
        function sv(dst,src){ if(dst&&dst.set&&src&&src.length===3){ try{ dst.set(src[0],src[1],src[2]); return true }catch(e){} } return false }
        sv(oc.target,o.orbit.target);
        sv(oc.targetEnd,o.orbit.targetEnd||o.orbit.target);
        sv(oc.focalOffset,o.orbit.focal);
        sv(oc.focalOffsetEnd,o.orbit.focalEnd||o.orbit.focal);
        if(typeof o.orbit.radius==='number'){
          oc.radius=o.orbit.radius;
          oc.radiusEnd=(o.orbit.radiusEnd!==undefined&&o.orbit.radiusEnd!==null)?o.orbit.radiusEnd:o.orbit.radius;
        }
        if(typeof o.orbit.zoom==='number'){
          oc.zoom=o.orbit.zoom;
          oc.zoomEnd=(o.orbit.zoomEnd!==undefined&&o.orbit.zoomEnd!==null)?o.orbit.zoomEnd:o.orbit.zoom;
        }
        if(oc.quaternion&&oc.quaternion.set){
          try{ oc.quaternion.set(o.qx,o.qy,o.qz,o.qw) }catch(e7){}
          try{ if(oc.quaternionEnd&&oc.quaternionEnd.set) oc.quaternionEnd.set(o.qx,o.qy,o.qz,o.qw) }catch(e8){}
        }
        orbit=true;
      }
    }catch(e3){}
    if(persp&&proj){ try{ cam.updateProjectionMatrix() }catch(e4){} }
    try{ var oc2=vp.orbitControls; if(oc2&&typeof oc2.update==='function') oc2.update() }catch(e5){}
    try{ vp.setNeedsRender() }catch(e6){}
    return 'ok'+(proj?':proj':'')+(orbit?':orbit':':cam');
  }catch(e){ return 'EXC:'+(e&&e.message?e.message:String(e)) }})()"#;

/// v5.2: 转到当前选择正视(navigateToSelection, 实测相机立即响应;
/// 动画驱动, 调用方需等待 ~700ms 让相机稳定)。
const NAV_TO_SELECTION_JS: &str = r#"(function(){
  var ed=window.__PLA_EDITOR__;
  if(!ed||!ed.selection){ var f=window.__PLA_HOOK__; if(f) ed=f.findEditor(); if(ed) window.__PLA_EDITOR__=ed; }
  if(!ed||!ed.selection) return 'no-ed';
  var vp=null; ed.viewports.forEach(function(v){if(!vp)vp=v});
  if(!vp||typeof vp.navigateToSelection!=='function') return 'no-nav';
  try{ vp.navigateToSelection(); return 'ok' }catch(e){ return 'EXC' }})()"#;

/// R8-1: 视口状态采集 — 相机全参数(position/quaternion/zoom/fov) + orbitControls
/// (target/focalOffset/radius) + shading(mode.tag/_sectionMode.tag) + 剖面数组
/// (ed.sections.sections: enabled/p/n/x/y) + 参考面(vp._constructionPlane: p/n/x/y)
/// + renderer/ao/grid 设置。返回 JSON; err 字段非空 = 失败。
const VPSTATE_SNAPSHOT_JS: &str = r#"(function(){
  var ed=window.__PLA_EDITOR__;
  if(!ed||!ed.selection){ var f=window.__PLA_HOOK__; if(f) ed=f.findEditor(); if(ed) window.__PLA_EDITOR__=ed; }
  if(!ed||!ed.selection) return JSON.stringify({err:'no-editor'});
  var vps=[]; ed.viewports.forEach(function(v){vps.push(v)});
  if(!vps.length) return JSON.stringify({err:'no-viewport'});
  var vp=vps[0];
  var out={};
  // R10: 逐段记录失败原因。以前每段一个空 catch, 26.x 里任何结构变化(如
  // vp.settings 搬走、orbitControls 改名)都会让该段**静默消失**, 恢复时对应
  // 项跳过且仍返回 ok —— 用户看到的就是"全都恢复不了"却没有任何线索。
  out.failed=[];
  function v3(v){ return v&&v.x!==undefined ? [v.x,v.y,v.z] : null }
  try{
    var c=vp.camera;
    out.cam={px:c.position.x,py:c.position.y,pz:c.position.z,
      qx:c.quaternion.x,qy:c.quaternion.y,qz:c.quaternion.z,qw:c.quaternion.w,
      fov:(typeof c.fov==='number')?c.fov:null, zoom:(typeof c.zoom==='number')?c.zoom:null,
      near:(typeof c.near==='number')?c.near:null, far:(typeof c.far==='number')?c.far:null};
  }catch(e){ out.failed.push('cam:'+(e&&e.message?e.message:'EXC')) }
  try{
    var oc=vp.orbitControls;
    // R10(实测): orbitControls 是**阻尼**的 — 每个量都有 x / xEnd 两份, 每帧把
    // x 往 xEnd 插值。只写 x 而 xEnd 还是旧值时, 相机会被动画拉回旧取景 ——
    // 这就是"恢复后视角/缩放不对"的直接原因。两份都要存、都要写。
    out.orbit={target:v3(oc.target), targetEnd:v3(oc.targetEnd),
      focal:v3(oc.focalOffset), focalEnd:v3(oc.focalOffsetEnd),
      radius:(typeof oc.radius==='number')?oc.radius:null,
      radiusEnd:(typeof oc.radiusEnd==='number')?oc.radiusEnd:null,
      zoom:(typeof oc.zoom==='number')?oc.zoom:null,
      zoomEnd:(typeof oc.zoomEnd==='number')?oc.zoomEnd:null};
  }catch(e){ out.failed.push('orbit:'+(e&&e.message?e.message:'EXC')) }
  try{ out.shading={mode: ed.shading.mode?ed.shading.mode.tag:null,
    section: ed.shading._sectionMode?ed.shading._sectionMode.tag:null} }catch(e){ out.failed.push('shading:'+(e&&e.message?e.message:'EXC')) }
  try{
    var arr=[];
    ed.sections.sections.forEach(function(s){
      arr.push({enabled:!!s.enabled, p:v3(s.p), n:v3(s.n), x:v3(s.x), y:v3(s.y)});
    });
    out.sections=arr;
  }catch(e){ out.failed.push('sections:'+(e&&e.message?e.message:'EXC')) }
  try{
    var cp=vp._constructionPlane;
    if(cp) out.cplane={p:v3(cp.p), n:v3(cp.n), x:v3(cp.x), y:v3(cp.y)};
  }catch(e){ out.failed.push('cplane:'+(e&&e.message?e.message:'EXC')) }
  try{
    var r=vp.settings.Viewport.renderer;
    out.renderer={antiAlias:!!r.antiAlias, toneMapping:String(r.toneMapping||''),
      toneMappingExposure:(typeof r.toneMappingExposure==='number')?r.toneMappingExposure:null};
  }catch(e){ out.failed.push('renderer:'+(e&&e.message?e.message:'EXC')) }
  try{
    var ao=vp.settings.Viewport.ao;
    var av={};
    Object.keys(ao).forEach(function(k){ var v=ao[k]; if(typeof v==='number'||typeof v==='boolean') av[k]=v });
    out.ao=av;
  }catch(e){ out.failed.push('ao:'+(e&&e.message?e.message:'EXC')) }
  try{
    var g=vp.settings.Grid;
    out.grid={every:(typeof g.every==='number')?g.every:null, size:(typeof g.size==='number')?g.size:null};
  }catch(e){ out.failed.push('grid:'+(e&&e.message?e.message:'EXC')) }
  // 视口 DOM rect(CSS 像素) + dpr — 截图裁剪用
  try{
    var r2=vp.container.getBoundingClientRect();
    out.vpRect=[r2.x,r2.y,r2.width,r2.height];
    out.dpr=window.devicePixelRatio||1;
  }catch(e){}
  out.err='';
  return JSON.stringify(out);})()"#;

/// R8-1: 视口状态恢复 — 与 VPSTATE_SNAPSHOT_JS 对称。相机直写(正交模式禁碰
/// 投影矩阵, v6 经验); shading 切换链 = 写 mode.tag + 构造合法 prev + dispatch
/// shadingChanged(mode) + setNeedsRender(实测, R8-1 探测验证); 剖面/参考面数值
/// 直写 + setNeedsRender。返回 'ok' 或错误描述。
const VPSTATE_RESTORE_JS_TMPL: &str = r#"(function(){
  var S=__STATE__;
  var ed=window.__PLA_EDITOR__;
  if(!ed||!ed.selection){ var f=window.__PLA_HOOK__; if(f) ed=f.findEditor(); if(ed) window.__PLA_EDITOR__=ed; }
  if(!ed||!ed.selection) return 'no-editor';
  var vps=[]; ed.viewports.forEach(function(v){vps.push(v)});
  if(!vps.length) return 'no-viewport';
  var vp=vps[0];
  var applied=[];
  // R10: 以前每段空 catch + 只回 applied 列表 —— 任何一段没生效就"无声跳过",
  // 面板显示"已恢复"但界面/视角纹丝不动。现在分段记录 missing(存档里没有)
  // 与 failed(写的时候抛了), 回包原样交回 Rust 侧如实显示。
  var missing=[];
  var failed=[];
  function ex(e){ return e&&e.message?e.message:'EXC' }
  function setV3(dst,src){ if(dst&&dst.set&&src){ try{ dst.set(src[0],src[1],src[2]); return true }catch(e){} } return false }
  try{
    if(S.cam){
      var c=vp.camera;
      c.position.set(S.cam.px,S.cam.py,S.cam.pz);
      c.quaternion.set(S.cam.qx,S.cam.qy,S.cam.qz,S.cam.qw);
      // R10: 缩放(fov/zoom)以前**根本没有恢复** —— 相机位姿对了但取景范围
      // 不对, 用户看到的就是"视角恢复不了"。只在透视相机上写: 正交
      // ProxyCamera 的 updateProjectionMatrix 有破坏性副作用(R7-31h)。
      var persp = (c.isPerspectiveCamera===true);
      var proj=false;
      if(typeof S.cam.zoom==='number' && typeof c.zoom==='number'){
        if(persp){ c.zoom=S.cam.zoom; proj=true } else { missing.push('zoom(正交未应用)') }
      }
      if(typeof S.cam.fov==='number' && typeof c.fov==='number'){
        if(persp){ c.fov=S.cam.fov; proj=true } else { missing.push('fov(正交未应用)') }
      }
      if(proj){ try{ c.updateProjectionMatrix() }catch(e1){} applied.push('proj') }
      applied.push('cam');
    } else { missing.push('cam') }
  }catch(e){ failed.push('cam:'+ex(e)) }
  try{
    if(S.orbit){
      var oc=vp.orbitControls;
      // R10(实测): x / xEnd 成对写 —— 只写当前值会被阻尼动画拉回旧取景。
      function svEnd(dst,src){ if(dst&&dst.set&&src){ try{ dst.set(src[0],src[1],src[2]); return true }catch(e){} } return false }
      setV3(oc.target,S.orbit.target);
      svEnd(oc.targetEnd,S.orbit.targetEnd||S.orbit.target);
      setV3(oc.focalOffset,S.orbit.focal);
      svEnd(oc.focalOffsetEnd,S.orbit.focalEnd||S.orbit.focal);
      if(typeof S.orbit.radius==='number'){
        oc.radius=S.orbit.radius;
        if(S.orbit.radiusEnd!==undefined&&S.orbit.radiusEnd!==null) oc.radiusEnd=S.orbit.radiusEnd; else oc.radiusEnd=S.orbit.radius;
      }
      if(typeof S.orbit.zoom==='number'){
        oc.zoom=S.orbit.zoom;
        if(S.orbit.zoomEnd!==undefined&&S.orbit.zoomEnd!==null) oc.zoomEnd=S.orbit.zoomEnd; else oc.zoomEnd=S.orbit.zoom;
      }
      // 朝向也同步给 orbit(否则用户下一次拖动会从旧朝向"跳"回来)
      if(S.cam&&oc.quaternion&&oc.quaternion.set){
        try{ oc.quaternion.set(S.cam.qx,S.cam.qy,S.cam.qz,S.cam.qw) }catch(e2){}
        try{ if(oc.quaternionEnd&&oc.quaternionEnd.set) oc.quaternionEnd.set(S.cam.qx,S.cam.qy,S.cam.qz,S.cam.qw) }catch(e3){}
      }
      try{ if(typeof oc.update==='function') oc.update() }catch(e2){}
      applied.push('orbit');
    } else { missing.push('orbit') }
  }catch(e){ failed.push('orbit:'+ex(e)) }
  try{
    if(S.shading&&S.shading.mode){
      var m=ed.shading.mode;
      if(m&&m.tag!==S.shading.mode){
        m.prev={tag:m.tag, materials:m.materials, isTinted:!!m.isTinted};
        m.tag=S.shading.mode;
        try{ ed.shading.signals.shadingChanged.dispatch(m) }catch(e2){ failed.push('shading:dispatch') }
      } else if(!m){ failed.push('shading:no-mode') }
      if(S.shading.section&&ed.shading._sectionMode){
        try{ ed.shading._sectionMode.tag=S.shading.section }catch(e2){ failed.push('shading:section') }
      }
      applied.push('shading');
    } else { missing.push('shading') }
  }catch(e){ failed.push('shading:'+ex(e)) }
  try{
    if(S.sections&&Array.isArray(S.sections)){
      var cur=ed.sections.sections;
      var curArr=Array.isArray(cur)?cur:(function(){var a=[];try{cur.forEach(function(x){a.push(x)})}catch(e9){}return a})();
      // 数量对齐: 多余的禁用, 缺的无法凭空造(如实跳过)
      for(var i=0;i<curArr.length&&i<S.sections.length;i++){
        var d=S.sections[i], s=curArr[i];
        try{ s.enabled=!!d.enabled }catch(e2){}
        setV3(s.p,d.p); setV3(s.n,d.n); setV3(s.x,d.x); setV3(s.y,d.y);
      }
      for(var j=S.sections.length;j<curArr.length;j++){ try{ curArr[j].enabled=false }catch(e3){} }
      applied.push('sections:'+Math.min(curArr.length,S.sections.length));
    } else { missing.push('sections') }
  }catch(e){ failed.push('sections:'+ex(e)) }
  try{
    if(S.cplane){
      var cp=vp._constructionPlane;
      if(cp){ setV3(cp.p,S.cplane.p); setV3(cp.n,S.cplane.n); setV3(cp.x,S.cplane.x); setV3(cp.y,S.cplane.y);
        applied.push('cplane'); } else { failed.push('cplane:null') }
    } else { missing.push('cplane') }
  }catch(e){ failed.push('cplane:'+ex(e)) }
  try{
    if(S.renderer){
      var r=vp.settings.Viewport.renderer;
      if(typeof S.renderer.antiAlias==='boolean') r.antiAlias=S.renderer.antiAlias;
      if(S.renderer.toneMapping) r.toneMapping=S.renderer.toneMapping;
      if(typeof S.renderer.toneMappingExposure==='number') r.toneMappingExposure=S.renderer.toneMappingExposure;
      applied.push('renderer');
    } else { missing.push('renderer') }
  }catch(e){ failed.push('renderer:'+ex(e)) }
  try{
    if(S.ao){
      var ao=vp.settings.Viewport.ao;
      Object.keys(S.ao).forEach(function(k){ try{ ao[k]=S.ao[k] }catch(e2){} });
      applied.push('ao');
    } else { missing.push('ao') }
  }catch(e){ failed.push('ao:'+ex(e)) }
  try{
    if(S.grid){
      var g=vp.settings.Grid;
      if(typeof S.grid.every==='number') g.every=S.grid.every;
      if(typeof S.grid.size==='number') g.size=S.grid.size;
      applied.push('grid');
    } else { missing.push('grid') }
  }catch(e){ failed.push('grid:'+ex(e)) }
  try{ vp.setNeedsRender() }catch(e){}
  return JSON.stringify({applied:applied, missing:missing, failed:failed});})()"#;


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

/// R10: 把 VPSTATE_RESTORE_JS_TMPL 的回包翻成人话。**不再谎报成功**: 只要
/// failed 非空就明确说"部分失败", missing(存档里本来就没有)单独列出。
///
/// 回包形状: `{"applied":[..],"missing":[..],"failed":[..]}`;
/// 老格式(`ok:cam,orbit`)或任何解析失败都原样透出, 保证不丢信息。
///
/// @param raw CDP 回包字符串
/// @return 给面板显示的中文摘要
fn fmt_vpstate_apply_result(raw: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return format!("视口状态已恢复: {raw}");
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

    let mut msg = if applied.is_empty() && failed.is_empty() {
        "视口状态恢复: 没有任何项被应用".to_string()
    } else {
        format!("视口状态已恢复: {}", applied.join("/"))
    };
    if !failed.is_empty() {
        msg.push_str(&format!(" | 部分失败: {}", failed.join(", ")));
    }
    if !missing.is_empty() {
        msg.push_str(&format!(" | 存档未含: {}", missing.join(", ")));
    }
    msg
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
fn view_delta(target_json: &str, actual_json: &str) -> anyhow::Result<f64> {
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
        let js = apply_sel_js(&entries, false).unwrap();
        eprintln!("===GENERATED-JS-BEGIN===");
        eprintln!("{js}");
        eprintln!("===GENERATED-JS-END===");
        // 基本健全性: 不得含 // 注释痕迹, 必须完整收尾
        assert!(!js.contains("//"), "JS 内含 // 注释会吞掉后续代码");
        assert!(js.ends_with("})()"), "JS 未完整收尾");
        // v5: Phase A 关键步骤存在
        assert!(js.contains("cancelActiveCommand"), "缺少取消活动命令");
        // R8: keep_sel 分支 — 首组有 clear, 后续组没有
        let js_keep = apply_sel_js(&entries, true).unwrap();
        assert!(!js_keep.contains("click.clear"), "keep_sel 组不应清选择");
        assert!(js.contains("click.clear"), "首组应清选择");
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
        let s = fmt_vpstate_apply_result(ok);
        assert!(s.contains("cam"), "{s}");
        assert!(!s.contains("失败"), "{s}");
        // 部分失败必须如实带出来(旧实现会谎报"已恢复")
        let bad = r#"{"applied":["cam"],"missing":["grid"],"failed":["shading:cannot read"]}"#;
        let s = fmt_vpstate_apply_result(bad);
        assert!(s.contains("shading:cannot read"), "{s}");
        assert!(s.contains("存档未含"), "{s}");
        // 什么都没应用
        let none = r#"{"applied":[],"missing":["cam"],"failed":[]}"#;
        assert!(fmt_vpstate_apply_result(none).contains("没有任何项被应用"));
        // 老格式 / 坏数据原样透出, 不丢信息
        assert_eq!(fmt_vpstate_apply_result("ok:cam,orbit"), "视口状态已恢复: ok:cam,orbit");
    }

    #[test]
    fn view_apply_template_is_well_formed() {
        let js = PLA_VIEW_APPLY_JS_TMPL.replace(
            "__VIEW__",
            r#"{"px":1,"py":2,"pz":3,"qx":0,"qy":0,"qz":0,"qw":1}"#,
        );
        assert!(!js.contains("__VIEW__"), "占位符未替换");
        assert!(!js.contains("//"), "JS 内含 // 注释会吞掉后续代码");
        assert!(js.ends_with("})()"), "JS 未完整收尾");
        assert!(js.contains("orbitControls"), "缺少 orbitControls 同步(会被下一帧拉回)");
        assert!(js.contains("setNeedsRender"));
    }
}
