"""布局中间层(D9a, 2026-11-19 用户提议): 只管"放在哪儿", 不管"标什么"。

纯 Python(不依赖 FreeCAD), 出图编译器(compile_freecad.py)和 agent 自己补的标注都交给它排版。
坐标: 页面毫米, x 向右、**y 向下**(同 SVG / GB.json 的标题栏坐标)。视图内的局部坐标 = 模型毫米、以视图中心为原点、x 右 y 上
(工程图习惯), 换算 page = (cx + x·s, cy − y·s)。

输入 problem(dict):
  paper  [w, h]; area [x0, y0, x1, y1] = 可画区域(内框); keepout [[x0, y0, x1, y1], …] = 禁区(标题栏 / 技术要求框 …)
  scale  可选, 固定比例(否则从 std.layout.scales 里挑能全部放下的最大的)
  views  [{id, w, h (模型毫米), rel?: {of, dir: right|left|below|above}, free?: bool}]
         有 rel 的按投影关系对齐(同行同中心 y / 同列同中心 x); free = 独立摆放(等轴测、局部放大); 第一个无 rel 且非 free 的是主视图
  dims   线性尺寸, 放在视图外: {id, view, a: [x, y], b: [x, y], dir: x|y, side?: top|bottom|left|right, text, prio?}
         inline 尺寸, 放在视图上(轴的直径): {id, view, kind: "inline", dir: y|x, span: [lo, hi](沿尺寸方向), slide: [lo, hi](可滑动范围,
         垂直于尺寸方向), text}
  labels 引出标注(孔说明 / 形位公差框 / 粗糙度 / 说明): {id, view, anchor: [x, y], text: "…" | ["行1", "行2"], prio?}
输出 solve() → dict:
  scale; views {id: {cx, cy, rect}}; dims {id: {side, lane, line, ext, text_at, text_rect, rot, outside}};
  labels {id: {leader: [[x, y] …], text_rect, text_at, lines}}; unplaced [{id, why}]; conflicts(应为空)
放不下的一律进 unplaced, 绝不默默丢。
"""
import json
import math
import os

HERE = os.path.dirname(os.path.abspath(__file__))


def load_std(name="GB"):
    with open(os.path.join(HERE, "standards", name + ".json"), encoding="utf-8") as f:
        return json.load(f)


# ------------------------------------------------------------------ 几何小工具(矩形 = (x0, y0, x1, y1))
def rect_hit(a, b, pad=0.0):
    return not (a[2] + pad <= b[0] or b[2] + pad <= a[0] or a[3] + pad <= b[1] or b[3] + pad <= a[1])


def rect_inside(r, outer, tol=1e-6):
    return outer[0] - tol <= r[0] and r[2] <= outer[2] + tol and outer[1] - tol <= r[1] and r[3] <= outer[3] + tol


def grow(r, d):
    return (r[0] - d, r[1] - d, r[2] + d, r[3] + d)


def seg_hit(p1, p2, p3, p4):
    """两线段相交(端点相接不算)。"""
    def orient(a, b, c):
        return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    d1, d2 = orient(p3, p4, p1), orient(p3, p4, p2)
    d3, d4 = orient(p1, p2, p3), orient(p1, p2, p4)
    return d1 * d2 < -1e-9 and d3 * d4 < -1e-9


def seg_rect_hit(p, q, r):
    """线段穿过矩形内部。"""
    if rect_inside((min(p[0], q[0]), min(p[1], q[1]), max(p[0], q[0]), max(p[1], q[1])), r):
        return True
    corners = [(r[0], r[1]), (r[2], r[1]), (r[2], r[3]), (r[0], r[3])]
    if any(seg_hit(p, q, corners[i], corners[(i + 1) % 4]) for i in range(4)):
        return True
    inside = lambda pt: r[0] < pt[0] < r[2] and r[1] < pt[1] < r[3]
    return inside(p) or inside(q)


def max_empty_rects(area, keepout):
    """area 去掉 keepout 后的极大空矩形(禁区少, 用边坐标暴力枚举)。"""
    xs = sorted({area[0], area[2]} | {k[i] for k in keepout for i in (0, 2) if area[0] < k[i] < area[2]})
    ys = sorted({area[1], area[3]} | {k[i] for k in keepout for i in (1, 3) if area[1] < k[i] < area[3]})
    cands = []
    for i, x0 in enumerate(xs):
        for x1 in xs[i + 1:]:
            for j, y0 in enumerate(ys):
                for y1 in ys[j + 1:]:
                    r = (x0, y0, x1, y1)
                    if not any(rect_hit(r, k) for k in keepout):
                        cands.append(r)
    out = [r for r in cands if not any(o != r and rect_inside(r, o) for o in cands)]
    return sorted(out, key=lambda r: -(r[2] - r[0]) * (r[3] - r[1]))


import re as _re

# 公差写成上下偏差叠放(用户 2026-11-20: 上下界用上下角标的格式): "⌀18 h7(0/-0.018)" / "12 +0.1/-0.05" →
# (前缀, 上偏差, 下偏差, 后缀, 是否带括号)。对称的 "±0.05" 照常同号写, 不叠。
_TOL_PAREN = _re.compile(r"^(.*?)\s?\(\s*([+\-]?\d+(?:\.\d+)?)\s*/\s*([+\-]?\d+(?:\.\d+)?)\s*\)(.*)$")
_TOL_BARE = _re.compile(r"^(.*?\d)\s+([+\-]\d+(?:\.\d+)?|0)\s*/\s*([+\-]\d+(?:\.\d+)?|0)\s*$")
TOL_SCALE = 0.7      # 偏差字号 = 0.7 × 尺寸字号


def split_tol(s):
    m = _TOL_PAREN.match(s or "")
    if m:
        return m.group(1).replace(" ", ""), m.group(2), m.group(3), m.group(4), True
    m = _TOL_BARE.match(s or "")
    if m:
        return m.group(1), m.group(2), m.group(3), "", False
    return None


class Text:
    def __init__(self, std):
        L, f = std["layout"], std["fonts"]
        self.cjk, self.latin = L["char_w"]["cjk"], L["char_w"]["latin"]
        self.h = f["dim"]
        self.line_h = L["line_h"]

    def plain_width(self, s, h=None, cw=None):
        h = h or self.h
        lat = cw if cw else self.latin
        return sum(h * (self.cjk if ord(ch) > 0x2E80 else lat) for ch in s)

    def width(self, s, h=None, cw=None):
        """文字宽度; 带上下偏差的按叠放算(偏差只占较宽的那一个的 0.7 倍; 用户 2026-11-20: 不加括号)。
        cw = 数字字母的字宽系数(编译器自己画的页面图形是 svg_char_w ≈ 0.5, TechDraw 尺寸是 char_w.latin = 0.85)。"""
        h = h or self.h
        t = split_tol(s)
        if not t:
            return self.plain_width(s, h, cw)
        pre, up, lo, post, paren = t
        w = self.plain_width(pre, h, cw) + TOL_SCALE * max(self.plain_width(up, h, cw), self.plain_width(lo, h, cw)) + self.plain_width(post, h, cw)
        return w + 0.3 * h

    def box(self, lines, h=None):
        h = h or self.h
        lines = lines if isinstance(lines, list) else [lines]
        return max(self.width(s, h) for s in lines), h * self.line_h * (len(lines) - 1) + h, lines


# ------------------------------------------------------------------ 布局
class Layout:
    def __init__(self, problem, std=None):
        self.p = problem
        self.std = std or load_std()
        self.L = self.std["layout"]
        self.D = self.std["dims"]
        self.T = Text(self.std)
        self.area = tuple(problem["area"])
        self.keepout = [tuple(k) for k in problem.get("keepout", [])]
        self.views = {v["id"]: v for v in problem["views"]}
        self.dims = problem.get("dims", [])
        self.labels = problem.get("labels", [])

    # ---- 视图局部 → 页面
    def to_page(self, vid, pt, place, s=None):
        c = place[vid]
        s = c.get("scale", s)   # free 视图可能有自己的比例
        return (c["cx"] + pt[0] * s, c["cy"] - pt[1] * s)

    # ---- 线性尺寸: 选边 + 分层(与位置无关, 只与比例有关)
    def auto_side(self, d):
        v = self.views[d["view"]]
        if d.get("side"):
            return d["side"]
        if d["dir"] == "x":
            my = (d["a"][1] + d["b"][1]) / 2
            return "top" if my > 0 else "bottom"
        mx = (d["a"][0] + d["b"][0]) / 2
        return "right" if mx > 0 else "left"

    def lanes(self, s, bump=None):
        """每个视图每一边: 尺寸按跨度从短到长逐层排, 同层尺寸线区间不重叠(文字位置后面单独找)。
        bump = {dim id: 最低层}(文字找不到位置的尺寸往外挪)。→ {dim id: (side, lane, interval, text_out)}"""
        bump = bump or {}
        out, per = {}, {}
        arrow, pad = self.L["arrow"], self.L["pad"]
        for d in self.dims:
            if d.get("kind", "linear") != "linear":
                continue
            side = self.auto_side(d)
            k = 0 if d["dir"] == "x" else 1
            lo, hi = sorted((d["a"][k] * s, d["b"][k] * s))
            tw = self.T.width(d["text"])
            per.setdefault((d["view"], side), []).append((hi - lo, lo, hi, tw, d))
        for key, items in per.items():
            items.sort(key=lambda t: (t[0], t[1]))
            lanes = []  # 每层已占区间(含放在里面的文字)
            for span, lo, hi, tw, d in items:
                inside = tw + 2 * arrow + 1.0 <= span
                iv = (lo, hi)
                li = bump.get(d["id"], 0)
                while True:
                    while len(lanes) <= li:
                        lanes.append([])
                    if all(iv[1] + pad <= a or b + pad <= iv[0] for a, b in lanes[li]):
                        lanes[li].append(iv)
                        out[d["id"]] = (key[1], li, (lo, hi), not inside)
                        break
                    li += 1
        # 去掉空层(bump 可能跳过某层): 每边按层号重新编号
        for key in per:
            used = sorted({out[d["id"]][1] for _, _, _, _, d in per[key]})
            ren = {l: i for i, l in enumerate(used)}
            for _, _, _, _, d in per[key]:
                sd, l, iv, to = out[d["id"]]
                out[d["id"]] = (sd, ren[l], iv, to)
        return out

    def halos(self, lanes, s=1.0):
        """每个视图四边要留的宽度(尺寸层 + 文字 + 标注行)。"""
        h = {vid: {"top": 0.0, "bottom": 0.0, "left": 0.0, "right": 0.0} for vid in self.views}
        for vid in self.views:          # 段直径文字写在零件外的那一条带(紧贴轮廓, 尺寸层在它外面)
            h[vid]["top"] = self.out_zone(vid)
        for did, (side, lane, _, _) in lanes.items():
            d = next(x for x in self.dims if x["id"] == did)
            need = self.out_zone(d["view"], side) + self.D["gap"] + lane * self.D["step"] + self.T.h + 1.0
            h[d["view"]][side] = max(h[d["view"]][side], need)
        # 预留(编译器自己画的东西: 坐标尺寸条、剖视名、剖切符号箭头 …): 叠在尺寸层外面
        for rsv in self.p.get("reserve", []):
            if rsv["view"] in h:
                h[rsv["view"]][rsv["side"]] += rsv["depth"]
        # 标注行(形位框格 / 基准 / 粗糙度): 叠在最外面; 行数按宽度在"视图宽 + 两边各 25"里装箱估计
        for (vid, side), labs in self.row_groups(s).items():
            limit = self.views[vid]["w"] * s + 50.0
            rows = self.pack_rows([self.label_w(l) for l in labs], limit)
            h[vid][side] += rows * (self.row_h(labs) + self.L.get("row_gap", 2.5)) + 3.0
        return h

    # ---- 标注行: 形位框格 / 基准 / 粗糙度排在视图上下的整齐行里, 引线尽量竖直(用户 2026-11-20: 标注密时不能乱拉长引线)
    def out_zone(self, vid, side="top"):
        """段直径文字写在零件外(outside: 尺寸线向上延长, 竖排文字在延长段上)要的带宽 = 最长的文字 + 3; 只在上侧。"""
        if side != "top":
            return 0.0
        ws = [self.T.width(d["text"], cw=d.get("cw")) for d in self.dims if d.get("kind") == "inline" and d.get("outside") and d["view"] == vid]
        return (max(ws) + 4.0) if ws else 0.0

    def long_inline(self, s):
        """这个比例下文字比自己的尺寸线还长的 inline ⌀(如 ⌀20 g7(-0.007/-0.028) 在 ⌀20 段上): 线留在零件上, 文字进标注行。
        outside 的(文字本来就写在零件外)不算。"""
        out = []
        for d in self.dims:
            if d.get("kind") == "inline" and d["dir"] == "y" and not d.get("outside"):
                c_scale = s
                if self.T.width(d["text"]) + 2 * self.L["arrow"] > (max(d["span"]) - min(d["span"])) * c_scale:
                    out.append(d["id"])
        return out

    def row_labels(self, s=None):
        base = [l for l in self.labels if l.get("row")]
        if s is None:
            return base
        extra = []
        for did in self.long_inline(s):
            d = next(x for x in self.dims if x["id"] == did)
            # 带叠放偏差的文字比一行字高 → 框高 1.35 倍(下偏差不压肩线)
            hh = self.T.h * (1.35 if split_tol(d["text"]) else 1.0)
            extra.append({"id": did + "#t", "view": d["view"], "text": d["text"], "row": True, "attach_dim": did, "prio": 2, "dimtext": True, "h": hh})
        return base + extra

    def label_w(self, lb):
        w, _, _ = self.T.box(lb["text"])
        return (lb.get("w") or w) + lb.get("pad_left", 0.0)

    def row_h(self, labs):
        return max([l.get("h") or self.T.h for l in labs] or [self.T.h])

    @staticmethod
    def pack_rows(widths, limit, gap=3.0):
        rows, used = 0, None
        for w in widths:
            if used is None or used + gap + w > limit:
                rows += 1
                used = w
            else:
                used += gap + w
        return rows

    def row_groups(self, s=1.0):
        """标签 → (视图, 上 / 下): 只许在给了锚点的那侧; 两侧都行的, 按宽度均衡(宽的先分); 挂在同一条尺寸线上的两个标签各占一端。按比例缓存。"""
        cache = self.__dict__.setdefault("_row_groups", {})
        if s in cache:
            return cache[s]
        used, out, ends = {}, {}, {}
        labs = self.row_labels(s)
        # 挂尺寸线的先分(端点有限), 再按宽度
        for lb in sorted(labs, key=lambda l: (0 if l.get("attach_dim") else 1, -self.label_w(l))):
            sides = [sd for sd in ("top", "bottom") if sd in (lb.get("anchors") or {}) or (lb.get("attach_dim") and sd in lb.get("sides", ("top", "bottom")))]
            if lb.get("attach_dim"):
                free = [sd for sd in sides if (lb["attach_dim"], sd) not in ends]
                sides = free or sides
            if not sides:
                continue
            pref = "bottom" if lb.get("dimtext") and "bottom" in sides and (lb.get("attach_dim"), "bottom") not in ends else None
            sd = pref or min(sides, key=lambda x: used.get((lb["view"], x), 0.0))
            used[(lb["view"], sd)] = used.get((lb["view"], sd), 0.0) + self.label_w(lb)
            out.setdefault((lb["view"], sd), []).append(lb)
            if lb.get("attach_dim"):
                ends[(lb["attach_dim"], sd)] = lb["id"]
        cache[s] = out
        return out

    def realize_rows(self, place, s, dims, dim_texts):
        """标注行(边界标注): 每个视图上 / 下, 框按要素的左右顺序排成一行(放不下才第二行), 横向可以错开;
        引线 = 要素 → 竖直出尺寸区 → 斜线到框。顺序不变 → 引线互不交叉。→ (placed, unplaced)"""
        placed, unplaced = {}, []
        pad = self.L["pad"]
        gap = self.L.get("row_gap", 2.5)
        hgap = 3.0
        for (vid, side), labs in self.row_groups(s).items():
            c = place[vid]
            r = c["rect"]
            sg = -1 if side == "top" else 1

            def page(pt):
                return (c["cx"] + pt[0] * c["scale"], c["cy"] - pt[1] * c["scale"])

            # 行从这一侧最外面的尺寸(线 / 文字 / 界线)之外开始
            ys = [r[1] if side == "top" else r[3]]
            for d in dims.values():
                if d.get("_view") != vid:
                    continue
                pts = list(d.get("line_ext") or d["line"]) + [p for e in d.get("ext", []) for p in e]
                tr = d["text_rect"]
                cand = [p[1] for p in pts] + ([] if d.get("notext") else [tr[1], tr[3]])
                ys += [y for y in cand if (y < r[1] if side == "top" else y > r[3])]
            edge = (min(ys) - 2.0) if side == "top" else (max(ys) + 2.0)
            rh = self.row_h(labs)
            # 行的左右边界: 可画区, 且不伸到同一高度旁边的视图上
            xlo, xhi = self.area[0], self.area[2]
            band = (edge - 3 * (rh + gap), edge) if side == "top" else (edge, edge + 3 * (rh + gap))
            for oid, oc in place.items():
                if oid == vid:
                    continue
                orr = grow(oc["rect"], 4.0)
                if orr[3] < band[0] or orr[1] > band[1]:
                    continue
                if orr[0] >= r[2] - 1e-6:
                    xhi = min(xhi, orr[0])
                elif orr[2] <= r[0] + 1e-6:
                    xlo = max(xlo, orr[2])
            # 每个标签的起点(引线从要素出来那一点)和竖直段的 x
            items = []
            for lb in labs:
                w = self.label_w(lb)
                h = lb.get("h") or self.T.h
                ext_line = None
                if lb.get("attach_dim") and lb["attach_dim"] in dims:
                    ln = dims[lb["attach_dim"]].get("line_ext") or dims[lb["attach_dim"]]["line"]
                    ends = sorted(ln, key=lambda p: p[1])
                    a = ends[0] if side == "top" else ends[-1]       # 尺寸线这一端: 与尺寸线对齐 = 指轴线
                    pre = [a]
                    lx = a[0]
                elif lb.get("anchors") and side in lb["anchors"]:
                    a = page(lb["anchors"][side])
                    if lb.get("approach") == "h":
                        # 边视的平面: 箭头 / 三角落在延长线上(零件外), 水平进入, 再竖直出去
                        a0 = a
                        a = (a[0], a[1] + sg * lb.get("ext_out", 3.0))
                        ext_line = [a0, (a[0], a[1] + sg * 1.5)]
                        dx = 7.0 if a[0] >= (r[0] + r[2]) / 2 else -7.0
                        lx = a[0] + dx
                        pre = [a, (lx, a[1])]
                    else:
                        if lb.get("slide"):
                            x0p, x1p = sorted((page([lb["slide"][0], 0])[0], page([lb["slide"][1], 0])[0]))
                        else:
                            x0p = x1p = a[0]
                        mid = (x0p + x1p) / 2
                        n = int((x1p - x0p) / 0.5) + 1
                        xs_c = sorted({min(max(mid + k * 0.5 * t, x0p), x1p) for k in range(n) for t in (1, -1)}, key=lambda x: abs(x - mid))
                        av = lb.get("avoid_dim")
                        if av and av in dims:            # 与本段 ⌀ 尺寸线错开 ≥ 4 mm(对齐 = 指轴线)
                            lxd = dims[av]["line"][0][0]
                            xs_c = [x for x in xs_c if abs(x - lxd) >= 4.0] or xs_c
                        lx = None
                        for x in xs_c:                   # 竖直段不许穿过尺寸文字
                            if not any(seg_rect_hit((x, a[1]), (x, edge), t) for t in dim_texts):
                                lx = x
                                break
                        lx = xs_c[0] if lx is None else lx
                        a = (lx, a[1])
                        pre = [a]
                else:
                    unplaced.append({"id": lb["id"], "why": "no anchor on this side"})
                    continue
                items.append({"lb": lb, "w": w, "h": h, "lx": lx, "pre": pre, "ext": ext_line})
            items.sort(key=lambda it: it["lx"])
            # 分行: 一行装得下就一行, 否则隔一个放到第二行(保持各行内的左右顺序)
            avail = xhi - xlo

            def spread_(row):
                xs = [it["lx"] - it["w"] / 2 for it in row]
                for i in range(len(row)):
                    lo_ = xlo if i == 0 else xs[i - 1] + row[i - 1]["w"] + hgap
                    xs[i] = max(xs[i], lo_)
                for i in range(len(row) - 1, -1, -1):
                    hi_ = xhi - row[i]["w"] if i == len(row) - 1 else xs[i + 1] - hgap - row[i]["w"]
                    xs[i] = min(xs[i], hi_)
                return xs

            def drift_(rows):
                return sum(abs(x + it["w"] / 2 - it["lx"]) for row in rows for it, x in zip(row, spread_(row)))

            tot = sum(it["w"] for it in items) + hgap * max(0, len(items) - 1)
            one, two = [items], [items[0::2], items[1::2]]
            # 一行放得下也要看框离要素多远: 两行能把横向偏移砍掉一大半就用两行(引线短, 不扎堆)
            rows_ = one if tot <= avail and (len(items) < 3 or drift_(one) <= 1.6 * drift_(two) + 10.0) else two
            # 两行时: 里行散开时给外行每个标签留一条窄缝(按左右顺序插在里行中间), 外行引线从缝里竖直穿过, 不压里行的框
            slot_w = 3.0
            if len(rows_) == 2:
                seq = sorted([(it, 0) for it in rows_[0]] + [(it, 1) for it in rows_[1]], key=lambda p: p[0]["lx"])
                ws_ = [it["w"] if r_ == 0 else slot_w for it, r_ in seq]
                xs_ = [it["lx"] - w / 2 for (it, _), w in zip(seq, ws_)]
                for i in range(len(seq)):
                    lo_ = xlo if i == 0 else xs_[i - 1] + ws_[i - 1] + hgap
                    xs_[i] = max(xs_[i], lo_)
                for i in range(len(seq) - 1, -1, -1):
                    hi_ = xhi - ws_[i] if i == len(seq) - 1 else xs_[i + 1] - hgap - ws_[i]
                    xs_[i] = min(xs_[i], hi_)
                for (it, r_), x_, w in zip(seq, xs_, ws_):
                    if r_ == 0:
                        it["x0"] = x_
                    else:
                        it["slot"] = x_ + w / 2
                # 外行: 框中心对准各自的缝
                for it in rows_[1]:
                    it["aim"] = it["slot"]
                xs1 = [it["aim"] - it["w"] / 2 for it in rows_[1]]
                for i in range(len(rows_[1])):
                    lo_ = xlo if i == 0 else xs1[i - 1] + rows_[1][i - 1]["w"] + hgap
                    xs1[i] = max(xs1[i], lo_)
                for i in range(len(rows_[1]) - 1, -1, -1):
                    hi_ = xhi - rows_[1][i]["w"] if i == len(rows_[1]) - 1 else xs1[i + 1] - hgap - rows_[1][i]["w"]
                    xs1[i] = min(xs1[i], hi_)
                for it, x_ in zip(rows_[1], xs1):
                    it["x0"] = x_
            else:
                for it, x_ in zip(rows_[0], spread_(rows_[0])):
                    it["x0"] = x_
            by1s = [(edge - 6.0 - ri * (rh + gap)) if side == "top" else (edge + 6.0 + ri * (rh + gap) + rh) for ri in range(len(rows_))]
            for ri, row in enumerate(rows_):
                by1 = by1s[ri]
                for it in row:
                    x0 = it["x0"]
                    lb = it["lb"]
                    h = it["h"]
                    box = (x0, by1 - h, x0 + it["w"], by1) if side == "top" else (x0, by1 - rh, x0 + it["w"], by1 - rh + h)
                    bedge = box[3] if side == "top" else box[1]
                    if ri == 0:
                        att = min(max(it["lx"], box[0] + 1.0), box[2] - 1.0)
                        lead = it["pre"] + [(it["lx"], edge), (att, bedge)]
                        if lb.get("dimtext") and side == "bottom":
                            # 文字 + 肩线(肩线在框底): 引线从上面来, 沿文字外侧落到肩线一端, 不穿过文字
                            ex_ = box[0] - 0.6 if it["lx"] <= (box[0] + box[2]) / 2 else box[2] + 0.6
                            lead = it["pre"] + [(it["lx"], edge), (ex_, box[1]), (ex_, box[3])]
                    else:
                        # 外行: 要素 → 竖直出尺寸区 → 斜到里行的缝 → 竖直穿过里行 → 进框
                        sx = it["slot"]
                        b0 = by1s[0]
                        in0 = (b0 + 1.0) if side == "top" else (b0 - rh - 1.0)    # 里行靠要素一侧的外沿
                        out0 = (b0 - rh - 1.0) if side == "top" else (b0 + 1.0)
                        att = min(max(sx, box[0] + 1.0), box[2] - 1.0)
                        lead = it["pre"] + [(it["lx"], edge), (sx, in0), (sx, out0), (att, bedge)]
                        if lb.get("dimtext") and side == "bottom":
                            ex_ = box[0] - 0.6 if sx <= (box[0] + box[2]) / 2 else box[2] + 0.6
                            lead = it["pre"] + [(it["lx"], edge), (sx, in0), (sx, out0), (ex_, box[1]), (ex_, box[3])]
                    bad = None
                    if x0 < xlo - 0.5 or x0 + it["w"] > xhi + 0.5 or not rect_inside(box, self.area) or any(rect_hit(box, k) for k in self.keepout):
                        bad = "no room in the annotation rows"
                    elif any(rect_hit(box, pr, pad) for cc in place.values() for pr in cc.get("parts", [cc["rect"]])):
                        bad = "annotation row runs into a view"
                    if bad:
                        unplaced.append({"id": lb["id"], "why": bad})
                        continue
                    flags = []
                    if any(seg_rect_hit(p, q, t) for p, q in zip(lead, lead[1:]) for t in dim_texts):
                        flags.append("leader crosses a dimension text")
                    placed[lb["id"]] = {"leader": lead, "text_rect": box, "text_at": (box[0] + 0.5, box[1]),
                                        "lines": [lb["text"]] if isinstance(lb["text"], str) else lb["text"],
                                        "cost": 0.0, "flags": flags, "side": side, "row": True, "ext_line": it["ext"]}
        # 第二行的引线可能穿过第一行的框: 查出来记为难看(不静默)
        boxes = {k: v["text_rect"] for k, v in placed.items()}
        for k, v in placed.items():
            ld = v["leader"]
            for k2, b2 in boxes.items():
                if k2 != k and any(seg_rect_hit(p, q, b2) for p, q in zip(ld, ld[1:])):
                    v["flags"].append("leader crosses label " + k2)
        return placed, unplaced

    def reserved_rects(self, place):
        """预留区域(页面矩形): 标签不进去。"""
        out = []
        for rsv in self.p.get("reserve", []):
            c = place.get(rsv["view"])
            if not c:
                continue
            r, dep = c["rect"], rsv["depth"]
            out.append({"top": (r[0], r[1] - dep, r[2], r[1]), "bottom": (r[0], r[3], r[2], r[3] + dep),
                        "left": (r[0] - dep, r[1], r[0], r[3]), "right": (r[2], r[1], r[2] + dep, r[3])}[rsv["side"]])
        return out

    # ---- 视图摆放
    def grid(self):
        """投影关系 → (行, 列)。"""
        pos, free = {}, []
        main = next((v["id"] for v in self.p["views"] if not v.get("rel") and not v.get("free")), None)
        if main is None:
            raise ValueError("no main view (a view without rel and not free)")
        pos[main] = (0, 0)
        pending = [v for v in self.p["views"] if v.get("rel")]
        for _ in range(len(pending) + 1):
            for v in list(pending):
                of = v["rel"]["of"]
                if of in pos:
                    r, c = pos[of]
                    dr, dc = {"right": (0, 1), "left": (0, -1), "below": (1, 0), "above": (-1, 0)}[v["rel"]["dir"]]
                    if (r + dr, c + dc) in pos.values():
                        raise ValueError("views %s and %s claim the same grid cell"
                                         % (v["id"], next(k for k, p in pos.items() if p == (r + dr, c + dc))))
                    pos[v["id"]] = (r + dr, c + dc)
                    pending.remove(v)
        if pending:
            raise ValueError("view relation chain broken: " + ", ".join(v["id"] for v in pending))
        free = [v["id"] for v in self.p["views"] if v.get("free")]
        return pos, free

    def place_views(self, s, halo):
        """主视图组按网格对齐 + 余量均分, 放进最大的空矩形; free 视图放进剩下的空间。→ (place, ok, why)"""
        pos, free = self.grid()
        gap = self.L["view_gap"]
        rows = sorted({r for r, _ in pos.values()})
        cols = sorted({c for _, c in pos.values()})
        # 每列: 中心线左 / 右需要的宽度; 每行: 中心线上 / 下需要的高度
        cl = {c: 0.0 for c in cols}; cr = dict(cl)
        rt = {r: 0.0 for r in rows}; rb = dict(rt)
        for vid, (r, c) in pos.items():
            v, hh = self.views[vid], halo[vid]
            cl[c] = max(cl[c], v["w"] * s / 2 + hh["left"]); cr[c] = max(cr[c], v["w"] * s / 2 + hh["right"])
            rt[r] = max(rt[r], v["h"] * s / 2 + hh["top"]); rb[r] = max(rb[r], v["h"] * s / 2 + hh["bottom"])
        need_w = sum(cl[c] + cr[c] for c in cols) + gap * (len(cols) - 1)
        need_h = sum(rt[r] + rb[r] for r in rows) + gap * (len(rows) - 1)
        best = None
        for mer in max_empty_rects(self.area, self.keepout):
            mw, mh = mer[2] - mer[0], mer[3] - mer[1]
            if need_w + 2 * self.L["edge_gap"] > mw or need_h + 2 * self.L["edge_gap"] > mh:
                continue
            # 余量均分(列间 / 行间 / 两端), 有上限, 多出来的整体居中; free 视图放不进剩余空间时: 主视图组靠一侧给它让位,
            # 还不行就把 free 视图缩小(等轴测 / 局部放大本来就常用别的比例)
            for mode in ("spread", "left", "top"):
                ex = mw - need_w
                ey = mh - need_h
                if mode == "spread":
                    gx = min(ex / (len(cols) + 1), self.L["max_gap"])
                    gy = min(ey / (len(rows) + 1), self.L["max_gap"])
                    x = mer[0] + (mw - (need_w + gx * (len(cols) - 1))) / 2
                    y = mer[1] + (mh - (need_h + gy * (len(rows) - 1))) / 2
                elif mode == "left":
                    gx, gy = 0.0, min(ey / (len(rows) + 1), self.L["max_gap"])
                    x = mer[0] + self.L["edge_gap"]
                    y = mer[1] + (mh - (need_h + gy * (len(rows) - 1))) / 2
                else:
                    gx, gy = min(ex / (len(cols) + 1), self.L["max_gap"]), 0.0
                    x = mer[0] + (mw - (need_w + gx * (len(cols) - 1))) / 2
                    y = mer[1] + self.L["edge_gap"]
                place, cx = {}, {}
                xx = x
                for c in cols:
                    cx[c] = xx + cl[c]
                    xx += cl[c] + cr[c] + gap + gx
                cy, yy = {}, y
                for r in rows:
                    cy[r] = yy + rt[r]
                    yy += rt[r] + rb[r] + gap + gy
                group = (x, y, xx - gap - gx, yy - gap - gy)
                for vid, (r, c) in pos.items():
                    place[vid] = {"cx": cx[c], "cy": cy[r], "scale": s}
                cap = min([self.views[f].get("max_rel", 1.0) for f in free] or [1.0])
                for fi, fs in enumerate(self.free_scales(s, cap)):
                    ok = self.place_free(place, free, fs, [grow(group, 0)])
                    if ok is not None:
                        score = (fi, 0 if mode == "spread" else 1, -(mw * mh))
                        if best is None or score < best[0]:
                            best = (score, ok)
                        break
        if best is None:
            return None, "views do not fit at scale %g" % s
        place = best[1]
        for vid, c in place.items():
            v, vs = self.views[vid], c["scale"]
            c["rect"] = (c["cx"] - v["w"] * vs / 2, c["cy"] - v["h"] * vs / 2, c["cx"] + v["w"] * vs / 2, c["cy"] + v["h"] * vs / 2)
            # 轮廓(可选): 视图实际占的几块矩形(局部坐标 [x0, y0, x1, y1]); 标签 / 文字可以进包围盒里空着的地方
            parts = v.get("parts")
            if parts:
                c["parts"] = [(c["cx"] + min(p[0], p[2]) * vs, c["cy"] - max(p[1], p[3]) * vs, c["cx"] + max(p[0], p[2]) * vs, c["cy"] - min(p[1], p[3]) * vs) for p in parts]
            else:
                c["parts"] = [c["rect"]]
        return place, None

    def free_scales(self, s, cap=1.0):
        """free 视图可用的比例: 不超过 s × cap(max_rel, 如轴测只作参考 ≤ 0.75 × 主视), 依次取更小的标准比例(最多小到 1/4)。"""
        top = s * cap + 1e-9
        out = [x for x in sorted(set(self.L["scales"] + [s]), reverse=True) if x <= top and x >= s / 4.0 - 1e-9]
        return out or [s * cap]

    def place_free(self, place, free, fs, taken):
        place = {k: dict(v) for k, v in place.items()}
        eg = self.L["edge_gap"]
        for fid in free:
            w, h = self.views[fid]["w"] * fs + 2 * eg, self.views[fid]["h"] * fs + 2 * eg
            spot = None
            for mer in max_empty_rects(self.area, self.keepout + [tuple(t) for t in taken]):
                if w <= mer[2] - mer[0] and h <= mer[3] - mer[1]:
                    # 最大的空矩形里居中
                    spot = ((mer[0] + mer[2]) / 2, (mer[1] + mer[3]) / 2)
                    break
            if spot is None:
                return None
            place[fid] = {"cx": spot[0], "cy": spot[1], "scale": fs}
            taken.append((spot[0] - w / 2, spot[1] - h / 2, spot[0] + w / 2, spot[1] + h / 2))
        return place

    # ---- 落线性尺寸: 先定全部尺寸线 / 界线, 再逐个给文字找位置(避开所有线、别的文字、视图)
    def realize_dims(self, place, lanes, s):
        """→ (out, failed): failed = 文字找不到位置的尺寸 id(调用方把它往外挪一层再来)。"""
        out = {}
        g, st, eg, eo = self.D["gap"], self.D["step"], self.L["ext_gap"], self.L["ext_over"]
        th, pad = self.T.h, self.L["pad"]
        for d in self.dims:
            if d.get("kind", "linear") != "linear" or d["id"] not in lanes:
                continue
            side, lane, (lo, hi), text_out = lanes[d["id"]]
            r = place[d["view"]]["rect"]
            A, B = self.to_page(d["view"], d["a"], place, s), self.to_page(d["view"], d["b"], place, s)
            off = self.out_zone(d["view"], side) + g + lane * st
            if d["dir"] == "x":
                y = r[1] - off if side == "top" else r[3] + off
                x0, x1 = sorted((A[0], B[0]))
                sgn = -1 if side == "top" else 1
                out[d["id"]] = {"_view": d["view"], "side": side, "lane": lane, "line": [(x0, y), (x1, y)], "rot": 0, "outside": text_out, "text": d["text"],
                                "ext": [[(p[0], p[1] + sgn * eg), (p[0], y + sgn * eo)] for p in (A, B)]}
            else:
                x = r[0] - off if side == "left" else r[2] + off
                y0, y1 = sorted((A[1], B[1]))
                sgn = -1 if side == "left" else 1
                out[d["id"]] = {"_view": d["view"], "side": side, "lane": lane, "line": [(x, y0), (x, y1)], "rot": -90, "outside": text_out, "text": d["text"],
                                "ext": [[(p[0] + sgn * eg, p[1]), (x + sgn * eo, p[1])] for p in (A, B)]}
        # 线 → 细长矩形(文字不能压别人的线)
        def thin(p, q, w=0.35):
            return (min(p[0], q[0]) - w, min(p[1], q[1]) - w, max(p[0], q[0]) + w, max(p[1], q[1]) + w)
        lines = {k: [thin(*d["line"])] + [thin(*e) for e in d["ext"]] for k, d in out.items()}
        view_rects = [pr for c in place.values() for pr in c["parts"]]
        # 文字放不进段里的 ⌀(线在段中间, 文字进标注行): 它的引线沿尺寸线竖直穿出视图 → 尺寸文字给这条通道让路
        keep_clear = []
        for did in self.long_inline(s):
            dd = next(x for x in self.dims if x["id"] == did)
            cc = place[dd["view"]]
            x = cc["cx"] + (sum(dd["slide"]) / 2) * cc["scale"]
            keep_clear.append((x - 0.8, self.area[1], x + 0.8, cc["rect"][1]))
            keep_clear.append((x - 0.8, cc["rect"][3], x + 0.8, self.area[3]))
        texts, failed = [], []
        for k in sorted(out, key=lambda k: (out[k]["lane"], out[k]["line"][0])):
            d = out[k]
            tw = self.T.width(d["text"])
            (p, q) = d["line"]
            cands = []
            if d["rot"] == 0:
                y, x0, x1 = p[1], p[0], q[0]
                away = -1 if d["side"] == "top" else 1   # 离开视图的方向
                above = (y - 0.6 - th, y - 0.6)
                below = (y + 0.6, y + 0.6 + th)
                mid = (x0 + x1) / 2
                if not d["outside"]:
                    cands.append(("mid", (mid - tw / 2, above[0], mid + tw / 2, above[1])))
                cands += [("right", (x1 + 2.0, above[0], x1 + 2.0 + tw, above[1])), ("left", (x0 - 2.0 - tw, above[0], x0 - 2.0, above[1]))]
                # 错开: 文字挪到线的另一侧(外侧), 居中 / 左右
                yo = below if away > 0 else (y - 0.6 - th - (th + 1.2), y - 0.6 - (th + 1.2))
                cands += [("mid2", (mid - tw / 2, yo[0], mid + tw / 2, yo[1])), ("right2", (x1 + 2.0, yo[0], x1 + 2.0 + tw, yo[1])), ("left2", (x0 - 2.0 - tw, yo[0], x0 - 2.0, yo[1]))]
            else:
                x, y0, y1 = p[0], p[1], q[1]
                left = (x - 0.6 - th, x - 0.6)
                mid = (y0 + y1) / 2
                if not d["outside"]:
                    cands.append(("mid", (left[0], mid - tw / 2, left[1], mid + tw / 2)))
                cands += [("up", (left[0], y0 - 2.0 - tw, left[1], y0 - 2.0)), ("down", (left[0], y1 + 2.0, left[1], y1 + 2.0 + tw))]
                right = (x + 0.6, x + 0.6 + th)
                cands += [("mid2", (right[0], mid - tw / 2, right[1], mid + tw / 2))]
            own = set(range(len(lines[k])))
            chosen = None
            # 第一遍还要给段直径的引线通道让路, 都不行再放宽(那条引线会被标为难看, 不会默默压字)
            for strict in (True, False):
                for name, rr in cands:
                    if not rect_inside(rr, self.area) or any(rect_hit(rr, kk) for kk in self.keepout):
                        continue
                    if any(rect_hit(rr, vr, pad) for vr in view_rects):
                        continue
                    if any(rect_hit(rr, t, pad) for t in texts):
                        continue
                    if any(rect_hit(rr, ln) for kk, ls in lines.items() if kk != k for ln in ls):
                        continue
                    if strict and any(rect_hit(rr, kc) for kc in keep_clear):
                        continue
                    if any(rect_hit(rr, ln) for i, ln in enumerate(lines[k]) if i > 0):   # 自己的界线
                        continue
                    chosen = (name, rr)
                    break
                if chosen:
                    break
            if chosen is None:
                failed.append(k)
                chosen = cands[0]
            name, rr = chosen
            d["text_rect"] = rr
            d["text_pos"] = name
            d["text_at"] = ((rr[0] + rr[2]) / 2, (rr[1] + rr[3]) / 2)
            # 文字放在外面时尺寸线延长到文字下面(GB)
            if name.startswith(("right", "left", "up", "down")):
                if d["rot"] == 0:
                    d["line_ext"] = [(min(p[0], rr[0]), p[1]), (max(q[0], rr[2]), p[1])]
                else:
                    d["line_ext"] = [(p[0], min(p[1], rr[1])), (p[0], max(q[1], rr[3]))]
            if name.endswith("2") and d["rot"] == 0:
                # 错开到另一侧的文字: 一小段竖线连到尺寸线
                d["tick"] = [((rr[0] + rr[2]) / 2 if name == "mid2" else (rr[0] if name == "right2" else rr[2]), p[1]),
                             ((rr[0] + rr[2]) / 2 if name == "mid2" else (rr[0] if name == "right2" else rr[2]), rr[1] if rr[1] > p[1] else rr[3])]
            texts.append(rr)
        return out, failed

    # ---- inline 尺寸(视图上, 如轴的各段直径): 在 slide 范围里滑动, 文字互不重叠
    def realize_inline(self, place, s, taken_text, lines=()):
        out, unplaced = {}, []
        th, pad = self.T.h, self.L["pad"]
        items = [d for d in self.dims if d.get("kind") == "inline"]
        items.sort(key=lambda d: (d["slide"][0] + d["slide"][1]) / 2)
        for d in items:
            tw = self.T.width(d["text"])
            c = place[d["view"]]
            lo, hi = sorted(d["span"])
            a, b = sorted(d["slide"])
            mid = (a + b) / 2
            # outside(用户 2026-11-20: 轴径拉到轴外面标): 尺寸线在段上(避开视图里的内部轮廓线), 向上延长出零件, 竖排文字(字头朝左)写在延长段上
            if d.get("outside") and d["dir"] == "y":
                tw = self.T.width(d["text"], cw=d.get("cw"))
                avoid = [c["cx"] + ax * c["scale"] for ax in (self.views[d["view"]].get("inline_avoid") or [])]
                steps = int(max(0, (b - a) * s) / 0.5) + 1
                cands = sorted({min(max(mid + k * 0.5 / s * sgn, a), b) for k in range(steps) for sgn in (1, -1)}, key=lambda t: abs(t - mid))
                # 离内部轮廓线越远越好(到 3 mm 就够), 同样远时靠段中间 —— 只求"不贴线"会落在两条线之间的窄缝里
                clr = lambda t: min([abs(c["cx"] + t * c["scale"] - xa) for xa in avoid] or [99.0])  # noqa: E731
                ok_t = sorted([t for t in cands if clr(t) >= 1.2] or cands, key=lambda t: (-min(clr(t), 3.0), abs(t - mid)))
                y0, y1 = c["cy"] - hi * s, c["cy"] - lo * s      # 上 / 下轮廓
                done = None
                for up in (True, False):
                    for t in ok_t:
                        x = c["cx"] + t * c["scale"]
                        ty1 = (y0 - 2.0) if up else (y1 + 2.0 + tw)
                        ty0 = ty1 - tw
                        for sd in (-1, 1):                         # 文字在延长线左侧(GB 首选)/ 右侧
                            tx = x - 0.6 - th / 2 if sd < 0 else x + 0.6 + th / 2
                            trect = (tx - th / 2, ty0, tx + th / 2, ty1)
                            if not rect_inside(trect, self.area) or any(rect_hit(trect, u, pad) for u in taken_text):
                                continue
                            if any(rect_hit(trect, pr, 0.3) for cc in place.values() for pr in cc.get("parts", [cc["rect"]])):
                                continue
                            ext_ = ((x, y0), (x, ty0 - 1.0)) if up else ((x, y1), (x, ty1 + 1.0))
                            if any(seg_hit(ext_[0], ext_[1], p_, q_) for p_, q_ in lines):
                                continue
                            done = {"line": [(x, y0), (x, y1)], "line_ext": [(x, min(y0, ext_[1][1])), (x, max(y1, ext_[1][1]))],
                                    "text_at": (tx, (ty0 + ty1) / 2), "rot": -90, "text_rect": trect, "text": d["text"], "at": t,
                                    "kind": "inline", "outside": True, "_view": d["view"]}
                            break
                        if done:
                            break
                    if done:
                        break
                if done:
                    out[d["id"]] = done
                    taken_text.append(done["text_rect"])
                else:
                    unplaced.append({"id": d["id"], "why": "no room for the diameter text outside the part"})
                continue
            # 文字比尺寸线还长(⌀20 g7(-0.007/-0.028) 在 ⌀20 段上): 线留在零件上, 文字进标注行(挂在这条尺寸线一端, 见 row_labels)
            if d["id"] in self.long_inline(s):
                x = c["cx"] + mid * c["scale"]
                y0, y1 = c["cy"] - hi * s, c["cy"] - lo * s
                out[d["id"]] = {"line": [(x, y0), (x, y1)], "text_at": (x, y0), "rot": -90, "text_rect": (x, y0, x, y0), "text": "",
                                "at": mid, "kind": "inline", "notext": True, "_view": d["view"]}
                continue
            steps = int(max(0, (b - a) * s) / 0.5) + 1
            cands = sorted({min(max(mid + k * 0.5 / s * sgn, a), b) for k in range(steps) for sgn in (1, -1)}, key=lambda t: abs(t - mid))
            done = None
            for t in cands:
                # 文字在尺寸线左 / 右(上 / 下)两侧都试: 贴着视图边的段, 一侧会伸出可画区
                for side in (-1, 1):
                    if d["dir"] == "y":
                        x = c["cx"] + t * c["scale"]
                        y0, y1 = c["cy"] - hi * s, c["cy"] - lo * s
                        cyy = (y0 + y1) / 2
                        tx = x - 0.6 - th / 2 if side < 0 else x + 0.6 + th / 2
                        trect = (tx - th / 2, cyy - tw / 2, tx + th / 2, cyy + tw / 2)
                        rec = {"line": [(x, y0), (x, y1)], "text_at": (tx, cyy), "rot": -90}
                    else:
                        y = c["cy"] - t * s
                        x0, x1 = c["cx"] + lo * s, c["cx"] + hi * s
                        cxx = (x0 + x1) / 2
                        ty = y - 0.6 - th / 2 if side < 0 else y + 0.6 + th / 2
                        trect = (cxx - tw / 2, ty - th / 2, cxx + tw / 2, ty + th / 2)
                        rec = {"line": [(x0, y), (x1, y)], "text_at": (cxx, ty), "rot": 0}
                    if not rect_inside(trect, self.area):
                        continue
                    if not any(rect_hit(trect, u, pad) for u in taken_text):
                        rec.update({"text_rect": trect, "text": d["text"], "at": t, "kind": "inline", "_view": d["view"]})
                        done = rec
                        break
                if done:
                    break
            if done:
                out[d["id"]] = done
                taken_text.append(done["text_rect"])
            else:
                unplaced.append({"id": d["id"], "why": "inline text has no free spot in its slide range"})
        return out, unplaced

    # ---- 引出标注
    def realize_labels(self, place, s, obstacles, dim_lines, extra=None, dim_texts=()):
        L = self.L
        th, pad = self.T.h, L["pad"]
        view_rects = {vid: c["parts"] for vid, c in place.items()}
        placed, unplaced = {}, []
        order = sorted([l for l in self.labels if not l.get("row")] + list(extra or []), key=lambda l: (-l.get("prio", 0), l["id"]))

        def candidates(lb, others):
            w, h, lines = self.T.box(lb["text"])
            # 固定尺寸的标签(形位公差框格、粗糙度符号 + 值): 调用方给 w / h; pad_left = 文字左边要留的地方(符号)
            if lb.get("w"):
                w = lb["w"]
            if lb.get("h"):
                h = lb["h"]
            w += lb.get("pad_left", 0.0)
            ax, ay = self.to_page(lb["view"], lb["anchor"], place, s)
            cs = []
            own = view_rects[lb["view"]]
            exits = []
            for ang in L["label_angles"]:
                for r in L["label_r"]:
                    cs.append((ang, r))
                # 锚点在视图深处: 沿这个方向走出视图轮廓所需的长度 + 余量
                ux, uy = math.cos(math.radians(ang)), -math.sin(math.radians(ang))
                rr = 0.0
                while rr < 400 and any(o[0] < ax + ux * rr < o[2] and o[1] < ay + uy * rr < o[3] for o in own):
                    rr += 1.0
                exits.append(rr)
                if rr > L["label_r"][0] - 4:
                    for extra in (4.0, 9.0, 16.0, 24.0):   # 长的两档: 越过视图外侧的尺寸线
                        cs.append((ang, rr + extra))
            min_exit[lb["id"]] = min(exits) if exits else 0.0
            # 对齐候选: 与已放标签同一条横线(肩线)
            for o in others.values():
                ky = o["leader"][1][1]
                dy = ky - ay
                for ang in (45, 60, 30, 135, 120, 150, 225, 240, 210, 315, 300, 330):
                    sa = -math.sin(math.radians(ang))
                    if abs(sa) > 1e-6 and dy / sa > 4:
                        cs.append((ang, dy / sa))
            out = []
            for ang, r in cs:
                kx, ky = ax + r * math.cos(math.radians(ang)), ay - r * math.sin(math.radians(ang))
                cs_ = math.cos(math.radians(ang))
                # 肩线朝引线前进的方向; 竖直引线(90° / 270°)两边都行(引线不会穿过自己的文字)
                sides = [True, False] if abs(cs_) < 1e-6 else [cs_ > 0]
                for right in sides:
                    ex = kx + (w + 1.0) if right else kx - (w + 1.0)
                    box = (min(kx, ex), ky - 0.6 - h, max(kx, ex), ky - 0.6)
                    out.append(((ax, ay), (kx, ky), (ex, ky), box, ang, r, lines, w, h))
            return out

        def cost(c, others):
            (a, k, e, box, ang, r, lines, w, h) = c
            if not rect_inside(box, self.area) or any(rect_hit(box, kk) for kk in self.keepout):
                return None
            if any(rect_hit(box, pr, pad) for prs in view_rects.values() for pr in prs):
                return None
            if any(rect_hit(box, ob, pad) for ob in obstacles):
                return None
            # 引线不许穿过尺寸文字(实测: 形位框格的引线压在总长 "96.6" 上)
            if any(seg_rect_hit(a, k, t) or seg_rect_hit(k, e, t) for t in dim_texts):
                return None
            for o in others.values():
                if rect_hit(box, o["text_rect"], pad) or seg_rect_hit(a, k, o["text_rect"]):
                    return None
            sc = r * 0.25 + (0 if ang % 90 == 45 else 1.0 if ang % 30 == 0 else 2.0)
            lb = c_view[0]
            flags = []
            # 朝外: 引线方向与"视图中心 → 锚点"的夹角越小越好; 引线在自己视图里走得越短越好(别横穿零件)
            v = self.views[lb["view"]]
            ox, oy = lb["anchor"][0] / max(v["w"] / 2, 1e-6), -lb["anchor"][1] / max(v["h"] / 2, 1e-6)
            if math.hypot(ox, oy) > 1e-6:
                dx, dy = k[0] - a[0], k[1] - a[1]
                cosd = (dx * ox + dy * oy) / (math.hypot(dx, dy) * math.hypot(ox, oy) + 1e-9)
                sc += math.degrees(math.acos(max(-1.0, min(1.0, cosd)))) * 0.05
            own = view_rects[lb["view"]]
            n_in = sum(1 for i in range(1, 20) if any(o[0] < a[0] + (k[0] - a[0]) * i / 20 < o[2] and o[1] < a[1] + (k[1] - a[1]) * i / 20 < o[3] for o in own))
            # 引线在零件里走的长度: 超出"最短出路"的部分重罚(锚点在视图深处时, 穿过一段零件是免不了的)
            inside = r * n_in / 20
            excess = max(0.0, inside - min_exit.get(lb["id"], 0.0) - 2.0)
            sc += 0.6 * excess + 0.1 * inside
            if excess > self.L.get("ugly_inside", 10.0):
                flags.append("leader runs %.0f mm through the part" % inside)
            # 引线穿过别的视图 / 别的标签引线 / 尺寸线
            for vid, prs in view_rects.items():
                if vid != lb["view"] and any(seg_rect_hit(a, k, pr) for pr in prs):
                    sc += 25
                    flags.append("leader crosses view " + vid)
            for o in others.values():
                if seg_hit(a, k, o["leader"][0], o["leader"][1]) or seg_hit(k, e, o["leader"][0], o["leader"][1]):
                    sc += 30
                    flags.append("leader crosses another leader")
            for p, q in dim_lines:
                if seg_hit(a, k, p, q):
                    sc += 4
            # 对齐奖励: 肩线与别的标签同高 / 文字左边对齐
            for o in others.values():
                if abs(o["leader"][1][1] - k[1]) < 0.3:
                    sc -= 3
                if abs(o["text_rect"][0] - box[0]) < 0.3:
                    sc -= 2
            return sc, flags

        c_view = [None]
        min_exit = {}
        for _pass in range(3):
            for lb in order:
                others = {k: v for k, v in placed.items() if k != lb["id"]}
                c_view[0] = lb
                best = None
                for c in candidates(lb, others):
                    r_ = cost(c, others)
                    if r_ is not None and (best is None or r_[0] < best[0]):
                        best = (r_[0], c, r_[1])
                if best is None:
                    if _pass == 2 and lb["id"] not in placed:
                        unplaced.append({"id": lb["id"], "why": "no free spot for the label"})
                    continue
                (a, k, e, box, ang, r, lines, w, h) = best[1]
                cur = placed.get(lb["id"])
                if cur is None or best[0] < cur["cost"] - 1e-6:
                    placed[lb["id"]] = {"leader": [a, k, e], "text_rect": box, "text_at": (box[0] + 0.5, box[1]), "lines": lines, "cost": best[0], "flags": best[2]}
        return placed, unplaced

    # ---- 总装
    def solve(self):
        scales = [self.p["scale"]] if self.p.get("scale") else self.L["scales"]
        best, first_ok = None, None
        for idx, s in enumerate(scales):
            idx = -idx   # 越往后比例越小: first_ok[0] − idx = 退了几档
            bump, place = {}, None
            for _round in range(12):
                lanes = self.lanes(s, bump)
                halo = self.halos(lanes, s)
                place, why = self.place_views(s, halo)
                if place is None:
                    break
                dims, failed = self.realize_dims(place, lanes, s)
                if not failed:
                    break
                for k in failed:   # 文字没地方 → 往外挪一层
                    bump[k] = lanes[k][1] + 1
            if place is None:
                continue
            un0 = [{"id": k, "why": "dimension text has no free spot"} for k in failed]
            texts = [d["text_rect"] for d in dims.values()]
            lines0 = [tuple(d["line"]) for d in dims.values()] + [tuple(e) for d in dims.values() for e in d.get("ext", [])]
            inl, un1 = self.realize_inline(place, s, list(texts), lines0)
            dims.update(inl)
            # 在段上放不下的 inline(窄台阶的 ⌀): 改成引出标注(引线指到该段轮廓的上沿), 不拖累整张图的比例
            extra = []
            for u in un1:
                d = next(x for x in self.dims if x["id"] == u["id"])
                a, b = sorted(d["slide"])
                anchor = [(a + b) / 2, max(d["span"])] if d["dir"] == "y" else [max(d["span"]), (a + b) / 2]
                extra.append({"id": d["id"], "view": d["view"], "anchor": anchor, "text": d["text"], "prio": 1, "from_inline": True})
            un1 = []
            obstacles = [d["text_rect"] for d in dims.values()] + self.reserved_rects(place)   # inline 文字可能伸出零件轮廓, 也算
            dim_lines = [tuple(d.get("line_ext") or d["line"]) for d in dims.values()] + [tuple(e) for d in dims.values() for e in d.get("ext", [])] \
                + [tuple(d["tick"]) for d in dims.values() if d.get("tick")]
            # 尺寸线本身也是障碍(标签文字不能压线): 线 → 细长矩形
            for p, q in dim_lines:
                obstacles.append((min(p[0], q[0]) - 0.3, min(p[1], q[1]) - 0.3, max(p[0], q[0]) + 0.3, max(p[1], q[1]) + 0.3))
            dtexts = [d["text_rect"] for d in dims.values()] + list(self.p.get("text_obstacles", []))
            rows, un3 = self.realize_rows(place, s, dims, dtexts)
            # 行里的框和竖直引线也是障碍
            for l in rows.values():
                obstacles.append(l["text_rect"])
                for p, q in zip(l["leader"], l["leader"][1:]):
                    obstacles.append((min(p[0], q[0]) - 0.3, min(p[1], q[1]) - 0.3, max(p[0], q[0]) + 0.3, max(p[1], q[1]) + 0.3))
            labels, un2 = self.realize_labels(place, s, obstacles, dim_lines, extra, dtexts + [l["text_rect"] for l in rows.values()])
            labels.update(rows)
            res = {"scale": s, "views": place, "dims": dims, "labels": labels, "unplaced": un0 + un1 + un2 + un3}
            res["conflicts"] = conflicts(res, self)
            # 观感门槛: 有标签引线横穿别的视图 / 与别的引线交叉 / 在零件里走很长(代价高)→ 这个比例太挤, 试更小的
            ugly = [{"id": k, "why": l["flags"]} for k, l in labels.items() if l["flags"]]
            res["ugly"] = ugly
            bad = len(res["unplaced"]) + len(res["conflicts"])
            if not bad and not ugly:
                # 干净的比例: 比最大的"放得下、不重叠"比例只小一档才要; 小得更多就宁可用大比例(引线观感只报告)
                # —— 2026-11-20 编码器轴半剖: 2:1 有一处引线交叉 → 一路退到 1:1, 实际孔说明文字反而叠在一起
                # 例外: "引线在零件里走得长"随比例变短, 缩小确实能治(磁环座内锥: 5:1 走 57 mm, 3:1 合格)→ 照常往下退
                if first_ok is not None and first_ok[0] - idx > 1 and not first_ok[2]:
                    return first_ok[1]
                return res
            if not bad and first_ok is None:
                fixable = all("through the part" in w for u in ugly for w in u["why"])
                first_ok = (idx, res, fixable)
            # 都不干净时: 放不下 / 重叠最少的里面取比例最大的(缩小比例解决不了标注挤 —— 字号不变, 零件变小反而更挤);
            # 引线观感问题只当警告(res["ugly"])
            key = (bad, -s, len(ugly))
            if best is None or key < best[0]:
                best = (key, res)
        if best is not None:
            return best[1]
        return {"scale": None, "views": {}, "dims": {}, "labels": {}, "unplaced": [{"id": "*", "why": "views do not fit at any scale"}], "conflicts": [], "ugly": []}


def conflicts(res, lay):
    """落位后的复查: 文字框之间、文字框与视图(inline 文字除外)、与禁区 / 出界。→ [(a, b)]"""
    items = []
    for k, d in res["dims"].items():
        if d.get("notext"):
            continue   # 文字在标注行里(那边单独检查)
        items.append(("dim:" + k, d["text_rect"], d.get("kind") == "inline"))
    for k, l in res["labels"].items():
        items.append(("label:" + k, l["text_rect"], False))
    out = []
    for i in range(len(items)):
        for j in range(i + 1, len(items)):
            if rect_hit(items[i][1], items[j][1], 0.2):
                out.append((items[i][0], items[j][0]))
    for n, r, inl in items:
        if not rect_inside(r, lay.area):
            out.append((n, "outside area"))
        for kk in lay.keepout:
            if rect_hit(r, kk):
                out.append((n, "keepout"))
        if not inl:
            for vid, c in res["views"].items():
                if any(rect_hit(r, pr, 0.0) for pr in c["parts"]):
                    out.append((n, "view:" + vid))
    vs = list(res["views"].items())
    for i in range(len(vs)):
        for j in range(i + 1, len(vs)):
            if rect_hit(vs[i][1]["rect"], vs[j][1]["rect"], 0.0):
                out.append(("view:" + vs[i][0], "view:" + vs[j][0]))
    return out


def solve(problem, std=None):
    return Layout(problem, std).solve()


# ------------------------------------------------------------------ 调试用 SVG(看排版效果; 不是正式出图)
def render_svg(problem, res, path, std=None):
    std = std or load_std()
    T = Text(std)
    W, H = problem["paper"]
    o = ['<svg xmlns="http://www.w3.org/2000/svg" width="%gmm" height="%gmm" viewBox="0 0 %g %g" font-family="FangSong, SimSun, sans-serif">' % (W, H, W, H),
         '<rect width="100%" height="100%" fill="white"/>']
    a = problem["area"]
    o.append('<rect x="%g" y="%g" width="%g" height="%g" fill="none" stroke="black" stroke-width="0.5"/>' % (a[0], a[1], a[2] - a[0], a[3] - a[1]))
    for k in problem.get("keepout", []):
        o.append('<rect x="%g" y="%g" width="%g" height="%g" fill="#eee" stroke="#999" stroke-width="0.25"/>' % (k[0], k[1], k[2] - k[0], k[3] - k[1]))
    for vid, c in res["views"].items():
        r = c["rect"]
        o.append('<rect x="%.2f" y="%.2f" width="%.2f" height="%.2f" fill="none" stroke="#9ab" stroke-width="0.2" stroke-dasharray="1,1"/>' % (r[0], r[1], r[2] - r[0], r[3] - r[1]))
        for pr in c.get("parts", [r]):
            o.append('<rect x="%.2f" y="%.2f" width="%.2f" height="%.2f" fill="#f4f8ff" stroke="#246" stroke-width="0.5"/>' % (pr[0], pr[1], pr[2] - pr[0], pr[3] - pr[1]))
        o.append('<text x="%.2f" y="%.2f" font-size="3" fill="#69c" text-anchor="middle">%s</text>' % (c["cx"], c["cy"], vid))

    def line(p, q, w=0.25, col="black"):
        o.append('<line x1="%.2f" y1="%.2f" x2="%.2f" y2="%.2f" stroke="%s" stroke-width="%g"/>' % (p[0], p[1], q[0], q[1], col, w))

    def arrow(p, q):
        dx, dy = q[0] - p[0], q[1] - p[1]
        n = math.hypot(dx, dy) or 1
        ux, uy = dx / n, dy / n
        L = std["layout"]["arrow"]
        for tip, sgn in ((p, 1), (q, -1)):
            bx, by = tip[0] + sgn * ux * L, tip[1] + sgn * uy * L
            o.append('<polygon points="%.2f,%.2f %.2f,%.2f %.2f,%.2f" fill="black"/>' % (tip[0], tip[1], bx - uy * 0.5, by + ux * 0.5, bx + uy * 0.5, by - ux * 0.5))

    def text(at, s, rot=0, anchor="middle", h=None):
        h = h or T.h
        o.append('<text x="%.2f" y="%.2f" font-size="%g" text-anchor="%s" dominant-baseline="central" transform="rotate(%g %.2f %.2f)">%s</text>'
                 % (at[0], at[1], h, anchor, rot, at[0], at[1], s.replace("&", "&amp;").replace("<", "&lt;")))

    for k, d in res["dims"].items():
        for e in d.get("ext", []):
            line(e[0], e[1])
        if d.get("line_ext"):
            line(d["line_ext"][0], d["line_ext"][1])
        if d.get("tick"):
            line(d["tick"][0], d["tick"][1])
        line(d["line"][0], d["line"][1])
        arrow(d["line"][0], d["line"][1])
        text(d["text_at"], d["text"], d.get("rot", 0))
    for k, l in res["labels"].items():
        pts = l["leader"]
        for p_, q_ in zip(pts, pts[1:]):
            line(p_, q_)
        o.append('<circle cx="%.2f" cy="%.2f" r="0.5" fill="black"/>' % tuple(pts[0]))
        r = l["text_rect"]
        for i, s in enumerate(l["lines"]):
            text((r[0] + 0.5, r[1] + T.h / 2 + i * T.h * T.line_h), s, 0, "start")
    for u in res.get("unplaced", []):
        o.append('<text x="10" y="%g" font-size="3" fill="red">unplaced: %s (%s)</text>' % (H - 3, u["id"], u["why"]))
    o.append("</svg>")
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(o))
