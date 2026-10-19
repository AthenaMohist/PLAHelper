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
        "members": items,
    });
    let path = dir.join(format!("{}.json", sanitize(name)));
    std::fs::write(&path, serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?)
        .map_err(|e| format!("写文件失败: {e}"))?;
    Ok(path.display().to_string())
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
