//! Win32 窗口发现 + 多实例绑定(吸附目标) + 置顶管理。
//!
//! R19 多实例策略 = 「认领」: 每个 pla-ui 把自己绑定的 Plasticity HWND 写在自己工具栏
//! 窗口的属性 `PLA_BOUND_HWND` 上(SetPropW, 跨进程可读)。绑定一旦建立就**跟着 HWND 走**
//! (换文档、改标题、最小化都不换绑), 只有该窗口关闭才重新认领。新认领时跳过别的实例
//! 已认领的窗口: 优先前台的 Plasticity 窗口, 否则按 Z 序取最近用过的那个。
//! 两个实例同时认领同一窗口时, 先启动的保留, 后启动的让出。
//! (旧「进程序号 + 文档名」方案每次轮询都重算, 标题一变就可能换绑、两个助手抢同一窗口,
//! 快照墙因此混进别的文件的快照 — pitfalls #31。)
//!
//! 置顶: 只在绑定的 Plasticity 窗口(或其弹出窗口、或助手自己)处于前台时置顶;
//! 切到别的程序就取消置顶并贴在自己的 Plasticity 窗口正上方(Z 序), 被别的窗口正常盖住。
#![allow(clippy::upper_case_acronyms)]

#[cfg(windows)]
mod imp {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, BOOL, FILETIME, HANDLE, HWND, LPARAM, RECT};
    use windows::Win32::System::Threading::{
        AttachThreadInput, GetCurrentThreadId, GetProcessTimes, OpenProcess,
        QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetAncestor, GetForegroundWindow, GetPropW, GetWindow, GetWindowLongPtrW,
        GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
        IsWindowVisible, RemovePropW, SetForegroundWindow, SetPropW, SetWindowPos, GA_ROOTOWNER,
        GWL_EXSTYLE, GW_HWNDPREV, HWND_BOTTOM, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE, WS_EX_TOPMOST,
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
        /// 已最小化(矩形无意义, 不做吸附定位; 但绑定保留)。
        pub minimized: bool,
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

    /// 是否为 Plasticity 主窗口。R6 修正: 进程映像名可查时以映像名为准 —
    /// 之前「标题含 plasticity」的无条件兜底会把标题里恰好带 "plasticity"
    /// 字样的异己进程窗口(如某预览器渲染的 .plasticity 文档窗)误判为目标,
    /// 导致工具栏吸错窗口。标题兜底仅在映像名拿不到时生效, 且要求含
    /// ".plasticity"(比裸词更严)。
    unsafe fn is_plasticity_window(hwnd: HWND, title: &str) -> bool {
        match process_image_name(hwnd) {
            Some(path) => image_file_name(&path).eq_ignore_ascii_case("plasticity.exe"),
            None => title.to_lowercase().contains(".plasticity"),
        }
    }

    /// Plasticity 主程序完整路径(取任意一个 Plasticity 主窗口所属进程)。
    /// 用于「快照墙点缩略图 → 新 Plasticity 窗口打开快照」(R7-5)。
    pub fn plasticity_exe_path() -> Option<std::path::PathBuf> {
        find_plasticity_windows()
            .first()
            .and_then(|w| unsafe { process_image_name(HWND(w.hwnd)) })
            .map(std::path::PathBuf::from)
    }

    /// 全部可见、未最小化的 Plasticity 主窗口, 按 Z 序(最前 → 最后, 即最近用过的在前)。
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
                    minimized: false,
                });
            }
        }
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

    // ===================== R19: 认领式绑定 =====================

    /// 认领属性名(写在本实例工具栏窗口上, 值 = 绑定的 Plasticity HWND)。
    fn claim_prop() -> Vec<u16> {
        "PLA_BOUND_HWND\0".encode_utf16().collect()
    }

    unsafe fn window_pid(hwnd: HWND) -> u32 {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid
    }

    /// 本进程里标题为 `root_title` 的顶层窗口(工具栏根窗口)。
    pub fn own_root_window(root_title: &str) -> Option<isize> {
        let my_pid = std::process::id();
        all_top_level_windows()
            .into_iter()
            .find(|&h| unsafe { window_pid(h) == my_pid && window_title(h) == root_title })
            .map(|h| h.0)
    }

    /// 把认领写到本实例工具栏上(None = 撤销认领)。
    pub fn set_claim(root: isize, target: Option<isize>) {
        let name = claim_prop();
        unsafe {
            match target {
                Some(t) => {
                    let _ = SetPropW(HWND(root), PCWSTR(name.as_ptr()), HANDLE(t));
                }
                None => {
                    let _ = RemovePropW(HWND(root), PCWSTR(name.as_ptr()));
                }
            }
        }
    }

    /// 其他 pla-ui 实例的认领: (认领的 HWND, 认领者进程创建时间)。
    fn foreign_claims() -> Vec<(isize, u64)> {
        let my_pid = std::process::id();
        let name = claim_prop();
        let mut out = Vec::new();
        for h in all_top_level_windows() {
            unsafe {
                let pid = window_pid(h);
                if pid == 0 || pid == my_pid {
                    continue;
                }
                let v = GetPropW(h, PCWSTR(name.as_ptr()));
                if v.0 == 0 {
                    continue;
                }
                let is_helper = process_image_name(h)
                    .as_deref()
                    .map(|p| image_file_name(p).eq_ignore_ascii_case("pla-ui.exe"))
                    .unwrap_or(false);
                if is_helper {
                    out.push((v.0, process_creation_key(pid)));
                }
            }
        }
        out
    }

    /// 读取一个仍然存在的 Plasticity 窗口(最小化也算, 标记 minimized)。
    fn target_info(hwnd: isize) -> Option<TargetWindow> {
        unsafe {
            let h = HWND(hwnd);
            if hwnd == 0 || !IsWindow(h).as_bool() || !IsWindowVisible(h).as_bool() {
                return None;
            }
            let title = window_title(h);
            if !is_plasticity_window(h, &title) {
                return None;
            }
            let minimized = IsIconic(h).as_bool();
            let rect = window_rect(h).unwrap_or_default();
            Some(TargetWindow {
                hwnd,
                title,
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
                minimized,
            })
        }
    }

    /// 绑定决策(纯逻辑, 便于单测)。
    /// `current` = 现绑定且仍存在的窗口; `windows` = 可认领候选(Z 序); `fg` = 前台窗口;
    /// `claims` = 他人认领 (HWND, 认领者创建时间); `me` = 本进程创建时间。
    pub fn decide_binding(
        current: Option<isize>,
        windows: &[isize],
        fg: Option<isize>,
        claims: &[(isize, u64)],
        me: u64,
    ) -> Option<isize> {
        let claimed_by_older = |h: isize| claims.iter().any(|&(c, t)| c == h && t < me);
        let claimed = |h: isize| claims.iter().any(|&(c, _)| c == h);
        if let Some(cur) = current {
            // 已绑定: 除非被更早启动的实例认领(同时认领的冲突), 否则一直保持。
            if !claimed_by_older(cur) {
                return Some(cur);
            }
        }
        let free: Vec<isize> = windows.iter().copied().filter(|&h| !claimed(h)).collect();
        if let Some(f) = fg.filter(|f| free.contains(f)) {
            return Some(f);
        }
        free.first().copied()
    }

    /// 解析本实例应绑定的窗口并更新认领。`current` = 上一轮绑定的 HWND。
    pub fn bind_target(current: Option<isize>, root: Option<isize>) -> Option<TargetWindow> {
        let alive = current.and_then(target_info);
        let windows: Vec<isize> = find_plasticity_windows().iter().map(|w| w.hwnd).collect();
        let fg = unsafe {
            let f = GetForegroundWindow();
            let r = GetAncestor(f, GA_ROOTOWNER);
            if r.0 != 0 { Some(r.0) } else if f.0 != 0 { Some(f.0) } else { None }
        };
        let me = process_creation_key(std::process::id());
        let chosen = decide_binding(alive.as_ref().map(|w| w.hwnd), &windows, fg, &foreign_claims(), me);
        if let Some(root) = root {
            set_claim(root, chosen);
        }
        match chosen {
            Some(h) if alive.as_ref().map(|w| w.hwnd) == Some(h) => alive,
            Some(h) => target_info(h),
            None => None,
        }
    }

    // ===================== R19: 置顶管理 =====================

    /// 绑定窗口是否"处于前台": 前台是它本身、它的弹出/被拥有窗口, 或助手自己的窗口。
    pub fn target_is_active(target: isize) -> bool {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.0 == 0 {
                return false;
            }
            if fg.0 == target || GetAncestor(fg, GA_ROOTOWNER).0 == target {
                return true;
            }
            window_pid(fg) == std::process::id()
        }
    }

    /// 本进程可见的顶层窗口(工具栏、面板、提示浮窗)。
    fn own_windows() -> Vec<HWND> {
        let my_pid = std::process::id();
        all_top_level_windows()
            .into_iter()
            .filter(|&h| unsafe { window_pid(h) == my_pid && IsWindowVisible(h).as_bool() })
            .collect()
    }

    /// 按需切换置顶。返回是否做了改动。
    /// * `active` → 全部置顶;
    /// * 否则 → 取消置顶, 并放到绑定窗口正上方(被别的程序正常遮挡); 绑定窗口最小化或
    ///   未绑定时沉到底部(不在桌面上漂着)。
    /// 只在 WS_EX_TOPMOST 与期望不符时动手(winit 改样式不碰置顶位, 不会来回打架)。
    pub fn sync_topmost(target: Option<&TargetWindow>, active: bool) -> bool {
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        let mut changed = false;
        for h in own_windows() {
            unsafe {
                let topmost = GetWindowLongPtrW(h, GWL_EXSTYLE) & (WS_EX_TOPMOST.0 as isize) != 0;
                if active == topmost {
                    continue;
                }
                changed = true;
                if active {
                    let _ = SetWindowPos(h, HWND_TOPMOST, 0, 0, 0, 0, flags);
                    continue;
                }
                let _ = SetWindowPos(h, HWND_NOTOPMOST, 0, 0, 0, 0, flags);
                match target {
                    Some(t) if !t.minimized => {
                        // 插到"绑定窗口上面那个窗口"之后 = 紧贴绑定窗口之上。
                        let prev = GetWindow(HWND(t.hwnd), GW_HWNDPREV);
                        if prev.0 != 0 && prev != h {
                            let _ = SetWindowPos(h, prev, 0, 0, 0, 0, flags);
                        }
                    }
                    _ => {
                        let _ = SetWindowPos(h, HWND_BOTTOM, 0, 0, 0, 0, flags);
                    }
                }
            }
        }
        changed
    }

    /// 把前台焦点交还给定窗口(R7-26)。
    /// 辅助工具栏是独立无边框窗口, 用户点完「应用」后焦点留在工具栏,
    /// Plasticity 收不到键盘事件 → 主程序"任何操作都不生效"。
    /// 这里在应用完成后把焦点还给 Plasticity, 用户的下一步(shift+D / E / G)
    /// 可以直接按。
    pub fn focus_window(hwnd: isize) -> bool {
        if hwnd == 0 {
            return false;
        }
        unsafe {
            // AttachThreadInput: 工具栏与 Plasticity 属不同输入队列时,
            // SetForegroundWindow 可能被系统拒绝, 先挂上目标线程再调用。
            let target = HWND(hwnd);
            let fg = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(fg, None);
            let cur_thread = GetCurrentThreadId();
            let attached = fg_thread != cur_thread && fg_thread != 0;
            if attached {
                let _ = AttachThreadInput(cur_thread, fg_thread, true);
            }
            let ok = SetForegroundWindow(target).as_bool();
            if attached {
                let _ = AttachThreadInput(cur_thread, fg_thread, false);
            }
            ok
        }
    }
}

#[cfg(not(windows))]
mod imp {
    /// 非 Windows 平台占位: 永远未绑定(UI 仍可用, 只是没有吸附)。
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct TargetWindow {
        pub hwnd: isize,
        pub title: String,
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
        pub minimized: bool,
    }

    pub fn find_plasticity_windows() -> Vec<TargetWindow> {
        Vec::new()
    }

    pub fn own_root_window(_root_title: &str) -> Option<isize> {
        None
    }

    pub fn set_claim(_root: isize, _target: Option<isize>) {}

    pub fn bind_target(_current: Option<isize>, _root: Option<isize>) -> Option<TargetWindow> {
        None
    }

    pub fn target_is_active(_target: isize) -> bool {
        false
    }

    pub fn sync_topmost(_target: Option<&TargetWindow>, _active: bool) -> bool {
        false
    }

    /// 非 Windows 平台占位: 无焦点操作。
    pub fn focus_window(_hwnd: isize) -> bool {
        false
    }
}

pub use imp::*;

#[cfg(all(test, windows))]
mod tests {
    use super::decide_binding;

    #[test]
    fn binding_sticks_and_respects_claims() {
        let me = 100;
        // 已绑定 → 保持(即使不在前台、Z 序不在最前)
        assert_eq!(decide_binding(Some(2), &[1, 2], Some(1), &[], me), Some(2));
        // 首次: 优先未被认领的前台窗口
        assert_eq!(decide_binding(None, &[1, 2, 3], Some(3), &[], me), Some(3));
        // 前台窗口已被别人认领 → 取 Z 序最前的空闲窗口
        assert_eq!(decide_binding(None, &[1, 2, 3], Some(3), &[(3, 50)], me), Some(1));
        // 全被认领 → 不绑定
        assert_eq!(decide_binding(None, &[1, 2], Some(1), &[(1, 50), (2, 200)], me), None);
        // 同时认领冲突: 更早启动的实例(50 < 100)拿着同一窗口 → 我让出, 改认领空闲的
        assert_eq!(decide_binding(Some(1), &[1, 2], Some(1), &[(1, 50)], me), Some(2));
        // 更晚启动的实例(200)认领了我的窗口 → 我保留(它会让出)
        assert_eq!(decide_binding(Some(1), &[1, 2], Some(1), &[(1, 200)], me), Some(1));
        // 绑定窗口已关闭(current=None)→ 重新认领
        assert_eq!(decide_binding(None, &[2], None, &[], me), Some(2));
    }
}
