//! R34: 材质状态存档 —— 材质定义(glTF 风格, uuid 稳定) + 物体指派(稳定 id → 材质 uuid)。
//!
//! 存储布局(跟随文档): `<文档目录>/.pla/materials/<doc_key>/MS001.json` + `MS001.png`。
//! 状态本体是垫片 `mat.save` 的 JSON, 恢复时原样交给垫片 `mat.restore`(memento, 不进撤销栈)。
//! 目录/列表/收藏/改名/删除/更新与视口状态共用 `state_store`。

use pla_bridge::t;

use crate::state_store::StateStore;

pub const STORE: StateStore = StateStore { subdir: "materials", legacy_subdir: None, prefix: "MS" };

/// 悬浮摘要: 材质数 / 指派数 / 前几个材质名。解析不了如实说。
pub fn brief(state: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(state) else {
        return pla_bridge::i18n::tr("状态数据无法解析", "State data unparsable").into();
    };
    let mats = v["materials"].as_array().cloned().unwrap_or_default();
    let n_assign = v["assign"].as_array().map(|a| a.len()).unwrap_or(0);
    let mut names: Vec<String> = mats
        .iter()
        .take(4)
        .map(|m| m["name"].as_str().unwrap_or("?").to_string())
        .collect();
    if mats.len() > 4 {
        names.push("…".into());
    }
    t!(
        "材质 {} 个 / 指派 {} 个 · {}",
        "{} materials / {} assignments · {}",
        mats.len(),
        n_assign,
        if names.is_empty() { "-".to_string() } else { names.join(", ") }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ms_prefix_and_brief() {
        let doc = format!("__test_ms_{}", std::process::id());
        let s = r#"{"v":1,"materials":[{"name":"Red","uuid":"u1"}],"assign":[["a","u1"],["b","u1"]]}"#;
        let stem = STORE.save(&doc, s, None).unwrap();
        assert_eq!(stem, "MS001");
        let b = brief(&STORE.load(&doc, &stem).unwrap());
        assert!(b.contains("Red") && b.contains('2'), "{b}");
        assert!(brief("nope").contains("无法解析") || brief("nope").contains("unparsable"));
        let _ = STORE.delete(&doc, &[stem]);
        let _ = std::fs::remove_dir_all(STORE.dir(&doc));
    }
}
