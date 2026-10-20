# Plasticity Renderer 内部 API 实锤（CDP 破解路线, R7-13）

## 破解链（全部已验证）
1. 应用主进程启动时主动 `removeSwitch('remote-debugging-port'/'remote-debugging-pipe')`（反调试）;
   但主进程入口 `.webpack/main/index.js` 是**明文**、fuses 未校验 → 可注入。
2. 注入补丁（v6, 已部署于 E:/.../app-26.1.3/resources/app/.webpack/main/index.js，
   备份在同目录 index.js.pla-backup）:
   - 中和 removeSwitch(两个调试开关)；
   - 无外部调试开关时自动 `appendSwitch('remote-debugging-port', 9333)`；
   - env `PLA_CDP_INSTANCE=1` 时猴补 `requestSingleInstanceLock=()=>true` +
     setPath('userData') → **多实例**（绕过单例锁）;
   - env `PLA_CDP_PROFILE` 自定义 profile 目录;
   - exit/uncaughtException/unhandledRejection 全量落日志到 C:/Users/Frank/.pla-cdp-patch.log。
3. **环境陷阱**：WorkBuddy shell 带 `ELECTRON_RUN_AS_NODE=1`，Electron 被退化成纯 Node
   （秒退/"bad option"）——启动前必须 unset。Hermes 当年失败大概率栽在这+开关剥离组合。
4. CDP 上线后：GET /json/list → page target（title "model1.plasticity - Plasticity"）→
   webSocketDebuggerUrl → Runtime.evaluate。
5. 应用自带 argv 白名单（未知 CLI 参数报 "bad option" 直接退出）——给应用传参别用未知开关。

## Renderer 内部 API（Runtime.evaluate 可达, 全部实测）
- window 全局: `__grim__`(事件总线, 但 handlers 表为 null 的非活动实例)、`__THREE__`、
  `process`（无 require）。
- UI 框架: **Preact**（DOM 节点挂 `__k`/`l` 钥匙）。UI 是真实 DOM（481 DIV）+ three.js 视口
  (`<plasticity-viewport>` 自定义元素)。
- **入口: `document.getElementById('root').__k`** → BFS vnode 树（`v.__k` 子代,
  函数组件 `v.type.name`, 组件实例 `v.__c`）。
- 关键组件: `App` → `LeftSidebar` → `Outliner(props.model)` →
  **`OutlinerModel{editor, selectorState}`** → **`editor`（上帝对象）**。
- `editor` 字段(节选): selection, geo(GeometryDatabase), document, scene, objects
  (path2object Map), curves, nodes, groups, highlights, query, commands(全部造型命令
  按 name 索引, 如 CenterBoxCommand), signals, db, history, websocket, serialize。
- **`editor.selection` = SelectionDatabase**:
  - `.selected` = Selection 对象，getter: `size/filter/find/all/itemIds/solidIds/
    sheetIds/faceIds/edgeIds/segmentIds/curveIds/curveVertexIds/curveCvIds/groupIds/
    planeIds/measurementIds` 与实体集合 `items/solids/sheets/faces/edges/segments/
    curves/curveVertices/groups/planes`；
  - 变更: `add/remove/addById/removeById/hasId/copy/delta`；
  - **`signals` + `selectedSignals`** = 实时变更订阅点（推模式）。
- 文档模型: OutlinerSection `props.object` = VirtualGroup/Groups_Group
  （字段: id/curves/solids/sheets/images/meshes/instances/measurements/planes）——
  **bridge 不下发的 curves/instances 在这里全有**。
- 命令系统: `editor.commands.<Name>Command` 全部造型/编辑命令可编程调用。

## 对 PLAHelper 的意义
选择集"实时读取窗口中选中的所有东西（体/面/边/点/curve）"在 renderer 内部 API
完全可行：`editor.selection.selected` + signals 订阅。pla-ui 集成方案：
拉起 Plasticity 时注入补丁（或引导用户以补丁版启动）→ CDP 常连 →
注入常驻 JS 钩子把 selection 变更推回本地通道。

## REA 状态
rea-agents 6.7.0 已装（managed node 全局）。`capture-electron-scenario` 与应用的
启动序列互斥（应用在 ready 前主动 quit，判定点在 jsc 字节码内），暂不可用；
其 JS 静态分析对 bytenode .jsc 无效（字节码非 JS 源）。CDP 补丁路线已覆盖需求。
