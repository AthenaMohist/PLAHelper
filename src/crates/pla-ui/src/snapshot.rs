//! [PLA-000 P0-1] 快照文件管家 — 文件级快照(PRD 兜底方案, 唯一实锤可行路径)。
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn snapshot_dir() -> PathBuf {
    let exe = std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = exe.join("snapshots");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn now_stamp() -> String {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    // 简易 UTC+8 时间戳格式化(不引 chrono)
    let days = t / 86400;
    let secs = t % 86400;
    let (h, m, s) = (secs / 3600 + 8, (secs % 3600) / 60, secs % 60);
    // 注意: 分隔符必须用 '-', 不能用 ':' — Windows 文件名非法字符(os error 123)。
    format!("snap-d{days}_{h:02}-{m:02}-{s:02}")
}

/// Windows 文件名净化: 非法字符 `\/:*?"<>|` → '-', 去首尾空白与结尾点,
/// 空结果退回 "snapshot"。(QA Bug A: 冒号落盘 os error 123)
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').to_string();
    if trimmed.is_empty() {
        "snapshot".into()
    } else {
        trimmed
    }
}

/// 手动快照: 找最近修改的 .plasticity 文档(常见位置), 复制到 snapshots/。
pub fn take_manual_snapshot(doc_key: &str) -> Result<String, String> {
    let src = find_latest_doc(doc_key)
        .ok_or_else(|| "未找到最近修改的 .plasticity 文档 — 请先在 Plasticity 中 Ctrl+S 保存, 并确认文档在常见位置(文档/桌面/下载)".to_string())?;
    // 文件名过净化函数: doc 来源 stem + 时间戳均可能含非法字符。
    let name = sanitize_filename(&format!(
        "{}-{}",
        src.file_stem().and_then(|s| s.to_str()).unwrap_or("doc"),
        now_stamp()
    ));
    let dst = snapshot_dir().join(format!("{name}.plasticity"));
    std::fs::copy(&src, &dst).map_err(|e| format!("复制失败: {e}"))?;
    Ok(name)
}

/// 打开快照文件夹(explorer)。
pub fn open_snapshot_dir() -> Result<(), String> {
    let dir = snapshot_dir();
    std::process::Command::new("explorer")
        .arg(dir.as_os_str())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开失败: {e}"))
}

/// 启动时列出现有快照(原 TODO G2-2): exe 同级 snapshots/*.plasticity, 文件名新→旧。
pub fn list_snapshots() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(snapshot_dir())
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|x| x == "plasticity")
                        .unwrap_or(false)
                })
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.reverse();
    names
}

fn find_latest_doc(doc_key: &str) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    let roots = [
        dirs_home().join("Documents"),
        dirs_home().join("Desktop"),
        dirs_home().join("Downloads"),
        dirs_home(),
    ];
    for root in roots {
        let Ok(rd) = std::fs::read_dir(&root) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let matches = name.ends_with(".plasticity")
                && (doc_key.is_empty() || name.contains(doc_key.trim_end_matches(".plasticity")));
            if !matches { continue; }
            if let Ok(meta) = p.metadata() {
                let mt = meta.modified().unwrap_or(UNIX_EPOCH);
                if best.as_ref().map(|(t, _)| mt > *t).unwrap_or(true) {
                    best = Some((mt, p));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

fn dirs_home() -> PathBuf {
    std::env::var("USERPROFILE").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("C:\\Users"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 净化函数: 非法字符全替换, 边界输入兜底(返工单要求)。
    #[test]
    fn sanitize_filename_replaces_illegal_chars() {
        assert_eq!(sanitize_filename("12:30:45"), "12-30-45");
        assert_eq!(sanitize_filename("a/b\\c:d*e?f\"g<h>i|j"), "a-b-c-d-e-f-g-h-i-j");
        assert_eq!(sanitize_filename("普通:名称"), "普通-名称");
        assert_eq!(sanitize_filename("  "), "snapshot");
        assert_eq!(sanitize_filename("..."), "snapshot");
        assert_eq!(sanitize_filename("end."), "end");
        // 组合名(文档 stem + 时间戳)整体净化后可直接落盘。
        let name = sanitize_filename(&format!("my{}doc-{}", ":", now_stamp()));
        let path = std::env::temp_dir().join(format!("{name}.plasticity"));
        std::fs::write(&path, b"probe").expect("净化后文件名仍无法落盘");
        std::fs::remove_file(&path).ok();
    }

    /// 回归(QA Bug A): 快照文件名不得含 Windows 非法字符, 且能实际落盘。
    #[test]
    fn snapshot_filename_legal_and_writable_on_windows() {
        let name = format!("test_doc-{}", now_stamp());
        assert!(!name.contains(':'), "文件名含非法字符 ':' : {name}");
        assert!(!name.contains(['\\', '/', '*', '?', '"', '<', '>', '|']));
        let path = std::env::temp_dir().join(format!("{name}.plasticity"));
        std::fs::write(&path, b"probe").expect("快照文件名无法落盘");
        assert!(path.exists());
        std::fs::remove_file(&path).ok();
    }
}
