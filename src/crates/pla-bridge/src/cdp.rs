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
    /// 应用选择集: 把数值 id 列表 addById 进当前选择。
    ApplyIds(Vec<u32>),
    /// 立即拉一次实体树。
    RefreshTree,
    /// 通用 evaluate(R7-16): 尺寸驱动命令派发等自动化尝试。
    Eval(String),
    Shutdown,
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
#[derive(Debug, Clone)]
pub struct SelItem {
    pub kind: &'static str,
    pub id: String,
    pub name: String,
    pub ctor: String,
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
  function idset(n){ try{ var v=s[n]; if(v&&typeof v.forEach==='function'){ var o=[]; v.forEach(function(x){ o.push(String(x)) }); return o } return [] }catch(e){ return [] } }
  var all = [];
  try { (s.all||[]).forEach(function(x){ if(x&&typeof x==='object') all.push({
      ctor: (x.constructor&&x.constructor.name)||'',
      id: String(x.entityId!==undefined? x.entityId : (x.id===undefined? '' : x.id)),
      name: String(x.name||''),
      parent: (x.parent&&x.parent.constructor)? x.parent.constructor.name : ''
  }) }) } catch(e){}
  var undo=-1, geoVer=-1;
  try{ undo = ed.history._undoStack.length }catch(e){}
  try{ geoVer = ed.geo._version }catch(e){}
  return JSON.stringify({ st:'ok', size: (typeof s.size==='number')? s.size : -1,
    undo: undo, geoVer: geoVer,
    ids: idset('ids'),
    solidIds: idset('solidIds'), sheetIds: idset('sheetIds'),
    faceIds: idset('faceIds'), edgeIds: idset('edgeIds'),
    segmentIds: idset('segmentIds'), curveIds: idset('curveIds'),
    curveVertexIds: idset('curveVertexIds'), groupIds: idset('groupIds'),
    planeIds: idset('planeIds'), all: all });
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
            // 连接阶段: 反复重试(应用未启动/未打补丁时静默等待)
            match connect_page(port).await {
                Ok(mut ws) => {
                    let _ = ev_tx.send(CdpEvent::Status("CDP 已连接".into()));
                    if let Err(e) = session_loop(&mut ws, &cmd_rx, &ev_tx).await {
                        let _ = ev_tx.send(CdpEvent::Status(format!("CDP 断开: {e}")));
                    }
                }
                Err(e) => {
                    let _ = ev_tx.send(CdpEvent::Status(format!("CDP 等待中({e})")));
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
            Ok(CdpCmd::ApplyIds(ids)) => {
                let js = format!(
                    "(function(){{ var ed=window.__PLA_EDITOR__; if(!ed||!ed.selection) return 'no-editor'; var s=ed.selection.selected; var out=[]; var ids={}; for(var i=0;i<ids.length;i++){{ try{{ s.addById(ids[i]); out.push(ids[i]) }}catch(e){{}} }} return JSON.stringify(out) }})()",
                    serde_json::to_string(&ids)?
                );
                let _ = eval_roundtrip(ws, &mut mid, &js).await?;
                let _ = ev_tx.send(CdpEvent::Status("选择集已应用".into()));
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
                    // 每类型 id 集(Set of hex id 字符串) → SelItem
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
                                items.push(SelItem {
                                    kind,
                                    id: it.as_str().unwrap_or("").to_string(),
                                    name: String::new(),
                                    ctor: kind.to_string(),
                                });
                            }
                        }
                    }
                    // all: 实体对象(含 entityId/name/parent ctor) — 补充名称
                    if let Some(arr) = v["all"].as_array() {
                        for it in arr {
                            items.push(SelItem {
                                kind: "item",
                                id: it["id"].as_str().unwrap_or("").to_string(),
                                name: it["name"].as_str().unwrap_or("").to_string(),
                                ctor: it["ctor"].as_str().unwrap_or("").to_string(),
                            });
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
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await?;
    let text = String::from_utf8_lossy(&buf);
    Ok(text.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
}
