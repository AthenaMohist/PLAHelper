//! R25: pla-ui 偏好(exe 同级 `ui-prefs.json`)。目前只有界面语言。
//!
//! 读不到 / 解析失败 → 默认值(简体中文), 不报错; 写失败返回错误文本给界面显示。

use std::path::PathBuf;

use pla_bridge::i18n::Lang;

fn prefs_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ui-prefs.json")
}

/// 读偏好里的语言; 没有则简体中文。
pub fn load_lang() -> Lang {
    std::fs::read_to_string(prefs_path())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["lang"].as_str().and_then(Lang::from_code))
        .unwrap_or(Lang::Zh)
}

/// 保存语言(保留文件里其他键)。
pub fn save_lang(lang: Lang) -> Result<(), String> {
    let path = prefs_path();
    let mut v: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .filter(|v: &serde_json::Value| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    v["lang"] = serde_json::Value::String(lang.code().into());
    std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap_or_default()).map_err(|e| e.to_string())
}
