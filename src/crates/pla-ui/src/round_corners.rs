//! 面板圆角: Windows 11 DWM 系统圆角(R13, 取代 R5 的 `SetWindowRgn`)。
//!
//! 为什么换:
//!   - `SetWindowRgn` 是 1 位掩码 → 圆角边缘锯齿;
//!   - 窗口带区域后 DWM 不再合成非客户区, 激活切换(WM_NCACTIVATE)时
//!     DefWindowProc 会把经典样式标题栏直接画到窗口上 = 切换二级面板闪出的
//!     "Win98 标题栏";
//!   - 子视口(二级面板)在 glow 后端下窗口透明不生效(R13 实测四角为不透明
//!     #000000; 根视口工具栏透明正常), 所以不能只靠 egui 画圆角。
//!
//! DWM 圆角(`DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND`)由系统合成器做
//! 抗锯齿裁剪, 与窗口是否透明无关; 系统边框关掉(`DWMWA_BORDER_COLOR =
//! DWMWA_COLOR_NONE`), 轮廓仍由 egui 绘制。系统圆角半径为 8 逻辑 px(随 DPI
//! 缩放), egui 侧面板外圆角与之对齐(main.rs `PANEL_OUTER_ROUNDING`)。
//! Windows 10 不支持该属性, 调用失败即保持方角, 不影响功能。
//!
//! 同一轮枚举顺带处理任务栏: 二级面板/提示浮窗设为工具栏窗口的**被拥有窗口**
//! (`GWLP_HWNDPARENT`)。系统不会给被拥有窗口建任务栏按钮(R13 前每个面板都在
//! 任务栏上显示一个"未知程序"图标)。不用 `WS_EX_TOOLWINDOW`: winit 每次窗口
//! 标志变化(含显示/隐藏)都会整体重写 GWL_EXSTYLE, 会把它冲掉; owner 它不碰。
//! 必须在面板首次显示前设置(面板先隐藏渲染, 见 main.rs viewport_reveal)。
#![allow(clippy::upper_case_acronyms)]

#[cfg(windows)]
mod imp {
    use std::collections::HashSet;

    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE,
        DWMWCP_ROUND,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowLongPtrW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
        IsWindow, SetWindowLongPtrW, GWLP_HWNDPARENT, GWL_EXSTYLE, WS_EX_APPWINDOW,
    };

    /// `DWMWA_COLOR_NONE`: 不画系统边框。
    const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;

    /// 已处理过的 HWND(圆角 / 设 owner 分别记录)。
    #[derive(Default)]
    pub struct RoundState {
        applied: HashSet<isize>,
        owned: HashSet<isize>,
    }

    impl RoundState {
        pub fn new() -> Self {
            Self::default()
        }
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<HWND>);
        list.push(hwnd);
        true.into()
    }

    unsafe fn window_title(hwnd: HWND) -> String {
        let mut buf = [0u16; 128];
        let n = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    /// 给本进程全部顶层窗口(含尚未显示的新建面板)设置 DWM 圆角; 除根窗口(标题 =
    /// `root_title`, 即工具栏)外的窗口设 owner = 根窗口(不占任务栏)。每个 HWND 各只设一次。
    /// 开销 = 一次 EnumWindows; 调用方在面板隐藏阶段每帧调用, 平时低频轮询兜底。
    pub fn apply_round_corners(state: &mut RoundState, root_title: &str) {
        let my_pid = std::process::id();
        state
            .applied
            .retain(|&id| unsafe { IsWindow(HWND(id)).as_bool() });
        state
            .owned
            .retain(|&id| unsafe { IsWindow(HWND(id)).as_bool() });
        let mut hwnds: Vec<HWND> = Vec::new();
        unsafe {
            let ptr = &mut hwnds as *mut Vec<HWND> as isize;
            let _ = EnumWindows(Some(enum_proc), LPARAM(ptr));
        }
        // 本进程的有效顶层窗口(跳过 IME/消息类辅助窗口: 0 尺寸或极小)。
        let mine: Vec<HWND> = hwnds
            .into_iter()
            .filter(|&hwnd| unsafe {
                let mut pid: u32 = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid != my_pid {
                    return false;
                }
                let mut rect = RECT::default();
                GetWindowRect(hwnd, &mut rect).is_ok()
                    && rect.right - rect.left >= 40
                    && rect.bottom - rect.top >= 20
            })
            .collect();
        let root = mine
            .iter()
            .copied()
            .find(|&h| unsafe { window_title(h) } == root_title);
        if let Some(root) = root {
            for &hwnd in &mine {
                if hwnd == root {
                    continue;
                }
                unsafe {
                    if !state.owned.contains(&hwnd.0) {
                        let _ = SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, root.0);
                        state.owned.insert(hwnd.0);
                    }
                    // winit 给所有无 owner 窗口加 WS_EX_APPWINDOW(强制上任务栏, 对被拥有窗口
                    // 同样生效), 且每次窗口标志变化都会重写。它在 ShowWindow **之后**才重写,
                    // 所以隐藏阶段清掉即可: 系统在窗口首次显示时决定是否建任务栏按钮。
                    // 轮询时再清一次, 保持样式一致。
                    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                    let app = WS_EX_APPWINDOW.0 as isize;
                    if ex & app != 0 {
                        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !app);
                    }
                }
            }
        }
        for hwnd in mine {
            if state.applied.contains(&hwnd.0) {
                continue;
            }
            unsafe {
                let pref = DWMWCP_ROUND;
                let ok = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &pref as *const _ as *const core::ffi::c_void,
                    std::mem::size_of_val(&pref) as u32,
                )
                .is_ok();
                let none = DWMWA_COLOR_NONE;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_BORDER_COLOR,
                    &none as *const _ as *const core::ffi::c_void,
                    std::mem::size_of_val(&none) as u32,
                );
                // 失败(如 Windows 10)也记为已处理, 不反复重试。
                let _ = ok;
                state.applied.insert(hwnd.0);
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    /// 非 Windows 平台占位: 圆角完全由 egui 绘制。
    #[derive(Default)]
    pub struct RoundState;

    impl RoundState {
        pub fn new() -> Self {
            Self
        }
    }

    pub fn apply_round_corners(_state: &mut RoundState, _root_title: &str) {}
}

pub use imp::*;
