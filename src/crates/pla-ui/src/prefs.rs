//! R25: pla-ui 偏好(exe 同级 `ui-prefs.json`)。界面语言 + R30 笔记窗口位置/大小。
//!
//! 读不到 / 解析失败 → 默认值, 不报错; 写失败返回错误文本给界面显示。
//! 所有键共用一个文件, 写入时保留其他键(`save_value` 是唯一写入口)。

use std::path::PathBuf;

use pla_bridge::i18n::Lang;

fn prefs_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ui-prefs.json")
}

fn load_all() -> serde_json::Value {
    std::fs::read_to_string(prefs_path())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}))
}

/// 读一个键(不存在 → None)。
pub fn load_value(key: &str) -> Option<serde_json::Value> {
    let v = load_all();
    v.get(key).cloned().filter(|x| !x.is_null())
}

/// 写一个键(保留文件里其他键)。
pub fn save_value(key: &str, value: serde_json::Value) -> Result<(), String> {
    let mut v = load_all();
    v[key] = value;
    std::fs::write(prefs_path(), serde_json::to_string_pretty(&v).unwrap_or_default()).map_err(|e| e.to_string())
}

/// 读偏好里的语言; 没有则简体中文。
pub fn load_lang() -> Lang {
    load_value("lang").and_then(|v| v.as_str().and_then(Lang::from_code)).unwrap_or(Lang::Zh)
}

/// 保存语言。
pub fn save_lang(lang: Lang) -> Result<(), String> {
    save_value("lang", serde_json::Value::String(lang.code().into()))
}

/// R30: 笔记窗口外框(逻辑 pt: x, y, 宽, 高); 从未打开过 → None(用默认位置)。
pub fn load_notes_rect() -> Option<[f32; 4]> {
    let a = load_value("notes_rect")?;
    let a = a.as_array()?;
    if a.len() != 4 {
        return None;
    }
    let mut r = [0f32; 4];
    for (i, x) in a.iter().enumerate() {
        r[i] = x.as_f64()? as f32;
    }
    // R30b: 防御 — 早期拖动 bug 可能存下离谱尺寸, 读入时封顶
    r[2] = r[2].min(1600.0);
    r[3] = r[3].min(1400.0);
    (r[2] >= 120.0 && r[3] >= 120.0).then_some(r)
}

pub fn save_notes_rect(r: [f32; 4]) -> Result<(), String> {
    save_value("notes_rect", serde_json::json!(r.iter().map(|x| x.round()).collect::<Vec<_>>()))
}
