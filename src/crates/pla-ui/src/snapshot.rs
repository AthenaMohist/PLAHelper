//! [PLA-000 P0-1] 快照文件管家 — 文件级快照(PRD 兜底方案, 唯一实锤可行路径)。
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// 快照目录策略(R7-4): 跟随文档 — 文档同级的 snapshot/ 文件夹, 每个文档的快照各归其位。
/// doc 为 None(尚未定位到文档)时退回 exe 同级 snapshots/(旧行为兜底)。
pub fn snapshot_dir_for(doc: Option<&PathBuf>) -> PathBuf {
    match doc {
        Some(d) => d
            .parent()
            .map(|p| p.join("snapshot"))
            .unwrap_or_else(fallback_dir),
        None => fallback_dir(),
    }
}

fn fallback_dir() -> PathBuf {
    let exe = std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    exe.join("snapshots")
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

/// 手动快照: 定位当前文档(最近修改的 .plasticity), 复制到文档同级 snapshot/ 文件夹,
/// 并截取 Plasticity 主窗口画面存为同名 .png(快照墙缩略图, 尽力而为, 失败不阻断)。
/// 文档定位三路(R7-3, 本机实测 D 盘项目目录的文档只有 Recent 能覆盖):
///   1. Windows Recent *.plasticity.lnk 解析(IShellLink COM) — 覆盖任意保存位置, 最可靠;
///   2. 已知文件夹(SHGetKnownFolderPath, 处理下载重定向到 D:\下载 之类)递归扫深度 2;
///   3. USERPROFILE 下的 Documents/Desktop/Downloads 兜底。
/// doc_key 是软过滤: 命中则取命中集合里最新的, 全体为空时回退全局最新(不再因硬过滤报"找不到")。
pub fn take_manual_snapshot(doc_key: &str, plasticity_hwnd: Option<isize>) -> Result<String, String> {
    let src = find_latest_doc(doc_key).ok_or_else(|| {
        format!(
            "未找到最近修改的 .plasticity 文档 — 已扫 Windows Recent 记录 + 已知文件夹(文档/桌面/下载), \
             关键字 '{doc_key}' 无命中且无任何 .plasticity 候选。请先在 Plasticity 中 Ctrl+S 保存一次。"
        )
    })?;
    // 文件名过净化函数: doc 来源 stem + 时间戳均可能含非法字符。
    let name = sanitize_filename(&format!(
        "{}-{}",
        src.file_stem().and_then(|s| s.to_str()).unwrap_or("doc"),
        now_stamp()
    ));
    let dir = snapshot_dir_for(Some(&src));
    let _ = std::fs::create_dir_all(&dir);
    let dst = dir.join(format!("{name}.plasticity"));
    std::fs::copy(&src, &dst).map_err(|e| format!("复制失败: {e}"))?;
    // 缩略图: PrintWindow 抓 Plasticity 主窗口 → 同名 .png
    if let Some(hwnd) = plasticity_hwnd {
        let png = dir.join(format!("{name}.png"));
        if let Err(e) = crate::wincap::capture_window_png(hwnd, &png) {
            eprintln!("[snapshot] 窗口截图失败(不阻断快照): {e}");
        }
    }
    Ok(name)
}

/// 当前文档对应的快照目录(不创建)。
pub fn snapshot_dir_of(doc_key: &str) -> PathBuf {
    snapshot_dir_for(find_latest_doc(doc_key).as_ref())
}

// ---------------- 收藏/改名/删除(R7-6) ----------------
/// 收藏状态持久化: 快照目录下 meta.json — {快照文件名(含扩展名): true}。
/// 轻量边车文件, 不动快照本体; 改名/删除时同步迁移。

fn meta_path(dir: &PathBuf) -> PathBuf {
    dir.join("meta.json")
}

fn load_meta(doc_key: &str) -> std::collections::HashMap<String, bool> {
    let p = meta_path(&snapshot_dir_of(doc_key));
    std::fs::read(&p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_meta(doc_key: &str, meta: &std::collections::HashMap<String, bool>) -> Result<(), String> {
    let dir = snapshot_dir_of(doc_key);
    let _ = std::fs::create_dir_all(&dir);
    let bytes = serde_json::to_vec_pretty(meta).map_err(|e| format!("序列化 meta: {e}"))?;
    std::fs::write(meta_path(&dir), bytes).map_err(|e| format!("写 meta: {e}"))
}

/// 收藏的快照名集合。
pub fn load_favs(doc_key: &str) -> std::collections::HashSet<String> {
    load_meta(doc_key)
        .into_iter()
        .filter(|(_, v)| *v)
        .map(|(k, _)| k)
        .collect()
}

/// 设置/取消收藏。
pub fn set_fav(doc_key: &str, name: &str, fav: bool) -> Result<(), String> {
    let mut meta = load_meta(doc_key);
    meta.insert(name.to_string(), fav);
    save_meta(doc_key, &meta)
}

/// 批量删除快照(.plasticity + 同名 .png + meta 条目)。返回实际删除个数。
pub fn delete_snapshots(doc_key: &str, names: &[String]) -> Result<usize, String> {
    let dir = snapshot_dir_of(doc_key);
    let mut removed = 0usize;
    let mut errs = Vec::new();
    for n in names {
        let stem = n.trim_end_matches(".plasticity");
        let mut ok = true;
        for f in [format!("{n}"), format!("{stem}.png")] {
            let p = dir.join(sanitize_filename(&f));
            match std::fs::remove_file(&p) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    ok = false;
                    errs.push(format!("{}: {e}", p.display()));
                }
            }
        }
        if ok {
            removed += 1;
        }
    }
    let mut meta = load_meta(doc_key);
    let before = meta.len();
    meta.retain(|k, _| !names.iter().any(|n| n == k));
    if meta.len() != before {
        save_meta(doc_key, &meta)?;
    }
    if !errs.is_empty() {
        return Err(format!("部分删除失败: {}", errs.join("; ")));
    }
    Ok(removed)
}

/// 快照改名(同步 .plasticity/.png/meta 键)。new_stem 会过文件名净化。
pub fn rename_snapshot(doc_key: &str, old_name: &str, new_stem: &str) -> Result<String, String> {
    let old_stem = old_name.trim_end_matches(".plasticity");
    let new_stem = sanitize_filename(new_stem);
    if new_stem.is_empty() {
        return Err("新名称无效".into());
    }
    let dir = snapshot_dir_of(doc_key);
    for (a, b) in [
        (format!("{old_stem}.plasticity"), format!("{new_stem}.plasticity")),
        (format!("{old_stem}.png"), format!("{new_stem}.png")),
    ] {
        let pa = dir.join(sanitize_filename(&a));
        let pb = dir.join(sanitize_filename(&b));
        if pa.exists() {
            if pb.exists() {
                return Err(format!("目标已存在: {}", pb.display()));
            }
            std::fs::rename(&pa, &pb).map_err(|e| format!("重命名 {} 失败: {e}", pa.display()))?;
        }
    }
    let mut meta = load_meta(doc_key);
    if let Some(v) = meta.remove(old_name) {
        meta.insert(format!("{new_stem}.plasticity"), v);
        save_meta(doc_key, &meta)?;
    }
    Ok(format!("{new_stem}.plasticity"))
}

/// 快照墙点击 → 用 Plasticity 打开快照文件(新进程 → 新窗口)。
pub fn open_in_plasticity(snapshot_name: &str, doc_key: &str) -> Result<(), String> {
    let dir = snapshot_dir_of(doc_key);
    let path = dir.join(sanitize_filename(snapshot_name));
    if !path.is_file() {
        return Err(format!("快照文件不存在: {}", path.display()));
    }
    let exe = crate::window_finder::plasticity_exe_path()
        .ok_or_else(|| "未找到 Plasticity 进程 — 请先打开 Plasticity".to_string())?;
    std::process::Command::new(&exe)
        .arg(&path)
        .spawn()
        .map_err(|e| format!("启动 Plasticity 失败: {e}"))?;
    Ok(())
}

/// 打开当前文档的快照文件夹(explorer); 目录不存在则先创建。
pub fn open_snapshot_dir(doc_key: &str) -> Result<(), String> {
    let dir = snapshot_dir_for(find_latest_doc(doc_key).as_ref());
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建快照目录失败: {e}"))?;
    std::process::Command::new("explorer")
        .arg(dir.as_os_str())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开失败: {e}"))
}

/// 列出当前文档的现有快照(原 TODO G2-2): 文档同级 snapshot/*.plasticity, 文件名新→旧。
pub fn list_snapshots(doc_key: &str) -> Vec<String> {
    let dir = snapshot_dir_for(find_latest_doc(doc_key).as_ref());
    let mut names: Vec<String> = std::fs::read_dir(&dir)
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

/// 全量候选收集 → doc_key 软过滤 → 取最新。
fn find_latest_doc(doc_key: &str) -> Option<PathBuf> {
    let mut cands: Vec<(SystemTime, PathBuf)> = Vec::new();
    let mut push = |p: PathBuf| {
        if !p.is_file() {
            return; // Recent 里可能有已删除/改名的旧目标, 只收现存文件
        }
        // R7-6: 排除我们自己的快照 — 快照墙开进 Plasticity 后会进 Recent,
        // 文件名又含文档名, 不排除会劫持「当前文档」判定(23:10 实测)。
        // R7-9: 快照的快照防护 — 路径任一层含 snapshot 目录的 .plasticity
        // 一律不作候选(改名/挪动也逃不掉), 杜绝对快照再拍快照的循环嵌套。
        let in_snapshot_dir = p.components().any(|c| {
            c.as_os_str()
                .to_str()
                .map(|s| s.eq_ignore_ascii_case("snapshot"))
                .unwrap_or(false)
        });
        if in_snapshot_dir {
            return;
        }
        if p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.contains("-snap-d"))
            .unwrap_or(false)
        {
            return;
        }
        if let Ok(meta) = p.metadata() {
            let mt = meta.modified().unwrap_or(UNIX_EPOCH);
            cands.push((mt, p));
        }
    };

    // 1) Recent .lnk 解析 — 本机实锤唯一能覆盖 D:\...\PLAhelper\test\model1.plasticity 的通道。
    for p in recent_plasticity_targets() {
        push(p);
    }
    // 2) 已知文件夹(SHGetKnownFolderPath, 跟随系统重定向) 递归深度 2。
    for root in known_roots() {
        scan_dir_recursive(&root, 2, &mut push);
    }

    if cands.is_empty() {
        return None;
    }
    cands.sort_by(|a, b| b.0.cmp(&a.0)); // 新→旧
    let key = doc_key.trim().trim_end_matches(".plasticity");
    if key.is_empty() {
        return cands.first().map(|(_, p)| p.clone());
    }
    // 软过滤: 命中集合非空才用, 否则回退全局最新(全局都空时上面已返回 None)。
    cands.iter()
        .find(|(_, p)| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains(key))
                .unwrap_or(false)
        })
        .or_else(|| cands.first())
        .map(|(_, p)| p.clone())
}

/// 已知文件夹扫描根: SHGetKnownFolderPath(跟随重定向) + USERPROFILE 兜底, 去重。
fn known_roots() -> Vec<PathBuf> {
    use known_folder::folder_id;
    let mut roots = Vec::new();
    for fid in [
        folder_id::DOCUMENTS,
        folder_id::DESKTOP,
        folder_id::DOWNLOADS,
    ] {
        if let Some(p) = known_folder::path(fid) {
            roots.push(p);
        }
    }
    let home = dirs_home();
    roots.push(home.join("Documents"));
    roots.push(home.join("Desktop"));
    roots.push(home.join("Downloads"));
    roots.push(home.clone());
    let mut seen = std::collections::HashSet::new();
    roots.retain(|p| seen.insert(p.canonicalize().unwrap_or_else(|_| p.clone())));
    roots
}

/// 深度受限递归扫描 .plasticity 文件(深度 0 = 只扫本层)。
fn scan_dir_recursive(dir: &PathBuf, depth: u8, push: &mut impl FnMut(PathBuf)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            // 跳过明显的系统/缓存目录, 避免扫 home 全树时踩进 AppData
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "AppData" {
                continue;
            }
            if depth > 0 {
                scan_dir_recursive(&p, depth - 1, push);
            }
        } else if p
            .extension()
            .map(|x| x == "plasticity")
            .unwrap_or(false)
        {
            push(p);
        }
    }
}

/// Known Folder API 封装(windows crate, 仅 cfg(windows) 生效, 其它平台空实现)。
mod known_folder {
    #[cfg(windows)]
    pub mod folder_id {
        pub use windows::Win32::UI::Shell::{
            FOLDERID_Desktop as DESKTOP, FOLDERID_Documents as DOCUMENTS,
            FOLDERID_Downloads as DOWNLOADS,
        };
    }

    /// SHGetKnownFolderPath → PWSTR(调用方持有, 用完 CoTaskMemFree)。
    #[cfg(windows)]
    pub fn path(fid: windows::core::GUID) -> Option<std::path::PathBuf> {
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows::Win32::UI::Shell::{KNOWN_FOLDER_FLAG, SHGetKnownFolderPath};
        unsafe {
            match SHGetKnownFolderPath(&fid, KNOWN_FOLDER_FLAG(0), None) {
                Ok(pw) => {
                    let s = pw.to_string().ok();
                    CoTaskMemFree(Some(pw.as_ptr() as *const _));
                    s.map(std::path::PathBuf::from)
                }
                Err(_) => None,
            }
        }
    }

    #[cfg(not(windows))]
    pub mod folder_id {
        pub const DOCUMENTS: u32 = 0;
        pub const DESKTOP: u32 = 0;
        pub const DOWNLOADS: u32 = 0;
    }
    #[cfg(not(windows))]
    pub fn path(_fid: u32) -> Option<std::path::PathBuf> {
        None
    }
}

/// 解析 Windows Recent 中所有 *.plasticity.lnk 的目标完整路径。
/// COM: CoInitializeEx → CoCreateInstance(ShellLink) → IPersistFile::Load → IShellLinkW::GetPath。
/// 注: windows-0.52 未生成 CLSID_ShellLink 常量, 手写 GUID(00021401-0000-0000-C000-000000000046)。
#[cfg(windows)]
fn recent_plasticity_targets() -> Vec<PathBuf> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{ComInterface, GUID, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, STGM_READ,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, SHGetKnownFolderPath, FOLDERID_Recent, KNOWN_FOLDER_FLAG};

    // CLSID_ShellLink 的稳定值(自 Windows 95 起未变)
    const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);

    let mut out = Vec::new();
    unsafe {
        // 可能已被宿主初始化为其它模式, RPC_E_CHANGED_MODE 时 apartment 仍就绪, 照常继续
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let recent_dir = match SHGetKnownFolderPath(&FOLDERID_Recent, KNOWN_FOLDER_FLAG(0), None) {
            Ok(pw) => {
                let s = pw.to_string().ok().map(PathBuf::from);
                windows::Win32::System::Com::CoTaskMemFree(Some(pw.as_ptr() as *const _));
                s
            }
            Err(_) => None,
        };
        let Some(recent_dir) = recent_dir else { return out };

        let Ok(rd) = std::fs::read_dir(&recent_dir) else { return out };
        let mut lnks: Vec<std::path::PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.to_lowercase().ends_with(".plasticity.lnk"))
                    .unwrap_or(false)
            })
            .collect();
        lnks.sort_by_key(|p| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH)
        });
        lnks.reverse(); // 新→旧

        let shell_link: IShellLinkW =
            match CoCreateInstance(&CLSID_SHELL_LINK, None, CLSCTX_INPROC_SERVER) {
                Ok(sl) => sl,
                Err(_) => return out,
            };
        let persist: IPersistFile = match shell_link.cast() {
            Ok(p) => p,
            Err(_) => return out,
        };

        for lnk in lnks {
            let wide: Vec<u16> = std::ffi::OsStr::new(&lnk)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            if persist.Load(PCWSTR(wide.as_ptr()), STGM_READ).is_err() {
                continue;
            }
            let mut buf = [0u16; 1024];
            // GetPath(pszfile, pfd: *mut WIN32_FIND_DATAW, fflags) — 不需要 find data, 传 null
            if shell_link.GetPath(&mut buf, std::ptr::null_mut(), 0).is_err() {
                continue;
            }
            let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
            if len == 0 {
                continue;
            }
            let target = String::from_utf16_lossy(&buf[..len]);
            let target = target.trim_end_matches('\0').trim();
            if target.to_lowercase().ends_with(".plasticity") {
                out.push(PathBuf::from(target));
            }
        }
    }
    out
}

#[cfg(not(windows))]
fn recent_plasticity_targets() -> Vec<PathBuf> {
    Vec::new()
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

    /// 冒烟(本机手动跑: cargo test -p pla-ui --release -- --ignored --nocapture):
    /// 验证 Recent .lnk 解析能定位到真实文档(用户实际文档在 D 盘项目目录,
    /// 旧版只扫 USERPROFILE 必然报"未找到" — R7-3 根因回归)。
    #[test]
    #[ignore]
    fn recent_lnk_resolves_real_doc_and_snapshot_succeeds() {
        let targets = recent_plasticity_targets();
        println!("Recent 解析到 {} 个 .plasticity 目标:", targets.len());
        for t in &targets {
            println!("  {t:?} (exists={})", t.is_file());
        }
        match find_latest_doc("model1") {
            Some(p) => println!("doc_key=model1 → {p:?}"),
            None => println!("doc_key=model1 → 无候选"),
        }
        let result = take_manual_snapshot("model1", None);
        println!("take_manual_snapshot(model1) = {result:?}");
        assert!(result.is_ok(), "手动快照失败: {result:?}");
        // R7-4: 快照必须落在文档同级 snapshot/ 目录
        if let Some(doc) = find_latest_doc("model1") {
            let expect = snapshot_dir_for(Some(&doc));
            println!("期望快照目录: {expect:?} (exists={})", expect.is_dir());
            assert!(expect.is_dir(), "文档同级 snapshot/ 目录未创建");
            let list = list_snapshots("model1");
            println!("list_snapshots(model1) = {list:?}");
            assert!(!list.is_empty(), "文档同级 snapshot/ 中列不到快照");
        }
        // doc_key 无命中时软过滤回退全局最新, 也应成功
        let fallback = take_manual_snapshot("no-such-key-xyz", None);
        println!("take_manual_snapshot(无命中key) = {fallback:?}");
        assert!(fallback.is_ok(), "软过滤回退失败: {fallback:?}");
    }
}
