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
| 14 | 切换面板时旧面板滞留、新面板不出现、点击丢失 | R12 加深的文档定位(`find_latest_doc`)每次未命中都全盘深扫(热缓存 ~2s, 冷缓存 ~19s), 面板每帧取文档目录, 2s TTL 一过就在 UI 线程重扫 | 分阶段: leveldb + Recent 命中即返回; 深扫未命中负缓存 60s(R13) |

## 假按钮教训
v1 GUI 多个按钮点了没反应（骨架期只接了 2 个 action）。R2 改 run_action 集中分发:
未接线的功能弹诚实提示(说明缺什么), 不允许静默无反应。
