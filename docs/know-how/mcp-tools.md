# pla-mcp 工具(A6 + 出图, 2026-11-17)

`src/target/release/pla-mcp.exe` = stdio MCP 服务器(JSON-RPC, 一行一条; 协议版本 2025-06-18 / 2025-03-26 / 2024-11-05)。
仓库根 `.mcp.json` 已登记(Claude Code 在本项目里首次使用时要批准); 别处: `claude mcp add plahelper -- <repo>\src\target\release\pla-mcp.exe`。

## 两类工具, 两条通路

| 工具 | 通路 | 需要 |
|---|---|---|
| `model_state` `model_snaps` `model_measure` `model_view`(只读)<br>`model_exec` `model_hole` `model_transaction`(改模型) | pla-mcp 直接经 CDP 9333 连 Plasticity(`pla_bridge::query::call_once` / `exec::run_once`) | Plasticity 以 CDP 模式运行; pla-ui 在跑时用它吸附的窗口(state.json 的 hwnd), 否则代理挑唯一的那个 |
| `status` `list_parts` `get_drawing` `apply_drawing_changes` `draw` `list_rules` `remember_rule` `show_part` | 读: 直接读出图目录的文件; 写 / 出图: 文件队列交给 pla-ui(`pla_ui::mcp_bridge`) | pla-ui 在跑(心跳 15 s 内); 零件清单空时自动让它扫一次 BOM(只读) |

## 后台实例(2026-11-18)

`background {action: start|status|stop, doc?}` 起一个独立、隐藏的 Plasticity(不闪现、不进任务栏、不抢焦点 / 鼠标), 之后任何 `model_*` 加 `instance: "background"` 就在它里面做; = 前台(用户正在用的)。细节 / 实测: know-how/background-instance.md。

## 建模的工作方式(给 agent 的要点)

1. 先 `model_state`(概览) → 再按物体查 `detail: faces/edges`; 面 / 边 id 形如 `34f9439`, **物体版本 id(34)每次改动都变**, 改完要重新查。
2. 改模型: `model_exec {ops}`, 默认 atomic(某步失败整次撤回)。配方(唯一来源 = 垫片 `NAMES.recipes`): box / cylinder(打孔 = subtract)/ fillet / chamfer / offsetFace / refillet / moveFaces;
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
