# 按数据恢复选择：经 executor 执行选择命令（2026-11-13 技术验证，26.2.3）

## 结论

选择集应用不必再走「悬停扫网格 → 真实鼠标点击」。**按 id 查到面对象，构造命中，在一个自建的选择命令里调用
`changeSelection.onClick`，经 `editor.enqueue` 执行**——结果与手点完全一致（Move 能正常接手选择），
且不碰屏幕、不受遮挡/视角/后台节流影响：

| 测试（demo1） | 结果 | 耗时 | 撤销栈 |
|---|---|---|---|
| 4 个实体各 1 面 | 4/4，无多余 | 10 ms | 每面 +1（逐个命令） |
| 30 面 / 14 个实体（含背面） | 30/30，无多余 | 86 ms | +30（逐个命令） |
| 40 面，**一个命令内完成** | 40/40，无多余 | **10 ms** | **+1** |
| 选中后启动 Move | MoveCommand → MoveFaceCommand 接手，选择保留；取消后仍保留，几何未变 | — | 不变 |

对比现行扫描管线：用户截图 16 面成功 5、11 项未扫到，耗时数十秒（后台窗口 3 分钟）。

## 关键事实（全部实测）

1. **真实点击 = 一条命令链**：`ClickChangeSelectionCommand` 经 executor 执行（`selectionDelta` 派发，撤销栈 +1）→
   `SelectionCommandManager`（`executor.commandForSelection`）挂默认后续命令：选中面 → `PushFaceCommand` → `OffsetFaceCommand`。
2. **`click.addById` 不是点击**：不产生命令、不派发 `selectionDelta`、不挂后续命令；且 `addById('2:<vid>')` 写进选择的是
   **畸形条目** `"<vid>:undefined"`（现行补救路径 `sel.addFaceById` 因此在写垃圾）。这就是当年"缺活跃实体上下文"的真相。
3. 点击命令字段 `(editor, intersection[], modifier, option)`；命中结构 `{distance, point, intersectable: Face, object: Face, distance2d}`。
   - modifier：0 替换、1 追加、2 移除、3 切换。
   - option：2 = 选中一串关联面（相切链?），4 = 取消选择；1/8/16 同 0。
4. 面对象：`editor.geo.lookupTopologyItemById(versionId).view`（`Face`，有 `versionId`、`parentItem`、`getBoundingBox()`）。
   实体：`editor.geo.items[i] = {view: Solid, model: SolidBody}`，`view.faces`（FaceGroup）含 `faceIds`、逐面 `bboxes`、网格。
5. `ClickChangeSelectionCommand` 类**只在闭包里**（editor 对象图里没有引用，`ed.commands` 不含）。
   但已注册的选择类命令（`DeselectAllCommand` 等）共享基类 **`SelectionCommand`**，可从其原型链拿到 → 自建子类即可，
   不需要先截获真实点击。自建子类不会触发后续的 OffsetFace（`SelectionCommandManager` 只认原生点击命令），
   对恢复选择集反而更好（不弹偏移 gizmo）。
6. 活动命令（如 OffsetFace）在等待输入时，新入队的命令会**排队不执行** → 入队前先 `executor.cancelActiveCommand()`。
7. `editor.enqueue(cmd)` 的 Promise 要等**整条命令链**结束才兑现（后续命令等输入就永远不兑现）→ 不能 await，
   改为监听 `commandEnded` 判定本命令结束。
8. `command:move` 是挂在 `plasticity-viewport` 上的**选择器式**监听（不在 inline 表里）→ 派发要以视口元素为目标。
9. 点击语义（ClickStrategy.topologicalItem）对合成命中同样生效：未介入实体上的面可能先选中实体 → 在同一命令内检查、
   需要时再 onClick 一次下钻（本轮 40 面实测 0 次下钻，保留为兜底）。

## 落地方案（待用户确认后实施）

- 垫片新增 `sel.apply({ items: [{kind, id}], replace })`：
  - 按 id 查视图对象（面/边 `lookupTopologyItemById`，实体 `lookupItemById`），查不到的列入 missing；
  - 自建 `SelectionCommand` 子类，在一个 execute 里逐项 `changeSelection.onClick`（首项替换或追加、其余追加，未选中则下钻一次）；
  - 入队前取消活动命令，监听 `commandEnded` 等本命令结束，回读并返回 `{selected, missing, extra, drills}`。
  - 类与方法全部经 LOC 表（`ed.geo`、`cs.onClick`、`exec.cancel`、基类来源 `ed.commands.DeselectAllCommand`）。
- Rust：`ApplySel` 改为一次 `sel.apply`；删除扫描 / 点击 / 遮挡补救 / 下钻补点 / 相机分组整套（约 400 行）与
  `scan.*`、`sel.addFaceById`（写畸形条目）。保存时的相机仍随集存储，作为"应用后可选切到保存视角"的附加功能。
- F3 自检：可以加选择回环了（保存当前选择 → `sel.apply` 原样恢复 → 比对），不再改变用户选择。
- 这同时是 **N2 命令派发层**的第一块：自建命令子类 + `editor.enqueue` + `commandEnded` 判定，材质 / 打孔 / 参数化修改复用同一套。

## 待验证（实施时一并测）

- 边、实体、曲线等其他类型的命中构造（`onIntersection` 按命中对象类型分派）。
- 隐藏对象、被锁定对象的行为；文档几何变化后旧 id 失效的报告。
- 撤销（Ctrl+Z）一次是否恢复到应用前的选择。
