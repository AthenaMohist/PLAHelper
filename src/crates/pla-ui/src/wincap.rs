//! Plasticity 主窗口截图 → PNG(快照墙缩略图, R7-5)。
//!
//! 复用 titlebar_probe 的 PrintWindow(PW_RENDERFULLCONTENT) 离屏渲染通道 —
//! 渲染目标窗口自身内容, 不受本工具栏(AlwaysOnTop)遮挡。
//! PNG 编码零依赖手写: RGBA 行滤波 0 + zlib 存储块(不经压缩, 体积大但
//! 实现可靠, 截图用途可接受) + CRC32/Adler32。

/// BGRA(32bpp BI_RGB)缓冲编码为 PNG 文件字节(RGBA8)。
pub fn encode_png_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let expected = width as usize * height as usize * 4;
    debug_assert_eq!(rgba.len(), expected);

    // raw: 每行前缀 filter byte(0 = None)
    let row_bytes = width as usize * 4;
    let mut raw = Vec::with_capacity((row_bytes + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0u8);
        raw.extend_from_slice(&rgba[y * row_bytes..(y + 1) * row_bytes]);
    }

    // zlib 流: 0x78 0x01 + 存储块(每块 ≤65535) + adler32(raw)
    let mut z = vec![0x78u8, 0x01];
    let mut i = 0usize;
    while i < raw.len() {
        let n = (raw.len() - i).min(65535);
        let last = i + n >= raw.len();
        z.push(if last { 1 } else { 0 });
        z.extend_from_slice(&(n as u16).to_le_bytes());
        z.extend_from_slice(&(!(n as u16)).to_le_bytes());
        z.extend_from_slice(&raw[i..i + n]);
        i += n;
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = vec![0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    // IHDR: 宽, 高, 位深8, 颜色类型6(RGBA), 压缩0, 滤波0, 隔行0
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    png_chunk(&mut png, b"IHDR", &ihdr);
    png_chunk(&mut png, b"IDAT", &z);
    png_chunk(&mut png, b"IEND", &[]);
    png
}

fn png_chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ty);
    out.extend_from_slice(data);
    // 标准 CRC32: 初值 FFFF_FFFF 连续喂 ty+data, 末尾异或 FFFF_FFFF
    let mut crc = crc32_update(0xFFFF_FFFF, ty);
    crc = crc32_update(crc, data);
    out.extend_from_slice(&(crc ^ 0xFFFF_FFFF).to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    crc32_update(0xFFFF_FFFF, data) ^ 0xFFFF_FFFF
}

fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    // 无表实现(一次一位, 调用频率低, 不追求速度)
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// PrintWindow 抓取整窗内容 → PNG 落盘。
#[cfg(windows)]
pub fn capture_window_png(hwnd: isize, out: &std::path::Path) -> Result<(), String> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, IsWindow, PW_RENDERFULLCONTENT};

    unsafe {
        let hwnd = HWND(hwnd);
        if !IsWindow(hwnd).as_bool() {
            return Err("Plasticity 窗口已关闭".into());
        }
        let mut rc = RECT::default();
        GetClientRect(hwnd, &mut rc)
            .ok()
            .ok_or_else(|| "GetClientRect 失败".to_string())?;
        let w = (rc.right - rc.left).max(1);
        let h = (rc.bottom - rc.top).max(1);

        let hdc_screen = GetDC(None);
        if hdc_screen.is_invalid() {
            return Err("GetDC 失败".into());
        }
        let result = (|| {
            let mem = CreateCompatibleDC(hdc_screen);
            if mem.is_invalid() {
                return Err("CreateCompatibleDC 失败".into());
            }
            let captured = (|| {
                let bmp = CreateCompatibleBitmap(hdc_screen, w, h);
                if bmp.is_invalid() {
                    return Err("CreateCompatibleBitmap 失败".into());
                }
                let old = SelectObject(mem, bmp);
                let r = (|| {
                    let printed =
                        PrintWindow(hwnd, mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
                    if !printed {
                        return Err("PrintWindow 失败".into());
                    }
                    let mut bmi = BITMAPINFO::default();
                    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                    bmi.bmiHeader.biWidth = w;
                    bmi.bmiHeader.biHeight = -h; // top-down
                    bmi.bmiHeader.biPlanes = 1;
                    bmi.bmiHeader.biBitCount = 32;
                    bmi.bmiHeader.biCompression = BI_RGB.0;
                    let row = w as usize * 4;
                    let mut buf = vec![0u8; row * h as usize];
                    let ok = GetDIBits(
                        mem,
                        bmp,
                        0,
                        h as u32,
                        Some(buf.as_mut_ptr().cast()),
                        &mut bmi,
                        DIB_RGB_COLORS,
                    );
                    if ok == 0 {
                        return Err("GetDIBits 失败".into());
                    }
                    // BGRA → RGBA, alpha 拍实(PrintWindow 的 alpha 通道不可靠)
                    for px in buf.chunks_exact_mut(4) {
                        px.swap(0, 2);
                        px[3] = 255;
                    }
                    let png = encode_png_rgba(w as u32, h as u32, &buf);
                    std::fs::write(out, png).map_err(|e| format!("写 PNG: {e}"))?;
                    Ok(())
                })();
                SelectObject(mem, old);
                let _ = DeleteObject(bmp);
                r
            })();
            let _ = DeleteDC(mem);
            captured
        })();
        ReleaseDC(None, hdc_screen);
        result
    }
}

#[cfg(not(windows))]
pub fn capture_window_png(_hwnd: isize, _out: &std::path::Path) -> Result<(), String> {
    Err("仅支持 Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PNG 编码器自检: 输出必须是合法 PNG 签名 + 可解析的块结构。
    /// zlib 存储块可被 Python zlib 直接解压(与 measure 脚本同源验证法)。
    #[test]
    fn encode_png_structure_valid() {
        let (w, h) = (4u32, 3u32);
        let px: Vec<u8> = (0..w * h * 4).map(|i| (i % 251) as u8).collect();
        let png = encode_png_rgba(w, h, &px);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        // 遍历块: IHDR(13B) → IDAT → IEND, CRC 自洽
        let mut off = 8usize;
        let mut seen = Vec::new();
        while off + 8 <= png.len() {
            let len = u32::from_be_bytes(png[off..off + 4].try_into().unwrap()) as usize;
            let ty = &png[off + 4..off + 8];
            let data = &png[off + 8..off + 8 + len];
            let crc = u32::from_be_bytes(png[off + 8 + len..off + 12 + len].try_into().unwrap());
            // 标准 CRC32: 初值 FFFF_FFFF 连续喂 ty+data, 末尾异或(与编码器同法独立验证)
            let mut c = crc32_update(0xFFFF_FFFF, ty);
            c = crc32_update(c, data);
            assert_eq!(crc, c ^ 0xFFFF_FFFF, "chunk {ty:?} CRC 不自洽");
            seen.push(String::from_utf8_lossy(ty).to_string());
            off += 12 + len;
        }
        assert_eq!(seen, ["IHDR", "IDAT", "IEND"]);
        // IDAT 位置: IHDR 块起于 8, 数据 13 字节 → IDAT len 字段在 8+12+13
        let idat_len_off = 8 + 12 + 13;
        let idat_len = u32::from_be_bytes(png[idat_len_off..idat_len_off + 4].try_into().unwrap()) as usize;
        let idat = &png[idat_len_off + 8..idat_len_off + 8 + idat_len];
        assert_eq!(idat[0], 0x78, "zlib CMF");
        let raw = zlib_store_decompress(idat);
        assert_eq!(raw.len(), (row_bytes_of(w) as usize + 1) * h as usize);
        // adler32 校验
        let adler = u32::from_be_bytes(raw_raw_last(idat).try_into().unwrap());
        assert_eq!(adler, adler32(&raw));
    }

    fn row_bytes_of(w: u32) -> u32 {
        w * 4
    }
    fn raw_raw_last(idat: &[u8]) -> [u8; 4] {
        idat[idat.len() - 4..].try_into().unwrap()
    }
    /// 解 zlib 存储块(测试辅助, 只解本编码器产出的格式)。
    fn zlib_store_decompress(z: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 2usize; // CMF/FLG
        loop {
            let last = z[i] == 1;
            let n = u16::from_le_bytes(z[i + 1..i + 3].try_into().unwrap()) as usize;
            out.extend_from_slice(&z[i + 5..i + 5 + n]);
            i += 5 + n;
            if last {
                break;
            }
        }
        out
    }
}
