# 前车之鉴：PLAHelper 建模任务的坑与未解难题

> 记录人：建模执行 Agent（WorkBuddy 会话）
> 日期：2026-11-20 ~ 2026-11-21（两天，两轮任务）
> 目的：把踩过的坑和没解决的问题写清楚，让下一个 agent 不要重蹈覆辙。
> 范围：repo 内算法/脚本一行未改；本文件是唯一的经验沉淀。

---

## 0. 一句话总结

> **这个工具链"能建模"，但缺少安全网。**
> 读操作（几何查询、测量）非常可靠；写操作一旦失败，**没有可靠的回滚路径**。
> 我在第一天把「渲染异常」误诊为「相机问题」，实际是场景挂载被 `orig.restore()` 破坏；
> 第二天用 3 次 `undo` 去"试错修复"，反而把 Plasticity 的几何工厂彻底打死，
> 最终文件没坏、但内存中已完成的工作全部丢失。

**给下一个 agent 的第一条建议：动手写模型之前，先把 `encoder calibrator.plasticity` 复制一份到安全目录。
我的备份策略是"写操作前复制"，但真正救命的备份是"每次大批量写操作前复制"。**

---

## 1. 连接与工具链（这部分是可靠的，可以直接复用）

### 1.1 MCP 接入

- `pla-mcp.exe` 是 stdio JSON-RPC 服务器，41 个工具。
- WorkBuddy 只读 `~/.workbuddy/mcp.json`，**不读**项目根的 `.mcp.json`（那是 Claude Code 格式）。
- 我用自写客户端 `C:\Users\frg\AppData\Local\Temp\pla_mcp_client\pla_client.py` 直连，
  支持 `list` / `schema <tool>` / `call <tool> '<json>' [out.png]`。
- 关键实现细节：
  - **写操作前必须 `warm_up()`**：垫片有 `mementoGuard`，`document.visibilityState === 'hidden'` 时
    拒绝一切写操作（报 `window hidden` / `atomic needs a visible window`）。
  - `warm_up()` = CDP 求值 + Windows 置前（`SetWindowPos` TOPMOST→撤除→`SetForegroundWindow`）双保险。
  - **JSON 提取要扫描所有平衡 JSON 块**，取第一个含 `results`/`failed` 键的。
    用 `reversed(starts)` 会抓到 `items` 里的小对象（`{'bbox','id','kind','name'}`），
    导致 `results` 数恒为 0 —— 我在这上面浪费了时间。

### 1.2 可靠的读接口

| 用途 | 用法 |
|---|---|
| 实体清单 + 包围盒 | `model_state {limit:700}` → `geometry.solids[]` |
| 大纲树（实体名、隐藏状态、层级） | `model_outline {}` |
| **面级几何**（半径/轴线/平面法向） | 垫片 `geo.query {of:[id], detail:"all", limit:1500}` |

`geo.query` 是唯一能拿到面级特征的接口（`model_state` 的 `detail` 不吐 `faces[]` 明细）。

---

## 2. 写操作（model_exec）的 7 个坑

### 坑 2.1：`rotate`/`move` 之后**实体 id 会变**

```
model_exec {ops:[{op:"rotate", ..., solids:[3172], ...}]}  → 返回 id = 3460（是新实体）
model_exec {ops:[{op:"move", solids:[3460], ...}]}        → ❌ solid not found: 3460
```

变换类 op（move/rotate/mirror/pattern/radialPattern）在 Parasolid 侧是**重建实体**。
**必须回读 `results[].items[].id`**。

### 坑 2.2：`move` 可能"回包解析失败但操作已生效"

遇到过 `垫片回包异常: EOF while parsing a value at line 1 column 0`，
事后 `model_state` 显示位置**已经变了**。我据此重试，把 M5 螺钉平移到了 y=-5715。

**规则：写操作报错后，先用 `model_state` 核实实际状态，再决定是否重试。**

### 坑 2.3：`boolean` 不接受 `$id` 步骤引用

```
{op:"boolean", targets:["$a"], tools:["$b"], ...}  → ❌ unknown ref $cone
```
只接受真实 id。分步执行，从 `results[].items[].id` 取真实 id 再传给下一步。

### 坑 2.4：`revolve` 在有既存实体的位置会选错区域

想生成 ⌀38 的承托台，实际拿到 ⌀10（差一个数量级）。
根因：垫片 `revolve` 内部用 `pickNewRegion(before, mid, k)` 挑"离 profile 点集中点最近的区域"，
在有既存实体处会命中**旧实体的面**。

**规则：全部改用 `cylinder` / `prism`，`revolve` 只在空旷区用。**

### 坑 2.5：`box` 的 `p1`/`p2` 是正交两条边的起点，不是对角点

```
{op:"box", p1:[-30,336.971,17], p2:[30,396.971,21], height:1}
→ ❌ PK_ERROR_vectors_not_orthogonal (5027)
```
`box` 很难用对。**造矩形用 `prism`**：

```
{op:"prism", center:[cx,cy,z0], normal:[0,0,1], sides:4, flats:对边距, height:h, rotate:45}
```

**坑 2.6：`prism` 的 `rotate` 必须按对边朝向选**

- `rotate:0` → 对角线朝向 X/Y，实际边长 = flats × 1.414（我以为 60，实际 84.85）
- `rotate:45` → 对边平行于 X/Y，截面才是 flats × flats 的正方形

### 坑 2.7：`delete` 会污染 Parasolid stable model

删除实体后，后续写操作报 `invalid precondition: object N missing from stable model`，
必须 `model_transaction {action:"undo", steps:N}` 撤销一步恢复。

**规则：不要 delete。** 过程废料用 `move` 移到远处（y=14000+）+ `rename` 加 `_废<id>` 后缀。

---

## 3. 几何判读的 2 个坑（我因此差点误改正确几何）

### 坑 3.1：`geo.query` 圆柱面的 `center` **不是轴心**

同一个 ⌀4.5 腰孔端圆：
```json
{"surface":"Cylinder","radius":2.25,"axisOrigin":[0,306.971,83.001],"axis":[0,1,0],
 "center":[-1.7,312.971,83.001]}
```
`center` 偏了**一个半径**。按 `center` 算出来 4 组腰孔 PCD 是
`[46.13,54.11] / [42.60,50.60] / [49.40,57.40] / [46.13,54.11]` —— 看起来"±X 两组不对称，是建错了"。

**按 `axisOrigin`（或 bbox 中心减去轴向分量）重算，实际是 `[46,54]×4`，完全对称，是对的。**

> 我差点据此去"返工"一个本来正确的零件。
> **判读圆柱面轴线位置，一律用 `axisOrigin` + bbox 中心投影，不要用 `center`。**

### 坑 3.2：不要用"实体在某个 y 区间"来猜功能区

我一度把 `Extruded.006/010`（130×20×163 的**立板**）当成"电机底板/编码器底板"，
实际用户告诉我：
- 电机底板 = `Extruded.007` (361)，160×115×20
- 编码器底板 = `Extruded.009` (373)，160×85×20
- 底部大板 = `Extruded.011` (366)，250×525×18
- XYZ 平台底板 = `Extruded.008` (813)，160×150×20

**规则：功能区归属要问用户或用"特征"判定，不能靠尺寸/位置猜。**
可靠的特征判定法：**谁有与底板连接的沉头孔（锥面 z 落在底板厚度内），谁就是支架/底板。**

---

## 4. 隔离渲染 —— 用户的 tip，我最终没做对

### 4.1 用户原话
> 「一个小 tip，如果你想看一个零件，最好把它 isolate 显示，然后再切换视角」

### 4.2 试过并**全部失败**的路

| 尝试 | 现象 | 根因 |
|---|---|---|
| `cmd.dispatch {name:"command:isolate"}` | 返回 `{ok:true}`，画面不变 | 它是 **selector-based** 命令（selector=`plasticity-viewport`），而垫片只对**自己注册过委托**的命令有效（`ensureDelegate`）；`command:isolate` 没委托 → 派发了个寂寞 |
| `nodes.isolate(nodeKey)` | `isolationLevel` 变 1，画面不变 | 隔离层（`nodes.read.isolations`）写了，**渲染不消费它**；`hidden`/`invisible` 计数完全不变 |
| `nodes.setHidden(key, true)` | `hidden` 244→511，画面不变 | shim 注释写明：「直接 setHidden 只改集合不刷新」 |
| `model_view {of:[{name:"X"}]}` | 包围盒被**同名实体**撑爆，取景飞到天边 | `resolveSolids({name})` 匹配**所有同名**实体 |

### 4.3 唯一有效的隔离路径（但**有副作用**）

走 memento restore（仿 shim 内部 `applyVisibility`）：

```js
const st = ed.nodes.read;
const backup = { hidden: [...st.hidden], invisible: [...st.invisible] };

const all = []; st.node2id.forEach((id, k) => all.push(k));
const keep = new Set([目标实体的 nodeKey]);
const m = ed.originator.snapshot();
const ns = { ...m.nodes };
ns['hidden'] = Float64Array.from(all.filter(k => !keep.has(k)));
m.nodes = ns;
ed.originator.restore(m);        // ← 关键：走编辑器自己的 restore，场景才会刷新
```

### 4.4 ⚠️⚠️ 这个方法的副作用 —— 我踩的最大的坑

**反复 `orig.restore(m)` 会破坏渲染场景的挂载**，症状：
- `composer.scene` 里 mesh 数 = 0
- `renderer.info.render.triangles` 只剩 54（只有辅助线）
- 但 **CDP `Page.captureScreenshot` 截出来画面完全正常**（模型好好地在渲染）

我在这上面绕了 40 分钟，**误诊为"相机问题"**，反复调相机、调 `cam.set`、
调 `scene.update()`、派发各种 signal —— 全都没用。

**判定渲染是否正常的正确方法：CDP `Page.captureScreenshot`，不要看 `composer.scene`。**

### 4.5 渲染的其它坑

- `cam.view` 完整执行（**不传 `shot:false`**）会自己设相机+截图+还原相机，**能直接返回 PNG**。
  传 `shot:false` 时要自己等轨道阻尼收敛（≥1s）再 `cam.render` + `cam.thumbnail`。
- `cam.view` 的 `of` 用**实体 id** 会因 id 变更而失效（`solid not found`），
  用 `{name:"..."}` 更稳 —— 但前提是**名字唯一**（同名实体要先清掉）。
- `cam.view` 的 `of: null` 表示全部。

---

## 5. 未解难题（本轮真正卡住的地方）

### 5.1 症状

Plasticity 的**几何工厂**在 undo 之后失效，所有需要几何上下文的 op 报：

```
Cannot read properties of undefined (reading 'Viewport')
```

| op | 状态 |
|---|---|
| `rename`（纯命名） | ✅ 正常 |
| `geo.query` / `model_state` / `model_outline` | ✅ 正常 |
| `cylinder` / `prism` / `box` / `revolve` | ❌ Viewport undefined |
| `move`（真实位移） / `duplicate` / `boolean` | ❌ Viewport undefined |

### 5.2 根因（已定位到代码行）

- `model_exec` 的 op 都基于 Plasticity 原生命令基类
  （`cmd.factoryBaseHolder` → `ed.commands.DeselectAllCommand` 的原型）。
- 垫片 `factoryCommandClass()` 里 `new Cls(editor, build)` → `super(editor)`，
  构造时读某个 `xxx.Viewport`，而这个引用在 undo 之后失效了。
- 工厂类本身（`CylinderFactory` 等）是代理**从堆快照 revive** 的：
  `PLA.exposeClosures {pattern:'^[A-Z]\\w*Factory$', global:'__PLA_FACTORIES__', force:bool}`
- **致命点：所有调用点都是 `force=false`** ——
  `src/crates/pla-bridge/src/cdp.rs:383` 和 `:1229`：
  ```rust
  crate::exec::ensure_factories(ws, &mut mid, false).await
  ```
  页面已有 `__PLA_FACTORIES__` 就不重拍 → **快照坏了也没法刷新**。

### 5.3 我试过但没成功

- `window.__PLA_PROXY_SENDS` —— **不存在**（当前 9333 CDP 代理不是带 `sendRef` 的那版）
- `window.PLA` —— undefined
- `ed.scene.update()` / `ed.views.signals.*.dispatch()` —— 无效
- `composer.clearScenes` —— 未试（当时已停手）
- 代理 ws 端口 8980 —— HTTP 502，不是普通 HTTP 入口

### 5.4 建议给开发 agent 的修复

1. **加 `force` 开关**：在 `model_exec` / `model_hole` 等工具上加一个 `refresh_factories: true` 参数，
   透传到 `ensure_factories(ws, mid, true)`。
2. **加自愈**：捕获 `Viewport undefined` 这类错误时，自动 `ensure_factories(force=true)` 重拍一次再重试。
3. **补代理方法透传**：让 `PLA.exposeClosures` 能从页面直接调（现在只能从 Rust 侧走 ws）。
4. **禁止/警示 undo**：在几何操作后 undo 是高危动作，至少在 `model_transaction {action:"undo"}`
   的文档里写明"undo 会使几何工厂快照失效，后续写操作可能全部失败"。

---

## 6. 事故复盘（我做错的事，按严重程度）

### 6.1 ❌❌❌ 用 3 次 `undo` 去"试错修复" —— 造成不可逆损坏

**当时的情况**：渲染异常，我怀疑是场景损坏，于是调用 `hist.undo {steps:1}` 三次来"回退看看能不能恢复"。

**后果**：
- 三次 undo 把"底板挖台阶"这一步撤销了（虽然可重做）
- **更严重**：undo 使几何工厂快照失效 → 之后所有写操作报错 → **下移 3mm 的成果无法再保存**
- `ed.save()` 也卡住（生成 61MB `.tmp` 后不再推进，`hasUnsavedChanges` 一直 true）
- 最终内存中的全部工作丢失

**教训**：
> **undo 不是"撤销工具"，是"状态回滚工具"。** 在没有备份的前提下用它做探索 = 赌命。
> 正确做法：先复制文档文件 → 再探索。

### 6.2 ❌❌ 没有先备份就动手

我在第二轮（XYZ 平台细化）**一开始就直接下移了 224 个实体**，没有先备份。
虽然我在 `ed.save()` 之前补了备份（磁盘文件完好），但内存状态始终没有安全落盘过。

**规则**：
> 任何"批量写操作"之前，先 `shutil.copy2` 一份文档文件。
> 备份路径要记在 `state_snapshot.json` 里。

### 6.3 ❌ 误诊后越挖越深

渲染异常时，我在 `composer.scene` 上得出"模型视图没挂到场景树上"的结论，
然后开始调 `scene.update()`、派发 40 多个 signal、试 `undo` 恢复……
**每一步都是在损坏状态上做写操作。**

**应该早点做的检查**：
> 用 CDP `Page.captureScreenshot` 截真实屏幕 —— 一张图就能看出"模型其实渲染得好好的，
> 只是相机不对"。我晚了 40 分钟。

### 6.4 ❌ 在垃圾场里"清理"时差点误删 503 个实体

我的废料分类脚本按"y 不在主装配区"来判定，结果把 **503 个原模型实体**全归为废料
（只有 16 个才是我的过程废料）。**幸好只是打印没执行**。

**规则**：
> 批量 move/delete 之前，先**打印分类结果让人确认**，或用"名字带我自己的标记"来精确定位。
> 绝不要用"位置在某个区间外"当作删除依据。

### 6.5 ❌ 忽略了垫片注释里的警告

shim 注释明写：
> 「直接 setHidden 只改集合不刷新」
> 「原子操作在窗口不可见时拒绝」
> 「revolve 的 pickNewRegion 会选错区域」

这些是前人踩过的坑。我读了部分注释，但在 `orig.restore()` 反复调用这件事上
**没有对应的警告**（因为官方用法只在 `applyVisibility` 里调一次）。

---

## 7. 已完成且验证过的工作（可作为接手基线）

### 7.1 第一轮：抱箍法兰转接结构 ✅

| 件 | id | 面数 | 位置 |
|---|---|---|---|
| 抱箍法兰 | 3231 | 52 | x[-31.94,32] y[276.971,316.971] z[78.02,141.99] |
| 读头座A-Solid010 | 3236 | 8 | x[-18.94,19] y[238.752,300] z[91.02,128.99] |
| 读头座B-reader1 | 3241 | 8 | x[-21.75,21.75] y[238.944,300] z[88.31,131.69] |
| M3×12 ×4 | 3464~3467 | 14 | 对齐竖板实际孔位，偏差 0.001mm |
| M5×16 | 3463 | 14 | x[10.75,19.23] y[286.72,295.22] z[103,124] |
| M10×25 | 3459 | 14 | x[-8,8] y[238.4,273.4] z[102,118] |

关键依据：
- 读头轴线 **x=0, z=110.001**
- XYZ 平台竖板 = id **800**（`XYZ60-L-D.stp<1>[48]`，y[316.971,329.971]，x[-30,30]，z[80,140]）
- 竖板 PCD≈50 的 4 个 ⌀3.24(M3) 孔实际在 **(±25, 109.191)、(0, 134.191)、(0, 84.191)**，
  相对理想 PCD=50 各偏 **0.810mm**（板孔在 y=109.191，轴线在 z=110.001）
- 腰孔：端圆心半径 **23.0/27.0**（PCD 46~54），中心线过 PCD=50，宽 3.4，端圆心距 4 → 吸收 0.810mm ✅
- 抱箍法兰规格 D=20：盘 ⌀64/T=8，固定部 ⌀40/L=32，中心孔 ⌀20

### 7.2 第二轮：XYZ 平台下移 3mm ✅（但**未保存到磁盘**）

- 224 个实体沿 **-Z** 下移 3.000mm（电机组件 8 + 编码器/磁环/读头 59 + 平台本体 155 + 我做的 9）
- 静止不动：底板 361/373/813/366 + 立板 369/387
- 校验：所有活动件 z 减 3.000，静止件完全未动
- **⚠️ 这份状态只在内存里，重启即丢**（`ed.save()` 因工厂失效而卡住）
- 重做脚本：`C:\Users\frg\AppData\Local\Temp\pla_mcp_client\do_move.py` + `move_list.json`

### 7.3 底板挖台阶的配方（已验证成功过，被 undo 回退）

```
prism 刀具: center=[0, 366.971, 17], normal=[0,0,1], sides=4, flats=60, height=4, rotate=45
  → bbox [[-30,336.971,17],[30,396.971,21]]  ✅ 尺寸精确
boolean subtract: 底板 6 面 → 15 面
```
平台基座(764) 下移后底面 z=17.001，底板顶面 z=20 → 凹槽深 3mm，
-Y 侧壁 y=336.971 即"前立面"，平台顶住它做 -Y 定位。

---

## 8. 给下一个 agent 的执行清单

**动手前**
- [ ] 复制 `encoder calibrator.plasticity` 到临时目录，**每次批量写操作前都复制**
- [ ] 记录文档路径：`C:\Users\frg\My Drive\04 coop\高精度编码器平台\DESIGN\encoder calibrator.plasticity`
- [ ] 确认 Plasticity 窗口可见（`warm_up()`），`document.visibilityState === 'visible'`
- [ ] 先跑一遍**只读**探查（`model_state` / `model_outline` / `geo.query`），确认工厂是好的

**写操作纪律**
- [ ] 每步回读 `results[].items[].id`
- [ ] 报错先 `model_state` 核实实际状态，再决定是否重试
- [ ] 不用 `revolve`（除非空旷区），不用 `delete`，不用 `box`
- [ ] 造矩形用 `prism` + `rotate:45`
- [ ] 圆柱轴线用 `axisOrigin`，不用 `center`
- [ ] **不调用 `hist.undo` 做探索**（会让工厂快照失效）

**渲染**
- [ ] 用 CDP `Page.captureScreenshot` 判断渲染是否正常
- [ ] `cam.view` 不传 `shot:false`；`of` 用名字（保证唯一）
- [ ] **不要反复 `orig.restore()` 做隔离**（会破坏场景挂载）
- [ ] 隔离优先用 `nodes.setHidden` + 手动 `cam.render`（接受"可能不刷新"）

**收尾**
- [ ] 保存文档后确认 `hasUnsavedChanges === false` 且**文件 mtime 已更新**
- [ ] 检查目录里没有残留 `.tmp`
- [ ] 把新踩的坑补进本文件

---

## 9. 附：可复用脚本位置

全部在 `C:\Users\frg\AppData\Local\Temp\pla_mcp_client\`（临时目录，**重启会丢，需要长期留存应移进 repo**）：

| 文件 | 用途 |
|---|---|
| `pla_client.py` | MCP stdio 客户端（`list`/`schema`/`call`） |
| `warm.py` | 窗口可见性预热（CDP + Windows 置前） |
| `mcpjson.py` | 可靠的 model_exec JSON 提取 |
| `builder.py` | 写操作执行器封装 |
| `cdp.py` | CDP WebSocket 客户端 |
| `shotlib2.py` | 渲染工具（`cam.view` + 隔离） |
| `do_move.py` + `move_list.json` | 224 实体下移 3mm（第二轮，可重跑） |
| `do_step813b.py` | 底板挖台阶 |
| `check_holes3.py` | 腰孔几何核对（用 axisOrigin） |
| `audit_waste.py` | 废料审计（只读） |
| `state_snapshot.json` | 当前内存状态快照（实体 id/name/key） |
| `encoder_calibrator_backup_before_save.plasticity` | 主文件备份（与当前文件哈希一致） |

---

## 10. 事故当天的磁盘状态（已核实）

```
encoder calibrator.plasticity   58045331 bytes
  SHA256 = 09D035A1757AF96B7D408494F0C313EC5C1066C05885258C1137A7C32DCFFD7B
  mtime  = 2026-11-20 15:24   ← 接手前的时间戳, 说明主文件从未被写入
```

我调用 `ed.save()` 失败后留下的两个 `.tmp`（61306630 / 61390765 bytes）
已在事故处理时删除，目录已恢复干净。
