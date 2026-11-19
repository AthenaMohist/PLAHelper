"""layout.py 的合成用例(仿编码器轴: 台阶轴主视 + B 向视图 + 等轴测, 链式轴向尺寸、各段直径带配合、孔说明 / 形位公差 / 粗糙度引出)。
运行: <FreeCAD>/bin/python.exe test_layout.py [out.svg]   (纯 Python, 不需要 FreeCAD 模块)
"""
import math
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import layout  # noqa: E402

STD = layout.load_std()
A3_AREA = [12, 12, 413, 290]
A3_KEEPOUT = [[10, 238, 100, 292], [100, 238, 286, 265], [100, 265, 415, 292], [368, 10, 415, 24]]


def shaft_problem():
    segs = [(60, 18, "⌀18h7(0/-0.018)"), (10, 20, "⌀20g7(-0.007/-0.028)"), (10, 29.5, "⌀29.5h11"), (6, 44, "⌀44±0.1"), (3.57, 13, None), (7, 13, None)]
    total = sum(s[0] for s in segs)
    x = -total / 2
    dims, edges, parts = [], [x], []
    for i, (L, D, txt) in enumerate(segs):
        parts.append([x, -D / 2, x + L, D / 2])
        if txt:
            dims.append({"id": "D%d" % i, "view": "front", "kind": "inline", "dir": "y", "span": [-D / 2, D / 2], "slide": [x + 0.8, x + L - 0.8], "text": txt})
        x += L
        edges.append(x)
    texts = ["60", "10±0.5", "10±0.2", "6±0.05", "3.57±0.05", "7"]
    for i in range(len(segs)):
        dims.append({"id": "L%d" % i, "view": "front", "dir": "x", "side": "bottom", "a": [edges[i], -segs[i][1] / 2], "b": [edges[i + 1], -segs[i][1] / 2], "text": texts[i]})
    dims.append({"id": "Ltot", "view": "front", "dir": "x", "side": "bottom", "a": [edges[0], -9], "b": [edges[-1], -6.5], "text": "96.57±0.1"})
    dims.append({"id": "L70", "view": "front", "dir": "x", "side": "top", "a": [edges[0], 9], "b": [edges[2], 10], "text": "70.03±0.1"})
    dims.append({"id": "Hb", "view": "side", "dir": "y", "side": "right", "a": [22, 22], "b": [22, -22], "text": "⌀44"})
    labels = [
        {"id": "runout18", "view": "front", "anchor": [-20, -9], "text": "↗ 0.01 A", "prio": 2},
        {"id": "cyl20", "view": "front", "anchor": [ edges[1] + 5 , 10], "text": "⌭ 0.003", "prio": 2},
        {"id": "runoutB", "view": "front", "anchor": [edges[2], 14.75], "text": "↗ 0.005 A", "prio": 2},
        {"id": "cone", "view": "front", "anchor": [edges[5] + 3.5, -6], "text": ["1:10 ◁", "↗ 0.003 A"], "prio": 2},
        {"id": "ra18", "view": "front", "anchor": [-35, 9], "text": "Ra0.8", "prio": 1},
        {"id": "ra20", "view": "front", "anchor": [edges[1] + 2, -10], "text": "Ra0.4", "prio": 1},
        {"id": "datumA", "view": "front", "anchor": [edges[1] + 8, -10], "text": "A", "prio": 3},
        {"id": "thread", "view": "front", "anchor": [edges[-1], 1.0], "text": "M3-6H↧6", "prio": 1},
    ]
    for i, (n, r, a0, txt) in enumerate([(4, 17.5, 67.5, "4×⌀3.4 通 EQS ⌖⌀0.1 A"), (2, 17.5, 22.5, "2×M3-6H 通"), (2, 12.5, -22.5, "2×⌀2.5H12↧7 ⌖⌀0.05 A")]):
        a = math.radians(a0)
        labels.append({"id": "hole%d" % i, "view": "side", "anchor": [r * math.cos(a), r * math.sin(a)], "text": txt, "prio": 2})
    return {
        "paper": [420, 297], "area": A3_AREA, "keepout": A3_KEEPOUT,
        "views": [
            {"id": "front", "w": total, "h": 44, "parts": parts},
            {"id": "side", "w": 44, "h": 44, "rel": {"of": "front", "dir": "right"},
             "parts": [[-22, -9, 22, 9], [-19, -15, 19, 15], [-15, -19, 15, 19], [-9, -22, 9, 22]]},   # 圆的近似轮廓
            {"id": "iso", "w": 70, "h": 45, "free": True},
        ],
        "dims": dims, "labels": labels,
    }


def check(res, prob):
    assert not res["unplaced"], res["unplaced"]
    assert not res["conflicts"], res["conflicts"]
    assert not res["ugly"], ("labels with ugly leaders", res["ugly"])
    v = res["views"]
    assert abs(v["front"]["cy"] - v["side"]["cy"]) < 1e-6, "B 向视图与主视图同高(投影对齐)"
    assert v["side"]["cx"] > v["front"]["cx"]
    # 短尺寸在里层, 总长在最外层
    lanes = {k: d["lane"] for k, d in res["dims"].items() if "lane" in d}
    assert lanes["Ltot"] == max(lanes[k] for k in lanes if k.startswith("L") and k != "L70")
    # 视图在可画区、不进禁区
    for c in v.values():
        assert layout.rect_inside(c["rect"], prob["area"])
        assert not any(layout.rect_hit(c["rect"], k) for k in prob["keepout"])
    # 比例: 这张图在 A3 上应能用 2:1
    assert res["scale"] >= 2, res["scale"]


def test_tiny_and_errors():
    p = {"paper": [420, 297], "area": A3_AREA, "keepout": A3_KEEPOUT, "views": [{"id": "f", "w": 10, "h": 5}], "dims": [], "labels": []}
    r = layout.solve(p)
    assert r["scale"] == 10 and not r["unplaced"], r
    # 关系链断了 → 报错
    try:
        layout.solve({**p, "views": [{"id": "f", "w": 10, "h": 5}, {"id": "g", "w": 5, "h": 5, "rel": {"of": "nope", "dir": "right"}}]})
        raise AssertionError("broken relation must raise")
    except ValueError:
        pass
    # 放不下 → unplaced 而不是静默
    big = {**p, "views": [{"id": "f", "w": 50000, "h": 50000}]}
    r = layout.solve(big)
    assert r["unplaced"], "too big must be reported"


if __name__ == "__main__":
    prob = shaft_problem()
    res = layout.solve(prob)
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "out", "layout_test.svg")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    layout.render_svg(prob, res, out)
    print("scale", res["scale"], "unplaced", res["unplaced"], "conflicts", res["conflicts"][:5])
    check(res, prob)
    test_tiny_and_errors()
    print("ok", out)
