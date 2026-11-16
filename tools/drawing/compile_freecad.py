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
    progress(name)


def progress(msg):
    """进度写进 <out>/progress.log(挂住时看停在哪一步)。"""
    try:
        with open(os.path.join(OUT, "progress.log"), "a", encoding="utf-8") as f:
            f.write("%7.2f %s\n" % (time.time() - T0, msg))
    except Exception:
        pass


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
                      "d": round(2 * rmin, 4), "through": through, "depth": depth, "cb": cb, "len": round(span_hi - span_lo, 4)})
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
def hole_records():
    """打孔工具的孔记录(孔特征的 spec: at 入口点 / normal / d / depth / type / std 如 M4 / usage tap = 螺纹底孔 / solidName)。
    来源: spec.hole_records(列表)或 spec.dims_file(文档的 dims.json, 取 kind = hole 的组; 给了 spec.part.name 就只取这个实体的)。"""
    recs = list(spec.get("hole_records") or [])
    p = spec.get("dims_file")
    if p and os.path.exists(p):
        want = (spec.get("part") or {}).get("name")
        for g in load_json(p).get("groups", []):
            s = g.get("spec") or {}
            if g.get("kind") == "hole" and s.get("at") and (not want or s.get("solidName") in (None, want)):
                recs.append(s)
    return recs


HOLE_RECS = hole_records()


def load_step(path, volume=None, area=None):
    """读 STEP。只有一个实体就用它。多个实体时(零件在组里, Plasticity 会把全部可见实体都写进去; FreeCAD 读进来不保留
    实体名)按体积(+ 面积)挑最接近的 —— 相同件挑哪个都一样; 差 > 0.5% 告警。没给体积就合成一个画全部(并告警)。
    有孔记录时, 在体积相符的几个里优先挑孔记录的入口点落在它表面上的那一个(孔记录属于具体那个实例)。"""
    s = Part.Shape()
    s.read(path)
    sols = s.Solids
    if len(sols) == 1:
        return sols[0]
    if len(sols) > 1 and volume:
        def err(so):
            e = abs(so.Volume - volume) / max(volume, 1e-9)
            if area:
                e += abs(so.Area - area) / max(area, 1e-9)
            return e

        def on_it(so):
            n = 0
            for r in HOLE_RECS:
                try:
                    if so.distToShape(Part.Vertex(V(*r["at"])))[0] < 1e-3:
                        n += 1
                except Exception:
                    pass
            return n
        best = min(sols, key=err)
        if HOLE_RECS:
            close = [so for so in sols if err(so) <= max(err(best), 0.005)]
            hit = max(close, key=on_it)
            if on_it(hit) > 0:
                best = hit
        rep["part_pick"] = {"solids": len(sols), "volume": round(best.Volume, 3), "want_volume": volume, "rel_err": round(err(best), 6)}
        if err(best) > 0.005:
            rep["warnings"].append("closest solid differs by %.2f%% from the wanted volume/area" % (err(best) * 100))
        return best
    if len(sols) > 1:
        rep["warnings"].append("STEP has %d solids and no part.volume: drawing all of them" % len(sols))
        return Part.makeCompound(sols)
    return s


pspec = spec.get("part", {})
if pspec.get("step"):
    shape = load_step(pspec["step"], pspec.get("volume"), pspec.get("area"))
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
front_spec = spec.get("front", "auto")
if front_spec == "auto":
    # 主视 = 最能反映形状的面: 沿最薄方向看(板件看板面, 与用户 OYM 图一致); 视线从正侧来
    exts = [(bb.XLength, V(1, 0, 0)), (bb.YLength, V(0, 1, 0)), (bb.ZLength, V(0, 0, 1))]
    front = min(exts, key=lambda e: e[0])[1]
    rep["front"] = "auto"
else:
    front = V(*front_spec)
front = front.normalize()
# 三根轴: ax_d = 视线(主视方向), ax_h = 页面"上", ax_w = 页面"右" = 上 × 视线
ax_d = V(abs(front.x), abs(front.y), abs(front.z))
if front_spec == "auto":
    # 剩下两根轴里长的横放(用户 OYM 图: 长边水平)
    others = sorted([e for e in exts if abs(e[1].dot(ax_d)) < 0.5], key=lambda e: -e[0])
    ax_h = others[1][1]
else:
    ax_h = V(0, 0, 1) if abs(front.z) < 0.9 else V(0, 1, 0)
# 方案 up = 主视里朝上的方向(模型坐标; 自动选的上下 / 正反不合意时用, 如用户图与自动图上下颠倒 → up 取反)
if spec.get("up"):
    u = V(*spec["up"]).normalize()
    if abs(u.dot(front)) > 1e-6:
        rep["warnings"].append("up is not perpendicular to front: ignored")
    else:
        ax_h = u
ax_w = ax_h.cross(front)
rep["axes"] = {"front": [front.x, front.y, front.z], "right": [ax_w.x, ax_w.y, ax_w.z], "up": [ax_h.x, ax_h.y, ax_h.z]}


def ext(a):
    return abs(a.x) * bb.XLength + abs(a.y) * bb.YLength + abs(a.z) * bb.ZLength


Wm, Hm, Dm = ext(ax_w), ext(ax_h), ext(ax_d)  # 主视宽 / 高, 深(俯视的高 = 左视的宽)

# ---------------------------------------------------------------- 比例: 国标优先系列里放得下的最大
SCALES = [5, 4, 2.5, 2, 1.5, 1, 1 / 1.5, 1 / 2, 1 / 2.5, 1 / 3, 1 / 4, 1 / 5, 1 / 10, 1 / 20]
MARGIN = 26.0  # 视图四周给尺寸留的地方(页面毫米)
GAPV = 22.0  # 视图之间
aw, ah = AREA[2] - AREA[0], AREA[3] - AREA[1]


def fits(s, sec_at):
    """sec_at = 剖视放哪: right = 俯视右边(横向要两个主视宽), below = 俯视下面(纵向多一个厚度)。右边留 ~15% 给轴测。"""
    if sec_at == "right":
        w = max((Wm + Dm) * s + GAPV + 2 * MARGIN, 2 * Wm * s + GAPV + 3 * MARGIN)
        h = (Hm + Dm) * s + GAPV + 2 * MARGIN
    else:
        w = (Wm + Dm) * s + GAPV + 2 * MARGIN
        h = (Hm + 2 * Dm) * s + 2 * GAPV + 3 * MARGIN
    return w <= aw * 0.85 and h <= ah


scale = spec.get("scale", "auto")
SEC_AT = "right"
if scale == "auto":
    s_r = next((s for s in SCALES if fits(s, "right")), SCALES[-1])
    s_b = next((s for s in SCALES if fits(s, "below")), SCALES[-1])
    scale, SEC_AT = (s_r, "right") if s_r >= s_b else (s_b, "below")
rep["scale"] = scale
rep["section_at"] = SEC_AT


def scale_text(s):
    return "1:1" if abs(s - 1) < 1e-9 else ("%s:1" % fmt(s, 2) if s > 1 else "1:%s" % fmt(1 / s, 2))


# ---------------------------------------------------------------- 三视图(第一角)
grp = doc.addObject("TechDraw::DrawProjGroup", "Proj")
page.addView(grp)
grp.Source = [part]
grp.ProjectionType = std.get("projection", "First angle")
grp.ScaleType = "Custom"
grp.Scale = scale
# 先建主视并定方向, 再加俯视 / 左视: 子视图方向在 addProjection 时按锚点当时的方向算(后改锚点不跟着变)
for name in ("Front", "Top", "Left"):
    it = grp.addProjection(name)
    try:
        it.HardHidden = True  # 虚线(不可见轮廓); 建好就设, 别等后台消隐算完再改
    except Exception:
        pass
    if name == "Front":
        try:
            grp.Anchor.Direction = front
            grp.Anchor.XDirection = ax_w
            doc.recompute()
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


# 每个视图: (页面横向对应的模型轴, 页面竖向对应的模型轴, 视线方向轴)。第一角: 俯视在主视下, 左视在主视右。
VAX = {"Front": (ax_w, ax_h, ax_d), "Top": (ax_w, ax_d, ax_h), "Left": (ax_d, ax_h, ax_w)}
R = {n: vrect(views[n], ext(VAX[n][0]) * scale, ext(VAX[n][1]) * scale) for n in VAX}

# ---------------------------------------------------------------- 孔
holes = spec.get("holes", "auto")
if holes == "auto":
    holes = find_holes(shape)


def apply_threads(holes):
    """螺纹: ① 孔记录(打孔工具里选了 M4 且用途 = 螺纹底孔)按位置对上 → tap = M4; ② 没记录的按"孔径 → 螺纹"规则
    (规范 thread_by_diameter, 方案 threads 覆盖; 是项目 / 个人建模习惯 —— ⌀3.2 也可能是 M3 的过孔, 所以不内置)。
    孔记录还带 key(关键尺寸 *)/ tol(公差带)时一并带上。"""
    rules = dict(std.get("thread_by_diameter") or {})
    rules.update(spec.get("threads") or {})
    rule_d = [(float(k), v) for k, v in rules.items()]
    used = 0
    for h in holes:
        at, ax = V(*h["at"]), V(*h["axis"])
        for r in HOLE_RECS:
            p = V(*r["at"])
            off = (p - at) - ax * (p - at).dot(ax)  # 记录入口点到孔轴线的距离
            if off.Length < 0.05 and abs(float(r.get("d", h["d"])) - h["d"]) < 0.05:
                if r.get("usage") == "tap" and r.get("std"):
                    h["tap"] = r["std"]
                for k in ("key", "tol"):
                    if r.get(k):
                        h[k] = r[k]
                h["from"] = "record"
                used += 1
                break
        if not h.get("tap"):
            for d, t in rule_d:
                if abs(h["d"] - d) < 0.01:
                    h["tap"] = t
                    h["from"] = "rule"
                    break
        if h.get("tap") and std.get("thread_class"):
            h["tap"] = h["tap"] + std["thread_class"]
    rep["threads"] = {"records": len(HOLE_RECS), "matched": used, "rules": rules}
    return holes


holes = apply_threads(holes)
rep["holes"] = holes

def hole_view(h):
    """孔在哪个视图里是圆: 孔轴 ∥ 该视图视线。"""
    a = V(*h["axis"])
    for n, (_, _, d) in VAX.items():
        if abs(abs(a.dot(d)) - 1) < 1e-3:
            return n
    return None


# ---------------------------------------------------------------- 剖视: 在"孔最多的视图"上横着剖, 剖切面过孔最多的那排孔轴
sec = None
sec_spec = spec.get("section", "auto")
sec_base = None
if sec_spec == "auto" and holes:
    from collections import Counter

    per_view = Counter(hole_view(h) for h in holes if hole_view(h))
    if per_view:
        sec_base = per_view.most_common(1)[0][0]
        bw, bh, bd = VAX[sec_base]
        cnt = Counter(round(V(*h["at"]).dot(bh), 2) for h in holes if hole_view(h) == sec_base)
        hval = cnt.most_common(1)[0][0]
        o = V(bb.Center.x, bb.Center.y, bb.Center.z)
        o = o + bh * (hval - o.dot(bh))
        sec_spec = {"base": sec_base, "origin": [o.x, o.y, o.z], "normal": [bh.x, bh.y, bh.z], "symbol": "A"}
    else:
        sec_spec = None
if isinstance(sec_spec, dict):
    sec_base = sec_spec.get("base", "Front")
    bw, bh, bd = VAX[sec_base]
    sec = doc.addObject("TechDraw::DrawViewSection", "SectionA")
    page.addView(sec)
    sec.BaseView = views[sec_base]
    sec.Source = [part]
    sec.SectionNormal = V(*sec_spec["normal"])
    sec.SectionOrigin = V(*sec_spec["origin"])
    sec.Direction = V(*sec_spec["normal"])
    sec.XDirection = bw
    sec.SectionSymbol = sec_spec.get("symbol", "A")
    sec.Label = "%s-%s" % (sec.SectionSymbol, sec.SectionSymbol)  # 国标剖视名 "A-A", 界面里 ShowLabel 时显示
    sec.Scale = scale
    sec.ScaleType = "Custom"
    try:
        sec.CutSurfaceDisplay = "PatHatch"
    except Exception:
        pass
    # 剖视大小 = 基视图宽 × 视线方向厚度; 放在俯视右边(左视下面那块空地), 与俯视同高; 出界就往下挪
    sw, sh_ = ext(bw) * scale, ext(bd) * scale
    if SEC_AT == "right":
        sec.X = R["Top"][2] + GAPV + MARGIN + sw / 2
        sec.Y = (R["Top"][1] + R["Top"][3]) / 2
    else:
        sec.X = (R["Top"][0] + R["Top"][2]) / 2
        sec.Y = R["Top"][1] - GAPV - MARGIN - sh_ / 2
    R["Section"] = vrect(sec, sw, sh_)
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
# 轴测方向 = 主视方向 + 右 + 上(正等测), X 方向 = 右 − 主视方向(与视线垂直)
iso.Direction = (front + ax_w + ax_h).normalize()
iso.XDirection = (ax_w - front).normalize()
used_r = max(r[2] for r in R.values())
free_w = AREA[2] - used_r - 10
diag = math.sqrt(bb.XLength ** 2 + bb.YLength ** 2 + bb.ZLength ** 2)
iso_s = next((s for s in SCALES if diag * s <= max(free_w, 60) and s <= scale * 0.75), SCALES[-1])  # 轴测只作参考, 不比主视大
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
FSPEC = "%." + str(DIMS["decimals"]) + "w"


def place_linear(vname, axis, side, name, value):
    """外形尺寸: vname 视图里沿 axis("x" 横 / "y" 竖)最长的边, 放在 side(top / bottom / left / right)外侧。"""
    v, r = views[vname], R[vname]
    e = longest(v, axis)
    if not e:
        return
    cx, cy = (r[0] + r[2]) / 2, (r[1] + r[3]) / 2
    # 左右两侧多让 6 mm: 剖切符号的箭头伸在视图左右外面
    p = {"top": (cx, r[3] + G), "bottom": (cx, r[1] - G), "left": (r[0] - G - 6, cy), "right": (r[2] + G + 6, cy)}[side]
    add_dim(v, "DistanceX" if axis == "x" else "DistanceY", [e], name, p, fmt_spec=FSPEC)
    placer.take(text_rect(p[0], p[1], fmt(value)))


# ---------------------------------------------------------------- 孔位: 坐标式(ordinate)尺寸, 照用户 OYM 图
# TechDraw 没有坐标尺寸类型 → 自己画: 视图边外一小段引出线(装饰线) + 数值文字; 基准 = 视图左下角, 0 也标。
# Y 值在主视左边(横排, 右对齐), X 值在主视上边(竖排)。相邻值在页面上挤 → 引出线长短交替错开。
ORD = std.get("ordinate", {"tick": 5.0, "gap": 1.0, "min_sep": 4.2})
ord_extra = {"left": 0.0, "top": 0.0}


def spread(pos, sep):
    """一维标签散开: 期望位置 pos(已排序)→ 实际位置, 相邻至少 sep, 整体尽量不偏(迭代推开)。"""
    out = list(pos)
    for _ in range(200):
        moved = False
        for i in range(1, len(out)):
            d = out[i] - out[i - 1]
            if d < sep - 1e-6:
                push = (sep - d) / 2
                out[i - 1] -= push
                out[i] += push
                moved = True
        if not moved:
            break
    return out


PAGE_SVG = []  # 页面级图形(页面毫米, SVG 坐标 y 向下): 坐标尺寸的引出线 + 数值, 最后合成一个 DrawViewSymbol


def ordinates(vname, hole_list):
    """坐标尺寸: 只取 hole_list 里孔的中心(不取圆角弧心), 基准 = 视图左下角, 末值 = 外形尺寸(所以不再另标总长 / 总宽)。
    引出线: 视图边外一小段 → 斜折到散开后的文字位置。线和字都画进页面级 SVG(一个对象):
    用视图装饰线时每加一条 TechDraw 就重算一次消隐(80 条 ≈ 200 s), 而且 3D 装饰线导出里不显示。"""
    r = R[vname]
    bw_, bh_, bd_ = VAX[vname]
    pts = []
    for h in hole_list:
        at = V(*h["at"])
        pts.append(((at - bb.Center).dot(bw_), (at - bb.Center).dot(bh_)))
    if not pts:
        return False
    cxp, cyp = (r[0] + r[2]) / 2, (r[1] + r[3]) / 2
    hw, hh_ = ext(bw_) / 2, ext(bh_) / 2  # 模型单位, 以包围盒中心为原点
    fs = std["fonts"]["dim"]
    lw = std["lines"]["thin"]
    fam = std["fonts"]["latin"]
    tick, gap, jog = ORD["tick"], ORD["gap"], 4.0  # 页面毫米

    def uniq(vals):
        out = []
        for val in sorted(vals):
            if not out or abs(val - out[-1]) > 0.01:
                out.append(val)
        return out

    def sy(y):
        return H - y  # 页面 y(向上)→ SVG y(向下)

    def poly(*pts_):
        PAGE_SVG.append('<polyline points="%s" style="fill:none;stroke:#000;stroke-width:%g"/>'
                        % (" ".join("%.3f,%.3f" % (x, sy(y)) for x, y in pts_), lw))

    def text(x, y, s, anchor, rot=0):
        tr_ = ' transform="rotate(%g %.3f %.3f)"' % (rot, x, sy(y)) if rot else ""
        PAGE_SVG.append('<text x="%.3f" y="%.3f" style="font-family:%s;font-size:%gpx;text-anchor:%s;fill:#000"%s>%s</text>'
                        % (x, sy(y), fam, fs, anchor, tr_, s))

    # Y: 左边, 文字横排右对齐
    ys = uniq([-hh_, hh_] + [p[1] for p in pts])
    got = spread([cyp + y * scale for y in ys], fs + 0.8)
    wmax = 0.0
    for y, pg in zip(ys, got):
        py_ = cyp + y * scale
        x0_ = r[0] - gap
        x1_ = x0_ - tick
        x2_ = x1_ - jog
        poly((x0_, py_), (x1_, py_), (x2_, pg))
        txt = fmt(y + hh_)
        w = text_rect(0, 0, txt)[2] * 2
        text(x2_ - 1.0, pg - fs * 0.35, txt, "end")
        placer.take((x2_ - 1.0 - w, pg - fs * 0.6, x2_ - 1.0, pg + fs * 0.6))
        wmax = max(wmax, w)
    ord_extra["left"] = gap + tick + jog + 1.0 + wmax + 2
    # X: 上边, 文字竖排(从下往上读)
    xs = uniq([-hw, hw] + [p[0] for p in pts])
    got = spread([cxp + x * scale for x in xs], fs + 0.8)
    hmax = 0.0
    for x, pg in zip(xs, got):
        px_ = cxp + x * scale
        y0_ = r[3] + gap
        y1_ = y0_ + tick
        y2_ = y1_ + jog
        poly((px_, y0_), (px_, y1_), (pg, y2_))
        txt = fmt(x + hw)
        w = text_rect(0, 0, txt)[2] * 2
        text(pg + fs * 0.35, y2_ + 1.0, txt, "start", rot=-90)
        placer.take((pg - fs * 0.6, y2_ + 1.0, pg + fs * 0.6, y2_ + 1.0 + w))
        hmax = max(hmax, w)
    ord_extra["top"] = gap + tick + jog + 1.0 + hmax + 2
    rep["ordinates"] = {"view": vname, "x": [fmt(x + hw) for x in xs], "y": [fmt(y + hh_) for y in ys]}
    return True


def flush_page_svg():
    """PAGE_SVG → 一个铺满整页的 DrawViewSymbol(中心放在页面中心, 比例 1)。"""
    if not PAGE_SVG:
        return
    sym = doc.addObject("TechDraw::DrawViewSymbol", "PageGraphics")
    page.addView(sym)
    sym.Symbol = ('<svg xmlns="http://www.w3.org/2000/svg" width="%gmm" height="%gmm" viewBox="0 0 %g %g">%s</svg>'
                  % (W, H, W, H, "".join(PAGE_SVG)))
    sym.Scale = 1.0
    sym.X, sym.Y = W / 2, H / 2




# ---------------------------------------------------------------- 孔组: 按"在哪个视图里是圆" + 标注文字分组, 编号 H1…
# 编号顺序固定(视图 主 → 俯 → 左, 组内件数多的在前, 再按文字), 方案 / 偏好 / agent 用编号挑"标哪些孔位"。
def hole_groups():
    out = []
    for vname in VAX:
        groups = {}
        for h in holes:
            if hole_view(h) == vname:
                groups.setdefault(callout(h, 1), []).append(h)  # 按标注文字分组: 深度 11.9999 与 12 是同一组
        for k, hs in sorted(groups.items(), key=lambda kv: (-len(kv[1]), kv[0])):
            out.append({"view": vname, "text": callout(dict(hs[0]), len(hs)), "holes": hs})
    for i, g in enumerate(out):
        g["id"] = "H%d" % (i + 1)
    return out


HOLE_GROUPS = hole_groups()
HOLE_IDS = {g["id"] for g in HOLE_GROUPS}  # 孔标注尺寸对象的名字 = 孔组编号(导出时据此改成水平文字 + 引线)


def ordinate_groups():
    """方案 ordinate: true / 省略 = 主视里全部孔组; false = 不标孔位; {"groups": ["H1", …]} 只标这些;
    {"exclude": [...]} 除去这些。返回要标孔位的孔组。"""
    o = spec.get("ordinate", True)
    front = [g for g in HOLE_GROUPS if g["view"] == "Front"]
    if o is False:
        return []
    if isinstance(o, dict):
        if o.get("groups") is not None:
            return [g for g in front if g["id"] in o["groups"]]
        if o.get("exclude"):
            return [g for g in front if g["id"] not in o["exclude"]]
    return front


def group_positions(g):
    """孔组里各孔在其视图里的坐标(基准 = 视图左下角, 文档单位), 写进报告供挑选。"""
    bw_, bh_, _ = VAX[g["view"]]
    hw, hh_ = ext(bw_) / 2, ext(bh_) / 2
    return [[fmt((V(*h["at"]) - bb.Center).dot(bw_) + hw), fmt((V(*h["at"]) - bb.Center).dot(bh_) + hh_)] for h in g["holes"]]


ORD_GROUPS = ordinate_groups()
rep["hole_groups"] = [{"id": g["id"], "view": g["view"], "text": g["text"], "count": len(g["holes"]),
                       "positions": group_positions(g), "ordinate": g in ORD_GROUPS} for g in HOLE_GROUPS]
has_ord = bool(ORD_GROUPS) and ordinates("Front", [h for g in ORD_GROUPS for h in g["holes"]])


# ---------------------------------------------------------------- 圆形件: 视图轮廓是整圆 → 标直径, 不标长 × 宽
def outer_round(vname):
    """视图方向上最大的外圆柱面(凸, 轴线 ∥ 视线), 且直径 ≈ 视图外形(≥ 98%)→ 返回半径, 否则 None。"""
    _, _, d = VAX[vname]
    best = None
    for f in shape.Faces:
        s = f.Surface
        if not isinstance(s, Part.Cylinder) or abs(abs(s.Axis.normalize().dot(d)) - 1) > 1e-6:
            continue
        u0, u1, v0, v1 = f.ParameterRange
        p, n = f.valueAt((u0 + u1) / 2, (v0 + v1) / 2), f.normalAt((u0 + u1) / 2, (v0 + v1) / 2)
        ax = s.Axis.normalize()
        rad = (p - s.Center) - ax * ((p - s.Center).dot(ax))
        if rad.Length > 1e-6 and n.dot(rad) > 0 and (best is None or s.Radius > best):
            best = s.Radius
    bw_, bh_, _ = VAX[vname]
    if best and 2 * best >= 0.98 * max(ext(bw_), ext(bh_)):
        return best
    return None


def place_diameter(vname, radius, name):
    """在视图里找半径 = radius 的圆边, 标 ⌀(位置: 视图右上 / 左上 / 右下外侧, 避让)。"""
    v, r = views[vname], R[vname]
    circ = [(n, ed) for n, ed in edges_of(v) if isinstance(ed.Curve, Part.Circle) and abs(ed.Curve.Radius - radius) < 1e-3]
    if not circ:
        rep["warnings"].append("round %s: circle edge not found" % vname)
        return False
    txt = DIMS["diameter"] + fmt(2 * radius)
    for ring in range(8):
        off = G * 1.6 + ring * STEP
        for p in ((r[2] + off, r[3] + off * 0.6), (r[0] - off, r[3] + off * 0.6), (r[2] + off, r[1] - off * 0.6)):
            rr = text_rect(p[0], p[1], txt)
            if placer.free(rr):
                placer.take(rr)
                add_dim(v, "Diameter", [circ[0][0]], name, p, fmt_spec=DIMS["diameter"] + FSPEC)
                return True
    return False


# 外形: 主视 宽(上)、高(左); 厚度标在左视(上)。第一角里主视下面是俯视, 所以宽标在上面。
# 有坐标尺寸时末值就是总长 / 总宽, 不再重复标; 主视轮廓是整圆 → 标直径。
front_r = outer_round("Front")
rep["round_front"] = bool(front_r)
if front_r:
    place_diameter("Front", front_r, "DiaOuter")
elif not has_ord:
    place_linear("Front", "x", "top", "LenW", Wm)
    place_linear("Front", "y", "left", "LenH", Hm)
place_linear("Left", "x", "top", "LenD", Dm)


def circle_for(vname, circles, h):
    """视图里与孔对应的圆边: 半径 = d/2, 圆心最接近孔轴投影。视图边坐标 = 模型单位(未乘比例)、以零件包围盒中心为原点,
    横 / 竖方向的正负不确定(四种都试)。返回 (边名, 圆心在页面上相对视图中心的偏移 = 边坐标 × 比例)。"""
    bw_, bh_, _ = VAX[vname]
    at = V(*h["at"])
    px, py = (at - bb.Center).dot(bw_), (at - bb.Center).dot(bh_)
    best, bdist = None, 1e9
    for n, ed in circles:
        if abs(ed.Curve.Radius - h["d"] / 2) > 1e-3:
            continue
        c = ed.Curve.Center
        for sx in (1, -1):
            for sy in (1, -1):
                dd = math.hypot(c.x - sx * px, c.y - sy * py)
                if dd < bdist:
                    best, bdist = (n, V(c.x * scale, c.y * scale, 0)), dd
    return best


# 孔: 按"在哪个视图里是圆"分视图, 同规格合并成一条标注("6 x ⌀4.3 贯穿"), 引线指向其中一个
circles_of = {}
for g in HOLE_GROUPS:
    vname, hs, txt = g["view"], g["holes"], g["text"]
    v, r = views[vname], R[vname]
    if vname not in circles_of:
        circles_of[vname] = [(n, ed) for n, ed in edges_of(v) if isinstance(ed.Curve, Part.Circle)]
    circles = circles_of[vname]
    cxV, cyV = (r[0] + r[2]) / 2, (r[1] + r[3]) / 2
    hits = [(hh, circle_for(vname, circles, hh)) for hh in hs]
    hits = [(hh, ht) for hh, ht in hits if ht]
    if not hits:
        rep["warnings"].append("no circle for hole group %s in %s" % (g["id"], vname))
        continue
    # 候选位置: 视图四周外侧一圈圈往外; 对组里每个孔 × 每个空位算引线长, 取最短(引线短、少横穿图形)
    best = None
    for hh, (n, c) in hits:
        px_, py_ = cxV + c.x, cyV + c.y
        for ring in range(8):
            off = G * 1.6 + ring * STEP
            for p in ((px_ + off * 0.5, r[3] + off), (px_ - off * 0.5, r[3] + off),
                      (px_ + off * 0.5, r[1] - off), (px_ - off * 0.5, r[1] - off),
                      (r[2] + off + 12, py_ + off * 0.3), (r[0] - off - 12, py_ + off * 0.3)):
                rr = text_rect(p[0], p[1], txt)
                if not placer.free(rr):
                    continue
                dist = math.hypot(p[0] - px_, p[1] - py_)
                if best is None or dist < best[0]:
                    best = (dist, n, p, rr)
    if best is None:
        hh, (n, c) = hits[0]
        spot = (r[2] + G * 2, cyV + c.y)
        rep["warnings"].append("callout overlaps: " + txt)
    else:
        _, n, spot, rr = best
        placer.take(rr)
    add_dim(v, "Diameter", [n], g["id"], spot, text=txt)


# ---------------------------------------------------------------- 螺纹画法(国标): 孔是圆的视图里, 大径画约 3/4 圈细实线
# (底孔 = 小径, 视图里本来就是粗实线圆)。缺口在右上象限。只画看得见的: 贯穿, 或入口朝着看的人; 背面的盲孔不画。
# 视图边坐标与页面的正负关系: 用视图里全部孔对圆边做四种正负的匹配, 取对上最多的(在主视上校准, 主视的模型 → 页面已由坐标尺寸验证)。
def edge_signs(vname):
    bw_, bh_, _ = VAX[vname]
    circles = circles_of.get(vname) or [(n, ed) for n, ed in edges_of(views[vname]) if isinstance(ed.Curve, Part.Circle)]
    hs = [h for h in holes if hole_view(h) == vname]
    best, bn = (1, 1), -1
    for sx in (1, -1):
        for sy in (1, -1):
            n_ = 0
            for h in hs:
                at = V(*h["at"])
                px, py = (at - bb.Center).dot(bw_), (at - bb.Center).dot(bh_)
                if any(abs(ed.Curve.Radius - h["d"] / 2) < 1e-3 and math.hypot(ed.Curve.Center.x - sx * px, ed.Curve.Center.y - sy * py) < 1e-2
                       for _, ed in circles):
                    n_ += 1
            if n_ > bn:
                best, bn = (sx, sy), n_
    return best, bn, len(hs)


def thread_major(tap):
    import re

    m = re.match(r"\s*M\s*(\d+(?:\.\d+)?)", tap or "")
    return float(m.group(1)) if m else None


def draw_threads():
    taps = [h for h in holes if h.get("tap") and thread_major(h["tap"]) and hole_view(h)]
    if not taps:
        return
    # 边坐标 → 页面的正负: 主视 模型 → 页面 = (+, +), 所以 主视里 模型 → 边 的正负 = 边 → 页面的正负(各视图通用)
    (ex, ey), got, tot = edge_signs("Front")
    if tot == 0 or got == 0:
        ex, ey = 1, 1
        rep["warnings"].append("thread arcs: no holes in Front to calibrate edge signs, assumed (+,+)")
    lw = std["lines"]["thin"]
    drawn = 0
    for vname in VAX:
        hs = [h for h in taps if hole_view(h) == vname]
        if not hs:
            continue
        (sx, sy), _, _ = edge_signs(vname)
        bw_, bh_, _ = VAX[vname]
        vdir = V(views[vname].Direction) if hasattr(views[vname], "Direction") else front  # 视线: 由零件指向看的人
        cxV, cyV = vcenter(views[vname])
        circles = circles_of.get(vname) or [(n, ed) for n, ed in edges_of(views[vname]) if isinstance(ed.Curve, Part.Circle)]
        for h in hs:
            if not h.get("through") and V(*h["axis"]).dot(vdir) <= 0:
                continue  # 背面的盲孔: 看不见
            at = V(*h["at"])
            px, py = (at - bb.Center).dot(bw_), (at - bb.Center).dot(bh_)
            hit = next((ed for _, ed in circles if abs(ed.Curve.Radius - h["d"] / 2) < 1e-3
                        and math.hypot(ed.Curve.Center.x - sx * px, ed.Curve.Center.y - sy * py) < 1e-2), None)
            if hit is None:
                rep["warnings"].append("thread arc: no circle for %s at %s in %s" % (h["tap"], h["at"], vname))
                continue
            X = cxV + ex * hit.Curve.Center.x * scale
            Y = H - (cyV + ey * hit.Curve.Center.y * scale)  # SVG y 向下
            r_ = thread_major(h["tap"]) / 2 * scale
            # 从顶点出发逆时针(屏幕上向左)走 270° 到右点: 缺口在右上象限
            PAGE_SVG.append('<path d="M %.3f %.3f A %.3f %.3f 0 1 0 %.3f %.3f" style="fill:none;stroke:#000;stroke-width:%g"/>'
                            % (X, Y - r_, r_, r_, X + r_, Y, lw))
            drawn += 1
    rep["thread_arcs"] = {"drawn": drawn, "tapped": len(taps), "edge_signs": [ex, ey]}
    draw_thread_sides(taps, ex, ey)


# 孔轴 ⟂ 视线的视图里的螺纹(国标): 大径两条细线平行孔轴, 画到螺纹长度, 螺纹终止线 = 粗线横过大径。
#   剖视里(孔轴在剖切面上, 被剖开): 实线; 剖面线 TechDraw 已画到孔壁(= 小径)。
#   一般视图里(孔在里面看不见): 全画虚线。
# 视图边坐标(实测): e = ((p − C)·X, (p − C)·U), X = 视图 XDirection, U = Direction × XDirection,
# C = 视图所画形体的包围盒中心(剖视 = 留下的那一半)。边 → 页面的正负沿用主视校准的 (ex, ey)。
def view_frame(v, sh):
    X = V(v.XDirection).normalize()
    U = V(v.Direction).normalize().cross(X)
    return X, U, sh.BoundBox.Center


def kept_half(v, sec_spec_):
    """剖视留下的那一半: 两半里投影外形与视图边包围盒对得上的那个(对不上就用整体)。"""
    X = V(v.XDirection).normalize()
    U = V(v.Direction).normalize().cross(X)
    es = [ed for _, ed in edges_of(v)]
    if not es:
        return shape
    xs = [p.Point.x for ed in es for p in ed.Vertexes]
    ys = [p.Point.y for ed in es for p in ed.Vertexes]
    want = (max(xs) - min(xs), max(ys) - min(ys))
    O, N = V(*sec_spec_["origin"]), V(*sec_spec_["normal"]).normalize()
    big = 4 * (bb.DiagonalLength + 10)
    best, berr = shape, 1e9
    for sgn in (1, -1):
        try:
            # 半空间: 以 O 为中心、沿 sgn·N 一侧的大盒子
            n = N * sgn
            u = n.cross(V(1, 0, 0)) if abs(n.x) < 0.9 else n.cross(V(0, 1, 0))
            u.normalize()
            w = n.cross(u)
            box = Part.makeBox(big, big, big)
            m = App.Matrix(u.x, w.x, n.x, 0, u.y, w.y, n.y, 0, u.z, w.z, n.z, 0, 0, 0, 0, 1)
            box = box.transformGeometry(m)
            box.translate(O - u * (big / 2) - w * (big / 2))
            half = shape.common(box)
            if half.isNull() or half.Volume < 1e-6:
                continue
            hb = half.BoundBox
            corners = [V(x, y, z) for x in (hb.XMin, hb.XMax) for y in (hb.YMin, hb.YMax) for z in (hb.ZMin, hb.ZMax)]
            px = [c.dot(X) for c in corners]
            py = [c.dot(U) for c in corners]
            err = abs((max(px) - min(px)) - want[0]) + abs((max(py) - min(py)) - want[1])
            if err < berr:
                best, berr = half, err
        except Exception as e:
            rep["warnings"].append("section half: %s" % e)
    return best


def draw_thread_sides(taps, ex, ey):
    lw, lk = std["lines"]["thin"], std["lines"]["visible"]
    dash = std["lines"].get("hidden_dash", "2 1")
    targets = [(views[n], shape, False) for n in VAX]
    if sec is not None and isinstance(sec_spec, dict):
        targets.append((sec, kept_half(sec, sec_spec), True))
    drawn = 0
    for v, sh, is_sec in targets:
        X, U, C = view_frame(v, sh)
        D = V(v.Direction).normalize()
        vx, vy = vcenter(v)

        def page(p):
            e0, e1 = (p - C).dot(X), (p - C).dot(U)
            return vx + ex * e0 * scale, H - (vy + ey * e1 * scale)

        for h in taps:
            a = V(*h["axis"]).normalize()  # 由材料指向入口外侧
            if abs(a.dot(D)) > 1e-3:
                continue  # 孔在这个视图里是圆(另有 3/4 圈画法)
            at = V(*h["at"])
            if is_sec:
                N = V(*sec_spec["normal"]).normalize()
                if abs((at - V(*sec_spec["origin"])).dot(N)) > 1e-3:
                    continue  # 不在剖切面上: 剖视里不画虚线
            L = h.get("len") if h.get("through") else h.get("depth")
            L = L or h.get("len") or h.get("depth")
            if not L:
                continue
            R_ = thread_major(h["tap"]) / 2
            side = a.cross(D).normalize()  # 视图平面内、垂直孔轴
            end = at - a * L
            style = "fill:none;stroke:#000;stroke-width:%g%s" % (lw, "" if is_sec else ";stroke-dasharray:%s" % dash)
            for s_ in (1, -1):
                p0, p1 = page(at + side * (R_ * s_)), page(end + side * (R_ * s_))
                PAGE_SVG.append('<path d="M %.3f %.3f L %.3f %.3f" style="%s"/>' % (p0[0], p0[1], p1[0], p1[1], style))
            if not h.get("through"):
                p0, p1 = page(end + side * R_), page(end - side * R_)
                st = ("fill:none;stroke:#000;stroke-width:%g" % lk) if is_sec else style
                PAGE_SVG.append('<path d="M %.3f %.3f L %.3f %.3f" style="%s"/>' % (p0[0], p0[1], p1[0], p1[1], st))
            drawn += 1
    rep["thread_sides"] = {"drawn": drawn}


draw_threads()
flush_page_svg()
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
        if vo is not None and o.Name in HOLE_IDS and hasattr(vo, "StandardAndStyle"):
            try:
                vo.StandardAndStyle = "ISO Referencing"
            except Exception as e:
                rep["warnings"].append("hole style: %s" % e)
    progress("gui: open page")
    page.ViewObject.doubleClicked()
    for _ in range(20):
        Gui.updateGui()
        time.sleep(0.05)
    progress("gui: export svg")
    # 正式 PDF 不用 Qt 的写出器: 它嵌的字体子集(仿宋 / Arial Unicode / 回退到的 3ds-Light)Acrobat 报"页面有错误"、
    # 尺寸文字不显示(浏览器能看)。正式 PDF 由 draw.ps1 在 FreeCAD 退出后用无头 Edge / Chrome 把 SVG 打印出来
    # (在 FreeCAD 进程里起浏览器会挂住)。这里的 Qt PDF 只作没有浏览器时的备用: <base>.qt.pdf。
    TechDrawGui.exportPageAsSvg(page, os.path.join(OUT, BASE + ".svg"))
    progress("gui: export qt pdf")
    TechDrawGui.exportPageAsPdf(page, os.path.join(OUT, BASE + ".qt.pdf"))
    progress("gui: done")
    rep["paper_mm"] = [W, H]
else:
    rep["warnings"].append("no GUI: PDF / SVG skipped (run freecad.exe with QT_QPA_PLATFORM=offscreen)")
doc.saveAs(fc)
lap("total")
rep["out"] = {"dir": OUT, "base": BASE}
with open(os.path.join(OUT, BASE + ".report.json"), "w", encoding="utf-8") as f:
    json.dump(rep, f, ensure_ascii=False, indent=1)
