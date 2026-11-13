# PLAhelper 待办总表（2026-11-12 重写）

> 唯一待办来源。路线依据：`docs/research/2026-11-12-route-decision.md` §8（Plasticity 优先）。
> 旧版 G2–G4（bridge 协议时代）已作废，见文末「已废弃」。
> 状态：`[ ]` 未做 · `[~]` 有代码但不可用/待验证 · `[x]` 完成并真机验证。

## 执行顺序

1. **Q** 快速见效：步进视角
2. **U** UI 硬伤：标题栏闪烁、圆角锯齿、面板位置
3. **B** 现有功能 bug：选择集、视口状态、快照墙（真机复现后逐条修）
4. **F** CDP 地基：垫片层、能力探测、契约测试、自动重定位
5. **N** 新功能：材质方案 → 命令派发层 → 参数化修改 → 打孔工具 → 常用件库 → AI 建模 → 轻量装配

---

## Q — 快速见效

- [x] **Q1 方向键步进旋转**（R14 完成，CDP 实测 + 用户手按确认 2026-11-13）
      - 设置面板 ORBIT STEP：勾选框 = 在 `keymap.json` 的 `'plasticity-viewport'` 块加/删 4 条
        方向键 → `orbit:rotate:{left,right,up,down}`；输入框 = 每按一下转动的角度（°），
        写 `settings.json` 的 `OrbitControls.rotateDiscreteIncrement`（**单位弧度**）。
      - 最小文本改动（不重排用户文件），首次写前备份 `*.json.pla-bak`；`plasticity_config.rs` 11 项单测。
      - 生效方式（26.2.3 实测）：keymap.json 由 Plasticity 自动重载；settings.json **不重载**，
        步进角缓存在各视口 `orbitControls.rotateDiscreteIncrement` → CDP 已连接时 pla-ui 直接改它
        （30/15° 实测立即生效），未连接时重启 Plasticity 生效。
      - 首版失败原因：绑定写在 `'orbit-controls'` —— 那是鼠标手势用的虚拟选择器，页面上没有这个元素。
- [~] **Q1b 方向对调 + Alt 滚转**（R14，CDP 实测，待用户手按确认）：←/→、↑/↓ 与原生方向对调；
      Alt+←/→ = 绕屏幕轴按步进角滚转（`pla:roll:*`，pla-bridge `ROLL_HOOK_JS` 经 CDP 注册，
      改轨道控制器 `quaternionEnd`；Plasticity 自带 `viewport:roll:*` 固定 90°）。未连 CDP 时 Alt+←/→ 无效。
- [x] **F7 Plasticity 默认以 CDP 模式运行**（R14，pla-ui 托管；2026-11-13 实测：任务栏快捷方式启动 →
      3s 调试端口就绪 → pla-ui 2s 内自动拉起代理 → 单次堆快照定位 editor → 方向键/滚转/角度全部可用；
      代理在 pla-ui 重启后存活且不重复拉起）
      - 设置面板 CDP MODE 勾选：用户级快捷方式（任务栏固定、用户开始菜单、用户桌面）指向已打 fuse 的 exe
        并带 `--inspect=127.0.0.1:9334`；`.plasticity` 双击 = HKCU 覆盖 open 命令（原值备份为
        `PlaHelperPrevCommand`，取消勾选写回）。公用桌面/全体开始菜单需管理员，只报告不改。
      - 代理托管：9334 在听且 9333 无人监听 → 自动 `node tools/pla-cdp/pla-cdp-v8.js proxy`
        （无窗口，日志 `%TEMP%\pla-cdp-proxy.log`），代理独立于 pla-ui 存活。
      - 后续：代理用 Rust 重写进 pla-bridge（去掉 node 依赖）；Plasticity 升级后提示重新打 fuse。
- [ ] **Q2 视口立方体右键旋转按钮跟随步进角**（可选）：按钮命令是 `viewport:rotate:*`/`viewport:roll:*`，
      固定 90°，不读 rotateDiscreteIncrement。要改只能在 CDP 连接时替换这组命令的监听器，待用户决定。

## U — UI 硬伤

- [~] **U1 切换二级面板闪出 Win98 式标题栏**（R13 已修，屏幕连拍未再出现，待用户日常使用确认）
      根因两条：`SetWindowRgn` 让 DWM 停止合成非客户区（激活时经典标题栏被直接画出）；
      文档定位深扫让 UI 线程长时间卡死（>5s 系统换成带标题栏的幽灵窗口）。均已消除，见 pitfalls #11/#14。
- [~] **U2 圆角锯齿**（R13 已修）：改用 Win11 DWM 系统圆角（抗锯齿），删除 `SetWindowRgn`；
      子视口窗口透明在 glow 下本就不生效（四角黑），圆角必须交给 DWM。Windows 10 下会退化为方角。
- [x] **U3 面板位置不对**（R13 已修截图里的问题）：二级面板左缘比工具栏右偏 2pt → 对齐；
      新面板先隐藏渲染 2 帧再显示，不再在错误位置以白底闪现。
      **最大化时工具栏偏低 11px、偏右 11px**：标题栏探测把 PrintWindow 位图当成从客户区原点开始
      （实际从窗口矩形原点开始，最大化时差 11px），再叠加客户区偏移 = 双重补偿。已修，最大化实测上下各留 3px；
      窗口化与最大化均经用户确认（2026-11-13）。
- [x] **U4 面板边框**（R13）：1.5pt 灰180（150% 缩放约 2 物理像素），用户确认（2026-11-13）。
- [x] **U5 切换面板的淡入淡出**：DWM 系统动画（旧面板淡出约 0.1s、新面板淡入），用户确认保留。
      如需关掉：`DWMWA_TRANSITIONS_FORCEDISABLED`。
- [~] **U9 设置面板原生风格**（R15，待用户确认观感）：圆角分组卡片 + 大写粗体标题；左标签右控件；
      胶囊开关（开 = 强调黄）；圆角数值框（白字居中）；步进角 −/+ 步进器（±5°）；行标签悬停变白加粗；
      说明文字移入悬停三级面板（快捷键用键帽排版），有说明的行带 "?" 标记。其他二级面板尚未改成卡片风格。
- [x] **U10 二级面板文字看不清 / 悬停变黑**（R15）：egui 0.29 主题跟随系统，系统浅色时子面板用了
      浅色主题控件 → 固定深色主题并把样式写入深/浅两套；COL_MUTED 提亮到 #a0a0a0。
- [x] **U11 多开绑定 + 置顶**（R19，双窗口双助手经用户确认 2026-11-13）
      - 认领式绑定：每个助手把绑定的 Plasticity HWND 写在自己工具栏的窗口属性 `PLA_BOUND_HWND` 上；
        绑定跟着 HWND 走（换文档/改标题/最小化都不换绑），窗口关闭才重新认领；新认领优先前台窗口、
        跳过他人已认领的；同时认领冲突时先启动的保留。取代旧「进程序号 + 文档名」每轮重算。
      - 置顶：只在绑定窗口（或其弹窗、或助手自己）处于前台时置顶；切到别的程序 → 取消置顶并贴在自己的
        Plasticity 窗口正上方（实测：Explorer 前台时工具栏非置顶、Z 序紧贴其 Plasticity 窗口之上）；
        绑定窗口最小化 → 沉到底部。⇒ 激活哪个 Plasticity 窗口，标题栏上出现的就是绑定它的那个助手。
      - CDP 跟随绑定窗口：代理 `/json/list` 每个 Plasticity 窗口一个目标（带 `hwnd`，ws 路径 `/hwnd/<n>`），
        pla-bridge 按绑定 HWND 连接、换绑即重连 —— 以前代理只连第一个窗口，第二个窗口的助手实际在读写第一个窗口。
- [~] **U12 面板一波优化**（R23，待用户目测）
      - 快照墙 / 选择集 / 视口状态的管理条统一：已选 N · **全选**（全部已选时再按 = 全部取消）· **反选** · 删除所选 · 完成
        （共用 `select_all_invert_buttons`；视口状态原先连"删除所选"都没有，一并补上）。
      - HUD（悬停提示 / 通知）放在"工具栏与当前打开的二级面板中较宽者"的右侧，不再压在宽面板上（`hud_left`）。
      - 选择集面板改两列：左列选择集墙占满高度（原先封顶屏高 1/3 还和 ENTITIES 共列，收藏放不下一个），
        右列单独的 ENTITIES（实时选择 + 报错）；面板宽 280 → 520。视口状态墙同样改为占满剩余高度。
      - 选择集截图丢失：保存与读取目录不一致（pitfalls #46），统一经 `cdp_selections_dir()`，旧截图自动补到新目录。
      - R24 ENTITIES 列可区分：选择快照每项带描述（所属物体名、自身名字、曲面类型），按所属物体分组显示
        「物体名 · 16 面」+「平面 #594624」。来源：物体名 `nodes.getName(nodes.item2key(视图))`、
        曲面类型 `面模型.GetSurface().surface` 的构造函数名（Plane/Cylinder/Sphere/Torus/BlendSurf…，中文名表在 `sel_labels.rs`）；
        按版本 id 缓存，轮询不重复查。
      - R25 状态与结果分离 + 界面双语：CDP 事件拆成 `Conn(ConnState)`（连接行只显示连接状态）和 `Report{ok,msg}`
        （操作结果走提示，成功绿/失败红，不再靠字符串匹配判颜色）。设置面板新增 LANGUAGE 卡（简体中文 | English），
        一键切换即时生效，记入 exe 同级 `ui-prefs.json`。实现：`pla_bridge::i18n` 的 `tr("中","En")` / `t!(...)`，
        两种语言写在同一调用点，不另建字典；日志、注释、测试、落盘内容保持中文。
        已知：切换前生成的提示/状态文字保留旧语言直到被替换；代理状态最多 3 秒后刷新。待用户验证。
      - R26 同窗口内换文件，快照/选择集/视口状态跟着换：① 工具栏被拖离吸附位时 `update_snap` 提前 return，
        doc_key 不再跟标题更新 → 抽成 `sync_doc_key()`，两条路径都调；② 文档切换只重置了一部分缓存
        （视口状态列表、选择集/视口截图按名字缓存，新文档同名 SG001 显示旧图）→ `reset_doc_scoped()`
        作为按文档存的状态的唯一清单。截图纹理 URI 带内容指纹（egui 按 URI 缓存，同名 SS001 曾显示旧图）。用户已确认。
      - R27 英文界面排版 + BRIDGE 卡迁移：设置行标签按剩余宽度缩字号（12.5→10.5）再省略号（`fit_text`，截断时悬停看全文），
        value_row 标签与值分宽；管理条/快捷键行/尺寸面板改 `horizontal_wrapped`，英文按钮改短（All/None/Invert/Delete）。
        设置里 BRIDGE 卡删除：绑定窗口移到能力面板 OVERVIEW 首行，Bridge 状态/地址成为能力面板单独的 BRIDGE 卡。待用户验证。
      - R28 管理条统一 + 程序图标：快照墙/选择集/视口状态共用 `manage_bar()`（列表顶部、红色删除、全选/反选/完成）；
        视口状态面板改用自己的 `vp_manage/vp_selected`（以前借用选择集的，两个面板一起进管理模式，且没有勾选框）。
        图标 PH（白色几何 P 同 Plasticity 风格 + H 琥珀横梁，深色圆角底）：`tools/icon/make-icon.ps1` 生成
        `assets/pla-helper.ico/.png`；`build.rs` 手写只含 .rsrc 的 COFF 目标文件嵌进 exe（本机 GNU 工具链无 windres/rc）；
        任务栏用 `with_icon` 设窗口图标。待用户验证。
      - R29 快照墙/视口状态面板宽 280；缩略图高度由格宽推出（`fav_img_h_for`/`big_img_h_for`）。
      - R30 笔记面板（记录 + 计算器）：工具栏 📝 按钮；独立浮窗（不互斥、不自动关、首次默认位置，
        之后位置/大小记在 ui-prefs.json `notes_rect`；标题行 StartDrag 拖动，四边四角 BeginResize 调大小）。
        跟文档走：`<文档目录>/.pla/notes/<doc>/notes.md`（普通 Markdown，0.6s 防抖保存，切文档/关闭/退出立即写）。
        块编辑：只显示渲染效果，点行编辑原文；`- ` `1. ` `[] ` `# ` `---` 即时转换；待办打勾加删除线；拖动柄重排，
        右键柄删除/复制/转换；Tab 缩进；`=` 开头的行即时计算。表格：Excel 公式（`formula.rs`：A1/范围/SUM/AVERAGE/IF/ROUND…
        共 40+ 函数，循环引用 #CIRC!，^ 左结合同 Excel），右键行/列号插入删除。
        模块：`notes.rs`（模型/解析/序列化）、`notes_ui.rs`（界面）、`formula.rs`（公式）。待用户验证。
        已知限制：插入/删除行列不会自动改写公式里的引用；表格公式只能引用本表。
      - [x] **R31 笔记面板改进（2026-11-14 实现，待用户验证）** —— 下列 N31-1..9 全部落地:
        表格独立模块 `notes_table.rs`(手绘布局 + 自管指针: 选区/编辑分离、填充柄、复制粘贴、Ctrl+Enter、列宽行高、横向滚动、点选引用、
        右键插删行列并改写公式); `formula.rs` 新增 `shift_formula/insert_rows/delete_rows/insert_cols/delete_cols/ref_name/parse_ref`
        (`$` 锁定); `notes.rs` 的 `Table{cells,col_w,row_h}` + `<!-- pla:table -->` 尺寸注释 + 单元格换行存 `<br>`;
        文本格式: Ctrl+B/I 切换、文字色/底色调色板(`<span style>`)、编辑态 layouter 淡化标记。
        已知限制: 剪切后粘贴不会像 Excel 那样移动引用(当作复制+清空); 清除颜色按整段 span 清。
      - [x] **R31b 用户反馈(2026-11-14)**: ① 笔记 Ctrl+Z 撤销 / Ctrl+Y、Ctrl+Shift+Z 重做(整篇快照历史, 同行连续输入 1.2s 内合并,
        切文档清空; 在面板入口先吃掉按键, 行内 TextEdit 自带撤销不再单独生效); ② 表格右键"数量 [x]"+ 上/下添加 x 行、左/右添加 x 列;
        ③ 行号/列号拖动多选整行整列, Shift+点行号/列号扩选, 右键删除作用于选中的全部行/列(右键未选中的行号先选中它);
        拖已选中整行/整列的行号/列号 = 调换顺序(蓝线指示落点, 公式引用跟着走: `formula::move_rows/move_cols`)。待用户验证。
        - [ ] N31-1 粗体/斜体快捷键：编辑中 Ctrl+B / Ctrl+I 给选中文字包 `**…**` / `*…*`（无选区 = 插入成对标记、光标放中间；
          已包裹再按 = 去掉）。做法：TextEdit 前 `consume_key(COMMAND, B/I)`，读 `cursor.char_range()` 改文本后回写光标。
          渲染已支持（`inline_spans`）；粗体目前用"更亮的白色"模拟，若要真粗体需加载粗体字体（simhei 无粗体字重，考虑描边/双绘）。
        - [ ] N31-2 文字颜色 / 背景色：工具条加调色板（预设 6–8 色，不做任意取色）。存盘格式需兼容 Markdown：
          用 `<span style="color:#e06c75">…</span>` / `background:`（Obsidian/Typora 能显示），`inline_spans` 增加 span 解析，
          Style 加 `color/bg: Option<Color32>`。编辑模式下显示原文会露出 span 标签 —— 考虑编辑模式也用 layouter 着色并隐藏标签（成本中）。
        - [ ] N31-3 表格尺寸：列宽当前按面板宽均分且无滚动条 → C 列被裁。改为：每列独立宽度（默认 72，拖列头右边界调整，
          行高同理拖行号下边界），宽度存进 Markdown 表格前的注释 `<!-- pla:cols=72,90,60 rows=22 -->`；
          表格外包水平 ScrollArea 并显示滚动条（现在的 ScrollArea 没给固定宽度 → 不出滚动条，需 `max_width(avail)`）。
          行号列由 22 缩到 ~16（字号 10）。
        - [ ] N31-4 写公式时点选单元格（成本评估：中，可做）：单元格处于编辑且文本以 `=` 开头、光标前是运算符/`(`/`,` 时，
          点击别的单元格 = 在光标处插入其引用（A1），按住拖动 = 插入范围（A1:B3），不切换编辑格；被引用单元格高亮边框。
          需要: 编辑格保持焦点（点击时先记光标位置，插入后 request_focus 并恢复光标）、拖选矩形绘制。
        - [ ] N31-5 Excel 式操作手感（用户 2026-11-13 追加；N31-6..9 都依赖它，先做）：
          表格引入"选区"状态（活动格 + 矩形选区），与"编辑中"分开。单击 = 选中（蓝框），再单击/双击/F2/直接打字 = 进入编辑
          （直接打字覆盖原值，F2/双击保留原值）；方向键/Tab/Enter 移动活动格；Shift+点击/Shift+方向键/拖动 = 扩选；
          Delete 清空选区；Esc 取消编辑恢复原值。
        - [ ] N31-6 填充柄拖拉：选区右下角小方块，拖向下/右把公式按相对引用平移填充到多行/多列（纯数字/文字原样复制；
          以后可加等差序列识别）。
        - [ ] N31-7 `$` 绝对引用：`formula.rs` 已能解析 `$A$1`/`A$1`/`$A1`（目前计算时忽略）；需在 AST 里保留锁定标记，
          新增 `shift_formula(src, d_row, d_col) -> String`（锁定的行/列不动，越界 → `#REF!`），填充柄和粘贴共用。
          也可顺带解决 R30 已知限制：插入/删除行列时用同一个改写器修正引用。
        - [ ] N31-8 复制 / 粘贴：Ctrl+C 复制选区（内部剪贴板存原始公式 + 系统剪贴板放 TSV，可直接粘到 Excel）；
          Ctrl+V 粘贴公式（按相对位移改写引用，多格选区原样铺开；从 Excel 复制来的 TSV 也能粘）；
          Ctrl+Shift+V 只粘贴计算后的值。Ctrl+X 剪切。
        - [ ] N31-9 多格同时填值：选中多个单元格，输入内容后按组合键应用到选区每一格（公式按相对位移改写）。
          按 Excel 习惯（用户已确认）: **Ctrl+Enter** = 应用到选区每一格; Alt+Enter = 单元格内换行。
- [x] **U6 提示浮窗标题为 "egui window"**：随 U8 一起解决（被拥有窗口不进任务栏，也不进 Alt+Tab）。
- [x] **U7 连接指示灯跟随主题色变黄**（R13）：R12 把强调色改黄后，"已连接"也变黄，和"连接中"无法区分。
      状态语义色改为固定色、不跟主题：灰 = 未绑定 / 黄 = 连接中 / 绿 = 已连接、成功 / 红 = 失败。
- [x] **U8 二级面板在任务栏显示为"未知程序"**（R13）：面板与提示浮窗设为工具栏的被拥有窗口，
      并在首次显示前清掉 winit 加的 `WS_EX_APPWINDOW`。任务栏只剩 pla-ui 一个按钮（UI 自动化实测）。

## B — 现有功能 bug（26.2.3 上未做过真机回归）

先按「复现步骤 → 期望 → 实际」逐条登记到本节，再修。已知线索：

- [x] **B1 选择集** —— R22 已整条替换为**按数据恢复**（用户界面复测通过 2026-11-13）：垫片 `sel.apply` 按 id 查对象，
      在一个自建 `SelectionCommand` 子类里逐项 `changeSelection.onClick`（未选中则下钻一次），经 `editor.enqueue` 执行。
      真机：60 面 14 ms 全中、撤销 +1、下钻 10 次；完整链路（选中 → G / Shift+Delete 真的改几何 → Ctrl+Z）已验证。
      删除悬停扫描 / 真实点击 / 遮挡补救 / 下钻补点 / 相机分组整套（cdp.rs −805 行）与 `scan.*`、`sel.addFaceById`、`sel.faceIds`；
      `sel.clear` 改为执行 `DeselectAllCommand`（`click.clear` 实测不清空）。F3 自检加了选择回环（撤销栈 +1 属预期，守护段扣除）。
      保存时的相机仍随集存储（以后可做"应用后切到保存视角"）。依据见 `docs/know-how/selection-by-command.md`。以下旧线索作废：
      - 多视角分组还原（R8 相机分组 + R10 补全 zoom/orbit 存档，端到端未验证）；
      - 遮挡补救链（自身相机 → addById → navigateToSelection → 重扫）；
      - 应用后焦点交还 Plasticity、实时镜像与存档成员一致性。
- [~] **B2 视口状态**：相机/shading(matcap)/剖面/构造面恢复（26.1.3 像素级通过，26.2.3 未测）；正交相机 zoom 目前跳过。
      R16 能力探测发现：26.2.3 视口上已没有 `settings.Viewport.renderer/ao`、`settings.Grid` → 这三段保存/恢复必然失败，
      需在 F1b 找到新位置（在垫片 LOC 表补备选路径即可）。
- [~] **B3 快照墙**：R12 的文档定位改动（leveldb 最近文件 + 严格名称匹配）待真机验证；快照里打开/切换流程回归。
      R13 已修其性能问题（未命中时每 2s 在 UI 线程全盘深扫）。仍可改进：文档定位整体挪到后台线程。
- [ ] **B3b 快照墙混进别的文件的快照**（用户 2026-11-13 观察，先记录）：一个文档的快照墙里同时有两个文件的快照；
      双开双绑定后新增的快照是对的。推测是旧绑定方案的锅：每轮按「进程序号 + 文档名」重算绑定，标题一变就可能换绑，
      doc_key 跟着跳，快照写进了另一个文档的目录（pitfalls #31）。R19 已改认领式绑定；若再出现，查 `snapshot_dir_of(doc_key)`
      的归档与 doc_key 来源，必要时按快照元数据里的源文件名过滤。
- [~] **B7 Bridge 连不上但设置里端口开着**（R19 诊断）：CDP 实查 Plasticity 的 Bridge 服务端 `_state=2`（已关闭）、无监听地址，
      多半是启动时 8980 被上一个没退干净的 Plasticity 占着（见 B6），监听失败后服务端自行关闭，设置开关仍显示开。
      R21 补充：**多窗口时每个窗口都想监听 8980，只有一个能成功**，其余窗口的服务端必然是关闭状态（自检对第二个窗口报 WARN）。
      恢复：Plasticity 偏好设置 → Network 把 Server 关掉再打开。pla-ui 现在会识别这种情况并在状态里直接写明原因和办法。
- [ ] **B4 renderer 重载后注入钩子丢失**：目前要重连才恢复（与 F4 一并解决）。
- [x] **B5 pla-ui 无声退出**：经用户确认是手动退出，不是 bug。保留诊断日志 `%TEMP%\pla-ui.log`
      （启动 / 正常退出 / 点 × / panic 含 backtrace），以后真出问题时可查。
- [ ] **B6 关闭 Plasticity 后进程残留**：旧 launch 方式下（代理持有调试会话）关窗后主进程 + 3 个辅助进程
      不退出，直到代理结束。新方式（快捷方式启动、代理独立）下是否还有，待用户下次关闭 Plasticity 时确认。

## F — CDP 地基（主通道可持续）

- [~] **F1 JS 垫片层**：注入带版本号的 `window.__PLA__`，把 `cdp.rs` 里散落的 JS 片段收口；Rust 只调垫片。
      设计与四条硬规则（内部结构单表、能力由表生成、接口名单一来源、常量不复写）见 `docs/know-how/cdp-shim-design.md`。
      - [x] **F1a**（R16，2026-11-13 真机验证）：`src/crates/pla-bridge/js/pla-shim.js` + `pla_bridge::shim`；
            框架（LOC 表/CAPS/接口表/钩子代际管理/热替换）+ 只读接口 `core.* / sel.snapshot / hist.state / cam.get / err.last / tree.list`；
            选择轮询改为一次 `batch`（每项独立降级）；删掉 `INJECT/HOOK_PROBE/SEL_SNAPSHOT/TREE/ERR_HOOK` 五段旧 JS；
            `pla-app cdp-probe` 一键注入 + 探测 + 只读回读；node 契约测试 11 项。
            真机：全部能力可用，editor 由 **Preact 遍历**找到（26.2.3 可用 → F4 重定位可免堆快照）。
      - [~] **F1b**（R21，契约测试 21 项 + 双窗口真机自检 0 FAIL；待用户手动回归）：
            写路径全部迁入垫片——`sel.cancelCommand/clear/addFaceById/faceIds`、`scan.begin/read/clear/end`（结束时还原 onHover，
            不再常驻包装）、`cam.navigateToSelection/render/vpRect/setOrbitStep/roll`、`keymap.setBindings`、`vp.save/restore`（存档格式兼容）。
            cdp.rs 删掉 13 段旧 JS（409 行），pla-ui 的两段即时生效 JS 改为 `CdpCmd::Call{接口名, JSON 参数}`；`__PLA_HOOK__` 别名删除；
            滚转命令名单一来源 = `shim::ROLL_*_COMMAND`。内部结构表把方法挂到所属对象根下（click/cs/exec/km/kb/reg），调用 this 正确。
            注册到 Plasticity 的命令回调改为**委托式**（触发时找当前 `__PLA__`），热替换后不再落到旧实例（pitfalls #36）。
            F3 自检补上 `vp.restore ← vp.save` 回环；选择写入走真实点击管线（会改变用户选择），自检不做，由契约测试覆盖。
            用户回归（2026-11-13）：方向键 / Alt 滚转 / 步进角 / 方向键开关 ✓；选择集跨实体只选中实体、视口恢复缩放滞后 → R21.1 修。
      - [~] **R21.1**（待用户复测）：
            · 选择集跨实体：实测 Plasticity 点击语义 = 点未选实体上的面先选中**整个实体**，再点一次才换成面（pitfalls #38）。
              新增下钻补点：垫片 `sel.faceIds{targets}` 判定"未选中但所属实体已介入"的目标，Rust 在原坐标再 shift+点，最多 3 轮；
              转视角补救的补点后同样下钻。拓扑 id → 所属实体的规则只在垫片 NAMES.topoOwnerPattern 一处。
            · 视口恢复缩放要动一下才生效：`cam.set` 写完没标记轨道控制器需要更新 → 加 `setNeedsUpdate()`（滚转共用同一函数）。
            · B2 视口状态的其他问题用户稍后细看。
- [~] **F2 能力探测**（R17，待用户确认设置面板观感）：连接时逐项探测所需内部结构 → 功能开关；面板明示失效项，杜绝静默失败。
      - pla-bridge：`shim::cap` 能力名（契约测试与 JS `CAPS` 对齐）+ `CapsReport`；`CdpEvent::Caps` 在注入后、
        ok↔noed 变化时、每 ~30s 重探（有变化才发），断开发 `None`。
      - pla-ui：`features.rs` = 功能 → 能力的唯一映射（11 项功能）；动作入口统一 `require_feature()` 拦截并提示原因
        （选择集保存/应用、视口状态保存/恢复；步进角/方向键的即时生效按能力判定）；
        工具栏状态点外环 = 能力健康度（绿 全部可用 / 黄 部分 / 红 无编辑器 / 无环 未连接），悬停有摘要。
      - R18（用户要求）：能力显示从设置面板移到独立的 **CDP 能力面板**，单击工具栏状态点开关（双击仍为归位）；
        面板 = 概览（连接/代理/编辑器/功能汇总/垫片/上次探测/重新探测）+ 按组（SELECTION / VIEW / INPUT / DOCUMENT）
        逐功能列出所需能力及缺失的内部结构 + ROADMAP 预留能力 + OTHER（未被功能引用的能力自动收录）+ ALT PATHS；
        分组与能力中文说明同在 `features.rs`。修复状态点悬浮提示被裁：HUD 高度按内容实测、文字换行、长标题截断。
      - R18.1（用户反馈"已连接但灯是红色"）：状态点内点改为 **CDP 连接状态**（灰 未绑定 / 黄 连接中 / 绿 已连接 / 红 未连上）；
        外环只在能力受损时出现（黄 部分可用 / 红 未找到编辑器）；Bridge（只读冗余）移到悬停提示与能力面板概览。
      - 真机（26.2.3）：外环为黄 —— 视口状态保存/恢复因 `vp.settings.*` 缺失而降级，其余全部可用。
- [x] **F3 契约测试**（R20，2026-11-13 真机：33 PASS · 1 WARN · 0 FAIL · 6 SKIP）：
      `pla-app cdp-selftest [--hwnd N] [--json 文件]`，退出码 0 通过 / 1 有失败 / 2 连不上。逻辑在 `pla_bridge::selftest`（pla-ui 以后可加按钮复用）。
      六段：传输（9334/9333/目标窗口/会话）→ 垫片（注入、接口版本、rev、接口清单）→ 能力（逐项，由能力表自动生成）
      → 只读（结构校验）→ 回环（`cam.set ← cam.get` 回读偏差、`cmd.ping` 注册→派发→回调）→ 守护（撤销栈/几何版本/选择/相机前后一致，
      变了即 FAIL）。选择与视口状态的回环待 F1b 迁入垫片后加入（现为 SKIP）。
      顺带：垫片新增 `cam.set`、`cmd.dispatch`、`cmd.ping`；cdp.rs 的相机读写（原 `CAM_SNAPSHOT_JS` / `PLA_VIEW_APPLY_JS_TMPL`）
      已全部改走垫片（F1b 的一部分）；命令注册表改为独立根 `reg`（LOC 表内不再出现硬编码路径）。
- [ ] **F4 Editor 自动重定位**：挂 `Page.loadEventFired` / target 变化，自动重新堆定位并重注垫片。
      R16 已知：代理不转发 CDP 事件、假 target id 固定 → 必须在主进程侧感知重载；
      垫片已能经 Preact 遍历自取 editor，重载后只要重注垫片即可，堆快照降为后备。
- [ ] **F5 锁版本**：关闭 Plasticity 自动更新，归档已验证版本安装包；升级流程 = 先跑 F3。
- [ ] **F6 堆快照解析**：JS 版一次性 `JSON.parse`，快照过大（>~500MB）会失败 → 改流式解析或加尺寸告警。

## N — 新功能

- [ ] **N1 材质 / 配色方案**：方案存 `<doc>/.pla/materials/<doc_key>`；CDP 调 `SetMaterialCommand` 族应用；
      PoC 先做单实体改材质，再做方案保存/切换。降级：改文档 JSON 头写副本后重开。
- [ ] **N2 命令派发层**（PRD-ai-modeling M-A）：带参数可靠执行 Plasticity 命令，产出命令/参数格式 know-how。N3/N4/N6 共用。
- [~] **N3 参数化修改（尺寸驱动 P1-1）**：现有 `pla-dim` 的设计是「网格顶点平移 + bridge PUT_SOME 写回」——
      **此路不通**，只存在于纸面：bridge 唯一的写入通道 PUT_SOME 是 Blender「Outbox → PolySplines」用的，
      把多边形网格转成光滑 NURBS 曲面（Studio 功能），不能改已有实体的尺寸，也会丢掉倒角等特征。
      重做：尺寸 = 面选择集 + 方向 + 数值，通过 N2 调 Plasticity 的移动面/偏移面类命令改值；支持记录、修改、重放。
- [ ] **N4 打孔工具**：把 SW 异型孔向导体验搬进 Plasticity。
      规格库（螺纹底孔 / 自攻孔按材料 / 螺套 / 热熔螺母 / 沉头 / 过孔，可录入 + 上传模型）→
      选面/点 → 自动放负体、定长度、布尔、放螺丝实例。一期可先自动化现有 asset 负体流程。
- [ ] **N5 常用件库**：常用件白名单入库（STEP + 缩略图 + 规格字段，SQLite 索引），PLAhelper 搜索面板一键放入当前文档；
      常用度按本人插入次数统计；商家具体型号用到时下载归档。
- [ ] **N6 AI 建模**：LLM 生成 build123d → STEP → 导入 Plasticity；N2 成熟后再试直接驱动 Plasticity 命令。
- [ ] **N7 轻量装配**：实例 + PLAhelper 记录装配关系 + 干涉检查 + 隐藏线/SVG 导出。完整配合约束和工程图不在范围内。
- [ ] **N8 交付出口**：STEP 导出 → SolidWorks / COMSOL；网页 3D（glTF + three.js 单文件）。

## 工程债

- [ ] 远端推送纳入每轮收尾（origin 已改 HTTPS，见 R12 后的推送记录）。
- [ ] `pla-dim` 旧实现与 mock 测试随 N3 重做时清理。
- [ ] 编译警告清理。

---

## 已废弃（2026-11-12）

旧 G2–G4 基于 8980 bridge 写回（PUT_SOME 建组、facet 写回、分块查询、嵌套 id 对齐）——bridge 下发只有网格，
唯一写入 PUT_SOME 只用于 PolySplines（网格 → NURBS 曲面，Studio），不能编辑已有实体，
这些路线全部由 CDP 主通道取代。bridge 保留作只读冗余数据源；PUT_SOME 可作为将来"外部网格 → Plasticity 曲面"的入口。
