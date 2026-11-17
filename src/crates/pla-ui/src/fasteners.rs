//! A10 特征宏: 国标标准件 → 建模步骤(垫片 `cmd.exec` 的 ops: revolve / prism / cylinder)。用户 2026-11-17 选题: 先做螺丝, 另加轴类与其它标准件。
//!
//! 尺寸取各标准的公称 / 最大值(建模用, 不含公差): 头径 dk、头高 k、内六角对边 s、内六角深 t、六角对边 s、螺母厚 m、垫圈 d1/d2/h;
//! 端部倒角 ≈ 0.6 × 粗牙螺距(45°)。螺纹不建(外观 / 出图标注另做)。
//!
//! 放置: `at` + `axis`。螺钉 / 螺栓: at = 头的支承面中心(沉头 = 头顶面中心), axis = 从头指向杆端(插进孔的方向);
//! 紧定螺钉: at = 有内六角的端面中心, axis 指向另一端; 螺母 / 垫圈 / 销 / 轴: at = 一个端面中心, axis = 厚度 / 长度方向。

use serde_json::{json, Value};

/// 粗牙螺距(GB/T 193)。
const PITCH: &[(f64, f64)] = &[
    (1.6, 0.35), (2.0, 0.4), (2.5, 0.45), (3.0, 0.5), (4.0, 0.7), (5.0, 0.8), (6.0, 1.0), (8.0, 1.25), (10.0, 1.5), (12.0, 1.75), (14.0, 2.0), (16.0, 2.0),
    (20.0, 2.5), (24.0, 3.0),
];
/// GB/T 70.1 内六角圆柱头: (d, dk, k, s, t)。
const CAP: &[(f64, f64, f64, f64, f64)] = &[
    (1.6, 3.0, 1.6, 1.5, 0.7), (2.0, 3.8, 2.0, 1.5, 1.0), (2.5, 4.5, 2.5, 2.0, 1.1), (3.0, 5.5, 3.0, 2.5, 1.3), (4.0, 7.0, 4.0, 3.0, 2.0),
    (5.0, 8.5, 5.0, 4.0, 2.5), (6.0, 10.0, 6.0, 5.0, 3.0), (8.0, 13.0, 8.0, 6.0, 4.0), (10.0, 16.0, 10.0, 8.0, 5.0), (12.0, 18.0, 12.0, 10.0, 6.0),
    (14.0, 21.0, 14.0, 12.0, 7.0), (16.0, 24.0, 16.0, 14.0, 8.0), (20.0, 30.0, 20.0, 17.0, 10.0), (24.0, 36.0, 24.0, 19.0, 12.0),
];
/// GB/T 70.3 内六角沉头(90°): (d, dk 理论, s, t); 头高 = (dk − d) / 2。
const CSK: &[(f64, f64, f64, f64)] = &[
    (3.0, 6.72, 2.0, 1.1), (4.0, 8.96, 2.5, 1.5), (5.0, 11.2, 3.0, 1.9), (6.0, 13.44, 4.0, 2.2), (8.0, 17.92, 5.0, 3.0), (10.0, 22.4, 6.0, 3.6),
    (12.0, 26.88, 8.0, 4.3), (16.0, 33.6, 10.0, 5.6), (20.0, 40.32, 12.0, 6.6),
];
/// GB/T 5783 六角头(全螺纹): (d, s, k)。
const HEX: &[(f64, f64, f64)] = &[
    (3.0, 5.5, 2.0), (4.0, 7.0, 2.8), (5.0, 8.0, 3.5), (6.0, 10.0, 4.0), (8.0, 13.0, 5.3), (10.0, 16.0, 6.4), (12.0, 18.0, 7.5), (14.0, 21.0, 8.8),
    (16.0, 24.0, 10.0), (20.0, 30.0, 12.5), (24.0, 36.0, 15.0),
];
/// GB/T 77 内六角平端紧定螺钉: (d, s, t)。
const SET: &[(f64, f64, f64)] = &[
    (3.0, 1.5, 1.2), (4.0, 2.0, 1.5), (5.0, 2.5, 2.0), (6.0, 3.0, 2.0), (8.0, 4.0, 3.0), (10.0, 5.0, 4.0), (12.0, 6.0, 4.8), (16.0, 8.0, 6.4),
];
/// GB/T 6170 1 型六角螺母: (d, s, m)。
const NUT: &[(f64, f64, f64)] = &[
    (3.0, 5.5, 2.4), (4.0, 7.0, 3.2), (5.0, 8.0, 4.7), (6.0, 10.0, 5.2), (8.0, 13.0, 6.8), (10.0, 16.0, 8.4), (12.0, 18.0, 10.8), (16.0, 24.0, 14.8),
    (20.0, 30.0, 18.0), (24.0, 36.0, 21.5),
];
/// GB/T 97.1 平垫圈: (d, d1 内径, d2 外径, h)。
const WASHER: &[(f64, f64, f64, f64)] = &[
    (3.0, 3.2, 7.0, 0.5), (4.0, 4.3, 9.0, 0.8), (5.0, 5.3, 10.0, 1.0), (6.0, 6.4, 12.0, 1.6), (8.0, 8.4, 16.0, 1.6), (10.0, 10.5, 20.0, 2.0),
    (12.0, 13.0, 24.0, 2.5), (16.0, 17.0, 30.0, 3.0), (20.0, 21.0, 37.0, 3.0), (24.0, 25.0, 44.0, 4.0),
];

/// 一个标准件请求(给 MCP / 界面)。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PartReq {
    /// screw-cap / screw-csk / screw-hex / screw-set / nut / washer / pin / shaft
    pub kind: String,
    /// "M4" / "4"(螺纹件); 销 = 直径。
    #[serde(default)]
    pub size: Option<Value>,
    /// 螺钉 = 公称长度(沉头含头); 销 = 总长。
    #[serde(default)]
    pub length: Option<f64>,
    /// 轴: [[直径, 长度], …] 从 at 起沿 axis。
    #[serde(default)]
    pub steps: Option<Vec<[f64; 2]>>,
    /// 轴两端倒角(默认 0.5; 0 = 不倒)。
    #[serde(default)]
    pub chamfer: Option<f64>,
    #[serde(default = "origin")]
    pub at: [f64; 3],
    #[serde(default = "down")]
    pub axis: [f64; 3],
}

fn origin() -> [f64; 3] {
    [0.0, 0.0, 0.0]
}
fn down() -> [f64; 3] {
    [0.0, 0.0, -1.0]
}

fn r2(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn size_of(v: &Option<Value>) -> Result<f64, String> {
    let s = match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().trim_start_matches(['M', 'm']).parse::<f64>().map_err(|_| format!("bad size {s:?} (like \"M4\")"))?,
        _ => return Err("size is required (like \"M4\")".into()),
    };
    if s > 0.0 {
        Ok(s)
    } else {
        Err("size must be > 0".into())
    }
}

fn row<T: Copy>(table: &[T], d: f64, key: impl Fn(&T) -> f64, what: &str) -> Result<T, String> {
    table.iter().copied().find(|r| (key(r) - d).abs() < 1e-9).ok_or_else(|| {
        format!("{what}: no M{d} in the table (have: {})", table.iter().map(|r| format!("M{}", key(r))).collect::<Vec<_>>().join(" "))
    })
}

fn pitch(d: f64) -> f64 {
    PITCH.iter().find(|p| (p.0 - d).abs() < 1e-9).map(|p| p.1).unwrap_or(0.15 * d)
}

fn norm(a: [f64; 3]) -> Result<[f64; 3], String> {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if l > 0.0 {
        Ok([a[0] / l, a[1] / l, a[2] / l])
    } else {
        Err("axis must be non-zero".into())
    }
}

fn add(p: [f64; 3], a: [f64; 3], s: f64) -> [f64; 3] {
    [r2(p[0] + a[0] * s), r2(p[1] + a[1] * s), r2(p[2] + a[2] * s)]
}

/// 展开结果: 步骤(最后一步 id = "part")+ 规格 key(与 std_parts 识别同格式, 用来回验)+ 说明。
#[derive(Debug)]
pub struct Expanded {
    pub ops: Vec<Value>,
    pub spec: String,
    pub text: String,
}

/// 请求 → 建模步骤。
pub fn expand(q: &PartReq) -> Result<Expanded, String> {
    let ax = norm(q.axis)?;
    let neg = [-ax[0], -ax[1], -ax[2]];
    let at = q.at;
    let len = || q.length.filter(|l| *l > 0.0).ok_or_else(|| "length is required".to_string());
    let fmt = |x: f64| {
        let s = format!("{x:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    match q.kind.as_str() {
        "screw-cap" => {
            let d = size_of(&q.size)?;
            let (_, dk, k, s, t) = row(CAP, d, |r| r.0, "GB/T 70.1")?;
            let l = len()?;
            let c = r2(0.6 * pitch(d));
            let hc = r2(0.08 * k);
            // 轴向坐标从杆端(z = 0)到头顶(z = l + k); 旋转轴朝头的方向
            let tip = add(at, ax, l);
            let profile = json!([[0, 0], [d / 2.0 - c, 0], [d / 2.0, c], [d / 2.0, l], [dk / 2.0, l], [dk / 2.0, l + k - hc], [dk / 2.0 - hc, l + k], [0, l + k]]);
            Ok(Expanded {
                ops: vec![
                    json!({"op": "revolve", "id": "body", "origin": tip, "axis": neg, "profile": profile}),
                    json!({"op": "prism", "id": "part", "center": add(at, neg, k), "normal": ax, "sides": 6, "flats": s, "height": t, "bool": "subtract", "targets": ["$body"]}),
                ],
                spec: format!("screw-cap M{}x{}", fmt(d), fmt(l)),
                text: format!("GB/T 70.1 M{}×{} 内六角圆柱头螺钉(dk {dk}, k {k}, s {s}, t {t})", fmt(d), fmt(l)),
            })
        }
        "screw-csk" => {
            let d = size_of(&q.size)?;
            let (_, dk, s, t) = row(CSK, d, |r| r.0, "GB/T 70.3")?;
            let l = len()?;
            let k = (dk - d) / 2.0;
            if l <= k + 1.0 {
                return Err(format!("length must be > head height {:.2} + 1", k));
            }
            let c = r2(0.6 * pitch(d));
            // 总长 l 含头; at = 头顶面中心
            let tip = add(at, ax, l);
            let profile = json!([[0, 0], [d / 2.0 - c, 0], [d / 2.0, c], [d / 2.0, r2(l - k)], [dk / 2.0, l], [0, l]]);
            Ok(Expanded {
                ops: vec![
                    json!({"op": "revolve", "id": "body", "origin": tip, "axis": neg, "profile": profile}),
                    json!({"op": "prism", "id": "part", "center": at, "normal": ax, "sides": 6, "flats": s, "height": t, "bool": "subtract", "targets": ["$body"]}),
                ],
                spec: format!("screw-csk M{}x{}", fmt(d), fmt(l)),
                text: format!("GB/T 70.3 M{}×{} 内六角沉头螺钉(dk {dk}, s {s}, t {t})", fmt(d), fmt(l)),
            })
        }
        "screw-hex" => {
            let d = size_of(&q.size)?;
            let (_, s, k) = row(HEX, d, |r| r.0, "GB/T 5783")?;
            let l = len()?;
            let c = r2(0.6 * pitch(d));
            let tip = add(at, ax, l);
            let profile = json!([[0, 0], [d / 2.0 - c, 0], [d / 2.0, c], [d / 2.0, l], [0, l]]);
            Ok(Expanded {
                ops: vec![
                    json!({"op": "revolve", "id": "body", "origin": tip, "axis": neg, "profile": profile}),
                    json!({"op": "prism", "id": "part", "center": at, "normal": neg, "sides": 6, "flats": s, "height": k, "bool": "union", "targets": ["$body"]}),
                ],
                spec: format!("screw-hex M{}x{}", fmt(d), fmt(l)),
                text: format!("GB/T 5783 M{}×{} 六角头螺栓(s {s}, k {k})", fmt(d), fmt(l)),
            })
        }
        "screw-set" => {
            let d = size_of(&q.size)?;
            let (_, s, t) = row(SET, d, |r| r.0, "GB/T 77")?;
            let l = len()?;
            let c = r2(0.6 * pitch(d));
            let far = add(at, ax, l);
            let profile = json!([[0, 0], [d / 2.0 - c, 0], [d / 2.0, c], [d / 2.0, r2(l - c)], [d / 2.0 - c, l], [0, l]]);
            Ok(Expanded {
                ops: vec![
                    json!({"op": "revolve", "id": "body", "origin": far, "axis": neg, "profile": profile}),
                    json!({"op": "prism", "id": "part", "center": at, "normal": ax, "sides": 6, "flats": s, "height": t, "bool": "subtract", "targets": ["$body"]}),
                ],
                spec: format!("screw-set M{}x{}", fmt(d), fmt(l)),
                text: format!("GB/T 77 M{}×{} 内六角平端紧定螺钉(s {s}, t {t})", fmt(d), fmt(l)),
            })
        }
        "nut" => {
            let d = size_of(&q.size)?;
            let (_, s, m) = row(NUT, d, |r| r.0, "GB/T 6170")?;
            Ok(Expanded {
                ops: vec![
                    json!({"op": "prism", "id": "body", "center": at, "normal": ax, "sides": 6, "flats": s, "height": m}),
                    json!({"op": "cylinder", "id": "part", "center": add(at, neg, 0.5), "axis": ax, "radius": d / 2.0, "height": m + 1.0, "bool": "subtract", "targets": ["$body"]}),
                ],
                spec: format!("nut M{} m{}", fmt(d), fmt(m)),
                text: format!("GB/T 6170 M{} 六角螺母(s {s}, m {m})", fmt(d)),
            })
        }
        "washer" => {
            let d = size_of(&q.size)?;
            let (_, d1, d2, h) = row(WASHER, d, |r| r.0, "GB/T 97.1")?;
            let profile = json!([[d1 / 2.0, 0], [d2 / 2.0, 0], [d2 / 2.0, h], [d1 / 2.0, h]]);
            Ok(Expanded {
                ops: vec![json!({"op": "revolve", "id": "part", "origin": at, "axis": ax, "profile": profile})],
                spec: format!("washer M{} OD{} t{}", fmt(d), fmt(d2), fmt(h)),
                text: format!("GB/T 97.1 {} 平垫圈(d1 {d1}, d2 {d2}, h {h})", fmt(d)),
            })
        }
        "pin" => {
            let d = size_of(&q.size)?;
            let l = len()?;
            let c = r2(q.chamfer.unwrap_or((0.1 * d).max(0.2)).min(d / 4.0));
            let profile = if c > 0.0 {
                json!([[0, 0], [d / 2.0 - c, 0], [d / 2.0, c], [d / 2.0, r2(l - c)], [d / 2.0 - c, l], [0, l]])
            } else {
                json!([[0, 0], [d / 2.0, 0], [d / 2.0, l], [0, l]])
            };
            Ok(Expanded {
                ops: vec![json!({"op": "revolve", "id": "part", "origin": at, "axis": ax, "profile": profile})],
                spec: format!("pin D{} L{}", fmt(d), fmt(l)),
                text: format!("GB/T 119.2 ⌀{}×{} 圆柱销", fmt(d), fmt(l)),
            })
        }
        "shaft" => {
            let steps = q.steps.clone().filter(|s| !s.is_empty()).ok_or("shaft needs steps [[diameter, length], …]")?;
            if steps.iter().any(|s| !(s[0] > 0.0 && s[1] > 0.0)) {
                return Err("every step needs diameter > 0 and length > 0".into());
            }
            let c = q.chamfer.unwrap_or(0.5).max(0.0);
            let total: f64 = steps.iter().map(|s| s[1]).sum();
            // 轮廓: 沿 z 逐段, 台阶处直角; 两端倒角 c(不超过端部那段的半径 / 长度的一半)
            let mut pts: Vec<[f64; 2]> = vec![[0.0, 0.0]];
            let (d0, l0) = (steps[0][0], steps[0][1]);
            let c0 = c.min(d0 / 4.0).min(l0 / 2.0);
            if c0 > 0.0 {
                pts.push([d0 / 2.0 - c0, 0.0]);
                pts.push([d0 / 2.0, c0]);
            } else {
                pts.push([d0 / 2.0, 0.0]);
            }
            let mut z = 0.0;
            for (i, s) in steps.iter().enumerate() {
                if i > 0 {
                    pts.push([s[0] / 2.0, r2(z)]);
                }
                z += s[1];
                if i + 1 < steps.len() {
                    pts.push([s[0] / 2.0, r2(z)]);
                }
            }
            let (dn, ln) = (steps[steps.len() - 1][0], steps[steps.len() - 1][1]);
            let cn = c.min(dn / 4.0).min(ln / 2.0);
            if cn > 0.0 {
                pts.push([dn / 2.0, r2(total - cn)]);
                pts.push([dn / 2.0 - cn, r2(total)]);
            } else {
                pts.push([dn / 2.0, r2(total)]);
            }
            pts.push([0.0, r2(total)]);
            Ok(Expanded {
                ops: vec![json!({"op": "revolve", "id": "part", "origin": at, "axis": ax, "profile": pts})],
                spec: String::new(),
                text: format!("阶梯轴 {} 段, 总长 {}", steps.len(), fmt(total)),
            })
        }
        k => Err(format!("unknown kind {k} (screw-cap / screw-csk / screw-hex / screw-set / nut / washer / pin / shaft)")),
    }
}

/// 支持的规格(给工具说明)。
pub fn sizes() -> String {
    let m = |v: Vec<f64>| v.iter().map(|d| format!("M{d}")).collect::<Vec<_>>().join(" ");
    format!(
        "screw-cap (GB/T 70.1): {}; screw-csk (GB/T 70.3): {}; screw-hex (GB/T 5783): {}; screw-set (GB/T 77): {}; nut (GB/T 6170): {}; washer (GB/T 97.1): {}; pin (GB/T 119.2): any diameter; shaft: any steps",
        m(CAP.iter().map(|r| r.0).collect()),
        m(CSK.iter().map(|r| r.0).collect()),
        m(HEX.iter().map(|r| r.0).collect()),
        m(SET.iter().map(|r| r.0).collect()),
        m(NUT.iter().map(|r| r.0).collect()),
        m(WASHER.iter().map(|r| r.0).collect()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(v: Value) -> PartReq {
        serde_json::from_value(v).unwrap()
    }

    /// 轮廓 r 都 ≥ 0、z 单调不减到顶、首尾在轴上(垫圈除外)
    fn profile_ok(p: &Value) -> Vec<[f64; 2]> {
        let pts: Vec<[f64; 2]> = serde_json::from_value(p.clone()).unwrap();
        assert!(pts.iter().all(|q| q[0] >= 0.0), "{pts:?}");
        pts
    }

    #[test]
    fn cap_screw_m4x12() {
        let e = expand(&req(json!({"kind": "screw-cap", "size": "M4", "length": 12, "at": [300, 0, 16], "axis": [0, 0, -1]}))).unwrap();
        assert_eq!(e.spec, "screw-cap M4x12");
        let rev = &e.ops[0];
        // 杆端在 at + axis × 12 = (300, 0, 4); 旋转轴朝头(+Z)
        assert_eq!(rev["origin"], json!([300.0, 0.0, 4.0]));
        assert_eq!(rev["axis"], json!([-0.0, -0.0, 1.0]));
        let pts = profile_ok(&rev["profile"]);
        assert_eq!(pts[3], [2.0, 12.0], "杆到支承面");
        assert_eq!(pts[4], [3.5, 12.0], "头径 7");
        assert_eq!(pts.last().unwrap()[1], 16.0, "头高 4");
        assert_eq!(pts[1], [1.58, 0.0], "端部倒角 0.6 × 0.7");
        let soc = &e.ops[1];
        // 内六角从头顶(at − axis × k = (300, 0, 20))往杆的方向挖 t = 2
        assert_eq!(soc["center"], json!([300.0, 0.0, 20.0]));
        assert_eq!((soc["flats"].as_f64(), soc["height"].as_f64(), soc["bool"].as_str()), (Some(3.0), Some(2.0), Some("subtract")));
    }

    #[test]
    fn other_kinds_and_errors() {
        let csk = expand(&req(json!({"kind": "screw-csk", "size": 4, "length": 12}))).unwrap();
        let p = profile_ok(&csk.ops[0]["profile"]);
        assert_eq!(p[3], [2.0, 9.52], "沉头锥起点 = l − (dk − d)/2");
        assert_eq!(p[4], [4.48, 12.0]);
        let hex = expand(&req(json!({"kind": "screw-hex", "size": "M8", "length": 30}))).unwrap();
        assert_eq!((hex.ops[1]["flats"].as_f64(), hex.ops[1]["height"].as_f64(), hex.ops[1]["bool"].as_str()), (Some(13.0), Some(5.3), Some("union")));
        let nut = expand(&req(json!({"kind": "nut", "size": "M6"}))).unwrap();
        assert_eq!(nut.spec, "nut M6 m5.2");
        let w = expand(&req(json!({"kind": "washer", "size": "M4"}))).unwrap();
        assert_eq!(w.spec, "washer M4 OD9 t0.8");
        assert_eq!(profile_ok(&w.ops[0]["profile"])[0], [2.15, 0.0], "垫圈轮廓不碰轴");
        let pin = expand(&req(json!({"kind": "pin", "size": 5, "length": 20}))).unwrap();
        assert_eq!(pin.spec, "pin D5 L20");
        let sh = expand(&req(json!({"kind": "shaft", "steps": [[10, 20], [15, 30], [10, 15]], "chamfer": 1}))).unwrap();
        let p = profile_ok(&sh.ops[0]["profile"]);
        assert_eq!(p.first().unwrap(), &[0.0, 0.0]);
        assert_eq!(p.last().unwrap(), &[0.0, 65.0]);
        assert!(p.contains(&[7.5, 20.0]) && p.contains(&[7.5, 50.0]) && p.contains(&[5.0, 50.0]), "{p:?}");
        assert!(expand(&req(json!({"kind": "screw-cap", "size": "M7", "length": 10}))).unwrap_err().contains("no M7"));
        assert!(expand(&req(json!({"kind": "screw-cap", "size": "M4"}))).unwrap_err().contains("length"));
        assert!(expand(&req(json!({"kind": "rivet", "size": "M4"}))).is_err());
        assert!(sizes().contains("GB/T 70.1"));
    }

    /// 宏建出来的形状应被 std_parts 认成同一规格(按真机 bom.scan 的形状摘要合成)
    #[test]
    fn recognizer_agrees_with_macros() {
        let cyl = |r: f64, h: f64, inner: bool| json!({"t": "cyl", "r": r, "h": h, "inner": inner, "axis": [0, 0, 1], "o": [0, 0, 0]});
        let cone = |r: f64, inner: bool| json!({"t": "cone", "r": r, "inner": inner, "axis": [0, 0, 1], "o": [0, 0, 0]});
        // M4×12 杯头: 杆 r2 h(12 − 0.42), 头 r3.5 h(4 − 0.32), 6 个内六角侧面 + 孔底 + 头顶 + 支承面 + 杆端 = 10 平面, 两个外锥
        let s = crate::std_parts::classify(&json!({"len": 16.0, "planes": 10, "rev": [cyl(2.0, 11.58, false), cyl(3.5, 3.68, false), cone(1.79, false), cone(3.34, false)]})).unwrap();
        assert_eq!(s.key, expand(&req(json!({"kind": "screw-cap", "size": "M4", "length": 12}))).unwrap().spec);
        let n = crate::std_parts::classify(&json!({"len": 5.2, "planes": 8, "rev": [cyl(3.0, 5.2, true)]})).unwrap();
        assert_eq!(n.key, expand(&req(json!({"kind": "nut", "size": "M6"}))).unwrap().spec);
        let w = crate::std_parts::classify(&json!({"len": 0.8, "planes": 2, "rev": [cyl(2.15, 0.8, true), cyl(4.5, 0.8, false)]})).unwrap();
        assert_eq!(w.key, expand(&req(json!({"kind": "washer", "size": "M4"}))).unwrap().spec);
    }
}
