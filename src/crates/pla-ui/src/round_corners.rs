//! Win32 `SetWindowRgn` 圆角裁剪 — 面板四角圆角的可靠路径(R5 修复)。
//!
//! 背景: egui `Rounding::same(6.0)` 画的是四角一致圆角, 但 Windows glow 后端
//! 的透明窗口未真正生效(R5 实测只有左上角圆, 其余三角露出不透明底色)。
//! 本模块不赌透明修复, 直接用 Win32 区域裁剪把 OS 窗口四角切圆:
//!   - 主工具栏窗口(固定尺寸)与全部子视口面板(快照墙/实体树/尺寸/设置/
//!     提示浮窗, 尺寸动态)都是本进程的可见顶层窗口, 统一枚举处理。
//!   - 已套过 Rgn 的 HWND 记录 (宽, 高); 尺寸未变不重复套(移动不影响 —
//!     Rgn 相对窗口客户区原点)。子视口尺寸变化时按 GetWindowRect 实际
//!     物理像素重建 Rgn。
//!   - Rgn 一律用物理像素(GetWindowRect 语义), 与 DPI 无关地贴合窗口。
#![allow(clippy::upper_case_acronyms)]

#[cfg(windows)]
mod imp {
    use std::collections::HashMap;

    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateRoundRectRgn, DeleteObject, SetWindowRgn, HRGN,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindow, IsWindowVisible,
    };

    /// 圆角半径(物理像素): 12px 椭圆对应 6pt 逻辑圆角(@1x 缩放)。
    const RADIUS_PX: i32 = 12;

    /// 圆角裁剪状态: 已套 Rgn 的 HWND → 套用时窗口 (宽, 高) 物理像素。
    #[derive(Default)]
    pub struct RoundState {
        applied: HashMap<isize, (i32, i32)>,
    }

    impl RoundState {
        pub fn new() -> Self {
            Self::default()
        }
    }

    /// EnumWindows 回调: 收集全部顶层 HWND。
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<HWND>);
        list.push(hwnd);
        true.into()
    }

    /// 轮询本进程全部可见顶层窗口, 为新出现或尺寸变化的窗口套圆角 Rgn。
    /// 每帧调用太频(枚举开销), 调用方按 ~500ms 节流(main.rs ROUND_POLL_MS)。
    pub fn poll_round_windows(state: &mut RoundState) {
        let my_pid = std::process::id();
        // 清理已销毁窗口的死条目(QA 返工备注): HWND 会被系统复用, 不清理
        // 则 applied 表随子视口反复开合缓慢膨胀。残余极低危场景 — 复用后
        // 尺寸恰好与旧条目相同会漏套一次 — 由下一次尺寸变化兜底重套。
        state.applied.retain(|&hwnd_id, _| unsafe {
            IsWindow(HWND(hwnd_id)).as_bool()
        });
        let mut hwnds: Vec<HWND> = Vec::new();
        unsafe {
            let ptr = &mut hwnds as *mut Vec<HWND> as isize;
            // 失败(极少)时返回已收集部分, 容忍即可。
            let _ = EnumWindows(Some(enum_proc), LPARAM(ptr));
        }
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
                let mut rect = RECT::default();
                if GetWindowRect(hwnd, &mut rect).is_err() {
                    continue;
                }
                let w = rect.right - rect.left;
                let h = rect.bottom - rect.top;
                // 最小化窗口 GetWindowRect 返回 (-32000,-32000) 一类离屏坐标,
                // 尺寸无意义, 跳过(恢复后尺寸变化会重新套)。
                if w <= 0 || h <= 0 || rect.left < -20000 || rect.top < -20000 {
                    continue;
                }
                // 已套过且尺寸未变: 不重复套(Rgn 相对窗口, 移动无需重套)。
                if state.applied.get(&hwnd.0) == Some(&(w, h)) {
                    continue;
                }
                // CreateRoundRectRgn 返回裸 HRGN(失败返回空句柄)。
                let rgn: HRGN = CreateRoundRectRgn(0, 0, w + 1, h + 1, RADIUS_PX, RADIUS_PX);
                if rgn.is_invalid() {
                    continue;
                }
                // 成功后系统接管 Rgn 所有权, 不得 DeleteObject; 失败则手动释放。
                if SetWindowRgn(hwnd, rgn, true) != 0 {
                    state.applied.insert(hwnd.0, (w, h));
                } else {
                    let _ = DeleteObject(rgn);
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    /// 非 Windows 平台占位: 无 Win32 区域裁剪, UI 仍是 egui 画的圆角。
    #[derive(Default)]
    pub struct RoundState;

    impl RoundState {
        pub fn new() -> Self {
            Self
        }
    }

    pub fn poll_round_windows(_state: &mut RoundState) {}
}

pub use imp::*;
