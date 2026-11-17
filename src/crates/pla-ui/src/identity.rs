//! 增量更新: 模型改了 → 零件的几何指纹(BOM key)变了, 但 Plasticity 里还是那几个实体(稳定 id 不变)。
//!
//! 每次扫 BOM 记下 key → 实例 id(`<bom 目录>/ids.json`, 只增不减; 按配置扫只看到一部分也不丢)。
//! 下次扫描: 旧 key 已经不在这次结果里, 而它的实例 id 出现在某个新 key 下 = 同一个零件改了几何 → 旧 key 改名成新 key。
//! 调用方把改名用到 BOM 填写(料号 / 单价 / 链接 / 排除)和出图设置(朝向 / 公差 / 隐藏 / 补尺寸 / 上传图纸 …)上, 不再从零开始。
//! 只改了其中几个实例(旧 key 还在)= 那几个变成了另一种零件, 不迁移。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

pub type Ids = BTreeMap<String, Vec<String>>;

fn path(doc_key: &str) -> std::path::PathBuf {
    crate::bom::STORE.dir(doc_key).join("ids.json")
}

pub fn load(doc_key: &str) -> Ids {
    std::fs::read_to_string(path(doc_key)).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
}

pub fn save(doc_key: &str, ids: &Ids) -> Result<(), String> {
    let p = path(doc_key);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(p, serde_json::to_vec(ids).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// 扫描结果里每个 key 的实例 id。
pub fn scan_ids(scan: &Value) -> Ids {
    scan["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| {
            let k = l["key"].as_str()?.to_string();
            let ids: Vec<String> = l["members"].as_array()?.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect();
            Some((k, ids))
        })
        .collect()
}

/// 找出改名(旧 key → 新 key), 并把这次扫描并进记录。一个旧 key 的实例分到几个新 key 时, 取重合最多的那个。
pub fn track(prev: &mut Ids, scan: &Value) -> Vec<(String, String)> {
    let now = scan_ids(scan);
    let mut renames = Vec::new();
    let mut taken: BTreeSet<String> = BTreeSet::new();
    for (old, old_ids) in prev.iter() {
        if now.contains_key(old) {
            continue;
        }
        let best = now
            .iter()
            .filter(|(k, _)| !prev.contains_key(*k) && !taken.contains(*k))
            .map(|(k, ids)| (k, ids.iter().filter(|i| old_ids.contains(i)).count()))
            .filter(|(_, n)| *n > 0)
            .max_by_key(|(_, n)| *n);
        if let Some((k, _)) = best {
            taken.insert(k.clone());
            renames.push((old.clone(), k.clone()));
        }
    }
    for (old, _) in &renames {
        prev.remove(old);
    }
    for (k, ids) in now {
        prev.insert(k, ids);
    }
    renames
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scan(lines: &[(&str, &[&str])]) -> Value {
        json!({"lines": lines.iter().map(|(k, ids)| json!({"key": k, "members": ids.iter().map(|i| json!({"id": i})).collect::<Vec<_>>()})).collect::<Vec<_>>()})
    }

    #[test]
    fn renames_follow_entity_ids() {
        let mut ids = Ids::new();
        assert!(track(&mut ids, &scan(&[("plate", &["a"]), ("screw", &["s1", "s2", "s3"])])).is_empty());
        // 板改了几何: key 变了, 实体 a 还在 → plate → plate2
        let r = track(&mut ids, &scan(&[("plate2", &["a"]), ("screw", &["s1", "s2", "s3"])]));
        assert_eq!(r, vec![("plate".to_string(), "plate2".to_string())]);
        assert!(!ids.contains_key("plate") && ids.contains_key("plate2"));
        // 只改了一颗螺钉: 旧 key 还在 → 改的那颗是新零件, 不迁移
        assert!(track(&mut ids, &scan(&[("plate2", &["a"]), ("screw", &["s1", "s2"]), ("screw-long", &["s3"])])).is_empty());
        // 按配置扫只看到一部分: 看不到的旧 key 不算改名, 记录也不丢
        assert!(track(&mut ids, &scan(&[("plate2", &["a"])])).is_empty());
        assert!(ids.contains_key("screw"));
        // 全部螺钉一起改长: 取重合最多的新 key
        let r = track(&mut ids, &scan(&[("plate2", &["a"]), ("screw-x", &["s1", "s2"]), ("screw-y", &["s9"])]));
        assert_eq!(r, vec![("screw".to_string(), "screw-x".to_string())]);
    }
}
