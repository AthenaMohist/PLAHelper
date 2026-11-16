# 出图编译器共用的几何小工具(零件图 compile_freecad.py / 装配图 compile_assembly.py)。
import math

import FreeCAD as App
import Part

V = App.Vector


def edge_circle(ed, tol=2e-3):
    """视图里的边是不是圆 / 圆弧(B 样条孔投影出来是 B 样条曲线)→ (圆心 Vector, 半径) 或 None。
    取 16 个点, 三点定圆再验证所有点到圆心距离一致, 且弧跨度 > 60°。"""
    try:
        pts = ed.discretize(16)
    except Exception:
        return None
    if len(pts) < 5:
        return None
    a, b, c = pts[0], pts[len(pts) // 2], pts[-1]
    if (c - a).Length < 1e-6:  # 整圆: 首尾重合, 换一个点
        c = pts[len(pts) // 4]
    # 2D(视图边在 z = 0 平面)三点定圆
    ax_, ay_, bx_, by_, cx_, cy_ = a.x, a.y, b.x, b.y, c.x, c.y
    d = 2 * (ax_ * (by_ - cy_) + bx_ * (cy_ - ay_) + cx_ * (ay_ - by_))
    if abs(d) < 1e-9:
        return None
    ux = ((ax_ ** 2 + ay_ ** 2) * (by_ - cy_) + (bx_ ** 2 + by_ ** 2) * (cy_ - ay_) + (cx_ ** 2 + cy_ ** 2) * (ay_ - by_)) / d
    uy = ((ax_ ** 2 + ay_ ** 2) * (cx_ - bx_) + (bx_ ** 2 + by_ ** 2) * (ax_ - cx_) + (cx_ ** 2 + cy_ ** 2) * (bx_ - ax_)) / d
    ctr = V(ux, uy, 0)
    r = math.hypot(ax_ - ux, ay_ - uy)
    if r < 1e-6 or any(abs(math.hypot(p.x - ux, p.y - uy) - r) > tol * max(r, 1) for p in pts):
        return None
    # 弧跨度: 首尾 + 中点的角度
    ang = lambda p: math.atan2(p.y - uy, p.x - ux)  # noqa: E731
    span = 0.0
    prev = ang(pts[0])
    for p in pts[1:]:
        x = ang(p)
        dx = x - prev
        while dx > math.pi:
            dx -= 2 * math.pi
        while dx < -math.pi:
            dx += 2 * math.pi
        span += dx
        prev = x
    if abs(span) < math.radians(60):
        return None
    return ctr, r


def cylinder_of(f, tol=1e-3):
    """面是圆柱(解析圆柱, 或几何上是圆柱的 B 样条面)→ {ax, c, r, internal, t0, t1, span}, 否则 None。
    Plasticity 里阵列 / 布尔出来的孔有时存成 B 样条面(指纹里的 BSurf), 实测 encoder calibrator 的法兰 16 个孔全是 → 以前一个都没认出来。
    B 样条判圆柱: 面上 3 × 3 取点和法向, 法向都垂直同一根轴、各点到轴的距离一样。"""
    s = f.Surface
    try:
        u0, u1, v0, v1 = f.ParameterRange
    except Exception:
        return None
    if isinstance(s, Part.Cylinder):
        ax = s.Axis.normalize()
        c = s.Center
        r = s.Radius
        p = f.valueAt((u0 + u1) / 2, (v0 + v1) / 2)
        n = f.normalAt((u0 + u1) / 2, (v0 + v1) / 2)
        span = abs(u1 - u0)
    elif isinstance(s, (Part.BSplineSurface, Part.BezierSurface)) or type(s).__name__ in ("SurfaceOfRevolution", "SurfaceOfExtrusion"):
        pts = []
        for a in (0.1, 0.5, 0.9):
            for b in (0.1, 0.5, 0.9):
                uv = (u0 + (u1 - u0) * a, v0 + (v1 - v0) * b)
                try:
                    pts.append((f.valueAt(*uv), f.normalAt(*uv)))
                except Exception:
                    return None
        # 轴向 = 两个不平行法向的叉积
        ax = None
        for i in range(len(pts)):
            for j in range(i + 1, len(pts)):
                cr = pts[i][1].cross(pts[j][1])
                if cr.Length > 0.2:
                    ax = cr.normalize()
                    break
            if ax:
                break
        if ax is None or any(abs(n.dot(ax)) > 1e-3 for _, n in pts):
            return None
        # 半径: 两点在垂直轴的平面上的弦长 / (2 sin(法向夹角 / 2))
        (pa, na), (pb, nb) = pts[0], pts[-1]
        d = (pb - pa) - ax * (pb - pa).dot(ax)
        ang = math.acos(max(-1.0, min(1.0, na.dot(nb))))
        if ang < 0.05:
            return None
        r = d.Length / (2 * math.sin(ang / 2))
        # 轴心: 沿法向退 r(内表面法向指向轴, 外表面背向轴, 两种都试)
        c = None
        for sg in (1, -1):
            cc = pa - na * (r * sg)
            ok = True
            for p_, n_ in pts:
                q = p_ - cc
                q = q - ax * q.dot(ax)
                if abs(q.Length - r) > tol * max(r, 1):
                    ok = False
                    break
            if ok:
                c = cc
                break
        if c is None:
            return None
        # 精修: 沿 u 取 13 个点, 在垂直轴的平面里最小二乘拟合圆(Kåsa), 圆心 / 半径比两点估计准(实测 ⌀83 两点估成 83.01)
        e1_ = (pts[0][0] - c) - ax * (pts[0][0] - c).dot(ax)
        if e1_.Length > 1e-9:
            e1_.normalize()
            e2_ = ax.cross(e1_)
            xy = []
            vm = (v0 + v1) / 2
            for k in range(13):
                try:
                    q = f.valueAt(u0 + (u1 - u0) * k / 12.0, vm) - c
                except Exception:
                    continue
                xy.append((q.dot(e1_), q.dot(e2_)))
            if len(xy) >= 5:
                # 解 x² + y² + D x + E y + F = 0
                sxx = sum(x * x for x, _ in xy); syy = sum(y * y for _, y in xy); sxy = sum(x * y for x, y in xy)  # noqa: E702
                sx = sum(x for x, _ in xy); sy = sum(y for _, y in xy); n_ = len(xy)  # noqa: E702
                sz = [-(x * x + y * y) for x, y in xy]
                b1 = sum(z * x for z, (x, _) in zip(sz, xy)); b2 = sum(z * y for z, (_, y) in zip(sz, xy)); b3 = sum(sz)  # noqa: E702
                M = [[sxx, sxy, sx], [sxy, syy, sy], [sx, sy, n_]]
                det = (M[0][0] * (M[1][1] * M[2][2] - M[1][2] * M[2][1]) - M[0][1] * (M[1][0] * M[2][2] - M[1][2] * M[2][0])
                       + M[0][2] * (M[1][0] * M[2][1] - M[1][1] * M[2][0]))
                if abs(det) > 1e-12:
                    def solve(col):
                        A = [row[:] for row in M]
                        for i_ in range(3):
                            A[i_][col] = (b1, b2, b3)[i_]
                        return (A[0][0] * (A[1][1] * A[2][2] - A[1][2] * A[2][1]) - A[0][1] * (A[1][0] * A[2][2] - A[1][2] * A[2][0])
                                + A[0][2] * (A[1][0] * A[2][1] - A[1][1] * A[2][0])) / det
                    D_, E_, F_ = solve(0), solve(1), solve(2)
                    ox, oy = -D_ / 2, -E_ / 2
                    rr = math.sqrt(max(ox * ox + oy * oy - F_, 0.0))
                    if rr > 0 and abs(rr - r) < 0.05 * r:
                        c = c + e1_ * ox + e2_ * oy
                        r = rr
        p, n = pts[4]
        # 角度覆盖: 沿 u 取点看绕轴转了多少
        ref = (pts[0][0] - c) - ax * (pts[0][0] - c).dot(ax)
        e1 = ref.normalize()
        e2 = ax.cross(e1)
        angs = []
        for a in [k / 12.0 for k in range(13)]:
            try:
                q = f.valueAt(u0 + (u1 - u0) * a, (v0 + v1) / 2) - c
            except Exception:
                continue
            q = q - ax * q.dot(ax)
            angs.append(math.atan2(q.dot(e2), q.dot(e1)))
        # 展开后的最大跨度
        span = 0.0
        if angs:
            un = [angs[0]]
            for x in angs[1:]:
                while x - un[-1] > math.pi:
                    x -= 2 * math.pi
                while x - un[-1] < -math.pi:
                    x += 2 * math.pi
                un.append(x)
            span = min(2 * math.pi, max(un) - min(un))
            if span > 2 * math.pi - 0.6:
                span = 2 * math.pi
    else:
        return None
    rad = (p - c) - ax * ((p - c).dot(ax))
    if rad.Length < 1e-6:
        return None
    internal = n.dot(rad) < 0
    ts = [(vx.Point - c).dot(ax) for vx in f.Vertexes] or [0.0]
    if len(ts) < 2:
        # 整圈的 B 样条面可能只有缝上的顶点: 用包围盒角点估轴向范围
        bb = f.BoundBox
        ts = [(V(x, y, z) - c).dot(ax) for x in (bb.XMin, bb.XMax) for y in (bb.YMin, bb.YMax) for z in (bb.ZMin, bb.ZMax)]
        ts = [min(ts), max(ts)]
    return {"ax": ax, "c": c, "r": r, "internal": internal, "t0": min(ts), "t1": max(ts), "span": span}
