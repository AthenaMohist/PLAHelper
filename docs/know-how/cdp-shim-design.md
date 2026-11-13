# CDP 垫片层 `window.__PLA__` 设计（F1–F4，2026-11-13 定稿）

## 0. 目的与方法论（用户确认）

垫片层的存在就是为了让整个架构**保持灵活、可扩展**：
1. 覆盖路线图上的所有功能（选择集、视口状态、步进/滚转、材质、命令派发、打孔、常用件、AI 建模……）；
2. 吸收 Plasticity 本体的版本迭代——升级时只改垫片，最好只改一张表。

因此有四条硬规则，评审代码时逐条对照：

| # | 规则 | 落在哪里 |
|---|---|---|
| R1 | **Plasticity 内部结构只出现在一张表里**（带备选路径）。功能代码一律 `get('键')`，禁止直接写 `ed.xxx._yyy`。 | `pla-shim.js` 的 `LOC` / `NAMES` |
| R2 | **能力 = 表键的组合**。探测由 `CAPS` 自动生成，不手写探测代码；失效能力调用时返回 `cap:<名>` 错误，绝不静默。 | `CAPS` + `call()` |
| R3 | **接口名单一来源**：对外只有 `API` 表里的名字；Rust 侧 `shim::api::*` 常量与之一一对应，契约测试保证两边同步（缺一个就测试失败）。 | `def()` / `shim.rs` / `shim-contract.test.js` |
| R4 | **常量不复写**：全局名、编辑器特征、DOM 选择器、命令名由 Rust 配置注入垫片；pla-ui 等其他 crate 引用 `pla_bridge::shim` 的常量，不再各写一份。传参一律 JSON 序列化，不再字符串替换拼代码。 | `shim::config_json()` |

新功能 = 在 `LOC` 加键 → 在 `CAPS` 声明能力 → 用 `def()` 加接口 → Rust 加常量。**不允许**再在 Rust 里写一段临时 JS。

## 1. 文件与注入

- 源文件：`src/crates/pla-bridge/js/pla-shim.js`，整个文件是一个函数表达式 `(function (CFG) {...})`。
- Rust：`pla_bridge::shim`，`include_str!` 编入；`install_expr()` = `<文件>(<配置 JSON>)`。
- 版本：
  - `api`（整数，JS 与 Rust 各一份常量，契约测试强制相等）= 接口契约版本，只在破坏性改动时 +1；
  - `rev` = 构建时对 JS 文件内容算的 FNV-1a 哈希。注入时页面里 `rev` 不同就 `dispose()` 旧实例再装新的（开发期热替换，不重复挂监听）。
  - 探活：`api` 不一致才判定为过期（避免不同构建的 pla-ui / pla-app 互相反复重注）。
- Plasticity 版本号：F4 起由主进程侧 `app.getVersion()` 注入配置，写进 `core.probe` 结果。

## 2. 调用约定

- Rust 只发 `__PLA__.call(名, 参数JSON)` 或 `__PLA__.batch([[名, 参数], …])`。
- 回包统一：`{ok:true, v}` / `{ok:false, err:'cap:<能力>'|'exc:<消息>'|'unknown-api:<名>', missing:[表键…]}`。
- `batch` 每项独立成败：一项能力失效只影响那一项（例如 undo 栈改名不会拖垮选择读取）。

## 3. 编辑器获取

- `FINDERS` 依次尝试：配置给的全局名（代理堆快照定位后写入）→ Preact 树里的 `Outliner` 组件（26.1.3 路线，26.2.3 待实测）。
- `isEditor()` 用配置里的编辑器特征（`selection/shading/viewports`）校验——与代理堆快照搜索用的是**同一份特征**（F4 起代理从 Rust 常量取）。
- 兼容期保留 `window.__PLA_HOOK__.findEditor` 别名指向垫片，旧片段在 F1b 迁移完之前也走同一个获取入口。

## 4. 接口清单（✓ = 已实现）

| 命名空间 | 接口 | 能力 | 状态 |
|---|---|---|---|
| core | `core.probe` `core.health` `core.apis` | — | ✓ F1a |
| sel | `sel.snapshot` | sel.read | ✓ F1a |
| hist | `hist.state` | hist.read | ✓ F1a |
| cam | `cam.get` | cam.read | ✓ F1a |
| err | `err.last`（安装时挂监听） | err.hook | ✓ F1a |
| tree | `tree.list` | tree | ✓ F1a |
| sel | `sel.faceIds` `sel.clear` `sel.addById` `sel.cancelCommand` | sel.write | F1b |
| scan | `scan.begin` `scan.read` `scan.clear` `scan.end`（还原 onHover） | scan | F1b |
| cam | `cam.set` | cam.write | ✓ R20（cdp.rs 相机读写已全部改走垫片） |
| cmd | `cmd.dispatch` `cmd.ping`（自检: 注册→派发→回调） | cmd.dispatch | ✓ R20 |
| cam | `cam.navigateToSelection` `cam.roll` `cam.setOrbitStep` `cam.vpRect` `cam.render` | cam.nav / cam.step / cam.roll | F1b |
| vp | `vp.save` `vp.restore` | vp.state | F1b |
| cmd | `cmd.setArrowBindings` | keymap | F1b |
| 预留 | `mat.set`(N1) `cmd.exec`(N2) `bool.run`(N4) `asset.place`(N5) | 探测显示 todo | N 节 |

## 4b. 能力 → 功能 → 界面（F2）

- 能力名：JS `CAPS` ↔ Rust `shim::cap`（契约测试对齐）。
- 功能 → 能力：pla-ui `features.rs` 唯一一张表；`availability()` 给出 可用 / 降级（缺可选结构）/ 不可用（原因）。
- 界面全部从这张表推导：动作入口 `require_feature()` 拦截并提示、CDP 能力面板（单击工具栏状态点，按 `Group` 分组）、工具栏外环。
  **禁止**在界面代码里另写 "CDP 连上没有 / 某字段在不在" 的判断。
- 新功能上线 = 垫片加能力 → `shim::cap` 加常量 → `features.rs` 加一行 → 动作入口调 `require_feature()`。

## 5. 落地顺序

R16 F1a（框架 + 只读接口）→ R17 F2（Caps 事件 + 界面置灰 + 能力卡片）→ R18 F3（`pla-app cdp-selftest`）→
R19 F1b（迁移写路径，删除旧常量）→ R20 F4 + F6（Rust 实现 inspector 通道、renderer 重载自动重定位、流式堆快照解析；Node 代理保留为后备）。

## 6. 实测结论与待实测

- ✅（R16）Preact `Outliner` 遍历在 26.2.3 **可用**：`pla-app cdp-probe` 显示 `finder: "preact"`，全部能力可用。
  → 重载后重注垫片即可自取 editor，堆快照降为后备。
- ✅（R16）`vp.settings.Viewport.renderer/ao`、`vp.settings.Grid` 在 26.2.3 不存在（探测列为 optionalMissing），F1b 找新路径。
- 代理目前不转发 CDP 事件、假 target id 固定 → F4 必须在主进程侧感知重载。
- 现有逻辑只用第一个视口；视口选择策略集中在 `viewport()` 一处，多视口时只改这里。
