//! 选择集本地存档 — 保存为 exe 同级 `selections/<doc_key>/<名称>.json`。
//!
//! 「应用」需要 PUT_SOME 组创建语义(PLA-004 G4), 当前仅本地保存(诚实提示)。
use std::path::PathBuf;

/// 选择集存档目录(按 doc_key 归档)。
pub fn selections_dir(doc_key: &str) -> PathBuf {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = exe.join("selections").join(sanitize(doc_key));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Windows 文件名非法字符 → '_'。
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// 保存选择集(成员 id 列表)。返回完整文件路径或错误文本。
pub fn save_selection(doc_key: &str, name: &str, members: &[String]) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("选择集名称为空 — 请先在名称框输入".into());
    }
    if members.is_empty() {
        return Err("选择集没有勾选任何成员".into());
    }
    let json = serde_json::json!({
        "doc_key": doc_key,
        "name": name,
        "members": members,
    });
    let path = selections_dir(doc_key).join(format!("{}.json", sanitize(name)));
    std::fs::write(&path, serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?)
        .map_err(|e| format!("写入失败: {e}"))?;
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdp_sel_fav_rename_delete_flow() {
        let doc = format!("__test_{}", std::process::id());
        let mem = || vec![("solid".to_string(), "1".to_string(), "n".to_string())];
        save_cdp_selection(&doc, "meta_a", &mem()).unwrap();
        // 旧文件/缺省: fav=false
        let m = cdp_sel_meta(&doc, "meta_a").unwrap();
        assert!(!m.fav && m.n_members == 1);
        set_cdp_selection_fav(&doc, "meta_a", true).unwrap();
        assert!(cdp_sel_meta(&doc, "meta_a").unwrap().fav);
        // 改名保留 fav 与成员, 旧名消失
        rename_cdp_selection(&doc, "meta_a", "meta_b").unwrap();
        let m = cdp_sel_meta(&doc, "meta_b").unwrap();
        assert!(m.fav && m.n_members == 1 && m.name == "meta_b");
        assert!(cdp_sel_meta(&doc, "meta_a").is_none());
        // 重名拒绝
        save_cdp_selection(&doc, "meta_c", &mem()).unwrap();
        assert!(rename_cdp_selection(&doc, "meta_b", "meta_c").is_err());
        // 列表: 收藏优先
        let list = list_cdp_selections_meta(&doc);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "meta_b");
        // 删除
        delete_cdp_selection(&doc, "meta_b").unwrap();
        delete_cdp_selection(&doc, "meta_c").unwrap();
        assert!(list_cdp_selections_meta(&doc).is_empty());
    }

    #[test]
    fn sanitize_strips_illegal_chars() {
        assert_eq!(sanitize("a/b\\c:d*e?f\"g<h>i|j"), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanitize("普通名称-1"), "普通名称-1");
    }

    #[test]
    fn save_rejects_empty_name_and_members() {
        let err = save_selection("model1", "  ", &["x".into()]).unwrap_err();
        assert!(err.contains("名称为空"));
        let err = save_selection("model1", "sel", &[]).unwrap_err();
        assert!(err.contains("没有勾选"));
    }

    #[test]
    fn save_writes_json_file() {
        let doc = format!("__test_{}", std::process::id());
        let path = save_selection(&doc, "test_sel", &["1".into(), "2".into()]).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("test_sel"));
        assert!(content.contains("\"members\""));
        std::fs::remove_file(&path).ok();
    }
}

/// CDP 选择集成员(R7-14): kind/id/name 三元组。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CdpMember {
    pub kind: String,
    pub id: String,
    pub name: String,
}

/// 保存 CDP 选择集(R7-14): exe 同级 `selections-cdp/<doc_key>/<名称>.json`。
/// 成员为结构化三元组, 与桥 id 空间隔离; 应用走 CDP `Selection.addById`。
pub fn save_cdp_selection(
    doc_key: &str,
    name: &str,
    members: &[(String, String, String)],
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("选择集名称为空 — 请先在名称框输入".into());
    }
    if members.is_empty() {
        return Err("实时选择勾选为空".into());
    }
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = exe.join("selections-cdp").join(sanitize(doc_key));
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败: {e}"))?;
    let items: Vec<CdpMember> = members
        .iter()
        .map(|(k, i, n)| CdpMember {
            kind: k.clone(),
            id: i.clone(),
            name: n.clone(),
        })
        .collect();
    let json = serde_json::json!({
        "doc_key": doc_key,
        "name": name,
        "fav": false,
        "members": items,
    });
    let path = dir.join(format!("{}.json", sanitize(name)));
    std::fs::write(&path, serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?)
        .map_err(|e| format!("写文件失败: {e}"))?;
    Ok(path.display().to_string())
}

/// CDP 选择集元数据(列表行用): 名 + 收藏 + 成员数。收藏优先排序。
#[derive(Debug, Clone)]
pub struct CdpSelMeta {
    pub name: String,
    pub fav: bool,
    pub n_members: usize,
}

fn cdp_sel_path(doc_key: &str, name: &str) -> PathBuf {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    exe.join("selections-cdp")
        .join(sanitize(doc_key))
        .join(format!("{}.json", sanitize(name)))
}

/// 读取单个选择集元数据(无文件→None; fav 缺省 false, 旧文件兼容)。
pub fn cdp_sel_meta(doc_key: &str, name: &str) -> Option<CdpSelMeta> {
    let data = std::fs::read_to_string(cdp_sel_path(doc_key, name)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&data).ok()?;
    Some(CdpSelMeta {
        name: name.to_string(),
        fav: v["fav"].as_bool().unwrap_or(false),
        n_members: v["members"].as_array().map(|a| a.len()).unwrap_or(0),
    })
}

/// 写回单个选择集的 fav 标记(其余字段原样保留)。
pub fn set_cdp_selection_fav(doc_key: &str, name: &str, fav: bool) -> Result<(), String> {
    let path = cdp_sel_path(doc_key, name);
    let data = std::fs::read_to_string(&path).map_err(|e| format!("读取失败: {e}"))?;
    let mut v: serde_json::Value =
        serde_json::from_str(&data).map_err(|e| format!("解析失败: {e}"))?;
    v["fav"] = serde_json::Value::Bool(fav);
    std::fs::write(&path, serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?)
        .map_err(|e| format!("写文件失败: {e}"))
}

/// 删除选择集文件。
pub fn delete_cdp_selection(doc_key: &str, name: &str) -> Result<(), String> {
    let path = cdp_sel_path(doc_key, name);
    std::fs::remove_file(&path).map_err(|e| format!("删除失败: {e}"))
}

/// 改名: 校验新名 → 更新 json 内 name 字段 → 移动文件。
pub fn rename_cdp_selection(doc_key: &str, old: &str, new: &str) -> Result<(), String> {
    let new = new.trim();
    if new.is_empty() {
        return Err("新名称为空".into());
    }
    if new == old {
        return Ok(());
    }
    let old_path = cdp_sel_path(doc_key, old);
    let new_path = cdp_sel_path(doc_key, new);
    if new_path.exists() {
        return Err(format!("「{new}」已存在"));
    }
    let data = std::fs::read_to_string(&old_path).map_err(|e| format!("读取失败: {e}"))?;
    let mut v: serde_json::Value =
        serde_json::from_str(&data).map_err(|e| format!("解析失败: {e}"))?;
    v["name"] = serde_json::Value::String(new.to_string());
    std::fs::write(&new_path, serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?)
        .map_err(|e| format!("写文件失败: {e}"))?;
    std::fs::remove_file(&old_path).map_err(|e| format!("移除旧文件失败: {e}"))
}

/// 列出已保存的 CDP 选择集(R7-16)。
pub fn list_cdp_selections(doc_key: &str) -> Vec<String> {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = exe.join("selections-cdp").join(sanitize(doc_key));
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if e.path().extension().map(|x| x == "json").unwrap_or(false) {
                if let Some(n) = e.file_name().to_str() {
                    out.push(n.trim_end_matches(".json").to_string());
                }
            }
        }
    }
    out.sort();
    out
}

/// 列出已保存的 CDP 选择集(带元数据, 收藏优先, 其余按名排序)。
pub fn list_cdp_selections_meta(doc_key: &str) -> Vec<CdpSelMeta> {
    let mut out: Vec<CdpSelMeta> = list_cdp_selections(doc_key)
        .iter()
        .filter_map(|n| cdp_sel_meta(doc_key, n))
        .collect();
    out.sort_by(|a, b| b.fav.cmp(&a.fav).then_with(|| a.name.cmp(&b.name)));
    out
}

/// 读取 CDP 选择集成员。
pub fn load_cdp_selection(doc_key: &str, name: &str) -> Option<Vec<CdpMember>> {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let path = exe
        .join("selections-cdp")
        .join(sanitize(doc_key))
        .join(format!("{}.json", sanitize(name)));
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok().map(|v: serde_json::Value| {
        v["members"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|m| CdpMember {
                        kind: m["kind"].as_str().unwrap_or("").to_string(),
                        id: m["id"].as_str().unwrap_or("").to_string(),
                        name: m["name"].as_str().unwrap_or("").to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}
