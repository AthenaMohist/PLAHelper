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

/// 探测命中结果(物理像素): `right` = 文件名块右缘(屏幕坐标系),
/// `cy` = 文件名块垂直中心(相对窗口顶) — 顶栏内容垂直居中,
/// 工具栏按它垂直居中可同时适配正常/最大化两种顶栏高度(R6-2i)。
pub struct ProbeHit {
    pub right: i32,
    pub cy: i32,
}

#[cfg(windows)]
mod imp {
    use super::ProbeHit;
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
        GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS, SRCCOPY,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, PW_RENDERFULLCONTENT};

    /// 探测目标窗口顶栏「文件名文字块」(物理像素)。
    /// `ppp` = egui pixels_per_point, 用于尺度相关阈值。
    pub fn probe_filename_right_edge(target: &crate::window_finder::TargetWindow, ppp: f32) -> Option<ProbeHit> {
        let ppp = if ppp.is_finite() && ppp >= 0.5 { ppp } else { 1.0 };
        unsafe { probe_impl(target, ppp) }
    }

    unsafe fn probe_impl(
        target: &crate::window_finder::TargetWindow,
        ppp: f32,
    ) -> Option<ProbeHit> {
        let hwnd = HWND(target.hwnd);
        // 与 wincap.rs 一致: 取客户区真实尺寸做全窗 PrintWindow, 再裁顶部条带。
        // 原因:
        //   1. Windows 10/11 下 GetWindowRect 含 ~8-10px 隐形缩放边框(最大化时 left/top 为 -8),
        //      而 Electron 隐藏标题栏窗口的可视顶栏原点在 ClientToScreen(0,0);
        //   2. DWM 的 PrintWindow(PW_RENDERFULLCONTENT) 对非完整窗口高度的兼容位图
        //      可能按比例纵向压缩整窗或偏移, 全尺寸渲染再 BitBlt 顶部条带可确保 1:1 像素准确。
        let mut rc = RECT::default();
        if GetClientRect(hwnd, &mut rc).is_err() {
            return None;
        }
        let width = rc.right - rc.left;
        let height = rc.bottom - rc.top;
        if width < 200 || height < 80 {
            return None;
        }
        let mut client_pt = POINT { x: 0, y: 0 };
        let (origin_x, origin_y) = if ClientToScreen(hwnd, &mut client_pt).as_bool() {
            (client_pt.x, client_pt.y)
        } else {
            (target.left, target.top)
        };

        let strip_h = ((88.0 * ppp).round() as i32).clamp(48, 160).min(height);
        let y0 = 2;
        let y1 = strip_h - 2;

        let hdc_screen = GetDC(None);
        if hdc_screen.is_invalid() {
            return None;
        }
        let result = (|| {
            let mem_full = CreateCompatibleDC(hdc_screen);
            if mem_full.is_invalid() {
                return None;
            }
            let bmp_full = CreateCompatibleBitmap(hdc_screen, width, height);
            if bmp_full.is_invalid() {
                let _ = DeleteDC(mem_full);
                return None;
            }
            let old_full = SelectObject(mem_full, bmp_full);
            let printed =
                PrintWindow(hwnd, mem_full, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
            let mut out = None;
            if printed {
                let mem_strip = CreateCompatibleDC(hdc_screen);
                if !mem_strip.is_invalid() {
                    let bmp_strip = CreateCompatibleBitmap(hdc_screen, width, strip_h);
                    if !bmp_strip.is_invalid() {
                        let old_strip = SelectObject(mem_strip, bmp_strip);
                        let blt_ok = BitBlt(
                            mem_strip, 0, 0, width, strip_h, mem_full, 0, 0, SRCCOPY,
                        )
                        .is_ok();
                        if blt_ok {
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
                                mem_strip,
                                bmp_strip,
                                0,
                                strip_h as u32,
                                Some(buf.as_mut_ptr().cast()),
                                &mut bmi,
                                DIB_RGB_COLORS,
                            );
                            if ok != 0 {
                                if std::env::var("PLA_PROBE_DEBUG").as_deref() == Ok("1") {
                                    let path = std::env::temp_dir().join("pla_probe_strip.bin");
                                    let _ = std::fs::write(
                                        &path,
                                        width
                                            .to_le_bytes()
                                            .iter()
                                            .chain(strip_h.to_le_bytes().iter())
                                            .chain(buf.iter())
                                            .copied()
                                            .collect::<Vec<u8>>(),
                                    );
                                    eprintln!("[probe] strip dump -> {}", path.display());
                                }
                                out = scan_filename_edge(&buf, width, strip_h, y0, y1, ppp).map(
                                    |(edge, y_top, y_bot)| ProbeHit {
                                        right: origin_x + edge,
                                        cy: (origin_y - target.top) + (y_top + y_bot + 1) / 2,
                                    },
                                );
                            }
                        }
                        SelectObject(mem_strip, old_strip);
                        let _ = DeleteObject(bmp_strip);
                    }
                    let _ = DeleteDC(mem_strip);
                }
            }
            SelectObject(mem_full, old_full);
            let _ = DeleteObject(bmp_full);
            let _ = DeleteDC(mem_full);
            out
        })();
        ReleaseDC(None, hdc_screen);
        result
    }

    /// 在 BGRA 条带像素里找文件名块: 返回 (右缘相对左缘, 块顶 y, 块底 y)。
    pub(crate) fn scan_filename_edge(
        buf: &[u8],
        width: i32,
        strip_h: i32,
        y0: i32,
        y1: i32,
        ppp: f32,
    ) -> Option<(i32, i32, i32)> {
        let w = width as usize;
        if w < 200 {
            return None;
        }
        let row = w * 4;
        let (ys, ye) = (y0.max(0) as usize, (y1 as usize).min(strip_h as usize));
        if ye <= ys {
            return None;
        }

        // 1) 探测顶栏上下边界 [title_ys..title_ye]:
        //    以顶栏中右段空白区(x = w/2 .. 3w/4)在顶栏垂直中段(y ≈ 12*ppp .. 20*ppp)
        //    的均值为基准背景色:
        //    - 向上扫描跳过可能存在的 Win11 窗口顶边框线;
        //    - 向下扫描精确截停在 3D 视口上缘(如浅灰视口 #cdcdc6, lum=203 vs 顶栏 #121212, lum=18),
        //      同时限制顶栏最大高度 ~48*ppp, 避免深色视口下混入第二行模式按钮。
        let x_blank_s = w / 2;
        let x_blank_e = (w * 3) / 4;
        let ref_ys = ((12.0 * ppp).round() as usize).clamp(ys + 2, ye.saturating_sub(6));
        let ref_ye = (ref_ys + 6).min(ye);
        let mut bg_sum = 0u64;
        let mut bg_cnt = 0u64;
        for y in ref_ys..ref_ye {
            for x in (x_blank_s..x_blank_e).step_by(4) {
                let o = y * row + x * 4;
                let lum = (u32::from(buf[o]) + u32::from(buf[o + 1]) + u32::from(buf[o + 2])) / 3;
                bg_sum += u64::from(lum);
                bg_cnt += 1;
            }
        }
        let mut title_ys = ys;
        let max_title_ye = (ys + ((48.0 * ppp).round() as usize)).min(ye);
        let mut title_ye = max_title_ye;
        if bg_cnt > 0 {
            let bg_lum = (bg_sum / bg_cnt) as i32;
            for y in (ys..ref_ys).rev() {
                let mut r_sum = 0u64;
                let mut r_cnt = 0u64;
                for x in (x_blank_s..x_blank_e).step_by(4) {
                    let o = y * row + x * 4;
                    let lum =
                        (u32::from(buf[o]) + u32::from(buf[o + 1]) + u32::from(buf[o + 2])) / 3;
                    r_sum += u64::from(lum);
                    r_cnt += 1;
                }
                if r_cnt > 0 && ((r_sum / r_cnt) as i32 - bg_lum).abs() > 12 {
                    title_ys = y + 1;
                    break;
                }
            }
            for y in ref_ye..ye {
                let mut r_sum = 0u64;
                let mut r_cnt = 0u64;
                for x in (x_blank_s..x_blank_e).step_by(4) {
                    let o = y * row + x * 4;
                    let lum =
                        (u32::from(buf[o]) + u32::from(buf[o + 1]) + u32::from(buf[o + 2])) / 3;
                    r_sum += u64::from(lum);
                    r_cnt += 1;
                }
                if r_cnt > 0 && ((r_sum / r_cnt) as i32 - bg_lum).abs() > 12 {
                    title_ye = y;
                    break;
                }
            }
            title_ye = title_ye.min((title_ys + ((48.0 * ppp).round() as usize)).min(ye));
            if title_ye <= title_ys + 8 {
                title_ys = ys;
                title_ye = max_title_ye;
            }
        }

        // 2) 自适应极性(仅在顶栏行区间 title_ys..title_ye 内统计, 不受视口背景干扰)。
        let mut dark = 0u32;
        let mut light = 0u32;
        for y in title_ys..title_ye {
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
        let text_is_light = dark > light;
        let lum_min: u32 = if text_is_light { 140 } else { 0 };
        let lum_max: u32 = if text_is_light { 255 } else { 130 };

        // 3) 在顶栏左半区(0..w/2)找出高度最大的连续文字/图标行带 [scan_ys..scan_ye],
        //    既能跳过顶部 1~2px 噪点, 又能彻底隔离第二行视口左上角按钮。
        let mut bands: Vec<(usize, usize)> = Vec::new();
        let mut cur_top: Option<usize> = None;
        let mut cur_bot: usize = 0;
        let mut empty_rows = 0usize;
        for y in title_ys..title_ye {
            let mut row_px = 0u32;
            for x in 0..(w / 2) {
                let o = y * row + x * 4;
                let lum = (u32::from(buf[o]) + u32::from(buf[o + 1]) + u32::from(buf[o + 2])) / 3;
                if lum >= lum_min && lum <= lum_max {
                    row_px += 1;
                }
            }
            // 排除整行几乎全亮的高亮边框线(row_px > w/4 不算文字行)
            if row_px >= 2 && (row_px as usize) < (w / 4) {
                if cur_top.is_none() {
                    cur_top = Some(y);
                }
                cur_bot = y;
                empty_rows = 0;
            } else if let Some(t) = cur_top {
                empty_rows += 1;
                if empty_rows >= 3 {
                    bands.push((t, cur_bot));
                    cur_top = None;
                    empty_rows = 0;
                }
            }
        }
        if let Some(t) = cur_top {
            bands.push((t, cur_bot));
        }
        let (scan_ys, scan_ye) = bands
            .into_iter()
            .filter(|(t, b)| b + 1 - t >= 5)
            .max_by_key(|(t, b)| b + 1 - t)
            .map(|(t, b)| (t, (b + 1).min(title_ye)))
            .unwrap_or((title_ys, title_ye));

        // 4) 仅在顶栏文字行带 [scan_ys..scan_ye] 内统计每列文字像素数。
        let mut cols = vec![0u32; w];
        for y in scan_ys..scan_ye {
            for x in 0..w {
                let o = y * row + x * 4;
                let lum = (u32::from(buf[o]) + u32::from(buf[o + 1]) + u32::from(buf[o + 2])) / 3;
                if lum >= lum_min && lum <= lum_max {
                    cols[x] += 1;
                }
            }
        }
        let band = (scan_ye - scan_ys) as u32;
        // 文字列判定: 该列至少 2px 文字像素。整条文字像素太少 → 空白帧/渲染失败。
        let text_total: u32 = cols.iter().map(|c| (*c).min(band)).sum();
        if text_total < w as u32 / 50 {
            return None;
        }
        // 块聚类: 文字列连续(间隙 < 14px 合并 — 文件名后的 ● 修改点与文字间隙小, 归入同块)。
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
        let (left, right) = blocks.into_iter().max_by_key(|(_, e)| *e)?;
        // 块的垂直范围: 在 [scan_ys..scan_ye] 内精确测量该块自身的上下界。
        let mut y_top: i32 = -1;
        let mut y_bot: i32 = -1;
        let mut empty_run = 0i32;
        for y in scan_ys..scan_ye {
            let mut px = 0u32;
            for x in left.max(0)..=(right.min(w as i32 - 1)) {
                let o = y * row + x as usize * 4;
                let lum = (u32::from(buf[o]) + u32::from(buf[o + 1]) + u32::from(buf[o + 2])) / 3;
                if lum >= lum_min && lum <= lum_max {
                    px += 1;
                    if px >= 2 {
                        break;
                    }
                }
            }
            if px >= 2 {
                if y_top < 0 {
                    y_top = y as i32;
                }
                y_bot = y as i32;
                empty_run = 0;
            } else if y_top >= 0 {
                empty_run += 1;
                if empty_run >= 3 {
                    break;
                }
            }
        }
        if y_top < 0 {
            return None;
        }
        Some((right + 1, y_top, y_bot))
    }
}

#[cfg(not(windows))]
mod imp {
    pub struct ProbeHit {
        pub right: i32,
        pub cy: i32,
    }

    pub fn probe_filename_right_edge(
        _target: &crate::window_finder::TargetWindow,
        _ppp: f32,
    ) -> Option<ProbeHit> {
        None
    }
}

pub use imp::probe_filename_right_edge;

#[cfg(all(test, windows))]
mod tests {
    use super::imp::scan_filename_edge;

    #[test]
    fn dark_titlebar_with_bright_viewport_does_not_bleed_filename_edge() {
        // 复刻 SS002.png 实测几何: 宽 1773, strip_h = 110 (ppp=1.25),
        // 顶栏 y=0..43 为深色 #100e0a, y=43..110 中间 x=284..1485 为浅灰 3D 视口 #cdcdc6,
        // 文件名文字块在 x=297..=407, y=14..=31 (#ffffff)。
        let w = 1773i32;
        let h = 110i32;
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let set_px = |buf: &mut [u8], x: i32, y: i32, r: u8, g: u8, b: u8| {
            let o = ((y * w + x) * 4) as usize;
            buf[o] = b;
            buf[o + 1] = g;
            buf[o + 2] = r;
            buf[o + 3] = 255;
        };
        // 1) 全图默认深底 #100e0a
        for y in 0..h {
            for x in 0..w {
                set_px(&mut buf, x, y, 0x10, 0x0e, 0x0a);
            }
        }
        // 2) y=43..110 3D 视口亮底 #cdcdc6 (旧算法在此处将 x=284..1772 全部算作文字列!)
        for y in 43..h {
            for x in 284..1485 {
                set_px(&mut buf, x, y, 0xcd, 0xcd, 0xc6);
            }
        }
        // 3) 顶栏窄图标(x=24..=39, x=217..=232) + 文件名块(x=297..=407, y=14..=31)
        for y in 14..=31 {
            for x in 24..=39 {
                set_px(&mut buf, x, y, 220, 220, 220);
            }
            for x in 217..=232 {
                set_px(&mut buf, x, y, 220, 220, 220);
            }
            for x in 297..=407 {
                set_px(&mut buf, x, y, 235, 235, 235);
            }
        }
        let hit = scan_filename_edge(&buf, w, h, 2, h - 2, 1.25).expect("应命中文件名块");
        assert_eq!(hit, (408, 14, 31));
    }
}
