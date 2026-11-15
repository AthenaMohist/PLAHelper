//! N4b: 孔规格方案(用户 2026-11-15)。每个方案一个 JSON 文件: `<exe 目录>/hole-presets/<方案名>.json`,
//! 文件名 = 方案名(改名 = 改文件名)。界面下拉切换、新建(复制当前)、改名、"编辑"用系统默认程序打开 JSON;
//! 文件改动按修改时间热生效(解析失败如实报错, 保留上一次能用的内容)。
//!
//! 格式: `{ "sizes": [ { "size": "M4", "clearance": 4.5, "tap": 3.3, "cbD": 8.0, "cbDepth": 4.4, "cskD": 9.0 }, … ] }`
//! 目录为空时写入内置方案(ISO 公制: 过孔 ISO 273 中等、柱形沉头按 ISO 4762 头部、锥形沉头 90° ISO 10642)。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 一档孔规格(长度单位 = 文档单位, 内置方案按 mm)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HoleSize {
    pub size: String,
    /// 过孔直径。
    pub clearance: f64,
    /// 螺纹底孔直径。
    pub tap: f64,
    /// 柱形沉头直径 / 深度。
    #[serde(rename = "cbD")]
    pub cb_d: f64,
    #[serde(rename = "cbDepth")]
    pub cb_depth: f64,
    /// 锥形沉头(90°)直径。
    #[serde(rename = "cskD")]
    pub csk_d: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresetFile {
    #[serde(default)]
    pub sizes: Vec<HoleSize>,
}

/// 内置方案名与内容。
pub const BUILTIN_NAME: &str = "ISO metric";

pub fn builtin() -> Vec<HoleSize> {
    [
        ("M2", 2.4, 1.6, 4.4, 2.3, 4.4),
        ("M2.5", 2.9, 2.05, 5.5, 2.8, 5.5),
        ("M3", 3.4, 2.5, 6.5, 3.3, 6.7),
        ("M4", 4.5, 3.3, 8.0, 4.4, 9.0),
        ("M5", 5.5, 4.2, 10.0, 5.4, 11.2),
        ("M6", 6.6, 5.0, 11.0, 6.5, 13.4),
        ("M8", 9.0, 6.8, 15.0, 8.6, 17.9),
        ("M10", 11.0, 8.5, 18.0, 10.8, 22.4),
        ("M12", 13.5, 10.2, 20.0, 13.0, 26.9),
    ]
    .iter()
    .map(|&(s, c, t, cbd, cbh, csk)| HoleSize { size: s.into(), clearance: c, tap: t, cb_d: cbd, cb_depth: cbh, csk_d: csk })
    .collect()
}

/// 方案目录。
pub fn dir() -> PathBuf {
    crate::prefs::data_dir().join("hole-presets")
}

pub fn path_in(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.json"))
}

/// 方案名列表(按名字排序); 目录为空 / 不存在时先写入内置方案。
pub fn list_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let p = e.path();
                    (p.extension().and_then(|x| x.to_str()) == Some("json")).then(|| p.file_stem().and_then(|s| s.to_str()).map(String::from)).flatten()
                })
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() {
        if write_in(dir, BUILTIN_NAME, &builtin()).is_ok() {
            names.push(BUILTIN_NAME.into());
        }
    }
    names.sort();
    names
}

/// 读一个方案(解析失败 → 错误文本, 含行列)。
pub fn load_in(dir: &Path, name: &str) -> Result<Vec<HoleSize>, String> {
    let txt = std::fs::read_to_string(path_in(dir, name)).map_err(|e| e.to_string())?;
    let f: PresetFile = serde_json::from_str(txt.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    if f.sizes.is_empty() {
        return Err("no sizes".into());
    }
    Ok(f.sizes)
}

pub fn write_in(dir: &Path, name: &str, sizes: &[HoleSize]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let body = serde_json::to_vec_pretty(&PresetFile { sizes: sizes.to_vec() }).map_err(|e| e.to_string())?;
    std::fs::write(path_in(dir, name), body).map_err(|e| e.to_string())
}

/// 新建: 复制 base 的内容(读不到就用内置), 名字取"<base> 2/3…"里第一个没用过的, 返回新名字。
pub fn create_in(dir: &Path, base: &str) -> Result<String, String> {
    let sizes = load_in(dir, base).unwrap_or_else(|_| builtin());
    let mut k = 2;
    let mut name = format!("{base} {k}");
    while path_in(dir, &name).exists() {
        k += 1;
        name = format!("{base} {k}");
    }
    write_in(dir, &name, &sizes)?;
    Ok(name)
}

/// 改名(= 改文件名); 新名不能为空、不能含路径字符、不能和别的方案重名。
pub fn rename_in(dir: &Path, old: &str, new: &str) -> Result<(), String> {
    let new = new.trim();
    if new.is_empty() || new.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        return Err(format!("bad name: {new}"));
    }
    if new == old {
        return Ok(());
    }
    if path_in(dir, new).exists() {
        return Err(format!("preset {new} already exists"));
    }
    std::fs::rename(path_in(dir, old), path_in(dir, new)).map_err(|e| e.to_string())
}

/// 文件修改时间(热生效用)。
pub fn mtime_in(dir: &Path, name: &str) -> Option<std::time::SystemTime> {
    std::fs::metadata(path_in(dir, name)).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_roundtrip() {
        let d = std::env::temp_dir().join(format!("pla_hole_presets_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(list_in(&d), vec![BUILTIN_NAME.to_string()], "空目录 → 写入内置方案");
        let s = load_in(&d, BUILTIN_NAME).unwrap();
        assert_eq!(s.iter().find(|x| x.size == "M4").map(|x| (x.clearance, x.cb_d)), Some((4.5, 8.0)));
        let n = create_in(&d, BUILTIN_NAME).unwrap();
        assert_eq!(n, format!("{BUILTIN_NAME} 2"));
        rename_in(&d, &n, "我的规格").unwrap();
        assert!(rename_in(&d, "我的规格", BUILTIN_NAME).is_err(), "重名");
        assert!(rename_in(&d, "我的规格", "a/b").is_err(), "路径字符");
        assert_eq!(list_in(&d), vec![BUILTIN_NAME.to_string(), "我的规格".to_string()]);
        // 手改坏 JSON → 报错(界面保留上一次能用的)
        std::fs::write(path_in(&d, "我的规格"), "{ \"sizes\": [ { \"size\": 1 } ").unwrap();
        assert!(load_in(&d, "我的规格").is_err());
        // JSON 里的键名(cbD / cbDepth / cskD)
        let txt = std::fs::read_to_string(path_in(&d, BUILTIN_NAME)).unwrap();
        assert!(txt.contains("\"cbD\"") && txt.contains("\"cskD\""));
        let _ = std::fs::remove_dir_all(&d);
    }
}
