//! [PLA-000 P0-1] 快照文件管家 — 文件级快照(PRD 兜底方案, 唯一实锤可行路径)。
use std::path::{Path, PathBuf};
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
    // days since 1970 → y/m/d (简化: 只用日期序号, 文件名可读性够用)
    format!("snap-d{days}_{h:02}:{m:02}:{s:02}")
}

/// 手动快照: 找最近修改的 .plasticity 文档(常见位置), 复制到 snapshots/。
pub fn take_manual_snapshot(doc_key: &str) -> Result<String, String> {
    let src = find_latest_doc(doc_key)
        .ok_or_else(|| "未找到最近修改的 .plasticity 文档 — 请先在 Plasticity 中 Ctrl+S 保存, 并确认文档在常见位置(文档/桌面/下载)".to_string())?;
    let name = format!(
        "{}-{}",
        src.file_stem().and_then(|s| s.to_str()).unwrap_or("doc"),
        now_stamp()
    );
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
