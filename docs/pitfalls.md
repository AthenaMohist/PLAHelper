# 踩过的坑（全部实锤）

| # | 坑 | 根因 | 解法 |
|---|---|---|---|
| 1 | entities 只出 1 个实体，且两次跑实体不同 | 码20 只回顶层; 码22 把全部实体打包在单容器 payload, 顶层只解析出 1 条 | 码22 + 嵌套扫描(tag=6+slen+name+parent=ff)平铺展开 |
| 2 | 曲线组(Circle/Polygon/Curve6-10/Spiral)消失 | 码22 单帧上限≈368040B, 完整树超限被**静默截断**(声明367944 实收367848) | 解析层 fail-loud 报 truncated; 根治需分块查询(待办) |
| 3 | GUI 全部文字方块 | egui 默认字体无 CJK 字形 | 启动注入系统 TTF + 内嵌 simhei 兜底 |
| 4 | 双击 exe 无反应 | **msyh.ttc = TrueType Collection, ab_glyph 不支持 ttc → 渲染 panic**; 无控制台看不到 | 候选字体只用单文件 TTF(simhei/Deng); ttc 禁入清单已写注释 |
| 5 | 顶栏按钮重复两次 | clicked 与 secondary_clicked 各构建了一次 button | 一次构建, resp.clicked()/secondary_clicked() 分流 |
| 6 | mock 测试误报嵌套实体 | 嵌套扫描对平铺小记录的 pad 字节误判名字 | 4KB 阈值: 只对容器级大记录扫描 |
| 7 | eframe 0.29 API | run_native 回调要返回 Result; ui.columns 闭包参数是 &mut [Ui] | 用 SidePanel::show_inside 替代 columns |
| 8 | BridgeClient::connect 是同步 fn | 包 tokio::time::timeout 报 not-a-future | connect 直接调用, timeout 只包 async 的 list_visible |
| 9 | bin 里 crate:: 找不到 lib 模块 | bin 的 crate root 是 main.rs 自己 | 用 pla_ui::(lib 路径)引用 |
| 10 | 面板圆角锯齿 | `SetWindowRgn` 是 1 位掩码, 无抗锯齿 | Win11 DWM 系统圆角 `DWMWA_WINDOW_CORNER_PREFERENCE=ROUND` + 关系统边框(R13) |
| 11 | 切换二级面板闪 Win98 标题栏 | 窗口带 Rgn 后 DWM 不合成非客户区, WM_NCACTIVATE 时 DefWindowProc 直接画经典标题栏; UI 线程卡 >5s 时系统还会换成带标题栏的"幽灵窗口" | 去掉 Rgn; 消除 UI 线程长阻塞(见 #14) |
| 12 | 子视口(二级面板)四角露黑 | glow 后端下**只有根视口**透明生效, immediate 子视口四角是不透明 #000000(R5 起被 Rgn 掩盖) | 圆角交给 DWM 裁剪, 不依赖窗口透明 |
| 13 | 新面板先白底、错位闪现 ~1s | immediate 子视口建好就显示, 下一次重绘前是未绘制白底 | 先 `with_visible(false)` 渲染 2 帧(定位+圆角)再显示。**egui 0.29 坑**: `ViewportBuilder::patch` 判断 visible 变化时误与 `self.active` 比较, `with_active(true)` 的窗口永远收不到 Visible(true) → 必须在子视口内显式 `send_viewport_cmd(Visible(true))`, 且显示后 builder 不再带 with_visible |
| 15 | 最大化时工具栏不在顶栏垂直居中(偏低 11px) | `PrintWindow` 位图原点是**窗口矩形**(最大化时为 -11,-11), 不是客户区; R12 从位图 (0,0) 裁条带又加了客户区偏移 = 双重补偿 | 位图按窗口矩形尺寸分配, 条带从 (客户区原点 - 窗口原点) 处裁(R13) |
| 16 | 每个二级面板在任务栏都有一个"未知程序"按钮 | winit 给所有无 owner 窗口加 `WS_EX_APPWINDOW`(强制上任务栏, 被拥有窗口也一样), 且每次窗口标志变化都整体重写 GWL_EXSTYLE(`WS_EX_TOOLWINDOW` 会被冲掉); `with_taskbar(false)` 只在创建时 DeleteTab, 隐藏创建后再显示会失效 | 隐藏阶段设 owner(`GWLP_HWNDPARENT` = 工具栏) + 清 `WS_EX_APPWINDOW`; winit 先 ShowWindow 后重写样式, 系统在首次显示时决定建不建按钮(R13) |
| 17 | keymap 加的方向键绑定完全无效 | 写在 `'orbit-controls'` 选择器下: 它是轨道控制器自己按名字读的**虚拟选择器**(鼠标手势用), 页面里没有这个元素, 键盘匹配永远落空 | 键盘绑定必须用真实 DOM 选择器: `plasticity-viewport`(`orbit:rotate:*` 监听器挂在这里; 键盘管理器把按键目标设为最后指针所在的 CANVAS)(R14) |
| 18 | 改 settings.json 的 rotateDiscreteIncrement 不生效 | Plasticity 不重载 settings.json; 值被缓存在各视口 `orbitControls.rotateDiscreteIncrement`, 改 `ed.settings`/调 `onSettingsChanged` 都无效 | CDP 直接改各视口 `orbitControls.rotateDiscreteIncrement`(并同步 `ed.settings`, 防 Plasticity 写回旧值)(R14) |
| 19 | 自动化测试里方向键送不到 Plasticity | 为抢前台点了一下 Alt: Electron 窗口进入菜单模式, 后续方向键被菜单吃掉 | 抢前台用 Shift 等不触发菜单的键; 或不注入按键, 直接对 `<plasticity-viewport>` 派发命令事件 |
| 20 | CDP 期间 Plasticity 崩溃 | `pla-cdp-v8.js` 的"已暴露 editor"快路径判断 `pre === true` 永不成立(返回的是 CDP 结果对象) → **每个新会话都拍一次 ~160MB 堆快照**; 并发会话还会写坏同一快照文件; 每个会话还 detach/attach 共享调试器, 把其他会话踢掉 | 快路径改判 `pre.result.value`; 定位单飞(串行); 调试器只 attach 一次(R14) |
| 21 | 经 CDP 注册的自定义命令不生效 | Plasticity 的 `CommandRegistry.add` 只有对象形式 `add(target, {命令名: 回调})`(arity 2), Atom 的三参 `add(target, name, fn)` 静默无效 | 用对象形式; 幂等判断查 `inlineListenersByCommandName[名].has(el)`, 不靠自设标记(R14) |
| 22 | 关掉 `pla-cdp-v8.js launch` 的终端, Plasticity 跟着被杀(未保存的工作丢失) | Node(libuv)把非 detached 子进程放进"父进程退出即杀"的作业对象 | launch 改 `detached: true`; 默认改为快捷方式直接带参数启动 Plasticity, 代理是独立进程, 挂了不影响 Plasticity(R14) |
| 23 | 停用"CDP 模式启动"会删掉 .plasticity 的用户级关联 | Plasticity 安装器本身就在 HKCU 写了 `plasticity_auto_file\shell\open\command`, 不是我们新建的 | 启用时把原命令备份到 `PlaHelperPrevCommand`, 停用时写回; 只有整键是我们新建(`PlaHelperCreated`)才整键删除(R14) |
| 24 | 二级面板输入框白底、文字悬停变黑、很多字看不清 | egui 0.29 `set_style` 只改"当前主题", 主题默认跟随系统; 系统浅色时子面板用浅色主题控件, 贴在我们手绘的深色面板上 | `set_theme(Dark)` + `set_style_of(Dark/Light, 同一样式)`(R15) |
| 25 | 悬停说明浮窗右侧文字被截断 | 横排(`ui.horizontal`)里的 Label 默认不换行, 超宽会撑大内容区, 连带后续换行文字都越过右边框 | 说明文字一律 `Label::new(..).wrap()`(R15) |
| 26 | 视口状态的渲染/AO/网格段在 26.2.3 恢复不了, 却没人发现 | `vp.settings.Viewport.renderer/ao`、`vp.settings.Grid` 在 26.2.3 已不存在; 旧片段每段各自 try/catch, 失败只进 failed 列表 | 垫片能力探测把它列为 `optionalMissing`(R16); 原则: 内部路径只写在垫片 LOC 表里, 由探测统一报缺失 |
| 27 | 以为 26.2.3 只能靠堆快照找 editor | 当年的死路清单没试 Preact vnode 树; 实测 `#root.__k` → `Outliner.props.model.editor` 在 26.2.3 仍可用(组件名未被压缩) | 垫片 FINDERS: 全局名 → Preact 遍历; 堆快照只作后备(R16) |
| 28 | 代理链路收不到 `Page.loadEventFired` | pla-cdp-v8 代理只回转发命令的回包, `eventCb` 从未调用; 假 target id 写死, 也看不到 target 变化 | F4 在主进程侧感知 renderer 重载(待做) |
| 29 | 工具栏状态点的悬浮提示右侧、底部被裁 | HUD 视口固定 260×56, 提示文字不换行; 窗口标题一长或多一行就溢出 | HUD 高度按内容实测(`hud_size`, 用同一换行宽度 layout), Label 一律 `.wrap()`, 长标题截断(R18) |
| 30 | CDP 已连接, 工具栏状态灯却是红色 | 状态点内点沿用 U7 时代的语义 = 官方 bridge(8980)状态; 路线改为 CDP 主通道后 bridge 只是只读冗余, 它断开不影响任何功能, 却占着最显眼的灯 | 内点 = CDP 连接状态, 外环 = 能力受损提示, bridge 只进悬停提示/能力面板(R18.1) |
| 31 | 多开时助手绑错窗口 / 两个助手叠在同一窗口 / 快照墙混进别的文件 | 旧绑定每轮按「pla-ui 启动序号 + 标题含文档名」重算: 标题一变(开文件、切文档、快照窗口弹出)就可能换绑, doc_key 跟着跳 | 认领式绑定: HWND 写在自己工具栏的窗口属性上, 跟着 HWND 走, 跳过他人认领(R19) |
| 32 | Bridge 连不上, 但 Plasticity 设置里 Server 开着、端口 8980 | 服务端(ws WebSocketServer)`_state=2` 已关闭: 启动时端口被占用(上一个 Plasticity 没退干净)→ 监听失败自行关闭, 设置开关不会反映 | 设置里关掉再打开 Server; pla-ui 经垫片读服务端状态, 识别后直接提示(R19) |
| 33 | 两个 Plasticity 窗口时, 第二个窗口的助手读写的是第一个窗口 | 一个进程多个 BrowserWindow; 代理写死 `getAllWindows()[0]` | 代理按窗口列目标(带 hwnd, ws 路径 /hwnd/<n>), pla-bridge 按绑定 HWND 连接(R19) |
| 34 | 助手在切到别的程序后仍压在最上面 | 所有视口固定 `AlwaysOnTop` | 前台判定后用 SetWindowPos 切换: 非前台时 NOTOPMOST 并插到绑定窗口正上方; 只在 WS_EX_TOPMOST 与期望不符时动手(winit 改样式不碰置顶位)(R19) |
| 35 | 重建 pla-ui 后助手全不见了 | 用户多开时有多个 pla-ui 进程; `Stop-Process -Name pla-ui` 全停, 而 `(Get-Process pla-ui).Path` 是数组, `Start-Process` 直接报错, 一个都没起来 | 构建前记下实例数, 按实例数逐个启动(间隔几秒, 各自认领窗口); 用工具栏窗口属性 `PLA_BOUND_HWND` 核对绑定(R20) |
| 36 | 垫片热替换后 `cmd.ping` "已注册但回调没执行" | 旧实例注册到 Plasticity 的命令回调闭包捕获了旧实例的状态; 新实例看到"已注册"就不再注册, 回调却在给旧实例计数 | 命令回调一律委托: 触发时找当前 `window.__PLA__._onCommand`; 已挂委托的元素记在页面级 `__PLA_PAGE__`(按注册表区分), 热替换不重复注册(R21) |
| 37 | 第二个 Plasticity 窗口的 Bridge 永远连不上 | 每个窗口的 Bridge 服务端都监听 8980, 只有第一个成功, 其余监听失败后自行关闭 | 多窗口时属正常; 助手/自检提示中写明(R21) |
| 38 | 选择集应用: 第一个实体的面选中了, 其他实体却整个被选中 | Plasticity 点击语义(R21 实测): 点**未选实体**上的面 → 选中整个实体; 在同一点再点一次 → 换成该面(下钻), 第三次不变。管线每个面只点一次 | 点完后由垫片判定"未选中但所属实体已介入"的目标, 原坐标再 shift+点, 最多 3 轮(R21.1) |
| 39 | 恢复视口后缩放不生效, 动一下视口才变 | `cam.set` 写了轨道控制器的 zoom/zoomEnd 等, 但没调 `setNeedsUpdate()`(`needsUpdate` 是计数器), 控制器的更新循环要等下次交互才跑 | 写完统一 `markOrbitDirty`; 自检回读要等控制器收敛(~1.5s, 后台窗口帧率被节流)(R21.1) |
| 40 | node 内置 WebSocket 连不上 9333 代理(1006) | 代理手写的握手与 undici 的 WebSocket 客户端不兼容(Rust tungstenite 正常) | 调试脚本改走主进程 inspector + `__PLA_PROXY_SENDS[hwnd]`; F4 用 Rust 重写代理时一并解决 |
| 41 | `click.addById` 选中后 Move 一接手选择就没了("缺活跃实体上下文") | addById 不产生命令、不派发 selectionDelta; 真实点击是经 executor 执行的 `ClickChangeSelectionCommand`; 且 `addById('2:<vid>')` 写进的是畸形条目 `"<vid>:undefined"` | 自建 `SelectionCommand` 子类, execute 里调 `changeSelection.onClick`, 经 `editor.enqueue` 执行(selection-by-command.md) |
| 42 | `await editor.enqueue(cmd)` 永远不返回 | Promise 要等整条命令链结束, 后续默认命令(如 OffsetFace)在等输入 | 不 await; 监听 `commandEnded` 判断本命令结束 |
| 43 | 入队的选择命令不执行 | 活动命令等待输入时新命令排队 | 入队前 `executor.cancelActiveCommand()` |
| 44 | 在 body 上派发 `command:move` 没反应 | 它是挂在 `plasticity-viewport` 上的选择器式监听 | 以视口元素为目标派发 |
| 45 | 自动化截全屏 / 模拟鼠标点到了用户正在用的别的程序(截到私人聊天窗口) | 用户在用电脑时切了窗口; 助手按 R19 只在 Plasticity 前台时置顶, 工具栏已不在原位, 屏幕坐标点击落到别的窗口 | 用户在场时**不做桌面级截图/鼠标模拟**; 验证 Plasticity 用 CDP `Page.captureScreenshot`(只截渲染进程), 验证助手界面请用户目测(R23) |
| 46 | 选择集面板一律"无截图" | R9 把存档挪到文档同级, 但保存/列表/读取/取名仍写 exe 同级, 只有截图/收藏/改名/删除用新目录; 迁移只在新目录不存在时复制一次 | 全部经唯一入口 `cdp_selections_dir()`; 迁移改为每进程一次"补齐缺失或更新的文件"(R23) |
| 47 | 同窗口换文件后快照/选择集截图仍是旧文件的(文件和恢复都对) | egui 按 URI 缓存解码纹理, URI 只用名字 `bytes://SS001`; 清自己的字节缓存不会清 egui 的。另: 拖动模式提前 return 不更新 doc_key; 文档切换只重置部分缓存 | URI 带内容指纹 `png_uri()`; 切文档 `forget_all_images()`; `sync_doc_key()` 两条路径都调, `reset_doc_scoped()` 为按文档状态唯一清单(R26) |
| 48 | 想给 exe 嵌图标, 但 embed-resource/winresource 都要 windres 或 rc.exe | 本机 Rust 是 x86_64-pc-windows-gnu, 没装 binutils/Windows SDK | `build.rs` 自己生成只含 `.rsrc` 节的 COFF(.o)(资源树 + ADDR32NB 重定位), `rustc-link-arg-bins` 交给 ld 合并; 用 `ExtractAssociatedIcon` 验证(R28) |
| 49 | 笔记窗口拖动/调大小后鼠标移到别的窗口再回来, 窗口自己跟着拖、越拉越大 | 用了 OS `StartDrag`/`BeginResize`: 模态循环吞掉松手事件, egui 的按键状态卡在"按下", 之后任意 drag_started 又触发一次(同 R9 一级面板的老坑, 新窗口重踩) | 自管会话: `SetCapture` + 屏幕坐标 delta 发 `OuterPosition`/`InnerSize`; 开始/结束以 `primary_button_down()`(GetAsyncKeyState)为准。新窗口一律不要用 StartDrag/BeginResize(R30b) |
| 50 | 笔记表格选 3 行右键删除 → 助手闪退 | 选区只在帧首夹回表内; 右键菜单在帧中途删了行, 同一帧后面的状态栏还按旧选区下标 `t.cells[r][c]` 读 → 越界 panic(整个进程崩) | `TableState::clamp_to()` 在每次结构变化后立即调用 + 帧内二次夹回; 读格子改 `.get()`。panic 日志在 `%TEMP%\pla-ui.log`(R31c) |
| 51 | 夜间无人时 CDP 截图"冻结"、memento 恢复抛 "reading 'Viewport'" | 显示器关闭/窗口被遮挡 → 页面 visibilityState=hidden, rAF 停(0 帧), 截图一直是最后一帧; 此时 `originator.restore` 连原样恢复当前快照都在 Plasticity 内部抛错 | 验收前先查 `document.visibilityState`/rAF; 垫片 `mementoGuard()` 在 hidden 时拒绝 memento 恢复并如实报告(R33) |
| 52 | 视口状态实验把着色弄坏: 视口冻结、右侧面板不再刷新 | shader 模式下调 `shading.setMatcap(file)` → matcap 变 undefined → `TextureLoader.lookup` 每帧抛 → 渲染循环停; 面板 React 树随之失效(点击仍生效但界面不刷新) | 着色一律点面板按钮恢复; 垫片在 shader 模式下禁用 setMatcap 兜底; 面板失效需重开该文件(R33, know-how/viewport-state.md) |
| 53 | 每次保存视口状态/选择集, 整个屏幕闪一下黑 | 缩略图用 CDP `Page.captureScreenshot` 从合成器表面截取, 某些 GPU 上会让窗口闪黑 | 垫片 `cam.thumbnail`: 同一 JS 任务里 `vp.handleRender()` → 读 WebGL canvas(没开 preserveDrawingBuffer, 跨任务就读到旧缓冲) → 铺页面背景色(canvas 透明)→ 缩到 960 宽; CDP 截图只作兜底(R33d) |
| 54 | 材质状态恢复"成功"、数据也对, 但视口还是旧颜色 | memento `originator.restore` 改了材质/指派, 但不会请求重画; 下一次交互才刷新(像是恢复没生效) | 恢复后主动 `vp.render`(LOC 表已有); 验收截图前不要手动强制渲染, 否则会掩盖这个问题(R34c) |
| 55 | 材质状态恢复, 某几个存档(含 Gold foil)报 "垫片异常: undefined is not a function", 之后物体指向不存在的材质 | ① `mat.save` 把材质里的 THREE.Texture JSON 化了, Plasticity 的 deserializeTexture 要真 Texture; ② memento 恢复中途抛错不回滚, 指派已写、材质没写 | 恢复前按文件名经 `materials.textures.get` 重建 Texture(同文件沿用当前对象); restore 失败用恢复前快照整体退回(R34d)。复现要点: 只调 `ed.materials.restore(...)` 就能拿到堆栈, 窗口隐藏时也能做 |
| 56 | 第一次执行脚本命令要等 78 s | 代理按名复活工厂类时每个都调一次 `HeapProfiler.getObjectByHeapObjectId`, 它每次都扫整个堆(~0.3 s × 229 个) | 只按 id 复活一个类 → 沿原型链到类层次的根 → `Runtime.queryObjects(根)` 一次扫堆拿到全部子类; 没覆盖到的名字再重复(实测 10 次扫堆, 8 s, 其中快照 2.7 s)(N2) |
| 57 | 工厂/命令报 PK_ERROR_field_of_wrong_type, 或按 `edgeIds` 查边抛 "object null is not iterable" | 26.x 几何内核是 Parasolid: 枚举值是 PK 常量(合并 = 15903, 不是 0/1/2); 拓扑 id 是字符串 `<物体版本id>e<边id>` / `f`(面), 组里的 `edgeIds`/`faceIds` 是裸数字, 不能直接查 | 枚举只写在垫片 NAMES 表; 边/面 id 取组上的 `versionIds`(与 `bboxes` 同序)(N2) |
| 58 | 视口状态恢复: 存的是正交, 恢复后仍是透视(恢复结果里却有 projection) | 视口的 `fov` 不是视场角, 是投影开关(透视 1 / 正交 0), 写它(任何值, 包括 0)都会切成透视; 标量段排在视角段之后又写了一次 fov | fov 移出 `NAMES.vpScalars`, 投影只由 view 段 `togglePerspective` 负责; 老存档的 scalars.fov 忽略。附带: `vp.init` 收到 mode=perspective 时会自己切成透视, 反方向不会(R33, 2026-11-15) |
| 59 | 原样恢复的材质状态被当成"改动"(走 memento, 窗口隐藏时报 window hidden); 视口状态恢复可能把文档写成"未保存" | 存档经 Rust 转一圈: serde_json 没开 preserve_order, 对象键按字母重排; 垫片拿 `JSON.stringify` 直接比字符串 | 比较一律用键序无关的 `sameJson`(垫片); 以后凡"没变化就不写"的判断都不要直接比 JSON 字符串 |
| 60 | 升级 pla-ui(垫片热替换)后 Alt+←/→ 滚转、自检 cmd.ping 失灵, 重载页面才好 | 旧实例 `dispose` 注销了自己挂的命令监听, 但页面级"已挂委托"记录(`__PLA_PAGE__`)跨实例保留, 新实例以为挂过不再补挂 | `ensureDelegate`: 记录在**且**注册表里确实还在才跳过 |
| 14 | 切换面板时旧面板滞留、新面板不出现、点击丢失 | R12 加深的文档定位(`find_latest_doc`)每次未命中都全盘深扫(热缓存 ~2s, 冷缓存 ~19s), 面板每帧取文档目录, 2s TTL 一过就在 UI 线程重扫 | 分阶段: leveldb + Recent 命中即返回; 深扫未命中负缓存 60s(R13) |

## 假按钮教训
v1 GUI 多个按钮点了没反应（骨架期只接了 2 个 action）。R2 改 run_action 集中分发:
未接线的功能弹诚实提示(说明缺什么), 不允许静默无反应。
