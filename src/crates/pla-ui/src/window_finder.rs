//! Win32 窗口发现 + 多实例绑定(吸附目标)。
//!
//! 多实例策略(两方案择一): 采用「进程序号」方案 — 每个实例枚举所有 pla-ui.exe
//! 顶层窗口得到实例 PID 列表, 按进程创建时间排序后取自身位置 N, 绑定「按 HWND
//! 排序的第 N 个 Plasticity 窗口」。相比「占用电」方案无需任何进程间共享状态,
//! 实现简单且结果确定; 占用电不持久、重启重排可接受(需求已认可)。
#![allow(clippy::upper_case_acronyms)]

#[cfg(windows)]
mod imp {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, BOOL, FILETIME, HWND, LPARAM, RECT};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    };

    /// 一个可吸附的目标窗口(Plasticity 主窗口)。
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct TargetWindow {
        pub hwnd: isize,
        pub title: String,
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    impl TargetWindow {
        pub fn width(&self) -> i32 {
            self.right - self.left
        }
        pub fn height(&self) -> i32 {
            self.bottom - self.top
        }
    }

    /// EnumWindows 回调: 收集全部顶层 HWND。
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<HWND>);
        list.push(hwnd);
        true.into()
    }

    fn all_top_level_windows() -> Vec<HWND> {
        let mut out: Vec<HWND> = Vec::new();
        unsafe {
            let ptr = &mut out as *mut Vec<HWND> as isize;
            // 失败(极少)时返回已收集部分, 容忍即可。
            let _ = EnumWindows(Some(enum_proc), LPARAM(ptr));
        }
        out
    }

    unsafe fn window_rect(hwnd: HWND) -> Option<RECT> {
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r).ok()?;
        Some(r)
    }

    unsafe fn window_title(hwnd: HWND) -> String {
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n <= 0 {
            String::new()
        } else {
            String::from_utf16_lossy(&buf[..n as usize])
        }
    }

    /// 窗口所属进程的映像名(如 "Plasticity.exe"); 拿不到返回 None。
    unsafe fn process_image_name(hwnd: HWND) -> Option<String> {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let name = (|| {
            let mut buf = [0u16; 1024];
            let mut len: u32 = buf.len() as u32;
            QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
                .ok()?;
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        })();
        let _ = CloseHandle(handle);
        name
    }

    fn image_file_name(path: &str) -> &str {
        path.rsplit(['\\', '/']).next().unwrap_or(path)
    }

    /// 是否为 Plasticity 主窗口: 优先进程映像名 == Plasticity.exe,
    /// 拿不到映像名时退回标题含 ".plasticity" / "plasticity"。
    unsafe fn is_plasticity_window(hwnd: HWND, title: &str) -> bool {
        let by_image = process_image_name(hwnd)
            .as_deref()
            .map(|p| image_file_name(p).eq_ignore_ascii_case("plasticity.exe"))
            .unwrap_or(false);
        let by_title = {
            let t = title.to_lowercase();
            t.contains(".plasticity") || t.contains("plasticity")
        };
        by_image || by_title
    }

    /// 全部可见的 Plasticity 主窗口, 按 HWND 升序。
    pub fn find_plasticity_windows() -> Vec<TargetWindow> {
        let mut out = Vec::new();
        for hwnd in all_top_level_windows() {
            unsafe {
                if !IsWindowVisible(hwnd).as_bool() {
                    continue;
                }
                let title = window_title(hwnd);
                if !is_plasticity_window(hwnd, &title) {
                    continue;
                }
                // 过滤幽灵/通知类小窗。
                let Some(rect) = window_rect(hwnd) else { continue };
                if rect.right - rect.left < 200 || rect.bottom - rect.top < 200 {
                    continue;
                }
                // 最小化窗口 GetWindowRect 返回 (-32000,-32000): 视为不可吸附,
                // 否则工具栏会被定位到屏幕外(QA 建议)。
                if rect.left < -20000 || rect.top < -20000 {
                    continue;
                }
                out.push(TargetWindow {
                    hwnd: hwnd.0,
                    title,
                    left: rect.left,
                    top: rect.top,
                    right: rect.right,
                    bottom: rect.bottom,
                });
            }
        }
        out.sort_by_key(|w| w.hwnd);
        out
    }

    /// 进程创建时间(FILETIME 64 位); 拿不到则退回 PID 值做排序键。
    fn process_creation_key(pid: u32) -> u64 {
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return u64::from(pid);
            };
            let mut create = FILETIME::default();
            let mut exit = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            let ok = GetProcessTimes(handle, &mut create, &mut exit, &mut kernel, &mut user).is_ok();
            let _ = CloseHandle(handle);
            if ok {
                (u64::from(create.dwHighDateTime) << 32) | u64::from(create.dwLowDateTime)
            } else {
                u64::from(pid)
            }
        }
    }

    /// 当前实例序号: 所有 pla-ui.exe 可见顶层窗口对应 PID(去重)按创建时间
    /// 排序后, 自身 PID 的位置。0 = 最早启动的实例。
    pub fn instance_ordinal() -> usize {
        let my_pid = std::process::id();
        let mut pids: Vec<u32> = Vec::new();
        for hwnd in all_top_level_windows() {
            unsafe {
                if !IsWindowVisible(hwnd).as_bool() {
                    continue;
                }
                let is_self_image = process_image_name(hwnd)
                    .as_deref()
                    .map(|p| image_file_name(p).eq_ignore_ascii_case("pla-ui.exe"))
                    .unwrap_or(false);
                if !is_self_image {
                    continue;
                }
                let mut pid: u32 = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid != 0 && !pids.contains(&pid) {
                    pids.push(pid);
                }
            }
        }
        let mut keyed: Vec<(u64, u32)> =
            pids.into_iter().map(|pid| (process_creation_key(pid), pid)).collect();
        keyed.sort_unstable();
        keyed
            .iter()
            .position(|(_, pid)| *pid == my_pid)
            .unwrap_or(0)
    }

    /// 获取本实例应绑定的 Plasticity 窗口(第 N 个); 不足 N 个则 None(未绑定)。
    pub fn acquire_target() -> Option<TargetWindow> {
        let idx = instance_ordinal();
        find_plasticity_windows().into_iter().nth(idx)
    }
}

#[cfg(not(windows))]
mod imp {
    /// 非 Windows 平台占位: 永远未绑定(UI 仍可用, 只是没有吸附)。
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct TargetWindow {
        pub title: String,
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    pub fn find_plasticity_windows() -> Vec<TargetWindow> {
        Vec::new()
    }

    pub fn instance_ordinal() -> usize {
        0
    }

    pub fn acquire_target() -> Option<TargetWindow> {
        None
    }
}

pub use imp::*;
