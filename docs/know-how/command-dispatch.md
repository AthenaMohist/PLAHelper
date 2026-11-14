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

## 待做(设计见 TODO N2)

- 类发现: 运行时一次性堆快照(Rust inspector 通道)还是别的途径; 结果缓存到 renderer 重载为止。
- 配方表(单一来源): 对外操作名 → 工厂类名 + 参数类型(point / length / angle / count / edges / faces / bool), 由类型决定单位换算与对象构造。
- 拓扑选择: fillet / extrude / offset 需要边/面对象 —— 按 id(同 sel.apply 的 lookupTopologyItemById)或按几何条件(边中点、面法向)查找。
- M-A 验收: 脚本画 L 形支架(2 box 合并 + 1 fillet)。
