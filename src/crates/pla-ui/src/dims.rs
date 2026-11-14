//! N3d: 尺寸记录(参数化的"记住尺寸, 以后反复改")。
//!
//! 存 `<文档目录>/.pla/models/<doc_key>/dims.json`(A9 映射表的位置; 设计见 docs/PRD/PRD-ai-modeling-v2.md §4)。
//! 面 id 每次修改都会变 → 记录里存面的**几何签名**(垫片 `dim.signature`), 用时经 `geo.resolve` 重新定位;
//! 本工具改完尺寸后按已知平移量预测新签名再定位, 刷新记录, 记录始终跟得上模型。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state_store::StateStore;

/// 目录与其它按文档存的状态一致(`.pla/<subdir>/<doc_key>/`)。
pub const STORE: StateStore = StateStore { subdir: "models", legacy_subdir: None, prefix: "D" };
const FILE: &str = "dims.json";

/// 一条尺寸记录。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DimRecord {
    /// D001, D002…(可改名)。
    pub name: String,
    /// "face" = 单面(厚度 / 半径 / 圆角半径); "distance" = 参考面 → 动的面组。
    pub mode: String,
    /// 尺寸类型(thickness / radius / filletRadius / distance), 界面显示用。
    pub kind: String,
    /// 面签名: face 模式 = [选中面]; distance 模式 = [参考面, 动的面…](顺序即选择顺序)。
    pub faces: Vec<Value>,
    /// face 模式的厚度: 所选对面的签名。
    #[serde(default)]
    pub opposite: Option<Value>,
    /// N3b 改哪一侧(0 选中面 / 1 对面 / 2 两侧各半)。
    #[serde(default)]
    pub side: u8,
    /// 最近一次读到 / 改成的值(文档单位)。
    pub value: f64,
    #[serde(default)]
    pub units: String,
    #[serde(default)]
    pub created: u64,
}

fn path(doc_key: &str) -> PathBuf {
    STORE.dir(doc_key).join(FILE)
}

/// 读全部记录(没有文件 / 读不了 → 空)。
pub fn load(doc_key: &str) -> Vec<DimRecord> {
    std::fs::read_to_string(path(doc_key))
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<DimRecord>>(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

/// 写全部记录(先写临时文件再改名, 不留半截文件)。
pub fn save(doc_key: &str, list: &[DimRecord]) -> Result<(), String> {
    let p = path(doc_key);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

/// 下一个自动名: D + 三位序号(取现有最大号 + 1)。
pub fn next_name(list: &[DimRecord]) -> String {
    let max = list
        .iter()
        .filter_map(|r| r.name.strip_prefix(STORE.prefix).and_then(|n| n.parse::<u32>().ok()))
        .max()
        .unwrap_or(0);
    format!("{}{:03}", STORE.prefix, max + 1)
}

/// 新增一条(自动命名), 返回名字。
pub fn add(doc_key: &str, mut rec: DimRecord) -> Result<String, String> {
    let mut list = load(doc_key);
    rec.name = next_name(&list);
    if rec.created == 0 {
        rec.created = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    }
    let name = rec.name.clone();
    list.push(rec);
    save(doc_key, &list)?;
    Ok(name)
}

/// 按名字替换一条(刷新签名 / 值后写回)。
pub fn update(doc_key: &str, rec: &DimRecord) -> Result<(), String> {
    let mut list = load(doc_key);
    match list.iter_mut().find(|r| r.name == rec.name) {
        Some(r) => *r = rec.clone(),
        None => return Err(format!("no dimension named {}", rec.name)),
    }
    save(doc_key, &list)
}

pub fn delete(doc_key: &str, name: &str) -> Result<(), String> {
    let mut list = load(doc_key);
    let n = list.len();
    list.retain(|r| r.name != name);
    if list.len() == n {
        return Err(format!("no dimension named {name}"));
    }
    save(doc_key, &list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(kind: &str) -> DimRecord {
        DimRecord {
            name: String::new(),
            mode: "face".into(),
            kind: kind.into(),
            faces: vec![json!({ "surface": "Plane", "center": [1, 2, 3], "normal": [0, 0, 1] })],
            opposite: None,
            side: 0,
            value: 5.0,
            units: "millimeter".into(),
            created: 0,
        }
    }

    #[test]
    fn add_update_delete_roundtrip() {
        let doc = format!("__test_dims_{}", std::process::id());
        let _ = std::fs::remove_dir_all(STORE.dir(&doc));
        assert!(load(&doc).is_empty());
        assert_eq!(add(&doc, rec("thickness")).unwrap(), "D001");
        assert_eq!(add(&doc, rec("radius")).unwrap(), "D002");
        let mut l = load(&doc);
        assert_eq!(l.len(), 2);
        assert!(l[0].created > 0);
        l[0].value = 8.0;
        update(&doc, &l[0]).unwrap();
        assert_eq!(load(&doc)[0].value, 8.0);
        delete(&doc, "D001").unwrap();
        assert_eq!(load(&doc).iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["D002"]);
        assert_eq!(add(&doc, rec("thickness")).unwrap(), "D003", "取最大号 + 1, 不复用删掉的号");
        assert!(delete(&doc, "nope").is_err());
        let _ = std::fs::remove_dir_all(STORE.dir(&doc));
    }
}
