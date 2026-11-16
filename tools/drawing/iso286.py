# ISO 286-1 / GB/T 1800.1 极限偏差查表(公称尺寸 ≤ 500 mm, 常用孔 / 轴公差带)。
#   limits(10, "H7") → (0.015, 0.0)        上 / 下极限偏差, 毫米
#   limits(10, "g6") → (-0.005, -0.014)
#   tol_text(10, "H7") → "H7(+0.015/0)"
# 支持: 标准公差 IT4–IT12; 轴 d e f g h js k m n p r s; 孔 D E F G H JS K M N(≤ IT8)P R S(≤ IT7)。
# 孔 K–N / P–S 用"特殊规则": ES = −ei(同字母轴) + Δ, Δ = IT(n) − IT(n−1)。
# 自测: python iso286.py(对照标准表里的几个已知值)。
import re

# 尺寸段上限(mm), 区间 (前一个, 本个]
STEPS = [3, 6, 10, 18, 30, 50, 80, 120, 180, 250, 315, 400, 500]

IT = {  # 标准公差 µm
    4: [3, 4, 4, 5, 6, 7, 8, 10, 12, 14, 16, 18, 20],
    5: [4, 5, 6, 8, 9, 11, 13, 15, 18, 20, 23, 25, 27],
    6: [6, 8, 9, 11, 13, 16, 19, 22, 25, 29, 32, 36, 40],
    7: [10, 12, 15, 18, 21, 25, 30, 35, 40, 46, 52, 57, 63],
    8: [14, 18, 22, 27, 33, 39, 46, 54, 63, 72, 81, 89, 97],
    9: [25, 30, 36, 43, 52, 62, 74, 87, 100, 115, 130, 140, 155],
    10: [40, 48, 58, 70, 84, 100, 120, 140, 160, 185, 210, 230, 250],
    11: [60, 75, 90, 110, 130, 160, 190, 220, 250, 290, 320, 360, 400],
    12: [100, 120, 150, 180, 210, 250, 300, 350, 400, 460, 520, 570, 630],
}

# 轴基本偏差 µm: a–h 给 es(上偏差), k–s 给 ei(下偏差)
SHAFT_ES = {
    "d": [-20, -30, -40, -50, -65, -80, -100, -120, -145, -170, -190, -210, -230],
    "e": [-14, -20, -25, -32, -40, -50, -60, -72, -85, -100, -110, -125, -135],
    "f": [-6, -10, -13, -16, -20, -25, -30, -36, -43, -50, -56, -62, -68],
    "g": [-2, -4, -5, -6, -7, -9, -10, -12, -14, -15, -17, -18, -20],
    "h": [0] * 13,
}
SHAFT_EI = {
    "k": [0, 1, 1, 1, 2, 2, 2, 3, 3, 4, 4, 4, 5],  # IT4–IT7; 其余等级 0
    "m": [2, 4, 6, 7, 8, 9, 11, 13, 15, 17, 20, 21, 23],
    "n": [4, 8, 10, 12, 15, 17, 20, 23, 27, 31, 34, 37, 40],
    "p": [6, 12, 15, 18, 22, 26, 32, 37, 43, 50, 56, 62, 68],
}
# r / s 在 50 mm 以上再细分: (段上限, 值)
SHAFT_EI_FINE = {
    "r": [(3, 10), (6, 15), (10, 19), (18, 23), (30, 28), (50, 34), (65, 41), (80, 43), (100, 51), (120, 54), (140, 63), (160, 65),
          (180, 68), (200, 77), (225, 80), (250, 84), (280, 94), (315, 98), (355, 108), (400, 114), (450, 126), (500, 132)],
    "s": [(3, 14), (6, 19), (10, 23), (18, 28), (30, 35), (50, 43), (65, 53), (80, 59), (100, 71), (120, 79), (140, 92), (160, 100),
          (180, 108), (200, 122), (225, 130), (250, 140), (280, 158), (315, 170), (355, 190), (400, 208), (450, 232), (500, 252)],
}


def step_index(d):
    if not (0 < d <= 500):
        raise ValueError("size %.3f mm out of range (0, 500]" % d)
    for i, hi in enumerate(STEPS):
        if d <= hi + 1e-9:
            return i
    raise ValueError("size out of range")


def it(d, grade):
    if grade not in IT:
        raise ValueError("IT%d not supported (IT4–IT12)" % grade)
    return IT[grade][step_index(d)]


def shaft_ei_fine(letter, d):
    for hi, v in SHAFT_EI_FINE[letter]:
        if d <= hi + 1e-9:
            return v
    raise ValueError("size out of range")


def shaft_dev(d, letter, grade):
    """轴: (es, ei) µm。"""
    t = it(d, grade)
    i = step_index(d)
    if letter in SHAFT_ES:
        es = SHAFT_ES[letter][i]
        return es, es - t
    if letter == "js":
        return t / 2.0, -t / 2.0
    if letter == "k":
        ei = SHAFT_EI["k"][i] if 4 <= grade <= 7 else 0
    elif letter in SHAFT_EI:
        ei = SHAFT_EI[letter][i]
    elif letter in SHAFT_EI_FINE:
        ei = shaft_ei_fine(letter, d)
    else:
        raise ValueError("shaft deviation %s not supported" % letter)
    return ei + t, ei


def hole_dev(d, letter, grade):
    """孔: (ES, EI) µm。字母大写。"""
    t = it(d, grade)
    low = letter.lower()
    if low in SHAFT_ES:  # A–H: EI = −es
        ei = -SHAFT_ES[low][step_index(d)]
        return ei + t, ei
    if low == "js":
        return t / 2.0, -t / 2.0
    if low in ("k", "m", "n"):
        if grade > 8:
            raise ValueError("%s%d not supported (≤ IT8)" % (letter, grade))
        lim = 8
    elif low in ("p", "r", "s"):
        if grade > 7:
            raise ValueError("%s%d not supported (≤ IT7)" % (letter, grade))
        lim = 7
    else:
        raise ValueError("hole deviation %s not supported" % letter)
    _ = lim
    # 特殊规则: ES = −ei(同字母轴, 按本等级) + Δ; ≤ 3 mm 时 Δ = 0
    _, ei_shaft = shaft_dev(d, low, grade)
    delta = 0 if d <= 3 else it(d, grade) - it(d, grade - 1)
    es = -ei_shaft + delta
    return es, es - t


CODE = re.compile(r"^\s*([A-Za-z]{1,2})\s*(\d{1,2})\s*$")


def limits(d, code):
    """公差带代号(H7 / g6 / JS7 / js6 …)→ (上偏差, 下偏差) 毫米。大写 = 孔, 小写 = 轴。"""
    m = CODE.match(code or "")
    if not m:
        raise ValueError("bad tolerance code %r" % code)
    letter, grade = m.group(1), int(m.group(2))
    hi, lo = hole_dev(d, letter, grade) if letter[0].isupper() else shaft_dev(d, letter, grade)
    return round(hi / 1000.0, 4), round(lo / 1000.0, 4)


def fmt_dev(v):
    if abs(v) < 1e-9:
        return "0"
    s = ("%+.4f" % v).rstrip("0").rstrip(".")
    return s


def tol_text(d, code):
    """"H7(+0.015/0)"; 对称(js)写 "js6(±0.0065)"。"""
    hi, lo = limits(d, code)
    if abs(hi + lo) < 1e-9 and hi > 0:
        return "%s(±%s)" % (code.strip(), fmt_dev(hi).lstrip("+"))
    return "%s(%s/%s)" % (code.strip(), fmt_dev(hi), fmt_dev(lo))


def split_fit(fit):
    """"H7/g6" → ("H7", "g6")。"""
    a, _, b = (fit or "").partition("/")
    return a.strip(), b.strip()


if __name__ == "__main__":
    # 标准表里的已知值(µm → mm)
    cases = [
        (10, "H7", (0.015, 0.0)), (10, "g6", (-0.005, -0.014)), (25, "p6", (0.035, 0.022)), (25, "H7", (0.021, 0.0)),
        (15, "K7", (0.006, -0.012)), (15, "M7", (0.0, -0.018)), (15, "N7", (-0.005, -0.023)), (15, "P7", (-0.011, -0.029)),
        (40, "f7", (-0.025, -0.05)), (40, "k6", (0.018, 0.002)), (60, "r6", (0.06, 0.041)), (60, "s6", (0.072, 0.053)),
        (8, "h6", (0.0, -0.009)), (20, "js6", (0.0065, -0.0065)), (100, "H8", (0.054, 0.0)), (30, "F8", (0.053, 0.02)),
        (12, "G7", (0.024, 0.006)), (50, "R7", (-0.025, -0.05)), (2, "P7", (-0.006, -0.016)), (5, "m6", (0.012, 0.004)),
    ]
    bad = 0
    for d, c, want in cases:
        got = limits(d, c)
        ok = abs(got[0] - want[0]) < 1e-6 and abs(got[1] - want[1]) < 1e-6
        bad += not ok
        print("%s %6.1f %-4s %s want %s" % ("ok " if ok else "BAD", d, c, got, want))
    print(tol_text(10, "H7"), tol_text(10, "g6"), tol_text(20, "js6"))
    raise SystemExit(1 if bad else 0)
