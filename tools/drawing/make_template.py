# 由制图规范 JSON 生成 FreeCAD TechDraw 图框模板(SVG)。标题栏布局只写在规范 JSON 里, 这里只负责画。
# 用法: python make_template.py <standard.json> <paper, 如 A3> <out.svg>
import json
import sys
from xml.sax.saxutils import escape

A3 = (420.0, 297.0)


def build(std, paper):
    W, H = std["paper"][paper]
    fr = std["frame"]
    o, band = fr["outer"], fr["zone_band"]
    fonts, lines = std["fonts"], std["lines"]
    fam = "%s,'仿宋',serif" % fonts["cn"]
    dx, dy = W - A3[0], H - A3[1]  # 大图幅: 右下角块整体平移
    out = []
    a = out.append
    a('<?xml version="1.0" encoding="UTF-8" standalone="no"?>')
    a('<svg xmlns="http://www.w3.org/2000/svg" xmlns:freecad="https://www.freecad.org/wiki/index.php?title=Svg_Namespace" '
      'width="%gmm" height="%gmm" viewBox="0 0 %g %g" version="1.1">' % (W, H, W, H))
    a('<title>PLAhelper %s %s</title>' % (escape(std["name"]), paper))
    a('<g style="fill:none;stroke:#000;stroke-linecap:square;stroke-linejoin:miter">')
    # 外框 + 分区带(上 / 左)+ 内框
    x0, y0, x1, y1 = o, o, W - o, H - o
    ix, iy = x0 + band, y0 + band
    a('<rect x="%g" y="%g" width="%g" height="%g" style="stroke-width:%g"/>' % (x0, y0, x1 - x0, y1 - y0, lines["thin"]))
    a('<rect x="%g" y="%g" width="%g" height="%g" style="stroke-width:%g"/>' % (ix, iy, x1 - ix, y1 - iy, lines["frame"]))
    a('<path d="M %g %g L %g %g" style="stroke-width:%g"/>' % (x0, y0, ix, iy, lines["thin"]))
    a('<path d="M %g %g H %g M %g %g V %g" style="stroke-width:%g"/>' % (ix, y0, x1, x0, iy, y1, lines["thin"]))
    nx, ny = len(fr["zones_x"]), fr["zones_y"]
    zw, zh = (x1 - ix) / nx, (y1 - iy) / ny
    for i in range(1, nx):
        a('<path d="M %g %g V %g" style="stroke-width:%g"/>' % (ix + i * zw, y0, iy, lines["thin"]))
    for j in range(1, ny):
        a('<path d="M %g %g H %g" style="stroke-width:%g"/>' % (x0, iy + j * zh, ix, lines["thin"]))
    a('</g>')
    t = '<text x="%g" y="%g" style="font-family:%s;font-size:%gpx;text-anchor:%s;fill:#000;stroke:none"%s>%s</text>'
    for i, c in enumerate(fr["zones_x"]):
        a(t % (ix + (i + .5) * zw, y0 + band * .75, fam, fonts["small"], "middle", "", c))
    for j in range(ny):
        a(t % (x0 + band / 2, iy + (j + .5) * zh + 1, fam, fonts["small"], "middle", "", j + 1))

    tb = std["title_block"]
    rh = tb["row_h"]

    def sh(anchor, x, y):
        return (x + dx, y + dy) if anchor == "br" else (x, y + dy)

    def field(name, x, y, size, anchor="start", value=""):
        return t % (x, y, fam, size, anchor, ' freecad:editable="%s"' % name, "<tspan>%s</tspan>" % escape(value))

    g = ['<g style="fill:none;stroke:#000;stroke-width:%g">' % lines["thin"]]
    for b in tb["boxes"]:
        x, y = sh(b["anchor"], b["x"], b["y"])
        g.append('<rect x="%g" y="%g" width="%g" height="%g" style="stroke-width:%g"/>' % (x, y, b["w"], b["h"], lines["frame"] if b["kind"] != "rev" else lines["thin"]))
    # 修订表: 竖线 + 行线
    rv = tb["rev"]
    rx = [v + dx for v in rv["x"]]
    ry0 = tb["rows_top"] + dy
    nrows = rv["rows"] + 1
    for k in range(1, nrows):
        g.append('<path d="M %g %g H %g"/>' % (rx[0], ry0 + k * rh, rx[-1]))
    for xv in rx[1:-1]:
        g.append('<path d="M %g %g V %g"/>' % (xv, ry0, ry0 + nrows * rh))
    # 标题栏格子
    for c in tb["cells"]:
        x, y = sh("br", c["x"], c["y"])
        g.append('<rect x="%g" y="%g" width="%g" height="%g"/>' % (x, y, c["w"], c.get("h", rh)))
    g.append('</g>')
    out.extend(g)
    # 修订表文字: 行号(下往上 1, 2 …)、表头、可编辑字段
    hy = ry0 + rv["rows"] * rh
    for i, h in enumerate(rv["header"]):
        a(t % ((rx[i] + rx[i + 1]) / 2, hy + rh * .68, fam, fonts["text"], "middle", "", escape(h)))
    for r in range(rv["rows"]):
        y = ry0 + (rv["rows"] - 1 - r) * rh
        a(t % ((rx[0] + rx[1]) / 2, y + rh * .68, fam, fonts["text"], "middle", "", r + 1))
        for i, f in enumerate(rv["fields"][1:], start=1):
            a(field("rev%d_%s" % (r + 1, f), (rx[i] + rx[i + 1]) / 2, y + rh * .68, fonts["small"], "middle"))
    # 标题栏文字
    for c in tb["cells"]:
        x, y = sh("br", c["x"], c["y"])
        h = c.get("h", rh)
        kind = c.get("kind")
        if "label" in c:
            a(t % (x + c["w"] / 2, y + h * .68, fam, fonts["text"], "middle", "", escape(c["label"])))
        elif kind == "projection":
            # 第一角投影符号: 左 = 截锥侧视(梯形), 右 = 两个同心圆
            cx, cy = x + c["w"] / 2, y + h / 2
            a('<g style="fill:none;stroke:#000;stroke-width:%g">' % lines["thin"]
              + '<path d="M %g %g L %g %g L %g %g L %g %g Z"/>' % (cx - 11, cy - 1.6, cx - 1, cy - 3, cx - 1, cy + 3, cx - 11, cy + 1.6)
              + '<circle cx="%g" cy="%g" r="3"/><circle cx="%g" cy="%g" r="1.6"/>' % (cx + 6, cy, cx + 6, cy)
              + '<path d="M %g %g H %g M %g %g V %g" style="stroke-width:%g"/>' % (cx - 13, cy, cx + 10.5, cx + 6, cy - 4.5, cy + 4.5, lines["center"])
              + '</g>')
        elif kind == "logo":
            a(field("logo", x + c["w"] / 2, y + h * .6, fonts["title"], "middle"))
        elif c.get("field"):
            a(field(c["field"], x + 1.5, y + h * .68, fonts["text"]))
    # 技术要求
    tech = next(b for b in tb["boxes"] if b["kind"] == "tech")
    tx, ty = sh("bl", tech["x"], tech["y"])
    a(t % (tx + 2, ty + 7, fam, fonts["title"], "start", "", "技术要求"))
    lines_ = std["tech_requirements"]
    for i in range(12):
        a(field("tech_%d" % (i + 1), tx + 2, ty + 12 + i * 3.4, fonts["small"], "start", lines_[i] if i < len(lines_) else ""))
    # 其余粗糙度(右上角): "其余" + 去除材料符号 + Ra 值
    rg = std["roughness"]
    rx_, ry_ = rg["x"] + dx, rg["y"]
    a(t % (rx_, ry_ + 2, fam, fonts["text"], "end", "", "其余"))
    a('<path d="M %g %g L %g %g L %g %g H %g M %g %g H %g" style="fill:none;stroke:#000;stroke-width:%g"/>'
      % (rx_ + 2, ry_ - 2, rx_ + 5, ry_ + 3, rx_ + 11, ry_ - 7, rx_ + 20, rx_ + 2.6, ry_ - 1, rx_ + 7.4, lines["thin"]))
    a(field("roughness", rx_ + 9, ry_ - 7.8, fonts["text"], "start", std.get("roughness_default", "")))
    a('</svg>')
    return "\n".join(out)


if __name__ == "__main__":
    std = json.load(open(sys.argv[1], encoding="utf-8"))
    paper = sys.argv[2] if len(sys.argv) > 2 else "A3"
    svg = build(std, paper)
    with open(sys.argv[3], "w", encoding="utf-8") as f:
        f.write(svg)
    print("wrote", sys.argv[3])
