//! B4: 零件库(全局, 跨文档; PRD-bom-drawings §3)。`<exe 目录>/parts-library/library.json`。
//!
//! 一条 = 料号(唯一)+ 采购名 + 字段(键 = BOM 模板的列 id: model / origin / unit / pcs_unit / flag …)
//! + 供应商列表(名称 / 链接 / 单价 / 币种 / 起订量 / 交期 / 记录日期)+ 首选供应商 + 匹配信息(别名、零件 key)。
//! BOM 里某个零件链接到料号后, 名称 / 这些字段 / 单价(首选供应商)从库里带出; 文档里的手工覆盖仍然优先。
//! 只记录采购信息, 不做采购动作。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Supplier {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    /// 单价(文本, 允许空)。
    #[serde(default)]
    pub price: String,
    #[serde(default)]
    pub currency: String,
    #[serde(default)]
    pub moq: String,
    #[serde(default)]
    pub lead: String,
    /// 价格记录日期(YYYY-MM-DD; 改价格时自动更新), 便于判断是否过期。
    #[serde(default)]
    pub date: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub pn: String,
    #[serde(default)]
    pub name: String,
    /// 料号分类码(料号规则里的一段, 如 001 / 002 / 009)。
    #[serde(default)]
    pub category: String,
    /// BOM 模板列 id → 值(model / origin / unit / pcs_unit / flag …)。
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub suppliers: Vec<Supplier>,
    #[serde(default)]
    pub preferred: usize,
    /// 匹配: 模型里的名字别名、零件 key(几何指纹)。
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

/// 料号规则(用户 2026-11-15): 前三位 = 项目编号(904 = OYM; 每个文档在 BOM 里设自己的项目编号, 这里是默认值),
/// 中间三位 = 类型, 后三位 = 序号。分类: 码 → 说明(001 塑胶非金属 / 002 金属 / 009 配件工具 由用户定, 其余为建议, 可在 library.json 里改)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PnRule {
    pub prefix: String,
    pub digits: usize,
    pub categories: BTreeMap<String, String>,
}

impl Default for PnRule {
    fn default() -> Self {
        let c: BTreeMap<String, String> = [
            ("001", "塑胶 / 非金属件(机加 / 注塑 / 3D 打印 / 电木)"),
            ("002", "金属件(机加 / 钣金 / 铸件)"),
            ("003", "橡胶 / 硅胶 / 密封件"),
            ("004", "标准紧固件(螺钉 / 螺母 / 垫片 / 销)"),
            ("005", "传动 / 导向件(轴承 / 光轴 / 直线轴承 / 导轨)"),
            ("006", "电子电气(线缆 / 连接器 / 传感器 / PCB)"),
            ("007", "外购组件(快夹 / 合页 / 搭扣 / 气动件)"),
            ("008", "包装 / 标签 / 耗材"),
            ("009", "配件工具(工具 / 量具)"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        PnRule { prefix: "904".into(), digits: 3, categories: c }
    }
}

/// 由材料名猜类型码(导入时的一致性检查、存入库时给分类; 不改已有料号): 塑胶 → 001, 金属 → 002, 认不出 → None。
pub fn material_category(material: &str) -> Option<&'static str> {
    let m = material.to_lowercase();
    const METAL: [&str; 10] = ["钢", "铝", "铜", "铁", "钛", "不锈", "steel", "alu", "brass", "6061"];
    const PLASTIC: [&str; 14] = ["pom", "pc", "abs", "pa6", "pa12", "尼龙", "电木", "塑", "pla", "petg", "树脂", "nylon", "peek", "亚克力"];
    if METAL.iter().any(|k| m.contains(k)) {
        Some("002")
    } else if PLASTIC.iter().any(|k| m.contains(k)) {
        Some("001")
    } else {
        None
    }
}

/// 导入报告。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportReport {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    /// (CSV 行号, 原因)
    pub skipped: Vec<(usize, String)>,
    /// 料号类型码与材料不一致: (料号, 材料, 料号里的类型码, 按材料应是)
    pub mismatched: Vec<(String, String, String, String)>,
}

/// 解析 CSV(支持双引号包裹、引号转义、字段内换行)。
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.trim_start_matches('\u{feff}');
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (true, _) => cell.push(c),
            (false, '"') if cell.is_empty() => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut cell)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            (false, _) => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

/// 从现有 BOM 表(CSV, 如 OYM testbed BOM)导入零件库。表头行 = 同时含"名称"和"料号"列标题的那一行(前面的空行跳过);
/// 列按 BOM 模板的表头对到列 id(headers: 列 id → 表头文字)。每行有料号才导入: 新料号 → 新条目; 已有 → 非空字段覆盖、单价写进首选供应商。
/// 同一个料号在表里出现两次且型号不同 → 第二次跳过并报告; 料号类型码与材料不符 → 只报告, 料号不改。
pub fn import_csv(lib: &mut Library, text: &str, headers: &[(String, String)]) -> ImportReport {
    let mut rep = ImportReport::default();
    let rows = parse_csv(text);
    let find_col = |row: &[String], h: &str| row.iter().position(|c| c.trim() == h);
    let header_of = |id: &str| headers.iter().find(|(i, _)| i == id).map(|(_, h)| h.clone()).unwrap_or_default();
    let (name_h, pn_h) = (header_of("name"), header_of("pn"));
    let Some(hr) = rows.iter().position(|r| find_col(r, &name_h).is_some() && find_col(r, &pn_h).is_some()) else {
        rep.skipped.push((0, format!("no header row with \"{name_h}\" and \"{pn_h}\"")));
        return rep;
    };
    let head = &rows[hr];
    let idx: BTreeMap<String, usize> = headers.iter().filter_map(|(id, h)| find_col(head, h).map(|i| (id.clone(), i))).collect();
    let get = |r: &[String], id: &str| idx.get(id).and_then(|&i| r.get(i)).map(|s| s.trim().to_string()).unwrap_or_default();
    // 只进库的字段(项目相关的 用途 / 套数 / 冗余系数 / 一套用量 不进库; 用途记进备注)
    const LIB_FIELDS: [&str; 6] = ["model", "origin", "pcs_unit", "unit", "flag", "material"];
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for (ri, r) in rows.iter().enumerate().skip(hr + 1) {
        let pn = get(r, "pn");
        let name = get(r, "name");
        if pn.is_empty() {
            if !name.is_empty() {
                rep.skipped.push((ri + 1, format!("{name}: no part number")));
            }
            continue;
        }
        let model = get(r, "model");
        if let Some(prev) = seen.get(&pn) {
            if *prev != model {
                rep.skipped.push((ri + 1, format!("{pn} used twice ({prev} / {model}); kept the first")));
            }
            continue;
        }
        seen.insert(pn.clone(), model.clone());
        let existed = lib.find(&pn).is_some();
        let mut e = lib.find(&pn).cloned().unwrap_or_else(|| Entry { pn: pn.clone(), ..Default::default() });
        if !name.is_empty() {
            e.name = name;
        }
        if e.category.is_empty() && pn.len() >= lib.pn_rule.prefix.len() + 3 {
            e.category = pn[lib.pn_rule.prefix.len()..lib.pn_rule.prefix.len() + 3].to_string();
        }
        for f in LIB_FIELDS {
            let v = get(r, f);
            if !v.is_empty() {
                e.fields.insert(f.into(), v);
            }
        }
        let usage = get(r, "usage");
        if !usage.is_empty() && !e.notes.contains(&usage) {
            e.notes = if e.notes.is_empty() { usage } else { format!("{}; {usage}", e.notes) };
        }
        let price = get(r, "unit_price");
        if price.parse::<f64>().is_ok() {
            match e.suppliers.get_mut(e.preferred) {
                Some(s) if s.price != price => {
                    s.price = price;
                    s.date = today();
                }
                Some(_) => {}
                None => e.suppliers.push(Supplier { price, currency: "CNY".into(), date: today(), ..Default::default() }),
            }
        }
        let mat = e.fields.get("material").cloned().unwrap_or_default();
        if let Some(want) = material_category(&mat) {
            if !e.category.is_empty() && e.category != want && (e.category == "001" || e.category == "002") {
                rep.mismatched.push((pn.clone(), mat, e.category.clone(), want.to_string()));
            }
        }
        lib.upsert(e);
        if existed { rep.updated.push(pn) } else { rep.added.push(pn) }
    }
    rep
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Library {
    #[serde(default)]
    pub pn_rule: PnRule,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

pub fn dir() -> PathBuf {
    crate::prefs::data_dir().join("parts-library")
}

fn path_in(dir: &Path) -> PathBuf {
    dir.join("library.json")
}

pub fn load_in(dir: &Path) -> Library {
    let mut lib: Library = std::fs::read_to_string(path_in(dir)).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_default();
    // 早期默认分类表(001 代工 非金属 / 002 代工 金属 / 009 采购件)原样未改过 → 换成现在的默认表
    let old: [(&str, &str); 3] = [("001", "代工 非金属"), ("002", "代工 金属"), ("009", "采购件")];
    if lib.pn_rule.categories.len() == 3 && old.iter().all(|(k, v)| lib.pn_rule.categories.get(*k).map(String::as_str) == Some(*v)) {
        lib.pn_rule.categories = PnRule::default().categories;
    }
    lib
}

pub fn save_in(dir: &Path, lib: &Library) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let p = path_in(dir);
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(lib).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

pub fn mtime_in(dir: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path_in(dir)).and_then(|m| m.modified()).ok()
}

/// 今天(本地时区不可得时用 UTC 日期即可, 只做"记录日期")。
pub fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    // 1970-01-01 起的天数 → 公历(Howard Hinnant 算法)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

impl Library {
    pub fn find(&self, pn: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.pn == pn)
    }

    pub fn find_mut(&mut self, pn: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.pn == pn)
    }

    /// 某项目某分类的下一个料号(项目编号 + 分类 + 该组合已有最大序号 + 1, 补零到 digits 位); project 空 = 规则里的默认前缀。
    pub fn next_pn_for(&self, project: &str, category: &str) -> String {
        let p = if project.trim().is_empty() { self.pn_rule.prefix.as_str() } else { project.trim() };
        let head = format!("{p}{category}");
        let max = self.entries.iter().filter_map(|e| e.pn.strip_prefix(&head).and_then(|s| s.parse::<u64>().ok())).max().unwrap_or(0);
        format!("{head}{:0width$}", max + 1, width = self.pn_rule.digits)
    }

    /// 默认项目编号下某分类的下一个料号。
    pub fn next_pn(&self, category: &str) -> String {
        self.next_pn_for("", category)
    }

    /// 建议链接: 零件 key 精确命中 > 名字等于别名 / 采购名 > 名字互相包含。返回料号。
    pub fn suggest(&self, name: &str, key: &str) -> Option<String> {
        if let Some(e) = self.entries.iter().find(|e| e.keys.iter().any(|k| k == key)) {
            return Some(e.pn.clone());
        }
        let n = name.trim().to_lowercase();
        if n.is_empty() {
            return None;
        }
        if let Some(e) = self.entries.iter().find(|e| e.name.to_lowercase() == n || e.aliases.iter().any(|a| a.to_lowercase() == n)) {
            return Some(e.pn.clone());
        }
        self.entries
            .iter()
            .find(|e| {
                let en = e.name.to_lowercase();
                (!en.is_empty() && (en.contains(&n) || n.contains(&en))) || e.aliases.iter().any(|a| { let a = a.to_lowercase(); !a.is_empty() && (a.contains(&n) || n.contains(&a)) })
            })
            .map(|e| e.pn.clone())
    }

    /// 新增或更新一条(按料号)。
    pub fn upsert(&mut self, e: Entry) {
        match self.entries.iter_mut().find(|x| x.pn == e.pn) {
            Some(x) => *x = e,
            None => self.entries.push(e),
        }
        self.entries.sort_by(|a, b| a.pn.cmp(&b.pn));
    }

    pub fn remove(&mut self, pn: &str) {
        self.entries.retain(|e| e.pn != pn);
    }
}

impl Entry {
    /// 带进 BOM 的字段: 自身字段 + name + pn + 首选供应商单价(列 id "unit_price")。
    pub fn bom_fields(&self) -> BTreeMap<String, String> {
        let mut f = self.fields.clone();
        f.insert("pn".into(), self.pn.clone());
        if !self.name.is_empty() {
            f.insert("name".into(), self.name.clone());
        }
        if let Some(s) = self.suppliers.get(self.preferred).or_else(|| self.suppliers.first()) {
            if !s.price.trim().is_empty() {
                f.insert("unit_price".into(), s.price.trim().to_string());
            }
        }
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_pn_suggest_fields_roundtrip() {
        let mut lib = Library::default();
        assert_eq!(lib.next_pn("009"), "904009001");
        let mut e = Entry { pn: lib.next_pn("009"), name: "304不锈钢杯头内六角螺丝".into(), category: "009".into(), ..Default::default() };
        e.fields.insert("model".into(), "M4*14[50个]".into());
        e.fields.insert("origin".into(), "采购".into());
        e.fields.insert("pcs_unit".into(), "50".into());
        e.fields.insert("unit".into(), "包".into());
        e.suppliers.push(Supplier { name: "淘宝 A".into(), price: "6.1".into(), date: "2026-11-15".into(), ..Default::default() });
        e.suppliers.push(Supplier { name: "淘宝 B".into(), price: "5.8".into(), ..Default::default() });
        e.aliases.push("杯内六M4x14".into());
        e.keys.push("k-screw".into());
        lib.upsert(e);
        assert_eq!(lib.next_pn("009"), "904009002");
        assert_eq!(lib.next_pn("001"), "904001001");
        // 建议: key > 别名 > 包含
        assert_eq!(lib.suggest("whatever", "k-screw").as_deref(), Some("904009001"));
        assert_eq!(lib.suggest("杯内六M4x14", "x").as_deref(), Some("904009001"));
        assert_eq!(lib.suggest("杯内六M4x14 copy", "x").as_deref(), Some("904009001"));
        assert_eq!(lib.suggest("Base side", "x"), None);
        // 带进 BOM: 首选供应商的单价
        let f = lib.find("904009001").unwrap().bom_fields();
        assert_eq!((f["pn"].as_str(), f["unit_price"].as_str(), f["pcs_unit"].as_str()), ("904009001", "6.1", "50"));
        lib.find_mut("904009001").unwrap().preferred = 1;
        assert_eq!(lib.find("904009001").unwrap().bom_fields()["unit_price"], "5.8");
        // 落盘
        let d = std::env::temp_dir().join(format!("pla_parts_lib_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        save_in(&d, &lib).unwrap();
        assert_eq!(load_in(&d), lib);
        assert_eq!(today().len(), 10);
        let _ = std::fs::remove_dir_all(&d);
        // 其它项目编号
        assert_eq!(lib.next_pn_for("905", "001"), "905001001");
    }

    /// 手动: 把一份真实 BOM CSV 导入某个零件库目录。
    /// PLA_IMPORT_CSV=<csv> PLA_LIB_DIR=<dir> cargo test -p pla-ui --release --lib import_real_csv -- --ignored --nocapture
    #[test]
    #[ignore]
    fn import_real_csv() {
        let (Ok(csv), Ok(dir)) = (std::env::var("PLA_IMPORT_CSV"), std::env::var("PLA_LIB_DIR")) else { return };
        let dir = std::path::PathBuf::from(dir);
        let mut lib = load_in(&dir);
        let headers: Vec<(String, String)> = crate::bom_table::builtin().columns.iter().map(|c| (c.id.clone(), c.header.clone())).collect();
        let rep = import_csv(&mut lib, &std::fs::read_to_string(csv).unwrap(), &headers);
        save_in(&dir, &lib).unwrap();
        println!("added {} updated {}\nskipped {:#?}\nmismatched {:#?}", rep.added.len(), rep.updated.len(), rep.skipped, rep.mismatched);
    }

    #[test]
    fn import_oym_like_csv() {
        // 前面空行 + 表头 + 代工 / 采购行 + 同料号两次 + 带引号逗号的用途 + 一行类型码与材料不符(铝放在 001)
        let csv = "\u{feff},,,\r\n,,,\r\n,名称,料号,Column 16,型号,用途,材料,来源,一套用量,套数,总数量,冗余系数,pcs/unit,unit,unit quantity,unit price,total cost\r\n\
,Base down,904001001,,1,,电木,代工,1,5,5,1.2,1,pc,6,300,1800\r\n\
,big platform,904002001,,1,,铝合金6061,代工,1,5,5,1.2,1,pc,6,350,2100\r\n\
,odd plate,904001002,,1,,铝合金6061,代工,1,5,5,1.2,1,pc,6,50,300\r\n\
,304不锈钢杯头内六角螺丝,904009022,1,M3*12[100个],锁舌固定x2,,采购,6,5,30,1.2,100,包,1,7.3,7.3\r\n\
,304不锈钢杯头内六角螺丝,904009022,1,M4*18[50个],固定x4,,采购,8,5,40,1.2,50,包,1,7.3,7.3\r\n\
,304不锈钢杯头内六角螺丝,904009023,1,M4*14[50个],\"箱体前立面x7，箱体后立面x8, 光轴x2\",,采购,53,5,265,1.2,50,包,7,6.1,42.7\r\n\
,没有料号的行,,,,,,,,,,,,,,,\r\n,,,,,,,,,,,,,,,total,38169.1\r\n";
        let headers: Vec<(String, String)> = crate::bom_table::builtin().columns.iter().map(|c| (c.id.clone(), c.header.clone())).collect();
        let mut lib = Library::default();
        let rep = import_csv(&mut lib, csv, &headers);
        assert_eq!(rep.added, vec!["904001001", "904002001", "904001002", "904009022", "904009023"]);
        assert!(rep.skipped.iter().any(|(_, w)| w.contains("904009022 used twice")), "{:?}", rep.skipped);
        assert!(rep.skipped.iter().any(|(_, w)| w.contains("没有料号的行")));
        assert_eq!(rep.mismatched, vec![("904001002".to_string(), "铝合金6061".to_string(), "001".to_string(), "002".to_string())]);
        let s = lib.find("904009023").unwrap();
        assert_eq!((s.category.as_str(), s.fields["model"].as_str(), s.fields["pcs_unit"].as_str(), s.fields["unit"].as_str()), ("009", "M4*14[50个]", "50", "包"));
        assert_eq!(s.bom_fields()["unit_price"], "6.1");
        assert!(s.notes.contains("光轴x2"), "带逗号的用途完整: {}", s.notes);
        assert_eq!(lib.find("904001001").unwrap().fields["material"], "电木");
        // 再导一次: 全部是更新, 不重复加供应商
        let rep2 = import_csv(&mut lib, csv, &headers);
        assert_eq!((rep2.added.len(), rep2.updated.len()), (0, 5));
        assert_eq!(lib.find("904009023").unwrap().suppliers.len(), 1);
    }
}
