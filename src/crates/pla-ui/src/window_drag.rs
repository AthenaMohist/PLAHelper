//! R9: 自管窗口拖动(Win32 `SetCapture` + 自算 delta) — 取代 `ViewportCommand::StartDrag`。
//!
//! 背景: `StartDrag` 走的是 OS 模态循环(winit 内部 `PostMessageW(WM_NCLBUTTONDOWN,
//! HTCAPTION)` → DefWindowProc 接管消息泵)。实测表现:
//!   - 拖动期间 egui 的指针状态与窗口外框回读都不可靠(松手事件常被模态循环吞掉,
//!     `primary_down()` 恒 true → 拖动会话收不了口);
//!   - 跟随面板每帧按「回读外框 → 算目标 → 发 OuterPosition」追逐一个滞后的坐标
//!     → 拖动一级面板(工具栏)时二级面板疯狂瞬移。
//!
//! 本模块: 拖动开始时 `SetCapture(hwnd)` — 之后即使鼠标移出这个 246x30 的小窗口,
//! `WM_MOUSEMOVE` 仍送达本窗口, egui 帧循环照常运行, 指针位置连续可读 →
//! 自算 delta 后发 `OuterPosition`。跟随方与拖动方在同一帧拿到**同一个权威坐标**,
//! 因此同步且平滑。鼠标松开时系统自动释放 capture 并送达 `WM_LBUTTONUP`,
//! 保险起见再显式 `ReleaseCapture()`。
//!
//! hwnd 定位: 按窗口标题 + 期望宽度(物理像素容差 40)在本进程可见顶层窗口里挑最
//! 接近的 — 标题消歧(快照墙/视口状态宽度相同), 宽度兜开同名浮窗(如 HUD)。
//! 找不到(非 Windows / 窗口尚未创建)时调用方降级回 `StartDrag`。

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT};
    // SetCapture/ReleaseCapture 在 KeyboardAndMouse(不在 WindowsAndMessaging)。
    use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetCursorPos, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible,
    };

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<HWND>);
        list.push(hwnd);
        true.into()
    }

    /// 本进程中标题为 `title`、宽度最接近 `expect_w_pt * ppp` 的可见顶层窗口。
    /// 返回 HWND 的裸值(isize), 便于跨帧缓存; 窗口销毁后由下次查找自然失效。
    pub fn find_own_window(title: &str, expect_w_pt: f32, ppp: f32) -> Option<isize> {
        if title.is_empty() {
            return None;
        }
        let want = expect_w_pt * ppp;
        let my_pid = std::process::id();
        let mut hwnds: Vec<HWND> = Vec::new();
        unsafe {
            let ptr = &mut hwnds as *mut Vec<HWND> as isize;
            let _ = EnumWindows(Some(enum_proc), LPARAM(ptr));
        }
        let mut best: Option<(isize, f32)> = None;
        for hwnd in hwnds {
            unsafe {
                if !IsWindowVisible(hwnd).as_bool() {
                    continue;
                }
                let mut pid: u32 = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid != my_pid {
                    continue;
                }
                let mut buf = [0u16; 256];
                let n = GetWindowTextW(hwnd, &mut buf);
                if n <= 0 {
                    continue;
                }
                let got = String::from_utf16_lossy(&buf[..(n as usize).min(buf.len())]);
                if got != title {
                    continue;
                }
                let mut r = RECT::default();
                if GetWindowRect(hwnd, &mut r).is_err() {
                    continue;
                }
                let d = ((r.right - r.left) as f32 - want).abs();
                // 宽度差超过 40 物理像素视为不是目标窗口(排除同名浮窗)。
                if d > 40.0 {
                    continue;
                }
                if best.map_or(true, |(_, bd)| d < bd) {
                    best = Some((hwnd.0, d));
                }
            }
        }
        best.map(|(h, _)| h)
    }

    /// 当前鼠标的**屏幕物理坐标**(`GetCursorPos`)。
    ///
    /// 关键: 不能用 egui 的 `pointer.latest_pos()` 做 delta — 那是**窗口内相对
    /// 坐标**, 窗口一移动同一屏幕位置的相对坐标就变, delta 会自我抵消
    /// (target 被拉回原点 = 拖不动/乱跳)。屏幕绝对坐标与窗口位置无关,
    /// 是 OS 拖动的等价语义。
    pub fn cursor_pos_phys() -> Option<(f32, f32)> {
        let mut pt = POINT::default();
        unsafe {
            if GetCursorPos(&mut pt).is_err() {
                return None;
            }
        }
        Some((pt.x as f32, pt.y as f32))
    }

    /// R30b: 鼠标左键**物理**状态(GetAsyncKeyState, 不经 egui)。
    /// egui 的 `primary_down()` 在 OS 模态拖动/调整后可能卡在 true(松手事件被吞),
    /// 拖动/调整会话的开始与结束都以这里为准。左右键交换时 VK_LBUTTON 仍指主键。
    pub fn primary_button_down() -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
        unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 }
    }

    /// 捕获鼠标: 之后鼠标移出窗口也能继续收到移动事件(拖动连贯的前提)。
    pub fn set_capture(hwnd: isize) {
        unsafe {
            let _ = SetCapture(HWND(hwnd));
        }
    }

    /// 释放捕获(系统通常在 WM_LBUTTONUP 时自动释放, 这里显式兜底)。
    pub fn release_capture() {
        unsafe {
            let _ = ReleaseCapture();
        }
    }
}

#[cfg(not(windows))]
mod imp {
    /// 非 Windows 平台占位: 无 Win32 capture, 调用方降级到 `StartDrag`。
    pub fn find_own_window(_title: &str, _expect_w_pt: f32, _ppp: f32) -> Option<isize> {
        None
    }
    pub fn set_capture(_hwnd: isize) {}
    pub fn release_capture() {}
    pub fn primary_button_down() -> bool {
        false
    }
    pub fn cursor_pos_phys() -> Option<(f32, f32)> {
        None
    }
}

pub use imp::*;
