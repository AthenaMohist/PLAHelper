//! 标题栏「文件名区」探测 — 吸附定位用(R6)。
//!
//! 背景: Plasticity 顶栏布局 = [logo][侧栏切换图标][文件名 ●][空白...]，
//! 助手工具栏应吸附在文件名文字块右侧。固定偏移在不同文件名长度下必然
//! 时准时不准(R5/R6 实测)，改为像素探测:
//!   1. `PrintWindow(PW_RENDERFULLCONTENT)` 渲染目标窗口自身内容到内存 DC
//!      — 渲染的是目标窗口的离屏内容，不受本工具栏(AlwaysOnTop 且就压在
//!      顶栏上)遮挡影响，也无需抓屏幕。
//!   2. 取顶部条带，自适应极性(深色主题=亮字 / 浅色主题=深字)做列扫描，
//!      聚类出文字/图标块。
//!   3. 文件名块判别: 在左半区(起点 < 窗口宽 50%)找「最靠右且足够宽
//!      (≥ 22×ppp, 图标没这么宽)」的块 — logo/侧栏图标都是窄块被自然过滤。
//!   4. 返回该块右缘(物理像素, GetWindowRect 坐标系) + 调用方加间隙。
//! 任何一步失败(渲染失败/空白帧/找不到文字块)返回 None，调用方降级固定偏移。
#![allow(clippy::upper_case_acronyms)]

/// 文件名块右缘与工具栏左缘的间隙(物理像素)。
pub const FILENAME_GAP_PX: f32 = 16.0;

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::PW_RENDERFULLCONTENT;

    /// 探测目标窗口顶栏「文件名文字块」右缘(物理像素, 屏幕坐标系)。
    /// `ppp` = egui pixels_per_point, 用于尺度相关阈值。
    pub fn probe_filename_right_edge(target: &crate::window_finder::TargetWindow, ppp: f32) -> Option<i32> {
        let ppp = if ppp.is_finite() && ppp >= 0.5 { ppp } else { 1.0 };
        unsafe { probe_impl(target, ppp) }
    }

    unsafe fn probe_impl(
        target: &crate::window_finder::TargetWindow,
        ppp: f32,
    ) -> Option<i32> {
        let width = target.width();
        if width < 200 {
            return None;
        }
        // 条带高度: 顶栏 ~36 逻辑 px, 随 DPI 缩放; 上限防呆。
        let strip_h = ((52.0 * ppp).round() as i32).clamp(32, 88);
        // 只扫描条带中段(避开上边框线和下缘渐变)。
        let y0 = 2;
        let y1 = strip_h - 2;

        let hwnd = HWND(target.hwnd);
        let hdc_screen = GetDC(None);
        if hdc_screen.is_invalid() {
            return None;
        }
        let result = (|| {
            let mem = CreateCompatibleDC(hdc_screen);
            if mem.is_invalid() {
                return None;
            }
            let bmp = CreateCompatibleBitmap(hdc_screen, width, strip_h);
            if bmp.is_invalid() {
                DeleteDC(mem);
                return None;
            }
            let old = SelectObject(mem, bmp);
            // PrintWindow 离屏渲染目标窗口(含 GPU 合成内容), DC 只有条带高,
            // 渲染原点在窗口左上 → 恰好得到顶部条带, 且不被本工具栏遮挡。
            let printed = PrintWindow(hwnd, mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
            let mut out = None;
            if printed {
                let mut bmi = BITMAPINFO::default();
                bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = width;
                bmi.bmiHeader.biHeight = -strip_h; // top-down
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 32;
                bmi.bmiHeader.biCompression = BI_RGB.0;
                let row = width as usize * 4;
                let mut buf = vec![0u8; row * strip_h as usize];
                let ok = GetDIBits(
                    mem,
                    bmp,
                    0,
                    strip_h as u32,
                    Some(buf.as_mut_ptr().cast()),
                    &mut bmi,
                    DIB_RGB_COLORS,
                );
                if ok != 0 {
                    // 调试出口: PLA_PROBE_DEBUG=1 时导出原始条带, 供离线对拍。
                    if std::env::var("PLA_PROBE_DEBUG").as_deref() == Ok("1") {
                        let path = std::env::temp_dir().join("pla_probe_strip.bin");
                        let _ = std::fs::write(
                            &path,
                            width.to_le_bytes().iter().chain(strip_h.to_le_bytes().iter()).chain(buf.iter()).copied().collect::<Vec<u8>>(),
                        );
                        eprintln!("[probe] strip dump -> {}", path.display());
                    }
                    out = scan_filename_edge(&buf, width, strip_h, y0, y1, ppp)
                        .map(|edge| target.left + edge);
                }
            }
            SelectObject(mem, old);
            let _ = DeleteObject(bmp);
            let _ = DeleteDC(mem);
            out
        })();
        ReleaseDC(None, hdc_screen);
        result
    }

    /// 在 BGRA 条带像素里找文件名块右缘(相对窗口左缘, 物理像素)。
    fn scan_filename_edge(
        buf: &[u8],
        width: i32,
        strip_h: i32,
        y0: i32,
        y1: i32,
        ppp: f32,
    ) -> Option<i32> {
        let w = width as usize;
        let row = w * 4;
        let (ys, ye) = (y0.max(0) as usize, (y1 as usize).min(strip_h as usize));
        if ye <= ys {
            return None;
        }
        // 自适应极性: 统计条带内暗/亮像素占比, 文字 = 少数派那一侧。
        let mut dark = 0u32;
        let mut light = 0u32;
        for y in ys..ye {
            for x in (0..w).step_by(2) {
                let o = y * row + x * 4;
                let lum = (u32::from(buf[o]) + u32::from(buf[o + 1]) + u32::from(buf[o + 2])) / 3;
                if lum < 100 {
                    dark += 1;
                } else if lum > 170 {
                    light += 1;
                }
            }
        }
        if dark + light == 0 {
            return None; // 全灰? 无法分辨
        }
        // 极性: 背景像素永远占多数 → 暗像素多 = 深底亮字, 亮像素多 = 浅底深字。
        // (R6 实测修正: 最初写成 light >= dark, 在深色主题下把背景当文字, 全盘反了)
        let text_is_light = dark > light;
        let lum_min: u32 = if text_is_light { 140 } else { 0 };
        let lum_max: u32 = if text_is_light { 255 } else { 130 };

        // 每列文字像素数。
        let mut cols = vec![0u32; w];
        for y in ys..ye {
            for x in 0..w {
                let o = y * row + x * 4;
                let lum = (u32::from(buf[o]) + u32::from(buf[o + 1]) + u32::from(buf[o + 2])) / 3;
                if lum >= lum_min && lum <= lum_max {
                    cols[x] += 1;
                }
            }
        }
        let band = (ye - ys) as u32;
        // 文字列判定: 该列至少 2px 文字像素。整条文字像素太少 → 空白帧/渲染失败。
        let text_total: u32 = cols.iter().map(|c| (*c).min(band)).sum();
        if text_total < w as u32 / 50 {
            return None;
        }
        // 块聚类: 文字列连续(间隙 < 10px 合并 — 文件名后的 ● 修改点与文字间隙小, 归入同块)。
        let min_block_w = (30.0 * ppp).round() as i32; // logo/窄图标被过滤, 文字块远超此值
        let merge_gap = 14i32;
        let mut blocks: Vec<(i32, i32)> = Vec::new();
        let mut start: Option<i32> = None;
        let mut gap = 0i32;
        for x in 0..w as i32 {
            if cols[x as usize] >= 2 {
                if start.is_none() {
                    start = Some(x);
                }
                gap = 0;
            } else if let Some(s) = start {
                gap += 1;
                if gap >= merge_gap {
                    blocks.push((s, x - gap));
                    start = None;
                    gap = 0;
                }
            }
        }
        if let Some(s) = start {
            blocks.push((s, w as i32 - 1));
        }
        // 文件名块: 起点 < 窗口宽 50%, 宽度 ≥ min_block_w, 取其中最靠右者。
        blocks.retain(|(s, e)| *s < w as i32 / 2 && e - s + 1 >= min_block_w);
        let (_, right) = blocks.into_iter().max_by_key(|(_, e)| *e)?;
        Some(right + 1)
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn probe_filename_right_edge(
        _target: &crate::window_finder::TargetWindow,
        _ppp: f32,
    ) -> Option<i32> {
        None
    }
}

pub use imp::probe_filename_right_edge;
