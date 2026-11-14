//! R8-1: 视口状态存档 — 保存/恢复摄像机、缩放、剖面、参考面、显示模式。
//!
//! 存储布局(R9 起跟随文档): `<文档目录>/.pla/viewstates/<doc_key>/VS001.json` + `VS001.png`;
//! 定位不到文档时退回 exe 同级 `viewstates/<doc_key>`(并迁移)。状态本体是垫片 `vp.save`
//! 的 JSON, 恢复时原样交给垫片 `vp.restore`。目录/列表/收藏/改名/删除/更新的实现见
//! `state_store`(与材质状态共用)。

use std::path::PathBuf;

use crate::state_store::{StateMeta, StateStore};

pub const STORE: StateStore = StateStore { subdir: "viewstates", legacy_subdir: Some("viewstates"), prefix: "VS" };

pub type VpStateMeta = StateMeta;

pub fn vpstate_dir(doc_key: &str) -> PathBuf {
    STORE.dir(doc_key)
}
pub fn save_vpstate(doc_key: &str, state: &str, png: Option<&[u8]>) -> Result<String, String> {
    STORE.save(doc_key, state, png)
}
pub fn update_vpstate(doc_key: &str, name: &str, state: &str, png: Option<&[u8]>) -> Result<(), String> {
    STORE.update(doc_key, name, state, png)
}
pub fn load_vpstate(doc_key: &str, name: &str) -> Option<String> {
    STORE.load(doc_key, name)
}
pub fn list_vpstates(doc_key: &str) -> Vec<VpStateMeta> {
    STORE.list(doc_key)
}
pub fn set_fav(doc_key: &str, name: &str, fav: bool) -> Result<(), String> {
    STORE.set_fav(doc_key, name, fav)
}
pub fn rename_vpstate(doc_key: &str, old: &str, new: &str) -> Result<(), String> {
    STORE.rename(doc_key, old, new)
}
pub fn delete_vpstates(doc_key: &str, names: &[String]) -> Result<usize, String> {
    STORE.delete(doc_key, names)
}
pub fn open_dir(doc_key: &str) -> Result<(), String> {
    STORE.open_dir(doc_key)
}

/// 悬浮摘要: 显示模式 / 剖面 / 透视正交 / 缩放。
pub fn brief(state: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(state) else {
        return pla_bridge::i18n::tr("状态数据无法解析", "State data unparsable").into();
    };
    let mode = v["shading"]["mode"].as_str().unwrap_or("-");
    let sec = v["shading"]["section"].as_str().unwrap_or("none");
    let nsec = v["sections"].as_array().map(|a| a.len()).unwrap_or(0);
    let zoom = v["orbit"]["zoom"].as_f64();
    let fov = v["cam"]["fov"].as_f64();
    pla_bridge::t!(
        "显示: {mode} / 剖面模式: {sec}({nsec}) / {} / zoom {}",
        "Shading: {mode} / Section: {sec} ({nsec}) / {} / zoom {}",
        if fov == Some(0.0) { pla_bridge::i18n::tr("正交", "Ortho") } else { pla_bridge::i18n::tr("透视", "Persp") },
        zoom.map(|z| format!("{z:.1}")).unwrap_or("-".into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_list_fav_delete_flow() {
        let doc = format!("__test_vp_{}", std::process::id());
        let stem = save_vpstate(&doc, r#"{"cam":{"px":1}}"#, None).unwrap();
        assert!(stem.starts_with("VS"));
        let list = list_vpstates(&doc);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, stem);
        set_fav(&doc, &stem, true).unwrap();
        assert!(list_vpstates(&doc)[0].fav);
        assert_eq!(load_vpstate(&doc, &stem).as_deref(), Some(r#"{"cam":{"px":1}}"#));
        rename_vpstate(&doc, &stem, "renamed").unwrap();
        assert!(load_vpstate(&doc, "renamed").is_some());
        assert_eq!(delete_vpstates(&doc, &["renamed".into()]).unwrap(), 1);
        assert!(list_vpstates(&doc).is_empty());
    }
}
