//! 按文档的作业逻辑(BOM 零件 / 零件身份迁移 / 出图方案 / 外发包), pla-ui 和后台作业(pla-mcp 的 background_*)共用 —— 唯一来源。
//! 用户 2026-11-18: 后台 Plasticity 实例也要能出图、扫 BOM、出外发包。这里只管数据(读写文档的 `.pla/…` 存档), 不碰界面 / CDP。

use crate::bom_table::{self, Edits, Row, Template};
use crate::drawing::{self, Prefs, Sheet, Sheets};
use crate::mcp_bridge::PartInfo;
use crate::parts_lib::{self, Library};
use crate::release::{self, PartSnap, Suppliers};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 一个零件: 标题栏字段(pn / name / material / finish / qty)、第一个实例(稳定 id, 名字)、摘要。
pub type Part = (BTreeMap<String, String>, (String, String), PartInfo);

/// 一个文档的作业状态(从存档读; pla-ui 里同样的东西放在界面状态里)。
pub struct DocState {
    pub key: String,
    pub tpl: Template,
    pub edits: Edits,
    pub lib: Library,
    pub suppliers: Suppliers,
    pub sheets: Sheets,
}

impl DocState {
    /// 按 pla-ui 的规则读: BOM 模板 = 本文档记住的那个(没有就模板目录里第一个, 再没有就内置)。
    pub fn load(key: &str) -> Self {
        let edits = bom_table::load_edits(key);
        let dir = bom_table::dir();
        let list = bom_table::list_in(&dir);
        let name = if !edits.template.is_empty() && list.contains(&edits.template) { edits.template.clone() } else { list.first().cloned().unwrap_or_default() };
        let tpl = if name.is_empty() { bom_table::builtin() } else { bom_table::load_in(&dir, &name).unwrap_or_else(|_| bom_table::builtin()) };
        DocState {
            key: key.to_string(),
            tpl,
            edits,
            lib: parts_lib::load_in(&parts_lib::dir()),
            suppliers: Suppliers::load(),
            sheets: drawing::load_in(&drawing::dir(key)),
        }
    }
    pub fn save_sheets(&self) -> Result<(), String> {
        drawing::save_in(&drawing::dir(&self.key), &self.sheets)
    }
}

/// BOM 扫描结果 → 零件(手填行除外)。
pub fn bom_parts(scan: &Value, tpl: &Template, edits: &Edits, lib: &Library, purchased: &[String]) -> Vec<Part> {
    let (rows, _) = bom_table::build(scan, tpl, edits, lib);
    let cols = &tpl.columns;
    let mut out = Vec::new();
    for r in rows.iter().filter(|r| !r.manual) {
        let get = |id: &str| cols.iter().position(|c| c.id == id).map(|i| r.cells[i].trim().to_string()).unwrap_or_default();
        let line = scan["lines"].as_array().into_iter().flatten().find(|l| l["key"].as_str() == Some(r.key.as_str()));
        let m = line.and_then(|l| l["members"].as_array()).and_then(|a| a.first());
        let member = m.map(|m| (m["id"].as_str().unwrap_or("").to_string(), m["name"].as_str().unwrap_or("").to_string())).unwrap_or_default();
        let qty = line.and_then(|l| l["qty"].as_u64()).unwrap_or(1);
        let mut title = BTreeMap::new();
        for id in ["pn", "name", "material", "finish"] {
            let x = get(id);
            if !x.is_empty() {
                title.insert(id.to_string(), x);
            }
        }
        title.insert("qty".into(), qty.to_string());
        let name = if get("name").is_empty() { member.1.clone() } else { get("name") };
        let bought = line.is_some_and(|l| drawing::member_bought(&l["members"], purchased)) || drawing::looks_standard(&name, &get("pn"));
        let info = PartInfo { key: r.key.clone(), pn: get("pn"), name, qty, material: get("material"), supplier: release::process_of(edits, &r.key).to_string(), bought };
        out.push((title, member, info));
    }
    out
}

/// 零件身份迁移的结果: 改过几何的零件(key 变了、实体还是那个)把 BOM 填写 / 出图设置 / 版本记录跟过去。
#[derive(Debug, Default)]
pub struct Migrated {
    pub renames: Vec<(String, String)>,
    pub bom: usize,
    pub drawings: usize,
}

/// 存档里做零件身份迁移(读 → 改 → 写)。pla-ui 调完按需重读界面状态。
pub fn identity_migrate(key: &str, scan: &Value) -> Result<Migrated, String> {
    let dir = drawing::dir(key);
    let mut sheets = drawing::load_in(&dir);
    let mut ids = crate::identity::load(key);
    for s in sheets.sheets.iter().filter(|s| !drawing::is_asm(&s.key) && !s.member.is_empty()) {
        ids.entry(s.key.clone()).or_insert_with(|| vec![s.member.clone()]);
    }
    let renames = crate::identity::track(&mut ids, scan);
    crate::identity::save(key, &ids).map_err(|e| format!("saving part identities: {e}"))?;
    let mut m = Migrated { renames, ..Default::default() };
    if m.renames.is_empty() {
        return Ok(m);
    }
    let mut edits = bom_table::load_edits(key);
    let rroot = release::STORE.dir(key);
    let mut hist = release::History::load(&rroot);
    let mut hist_changed = false;
    for (old, new) in &m.renames {
        m.bom += usize::from(edits.rename_key(old, new));
        m.drawings += usize::from(sheets.rename_key(&dir, old, new));
        hist_changed |= hist.rename_key(old, new);
    }
    if hist_changed {
        hist.save(&rroot).map_err(|e| format!("saving the version history: {e}"))?;
    }
    if m.bom > 0 {
        bom_table::save_edits(key, &edits).map_err(|e| format!("saving BOM entries: {e}"))?;
    }
    if m.drawings > 0 {
        drawing::save_in(&dir, &sheets).map_err(|e| format!("saving drawing settings: {e}"))?;
    }
    Ok(m)
}

/// 外发快照: 当前全部零件(外发时存, 之后跟它比出变更)。
pub fn rel_snapshot(key: &str, parts: &[Part], sheets: &Sheets) -> BTreeMap<String, PartSnap> {
    let ids = crate::identity::load(key);
    parts
        .iter()
        .map(|(_, member, p)| {
            let sh = sheets.get(&p.key);
            let snap = PartSnap {
                pn: p.pn.clone(),
                name: p.name.clone(),
                qty: p.qty,
                material: p.material.clone(),
                supplier: p.supplier.clone(),
                ids: ids.get(&p.key).cloned().unwrap_or_else(|| if member.0.is_empty() { vec![] } else { vec![member.0.clone()] }),
                sheet: sh.map(release::sheet_hash).unwrap_or_default(),
                upload: sh.and_then(|s| s.uploaded.as_ref()).map(|u| format!("{} {}", u.file, u.date)).unwrap_or_default(),
            };
            (p.key.clone(), snap)
        })
        .collect()
}

/// 各家的包内容(按当前 BOM / 填写 / 图)+ 总 BOM 的 CSV。
pub fn rel_packs(key: &str, suppliers: &Suppliers, tpl: &Template, rows: &[Row], totals: &[String], edits: &Edits, sheets: &Sheets) -> (Vec<release::Pack>, String) {
    let ddir = drawing::dir(key);
    let mut packs = Vec::new();
    for s in &suppliers.list {
        let (h, t, keys) = release::supplier_table(s, tpl, rows, edits);
        if t.is_empty() {
            continue;
        }
        let mut files = Vec::new();
        let mut missing = Vec::new();
        for (k, row) in keys.iter().zip(&t) {
            let (f, m) = release::part_files(&ddir, sheets.get(k), &s.files);
            files.extend(f);
            if !m.is_empty() {
                missing.push((row.get(2).cloned().unwrap_or_default(), m));
            }
        }
        packs.push(release::Pack { supplier: s.clone(), csv: release::table_csv(&h, &t), files, missing });
    }
    (packs, bom_table::to_csv(tpl, rows, totals))
}

/// 零件第一次出图: 建一张图(图名 = 料号或名字, 重名加 -2…; 密度按材料猜); 已有的跟着 BOM 换实例。返回是否改了。
pub fn ensure_sheet(sheets: &mut Sheets, key: &str, title: &BTreeMap<String, String>, member: &(String, String)) -> bool {
    if sheets.get(key).is_none() {
        let want = drawing::base_name(title.get("pn").map(|s| s.as_str()).unwrap_or(""), title.get("name").map(|s| s.as_str()).unwrap_or(&member.1));
        let base = sheets.unique_base(key, &want);
        let mut s = Sheet::new(key, &base, &member.0, &member.1);
        s.density = title.get("material").and_then(|m| drawing::guess_density(m));
        sheets.upsert(s);
        return true;
    }
    if let Some(mut s) = sheets.get(key).cloned() {
        if !member.0.is_empty() && (s.member != member.0 || s.model_name != member.1) {
            s.member = member.0.clone();
            s.model_name = member.1.clone();
            sheets.upsert(s);
            return true;
        }
    }
    false
}

/// 一张零件图的编译输入: 偏好垫底 + BOM / 本图覆盖, 变更表(最近两条 ECN), 装配图记下的配合 → 写 spec.json, 返回它的路径。
pub fn write_part_spec(doc_key: &str, key: &str, sheets: &Sheets, prefs: &Prefs, bom_title: &BTreeMap<String, String>) -> Result<PathBuf, String> {
    let s = sheets.get(key).cloned().ok_or_else(|| format!("no sheet {key}"))?;
    let dir = drawing::dir(doc_key);
    let (title, mut run) = prefs.apply(&s, bom_title);
    run.title = title;
    let hist = release::History::load(&release::STORE.dir(doc_key));
    for (i, (no, desc, by, date)) in hist.rev_rows(key).into_iter().enumerate() {
        let r = i + 1;
        run.title.insert(format!("rev{r}_ecn"), no);
        run.title.insert(format!("rev{r}_desc"), desc);
        run.title.insert(format!("rev{r}_by"), by);
        run.title.insert(format!("rev{r}_date"), date);
    }
    let mut v = drawing::spec(&run, &dir, Some(&crate::dims::path(doc_key)));
    let fits: Vec<Value> = sheets.sheets.iter().filter(|a| drawing::is_asm(&a.key)).flat_map(|a| drawing::part_fits(&a.fits, key)).collect();
    if !fits.is_empty() {
        v["fits"] = serde_json::json!(fits);
    }
    let spec_file = drawing::file(&dir, &s, "spec.json");
    std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&spec_file, serde_json::to_vec_pretty(&v).unwrap_or_default())).map_err(|e| format!("writing the drawing spec: {e}"))?;
    Ok(spec_file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_sheet_creates_and_follows_member() {
        let mut sh = Sheets::default();
        let mut t = BTreeMap::new();
        t.insert("pn".to_string(), "P-001".to_string());
        t.insert("material".to_string(), "6061".to_string());
        assert!(ensure_sheet(&mut sh, "k1", &t, &("12".into(), "Body".into())));
        assert_eq!(sh.get("k1").map(|s| s.member.as_str()), Some("12"));
        assert!(!ensure_sheet(&mut sh, "k1", &t, &("12".into(), "Body".into())), "没变就不改");
        assert!(ensure_sheet(&mut sh, "k1", &t, &("34".into(), "Body".into())), "实例换了跟过去");
        assert_eq!(sh.get("k1").map(|s| s.member.as_str()), Some("34"));
    }
}
