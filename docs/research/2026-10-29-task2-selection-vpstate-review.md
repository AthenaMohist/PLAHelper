# 任务 #2：选择集多视角还原 + 视口状态恢复（2026-10-29，含实测）

> **实测环境**：`app-26.1.3` + v6 补丁，CDP 9333 正常（`model1.plasticity` 已打开）、官方 bridge 8980 正常。
> 26.2.3 无 CDP（见 `docs/know-how/plasticity-2623-hardening.md`），所以**26.2.3 上的一切仍待实测**。
>
> ⚠ **启动陷阱（踩过一次）**：从 WorkBuddy/终端里拉起 26.1.3 时环境里有
> `ELECTRON_RUN_AS_NODE=1` 和 `NODE_OPTIONS=--require=...`，旧版 Electron 会照单执行 →
> **表现为"启动即挂死"（36MB 单进程、无日志、无端口）**。必须先
> `env -u ELECTRON_RUN_AS_NODE -u NODE_OPTIONS` 再启动。26.2.3 的 fuse 关了这两个开关，
> 所以只有旧版会中招 —— 这也是我一度误判"26.1.3 已死"的原因。

---

## 0. 实测结论速览

| 项目 | 结论 |
| --- | --- |
| #2B 视口状态恢复 | ✅ **已修好并像素级验收通过**：恢复后截图与基线**完全一致（MAD=0）** |
| #2A 选择集多视角 | 根因已定位到「成员相机信息不完整」（无 zoom/orbit），已修；**端到端仍需 GUI 实测** |
| 「相机写完会被 orbitControls 拉回」 | ❌ **已被实测否掉**：静止状态下写 `camera.position` 会保留（2 秒后仍在，派发 pointermove 也不回弹） |

---

## 0.1 实测发现的运行时机制（26.1.3，CDP 直读）

```
vp.camera          : ProxyCamera, isPerspectiveCamera = true, zoom = 11.2167, fov = 35
vp.orbitControls   : target / targetEnd, focalOffset / focalOffsetEnd,
                     quaternion / quaternionEnd, radius / radiusEnd, zoom / zoomEnd
                     → **每个量都是 x / xEnd 两份, 阻尼插值**
vp.settings        : Viewport{renderer, ao}, Grid, Camera, OrbitControls, … (全部健在)
ed.shading          : mode{tag, materials, isTinted, prev}, _sectionMode, signals.shadingChanged
ed.sections.sections: 健在
vp._constructionPlane: 健在
```

三条硬事实（逐步实测得出）：

1. **写 `camera.position` 不会被 orbitControls 拉回**（静止 2 s 不回弹；派发合成
   pointermove 也不回弹）。⇒ 多视角失败不是"相机写不住"。
2. **`cam.zoom` 与 `oc.zoom` 是两个量**，互相不派生；但
   `camera.updateProjectionMatrix()` 会**按当前位姿重算 zoom / near / far**
   （实测：相机挪到 40 单位外后调用它，zoom 从 7 变成 0.1182，near/far 也一起变）。
   ⇒ 恢复顺序必须是：**先 orbit（两端）→ 再 cam（pos/quat/zoom/fov）→ 最后
   updateProjectionMatrix + setNeedsRender**。
3. **orbit 的 `*End` 必须一起写**：只写当前值而 `xEnd` 还是旧值时，后续帧会把相机
   插值回旧取景（这就是"恢复了但视角又漂走"的机理）。

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

**H1（已被实测否掉）~~切视角只写 `camera`，下一帧就被 orbitControls 拉回去~~**
实测：写完 `cam.position.x += 40` 后 2 s 仍在（40.71），派发合成 `pointermove` 后也仍在。
所以"相机写不住"**不是**根因。（写 orbit.target 也不会带动 camera —— 二者不是被动联动的。）

**H1′（现在的头号嫌疑）成员相机存档信息不完整：只有 7 个数，没有 zoom/orbit**
`CAM_SNAPSHOT_JS` 以前只输出 `px,py,pz,qx,qy,qz,qw`。切到"保存视角"时位姿对了但
**取景范围（zoom/fov、orbit 半径）不对** ⇒ 目标面跑到屏幕外或极小 ⇒ 该组「未扫到」
⇒ 整个多视角还原失败。这与 #2B 的根因**是同一个**（见下）。
- 已改：`CAM_SNAPSHOT_JS` 附带 `zoom/fov` 与 `orbit{target,targetEnd,focal,focalEnd,radius,radiusEnd,zoom,zoomEnd}`；
  `PLA_VIEW_APPLY_JS_TMPL` 成对写 x/xEnd，老存档（只有 7 个数）照旧可用。
- **仍需 GUI 端到端实测**（要真的保存一个多视角选择集再应用）。
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

### 实测结果（26.1.3，CDP 真机）

验收方法：基线截图 → 扰动（相机挪位 + 转向 + 缩到 6 + shading 改 uniform）→ 恢复 → 再截图，
用 `pngdiff.js`（自写极简 PNG 解码器，无依赖）比对。**渲染是确定性的**（同一状态连拍两张
MAD=0），所以 MAD 是可信指标。

```
基线 vs 扰动      : MAD 7.87   (ratio 0.115)  ← 确实变了
基线 vs 恢复后    : MAD 0      (ratio 0)      ← ★ 逐像素完全一致
```

恢复回包：`{"applied":["proj","cam","orbit","shading","sections:0","cplane","renderer","ao","grid"],
"missing":[],"failed":[]}` —— 快照各段在 26.1.3 上**全部健在**（没有结构漂移）。

**根因确认**：旧代码从未恢复 `cam.zoom/fov`（采集了却不写），也没写 orbit 的
`*End` —— 位姿对了、取景不对，用户看到的就是"视角恢复不了"。修复后逐像素还原。

### 下一步（26.2.3 上待做）

1. 在 26.2.3 上跑一次同样的往返，看面板那行「部分失败 / 存档未含」：
   - `failed:[orbit:...]` / `failed:[renderer:...]` ⇒ 结构漂移，用
     `Object.keys(vp)` / `Object.keys(vp.settings)` 重新内省；
   - `missing:[zoom(正交未应用)]` ⇒ 默认相机是正交，需另找正交缩放写入姿势；
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
