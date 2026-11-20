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
sys.path.insert(0, HERE)
import geomutil  # noqa: E402

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
        cy = geomutil.cylinder_of(f)  # 解析圆柱, 或几何上是圆柱的 B 样条面
        if cy is None or not cy["internal"]:
            continue  # 不是圆柱 / 外圆柱(凸台)
        cyl.append({"ax": cy["ax"], "c": cy["c"], "r": cy["r"], "t0": cy["t0"], "t1": cy["t1"], "span": cy["span"]})
    groups = []
    for cy in cyl:
        for g in groups:
            a = g[0]
            if abs(abs(a["ax"].dot(cy["ax"])) - 1) < 1e-6:
                d = cy["c"] - a["c"]
                off = d - a["ax"] * d.dot(a["ax"])
                if off.Length < max(0.01, 2e-3 * max(a["r"], cy["r"])):  # B 样条孔拟合出的轴心有误差(大孔可达几十微米); 真不同轴的孔不会这么近
                    cy["t0"], cy["t1"] = sorted([(cy["c"] + cy["ax"] * cy["t0"] - a["c"]).dot(a["ax"]), (cy["c"] + cy["ax"] * cy["t1"] - a["c"]).dot(a["ax"])])
                    g.append(cy)
                    break
        else:
            groups.append([cy])
    holes = []
    partial = []  # 不满一圈的同轴组(长圆孔两端的半圆 / 凹圆角)
    for g in groups:
        a = g[0]
        ax, c = a["ax"], a["c"]
        # 真孔一圈是满的: 最小半径那段的各个面角度加起来 > 200°(圆柱面常被切成两个半圆面; 横孔穿过会缺一块, 所以不要求满 360°)。
        # 凹圆角、长圆孔两端的半圆只有一部分 → 不算圆孔(实测 encoder calibrator 的板上长圆孔被当成"6 x ⌀6.18", 坐标挤成一团)
        rmin_ = min(round(cy["r"], 4) for cy in g)
        cover = sum(cy["span"] for cy in g if abs(round(cy["r"], 4) - rmin_) < 1e-6)
        segs = {}
        for cy in g:
            r = round(cy["r"], 4)
            s0, s1 = segs.get(r, (1e9, -1e9))
            segs[r] = (min(s0, cy["t0"]), max(s1, cy["t1"]))
        if cover < math.radians(200):
            partial.append({"ax": ax, "c": c, "segs": segs, "cover": cover})
            continue
        h = make_hole(bb, c, ax, segs)
        if h:
            holes.append(h)
    holes += find_slots(bb, partial)
    return holes


def find_slots(bb, partial):
    """长圆孔: 两个约 180° 的半圆柱面(同半径、轴平行、同一轴向范围)= 一段槽; 同一槽心的几段(不同半径)= 槽 + 沉槽。
    槽心当孔位, d = 槽宽, slot = [两半圆心距(按半径), 方向]。凑不成对的(凹圆角等)记进报告。"""
    used = set()
    pieces = []  # (槽心, 轴, 半径, 心距, 方向, (t0, t1))
    half = [i for i, p in enumerate(partial) if len(p["segs"]) == 1 and abs(p["cover"] - math.pi) < math.radians(15)]
    for ii, i in enumerate(half):
        if i in used:
            continue
        pi_ = partial[i]
        (r, (t0, t1)), = pi_["segs"].items()
        for j in half[ii + 1:]:
            if j in used:
                continue
            pj = partial[j]
            (r2, (u0, u1)), = pj["segs"].items()
            if abs(r - r2) > 1e-4 or abs(abs(pi_["ax"].dot(pj["ax"])) - 1) > 1e-6:
                continue
            w = pj["c"] - pi_["c"]
            w = w - pi_["ax"] * w.dot(pi_["ax"])
            L = w.Length
            # 两个半圆的轴向范围(换到 i 的轴向坐标)要一样
            sgn = 1 if pi_["ax"].dot(pj["ax"]) > 0 else -1
            base = (pj["c"] - pi_["c"]).dot(pi_["ax"])
            uu = sorted([base + sgn * u0, base + sgn * u1])
            if not (max(0.1, 0.02 * r) < L <= 20 * r) or abs(uu[0] - t0) > 1e-3 or abs(uu[1] - t1) > 1e-3:
                continue
            used.update((i, j))
            ctr = pi_["c"] + w * 0.5
            pieces.append((ctr, pi_["ax"], r, L, V(w).normalize(), (t0, t1)))
            break
    for i, p in enumerate(partial):
        if i not in used:
            rmin_ = min(p["segs"])
            rep.setdefault("partial_cylinders", []).append({"d": round(2 * rmin_, 3), "cover_deg": round(math.degrees(p["cover"])),
                                                           "at": [round(p["c"].x, 2), round(p["c"].y, 2), round(p["c"].z, 2)]})
    # 同槽心、同轴的几段合成一个(小的 = 槽, 大的 = 沉槽)
    out, taken = [], set()
    for k, (ctr, ax, r, L, wdir, tr) in enumerate(pieces):
        if k in taken:
            continue
        segs, lens = {round(r, 4): tr}, {round(r, 4): L}
        for m in range(k + 1, len(pieces)):
            c2, ax2, r2, L2, w2, tr2 = pieces[m]
            dv = c2 - ctr
            if m in taken or abs(abs(ax.dot(ax2)) - 1) > 1e-6 or (dv - ax * dv.dot(ax)).Length > 1e-3:
                continue
            base = dv.dot(ax)
            sgn = 1 if ax.dot(ax2) > 0 else -1
            segs[round(r2, 4)] = tuple(sorted([base + sgn * tr2[0], base + sgn * tr2[1]]))
            lens[round(r2, 4)] = L2
            taken.add(m)
        h = make_hole(bb, ctr, ax, segs)
        if h:
            rs = sorted(segs)
            h["slot"] = round(lens[rs[0]], 4)
            h["slot_dir"] = [round(wdir.x, 6), round(wdir.y, 6), round(wdir.z, 6)]
            if h.get("cb"):
                h["cb_slot"] = round(lens[rs[-1]], 4)
            out.append(h)
    rep["slots"] = len(out)
    return out


def make_hole(bb, c, ax, segs):
    """同轴各半径段 segs {半径: (t0, t1)}(沿 ax、以 c 为原点)→ 孔 {at 入口点, axis 朝外, d, through, depth, cb, len}。"""
    a_ax = ax
    if True:
        # 实体沿轴向的范围(用包围盒八角投影)
        corners = [V(x, y, z) for x in (bb.XMin, bb.XMax) for y in (bb.YMin, bb.YMax) for z in (bb.ZMin, bb.ZMax)]
        ts = [(p - c).dot(ax) for p in corners]
        lo, hi = min(ts), max(ts)
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
        top = c + a_ax * (span_hi if entry_hi else span_lo)
        return {"at": [round(top.x, 4), round(top.y, 4), round(top.z, 4)], "axis": [round(ax.x, 6), round(ax.y, 6), round(ax.z, 6)],
                "d": round(2 * rmin, 4), "through": through, "depth": depth, "cb": cb, "len": round(span_hi - span_lo, 4)}


def callout(h, n):
    """国标孔标注文字: 6 x ⌀4.3 贯穿 / 2 x ⌀6.6 贯穿 ⌴⌀11↧6.5 / 4 x M4 ↧10。"""
    D = DIMS
    s = D["count_format"].format(n=n) if n > 1 else ""
    if h.get("key"):
        s = D["key_mark"] + s
    if h.get("slot"):
        # 长圆孔: 宽 × 总长(= 宽 + 两半圆心距)
        s += D.get("slot", "长圆孔") + " " + fmt(h["d"]) + "×" + fmt(h["d"] + h["slot"])
    elif h.get("tap"):
        s += h["tap"]
    else:
        s += D["diameter"] + fmt(h["d"])
    if h.get("tol"):
        s += " " + h["tol"]
    s += " " + D["through"] if h.get("through") else (" " + D["depth"] + fmt(h["depth"]) if h.get("depth") else "")
    if h.get("cb") and h.get("slot"):
        s += "  " + D["counterbore"] + fmt(h["cb"][0]) + "×" + fmt(h["cb"][0] + h.get("cb_slot", h["slot"])) + D["depth"] + fmt(h["cb"][1])
    elif h.get("cb"):
        s += "  " + D["counterbore"] + D["diameter"] + fmt(h["cb"][0]) + (" " + h["cb_tol"] + " " if h.get("cb_tol") else "") + D["depth"] + fmt(h["cb"][1])
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
App.ParamGet(P + "General").SetBool("HandleFaces", True)  # 剖面线要找面(装配图编译器会临时关掉, 这里显式打开)

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

# ---------------------------------------------------------------- 回转件(轴 / 盘 / 套): 外圆柱面基本都同轴
def turned_axis():
    """外圆柱面(整圈)按轴线分组, 面积最大的一组占 ≥ 80% 且最大外圆在这组、轴线是坐标轴、垂直轴向的外形 ≈ 最大直径 → 回转件。
    → {ax, c, rmax, steps: [{d, t0, t1}](沿轴, 以 c 为原点)} 或 None。"""
    cyls = []
    for f in shape.Faces:
        cy = geomutil.cylinder_of(f)
        # 整圈或半圈(Plasticity 导出的圆柱常拆成两个半面; 同一段按 直径 + 轴向位置 去重)
        if cy and not cy["internal"] and cy["span"] >= math.pi - 0.05:
            cyls.append((cy, f.Area))
    if not cyls:
        return None
    groups = []
    for cy, a in cyls:
        for g in groups:
            if abs(abs(g["ax"].dot(cy["ax"])) - 1) < 1e-6:
                d = cy["c"] - g["c"]
                if (d - g["ax"] * d.dot(g["ax"])).Length < 1e-3:
                    g["area"] += a
                    g["m"].append(cy)
                    break
        else:
            groups.append({"ax": V(cy["ax"]), "c": V(cy["c"]), "area": a, "m": [cy]})
    g = max(groups, key=lambda g: g["area"])
    total = sum(a for _, a in cyls)
    rmax = max(cy["r"] for cy, _ in cyls)
    if g["area"] < 0.8 * total or max(m["r"] for m in g["m"]) < rmax - 1e-6:
        return None
    ax = g["ax"]
    if max(abs(ax.x), abs(ax.y), abs(ax.z)) < 1 - 1e-6:
        return None   # 轴线不是坐标轴(视图方向按坐标轴取)
    perp = [L for L, e in ((bb.XLength, V(1, 0, 0)), (bb.YLength, V(0, 1, 0)), (bb.ZLength, V(0, 0, 1))) if abs(e.dot(ax)) < 0.5]
    if any(abs(L - 2 * rmax) > 0.02 * 2 * rmax for L in perp):
        return None
    steps = []
    for m in g["m"]:
        # 面自己的 t 沿它自己的轴向量量(可能与组轴反向): 位置 = 轴心差投影 + 方向符号 × t
        sh_ = (m["c"] - g["c"]).dot(ax)
        sg = 1.0 if m["ax"].dot(ax) > 0 else -1.0
        a_, b_ = sh_ + sg * m["t0"], sh_ + sg * m["t1"]
        steps.append({"d": round(2 * m["r"], 4), "t0": min(a_, b_), "t1": max(a_, b_)})
    steps.sort(key=lambda s: s["t0"])
    return {"ax": V(abs(ax.x), abs(ax.y), abs(ax.z)), "axs": V(ax), "c": g["c"], "rmax": rmax, "steps": steps}


TURN = turned_axis() if spec.get("turned", "auto") == "auto" else None
rep["turned"] = bool(TURN)

# ---------------------------------------------------------------- 视图方向
front_spec = spec.get("front", "auto")
if front_spec == "auto" and TURN:
    # GB: 轴 / 盘类主视轴线水平放置(侧面看), 端面形状另用左视表达
    exts = [(bb.XLength, V(1, 0, 0)), (bb.YLength, V(0, 1, 0)), (bb.ZLength, V(0, 0, 1))]
    side = [e for e in exts if abs(e[1].dot(TURN["ax"])) < 0.5]
    front = side[0][1]
    rep["front"] = "auto (turned: axis horizontal)"
elif front_spec == "auto":
    # 主视 = 最能反映形状的面: 沿最薄方向看(板件看板面, 与用户 OYM 图一致); 视线从正侧来
    exts = [(bb.XLength, V(1, 0, 0)), (bb.YLength, V(0, 1, 0)), (bb.ZLength, V(0, 0, 1))]
    front = min(exts, key=lambda e: e[0])[1]
    rep["front"] = "auto"
else:
    front = V(*front_spec)
front = front.normalize()
# 三根轴: ax_d = 视线(主视方向), ax_h = 页面"上", ax_w = 页面"右" = 上 × 视线
ax_d = V(abs(front.x), abs(front.y), abs(front.z))
if front_spec == "auto" and TURN:
    # 页面"上" = 与轴线、视线都垂直的那根 → 页面"右" = 轴线
    ax_h = next(e[1] for e in exts if abs(e[1].dot(TURN["ax"])) < 0.5 and abs(e[1].dot(ax_d)) < 0.5)
elif front_spec == "auto":
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

# ======== 内容(只依赖模型轴, 与视图无关): 先算好, 交给布局层(layout.py)决定比例 / 视图位置 / 标注位置 ========
# 每个视图: (页面横向对应的模型轴, 页面竖向对应的模型轴, 视线方向轴)。第一角: 俯视在主视下, 左视在主视右。
VAX = {"Front": (ax_w, ax_h, ax_d), "Top": (ax_w, ax_d, ax_h), "Left": (ax_d, ax_h, ax_w)}
if TURN:
    del VAX["Top"]   # 回转件不出俯视(与主视相同)
VIEWS3 = [n for n in ("Front", "Top", "Left") if n in VAX]
# ---------------------------------------------------------------- 孔
holes = spec.get("holes", "auto")
if holes == "auto":
    holes = find_holes(shape)
    if rep.get("partial_cylinders"):
        rep["warnings"].append("%d concave partial cylinder(s) not treated as holes (fillets / slot ends / cut holes, < 200 deg)"
                               % len(rep["partial_cylinders"]))


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


# D4: 配合 → 本件公差(ISO 286 查表)。方案 fits: [{"d": 8, "role": "hole" | "shaft", "code": "H7"}](来自装配图里记的配合)。
# 孔: 同直径的孔标注带公差带 + 偏差 "⌀8 H7(+0.015/0)"; 轴: 同直径的外圆柱标 ⌀ 带公差(见下面 SHAFT_TOL)。
import iso286  # noqa: E402  (HERE 已在 sys.path)

SHAFT_TOL = {}  # 直径(round 2)→ 公差文字
for f_ in spec.get("fits") or []:
    try:
        d_, code_ = float(f_["d"]), str(f_["code"]).strip()
        txt_ = iso286.tol_text(d_, code_)
    except Exception as e:
        rep["warnings"].append("fit %s: %s" % (f_, e))
        continue
    if f_.get("role") == "hole":
        n_ = 0
        for h in holes:
            if abs(h["d"] - d_) < 0.05 and not h.get("tap"):
                h["tol"] = txt_
                n_ += 1
            elif h.get("cb") and abs(h["cb"][0] - d_) < 0.05:
                # 配合在台阶孔的大径段(如电机止口 ⌀100 套在 ⌀83 通孔外面): 公差写在沉孔那段
                h["cb_tol"] = txt_
                n_ += 1
        if not n_:
            rep["warnings"].append("fit hole %s %s: no hole of that size" % (fmt(d_), code_))
    else:
        SHAFT_TOL[round(d_, 2)] = txt_
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
if sec_spec == "auto" and TURN:
    # 回转件: 过轴线剖主视(GB 轴 / 盘类的常规表达; 实测编码器轴按"孔最多的视图"横剖, 剖视是空的)
    o_ = TURN["c"]
    n_ = VAX["Front"][2]
    sec_spec = {"base": "Front", "origin": [o_.x, o_.y, o_.z], "normal": [n_.x, n_.y, n_.z], "symbol": "A"}
    sec_base = "Front"
elif sec_spec == "auto" and holes:
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


def bolt_circle(g):
    """孔组的孔在视图平面上均布在一个圆上(≥ 3 个, 半径一致、相邻角度相等)→ (圆心(视图平面坐标), 节圆直径), 否则 None。
    国标: 这种孔标 "8 x ⌀5 ↧5  ⌀62 均布", 不逐个标坐标(实测法兰 14 个孔的坐标挤成一串)。"""
    if len(g["holes"]) < 3:
        return None
    bw_, bh_, _ = VAX[g["view"]]
    pts = [((V(*h["at"]) - bb.Center).dot(bw_), (V(*h["at"]) - bb.Center).dot(bh_)) for h in g["holes"]]
    cx_ = sum(p[0] for p in pts) / len(pts)
    cy_ = sum(p[1] for p in pts) / len(pts)  # 均布的点, 圆心 = 质心
    rs = [math.hypot(p[0] - cx_, p[1] - cy_) for p in pts]
    R_ = sum(rs) / len(rs)
    if R_ < 1e-3 or max(abs(r - R_) for r in rs) > max(0.02, 1e-3 * R_):
        return None
    angs = sorted(math.atan2(p[1] - cy_, p[0] - cx_) for p in pts)
    gaps = [angs[i + 1] - angs[i] for i in range(len(angs) - 1)] + [angs[0] + 2 * math.pi - angs[-1]]
    step = 2 * math.pi / len(pts)
    if max(abs(x - step) for x in gaps) > math.radians(0.5):
        return None
    return (cx_, cy_), 2 * R_


HOLE_GROUPS = hole_groups()
for g_ in HOLE_GROUPS:
    bc_ = bolt_circle(g_)
    if bc_:
        g_["pcd"] = round(bc_[1], 4)
        g_["text"] += "  " + DIMS["diameter"] + fmt(bc_[1]) + DIMS.get("equally_spaced", "均布")
HOLE_IDS = {g["id"] for g in HOLE_GROUPS}  # 孔标注尺寸对象的名字 = 孔组编号(导出时据此改成水平文字 + 引线)


def datum_faces(vname):
    """坐标尺寸的基准(视图左下角)是不是实际要素: 视图横向最小处有法向 ∥ 横轴的平面, 竖向最小处有法向 ∥ 竖轴的平面。"""
    bw_, bh_, _ = VAX[vname]
    lo_w = min(V(x, y, z).dot(bw_) for x in (bb.XMin, bb.XMax) for y in (bb.YMin, bb.YMax) for z in (bb.ZMin, bb.ZMax))
    lo_h = min(V(x, y, z).dot(bh_) for x in (bb.XMin, bb.XMax) for y in (bb.YMin, bb.YMax) for z in (bb.ZMin, bb.ZMax))
    got_w = got_h = False
    for f in shape.Faces:
        try:
            if not isinstance(f.Surface, Part.Plane):
                continue
            n = f.normalAt(0, 0)
            c = f.CenterOfMass
        except Exception:
            continue
        if abs(abs(n.dot(bw_)) - 1) < 1e-6 and abs(c.dot(bw_) - lo_w) < 1e-3:
            got_w = True
        if abs(abs(n.dot(bh_)) - 1) < 1e-6 and abs(c.dot(bh_) - lo_h) < 1e-3:
            got_h = True
    return got_w and got_h


def ordinate_groups():
    """方案 ordinate: true / 省略 = 主视里全部孔组; false = 不标孔位; {"groups": ["H1", …]} 只标这些;
    {"exclude": [...]} 除去这些。返回要标孔位的孔组。"""
    o = spec.get("ordinate", True)
    front = [g for g in HOLE_GROUPS if g["view"] == "Front"]
    if o is False:
        return []
    # 用户 2026-11-19: 圆形件不用坐标尺寸(孔按直径 / 节圆标); 坐标基准 0 必须是实际要素 ——
    # 主视左边、下边都得有平面(加工 / 测量的基准面), 否则基准是悬空的包围盒角点, 不能用
    if o is True or o is None:
        if outer_round("Front"):
            rep["ordinate_skipped"] = "round part: holes are dimensioned by diameter / pitch circle"
            return []
        if not datum_faces("Front"):
            rep["ordinate_skipped"] = "no flat datum faces at the left / bottom of the front view"
            return []
    if isinstance(o, dict):
        if o.get("groups") is not None:
            return [g for g in front if g["id"] in o["groups"]]
        if o.get("exclude"):
            return [g for g in front if g["id"] not in o["exclude"]]
    # 默认: 均布在节圆上的孔组不标坐标(已在孔标注里写 "⌀PCD 均布"); 明确点名的照点名
    return [g for g in front if not g.get("pcd")]


def group_positions(g):
    """孔组里各孔在其视图里的坐标(基准 = 视图左下角, 文档单位), 写进报告供挑选。"""
    bw_, bh_, _ = VAX[g["view"]]
    hw, hh_ = ext(bw_) / 2, ext(bh_) / 2
    return [[fmt((V(*h["at"]) - bb.Center).dot(bw_) + hw), fmt((V(*h["at"]) - bb.Center).dot(bh_) + hh_)] for h in g["holes"]]


# ---------------------------------------------------------------- 圆形件: 视图轮廓是整圆 → 标直径, 不标长 × 宽
def outer_round(vname):
    """视图方向上最大的外圆柱面(凸, 轴线 ∥ 视线), 且直径 ≈ 视图外形(≥ 98%)→ 返回半径, 否则 None。"""
    _, _, d = VAX[vname]
    best = None
    for f in shape.Faces:
        cy = geomutil.cylinder_of(f)
        if cy is None or cy["internal"] or abs(abs(cy["ax"].dot(d)) - 1) > 1e-6:
            continue
        if best is None or cy["r"] > best:
            best = cy["r"]
    bw_, bh_, _ = VAX[vname]
    if best and 2 * best >= 0.98 * max(ext(bw_), ext(bh_)):
        return best
    return None


def shaft_tol(radius):
    return next((t for d, t in SHAFT_TOL.items() if abs(d - 2 * radius) < 0.05), None)


def outer_cyl_views(d):
    """直径 d 的外圆柱面在哪个视图里是圆(轴线 ∥ 视线)。"""
    out = []
    for f in shape.Faces:
        cy = geomutil.cylinder_of(f)
        if cy is None or cy["internal"] or abs(2 * cy["r"] - d) > 0.05:
            continue
        ax = cy["ax"]
        for vn, (_, _, vd) in VAX.items():
            if abs(abs(ax.dot(vd)) - 1) < 1e-6 and vn not in out:
                out.append(vn)
    return out


# 方案 dims: 面板里对单个尺寸的修正(按尺寸对象名: LenW / LenH / LenD / DiaOuter / DiaFit1 / 孔组 H1 …)
#   {"hide": ["LenD"], "text": {"H2": "4 x ⌀6.6 贯穿"}, "key": ["LenW"]}   key = 关键尺寸(前面加 *)
DIM_EDIT = spec.get("dims") or {}
DIM_HIDE = set(DIM_EDIT.get("hide") or [])
DIM_TEXT = dict(DIM_EDIT.get("text") or {})
DIM_KEY = set(DIM_EDIT.get("key") or [])
# tol: {"LenW": "h7" | "±0.05" | "+0.1/-0.05"}  公差(ISO 代号按 GB/T 1800 查偏差; 其它原样跟在数字后)
DIM_TOL = {k: str(v).strip() for k, v in (DIM_EDIT.get("tol") or {}).items() if str(v).strip()}


def tol_suffix(value, tol):
    """公差文字: "h7" → " h7(0/-0.03)"; "±0.05" → "±0.05"; "+0.1/-0.05" → " +0.1/-0.05"。代号查不到(超范围)→ 原样。"""
    if iso286.CODE.match(tol):
        try:
            return " " + iso286.tol_text(value, tol)
        except Exception as e:
            rep["warnings"].append("tolerance %s on %s: %s" % (tol, fmt(value), e))
            return " " + tol
    return tol if tol.startswith("±") else " " + tol


# 锥面(用户 2026-11-19: 锥孔没有标锥度): 解析圆锥面 → 锥度 C = 2·tan(半锥角), 标 "1:x"(符号尖朝小端)
def cone_features():
    out = []
    for f in shape.Faces:
        s_ = f.Surface
        if not isinstance(s_, Part.Cone):
            continue
        try:
            ha = abs(s_.SemiAngle)
            ax = V(s_.Axis).normalize()
            apex = V(s_.Apex)
            u0, u1, v0, v1 = f.ParameterRange
            mid = f.valueAt((u0 + u1) / 2, (v0 + v1) / 2)
        except Exception:
            continue
        # 锥度只标"锥"(半锥角 < 15°, 如 1:10 定心锥); 45° 左右的是倒角 / 沉孔口, 另按倒角标
        if ha < math.radians(0.2) or ha > math.radians(15):
            continue
        C = 2 * math.tan(ha)
        x = 1 / C
        txt = "1:%d" % round(x) if abs(x - round(x)) < 0.05 * max(1.0, x / 10) else "1:%s" % fmt(x, 1)
        # 同一个锥常被存成两个半面(接缝): 按 锥顶 + 轴线 + 锥度 去重
        if any(abs(o["ratio"] - C) < 1e-4 and (o["apex"] - apex).Length < 1e-3 and abs(abs(o["axis"].dot(ax)) - 1) < 1e-6 for o in out):
            continue
        rad = (mid - apex) - ax * (mid - apex).dot(ax)   # 轴线 → 面上点的径向
        out.append({"ratio": C, "half": ha, "axis": ax, "apex": apex, "mid": mid, "text": txt,
                    "internal": rad.dot(f.normalAt((u0 + u1) / 2, (v0 + v1) / 2)) < 0})   # 法向朝轴线 = 内锥(锥孔)
    return out


CONES = cone_features()


def cone_view(c):
    """锥面在哪个视图里显示成斜线(轴 ⟂ 视线): 有剖视且剖切面含轴线就用剖视(内锥要剖开才看得见), 否则主 → 俯 → 左。"""
    if isinstance(sec_spec, dict) and abs(c["axis"].dot(V(*sec_spec["normal"]))) < 1e-6:
        return "Section"
    for n in VIEWS3:
        if abs(c["axis"].dot(VAX[n][2])) < 1e-6:
            return n
    return None


rep["cones"] = [{"taper": c["text"], "half_angle_deg": round(math.degrees(c["half"]), 4), "internal": c["internal"], "view": cone_view(c)} for c in CONES]

# ---------------------------------------------------------------- 基准 / 形位公差 / 粗糙度(方案给内容, 这里定位 + 自洽检查; 布局层定位置)
# 方案:
#   "datums":    [{"label": "A", "on": 选择器}]
#   "gdt":       [{"symbol": "circular_runout", "value": 0.005, "datums": ["A"], "on": 选择器, "dia": false}]
#   "roughness": [{"ra": 0.4, "on": 选择器}]          (其余 = 右上角, 规范 roughness_default)
# 选择器(按几何, agent 从 model_state / model_measure 拿数): {"cyl": 20} 外圆 ⌀20 | {"bore": 8} 内孔 ⌀8 | {"cone": true} / {"cone": "1:10"}
#   | {"shoulder": {"d": 29.5, "side": "start" | "end"}} 回转件 ⌀29.5 段起 / 止端的台阶面 | {"plane": {"normal": [x,y,z], "at": [x,y,z]}}
GDT_SYM = {"straightness": "⏤", "flatness": "⏥", "roundness": "○", "cylindricity": "⌭", "profile_line": "⌒", "profile_surface": "⌓",
           "parallelism": "∥", "perpendicularity": "⊥", "angularity": "∠", "position": "⌖", "coaxiality": "◎", "concentricity": "◎",
           "symmetry": "⌯", "circular_runout": "↗", "total_runout": "⌰"}
GDT_NO_DATUM = {"straightness", "flatness", "roundness", "cylindricity"}
GDT_AXIAL = {"coaxiality", "concentricity"}          # 被测 / 基准都是轴线
GDT_RUNOUT = {"circular_runout", "total_runout"}


def resolve_sel(sel):
    """选择器 → {kind: cyl | bore | cone | plane, face, ax, c, r, n, p} 或抛错(说清找不到什么)。"""
    if not isinstance(sel, dict):
        raise ValueError("selector must be an object, got %r" % (sel,))
    if "cyl" in sel or "bore" in sel:
        d_ = float(sel.get("cyl", sel.get("bore")))
        want_int = "bore" in sel
        for f in shape.Faces:
            cy = geomutil.cylinder_of(f)
            if cy and cy["internal"] == want_int and abs(2 * cy["r"] - d_) < 0.01:
                return {"kind": "bore" if want_int else "cyl", "face": f, "ax": V(cy["ax"]), "c": V(cy["c"]), "r": cy["r"], "t0": cy["t0"], "t1": cy["t1"]}
        raise ValueError("no %s of diameter %s" % ("bore" if want_int else "outer cylinder", fmt(d_)))
    if "cone" in sel:
        for cn in CONES:
            if sel["cone"] is True or str(sel["cone"]) == cn["text"]:
                return {"kind": "cone", "ax": cn["axis"], "c": cn["apex"], "mid": cn["mid"], "cone": cn}
        raise ValueError("no taper cone %s" % sel["cone"])
    if "shoulder" in sel:
        if not TURN:
            raise ValueError("shoulder selector needs a turned part")
        s_ = sel["shoulder"]
        d_ = float(s_["d"])
        st_ = [x for x in TURN["steps"] if abs(x["d"] - d_) < 0.01]
        if not st_:
            raise ValueError("no step of diameter %s" % fmt(d_))
        t_ = min(x["t0"] for x in st_) if s_.get("side", "start") == "start" else max(x["t1"] for x in st_)
        best = None
        for f in shape.Faces:
            if not isinstance(f.Surface, Part.Plane):
                continue
            n = f.normalAt(0, 0)
            if abs(abs(n.dot(TURN["axs"])) - 1) > 1e-6:
                continue
            tf = (f.CenterOfMass - TURN["c"]).dot(TURN["axs"])
            if abs(tf - t_) < 1.5 and (best is None or abs(tf - t_) < best[0]):   # 倒角会让台阶面离段端点差一点
                best = (abs(tf - t_), f, tf)
        if not best:
            raise ValueError("no shoulder face at the %s of the %s step" % (s_.get("side", "start"), fmt(d_)))
        f = best[1]
        rs = [((vx.Point - TURN["c"]) - TURN["axs"] * (vx.Point - TURN["c"]).dot(TURN["axs"])).Length for vx in f.Vertexes] or [d_ / 2]
        return {"kind": "plane", "face": f, "n": V(TURN["axs"]), "p": TURN["c"] + TURN["axs"] * best[2], "rmin": min(rs), "rmax": max(rs)}
    if "plane" in sel:
        n_ = V(*sel["plane"]["normal"]).normalize()
        p_ = V(*sel["plane"]["at"])
        for f in shape.Faces:
            if isinstance(f.Surface, Part.Plane) and abs(abs(f.normalAt(0, 0).dot(n_)) - 1) < 1e-6 and abs((f.CenterOfMass - p_).dot(n_)) < 0.01:
                return {"kind": "plane", "face": f, "n": n_, "p": f.CenterOfMass}
        raise ValueError("no planar face with that normal through that point")
    raise ValueError("unknown selector %r (use cyl / bore / cone / shoulder / plane)" % (sel,))


def feature_anchor(ft):
    """被测 / 基准要素在主视里的锚点(局部坐标): 圆柱 / 锥 = 上轮廓线(面本身), 平面 = 边视的那条线上。只支持在主视里看得到边的要素。"""
    bw_, bh_, bd_ = VAX["Front"]
    if ft["kind"] in ("cyl", "bore"):
        if abs(ft["ax"].dot(bd_)) > 1e-6:
            return None
        tm = (ft["t0"] + ft["t1"]) / 2
        up = bh_ if abs(ft["ax"].dot(bh_)) < 0.5 else bw_
        return local_of("Front", ft["c"] + ft["ax"] * tm + up * ft["r"])
    if ft["kind"] == "cone":
        if abs(ft["ax"].dot(bd_)) > 1e-6:
            return None
        return local_of("Front", ft["mid"]) if abs(local_of("Front", ft["mid"])[1]) > 0 else None
    if ft["kind"] == "plane":
        if abs(ft["n"].dot(bd_)) > 1e-6:
            return None
        if "rmax" in ft:
            up = bh_ if abs(ft["n"].dot(bh_)) < 0.5 else bw_
            return local_of("Front", ft["p"] + up * ((ft["rmin"] + ft["rmax"]) / 2))
        return local_of("Front", ft["p"])
    return None


def feature_rowgeom(ft):
    """回转件主视里的标注行几何(局部坐标): {anchors: {top, bottom}, slide?, approach, axis_d?} 或 None(不是同轴要素 → 用自由引出)。
    圆柱: 上 / 下轮廓线, 可沿段滑动; 锥: 两侧轮廓线中点; 台阶面(边视的平面): 外沿的延长线上, 水平进去。"""
    if not TURN:
        return None
    bw_, bh_, _ = VAX["Front"]
    axs = TURN["axs"]
    yc = (TURN["c"] - bb.Center).dot(bh_)
    xt = lambda t: (TURN["c"] + axs * t - bb.Center).dot(bw_)
    if ft["kind"] in ("cyl", "bore", "cone") and abs(abs(ft["ax"].dot(axs)) - 1) > 1e-6:
        return None
    if ft["kind"] in ("cyl", "bore"):
        d_ = (ft["c"] - TURN["c"])
        if (d_ - axs * d_.dot(axs)).Length > 1e-3:
            return None
        ts = [(vx.Point - TURN["c"]).dot(axs) for vx in ft["face"].Vertexes] or [0.0]
        x0, x1 = sorted((xt(min(ts)), xt(max(ts))))
        m = min(1.5, (x1 - x0) / 4)
        r = ft["r"]
        mid = (x0 + x1) / 2
        return {"anchors": {"top": [mid, yc + r], "bottom": [mid, yc - r]}, "slide": [x0 + m, x1 - m], "approach": "v", "d": round(2 * r, 3)}
    if ft["kind"] == "cone":
        mx, my = local_of("Front", ft["mid"])
        return {"anchors": {"top": [mx, yc + abs(my - yc)], "bottom": [mx, yc - abs(my - yc)]}, "approach": "v"}
    if ft["kind"] == "plane" and "rmax" in ft:
        x = local_of("Front", ft["p"])[0]
        return {"anchors": {"top": [x, yc + ft["rmax"]], "bottom": [x, yc - ft["rmax"]]}, "approach": "h", "ext_out": 3.0}
    return None


def rowify(item, ft, axis=False, top_only=False):
    """回转件: 标签进标注行(带两侧锚点 / 滑动范围 / 进入方式); axis = 指轴线(挂在该直径的尺寸线上); top_only = 只许在上侧(粗糙度符号)。"""
    g = feature_rowgeom(ft)
    if not g:
        return item
    item.update({"row": True, "anchors": ({"top": g["anchors"]["top"]} if top_only else g["anchors"]), "approach": g["approach"]})
    for k in ("slide", "ext_out"):
        if k in g:
            item[k] = g[k]
    if "d" in g:
        item["axis_d" if axis else "avoid_d"] = g["d"]
    return item


def gdt_items():
    """方案里的基准 / 形位 / 粗糙度 → 布局标签 + 自洽检查。违规的不画, 写进报告(用户 2026-11-19: 基准与被测要素要说得通, 引线指到被测要素本身)。"""
    out, issues = [], []
    dmap = {}
    for i_, d_ in enumerate(spec.get("datums") or []):
        try:
            ft = resolve_sel(d_["on"])
        except Exception as e:
            issues.append("datum %s: %s" % (d_.get("label"), e))
            continue
        dmap[str(d_["label"])] = ft
        a_ = feature_anchor(ft)
        if a_ is None:
            issues.append("datum %s: not visible as an edge in the front view" % d_["label"])
            continue
        out.append(rowify({"id": "DAT%d" % (i_ + 1), "view": "Front", "anchor": a_, "text": str(d_["label"]), "w": 6.0, "h": 6.0, "prio": 3,
                           "kind": "datum", "axis": ft["kind"] in ("cyl", "bore", "cone")}, ft, axis=ft["kind"] in ("cyl", "bore")))
    for i_, g_ in enumerate(spec.get("gdt") or []):
        sym = str(g_.get("symbol", ""))
        tag = "gdt %d (%s)" % (i_ + 1, sym)
        if sym not in GDT_SYM:
            issues.append("%s: unknown symbol (use %s)" % (tag, ", ".join(sorted(GDT_SYM))))
            continue
        ds = [str(x) for x in (g_.get("datums") or [])]
        if sym in GDT_NO_DATUM and ds:
            issues.append("%s: form tolerances take no datum" % tag)
            continue
        if sym not in GDT_NO_DATUM and not ds:
            issues.append("%s: needs a datum" % tag)
            continue
        miss = [x for x in ds if x not in dmap]
        if miss:
            issues.append("%s: datum %s not defined (add it to datums)" % (tag, ", ".join(miss)))
            continue
        try:
            ft = resolve_sel(g_["on"])
        except Exception as e:
            issues.append("%s: %s" % (tag, e))
            continue
        if sym in GDT_RUNOUT or sym in GDT_AXIAL:
            dax = dmap[ds[0]]
            if dax["kind"] not in ("cyl", "bore", "cone"):
                issues.append("%s: runout / coaxiality need an axis datum (a cylinder or cone), %s is a plane" % (tag, ds[0]))
                continue
            if ft["kind"] in ("cyl", "bore", "cone"):
                if abs(abs(ft["ax"].dot(dax["ax"])) - 1) > 1e-6:
                    issues.append("%s: the feature is not coaxial with datum %s" % (tag, ds[0]))
                    continue
            elif ft["kind"] == "plane":
                if sym in GDT_AXIAL or abs(abs(ft["n"].dot(dax["ax"])) - 1) > 1e-6:
                    issues.append("%s: a plane can only take axial runout, and must be perpendicular to datum axis %s" % (tag, ds[0]))
                    continue
        if sym in ("perpendicularity", "parallelism") and ft["kind"] == "plane" and dmap[ds[0]]["kind"] == "plane":
            c_ = abs(ft["n"].dot(dmap[ds[0]]["n"]))
            if (sym == "parallelism" and c_ < 1 - 1e-6) or (sym == "perpendicularity" and c_ > 1e-6):
                issues.append("%s: the plane is not %s to datum %s in the model" % (tag, "parallel" if sym == "parallelism" else "perpendicular", ds[0]))
                continue
        a_ = feature_anchor(ft)
        if a_ is None:
            issues.append("%s: feature not visible as an edge in the front view" % tag)
            continue
        val = ("⌀" if g_.get("dia") or sym in GDT_AXIAL else "") + fmt(float(g_["value"]), 4)   # 公差值到微米(0.003), 不按尺寸的 2 位
        cells = [GDT_SYM[sym], val] + ds
        T_ = LAYM.Text(std)
        ws = [max(6.0, T_.width(c_) + 2.0) for c_ in cells]
        out.append(rowify({"id": "GDT%d" % (i_ + 1), "view": "Front", "anchor": a_, "text": " ".join(cells), "w": sum(ws), "h": 6.0, "prio": 2,
                           "kind": "gdt", "cells": cells, "cell_w": ws}, ft, axis=sym in GDT_AXIAL))
    for i_, r_ in enumerate(spec.get("roughness") or []):
        try:
            ft = resolve_sel(r_["on"])
        except Exception as e:
            issues.append("roughness %d: %s" % (i_ + 1, e))
            continue
        a_ = feature_anchor(ft)
        if a_ is None:
            issues.append("roughness %d: surface not visible as an edge in the front view" % (i_ + 1))
            continue
        txt = "Ra %s" % fmt(float(r_["ra"]))
        # 符号高 7 mm(长边), 框高按符号算, 否则符号伸出框外压到别的标注
        out.append(rowify({"id": "RA%d" % (i_ + 1), "view": "Front", "anchor": a_, "text": txt, "pad_left": 7.0, "h": 7.5, "prio": 1, "kind": "roughness"},
                          ft, top_only=True))
    rep["gdt"] = {"items": [{"id": o["id"], "text": o["text"]} for o in out], "issues": issues}
    for s_ in issues:
        rep["warnings"].append("annotation skipped: " + s_)
    return out


ORD_GROUPS = ordinate_groups()
rep["hole_groups"] = [{"id": g["id"], "view": g["view"], "text": g["text"], "count": len(g["holes"]),
                       "positions": group_positions(g), "ordinate": g in ORD_GROUPS} for g in HOLE_GROUPS]


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


# ---------------------------------------------------------------- 布局层(D9a, layout.py): 比例 / 视图位置 / 外形尺寸 / 引出标注位置
# 布局层用 y 向下的页面毫米, TechDraw 用 y 向上: y_dn = H − y_up。方案 "layout": false = 用旧的固定边距排法。
import layout as LAYM  # noqa: E402


def shown_text(name, typ, text=None, value=None):
    """add_dim 最后显示的文字(同一套规则: 方案改字 > 公差 > 原文; 关键尺寸加 *)—— 布局要按真实文字宽度排。"""
    if name in DIM_TEXT and str(DIM_TEXT[name]).strip():
        t = str(DIM_TEXT[name]).strip()
    else:
        pre = {"Diameter": DIMS["diameter"], "Radius": "R"}.get(typ, "")
        t = text if text is not None else pre + fmt(value)
        if name in DIM_TOL and value is not None and name not in DIM_TEXT:
            t = t + tol_suffix(value, DIM_TOL[name])
    if name in DIM_KEY and not t.startswith(DIMS["key_mark"]):
        t = DIMS["key_mark"] + t
    return t


def ord_reserve(vname, hole_list):
    """坐标尺寸(ordinates)在视图左 / 上要占的深度(同 ordinates() 的画法)。"""
    bw_, bh_, _ = VAX[vname]
    hw, hh_ = ext(bw_) / 2, ext(bh_) / 2
    pts = [((V(*h["at"]) - bb.Center).dot(bw_), (V(*h["at"]) - bb.Center).dot(bh_)) for h in hole_list]
    if not pts:
        return None
    tw = LAYM.Text(std).width
    wy = max(tw(fmt(y + hh_)) for y in [-hh_, hh_] + [p[1] for p in pts])
    wx = max(tw(fmt(x + hw)) for x in [-hw, hw] + [p[0] for p in pts])
    o_ = std.get("ordinate", {"tick": 5.0, "gap": 1.0})
    base = o_["gap"] + o_["tick"] + 4.0 + 1.0 + 2.0
    return {"left": base + wy, "top": base + wx}


def view_axes(vname):
    """视图的(横, 竖)模型轴; 剖视 = 基视图的横轴 + 基视图的视线方向(剖切后从剖切面法向看)。"""
    if vname == "Section" and isinstance(sec_spec, dict):
        bw_, _, bd_ = VAX[sec_spec.get("base", "Front")]
        return bw_, bd_
    return VAX[vname][0], VAX[vname][1]


def local_of(vname, pt):
    """模型点 → 视图局部坐标(模型毫米, 以包围盒中心为原点, x 右 y 上)。符号在建视图前只是估计, 第二遍用视图里的边。"""
    bw_, bh_ = view_axes(vname)
    return [(pt - bb.Center).dot(bw_), (pt - bb.Center).dot(bh_)]


def taper_text(c, vname):
    """锥度标注文字: 符号尖朝小端(GB/T 15754), 方向按小端在视图里朝左还是朝右。"""
    bw_, _ = view_axes(vname)
    left = (c["apex"] - c["mid"]).dot(bw_) < 0
    return ("◁ " if left else "▷ ") + c["text"]


GDT_LABELS = gdt_items()   # 要 local_of / LAYM, 所以在这里算
GDT_BY_ID = {o["id"]: o for o in GDT_LABELS}


def layout_problem(anchors=None, fixed_scale=None, sec_dir=None):
    if sec_dir is None:
        sec_dir = ("Front", "below") if TURN else ("Top", "right")
    """anchors = {标签 id: (视图, [x, y])} 第二遍用视图里实际的圆边位置覆盖估计值。"""
    anchors = anchors or {}
    area = [AREA[0], H - AREA[3], AREA[2], H - AREA[1]]
    keep = []
    rg = std.get("roughness")
    if rg:
        keep.append([rg["x"] - 30, rg["y"] - 13, inner_r, rg["y"] + 5])   # 右上角"其余 ∇6.3"(符号 + Ra 值伸到 y − 11.5)
    views_ = [{"id": "Front", "w": Wm, "h": Hm}] + ([] if TURN else [{"id": "Top", "w": Wm, "h": Dm, "rel": {"of": "Front", "dir": "below"}}]) + [
              {"id": "Left", "w": Dm, "h": Hm, "rel": {"of": "Front", "dir": "right"}}]
    reserve = []
    if isinstance(sec_spec, dict):
        bw_, bh_, bd_ = VAX[sec_spec.get("base", "Front")]
        views_.append({"id": "Section", "w": ext(bw_), "h": ext(bd_), "rel": {"of": sec_dir[0], "dir": sec_dir[1]}})
        reserve.append({"view": "Section", "side": "top", "depth": 10.0})          # 剖视名 "A-A"
        for sd in ("left", "right"):
            reserve.append({"view": sec_spec.get("base", "Front"), "side": sd, "depth": 7.0})   # 剖切符号的箭头
    diag_ = math.sqrt(bb.XLength ** 2 + bb.YLength ** 2 + bb.ZLength ** 2)
    views_.append({"id": "Iso", "w": diag_, "h": diag_ / 1.2, "free": True, "max_rel": 0.75})
    dims_, labels_ = [], []
    if ORD_GROUPS:
        rs = ord_reserve("Front", [h for g in ORD_GROUPS for h in g["holes"]])
        if rs:
            reserve += [{"view": "Front", "side": k, "depth": v} for k, v in rs.items()]
    fr_ = outer_round("Front")
    dstep_ids = {}   # 段直径 → 它的 ⌀ 尺寸 id(轴线基准 / 同轴度挂在这条尺寸线上; 表面要求与它错开)
    if TURN:
        # 回转件(主视轴线水平): 每段外圆标 ⌀(在视图上, 段内滑动避让) + 轴向链式尺寸(主视下方); 总长仍在上方(极值点)
        yc = (TURN["c"] - bb.Center).dot(VAX["Front"][1])
        xt = lambda t: (TURN["c"] + TURN["axs"] * t - bb.Center).dot(VAX["Front"][0])
        segs = []
        for s_ in TURN["steps"]:
            L_ = s_["t1"] - s_["t0"]
            if L_ < 1.0:      # 倒角 / 退刀槽边的窄环面不单标
                continue
            if any(abs(o["d"] - s_["d"]) < 1e-3 and abs(o["t0"] - s_["t0"]) < 1e-3 for o in segs):
                continue
            segs.append(s_)
        for i_, s_ in enumerate(segs):
            x0_, x1_ = sorted((xt(s_["t0"]), xt(s_["t1"])))
            id_ = "Dstep%d" % (i_ + 1)
            dstep_ids.setdefault(round(s_["d"], 3), id_)
            tol_ = shaft_tol(s_["d"] / 2)
            dims_.append({"id": id_, "view": "Front", "kind": "inline", "dir": "y", "span": [yc - s_["d"] / 2, yc + s_["d"] / 2],
                          "slide": [x0_ + min(0.8, (x1_ - x0_) / 4), x1_ - min(0.8, (x1_ - x0_) / 4)],
                          "text": shown_text(id_, "Diameter", DIMS["diameter"] + fmt(s_["d"]) + (" " + tol_ if tol_ else ""), s_["d"])})
        if len(segs) > 1:
            for i_, s_ in enumerate(sorted(segs, key=lambda s: min(xt(s["t0"]), xt(s["t1"])))):
                x0_, x1_ = sorted((xt(s_["t0"]), xt(s_["t1"])))
                id_ = "Lstep%d" % (i_ + 1)
                dims_.append({"id": id_, "view": "Front", "dir": "x", "side": "bottom", "a": [x0_, yc - s_["d"] / 2], "b": [x1_, yc - s_["d"] / 2],
                              "text": shown_text(id_, "DistanceX", value=x1_ - x0_)})
        dims_.append({"id": "LenW", "view": "Front", "dir": "x", "side": "top", "a": [-Wm / 2, Hm / 2], "b": [Wm / 2, Hm / 2], "text": shown_text("LenW", "DistanceX", value=Wm)})
    elif not fr_ and not ORD_GROUPS:
        dims_.append({"id": "LenW", "view": "Front", "dir": "x", "side": "top", "a": [-Wm / 2, Hm / 2], "b": [Wm / 2, Hm / 2], "text": shown_text("LenW", "DistanceX", value=Wm)})
        dims_.append({"id": "LenH", "view": "Front", "dir": "y", "side": "left", "a": [-Wm / 2, -Hm / 2], "b": [-Wm / 2, Hm / 2], "text": shown_text("LenH", "DistanceY", value=Hm)})
    if not TURN:
        dims_.append({"id": "LenD", "view": "Left", "dir": "x", "side": "top", "a": [-Dm / 2, Hm / 2], "b": [Dm / 2, Hm / 2], "text": shown_text("LenD", "DistanceX", value=Dm)})
    k45 = math.sqrt(0.5)

    def lab(id_, vname, center, r, text):
        a_ = anchors.get(id_, (vname, [center[0] + r * k45, center[1] + r * k45]))
        labels_.append({"id": id_, "view": a_[0], "anchor": a_[1], "text": text, "prio": 2})

    if fr_:
        tol_ = shaft_tol(fr_)
        lab("DiaOuter", "Front", [0, 0], fr_, shown_text("DiaOuter", "Diameter", DIMS["diameter"] + fmt(2 * fr_) + (" " + tol_ if tol_ else ""), 2 * fr_))
    for i_, (d_, _) in enumerate([] if TURN else sorted(SHAFT_TOL.items())):   # 回转件: 配合已在段直径上
        if fr_ and abs(2 * fr_ - d_) < 0.05:
            continue
        vs_ = outer_cyl_views(d_)
        if vs_:
            tol_ = shaft_tol(d_ / 2)
            lab("DiaFit%d" % (i_ + 1), vs_[0], [0, 0], d_ / 2, shown_text("DiaFit%d" % (i_ + 1), "Diameter", DIMS["diameter"] + fmt(d_) + (" " + tol_ if tol_ else ""), d_))
    for g in HOLE_GROUPS:
        # 估计锚点: 组里离视图边最近的孔(出路最短)
        bw_, bh_, _ = VAX[g["view"]]
        hw, hh_ = ext(bw_) / 2, ext(bh_) / 2
        pts = [(local_of(g["view"], V(*h["at"])), h) for h in g["holes"]]
        (c_, h_) = min(pts, key=lambda ph: min(hw - abs(ph[0][0]), hh_ - abs(ph[0][1])))
        lab(g["id"], g["view"], c_, h_["d"] / 2, shown_text(g["id"], "Diameter", g["text"]))
    for i_, cn in enumerate(CONES):
        vn = cone_view(cn)
        if vn is None or (vn == "Section" and not isinstance(sec_spec, dict)):
            vn = next((n for n in VIEWS3 if abs(cn["axis"].dot(VAX[n][2])) < 1e-6), None)
        if vn is None:
            continue
        id_ = "Taper%d" % (i_ + 1)
        a_ = anchors.get(id_, (vn, local_of(vn, cn["mid"])))
        labels_.append({"id": id_, "view": a_[0], "anchor": a_[1], "text": taper_text(cn, a_[0]), "prio": 2})
    # 基准 / 形位公差 / 粗糙度(锚点都在要素本身上)
    for o_ in GDT_LABELS:
        lb_ = {k: o_[k] for k in ("id", "view", "anchor", "text", "prio", "w", "h", "pad_left", "row", "anchors", "slide", "approach", "ext_out") if k in o_}
        if o_.get("axis_d") is not None and o_["axis_d"] in dstep_ids:
            lb_["attach_dim"] = dstep_ids[o_["axis_d"]]
        if o_.get("avoid_d") is not None and o_["avoid_d"] in dstep_ids:
            lb_["avoid_dim"] = dstep_ids[o_["avoid_d"]]
        labels_.append(lb_)
    # 回转件的实际轮廓(布局层据此判断"引线在零件里走了多少"、标签能不能放进包围盒里空着的地方):
    # 主视 / 剖视 = 各段矩形 + 段间缝隙(倒角 / 锥)按相邻较小直径补上; 左视(端面圆)= 4 块矩形近似
    if TURN:
        yc_ = (TURN["c"] - bb.Center).dot(VAX["Front"][1])
        xt_ = lambda t: (TURN["c"] + TURN["axs"] * t - bb.Center).dot(VAX["Front"][0])
        ss_ = sorted(((min(xt_(s["t0"]), xt_(s["t1"])), max(xt_(s["t0"]), xt_(s["t1"])), s["d"]) for s in TURN["steps"]), key=lambda s: s[0])
        parts_ = [[a, yc_ - d / 2, b, yc_ + d / 2] for a, b, d in ss_]
        for (a0, b0, d0), (a1, b1, d1) in zip(ss_, ss_[1:]):
            if a1 > b0 + 1e-6:
                parts_.append([b0, yc_ - min(d0, d1) / 2, a1, yc_ + min(d0, d1) / 2])
        lo_, hi_ = -Wm / 2, Wm / 2
        if ss_ and ss_[0][0] > lo_ + 1e-6:
            parts_.append([lo_, yc_ - ss_[0][2] / 2, ss_[0][0], yc_ + ss_[0][2] / 2])
        if ss_ and ss_[-1][1] < hi_ - 1e-6:
            parts_.append([ss_[-1][1], yc_ - ss_[-1][2] / 2, hi_, yc_ + ss_[-1][2] / 2])
        rr_ = TURN["rmax"]
        circ_parts = [[-rr_ * k1, -rr_ * k2, rr_ * k1, rr_ * k2] for k1, k2 in ((1.0, 0.42), (0.88, 0.68), (0.68, 0.88), (0.42, 1.0))]
        for v_ in views_:
            if v_["id"] in ("Front",) or (v_["id"] == "Section" and isinstance(sec_spec, dict) and sec_spec.get("base") == "Front"):
                v_["parts"] = parts_
            elif v_["id"] == "Left":
                v_["parts"] = circ_parts
    p_ = {"paper": [W, H], "area": area, "keepout": keep, "views": views_, "dims": dims_, "labels": labels_, "reserve": reserve}
    if fixed_scale:
        p_["scale"] = fixed_scale
    elif spec.get("scale", "auto") != "auto":
        p_["scale"] = float(spec["scale"])
    return p_


LAY = None
if spec.get("layout", True) is not False:
    try:
        # 剖视的位置: 俯视右边 / 下面, 或紧挨它的基视图右边 / 下面, 都试, 取比例大的(同比例取靠前的)
        b_ = sec_spec.get("base", "Front") if isinstance(sec_spec, dict) else "Front"
        if TURN:   # 没有俯视: 剖视放主视下面(主视右边是左视), 或左视的右边 / 下面
            cands_ = [("Front", "below"), ("Left", "right"), ("Left", "below")]
        else:
            cands_ = [("Top", "right")]
            if isinstance(sec_spec, dict):
                cands_ += [("Top", "below")] + ([(b_, "right"), (b_, "below")] if b_ != "Top" else [])
        LAY, SEC_DIR = None, cands_[0]
        for sd_ in cands_:
            try:
                r_ = LAYM.solve(layout_problem(sec_dir=sd_), std)
            except ValueError as e_:
                rep["warnings"].append("layout candidate %s: %s" % (sd_, e_))
                continue
            ok_ = r_.get("scale") and not r_["unplaced"]
            if ok_ and (LAY is None or r_["scale"] > LAY["scale"] + 1e-9):
                LAY, SEC_DIR = r_, sd_
        if LAY is None:
            LAY = LAYM.solve(layout_problem(), std)
        # 排查用: 布局输入原样存一份(可脱离 FreeCAD 重放 layout.solve)
        try:
            with open(os.path.join(OUT, BASE + ".layout.json"), "w", encoding="utf-8") as f_:
                json.dump(layout_problem(sec_dir=SEC_DIR), f_, ensure_ascii=False, indent=1)
        except Exception:
            pass
        if not LAY.get("scale"):
            rep["warnings"].append("layout: %s - fell back to fixed margins" % LAY["unplaced"])
            LAY = None
    except Exception as e:
        rep["warnings"].append("layout failed: %s - fell back to fixed margins" % e)
        LAY = None
if LAY:
    scale = LAY["scale"]
    rep["scale"] = scale
    rep["section_at"] = "%s %s" % SEC_DIR
    SEC_AT = SEC_DIR[1]
    rep["layout"] = {"pass1": {"unplaced": LAY["unplaced"], "conflicts": LAY["conflicts"][:10], "ugly": LAY.get("ugly", [])}}


def lay_xy(vname):
    """布局给的视图中心 → TechDraw 页面坐标(y 向上)。"""
    c = LAY["views"][vname]
    return c["cx"], H - c["cy"]


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
for name in VIEWS3:   # 回转件: 俯视与主视一样, 不要
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
if LAY:
    # 布局层定的位置: 关掉自动分布, 子视图 X / Y = 相对主视(组锚点)的偏移
    fx, fy = lay_xy("Front")
    grp.X, grp.Y = fx, fy
    try:
        grp.AutoDistribute = False
    except Exception as e:
        rep["warnings"].append("autodistribute: %s" % e)
    doc.recompute()
    for v_ in grp.Views:
        if v_.Type in ("Top", "Left"):
            x_, y_ = lay_xy(v_.Type)
            v_.X, v_.Y = x_ - fx, y_ - fy
else:
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
    late = []
    for v in vs:
        try:
            if v.getEdgeByIndex(0) is None:
                late.append(v.Name)
        except Exception:
            late.append(v.Name)
    rep["warnings"].append("views not ready after %.0fs: %s" % (timeout, ", ".join(late)))
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


R = {n: vrect(views[n], ext(VAX[n][0]) * scale, ext(VAX[n][1]) * scale) for n in VAX if n in views}

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
    if LAY and "Section" in LAY["views"]:
        sec.X, sec.Y = lay_xy("Section")
    elif "Top" not in R:
        # 没有俯视(回转件)的旧排法: 剖视放主视下面
        sec.X = (R["Front"][0] + R["Front"][2]) / 2
        sec.Y = R["Front"][1] - GAPV - MARGIN - sh_ / 2
    elif SEC_AT == "right":
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
if LAY and "Iso" in LAY["views"]:
    iso_s = LAY["views"]["Iso"]["scale"]
iso.Scale = iso_s
iso.ScaleType = "Custom"
if LAY and "Iso" in LAY["views"]:
    iso.X, iso.Y = lay_xy("Iso")
else:
    iso.X = AREA[2] - diag * iso_s / 2 - 4
    iso.Y = AREA[3] - diag * iso_s / 2.4 - 12
R["Iso"] = vrect(iso, diag * iso_s, diag * iso_s / 1.2)
doc.recompute()
wait_views([iso] + ([sec] if sec else []), timeout=30.0 if sec else 20.0)
# 剖视没出图(实测编码器轴: 等 90 s 也是空的 —— 剖切失败, 不是慢): 删掉剖视和 "A-A", 别在图上留一个空标题
if sec is not None:
    try:
        empty_ = sec.getEdgeByIndex(0) is None
    except Exception:
        empty_ = True
    if empty_:
        rep["warnings"].append("section %s produced no geometry: removed" % sec.Label)
        for o_ in (sec, lab):
            try:
                doc.removeObject(o_.Name)
            except Exception:
                pass
        sec = None
        sec_spec = None
        SEC_REMOVED = True
        R.pop("Section", None)
        R.pop("SectionLabel", None)
        doc.recompute()
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


def add_dim(view, typ, refs, name, page_xy, text=None, fmt_spec=None, value=None):
    info = {"name": name, "type": typ, "view": view.Name, "text": text, "at": [round(page_xy[0], 1), round(page_xy[1], 1)]}
    if value is not None:
        info["value"] = round(value, 4)
    if name in DIM_HIDE:
        info["hidden"] = True
        rep["placed"].append(info)
        return None
    if name in DIM_TEXT and str(DIM_TEXT[name]).strip():
        text = str(DIM_TEXT[name]).strip()
        info["custom"] = True
    elif name in DIM_TOL and value is not None:
        pre = {"Diameter": DIMS["diameter"], "Radius": "R"}.get(typ, "")
        base = text if text is not None else pre + fmt(value)
        text = base + tol_suffix(value, DIM_TOL[name])
        info["tol"] = DIM_TOL[name]
    if name in DIM_KEY:
        info["key"] = True
        if text is not None:
            text = DIMS["key_mark"] + text if not text.startswith(DIMS["key_mark"]) else text
        else:
            fmt_spec = DIMS["key_mark"] + (fmt_spec or FSPEC)
    d = doc.addObject("TechDraw::DrawViewDimension", name)
    d.Type = typ
    d.References2D = [(view, r) for r in refs]
    page.addView(d)
    vx, vy = vcenter(view)
    d.X, d.Y = page_xy[0] - vx, page_xy[1] - vy
    if fmt_spec:
        d.FormatSpec = fmt_spec
    # 不对称公差(用户 2026-11-20: 上下界写成上下角标): 用 TechDraw 自己的上 / 下偏差(叠放显示), 不拼进文字; 对称 ± 照常同号写
    tsp = LAYM.split_tol(text) if text is not None and not info.get("custom") else None
    if tsp and value is not None:
        # 只处理"数值 [代号] (上/下)"这种单纯的尺寸; 孔说明("2 x ⌀8 H7(+0.015/0) 贯穿")等带别的字的不动
        core_ = tsp[0].replace(DIMS["key_mark"], "").replace(DIMS["diameter"], "").lstrip("R")
        if tsp[3].strip() or not core_.startswith(fmt(value)):
            tsp = None
    if tsp and value is not None and typ in ("DistanceX", "DistanceY", "Distance", "Diameter", "Radius"):
        pre, up, lo, post, paren = tsp
        try:
            code_ = pre.split(fmt(value))[-1] if fmt(value) in pre else ""
            lead_ = {"Diameter": DIMS["diameter"], "Radius": "R"}.get(typ, "")
            d.FormatSpec = (DIMS["key_mark"] if info.get("key") else "") + lead_ + FSPEC + (code_ if code_ else "")
            d.EqualTolerance = False
            d.OverTolerance = float(up)
            d.UnderTolerance = float(lo)
            for prop, v_ in (("FormatSpecOverTolerance", "%+.3w"), ("FormatSpecUnderTolerance", "%+.3w")):
                if hasattr(d, prop):
                    setattr(d, prop, v_)
            info["tolerance"] = [up, lo]
            text = None
        except Exception as e:
            rep["warnings"].append("stacked tolerance on %s: %s (kept as text)" % (name, e))
    if text is not None:
        d.Arbitrary = True
        d.FormatSpec = text
    dims_out.append(d)
    info["shown"] = text
    rep["placed"].append(info)
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


def rtol(r):
    """圆半径比较的容差: 解析圆 0.001; B 样条边拟合出来的半径会差一点, 按半径的 0.3% 放宽。"""
    return max(1e-3, 3e-3 * r)


def circle_edges(view):
    """视图里的圆 / 圆弧边: 解析圆, 加上几何上是圆的 B 样条曲线(B 样条孔的投影)—— 包成同样的 .Curve.Radius / .Curve.Center。"""
    from types import SimpleNamespace

    out = []
    for n, ed in edges_of(view):
        if isinstance(ed.Curve, Part.Circle):
            out.append((n, ed))
            continue
        if isinstance(ed.Curve, Part.Line):
            continue
        ci = geomutil.edge_circle(ed)
        if ci:
            out.append((n, SimpleNamespace(Curve=SimpleNamespace(Radius=ci[1], Center=ci[0]))))
    return out


def longest(view, axis):
    """视图里沿 x / y 最长的直线边。"""
    ls = [(n, e) for n, e in edges_of(view) if isinstance(e.Curve, Part.Line)]
    k = 0 if axis == "x" else 1
    return max(ls, key=lambda ne: abs(ne[1].Vertexes[0].Point[k] - ne[1].Vertexes[-1].Point[k]))[0] if ls else None


G, STEP = DIMS["gap"], DIMS["step"]
FSPEC = "%." + str(DIMS["decimals"]) + "w"


def verts_of(view):
    out = []
    i = 0
    while True:
        try:
            p = view.getVertexByIndex(i)
        except Exception:
            break
        if p is None:
            break
        out.append(("Vertex%d" % i, p))
        i += 1
    return out


def extreme_pair(view, axis):
    """外形尺寸的两个端点: 视图里沿 axis 最靠两端的顶点(含倒角 —— 用户 2026-11-19: 厚度没把倒角含进去; 轴的总长挂在 Ø18 段上)。
    同一端有好几个顶点时取离视图中线最近的(界线短)。→ [左 / 下端名, 右 / 上端名] 或 None"""
    k = 0 if axis == "x" else 1
    vs = []
    for n, p in verts_of(view):
        pt = getattr(p, "Point", p)
        vs.append((n, pt.x, pt.y))
    if len(vs) < 2:
        return None
    vals = [v[1 + k] for v in vs]
    lo, hi = min(vals), max(vals)
    if hi - lo < 1e-6:
        return None
    other = lambda v: abs(v[2 - k])
    a = min((v for v in vs if abs(v[1 + k] - lo) < 1e-4), key=other)
    b = min((v for v in vs if abs(v[1 + k] - hi) < 1e-4), key=other)
    return [a[0], b[0]]


def place_linear(vname, axis, side, name, value):
    """外形尺寸: vname 视图里沿 axis("x" 横 / "y" 竖)两端的极值顶点之间, 放在 side(top / bottom / left / right)外侧。"""
    v, r = views[vname], R[vname]
    pair = extreme_pair(v, axis)
    e = longest(v, axis)
    if not pair and not e:
        return
    refs_ = pair if pair else [e]
    cx, cy = (r[0] + r[2]) / 2, (r[1] + r[3]) / 2
    if name in LAY_DIM:
        # 布局层: 尺寸线所在位置(TechDraw 的尺寸位置 = 文字处, 尺寸线跟着走)
        d_ = LAY_DIM[name]
        (p0, p1) = d_["line"]
        p = (d_["text_at"][0], H - p0[1]) if axis == "x" else (p0[0], H - d_["text_at"][1])
        add_dim(v, "DistanceX" if axis == "x" else "DistanceY", refs_, name, p, fmt_spec=FSPEC, value=value)
        return
    # 左右两侧多让 6 mm: 剖切符号的箭头伸在视图左右外面
    p = {"top": (cx, r[3] + G), "bottom": (cx, r[1] - G), "left": (r[0] - G - 6, cy), "right": (r[2] + G + 6, cy)}[side]
    add_dim(v, "DistanceX" if axis == "x" else "DistanceY", refs_, name, p, fmt_spec=FSPEC, value=value)
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


# 坐标值的界线: full = 从孔心拉到坐标值(看得出量的是哪个孔), stub = 只在轮廓外留一小段(方案 "ordinate_ext")
ORD_EXT = spec.get("ordinate_ext", "full")


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
    # 界线从孔心引出(用户 2026-11-19: 尺寸线要看得出连在哪个点上); 外形两端的值本身就在轮廓边上, 不用引
    for y, pg in zip(ys, got):
        py_ = cyp + y * scale
        x0_ = r[0] - gap
        x1_ = x0_ - tick
        x2_ = x1_ - jog
        own = [p[0] for p in pts if abs(p[1] - y) < 0.01]
        if ORD_EXT == "full" and own and abs(abs(y) - hh_) > 0.01:
            poly((cxp + min(own) * scale, py_), (x0_, py_))
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
        own = [p[1] for p in pts if abs(p[0] - x) < 0.01]
        if ORD_EXT == "full" and own and abs(abs(x) - hw) > 0.01:
            poly((px_, cyp + max(own) * scale), (px_, y0_))
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




def circle_for(vname, circles, h):
    """视图里与孔对应的圆边: 半径 = d/2, 圆心最接近孔轴投影。视图边坐标 = 模型单位(未乘比例)、以零件包围盒中心为原点,
    横 / 竖方向的正负不确定(四种都试)。返回 (边名, 圆心在页面上相对视图中心的偏移 = 边坐标 × 比例)。"""
    bw_, bh_, _ = VAX[vname]
    at = V(*h["at"])
    px, py = (at - bb.Center).dot(bw_), (at - bb.Center).dot(bh_)
    best, bdist = None, 1e9
    for n, ed in circles:
        if abs(ed.Curve.Radius - h["d"] / 2) > rtol(h["d"] / 2):
            continue
        c = ed.Curve.Center
        for sx in (1, -1):
            for sy in (1, -1):
                dd = math.hypot(c.x - sx * px, c.y - sy * py)
                if dd < bdist:
                    best, bdist = (n, V(c.x * scale, c.y * scale, 0)), dd
    return best


# ---------------------------------------------------------------- 布局第二遍: 视图建好了, 引出标注的锚点换成视图里实际的圆边(位置 / 正负号准), 比例固定
LAY_LAB, LAY_DIM, LAY_EDGE = {}, {}, {}
if LAY:
    anchors_ = {}
    k45_ = math.sqrt(0.5)
    circ_cache_ = {}

    def circ_(vn):
        if vn not in circ_cache_:
            circ_cache_[vn] = circle_edges(views[vn])
        return circ_cache_[vn]

    for g in HOLE_GROUPS:
        vn = g["view"]
        bw_, bh_, _ = VAX[vn]
        hw_, hh2_ = ext(bw_) / 2, ext(bh_) / 2
        best_ = None
        for hh in g["holes"]:
            ht = circle_for(vn, circ_(vn), hh)
            if not ht:
                continue
            n_, c_ = ht
            lx, ly = c_.x / scale, c_.y / scale          # 视图边坐标(模型毫米, y 向上)
            edge_d = min(hw_ - abs(lx), hh2_ - abs(ly))   # 离视图边多近(出路短)
            if best_ is None or edge_d < best_[0]:
                best_ = (edge_d, n_, lx, ly, hh["d"] / 2)
        if best_:
            _, n_, lx, ly, r_ = best_
            anchors_[g["id"]] = (vn, [lx + r_ * k45_, ly + r_ * k45_])
            LAY_EDGE[g["id"]] = n_

    def dia_anchor_(id_, vn, radius):
        cs_ = [(n_, ed) for n_, ed in circ_(vn) if abs(ed.Curve.Radius - radius) < rtol(radius)]
        if cs_:
            n_, ed = cs_[0]
            c_ = ed.Curve.Center
            anchors_[id_] = (vn, [c_.x + radius * k45_, c_.y + radius * k45_])
            LAY_EDGE[id_] = n_

    fr2_ = outer_round("Front")
    if fr2_:
        dia_anchor_("DiaOuter", "Front", fr2_)
    for i_, (d_, _) in enumerate([] if TURN else sorted(SHAFT_TOL.items())):   # 回转件: 配合已在段直径上
        if fr2_ and abs(2 * fr2_ - d_) < 0.05:
            continue
        vs_ = outer_cyl_views(d_)
        if vs_:
            dia_anchor_("DiaFit%d" % (i_ + 1), vs_[0], d_ / 2)
    # 锥面: 视图里与轴成半锥角的直线边(锥面的轮廓线), 取离估计点最近的那条的中点
    for i_, cn in enumerate(CONES):
        vn = cone_view(cn)
        vobj = sec if vn == "Section" else views.get(vn)
        if vobj is None:
            continue
        bw_, bh_ = view_axes(vn)
        ax2 = (cn["axis"].dot(bw_), cn["axis"].dot(bh_))
        la = math.hypot(*ax2)
        if la < 1e-6:
            continue
        ax2 = (ax2[0] / la, ax2[1] / la)
        est = local_of(vn, cn["mid"])
        best_ = None
        for n_, ed in edges_of(vobj):
            # 剖视里锥面的轮廓线常是 B 样条(实测磁环座): 只要是直的(中点在弦上)就算
            if len(ed.Vertexes) < 2:
                continue
            p0_, p1_ = ed.Vertexes[0].Point, ed.Vertexes[-1].Point
            dx, dy = p1_.x - p0_.x, p1_.y - p0_.y
            ll = math.hypot(dx, dy)
            if ll < 0.5:
                continue
            if not isinstance(ed.Curve, Part.Line):
                try:
                    pm = ed.valueAt((ed.FirstParameter + ed.LastParameter) / 2)
                except Exception:
                    continue
                if abs((pm.x - p0_.x) * dy - (pm.y - p0_.y) * dx) / ll > 0.02:
                    continue
            ang = math.acos(min(1.0, abs(dx * ax2[0] + dy * ax2[1]) / ll))
            if abs(ang - cn["half"]) > math.radians(0.5):
                continue
            m_ = ((p0_.x + p1_.x) / 2, (p0_.y + p1_.y) / 2)
            dd = min(math.hypot(m_[0] - sx * est[0], m_[1] - sy * est[1]) for sx in (1, -1) for sy in (1, -1))
            if best_ is None or dd < best_[0]:
                best_ = (dd, m_)
        if best_:
            anchors_["Taper%d" % (i_ + 1)] = (vn, [best_[1][0], best_[1][1]])
    try:
        # 剖视被删掉(空出地方)且方案没定比例 → 第二遍可以重新选比例(可能更大)
        free_scale_ = globals().get("SEC_REMOVED") and spec.get("scale", "auto") == "auto"
        L2_ = LAYM.solve(layout_problem(anchors_, fixed_scale=None if free_scale_ else scale, sec_dir=SEC_DIR), std)
        if L2_.get("scale") and abs(L2_["scale"] - scale) > 1e-9:
            scale = L2_["scale"]
            rep["scale"] = scale
            grp.Scale = scale
            doc.recompute()
            wait_views(list(grp.Views))
        same_ = L2_.get("scale") and all(abs(L2_["views"][k]["cx"] - LAY["views"][k]["cx"]) < 1e-6 and abs(L2_["views"][k]["cy"] - LAY["views"][k]["cy"]) < 1e-6
                                         for k in LAY["views"])
        if same_:
            LAY = L2_
        elif L2_.get("scale"):
            # 视图位置变了(如剖视没出图被删掉, 空出地方): 照第二遍挪视图(此时还没画任何尺寸)
            LAY = L2_
            fx, fy = lay_xy("Front")
            grp.X, grp.Y = fx, fy
            for v_ in grp.Views:
                if v_.Type in ("Top", "Left"):
                    x_, y_ = lay_xy(v_.Type)
                    v_.X, v_.Y = x_ - fx, y_ - fy
            if "Iso" in LAY["views"]:
                iso.Scale = LAY["views"]["Iso"]["scale"]
                iso.X, iso.Y = lay_xy("Iso")
            if sec is not None and "Section" in LAY["views"]:
                sec.X, sec.Y = lay_xy("Section")
            doc.recompute()
            R = {n: vrect(views[n], ext(VAX[n][0]) * scale, ext(VAX[n][1]) * scale) for n in VAX if n in views}
            iso_s = q(iso.Scale)
            R["Iso"] = vrect(iso, diag * iso_s, diag * iso_s / 1.2)
            if sec is not None:
                R["Section"] = vrect(sec, ext(VAX[sec_base][0]) * scale, ext(VAX[sec_base][2]) * scale)
            placer = Placer(list(R.values()))
            rep["layout"]["moved"] = True
    except Exception as e:
        rep["warnings"].append("layout pass 2 failed: %s" % e)
    rep["layout"]["pass2"] = {"unplaced": LAY["unplaced"], "conflicts": LAY["conflicts"][:10], "ugly": LAY.get("ugly", [])}
    for w_ in LAY["unplaced"]:
        rep["warnings"].append("layout: %s not placed (%s)" % (w_["id"], w_["why"]))
    LAY_DIM.update(LAY["dims"])
    LAY_LAB.update(LAY["labels"])
    # 锥度标注: 引线(锥面上一点 → 折点 → 肩线)+ 文字, 画进页面图形(布局坐标本来就是 y 向下的页面毫米)
    fs_, lw_, fam_ = std["fonts"]["dim"], std["lines"]["thin"], std["fonts"]["latin"]
    # 回转件的段直径(视图上)/ 轴向链式尺寸(主视下方): 照布局结果画尺寸线 + 箭头 + 界线 + 文字
    def svg_arrow_(tip, toward):
        dx, dy = toward[0] - tip[0], toward[1] - tip[1]
        n = math.hypot(dx, dy) or 1.0
        ux, uy = dx / n, dy / n
        L_ = std["layout"]["arrow"]
        bx, by = tip[0] + ux * L_, tip[1] + uy * L_
        PAGE_SVG.append('<polygon points="%.3f,%.3f %.3f,%.3f %.3f,%.3f" style="fill:#000"/>'
                        % (tip[0], tip[1], bx - uy * 0.5, by + ux * 0.5, bx + uy * 0.5, by - ux * 0.5))

    def svg_line_(p, q):
        PAGE_SVG.append('<line x1="%.3f" y1="%.3f" x2="%.3f" y2="%.3f" style="stroke:#000;stroke-width:%g"/>' % (p[0], p[1], q[0], q[1], lw_))

    T_ = LAYM.Text(std)

    def svg_text_(x, cy, s, rot=0, anchor="middle"):
        """文字(竖向的转 -90°: 字头朝左, 从下往上读 —— GB); 带上下偏差的写成叠放的小字(上偏差在上、下偏差在下)。
        x = 中心(anchor middle)或左端(anchor start), cy = 文字竖直中线。"""
        t = LAYM.split_tol(s)
        g0 = '<g transform="rotate(-90 %.3f %.3f)">' % (x, cy) if rot == -90 else "<g>"   # 绕给定点(中心 / 左端)转
        base = cy + fs_ * 0.35
        # 页面图形由 TechDraw 的 SVG 渲染器画: 不认 tspan 的字号 / 偏移 / 透明 → 每段单独一个 <text>, 按实测字宽摆
        # (仿宋: 汉字 1.0 × 字高, 数字字母 svg_char_w × 字高, 实测 0.50; layout 的 0.85 是 TechDraw 尺寸字体的)
        cw = std["layout"].get("svg_char_w", 0.5)
        wdt = lambda u, h: sum(h * (1.0 if ord(ch) > 0x2E80 else cw) for ch in u)
        out = [g0]
        if not t:
            W = wdt(s, fs_)
            x0 = x - W / 2 if anchor == "middle" else x
            out.append('<text x="%.3f" y="%.3f" style="font-family:%s;font-size:%gpx;fill:#000">%s</text>' % (x0, base, fam_, fs_, s))
        else:
            pre, up, lo, post, paren = t
            fs2 = fs_ * LAYM.TOL_SCALE
            head, tail = pre + ("(" if paren else ""), (")" if paren else "") + post
            wd = max(wdt(up, fs2), wdt(lo, fs2))
            W = wdt(head, fs_) + (0 if paren else 0.25 * fs_) + wd + wdt(tail, fs_)
            x0 = x - W / 2 if anchor == "middle" else x
            out.append('<text x="%.3f" y="%.3f" style="font-family:%s;font-size:%gpx;fill:#000">%s</text>' % (x0, base, fam_, fs_, head))
            xx = x0 + wdt(head, fs_) + (0 if paren else 0.25 * fs_)
            # 上偏差的底线在中线略上, 下偏差的底线在文字底线附近(两行合起来约一个字高), 左端对齐
            out.append('<text x="%.3f" y="%.3f" style="font-family:%s;font-size:%gpx;fill:#000">%s</text>' % (xx, cy - fs_ * 0.08, fam_, fs2, up))
            out.append('<text x="%.3f" y="%.3f" style="font-family:%s;font-size:%gpx;fill:#000">%s</text>' % (xx, cy + fs_ * 0.62, fam_, fs2, lo))
            if tail:
                out.append('<text x="%.3f" y="%.3f" style="font-family:%s;font-size:%gpx;fill:#000">%s</text>' % (xx + wd, base, fam_, fs_, tail))
        out.append("</g>")
        PAGE_SVG.append("".join(out))

    for id_, d_ in LAY["dims"].items():
        if not id_.startswith(("Dstep", "Lstep")):
            continue
        (p_, q_) = d_["line"]
        for e_ in d_.get("ext", []):
            svg_line_(e_[0], e_[1])
        lx = d_.get("line_ext") or d_["line"]
        svg_line_(lx[0], lx[1])
        if d_.get("tick"):
            svg_line_(d_["tick"][0], d_["tick"][1])
        svg_arrow_(p_, q_)
        svg_arrow_(q_, p_)
        r_ = d_["text_rect"]
        cx_, cy_ = (r_[0] + r_[2]) / 2, (r_[1] + r_[3]) / 2
        if d_.get("text"):
            svg_text_(cx_, cy_, d_["text"], d_.get("rot", 0))
        rep["placed"].append({"name": id_, "type": "Diameter" if id_.startswith("D") else "DistanceX", "text": d_["text"], "at": [round(cx_, 1), round(H - cy_, 1)]})
    for id_, l_ in LAY["labels"].items():
        # 锥度; 窄台阶放不下的段直径(布局改成了引出标注)
        if not id_.startswith(("Taper", "Dstep")) or id_.endswith("#t"):   # "#t" = 标注行里的段直径文字, 下面单独画
            continue
        a_, k_, e_ = l_["leader"]
        PAGE_SVG.append('<polyline points="%.3f,%.3f %.3f,%.3f %.3f,%.3f" style="fill:none;stroke:#000;stroke-width:%g"/>'
                        % (a_[0], a_[1], k_[0], k_[1], e_[0], e_[1], lw_))
        PAGE_SVG.append('<circle cx="%.3f" cy="%.3f" r="0.5" style="fill:#000"/>' % (a_[0], a_[1]))   # 引线落在面上 = 圆点
        r_ = l_["text_rect"]
        PAGE_SVG.append('<text x="%.3f" y="%.3f" style="font-family:%s;font-size:%gpx;fill:#000">%s</text>'
                        % (r_[0] + 0.5, r_[3] - 0.4, fam_, fs_, l_["lines"][0]))
        rep["placed"].append({"name": id_, "type": "Taper", "text": l_["lines"][0], "at": [round(r_[0], 1), round(H - r_[3], 1)]})
    # 基准(实心三角在要素上 + 方框字母)/ 形位公差框格(箭头指到被测要素)/ 粗糙度(去除材料符号 + Ra 值, 引线指到表面)
    sym_font_ = fam_ + ", Segoe UI Symbol"
    # 段直径的文字放不进段里时进了标注行: 尺寸线一端 → 行里的文字(细线连过去, 文字下面一条肩线)
    for id_, l_ in LAY["labels"].items():
        if not id_.endswith("#t"):
            continue
        pts_ = l_["leader"]
        r_ = l_["text_rect"]
        for p_, q_ in zip(pts_, pts_[1:]):
            svg_line_(p_, q_)
        svg_line_((r_[0], r_[3]), (r_[2], r_[3]))
        svg_text_(r_[0] + 0.5, (r_[1] + r_[3]) / 2 - 0.4, l_["lines"][0], 0, "start")
        rep["placed"].append({"name": id_, "type": "DiameterText", "text": l_["lines"][0], "at": [round(r_[0], 1), round(H - r_[3], 1)]})
    for id_, l_ in LAY["labels"].items():
        o_ = GDT_BY_ID.get(id_)
        if not o_:
            continue
        pts_ = l_["leader"]
        a_, k_, e_ = pts_[0], pts_[1], pts_[-1]
        r_ = l_["text_rect"]
        row_ = l_.get("row")
        rest_ = list(zip(pts_[1:], pts_[2:]))   # 折点之后的各段(标注行: 竖直出尺寸区 → 斜线到框)
        if l_.get("ext_line"):
            svg_line_(l_["ext_line"][0], l_["ext_line"][1])   # 边视平面的延长线(箭头 / 三角落在上面)
        if o_["kind"] == "gdt":
            # 引线: 要素 → 折点 → 框格(箭头在要素一端)
            svg_line_(a_, k_)
            if row_:
                for p_, q_ in rest_:
                    svg_line_(p_, q_)
            else:
                svg_line_(k_, (k_[0], r_[3]))
            svg_arrow_(a_, k_)
            x_ = r_[0]
            for c_, w_ in zip(o_["cells"], o_["cell_w"]):
                PAGE_SVG.append('<rect x="%.3f" y="%.3f" width="%.3f" height="%.3f" style="fill:none;stroke:#000;stroke-width:%g"/>'
                                % (x_, r_[1], w_, r_[3] - r_[1], lw_))
                PAGE_SVG.append('<text x="%.3f" y="%.3f" style="font-family:%s;font-size:%gpx;text-anchor:middle;fill:#000">%s</text>'
                                % (x_ + w_ / 2, (r_[1] + r_[3]) / 2 + fs_ * 0.35, sym_font_, fs_, c_))
                x_ += w_
        elif o_["kind"] == "datum":
            # 实心三角: 底边在要素上(垂直于引线), 顶点朝外; 再连到方框字母
            dx, dy = k_[0] - a_[0], k_[1] - a_[1]
            n_ = math.hypot(dx, dy) or 1.0
            ux, uy = dx / n_, dy / n_
            hb, ht = 1.8, 3.0
            p1 = (a_[0] - uy * hb, a_[1] + ux * hb)
            p2 = (a_[0] + uy * hb, a_[1] - ux * hb)
            tp = (a_[0] + ux * ht, a_[1] + uy * ht)
            PAGE_SVG.append('<polygon points="%.3f,%.3f %.3f,%.3f %.3f,%.3f" style="fill:#000"/>' % (p1[0], p1[1], p2[0], p2[1], tp[0], tp[1]))
            bx = (r_[0] + r_[2]) / 2
            svg_line_(tp, k_)
            if row_:
                for p_, q_ in rest_:
                    svg_line_(p_, q_)
            else:
                svg_line_(k_, (bx, r_[3]) if abs(k_[0] - bx) > 0.1 else k_)
            PAGE_SVG.append('<rect x="%.3f" y="%.3f" width="%.3f" height="%.3f" style="fill:none;stroke:#000;stroke-width:%g"/>'
                            % (r_[0], r_[1], r_[2] - r_[0], r_[3] - r_[1], lw_))
            PAGE_SVG.append('<text x="%.3f" y="%.3f" style="font-family:%s;font-size:%gpx;text-anchor:middle;fill:#000">%s</text>'
                            % (bx, (r_[1] + r_[3]) / 2 + fs_ * 0.35, fam_, fs_, o_["text"]))
        else:
            svg_line_(a_, k_)
            for p_, q_ in (rest_ if row_ else [(k_, e_)]):
                svg_line_(p_, q_)
            svg_arrow_(a_, k_)
            x0_ = r_[0] + 0.5
            s_ = k_[1]
            if row_:
                # 行里: 肩线 = 框底边, 符号尖落在肩线上
                s_ = r_[3]
                svg_line_((r_[0], s_), (r_[2], s_))
            A_, Tp_, B_ = (x0_, s_ - 3.0), (x0_ + 2.0, s_), (x0_ + 5.5, s_ - 7.0)
            PAGE_SVG.append('<polyline points="%.3f,%.3f %.3f,%.3f %.3f,%.3f" style="fill:none;stroke:#000;stroke-width:%g"/>'
                            % (A_[0], A_[1], Tp_[0], Tp_[1], B_[0], B_[1], lw_))
            svg_line_(A_, (x0_ + 3.5, s_ - 3.0))   # 去除材料: 短边封口
            PAGE_SVG.append('<text x="%.3f" y="%.3f" style="font-family:%s;font-size:%gpx;fill:#000">%s</text>'
                            % (x0_ + 7.0, s_ - 0.8, fam_, fs_, o_["text"]))
        rep["placed"].append({"name": id_, "type": o_["kind"], "text": o_["text"], "at": [round(r_[0], 1), round(H - r_[3], 1)]})
    # 布局排好的文字占位交给旧的避让器(后面补的尺寸 / 螺纹等照常避开)
    for d_ in list(LAY["dims"].values()) + list(LAY["labels"].values()):
        r_ = d_["text_rect"]
        placer.take((r_[0], H - r_[3], r_[2], H - r_[1]))


def lay_text_center(item):
    """布局给的文字框中心 → TechDraw 页面坐标(y 向上)。"""
    r_ = item["text_rect"]
    return ((r_[0] + r_[2]) / 2, H - (r_[1] + r_[3]) / 2)


has_ord = bool(ORD_GROUPS) and ordinates("Front", [h for g in ORD_GROUPS for h in g["holes"]])


def place_diameter(vname, radius, name):
    """在视图里找半径 = radius 的圆边, 标 ⌀(位置: 视图右上 / 左上 / 右下外侧, 避让)。有配合公差就带上: ⌀8 g6(-0.005/-0.014)。"""
    v, r = views[vname], R[vname]
    circ = [(n, ed) for n, ed in circle_edges(v) if abs(ed.Curve.Radius - radius) < rtol(radius)]
    if not circ:
        rep["warnings"].append("round %s: circle edge not found" % vname)
        return False
    tol = shaft_tol(radius)
    txt = DIMS["diameter"] + fmt(2 * radius) + (" " + tol if tol else "")
    if name in LAY_LAB:
        p = lay_text_center(LAY_LAB[name])
        edge = LAY_EDGE.get(name, circ[0][0])
        if tol:
            add_dim(v, "Diameter", [edge], name, p, text=txt, value=2 * radius)
        else:
            add_dim(v, "Diameter", [edge], name, p, fmt_spec=DIMS["diameter"] + FSPEC, value=2 * radius)
        return True
    for ring in range(8):
        off = G * 1.6 + ring * STEP
        for p in ((r[2] + off, r[3] + off * 0.6), (r[0] - off, r[3] + off * 0.6), (r[2] + off, r[1] - off * 0.6)):
            rr = text_rect(p[0], p[1], txt)
            if placer.free(rr):
                placer.take(rr)
                if tol:
                    add_dim(v, "Diameter", [circ[0][0]], name, p, text=txt, value=2 * radius)
                else:
                    add_dim(v, "Diameter", [circ[0][0]], name, p, fmt_spec=DIMS["diameter"] + FSPEC, value=2 * radius)
                return True
    return False


# 外形: 主视 宽(上)、高(左); 厚度标在左视(上)。第一角里主视下面是俯视, 所以宽标在上面。
# 有坐标尺寸时末值就是总长 / 总宽, 不再重复标; 主视轮廓是整圆 → 标直径。
front_r = outer_round("Front")
rep["round_front"] = bool(front_r)
if TURN:
    # 回转件: 总长在上; 高 / 厚 = 最大外圆, 已在视图上按段标 ⌀(左视是端面圆, 不再标宽)
    place_linear("Front", "x", "top", "LenW", Wm)
elif front_r:
    place_diameter("Front", front_r, "DiaOuter")
elif not has_ord:
    place_linear("Front", "x", "top", "LenW", Wm)
    place_linear("Front", "y", "left", "LenH", Hm)
if not TURN:
    place_linear("Left", "x", "top", "LenD", Dm)


# 配合的轴径(主视外径已标的除外): 在它是圆的视图里标 ⌀ + 公差
for i_, (d_, _) in enumerate([] if TURN else sorted(SHAFT_TOL.items())):   # 回转件: 配合已在段直径上
    if front_r and abs(2 * front_r - d_) < 0.05:
        continue
    vs_ = outer_cyl_views(d_)
    if not vs_ or not place_diameter(vs_[0], d_ / 2, "DiaFit%d" % (i_ + 1)):
        rep["warnings"].append("fit shaft %s: no view shows it as a circle" % fmt(d_))


# 孔: 按"在哪个视图里是圆"分视图, 同规格合并成一条标注("6 x ⌀4.3 贯穿"), 引线指向其中一个
circles_of = {}
for g in HOLE_GROUPS:
    vname, hs, txt = g["view"], g["holes"], g["text"]
    v, r = views[vname], R[vname]
    if vname not in circles_of:
        circles_of[vname] = circle_edges(v)
    circles = circles_of[vname]
    cxV, cyV = (r[0] + r[2]) / 2, (r[1] + r[3]) / 2
    hits = [(hh, circle_for(vname, circles, hh)) for hh in hs]
    hits = [(hh, ht) for hh, ht in hits if ht]
    if not hits:
        rep["warnings"].append("no circle for hole group %s in %s" % (g["id"], vname))
        continue
    if g["id"] in LAY_LAB and g["id"] in LAY_EDGE:
        add_dim(v, "Diameter", [LAY_EDGE[g["id"]]], g["id"], lay_text_center(LAY_LAB[g["id"]]), text=txt)
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
    circles = circles_of.get(vname) or circle_edges(views[vname])
    hs = [h for h in holes if hole_view(h) == vname]
    best, bn = (1, 1), -1
    for sx in (1, -1):
        for sy in (1, -1):
            n_ = 0
            for h in hs:
                at = V(*h["at"])
                px, py = (at - bb.Center).dot(bw_), (at - bb.Center).dot(bh_)
                if any(abs(ed.Curve.Radius - h["d"] / 2) < rtol(h["d"] / 2) and math.hypot(ed.Curve.Center.x - sx * px, ed.Curve.Center.y - sy * py) < 1e-2
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
        circles = circles_of.get(vname) or circle_edges(views[vname])
        for h in hs:
            if not h.get("through") and V(*h["axis"]).dot(vdir) <= 0:
                continue  # 背面的盲孔: 看不见
            at = V(*h["at"])
            px, py = (at - bb.Center).dot(bw_), (at - bb.Center).dot(bh_)
            hit = next((ed for _, ed in circles if abs(ed.Curve.Radius - h["d"] / 2) < rtol(h["d"] / 2)
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


# ---------------------------------------------------------------- 补尺寸(面板里从 Plasticity 选的面)
# dims.add = [{"name": "U1", "faces": [签名…]}]; 签名 = Plasticity 面的 center(面上一点)/ normal / surface / radius / axis。
#   1 个圆柱面 → 直径; 1 个平面 → 到对面平行平面的厚度; 2 个面(平面 / 圆柱轴线, 互相平行)→ 距离。
# 自己画进页面 SVG(尺寸线 / 尺寸界线 / 箭头 / 数字), 不走 TechDraw 引用(省消隐重算, 也不依赖边编号)。
_VSIGN = {}


def view_signs(vname):
    """模型(VAX 轴, 相对包围盒中心)→ 页面的正负。主视 = (+, +); 其它视图: 先用模型顶点对上视图边端点得 模型 → 边 的正负,
    再乘主视的 边 → 页面 正负(与 draw_threads 同一个约定)。"""
    if vname in _VSIGN:
        return _VSIGN[vname]

    def m2e(vn):
        bw_, bh_, _ = VAX[vn]
        ev = {(round(p.Point.x, 1), round(p.Point.y, 1)) for _, ed in edges_of(views[vn]) for p in ed.Vertexes}
        mv = [((p.Point - bb.Center).dot(bw_), (p.Point - bb.Center).dot(bh_)) for p in shape.Vertexes[:600]]
        sc = {(sx, sy): sum((round(sx * x, 1), round(sy * y, 1)) in ev for x, y in mv) for sx in (1, -1) for sy in (1, -1)}
        return max(sc, key=sc.get)

    if "Front" not in _VSIGN:
        _VSIGN["_fe"] = m2e("Front")  # 主视 模型 → 边; 主视 模型 → 页面 = (+, +) ⇒ 边 → 页面 = 同号
        _VSIGN["Front"] = (1, 1)
    if vname != "Front":
        fe, s = _VSIGN["_fe"], m2e(vname)
        _VSIGN[vname] = (fe[0] * s[0], fe[1] * s[1])
    return _VSIGN[vname]


def to_page(vname, p):
    bw_, bh_, _ = VAX[vname]
    sx, sy = view_signs(vname)
    cx, cy = vcenter(views[vname])
    return cx + sx * (p - bb.Center).dot(bw_) * scale, cy + sy * (p - bb.Center).dot(bh_) * scale


def match_face(sig):
    """签名 → 零件上的面: 面上那一点到面的距离最小、且该点法向与签名法向平行(反向也算: STEP 面朝向可能反)。"""
    c = V(*sig["center"])
    n = V(*sig["normal"]).normalize() if sig.get("normal") else None
    best, bd = None, 1e9
    for f in shape.Faces:
        try:
            d, _, info = f.distToShape(Part.Vertex(c))
        except Exception:
            continue
        if d > bd or d > 0.05:
            continue
        if n is not None:
            try:
                uv = f.Surface.parameter(c)
                fn = f.normalAt(uv[0], uv[1])
                if abs(abs(fn.dot(n)) - 1) > 1e-3:
                    continue
            except Exception:
                pass
        best, bd = f, d
    return best


def feature_of(f):
    """面 → ("plane", 点, 法向) / ("axis", 轴上一点, 轴向, 半径); 其它曲面 None。"""
    cy = geomutil.cylinder_of(f)
    if cy is not None:
        return ("axis", V(cy["c"]), V(cy["ax"]).normalize(), cy["r"])
    if isinstance(f.Surface, Part.Plane):
        uv = f.Surface.parameter(f.CenterOfMass)
        return ("plane", f.CenterOfMass, f.normalAt(uv[0], uv[1]).normalize())
    return None


def opposite_plane(f, ft):
    """厚度: 与 f 平行、法向相反、沿 -法向 最近、且投影有重叠的平面。"""
    _, p0, n = ft
    best, bd = None, 1e9
    for g in shape.Faces:
        if g.isSame(f) or not isinstance(g.Surface, Part.Plane):
            continue
        gt = feature_of(g)
        if gt is None or abs(gt[2].dot(n) + 1) > 1e-3:
            continue
        d = (p0 - gt[1]).dot(n)
        if 1e-4 < d < bd:
            best, bd = g, d
    return best


def add_measure(item):
    """一条补尺寸 → (方向 d, 点 A, 点 B, 值, 前缀, 涉及的面) 或 抛出原因。"""
    sigs = item.get("faces") or []
    fs = [match_face(s) for s in sigs]
    if not fs or any(f is None for f in fs):
        raise ValueError("face not found in the exported part")
    fts = [feature_of(f) for f in fs]
    if any(t is None for t in fts):
        raise ValueError("only planes and cylinders can be dimensioned")
    if len(fs) == 1:
        t = fts[0]
        if t[0] == "axis":
            ax, r = t[2], t[3]
            d = ax.cross(V(1, 0, 0)) if abs(ax.x) < 0.9 else ax.cross(V(0, 1, 0))
            return ("dia", t, d.normalize(), t[1] - d.normalize() * r, t[1] + d.normalize() * r, 2 * r, DIMS["diameter"], fs)
        g = opposite_plane(fs[0], t)
        if g is None:
            raise ValueError("no opposite parallel face for a thickness")
        fts.append(feature_of(g))
        fs.append(g)
    a, b = fts[0], fts[1]
    if a[0] == "plane":
        d = a[2]
    elif b[0] == "plane":
        d = b[2]
    else:  # 两根轴线: 中心距
        if abs(abs(a[2].dot(b[2])) - 1) > 1e-3:
            raise ValueError("axes are not parallel")
        b = (b[0], b[1] - a[2] * (b[1] - a[1]).dot(a[2]), b[2], b[3])  # 第二根轴上取与第一点同高的点
        w = b[1] - a[1]
        d = w - a[2] * w.dot(a[2])
        if d.Length < 0.01:
            raise ValueError("coaxial: distance is 0")
        d.normalize()
    for t in (a, b):
        if t[0] == "plane" and abs(abs(t[2].dot(d)) - 1) > 1e-3:
            raise ValueError("faces are not parallel")
        if t[0] == "axis" and abs(t[2].dot(d)) > 1e-3:
            raise ValueError("axis is not parallel to the face")
    A, B = a[1], b[1]
    val = abs((B - A).dot(d))
    if val < 0.01:
        raise ValueError("distance is 0")
    return ("lin", a, d, A, A + d * (B - A).dot(d), val, "", fs)  # B 换成 b 上正对 A 的点: 尺寸线 ∥ d


def draw_added_dim(item):
    name = str(item.get("name") or "U")
    info = {"name": name, "type": "Added", "view": None, "text": None, "at": [0, 0]}
    try:
        kind, t0, d, A, B, val, pre, fs = add_measure(item)
    except Exception as e:
        info["error"] = str(e)
        rep["added_failed"].append(info)
        rep["warnings"].append("added dim %s: %s" % (name, e))
        return
    info["value"] = round(val, 4)
    # 视图: d 在视图平面里(⟂ 视线); 圆柱还要求轴线也在视图平面里(侧面看成两条线), 优先主视
    if kind == "dia":
        cand = [vn for vn in VIEWS3 if abs(t0[2].dot(VAX[vn][2])) < 1e-3]
    else:
        cand = [vn for vn in VIEWS3 if abs(d.dot(VAX[vn][2])) < 1e-3]
    if not cand:
        info["error"] = "no view shows this distance"
        rep["added_failed"].append(info)
        return
    vn = cand[0]
    info["view"] = vn
    if kind == "dia":  # 直径在视图平面里、垂直轴线量
        d = t0[2].cross(VAX[vn][2]).normalize()
        A, B = t0[1] - d * t0[3], t0[1] + d * t0[3]
    pa, pb = to_page(vn, A), to_page(vn, B)
    ux, uy = pb[0] - pa[0], pb[1] - pa[1]
    L = math.hypot(ux, uy)
    if L < 1e-6:
        info["error"] = "zero length on paper"
        rep["added_failed"].append(info)
        return
    ux, uy = ux / L, uy / L
    wx, wy = -uy, ux  # 尺寸线的偏移方向(两侧都试)
    # 面的投影在 w 方向上的范围(尺寸界线从这里起)
    pts = [to_page(vn, p.Point) for f in fs for p in f.Vertexes] or [pa, pb]
    if kind == "dia":
        pts += [pa, pb]
    txt = str(DIM_TEXT.get(name) or "").strip() or pre + fmt(val)
    if name not in DIM_TEXT and name in DIM_TOL:
        txt += tol_suffix(val, DIM_TOL[name])
        info["tol"] = DIM_TOL[name]
    if name in DIM_KEY:
        txt = DIMS["key_mark"] + txt
        info["key"] = True
    if name in DIM_HIDE:
        info["hidden"] = True
        info["at"] = [round((pa[0] + pb[0]) / 2, 1), round((pa[1] + pb[1]) / 2, 1)]
        rep["placed"].append(info)
        return
    r = R[vn]
    fsz = std["fonts"]["dim"]
    for ring in range(10):
        for sgn in (1, -1):
            ww = (wx * sgn, wy * sgn)
            ext_w = max((p[0] - pa[0]) * ww[0] + (p[1] - pa[1]) * ww[1] for p in pts)
            # 视图矩形在 w 上的外沿, 尺寸线放到它外面
            corners = [(r[0], r[1]), (r[0], r[3]), (r[2], r[1]), (r[2], r[3])]
            out_w = max((c[0] - pa[0]) * ww[0] + (c[1] - pa[1]) * ww[1] for c in corners)
            off = max(ext_w, out_w) + G + ring * STEP
            m = ((pa[0] + pb[0]) / 2 + ww[0] * off, (pa[1] + pb[1]) / 2 + ww[1] * off)
            vertical = abs(ux) < 0.5
            # GB: 数字在水平尺寸线上方、竖直尺寸线左侧(不论尺寸放在视图哪一边)
            tc = (m[0] - fsz * 0.9, m[1]) if vertical else (m[0], m[1] + fsz * 0.9)
            tr_ = text_rect(tc[0], tc[1], txt)
            if vertical:
                hw_, hh_ = (tr_[2] - tr_[0]) / 2, (tr_[3] - tr_[1]) / 2
                tr_ = (tc[0] - hh_, tc[1] - hw_, tc[0] + hh_, tc[1] + hw_)
            if placer.free(tr_):
                placer.take(tr_)
                svg_linear_dim(pa, pb, pts, ww, off, (ux, uy), txt, tc, vertical)
                info["at"] = [round(tc[0], 1), round(tc[1], 1)]
                info["shown"] = txt
                rep["placed"].append(info)
                return
    info["error"] = "no free place on the sheet"
    rep["added_failed"].append(info)


def svg_linear_dim(pa, pb, pts, ww, off, u, txt, tc, vertical):
    """页面坐标(y 向上)的线性尺寸 → PAGE_SVG: 两条尺寸界线 + 尺寸线 + 两个实心箭头 + 数字(竖直尺寸字头朝左)。"""
    lw = std["lines"]["thin"]
    fam = std["fonts"]["latin"]
    fsz = std["fonts"]["dim"]
    ar, aw = 3.0, 0.5  # 箭头长 / 半宽(页面毫米)

    def s(p):
        return p[0], H - p[1]

    def line(p, q_):
        a, b = s(p), s(q_)
        PAGE_SVG.append('<path d="M %.3f %.3f L %.3f %.3f" style="fill:none;stroke:#000;stroke-width:%g"/>' % (a[0], a[1], b[0], b[1], lw))

    ends = []
    for p in (pa, pb):
        base = max((q_[0] - p[0]) * ww[0] + (q_[1] - p[1]) * ww[1] for q_ in pts)  # 面投影最外点(相对 p)
        foot = (p[0] + ww[0] * (base + 1.0), p[1] + ww[1] * (base + 1.0))
        mid_ = ((pa[0] + pb[0]) / 2, (pa[1] + pb[1]) / 2)
        # 尺寸线上的点: p 沿 w 推到 off(off 相对中点, 换算到 p)
        k = off - ((p[0] - mid_[0]) * ww[0] + (p[1] - mid_[1]) * ww[1])
        e = (p[0] + ww[0] * k, p[1] + ww[1] * k)
        line(foot, (e[0] + ww[0] * 2.0, e[1] + ww[1] * 2.0))
        ends.append(e)
    line(ends[0], ends[1])
    for e, sg in ((ends[0], 1), (ends[1], -1)):
        tip = s(e)
        b1 = s((e[0] + u[0] * ar * sg + ww[0] * aw, e[1] + u[1] * ar * sg + ww[1] * aw))
        b2 = s((e[0] + u[0] * ar * sg - ww[0] * aw, e[1] + u[1] * ar * sg - ww[1] * aw))
        PAGE_SVG.append('<path d="M %.3f %.3f L %.3f %.3f L %.3f %.3f Z" style="fill:#000;stroke:none"/>' % (tip[0], tip[1], b1[0], b1[1], b2[0], b2[1]))
    x, y = s(tc)
    rot = ' transform="rotate(-90 %.3f %.3f)"' % (x, y) if vertical else ""
    PAGE_SVG.append('<text x="%.3f" y="%.3f" style="font-family:%s;font-size:%gpx;text-anchor:middle;dominant-baseline:central;fill:#000"%s>%s</text>'
                    % (x, y, fam, fsz, rot, txt))


rep["added_failed"] = []
for it_ in DIM_EDIT.get("add") or []:
    draw_added_dim(it_)

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
# 技术要求: 人改过的(spec.tech)优先; 否则规范默认 + 配合面粗糙度(IT6 → Ra0.8, IT7 → Ra1.6 …)
fit_codes = []
for f_ in spec.get("fits") or []:
    try:
        fit_codes.append((float(f_["d"]), str(f_["code"]).strip()))
    except Exception:
        pass
tech_default = iso286.numbered(std["tech_requirements"], iso286.part_tech(fit_codes, DIMS["diameter"]))
tech = spec.get("tech") or tech_default
rep["tech"], rep["tech_default"] = tech, tech_default
tech_lines = iso286.wrap(tech, float(spec.get("tech_width_chars", 34)))
if len(tech_lines) > 12:
    rep["warnings"].append("technical requirements: %d lines after wrapping, the box holds 12" % len(tech_lines))
for i in range(12):
    title["tech_%d" % (i + 1)] = tech_lines[i] if i < len(tech_lines) else ""
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
