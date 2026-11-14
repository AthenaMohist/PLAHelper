# N2 命令派发层: 实测结论(26.2.3, 2026-11-14, MT_screws 副本)

## 结论(可行性已验证)

不模拟点选/键盘, 直接用 Plasticity 自己的**几何工厂**(`*Factory`)造/改几何:
`new XxxFactory(editor)` → 设参数(setter)→ 在**自建 Command 子类**的 `execute()` 里 `await factory.commit()` →
`editor.enqueue(cmd)`。结果与手工操作同路径: 进撤销栈(+1)、`databaseChanged` / `commandFinishedSuccessfully` /
`renderPrepared` 正常派发、视口正常绘制。

## 实测事实

1. **命令类可直接拿到**: `editor.commands` 是 187 个命令类的对象(`CornerBoxCommand`、`CylinderCommand`、`ExtrudeCommand`、
   `FilletCommand`…)。但命令的 `execute()` 是交互式的(等点选/gizmo), 不适合脚本驱动: 入队 `CornerBoxCommand` 后
   400 ms 内只注册了一个 CancellablePromise(点选器), 工厂要等第一次点选后才创建。
2. **工厂类在闭包里**, 编辑器对象图里没有引用(`editor.factoryCache` 为空 Map)。**堆快照可一次全部拿到**:
   `HeapProfiler.takeHeapSnapshot`(本例 62 MB, ~5 s)→ 按闭包名匹配 `/^[A-Z]\w*(Factory|Picker|Command)$/` →
   同一会话 `HeapProfiler.getObjectByHeapObjectId` 复活 → `Runtime.callFunctionOn` 挂到 window。
   本例 557 个类全部复活成功, 其中工厂 229 个(实验脚本: 会话 scratchpad 的 heap_factories.js, 走法同 tools/pla-cdp 找 editor)。
   注意: 快照 id 跨会话作废; renderer 重载后要重新做。
3. **工厂构造参数 = editor**(实例自有字段 `editor`); 传 `(db, materials, signals)` 会在 commit 时抛
   "reading 'factoryStateChanged'"。
4. **参数是 setter**(原型上 `name=`), 例:
   - `CornerBoxFactory`(继承 DiagonalBoxFactory → BooleanBodyFactory): `p1`、`p2`(底面对角)、`height`; `info` 回读 `{x,y,z,location}`;
     布尔: `operationType`、`targets`、`keepTools`(造盒子时直接与已有实体合并/相减)。
   - `CylinderFactory`: `radius`、`height`、`toggleMode`(+ 布尔同上)。
   - `ExtrudeFactory`(AbstractSweepFactory): `region(s)`、`curves`、`faces`、`edges`、`distance1/2`(待核)、`thickness`、`individual`、`degrees1`。
   - `MultiFilletShellFactory`: `edges`、`distance`/`distance1/2`、`shape`、`rho`、`chamferMode`、`filletMode`、`prolong`。
   - `OffsetFaceFactory`(MultiModifyFaceFactory): `faces`、`distance`(待核)、`degrees`、`reference` —— N3 尺寸驱动要用。
   - 所有工厂共有 `update()`(预览)、`commit()`、`cancel()`、`resource(cmd)`(随命令释放)。
5. **工厂在命令外 commit**: 几何进库(`geo.items` +1), 但**不进撤销栈**。必须包在 Command 里。
6. **自建命令**: 基类 = `Object.getPrototypeOf(editor.commands.CornerBoxCommand)`(即 `Command`);
   `class X extends Base { async execute() { const f = new F(this.editor).resource(this); …; await f.commit(); } }`;
   `editor.enqueue(new X(editor))`; 以 `signals.commandEnded` 判定结束(同 sel.apply, 不 await enqueue 的 Promise)。
7. **内部长度单位 = 米**: 文档单位 Millimeter 时, M 级螺钉零件包围盒 ~0.034 × 0.08。脚本按文档单位给参数, 派发层换算
   (×0.001 for mm)。第一次实验按"毫米数"直接给了 20/10/5 → 造出 20 m 的盒子, 在视口外看不到。
8. **验收截图**: 测试窗口被用户窗口遮住时, CDP `Page.captureScreenshot` 返回冻结的旧帧(两次字节完全相同);
   用垫片 `cam.thumbnail`(页面内渲染一帧读 canvas)才是当前画面。

9. **几何内核是 Parasolid**(报错前缀 `PK_`): 枚举取 PK 常量。布尔 `operationType`: 合并 = `PK_boolean_unite_c` 15903
   (实测; 给 2 报 `PK_ERROR_field_of_wrong_type`), 相减 15902、相交 15901(同一枚举, 未单独实测)。
   `CornerBoxFactory` 设 `targets = [实体视图]` + `operationType = 15903` → 新盒子与目标合并成一个实体, 目标被吃掉。
10. **拓扑 id**: 字符串 `<物体版本id>e<边id>` / `<物体版本id>f<面id>`(如 `31e9439`), 取自实体视图
   `view.edges.versionIds` / `view.faces.versionIds`, 与同组 `bboxes`(每项 6 个数: min xyz + max xyz, Float32)同序。
   组里的 `edgeIds`/`faceIds` 是裸数字, 直接给 `lookupTopologyItemById` 会抛 "object null is not iterable"。
   物体版本 id = `view.userData.versionId`(`lookupItemById` 用)。
11. **工厂源码看不到**: 打包成字节码(.jsc), `Function.prototype.toString` 只回空白 → setter 名从原型链读, 取值靠实验。
12. **页面里没有 webpack 模块表**(无 `webpackChunk*` 全局、无 `require`), 堆快照是拿到工厂类的唯一途径。

## 实现(2026-11-14)

链路: 脚本 → Rust `exec::run` → 垫片 `cmd.exec` → (首次) `need-factories` → 代理 `PLA.exposeClosures` → 重试。

- **垫片 `cmd.exec`**(`pla-shim.js` N2 段): 参数 `{ops: [...], units?}`; 每步 = 配方表里的工厂, 在自建
  `PlaFactoryCommand`(基类 = `CornerBoxCommand` 的父类)里 `new F(editor)` → `resource(cmd)` → setter → `commit()`,
  经 `runCommand`(取消活动命令 → enqueue → 等 `commandEnded`)执行。某步失败即停, 已完成的保留, 如实返回:
  `{units, results: [{index, op, id, items: [{kind, id, name, bbox}]}], failed: null | {index, op, err}}`。
  参数错(未知 op/参数、缺必填、bool 值不对、单位不认识)在执行任何一步之前就报, 不产生命令。
- **配方表** `NAMES.recipes`(唯一来源): 操作名 → 工厂表键(`LOC fac.*`) + 参数 `对外名: [setter, 类型]` + 必填 + 是否支持布尔。
  类型: point(文档单位 → 米, Vector3) / length / angle(度 → 弧度) / number / bool / edges / faces / solids。
  现有: `box {p1, p2, height}`(+ `bool: union|subtract|intersect`, `targets`)、`fillet {edges, distance}`。
- **引用**: 一步写 `id: "base"`, 后面用 `"$base"` 引用它的产物; 被后续步骤吃掉(如合并)再引用会报 `no longer exists`。
  也可给物体版本 id 或 `{name: 大纲名}`。
- **挑边/面**: 拓扑 id 字符串, 或 `{near: [x,y,z], of?: 物体引用, tol?: 容差(默认 0.5 文档单位)}` —— 取包围盒中心最近的一项;
  超出容差或两项一样近都报错, 不猜。
- **单位**: 默认读 `document.settings._Unit.length`(millimeter/centimeter/meter/inch/foot), `units` 可覆盖。
  回读 bbox 换回文档单位, 保留 7 位有效数字(bboxes 是 Float32)。
- **工厂类挂载**: 代理(`pla-cdp-v8.js`)自定义 CDP 方法 `PLA.exposeClosures {pattern, global, force}`, 与编辑器定位共用
  单飞队列(堆快照绝不并行)。拍快照(主进程收块) → 按闭包名匹配 → 复活: 只按 id 复活一个类, 沿原型链到类层次的根,
  `Runtime.queryObjects(根)` 一次拿到全部子类(`getObjectByHeapObjectId` 每次扫整个堆, 逐个复活 229 个要 73 s)。
  实测 64 MB 快照: 快照 2.7 s + 解析 0.2 s + 复活 5 s(10 次扫堆) = 8 s; 之后同一页面零等待(已有就直接返回 cached)。
  页面重载后全局没了, 下次使用时自动重做。常量(全局名、正则、方法名、错误文字)在 Rust `shim.rs`, 经配置注入垫片。
- **Rust**: `exec::run`(need-factories 自动挂载重试)、`exec::summary`(双语状态栏文字)、`CdpCmd::Exec` → `ExecResult` + `Report`;
  CLI `pla-app cdp-exec [--hwnd N] 脚本.json`(退出码 0 = 全部完成)。

### 例: L 形支架(M-A, 2026-11-15 副本实测通过)

3 步 0.3 s(工厂已挂上), 撤销栈 +3。倒圆用 `MultiFilletShellFactory`(`edges = [边视图]`, `distance` 米), commit 返回
`[新 Solid]`(替换原实体, 版本 id +1)。结果 8 个平面 + 1 个圆柱面, 圆柱面包围盒 205–208 × 0–20 × 5–8 mm(R3), 内角原边消失。
核对方法: 读实体 `faces.versionIds` → `lookupTopologyItemById(id).model.GetSurface().surface` 的类型 + 逐面 bbox;
截图用垫片 `cam.thumbnail`(先 `cam.set` 正视, 截完还原)。

```json
{ "ops": [
  { "op": "box", "id": "base", "p1": [100, 0, 0], "p2": [140, 20, 0], "height": 5 },
  { "op": "box", "id": "L", "p1": [100, 0, 0], "p2": [105, 20, 0], "height": 30, "bool": "union", "targets": ["$base"] },
  { "op": "fillet", "edges": [{ "near": [105, 10, 5], "of": "$L" }], "distance": 3 }
] }
```

## A3 事务回滚(2026-11-15)

- **撤销路径**: 在 `document.body` 上派发 `CustomEvent('edit:undo', {bubbles: true})` —— 与 Ctrl+Z 同一条路径
  (命令注册在 inline 监听里, WeakMap 键是某个祖先元素, 冒泡即可命中)。**异步**: 实测 300 ms 内撤销栈 −1、重做栈 +1、物体恢复;
  垫片按撤销栈长度轮询确认(50 ms / 步超时 5 s)。也有 `editor.history.undo()`, 但没走命令系统, 不用。
- 撤销栈项 = `{name, snapshot}`(memento); 自建工厂命令的项名是 `PlaFactory`。撤销走 memento 恢复 → **窗口不可见时拒绝**(`mementoGuard`)。
- 垫片(能力 `hist.write`):
  - `hist.begin` → `{depth}`: 记下撤销栈深度 + 每一项的对象引用; `hist.commit` → `{committed, steps}` 结束事务;
  - `hist.rollback`(异步) → `{undone, depth}`: 撤到 begin 时的深度。**安全检查**: begin 以下的撤销栈必须原样
    (用户撤到 begin 之前、或撤了又做了别的 → 拒绝, 一步都不撤)。会撤掉 begin 之后的全部步骤, 包括期间用户自己的操作。
  - `cmd.exec {atomic: true}`: 某步失败 → 撤回本次全部已完成步骤, 回包 `results: []` + `rolledBack: n`(撤回失败则 `rollbackErr`);
    窗口不可见时一开始就拒绝。CLI: `pla-app cdp-exec --atomic`。
- 真机(MT_screws 副本): 3 步脚本第 3 步倒圆被内核拒绝(`PK_BODY_fix_blends` 16067) → 前 2 步撤回, 前后撤销栈 12 / 实体 31, 共 270 ms;
  `hist.begin` → 2 次 `cmd.exec` → `hist.rollback` 撤 2 步 102 ms, 回到原样。

## N3 改尺寸用到的工厂(2026-11-15 副本实测)

- `OffsetFaceFactory`(→ MultiModifyFaceFactory → MultiModifyShellFactory): `faces = [面视图]`, `distance`(米)。
  **正 = 沿外法向长出去**(10 mm 高盒子顶面 +2 → 12)。圆柱面: 轴线不动、半径变(孔 = 凹, 外法向指向轴线 → 正值让孔变小)。
  对**倒圆面**用它: 半径变了但圆角鼓出两侧平面(包围盒变大)—— 不是 CAD 意义的改圆角。
- `RefilletFaceFactory`: `faces`, `distance` = **半径增量**(R2 + 0.003 → R5), 保持与两侧面相切(包围盒不变)。
- 区分倒圆面: 面模型 `GetBlendRadius()` → 倒圆面返回半径(米), 普通圆柱返回 −1。
- 垫片 `dim.face` 据此返回 `{kind: thickness|radius|filletRadius, value, op: offsetFace|refillet, offsetSign}`,
  改值 = `cmd.exec {atomic: true, ops: [{op, faces: [面], distance: (新 − 旧) × offsetSign}]}`。

- `MultiMoveFaceFactory`(→ MoveRotateScaleFactory2 → MultiTransformFaceFactory): `faces`, 平移要**同时**设
  `move`(实例上的 Vector3, 用 `.set`)和 `matrix`(Matrix4, `.makeTranslation`)。只设 move: commit 返回 null、无效果;
  只设 matrix: 同样无效果; `push()` 报 "Operation has no effect"; `prepare()` 抛 "reading 'elements'"。
  配方 `moveFaces {faces, move: [dx,dy,dz]}`(参数类型 translation)。方槽 5 个面一次平移, 孔形不变。
- 布尔相减 `operationType = 15902`(subtract)实测可用(盒子挖方槽)。

## 加一个新操作

1. 副本上读工厂原型链的 setter 名(本文 4.), 用 lab 脚本试出参数取值与 commit 返回值;
2. LOC 加 `fac.<名>`, NAMES.recipes 加一行; 契约测试的假工厂补上;
3. 副本上用 `pla-app cdp-exec` 跑一遍, 量包围盒/截图核对。
