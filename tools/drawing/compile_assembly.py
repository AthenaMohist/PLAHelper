# 装配图编译器(FreeCAD TechDraw 后端, D3): 出图方案 JSON(带 assembly)+ 制图规范 JSON → 装配图 → PDF / SVG / FCStd + 报告。
# 由 run_offscreen.py 在离屏 freecad.exe 里跑(方案有 "assembly" 键时), 与零件图共用 draw.ps1 / 模板 / 规范。
#
# 方案 JSON:
# { "standard": "GB", "paper": "auto" | "A3", "scale": "auto" | 0.5,
#   "assembly": { "step": "<整个装配的 STEP>",
#                 "lines": [ { "key": "<BOM 零件 key>", "pn": "...", "name": "...", "qty": 6, "material": "...", "note": "...",
#                              "volume": 123.4, "area": 567.8, "density": 2.7 }, … ] },     BOM 每种零件一行(体积 / 面积 = 指纹)
#   "front": [0,-1,0], "up": [0,0,1],          主视方向 / 主视朝上(省 = 从前往后看、Z 朝上)
#   "balloons": "auto",                        序号: 自动在主视(看不见的用左视 / 俯视)上找可见点
#   "title": {...}, "tech": [...], "out": "<目录>", "basename": "<文件名>" }
#
# 做法: STEP 里每个实体按体积 + 面积对上 BOM 行(相同件的多个实例归同一行); 三视图(第一角, 不画虚线)+ 轴测;
# 每行挑一个实例, 在视图里找一个看得见的点(面上取点, 朝看的人射线不被别的实体挡)→ 引线 + 圆圈序号;
# 序号绕视图顺时针排(左列由下往上 → 上排由左往右 → 右列由上往下), 号码按排列顺序编 → 明细表(标题栏正上方, 由下往上);
# 外形尺寸(长 / 宽 / 高)自己画进页面级 SVG; 标题栏重量 = Σ 数量 × 体积 × 密度(每行都有密度时)。
import json
import math
import os
import sys
import time

import FreeCAD as App
import Part
import TechDraw

HERE = os.path.dirname(os.path.abspath(__file__))
V = App.Vector
T0 = time.time()
rep = {"kind": "assembly", "steps": {}, "warnings": []}


def load_json(p):
    with open(p, encoding="utf-8-sig") as f:
        return json.load(f)


SPEC_PATH = os.environ.get("PLA_DRAW_SPEC", "")
spec = load_json(SPEC_PATH) if SPEC_PATH else {"assembly": {}}
std_ref = spec.get("standard", "GB")
std = load_json(std_ref if std_ref.endswith(".json") else os.path.join(HERE, "standards", std_ref + ".json"))
OUT = spec.get("out") or os.path.join(HERE, "out")
BASE = spec.get("basename") or (spec.get("title", {}).get("pn") or "assembly")
os.makedirs(OUT, exist_ok=True)
DIMS = std["dims"]
ASM = std.get("assembly", {})
GUI = False
try:
    import FreeCADGui as Gui

    GUI = bool(Gui.getMainWindow())
except Exception:
    GUI = False


def lap(name):
    rep["steps"][name] = round(time.time() - T0, 3)
    try:
        with open(os.path.join(OUT, "progress.log"), "a", encoding="utf-8") as f:
            f.write("%7.2f %s\n" % (time.time() - T0, name))
    except Exception:
        pass


def q(x):
    return float(getattr(x, "Value", x))


def fmt(v, dec=None):
    dec = DIMS["decimals"] if dec is None else dec
    s = ("%." + str(dec) + "f") % v
    if DIMS.get("strip_zeros") and "." in s:
        s = s.rstrip("0").rstrip(".")
    return s


def esc(s):
    return str(s).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


# ---------------------------------------------------------------- 实体 → BOM 行
def test_assembly():
    """内置样例(没给 STEP 时): 底板 + 4 个螺柱 + 顶块 + 侧板。"""
    plate = Part.makeBox(120, 80, 10)
    pins = []
    for x, y in ((15, 15), (105, 15), (15, 65), (105, 65)):
        plate = plate.cut(Part.makeCylinder(4, 10, V(x, y, 0)))
        pins.append(Part.makeCylinder(4, 50, V(x, y, 0)))  # 穿过底板: 配合候选 ⌀8
    sols = [plate] + pins
    sols.append(Part.makeBox(60, 40, 20, V(30, 20, 10)))
    sols.append(Part.makeBox(120, 6, 50, V(0, 80, 0)))
    return sols


aspec = spec.get("assembly") or {}
if aspec.get("step"):
    sh = Part.Shape()
    sh.read(aspec["step"])
    solids = list(sh.Solids)
else:
    solids = test_assembly()
lines = [dict(l) for l in (aspec.get("lines") or [])]
if not lines:
    # 没给 BOM: 按体积 + 面积自己归类(样例 / 命令行试用)
    for s in solids:
        hit = next((l for l in lines if abs(l["volume"] - s.Volume) <= 1e-3 * max(s.Volume, 1) and abs(l["area"] - s.Area) <= 1e-3 * max(s.Area, 1)), None)
        if hit:
            hit["qty"] += 1
        else:
            lines.append({"key": "L%d" % (len(lines) + 1), "name": "Part %d" % (len(lines) + 1), "qty": 1, "volume": s.Volume, "area": s.Area})


def fp_err(s, l):
    e = abs(s.Volume - l.get("volume", 0)) / max(l.get("volume") or 1e-9, 1e-9)
    if l.get("area"):
        e += abs(s.Area - l["area"]) / max(l["area"], 1e-9)
    return e


owner = []  # 每个实体属于哪一行(下标, 对不上 = None)
for s in solids:
    cands = [(fp_err(s, l), i) for i, l in enumerate(lines) if l.get("volume")]
    e, i = min(cands) if cands else (1e9, None)
    owner.append(i if e < 0.02 else None)
for i, l in enumerate(lines):
    l["found"] = sum(1 for o in owner if o == i)
    if l.get("qty") and l["found"] != int(l["qty"]):
        rep["warnings"].append("line %s (%s): BOM qty %s, solids in STEP %d" % (l.get("pn") or l.get("name"), l.get("key"), l.get("qty"), l["found"]))
unmatched = sum(1 for o in owner if o is None)
if unmatched:
    rep["warnings"].append("%d solid(s) in the STEP match no BOM line (drawn, no balloon)" % unmatched)
rep["solids"] = len(solids)
rep["lines"] = [{"key": l.get("key"), "found": l["found"], "qty": l.get("qty")} for l in lines]


# ---------------------------------------------------------------- D2 配合候选: 不同零件之间同轴、直径相同的孔(内圆柱)/ 轴(外圆柱)
def cyl_faces(s):
    out = []
    for f in s.Faces:
        su = f.Surface
        if not isinstance(su, Part.Cylinder):
            continue
        ax = su.Axis.normalize()
        u0, u1, v0, v1 = f.ParameterRange
        p, n = f.valueAt((u0 + u1) / 2, (v0 + v1) / 2), f.normalAt((u0 + u1) / 2, (v0 + v1) / 2)
        c = su.Center
        rad = (p - c) - ax * ((p - c).dot(ax))
        if rad.Length < 1e-6:
            continue
        internal = n.dot(rad) < 0
        ts = [(vx.Point - c).dot(ax) for vx in f.Vertexes] or [0.0]
        out.append({"ax": ax, "c": c, "r": su.Radius, "in": internal, "t0": min(ts), "t1": max(ts)})
    return out


FIT_TOL_D = 0.1  # 孔 / 轴建模直径差在这以内算"同一公称尺寸"
cyls = [cyl_faces(s) if owner[i] is not None else [] for i, s in enumerate(solids)]
fit_cands = {}
for i in range(len(solids)):
    for j in range(len(solids)):
        if i == j or owner[i] is None or owner[j] is None:
            continue
        for h in cyls[i]:
            if not h["in"]:
                continue
            for s_ in cyls[j]:
                if s_["in"] or abs(2 * h["r"] - 2 * s_["r"]) > FIT_TOL_D:
                    continue
                if abs(abs(h["ax"].dot(s_["ax"])) - 1) > 1e-6:
                    continue
                dv = s_["c"] - h["c"]
                if (dv - h["ax"] * dv.dot(h["ax"])).Length > 0.01:
                    continue  # 不同轴
                # 轴向重叠
                a0, a1 = h["t0"], h["t1"]
                b0, b1 = sorted([(s_["c"] + s_["ax"] * s_["t0"] - h["c"]).dot(h["ax"]), (s_["c"] + s_["ax"] * s_["t1"] - h["c"]).dot(h["ax"])])
                lo, hi_ = max(a0, b0), min(a1, b1)
                if hi_ - lo < 0.05:
                    continue
                d = round(2 * h["r"], 2)
                hk, sk = lines[owner[i]].get("key"), lines[owner[j]].get("key")
                cid = "%s|%s|%s" % (hk, sk, fmt(d))
                at = h["c"] + h["ax"] * ((lo + hi_) / 2)
                c_ = fit_cands.setdefault(cid, {"id": cid, "d": d, "hole_key": hk, "shaft_key": sk,
                                                "hole": lines[owner[i]].get("pn") or lines[owner[i]].get("name"),
                                                "shaft": lines[owner[j]].get("pn") or lines[owner[j]].get("name"),
                                                "count": 0, "at": [at.x, at.y, at.z], "axis": [h["ax"].x, h["ax"].y, h["ax"].z]})
                c_["count"] += 1
rep["fit_candidates"] = sorted(fit_cands.values(), key=lambda c: (-c["d"], c["id"]))

doc = App.newDocument("asm")
comp = Part.makeCompound(solids)
part = doc.addObject("Part::Feature", "Assembly")
part.Shape = comp
bb = comp.BoundBox
rep["bbox"] = [round(bb.XLength, 3), round(bb.YLength, 3), round(bb.ZLength, 3)]
lap("load")

# ---------------------------------------------------------------- 字体 / 样式
P = "User parameter:BaseApp/Preferences/Mod/TechDraw/"
App.ParamGet(P + "Labels").SetString("LabelFont", std["fonts"]["latin"])
App.ParamGet(P + "Labels").SetFloat("LabelSize", std["fonts"]["text"])
App.ParamGet(P + "Dimensions").SetFloat("FontSize", std["fonts"]["dim"])

# ---------------------------------------------------------------- 方向: 主视默认从前(-Y)往后看, Z 朝上(Plasticity 建模习惯)
front = V(*(spec.get("front") or [0, -1, 0])).normalize()
up = V(*(spec.get("up") or ([0, 0, 1] if abs(front.z) < 0.9 else [0, 1, 0]))).normalize()
if abs(up.dot(front)) > 1e-6:
    rep["warnings"].append("up is not perpendicular to front: using default")
    up = V(0, 0, 1) if abs(front.z) < 0.9 else V(0, 1, 0)
right = up.cross(front)  # 主视页面"右" = 上 × 视线(与零件图同)


def ext(a):
    return abs(a.x) * bb.XLength + abs(a.y) * bb.YLength + abs(a.z) * bb.ZLength


Wm, Hm, Dm = ext(right), ext(up), ext(front)
rep["axes"] = {"front": list(front), "up": list(up), "right": list(right)}

# ---------------------------------------------------------------- 纸张 + 比例(放得下视图 + 序号 + 外形尺寸, 且不压明细表)
sys.path.insert(0, HERE)
import make_template  # noqa: E402

PL = ASM.get("parts_list", {"row_h": 7, "columns": [{"id": "no", "label": "序号", "w": 10}, {"id": "name", "label": "名称", "w": 60}, {"id": "qty", "label": "数量", "w": 12}]})
BAL = ASM.get("balloon", {"r": 4.0, "font": 5.0, "gap": 1.5, "dot": 0.6, "offset": 14.0})
PL_W = sum(c["w"] for c in PL["columns"])
PL_H = PL["row_h"] * (len(lines) + 1)
SCALES = [5, 4, 2.5, 2, 1.5, 1, 1 / 1.5, 1 / 2, 1 / 2.5, 1 / 3, 1 / 4, 1 / 5, 1 / 10, 1 / 20, 1 / 50]
GAPV = 26.0
BAND = BAL["offset"] + 2 * BAL["r"] + 4  # 序号占的一圈
DIMB = 16.0  # 外形尺寸占的一圈


def layout(paper):
    W, H = std["paper"][paper]
    fr = std["frame"]
    inner_l, inner_r = fr["outer"] + fr["zone_band"], W - fr["outer"]
    inner_t = H - fr["outer"] - fr["zone_band"]
    tb_top = std["title_block"]["rows_top"]  # 标题栏上沿(按 A3 量, 跟着下边走 → 页面 y 向上 = 297 − rows_top)
    area_b = 297 - tb_top
    area = (inner_l + 4, area_b + 4, inner_r - 4, inner_t - 4)
    pl = (inner_r - PL_W, area_b, inner_r, area_b + PL_H)  # 明细表(页面坐标, y 向上)
    return W, H, area, pl


def views_box(s, area):
    """左上起: [序号列][主视][间隔][左视][外形尺寸], 主视下面是俯视; 返回 (主视中心, 整体矩形)。"""
    l = area[0] + BAND
    t = area[3] - BAND
    fx, fy = l + Wm * s / 2, t - Hm * s / 2
    r = l + Wm * s + GAPV + Dm * s + DIMB
    b = t - Hm * s - GAPV - Dm * s - DIMB
    return (fx, fy), (area[0], b, r, area[3])


def hit(a, b):
    return not (a[2] <= b[0] or b[2] <= a[0] or a[3] <= b[1] or b[3] <= a[1])


choice = None
papers = [spec["paper"]] if spec.get("paper") and spec.get("paper") != "auto" else ["A3", "A2", "A1"]
for paper in papers:
    W, H, area, pl = layout(paper)
    scales = [spec["scale"]] if isinstance(spec.get("scale"), (int, float)) else SCALES
    for s in scales:
        c, box = views_box(s, area)
        if box[1] >= area[1] and box[2] <= area[2] and not hit(box, pl):
            choice = (paper, s, c, box)
            break
    if choice:
        break
if not choice:
    paper = papers[-1]
    W, H, area, pl = layout(paper)
    s = SCALES[-1]
    c, box = views_box(s, area)
    choice = (paper, s, c, box)
    rep["warnings"].append("assembly does not fit: smallest scale used")
paper, scale, (fx, fy), vbox = choice
W, H, AREA, PLR = layout(paper)
rep["paper"], rep["scale"] = paper, scale

tpl_path = os.path.join(OUT, "_tpl_%s_%s_asm.svg" % (std["name"], paper))
with open(tpl_path, "w", encoding="utf-8") as f:
    f.write(make_template.build(std, paper, kind="assembly"))
page = doc.addObject("TechDraw::DrawPage", "Page")
tpl = doc.addObject("TechDraw::DrawSVGTemplate", "Template")
tpl.Template = tpl_path
page.Template = tpl

# ---------------------------------------------------------------- 三视图(第一角, 不画虚线)+ 轴测
grp = doc.addObject("TechDraw::DrawProjGroup", "Proj")
page.addView(grp)
grp.Source = [part]
grp.ProjectionType = std.get("projection", "First angle")
grp.ScaleType = "Custom"
grp.Scale = scale
for name in ("Front", "Top", "Left"):
    it = grp.addProjection(name)
    try:
        it.HardHidden = False
    except Exception:
        pass
    if name == "Front":
        grp.Anchor.Direction = front
        grp.Anchor.XDirection = right
        doc.recompute()
try:
    grp.AutoDistribute = True
    grp.spacingX = GAPV
    grp.spacingY = GAPV
except Exception as e:
    rep["warnings"].append("spacing: %s" % e)
grp.X, grp.Y = fx, fy
doc.recompute()
views = {v.Type: v for v in grp.Views}


def wait_views(vs, timeout=30.0):
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


def vcenter(v):
    if v in grp.Views:
        return q(grp.X) + q(v.X), q(grp.Y) + q(v.Y)
    return q(v.X), q(v.Y)


def frame(v):
    """视图边坐标(实测): e = ((p − C)·X, (p − C)·U), X = XDirection, U = Direction × XDirection, C = 包围盒中心。"""
    X = V(v.XDirection).normalize()
    D = V(v.Direction).normalize()
    return X, D.cross(X), D


def to_page(v, p):
    X, U, _ = frame(v)
    vx, vy = vcenter(v)
    return vx + (p - bb.Center).dot(X) * scale, vy + (p - bb.Center).dot(U) * scale


def vrect(v):
    X, U, _ = frame(v)
    vx, vy = vcenter(v)
    w, h = ext(X) * scale, ext(U) * scale
    return (vx - w / 2, vy - h / 2, vx + w / 2, vy + h / 2)


R = {n: vrect(views[n]) for n in ("Front", "Top", "Left")}

iso = doc.addObject("TechDraw::DrawViewPart", "Iso")
page.addView(iso)
iso.Source = [part]
iso.Direction = (front + right + up).normalize()
iso.XDirection = (right - front).normalize()
iso.HardHidden = False
diag = bb.DiagonalLength
free_l = max(r[2] for r in R.values()) + DIMB + 8
free_w = AREA[2] - free_l
free_h = AREA[3] - (PLR[3] + 8)
iso_s = next((s for s in SCALES if diag * s <= max(min(free_w, free_h * 1.2), 50) and s <= scale), SCALES[-1])
iso.Scale = iso_s
iso.ScaleType = "Custom"
iso.X = (free_l + AREA[2]) / 2 if free_w > 50 else AREA[2] - diag * iso_s / 2
iso.Y = (AREA[3] + PLR[3]) / 2 if free_w > 50 else AREA[3] - diag * iso_s / 2
doc.recompute()
wait_views([iso])
R["Iso"] = (q(iso.X) - diag * iso_s / 2, q(iso.Y) - diag * iso_s / 2.4, q(iso.X) + diag * iso_s / 2, q(iso.Y) + diag * iso_s / 2.4)
rep["views"] = {k: [round(x, 1) for x in r] for k, r in R.items()}
lap("views")

# ---------------------------------------------------------------- 页面级图形(页面毫米, SVG y 向下)
PAGE_SVG = []
LW = std["lines"]["thin"]
FAM = std["fonts"]["latin"]


def sy(y):
    return H - y


def line(x1, y1, x2, y2, w=None):
    PAGE_SVG.append('<path d="M %.3f %.3f L %.3f %.3f" style="fill:none;stroke:#000;stroke-width:%g"/>' % (x1, sy(y1), x2, sy(y2), w or LW))


def text(x, y, s, size, anchor="middle", rot=0):
    tr = ' transform="rotate(%g %.3f %.3f)"' % (rot, x, sy(y)) if rot else ""
    PAGE_SVG.append('<text x="%.3f" y="%.3f" style="font-family:%s;font-size:%gpx;text-anchor:%s;fill:#000"%s>%s</text>'
                    % (x, sy(y), FAM, size, anchor, tr, esc(s)))


def arrow(x, y, dx, dy):
    """实心箭头, 尖在 (x, y), 指向 (dx, dy) 方向。"""
    L, Wd = 3.0, 0.9
    n = math.hypot(dx, dy) or 1
    ux, uy = dx / n, dy / n
    bx, by = x - ux * L, y - uy * L
    PAGE_SVG.append('<path d="M %.3f %.3f L %.3f %.3f L %.3f %.3f Z" style="fill:#000;stroke:none"/>'
                    % (x, sy(y), bx - uy * Wd, sy(by + ux * Wd), bx + uy * Wd, sy(by - ux * Wd)))


def dim_h(x1, x2, y_ref, y_dim, value):
    """水平外形尺寸: 两条竖向尺寸界线 + 尺寸线 + 箭头 + 数字(在线上方)。"""
    s = 1 if y_dim > y_ref else -1
    for x in (x1, x2):
        line(x, y_ref + s * 1.5, x, y_dim + s * 2)
    line(x1, y_dim, x2, y_dim)
    arrow(x1, y_dim, -1, 0)
    arrow(x2, y_dim, 1, 0)
    text((x1 + x2) / 2, y_dim + 1.2, fmt(value), std["fonts"]["dim"])


def dim_v(y1, y2, x_ref, x_dim, value):
    s = 1 if x_dim > x_ref else -1
    for y in (y1, y2):
        line(x_ref + s * 1.5, y, x_dim + s * 2, y)
    line(x_dim, y1, x_dim, y2)
    arrow(x_dim, y1, 0, -1)
    arrow(x_dim, y2, 0, 1)
    text(x_dim - 1.2, (y1 + y2) / 2, fmt(value), std["fonts"]["dim"], rot=-90)


# 外形尺寸: 长(俯视下方)、宽 = 深(左视下方)、高(左视右侧)
rT, rL = R["Top"], R["Left"]
dim_h(rT[0], rT[2], rT[1], rT[1] - 10, Wm)
dim_h(rL[0], rL[2], rL[1], rL[1] - 10, Dm)
dim_v(rL[1], rL[3], rL[2], rL[2] + 10, Hm)


# ---------------------------------------------------------------- 序号: 每行一个实例上的可见点
def visible_point(si, v):
    """实体 si 在视图 v 里看得见的点: 朝向看的人的面上取点(大面优先), 朝看的人射线不碰别的实体(也不再穿回自己)。"""
    _, _, D = frame(v)
    s = solids[si]
    faces = sorted(s.Faces, key=lambda f: -f.Area)
    big = bb.DiagonalLength * 2 + 10
    def samples():
        # 每个面取 3 × 3 个参数点(中间优先), 留朝着看的人的(圆柱面只有一侧朝前)
        for f in faces:
            try:
                u0, u1, v0, v1 = f.ParameterRange
            except Exception:
                continue
            pts = []
            for a in (0.5, 0.25, 0.75):
                for b_ in (0.5, 0.25, 0.75):
                    try:
                        uv = (u0 + (u1 - u0) * a, v0 + (v1 - v0) * b_)
                        p = f.valueAt(*uv)
                        n = f.normalAt(*uv)  # 已按面的朝向(Reversed)翻过
                    except Exception:
                        continue
                    if n.dot(D) >= 0.3 and f.isInside(p, 1e-6, True):
                        pts.append((-n.dot(D), p))
            for _, p in sorted(pts, key=lambda t: t[0])[:2]:
                yield p

    tried = 0
    for p in samples():
        if tried >= 16:
            break
        tried += 1
        ray = Part.LineSegment(p + D * 0.05, p + D * big).toShape()
        rb = ray.BoundBox
        blocked = False
        for j, o in enumerate(solids):
            if not o.BoundBox.intersect(rb):
                continue
            try:
                if o.distToShape(ray)[0] < 1e-4:
                    blocked = True
                    break
            except Exception:
                pass
        if not blocked:
            return p
    return None


balloons = []  # {line, view, anchor(page), solid}
for li, l in enumerate(lines):
    idx = [i for i, o in enumerate(owner) if o == li]
    got = None
    for vname in ("Front", "Left", "Top"):
        for si in idx:
            p = visible_point(si, views[vname])
            if p is not None:
                got = (vname, si, p)
                break
        if got:
            break
    if not got:
        if idx:
            rep["warnings"].append("no visible point for %s: no balloon" % (l.get("pn") or l.get("name")))
        continue
    vname, si, p = got
    balloons.append({"line": li, "view": vname, "anchor": to_page(views[vname], p), "solid": si})
lap("visibility")


def spread(pos, sep, lo=None, hi=None):
    out = list(pos)
    for _ in range(300):
        moved = False
        for i in range(1, len(out)):
            d = out[i] - out[i - 1]
            if d < sep - 1e-6:
                push = (sep - d) / 2
                out[i - 1] -= push
                out[i] += push
                moved = True
        if lo is not None and out and out[0] < lo:
            sh_ = lo - out[0]
            out = [x + sh_ for x in out]
            moved = True
        if hi is not None and out and out[-1] > hi:
            sh_ = out[-1] - hi
            out = [x - sh_ for x in out]
            moved = True
        if not moved:
            break
    return out


# 排位: 主视 / 俯视里的点 → 主视左边一列 或 主视上面一排(看点在主视的左下半还是右上半); 左视里的点 → 上面一排(左视上方)
rF = R["Front"]
col_x = rF[0] - BAL["offset"]  # 左列圆心 x
row_y = max(rF[3], R["Left"][3]) + BAL["offset"]  # 上排圆心 y
left, top = [], []
for b in balloons:
    ax, ay = b["anchor"]
    if b["view"] == "Left":
        top.append(b)
        continue
    u = (ax - rF[0]) / max(rF[2] - rF[0], 1e-6)
    w = (ay - rF[1]) / max(rF[3] - rF[1], 1e-6)
    (top if w > u or (b["view"] == "Front" and w > 0.6) else left).append(b)
    if b["view"] == "Top" and b in top:
        top.remove(b)
        left.append(b)  # 俯视在主视下面: 只能走左列
sep = 2 * BAL["r"] + BAL["gap"]
left.sort(key=lambda b: b["anchor"][1])  # 由下往上
ys = spread([b["anchor"][1] for b in left], sep, lo=AREA[1] + BAL["r"] + 1, hi=AREA[3] - BAND)
for b, y in zip(left, ys):
    b["at"] = (col_x, y)
top.sort(key=lambda b: b["anchor"][0])  # 由左往右
xs = spread([b["anchor"][0] for b in top], sep, lo=col_x + sep, hi=AREA[2] - BAL["r"] - 1)
for b, x in zip(top, xs):
    b["at"] = (x, row_y)
# 编号 = 顺时针排列顺序: 左列由下往上, 再上排由左往右
order = left + top
for n, b in enumerate(order, start=1):
    b["no"] = n
    lines[b["line"]]["no"] = n
# 没有序号的行(看不见)排在最后
nxt = len(order) + 1
for l in lines:
    if "no" not in l:
        l["no"] = nxt
        nxt += 1

r_ = BAL["r"]
for b in order:
    (ax, ay), (bx, by) = b["anchor"], b["at"]
    d = math.hypot(ax - bx, ay - by) or 1
    ex_, ey_ = bx + (ax - bx) / d * r_, by + (ay - by) / d * r_  # 引线从圆周出发
    line(ex_, ey_, ax, ay)
    PAGE_SVG.append('<circle cx="%.3f" cy="%.3f" r="%g" style="fill:#000;stroke:none"/>' % (ax, sy(ay), BAL["dot"]))
    PAGE_SVG.append('<circle cx="%.3f" cy="%.3f" r="%g" style="fill:none;stroke:#000;stroke-width:%g"/>' % (bx, sy(by), r_, LW))
    text(bx, by - BAL["font"] * 0.35, b["no"], BAL["font"])
rep["balloons"] = [{"no": b["no"], "key": lines[b["line"]].get("key"), "view": b["view"], "anchor": [round(b["anchor"][0], 1), round(b["anchor"][1], 1)]} for b in order]

# ---------------------------------------------------------------- 配合标注: "⌀10 H7/g6", 引线指到孔轴线上(孔轴 ⟂ 视线的视图里)
fits_done = []
# 已占的地方: 视图(及轴测)、序号圈、外形尺寸带
OCC = [r for r in R.values()]
OCC += [(b["at"][0] - r_ - 1, b["at"][1] - r_ - 1, b["at"][0] + r_ + 1, b["at"][1] + r_ + 1) for b in order]
OCC += [(rT[0], rT[1] - 14, rT[2], rT[1]), (rL[0], rL[1] - 14, rL[2], rL[1]), (rL[2], rL[1], rL[2] + 14, rL[3])]
for f in spec.get("fits") or []:
    cand = fit_cands.get(f.get("id", ""))
    code = (f.get("code") or "").strip()
    if not cand or not code:
        continue
    at, ax = V(*cand["at"]), V(*cand["axis"])
    vname = next((n for n in ("Front", "Left", "Top") if abs(ax.dot(frame(views[n])[2])) < 0.2), None)
    if not vname:
        rep["warnings"].append("fit %s: no view shows the side of the hole" % cand["id"])
        continue
    px, py = to_page(views[vname], at)
    txt = "%s%s %s" % (DIMS["diameter"], fmt(cand["d"]), code)
    w = sum(std["fonts"]["dim"] * (1.0 if ord(ch) > 0x2E80 else 0.62) for ch in txt)
    rr = R[vname]
    # 文字放在视图外的空地(视图下方 → 右侧 → 上方, 一圈圈往外), 避开视图 / 序号 / 已放的配合标注; 引线折到轴线上的点
    fs_ = std["fonts"]["dim"]
    spot = None
    for ring in range(10):
        o = 5 + ring * 5.5
        for cx_, cy_ in ((px - w / 2, rr[1] - o), (rr[2] + o, py + 4), (px - w / 2, rr[3] + o), (px + 4, rr[1] - o), (px - w - 4, rr[1] - o)):
            box_ = (cx_ - 1, cy_ - 1.5, cx_ + w + 1, cy_ + fs_ + 0.5)
            inside = AREA[0] <= box_[0] and box_[2] <= AREA[2] and AREA[1] <= box_[1] and box_[3] <= AREA[3]
            if inside and not any(hit(box_, u) for u in OCC) and not hit(box_, PLR):
                spot = (cx_, cy_, box_)
                break
        if spot:
            break
    if not spot:
        rep["warnings"].append("fit %s: no free place for the label" % cand["id"])
        continue
    tx, ty, box_ = spot
    OCC.append(box_)
    line(px, py, tx if tx > px else tx + w, ty - 0.8)
    line(tx, ty - 0.8, tx + w, ty - 0.8)
    PAGE_SVG.append('<circle cx="%.3f" cy="%.3f" r="%g" style="fill:#000;stroke:none"/>' % (px, sy(py), BAL["dot"]))
    text(tx, ty, txt, std["fonts"]["dim"], anchor="start")
    fits_done.append({"id": cand["id"], "code": code, "view": vname})
rep["fits"] = fits_done

# ---------------------------------------------------------------- 明细表: 标题栏正上方, 表头在下, 序号由下往上
rh = PL["row_h"]
x0 = PLR[0]
yb = PLR[1]
lw_k = std["lines"]["frame"]
cols = PL["columns"]
xs_ = [x0]
for c in cols:
    xs_.append(xs_[-1] + c["w"])
nrow = len(lines) + 1
top_y = yb + rh * nrow
# 外框粗线, 内部细线
PAGE_SVG.append('<rect x="%.3f" y="%.3f" width="%.3f" height="%.3f" style="fill:none;stroke:#000;stroke-width:%g"/>'
                % (x0, sy(top_y), xs_[-1] - x0, rh * nrow, lw_k))
for k in range(1, nrow):
    line(x0, yb + k * rh, xs_[-1], yb + k * rh, lw_k if k == 1 else LW)
for x in xs_[1:-1]:
    line(x, yb, x, top_y, LW)
fs = std["fonts"]["text"]
for c, a_, b_ in zip(cols, xs_, xs_[1:]):
    text((a_ + b_) / 2, yb + rh * 0.3, c["label"], fs)


def fit(s, w, size):
    """单元格放不下就缩字(最小 2.2)。"""
    est = sum(size * (1.0 if ord(ch) > 0x2E80 else 0.6) for ch in s)
    return size if est <= w - 2 else max(2.2, size * (w - 2) / est)


for l in sorted(lines, key=lambda l: l["no"]):
    y = yb + rh * l["no"]
    vals = {"no": l["no"], "pn": l.get("pn", ""), "name": l.get("name", ""), "qty": l.get("qty", l.get("found", "")),
            "material": l.get("material", ""), "note": l.get("note", "")}
    for c, a_, b_ in zip(cols, xs_, xs_[1:]):
        v = str(vals.get(c["id"], l.get(c["id"], "")) or "")
        if not v:
            continue
        size = fit(v, c["w"], fs)
        if c["id"] in ("no", "qty"):
            text((a_ + b_) / 2, y + rh * 0.3, v, size)
        else:
            text(a_ + 1.2, y + rh * 0.3, v, size, anchor="start")
rep["parts_list"] = [{"no": l["no"], "key": l.get("key"), "pn": l.get("pn"), "name": l.get("name"), "qty": l.get("qty")} for l in sorted(lines, key=lambda l: l["no"])]

if PAGE_SVG:
    sym = doc.addObject("TechDraw::DrawViewSymbol", "PageGraphics")
    page.addView(sym)
    sym.Symbol = ('<svg xmlns="http://www.w3.org/2000/svg" width="%gmm" height="%gmm" viewBox="0 0 %g %g">%s</svg>'
                  % (W, H, W, H, "".join(PAGE_SVG)))
    sym.Scale = 1.0
    sym.X, sym.Y = W / 2, H / 2
doc.recompute()
lap("annotations")

# ---------------------------------------------------------------- 标题栏
title = dict(spec.get("title", {}))
title.setdefault("scale", "1:1" if abs(scale - 1) < 1e-9 else ("%s:1" % fmt(scale, 2) if scale > 1 else "1:%s" % fmt(1 / scale, 2)))
title.setdefault("paper", paper)
title.setdefault("qty", "1")
title.setdefault("design_date", time.strftime("%Y-%m-%d"))
title.setdefault("drawing_no", title.get("pn", ""))
if "weight" not in title and lines and all(l.get("density") and l.get("volume") for l in lines):
    g = sum(float(l["density"]) * float(l["volume"]) / 1000.0 * float(l.get("qty") or l.get("found") or 0) for l in lines)
    title["weight"] = (fmt(g / 1000.0, 2) + " kg") if g >= 10000 else (fmt(g, 1) + " g")
title.pop("density", None)
texts = dict(tpl.EditableTexts)
for i, ln in enumerate(spec.get("tech") or std.get("tech_requirements_assembly") or std["tech_requirements"]):
    title["tech_%d" % (i + 1)] = ln
for i in range(len(spec.get("tech") or std.get("tech_requirements_assembly") or []), 12):
    title.setdefault("tech_%d" % (i + 1), "")
for k, v in title.items():
    if k in texts:
        texts[k] = str(v)
    elif not k.startswith("tech_"):
        rep["warnings"].append("title field not in template: " + k)
tpl.EditableTexts = texts
doc.recompute()
lap("title")

# ---------------------------------------------------------------- 导出(与零件图同: SVG + 备用 Qt PDF; 正式 PDF 由 draw.ps1 用浏览器打印)
TechDraw.writeDXFPage(page, os.path.join(OUT, BASE + ".dxf"))
if GUI:
    import TechDrawGui

    for o in doc.Objects:
        vo = getattr(o, "ViewObject", None)
        if vo is not None and hasattr(vo, "Font"):
            try:
                vo.Font = std["fonts"]["latin"]
            except Exception:
                pass
    page.ViewObject.doubleClicked()
    for _ in range(20):
        Gui.updateGui()
        time.sleep(0.05)
    TechDrawGui.exportPageAsSvg(page, os.path.join(OUT, BASE + ".svg"))
    TechDrawGui.exportPageAsPdf(page, os.path.join(OUT, BASE + ".qt.pdf"))
    rep["paper_mm"] = [W, H]
else:
    rep["warnings"].append("no GUI: PDF / SVG skipped (run freecad.exe with QT_QPA_PLATFORM=offscreen)")
doc.saveAs(os.path.join(OUT, BASE + ".FCStd"))
lap("total")
rep["out"] = {"dir": OUT, "base": BASE}
with open(os.path.join(OUT, BASE + ".report.json"), "w", encoding="utf-8") as f:
    json.dump(rep, f, ensure_ascii=False, indent=1)
