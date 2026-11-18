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
    /// 模型数据: name / qty / material / volume / group / remark / spec(认出的标准件规格); "field" = 可编辑字段; 省略且有 formula = 公式列。
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

/// 模型数据列可用的来源(source 省略 + 有公式 = 公式列)。
pub const SOURCES: &[&str] = &["name", "qty", "material", "volume", "group", "remark", "spec", "field"];

/// 公式里引用的列 id(`{id}`)。
pub fn formula_refs(f: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = f;
    while let Some(a) = rest.find('{') {
        let after = &rest[a + 1..];
        match after.find('}') {
            Some(b) => {
                out.push(after[..b].trim().to_string());
                rest = &after[b + 1..];
            }
            None => break,
        }
    }
    out
}

/// 模板体检: 至少一列、id 非空不重复、来源合法、公式引用的列存在、公式之间不循环。
pub fn check(t: &Template) -> Result<(), String> {
    if t.columns.is_empty() {
        return Err("a template needs at least one column".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for c in &t.columns {
        if c.id.trim().is_empty() || c.id.contains(['{', '}']) {
            return Err(format!("bad column id {:?}", c.id));
        }
        if !seen.insert(c.id.as_str()) {
            return Err(format!("duplicate column id {}", c.id));
        }
        match (&c.source, &c.formula) {
            (Some(s), _) if !SOURCES.contains(&s.as_str()) => return Err(format!("column {}: unknown source {s} (have: {})", c.id, SOURCES.join(", "))),
            (Some(_), Some(_)) => return Err(format!("column {}: either source or formula, not both", c.id)),
            _ => {}
        }
    }
    for c in &t.columns {
        for r in c.formula.as_deref().map(formula_refs).unwrap_or_default() {
            if !seen.contains(r.as_str()) {
                return Err(format!("column {}: formula refers to unknown column {{{r}}}", c.id));
            }
        }
    }
    // 循环: 公式列之间的依赖做 DFS
    let deps = |id: &str| t.columns.iter().find(|c| c.id == id).and_then(|c| c.formula.as_deref()).map(formula_refs).unwrap_or_default();
    fn visit(id: &str, deps: &dyn Fn(&str) -> Vec<String>, path: &mut Vec<String>) -> Result<(), String> {
        if let Some(i) = path.iter().position(|p| p == id) {
            return Err(format!("formula loop: {} → {id}", path[i..].join(" → ")));
        }
        path.push(id.to_string());
        for d in deps(id) {
            visit(&d, deps, path)?;
        }
        path.pop();
        Ok(())
    }
    for c in t.columns.iter().filter(|c| c.formula.is_some()) {
        visit(&c.id, &deps, &mut Vec::new())?;
    }
    Ok(())
}

/// 删除模板文件(最后一个不删)。
pub fn remove_in(dir: &Path, name: &str) -> Result<(), String> {
    if list_in(dir).len() <= 1 {
        return Err("cannot remove the only BOM template".into());
    }
    std::fs::remove_file(path_in(dir, name)).map_err(|e| e.to_string())
}

/// 模板名能不能当文件名。
pub fn valid_name(n: &str) -> bool {
    !n.trim().is_empty() && !n.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
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
    /// B4: 行(零件 key / "manual:<id>")→ 零件库料号。
    #[serde(default)]
    pub links: BTreeMap<String, String>,
    /// 本文档的项目编号(料号前三位, 如 904 = OYM); 空 = 零件库规则里的默认前缀。
    #[serde(default)]
    pub project: String,
}

fn edits_path(doc_key: &str) -> PathBuf {
    crate::bom::STORE.dir(doc_key).join("edits.json")
}

pub fn load_edits(doc_key: &str) -> Edits {
    std::fs::read_to_string(edits_path(doc_key)).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
}

/// 填写存档的修改时间(pla-ui 检测外部修改用)。
pub fn edits_mtime(doc_key: &str) -> Option<std::time::SystemTime> {
    std::fs::metadata(edits_path(doc_key)).and_then(|m| m.modified()).ok()
}

pub fn save_edits(doc_key: &str, e: &Edits) -> Result<(), String> {
    let p = edits_path(doc_key);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, serde_json::to_vec_pretty(e).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

impl Edits {
    /// 增量更新: 零件改了几何(key 变了)→ 填写 / 链接 / 排除跟到新 key。新 key 上已有的不覆盖。返回是否动了。
    pub fn rename_key(&mut self, old: &str, new: &str) -> bool {
        let mut changed = false;
        if let Some(m) = self.parts.remove(old) {
            let dst = self.parts.entry(new.into()).or_default();
            for (k, v) in m {
                dst.entry(k).or_insert(v);
            }
            changed = true;
        }
        if let Some(pn) = self.links.remove(old) {
            self.links.entry(new.into()).or_insert(pn);
            changed = true;
        }
        if let Some(i) = self.excluded.iter().position(|k| k == old) {
            self.excluded.remove(i);
            if !self.excluded.iter().any(|k| k == new) {
                self.excluded.push(new.into());
            }
            changed = true;
        }
        changed
    }

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
    /// B4: 每格是否来自零件库。
    pub from_lib: Vec<bool>,
    pub manual: bool,
    /// B4: 已链接的料号 / 没链接时零件库给的建议料号。
    pub link: Option<String>,
    pub suggest: Option<String>,
    /// 认出来的标准件规格(std_parts; 手动行 / 认不出 = None)。
    pub spec: Option<crate::std_parts::StdSpec>,
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

/// `bom.scan` 结果 + 模板 + 编辑 + 零件库 → 表行(零件行在前, 手动行在后) + 合计行(sum 列求和, 其余空)。
/// 每格取值优先级: 文档里的手工覆盖 > 零件库(该行链接了料号)> 本文档默认值 / 模型数据 > 模板默认值。
pub fn build(scan: &Value, t: &Template, e: &Edits, lib: &crate::parts_lib::Library) -> (Vec<Row>, Vec<String>) {
    let mut rows = Vec::new();
    let field_default = |c: &Col| e.defaults.get(&c.id).cloned().or_else(|| c.default.clone()).unwrap_or_default();
    let fill = |base: &dyn Fn(&Col) -> String, over: Option<&BTreeMap<String, String>>, key: String, manual: bool, name_hint: &str, spec: Option<crate::std_parts::StdSpec>| -> Row {
        // 链接写的可能是改号前的旧料号 → 认到现条目, 显示现料号
        let linked = e.links.get(&key).and_then(|pn| lib.find_any(pn));
        let link = linked.map(|en| en.pn.clone());
        let libf = linked.map(|en| en.bom_fields()).unwrap_or_default();
        let suggest = if link.is_none() { lib.suggest(name_hint, &key, spec.as_ref().map(|s| s.key.as_str())) } else { None };
        let mut vals: BTreeMap<String, String> = BTreeMap::new();
        let mut overridden = vec![false; t.columns.len()];
        let mut from_lib = vec![false; t.columns.len()];
        // 多遍: 公式可以引用后面的列(链长最多 = 公式列数; 循环由 check 挡在保存前)
        let passes = 1 + t.columns.iter().filter(|c| c.formula.is_some()).count().max(1);
        for _ in 0..passes {
            for (i, c) in t.columns.iter().enumerate() {
                let v = if let Some(f) = &c.formula {
                    eval_formula(f, &vals)
                } else {
                    match (over.and_then(|m| m.get(&c.id)), libf.get(&c.id)) {
                        (Some(o), _) => {
                            overridden[i] = !manual;
                            o.clone()
                        }
                        (None, Some(l)) => {
                            from_lib[i] = true;
                            l.clone()
                        }
                        (None, None) => base(c),
                    }
                };
                vals.insert(c.id.clone(), v);
            }
        }
        Row { key, cells: t.columns.iter().map(|c| vals.get(&c.id).cloned().unwrap_or_default()).collect(), overridden, from_lib, manual, link, suggest, spec }
    };
    for line in scan["lines"].as_array().into_iter().flatten() {
        let key = line["key"].as_str().unwrap_or("").to_string();
        if e.excluded.contains(&key) {
            continue;
        }
        let remark = crate::bom::remark(line);
        let spec = crate::std_parts::classify(&line["shape"]);
        let spec_text = spec.as_ref().map(|s| s.text()).unwrap_or_default();
        let base = |c: &Col| match c.source.as_deref() {
            Some("field") | None => field_default(c),
            Some("spec") => spec_text.clone(),
            Some(src) => model_value(src, line, &remark),
        };
        rows.push(fill(&base, e.parts.get(&key), key.clone(), false, line["name"].as_str().unwrap_or(""), spec));
    }
    for m in &e.manual {
        let base = |c: &Col| match c.source.as_deref() {
            Some("qty") => "1".into(),
            Some("field") | None => field_default(c),
            _ => String::new(),
        };
        let hint = m.fields.get("name").cloned().unwrap_or_default();
        rows.push(fill(&base, Some(&m.fields), format!("manual:{}", m.id), true, &hint, None));
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
        let (rows, totals) = build(&scan, &t, &e, &crate::parts_lib::Library::default());
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
    fn library_layer_and_suggestions() {
        use crate::parts_lib::{Entry, Library, Supplier};
        let t = builtin();
        let scan = json!({ "lines": [
            { "key": "ks", "name": "杯内六M4x14", "names": ["杯内六M4x14"], "qty": 53, "material": "", "path": "" },
            { "key": "kb", "name": "Base side", "names": ["Base side"], "qty": 2, "material": "电木", "path": "" },
        ]});
        let mut lib = Library::default();
        let mut en = Entry { pn: "904009023".into(), name: "304不锈钢杯头内六角螺丝".into(), ..Default::default() };
        en.fields.insert("origin".into(), "采购".into());
        en.fields.insert("pcs_unit".into(), "50".into());
        en.fields.insert("unit".into(), "包".into());
        en.suppliers.push(Supplier { price: "6.1".into(), ..Default::default() });
        en.aliases.push("杯内六M4x14".into());
        lib.upsert(en);
        let mut e = Edits::default();
        e.defaults.insert("sets".into(), "5".into());
        let col = |id: &str| t.columns.iter().position(|c| c.id == id).unwrap();
        // 没链接: 只给建议
        let (rows, _) = build(&scan, &t, &e, &lib);
        assert_eq!((rows[0].link.as_deref(), rows[0].suggest.as_deref()), (None, Some("904009023")));
        assert_eq!(rows[1].suggest, None);
        // 链接后: 名称 / 料号 / 字段 / 单价从库里来, 公式跟着算; 手工覆盖仍然优先
        e.links.insert("ks".into(), "904009023".into());
        e.set_cell("ks", "unit", "盒", "\u{0}");
        let (rows, _) = build(&scan, &t, &e, &lib);
        let r = &rows[0];
        assert_eq!(r.link.as_deref(), Some("904009023"));
        assert_eq!((r.cells[col("name")].as_str(), r.cells[col("pn")].as_str()), ("304不锈钢杯头内六角螺丝", "904009023"));
        assert!(r.from_lib[col("pn")] && r.from_lib[col("unit_price")]);
        assert_eq!(r.cells[col("unit")], "盒");
        assert!(r.overridden[col("unit")] && !r.from_lib[col("unit")], "覆盖优先于库");
        assert_eq!((r.cells[col("unit_qty")].as_str(), r.cells[col("total_cost")].as_str()), ("7", "42.7"));
        // 库里删了这个料号 → 链接失效, 回到原值
        lib.remove("904009023");
        let (rows, _) = build(&scan, &t, &e, &lib);
        assert_eq!(rows[0].link, None);
        assert_eq!(rows[0].cells[col("name")], "杯内六M4x14");
    }

    #[test]
    fn template_check_and_forward_chain() {
        assert!(check(&builtin()).is_ok());
        assert_eq!(formula_refs("ROUNDUP({a}*{ b },0)"), vec!["a", "b"]);
        let mut t = builtin();
        t.columns.push(col("x", "X", None, Some("{nope}*2"), None, false));
        assert!(check(&t).unwrap_err().contains("nope"));
        let mut t = builtin();
        t.columns.push(col("p", "P", None, Some("{q}+1"), None, false));
        t.columns.push(col("q", "Q", None, Some("{p}+1"), None, false));
        assert!(check(&t).unwrap_err().contains("loop"));
        let mut t = builtin();
        t.columns.push(col("sets", "dup", Some("field"), None, None, false));
        assert!(check(&t).unwrap_err().contains("duplicate"));
        let mut t = builtin();
        t.columns[0].source = Some("colour".into());
        assert!(check(&t).is_err());
        // 前向引用链 a → b → c(c 在最后)也要算对
        let t = Template {
            columns: vec![
                col("a", "A", None, Some("{b}+1"), None, false),
                col("b", "B", None, Some("{c}*2"), None, false),
                col("c", "C", Some("field"), None, Some("5"), false),
            ],
        };
        assert!(check(&t).is_ok());
        let scan = json!({ "lines": [ { "key": "k1", "name": "p", "qty": 1 } ] });
        let (rows, _) = build(&scan, &t, &Edits::default(), &crate::parts_lib::Library::default());
        assert_eq!(rows[0].cells, vec!["11", "10", "5"]);
    }

    #[test]
    fn rename_key_moves_entries() {
        let mut e = Edits::default();
        e.set_cell("old", "unit_price", "6.1", "0");
        e.links.insert("old".into(), "904009023".into());
        e.excluded.push("gone".into());
        e.set_cell("new", "unit", "包", "pc");
        assert!(e.rename_key("old", "new"));
        assert_eq!((e.parts["new"]["unit_price"].as_str(), e.parts["new"]["unit"].as_str()), ("6.1", "包"));
        assert_eq!(e.links["new"], "904009023");
        assert!(!e.parts.contains_key("old") && !e.links.contains_key("old"));
        assert!(e.rename_key("gone", "gone2") && e.excluded == vec!["gone2".to_string()]);
        assert!(!e.rename_key("nothing", "x"));
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
