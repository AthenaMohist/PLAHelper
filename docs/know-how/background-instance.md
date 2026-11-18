# 后台 Plasticity 实例(2026-11-18 实测, Plasticity 26.2.3 / Electron 42)

用户需求: 人在前台用浏览器或另一个 Plasticity 做别的项目, helper + MCP 在后台起一个**独立**的 Plasticity 做独立任务, 互不打扰 ——
后台这个不能闪现、不能抢鼠标 / 焦点。实现: `tools/pla-cdp/pla-bg.js` + `pla-bg-hook.js`, pla-mcp `background` 工具 + 所有 `model_*` 的 `instance: "background"`。

## 为什么"真 headless"不行, 为什么这样行

- Plasticity 的编辑器 / 命令 / 撤销都在渲染进程(网页)里, 内核也经它调 → 必须有一个活着的页面; Electron 不支持图形应用 `--headless`, 改 app.asar 每次升级都失效。
- 页面"隐藏"的真正原因: 窗口被别的窗口盖住时 Windows 遮挡检测把页面标成 hidden, rAF 停到 0 帧(memento 恢复、截图都依赖它)。
  `webContents.setBackgroundThrottling(false)`(运行时可切)后被盖住照样 121 帧; **从没显示过的窗口 + 不节流 → 页面 visibilityState 就是 visible**,
  垫片的可见性保护照常放行。
- 合成事件(拖放导入资产、指针移动)全在页面里派发, 从不动真鼠标。

## 独立实例怎么起来(pla-bg.js start)

1. 单独的 userData: `%LOCALAPPDATA%\PLAhelper\bg-plasticity\userData`(`--user-data-dir`; Electron 的单实例锁按 userData 算, 不会转给前台那个)。
   首次从 `%APPDATA%\Plasticity` 拷 `Local Storage / Preferences / Local State`(登录 + 设置; 实测日志 "License validity: true")。
2. `--inspect-brk=127.0.0.1:9344`: **Electron 不会真停在第一行**, 只是等调试器(实测); 等的时候连 `process` 都还没有。
   → `Debugger.setInstrumentationBreakpoint beforeScriptExecution` → `Runtime.runIfWaitingForDebugger` → 跳过启动脚本, 停在第一个 `app.asar` 脚本前
   (Node 已就绪、Plasticity 的代码还没跑)→ `Runtime.evaluate` 注入 pla-bg-hook.js → 撤断点 → 继续。
3. 钩子钩 `Module._load`: 第一次 `require('electron')` 时换成打过补丁的版本 ——
   - userData 不是后台目录 → 立刻 `process.exit(3)`(在 Plasticity 申请单实例锁之前, 不会碰到前台实例);
   - `BrowserWindow` → 子类: `show:false`、`skipTaskbar`、`focusable:false`、`backgroundThrottling:false`; show / showInactive / focus / maximize / restore /
     moveTop / setFullScreen / flashFrame / setAlwaysOnTop / center 一律吞掉(计数 + 日志); 静态查找(getAllWindows / fromId / fromWebContents /
     getFocusedWindow)改用 BaseWindow 全集(继承来的只认原类实例, 回空 → 代理找不到窗口);
   - 原生对话框不弹: 另存为用 `__PLA_BG__.savePath`(另外 PLAhelper 的一次性应答照样能包在外面), 打开用 `openPaths`, 消息框回 cancelId / 0, 错误框只记;
     `shell.openExternal / openPath` 不执行。日志在主进程 `globalThis.__PLA_BG__`(`pla-bg.js status` 能看)。
4. 拷来的设置会让它**打开用户上次的文档** → 加载完立刻在视口元素上派发 `file:new`(同垫片 cmd.dispatch)→ "Untitled"。给了 `--doc` 就开那个。
5. 起 CDP 代理 9343(`pla-cdp-v8.js proxy`, 端口用环境变量 `PLA_INSPECT_PORT=9344` / `PLA_PROXY_PORT=9343`)。约定: inspector = 代理端口 + 1
   (`pla_bridge::export::inspect_port_for`), 导出 STEP / pack 的另存为应答就打到对的实例。

`stop` = 主进程 `app.exit(0)`(不问未保存)+ 杀代理。

## 实测(13:00–13:10, 前台 model1 一直开着, 每次都开 100 ms 一次的可见窗口监视)

- 启动 4 s, 停 2 s; 全程**没有任何新可见窗口**, 前台焦点没变。
- `model_part`(第一次 13 s, 含挂工厂 7 s)、`model_asset_library install`、`model_asset_place`(拖放导入, 支承面正好落点)、`model_view` 截图(离屏照常渲染)都正常。
- 后台建库 GB/T 97.1 全 14 规格 31 s(导出另存为被应答, 没弹任何对话框)。

## 坑

- 从 pla-mcp 起 node → node 起的常驻 Plasticity / 代理会**继承句柄**: 用 `output()` 等管道关闭永远等不到; MCP 客户端的 stdout 管道也会被带进去
  → pla-mcp 先把自己的 stdin/stdout/stderr 设成不可继承, node 的输出写临时文件、只等 node 退出。
- 建库(build_pack)直接调 cmd.exec, 新实例要先挂工厂(`exec::ensure_factories`), 否则 need-factories。
- 调试时第一次没钩上(以为 --inspect-brk 会停)→ 窗口正常创建、可能在前台闪过几秒; 之后的版本都先钩再放行。
