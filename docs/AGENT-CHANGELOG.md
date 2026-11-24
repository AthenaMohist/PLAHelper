# Agent 侧改动建议（只读观察，未改任何代码）

> 记录人：建模执行 Agent（WorkBuddy 会话）
> 日期：2026-11-20（第一轮只读观察）／2026-11-21（第二轮：执行建模后补充）
> 范围：**只观察、只提建议**。repo 里的 Rust / JS / Python 算法与脚本一行未改。
> 本文档供开发 agent 取用；每条都给了复现命令与实测证据。

---

## 0. 本次会话的连接方式（背景，非缺陷）

`pla-mcp.exe` 在本机**已经在跑**（`tasklist` 见 `pla-mcp.exe` PID 13672），`pla-ui.exe` 也在跑（PID 2764），
Plasticity 以 CDP 模式运行（9333 代理在 LISTEN，8980 官方 bridge 在 LISTEN）。
但 WorkBuddy 侧**没有** `plahelper` 这个 MCP 连接器，`.mcp.json` 是给 Claude Code 用的格式。

本次用自写的 stdio JSON-RPC 客户端直连 `pla-mcp.exe` 完成了全部调用（41 个工具全部可用），
客户端在 `C:\Users\frg\AppData\Local\Temp\pla_mcp_client\pla_client.py`（**临时目录，非 repo**）。

**建议 0-A｜给 WorkBuddy 提供 `.mcp.json` 兼容入口**

- 现状：WorkBuddy 只读 `~/.workbuddy/mcp.json`，不读项目里的 `.mcp.json`。
- 已做：我把 `C:\Users\frg\.workbuddy\mcp.json` 写成登记了 `plahelper`（command 指向绝对路径
  `...\repo\src\target\release\pla-mcp.exe`），**WorkBuddy 需在连接器管理页点"信任"后才会激活**。
- 建议：要么让 WorkBuddy 也探测项目根 `.mcp.json`，要么在 pla-mcp 的 README 里补一段 WorkBuddy 的
  `~/.workbuddy/mcp.json` 片段（现在文档只写了 `claude mcp add`）。

**建议 0-B｜`.mcp.json` 用相对路径导致换个机器就跑不起来**

- 文件内容是 `"command": "src/target/release/pla-mcp.exe"`。
- 这个相对路径只在 cwd = repo 根时成立。MCP 客户端的 cwd 未必是 repo 根。
- 建议：文档里同时给出绝对路径写法（Windows 反斜杠需在 JSON 里转义成 `\\`）。

---

## 1. `model_state` 实体名乱码（真缺陷，可复现）

### 现象

`model_state`（不带 detail）返回的 `geometry.solids[].name` 里，
**id=1** 这个实体（227 面，是整个装配的主骨架）名字是：

```
Œú‚Ý•t‚¯3
```

而不是正常的中文。同一次调用里其余 403 个实体的名字（`2026_09_24__10-06-03-207` 等）都正常。

### 定位到的关键事实

1. **不是我的客户端问题**：`model_outline` 返回的中文名全部正常
   （`兴大联轴器_CFC-40×50.stp`、`杯内六M3x20.001`、`备选方案_方式II_环形螺母` …）。
   `model_outline` 和 `model_state` 是两条不同的取名路径。
2. **不是终端编码问题**：`list_parts` 返回的 `磁环座` / `编码器轴` 正常，同一次进程、同一个 stdio 管道。
3. **只在 `model_state` 这一条路径上**。也就是说 `model_state` 取名时，**对某个字节序列做了错误的解码**。

### 还原尝试（都失败，说明不是简单的单次编码错位）

把乱码按 CP1252 还原成字节得到 `8c fa 82 dd 95 74 82 af 33`：

| 假设 | 结果 |
|---|---|
| bytes → gbk / gb18030 | `岤傒晅偗3`（不是有意义的词） |
| bytes → utf-8 / utf-16-le | UnicodeDecodeError |
| bytes → big5 | UnicodeDecodeError |

`岤傒晅偗3` 本身像是**二次错位**的产物（某个真实中文名先被按某编码解、再被按 UTF-8 存过一次）。

### 建议（给开发 agent）

- 在 `model_state` 的取名路径上，**不要**用 `String::from_utf8_lossy` 之类对已经是 `String` 的值再解一次；
  排查 `pla_bridge` / 垫片里 `name` 字段从 CDP JSON 到 Rust `String` 的转换是不是
  **对已解码的字符串又做了一次 UTF-8 → GBK 或 UTF-8 → Latin1 的往返**。
- **加一条自检**：`model_state` 返回的名字若含 `Œ` `‚` `Ý` `•` 这类 CP1252 特征字符，
  或 `UnicodeDecodeError` 静默失败，就说明该实体名在某条路径上被损坏了。
- 判定依据用现成的：`model_outline` 的同名实体是正常中文 —— 两条路径对拍即可定位。

### 附：为什么值得修

这个实体是装配主骨架，**它同时是 `model_measure {of: ...}` / `model_exec {solids: {"name": ...}}` 的可选引用方式**。
名字坏掉 → agent 没法用名字引用它 → 只能退回用 id（id 每次改动都会变，更难用）。
这是"能不能用自然语言驱动"的关键一环。

---

## 2. `model_view` 的 `of` 参数不接受零件名（易踩）

### 现象

```
model_view {of: "磁环座"}  →  垫片异常: solid not found: "磁环座"
model_view {of: 545}       →  正常返回 PNG
```

`of` 只接受**实体的 id 或实体名**，而「磁环座」是 **BOM 零件名**（出图侧的概念），不是实体名。
两者名字空间不同，但都在同一个 `list_parts` / `status` 的语境里，容易混。

### 建议

- 在 `model_view` / `model_measure` / `model_exec` 的报错信息里区分两种"找不到"：
  - 「实体名/ id 找不到」→ 提示「这可能是 BOM 零件名，实体请用 `model_outline` 查」。
  - 现在的 `solid not found: "磁环座"` 会让 agent 以为模型里没这个零件，从而误判。
- 建议加一个**零件名 → 实体**的解析辅助（现在零件 key 形如
  `3856.1|2939|3.2841/8.1533/8.1542|Cone4.Cylinder5.Plane6|0`，里面已经带了体积/面数/面类型信息，
  但没有实体 id，缺一条映射）。

---

## 3. `model_exec` 的单位与长度限制（已自解，仅记录）

`model_exec` 的长度按**文档单位**给（当前文档是 millimeter，读取自
`document.settings._Unit.length`）。底层 Parasolid 内部用米，派发层负责换算。
本次会话没踩到，但建议在错误信息里带上「当前文档单位 = millimeter」，
避免 agent 在 inch 文档里按 mm 给值（会差 25.4 倍）。

---

## 4. 建议补一个「装配树 → 零件」的映射工具（功能缺口）

### 现状

- `model_outline` 给的是 Plasticity 的**实体/文件夹树**（404 个实体 + 组）。
- `list_parts` 给的是 **BOM 零件清单**（70 个零件，按几何特征聚合：`体积|面数/面积/面类型|朝向`）。
- 两者**没有任何工具可以直接对上**。

### 影响

出图侧（`draw` / `apply_drawing_changes` / `get_drawing`）用 BOM 零件名；
建模侧（`model_exec` / `model_hole`）用实体 id / 实体名。
用户说"给磁环座加个 M4 沉头孔"，agent 必须自己想办法把「磁环座」这个 BOM 概念落到某个具体实体上。
现在只能靠 `list_parts` 的 key（体积 + 面数 + 面类型签名）去 `model_state` 里反查 —— 能做，但很脆：
- key 里的体积是**网格估算**（`export-step.md` 实测与 B-rep 差约 0.2%），多个同规格件会撞；
- 面数相同、材质相同的不同零件（大量 `XYZ60-L-D.stp<1>[N]`）光靠签名分不开。

### 建议

加一个只读工具，比如 `model_locate {part: "磁环座"}`，
返回该 BOM 零件对应的实体 id / 实体名 / 版本 id / 包围盒（可多个候选）。
实现上最省事的路子大概是复用出图侧已有的"体积+面积最接近"匹配逻辑，
但**必须加手性/包围盒判别**（`export-step.md` 已记：镜像件体积面积相同，按体积挑会挑错）。

---

## 5. `model_state` 默认返回 404 个实体，输出体积大

- 不带 `detail` 时也把全部 404 个实体的 `bbox` / `faces` / `edges` 全吐出来，单次响应约 100+ KB。
- 有 `limit` 参数可用（schema 里有），但默认行为偏重。
- 建议：默认给个更省的摘要（比如只给名字 + id + 粗包围盒），`limit` / `detail` 再展开。
  这对 token 成本影响明显 —— 一次 `model_state` 就能吃掉不少上下文。

---

## 6. 事务 / 撤销：能力齐备，无需改动（正面记录）

- `model_transaction {action: begin|commit|rollback}` 可把多次调用合成一次可整体撤回的操作。
- `model_exec` 默认 `atomic: true`，某步失败自动撤回本次全部步骤。
- 坑（已在 `docs/know-how/mcp-tools.md` 记过）：**窗口不可见时 atomic 一开始就拒绝**
  （撤销走 memento，隐藏窗口不行）；`model_exec` 会**取消用户正在进行的命令**。
- 本次会话没有触发这些路径（只做了只读操作），但记录在此供后续建模时注意：
  **建模前确认 Plasticity 窗口可见，且用户没有正在进行的命令。**

---

## 7. 本次会话**没有**做的事（边界声明）

- 没有调用任何 `model_exec` / `model_hole` / `model_transaction` 等改模型工具
  —— 用户尚未下达建模需求，不该擅自动模型。
- 没有调用 `apply_drawing_changes` / `draw` / `draw_parts` 等写出图工具 —— 同上。
- 没有修改 repo 内任何 `.rs` / `.js` / `.py` / `.json` 源文件。
- 唯一写入 repo 的文件就是本 md（`docs/AGENT-CHANGELOG.md`）和 `.workbuddy/memory/` 下的工作日志。
- 唯一写入 WorkBuddy 全局配置的是 `C:\Users\frg\.workbuddy\mcp.json`（登记 MCP，属于连接器配置，
  不属于 repo 算法/脚本）。

---

## 附：本次验证通过的调用清单（供开发 agent 对拍）

| 工具 | 调用 | 结果 |
|---|---|---|
| `status` | `{}` | document=`encoder calibrator`，parts=70，drawings_dir 已定位 |
| `model_state` | `{}` | 404 实体 / units=millimeter；id=1 名字乱码（见 §1） |
| `model_outline` | `{}` | 树正常，中文名正常 |
| `list_parts` | `{}` | 70 零件；磁环座/编码器轴 已出图，其余 not drawn |
| `get_drawing` | `{part:"磁环座"}` | 完整返回设置 + 编译报告（尺寸名 / 孔组 / 技术要求） |
| `model_view` | `{view:"iso"}` | 返回 960×640 PNG，模型是编码器校准平台装配体 |
| `model_view` | `{of:545, view:"iso"}` | 正常（单个实体） |
| `model_view` | `{of:"磁环座"}` | 报错（见 §2） |

---
---

# 第二轮：实际执行建模后补充（2026-11-21）

本轮执行了真实建模（抱箍法兰 + 两款读头座 + 6 颗螺丝），过程中踩到几个**会直接让 agent 做错事**的垫片行为，
以及一个**正确但极难自行发现的用法**（隔离渲染）。全部记录在下面 §8~§12。

## 8. 【严重】`model_exec` 的 `rotate` / `move` 之后，实体 id 会变 —— 必须回读 `results[].items[].id`

### 现象

```
model_exec {ops:[{op:"rotate", id:"rot", solids:[3172], pivot:[...], axis:[0,1,0], degrees:-90}]}
  → 返回 id = 3460        （这是"新"实体）
model_exec {ops:[{op:"move", id:"mv", solids:[3460], move:[...]}]}
  → ❌ failed: solid not found: 3460
```

### 根因

`rotate` / `move` 这类变换在 Parasolid 侧是**重建实体**（新版本 id），
而 `model_exec` 返回的 `results[].items[].id` 是**新的版本 id**。
调用方如果沿用传入的旧 id 去接下一步，会报 `solid not found`。

### 建议

- `model_exec` 的返回值里 `results[].items[].id` 与输入 `solids[]` 语义不同，
  建议在**文档里明确写**：「变换类 op（move/rotate/mirror/pattern/radialPattern）之后必须用返回值里的 id」。
- 更稳的做法：垫片在 `results[]` 里**额外回传 `inputIds`**，
  让 agent 能自己判断"id 变了还是没变"，避免靠 try/catch 猜。

### 附带坑：`move` 成功但回包解析失败 → 状态已改，别重试

本轮遇到过一次 `model_exec move` 报「垫片回包异常: EOF while parsing a value」，
但**操作其实已经生效**（事后 `model_state` 显示位置已变）。
agent 若直接重试，就会把零件平移两次（本轮把 M5 螺钉平移到了 y=-5715）。

**建议**：垫片在回包序列化失败时，应把**已生效的操作**如实回传
（例如 `{ok:false, err:"serialize", applied:[{op, ids}]}`），而不是整包丢弃。

---

## 9. 【严重】`geo.query` / `model_state` 的圆柱面 `center` 字段**不是轴心** —— 会导致误判孔位

### 现象

同一个 ⌀3.4 腰孔端圆，`geo.query` 给的：

```
{"surface":"Cylinder","radius":1.7,"axisOrigin":[0,306.971,83.001],"axis":[0,1,0],
 "center":[-1.7,312.971,83.001]}
```

- `axisOrigin` = 真实轴线上一点 ✓
- `center` = **偏了半径 (-1.7)** 的另一个点 ✗

按 `center` 算出来 4 组腰孔的 PCD 是 `[46.13, 54.11] / [42.60, 50.60] / [49.40, 57.40] / [46.13, 54.11]`
—— 看起来"±X 两组不对称，是建错了"。
按 `axisOrigin`（或 bbox 中心减去轴向分量）重算，实际是 `[46, 54] / [46, 54] / [46, 54] / [46, 54]`，**完全对称，是对的**。

### 根因

`center` 是从圆柱面的**参数化原点**推出来的（Parasolid 的 surface origin + u 方向偏移），
不是几何轴心。

### 建议

- `geo.query` / `model_state` 的圆柱面输出**补一个 `axisPoint`** 字段 = 轴线上任一点
  （推荐取 `bbox` 中心沿轴投影），语义明确、不用猜。
- 或者把 `center` 改名为 `surfaceOrigin`，并额外给 `axisPoint`。
- 现在 `center` 这个名字对圆柱/圆锥面是**误导性的**，agent 极易算错 —— 本轮就差点因此
  误判"腰孔建错 1.7mm"并去返工。

**复现**：`geo.query {of:[3231], detail:"all"}` 看腰孔 8 个端圆的 `center` vs `axisOrigin`。

---

## 10. 【中】`revolve` 在有既存实体的位置会选错区域 —— 建议加"空旷区"预检或自动提示

### 现象

在已经有法兰盘的位置执行 `revolve`（想生成 ⌀38 的承托台），
实际拿到的是 ⌀10 的东西（差一个数量级）。

### 根因

垫片 `revolve` 内部用 `pickNewRegion(before, mid, k)` 挑"离 profile 点集中点最近的区域"，
在有既存实体的位置，这个"最近的区域"可能是**旧实体里的面**，而不是新建的旋转体。

### 建议

- `revolve` 前**检查目标位置附近是否已有实体**，有则**明确报错**（而不是悄悄选错区域）。
  本轮靠"全部改用 `cylinder` / `prism`"绕过了（`cylinder` / `prism` 的 id 与位置都精确可靠）。

---

## 11. 【中】`boolean` 不接受 `$id` 步骤引用，只接受真实 id

### 现象

```
model_exec {ops:[{op:"boolean", targets:["$a"], tools:["$b"], operation:"subtract"}]}
  → ❌ unknown ref $cone
```

### 建议

- 垫片在 `boolean` 的 `targets` / `tools` 上做一次显式检查：
  遇到 `$` 开头就报「`boolean` 需要真实 id，请从 `results[].items[].id` 取」，
  而不是笼统的 `unknown ref`。
- 或者：`boolean` 支持 `$ref`，垫片在**同一次 `model_exec`** 内解析 `$refs` 表
  （`ctx.refs` 已经在 `cmd.exec` 里维护了，只是 `boolean` 没走这条路）。

---

## 12. 【工具缺口】没有「隔离显示 / 隔离渲染」的能力 —— 用户明确需要，建议补一个工具

### 用户的原话

> 「一个小 tip，如果你想看一个零件，最好把它 isolate 显示，然后再切换视角」

这是 Plasticity 用户的基本操作（`command:isolate`），但**当前 MCP 工具集里没有对应能力**，
导致 agent 出图时前景总被别的实体遮挡。

### 试过并失败的路（供开发 agent 省时间）

| 尝试 | 结果 | 为什么不行 |
|---|---|---|
| `cmd.dispatch {name:"command:isolate"}` | 返回 `{ok:true}`，**无任何效果** | `command:isolate` 是 **selector-based** 命令（selector = `plasticity-viewport`），而垫片只对**自己注册了委托**的命令有效（`ensureDelegate`）；`command:isolate` 没委托 → 派发了个寂寞 |
| `nodes.isolate(nodeKey)` | `isolationLevel` 变 1，但**画面不变** | 隔离层（`nodes.read.isolations`）写了，**渲染不消费它**；`hidden` / `invisible` 计数完全不变 |
| `nodes.setHidden(key, true)` | `hidden` 计数 244→511，**画面不变** | shim 注释已写明：「直接 setHidden 只改集合不刷新」 |
| `model_view {of:[{name:"X"}]}` | 包围盒被**同名实体**撑爆，取景偏到天边 | `resolveSolids({name})` 匹配**所有同名**实体；建过程留下 7 个同名"抱箍法兰" → bbox 横跨 y=5000~6000 |

### 有效的做法（已跑通，供封装参考）

**走 memento restore 路径**（和垫片内部 `applyVisibility` 同一个思路）：

```js
// 1) 备份可见性
const st = ed.nodes.read;
const backup = { hidden: [...st.hidden], invisible: [...st.invisible] };

// 2) 只留目标可见：hidden = 全部 nodeKey - 目标 key
const all = []; st.node2id.forEach((id, k) => all.push(k));
const m = ed.originator.snapshot();
const ns = { ...m.nodes };
ns['hidden'] = Float64Array.from(all.filter(k => !keep.has(k)));
m.nodes = ns;
ed.originator.restore(m);          // ← 关键：走编辑器自己的 restore，场景才会刷新

// 3) 出图：两段式（同一调用里 set 完立刻渲染会拿到旧画面）
__PLA__.call('cam.view', {view:'iso', of:[id], margin:1.25, shot:false});
await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
const png = __PLA__.call('cam.thumbnail', {maxWidth:1100});

// 4) 还原
// ns['hidden'] = Float64Array.from(backup.hidden); orig.restore(m);
```

### 建议新增工具

`model_view` 加一个 `isolate: true`（或新增 `model_isolate {show:[ids]} / {hide:[ids]}`）：

- 内部做上面这套「备份 → memento restore → 渲染 → 还原」；
- 渲染用**两段式**（`cam.view {shot:false}` → 等一帧 → `cam.thumbnail`），
  否则拿到的是**旧画面**（这点 shim 注释里也提过，但只在 `cam.view` 内部生效，没暴露成可用的两段式接口）。

### 附：顺手发现的小缺口

- `geo.query` 的 `of` 支持**实体 id**（`resolveSolids` 最后一个分支 `viewOf('solid', ref)`），
  但**不支持按名字唯一化**。建议同名时**报错并列出所有候选 id**，
  而不是全部匹配后把 bbox 撑爆。

---

## 13. 第二轮的建模结论（供后续复用）

### 交付物

| 件 | 实体 id | 面数 | 位置 (x, y, z) |
|---|---|---|---|
| 抱箍法兰 | 3231 | 52 | x[-31.94, 32], y[276.971, 316.971], z[78.02, 141.99] |
| 读头座A（配 Solid.010） | 3236 | 8 | x[-18.94, 19], y[238.752, 300], z[91.02, 128.99] |
| 读头座B（配 reader1） | 3241 | 8 | x[-21.75, 21.75], y[238.944, 300], z[88.31, 131.69] |
| M3×12 ×4 | 3464/3465/3466/3467 | 14 | 对齐竖板实际孔位 |
| M5×16 | 3463 | 14 | x[10.75, 19.23], y[286.72, 295.22], z[103, 124] |
| M10×25 | 3459 | 14 | x[-8, 8], y[238.4, 273.4], z[102, 118] |

### 关键几何依据（全部经 `geo.query` 实测，非估算）

- 读头轴线：**x = 0, z = 110.001**
- XYZ 平台竖板 = 实体 id **800**（`XYZ60-L-D.stp<1>[48]`，y[316.971, 329.971]，x[-30, 30]，z[80, 140]）
- 竖板上 PCD≈50 的 4 个 ⌀3.24 (M3) 孔，实际坐标：
  **(±25, 109.191)、(0, 134.191)、(0, 84.191)** —— 相对理想 PCD=50 各偏 **0.810mm**（沿 z）
  （因为板孔在 y=109.191，而读头轴线在 z=110.001）
- 抱箍法兰 D=20 / PCD52 底料 → 4 组 M3 腰孔：
  端圆心半径 **23.0 / 27.0**（PCD 46 ~ 54），中心线过 PCD=50，
  腰孔宽 3.4（覆盖 ⌀3.24）、端圆心距 4 → 0.810mm 偏差被完全吸收 ✓
- 4 颗 M3 螺钉最终**按板孔实际坐标**放置，偏差 0.001mm

### 标准件沿用（抱箍法兰规格表 D=20）

- 法兰盘 ⌀64、T=8；固定部 ⌀40、L=32；中心孔 ⌀20
- 夹紧缝 1.5×1.5 方形，x=18 处，沿 X 贯通
- M5 夹紧孔：⌀4.2 螺纹底孔 z[102.971, 110.601] + ⌀5.6 通孔 z[110.601, 124.230]，轴沿 Z

### 待用户确认的设计取舍

- **读头座轴 ⌀20 与法兰中心孔 ⌀20 是零间隙同心**（抱箍标准内孔就是 ⌀20）。
  若要滑动装配，轴应做 ⌀19.94~19.96。当前按"标准件不改、定制件让位"处理。
- 读头座轴长 58mm（y 242→300），插入法兰固定部 23.03mm，固定部内余 8.97mm 空腔。

### 本轮**没有**做的事

- 没有修改 repo 内任何 `.rs` / `.js` / `.py` / `.json` 源文件 —— 唯一写入 repo 的仍是本 md。
- 没有调用出图侧工具（`draw` / `apply_drawing_changes` / `get_drawing`）——
  用户本轮只要求建模与出**视图**（PNG），未要求出工程图。
- 没有删除任何实体：过程废料（16 个）改名为 `xxx_废<id>` 并停在 y=2276~14316 的垃圾场，
  **未 delete** —— 因为 delete 会污染 Parasolid stable model
  （后续写操作报 `invalid precondition: object N missing from stable model`，需 undo 一步恢复）。

---

## 附：经验沉淀另册

建模过程中踩过的坑、误诊过程、事故复盘、以及**未解决的难题**（几何工厂在 undo 后失效、
`ensure_factories` 全部 `force=false` 导致快照坏了无法刷新），
已单独整理到 **`docs/AGENT-LESSONS.md`**（前车之鉴，给后续 agent 用）。

可复用脚本已从临时目录移入 **`tools/agent-scripts/`**（临时目录重启即丢）：
`pla_client.py`(MCP 客户端) / `warm.py`(窗口预热) / `mcpjson.py`(JSON 提取) /
`cdp.py`(CDP) / `shotlib2.py`(渲染) / `do_move.py`+`move_list.json`(下移 3mm) /
`do_step813b.py`(底板挖台阶) / `check_holes3.py`(腰孔核对) / `audit_waste.py`(废料审计) /
`state_snapshot.json`(状态快照)

### 新增的改动建议（补充到上面 §5 之后）

**建议 5-A｜`model_exec` 等工具加 `refresh_factories` 参数（重要）**

- 现状：`src/crates/pla-bridge/src/cdp.rs:383` 与 `:1229` 都是
  `ensure_factories(ws, &mut mid, **false**)` —— 页面已有 `__PLA_FACTORIES__` 就不重拍。
- 后果：工厂快照一旦失效（undo 后即出现），**所有写操作永久失败**，
  且没有自愈路径（`PLA.exposeClosures` 的 `force:true` 没有从工具层透出来的入口）。
- 实测症状：`Cannot read properties of undefined (reading 'Viewport')`，
  `rename`/`geo.query` 正常，`cylinder`/`prism`/`box`/`move`/`duplicate` 全挂。
- 建议：① 工具层加 `refresh_factories: bool`；② 捕获该错误时自动 `force=true` 重拍并重试一次。

**建议 5-B｜`hist.undo` 加高危提示**

- undo 会使几何工厂快照失效（Plasolid/Plasticity 侧状态被回滚，堆快照里的类引用失效）。
- 建议在 `model_transaction {action:"undo"}` 的描述与返回里显式写明这一点。
