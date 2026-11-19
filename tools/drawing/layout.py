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


class Text:
    def __init__(self, std):
        L, f = std["layout"], std["fonts"]
        self.cjk, self.latin = L["char_w"]["cjk"], L["char_w"]["latin"]
        self.h = f["dim"]
        self.line_h = L["line_h"]

    def width(self, s, h=None):
        h = h or self.h
        return sum(h * (self.cjk if ord(ch) > 0x2E80 else self.latin) for ch in s)

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

    def halos(self, lanes):
        """每个视图四边要留的宽度(尺寸层 + 文字)。"""
        h = {vid: {"top": 0.0, "bottom": 0.0, "left": 0.0, "right": 0.0} for vid in self.views}
        for did, (side, lane, _, _) in lanes.items():
            d = next(x for x in self.dims if x["id"] == did)
            need = self.D["gap"] + lane * self.D["step"] + self.T.h + 1.0
            h[d["view"]][side] = max(h[d["view"]][side], need)
        # 预留(编译器自己画的东西: 坐标尺寸条、剖视名、剖切符号箭头 …): 叠在尺寸层外面
        for rsv in self.p.get("reserve", []):
            if rsv["view"] in h:
                h[rsv["view"]][rsv["side"]] += rsv["depth"]
        return h

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
            off = g + lane * st
            if d["dir"] == "x":
                y = r[1] - off if side == "top" else r[3] + off
                x0, x1 = sorted((A[0], B[0]))
                sgn = -1 if side == "top" else 1
                out[d["id"]] = {"side": side, "lane": lane, "line": [(x0, y), (x1, y)], "rot": 0, "outside": text_out, "text": d["text"],
                                "ext": [[(p[0], p[1] + sgn * eg), (p[0], y + sgn * eo)] for p in (A, B)]}
            else:
                x = r[0] - off if side == "left" else r[2] + off
                y0, y1 = sorted((A[1], B[1]))
                sgn = -1 if side == "left" else 1
                out[d["id"]] = {"side": side, "lane": lane, "line": [(x, y0), (x, y1)], "rot": -90, "outside": text_out, "text": d["text"],
                                "ext": [[(p[0] + sgn * eg, p[1]), (x + sgn * eo, p[1])] for p in (A, B)]}
        # 线 → 细长矩形(文字不能压别人的线)
        def thin(p, q, w=0.35):
            return (min(p[0], q[0]) - w, min(p[1], q[1]) - w, max(p[0], q[0]) + w, max(p[1], q[1]) + w)
        lines = {k: [thin(*d["line"])] + [thin(*e) for e in d["ext"]] for k, d in out.items()}
        view_rects = [pr for c in place.values() for pr in c["parts"]]
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
            for name, rr in cands:
                if not rect_inside(rr, self.area) or any(rect_hit(rr, kk) for kk in self.keepout):
                    continue
                if any(rect_hit(rr, vr, pad) for vr in view_rects):
                    continue
                if any(rect_hit(rr, t, pad) for t in texts):
                    continue
                if any(rect_hit(rr, ln) for kk, ls in lines.items() if kk != k for ln in ls):
                    continue
                if any(rect_hit(rr, ln) for i, ln in enumerate(lines[k]) if i > 0):   # 自己的界线
                    continue
                chosen = (name, rr)
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
    def realize_inline(self, place, s, taken_text):
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
                        rec.update({"text_rect": trect, "text": d["text"], "at": t, "kind": "inline"})
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
    def realize_labels(self, place, s, obstacles, dim_lines, extra=None):
        L = self.L
        th, pad = self.T.h, L["pad"]
        view_rects = {vid: c["parts"] for vid, c in place.items()}
        placed, unplaced = {}, []
        order = sorted(self.labels + list(extra or []), key=lambda l: (-l.get("prio", 0), l["id"]))

        def candidates(lb, others):
            w, h, lines = self.T.box(lb["text"])
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
        best = None
        for s in scales:
            bump, place = {}, None
            for _round in range(12):
                lanes = self.lanes(s, bump)
                halo = self.halos(lanes)
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
            inl, un1 = self.realize_inline(place, s, list(texts))
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
            labels, un2 = self.realize_labels(place, s, obstacles, dim_lines, extra)
            res = {"scale": s, "views": place, "dims": dims, "labels": labels, "unplaced": un0 + un1 + un2}
            res["conflicts"] = conflicts(res, self)
            # 观感门槛: 有标签引线横穿别的视图 / 与别的引线交叉 / 在零件里走很长(代价高)→ 这个比例太挤, 试更小的
            ugly = [{"id": k, "why": l["flags"]} for k, l in labels.items() if l["flags"]]
            res["ugly"] = ugly
            bad = len(res["unplaced"]) + len(res["conflicts"])
            if not bad and not ugly:
                return res
            key = (bad, len(ugly))
            if best is None or key < best[0]:
                best = (key, res)
        if best is not None:
            return best[1]
        return {"scale": None, "views": {}, "dims": {}, "labels": {}, "unplaced": [{"id": "*", "why": "views do not fit at any scale"}], "conflicts": [], "ugly": []}


def conflicts(res, lay):
    """落位后的复查: 文字框之间、文字框与视图(inline 文字除外)、与禁区 / 出界。→ [(a, b)]"""
    items = []
    for k, d in res["dims"].items():
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
        a_, kn, e = l["leader"]
        line(a_, kn); line(kn, e)
        o.append('<circle cx="%.2f" cy="%.2f" r="0.5" fill="black"/>' % a_)
        r = l["text_rect"]
        for i, s in enumerate(l["lines"]):
            text((r[0] + 0.5, r[1] + T.h / 2 + i * T.h * T.line_h), s, 0, "start")
    for u in res.get("unplaced", []):
        o.append('<text x="10" y="%g" font-size="3" fill="red">unplaced: %s (%s)</text>' % (H - 3, u["id"], u["why"]))
    o.append("</svg>")
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(o))
