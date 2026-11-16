//! D1: 出图面板的数据层(界面在 BOM 浮窗「出图」标签)。
//!
//! 一个零件一张图(`Sheet`), 按 BOM 的零件 key(几何指纹)存, 跨配置共享。存 `<文档目录>/.pla/drawings/<doc_key>/sheets.json`;
//! 同目录放这张图的全部产物: `<base>.step`(Plasticity 导出)、`<base>.spec.json`(出图方案, 交给 tools/drawing 编译器)、
//! `<base>.pdf / .svg / .png(预览)/ .dxf / .FCStd / .report.json`。
//! 修正 = 改 Sheet(主视方向 / 上下 / 剖视 / 标哪些孔位 / 孔径 → 螺纹 / 标题栏字段), 重新编译; 模型没变不用重新导出 STEP。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::state_store::StateStore;

pub const STORE: StateStore = StateStore { subdir: "drawings", legacy_subdir: None, prefix: "DW" };
const FILE: &str = "sheets.json";

fn yes() -> bool {
    true
}

/// 一张零件图的设置(= 出图方案里人能改的部分)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sheet {
    /// BOM 零件 key(几何指纹)。
    pub key: String,
    /// 文件名(料号, 没有就用零件名)。
    pub base: String,
    /// 导出 STEP 用的实体稳定 id(BOM 行里的第一个实例)。
    #[serde(default)]
    pub member: String,
    /// Plasticity 里的实体名(编译器按它从 dims.json 取孔记录)。
    #[serde(default)]
    pub model_name: String,
    /// 标题栏字段(pn / name / material / finish / project / designer / version / hardness …)。
    #[serde(default)]
    pub title: BTreeMap<String, String>,
    /// 主视方向(模型坐标, 从哪边看); None = 自动(沿最薄方向)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front: Option<[f64; 3]>,
    /// 主视里朝上的方向; None = 自动。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up: Option<[f64; 3]>,
    /// 自动剖视(A-A)。
    #[serde(default = "yes")]
    pub section: bool,
    /// 标孔位(坐标尺寸)的孔组; None = 主视全部孔组。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinate: Option<Vec<String>>,
    /// 孔径 → 螺纹(没有打孔记录的螺纹底孔), 如 "3.3" → "M4"。
    #[serde(default)]
    pub threads: BTreeMap<String, String>,
    /// 密度 g/cm³(重量 = 体积 × 密度); None = 不填重量。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<f64>,
    /// 导出时 Plasticity 报的体积 / 面积(零件在组里时 STEP 会含全部可见实体, 编译器按它挑)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<f64>,
    /// 上次导出 STEP / 出图的时间(本地时间文本)。
    #[serde(default)]
    pub exported: String,
    #[serde(default)]
    pub compiled: String,
    /// 装配图: 记下的配合(零件图出图时据此标本件公差)。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fits: Vec<Fit>,
    /// 单个尺寸的修正(按尺寸对象名: LenW / LenH / LenD / DiaOuter / 孔组 H1 …): 不标 / 关键尺寸 * / 改文字。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dims_hide: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dims_key: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dims_text: BTreeMap<String, String>,
}

impl Sheet {
    pub fn new(key: &str, base: &str, member: &str, model_name: &str) -> Self {
        Sheet {
            key: key.into(),
            base: base.into(),
            member: member.into(),
            model_name: model_name.into(),
            title: BTreeMap::new(),
            front: None,
            up: None,
            section: true,
            ordinate: None,
            threads: BTreeMap::new(),
            density: None,
            volume: None,
            area: None,
            exported: String::new(),
            compiled: String::new(),
            fits: Vec::new(),
            dims_hide: Vec::new(),
            dims_key: Vec::new(),
            dims_text: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Sheets {
    #[serde(default)]
    pub sheets: Vec<Sheet>,
}

impl Sheets {
    pub fn get(&self, key: &str) -> Option<&Sheet> {
        self.sheets.iter().find(|s| s.key == key)
    }

    pub fn upsert(&mut self, s: Sheet) {
        match self.sheets.iter_mut().find(|x| x.key == s.key) {
            Some(x) => *x = s,
            None => self.sheets.push(s),
        }
    }

    /// 文件名不和别的图重: 重了加 -2 / -3 …
    pub fn unique_base(&self, key: &str, want: &str) -> String {
        let taken = |b: &str| self.sheets.iter().any(|s| s.key != key && s.base.eq_ignore_ascii_case(b));
        if !taken(want) {
            return want.to_string();
        }
        (2..).map(|i| format!("{want}-{i}")).find(|b| !taken(b)).unwrap_or_else(|| want.to_string())
    }
}

pub fn dir(doc_key: &str) -> PathBuf {
    STORE.dir(doc_key)
}

pub fn load_in(dir: &Path) -> Sheets {
    std::fs::read_to_string(dir.join(FILE)).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
}

pub fn save_in(dir: &Path, s: &Sheets) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(FILE), serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// 文件名: 料号优先, 否则零件名; 去掉文件名不允许的字符。
pub fn base_name(pn: &str, name: &str) -> String {
    let src = if pn.trim().is_empty() { name.trim() } else { pn.trim() };
    let s: String = src.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    let s = s.trim().trim_end_matches('.').to_string();
    if s.is_empty() {
        "part".into()
    } else {
        s
    }
}

pub fn file(dir: &Path, s: &Sheet, ext: &str) -> PathBuf {
    dir.join(format!("{}.{ext}", s.base))
}

/// 出图方案 JSON(格式见 tools/drawing/compile_freecad.py 文件头)。
pub fn spec(s: &Sheet, dir: &Path, dims_file: Option<&Path>) -> Value {
    let mut part = json!({ "step": file(dir, s, "step").display().to_string(), "name": s.model_name });
    if let Some(v) = s.volume {
        part["volume"] = json!(v);
    }
    if let Some(a) = s.area {
        part["area"] = json!(a);
    }
    let mut title: serde_json::Map<String, Value> = s.title.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k.clone(), json!(v))).collect();
    if let Some(d) = s.density {
        title.insert("density".into(), json!(d));
    }
    let mut v = json!({
        "standard": "GB",
        "part": part,
        "title": title,
        "basename": s.base,
        "out": dir.display().to_string(),
        "section": if s.section { json!("auto") } else { Value::Null },
    });
    if let Some(f) = s.front {
        v["front"] = json!(f);
    }
    if let Some(u) = s.up {
        v["up"] = json!(u);
    }
    if let Some(g) = &s.ordinate {
        v["ordinate"] = if g.is_empty() { json!(false) } else { json!({ "groups": g }) };
    }
    if !s.threads.is_empty() {
        v["threads"] = json!(s.threads);
    }
    if let Some(p) = dims_file.filter(|p| p.exists()) {
        v["dims_file"] = json!(p.display().to_string());
    }
    let text: BTreeMap<&String, &String> = s.dims_text.iter().filter(|(_, t)| !t.trim().is_empty()).collect();
    if !s.dims_hide.is_empty() || !s.dims_key.is_empty() || !text.is_empty() {
        v["dims"] = json!({ "hide": s.dims_hide, "key": s.dims_key, "text": text });
    }
    v
}

/// 编译报告里放了的一个尺寸(页面毫米, 原点左下)。
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Placed {
    pub name: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub shown: Option<String>,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub at: [f64; 2],
    #[serde(default)]
    pub hidden: bool,
}

impl Placed {
    /// 列表里显示的文字: 实际标出的 → 原文字 → 数值。
    pub fn label(&self) -> String {
        self.shown.clone().or_else(|| self.text.clone()).or_else(|| self.value.map(fmt_d)).unwrap_or_default()
    }
}

/// 预览上点的位置(0..1, 原点左上)→ 最近的尺寸(页面上 25 mm 以内)。
pub fn nearest_dim<'a>(placed: &'a [Placed], paper: [f64; 2], fx: f64, fy: f64) -> Option<&'a Placed> {
    let (x, y) = (fx * paper[0], (1.0 - fy) * paper[1]);
    placed
        .iter()
        .map(|p| (((p.at[0] - x).powi(2) + (p.at[1] - y).powi(2)).sqrt(), p))
        .filter(|(d, _)| *d < 25.0)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, p)| p)
}

/// D3 装配图在图列表里的 key(不是 BOM 零件指纹)。
pub const ASM_KEY: &str = "@assembly";

/// 装配图的一行 BOM(明细表 + 实体指纹; ids = 全部实例的稳定 id, 导出整个装配用)。
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct AsmLine {
    pub key: String,
    pub pn: String,
    pub name: String,
    pub qty: u64,
    pub material: String,
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub density: Option<f64>,
    /// 外购组件(导入的 STEP / IGES 组)合成一行时: 组里各实体的指纹 [体积, 面积](编译器按它认实体)。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<[f64; 2]>,
    #[serde(skip)]
    pub ids: Vec<String>,
}

/// 明细表一行的文字(来自 BOM 表格: 料号 / 名称 / 材料 / 表处)。
#[derive(Debug, Clone, Default)]
pub struct RowInfo {
    pub pn: String,
    pub name: String,
    pub material: String,
    pub finish: String,
}

/// 组名像导入的文件(外购组件): .step / .stp / .igs / .iges / .x_t / .sat / .sldasm / .sldprt。
pub fn is_import_group(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    [".step", ".stp", ".igs", ".iges", ".x_t", ".sat", ".sldasm", ".sldprt"].iter().any(|e| n.ends_with(e))
}

/// 装配图 key: ASM_KEY = 整个范围; ASM_KEY + ":" + 组名 = 某个顶层组(方案 / 子装配)。
pub fn asm_key(group: Option<&str>) -> String {
    match group {
        Some(g) => format!("{ASM_KEY}:{g}"),
        None => ASM_KEY.to_string(),
    }
}

pub fn is_asm(key: &str) -> bool {
    key == ASM_KEY || key.starts_with(&format!("{ASM_KEY}:"))
}

/// bom.scan 的 lines → 装配图明细。group = 只要顶层组是它的实例(None = 全部)。
/// 路径里第一个"导入文件"组及其下面的全部实体合成一行外购件(名称 = 文件名去扩展名, 数量 = 组里各零件实例数的最小值)。
pub fn asm_lines(lines: &Value, info: &dyn Fn(&str) -> RowInfo, group: Option<&str>) -> Vec<AsmLine> {
    let mut out: Vec<AsmLine> = Vec::new();
    // 外购组: 前缀路径 → (行下标, 每个 BOM 行在组里的实例数)
    let mut subs: Vec<(String, usize, BTreeMap<String, u64>)> = Vec::new();
    for l in lines.as_array().into_iter().flatten() {
        let key = l["key"].as_str().unwrap_or("").to_string();
        let fp = [l["volume"].as_f64().unwrap_or(0.0), l["area"].as_f64().unwrap_or(0.0)];
        let mut own: Option<usize> = None;
        for m in l["members"].as_array().into_iter().flatten() {
            let path: Vec<String> = m["path"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(str::to_string)).collect();
            if let Some(g) = group {
                if path.first().map(String::as_str) != Some(g) {
                    continue;
                }
            }
            let id = m["id"].as_str().unwrap_or("").to_string();
            match path.iter().position(|p| is_import_group(p)) {
                Some(i) => {
                    let prefix = path[..=i].join("/");
                    let si = match subs.iter().position(|s| s.0 == prefix) {
                        Some(si) => si,
                        None => {
                            let stem = path[i].rsplit_once('.').map(|(a, _)| a).unwrap_or(&path[i]).trim().to_string();
                            out.push(AsmLine { key: format!("@sub:{prefix}"), name: stem, note: "外购".into(), ..Default::default() });
                            subs.push((prefix, out.len() - 1, BTreeMap::new()));
                            subs.len() - 1
                        }
                    };
                    let li = subs[si].1;
                    *subs[si].2.entry(key.clone()).or_default() += 1;
                    if !out[li].parts.iter().any(|p| p == &fp) {
                        out[li].parts.push(fp);
                    }
                    out[li].ids.push(id);
                }
                None => {
                    let li = *own.get_or_insert_with(|| {
                        let r = info(&key);
                        out.push(AsmLine {
                            key: key.clone(),
                            pn: r.pn,
                            name: if r.name.is_empty() { m["name"].as_str().unwrap_or("").to_string() } else { r.name },
                            density: guess_density(&r.material),
                            material: r.material,
                            note: r.finish,
                            volume: l["volume"].as_f64(),
                            area: l["area"].as_f64(),
                            ..Default::default()
                        });
                        out.len() - 1
                    });
                    out[li].qty += 1;
                    out[li].ids.push(id);
                }
            }
        }
    }
    for (_, li, counts) in subs {
        out[li].qty = counts.values().copied().min().unwrap_or(1).max(1);
    }
    out
}

/// 顶层组(方案 / 子装配)里够出装配图的: 合成外购件后至少 2 行; 外购组本身(顶层就是导入文件)不算。
pub fn asm_groups(lines: &Value) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for l in lines.as_array().into_iter().flatten() {
        for m in l["members"].as_array().into_iter().flatten() {
            if let Some(g) = m["path"].as_array().and_then(|p| p.first()).and_then(|g| g.as_str()) {
                if !is_import_group(g) && !names.iter().any(|n| n == g) {
                    names.push(g.to_string());
                }
            }
        }
    }
    names.retain(|g| asm_lines(lines, &|_| RowInfo::default(), Some(g)).len() >= 2);
    names
}

/// 装配图方案 JSON(格式见 tools/drawing/compile_assembly.py 文件头)。
pub fn asm_spec(s: &Sheet, dir: &Path, lines: &[AsmLine]) -> Value {
    let title: serde_json::Map<String, Value> = s.title.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k.clone(), json!(v))).collect();
    let mut v = json!({
        "standard": "GB",
        "assembly": { "step": file(dir, s, "step").display().to_string(), "lines": lines },
        "title": title,
        "basename": s.base,
        "out": dir.display().to_string(),
    });
    if let Some(f) = s.front {
        v["front"] = json!(f);
    }
    if let Some(u) = s.up {
        v["up"] = json!(u);
    }
    let fits: Vec<Value> = s.fits.iter().filter(|f| !f.code.trim().is_empty()).map(|f| json!({ "id": f.id, "code": f.code })).collect();
    if !fits.is_empty() {
        v["fits"] = json!(fits);
    }
    v
}

/// 按材料名猜密度(g/cm³); 认不出 → None(重量留空, 用户可填)。
pub fn guess_density(material: &str) -> Option<f64> {
    let m = material.to_lowercase();
    let has = |ks: &[&str]| ks.iter().any(|k| m.contains(k));
    Some(if has(&["7075"]) {
        2.81
    } else if has(&["6061", "6063", "5052", "2a12", "2024", "铝", "alu"]) || m.starts_with("al") {
        2.7
    } else if has(&["sus", "不锈钢", "stainless", "304", "316"]) {
        7.93
    } else if has(&["黄铜", "brass", "h59", "h62", "hpb"]) {
        8.5
    } else if has(&["紫铜", "copper", "t2", "铍铜"]) {
        8.9
    } else if has(&["钛", "tc4", "titan"]) {
        4.5
    } else if has(&["钢", "steel", "45#", "q235", "skd", "cr12", "40cr", "65mn", "spcc"]) {
        7.85
    } else if has(&["peek"]) {
        1.3
    } else if has(&["pom", "赛钢", "delrin"]) {
        1.41
    } else if has(&["pa66", "pa6", "nylon", "尼龙"]) {
        1.14
    } else if has(&["pmma", "亚克力", "acrylic"]) {
        1.18
    } else if has(&["abs"]) {
        1.05
    } else if has(&["ptfe", "铁氟龙", "teflon"]) {
        2.2
    } else if has(&["pc"]) {
        1.2
    } else if has(&["pp"]) {
        0.9
    } else {
        return None;
    })
}

// ---------------- 编译报告(编译器写的 <base>.report.json, 只取面板用的) ----------------

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct HoleGroup {
    pub id: String,
    #[serde(default)]
    pub view: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub count: usize,
    #[serde(default)]
    pub ordinate: bool,
}

/// 装配图编译器找到的配合候选(id 只当不透明字符串用)。
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct FitCand {
    pub id: String,
    pub d: f64,
    #[serde(default)]
    pub hole_key: String,
    #[serde(default)]
    pub shaft_key: String,
    #[serde(default)]
    pub hole: String,
    #[serde(default)]
    pub shaft: String,
    #[serde(default)]
    pub count: usize,
}

/// 记下的配合(装配图那张图上): 孔 / 轴零件 + 公称直径 + 配合代号 "H7/g6"。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Fit {
    pub id: String,
    pub d: f64,
    pub hole_key: String,
    pub shaft_key: String,
    pub code: String,
}

/// 常用配合(基孔制为主): 间隙 / 过渡 / 过盈。
pub const FIT_CODES: &[(&str, &str, &str)] = &[
    ("H7/g6", "间隙(精密滑动)", "clearance (sliding)"),
    ("H7/h6", "间隙(定位)", "clearance (locating)"),
    ("H8/f7", "间隙(转动)", "clearance (running)"),
    ("H7/js6", "过渡", "transition"),
    ("H7/k6", "过渡(定位)", "transition (locating)"),
    ("H7/n6", "过渡(紧)", "transition (tight)"),
    ("H7/p6", "过盈(压入)", "interference (press)"),
    ("H7/s6", "过盈(重压)", "interference (heavy press)"),
];

/// 直径显示: 两位小数, 去尾零(8 / 6.6 / 12.25)。
pub fn fmt_d(d: f64) -> String {
    let s = format!("{d:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// 某个零件在记下的配合里的角色 → 零件图方案的 fits: [{d, role, code}]("H7/g6" 孔取 H7、轴取 g6)。
pub fn part_fits(fits: &[Fit], key: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for f in fits {
        let (h, s) = f.code.split_once('/').map(|(a, b)| (a.trim(), b.trim())).unwrap_or(("", ""));
        if f.hole_key == key && !h.is_empty() {
            out.push(json!({ "d": f.d, "role": "hole", "code": h }));
        }
        if f.shaft_key == key && !s.is_empty() {
            out.push(json!({ "d": f.d, "role": "shaft", "code": s }));
        }
    }
    out
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq)]
pub struct Axes {
    pub front: [f64; 3],
    pub up: [f64; 3],
    pub right: [f64; 3],
}

impl Axes {
    fn neg(v: [f64; 3]) -> [f64; 3] {
        [-v[0] + 0.0, -v[1] + 0.0, -v[2] + 0.0]
    }
    /// 上下颠倒。
    pub fn flipped(&self) -> ([f64; 3], [f64; 3]) {
        (self.front, Self::neg(self.up))
    }
    /// 图面转 90°(原来朝右的变成朝上)。
    pub fn rotated(&self) -> ([f64; 3], [f64; 3]) {
        (self.front, self.right)
    }
    /// 从背面看(上不变)。
    pub fn back(&self) -> ([f64; 3], [f64; 3]) {
        (Self::neg(self.front), self.up)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Report {
    #[serde(default)]
    pub hole_groups: Vec<HoleGroup>,
    /// 装配图: 配合候选(不同零件之间同轴同径的孔 / 轴)。
    #[serde(default)]
    pub fit_candidates: Vec<FitCand>,
    /// 放了的尺寸(含被隐藏的)+ 纸张大小 mm: 预览上点选尺寸用。
    #[serde(default)]
    pub placed: Vec<Placed>,
    #[serde(default)]
    pub paper_mm: Option<[f64; 2]>,
    #[serde(default)]
    pub axes: Option<Axes>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub round_front: bool,
}

pub fn load_report(dir: &Path, s: &Sheet) -> Option<Report> {
    let p = dir.join(format!("{}.report.json", s.base));
    std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok())
}

// ---------------- 编译器位置与运行 ----------------

/// tools/drawing(含 compile_freecad.py / draw.ps1): 环境变量 PLA_DRAWING_DIR, 否则从 exe 目录往上找 tools/drawing 或 drawing。
pub fn tools_dir() -> Option<PathBuf> {
    let ok = |d: &Path| d.join("compile_freecad.py").exists() && d.join("draw.ps1").exists();
    if let Ok(p) = std::env::var("PLA_DRAWING_DIR") {
        let p = PathBuf::from(p);
        if ok(&p) {
            return Some(p);
        }
    }
    let mut d = std::env::current_exe().ok()?.parent()?.to_path_buf();
    for _ in 0..7 {
        for c in [d.join("tools").join("drawing"), d.join("drawing")] {
            if ok(&c) {
                return Some(c);
            }
        }
        d = d.parent()?.to_path_buf();
    }
    None
}

/// FreeCAD(出图后端)。draw.ps1 的默认位置; 找不到返回 None。
pub fn freecad_bin() -> Option<PathBuf> {
    let base = std::env::var("LOCALAPPDATA").ok()?;
    let p = PathBuf::from(base).join("Programs").join("FreeCAD 1.1").join("bin");
    p.join("freecad.exe").exists().then_some(p)
}

/// 编译一张图(阻塞, 15–90 s; 在后台线程调用): draw.ps1 -Spec <spec> -Png → PDF / SVG / PNG / 报告。
pub fn compile(tools: &Path, spec_file: &Path) -> Result<String, String> {
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(tools.join("draw.ps1")).arg("-Spec").arg(spec_file).arg("-Png");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| format!("powershell: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() {
        Ok(text.lines().last().unwrap_or("").to_string())
    } else {
        let err = spec_file.parent().and_then(|d| std::fs::read_to_string(d.join("compile_error.txt")).ok()).unwrap_or_default();
        let tail: Vec<&str> = err.lines().rev().take(3).collect();
        Err(format!("{text}\n{}", tail.into_iter().rev().collect::<Vec<_>>().join("\n")).trim().to_string())
    }
}

/// 多张图合成一份 PDF(tools/drawing/book.ps1: 每张 SVG 一页, 纸张按各自报告)。同步执行, 放后台线程里调。
pub fn book(tools: &Path, out: &Path, svgs: &[PathBuf]) -> Result<String, String> {
    // 路径清单写文件(文件名里可能有逗号, 不能走 -Svg a,b)
    let list = out.with_extension("list.txt");
    std::fs::write(&list, svgs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n")).map_err(|e| format!("{}: {e}", list.display()))?;
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(tools.join("book.ps1")).arg("-Out").arg(out).arg("-List").arg(&list);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let o = cmd.output().map_err(|e| format!("powershell: {e}"))?;
    let _ = std::fs::remove_file(&list);
    let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
    if o.status.success() && out.exists() {
        Ok(text.lines().last().unwrap_or("").to_string())
    } else {
        Err(text)
    }
}

/// 合订本的文件名: <文档名>.drawings.pdf, 放在出图目录里。
pub fn book_file(dir: &Path, doc_key: &str) -> PathBuf {
    let stem = doc_key.rsplit(['/', '\\']).next().unwrap_or("drawings");
    dir.join(format!("{}.drawings.pdf", base_name("", stem)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asm_lines_groups_and_purchased() {
        let lines = json!([
            {"key": "plate", "volume": 100.0, "area": 50.0, "members": [
                {"id": "p1", "name": "Plate", "path": ["Solution H"]},
                {"id": "p2", "name": "Plate", "path": ["SOLUTION V"]}]},
            {"key": "screw", "volume": 1.0, "area": 2.0, "members": [
                {"id": "s1", "name": "M3", "path": []}]},
            // 电机: 每个方案一台, 导入组里两个零件
            {"key": "m-body", "volume": 500.0, "area": 300.0, "members": [
                {"id": "b1", "name": null, "path": ["SOLUTION V", "motor.STEP"]},
                {"id": "b2", "name": null, "path": ["Solution H", "motor.STEP"]}]},
            {"key": "m-pin", "volume": 0.1, "area": 0.5, "members": [
                {"id": "q1", "name": null, "path": ["SOLUTION V", "motor.STEP", "sub"]},
                {"id": "q2", "name": null, "path": ["SOLUTION V", "motor.STEP", "sub"]},
                {"id": "q3", "name": null, "path": ["Solution H", "motor.STEP", "sub"]},
                {"id": "q4", "name": null, "path": ["Solution H", "motor.STEP", "sub"]}]},
            {"key": "clamp", "volume": 9.0, "area": 9.0, "members": [
                {"id": "c1", "name": "jaw", "path": ["clamp.stp", "jaw"]}]}
        ]);
        let info = |k: &str| RowInfo { pn: if k == "plate" { "9001".into() } else { String::new() }, material: "6061".into(), ..Default::default() };
        let h = asm_lines(&lines, &info, Some("Solution H"));
        assert_eq!(h.len(), 2, "plate + one purchased motor line");
        assert_eq!((h[0].pn.as_str(), h[0].qty, h[0].density), ("9001", 1, Some(2.7)));
        let m = &h[1];
        assert_eq!((m.key.as_str(), m.name.as_str(), m.qty, m.note.as_str()), ("@sub:Solution H/motor.STEP", "motor", 1, "外购"));
        assert_eq!(m.parts, vec![[500.0, 300.0], [0.1, 0.5]]);
        assert_eq!(m.ids, vec!["b2", "q3", "q4"]);
        let all = asm_lines(&lines, &info, None);
        assert_eq!(all.iter().map(|l| l.key.as_str()).collect::<Vec<_>>(), vec!["plate", "screw", "@sub:SOLUTION V/motor.STEP", "@sub:Solution H/motor.STEP", "@sub:clamp.stp"]);
        assert_eq!(all[0].qty, 2);
        assert_eq!(asm_groups(&lines), vec!["Solution H", "SOLUTION V"], "clamp.stp is a purchased part, not an assembly");
        assert!(is_asm(&asm_key(Some("Solution H"))) && is_asm(ASM_KEY) && !is_asm("@sub:x") && !is_asm("plate"));
        let v = serde_json::to_value(m).unwrap();
        assert_eq!(v["parts"], json!([[500.0, 300.0], [0.1, 0.5]]));
        assert!(v.get("volume").is_none());
    }

    #[test]
    fn dim_edits_and_pick() {
        let mut s = Sheet::new("k", "p", "id", "E");
        s.dims_hide.push("LenD".into());
        s.dims_text.insert("H2".into(), "M6 通孔".into());
        s.dims_text.insert("H3".into(), " ".into());
        let v = spec(&s, Path::new("x"), None);
        assert_eq!(v["dims"]["hide"], json!(["LenD"]));
        assert_eq!(v["dims"]["text"], json!({"H2": "M6 通孔"}), "blank overrides are dropped");
        assert!(spec(&Sheet::new("k", "p", "id", "E"), Path::new("x"), None).get("dims").is_none());
        let r: Report = serde_json::from_str(
            r#"{"placed":[{"name":"LenW","type":"DistanceX","view":"V","text":null,"at":[100,250],"value":80.0},
                          {"name":"H1","text":"2 x ⌀6.6","at":[60,190],"shown":"*2 x ⌀6.6"}],"paper_mm":[420,297]}"#,
        )
        .unwrap();
        assert_eq!(r.placed[0].label(), "80");
        assert_eq!(r.placed[1].label(), "*2 x ⌀6.6");
        // 预览左上为原点: (60/420, 1 − 190/297)
        assert_eq!(nearest_dim(&r.placed, [420.0, 297.0], 60.0 / 420.0, 1.0 - 192.0 / 297.0).map(|p| p.name.as_str()), Some("H1"));
        assert!(nearest_dim(&r.placed, [420.0, 297.0], 0.9, 0.9).is_none());
    }

    #[test]
    fn fits_to_parts() {
        let f = Fit { id: "a|b|8".into(), d: 8.0, hole_key: "plate".into(), shaft_key: "pin".into(), code: "H7/g6".into() };
        assert_eq!(part_fits(&[f.clone()], "plate"), vec![json!({"d": 8.0, "role": "hole", "code": "H7"})]);
        assert_eq!(part_fits(&[f.clone()], "pin"), vec![json!({"d": 8.0, "role": "shaft", "code": "g6"})]);
        assert!(part_fits(&[f.clone()], "other").is_empty());
        let mut s = Sheet::new(ASM_KEY, "asm", "", "");
        s.fits = vec![f, Fit { code: " ".into(), ..Default::default() }];
        let v = asm_spec(&s, Path::new("x"), &[]);
        assert_eq!(v["fits"], json!([{"id": "a|b|8", "code": "H7/g6"}]), "blank codes are not sent");
        let back: Sheets = serde_json::from_str(&serde_json::to_string(&Sheets { sheets: vec![s.clone()] }).unwrap()).unwrap();
        assert_eq!(back.sheets[0], s);
    }

    #[test]
    fn asm_spec_lines() {
        let dir = std::env::temp_dir().join("pla-drawing-test");
        let mut s = Sheet::new(ASM_KEY, "model1-asm", "", "");
        s.title.insert("name".into(), "总装".into());
        s.title.insert("pn".into(), "  ".into());
        let l = AsmLine { key: "k1".into(), name: "Box".into(), qty: 5, volume: Some(447040.0), area: Some(35087.0), density: Some(2.7), ids: vec!["a".into()], ..Default::default() };
        let v = asm_spec(&s, &dir, &[l]);
        assert!(v["assembly"]["step"].as_str().unwrap().ends_with("model1-asm.step"));
        assert_eq!(v["assembly"]["lines"][0]["qty"], 5);
        assert_eq!(v["assembly"]["lines"][0]["volume"], 447040.0);
        assert!(v["assembly"]["lines"][0].get("ids").is_none(), "ids are only for the export");
        assert_eq!(v["title"]["name"], "总装");
        assert!(v["title"].get("pn").is_none());
        assert!(v.get("part").is_none() && v.get("front").is_none());
    }

    #[test]
    fn spec_from_sheet() {
        let dir = std::env::temp_dir().join("pla-drawing-test");
        let mut s = Sheet::new("k1", &base_name("904002001", "big"), "id7", "Extruded.001");
        s.title.insert("pn".into(), "904002001".into());
        s.title.insert("finish".into(), " ".into());
        s.density = Some(2.7);
        s.volume = Some(1000.0);
        s.up = Some([0.0, 0.0, -1.0]);
        s.ordinate = Some(vec!["H4".into(), "H5".into()]);
        s.threads.insert("3.3".into(), "M4".into());
        let v = spec(&s, &dir, None);
        assert_eq!(v["basename"], "904002001");
        assert!(v["part"]["step"].as_str().unwrap().ends_with("904002001.step"));
        assert_eq!(v["part"]["name"], "Extruded.001");
        assert_eq!(v["part"]["volume"], 1000.0);
        assert_eq!(v["title"]["density"], 2.7);
        assert!(v["title"].get("finish").is_none(), "blank title fields are left to the template");
        assert_eq!(v["section"], "auto");
        assert_eq!(v["up"], json!([0.0, 0.0, -1.0]));
        assert!(v.get("front").is_none());
        assert_eq!(v["ordinate"]["groups"], json!(["H4", "H5"]));
        assert_eq!(v["threads"]["3.3"], "M4");
        s.ordinate = Some(vec![]);
        s.section = false;
        let v = spec(&s, &dir, None);
        assert_eq!(v["ordinate"], false);
        assert!(v["section"].is_null());
    }

    #[test]
    fn sheets_store_and_names() {
        let dir = std::env::temp_dir().join(format!("pla-drawing-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut all = Sheets::default();
        all.upsert(Sheet::new("a", "P1", "1", "A"));
        assert_eq!(all.unique_base("b", "p1"), "p1-2");
        assert_eq!(all.unique_base("a", "P1"), "P1");
        let mut s = all.get("a").unwrap().clone();
        s.section = false;
        all.upsert(s);
        assert_eq!(all.sheets.len(), 1);
        save_in(&dir, &all).unwrap();
        assert_eq!(load_in(&dir), all);
        // 老文件缺字段: section 默认 true
        std::fs::write(dir.join(FILE), r#"{"sheets":[{"key":"x","base":"x"}]}"#).unwrap();
        assert!(load_in(&dir).sheets[0].section);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(base_name("", "a/b:c."), "a_b_c");
        assert_eq!(base_name(" ", " "), "part");
    }

    #[test]
    fn density_and_axes() {
        assert_eq!(guess_density("6061-T6"), Some(2.7));
        assert_eq!(guess_density("SUS304"), Some(7.93));
        assert_eq!(guess_density("POM 黑色"), Some(1.41));
        assert_eq!(guess_density("45#钢"), Some(7.85));
        assert_eq!(guess_density("未知"), None);
        let a = Axes { front: [0.0, -1.0, 0.0], up: [0.0, 0.0, 1.0], right: [1.0, 0.0, 0.0] };
        assert_eq!(a.flipped(), ([0.0, -1.0, 0.0], [0.0, 0.0, -1.0]));
        assert_eq!(a.rotated(), ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0]));
        assert_eq!(a.back(), ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]));
        let r: Report = serde_json::from_str(r#"{"hole_groups":[{"id":"H1","view":"Front","text":"4 x M4 贯穿","count":4,"ordinate":true}],
            "axes":{"front":[0,-1,0],"up":[0,0,1],"right":[1,0,0]},"warnings":["w"],"steps":{}}"#).unwrap();
        assert_eq!(r.hole_groups[0].count, 4);
        assert_eq!(r.axes.unwrap().up, [0.0, 0.0, 1.0]);
    }
}
