# pla-mcp 工具(A6 + 出图, 2026-11-17)

`src/target/release/pla-mcp.exe` = stdio MCP 服务器(JSON-RPC, 一行一条; 协议版本 2025-06-18 / 2025-03-26 / 2024-11-05)。
仓库根 `.mcp.json` 已登记(Claude Code 在本项目里首次使用时要批准); 别处: `claude mcp add plahelper -- <repo>\src\target\release\pla-mcp.exe`。

## 两类工具, 两条通路

| 工具 | 通路 | 需要 |
|---|---|---|
| `model_state` `model_snaps` `model_measure` `model_view`(只读)<br>`model_exec` `model_hole` `model_transaction`(改模型) | pla-mcp 直接经 CDP 9333 连 Plasticity(`pla_bridge::query::call_once` / `exec::run_once`) | Plasticity 以 CDP 模式运行; pla-ui 在跑时用它吸附的窗口(state.json 的 hwnd), 否则代理挑唯一的那个 |
| `status` `list_parts` `get_drawing` `apply_drawing_changes` `draw` `list_rules` `remember_rule` `show_part` | 读: 直接读出图目录的文件; 写 / 出图: 文件队列交给 pla-ui(`pla_ui::mcp_bridge`) | pla-ui 在跑(心跳 15 s 内); 零件清单空时自动让它扫一次 BOM(只读) |

## 后台实例(2026-11-18)

`background {action: start|status|stop, doc?}` 起一个独立、隐藏的 Plasticity(不闪现、不进任务栏、不抢焦点 / 鼠标), 之后任何 `model_*` 加 `instance: "background"` 就在它里面做; 不加 = 前台(用户正在用的)。
`background` 还有 open / save_as(文档); 对后台文档的作业: `background_bom` / `background_draw` / `background_release`。
**前台开着的同名文档一律拒绝**(打开 / 另存为 / 作业), 报警并提示用户先在前台关掉。细节 / 实测: know-how/background-instance.md。

## 文档作业(2026-11-18, 前台 / 后台通用)

用户要求: 用语言描述就能完成任务, 所以按"操作"补齐 MCP, 不按界面 1:1。`doc`: `foreground`(默认, pla-ui 绑定的文档; pla-ui 没开就看前台窗口标题)/ `background`。
数据直接读写文档的 `.pla/…` 存档(逻辑 = `pla_ui::doc_jobs` / `bom_table` / `release`, 与 pla-ui 共用); pla-ui 按修改时间重读 BOM 填写与供应商, 界面马上看得到。

| 工具 | 做什么 |
|---|---|
| `bom_table` | 算好的 BOM: 列(id / 表头 / 可编辑 / 公式)、行(key、各列值、哪些是人工改的、归哪家、出图状态)、合计、排除、文档默认值、模板 / 供应商清单 |
| `bom_edit` | `set` 单元格(零件 = key / 料号 / 名字; 列 = id 或表头, 供应商自有列也行; null = 恢复)、`assign` 归哪家(id / 名字 / buy / "")、`exclude` / `include`、`add_rows` / `remove_rows` 手动行、`defaults`(如套数)、`template`、`project` |
| `bom_export` | 导 CSV |
| `draw_parts` | 直接出零件图(导 STEP + 编译, 两张以上合订); 前台会临时换选择再还原, 用户在用时优先 background 或排队的 `draw` |
| `release` / `release_history` | 外发包(版本 / ECN / 重出 ECN 涉及的图 / 总 BOM + 各家包; 报没分配供应商的零件)/ 版本记录 |
| `suppliers` | 供应商(全局): list / add / update / remove(要的文件、主表列、自有列) |
| `configs` | 配置: list / apply / save |
| `parts_library` | 零件库(全局): search / get / add(按分类 + 项目自动编号, 分类不给就猜)/ update(字段、供应商价格自动记日期、首选、改分类 = 重新编号旧号进 former)/ remove; 与 BOM 行: link / from_bom(把行存成新条目并链接)/ suggest |
| `bom_template` | BOM 模板(全局): list / get / create(from 模板名或 builtin)/ rename / remove(最后一个不删)/ edit(add_columns 带 after、update_columns(null = 清掉该项)、remove_columns、order、columns 整体替换)。保存前体检: id 唯一、来源合法、公式只引用存在的列、无循环。文档按名字选模板(`bom_edit template`, "" = 默认); **默认 = 按名字排第一个**, 新建的名字排到最前会变成没选过模板的文档的默认(create 回包会提醒)。删列不删文档里该列的填写 |
| `hole_table` | 孔规格方案(全局): list(含当前)/ get / create(默认复制当前)/ rename / remove / select(写偏好 hole_preset, pla-ui 孔面板 1 秒内跟着切)/ edit(set 按规格改或加, 新规格五个数都要给; remove_sizes; 按公称直径排序; > 0 校验, 可疑值给 warnings)。`model_hole` 可给 `size` + `usage` + `table`, 数值从方案里填(显式给的优先; 组合孔 d = 过孔、tapD = 底孔) |
| `kb_search` / `kb_get` / `kb_run` / `kb_propose` / `kb_sources` | 知识库接口(2026-11-19, 不依赖 Plasticity; `pla_ui::kb`): helper 不内置知识库, 用户在 `<数据目录>/knowledge.json` 里接自己的源 ——
`local`(目录格式见 mech-kb/SCHEMA.md: 卡片 / 外部资源 / 计算工具 / 草稿)、`command`(任意系统包一层脚本: stdin 一行 JSON search / get / run)、`ima`(腾讯 ima, 只按标题搜)。
agent 先 `kb_search`, 有工具就 `kb_run`(输入校验 + 每步公式与出处), 没有就手算注明出处, 做完可 `kb_propose` 把这类计算写成工具草稿(只进 proposals/, 用户审核后挪进 tools/)。
范例库 `PLAHelper/mech-kb`(独立 git): 5 个已校核工具(配合偏差、螺栓预紧 / 力矩、轴承寿命、轴最小直径、平键挤压, 36 个参考用例)+ 卡片 + mechtool.cn / awesome-mecheng 的资源链接 |
| `dims` | 尺寸记录与参数: list; `set`(按名字改尺寸, 事务内)/ `set_param`(改参数, 绑定的尺寸跟着改)经 pla-ui 在前台文档上做 |

实测(后台 bgtest): 读表 → 改料号 / 表面处理 / 归 CNC·3D 打印·外购 / 套数 2 / 加手动行(公式跟着算)→ 外发 R2: ECN-001(3 项)、重出 2 张图、CNC 包(PDF/DXF/STEP + BOM, 带表面处理)、3D 打印包(STEP + BOM)、zip;
配置存 / 列 / 切; 前台 dims 列表。前台(model1): 零件库 search / add / update / from_bom / link / remove(测试条目已删)、draw_parts(导出后若还原成空选择, 取消 Plasticity 自动起的移动命令)、release R1、model_exec 直接改模型。
模板 / 孔规格(2026-11-18): 建测试模板加 密度 / 体积 / 质量(公式 + 合计)列 → 套到 model1 BOM, 质量按 体积 × 密度 × 总数量 算对、合计行有 → 坏引用 / 循环被拒 → 换回默认、删测试模板; 测试孔规格方案改 / 加 / 删规格、新规格缺数被拒、select 后偏好保持(pla-ui 没回写)→ 换回 ISO metric、删测试方案。
用户 2026-11-18: 选择集 / 视口 / 材质状态 / 快照是人操作才需要的, 不做 MCP; 改模型直接用 model_* 工具, 不必经尺寸记录。

## 建模的工作方式(给 agent 的要点)

1. 先 `model_state`(概览) → 再按物体查 `detail: faces/edges`; 面 / 边 id 形如 `34f9439`, **物体版本 id(34)每次改动都变**, 改完要重新查。
2. 改模型: `model_exec {ops}`, 默认 atomic(某步失败整次撤回)。配方(唯一来源 = 垫片 `NAMES.recipes`): box / cylinder(打孔 = subtract)/ fillet / chamfer / offsetFace / refillet / moveFaces / revolve / prism;
   整体操作(2026-11-19): duplicate / move / rotate / mirror / pattern / radialPattern / boolean(已有实体之间)/ delete / rename(know-how/command-dispatch.md「A4b」);
   大纲(2026-11-19): `model_outline` 读树(文件夹嵌套 / 隐藏 / 锁定); `model_exec` 的 folder / moveTo / hide / show / ungroup / rename(folders)改树(know-how/command-dispatch.md「A4c」);
   步骤 `id` + `$id` 引用; 挑边 / 面可用 `{near, of, tol}`。参数错在执行前就报, 不产生命令。
3. 多次调用要能整体撤回: `model_transaction begin` → … → `commit` / `rollback`。
4. 核对: `model_measure`(距离 / 壁厚 normalDistance / 角度 / 体积)+ `model_view`(截图后还原相机)。

## 坑

- `model_exec` 会**取消用户正在进行的命令**; 窗口不可见时 atomic 一开始就拒绝(撤销走 memento, 隐藏窗口不行)—— 只在用户要求建模时用。
- 第一次 `model_exec` 要挂工厂类(堆快照, 实测 93 MB / 10 s), 之后同一页面零等待。
- `model_view` 只对准、不隔离: 埋在别的零件里的物体(如装配内部的磁环)从外面看会被挡住。
- 截图: 同一个同步调用里摆完相机立刻渲染, 窗口可见时画面还是旧相机(空渲一帧也没用)→ 分三步: `cam.view {shot: false}` 摆相机 →
  隔 150 ms 单独 `cam.thumbnail` → `cam.viewRestore` 还原(`pla_bridge::query::view_png_once`; 截完相机与截前逐字段相同)。
- 事务记录原来在垫片实例上, begin 之后垫片热替换(版本变了)就丢 → 改记在页面级 `__PLA_PAGE__`。另加 `hist.undo {steps}` = 撤销最近 N 步。
- 倒圆距离远大于板厚(4 mm 板 R30)Plasticity 也不报错、包围盒不变 —— 不能拿它当"必然失败"的测试步; 用不碰目标的 subtract 圆柱("Operation has no effect")。
- 输出是 UTF-8; 在 PowerShell 里直接管道看会乱码(客户端正常)。

## 真机(2026-11-17, encoder calibrator bkp, 只读)

tools/list 15 个; `model_transaction state` → undo 1; `model_state` 概览 0.5 s; 磁环(物体 3)`detail: faces` 4 面、`mass` 体积 271.46 / 面积 985.98、
上下两面 `distance` normalDistance 0.6237; `model_view` iso / front / top 对准物体、相机还原; `model_exec` 未知 op → 执行前报错、未改模型。
改模型的路径(exec / hole / rollback)没在这份文档上跑(不是测试文档 model1)。

## P1 验收(2026-11-17, model1, 全程经 pla-mcp)

起点: 26 个实体, 撤销栈 0。`model_transaction begin` →
1. `model_exec` L 形支架(底板 40×20×5 + 立板 5×20×30 合并 + 内角 R3)3 步 0.86 s → 实体 83, 包围盒 200–240 × 0–20 × 0–30;
2. `model_state detail:faces` → 8 平面 + 1 圆柱(R3), 底板顶面 `83f2667705`;
3. `model_hole` ⌀5.5 通孔 ×2(at 215 / 233)→ 实体 84 → 85, 回包直接给新的入口面 id(下一孔接着用);
4. `model_snaps circleCenter` 取孔口圆心 (215,10,5) / (233,10,5) → `model_measure` 孔距 18.000; 底板 normalDistance 5;
5. `model_view iso` 对准支架(两孔、R3 清楚), 相机还原;
6. 失败步: `model_exec` 盒子 + 不碰目标的 subtract 圆柱 → 第 2 步 "Operation has no effect", 本次 1 步撤回(0.22 s), 撤销栈不变;
7. 收尾: 事务记录因垫片热替换丢失(已修, 见上)→ 用新 `hist.undo 7` 撤回(0.94 s)→ 26 实体 / 撤销栈 0;
   修好后重跑 begin → 两次 exec(3 步)→ rollback 撤 3 步 → 26 实体 / 撤销栈 0。
