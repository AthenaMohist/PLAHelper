# 出图编译器(FreeCAD TechDraw 后端): 出图方案 JSON + 制图规范 JSON → 图页 → PDF / SVG / FCStd + 报告。
# 一次跑完(建页 + 导出), 在离屏界面里:
#   $env:QT_QPA_PLATFORM='offscreen'; $env:PLA_DRAW_SPEC=<spec.json>; freecad.exe compile_freecad.py
# 也可无界面跑(只出 FCStd / DXF / 报告): freecadcmd.exe compile_freecad.py
#
# 方案 JSON(都可省, 省了按规则补):
# { "standard": "<规范 JSON 路径或名字 GB>", "paper": "A3", "scale": "auto" | 0.5,
#   "part": { "step": "<STEP 路径>" | "builtin": "test_plate" },
#   "front": [0,-1,0],                      主视方向(从哪边看, 默认从前 -Y 看)
#   "title": { "pn": "...", "name": "...", "material": "...", "finish": "...", "project": "...", "designer": "...", ... },
#   "tech": ["1. ...", ...],                技术要求(省 = 规范默认)
#   "section": "auto" | null | { "origin": [x,y,z], "normal": [0,-1,0], "symbol": "A" },
#   "holes": "auto" | [ { "at": [x,y,z], "axis": [0,0,1], "d": 6.6, "through": true, "cb": [11, 6.5], "csk": 12, "tap": "M4", "key": false, "tol": "H7 +0.015/0" } ],
#   "out": "<输出目录>", "basename": "<文件名>" }
import json
import math
import os
import sys
import time
import traceback

import FreeCAD as App
import Part
import TechDraw

HERE = os.path.dirname(os.path.abspath(__file__))
V = App.Vector
T0 = time.time()
rep = {"steps": {}, "warnings": [], "placed": []}


def lap(name):
    rep["steps"][name] = round(time.time() - T0, 3)


def q(x):
    """FreeCAD 的长度属性是 Quantity → 取毫米数。"""
    return float(getattr(x, "Value", x))


def load_json(p):
    with open(p, encoding="utf-8-sig") as f:
        return json.load(f)


# ---------------------------------------------------------------- 输入
SPEC_PATH = os.environ.get("PLA_DRAW_SPEC", "")
spec = load_json(SPEC_PATH) if SPEC_PATH else {}
std_ref = spec.get("standard", "GB")
std = load_json(std_ref if std_ref.endswith(".json") else os.path.join(HERE, "standards", std_ref + ".json"))
OUT = spec.get("out") or os.path.join(HERE, "out")
BASE = spec.get("basename") or (spec.get("title", {}).get("pn") or "drawing")
os.makedirs(OUT, exist_ok=True)
DIMS = std["dims"]
GUI = False
try:
    import FreeCADGui as Gui

    GUI = bool(Gui.getMainWindow())
except Exception:
    GUI = False


def fmt(v, dec=None):
    """尺寸数字: 规范小数位, 去尾零。"""
    dec = DIMS["decimals"] if dec is None else dec
    s = ("%." + str(dec) + "f") % v
    if DIMS.get("strip_zeros") and "." in s:
        s = s.rstrip("0").rstrip(".")
    return s


def test_plate():
    s = Part.makeBox(80, 50, 12)
    s = s.cut(Part.makeCylinder(3.3, 12, V(15, 25, 0)))
    s = s.cut(Part.makeCylinder(5.5, 6.5, V(15, 25, 12 - 6.5)))
    s = s.cut(Part.makeCylinder(3.3, 12, V(15, 10, 0)))
    s = s.cut(Part.makeCylinder(5.5, 6.5, V(15, 10, 12 - 6.5)))
    s = s.cut(Part.makeCylinder(2.75, 12, V(65, 25, 0)))
    s = s.cut(Part.makeCylinder(2, 6, V(65, 40, 6)))
    s = s.cut(Part.makeBox(30, 16, 4, V(25, 17, 8)))
    return s


# ---------------------------------------------------------------- 孔识别(没有 hole.make 记录时从几何找)
def find_holes(shape, axis_hint=None):
    """同轴的内圆柱面 → 一个孔: 最小半径 = 孔径, 贯穿 = 小径段覆盖整个实体厚度, 端部大径段 = 沉孔。返回 [{at, axis, d, through, depth, cb}]。"""
    bb = shape.BoundBox
    cyl = []
    for f in shape.Faces:
        s = f.Surface
        if not isinstance(s, Part.Cylinder):
            continue
        ax = s.Axis.normalize()
        # 内表面: 面上一点的法向指向轴线
        u0, u1, v0, v1 = f.ParameterRange
        p = f.valueAt((u0 + u1) / 2, (v0 + v1) / 2)
        n = f.normalAt((u0 + u1) / 2, (v0 + v1) / 2)
        c = s.Center
        rad = (p - c) - ax * ((p - c).dot(ax))
        if rad.Length < 1e-6 or n.dot(rad) > 0:
            continue  # 外圆柱(凸台)
        # 轴向范围
        ts = [(vx.Point - c).dot(ax) for vx in f.Vertexes] or [0]
        cyl.append({"ax": ax, "c": c, "r": s.Radius, "t0": min(ts), "t1": max(ts)})
    groups = []
    for cy in cyl:
        for g in groups:
            a = g[0]
            if abs(abs(a["ax"].dot(cy["ax"])) - 1) < 1e-6:
                d = cy["c"] - a["c"]
                off = d - a["ax"] * d.dot(a["ax"])
                if off.Length < 1e-4:
                    cy["t0"], cy["t1"] = sorted([(cy["c"] + cy["ax"] * cy["t0"] - a["c"]).dot(a["ax"]), (cy["c"] + cy["ax"] * cy["t1"] - a["c"]).dot(a["ax"])])
                    g.append(cy)
                    break
        else:
            groups.append([cy])
    holes = []
    for g in groups:
        a = g[0]
        ax, c = a["ax"], a["c"]
        # 实体沿轴向的范围(用包围盒八角投影)
        corners = [V(x, y, z) for x in (bb.XMin, bb.XMax) for y in (bb.YMin, bb.YMax) for z in (bb.ZMin, bb.ZMax)]
        ts = [(p - c).dot(ax) for p in corners]
        lo, hi = min(ts), max(ts)
        # 合并同半径段
        segs = {}
        for cy in g:
            r = round(cy["r"], 4)
            s0, s1 = segs.get(r, (1e9, -1e9))
            segs[r] = (min(s0, cy["t0"]), max(s1, cy["t1"]))
        rs = sorted(segs)
        rmin = rs[0]
        s0, s1 = segs[rmin]
        span_lo = min(v[0] for v in segs.values())
        span_hi = max(v[1] for v in segs.values())
        through = span_lo <= lo + 1e-3 and span_hi >= hi - 1e-3
        # 入口端 = 大径段所在端; 没有大径段 → 朝 +轴向那端
        entry_hi = True
        cb = None
        if len(rs) > 1:
            r2 = rs[-1]
            b0, b1 = segs[r2]
            entry_hi = abs(b1 - span_hi) < abs(b0 - span_lo)
            cb = [round(2 * r2, 4), round(b1 - b0, 4)]
        if not entry_hi:
            ax = -ax
        depth = None if through else round(s1 - s0 if cb is None else (s1 - s0) - 0, 4)
        top = c + a["ax"] * (span_hi if entry_hi else span_lo)
        holes.append({"at": [round(top.x, 4), round(top.y, 4), round(top.z, 4)], "axis": [round(ax.x, 6), round(ax.y, 6), round(ax.z, 6)],
                      "d": round(2 * rmin, 4), "through": through, "depth": depth, "cb": cb})
    return holes


def callout(h, n):
    """国标孔标注文字: 6 x ⌀4.3 贯穿 / 2 x ⌀6.6 贯穿 ⌴⌀11↧6.5 / 4 x M4 ↧10。"""
    D = DIMS
    s = D["count_format"].format(n=n) if n > 1 else ""
    if h.get("key"):
        s = D["key_mark"] + s
    if h.get("tap"):
        s += h["tap"]
    else:
        s += D["diameter"] + fmt(h["d"])
    if h.get("tol"):
        s += " " + h["tol"]
    s += " " + D["through"] if h.get("through") else (" " + D["depth"] + fmt(h["depth"]) if h.get("depth") else "")
    if h.get("cb"):
        s += "  " + D["counterbore"] + D["diameter"] + fmt(h["cb"][0]) + D["depth"] + fmt(h["cb"][1])
    if h.get("csk"):
        s += "  " + D["countersink"] + D["diameter"] + fmt(h["csk"])
    return s


# ---------------------------------------------------------------- 文档 + 零件
doc = App.newDocument("draw")
pspec = spec.get("part", {})
if pspec.get("step"):
    shape = Part.Shape()
    shape.read(pspec["step"])
    if shape.Solids:
        shape = shape.Solids[0] if len(shape.Solids) == 1 else Part.makeCompound(shape.Solids)
else:
    shape = test_plate()
part = doc.addObject("Part::Feature", "Part")
part.Shape = shape
bb = shape.BoundBox
rep["part"] = {"source": pspec.get("step") or "test_plate", "bbox": [round(bb.XLength, 3), round(bb.YLength, 3), round(bb.ZLength, 3)], "faces": len(shape.Faces), "volume": round(shape.Volume, 3)}
lap("part")

# ---------------------------------------------------------------- 字体 / 尺寸样式(TechDraw 首选项)
P = "User parameter:BaseApp/Preferences/Mod/TechDraw/"
App.ParamGet(P + "Labels").SetString("LabelFont", std["fonts"]["latin"])
App.ParamGet(P + "Labels").SetFloat("LabelSize", std["fonts"]["text"])
App.ParamGet(P + "Dimensions").SetFloat("FontSize", std["fonts"]["dim"])
App.ParamGet(P + "Dimensions").SetInt("StandardAndStyle", 0)  # ISO oriented(国标同)
App.ParamGet(P + "Dimensions").SetBool("UseGlobalDecimals", False)
App.ParamGet(P + "Dimensions").SetInt("AltDecimals", DIMS["decimals"])

# ---------------------------------------------------------------- 图页 + 模板
paper = spec.get("paper", "A3")
W, H = std["paper"][paper]
sys.path.insert(0, HERE)
import make_template  # noqa: E402

tpl_path = os.path.join(OUT, "_tpl_%s_%s.svg" % (std["name"], paper))
with open(tpl_path, "w", encoding="utf-8") as f:
    f.write(make_template.build(std, paper))
page = doc.addObject("TechDraw::DrawPage", "Page")
tpl = doc.addObject("TechDraw::DrawSVGTemplate", "Template")
tpl.Template = tpl_path
page.Template = tpl

# 可画区域(页面坐标, TechDraw: 原点左下, Y 向上): 内框之内、标题栏之上
fr = std["frame"]
inner_l, inner_r = fr["outer"] + fr["zone_band"], W - fr["outer"]
inner_t = H - fr["outer"] - fr["zone_band"]
area_b = 297 - std["title_block"]["rows_top"]  # 标题栏上沿(Y 向上; 标题栏跟着下边走, 与图幅无关)
AREA = (inner_l + 6, area_b + 6, inner_r - 6, inner_t - 6)  # l, b, r, t

# ---------------------------------------------------------------- 视图方向
front = V(*spec.get("front", [0, -1, 0]))
# 主视 / 俯视 / 左视在页面上的尺寸(模型单位): 由主视方向决定的三根轴
if abs(front.z) > 0.9:
    ax_w, ax_h, ax_d = V(1, 0, 0), V(0, 1, 0), V(0, 0, 1)
elif abs(front.x) > 0.9:
    ax_w, ax_h, ax_d = V(0, 1, 0), V(0, 0, 1), V(1, 0, 0)
else:
    ax_w, ax_h, ax_d = V(1, 0, 0), V(0, 0, 1), V(0, 1, 0)


def ext(a):
    return abs(a.x) * bb.XLength + abs(a.y) * bb.YLength + abs(a.z) * bb.ZLength


Wm, Hm, Dm = ext(ax_w), ext(ax_h), ext(ax_d)  # 主视宽 / 高, 深(俯视的高 = 左视的宽)

# ---------------------------------------------------------------- 比例: 国标优先系列里放得下的最大
SCALES = [5, 4, 2.5, 2, 1.5, 1, 1 / 1.5, 1 / 2, 1 / 2.5, 1 / 3, 1 / 4, 1 / 5, 1 / 10, 1 / 20]
MARGIN = 26.0  # 视图四周给尺寸留的地方(页面毫米)
GAPV = 22.0  # 视图之间
aw, ah = AREA[2] - AREA[0], AREA[3] - AREA[1]


def fits(s):
    # 左边三视图(主 + 左, 主 + 俯)占 ~70% 宽, 右边留给轴测 / 剖视
    w = (Wm + Dm) * s + GAPV + 2 * MARGIN
    h = (Hm + Dm) * s + GAPV + 2 * MARGIN
    return w <= aw * 0.72 and h <= ah


scale = spec.get("scale", "auto")
if scale == "auto":
    scale = next((s for s in SCALES if fits(s)), SCALES[-1])
rep["scale"] = scale


def scale_text(s):
    return "1:1" if abs(s - 1) < 1e-9 else ("%s:1" % fmt(s, 2) if s > 1 else "1:%s" % fmt(1 / s, 2))


# ---------------------------------------------------------------- 三视图(第一角)
grp = doc.addObject("TechDraw::DrawProjGroup", "Proj")
page.addView(grp)
grp.Source = [part]
grp.ProjectionType = std.get("projection", "First angle")
grp.ScaleType = "Custom"
grp.Scale = scale
for name in ("Front", "Top", "Left"):
    it = grp.addProjection(name)
    try:
        it.HardHidden = True  # 虚线(不可见轮廓); 建好就设, 别等后台消隐算完再改
    except Exception:
        pass
try:
    grp.Anchor.Direction = front
    grp.Anchor.XDirection = ax_w
except Exception as e:
    rep["warnings"].append("anchor: %s" % e)
try:
    grp.AutoDistribute = True  # False = 子视图全叠在锚点上
    grp.spacingX = GAPV + 0.0
    grp.spacingY = GAPV + 0.0
except Exception as e:
    rep["warnings"].append("spacing: %s" % e)
# 主视中心: 左上, 给左边尺寸留 MARGIN
fx = AREA[0] + MARGIN + Wm * scale / 2
fy = AREA[3] - MARGIN - Hm * scale / 2
grp.X, grp.Y = fx, fy
doc.recompute()
views = {v.Type: v for v in grp.Views}
lap("projgroup")


def wait_views(vs, timeout=20.0):
    """界面模式下 TechDraw 在后台线程算消隐, recompute 返回时边可能还没有 → 等到每个视图都有边。"""
    t = time.time()
    while time.time() - t < timeout:
        try:
            if all(v.getEdgeByIndex(0) is not None for v in vs):
                return True
        except Exception:
            pass
        if GUI:
            Gui.updateGui()
        time.sleep(0.05)
    rep["warnings"].append("views not ready after %.0fs" % timeout)
    return False


wait_views(list(grp.Views))


def vrect(v, w, h):
    """视图在页面上的矩形(l, b, r, t)。投影组子视图的 X / Y 是相对组锚点的偏移。"""
    cx, cy = vcenter(v)
    return (cx - w / 2, cy - h / 2, cx + w / 2, cy + h / 2)


def vcenter(v):
    if v in grp.Views:
        return q(grp.X) + q(v.X), q(grp.Y) + q(v.Y)
    return q(v.X), q(v.Y)


R = {}
R["Front"] = vrect(views["Front"], Wm * scale, Hm * scale)
R["Top"] = vrect(views["Top"], Wm * scale, Dm * scale)
R["Left"] = vrect(views["Left"], Dm * scale, Hm * scale)

# ---------------------------------------------------------------- 孔
holes = spec.get("holes", "auto")
if holes == "auto":
    holes = find_holes(shape)
rep["holes"] = holes

# ---------------------------------------------------------------- 剖视(过孔最多的那排, 平行主视)
sec = None
sec_spec = spec.get("section", "auto")
if sec_spec == "auto" and holes:
    # 孔轴线在"深度"方向上的坐标, 取孔最多的那个
    from collections import Counter

    cnt = Counter(round(V(*h["at"]).dot(ax_d), 2) for h in holes)
    dval = cnt.most_common(1)[0][0]
    o = V(bb.Center.x, bb.Center.y, bb.Center.z)
    o = o + ax_d * (dval - o.dot(ax_d))
    sec_spec = {"origin": [o.x, o.y, o.z], "normal": [front.x, front.y, front.z], "symbol": "A"}
if isinstance(sec_spec, dict):
    sec = doc.addObject("TechDraw::DrawViewSection", "SectionA")
    page.addView(sec)
    sec.BaseView = views["Top"]
    sec.Source = [part]
    sec.SectionNormal = V(*sec_spec["normal"])
    sec.SectionOrigin = V(*sec_spec["origin"])
    sec.Direction = V(*sec_spec["normal"])
    sec.XDirection = ax_w
    sec.SectionSymbol = sec_spec.get("symbol", "A")
    sec.Label = "%s-%s" % (sec.SectionSymbol, sec.SectionSymbol)  # 国标剖视名 "A-A", 界面里 ShowLabel 时显示
    sec.Scale = scale
    sec.ScaleType = "Custom"
    try:
        sec.CutSurfaceDisplay = "PatHatch"
    except Exception:
        pass
    # 放在俯视右边(左视下面那块空地), 与俯视同高
    sec.X = R["Top"][2] + GAPV + MARGIN + Wm * scale / 2
    sec.Y = (R["Top"][1] + R["Top"][3]) / 2
    R["Section"] = vrect(sec, Wm * scale, Hm * scale)
    # 剖视名 "A-A" 写在剖视上方(界面侧的视图标签导出时不一定显示, 用注释文字最稳)
    lab = doc.addObject("TechDraw::DrawViewAnnotation", "SectionLabel")
    page.addView(lab)
    lab.Text = [sec.Label]
    lab.TextSize = std["fonts"]["title"]
    lab.Font = std["fonts"]["latin"]
    lx_, ly_ = q(sec.X), R["Section"][3] + 7
    lab.X, lab.Y = lx_, ly_
    R["SectionLabel"] = (lx_ - 8, ly_ - 3, lx_ + 8, ly_ + 3)

# ---------------------------------------------------------------- 轴测(右上角, 剩余空间里能放多大放多大)
iso = doc.addObject("TechDraw::DrawViewPart", "Iso")
page.addView(iso)
iso.Source = [part]
iso.Direction = V(1, -1, 1) if front.y < 0 else V(1, 1, 1)
iso.XDirection = V(1, 1, 0) if front.y < 0 else V(1, -1, 0)
used_r = max(r[2] for r in R.values())
free_w = AREA[2] - used_r - 10
diag = math.sqrt(bb.XLength ** 2 + bb.YLength ** 2 + bb.ZLength ** 2)
iso_s = next((s for s in SCALES if diag * s <= max(free_w, 60) and s <= scale), SCALES[-1])
iso.Scale = iso_s
iso.ScaleType = "Custom"
iso.X = AREA[2] - diag * iso_s / 2 - 4
iso.Y = AREA[3] - diag * iso_s / 2.4 - 12
R["Iso"] = vrect(iso, diag * iso_s, diag * iso_s / 1.2)
doc.recompute()
wait_views([iso] + ([sec] if sec else []))
lap("views")


# ---------------------------------------------------------------- 尺寸放置(避让)
class Placer:
    """页面上已占用的矩形(视图 + 已放文字); 新文字按候选位置依次试, 取第一个不重叠的。"""

    def __init__(self, rects):
        self.used = [r for r in rects]

    @staticmethod
    def hit(a, b, pad=2.0):
        return not (a[2] + pad < b[0] or b[2] + pad < a[0] or a[3] + pad < b[1] or b[3] + pad < a[1])

    def free(self, r):
        inside = AREA[0] <= r[0] and r[2] <= AREA[2] + 4 and AREA[1] - 4 <= r[1] and r[3] <= AREA[3] + 4
        return inside and not any(self.hit(r, u) for u in self.used)

    def take(self, r):
        self.used.append(r)


def text_rect(cx, cy, text, size=None):
    size = size or std["fonts"]["dim"]
    # 实测(A3 PDF): 汉字 ≈ 1.0 × 字高, 数字 / 字母 / 符号 ≈ 0.85 × 字高(尺寸字体偏宽)
    w = sum(size * (1.0 if ord(ch) > 0x2E80 else 0.85) for ch in text)
    return (cx - w / 2, cy - size * 0.8, cx + w / 2, cy + size * 0.8)


placer = Placer(list(R.values()))
dims_out = []


def add_dim(view, typ, refs, name, page_xy, text=None, fmt_spec=None):
    d = doc.addObject("TechDraw::DrawViewDimension", name)
    d.Type = typ
    d.References2D = [(view, r) for r in refs]
    page.addView(d)
    vx, vy = vcenter(view)
    d.X, d.Y = page_xy[0] - vx, page_xy[1] - vy
    if fmt_spec:
        d.FormatSpec = fmt_spec
    if text is not None:
        d.Arbitrary = True
        d.FormatSpec = text
    dims_out.append(d)
    rep["placed"].append({"name": name, "type": typ, "view": view.Name, "text": text, "at": [round(page_xy[0], 1), round(page_xy[1], 1)]})
    return d


def edges_of(view):
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


def longest(view, axis):
    """视图里沿 x / y 最长的直线边。"""
    ls = [(n, e) for n, e in edges_of(view) if isinstance(e.Curve, Part.Line)]
    k = 0 if axis == "x" else 1
    return max(ls, key=lambda ne: abs(ne[1].Vertexes[0].Point[k] - ne[1].Vertexes[-1].Point[k]))[0] if ls else None


G, STEP = DIMS["gap"], DIMS["step"]
# 外形: 俯视 长(下)、宽(右); 主视 高(左)
top, frontv, leftv = views["Top"], views["Front"], views["Left"]
rT, rF = R["Top"], R["Front"]
e = longest(top, "x")
if e:
    p = ((rT[0] + rT[2]) / 2, rT[1] - G)
    add_dim(top, "DistanceX", [e], "LenW", p, fmt_spec="%." + str(DIMS["decimals"]) + "w")
    placer.take(text_rect(p[0], p[1], fmt(Wm)))
e = longest(top, "y")
if e:
    p = (rT[2] + G, (rT[1] + rT[3]) / 2)
    add_dim(top, "DistanceY", [e], "LenD", p, fmt_spec="%." + str(DIMS["decimals"]) + "w")
    placer.take(text_rect(p[0], p[1], fmt(Dm)))
e = longest(frontv, "y")
if e:
    p = (rF[0] - G, (rF[1] + rF[3]) / 2)
    add_dim(frontv, "DistanceY", [e], "LenH", p, fmt_spec="%." + str(DIMS["decimals"]) + "w")
    placer.take(text_rect(p[0], p[1], fmt(Hm)))

# 孔: 同规格合并成一条标注, 引线指向其中一个(俯视里, 轴线朝深度方向 = 俯视看得到的孔)
groups = {}
for h in holes:
    if abs(abs(V(*h["axis"]).dot(ax_h)) - 1) > 1e-3:
        continue  # 只标俯视方向看得到的孔(其它方向的留给后续: 在对应视图里标)
    k = (h["d"], h.get("through"), h.get("depth"), tuple(h["cb"]) if h.get("cb") else None, h.get("tap"), h.get("tol"))
    groups.setdefault(k, []).append(h)
circles = [(n, ed) for n, ed in edges_of(top) if isinstance(ed.Curve, Part.Circle)]
rep["top_circles"] = [[n, round(ed.Curve.Radius, 3), round(ed.Curve.Center.x, 2), round(ed.Curve.Center.y, 2)] for n, ed in circles]
cxT, cyT = (rT[0] + rT[2]) / 2, (rT[1] + rT[3]) / 2


def circle_for(h):
    """俯视里与孔对应的圆边: 半径 = d/2, 圆心最接近孔轴投影。视图边坐标 = 模型单位(未乘比例)、以零件包围盒中心为原点。
    返回 (边名, 圆心在页面上相对视图中心的偏移 = 边坐标 × 比例)。"""
    at = V(*h["at"])
    px = (at - bb.Center).dot(ax_w)
    py = (at - bb.Center).dot(ax_d)
    best, bd = None, 1e9
    for n, ed in circles:
        if abs(ed.Curve.Radius - h["d"] / 2) > 1e-3:
            continue
        c = ed.Curve.Center
        for sy in (1, -1):
            dd = math.hypot(c.x - px, c.y - sy * py)
            if dd < bd:
                best, bd = (n, V(c.x * scale, c.y * scale, 0)), dd
    return best


k_i = 0
for k, hs in sorted(groups.items(), key=lambda kv: -len(kv[1])):
    h0 = dict(hs[0])
    txt = callout(h0, len(hs))
    hit = circle_for(h0)
    if not hit:
        rep["warnings"].append("no circle for hole %s" % h0)
        continue
    n, c = hit
    # 候选: 视图外上 / 下 / 左 / 右, 由近到远
    cands = []
    for ring in range(6):
        off = G * 1.6 + ring * STEP
        cands += [(cxT + c.x + off * 0.6, rT[3] + off), (cxT + c.x - off * 0.6, rT[3] + off),
                  (cxT + c.x + off * 0.6, rT[1] - off - G), (cxT + c.x - off * 0.6, rT[1] - off - G),
                  (rT[2] + off + 10, cyT + c.y + off * 0.4), (rT[0] - off - 10, cyT + c.y + off * 0.4)]
    spot = None
    for p in cands:
        r = text_rect(p[0], p[1], txt)
        if placer.free(r):
            spot = p
            placer.take(r)
            break
    if spot is None:
        spot = cands[0]
        rep["warnings"].append("callout overlaps: " + txt)
    add_dim(top, "Diameter", [n], "Hole%d" % k_i, spot, text=txt)
    k_i += 1
doc.recompute()
lap("dims")

# ---------------------------------------------------------------- 标题栏
title = dict(spec.get("title", {}))
title.setdefault("scale", scale_text(scale))
title.setdefault("paper", paper)
title.setdefault("qty", "1")
today = time.strftime("%Y-%m-%d")
title.setdefault("design_date", today)
title.setdefault("drawing_no", title.get("pn", ""))
if "weight" not in title and title.get("density"):
    title["weight"] = fmt(shape.Volume * float(title.pop("density")) / 1000.0, 1) + " g"
title.pop("density", None)
texts = dict(tpl.EditableTexts)
for i, line in enumerate(spec.get("tech") or std["tech_requirements"]):
    title["tech_%d" % (i + 1)] = line
for k, v in title.items():
    if k in texts:
        texts[k] = str(v)
    else:
        rep["warnings"].append("title field not in template: " + k)
tpl.EditableTexts = texts
doc.recompute()
lap("title")

# ---------------------------------------------------------------- 导出
fc = os.path.join(OUT, BASE + ".FCStd")
TechDraw.writeDXFPage(page, os.path.join(OUT, BASE + ".dxf"))
if GUI:
    import TechDrawGui

    for o in doc.Objects:
        vo = getattr(o, "ViewObject", None)
        if vo is not None and hasattr(vo, "ArcCenterMarks"):
            vo.ArcCenterMarks = True
        if vo is not None and hasattr(vo, "Font"):
            try:
                vo.Font = std["fonts"]["latin"]
            except Exception:
                pass
        if sec is not None and o is sec and vo is not None and hasattr(vo, "ShowLabel"):
            vo.ShowLabel = True
        # 孔标注: 水平文字 + 引线(ISO referencing); 线性尺寸保持 ISO oriented
        if vo is not None and o.Name.startswith("Hole") and hasattr(vo, "StandardAndStyle"):
            try:
                vo.StandardAndStyle = "ISO Referencing"
            except Exception as e:
                rep["warnings"].append("hole style: %s" % e)
    page.ViewObject.doubleClicked()
    for _ in range(20):
        Gui.updateGui()
        time.sleep(0.05)
    TechDrawGui.exportPageAsPdf(page, os.path.join(OUT, BASE + ".pdf"))
    TechDrawGui.exportPageAsSvg(page, os.path.join(OUT, BASE + ".svg"))
else:
    rep["warnings"].append("no GUI: PDF / SVG skipped (run freecad.exe with QT_QPA_PLATFORM=offscreen)")
doc.saveAs(fc)
lap("total")
rep["out"] = {"dir": OUT, "base": BASE}
with open(os.path.join(OUT, BASE + ".report.json"), "w", encoding="utf-8") as f:
    json.dump(rep, f, ensure_ascii=False, indent=1)
