//! 标准件识别: `bom.scan` 每行的回转面摘要(shape)→ 规格(M4×12 内六角圆柱头螺钉 / 垫圈 / 螺母 / 销 / 紧定螺钉)。
//!
//! 用途: 零件库匹配的第二层。第一层是几何指纹(完全相同的模型 → 同一料号); 不同来源的同规格件(asset pack 拖进来的、
//! 自己画的、建模细节不同)指纹不同, 但规格相同 → 按规格 key 建议链接到库里的料号。改了长度 = 规格变了 = 新物料。
//! 只认几何, 不看名字(名字随便改都不影响)。认不出来 → None(不瞎猜)。

use serde_json::Value;

/// 识别结果。`key` 是匹配用的规范文本(语言无关), `text()` 是界面显示。
#[derive(Debug, Clone, PartialEq)]
pub struct StdSpec {
    /// screw-cap / screw-csk / screw-hex / screw-set / washer / nut / pin
    pub kind: &'static str,
    /// 公称直径(螺纹件 = M 系列; 销 = 实测直径)。
    pub d: f64,
    /// 长度(螺钉 = 公称长度; 销 = 总长; 螺母 / 垫圈 = 厚度)。
    pub l: f64,
    /// 垫圈外径。
    pub od: Option<f64>,
    pub key: String,
}

const M_SERIES: &[f64] = &[1.6, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 20.0, 24.0];
const SCREW_LEN: &[f64] = &[
    2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 14.0, 15.0, 16.0, 18.0, 20.0, 22.0, 25.0, 28.0, 30.0, 35.0, 40.0, 45.0, 50.0, 55.0, 60.0, 65.0, 70.0, 75.0,
    80.0, 90.0, 100.0, 110.0, 120.0,
];

#[derive(Debug, Clone)]
struct Rev {
    cyl: bool,
    r: f64,
    h: f64,
    inner: bool,
    axis: [f64; 3],
    o: [f64; 3],
}

fn v3(v: &Value) -> Option<[f64; 3]> {
    let a = v.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// 点 p 到 (o, a) 直线的距离。
fn dist_line(p: [f64; 3], o: [f64; 3], a: [f64; 3]) -> f64 {
    let w = [p[0] - o[0], p[1] - o[1], p[2] - o[2]];
    let t = dot(w, a);
    ((w[0] - t * a[0]).powi(2) + (w[1] - t * a[1]).powi(2) + (w[2] - t * a[2]).powi(2)).sqrt()
}

fn fmt(x: f64) -> String {
    let s = format!("{:.2}", x);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// 实测直径 → M 系列(实测在 [0.8, 1.03] × 公称之间: 螺纹画成大径或小径都认), 取最接近的。
fn m_of(dia: f64) -> Option<f64> {
    M_SERIES.iter().copied().filter(|m| dia >= 0.8 * m && dia <= 1.03 * m).min_by(|a, b| (dia - a).abs().total_cmp(&(dia - b).abs()))
}

/// 垫圈内孔 → M(GB/T 97.1: 4.3 / 5.3 / 6.4 / 8.4 / 10.5 …, 即公称 + 0.1 ~ 1)。
fn m_of_washer(inner: f64) -> Option<f64> {
    M_SERIES.iter().copied().filter(|m| inner >= *m - 0.05 && inner <= m * 1.12 + 0.2).min_by(|a, b| (inner - a).abs().total_cmp(&(inner - b).abs()))
}

/// 螺钉长度: 离标准长度 ≤ 0.6 mm 取标准值, 否则取 0.5 的整倍数。
fn std_len(l: f64) -> f64 {
    SCREW_LEN.iter().copied().find(|s| (l - s).abs() <= 0.6).unwrap_or((l * 2.0).round() / 2.0)
}

fn spec(kind: &'static str, d: f64, l: f64, od: Option<f64>) -> StdSpec {
    let key = match kind {
        "washer" => format!("washer M{} OD{} t{}", fmt(d), fmt(od.unwrap_or(0.0)), fmt(l)),
        "nut" => format!("nut M{} m{}", fmt(d), fmt(l)),
        "pin" => format!("pin D{} L{}", fmt(d), fmt(l)),
        _ => format!("{kind} M{}x{}", fmt(d), fmt(l)),
    };
    StdSpec { kind, d, l, od, key }
}

impl StdSpec {
    /// 界面显示(双语)。
    pub fn text(&self) -> String {
        let (d, l) = (fmt(self.d), fmt(self.l));
        match self.kind {
            "screw-cap" => pla_bridge::t!("M{d}×{l} 内六角圆柱头螺钉", "M{d}×{l} socket head cap screw"),
            "screw-csk" => pla_bridge::t!("M{d}×{l} 沉头螺钉", "M{d}×{l} countersunk screw"),
            "screw-hex" => pla_bridge::t!("M{d}×{l} 六角头螺栓", "M{d}×{l} hex bolt"),
            "screw-set" => pla_bridge::t!("M{d}×{l} 紧定螺钉", "M{d}×{l} set screw"),
            "washer" => pla_bridge::t!("垫圈 {d}(⌀{}×{l})", "Washer {d} (⌀{}×{l})", fmt(self.od.unwrap_or(0.0))),
            "nut" => pla_bridge::t!("M{d} 螺母(厚 {l})", "M{d} nut ({l} thick)"),
            "pin" => pla_bridge::t!("圆柱销 ⌀{d}×{l}", "Dowel pin ⌀{d}×{l}"),
            _ => self.key.clone(),
        }
    }
}

/// `bom.scan` 一行的 `shape` → 规格; 认不出 → None。
pub fn classify(shape: &Value) -> Option<StdSpec> {
    let len = shape["len"].as_f64()?;
    let planes = shape["planes"].as_u64().unwrap_or(0);
    let revs: Vec<Rev> = shape["rev"]
        .as_array()?
        .iter()
        .filter_map(|r| {
            Some(Rev {
                cyl: r["t"].as_str()? == "cyl",
                r: r["r"].as_f64()?,
                h: r["h"].as_f64().unwrap_or(0.0),
                inner: r["inner"].as_bool().unwrap_or(false),
                axis: v3(&r["axis"])?,
                o: v3(&r["o"])?,
            })
        })
        .collect();
    let main = revs.iter().filter(|r| r.cyl).max_by(|a, b| (a.r * a.h).total_cmp(&(b.r * b.h)))?;
    // 全部回转面共轴(标准件都是单一回转轴)
    let tol = 0.02 * main.r.max(1.0);
    if !revs.iter().all(|r| dot(r.axis, main.axis).abs() > 0.9999 && dist_line(r.o, main.o, main.axis) < tol) {
        return None;
    }
    let outer: Vec<&Rev> = revs.iter().filter(|r| r.cyl && !r.inner).collect();
    let inner: Vec<&Rev> = revs.iter().filter(|r| r.cyl && r.inner).collect();
    let cones: Vec<&Rev> = revs.iter().filter(|r| !r.cyl).collect();
    let max_r = |v: &[&Rev]| v.iter().map(|r| r.r).fold(0.0, f64::max);
    let min_r = |v: &[&Rev]| v.iter().map(|r| r.r).fold(f64::INFINITY, f64::min);
    // 垫圈: 内孔 + 外圆, 薄
    if !inner.is_empty() && !outer.is_empty() {
        // 平垫圈比例(GB/T 97.1): 外径 / 内孔 1.5 ~ 3.2, 厚 0.05 ~ 0.35 d, 平面只有两个端面(+ 倒角)。
        // 实测排除: 磁环(外 / 内 1.38、厚 0.03d)、带很多平面的薄片零件
        let (di, od) = (2.0 * min_r(&inner), 2.0 * max_r(&outer));
        let ratio = od / di.max(1e-9);
        if planes <= 4 && (1.5..=3.2).contains(&ratio) {
            if let Some(m) = m_of_washer(di).filter(|m| len >= 0.05 * m && len <= 0.35 * m) {
                return Some(spec("washer", m, (len * 100.0).round() / 100.0, Some((od * 100.0).round() / 100.0)));
            }
        }
        return None;
    }
    // 螺母: 内孔 + 六角(平面), 没有外圆柱
    if !inner.is_empty() && outer.is_empty() {
        if planes >= 6 {
            // 内孔画成大径或小径都可能(⌀5 = M5 大径 或 M6 小径)→ 再按厚度 m ≈ 0.8d(GB/T 6170)挑
            let di = 2.0 * min_r(&inner);
            let m = M_SERIES
                .iter()
                .copied()
                .filter(|m| di >= 0.8 * m && di <= 1.03 * m && len > 0.3 * m && len < 1.3 * m)
                .min_by(|a, b| (len / a - 0.85).abs().total_cmp(&(len / b - 0.85).abs()))?;
            return Some(spec("nut", m, (len * 100.0).round() / 100.0, None));
        }
        return None;
    }
    if outer.is_empty() || !inner.is_empty() {
        return None;
    }
    // 外圆柱件: 杆 = 半径最小、且够长的外圆柱
    let shank = outer.iter().filter(|r| r.h >= 1.5 * r.r).min_by(|a, b| a.r.total_cmp(&b.r)).copied()?;
    let head = outer.iter().filter(|r| r.r > shank.r * 1.2).max_by(|a, b| a.r.total_cmp(&b.r)).copied();
    let all_same = outer.iter().all(|r| (r.r - shank.r).abs() < 0.02 * shank.r.max(1.0));
    // 内六角孔: 六个平面侧壁 + 孔底内锥(实测 Plasticity 杯头螺钉: 10 个平面 + 2 个内锥)
    let socket = planes >= 6 || revs.iter().any(|r| !r.cyl && r.inner);
    if let Some(hd) = head {
        let m = m_of(2.0 * shank.r)?;
        let dk = 2.0 * hd.r;
        // 没有内六角孔的台阶轴(实测 XYZ60 台阶销)不是螺钉
        if socket && dk >= 1.3 * m && dk <= 2.1 * m {
            return Some(spec("screw-cap", m, std_len(len - hd.h), None));
        }
        return None;
    }
    if let Some(_c) = cones.iter().find(|c| c.r > shank.r * 1.3) {
        let m = m_of(2.0 * shank.r)?;
        return Some(spec("screw-csk", m, std_len(len), None));
    }
    if all_same {
        if planes >= 6 {
            // 六角平面 + 只有一种外圆: 杆比总长短得多 = 六角头螺栓; 否则 = 内六角紧定螺钉
            let m = m_of(2.0 * shank.r)?;
            // 螺栓公称长度 = 头下到末端 ≈ 杆长(末端倒角 < 0.6 mm 由 std_len 吸收)
            return Some(if shank.h < 0.85 * len { spec("screw-hex", m, std_len(shank.h), None) } else { spec("screw-set", m, std_len(len), None) });
        }
        if planes <= 3 {
            return Some(spec("pin", (2.0 * shank.r * 100.0).round() / 100.0, (len * 100.0).round() / 100.0, None));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 手动: PLA_SHAPES=<bom.scan 导出的 [{name, len, planes, rev:[[t,r,h,in/out,tc]], axis, o:[…]}]> → 打印识别结果。
    #[test]
    #[ignore]
    fn dump_real_shapes() {
        let Ok(p) = std::env::var("PLA_SHAPES") else { return };
        let v: Value = serde_json::from_str(std::fs::read_to_string(p).unwrap().trim_start_matches('\u{feff}')).unwrap();
        for l in v.as_array().unwrap() {
            let rev: Vec<Value> = l["rev"]
                .as_array()
                .unwrap()
                .iter()
                .zip(l["o"].as_array().unwrap())
                .map(|(r, o)| json!({"t": r[0], "r": r[1], "h": r[2], "inner": r[3] == "in", "axis": l["axis"], "o": o}))
                .collect();
            let s = classify(&json!({"len": l["len"], "planes": l["planes"], "rev": rev}));
            println!("{} x{} → {}", l["name"], l["qty"], s.map(|s| s.key).unwrap_or("-".into()));
        }
    }

    fn cyl(r: f64, h: f64, inner: bool) -> Value {
        json!({"t": "cyl", "r": r, "h": h, "inner": inner, "axis": [0, 0, 1], "o": [10, 5, 0]})
    }
    fn cone(r: f64) -> Value {
        json!({"t": "cone", "r": r, "axis": [0, 0, -1], "o": [10, 5, 3]})
    }

    #[test]
    fn screws_washers_nuts_pins() {
        // M4×12 内六角圆柱头: 杆 ⌀4 长 11.6(端部倒角), 头 ⌀7 高 4, 总长 16, 内六角 6 个平面
        let s = classify(&json!({"len": 16.0, "planes": 8, "rev": [cyl(2.0, 11.6, false), cyl(3.5, 4.0, false), cone(1.0)]})).unwrap();
        assert_eq!(s.key, "screw-cap M4x12");
        // 螺纹画成小径(⌀3.3)也认 M4; 改长度 → 新规格
        let s2 = classify(&json!({"len": 20.0, "planes": 8, "rev": [cyl(1.65, 15.5, false), cyl(3.5, 4.0, false)]})).unwrap();
        assert_eq!(s2.key, "screw-cap M4x16");
        // 沉头: 锥面比杆粗
        assert_eq!(classify(&json!({"len": 10.0, "planes": 6, "rev": [cyl(2.5, 7.0, false), cone(5.0)]})).unwrap().key, "screw-csk M5x10");
        // 垫圈 M4: 内孔 4.3, 外径 9, 厚 0.8
        assert_eq!(classify(&json!({"len": 0.8, "planes": 2, "rev": [cyl(2.15, 0.8, true), cyl(4.5, 0.8, false)]})).unwrap().key, "washer M4 OD9 t0.8");
        // 螺母 M6: 内孔 ⌀5 (小径), 六角, 厚 5
        assert_eq!(classify(&json!({"len": 5.0, "planes": 8, "rev": [cyl(2.5, 5.0, true)]})).unwrap().key, "nut M6 m5");
        // 销 ⌀5×20
        assert_eq!(classify(&json!({"len": 20.0, "planes": 2, "rev": [cyl(2.5, 19.2, false), cone(2.3)]})).unwrap().key, "pin D5 L20");
        // 紧定螺钉 M5×8: 只有一种外圆 + 内六角平面, 杆几乎全长
        assert_eq!(classify(&json!({"len": 8.0, "planes": 7, "rev": [cyl(2.5, 7.6, false)]})).unwrap().key, "screw-set M5x8");
        // 不共轴(两根轴)/ 没有回转面 / 粗短的盘 → 认不出
        let off = json!({"t": "cyl", "r": 2.0, "h": 10.0, "inner": false, "axis": [0, 0, 1], "o": [30, 5, 0]});
        assert!(classify(&json!({"len": 16.0, "planes": 8, "rev": [cyl(2.0, 11.6, false), off]})).is_none());
        assert!(classify(&json!({"len": 16.0, "planes": 6, "rev": []})).is_none());
        assert!(classify(&json!({"len": 5.0, "planes": 2, "rev": [cyl(30.0, 5.0, false)]})).is_none());
        assert!(classify(&json!({"planes": 2})).is_none());
        // 实测(encoder calibrator): 杯内六M3x10 —— 杆 r1.5 h9.9, 头 r2.84 h2.7, 总长 13, 10 个平面, 内锥(孔底)
        let inner_cone = json!({"t": "cone", "r": 1.8, "inner": true, "axis": [0, 0, 1], "o": [10, 5, 0]});
        assert_eq!(classify(&json!({"len": 13.0, "planes": 10, "rev": [cyl(1.5, 9.9, false), cyl(2.84, 2.7, false), inner_cone]})).unwrap().key, "screw-cap M3x10");
        // 实测误判排除: 磁环(内 r12.4 外 r17.1 厚 0.62)不是垫圈; 无内六角的台阶销(r3 h5.5 + r5 h3.5, 3 个平面)不是螺钉
        assert!(classify(&json!({"len": 0.62, "planes": 2, "rev": [cyl(12.4, 0.62, true), cyl(17.1, 0.62, false)]})).is_none());
        assert!(classify(&json!({"len": 10.0, "planes": 3, "rev": [cyl(3.0, 5.5, false), cyl(5.0, 3.5, false)]})).is_none());
    }
}
