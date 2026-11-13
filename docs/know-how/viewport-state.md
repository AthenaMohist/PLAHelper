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
