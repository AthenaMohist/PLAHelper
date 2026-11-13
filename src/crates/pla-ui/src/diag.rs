//! 诊断日志 `%TEMP%\pla-ui.log`(R14)。
//!
//! 背景: 2026-11-13 两次出现 pla-ui 无声退出 — 用户正常启动时没有控制台, stderr 无处可看。
//! 这里记录: 启动、正常退出(eframe on_exit)、panic(含位置与 backtrace)。
//! 下次再"消失"时: 有 panic 记录 = 代码 bug; 有启动无退出 = 被外部结束或原生崩溃。

use std::io::Write;

fn path() -> std::path::PathBuf {
    std::env::temp_dir().join("pla-ui.log")
}

/// 追加一行(带本地时间与 pid), 失败静默 — 诊断日志不能反过来影响主程序。
pub fn log(line: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path()) {
        let _ = writeln!(f, "[{ts} pid {}] {line}", std::process::id());
    }
}

/// 安装 panic 钩子(保留默认钩子的 stderr 输出)。
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        log(&format!("PANIC: {info}\n{bt}"));
        default(info);
    }));
}
