//! B3: 按模板生成的 BOM 表(用户 2026-11-15: 参照实际项目 OYM testbed BOM, 其中一些列是计算生成的)。
//!
//! - **模板** = `<exe 目录>/bom-templates/<名字>.json`, 列定义: 模型数据 / 可编辑字段(带默认值)/ 公式(引用其它列的 `{列id}`,
//!   用笔记表格的公式引擎求值, 支持 ROUNDUP / CEILING / IF …)/ 是否在合计行求和。空目录写入内置模板(= OYM 的列)。
//! - **编辑** = 每个文档一份 `<文档目录>/.pla/bom/<doc_key>/edits.json`: 按零件身份(`bom.scan` 的 key, 几何指纹)存字段覆盖,
//!   跨配置共享(同一个零件在不同配置里的料号 / 单价是同一份); 排除标记; 手动行(模型里没有的采购件 / 工具); 字段默认值覆盖(如本项目套数)。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::formula;

/// 模板里的一列。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Col {
    pub id: String,
    pub header: String,
    /// 模型数据: name / qty / material / volume / group / remark; "field" = 可编辑字段; 省略且有 formula = 公式列。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// 公式, 如 `ROUNDUP({total_qty}*{redundancy}/{pcs_unit},0)`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    /// 可编辑字段的默认值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// 合计行里对这一列求和。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sum: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Template {
    #[serde(default)]
    pub columns: Vec<Col>,
}

pub const BUILTIN_NAME: &str = "OYM";

fn col(id: &str, header: &str, source: Option<&str>, formula: Option<&str>, default: Option<&str>, sum: bool) -> Col {
    Col { id: id.into(), header: header.into(), source: source.map(String::from), formula: formula.map(String::from), default: default.map(String::from), sum }
}

/// 内置模板: OYM testbed BOM 的列与公式(总数量 = 一套用量 × 套数; unit quantity = ⌈总数量 × 冗余系数 ÷ pcs/unit⌉; total cost = unit quantity × unit price)。
pub fn builtin() -> Template {
    Template {
        columns: vec![
            col("name", "名称", Some("name"), None, None, false),
            col("pn", "料号", Some("field"), None, None, false),
            col("flag", "Column 16", Some("field"), None, None, false),
            col("model", "型号", Some("field"), None, None, false),
            col("usage", "用途", Some("field"), None, None, false),
            col("material", "材料", Some("material"), None, None, false),
            col("origin", "来源", Some("field"), None, Some("代工"), false),
            col("per_set", "一套用量", Some("qty"), None, None, false),
            col("sets", "套数", Some("field"), None, Some("1"), false),
            col("total_qty", "总数量", None, Some("{per_set}*{sets}"), None, false),
            col("redundancy", "冗余系数", Some("field"), None, Some("1.2"), false),
            col("pcs_unit", "pcs/unit", Some("field"), None, Some("1"), false),
            col("unit", "unit", Some("field"), None, Some("pc"), false),
            col("unit_qty", "unit quantity", None, Some("ROUNDUP({total_qty}*{redundancy}/{pcs_unit},0)"), None, false),
            col("unit_price", "unit price", Some("field"), None, Some("0"), false),
            col("total_cost", "total cost", None, Some("{unit_qty}*{unit_price}"), None, true),
        ],
    }
}

// ---------------- 模板文件(同孔规格方案: 文件名 = 模板名) ----------------

pub fn dir() -> PathBuf {
    crate::prefs::data_dir().join("bom-templates")
}

pub fn path_in(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.json"))
}

pub fn list_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let p = e.path();
                    (p.extension().and_then(|x| x.to_str()) == Some("json")).then(|| p.file_stem().and_then(|s| s.to_str()).map(String::from)).flatten()
                })
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() && write_in(dir, BUILTIN_NAME, &builtin()).is_ok() {
        names.push(BUILTIN_NAME.into());
    }
    names.sort();
    names
}

pub fn load_in(dir: &Path, name: &str) -> Result<Template, String> {
    let txt = std::fs::read_to_string(path_in(dir, name)).map_err(|e| e.to_string())?;
    let t: Template = serde_json::from_str(txt.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    if t.columns.is_empty() {
        return Err("no columns".into());
    }
    Ok(t)
}

pub fn write_in(dir: &Path, name: &str, t: &Template) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(path_in(dir, name), serde_json::to_vec_pretty(t).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

pub fn mtime_in(dir: &Path, name: &str) -> Option<std::time::SystemTime> {
    std::fs::metadata(path_in(dir, name)).and_then(|m| m.modified()).ok()
}

// ---------------- 每个文档的编辑 ----------------

/// 一条手动行(模型里没有的件)。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ManualRow {
    pub id: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Edits {
    /// 零件 key → 列 id → 覆盖值。
    #[serde(default)]
    pub parts: BTreeMap<String, BTreeMap<String, String>>,
    /// 排除的零件 key(不进表、不进合计)。
    #[serde(default)]
    pub excluded: Vec<String>,
    #[serde(default)]
    pub manual: Vec<ManualRow>,
    /// 本文档的字段默认值覆盖(列 id → 值, 如 sets → 5)。
    #[serde(default)]
    pub defaults: BTreeMap<String, String>,
    /// 本文档用的模板名。
    #[serde(default)]
    pub template: String,
    #[serde(default)]
    pub next_manual: u64,
}

fn edits_path(doc_key: &str) -> PathBuf {
    crate::bom::STORE.dir(doc_key).join("edits.json")
}

pub fn load_edits(doc_key: &str) -> Edits {
    std::fs::read_to_string(edits_path(doc_key)).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
}

pub fn save_edits(doc_key: &str, e: &Edits) -> Result<(), String> {
    let p = edits_path(doc_key);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, serde_json::to_vec_pretty(e).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

impl Edits {
    /// 新手动行, 返回它的 id。
    pub fn add_manual(&mut self) -> String {
        self.next_manual += 1;
        let id = format!("m{}", self.next_manual);
        self.manual.push(ManualRow { id: id.clone(), fields: BTreeMap::new() });
        id
    }

    /// 设一个单元格(零件行 = key; 手动行 = "manual:<id>")。值等于"未覆盖时的值"就去掉覆盖。
    pub fn set_cell(&mut self, row_key: &str, col: &str, value: &str, base: &str) {
        if let Some(mid) = row_key.strip_prefix("manual:") {
            if let Some(r) = self.manual.iter_mut().find(|r| r.id == mid) {
                r.fields.insert(col.into(), value.into());
            }
            return;
        }
        let m = self.parts.entry(row_key.into()).or_default();
        if value == base {
            m.remove(col);
        } else {
            m.insert(col.into(), value.into());
        }
        if m.is_empty() {
            self.parts.remove(row_key);
        }
    }
}

// ---------------- 生成表 ----------------

/// 一行(已求值)。
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// 零件 key, 或 "manual:<id>"。
    pub key: String,
    pub cells: Vec<String>,
    /// 每格是否是人工覆盖(界面高亮)。
    pub overridden: Vec<bool>,
    pub manual: bool,
}

/// 格式化数字: 去掉多余的 0(12.0 → 12, 42.70 → 42.7)。
fn fmt_num(x: f64) -> String {
    let s = format!("{:.6}", x);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// 公式求值: `{列id}` 换成该行已有的值(数字原样, 文本加引号), 交给笔记表格的公式引擎。
fn eval_formula(src: &str, vals: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = src;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else { break };
        let id = &rest[i + 1..i + j];
        let v = vals.get(id).cloned().unwrap_or_default();
        if v.trim().parse::<f64>().is_ok() {
            out.push_str(v.trim());
        } else if v.trim().is_empty() {
            out.push('0');
        } else {
            out.push('"');
            out.push_str(&v.replace('"', "\"\""));
            out.push('"');
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    match formula::eval_expr(&format!("={out}")) {
        formula::Value::Num(n) => fmt_num(n),
        formula::Value::Text(t) => t,
        formula::Value::Bool(b) => if b { "TRUE".into() } else { "FALSE".into() },
        other => format!("{other:?}"),
    }
}

/// 模型数据列的值。
fn model_value(src: &str, line: &Value, remark: &str) -> String {
    match src {
        "name" => line["name"].as_str().unwrap_or("").into(),
        "qty" => line["qty"].as_u64().unwrap_or(0).to_string(),
        "material" => line["material"].as_str().unwrap_or("").into(),
        "volume" => line["volume"].as_f64().map(fmt_num).unwrap_or_default(),
        "group" => line["path"].as_str().unwrap_or("").into(),
        "remark" => remark.into(),
        _ => String::new(),
    }
}

/// `bom.scan` 结果 + 模板 + 编辑 → 表行(零件行在前, 手动行在后) + 合计行(sum 列求和, 其余空)。
pub fn build(scan: &Value, t: &Template, e: &Edits) -> (Vec<Row>, Vec<String>) {
    let mut rows = Vec::new();
    let field_default = |c: &Col| e.defaults.get(&c.id).cloned().or_else(|| c.default.clone()).unwrap_or_default();
    let fill = |base: &dyn Fn(&Col) -> String, over: Option<&BTreeMap<String, String>>, key: String, manual: bool| -> Row {
        let mut vals: BTreeMap<String, String> = BTreeMap::new();
        let mut overridden = vec![false; t.columns.len()];
        // 两遍: 公式可以引用后面的列
        for _ in 0..2 {
            for (i, c) in t.columns.iter().enumerate() {
                let v = if let Some(f) = &c.formula {
                    eval_formula(f, &vals)
                } else {
                    match over.and_then(|m| m.get(&c.id)) {
                        Some(o) => {
                            overridden[i] = !manual;
                            o.clone()
                        }
                        None => base(c),
                    }
                };
                vals.insert(c.id.clone(), v);
            }
        }
        Row { key, cells: t.columns.iter().map(|c| vals.get(&c.id).cloned().unwrap_or_default()).collect(), overridden, manual }
    };
    for line in scan["lines"].as_array().into_iter().flatten() {
        let key = line["key"].as_str().unwrap_or("").to_string();
        if e.excluded.contains(&key) {
            continue;
        }
        let remark = crate::bom::remark(line);
        let base = |c: &Col| match c.source.as_deref() {
            Some("field") | None => field_default(c),
            Some(src) => model_value(src, line, &remark),
        };
        rows.push(fill(&base, e.parts.get(&key), key.clone(), false));
    }
    for m in &e.manual {
        let base = |c: &Col| match c.source.as_deref() {
            Some("qty") => "1".into(),
            Some("field") | None => field_default(c),
            _ => String::new(),
        };
        rows.push(fill(&base, Some(&m.fields), format!("manual:{}", m.id), true));
    }
    let totals = t
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| if c.sum { fmt_num(rows.iter().filter_map(|r| r.cells[i].trim().parse::<f64>().ok()).sum()) } else { String::new() })
        .collect();
    (rows, totals)
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// 表 → CSV(带 UTF-8 BOM; 最后一行合计, 合计标签放在第一个求和列前一格)。
pub fn to_csv(t: &Template, rows: &[Row], totals: &[String]) -> String {
    let mut s = String::from("\u{feff}");
    s.push_str(&t.columns.iter().map(|c| csv_cell(&c.header)).collect::<Vec<_>>().join(","));
    s.push_str("\r\n");
    for r in rows {
        s.push_str(&r.cells.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        s.push_str("\r\n");
    }
    if let Some(first) = t.columns.iter().position(|c| c.sum) {
        let mut tot: Vec<String> = totals.to_vec();
        if first > 0 {
            tot[first - 1] = "total".into();
        }
        s.push_str(&tot.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        s.push_str("\r\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn oym_formulas_overrides_manual_totals() {
        let t = builtin();
        let scan = json!({ "lines": [
            { "key": "k1", "name": "Base side", "names": ["Base side"], "qty": 2, "material": "电木", "path": "" },
            { "key": "k2", "name": "杯头内六角 M4x14", "names": ["杯头内六角 M4x14"], "qty": 53, "material": "", "path": "" },
            { "key": "k3", "name": "参考体", "names": ["参考体"], "qty": 1, "material": "", "path": "" },
        ]});
        let mut e = Edits::default();
        e.defaults.insert("sets".into(), "5".into());
        // OYM 行 9: 2 × 5 = 10, ⌈10 × 1.2 / 1⌉ = 12, × 230 = 2760
        e.set_cell("k1", "unit_price", "230", "0");
        // OYM 行 52: 53 × 5 = 265, ⌈265 × 1.2 / 50⌉ = 7, × 6.1 = 42.7
        e.set_cell("k2", "origin", "采购", "代工");
        e.set_cell("k2", "pcs_unit", "50", "1");
        e.set_cell("k2", "unit", "包", "pc");
        e.set_cell("k2", "unit_price", "6.1", "0");
        e.excluded.push("k3".into());
        // 手动行(工具): 1 × 套数 1(覆盖) × 1 → 1 × 1205
        let mid = e.add_manual();
        e.set_cell(&format!("manual:{mid}"), "name", "博世电钻", "");
        e.set_cell(&format!("manual:{mid}"), "sets", "1", "");
        e.set_cell(&format!("manual:{mid}"), "redundancy", "1", "");
        e.set_cell(&format!("manual:{mid}"), "unit_price", "1205", "");
        let (rows, totals) = build(&scan, &t, &e);
        let col = |id: &str| t.columns.iter().position(|c| c.id == id).unwrap();
        assert_eq!(rows.len(), 3, "k3 排除, 手动行在后");
        let r1 = &rows[0];
        assert_eq!((r1.cells[col("total_qty")].as_str(), r1.cells[col("unit_qty")].as_str(), r1.cells[col("total_cost")].as_str()), ("10", "12", "2760"));
        assert_eq!(r1.cells[col("material")], "电木");
        assert!(r1.overridden[col("unit_price")] && !r1.overridden[col("sets")], "覆盖标记");
        let r2 = &rows[1];
        assert_eq!((r2.cells[col("total_qty")].as_str(), r2.cells[col("unit_qty")].as_str(), r2.cells[col("total_cost")].as_str()), ("265", "7", "42.7"));
        let r3 = &rows[2];
        assert!(r3.manual && r3.cells[col("name")] == "博世电钻");
        assert_eq!(r3.cells[col("total_cost")], "1205");
        assert_eq!(totals[col("total_cost")], "4007.7");
        // 覆盖值改回默认 → 覆盖去掉
        e.set_cell("k2", "unit", "pc", "pc");
        assert!(!e.parts["k2"].contains_key("unit"));
        // CSV: 表头 + 3 行 + 合计
        let csv = to_csv(&t, &rows, &totals);
        let lines: Vec<&str> = csv.trim_end().split("\r\n").collect();
        assert_eq!(lines.len(), 5);
        assert!(lines[0].trim_start_matches('\u{feff}').starts_with("名称,料号,Column 16,型号,用途,材料,来源,一套用量,套数,总数量"));
        assert!(lines[4].ends_with("total,4007.7"));
    }

    #[test]
    fn templates_dir_roundtrip() {
        let d = std::env::temp_dir().join(format!("pla_bom_tpl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(list_in(&d), vec![BUILTIN_NAME.to_string()]);
        assert_eq!(load_in(&d, BUILTIN_NAME).unwrap(), builtin());
        let _ = std::fs::remove_dir_all(&d);
    }
}
