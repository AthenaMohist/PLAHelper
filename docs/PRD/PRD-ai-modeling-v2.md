# PRD: 自动建模 pipeline v2（设计文档）

日期: 2026-11-15 · 状态: 方向已认可（用户 2026-11-15），按阶段推进 · 前身: `PRD-ai-modeling-v1.md`

v1 的目标工作流（一句话 → 调研 → 定尺寸 → 计划 → 派发 → 校验 → 重试）不变。v2 依据 N2 实测结果调整三点:

- **不再用 Python 编排**（本机无 Python，见 memory/machine-state）：稳定层做成 **MCP 服务**，任何 MCP 客户端
  （Claude Code 等）就是编排 agent；需要无人值守时再加独立 agent 进程。
- **大模型不直接写工厂调用**：中间加一层结构化的**设计规格**，由确定性的编译器翻译成 `cmd.exec` 步骤。
  大模型只负责"需求 → 规格"，几何正确性交给代码和校验。
- **编号不可靠**：N2 实测同一个实体三步之后版本 id 从 32 变成 34，面/边 id 全变 → 必须有语义命名层和外部映射表。

## 1. 分层

```
L4 设计师 agent      应用场景 → 概念方案 → 设计规格(多方案 + 评分 + 人工把关)
L3 经验层            案例库(检索) · 教训库 · 评测集 · 闭环自检
L2 规格层            设计规格 JSON → 编译器 → 步骤;  语义引用解析;  映射表持久化
L1 工具层(MCP)       exec · query · measure · view · undo/事务 · 特征宏
L0 通道(已有)        pla-bridge CDP · 垫片 cmd.exec(配方表) · 代理 PLA.exposeClosures
```

每一层只依赖下一层；下层能单独测试、单独使用（例如 L1 不用 L2 也能被人或大模型直接调用）。

## 2. L1 工具层（MCP + skill）

**MCP 是手，skill 是经验。** MCP 服务直接包 pla-bridge 现有 Rust 代码（新 crate `pla-mcp`，stdio JSON-RPC），不另起炉灶。

| 工具 | 作用 | 底层 |
|---|---|---|
| `plasticity_exec` | 执行步骤（配方表操作或特征宏），返回产物与失败步 | `cmd.exec` |
| `plasticity_query` | 列实体 / 面 / 边 / 顶点及几何签名（见 §4），可按条件过滤 | 新垫片接口 `geo.query` |
| `plasticity_measure` | 距离、角度、体积、面积、包围盒、两实体干涉 | 新垫片接口 `geo.measure` |
| `plasticity_view` | 指定视角截图（正视/俯视/等轴/对准某实体），截完还原视角 | `cam.set` + `cam.thumbnail` |
| `plasticity_undo` / 事务 | `begin` 记撤销栈深度；失败 `rollback` 回到起点；`commit` 结束 | 撤销栈 + undo 命令 |
| `plasticity_model` | 读写当前文档的设计规格与映射表（§3、§4） | `.pla/models/<doc_key>/` |

- **标准步骤代码化分两层**：底层 = 工厂配方（box、cylinder、sketch+extrude、hole、fillet、chamfer、shell、
  offsetFace、pattern、boolean）；上层 = 参数化**特征宏**（带孔安装板、L 形支架、凸台加筋、沉头孔、外壳抽壳、
  螺丝柱……）。宏是代码，可测试、可复用；大模型调宏比自己拼步骤可靠得多。
- **skill**：配方与宏的说明、规格写法、示例、常见坑（来自 pitfalls 与教训库）。随经验层更新。
- **查询比再加操作更紧迫**：没有查询，大模型建完不知道自己建对没有。

## 3. L2 设计规格（大模型的输出模板）

特征树式 JSON，思路接近 CadQuery / build123d，但不含代码:

```json
{
  "name": "sensor_bracket",
  "units": "millimeter",
  "params": { "w": 40, "d": 20, "t": 5, "h": 30, "r_in": 3, "hole_d": "M3.clearance" },
  "datums": { "origin": [0, 0, 0], "base_plane": "XY" },
  "features": [
    { "id": "base",    "type": "box",    "size": ["w", "d", "t"], "at": "origin" },
    { "id": "upright", "type": "box",    "size": ["t", "d", "h"], "at": "origin", "bool": "union", "into": "base" },
    { "id": "inner_r", "type": "fillet", "edges": ["edge(base.top, upright.inner)"], "r": "r_in" },
    { "id": "holes",   "type": "hole",   "spec": "hole_d", "on": "face(base.top)", "pattern": { "grid": [2, 1], "pitch": [24, 0] } }
  ],
  "checks": [
    { "bbox": [[0, 0, 0], ["w", "d", "h"]] },
    { "min_wall": 2 },
    { "count": { "faces.cylinder": 3 } }
  ],
  "intent": "固定 PCB 传感器的 L 形支架, 两孔 M3 过孔"
}
```

- **参数表**是尺寸的唯一来源，特征只引用参数名；标准规格（`M3.clearance`）查 N4 规格库。
- **语义引用，不用编号**：`face(base.top)`、`edge(base.top, upright.inner)`，由编译器在每步之后解析。
- **checks**：验收条件（包围盒、壁厚下限、面数、孔位、体积范围、与某零件不干涉），闭环自检用。
- **从图纸开始**：第一步只做"提取尺寸 → 填参数表 + 列疑点"；疑点问人，不猜，然后才写特征序列。
- 编译器（Rust，确定性）：规格 → 步骤序列，逐步执行、解析引用、更新映射表；任何一步失败 → 事务回滚，返回失败特征与原因。

## 4. L2 映射表（外部维护几何元素）

存 `<文档目录>/.pla/models/<doc_key>/`：`spec.json`（规格）、`map.json`（映射表）、`log.jsonl`（每步操作与结果）。

每个语义名存三样东西:

1. **解析规则**：如"实体 upright 中法向 +X、x 最大的平面"；每步之后重新解析。
2. **几何签名**（校验用）：曲面类型、法向/轴线、质心、面积/长度、包围盒。重新解析的结果与签名对不上 → 报错，不默默选错。
3. **最近一次的编号**（只作缓存）：实体版本 id、拓扑 id（`<物体>f<n>` / `e` / 顶点）。

- 待调研：Plasticity 内部 `store.version2stable` / `stable2version`（实测存在）能否给出实体级稳定 id；面/边的
  `stableIds` 字段在实测文档里为空。
- 用途：重放（改参数后整模重建）、N3 尺寸驱动（"把 base 加厚到 6"= 改参数 + 重放，或对 `face(base.top)` 做面偏移）、
  N4 打孔（落点挂在语义面上）、N7 装配（配合关系引用语义面）。

## 5. L3 经验层（越用越强）

先做"经验积累"，后做"训练":

- **验证案例库**：每个成功案例存"需求 → 规格 → 步骤 → 结果 + 截图 + checks 结果"；新任务按相似度检索几例放进上下文。性价比最高。
- **评测集**：几十个由易到难的任务，每个带几何断言（尺寸、孔位、体积、面数、干涉）。每次改提示词 / 宏 / 模型都跑回归。
  在副本文档上跑，结束撤销到起点。
- **闭环自检**：建完 → query / measure / view → 对照 checks → 不符则修，限定轮数；仍失败 → 回滚并交人。
- **教训库**：失败原因结构化记录（同 pitfalls.md），反馈进 skill。
- **训练（以后）**：成功/失败轨迹即训练数据，几何断言即奖励信号；数据量到几千条前，检索 + 评测优于微调。

## 6. L4 设计师 agent

独立角色：从应用场景产出概念方案和设计规格；建模 agent 负责把规格落成几何。它需要的是工程知识与工具:

- 标准件库（N5）、打孔规格库（N4）；
- 设计规则：注塑壁厚与拔模、钣金折弯、螺纹深度、公差配合；
- 校核：强度估算、干涉检查（N7），必要时导出 COMSOL（N8）。

"创造性"的工程做法：生成多个候选方案 → 规则与指标打分 → 人在概念阶段把关。研究性质，先在一类常做零件
（安装支架 / 外壳 / 夹具，待用户选定）上做深，再扩展。

## 7. 分阶段（验收）

| 阶段 | 内容 | 验收 |
|---|---|---|
| **P1 基础设施** | `geo.query` / `geo.measure` + `pla-app cdp-state`（= v1 M-B）；事务回滚；配方补齐（cylinder、sketch+extrude、hole、chamfer、shell、offsetFace、pattern）；`pla-mcp` 服务 | MCP 客户端里一句话调用：建 L 形支架 → 查询面列表 → 量孔距 → 截图；失败步自动回滚 |
| **P2 规格层** | 规格 JSON schema + 编译器 + 语义引用解析 + 映射表持久化 + 重放；5–10 个特征宏 | 同一份规格改一个参数后重放，几何与 checks 全部吻合 |
| **P3 经验层** | 评测集（≥20 题）+ 案例库检索 + skill 文档 + 闭环自检 | 评测集通过率可量化并随迭代上升；"20×10×5 带圆角底座"类需求全自动完成（= v1 M-C） |
| **P4 设计师 agent** | 窄领域试点：场景 → 多方案 → 评分 → 人工选 → 规格 → 建模 | 一个真实小项目从场景描述走到可用模型 |

P1 每一项都直接建立在 N2 上，风险最低；N3（尺寸驱动）与 N4（打孔）分别是 P2 映射表与 P1 hole 配方的首个用户。

## 8. 待确认

1. 先做哪一类零件（决定宏与评测集的选题）。
2. MCP 服务放在独立 crate `pla-mcp`，还是作为 `pla-app mcp` 子命令（倾向后者：复用现有 CLI 入口，少一个 exe）。
3. 无人值守 agent 的 LLM 端点（v1 待确认 1）——P3 之前不需要。
