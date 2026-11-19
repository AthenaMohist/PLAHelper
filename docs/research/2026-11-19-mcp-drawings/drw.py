"""Dimensioned drawings (A3) for 编码器轴 and 磁环座. Units on paper: mm."""
import math, sys, datetime
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.patches import Polygon, Rectangle, Circle

plt.rcParams["font.family"] = ["Microsoft YaHei", "Segoe UI Symbol"]
plt.rcParams["hatch.linewidth"] = 0.35
OUT = sys.argv[1]
TODAY = datetime.date.today().isoformat()
LW, THIN = 0.7, 0.3
FS = 3.2  # text height mm
PT = 72 / 25.4


class Sheet:
    def __init__(self, title, material, scale, notes):
        self.fig = plt.figure(figsize=(420 / 25.4, 297 / 25.4))
        ax = self.fig.add_axes([0, 0, 1, 1])
        ax.set_xlim(0, 420); ax.set_ylim(0, 297); ax.set_aspect("equal"); ax.axis("off")
        self.ax = ax
        self.r(10, 10, 410, 287, LW * 1.4)
        self.r(5, 5, 415, 292, THIN)
        # title block
        x0, y0 = 270, 10
        self.r(x0, y0, 410, 42, LW)
        for y in (18, 26, 34):
            self.l(x0, y, 410, y, THIN)
        self.l(300, 10, 300, 42, THIN); self.l(355, 10, 355, 42, THIN); self.l(375, 10, 375, 42, THIN)
        rows = [("名称", title, "比例", scale), ("材料", material, "数量", "1"), ("设计", "", "日期", TODAY), ("审核", "", "图幅", "A3")]
        for i, (a, b, c, d) in enumerate(rows):
            y = 38 - 8 * i
            self.t(285, y, a, 3); self.t(327, y, b, 3); self.t(365, y, c, 3); self.t(392, y, d, 3)
        # notes
        self.t(14, 92, "技术要求", 4.2, ha="left")
        for i, n in enumerate(notes):
            self.t(14, 86 - 5.0 * i, n, 2.8, ha="left")

    # primitives -----------------------------------------------------------
    def l(self, x1, y1, x2, y2, lw=LW, ls="-", c="k"):
        self.ax.plot([x1, x2], [y1, y2], lw=lw * PT / 2.0, ls=ls, color=c, solid_capstyle="butt")

    def pl(self, pts, lw=LW, ls="-", closed=False):
        p = list(pts) + ([pts[0]] if closed else [])
        self.ax.plot([q[0] for q in p], [q[1] for q in p], lw=lw * PT / 2.0, ls=ls, color="k")

    def r(self, x1, y1, x2, y2, lw=LW):
        self.pl([(x1, y1), (x2, y1), (x2, y2), (x1, y2)], lw, closed=True)

    def t(self, x, y, s, h=FS, ha="center", va="center", rot=0, box=False):
        kw = {}
        if box:
            kw["bbox"] = dict(boxstyle="square,pad=0.15", fc="white", ec="k", lw=0.5)
        self.ax.text(x, y, s, fontsize=h * PT / 0.72, ha=ha, va=va, rotation=rot, **kw)

    def center(self, x1, y1, x2, y2):
        self.l(x1, y1, x2, y2, THIN, ls=(0, (12, 3, 2, 3)))

    def arrow(self, x, y, dx, dy, size=2.2):
        n = math.hypot(dx, dy); ux, uy = dx / n, dy / n
        bx, by = x - ux * size, y - uy * size
        px, py = -uy * size * 0.18, ux * size * 0.18
        self.ax.add_patch(Polygon([(x, y), (bx + px, by + py), (bx - px, by - py)], closed=True, fc="k", ec="k", lw=0.2))

    def hatch(self, pts, fc="white"):
        self.ax.add_patch(Polygon(pts, closed=True, fc=fc, ec="none", hatch="////", lw=0))

    def fill(self, pts, fc="white"):
        self.ax.add_patch(Polygon(pts, closed=True, fc=fc, ec="none"))

    # dimensions -----------------------------------------------------------
    def dh(self, x1, x2, y, yref1, yref2, text, tpos=None, h=FS):
        """horizontal dim between x1 and x2 at height y; extension lines from yref."""
        for x, yr in ((x1, yref1), (x2, yref2)):
            s = 1 if y > yr else -1
            self.l(x, yr + s * 1.0, x, y + s * 1.5, THIN)
        self.l(x1, y, x2, y, THIN)
        if abs(x2 - x1) > 7:
            self.arrow(x1, y, -1, 0); self.arrow(x2, y, 1, 0)
        else:
            self.arrow(x1, y, 1, 0); self.arrow(x2, y, -1, 0)
            self.l(min(x1, x2) - 5, y, max(x1, x2) + 5, y, THIN)
        tx = (x1 + x2) / 2 if tpos is None else tpos
        self.t(tx, y + 2.2, text, h)

    def dv(self, x, y1, y2, xref, text, h=FS, side=1):
        """vertical dim (diameter) at x between y1, y2, ext lines from xref; text rotated."""
        for yy in (y1, y2):
            s = 1 if x > xref else -1
            self.l(xref + s * 1.0, yy, x + s * 1.5, yy, THIN)
        self.l(x, y1, x, y2, THIN)
        self.arrow(x, y1, 0, -1 if y1 < y2 else 1); self.arrow(x, y2, 0, 1 if y2 > y1 else -1)
        self.t(x - 2.2 * side, (y1 + y2) / 2, text, h, rot=90)

    def leader(self, x, y, tx, ty, text, h=FS, ha="left"):
        self.arrow(x, y, x - tx, y - ty, 2.0)
        self.l(x, y, tx, ty, THIN)
        w = len(text) * h * 0.62
        ex = tx + (w if ha == "left" else -w)
        self.l(tx, ty, ex, ty, THIN)
        self.t(tx + (1 if ha == "left" else -1), ty + 2.0, text, h, ha=ha)

    def fcf(self, x, y, cells, h=3.0):
        """feature control frame, left-bottom at (x, y)."""
        hh = h * 1.9
        for c in cells:
            w = max(hh, len(c) * h * 0.7 + 2)
            self.r(x, y, x + w, y + hh, 0.35)
            self.t(x + w / 2, y + hh / 2, c, h)
            x += w
        return x

    def datum(self, x, y, name, dx=0, dy=-1, L=7):
        # filled triangle on feature, square with letter
        px, py = -dy, dx
        self.ax.add_patch(Polygon([(x + px * 1.5, y + py * 1.5), (x - px * 1.5, y - py * 1.5), (x + dx * 2.6, y + dy * 2.6)], closed=True, fc="k"))
        ex, ey = x + dx * L, y + dy * L
        self.l(x, y, ex, ey, THIN)
        self.r(ex - 2.6, ey - 2.6 + dy * 2.6, ex + 2.6, ey + 2.6 + dy * 2.6, 0.35)
        self.t(ex, ey + dy * 2.6, name, 3.2)

    def ra(self, x, y, val, rot=0):
        self.t(x, y, "Ra" + val, 2.6)
        self.l(x - 2.5, y - 2.0, x - 1.2, y - 4.2, 0.3); self.l(x - 1.2, y - 4.2, x + 1.5, y - 0.2, 0.3)

    def save(self, path):
        self.fig.savefig(path + ".pdf")
        self.fig.savefig(path + ".png", dpi=110)
        plt.close(self.fig)


def mirror(pts):
    return [(x, -y) for x, y in pts]


# =============================== 编码器轴 ===============================
def shaft():
    S = 2.0
    notes = [
        "1. 材料 06Cr19Ni10(304) 固溶态; 靠近磁环, 须无磁。",
        "2. 基准A: Φ20g7 轴线; 基准B: Φ29.5 凸台端面(与RCN转子接触面)。",
        "3. 1:10 外锥: 基面 Φ13 距法兰后端面 3.57±0.05, 锥角公差 AT5,",
        "   与磁环座内锥配研, 涂色接触≥80% 且靠大端。",
        "4. 4×Φ3.4 用 M3×12 (GB/T70.1, 8.8级) + GB/T848 垫圈, 拧紧 1.25±0.1 N·m,",
        "   每次顺时针半圈交替拧紧 (按 HEIDENHAIN RCN2xx1 方式III)。",
        "5. 2×Φ2.5 弹簧销 ISO8752-2.5×10 压入, 伸出 B 面 4±0.2, 开口相对。",
        "6. 未注倒角 C0.3; 锐边去毛刺; 配合面不得有磕碰划伤。",
        "7. 未注尺寸公差 GB/T1804-f, 未注形位公差 GB/T1184-H。",
    ]
    sh = Sheet("编码器轴", "06Cr19Ni10 (304)", "2:1", notes)
    X0, Y0 = 40, 190  # paper origin of front end, axis height

    def P(x, r):
        return (X0 + x * S, Y0 + r * S)

    # profile (x from front end, r)
    xs = dict(b=70.026, sp=60.026, fl=80.026, flr=86.026, gp=89.596, end=96.596)
    prof = [(0, 8.5), (0.5, 9), (60.026, 9), (60.026, 9.5), (60.526, 10), (70.026, 10), (70.026, 14.45), (70.326, 14.75),
            (80.026, 14.75), (80.026, 21.5), (80.526, 22), (85.526, 22), (86.026, 21.5), (86.026, 6.5), (89.596, 6.5),
            (96.296, 5.83), (96.596, 5.53), (96.596, 0)]
    up = [P(0, 0)] + [P(x, r) for x, r in prof]
    lo = [P(0, 0)] + [P(x, -r) for x, r in prof]
    # upper half = section (hatched), lower half = view
    sh.hatch(up)
    # holes cut in section: pin hole r12.5 depth 7 from B; screw hole r17.5 through flange; end tap
    for (xa, xb, rc, rr) in ((70.026, 77.026, 12.5, 1.25), (80.026, 86.026, 17.5, 1.7)):
        rect = [P(xa, rc - rr), P(xb, rc - rr), P(xb, rc + rr), P(xa, rc + rr)]
        sh.fill(rect)
        sh.pl([P(xa, rc - rr), P(xb, rc - rr)], LW); sh.pl([P(xa, rc + rr), P(xb, rc + rr)], LW)
        if xb == 77.026:
            sh.pl([P(xb, rc - rr), P(xb + 0.7, rc), P(xb, rc + rr)], LW)
    tap = [P(88.596, 0), P(96.596, 0), P(96.596, 1.25), P(88.596, 1.25)]
    sh.fill(tap)
    sh.pl([P(88.596, 1.25), P(96.596, 1.25)], LW)
    sh.pl([P(88.596, 1.25), P(87.9, 0)], LW)
    sh.pl([P(90.596, 1.5), P(96.596, 1.5)], THIN)   # thread major (thin)
    sh.pl([P(90.596, 0), P(90.596, 1.5)], LW)       # thread end
    sh.pl(up[1:], LW); sh.pl(lo[1:], LW)
    # step lines in lower (view) half
    for x, r in ((0.5, 9), (60.026, 9.5), (60.526, 10), (70.326, 14.75), (80.526, 22), (85.526, 22), (89.596, 6.5)):
        sh.l(*P(x, -r), *P(x, 0 if x != 89.596 else -0), THIN if x in (0.5, 60.526, 70.326, 80.526, 85.526) else LW)
    for x, r in ((0.5, 9), (60.026, 9.5), (60.526, 10), (70.026, 14.45), (70.326, 14.75), (80.026, 21.5), (80.526, 22), (85.526, 22), (86.026, 21.5), (89.596, 6.5)):
        sh.l(*P(x, 0), *P(x, r), LW if x in (60.026, 70.026, 80.026, 86.026, 89.596) else THIN)
        sh.l(*P(x, 0), *P(x, -r), LW if x in (60.026, 70.026, 80.026, 86.026, 89.596) else THIN)
    sh.l(*P(96.296, -5.83), *P(96.296, 5.83), THIN)
    sh.center(X0 - 6, Y0, X0 + 96.6 * S + 8, Y0)
    sh.t(X0 + 30 * S, Y0 + 30 * S, "主视图 上半 A-A 旋转剖 (过销孔/螺钉孔)", 3.2)

    # diameters
    sh.dv(*[P(30, 0)[0]], P(30, -9)[1], P(30, 9)[1], P(30, 0)[0], "Φ18h7(  0 /-0.018)", 3.0)
    sh.dv(P(65, 0)[0], P(65, -10)[1], P(65, 10)[1], P(65, 0)[0], "Φ20g7(-0.007/-0.028)", 3.0)
    sh.dv(P(75, 0)[0] - 0, P(75, -14.75)[1], P(75, 14.75)[1], P(75, 0)[0], "Φ29.5h11", 3.0)
    yb = P(0, -22)[1] - 0
    # Φ44 at flange (dimension right of flange)
    sh.dv(P(83, 0)[0], P(83, -22)[1], P(83, 22)[1], P(83, 0)[0], "Φ44±0.1", 3.0)
    # taper: basic Φ13 at gauge plane (boxed), 1:10
    sh.leader(*P(93, 6.15), P(102, 18)[0], P(102, 18)[1], "1:10  ◁", 3.2)
    sh.leader(*P(89.596, 6.5), P(100, 26)[0], P(100, 26)[1], "基面 Φ13 (理论)", 3.0)
    sh.leader(*P(94, 1.4), P(104, 9)[0], P(104, 9)[1], "M3-6H深6 / 底孔Φ2.5深8", 2.8)

    # axial dims (below)
    yd1, yd2, yd3 = Y0 - 30 * S, Y0 - 36 * S, Y0 - 42 * S
    ref = lambda x, r: P(x, -r)[1]
    sh.dh(P(0, 0)[0], P(70.026, 0)[0], yd2, ref(0, 8.5), ref(70.026, 14.45), "70.03±0.1")
    sh.dh(P(60.026, 0)[0], P(70.026, 0)[0], yd1, ref(60.026, 10), ref(70.026, 14.45), "10±0.5")
    sh.dh(P(70.026, 0)[0], P(80.026, 0)[0], yd1, ref(70.026, 14.45), ref(80.026, 21.5), "10±0.2")
    sh.dh(P(80.026, 0)[0], P(86.026, 0)[0], yd1 + 0, ref(80.026, 21.5), ref(86.026, 21.5), "6±0.05", tpos=P(83, 0)[0])
    sh.dh(P(86.026, 0)[0], P(89.596, 0)[0], yd2, ref(86.026, 21.5), ref(89.596, 6.5), "3.57±0.05", tpos=P(90, 0)[0])
    sh.dh(P(89.596, 0)[0], P(96.596, 0)[0], yd1 - 0, ref(89.596, 6.5), ref(96.596, 5.53), "7", tpos=P(94, 0)[0] + 3)
    sh.dh(P(70.026, 0)[0], P(77.026, 0)[0], P(0, 30)[1], P(70, 14.75)[1], P(77, 14)[1], "深7", tpos=P(73.5, 0)[0])
    sh.dh(P(0, 0)[0], P(96.596, 0)[0], yd3, ref(0, 8.5), ref(96.596, 5.53), "96.6±0.1")

    # datums
    xA = P(65, 0)[0]
    sh.datum(xA, P(65, -10)[1] - 2.5, "A", 0, -1, 5)  # aligned with the Φ20g7 dimension line
    sh.datum(P(70.026, 0)[0], P(70.026, 20)[1] + 2, "B", 0, 1, 5)
    # GD&T frames
    xe = sh.fcf(P(40, 0)[0], P(0, 15)[1], ["⌭", "0.003"])
    sh.l(xe, P(0, 15)[1] + 2.8, P(63, 10)[0], P(63, 10)[1], THIN); sh.arrow(P(63, 10)[0], P(63, 10)[1], 1, -0.4)
    sh.fcf(P(48, 0)[0], P(0, 24)[1], ["↗", "0.005", "A"])
    sh.l(P(70.026, 17)[0], P(70.026, 17)[1], P(70, 25)[0] - 0, P(70, 25.6)[1], THIN)
    sh.t(P(56, 0)[0], P(0, 28.7)[1], "(B 面轴向跳动)", 2.5)
    sh.fcf(P(98, 0)[0], P(0, -16)[1], ["↗", "0.003", "A"])
    sh.l(P(93, -6.2)[0], P(93, -6.2)[1], P(98, -14)[0], P(98, -14)[1], THIN)
    sh.t(P(103, 0)[0], P(0, -17.6)[1], "(外锥径向跳动)", 2.5)
    sh.fcf(P(12, 0)[0], P(0, -20)[1], ["↗", "0.01", "A"])
    sh.l(P(25, -9)[0], P(25, -9)[1], P(20, -16)[0], P(20, -16)[1], THIN)
    # roughness
    sh.ra(P(65, 0)[0], P(0, 12.8)[1], "0.4"); sh.ra(P(93, 0)[0], P(0, 9.8)[1], "0.4"); sh.ra(P(25, 0)[0], P(0, 11.8)[1], "0.8")

    # ---- end view (from front, B side): flange holes ----
    cx, cy, Se = 320, 215, 2.0
    sh.ax.add_patch(Circle((cx, cy), 22 * Se, fill=False, lw=LW * PT / 2))
    sh.ax.add_patch(Circle((cx, cy), 21.5 * Se, fill=False, lw=THIN * PT / 2))
    sh.ax.add_patch(Circle((cx, cy), 14.75 * Se, fill=False, lw=LW * PT / 2))
    sh.ax.add_patch(Circle((cx, cy), 10 * Se, fill=False, lw=LW * PT / 2))
    sh.ax.add_patch(Circle((cx, cy), 9 * Se, fill=False, lw=LW * PT / 2))
    sh.ax.add_patch(Circle((cx, cy), 17.5 * Se, fill=False, lw=THIN * PT / 2, ls=(0, (12, 3, 2, 3))))
    sh.ax.add_patch(Circle((cx, cy), 12.5 * Se, fill=False, lw=THIN * PT / 2, ls=(0, (12, 3, 2, 3))))
    sh.center(cx - 50, cy, cx + 50, cy); sh.center(cx, cy - 50, cx, cy + 50)
    for k in range(4):
        a = math.radians(67.5 + 90 * k)
        sh.ax.add_patch(Circle((cx + 17.5 * Se * math.cos(a), cy + 17.5 * Se * math.sin(a)), 1.7 * Se, fill=False, lw=LW * PT / 2))
    for a0 in (22.5, 202.5):
        a = math.radians(a0); hx, hy = cx + 17.5 * Se * math.cos(a), cy + 17.5 * Se * math.sin(a)
        sh.ax.add_patch(Circle((hx, hy), 1.25 * Se, fill=False, lw=LW * PT / 2))
        sh.ax.add_patch(Circle((hx, hy), 1.5 * Se, fill=False, lw=THIN * PT / 2))
    for a0 in (157.5, -22.5):
        a = math.radians(a0)
        sh.ax.add_patch(Circle((cx + 12.5 * Se * math.cos(a), cy + 12.5 * Se * math.sin(a)), 1.25 * Se, fill=False, lw=LW * PT / 2))
    for a0 in (67.5, 157.5, 22.5, -22.5):
        a = math.radians(a0)
        sh.l(cx, cy, cx + 47 * math.cos(a), cy + 47 * math.sin(a), THIN, ls=(0, (12, 3, 2, 3)))
    for a0, rr, lab in ((67.5, 17.5, "a"), (22.5, 17.5, "b"), (-22.5, 12.5, "c")):
        a = math.radians(a0)
        sh.t(cx + (rr * Se + 6) * math.cos(a), cy + (rr * Se + 6) * math.sin(a), lab, 3.4)
    sh.t(cx + 30, cy + 18, "22.5°", 2.8); sh.t(cx + 6, cy + 30, "67.5°", 2.8); sh.t(cx + 28, cy - 6, "-22.5°", 2.8)
    yy = cy - 66
    for s_ in ("a: 4×Φ3.4 通, 分布圆 Φ35, 67.5°+90°×k, ⌖Φ0.1 A",
               "b: 2×M3-6H 通 (拆卸顶丝孔), 分布圆 Φ35, 22.5° / 202.5°",
               "c: 2×Φ2.5H12(+0.1/0) 深7 (自B面), 分布圆 Φ25, -22.5° / 157.5°, ⌖Φ0.05 A",
               "角度自 +X 逆时针, 自 B 面方向看; c 与 a 同径向线"):
        sh.t(262, yy, s_, 2.8, ha="left"); yy -= 5
    sh.t(cx, cy + 56, "B 向", 3.6)
    sh.save(OUT + r"\编码器轴_尺寸公差")


# =============================== 磁环座 ===============================
def seat():
    S = 4.0
    notes = [
        "1. 材料 06Cr19Ni10(304), 与编码器轴同材(线胀系数一致), 无磁。",
        "2. 基准A: 1:10 内锥轴线。内锥与编码器轴外锥配研, 涂色接触≥80% 且靠大端。",
        "3. 装配: 锥面清洁脱脂, 推入后用 M3×10 (GB/T70.1, A2-70) 锁紧约 1 N·m;",
        "   装配后前端面距轴法兰后端面 4.57±0.2 (参考, 由锥面公差决定)。",
        "4. 拆卸: 取下 M3, 旋入 M5 顶丝 (GB/T77, A2-70) 顶轴端面退出。",
        "5. 磁环粘入 Φ34.2 定位槽, 贴紧槽底; 胶层 ≤0.02。",
        "6. 未注倒角 C0.2; 锐边去毛刺; 锥面、槽底不得有磕碰划伤。",
        "7. 未注尺寸公差 GB/T1804-f, 未注形位公差 GB/T1184-H。",
    ]
    S = 3.0
    sh = Sheet("磁环座", "06Cr19Ni10 (304)", "3:1", notes)
    X0, Y0 = 170, 190

    def P(x, r):
        return (X0 + x * S, Y0 + r * S)

    outer = [(0, 6.75), (0, 17.5), (0.5, 18), (13.386, 18), (13.886, 17.5), (13.886, 17.1), (13.486, 17.1), (13.486, 3.25),
             (9.986, 3.25), (9.986, 2.1), (7.0, 2.1), (7.0, 6.15), (6.0, 6.15), (0.3, 6.42)]
    for sgn in (1, -1):
        pts = [P(x, sgn * r) for x, r in outer]
        sh.hatch(pts); sh.pl(pts, LW, closed=True)
    # thread M5 major (thin) in web
    for sgn in (1, -1):
        sh.l(*P(7.0, sgn * 2.5), *P(9.986, sgn * 2.5), THIN)
    # visible lines beyond section (bore edges)
    for x, r in ((0.3, 6.42), (6.0, 6.15), (7.0, 6.15), (9.986, 3.25), (13.486, 3.25)):
        sh.l(*P(x, -r), *P(x, r), THIN if x == 0.3 else LW)
    sh.center(X0 - 8, Y0, X0 + 13.886 * S + 10, Y0)
    sh.t(X0 + 7 * S, Y0 + 24 * S, "全剖", 3.4)

    sh.dv(P(-4, 0)[0], P(0, -18)[1], P(0, 18)[1], P(0.5, 0)[0], "Φ36±0.05", 3.0)
    sh.dv(P(18.5, 0)[0], P(13.886, -17.1)[1], P(13.886, 17.1)[1], P(13.886, 0)[0], "Φ34.2H8(+0.039/0)", 3.0, side=-1)
    sh.dv(P(16.0, 0)[0], P(13.486, -3.25)[1], P(13.486, 3.25)[1], P(13.486, 0)[0], "Φ6.5", 3.0, side=-1)
    sh.leader(*P(8.5, 2.5), P(-6, 20)[0], P(-6, 20)[1], "M5-6H 通 (顶出)", 3.0, ha="right")
    sh.leader(*P(3.0, 6.3), P(-6, 13)[0], P(-6, 13)[1], "1:10  ◁   前端面 Φ12.9 (理论)", 3.0, ha="right")
    sh.leader(*P(6.5, -6.15), P(-6, -12)[0], P(-6, -12)[1], "退刀 Φ12.3", 3.0, ha="right")
    yb = P(0, -25)[1]
    sh.dh(P(0, 0)[0], P(6.0, 0)[0], P(0, -21)[1], P(0, -6.5)[1], P(6, -6.2)[1], "6")
    sh.dh(P(6.0, 0)[0], P(7.0, 0)[0], P(0, -21)[1], P(6, -6.2)[1], P(7, -6.2)[1], "1", tpos=P(8.5, 0)[0])
    sh.dh(P(7.0, 0)[0], P(9.986, 0)[0], P(0, -24)[1], P(7, -2.1)[1], P(9.986, -3.25)[1], "3")
    sh.dh(P(0, 0)[0], P(13.486, 0)[0], P(0, -27)[1], P(0, -17.5)[1], P(13.486, -17.1)[1], "13.49±0.02")
    sh.dh(P(0, 0)[0], P(13.886, 0)[0], P(0, -30)[1], P(0, -17.5)[1], P(13.886, -17.5)[1], "13.89±0.05")
    sh.dh(P(13.486, 0)[0], P(13.886, 0)[0], P(0, 21)[1], P(13.486, 17.1)[1], P(13.886, 17.5)[1], "0.4±0.02", tpos=P(17, 0)[0])
    sh.dh(P(9.986, 0)[0], P(13.486, 0)[0], P(0, 9)[1], P(9.986, 3.25)[1], P(13.486, 3.25)[1], "3.5")
    sh.t(P(-1.5, 0)[0], P(0, 19.6)[1], "C0.5", 2.8); sh.t(P(1.5, 0)[0], P(0, 7.6)[1], "C0.3", 2.8)
    # datum A on taper bore
    sh.datum(*P(3.0, -6.3), "A", 0, 1, 0.01)
    # GD&T
    sh.fcf(P(17.5, 0)[0], P(0, 25)[1], ["◎", "Φ0.01", "A"])
    sh.l(P(15.3, 22.5)[0], P(15.3, 22.5)[1], P(17.5, 25)[0], P(17.5, 25)[1], THIN)
    sh.fcf(P(17.5, 0)[0], P(0, 12.5)[1], ["↗", "0.005", "A"])
    sh.fcf(P(17.5, 0)[0], P(0, 9.6)[1], ["⏥", "0.003"])
    sh.l(P(13.486, 8)[0], P(13.486, 8)[1], P(17.5, 13.7)[0], P(17.5, 13.7)[1], THIN)
    sh.t(P(26, 0)[0], P(0, 16.5)[1], "(槽底 = 磁环贴合面)", 2.5)
    sh.ra(P(3.5, 0)[0], P(0, 7.6)[1] + 2, "0.4"); sh.ra(P(11.5, 0)[0], P(0, 5.5)[1], "0.8")

    # end view from rear (magnet side)
    cx, cy, Se = 340, 190, 2.0
    for r, w, ls in ((18, LW, "-"), (17.5, THIN, "-"), (17.1, LW, "-"), (3.25, LW, "-"), (2.1, LW, "-"), (2.5, THIN, "-"), (6.45, THIN, "--")):
        sh.ax.add_patch(Circle((cx, cy), r * Se, fill=False, lw=w * PT / 2, ls=ls))
    sh.center(cx - 52, cy, cx + 52, cy); sh.center(cx, cy - 52, cx, cy + 52)
    sh.t(cx, cy + 56, "后视 (磁环侧)", 3.4)
    sh.save(OUT + r"\磁环座_尺寸公差")


shaft()
seat()
print("ok")

