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

/// 料号规则: 前缀 + 分类码 + 序号位数(OYM: 904 + 001 + 001)。分类: 码 → 说明。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PnRule {
    pub prefix: String,
    pub digits: usize,
    pub categories: BTreeMap<String, String>,
}

impl Default for PnRule {
    fn default() -> Self {
        let mut c = BTreeMap::new();
        c.insert("001".into(), "代工 非金属".into());
        c.insert("002".into(), "代工 金属".into());
        c.insert("009".into(), "采购件".into());
        PnRule { prefix: "904".into(), digits: 3, categories: c }
    }
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
    std::fs::read_to_string(path_in(dir)).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
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

    /// 某分类的下一个料号(前缀 + 分类 + 该分类已有最大序号 + 1, 补零到 digits 位)。
    pub fn next_pn(&self, category: &str) -> String {
        let head = format!("{}{}", self.pn_rule.prefix, category);
        let max = self.entries.iter().filter_map(|e| e.pn.strip_prefix(&head).and_then(|s| s.parse::<u64>().ok())).max().unwrap_or(0);
        format!("{head}{:0width$}", max + 1, width = self.pn_rule.digits)
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
    }
}
