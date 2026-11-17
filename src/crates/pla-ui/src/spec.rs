//! A7 / A8 设计规格层(PRD-ai-modeling-v2 §3–4): 大模型写"参数表 + 特征序列 + 验收条件", 本模块确定性地把它编成建模动作,
//! 并在每步之后按**语义引用**(`face(base.top)`、`edge(face(a.top), face(b.right))`)在当前几何里找面 / 边 —— 不用会变的编号。
//! 这里只有纯逻辑(可单元测试); 逐步执行 / 查询 / 回滚在 pla-mcp 的 `model_build`。
//!
//! 规格:
//! ```json
//! { "name": "sensor_bracket",
//!   "params": { "w": 40, "d": 20, "t": 5, "h": 30, "hd": "M3.clearance" },
//!   "features": [
//!     { "id": "base", "type": "box", "size": ["w", "d", "t"], "at": [0, 0, 0] },
//!     { "id": "upright", "type": "box", "size": ["t", "d", "h"], "at": [0, 0, 0], "bool": "union", "into": "base" },
//!     { "id": "r", "type": "fillet", "edges": ["edge(face(base.top), face(upright.right))"], "r": 3 },
//!     { "id": "holes", "type": "hole", "on": "face(base.top)", "d": "hd", "pattern": { "start": [17, 10, 5], "grid": [2, 1], "pitch": [16, 0] } } ],
//!   "checks": [ { "bbox": [[0, 0, 0], ["w", "d", "h"]] }, { "count": { "Cylinder": 3 } } ] }
//! ```
//! 数值处处可写表达式(`"w - 2*t"`), 名字 = 参数或标准规格(`M3.clearance` 过孔 GB/T 5277 中等, `M4.tap` 螺纹底孔 = d − P)。

use std::collections::BTreeMap;

use serde_json::{json, Value};

// ---------------------------------------------------------------- 表达式

/// GB/T 5277 中等装配过孔。
const CLEARANCE: &[(f64, f64)] = &[
    (1.6, 1.8), (2.0, 2.4), (2.5, 2.9), (3.0, 3.4), (4.0, 4.5), (5.0, 5.5), (6.0, 6.6), (8.0, 9.0), (10.0, 11.0), (12.0, 13.5), (14.0, 15.5), (16.0, 17.5),
    (20.0, 22.0), (24.0, 26.0),
];

fn std_ref(name: &str) -> Option<f64> {
    let (m, what) = name.split_once('.')?;
    let d: f64 = m.strip_prefix(['M', 'm'])?.parse().ok()?;
    match what {
        "clearance" => CLEARANCE.iter().find(|c| (c.0 - d).abs() < 1e-9).map(|c| c.1),
        "tap" => Some(((d - crate::fasteners::pitch(d)) * 100.0).round() / 100.0),
        "d" => Some(d),
        _ => None,
    }
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
}

/// 参数环境: 名字 → 值(惰性求值, 查环)。
pub struct Params<'a> {
    raw: &'a BTreeMap<String, Value>,
    cache: std::cell::RefCell<BTreeMap<String, f64>>,
    stack: std::cell::RefCell<Vec<String>>,
}

impl<'a> Params<'a> {
    pub fn new(raw: &'a BTreeMap<String, Value>) -> Self {
        Params { raw, cache: Default::default(), stack: Default::default() }
    }

    fn name(&self, n: &str) -> Result<f64, String> {
        if let Some(v) = self.cache.borrow().get(n) {
            return Ok(*v);
        }
        let Some(raw) = self.raw.get(n) else {
            return std_ref(n).ok_or_else(|| format!("unknown name {n:?} (not a param; standard refs look like M3.clearance / M4.tap)"));
        };
        if self.stack.borrow().iter().any(|s| s == n) {
            return Err(format!("parameter {n} refers to itself"));
        }
        self.stack.borrow_mut().push(n.to_string());
        let v = self.num(raw);
        self.stack.borrow_mut().pop();
        let v = v.map_err(|e| format!("param {n}: {e}"))?;
        self.cache.borrow_mut().insert(n.to_string(), v);
        Ok(v)
    }

    /// 数值或表达式。
    pub fn num(&self, v: &Value) -> Result<f64, String> {
        match v {
            Value::Number(n) => n.as_f64().ok_or_else(|| "bad number".into()),
            Value::String(s) => self.expr(s),
            other => Err(format!("expected a number or expression, got {other}")),
        }
    }

    pub fn expr(&self, s: &str) -> Result<f64, String> {
        let mut lx = Lexer { s: s.as_bytes(), i: 0 };
        let v = self.sum(&mut lx)?;
        if lx.peek().is_some() {
            return Err(format!("unexpected text in {s:?} at {}", lx.i));
        }
        if !v.is_finite() {
            return Err(format!("{s:?} is not finite"));
        }
        Ok(v)
    }

    fn sum(&self, lx: &mut Lexer) -> Result<f64, String> {
        let mut v = self.prod(lx)?;
        while let Some(c @ (b'+' | b'-')) = lx.peek() {
            lx.i += 1;
            let r = self.prod(lx)?;
            v = if c == b'+' { v + r } else { v - r };
        }
        Ok(v)
    }

    fn prod(&self, lx: &mut Lexer) -> Result<f64, String> {
        let mut v = self.atom(lx)?;
        while let Some(c @ (b'*' | b'/')) = lx.peek() {
            lx.i += 1;
            let r = self.atom(lx)?;
            v = if c == b'*' { v * r } else { v / r };
        }
        Ok(v)
    }

    fn atom(&self, lx: &mut Lexer) -> Result<f64, String> {
        match lx.peek() {
            Some(b'-') => {
                lx.i += 1;
                Ok(-self.atom(lx)?)
            }
            Some(b'(') => {
                lx.i += 1;
                let v = self.sum(lx)?;
                if lx.peek() != Some(b')') {
                    return Err("missing )".into());
                }
                lx.i += 1;
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                let st = lx.i;
                while lx.i < lx.s.len() && (lx.s[lx.i].is_ascii_digit() || lx.s[lx.i] == b'.') {
                    lx.i += 1;
                }
                std::str::from_utf8(&lx.s[st..lx.i]).unwrap_or("").parse().map_err(|_| "bad number".into())
            }
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => {
                let st = lx.i;
                while lx.i < lx.s.len() && (lx.s[lx.i].is_ascii_alphanumeric() || matches!(lx.s[lx.i], b'_' | b'.')) {
                    lx.i += 1;
                }
                self.name(std::str::from_utf8(&lx.s[st..lx.i]).unwrap_or(""))
            }
            _ => Err("expected a value".into()),
        }
    }

    fn v3(&self, v: &Value, what: &str) -> Result<[f64; 3], String> {
        let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| format!("{what} must be [x, y, z]"))?;
        Ok([self.num(&a[0])?, self.num(&a[1])?, self.num(&a[2])?])
    }

    /// 把一棵 JSON 里所有"数值位"求值(数组 / 对象逐层; 字符串当表达式, 求不出就原样留着)。
    fn eval_tree(&self, v: &Value) -> Value {
        match v {
            Value::String(s) => self.expr(s).map(|x| json!(x)).unwrap_or_else(|_| v.clone()),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.eval_tree(x)).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), if k == "kind" || k == "op" || k == "bool" || k == "type" { x.clone() } else { self.eval_tree(x) })).collect()),
            _ => v.clone(),
        }
    }
}

// ---------------------------------------------------------------- 规格 / 特征

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Spec {
    pub name: String,
    #[serde(default)]
    pub units: Option<String>,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    pub features: Vec<Value>,
    #[serde(default)]
    pub checks: Vec<Value>,
    #[serde(default)]
    pub intent: Option<String>,
}

/// 一个特征自己的包围盒(按规格算, 合并前), 解析 `face(id.side)` 用。
pub type Bbox = [[f64; 3]; 2];

/// 编译状态: 每个特征在哪个"体"里、自己的包围盒; 每个体当前的实体 id。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    /// 特征 id → (所在的体, 自己的包围盒)
    pub features: BTreeMap<String, (String, Option<Bbox>)>,
    /// 体 → 当前实体 id
    pub bodies: BTreeMap<String, i64>,
}

/// 一个建模动作。
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// cmd.exec 的步骤(引用已解析); 产物写进 body。
    Exec { ops: Vec<Value>, body: String },
    /// 一个孔: hole.make 参数(不含 face), face 引用在执行前按当前几何解析。
    Hole { face: String, args: Value },
}

/// 一个特征 → 动作 + 它的体 / 包围盒。引用(边 / 面)在这一步就要解析的(圆角)用 `geo` 解析; 孔的面留到执行时。
#[derive(Debug)]
pub struct Compiled {
    pub actions: Vec<Action>,
    pub body: String,
    pub bbox: Option<Bbox>,
}

fn axis_bbox(center: [f64; 3], axis: [f64; 3], r: f64, z0: f64, z1: f64) -> Option<Bbox> {
    // 只在轴与世界轴对齐时给精确包围盒
    let i = (0..3).find(|&i| (axis[i].abs() - 1.0).abs() < 1e-9)?;
    let mut mn = [0.0; 3];
    let mut mx = [0.0; 3];
    for k in 0..3 {
        if k == i {
            let (a, b) = (center[k] + axis[k] * z0, center[k] + axis[k] * z1);
            mn[k] = a.min(b);
            mx[k] = a.max(b);
        } else {
            mn[k] = center[k] - r;
            mx[k] = center[k] + r;
        }
    }
    Some([mn, mx])
}

fn norm3(a: [f64; 3]) -> Result<[f64; 3], String> {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if l > 0.0 {
        Ok([a[0] / l, a[1] / l, a[2] / l])
    } else {
        Err("axis must be non-zero".into())
    }
}

/// 打孔向导的面内两轴(与垫片 planeAxes 同一规则): u = 与法向最不平行的世界轴投到面上, v = n × u。
pub fn plane_axes(n: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let base = if n[2].abs() > 0.9 { [1.0, 0.0, 0.0] } else if n[0].abs() > 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let d = base[0] * n[0] + base[1] * n[1] + base[2] * n[2];
    let u = norm3([base[0] - d * n[0], base[1] - d * n[1], base[2] - d * n[2]]).unwrap_or([1.0, 0.0, 0.0]);
    let v = [n[1] * u[2] - n[2] * u[1], n[2] * u[0] - n[0] * u[2], n[0] * u[1] - n[1] * u[0]];
    (u, v)
}

/// 编译一个特征。`geo` = 当前几何查询结果(`geo.query detail: all` 的 faces / edges), 圆角 / 倒角解析边要用。
pub fn compile(f: &Value, p: &Params, st: &State, geo: &Value) -> Result<Compiled, String> {
    let id = f["id"].as_str().filter(|s| !s.is_empty()).ok_or("every feature needs an id")?.to_string();
    let ty = f["type"].as_str().ok_or_else(|| format!("{id}: missing type"))?;
    let at = |key: &str| -> Result<[f64; 3], String> { f.get(key).map(|v| p.v3(v, key)).unwrap_or(Ok([0.0; 3])) };
    // 布尔: into = 并进 / 减掉 / 交于 哪个体(按特征 id 找它所在的体)
    let (bool_, into_body) = match (f["bool"].as_str(), f["into"].as_str()) {
        (Some(b), Some(into)) => {
            let body = st.features.get(into).map(|x| x.0.clone()).ok_or_else(|| format!("{id}: into {into:?} is not an earlier feature"))?;
            let solid = *st.bodies.get(&body).ok_or_else(|| format!("{id}: body {body} has no solid yet"))?;
            (Some((b.to_string(), solid)), Some(body))
        }
        (Some(_), None) => return Err(format!("{id}: bool needs into (an earlier feature id)")),
        _ => (None, None),
    };
    let body = into_body.unwrap_or_else(|| id.clone());
    let with_bool = |mut op: Value| {
        if let Some((b, solid)) = &bool_ {
            op["bool"] = json!(b);
            op["targets"] = json!([solid]);
        }
        op
    };
    match ty {
        "box" => {
            let s = f["size"].as_array().filter(|a| a.len() == 3).ok_or_else(|| format!("{id}: size must be [x, y, z]"))?;
            let size = [p.num(&s[0])?, p.num(&s[1])?, p.num(&s[2])?];
            if size.iter().any(|x| *x <= 0.0) {
                return Err(format!("{id}: size must be > 0"));
            }
            let a = at("at")?;
            let op = with_bool(json!({"op": "box", "id": id, "p1": a, "p2": [a[0] + size[0], a[1] + size[1], a[2]], "height": size[2]}));
            Ok(Compiled { actions: vec![Action::Exec { ops: vec![op], body: body.clone() }], body, bbox: Some([a, [a[0] + size[0], a[1] + size[1], a[2] + size[2]]]) })
        }
        "cylinder" => {
            let r = match (f.get("d"), f.get("r")) {
                (Some(d), _) => p.num(d)? / 2.0,
                (None, Some(r)) => p.num(r)?,
                _ => return Err(format!("{id}: cylinder needs d or r")),
            };
            let h = p.num(f.get("h").ok_or_else(|| format!("{id}: cylinder needs h"))?)?;
            let a = at("at")?;
            let ax = norm3(f.get("axis").map(|v| p.v3(v, "axis")).unwrap_or(Ok([0.0, 0.0, 1.0]))?)?;
            let op = with_bool(json!({"op": "cylinder", "id": id, "center": a, "axis": ax, "radius": r, "height": h}));
            Ok(Compiled { actions: vec![Action::Exec { ops: vec![op], body: body.clone() }], body, bbox: axis_bbox(a, ax, r, 0.0, h) })
        }
        "revolve" | "prism" => {
            let mut op = p.eval_tree(f);
            let o = op.as_object_mut().ok_or("bad feature")?;
            for k in ["type", "into"] {
                o.remove(k);
            }
            o.insert("op".into(), json!(ty));
            o.remove("bool");
            let op = with_bool(op);
            let bbox = if ty == "revolve" {
                let org = p.v3(&f["origin"], "origin")?;
                let ax = norm3(p.v3(&f["axis"], "axis")?)?;
                let prof: Vec<[f64; 2]> = serde_json::from_value(op["profile"].clone()).map_err(|_| format!("{id}: profile must be [[r, z], …] numbers"))?;
                let rmax = prof.iter().map(|q| q[0]).fold(0.0, f64::max);
                let (z0, z1) = prof.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), q| (a.min(q[1]), b.max(q[1])));
                axis_bbox(org, ax, rmax, z0, z1)
            } else {
                let c = p.v3(&f["center"], "center")?;
                let n = norm3(f.get("normal").map(|v| p.v3(v, "normal")).unwrap_or(Ok([0.0, 0.0, 1.0]))?)?;
                let r = match (op["radius"].as_f64(), op["flats"].as_f64(), op["sides"].as_f64()) {
                    (Some(r), _, _) => r,
                    (None, Some(fl), Some(sd)) => fl / (2.0 * (std::f64::consts::PI / sd).cos()),
                    _ => 0.0,
                };
                let h = op["height"].as_f64().unwrap_or(0.0);
                axis_bbox(c, n, r, h.min(0.0), h.max(0.0))
            };
            Ok(Compiled { actions: vec![Action::Exec { ops: vec![op], body: body.clone() }], body, bbox })
        }
        "part" => {
            if bool_.is_some() {
                return Err(format!("{id}: standard parts are separate solids (no bool)"));
            }
            let q: crate::fasteners::PartReq = serde_json::from_value(p.eval_tree(f)).map_err(|e| format!("{id}: {e}"))?;
            let x = crate::fasteners::expand(&q).map_err(|e| format!("{id}: {e}"))?;
            Ok(Compiled { actions: vec![Action::Exec { ops: x.ops, body: id.clone() }], body: id, bbox: None })
        }
        "fillet" | "chamfer" => {
            let dist = p.num(f.get("r").or(f.get("d")).ok_or_else(|| format!("{id}: {ty} needs r (or d)"))?)?;
            let refs = f["edges"].as_array().filter(|a| !a.is_empty()).ok_or_else(|| format!("{id}: edges must be a non-empty list"))?;
            let mut ids: Vec<String> = Vec::new();
            let mut bodies: Vec<String> = Vec::new();
            for r in refs {
                let (es, b) = resolve_edges(r, p, st, geo)?;
                bodies.push(b);
                ids.extend(es);
            }
            bodies.dedup();
            if bodies.len() != 1 {
                return Err(format!("{id}: edges must all be on one body (got {})", bodies.join(", ")));
            }
            let body = bodies.remove(0);
            ids.dedup();
            Ok(Compiled { actions: vec![Action::Exec { ops: vec![json!({"op": ty, "id": id, "edges": ids, "distance": dist})], body: body.clone() }], body, bbox: None })
        }
        "hole" => {
            let on = f["on"].as_str().ok_or_else(|| format!("{id}: hole needs on (a face ref like face(base.top))"))?.to_string();
            let (feat, side) = parse_face_ref(&on)?;
            let body = st.features.get(&feat).map(|x| x.0.clone()).ok_or_else(|| format!("{id}: {feat} is not an earlier feature"))?;
            let n = side_normal(&side)?;
            let mut args = serde_json::Map::new();
            args.insert("type".into(), json!(f["kind"].as_str().unwrap_or("simple")));
            for k in ["d", "depth", "cbD", "cbDepth", "cskD", "tapD"] {
                if let Some(v) = f.get(k) {
                    args.insert(k.into(), json!(p.num(v).map_err(|e| format!("{id}.{k}: {e}"))?));
                }
            }
            if !args.contains_key("d") {
                return Err(format!("{id}: hole needs d (number, expression or M3.clearance)"));
            }
            let pts: Vec<[f64; 3]> = if let Some(pat) = f.get("pattern") {
                let start = p.v3(&pat["start"], "pattern.start")?;
                let g = pat["grid"].as_array().filter(|a| a.len() == 2).ok_or_else(|| format!("{id}: pattern.grid must be [nu, nv]"))?;
                let pi = pat["pitch"].as_array().filter(|a| a.len() == 2).ok_or_else(|| format!("{id}: pattern.pitch must be [pu, pv]"))?;
                let (nu, nv) = (p.num(&g[0])? as usize, p.num(&g[1])? as usize);
                let (pu, pv) = (p.num(&pi[0])?, p.num(&pi[1])?);
                let (u, v) = plane_axes(n);
                let mut out = Vec::new();
                for j in 0..nv.max(1) {
                    for i in 0..nu.max(1) {
                        let (a, b) = (i as f64 * pu, j as f64 * pv);
                        out.push([start[0] + u[0] * a + v[0] * b, start[1] + u[1] * a + v[1] * b, start[2] + u[2] * a + v[2] * b]);
                    }
                }
                out
            } else if let Some(list) = f["positions"].as_array() {
                list.iter().map(|q| p.v3(q, "positions[]")).collect::<Result<_, _>>()?
            } else {
                vec![p.v3(f.get("at").ok_or_else(|| format!("{id}: hole needs at, positions or pattern"))?, "at")?]
            };
            let actions = pts
                .into_iter()
                .map(|q| {
                    let mut a = args.clone();
                    a.insert("at".into(), json!(q));
                    Action::Hole { face: on.clone(), args: Value::Object(a) }
                })
                .collect();
            Ok(Compiled { actions, body, bbox: None })
        }
        t => Err(format!("{id}: unknown feature type {t} (box / cylinder / revolve / prism / part / hole / fillet / chamfer)")),
    }
}

// ---------------------------------------------------------------- 语义引用

fn side_normal(side: &str) -> Result<[f64; 3], String> {
    Ok(match side {
        "top" => [0.0, 0.0, 1.0],
        "bottom" => [0.0, 0.0, -1.0],
        "right" => [1.0, 0.0, 0.0],
        "left" => [-1.0, 0.0, 0.0],
        "back" => [0.0, 1.0, 0.0],
        "front" => [0.0, -1.0, 0.0],
        s => return Err(format!("unknown side {s:?} (top / bottom / left / right / front / back)")),
    })
}

/// `face(base.top)` → ("base", "top")。
pub fn parse_face_ref(s: &str) -> Result<(String, String), String> {
    let inner = s.trim().strip_prefix("face(").and_then(|x| x.strip_suffix(')')).ok_or_else(|| format!("{s:?} is not face(feature.side)"))?;
    let (f, side) = inner.split_once('.').ok_or_else(|| format!("{s:?}: write face(feature.side)"))?;
    Ok((f.trim().to_string(), side.trim().to_string()))
}

fn arr3(v: &Value) -> Option<[f64; 3]> {
    let a = v.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
}

/// 解析面引用 → (面 id, 所在的体)。规则: 该特征所在体的当前实体里, 法向 = 那一侧、中心落在特征自身包围盒那一侧表面上的平面; 多个取离表面中心最近的。
pub fn resolve_face(r: &str, st: &State, geo: &Value) -> Result<(String, String), String> {
    let (feat, side) = parse_face_ref(r)?;
    let (body, bbox) = st.features.get(&feat).cloned().ok_or_else(|| format!("{r}: {feat} is not an earlier feature"))?;
    let bb = bbox.ok_or_else(|| format!("{r}: {feat} has no known extent (standard parts / fillets can't be referenced this way)"))?;
    let solid = *st.bodies.get(&body).ok_or_else(|| format!("{r}: body {body} has no solid"))?;
    let n = side_normal(&side)?;
    let ax = (0..3).find(|&i| n[i] != 0.0).unwrap_or(2);
    let plane_at = if n[ax] > 0.0 { bb[1][ax] } else { bb[0][ax] };
    let mut want = [(bb[0][0] + bb[1][0]) / 2.0, (bb[0][1] + bb[1][1]) / 2.0, (bb[0][2] + bb[1][2]) / 2.0];
    want[ax] = plane_at;
    let tol = 0.01;
    let mut best: Option<(f64, String)> = None;
    for f in geo["faces"].as_array().into_iter().flatten() {
        if f["solid"].as_i64() != Some(solid) || f["surface"].as_str() != Some("Plane") {
            continue;
        }
        let (Some(c), Some(fnorm), Some(fb)) = (arr3(&f["center"]).or_else(|| bbox_center(&f["bbox"])), arr3(&f["normal"]), f["bbox"].as_array()) else { continue };
        if fnorm[0] * n[0] + fnorm[1] * n[1] + fnorm[2] * n[2] < 0.999 || (c[ax] - plane_at).abs() > tol {
            continue;
        }
        // 面要与特征的那一侧有重叠(其余两轴)
        let (lo, hi) = (arr3(&fb[0]).unwrap_or(c), arr3(&fb[1]).unwrap_or(c));
        let overlap = (0..3).filter(|&k| k != ax).all(|k| lo[k].max(bb[0][k]) <= hi[k].min(bb[1][k]) + tol);
        if !overlap {
            continue;
        }
        let d = ((c[0] - want[0]).powi(2) + (c[1] - want[1]).powi(2) + (c[2] - want[2]).powi(2)).sqrt();
        if best.as_ref().map_or(true, |b| d < b.0) {
            best = Some((d, f["id"].as_str().unwrap_or("").to_string()));
        }
    }
    best.map(|b| (b.1, body)).ok_or_else(|| format!("{r}: no {side} face of {feat} found in solid {solid}"))
}

fn bbox_center(b: &Value) -> Option<[f64; 3]> {
    let (lo, hi) = (arr3(&b[0])?, arr3(&b[1])?);
    Some([(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0])
}

/// 边引用: `edge(face(a.s1), face(b.s2))` = 两个面的交线上的边(端点都落在两个面的平面上); 也接受拓扑 id 字符串 / {near, of?} 原样传。
pub fn resolve_edges(r: &Value, _p: &Params, st: &State, geo: &Value) -> Result<(Vec<String>, String), String> {
    let s = r.as_str().ok_or("edge refs must be strings like edge(face(a.top), face(b.right))")?;
    let inner = s.trim().strip_prefix("edge(").and_then(|x| x.strip_suffix(')')).ok_or_else(|| format!("{s:?} is not edge(face(..), face(..))"))?;
    let split = inner.find("),").ok_or_else(|| format!("{s:?}: write edge(face(a.side), face(b.side))"))?;
    let (fa, fb) = (&inner[..split + 1], inner[split + 2..].trim());
    let (ida, body_a) = resolve_face(fa, st, geo)?;
    let (idb, body_b) = resolve_face(fb, st, geo)?;
    if body_a != body_b {
        return Err(format!("{s}: the two faces are on different bodies ({body_a}, {body_b})"));
    }
    let face = |id: &str| geo["faces"].as_array().into_iter().flatten().find(|f| f["id"].as_str() == Some(id)).cloned().unwrap_or_default();
    let (a, b) = (face(&ida), face(&idb));
    let plane = |f: &Value| -> Option<([f64; 3], [f64; 3])> { Some((arr3(&f["center"]).or_else(|| bbox_center(&f["bbox"]))?, arr3(&f["normal"])?)) };
    let ((ca, na), (cb, nb)) = (plane(&a).ok_or("face without plane data")?, plane(&b).ok_or("face without plane data")?);
    let on = |p: [f64; 3], c: [f64; 3], n: [f64; 3]| ((p[0] - c[0]) * n[0] + (p[1] - c[1]) * n[1] + (p[2] - c[2]) * n[2]).abs() < 0.01;
    let solid = *st.bodies.get(&body_a).unwrap_or(&-1);
    let ids: Vec<String> = geo["edges"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["solid"].as_i64() == Some(solid))
        .filter(|e| match (arr3(&e["start"]), arr3(&e["end"])) {
            (Some(p0), Some(p1)) => on(p0, ca, na) && on(p1, ca, na) && on(p0, cb, nb) && on(p1, cb, nb),
            _ => false,
        })
        .filter_map(|e| e["id"].as_str().map(String::from))
        .collect();
    if ids.is_empty() {
        return Err(format!("{s}: the two faces don't meet in an edge"));
    }
    Ok((ids, body_a))
}

// ---------------------------------------------------------------- 验收

/// 按最终几何检查: bbox(某个体的实体包围盒, 默认最后一个特征的体)/ count(某类面的个数)/ volume(min / max)。返回失败项(空 = 全过)。
pub fn check(checks: &[Value], p: &Params, st: &State, geo: &Value, mass: &dyn Fn(i64) -> Option<f64>, default_body: &str) -> Vec<String> {
    let mut fails = Vec::new();
    for c in checks {
        let body = c["of"].as_str().and_then(|f| st.features.get(f).map(|x| x.0.clone())).unwrap_or_else(|| default_body.to_string());
        let Some(solid) = st.bodies.get(&body).copied() else {
            fails.push(format!("{c}: body {body} has no solid"));
            continue;
        };
        if let Some(bb) = c.get("bbox") {
            let want = (|| -> Result<Bbox, String> { Ok([p.v3(&bb[0], "bbox[0]")?, p.v3(&bb[1], "bbox[1]")?]) })();
            let got = geo["solids"].as_array().into_iter().flatten().find(|s| s["id"].as_i64() == Some(solid)).and_then(|s| Some([arr3(&s["bbox"][0])?, arr3(&s["bbox"][1])?]));
            match (want, got) {
                (Ok(w), Some(g)) => {
                    let tol = c["tol"].as_f64().unwrap_or(0.05);
                    if (0..2).any(|i| (0..3).any(|k| (w[i][k] - g[i][k]).abs() > tol)) {
                        fails.push(format!("bbox of {body}: want {w:?}, got {g:?}"));
                    }
                }
                (Err(e), _) => fails.push(e),
                (_, None) => fails.push(format!("bbox of {body}: solid {solid} not found")),
            }
        }
        if let Some(m) = c["count"].as_object() {
            for (kind, n) in m {
                let got = geo["faces"].as_array().into_iter().flatten().filter(|f| f["solid"].as_i64() == Some(solid) && f["surface"].as_str() == Some(kind.as_str())).count();
                if Some(got as u64) != n.as_u64() {
                    fails.push(format!("{body}: {got} {kind} faces, want {n}"));
                }
            }
        }
        if let Some(v) = c.get("volume") {
            match mass(solid) {
                Some(vol) => {
                    let lo = v.get("min").map(|x| p.num(x)).transpose().unwrap_or(None).unwrap_or(f64::NEG_INFINITY);
                    let hi = v.get("max").map(|x| p.num(x)).transpose().unwrap_or(None).unwrap_or(f64::INFINITY);
                    if vol < lo || vol > hi {
                        fails.push(format!("{body}: volume {vol} not in [{lo}, {hi}]"));
                    }
                }
                None => fails.push(format!("{body}: volume unavailable")),
            }
        }
    }
    fails
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(v: Value) -> BTreeMap<String, Value> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn expressions_and_standard_refs() {
        let raw = params(json!({"w": 40, "t": 5, "inner": "w - 2*t", "hd": "M3.clearance", "tap": "M4.tap", "loop": "loop + 1"}));
        let p = Params::new(&raw);
        assert_eq!(p.expr("inner").unwrap(), 30.0);
        assert_eq!(p.expr("(w + t) / 3 * -1").unwrap(), -15.0);
        assert_eq!(p.expr("hd").unwrap(), 3.4);
        assert_eq!(p.expr("tap").unwrap(), 3.3);
        assert_eq!(p.expr("M6.clearance + 0").unwrap(), 6.6);
        assert!(p.expr("loop").unwrap_err().contains("refers to itself"));
        assert!(p.expr("nope * 2").unwrap_err().contains("unknown name"));
        assert!(p.expr("w +").is_err());
    }

    /// 假几何: L 形支架合并后(实体 7)的面 / 边, 照真机 geo.query 的形状
    fn bracket_geo() -> Value {
        let face = |id: &str, c: [f64; 3], n: [f64; 3], lo: [f64; 3], hi: [f64; 3]| json!({"id": id, "solid": 7, "surface": "Plane", "center": c, "normal": n, "bbox": [lo, hi]});
        json!({
            "solids": [{"id": 7, "bbox": [[0, 0, 0], [40, 20, 30]]}],
            "faces": [
                face("7f1", [22.5, 10.0, 5.0], [0.0, 0.0, 1.0], [5.0, 0.0, 5.0], [40.0, 20.0, 5.0]),     // 底板顶面(立板右边那段)
                face("7f2", [2.5, 10.0, 30.0], [0.0, 0.0, 1.0], [0.0, 0.0, 30.0], [5.0, 20.0, 30.0]),    // 立板顶面
                face("7f3", [5.0, 10.0, 17.5], [1.0, 0.0, 0.0], [5.0, 0.0, 5.0], [5.0, 20.0, 30.0]),     // 立板右侧(内侧)
                face("7f4", [40.0, 10.0, 2.5], [1.0, 0.0, 0.0], [40.0, 0.0, 0.0], [40.0, 20.0, 5.0]),    // 底板右端
                face("7f5", [20.0, 10.0, 0.0], [0.0, 0.0, -1.0], [0.0, 0.0, 0.0], [40.0, 20.0, 0.0]),
            ],
            "edges": [
                {"id": "7e1", "solid": 7, "start": [5, 0, 5], "end": [5, 20, 5]},     // 内角
                {"id": "7e2", "solid": 7, "start": [40, 0, 5], "end": [40, 20, 5]},
                {"id": "7e3", "solid": 7, "start": [5, 0, 30], "end": [5, 20, 30]},
            ]
        })
    }

    fn bracket_state() -> State {
        let mut st = State::default();
        st.features.insert("base".into(), ("base".into(), Some([[0.0, 0.0, 0.0], [40.0, 20.0, 5.0]])));
        st.features.insert("upright".into(), ("base".into(), Some([[0.0, 0.0, 0.0], [5.0, 20.0, 30.0]])));
        st.bodies.insert("base".into(), 7);
        st
    }

    #[test]
    fn compile_box_union_fillet_holes() {
        let raw = params(json!({"w": 40, "d": 20, "t": 5, "h": 30, "hd": "M3.clearance"}));
        let p = Params::new(&raw);
        let mut st = State::default();
        let geo0 = json!({});
        let c = compile(&json!({"id": "base", "type": "box", "size": ["w", "d", "t"], "at": [0, 0, 0]}), &p, &st, &geo0).unwrap();
        assert_eq!(c.actions, vec![Action::Exec { ops: vec![json!({"op": "box", "id": "base", "p1": [0.0, 0.0, 0.0], "p2": [40.0, 20.0, 0.0], "height": 5.0})], body: "base".into() }]);
        st.features.insert("base".into(), (c.body.clone(), c.bbox));
        st.bodies.insert("base".into(), 5);
        let c = compile(&json!({"id": "upright", "type": "box", "size": ["t", "d", "h"], "at": [0, 0, 0], "bool": "union", "into": "base"}), &p, &st, &geo0).unwrap();
        let Action::Exec { ops, body } = &c.actions[0] else { panic!() };
        assert_eq!((ops[0]["bool"].as_str(), ops[0]["targets"].clone(), body.as_str()), (Some("union"), json!([5]), "base"));
        // 合并后: 圆角挑内角, 孔落在底板顶面
        let st = bracket_state();
        let geo = bracket_geo();
        let c = compile(&json!({"id": "r", "type": "fillet", "edges": ["edge(face(base.top), face(upright.right))"], "r": 3}), &p, &st, &geo).unwrap();
        assert_eq!(c.actions, vec![Action::Exec { ops: vec![json!({"op": "fillet", "id": "r", "edges": ["7e1"], "distance": 3.0})], body: "base".into() }]);
        let c = compile(&json!({"id": "holes", "type": "hole", "on": "face(base.top)", "d": "hd", "pattern": {"start": [17, 10, 5], "grid": [2, 1], "pitch": [16, 0]}}), &p, &st, &geo).unwrap();
        assert_eq!(c.actions.len(), 2);
        let Action::Hole { face, args } = &c.actions[1] else { panic!() };
        assert_eq!((face.as_str(), args["d"].as_f64(), args["at"].clone()), ("face(base.top)", Some(3.4), json!([33.0, 10.0, 5.0])));
        assert_eq!(resolve_face("face(base.top)", &st, &geo).unwrap().0, "7f1", "底板顶面, 不是立板顶面");
        assert_eq!(resolve_face("face(upright.top)", &st, &geo).unwrap().0, "7f2");
        assert_eq!(resolve_face("face(base.right)", &st, &geo).unwrap().0, "7f4");
        // 错误如实报
        assert!(compile(&json!({"id": "x", "type": "box", "size": [1, 1, 1], "bool": "union", "into": "nope"}), &p, &st, &geo).unwrap_err().contains("not an earlier feature"));
        assert!(resolve_face("face(base.diagonal)", &st, &geo).is_err());
        assert!(compile(&json!({"id": "e", "type": "fillet", "edges": ["edge(face(base.top), face(base.bottom))"], "r": 1}), &p, &st, &geo).unwrap_err().contains("don't meet"));
    }

    #[test]
    fn revolve_part_and_checks() {
        let raw = params(json!({"L": 12}));
        let p = Params::new(&raw);
        let st = State::default();
        let c = compile(&json!({"id": "sh", "type": "revolve", "origin": [0, 0, 0], "axis": [0, 0, 1], "profile": [[0, 0], [5, 0], [5, "L"], [0, "L"]]}), &p, &st, &json!({})).unwrap();
        let Action::Exec { ops, .. } = &c.actions[0] else { panic!() };
        assert_eq!(ops[0]["profile"], json!([[0, 0], [5, 0], [5, 12.0], [0, 12.0]]));
        assert_eq!(c.bbox, Some([[-5.0, -5.0, 0.0], [5.0, 5.0, 12.0]]));
        let c = compile(&json!({"id": "s1", "type": "part", "kind": "screw-cap", "size": "M4", "length": "L", "at": [0, 0, 20], "axis": [0, 0, -1]}), &p, &st, &json!({})).unwrap();
        let Action::Exec { ops, body } = &c.actions[0] else { panic!() };
        assert_eq!((ops.len(), body.as_str()), (2, "s1"));
        // 验收
        let st = bracket_state();
        let geo = bracket_geo();
        let raw = params(json!({"w": 40, "d": 20, "h": 30}));
        let p = Params::new(&raw);
        let ok = check(&[json!({"bbox": [[0, 0, 0], ["w", "d", "h"]]}), json!({"count": {"Plane": 5}}), json!({"volume": {"min": 100, "max": 9000}})], &p, &st, &geo, &|_| Some(6540.0), "base");
        assert!(ok.is_empty(), "{ok:?}");
        let bad = check(&[json!({"bbox": [[0, 0, 0], ["w", "d", 31]]}), json!({"count": {"Cylinder": 1}})], &p, &st, &geo, &|_| None, "base");
        assert_eq!(bad.len(), 2, "{bad:?}");
    }
}
