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
    lap("step read")
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


# 体积 / 面积每次取都要重算质量属性(B 样条实体很慢): 每个实体只算一次
SVA = [(s.Volume, s.Area) for s in solids]


def fp_err(si, l):
    vol, area = SVA[si]
    e = abs(vol - l.get("volume", 0)) / max(l.get("volume") or 1e-9, 1e-9)
    if l.get("area"):
        e += abs(area - l["area"]) / max(l["area"], 1e-9)
    return e


owner = []  # 每个实体属于哪一行(下标, 对不上 = None)
# 指纹表: 普通行 = 自己的体积 / 面积; 外购组件行(parts)= 组里每种实体的 [体积, 面积]
FPS = []
for i, l in enumerate(lines):
    if l.get("parts"):
        FPS += [({"volume": p[0], "area": p[1]}, i) for p in l["parts"] if p and p[0]]
    elif l.get("volume"):
        FPS.append((l, i))
for si_, s in enumerate(solids):
    cands = [(fp_err(si_, f), i) for f, i in FPS]
    e, i = min(cands) if cands else (1e9, None)
    owner.append(i if e < 0.05 else None)  # 导入的 B 样条件: STEP 里算的体积 / 面积与 Plasticity 差得多一点
for i, l in enumerate(lines):
    l["found"] = sum(1 for o in owner if o == i)
    if l.get("qty") and not l.get("parts") and l["found"] != int(l["qty"]):
        rep["warnings"].append("line %s (%s): BOM qty %s, solids in STEP %d" % (l.get("pn") or l.get("name"), l.get("key"), l.get("qty"), l["found"]))
unmatched = sum(1 for o in owner if o is None)
ONLY_LISTED = bool(spec.get("only_listed"))  # 某个组的装配图: STEP 里有组外的可见实体(Plasticity 写出全部可见), 对不上的不画
if unmatched:
    rep["warnings"].append("%d solid(s) in the STEP match no BOM line (%s)" % (unmatched, "not drawn: outside this group" if ONLY_LISTED else "drawn, no balloon"))
if ONLY_LISTED and unmatched:
    keep_i = [i for i, o in enumerate(owner) if o is not None]
    solids = [solids[i] for i in keep_i]
    owner = [owner[i] for i in keep_i]
    SVA = [SVA[i] for i in keep_i]
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


lap("match")
FIT_TOL_D = 0.1  # 孔 / 轴建模直径差在这以内算"同一公称尺寸"
cyls = [cyl_faces(s) if owner[i] is not None else [] for i, s in enumerate(solids)]
fit_cands = {}
BBS = []
for s in solids:
    b_ = s.BoundBox
    b_.enlarge(0.1)
    BBS.append(b_)
for i in range(len(solids)):
    for j in range(len(solids)):
        # 同一行(外购件内部 / 同种零件)不算配合; 包围盒不碰的不用比
        if i == j or owner[i] is None or owner[j] is None or owner[i] == owner[j] or not cyls[i] or not cyls[j]:
            continue
        if not BBS[i].intersect(BBS[j]):
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
lap("fits")

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
HANDLE_FACES_WAS = App.ParamGet(P + "General").GetBool("HandleFaces", True)  # 用户设置: 导出完还原
App.ParamGet(P + "General").SetBool("HandleFaces", False)  # 不找面(大装配找面要几分钟; 装配图不剖面线)

# ---------------------------------------------------------------- 方向: 主视默认从前(-Y)往后看, Z 朝上(Plasticity 建模习惯)
# 没指定: Z 朝上, 两个水平方向里长的那边横着放(从短边那侧看), 与零件图"长边水平"一致
if spec.get("front"):
    front = V(*spec["front"]).normalize()
else:
    front = V(0, -1, 0) if bb.XLength >= bb.YLength else V(-1, 0, 0)
    rep["front"] = "auto"
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
# 行多时分栏(国标: 不够高就在左边接着排, 每栏有表头); 每栏最多 PL_ROWS 行
PL_ROWS = int(PL.get("max_rows", 22))
PL_COLS = max(1, -(-len(lines) // PL_ROWS))
PL_H = PL["row_h"] * (min(len(lines), PL_ROWS) + 1)
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
    pl = (inner_r - PL_W * PL_COLS, area_b, inner_r, area_b + PL_H)  # 明细表(页面坐标, y 向上; 多栏往左)
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

# 图上看不见的小实体(按这个比例画出来 < 0.8 mm, 如电路板上的 0402 电阻)不进视图: 消隐时间主要花在它们身上。
# 只在去掉后整体包围盒不变时才去(视图边坐标以包围盒中心为原点, 变了序号 / 尺寸就对不上)。
MIN_PAPER = float(spec.get("min_paper_mm", 0.8))
# B 样条面很多的实体(IGES 转来的外购件, 如伺服驱动器 1200+ 个 B 样条面)精确消隐要十几分钟 →
# 画成外形包围盒(国标允许外购件简化画外形)。可见点判断也用简化后的(distToShape 同样慢)。
ANALYTIC = (Part.Plane, Part.Cylinder, Part.Cone, Part.Sphere, Part.Toroid)
BS_MAX = int(spec.get("simplify_bspline_faces", 100))
# 外购件门槛更低: 实测 XYZ 台里一个 112 个 B 样条面的实体从上往下看消隐卡死(OCC; 从前面看 0.5 s)
BS_MAX_BOUGHT = int(spec.get("simplify_bspline_faces_purchased", 50))


def round_section(s, k):
    """沿第 k 轴看截面是圆的: 截面四个角附近(内缩 10%)在 5 个轴向位置上都不在实体里(方盒子的角是实的)。
    实测伺服驱动器包围盒两边也差不到 5%, 只看包围盒会被画成圆盘。"""
    b_ = s.BoundBox
    lo = [b_.XMin, b_.YMin, b_.ZMin]
    ln = [b_.XLength, b_.YLength, b_.ZLength]
    i, j = [n for n in range(3) if n != k]
    for t in (0.1, 0.3, 0.5, 0.7, 0.9):
        for ci in (0.1, 0.9):
            for cj in (0.1, 0.9):
                p = [0.0, 0.0, 0.0]
                p[k] = lo[k] + ln[k] * t
                p[i] = lo[i] + ln[i] * ci
                p[j] = lo[j] + ln[j] * cj
                try:
                    if s.isInside(V(*p), 1e-3, True):
                        return False
                except Exception:
                    return False
    return True


def outline(s):
    """简化外形: 包围盒两边差 < 5% → 沿第三轴的圆柱(编码器 / 电机), 否则长方体。包围盒不变。"""
    b_ = s.BoundBox
    L = [(b_.XLength, V(1, 0, 0)), (b_.YLength, V(0, 1, 0)), (b_.ZLength, V(0, 0, 1))]
    for k in range(3):
        a_, c_ = [L[j][0] for j in range(3) if j != k]
        if max(a_, c_) > 1e-6 and abs(a_ - c_) / max(a_, c_) < 0.05 and L[k][0] > 1e-3 and round_section(s, k):
            r_ = min(a_, c_) / 2
            ax = L[k][1]
            base = V(b_.Center.x, b_.Center.y, b_.Center.z) - ax * (L[k][0] / 2)
            # 不往复合体里塞孤立顶点保包围盒: 实测会让某个方向的消隐整个返回空(俯视 0 条边);
            # 视图原点本来就取所画形体的包围盒, 圆柱略小一点无妨
            return Part.makeCylinder(r_, L[k][0], base, ax)
    return Part.makeBox(max(b_.XLength, 1e-3), max(b_.YLength, 1e-3), max(b_.ZLength, 1e-3), V(b_.XMin, b_.YMin, b_.ZMin))


DRAW = []  # 画图 / 遮挡判断用的实体(下标与 solids 一致; None = 太小不画)
simplified = 0
for si_, s in enumerate(solids):
    if s.BoundBox.DiagonalLength * scale < MIN_PAPER:
        DRAW.append(None)
        continue
    nbs = sum(1 for f in s.Faces if not isinstance(f.Surface, ANALYTIC))
    bought = owner[si_] is not None and bool(lines[owner[si_]].get("parts"))
    if nbs > (BS_MAX_BOUGHT if bought else BS_MAX):
        DRAW.append(outline(s))
        simplified += 1
    else:
        DRAW.append(s)
keep = [d for d in DRAW if d is not None]
if keep and (simplified or len(keep) < len(solids)):
    part.Shape = Part.makeCompound(keep)
    # 视图边坐标的原点 = 所画形体的包围盒中心: 以画的为准(去掉 / 简化通常不改包围盒, 改了也只差一点)
    bb = part.Shape.BoundBox
rep["dropped_small"] = len(solids) - len(keep)
rep["simplified"] = simplified
if simplified:
    rep["warnings"].append("%d complex solid(s) drawn as a simplified outline (box / cylinder; > %d B-spline faces, purchased > %d)" % (simplified, BS_MAX, BS_MAX_BOUGHT))
W, H, AREA, PLR = layout(paper)
rep["paper"], rep["scale"] = paper, scale

TEMPLATE_SVG = make_template.build(std, paper, sheet="assembly")


# ---------------------------------------------------------------- 视图: 自己投影(TechDraw.projectEx, 同步), 自己写 SVG
# 不用 TechDraw 页面: 界面模式下视图在后台线程算、要靠刷界面推进, 110 个实体的装配 4 个视图等了 13 min 还没算完;
# 同样的投影直接调 projectEx 每个视图约 10 s。做法: 把装配平移到包围盒中心、转到视图坐标系(X = 页面右, Y = 页面上,
# Z = 朝看的人), 沿 +Z 投影 → 投影坐标就是视图坐标(实测: 沿 +Z 投影 x = 世界 X、y = 世界 Y, 不居中)。
class View:
    def __init__(self, name, X, Y, Z, s):
        self.Name, self.X, self.Y, self.Z, self.s = name, X.normalize(), Y.normalize(), Z.normalize(), s
        self.c = (0.0, 0.0)  # 页面上视图中心(= 包围盒中心的投影)

    def local(self, p):
        d = p - bb.Center
        return d.dot(self.X), d.dot(self.Y)

    def page(self, p):
        x, y = self.local(p)
        return self.c[0] + x * self.s, self.c[1] + y * self.s

    def size(self):
        corners = [V(x, y, z) for x in (bb.XMin, bb.XMax) for y in (bb.YMin, bb.YMax) for z in (bb.ZMin, bb.ZMax)]
        xs = [(c - bb.Center).dot(self.X) for c in corners]
        ys = [(c - bb.Center).dot(self.Y) for c in corners]
        return (max(xs) - min(xs)) * self.s, (max(ys) - min(ys)) * self.s

    def rect(self):
        w, h = self.size()
        return (self.c[0] - w / 2, self.c[1] - h / 2, self.c[0] + w / 2, self.c[1] + h / 2)


# 第一角: 俯视在主视下面(页面上 = −主视方向, 朝看的人 = 上), 左视在主视右边(页面右 = 主视方向, 朝看的人 = −右)
views = {
    "Front": View("Front", right, up, front, scale),
    "Top": View("Top", right, -front, up, scale),
    "Left": View("Left", front, up, -right, scale),
}
views["Front"].c = (fx, fy)
views["Top"].c = (fx, fy - (Hm * scale / 2 + GAPV + Dm * scale / 2))
views["Left"].c = (fx + Wm * scale / 2 + GAPV + Dm * scale / 2, fy)
R = {n: v.rect() for n, v in views.items()}
iso_dir = (front + right + up).normalize()
iso_x = (right - front).normalize()
iso_y = iso_dir.cross(iso_x)
diag = bb.DiagonalLength
free_l = max(r[2] for r in R.values()) + DIMB + 8
free_w = AREA[2] - free_l
free_h = AREA[3] - (PLR[3] + 8)
iso_s = next((s for s in SCALES if diag * s <= max(min(free_w, free_h * 1.2), 50) and s <= scale), SCALES[-1])
views["Iso"] = View("Iso", iso_x, iso_y, iso_dir, iso_s)
views["Iso"].c = ((free_l + AREA[2]) / 2 if free_w > 50 else AREA[2] - diag * iso_s / 2,
                  (AREA[3] + PLR[3]) / 2 if free_w > 50 else AREA[3] - diag * iso_s / 2)
R["Iso"] = views["Iso"].rect()
rep["views"] = {k: [round(x, 1) for x in r] for k, r in R.items()}

DRAW_SHAPE = Part.makeCompound([d for d in DRAW if d is not None])
VIEW_SVG = []
LWV = std["lines"]["visible"]
DEFL = float(spec.get("deflection_mm", 0.05))  # 曲线离散精度(页面毫米)


def project_view(v):
    """投影一个视图 → 可见轮廓的折线(页面坐标, SVG y 向下)。"""
    sh = DRAW_SHAPE.copy()
    sh.translate(-bb.Center)
    m = App.Matrix(v.X.x, v.X.y, v.X.z, 0, v.Y.x, v.Y.y, v.Y.z, 0, v.Z.x, v.Z.y, v.Z.z, 0, 0, 0, 0, 1)
    sh.transformShape(m)  # 刚体旋转(就地); 视图轴都是正交右手系
    if abs(m.determinant() - 1) > 1e-6:
        rep["warnings"].append("view %s: axes not a rotation (det %.3f)" % (v.Name, m.determinant()))
    t0 = time.time()
    r = TechDraw.projectEx(sh, V(0, 0, 1))
    edges = list(r[0].Edges) + list(r[3].Edges)  # 可见的棱边 + 外形轮廓线(圆柱等的侧影)
    out = []
    for e in edges:
        try:
            pts = e.discretize(Deflection=DEFL / max(v.s, 1e-6))
        except Exception:
            continue
        if len(pts) < 2:
            continue
        out.append(" ".join("%.3f,%.3f" % (v.c[0] + p.x * v.s, H - (v.c[1] + p.y * v.s)) for p in pts))
    rep.setdefault("hlr", {})[v.Name] = {"s": round(time.time() - t0, 1), "edges": len(edges)}
    if not edges:
        rep["warnings"].append("view %s: hidden-line removal returned no edges" % v.Name)
    return out


for vn in ("Front", "Top", "Left", "Iso"):
    lines_ = project_view(views[vn])
    VIEW_SVG.append('<g style="fill:none;stroke:#000;stroke-width:%g;stroke-linecap:round;stroke-linejoin:round">' % LWV
                    + "".join('<polyline points="%s"/>' % pl for pl in lines_) + "</g>")
lap("views")


def frame(v):
    return v.X, v.Y, v.Z


def to_page(v, p):
    return v.page(p)

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
    s = DRAW[si]
    if s is None:
        return None  # 太小, 图上没画
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
        if tried >= 10:
            break
        tried += 1
        ray = Part.LineSegment(p + D * 0.05, p + D * big).toShape()
        px_, py_, pz_ = p.dot(v.X), p.dot(v.Y), p.dot(D)
        blocked = False
        for j, o in enumerate(DRAW):
            if o is None:
                continue
            # 粗筛: 包围盒在视图平面上的投影要盖住这个点, 且有部分在点的前面(朝看的人一侧)
            x0_, x1_, y0_, y1_, z1_ = BOX2D[v.Name][j]
            if not (x0_ - 1e-3 <= px_ <= x1_ + 1e-3 and y0_ - 1e-3 <= py_ <= y1_ + 1e-3 and z1_ > pz_ + 0.04):
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


def box2d(o, v):
    b_ = o.BoundBox
    cs = [V(x, y, z) for x in (b_.XMin, b_.XMax) for y in (b_.YMin, b_.YMax) for z in (b_.ZMin, b_.ZMax)]
    xs, ys, zs = [c.dot(v.X) for c in cs], [c.dot(v.Y) for c in cs], [c.dot(v.Z) for c in cs]
    return (min(xs), max(xs), min(ys), max(ys), max(zs))


# 每个视图: 各实体包围盒在视图平面上的投影 + 最靠前的深度(遮挡粗筛用)
BOX2D = {vn: [box2d(o, views[vn]) if o is not None else None for o in DRAW] for vn in ("Front", "Left", "Top")}

balloons = []  # {line, view, anchor(page), solid}
for li, l in enumerate(lines):
    idx = sorted((i for i, o in enumerate(owner) if o == li), key=lambda i: -SVA[i][0])[:3]  # 大的先试(外购件里的小零件看不清)
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
    # 离哪条边近就走哪边(页面距离): 左边列 / 上面排; 右半边的点一律走上排(免得引线横穿整个视图)
    d_left, d_top = ax - rF[0], rF[3] - ay
    (top if d_top < d_left or ax > (rF[0] + rF[2]) / 2 else left).append(b)
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
yb = PLR[1]
lw_k = std["lines"]["frame"]
cols = PL["columns"]
fs = std["fonts"]["text"]


def col_xs(ci):
    """第 ci 栏(0 = 最右, 往左接续)的列分界 x。"""
    x0_ = PLR[2] - (ci + 1) * PL_W
    xs = [x0_]
    for c in cols:
        xs.append(xs[-1] + c["w"])
    return xs


for ci in range(PL_COLS):
    xs_ = col_xs(ci)
    n_here = min(PL_ROWS, len(lines) - ci * PL_ROWS)
    nrow = n_here + 1
    top_y = yb + rh * nrow
    # 外框粗线, 内部细线; 每栏都有表头
    PAGE_SVG.append('<rect x="%.3f" y="%.3f" width="%.3f" height="%.3f" style="fill:none;stroke:#000;stroke-width:%g"/>'
                    % (xs_[0], sy(top_y), xs_[-1] - xs_[0], rh * nrow, lw_k))
    for k in range(1, nrow):
        line(xs_[0], yb + k * rh, xs_[-1], yb + k * rh, lw_k if k == 1 else LW)
    for x in xs_[1:-1]:
        line(x, yb, x, top_y, LW)
    for c, a_, b_ in zip(cols, xs_, xs_[1:]):
        text((a_ + b_) / 2, yb + rh * 0.3, c["label"], fs)


def fit(s, w, size):
    """单元格放不下就缩字(最小 2.2)。"""
    est = sum(size * (1.0 if ord(ch) > 0x2E80 else 0.6) for ch in s)
    return size if est <= w - 2 else max(2.2, size * (w - 2) / est)


for l in sorted(lines, key=lambda l: l["no"]):
    ci, ri = divmod(l["no"] - 1, PL_ROWS)
    xs_ = col_xs(ci)
    y = yb + rh * (ri + 1)
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

lap("annotations")

# ---------------------------------------------------------------- 标题栏: 直接填模板里的可编辑字段(freecad:editable="字段")
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
tech = spec.get("tech") or std.get("tech_requirements_assembly") or std["tech_requirements"]
for i in range(12):
    title["tech_%d" % (i + 1)] = tech[i] if i < len(tech) else ""
import re  # noqa: E402

svg = TEMPLATE_SVG
seen = set()


def fill(m):
    k = m.group(1)
    seen.add(k)
    return m.group(0)[: m.start(2) - m.start(0)] + esc(title.get(k, "")) + m.group(0)[m.end(2) - m.start(0):]


svg = re.sub(r'freecad:editable="([^"]+)"[^>]*><tspan>(.*?)</tspan>', lambda m: fill(m) if m.group(1) in title else m.group(0), svg)
for k in title:
    if k not in seen and not k.startswith("tech_"):
        rep["warnings"].append("title field not in template: " + k)
lap("title")

# ---------------------------------------------------------------- 导出: 一页 SVG(模板 + 视图 + 序号 / 尺寸 / 明细表); PDF / PNG 由 draw.ps1 用浏览器出
body = "".join(VIEW_SVG) + "".join(PAGE_SVG)
svg = svg.replace("</svg>", body + "</svg>")
with open(os.path.join(OUT, BASE + ".svg"), "w", encoding="utf-8") as f:
    f.write(svg)
rep["paper_mm"] = [W, H]
App.ParamGet(P + "General").SetBool("HandleFaces", HANDLE_FACES_WAS)
lap("total")
rep["out"] = {"dir": OUT, "base": BASE}
with open(os.path.join(OUT, BASE + ".report.json"), "w", encoding="utf-8") as f:
    json.dump(rep, f, ensure_ascii=False, indent=1)
