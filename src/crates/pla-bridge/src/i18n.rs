//! R25: 界面语言(简体中文 / English), 运行时一键切换。
//!
//! 用法(两种语言写在同一个调用点, 不另建字典 —— 改文案时两种语言一起改, 不会对不上):
//!   * 纯文本: `tr("保存", "Save")` → `&'static str`
//!   * 带参数: `t!("已删除 {n} 个快照", "Deleted {n} snapshots")` → `String`(支持 `{x}` 隐式捕获与位置参数)
//!
//! 当前语言是进程级全局(原子量), 切换后下一帧所有界面文本即变。放在 pla-bridge(最底层)是因为
//! CDP 线程产生的部分提示(选择集应用结果等)也要跟着语言走。

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    /// 偏好设置文件里的代码。
    pub fn code(self) -> &'static str {
        match self {
            Lang::Zh => "zh-CN",
            Lang::En => "en",
        }
    }

    pub fn from_code(s: &str) -> Option<Lang> {
        match s {
            "zh-CN" | "zh" | "zh-Hans" => Some(Lang::Zh),
            "en" | "en-US" | "en-GB" => Some(Lang::En),
            _ => None,
        }
    }

    /// 语言自己的名字(切换控件上显示)。
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Zh => "简体中文",
            Lang::En => "English",
        }
    }

    pub const ALL: [Lang; 2] = [Lang::Zh, Lang::En];
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    LANG.store(if lang == Lang::En { 1 } else { 0 }, Ordering::Relaxed);
}

pub fn get() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 { Lang::En } else { Lang::Zh }
}

pub fn is_en() -> bool {
    get() == Lang::En
}

/// 按当前语言二选一(纯文本)。
pub fn tr(zh: &'static str, en: &'static str) -> &'static str {
    pick(get(), zh, en)
}

/// 按指定语言二选一(纯函数, 单测用 — 测试并行跑, 不能去改全局语言)。
pub fn pick<T>(lang: Lang, zh: T, en: T) -> T {
    if lang == Lang::En { en } else { zh }
}

/// 按当前语言二选一并格式化: `t!("中文 {x}", "English {x}")`。
#[macro_export]
macro_rules! t {
    ($zh:literal, $en:literal $(,)?) => {
        if $crate::i18n::is_en() { format!($en) } else { format!($zh) }
    };
    ($zh:literal, $en:literal, $($arg:tt)+) => {
        if $crate::i18n::is_en() { format!($en, $($arg)+) } else { format!($zh, $($arg)+) }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    // 注意: 测试并行运行, 这里不改全局语言(别的测试会断言中文文案); 默认语言即中文。
    #[test]
    fn picks_by_language_and_formats() {
        assert_eq!(pick(Lang::Zh, "保存", "Save"), "保存");
        assert_eq!(pick(Lang::En, "保存", "Save"), "Save");
        let n = 3;
        assert_eq!(get(), Lang::Zh, "默认简体中文");
        assert_eq!(crate::t!("已删除 {n} 个", "Deleted {n}"), "已删除 3 个", "隐式捕获");
        assert_eq!(crate::t!("{} / {}", "{} of {}", 1, 2), "1 / 2", "位置参数");
        assert_eq!(Lang::from_code("en"), Some(Lang::En));
        assert_eq!(Lang::from_code("zh-CN").unwrap().code(), "zh-CN");
        assert_eq!(Lang::from_code("fr"), None);
    }
}
