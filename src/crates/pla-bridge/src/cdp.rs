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
    /// 立即拉一次实体树。
    RefreshTree,
    /// 通用 evaluate(R7-16): 尺寸驱动命令派发等自动化尝试。
    Eval(String),
    Shutdown,
}

/// 一条待回放的选择成员。
#[derive(Debug, Clone)]
pub struct ApplyEntry {
    pub kind: String,
    pub id: String,
    /// 保存时采集的原始 id 元素 JSON(对象形); None = 纯数值 id。
    pub key: Option<String>,
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
  return JSON.stringify({ st:'ok', size: (typeof s.size==='number')? s.size : -1,
    undo: undo, geoVer: geoVer,
    ids: idset('ids'),
    solidIds: idset('solidIds'), sheetIds: idset('sheetIds'),
    faceIds: idset('faceIds'), edgeIds: idset('edgeIds'),
    segmentIds: idset('segmentIds'), curveIds: idset('curveIds'),
    curveVertexIds: idset('curveVertexIds'), groupIds: idset('groupIds'),
    planeIds: idset('planeIds') });
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
                // R7-21c: 回放策略(全部活体验证, 2026-10-20 22:4x 会话) —
                //  solid/sheet 数值 id: harvest 法 — SelectAll<Kind>Command 暴露全部
                //    Object3D, 与 idSet 顺序配对成 id→Object3D 映射, s.clear() 后先还原
                //    用户原选择(s.add(对象引用)), 再 s.add(目标)。数字直加(addById/
                //    addSolid(数字))实测产生 NaN 幽灵条目, 禁用。
                //  face/edge/segment/curve/curveVertex: GeometryDatabase.
                //    lookupTopologyItemById(hex) 返回 {view, model, topologyId},
                //    ref.view 即 renderer Face/ShellEdge Object3D proxy,
                //    s.addFace(ref.view)/addEdge(ref.view) 实测精确复原
                //    (face '9f1624' + edge '14e1984' 双双命中, 无幽灵)。
                //    注意 addFace(裸 {id,versionId} 或 ref 本体) 会抛
                //    "undefined reading versionId" — 必须传 ref.view。
                //  对象形 key 兜底: lookup 失败或无 db 时原样回放, 失败计数如实上报。
                // (JS 内不得写 // 注释: 该模板用反斜杠续行, 生成的是一整行 JS,
                //  行注释会把后面的代码全部吞掉 → SyntaxError: Unexpected end of input。
                //  说明一律写在 Rust 侧, 如上。)
                let js = format!(
                    "(function(){{ var ed=window.__PLA_EDITOR__; if(!ed||!ed.selection) return JSON.stringify({{ok:0,fail:0,errs:['no-editor']}}); var s=ed.selection.selected; var ents={ents}; var ok=0, fail=0, errs=[];\
 function err(m){{ if(errs.length<4) errs.push(String(m).slice(0,60)) }}\
 var TAGS={{edge:0,face:2,solid:4,sheet:5}};\
 var db=null; try{{ db=s.cache().db }}catch(eD){{}}\
 var memIds=[], unknown=[], groups={{}};\
 function parentOf(id){{ try{{ var v=db.lookupTopologyItemById(String(id)).view; var fg=v&&v.parent; var solid=fg&&fg.parent;\
   return (solid&&solid.versionId!==undefined)?String(solid.versionId):null }}catch(e){{ return null }} }}\
 ents.forEach(function(e){{\
  var t=TAGS[e.kind];\
  if(t===undefined){{ unknown.push(e); return }}\
  var memId=t+':'+e.id; memIds.push(memId);\
  if(e.kind==='face'||e.kind==='edge'){{ var p=parentOf(e.id); if(p){{ if(!groups[p]) groups[p]=[]; groups[p].push(memId) }} }}\
 }});\
 try{{\
  var m=s.saveToMemento();\
  m.ids=new Set(memIds);\
  var pc=m.parentsWithSelectedChildren;\
  if(pc){{ pc.counts=new Map(); pc.values=new Map();\
   Object.keys(groups).forEach(function(p){{ pc.counts.set(p, groups[p].length); pc.values.set(p, groups[p]) }}) }}\
  s.clear();\
  s.restoreFromMemento(m);\
  var got=0;\
  ents.forEach(function(e){{\
   var v=null;\
   try{{ v=(e.kind==='face')?s.faceIds:(e.kind==='edge')?s.edgeIds:(e.kind==='solid')?s.solidIds:(e.kind==='sheet')?s.sheetIds:null }}catch(eV){{}}\
   if(!v) return;\
   var has=false; v.forEach(function(x){{ if(String(x&&x.versionId!==undefined?x.versionId:x)===String(e.id)) has=true }});\
   if(has) got++; else err('id '+e.id+' 未命中(几何已变更?)');\
  }});\
  ok=got; fail=ents.length-got;\
 }}catch(eM){{ err('memento '+String(eM).slice(0,50)); fail=ents.length }}\
 unknown.forEach(function(e){{ err('kind '+e.kind+' 暂无 tag, 未恢复') }});\
 return JSON.stringify({{ok:ok,fail:fail,errs:errs}}) }})()",
                    ents = serde_json::to_string(&
                        entries
                            .iter()
                            .map(|e| serde_json::json!({
                                "kind": e.kind,
                                "id": e.id,
                                "key": e.key,
                            }))
                            .collect::<Vec<_>>()
                    )?
                );
                let res = eval_roundtrip(ws, &mut mid, &js).await.unwrap_or_default();
                let v: Value = serde_json::from_str(&res).unwrap_or(Value::Null);
                let ok = v["ok"].as_i64().unwrap_or(0);
                let fail = v["fail"].as_i64().unwrap_or(0);
                let errs = v["errs"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" | "))
                    .unwrap_or_default();
                let _ = ev_tx.send(CdpEvent::Status(if fail == 0 {
                    format!("选择集已应用: {ok} 项成功")
                } else {
                    format!("选择集应用: {ok} 项成功, {fail} 项失败 {errs}")
                }));
            }
            Err(TryRecvError::Disconnected) => return Err(anyhow::anyhow!("ui gone")),
            Err(_) => {}
        }

        // --- 轮询选择快照(每 2 个 tick ≈ 1s; 500ms 底噪) ---
        sel_tick += 1;
        if sel_tick % 2 == 0 {
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
                    }));
                } else {
                    let _ = ev_tx.send(CdpEvent::Selection(SelectionSnapshot {
                        ok: false,
                        state: v["st"].as_str().unwrap_or("unknown").to_string(),
                        size: -1,
                        items: Vec::new(),
                        undo_len: -1,
                        geo_ver: -1,
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

async fn ws_send(ws: &mut WsStream, v: Value) -> anyhow::Result<()> {
    use futures_util::SinkExt;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(v.to_string()))
        .await
        .map_err(|e| anyhow::anyhow!("ws send: {e}"))
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
