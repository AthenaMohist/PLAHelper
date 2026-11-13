//! R8-1: 视口状态存档 — 保存/恢复摄像机、缩放、剖面、参考面、显示模式。
//!
//! 存储布局: exe 同级 `viewstates/<doc_key>/VS001.json` + `VS001.png`(截图),
//! meta.json 存收藏/创建时间(与快照墙边车同构)。状态本体是垫片 `vp.save` 的 JSON
//! (格式与 R8-1 存档兼容), 恢复时原样交给垫片 `vp.restore`。

use std::path::PathBuf;

use pla_bridge::i18n::tr;
use pla_bridge::t;

/// 旧存档位置(exe 同级 viewstates/<doc_key>) — R9 起仅作迁移来源/兜底。
fn legacy_vpstate_dir(doc_key: &str) -> PathBuf {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    exe.join("viewstates").join(super::snapshot::sanitize_filename(doc_key))
}

/// 某文档的视口状态目录 —— **跟随文档**: `<文档目录>/.pla/viewstates/<doc_key>`
/// (R9, 与选择集同策略; 定位不到文档退回 exe 同级旧位置并迁移)。
pub fn vpstate_dir(doc_key: &str) -> PathBuf {
    let legacy = legacy_vpstate_dir(doc_key);
    let dir = super::snapshot::doc_side_dir(doc_key, "viewstates").unwrap_or_else(|| legacy.clone());
    if dir != legacy {
        super::snapshot::migrate_dir(&legacy, &dir);
    }
    dir
}

/// 下一个序号名 VS001/VS002…
fn next_stem(doc_key: &str) -> String {
    let mut max = 0u32;
    if let Ok(rd) = std::fs::read_dir(vpstate_dir(doc_key)) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let stem = n
                .strip_suffix(".json")
                .or_else(|| n.strip_suffix(".png"))
                .unwrap_or("");
            if let Some(num) = stem.strip_prefix("VS") {
                if let Ok(v) = num.parse::<u32>() {
                    max = max.max(v);
                }
            }
        }
    }
    format!("VS{:03}", max + 1)
}

/// 单条视口状态元数据(列表用)。
#[derive(Debug, Clone)]
pub struct VpStateMeta {
    pub name: String, // VS001
    pub fav: bool,
    pub created: String,
}

/// 保存一条视口状态(state = CDP 采集 JSON)。返回 stem。
pub fn save_vpstate(doc_key: &str, state: &str, png: Option<&[u8]>) -> Result<String, String> {
    if state.trim().is_empty() {
        return Err(tr("视口状态为空", "Viewport state is empty").into());
    }
    let dir = vpstate_dir(doc_key);
    std::fs::create_dir_all(&dir).map_err(|e| t!("创建目录失败: {e}", "Failed to create directory: {e}"))?;
    let stem = next_stem(doc_key);
    let json = serde_json::json!({
        "doc_key": doc_key,
        "name": stem,
        "fav": false,
        "created": super::snapshot::now_local_display(),
        "state": state,
    });
    let path = dir.join(format!("{stem}.json"));
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json).map_err(|e| t!("序列化失败: {e}", "Serialization failed: {e}"))?,
    )
    .map_err(|e| t!("写文件失败: {e}", "Failed to write file: {e}"))?;
    if let Some(bytes) = png {
        let _ = std::fs::write(dir.join(format!("{stem}.png")), bytes);
    }
    Ok(stem)
}

/// 读取一条视口状态的本体 JSON。
pub fn load_vpstate(doc_key: &str, name: &str) -> Option<String> {
    let data = std::fs::read_to_string(vpstate_dir(doc_key).join(format!("{name}.json"))).ok()?;
    let v: serde_json::Value = serde_json::from_str(&data).ok()?;
    v["state"].as_str().map(String::from)
}

/// 列出全部视口状态(收藏优先)。
pub fn list_vpstates(doc_key: &str) -> Vec<VpStateMeta> {
    let mut out: Vec<VpStateMeta> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(vpstate_dir(doc_key)) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let Some(stem) = n.strip_suffix(".json") else { continue };
            let meta = std::fs::read_to_string(vpstate_dir(doc_key).join(&n))
                .ok()
                .and_then(|d| serde_json::from_str::<serde_json::Value>(&d).ok());
            if let Some(m) = meta {
                out.push(VpStateMeta {
                    name: stem.to_string(),
                    fav: m["fav"].as_bool().unwrap_or(false),
                    created: m["created"].as_str().unwrap_or("").to_string(),
                });
            }
        }
    }
    out.sort_by(|a, b| b.fav.cmp(&a.fav).then_with(|| a.name.cmp(&b.name)));
    out
}

/// 收藏/取消收藏。
pub fn set_fav(doc_key: &str, name: &str, fav: bool) -> Result<(), String> {
    let path = vpstate_dir(doc_key).join(format!("{name}.json"));
    let data = std::fs::read_to_string(&path).map_err(|e| t!("读取失败: {e}", "Read failed: {e}"))?;
    let mut v: serde_json::Value =
        serde_json::from_str(&data).map_err(|e| t!("解析失败: {e}", "Parse failed: {e}"))?;
    v["fav"] = serde_json::Value::Bool(fav);
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&v).map_err(|e| t!("序列化失败: {e}", "Serialization failed: {e}"))?,
    )
    .map_err(|e| t!("写文件失败: {e}", "Failed to write file: {e}"))
}

/// 改名。
pub fn rename_vpstate(doc_key: &str, old: &str, new: &str) -> Result<(), String> {
    let new = new.trim();
    if new.is_empty() {
        return Err(tr("新名称为空", "New name is empty").into());
    }
    if new == old {
        return Ok(());
    }
    let dir = vpstate_dir(doc_key);
    let pj = dir.join(format!("{old}.json"));
    let nj = dir.join(format!("{new}.json"));
    if nj.exists() {
        return Err(t!("「{new}」已存在", "\"{new}\" already exists"));
    }
    let data = std::fs::read_to_string(&pj).map_err(|e| t!("读取失败: {e}", "Read failed: {e}"))?;
    let mut v: serde_json::Value =
        serde_json::from_str(&data).map_err(|e| t!("解析失败: {e}", "Parse failed: {e}"))?;
    v["name"] = serde_json::Value::String(new.to_string());
    std::fs::write(&nj, serde_json::to_vec_pretty(&v).unwrap_or_default())
        .map_err(|e| t!("写文件失败: {e}", "Failed to write file: {e}"))?;
    std::fs::remove_file(&pj).map_err(|e| t!("移除旧文件失败: {e}", "Failed to remove old file: {e}"))?;
    let pp = dir.join(format!("{old}.png"));
    if pp.exists() {
        let np = dir.join(format!("{new}.png"));
        std::fs::rename(&pp, &np).map_err(|e| t!("改名截图失败: {e}", "Failed to rename screenshot: {e}"))?;
    }
    Ok(())
}

/// 删除(含截图)。支持批量, 返回删除个数。
pub fn delete_vpstates(doc_key: &str, names: &[String]) -> Result<usize, String> {
    let dir = vpstate_dir(doc_key);
    let mut n = 0;
    for name in names {
        let mut ok = true;
        for f in [format!("{name}.json"), format!("{name}.png")] {
            if let Err(e) = std::fs::remove_file(dir.join(&f)) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    ok = false;
                }
            }
        }
        if ok {
            n += 1;
        }
    }
    Ok(n)
}

/// 打开目录(explorer)。
pub fn open_dir(doc_key: &str) -> Result<(), String> {
    let dir = vpstate_dir(doc_key);
    std::fs::create_dir_all(&dir).map_err(|e| t!("创建目录失败: {e}", "Failed to create directory: {e}"))?;
    std::process::Command::new("explorer")
        .arg(dir.as_os_str())
        .spawn()
        .map(|_| ())
        .map_err(|e| t!("打开失败: {e}", "Failed to open: {e}"))
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
        assert!(!list[0].fav);
        set_fav(&doc, &stem, true).unwrap();
        assert!(list_vpstates(&doc)[0].fav);
        assert_eq!(
            load_vpstate(&doc, &stem).as_deref(),
            Some(r#"{"cam":{"px":1}}"#)
        );
        rename_vpstate(&doc, &stem, "renamed").unwrap();
        assert!(load_vpstate(&doc, "renamed").is_some());
        assert_eq!(delete_vpstates(&doc, &["renamed".into()]).unwrap(), 1);
        assert!(list_vpstates(&doc).is_empty());
    }
}
