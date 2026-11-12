# 路线决策：破解深挖 vs bridge + 平行壳 + 第三方（2026-11-12）

> **最终决定见 §8（Plasticity 优先）**。§3–§7 的"平行壳 / SW 中枢"方案已被用户否决，仅留作记录。
> 不变的一条：**不往"解密 / 反编译 / 改内核调用"方向投入**。

## 0. 本次分析的范围

只用了可观察、不绕过保护的材料：安装目录文件清单、用户配置文件 `~/.plasticity/config/v2/*.json`、
用户自己的 `.plasticity` / `.plasticitypack` 文件头、仓库内既有 know-how 文档。
**没有**解密 `PLSEC001`、反编译字节码、读取授权文件。理由见 §2。

## 1. Plasticity 是怎么实现的（可观察事实）

| 层 | 实现 | 证据 |
|---|---|---|
| 外壳 | **Electron 42.11.8**（Chromium + Node），定制 exe + fuses 关闭注入开关 | `version` 文件；hardening 文档 |
| 业务代码 | 主进程/渲染进程 JS 均为**加密容器 PLSEC001**（26.2 起），asar 内容自校验 | hardening 文档 §1–§5 |
| UI | **Preact** 真实 DOM + **three.js** 视口（`<plasticity-viewport>`） | editor-cdp-facts |
| 几何内核 | **Siemens Parasolid**：`pk.node`（38MB N-API 绑定）+ `pskernel.dll`（75MB）+ `psbodyshop.dll`；文档 BIN 段 = Parasolid transmit（P37.0.245） | `app.asar.unpacked/.webpack/renderer/` |
| 格式转换 | **ODA Teigha/Drawings SDK 26.8**（`TD_*_26.8_16`，DWG/DXF）、**HOOPS Exchange**（`A3DLIBS.dll`）、一组 `*step30.dll / wxt / wsat / wiges / xnkernel / TfKernel`（STEP/X_T/SAT/IGES 转换器） | 同上 |
| SubD→NURBS | `PolyhedralSplinesLib.dll` | 同上 |
| 文档格式 | `plasticity` magic + 版本 + 2×长度 + `JSON` 头（相机/单位/材质/节点树…）+ `BIN` Parasolid transmit | 用户 `.plasticity` 与 `screws.plasticitypack` 文件头（**asset pack 与文档同一容器**） |
| 官方扩展面 | ① `keymap.json` / `settings.json` / `theme.json`（JSON5，用户可编辑）② WebSocket bridge :8980（**下发仅网格、不含材质**；唯一写入 PUT_SOME = Blender Outbox → PolySplines 网格转 NURBS，Studio 功能，不能编辑已有实体）③ 导入/导出 STEP/X_T/SAT/IGES/OBJ/STL/3MF/SVG/asset pack ④ asset pack | 配置文件实读；protocol-facts |
| 插件 API | **无**（无脚本、无插件机制） | — |

新确认的一条：`settings.json` 里 `OrbitControls.rotateDiscreteIncrement = 0.2617993877991494`
= **15°，单位是弧度**（关闭了旧调研的待验证项 V-05）。

## 2. 为什么不"死磕源代码和破解"

1. **能拿到的东西不比现在多。** 解密字节码最多得到 JS 逻辑；而 CDP 运行时内省已经能直接访问
   `editor`/`commands`/`selection`。真正的几何能力在 Parasolid 原生库里，读 JS 也拿不到。
2. **跟不上版本。** 26.1→26.2 一次加固就让旧补丁全灭；厂商在主动加固，每次升级都要重来。
3. **法律与授权风险集中在这里。** 解密/规避保护、绕过内容校验违反 EULA，在多数法域还触及反规避条款；
   绕过 Plasticity 直接调用 `pk.node`/Parasolid 等于**无授权使用 Siemens 内核**——这条线不能碰。
4. **Plasticity 没有参数化历史、没有装配、没有工程图。** 你的 3/4/5/6 号需求，核心能力本来就不在它里面，
   破解它也解决不了。

**现有 CDP 通道（fuse 单字节 + inspector）怎么处理：**
它只在你自己的机器上启用 Electron 自带的调试器，已经能用，保留。但它仍是 EULA 灰区、且随版本失效，所以：
- 收敛到一个适配层（`pla-bridge::cdp`），**所有功能必须在 CDP 不可用时有降级路径**或明确提示；
- 不分发打过补丁的程序或补丁工具；
- 新功能**优先选不依赖 CDP 的通道**，只有"必须读写 Plasticity 实时状态"的功能才用它。

## 3. 推荐架构：Plasticity 只当"交互造型前端"

```
┌─────────────────────────── PLAhelper（Rust, 编排 + 覆盖式 UI）───────────────────────────┐
│ 面板/工具栏 · 方案管理 · 规格库 · 任务调度                                                  │
│  适配器:  [配置文件] [文件 I/O 监听] [bridge 8980 只读] [OS 热键/截图] [CDP 可选加速]       │
└──────┬───────────────┬──────────────────────┬───────────────────────┬────────────────────┘
       │ STEP/glTF/JSON │ 子进程/RPC           │ WS/文件               │ 文件/API
┌──────▼──────┐ ┌──────▼──────────────┐ ┌─────▼──────────────┐ ┌──────▼─────────────────┐
│ Plasticity  │ │ 几何后端 Python      │ │ SolidWorks Add-in  │ │ FreeCAD(无头) / COMSOL │
│ 交互直接建模 │ │ build123d / OCCT     │ │ C#: 导入/装配/工程图│ │ TechDraw / mph 仿真    │
└─────────────┘ │ 孔工具·模型库·AI建模 │ └────────────────────┘ └────────────────────────┘
                └──────────────────────┘
```

- **交换格式**：B-rep 一律 **STEP AP242**（带颜色）；预览/网页用 **glTF**；Plasticity 内复用用 **asset pack**。
- **几何后端选 build123d（OpenCascade）**：开源、可脚本、STEP 读写、布尔/圆角/螺纹都有，
  也是目前"文本→CAD"最成熟的代码形态（LLM 写 Python，比驱动 Plasticity 内部命令稳得多）。
- **注意**：asset pack 的 BIN 段是 Parasolid transmit，**外部无法生成**（需要 Parasolid 授权）。
  所以模型库的规范格式是 STEP；要做成 asset pack，需在 Plasticity 里导入后导出一次（可半自动）。

## 4. 七项需求逐项落地

| # | 需求 | 推荐通道 | 依赖 CDP | 阶段 | 估时 |
|---|---|---|---|---|---|
| 2 | 按键步进视角 | **改 keymap.json + settings.json**；PLAhelper 面板一键切换 15/30/45/90° | 否 | P0 | 0.5–2 天 |
| 1 | 配色/材质方案一键切换 | 方案数据存 `<doc>/.pla/materials/`；**实时应用走 CDP 命令**（`SetMaterialCommand` 族）；降级 = 改文档 JSON 头的 `materials[]`/`nodes[].material` 写副本后重开 | 是（降级不需要） | P0 | 4–7 天 |
| 5 | 打孔工具 | ① 规格库（螺纹底孔/自攻孔按材料/螺套/热熔螺母/沉头/过孔，可录入+上传模型）② build123d 批量生成"负体 + 紧固件"→ STEP → 导入成 asset pack（升级你现在的手工流程）③ 二期：CDP 读选中面/点，自动放置+布尔+放螺丝 | ①② 否，③ 是 | P1 | ①② 5–8 天；③ 5–10 天 |
| 3 | SolidWorks bridge | STEP 文件交换 + C# Add-in（`LoadFile4`）+ 本地控制通道；双向实时联动不可行（Plasticity 无导入 API，X_T 是 P37 太新） | 仅"自动导出"一步 | P1 | 13–21 天（先 1–2 天宏 PoC） |
| 4.2 | 模型库 / datasheet 重建 | **参数化族**（build123d 模板按 datasheet 生成任意规格）为主；商城下载为辅、**按需下载+本地缓存**；索引 SQLite + 全文检索 + 缩略图，PLAhelper 搜索面板 | 否 | P2 | 2–3 周起 |
| 4 | AI 建模全流程 | LLM 生成 build123d 代码 → OCCT 求解 → 校验（包围盒/体积/截图）→ STEP 导入 Plasticity 精修。取代原 PRD 的"CDP 派发 Plasticity 命令"方案 | 否 | P2 | M1 PoC 1–2 周 |
| 4.3 | 出 2D/3D 图、HTML 3D | 2D：FreeCAD TechDraw 无头脚本（或 SW 工程图）；3D 网页：glTF + three.js 单文件 HTML（带测量） | 否 | P2 | 1–2 周 |
| 4.1 | COMSOL 仿真 | `mph`（Python 调 COMSOL API）读 STEP（需 CAD Import Module），参数扫描结果回写规格 | 否 | P3 | 视模型 |
| 6 | 装配模拟 + 公差 + 自动出图 | 放在 **SolidWorks**（配合、TolAnalyst 需 Premium、工程图 API）或 **FreeCAD 1.x Assembly**；一维尺寸链先用 Python 计算 | 否 | P3 | 月级 |
| 7 | wrap / 上层 UI | **PLAhelper 本身就是这个 wrap**：覆盖式 UI + 编排器。不建议把 Plasticity 窗口嵌进别的程序（Electron/GPU 下 SetParent 很脆） | 否 | 持续 | — |

## 5. 排期

- **P0（本周）**：#2 改配置（含 26.2.3 命令 id 核对：在 Plasticity 的快捷键编辑器里搜 rotate）；
  #1 CDP PoC（单实体改材质 → 方案保存/应用），同时验证"改 JSON 头写副本"降级路径。
- **P1（2–4 周）**：#5 规格库 + 生成器；#3 SW 宏 PoC → Add-in。
- **P2（1–3 月）**：build123d 几何后端 + AI 建模 M1；模型库；HTML 3D / FreeCAD 出图。
- **P3（长期）**：装配/公差（SW 或 FreeCAD）、COMSOL 闭环。

## 6. 用户确认（2026-11-12）与本机实查

- SolidWorks **Pro 2026 SP3**（`SLDWORKS.exe` 34.4.1）；**Toolbox 已装**（`C:\SOLIDWORKS Data\ToolboxStandards.xml`）。
- COMSOL **6.4 Multiphysics**；SW 插件列表里已注册 **LiveLink for COMSOL (6.4)**（授权是否含该模块待在 COMSOL 里确认）。
- 模型库：批量入库**只要常用件**；具体商家具体型号用到再下。
- 用户提出的"包一层 UI"= 像 Blender 那样在**另一个视口**里操作模型、跑设计流程，Plasticity 当工具库 → 见 §7。

## 7. 修订：外壳选 SolidWorks，而不是自研视口

### 7.1 "Plasticity 当工具库"的三种做法

先澄清 Blender 的模式：Blender **不调用** Plasticity 的建模工具，它只是通过 8980 Live Link
**接收网格**，然后在自己里面做渲染/UV/材质。造型动作仍然全部发生在 Plasticity 窗口里。

| 方案 | 做法 | 可行性 | 判定 |
|---|---|---|---|
| A 自研视口 + 远程调用 Plasticity 命令 | 外壳里点"圆角"，后台通过 CDP 让 Plasticity 执行，再把结果网格拉回来显示 | Plasticity 无 API，命令参数格式未公开、依赖 gizmo 交互；Plasticity 必须开着窗口运行；结果 B-rep 留在 Plasticity 里，外壳拿到的只是网格；每次升级都可能失效 | ❌ 本质还是深度依赖破解通道 |
| B 自研视口 + 自有内核（OCCT）+ Plasticity 造零件 | 自己做装配树/视口/特征，零件进出靠 STEP | 技术可行，但等于重新写一个 CAD（装配、约束求解、工程图、视口交互），月到年级工作量，且功能必然不如现成 CAD | ⚠️ 不划算 |
| **C SolidWorks 当外壳 + Plasticity 当自由造型工作站 + PLAhelper 当桥** | Plasticity 负责零件形状；SW 负责装配、孔/紧固件、材质外观、工程图、仿真对接；PLAhelper 负责一键传递、方案管理、库检索 | 全部用官方、有文档、跨版本稳定的接口（STEP + SW API + COMSOL LiveLink）；你已有授权 | ✅ **推荐** |

### 7.2 方案 C 对开发计划的覆盖

| # | 需求 | 在哪里做 | 说明 |
|---|---|---|---|
| 1 | 配色/材质方案 | Plasticity（建模时，CDP）+ **SW 外观 API**（工程阶段，稳定） | SW 的外观/材质 API 完整公开，方案切换做在 SW 插件里零风险 |
| 2 | 按键步进视角 | Plasticity 配置文件；SW 原生支持（选项→视图→方向键旋转增量） | 两边都不需要破解 |
| 3 | SW bridge | 骨干通道 | Plasticity 导出 STEP → SW 插件自动导入/更新实体 |
| 4.1 | COMSOL | **LiveLink for SOLIDWORKS**（已注册） | SW 参数改动可同步到 COMSOL，比 STEP 单向导入强得多 |
| 4.2 | 模型库 | SW Toolbox（GB/ISO 紧固件、轴承、销）+ 自建常用件库 + 商家型号按需下载 | 商家多提供 SLDPRT 原生件，SW 端直接用 |
| 4.3 | 出 2D/3D 图 | SW 工程图 API；3D 网页：eDrawings 或导出 glTF 做 three.js 单页 | — |
| 4 | AI 建模 | LLM 生成 **SW API 宏**（特征有历史、可再编辑）或 build123d → STEP | SW API 文档完整，LLM 训练语料多 |
| 5 | 打孔工具 | **异型孔向导 + Toolbox + Smart Fasteners**（螺纹底孔/过孔/沉头、自动配螺丝原生就有）；自攻孔/螺套/热熔螺母 = 自定义孔规格 + **Smart Component**（插入螺母时自动在宿主上切出关联底孔） | 一期主要是**配置 SW 数据库 + 录规格**，而不是写几何算法 |
| 6 | 装配 + 公差 + 出图 | SW 配合、干涉检查、基础运动算例、工程图；DimXpert 标注 | **Pro 没有 TolAnalyst**（Premium 才有）→ 尺寸链先用 Python/Excel 一维计算 |
| 7 | 外壳 UI | SW + PLAhelper（Plasticity 侧覆盖式 UI 已有；SW 侧做 TaskPane 插件） | — |

### 7.3 方案 C 的优缺点

**优点**
- 计划里 3/4/5/6 的核心能力（装配、孔向导、紧固件、工程图、COMSOL 联动）SW 都有成熟实现，只需二次开发，不用造轮子。
- 依赖的全是官方 API，SW/COMSOL/Plasticity 升级基本不影响；Plasticity 侧只用 STEP 导出，不依赖 CDP。
- 工程数据（装配、孔、BOM、图纸）落在行业标准格式，可交付给别人。

**缺点 / 代价**
- **单向流**：Plasticity → SW。在 SW 里加的孔、配合挂在导入实体的面上；回 Plasticity 改形状再导入时，面引用可能断。
  缓解：严格分阶段（"Plasticity 定型 → SW 工程化"）；孔位尽量用草图 + 基准面定位而非直接选导入面；用 SW 的"替换实体/更新导入"保留尽可能多的引用。
- 两个软件来回切换，上下文切换成本在；PLAhelper 要把"一键推送/一键回到 Plasticity 编辑该零件"做顺。
- 导入 SW 的是无特征实体（FeatureWorks 只能识别棱柱/回转类特征，自由曲面识别不了）——这正好符合分工：自由曲面留在 Plasticity。
- 公差分析受 Pro 版限制。

### 7.4 修订后的排期

- **P0（本周）**：#2 Plasticity 配置；#1 Plasticity 侧材质 PoC（CDP）。
- **P1（2–4 周）**：SW 插件骨架（C#，TaskPane）+ STEP 自动导入/更新；用 Plasticity 导出的 X_T 在 SW 2026 实测一次能否直读（能读则 X_T 比 STEP 更保真）；
  #5 一期：自定义孔规格 + 热熔螺母/螺套 Smart Component + 规格录入界面。
- **P2（1–3 月）**：常用件库（Toolbox 配置 + 自建库 + 使用频率统计决定入库）；SW 外观方案切换；工程图自动化；COMSOL LiveLink 打通。
- **P3（长期）**：AI 建模（LLM 生成 SW 宏/ build123d）、装配流程模拟、尺寸链计算。

### 7.5 模型库"批量但只要常用"的做法

- 标准件优先用 **Toolbox**（按需生成规格，不占库）；
- 自建"常用件白名单"（导轨、型材、联轴器、螺套、热熔螺母等），一次性入库：SLDPRT + STEP + 缩略图 + 规格字段，SQLite 索引供 PLAhelper 搜索；
- "常用"由**你自己的使用频率**决定（PLAhelper 记录插入次数），不去抓商城下载量；
- 具体商家具体型号：用到时下载、自动归档进库，下次直接搜到。

## 8. 最终决定：Plasticity 优先（2026-11-12，用户拍板）

用户否决"bridge + 另一个 UI 视口"和"SW 当外壳"：做 PLAhelper 的初衷是**把 SolidWorks 里顺手的操作迁移进
Plasticity**，让 Plasticity 本身更好用，而不是另起炉灶。§7 的问题在于把目标换成了"用最低风险覆盖功能清单"，
重心因此滑回了 SW。

### 8.1 立场调整

- **CDP 运行时通道从"可选加速层"升为主通道**：要在 Plasticity 里读写实时状态（选择、材质、放置资产、布尔），
  没有插件 API 的前提下只能靠它。
- **仍然不解密、不反编译**：运行时内省已经能拿到 `editor` / `commands` / `selection`，解密代码不会多给任何能力，只加风险。
- **SW 降级为"出口"**：仅在需要给用 SW 的同事/供应商交付时，STEP 导出 + SW 侧自动导入，低优先级。

### 8.2 让 CDP 主通道可持续（工程要求）

1. **JS 垫片层**：一次注入页面一个带版本号的 `__PLA__` 对象，对外只暴露稳定接口
   （`getSelection / setMaterial / placeAsset / boolean / orbitStep …`）。Rust 只调垫片；Plasticity 升级时只修垫片。
2. **能力探测 + 契约测试**：连接时逐项探测所需内部结构，结果变成功能开关；面板上明示哪些功能在当前版本失效，绝不静默失败。
   提供 `pla-app cdp-selftest` 一键跑全套契约。
3. **锁版本**：关闭 Plasticity 自动更新，保留已验证版本的安装包；新版本先跑契约测试再切换。
4. **走命令而非改数据库**：优先调用 Plasticity 自己的 Command，保证撤销栈完整（见 feasibility §2.6）。
5. **renderer 重载后自动重定位 Editor**（交接文档待办 #2）。
6. 不分发打过补丁的程序或补丁工具。

### 8.3 共同地基：命令派发层

#1 材质、#5 打孔、#4 AI 建模都需要"带参数可靠地执行 Plasticity 命令"——即 PRD-ai-modeling 的 M-A。
它是接下来最值得投入的一块，做成后三个功能共用。

### 8.4 七项需求（Plasticity 优先版）

| # | 做法 | 依赖 |
|---|---|---|
| 2 | 改 keymap/settings，PLAhelper 面板切换步进角 | 配置文件 |
| 1 | 方案存 `<doc>/.pla/materials/`，CDP 调材质命令一键应用 | CDP |
| 5 | **把 SW 异型孔向导的体验搬进 Plasticity**：选面/点 → 选规格（螺纹底孔/自攻孔/螺套/热熔螺母/沉头/过孔，可录入+上传模型）→ 自动放负体、定长度、布尔、放螺丝实例 | CDP + 规格库 + asset |
| 4.2 | 常用件库 + PLAhelper 搜索面板 → 一键放入当前文档（STEP 导入或 asset pack） | CDP（放置）/ 文件 |
| 4 | 外部生成几何（LLM 写 build123d → STEP）→ 导入 Plasticity；命令派发层成熟后再试直接驱动 Plasticity 命令 | 文件 → CDP |
| 4.1/4.3 | COMSOL、出图走 STEP 导出；网页 3D 用 glTF + three.js | 文件 |
| 3 | 仅作交付出口 | 文件 |
| 6 | **Plasticity 的真实天花板**：没有配合约束和工程图。先做轻量版（实例 + PLAhelper 记录的装配关系 + 干涉检查 + Plasticity 自带隐藏线/SVG 导出），完整工程图暂不追求 | — |
| 7 | 覆盖式 UI（现状）继续打磨：主题跟随 `theme.json`、快捷键风格与 Plasticity 一致 | — |

### 8.5 优先级

完整待办（含现有功能 bug、UI 硬伤、参数化修改重做）见 **`docs/TODO.md`**，以那里为准。顺序：
步进视角 → UI 硬伤 → 选择集/视口状态/快照墙 bug → CDP 地基 → 材质方案 → 命令派发层 → 参数化修改 → 打孔工具 → 常用件库 → AI 建模 → 轻量装配。

### 8.6 场外建议

向 Plasticity 官方（论坛/Discord）提需求：脚本或插件 API、材质方案、孔工具。一旦有官方 API，8.2 的大部分工程负担会消失。
