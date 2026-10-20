# PRD: AI 直接建模工作流（一句话 → 调研 → 定尺寸 → 出模型）v1

日期: 2026-10-20 · 状态: 待审批 · 来源: R7-19 用户提案

## 1. 可行性基础（已验证资产）

| 资产 | 状态 | 说明 |
|---|---|---|
| CDP 常驻通道 (pla-bridge/cdp.rs) | ✅ 已通 | Eval 任意 JS 到 renderer, 500ms 轮询 |
| editor 上帝对象 | ✅ 已通 | selection/geo/history/commands 全可达 |
| 全指令调用 | ✅ 逆向确认 | renderer 内命令系统可派发所有 Plasticity 指令 |
| 状态回读 | ✅ 已通 | 实体树/选择快照/undo 深度/geo_ver |
| mesh 顶点提取 | ✅ 已通 | dim 数据源, 可做几何校验 |
| pla-app cdp-eval CLI | ✅ 已通 | 外部进程可一次性 eval |
| 官方 bridge (8980) 协议解码 | ✅ 已通 | 只读增量, 作冗余数据源 |
| 自动快照 (undo 步数触发) | ✅ 已通 | 建模每步自动留档, 天然适配 |

**缺口**: ① 各建模指令的命令名/参数格式未逐一逆向 ② LLM 编排层不存在 ③ 校验→回滚→重试闭环未建。

## 2. 目标工作流

```
一句话需求
  → ① 资料调研 (WebSearch/domain_search, 产出精品调研摘要)
  → ② 计算/仿真定尺寸 (LLM 推导 + 公式; 需要时走 COMSOL Route B)
  → ③ 生成建模计划 JSON (有序指令序列, 每步含参数与预期)
  → ④ CDP 派发执行 (逐条 eval, 失败即停)
  → ⑤ 校验回读 (实体树+mesh 包围盒+截图 vs 预期)
  → 不符 → undo 回滚到步前 → 修正参数重试 (最多 N 轮)
```

## 3. 方案对比

| 方案 | 形态 | 优点 | 缺点 | 判定 |
|---|---|---|---|---|
| A | LLM 嵌进 pla-ui (Rust 直调 API) | 单 exe 交付 | Rust 写 agent 迭代极慢, HTTP/JSON 处理痛苦 | ❌ |
| B | 外部 agent 每步手工 cdp-eval | 零新代码 | 无状态封装, 每步裸 JS, 不可复现 | ❌ |
| **C** | **pla-app 做「通道 CLI」稳定层 + Python 编排 agent + LLM 产结构化 JSON** | 分层清晰: Rust 层极稳, agent 层快速迭代, 计划 JSON 可审计可重放 | 多一个脚本入口 | ✅ 推荐 |

方案 C 分层:
- **稳定层 (Rust, 已基本就绪)**: `pla-app cdp-eval` / 新增 `cdp-state`(输出树+选择+包围盒+undo 的 JSON) — 只做透明管道, 不含智能。
- **agent 层 (Python, 快速迭代)**: 调 LLM API → 产建模计划 JSON → 逐条派发 → 回读校验 → undo 重试。挂了不伤 Rust 层。
- **LLM**: 只负责「调研摘要 / 尺寸推导 / 计划 JSON」, 不直接碰 JS — 消除幻觉注入风险。

## 4. 分期计划

| 期 | 内容 | 验收 |
|---|---|---|
| M-A | 命令派发 PoC: cdp-eval 逆向命令注册表(ed.commands/工厂 API), 跑通 box/cylinder/extrude/fillet ≥4 条指令, 参数格式落 know-how 文档 | 在 Plasticity 里脚本画出 L 形支架(2 box+1 fillet) |
| M-B | `pla-app cdp-state` JSON 回读 CLI | 一条命令输出 LLM 可读的完整状态 |
| M-C | Python 编排 `pla_agent.py`: 一句话→调研→推导→计划 JSON→派发→校验→重试 | 一句话"画一个 20x10x5 带圆角底座"全自动完成且尺寸实测吻合 |
| M-D | 快照/收藏联动 + 失败诊断面板 | 每步自动快照, 失败步可人工接管 |

## 5. 待确认

1. LLM 端点选型: 硅基流动 DeepSeek(免费) / 智谱 / Pollinations 兜底 — 建议 M-C 先硬编码硅基流动, 后抽象。
2. M-A/B 纯技术逆向无 GUI 依赖, 可立即开跑 — 是否执行?
3. 调研环节深度: 轻量(单轮 WebSearch 摘要) vs 精品(多源交叉+对比表) — 建议 M-C 轻量, M-D 升级。
