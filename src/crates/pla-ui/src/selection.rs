//! 选择集本地存档 — 保存为 exe 同级 `selections/<doc_key>/<名称>.json`。
//!
//! 「应用」需要 PUT_SOME 组创建语义(PLA-004 G4), 当前仅本地保存(诚实提示)。
use std::path::PathBuf;

use pla_bridge::i18n::tr;
use pla_bridge::t;

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
        return Err(tr("选择集名称为空 — 请先在名称框输入", "Selection set name is empty — enter a name first").into());
    }
    if members.is_empty() {
        return Err(tr("选择集没有勾选任何成员", "No members checked in the selection set").into());
    }
    let json = serde_json::json!({
        "doc_key": doc_key,
        "name": name,
        "members": members,
    });
    let path = selections_dir(doc_key).join(format!("{}.json", sanitize(name)));
    std::fs::write(&path, serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?)
        .map_err(|e| t!("写入失败: {e}", "Write failed: {e}"))?;
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdp_sel_fav_rename_delete_flow() {
        let doc = format!("__test_{}", std::process::id());
        let mem = || vec![("solid".to_string(), "1".to_string(), "n".to_string(), None)];
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

/// CDP 选择集成员(R7-14): kind/id/name 三元组 + R7-21 原始 id 元素 key。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CdpMember {
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// 快照采集的原始 id 元素 JSON(对象形 {id,versionId,...}); None = 纯数值 id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// v6(R7-31l): 保存时刻的相机(整集共享, 存在每个成员上冗余但兼容);
    /// None = 旧版选择集无相机。内容为 CAM_SNAPSHOT_JS 的 JSON 字符串。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<String>,
}

/// v6: 带相机保存 — members 同上, camera 为 CAM_SNAPSHOT_JS 输出(None=不存)。
/// R8-1: 追加保存截图(视口裁剪 PNG 字节, 落盘 <名>.png)。旧集无截图照常。
pub fn save_cdp_selection_with_camera(
    doc_key: &str,
    name: &str,
    members: &[(String, String, String, Option<String>, Option<String>)],
    camera: Option<&str>,
    png: Option<&[u8]>,
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(tr("选择集名称为空 — 请先在名称框输入", "Selection set name is empty — enter a name first").into());
    }
    if members.is_empty() {
        return Err(tr("实时选择勾选为空", "Nothing checked in the live selection").into());
    }
    let dir = cdp_selections_dir(doc_key);
    std::fs::create_dir_all(&dir).map_err(|e| t!("创建目录失败: {e}", "Failed to create directory: {e}"))?;
    let items: Vec<CdpMember> = members
        .iter()
        .map(|(k, i, n, key, member_cam)| CdpMember {
            kind: k.clone(),
            id: i.clone(),
            name: n.clone(),
            key: key.clone(),
            // 每成员相机优先(多视角分组); 缺失回落整集相机(v6 首版行为)。
            camera: member_cam.clone().or_else(|| camera.map(|c| c.to_string())),
        })
        .collect();
    let path = dir.join(format!("{}.json", sanitize(name)));
    // R35: 覆盖已有的(更新按钮)时保留收藏
    let fav = cdp_sel_meta(doc_key, name).map_or(false, |m| m.fav);
    let json = serde_json::json!({
        "doc_key": doc_key,
        "name": name,
        "fav": fav,
        "members": items,
    });
    std::fs::write(&path, serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?)
        .map_err(|e| t!("写文件失败: {e}", "Failed to write file: {e}"))?;
    // R8-1: 截图覆盖保存(有则写, 无则删旧 — 保持与集内容一致)
    let png_path = dir.join(format!("{}.png", sanitize(name)));
    if let Some(bytes) = png {
        let _ = std::fs::write(&png_path, bytes);
    }
    Ok(path.display().to_string())
}

/// R8-1: 同上但不落截图(兼容旧调用)。
pub fn save_cdp_selection_only(
    doc_key: &str,
    name: &str,
    members: &[(String, String, String, Option<String>, Option<String>)],
    camera: Option<&str>,
) -> Result<String, String> {
    save_cdp_selection_with_camera(doc_key, name, members, camera, None)
}

/// R8-1: 选择集下一个自动名 SG001/SG002…(点保存按钮时用)。
pub fn next_sg_name(doc_key: &str) -> String {
    let dir = cdp_selections_dir(doc_key);
    let mut max = 0u32;
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let stem = n
                .strip_suffix(".json")
                .or_else(|| n.strip_suffix(".png"))
                .unwrap_or("");
            if let Some(num) = stem.strip_prefix("SG") {
                if let Ok(v) = num.parse::<u32>() {
                    max = max.max(v);
                }
            }
        }
    }
    format!("SG{:03}", max + 1)
}

/// 旧存档位置(exe 同级 selections-cdp/<doc_key>) — R9 起仅作迁移来源/兜底。
fn legacy_selections_dir(doc_key: &str) -> PathBuf {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    exe.join("selections-cdp").join(sanitize(doc_key))
}

/// R8-1 / R9: 选择集存档目录 —— **跟随文档**: `<文档目录>/.pla/selections/<doc_key>`。
/// 定位不到文档(未绑定/未保存)才退回 exe 同级旧位置。
/// **唯一入口**: 保存 / 列表 / 读取 / 截图 / 收藏 / 改名 / 删除全部经它取目录
/// (R23 前保存与列表还写 exe 同级、截图却从文档同级读 → 面板一律"无截图")。
pub fn cdp_selections_dir(doc_key: &str) -> PathBuf {
    let legacy = legacy_selections_dir(doc_key);
    let dir = crate::snapshot::doc_side_dir(doc_key, "selections").unwrap_or_else(|| legacy.clone());
    if dir != legacy {
        merge_legacy_once(&legacy, &dir);
    }
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// 把旧位置里新目录没有(或比新目录新)的文件补过去; 每个进程每对目录只做一次(本函数每帧都会被调用)。
/// 只复制不删除旧文件。
fn merge_legacy_once(legacy: &std::path::Path, dir: &std::path::Path) {
    thread_local! {
        static DONE: std::cell::RefCell<std::collections::HashSet<PathBuf>> = Default::default();
    }
    let first = DONE.with(|d| d.borrow_mut().insert(dir.to_path_buf()));
    if !first || !legacy.is_dir() {
        return;
    }
    let _ = std::fs::create_dir_all(dir);
    let Ok(rd) = std::fs::read_dir(legacy) else { return };
    for e in rd.flatten() {
        let src = e.path();
        if !src.is_file() {
            continue;
        }
        let dst = dir.join(e.file_name());
        let newer = |a: &std::path::Path, b: &std::path::Path| -> bool {
            match (a.metadata().and_then(|m| m.modified()), b.metadata().and_then(|m| m.modified())) {
                (Ok(x), Ok(y)) => x > y,
                _ => false,
            }
        };
        if !dst.exists() || newer(&src, &dst) {
            let _ = std::fs::copy(&src, &dst);
        }
    }
}

/// R8-1: 读选择集截图 PNG 字节。
pub fn load_cdp_selection_png(doc_key: &str, name: &str) -> Option<Vec<u8>> {
    std::fs::read(cdp_selections_dir(doc_key).join(format!("{}.png", sanitize(name)))).ok()
}

/// R8-1: 打开选择集目录。
pub fn open_cdp_selections_dir(doc_key: &str) -> Result<(), String> {
    let dir = cdp_selections_dir(doc_key);
    std::process::Command::new("explorer")
        .arg(dir.as_os_str())
        .spawn()
        .map(|_| ())
        .map_err(|e| t!("打开失败: {e}", "Failed to open: {e}"))
}

/// R8-1: 批量删除选择集(含截图)。返回删除数。
pub fn delete_cdp_selections_batch(doc_key: &str, names: &[String]) -> Result<usize, String> {
    let dir = cdp_selections_dir(doc_key);
    let mut n = 0;
    for name in names {
        let mut ok = true;
        for f in [format!("{}.json", sanitize(name)), format!("{}.png", sanitize(name))] {
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

/// 保存 CDP 选择集(R7-14): exe 同级 `selections-cdp/<doc_key>/<名称>.json`。
/// 成员为结构化四元组, 与桥 id 空间隔离; 应用走 CDP ApplySel 按类型回放。
pub fn save_cdp_selection(
    doc_key: &str,
    name: &str,
    members: &[(String, String, String, Option<String>)],
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(tr("选择集名称为空 — 请先在名称框输入", "Selection set name is empty — enter a name first").into());
    }
    if members.is_empty() {
        return Err(tr("实时选择勾选为空", "Nothing checked in the live selection").into());
    }
    let dir = cdp_selections_dir(doc_key);
    std::fs::create_dir_all(&dir).map_err(|e| t!("创建目录失败: {e}", "Failed to create directory: {e}"))?;
    let items: Vec<CdpMember> = members
        .iter()
        .map(|(k, i, n, key)| CdpMember {
            kind: k.clone(),
            id: i.clone(),
            name: n.clone(),
            key: key.clone(),
            camera: None,
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
        .map_err(|e| t!("写文件失败: {e}", "Failed to write file: {e}"))?;
    Ok(path.display().to_string())
}

/// CDP 选择集元数据(列表行用): 名 + 收藏 + 成员数。收藏优先排序。
#[derive(Debug, Clone)]
pub struct CdpSelMeta {
    pub name: String,
    pub fav: bool,
    pub n_members: usize,
    /// R8-1: 保存时间(存档 mtime, 悬浮信息用; 无 = "")。
    pub created: String,
}

fn cdp_sel_path(doc_key: &str, name: &str) -> PathBuf {
    cdp_selections_dir(doc_key).join(format!("{}.json", sanitize(name)))
}

/// 读取单个选择集元数据(无文件→None; fav 缺省 false, 旧文件兼容)。
pub fn cdp_sel_meta(doc_key: &str, name: &str) -> Option<CdpSelMeta> {
    let path = cdp_sel_path(doc_key, name);
    let data = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&data).ok()?;
    // R8-1: mtime → 简单本地时间串(Windows 文件时间 → 秒, 手写换算避免引 chrono;
    // 显示用途, 精度到分钟即可)
    let created = std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            let secs = t
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                + 8 * 3600;
            let days = (secs / 86400) as i64;
            // Howard Hinnant civil_from_days(同 snapshot.rs, 简版)
            let z = days + 719_468;
            let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
            let doe = (z - era * 146_097) as u64;
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let y = yoe as i64 + era * 400;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
            let mth = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
            let yy = if mth <= 2 { y + 1 } else { y };
            let rem = secs % 86400;
            format!(
                "{yy:04}-{mth:02}-{d:02} {:02}:{:02}",
                rem / 3600,
                (rem % 3600) / 60
            )
        })
        .unwrap_or_default();
    Some(CdpSelMeta {
        name: name.to_string(),
        fav: v["fav"].as_bool().unwrap_or(false),
        n_members: v["members"].as_array().map(|a| a.len()).unwrap_or(0),
        created,
    })
}

/// 写回单个选择集的 fav 标记(其余字段原样保留)。
pub fn set_cdp_selection_fav(doc_key: &str, name: &str, fav: bool) -> Result<(), String> {
    let path = cdp_sel_path(doc_key, name);
    let data = std::fs::read_to_string(&path).map_err(|e| t!("读取失败: {e}", "Read failed: {e}"))?;
    let mut v: serde_json::Value =
        serde_json::from_str(&data).map_err(|e| t!("解析失败: {e}", "Parse failed: {e}"))?;
    v["fav"] = serde_json::Value::Bool(fav);
    std::fs::write(&path, serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?)
        .map_err(|e| t!("写文件失败: {e}", "Failed to write file: {e}"))
}

/// 删除选择集文件。
pub fn delete_cdp_selection(doc_key: &str, name: &str) -> Result<(), String> {
    let path = cdp_sel_path(doc_key, name);
    std::fs::remove_file(&path).map_err(|e| t!("删除失败: {e}", "Delete failed: {e}"))
}

/// 改名: 校验新名 → 更新 json 内 name 字段 → 移动文件。
pub fn rename_cdp_selection(doc_key: &str, old: &str, new: &str) -> Result<(), String> {
    let new = new.trim();
    if new.is_empty() {
        return Err(tr("新名称为空", "New name is empty").into());
    }
    if new == old {
        return Ok(());
    }
    let old_path = cdp_sel_path(doc_key, old);
    let new_path = cdp_sel_path(doc_key, new);
    if new_path.exists() {
        return Err(t!("「{new}」已存在", "\"{new}\" already exists"));
    }
    let data = std::fs::read_to_string(&old_path).map_err(|e| t!("读取失败: {e}", "Read failed: {e}"))?;
    let mut v: serde_json::Value =
        serde_json::from_str(&data).map_err(|e| t!("解析失败: {e}", "Parse failed: {e}"))?;
    v["name"] = serde_json::Value::String(new.to_string());
    std::fs::write(&new_path, serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?)
        .map_err(|e| t!("写文件失败: {e}", "Failed to write file: {e}"))?;
    std::fs::remove_file(&old_path).map_err(|e| t!("移除旧文件失败: {e}", "Failed to remove old file: {e}"))?;
    // R8-1: 截图跟随改名
    let old_png = old_path.with_extension("png");
    let new_png = new_path.with_extension("png");
    if old_png.exists() {
        let _ = std::fs::rename(&old_png, &new_png);
    }
    Ok(())
}

/// 列出已保存的 CDP 选择集(R7-16)。
pub fn list_cdp_selections(doc_key: &str) -> Vec<String> {
    let dir = cdp_selections_dir(doc_key);
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
    let path = cdp_sel_path(doc_key, name);
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
                        key: m.get("key").and_then(|k| k.as_str()).map(String::from),
                        camera: m.get("camera").and_then(|c| c.as_str()).map(String::from),
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// 勾选区成员 key("kind:id")。
pub fn member_key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}
