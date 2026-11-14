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
- 体积 / 面积 / 质心: 还没找到接口(A2 测量时再找)。

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

## 真机核对(L 形支架, 物体 34)

`cdp-state --detail all --of 34` 278 ms: 9 面 = 8 平面(法向 ±X/±Y/±Z 各就各位)+ 1 圆柱(r 3, 轴 −Y, 轴心 (208,·,8),
中点法向 (0.707, 0, 0.707) 指向圆心); 21 边 = 19 直线 + 2 圆弧(长 4.712 = 3π/2, r 3); 14 顶点(截面 7 点 × 2)。
整个文档 `--detail all`: 203 ms, 872 KB(31 实体 / 481 面 / 993 边 / 642 顶点, 0 错误) —— 给大模型用时先看概览, 再按物体 / 条件细查。
