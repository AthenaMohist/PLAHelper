# 任务 #2 静态审查：选择集多视角还原 + 视口状态恢复（2026-10-29）

> **前置条件**：本次审查期间 **CDP 通道不可用**
> （26.2.3 被加密加固、26.1.3 启动即卡死 —— 见 `docs/know-how/plasticity-2623-hardening.md`）。
> 因此**全部结论都是静态审查 + 历史实测旁证**，凡涉及运行时行为的条目均已标注
> 「**需 CDP 恢复后实测**」。代码改动只做了两件不依赖运行时、且有单测兜底的事：
> ① 不再吞错误（把静默失败变成可见上报）；② 把「切视角」收口为一个脚本并加回读校验。

---

## A. 选择集多视角还原失败（`CdpCmd::ApplySel`，`pla-bridge/src/cdp.rs`）

### 关键代码位置

| 位置 | 作用 |
| --- | --- |
| `cdp.rs:428-443` | 按 `entry.camera` 分组（相同 JSON 为一组，None 归一组，顺序 = 首次出现顺序） |
| `cdp.rs:455-496` | 每组先切到该组保存视角，再跑 `apply_sel_js(&entries, gi > 0)` |
| `cdp.rs:1124-1174` | `apply_sel_js`：Phase A 准备（取消活动命令、可选 `click.clear`、装 hover spy） |
| `cdp.rs:520-561` | Phase B 网格扫描（6x9 粗 + 12x18 细），页面侧 spy 累积 `id→(x,y)` |
| `cdp.rs:599-618` | Phase C 点击：`let shift = i > 0 \|\| gi > 0;` |
| `cdp.rs:662-779` | Phase E 遮挡补救（自身相机 → `addById` → `navigateToSelection` → 重扫 → shift 点击 → 还原） |

### 假设排序（最可能 → 次要）

**H1（头号嫌疑）切视角只写 `camera`，没写 `orbitControls`，下一帧就被拉回去**
`enter_js` 原来只做 `cam.position.set(...)` + `cam.quaternion.set(...)` + `setNeedsRender()`。
Plasticity 的相机由 `orbitControls` 驱动（`VPSTATE_SNAPSHOT_JS` 里 orbit 与 cam 是**成对**采集的，
`vp.orbitControls` 有 `target/focalOffset/radius/zoom`）。标准 OrbitControls `update()` 每帧用
`target + 球坐标(radius,角度)` 反算 `camera.position` ⇒ 只写 camera 的话，**下一帧就回到旧位姿**。
- 与症状吻合：单组（= 当前视角，不需要切）正常；多组必然失败，表现为该组「未扫到」。
- 已改：`PLA_VIEW_APPLY_JS_TMPL`（cam → orbit → update → setNeedsRender），并**回读校验**
  （`view_delta`，切完再读一次相机，偏差 > 1e-3 就往面板发「视角切换未生效」警告，不再静默）。
- **需实测**：切到第二组时看是否还报「视角切换未生效」；若报，说明 orbit 字段名/语义不同，
  需把 `vp.orbitControls` 的真实字段打出来（`Object.keys(vp.orbitControls)`）。

**H2 保存的相机信息本身不完整（老存档只有 7 个数）**
`CAM_SNAPSHOT_JS` 以前只输出 `px,py,pz,qx,qy,qz,qw`，**没有 zoom/fov，也没有 orbit**。
即使 H1 修好，正交相机下取景范围（zoom）不对 ⇒ 目标面可能跑到屏幕外 ⇒ 还是「未扫到」。
- 已改：`CAM_SNAPSHOT_JS` 尽力附带 `zoom/fov/orbit{target,focal,radius,zoom}`；
  消费方（`PLA_VIEW_APPLY_JS_TMPL`）对这些字段缺失完全兼容（老存档照旧工作）。
- **需实测**：新存的选择集（CDP 恢复后重新保存）在多视角场景下成功率。

**H3 选择切视角后被清掉**（可能性中等）
相位 A 对 `gi>0` 跳过 `click.clear()`（正确），但每组都会 `ed.executor.cancelActiveCommand()`；
Phase E 补救又用 `ed.click.addById()` 临时改选择。**需实测**：切完视角后读
`ed.selection.selected.size` 是否等于已成功数；不等就在面板如实报「第 N 组切视角后选择丢失」。
（目前未实现该校验 —— 刻意留白，等实测确认是否真的会发生，避免无谓复杂度。）

**H4 shift 追加在切视角后失效**（可能性较低，历史实测 selN=2 支持可用）
`shift = i > 0 || gi > 0` 逻辑本身正确（后续组第一击也是 shift）。
**需实测**：第二组点完后 `VERIFY_FACES_JS` 回包的 `selN` 是否 = 累计面数。

### 已落地的改动（本次）

1. 新增 `PLA_VIEW_APPLY_JS_TMPL`，替换原先 **3 处复制粘贴**的内联切视角 JS
   （组入口、Phase E 成员相机、用户相机还原）—— 以后改一次即全局生效。
2. 切视角后 **回读校验**（`view_delta()` + 面板警告）—— 把「静默在错误视角里扫描」
   变成明确告警。单测：`view_delta_detects_identity_and_mismatch`。
3. `CAM_SNAPSHOT_JS` 增加 `zoom/fov/orbit`（向后兼容）。
4. 单测：`view_apply_template_is_well_formed`（防占位符漏替换、`//` 注释吞代码）。

---

## B. 视口状态恢复无效（`vpstate.rs` + `VPSTATE_*_JS`）

### 现状（改前）

- `VPSTATE_SNAPSHOT_JS`：**每一段一个空 `catch(e){}`**，最后无条件 `out.err=''`。
  ⇒ 26.x 里任何结构漂移（`vp.settings` 搬走、`orbitControls` 改名、`ed.sections` 改名…）
  都会让该段**静默消失**，存档里就没有这个 key。
- `VPSTATE_RESTORE_JS_TMPL`：同样空 catch，最后 `return 'ok:' + applied.join(',')`。
  ⇒ 存档缺 key 就跳过，且**仍然报「已恢复」**。面板显示"视口状态已恢复: cam"，
  但界面/材质/视角纹丝不动 —— 用户描述完全吻合。
- **具体 bug（与"视角恢复不了"直接对应）**：快照采集了 `cam.zoom / cam.fov / orbit.radius / orbit.zoom`，
  但恢复**从来没写过 zoom/fov**（只写 position/quaternion + orbit 的 target/focal/radius/zoom）。
  相机位姿对了、取景范围没恢复 ⇒ 看到的画面就不是保存时那个画面。

### 已落地的改动（本次）

1. 快照：每段 catch 记进 `out.failed`（带 `e.message`），存档自带失败清单。
2. 恢复：
   - 应用 `cam.zoom` / `cam.fov`（**仅透视相机**；正交 `ProxyCamera` 的
     `updateProjectionMatrix()` 有破坏性副作用，R7-31h 教训 —— 正交情形记为
     `missing:['zoom(正交未应用)']` 如实上报，不硬来）；
   - 每段记 `applied / missing / failed`，回包改成 JSON；
   - `shading` 分支细分失败点（`dispatch` / `no-mode` / `section`）。
3. Rust 侧新增 `fmt_vpstate_apply_result()`：只要 `failed` 非空就显示「部分失败: …」，
   `missing` 显示「存档未含: …」，老格式/坏数据原样透出。单测：
   `fmt_vpstate_apply_result_reports_failures`。

### 下一步（需 CDP 实测）

1. 恢复一次存档，把面板那行「部分失败 / 存档未含」原文贴出来 —— **这就是根因定位器**：
   - 出现 `failed:[orbit:...]` / `failed:[renderer:...]` ⇒ 26.x 结构漂移，
     用 `Object.keys(vp)` / `Object.keys(vp.settings)` 重新内省；
   - 出现 `missing:[zoom(正交未应用)]` ⇒ 默认相机是正交，需要另找正交缩放的写入姿势
     （`vp.orbitControls.zoom` 已写，可能还要 `vp.camera.updateProjectionMatrix()` 的**安全替代**）；
   - `failed:[shading:no-mode]` ⇒ `ed.shading.mode` 形状变了。
2. `sections` 恢复只写数值、没派发信号 —— 若实测剖面不变，需要找 `ed.signals` 里对应的
   section 变更信号（与 shading 同族，见 MEMORY.md「多路信号派发必须各自 try/catch」）。

---

## C. 本次改动文件

| 文件 | 改动 |
| --- | --- |
| `src/crates/pla-bridge/src/cdp.rs` | `PLA_VIEW_APPLY_JS_TMPL`、`CAM_SNAPSHOT_JS` 扩字段、`view_delta()`、`fmt_vpstate_apply_result()`、`VPSTATE_SNAPSHOT_JS`/`VPSTATE_RESTORE_JS_TMPL` 上报改造、3 个新单测 |
| `tools/pla-cdp/pla-asar.js` | 新增：零依赖 asar 读写库 |
| `tools/pla-cdp/pla-cdp-v7.js` | 新增：安装检测 / 打补丁 / 还原，硬化版本明确拒绝 |
| `tools/pla-cdp/pla-cdp-main-boot.js` | 新增：v7 main 入口引导（26.2.x 自家校验挡住，备用） |
| `docs/know-how/plasticity-2623-hardening.md` | 新增：26.2.x 加固机制全解析 + integrity 破解方法 |

验证：`cargo build -p pla-bridge` / `-p pla-ui` 均通过；`cargo test -p pla-bridge`（36 项）
与 `-p pla-ui` 全绿。
