# D0 出图编译器验证 · FreeCAD TechDraw 路线(无界面, freecadcmd 跑)。
# 用法: freecadcmd.exe d0_freecad.py  (参数走环境变量: D0_STEP=<零件 STEP, 空 = 内置测试板>  D0_OUT=<输出目录>)
# 产出: <out>/d0.dxf(整页)、<out>/views/*.svg(各视图片段)、<out>/d0.FCStd(可在 FreeCAD 里打开看)、<out>/report.json(耗时 / 视图 / 尺寸数)
import json
import os
import time

import FreeCAD as App
import Part
import TechDraw

T0 = time.time()
OUT = os.environ.get("D0_OUT") or os.path.join(os.path.dirname(os.path.abspath(__file__)), "out")
os.makedirs(os.path.join(OUT, "views"), exist_ok=True)
STEP = os.environ.get("D0_STEP", "")
V = App.Vector
rep = {"steps": {}, "warnings": []}


def lap(name):
    rep["steps"][name] = round(time.time() - T0, 3)


def test_plate():
    """80×50×12 板: 左边沉头孔 Φ6.6 / Φ11×6.5, 右边通孔 Φ5.5, 中间 30×16×4 槽。"""
    s = Part.makeBox(80, 50, 12)
    s = s.cut(Part.makeCylinder(3.3, 12, V(15, 25, 0)))
    s = s.cut(Part.makeCylinder(5.5, 6.5, V(15, 25, 12 - 6.5)))
    s = s.cut(Part.makeCylinder(2.75, 12, V(65, 25, 0)))
    s = s.cut(Part.makeBox(30, 16, 4, V(25, 17, 8)))
    return s


doc = App.newDocument("d0")
if STEP:
    shape = Part.Shape()
    shape.read(STEP)
else:
    shape = test_plate()
part = doc.addObject("Part::Feature", "Part")
part.Shape = shape
bb = shape.BoundBox
rep["part"] = {"source": STEP or "test_plate", "bbox": [round(bb.XLength, 3), round(bb.YLength, 3), round(bb.ZLength, 3)], "faces": len(shape.Faces)}
lap("part")

tpl_dir = os.path.join(App.getResourceDir(), "Mod", "TechDraw", "Templates", "ISO")
page = doc.addObject("TechDraw::DrawPage", "Page")
tpl = doc.addObject("TechDraw::DrawSVGTemplate", "Template")
tpl.Template = os.path.join(tpl_dir, "A3_Landscape_ISO5457_minimal.svg")
page.Template = tpl

# 第一角三视图(国标): 主视 = 从 -Y 看(前), 俯视、左视由投影组按第一角排
grp = doc.addObject("TechDraw::DrawProjGroup", "Proj")
page.addView(grp)
grp.Source = [part]
grp.ProjectionType = "First angle"
grp.ScaleType = "Custom"
grp.Scale = 1.5
grp.addProjection("Front")
grp.addProjection("Top")
grp.addProjection("Left")
grp.X, grp.Y = 150, 170
doc.recompute()
for v in grp.Views:
    v.HardHidden = True  # 虚线显示不可见轮廓(圆心标记是界面侧属性, 在 d0_export_gui.py 里开)
doc.recompute()
lap("projgroup")
views = {v.Type: v for v in grp.Views}
front = views.get("Front")

# 轴测
iso = doc.addObject("TechDraw::DrawViewPart", "Iso")
page.addView(iso)
iso.Source = [part]
iso.Direction = V(1, -1, 1)
iso.XDirection = V(1, 1, 0)
iso.Scale = 1.0
iso.X, iso.Y = 340, 220

# A-A 剖视: 过两孔轴线(y = 25), 从前往后剖, 放在主视右边
sec = doc.addObject("TechDraw::DrawViewSection", "SectionA")
page.addView(sec)
sec.BaseView = views["Top"]
sec.Source = [part]
sec.SectionNormal = V(0, -1, 0)
sec.SectionOrigin = V(bb.Center.x, 25, bb.Center.z)
sec.Direction = V(0, -1, 0)
sec.SectionSymbol = "A"
sec.Scale = 1.5
sec.X, sec.Y = 330, 90
try:
    sec.CutSurfaceDisplay = "PatHatch"
except Exception as e:  # 不同版本属性名不同
    rep["warnings"].append("hatch: " + str(e))
doc.recompute()
lap("views")


def edges_of(view):
    """视图里的 2D 边: (名字, 边)。"""
    out = []
    i = 0
    while True:
        try:
            e = view.getEdgeByIndex(i)
        except Exception:
            break
        if e is None:
            break
        out.append(("Edge%d" % i, e))
        i += 1
    return out


def add_dim(view, typ, refs, name, at=None):
    """at = 文字位置(相对视图中心, 页面毫米; TechDraw 的 Y 向上)。不给 = TechDraw 默认(都堆在中心, 不能用)。"""
    d = doc.addObject("TechDraw::DrawViewDimension", name)
    d.Type = typ
    d.References2D = [(view, r) for r in refs]
    page.addView(d)
    if at:
        d.X, d.Y = at
    return d


GAP = 8.0  # 尺寸线离轮廓的距离(页面毫米); 多条同侧尺寸依次再往外 GAP


dims = []
# 俯视: 圆 → 直径(孔标注); 外框 → 长宽
top = views["Top"]
# 视图里的边坐标 = 已乘比例、以视图中心为原点的页面毫米(Y 向上)
hx, hy = bb.XLength * grp.Scale / 2, bb.YLength * grp.Scale / 2
circles = [(n, e) for n, e in edges_of(top) if isinstance(e.Curve, Part.Circle)]
seen_r = set()
for n, e in (sorted(circles, key=lambda ne: -ne[1].Curve.Radius)):
    r = round(e.Curve.Radius, 3)
    if r in seen_r:
        continue
    seen_r.add(r)
    c = e.Curve.Center
    # 引出到视图外: 圆心在左半 → 往左上, 右半 → 往右上; 同侧多个依次往上错开
    side = -1 if c.x < 0 else 1
    dims.append(add_dim(top, "Diameter", [n], "Dia%d" % len(dims), (side * (hx + GAP * 2), hy + GAP * (1 + len(seen_r)))))
lines = [(n, e) for n, e in edges_of(top) if isinstance(e.Curve, Part.Line)]
if lines:
    longest_x = max(lines, key=lambda ne: abs(ne[1].Vertexes[0].Point.x - ne[1].Vertexes[-1].Point.x))
    longest_y = max(lines, key=lambda ne: abs(ne[1].Vertexes[0].Point.y - ne[1].Vertexes[-1].Point.y))
    dims.append(add_dim(top, "DistanceX", [longest_x[0]], "LenX", (0, -(hy + GAP))))
    dims.append(add_dim(top, "DistanceY", [longest_y[0]], "LenY", (hx + GAP, 0)))
# 主视: 高度(放右侧)
if front:
    fl = [(n, e) for n, e in edges_of(front) if isinstance(e.Curve, Part.Line)]
    if fl:
        tall = max(fl, key=lambda ne: abs(ne[1].Vertexes[0].Point.y - ne[1].Vertexes[-1].Point.y))
        dims.append(add_dim(front, "DistanceY", [tall[0]], "Height", (-(hx + GAP), 0)))
doc.recompute()
lap("dims")
rep["dims"] = [{"name": d.Name, "type": d.Type, "value": round(float(d.getRawValue()), 3) if hasattr(d, "getRawValue") else None} for d in dims]

# 标题栏字段
try:
    texts = dict(tpl.EditableTexts)
    rep["titleblock_fields"] = sorted(texts.keys())
except Exception as e:
    rep["warnings"].append("titleblock: " + str(e))

# 导出
TechDraw.writeDXFPage(page, os.path.join(OUT, "d0.dxf"))
lap("dxf")
for v in [*grp.Views, iso, sec]:
    try:
        svg = TechDraw.viewPartAsSvg(v)
        with open(os.path.join(OUT, "views", v.Name + ".svg"), "w", encoding="utf-8") as f:
            f.write('<svg xmlns="http://www.w3.org/2000/svg" viewBox="-150 -150 300 300">' + svg + "</svg>")
    except Exception as e:
        rep["warnings"].append("svg %s: %s" % (v.Name, e))
lap("svg")
rep["views"] = {v.Name: {"type": getattr(v, "Type", v.TypeId), "edges": len(edges_of(v))} for v in [*grp.Views, iso, sec]}
doc.saveAs(os.path.join(OUT, "d0.FCStd"))
lap("total")
with open(os.path.join(OUT, "report.json"), "w", encoding="utf-8") as f:
    json.dump(rep, f, ensure_ascii=False, indent=1)
print(json.dumps(rep, ensure_ascii=False))
