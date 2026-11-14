# A1 几何查询: 实测与接口(26.2.3, 2026-11-15, MT_screws 副本)

自动建模 pipeline 的第一块地基(`docs/PRD/PRD-ai-modeling-v2.md` §2/§4): 让程序和大模型能读出"现在模型里有什么"。
全部只读, 不改文档、选择、视角。

## 数据从哪来(实测)

几何库 `editor.geo.lookupTopologyItemById(拓扑 id)` 返回 `{view, model}`; `model` 是 Parasolid 封装对象,
方法名首字母大写(源码是字节码, 看不到实现, 以下全部靠调用实测):

| 对象 | 方法 | 返回 |
|---|---|---|
| 面 `Face` | `GetSurface()` | `{surface: Plane / Cylinder / Cone / Torus / Sphere / BlendSurf…, orientation, uMin, vMin, uMax, vMax}` |
| | `FindMidpoint()` | `{position, normal}`(面上中点 + **外**法向; 内凹圆角的法向指向圆心) |
| | `GetRadius()` / `GetBlendRadius()` | 圆柱 / 倒圆半径(米) |
| 曲面 | `GetInfo()` | 圆柱等: `{radius, basis}`; 平面: `Basis` |
| `Basis` | getter `Location` / `Axis` / `Ref` | 轴心、轴向、参考方向 |
| 边 `Edge` | `GetCurve()` | `{curve: Line / Circle / …, sense, uMin, uMax}` |
| 曲线 | `GetInfo()` | 直线 `{origin, direction}`; 圆 `{radius, basis}` |
| | `FindLength()` | `{length, min, max}` |
| | `GetVertices()` | `{left, right}`(顶点对象 `Vertex`: `GetPoint()`、`GetIdentifier()`) |
| 实体 `SolidBody` | `FindBox()` | `{min, max}` |
| | `GetCentroid()` | **包围盒中心, 不是质心**(L 形支架实测 = 盒心) |

- 拓扑 id: 面/边见 command-dispatch.md 10.(`<物体>f<n>` / `<物体>e<n>`)。**顶点没有拓扑 id**
  (`lookupTopologyItemById` 不认 `<物体>v<n>`), 查询里自拟 `<物体版本id>v<顶点标识>`, 只在同一版本内有效。
- 速度: 31 实体 / 481 面逐面 `GetSurface + FindMidpoint` 8 ms, 993 边 `GetCurve + FindLength` 14 ms。
- 体积 / 面积 / 质心: 内核没有对外接口, 用显示网格计算(见下文 A2)。

## 接口

垫片 `geo.query`(能力 `geo.read`; 内部名字都在 `NAMES.geoApi`):

```
args: { detail?: 'solids'(默认) | 'faces' | 'edges' | 'all',
        of?: [物体版本 id | {name: 大纲名}],  units?: 'millimeter' | …(默认文档单位),
        surface?: 'Plane' | 'Cylinder' | …,  curve?: 'Line' | 'Circle' | …,
        normal?: [x,y,z], angleTol?: 度(默认 1),  near?: [x,y,z], tol?: 文档单位(默认 0.5, 到包围盒中心),
        limit?: 每类上限(默认 2000) }
→ { units,
    solids:   [{id, name, bbox, faces, edges}],
    faces?:   [{id, solid, bbox, surface, center, normal, radius?, axis?, axisOrigin?}],
    edges?:   [{id, solid, bbox, curve, length, start, end, direction? | radius?, axis?, axisOrigin?}],
    vertices?:[{id, solid, point}],          // detail=all; 由边端点去重
    truncated?: {faces?: 被截掉的数量, …} }
```

- 长度按文档单位输出, 方向为单位向量, 数值保留 7 位有效数字。单项读失败写 `err`, 不影响其它项。
- 返回的面/边 id 与 `cmd.exec` 的选择器是同一套, 可以直接拿去倒圆、偏移。
- `near` 用包围盒中心(与 `cmd.exec` 的 `{near}` 选择器一致), 圆弧边的包围盒中心不在弧上。

Rust: `pla_bridge::query::state`(一次批量: `hist.state` + `sel.snapshot` + `geo.query`) →
CLI `pla-app cdp-state [--hwnd N] [--detail …] [--of 34,名字] [--units U] [--out 文件]`。

## A2 吸附点与测量(2026-11-15; 思路来自用户提示: 测量 / 自由移动命令里的吸附点, X-ray 下可点到内部点)

**吸附点来源**: `editor.snapScene`(`SnapScene`)按物体缓存 `canonicals: Map('Item:<物体版本id>|false' → entry)`,
`occurrences: [{entry, transform(16 个数, 列主序)}]`(实例变换)。新物体建完立即有条目(实测)。`entry.snapsForItem`:

| 路径 | 字段 | 含义 |
|---|---|---|
| `faces.centers` | `faceIds`, `positions`, `normals` | 平面面心 + 法向(圆柱等曲面不在其中) |
| `edges.halves` | `edgeIds`, `positions`, `tangents` | 边中点 + 切向 |
| `edges.circles` | `edgeIds`, `centers`, `axes`, `quarters`, `tangents` | 圆/圆弧: 圆心 + 轴; 弧上取点(R3 90° 弧每段 3 个) |
| `shellVertices` | `vertexIds`, `edgeIds`, `positions`, `tangents` | 顶点(**实体 id**, 与面/边拓扑 id 里的数字同一套) |
| `bbox.corners` / `faces.cvs` / `segments` / `curveVertices` | | 包围盒角 / 控制点 / 曲线段(实体上为空) |

`entry.positions` = 全部点按上表顺序排成的 Float32Array; `entry.lookup(i)` → 吸附对象(`FaceCenterPointSnap` 等, 带 `position/normal/faceSnap.view`)。
L 形支架: 8 面心 + 21 边中点 + 2 圆心 + 6 弧上点 + 14 顶点 = 51, 与 `positions` 数一致。
直接读缓存, 不进命令、不碰界面; X-ray 下能点到的内部点同样在缓存里(缓存不分可见性)。

**面积 / 体积 / 质心**: 内核没有对外方法(实体只有 `GetCentroid` = 包围盒中心; ContinuityAnalyzer 只管曲率/连续性)。
改用显示网格: `FaceGroup.mesh.geometry`(position Float32 + index), `FaceGroup.groups` = `[起点, 数量] × 面`(与 `versionIds` 同序);
三角形面积求和 = 面积, 有向四面体求和 = 体积与质心。曲面是近似: R3 圆角面 93.979 vs 理论 94.248(−0.29%),
L 形支架体积 6540.2 vs 6538.6(+0.025%); 平面精确。网格量保留 5 位有效数字(Float32 噪声在第 6 位)。

Plasticity 自己的数值: 右侧 SELECTION 卡片(组件 `StatsPanelBody`)状态里有 `solidVolume / solidArea / solidMass / faceArea /
edgeLength…`, 在 hook 里算(字节码), 显示时取整(L 形支架 6,500 mm³ / 3,450 mm²)。给模型原型方法挂记录器选中实体时没抓到
质量属性接口(窗口隐藏, 卡片也没算) → 暂用网格法; 要对照可在窗口可见时选中实体后读该组件状态。

**接口**(垫片; 能力 `snap.read` / `geo.read`):

```
geo.snaps   { of?, kinds?: [faceCenter|edgeMid|circleCenter|circleQuarter|vertex|bboxCorner], near?, tol?, units?, limit? }
            → { units, points: [{kind, solid, pos, id?(拓扑 id), normal|tangent|axis?}], noSnapData: [没缓存的物体], truncated? }
geo.measure { op: 'distance', a, b }  → { distance, delta, a, b, normalDistance?(两平面平行: 沿法向距离 = 壁厚) }
            { op: 'angle', a, b }     → { angle(度) }      // 面取法向, 直线边取方向, 数组当方向向量
            { op: 'mass', of?, faces? } → { solids: [{id, name, volume, area, centroid, faces?, method: 'mesh'}] }
参照 a/b: [x,y,z](文档单位) | '<物体>f<n>'(面心 + 法向) | '<物体>e<n>'(边中点 + 方向) | '<物体>v<n>'(顶点)
```

`geo.query` 的顶点也改用吸附数据里的顶点实体 id(没缓存时退回边端点 + Parasolid 标识)。
CLI: `pla-app cdp-call [--hwnd N] geo.snaps|geo.measure|… '<JSON>'`(任一垫片接口)。

真机(L 形支架 34): 底板 / 立板壁厚 `normalDistance` 均 5(面心直线距离 6.4); 顶面 ⟂ 立板内侧 90°;
质心 (213.22, 10, 8.2533), 手算 ≈ (213.226, 10, 8.255); 整文档 31 实体质量 112 ms。

## N3d 面签名与重新定位(2026-11-15)

面 id(`<物体>f<n>`)每次几何修改都会变(实测 84f9483 → 85f9483), 物体版本 id 也变, 但大纲名(Box.006)不变。

- `dim.signature {faces}` → 每面 `{id, solid, solidName, surface, center, normal, radius?, axis?, axisOrigin?, bbox}`(文档单位)。
- `geo.resolve {faces: [签名]}` → 每个签名 `{id, score(中心偏差), gap(次优 − 最优), sig(当前签名)}`:
  同名实体(没有同名 → 全部实体)→ 同曲面类型 → 法向平行(有轴的面比轴线平行)→ 中心最近。
- 改尺寸后的跟踪: 被移动的面中心 = 旧中心 + 已知平移量(厚度 = 法向 × Δ, 距离 = 方向 × Δ), 用预测签名定位, 偏差 0;
  半径 / 圆角的中心移动不好预测, 交给"最近匹配"。记录用定位到的当前签名刷新。
- 存储: `<文档目录>/.pla/models/<doc_key>/dims.json`(`pla_ui::dims`)。这也是 A9 映射表的位置与核心机制。

## A5 指定视角截图(2026-11-15)

垫片 `cam.view`(能力 `cam.write`): `{view?: front|back|left|right|top|bottom|iso(默认), of?, margin?: 1.15, maxWidth?: 960, keep?}`
→ `{view, data(PNG base64), width, height, target, zoom, restored}`。CLI `pla-app cdp-view [--hwnd N] [--view iso] [--of 34] [--out x.png] [--width W]`。

- 流程: 记下 `cam.get` → 透视则临时 `togglePerspective` 切正交 → 相机朝向由"视角方向 + 上方向"构造四元数(前视 = 绕 X +90°,
  与早先实测可用的值一致; iso = 右前上 (1,−1,1)) → 对准物体包围盒中心 → 包围盒 8 角投影到屏幕轴取半宽 / 半高,
  `zoom = top / (max(半高, 半宽 / aspect) × margin)`(正交 ProxyCamera: 可见半高 = `top / zoom`, top 固定 3)→ `cam.thumbnail`
  → 还原相机与投影。只动视角, 不改文档; 窗口隐藏时照样能渲染(实测)。
- 坑: 要把轨道控制器的 `focalOffset` 清零 —— 平移过视图会留下焦点偏移(实测 (0.016, 0.072, 0) m), 放大到 100 多倍后画面被推离物体,
  截出来只剩方向立方体。
- 真机(物体 34): 前 / 等轴 / 顶 三张, 物体居中占画面高 ~87%, 圆角清楚; 截完相机四元数 / 位置 / 缩放 / 半径与截前完全一致。

## 真机核对(L 形支架, 物体 34)

`cdp-state --detail all --of 34` 278 ms: 9 面 = 8 平面(法向 ±X/±Y/±Z 各就各位)+ 1 圆柱(r 3, 轴 −Y, 轴心 (208,·,8),
中点法向 (0.707, 0, 0.707) 指向圆心); 21 边 = 19 直线 + 2 圆弧(长 4.712 = 3π/2, r 3); 14 顶点(截面 7 点 × 2)。
整个文档 `--detail all`: 203 ms, 872 KB(31 实体 / 481 面 / 993 边 / 642 顶点, 0 错误) —— 给大模型用时先看概览, 再按物体 / 条件细查。
