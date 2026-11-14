# 视口状态(显示相关)的完整保存与恢复 —— Plasticity 26.2.3 实测(2026-11-14)

目标: "视口状态"要把**所有与显示相关**的东西完整保存/恢复: 视角(旋转/缩放/平移)、截面、隐藏零件、
shader mode、render mode、x-ray、摄像机参数、网格线、各类显示开关。

全部结论用 CDP 在 SS001(快照副本)上实测, 每条都做过"改 → 截图 → 改回 → 截图像素一致"的验证。
实验脚本: `tools/lab/lab.js`(页面里执行 JS)、`tools/lab/shot.js`(CDP 截 Plasticity 页面/视口)、
`tools/diag/png-diff.js`(像素差)。

## 验收手段(重要)

* **不能**用 `vp.renderer.domElement.toDataURL()` 验收: 没开 preserveDrawingBuffer, 读回的是旧缓冲,
  X-ray 开关前后指纹都一样。
* 用 `node tools/lab/shot.js <hwnd> out.png vp`(CDP `Page.captureScreenshot`, 经 `__PLA_PROXY_SENDS`)。
  连续两次截图像素完全一致(MAD 0), X-ray 切换能测到(ratio ≈ 1.3%)—— 可作像素级验收。
* 函数源码被 Plasticity 隐藏(`toString()` 返回空白), 只能靠行为实验。

## 状态地图

| 内容 | 位置 | 读 | 写(已验证) |
|---|---|---|---|
| 视角 + X-ray + overlays + 构造面 + 着色 | `vp.snapshot()` / `vp.init(s)` | snapshot 里 camera{mode,position,quaternion,zoom,fov}, target, focalOffset, isXRay, showOverlays, constructionPlane{n,o}, shading | `vp.init(改过的 snapshot)` 一次到位, 改回原 snapshot 像素一致 |
| 缩放 | `vp.orbitControls.zoom / zoomEnd` | snapshot.camera.zoom = 轨道控制器 zoom; `camera.zoom` 是派生值(差 ~0.6%) | 写 `oc.zoom` + `oc.zoomEnd`(阻尼两份); `vp.init` **不**恢复缩放 |
| render mode | `vp.isRenderMode` / `vp.toggleRenderMode()` | getter | toggle; 与右上角圆形图标等价(进入: per-object 材质 + studio 环境 + AO; 退出复原) |
| 视口标量 | `vp.fov / squish / curvatureScale / g0Scale / g1Scale / g2Scale` | getter | setter(均为 gs) |
| 显示开关 | `ed.layers.isShowingEdges / Faces / Curves / Meshes / Images / Constraints / ControlPoints / ContinuityLabels`, `isXray` | getter | setter |
| 着色 | `ed.shading.snapshot()`: mode, matcap(Texture, `userData.filename`), shader(材质实例, `.name`), isTinted, isSectioned, environment, rotation, intensity, shaderProperties | | 见下 |
| 截面模式 | `ed.shading._sectionMode.tag`、`ed.sections.sections` | | 旧实现 |
| 网格/色调/AO(文档级) | `ed.document.settings.snapshot()`: Grid{factor,every,size}, Display{ao{…}, toneMapping{type,exposure}}, Unit | | `init(s)` / `update(partial)` 写值有效; **会让文档变"未保存"** |
| 隐藏 / 眼睛关闭 / 隔离 | `ed.nodes.read.hidden`(Set<nodeKey>)、`invisible`、`isolations`; `read.node2id`: nodeKey → **稳定 id**(字符串, 跨会话不变) | | **只能**走 memento: `m = ed.originator.snapshot(); m.nodes = {...m.nodes, hidden: Float64Array.from(keys), invisible: …}; ed.originator.restore(m)` |

## 隔离(isolate, 2026-11-14 白天实测)

* `nodes.read.isolations` = 层栈(外 → 内), 每层是 nodeKey 的 `Set`; `nodes.isolationLevel` = 层数。memento 里是 `Float64Array[]`。
* **memento 里的 isolations 恢复时被忽略**(改了 restore 也不变)。
* 正确做法: `nodes.unisolate()`(一次清空**全部**层) → 每层 `nodes.isolate(new Set(keys))` 压栈 →
  `originator.restore(originator.snapshot())` 原样恢复一次让场景重算(不做这步显示不变)。实测两层往返: 0 层 22 个可见 ↔ 2 层 1 个可见。
* 存档按稳定 id: `vis.isolations = [[id…], [id…]]`。
* 26.2.3 上 `executor.hasActiveCommand` 是**常为 true 的属性**(默认选择命令常驻), 不能拿来"拒绝恢复";
  改为恢复前 `cancelActiveCommand()`(同 sel.apply)。

## 渲染环境(render mode 下)

* `shading.snapshot().environment` 是纹理, `userData.filename` 如 `.././img/outdoor_cloudy_01_1k.exr`; rotation / intensity 有 setter。
* **`shading.setEnvironment(filename)` 不能用**: 会把环境清空(实测事故, 点 outdoor_cloudy_01 按钮恢复)。
* 渲染面板有 8 个环境缩略图按钮, 图片名 = 环境文件名去扩展名再去 `_1k` 后缀(`outdoor_cloudy_01.png` ↔ `outdoor_cloudy_01_1k.exr`)。
  恢复 = 点对应按钮 + 写 rotation / intensity。

## 截面(section, 待实测)

* 视口截面在 `shading.section`(Section, Object3D: position / quaternion 即截面平面位姿)、`shading.isSectioned`、
  `shading._sectionMode.tag`(none / …)。旧实现存的 `ed.sections.sections`(SectionDatabase)是另一回事(截面分析), 不是视口截面。

## 材质状态(R34, 2026-11-14 白天实测, MT_screws 副本)

* 数据: `ed.materials`(BasicMaterialDatabase)的 `snapshot()` = `{version, counter, materials:[glTF 风格定义]}`, 定义带稳定 `uuid`
  与文档内 `id`; 指派在 `nodes.read.node2material`(nodeKey → materialId), 稳定 id 走 `node2id`。`db.get(id)` / `getMutable(id)`
  返回同一个 three.js `MeshPhysicalMaterial`, 改它的 `color` 后 `snapshot()` 立刻反映(baseColorFactor 由材质实时导出)。
* 恢复: memento 的 `materials` + `nodes.materials`(类型化数组对)换掉再 `originator.restore` —— 不进撤销栈、几何版本不变,
  窗口可见时不抛错。往返实测: 存(1 材质/12 指派)→ 改色为红 + 去掉一个物体的指派 → 恢复 → 数据逐字节一致, 截图与改动前像素一致(MAD 0)。
* **恢复后视口不自己重画**: 数据已变但画面停在旧颜色, 要等下一次交互; 垫片在 restore 后主动 `vp.render`(R34c)。
* 找不到的物体(`node:<稳定id>`)/材质(`material:<uuid>`)如实列在 missing, 其余照常应用。
* 自检(`pla-app cdp-selftest`)含 `mat.restore ← mat.save`: 原样恢复应识别为 `materials(same)`, 不碰 memento。

## 坑

1. **`nodes.setHidden(item, true)` 只改集合, 场景不刷新**(scene.update / renderList.fullRecompute /
   originator.advanceVersion 都没用, 截图不变)。走 `originator.restore(memento)` 才会重算场景
   (与撤销同一路径; 不进撤销栈, 实测 undo 栈长度不变)。只改 nodes 的三项, 其它取**当下**的快照, 等于原地刷新。
2. **着色恢复必须点 Shader Mode 面板里的按钮**(DOM click, 不是桌面鼠标): shader 是材质实例
   (`IsophotesMaterial` 等), 没有按名字查找的注册表, `setShader('name')` 抛 "undefined is not a function"。
   * `shading.setMatcap(filename, tint)` 能按文件名加载 matcap, **但在 shader 模式下调用会把 matcap 弄成 undefined**,
     `TextureLoader.lookup` 每帧抛异常 → **渲染循环停摆, 视口冻结**(实测事故, 点 ceramic_lightbulb 按钮后恢复)。
     所以 matcap 也一律点按钮; setMatcap 只作"自定义 matcap 且当前不是 shader 模式"的兜底。
   * 面板 15 个球: 6 个 matcap(ceramic_dark / metal_carpaint / dark_shiny / gradient / ceramic_lightbulb / color_matcap→basic_side+tint),
     black_silhouette / color_silhouette(都是 basic_side, 内部状态与 color_matcap 难区分), 7 个 shader:
     isophotes / draft / zebraHorizontal(reflection_check_horizontal) / zebraVertical(…vertical) / topo / faceDirection(face-direction) / curvature,
     各带 shaderProperties(如 zebra `{bands:16}`)。
   * 点按钮后再 `shading.init({...当前snapshot, shaderProperties: 存的值})` 恢复参数。
3. **右侧面板数值不跟着刷新**: 程序改了 squish / 网格后, 面板还显示旧值(React 组件自持状态, 派发
   settingsChanged / documentSettingsChanged / shadingChanged / viewportActivated / vp.changed 都不刷新)。
   真实状态是对的(截图里网格/模型正确)。待解决(TODO): 找到让面板重新挂载的方法, 或改走面板自己的输入控件。
4. 右上角 4 个图标: 第 4 个(圆)= render mode 开关; 前 3 个尚未弄清(点击不改变着色/渲染状态)。
5. memento 恢复 / 文档设置写入后标题出现"●"(未保存)。

## 实测事故记录(SS001, 已复原)

一次 A→B→A 往返里, B 用了 zebra shader, 回 A 时调 `setMatcap` → matcap 变 undefined → 视口冻结、面板停在 B。
处置: 点 ceramic_lightbulb 按钮恢复 matcap → 渲染恢复; render mode 被探测脚本打开后用同一图标关掉;
缩放写回原值。最终视口与 A 像素一致(仅右侧面板残留旧数字)。SS001 标题有"●", 未保存。
