//! C0: 配置 = 固化一个方案(用户 2026-11-15)—— 可见性(稳定 id)+ 材质指派, 附抓取时可见的实体清单(BOM 用)。
//!
//! 存储布局(跟随文档): `<文档目录>/.pla/configs/<doc_key>/CF001.json` + `CF001.png`。
//! 状态本体是垫片 `cfg.save` 的 JSON, 恢复时原样交给垫片 `cfg.restore`(memento, 不进撤销栈)。
//! 目录 / 列表 / 收藏 / 改名 / 删除 / 更新与视口状态、材质状态共用 `state_store`。

use pla_bridge::t;

use crate::state_store::StateStore;

pub const STORE: StateStore = StateStore { subdir: "configs", legacy_subdir: None, prefix: "CF" };

/// 配置里的可见实体(稳定 id, 名字); 解析不了 → 空。
pub fn members(state: &str) -> Vec<(String, String)> {
    serde_json::from_str::<serde_json::Value>(state)
        .ok()
        .and_then(|v| v["members"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|m| Some((m["id"].as_str()?.to_string(), m["name"].as_str().unwrap_or("").to_string())))
        .collect()
}

/// 悬浮摘要: 可见实体数 · 隐藏数 · 材质数 · 前几个可见实体名。解析不了如实说。
pub fn brief(state: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(state) else {
        return pla_bridge::i18n::tr("配置数据无法解析", "Configuration data unparsable").into();
    };
    let ms = members(state);
    let hidden = v["vis"]["hidden"].as_array().map_or(0, |a| a.len()) + v["vis"]["invisible"].as_array().map_or(0, |a| a.len());
    let mats = v["mat"]["materials"].as_array().map_or(0, |a| a.len());
    let mut names: Vec<String> = ms.iter().take(4).map(|m| m.1.clone()).collect();
    if ms.len() > 4 {
        names.push("…".into());
    }
    t!(
        "可见 {} 件 · 隐藏 {} · 材质 {} 个 · {}",
        "{} visible · {} hidden · {} materials · {}",
        ms.len(),
        hidden,
        mats,
        if names.is_empty() { "-".to_string() } else { names.join(", ") }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cf_prefix_members_brief() {
        let doc = format!("__test_cf_{}", std::process::id());
        let s = r#"{"v":1,"vis":{"hidden":["h1"],"invisible":[],"isolations":[]},"mat":{"v":1,"materials":[{"name":"Steel","uuid":"u1"}],"assign":[["a","u1"]]},"members":[{"id":"a","name":"底板"},{"id":"b","name":"螺钉"}]}"#;
        let stem = STORE.save(&doc, s, None).unwrap();
        assert_eq!(stem, "CF001");
        let loaded = STORE.load(&doc, &stem).unwrap();
        assert_eq!(members(&loaded), vec![("a".to_string(), "底板".to_string()), ("b".to_string(), "螺钉".to_string())]);
        let b = brief(&loaded);
        assert!(b.contains('2') && b.contains("底板"), "{b}");
        assert!(brief("nope").contains("无法解析") || brief("nope").contains("unparsable"));
        let _ = STORE.delete(&doc, &[stem]);
        let _ = std::fs::remove_dir_all(STORE.dir(&doc));
    }
}
