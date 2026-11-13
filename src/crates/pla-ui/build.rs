//! R28: 把 `assets/pla-helper.ico` 作为 Windows 图标资源嵌进 pla-ui.exe
//! (资源管理器 / 开始菜单 / 快捷方式显示的图标来自 exe 资源)。
//!
//! 本机工具链是 x86_64-pc-windows-gnu 且没有 windres / rc.exe, 所以不用 embed-resource 之类的 crate,
//! 直接在这里生成一个只含 `.rsrc` 节的 COFF 目标文件(windres 的产物就是这个格式), 交给链接器合并。
//! 资源树: RT_ICON(3) 每个尺寸一项 + RT_GROUP_ICON(14) id=1; 语言 0x0409。
//! ICO 里的 PNG 条目原样作为 RT_ICON 数据(Vista 起支持)。
//! 运行时窗口(任务栏)图标另由 main.rs 用 `with_icon` 设置, 不依赖这里。

use std::path::PathBuf;

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
const LANG: u32 = 0x0409;

fn main() {
    let ico = PathBuf::from("assets/pla-helper.ico");
    println!("cargo:rerun-if-changed={}", ico.display());
    println!("cargo:rerun-if-changed=build.rs");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    // 只在 Windows GNU x86_64 上嵌入(MSVC 链接器不直接吃带 .rsrc 的 COFF; 需要时再加 rc 路线)。
    if target_os != "windows" || target_env != "gnu" || target_arch != "x86_64" {
        return;
    }
    let bytes = match std::fs::read(&ico) {
        Ok(b) => b,
        Err(e) => {
            println!("cargo:warning=pla-helper.ico 读取失败, 不嵌图标: {e}");
            return;
        }
    };
    let images = match parse_ico(&bytes) {
        Some(v) if !v.is_empty() => v,
        _ => {
            println!("cargo:warning=pla-helper.ico 解析失败, 不嵌图标");
            return;
        }
    };
    let obj = build_rsrc_coff(&images);
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("pla-helper-icon.o");
    std::fs::write(&out, obj).expect("write icon object");
    // 只链进 bin(lib 的测试二进制不需要)。
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

/// ICO 条目: (宽, 高, 色深, 数据)。
struct IcoImage {
    w: u8,
    h: u8,
    bits: u16,
    data: Vec<u8>,
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn parse_ico(b: &[u8]) -> Option<Vec<IcoImage>> {
    if b.len() < 6 || u16le(b, 0) != 0 || u16le(b, 2) != 1 {
        return None;
    }
    let n = u16le(b, 4) as usize;
    let mut v = Vec::with_capacity(n);
    for i in 0..n {
        let e = 6 + 16 * i;
        if e + 16 > b.len() {
            return None;
        }
        let size = u32le(b, e + 8) as usize;
        let off = u32le(b, e + 12) as usize;
        if off + size > b.len() {
            return None;
        }
        v.push(IcoImage { w: b[e], h: b[e + 1], bits: u16le(b, e + 6).max(32), data: b[off..off + size].to_vec() });
    }
    Some(v)
}

fn align(v: &mut Vec<u8>, a: usize) {
    while v.len() % a != 0 {
        v.push(0);
    }
}

/// 生成 COFF(x86-64) 目标文件: 1 个 `.rsrc` 节 + ADDR32NB 重定位(数据项里的 RVA) + 1 个节符号。
fn build_rsrc_coff(images: &[IcoImage]) -> Vec<u8> {
    let n = images.len();
    // ---- 1) 资源节布局: 目录(根/类型/名/语言) → 数据项 → 数据 ----
    // 根目录: 2 个类型项。类型目录: RT_ICON n 项, RT_GROUP_ICON 1 项。语言目录: 每个资源 1 项。
    let dir = |entries: usize| 16 + 8 * entries;
    let root_off = 0;
    let icon_type_off = root_off + dir(2);
    let group_type_off = icon_type_off + dir(n);
    let mut lang_offs = Vec::new(); // n 个图标 + 1 个组
    let mut cur = group_type_off + dir(1);
    for _ in 0..=n {
        lang_offs.push(cur);
        cur += dir(1);
    }
    let data_entry_base = cur; // n+1 个 16 字节数据项
    cur += 16 * (n + 1);

    // 组图标数据(GRPICONDIR)
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(n as u16).to_le_bytes());
    for (i, im) in images.iter().enumerate() {
        group.push(im.w);
        group.push(im.h);
        group.push(0);
        group.push(0);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&im.bits.to_le_bytes());
        group.extend_from_slice(&(im.data.len() as u32).to_le_bytes());
        group.extend_from_slice(&((i + 1) as u16).to_le_bytes());
    }

    // 数据块偏移(8 字节对齐)
    let mut blobs: Vec<&[u8]> = images.iter().map(|i| i.data.as_slice()).collect();
    blobs.push(&group);
    let mut blob_offs = Vec::new();
    for b in &blobs {
        cur = (cur + 7) & !7;
        blob_offs.push(cur);
        cur += b.len();
    }

    let mut s = Vec::with_capacity(cur);
    let mut relocs: Vec<u32> = Vec::new(); // 需要 ADDR32NB 的字段偏移
    let write_dir = |s: &mut Vec<u8>, entries: &[(u32, u32)]| {
        s.extend_from_slice(&[0u8; 12]); // Characteristics, TimeDateStamp, Major/Minor
        s.extend_from_slice(&0u16.to_le_bytes()); // named
        s.extend_from_slice(&(entries.len() as u16).to_le_bytes()); // id
        for (id, off) in entries {
            s.extend_from_slice(&id.to_le_bytes());
            s.extend_from_slice(&off.to_le_bytes());
        }
    };
    const SUBDIR: u32 = 0x8000_0000;
    // 根
    write_dir(
        &mut s,
        &[(RT_ICON, SUBDIR | icon_type_off as u32), (RT_GROUP_ICON, SUBDIR | group_type_off as u32)],
    );
    // RT_ICON: id 1..n → 语言目录
    let icon_entries: Vec<(u32, u32)> =
        (0..n).map(|i| ((i + 1) as u32, SUBDIR | lang_offs[i] as u32)).collect();
    write_dir(&mut s, &icon_entries);
    // RT_GROUP_ICON: id 1
    write_dir(&mut s, &[(1, SUBDIR | lang_offs[n] as u32)]);
    // 语言目录 → 数据项(叶子, 无 SUBDIR 位)
    for i in 0..=n {
        write_dir(&mut s, &[(LANG, (data_entry_base + 16 * i) as u32)]);
    }
    // 数据项
    for i in 0..=n {
        relocs.push(s.len() as u32);
        s.extend_from_slice(&(blob_offs[i] as u32).to_le_bytes()); // 节内偏移, 由 ADDR32NB 重定位加上节 RVA
        s.extend_from_slice(&(blobs[i].len() as u32).to_le_bytes());
        s.extend_from_slice(&0u32.to_le_bytes()); // CodePage
        s.extend_from_slice(&0u32.to_le_bytes()); // Reserved
    }
    for (i, b) in blobs.iter().enumerate() {
        align(&mut s, 8);
        debug_assert_eq!(s.len(), blob_offs[i]);
        s.extend_from_slice(b);
    }
    align(&mut s, 4);

    // ---- 2) COFF 外壳 ----
    let hdr = 20usize;
    let sec_hdr = 40usize;
    let raw_ptr = hdr + sec_hdr;
    let reloc_ptr = raw_ptr + s.len();
    let sym_ptr = reloc_ptr + 10 * relocs.len();
    let mut o = Vec::new();
    o.extend_from_slice(&0x8664u16.to_le_bytes()); // Machine: AMD64
    o.extend_from_slice(&1u16.to_le_bytes()); // NumberOfSections
    o.extend_from_slice(&0u32.to_le_bytes()); // TimeDateStamp
    o.extend_from_slice(&(sym_ptr as u32).to_le_bytes()); // PointerToSymbolTable
    o.extend_from_slice(&1u32.to_le_bytes()); // NumberOfSymbols
    o.extend_from_slice(&0u16.to_le_bytes()); // SizeOfOptionalHeader
    o.extend_from_slice(&0u16.to_le_bytes()); // Characteristics
    // 节头
    o.extend_from_slice(b".rsrc\0\0\0");
    o.extend_from_slice(&0u32.to_le_bytes()); // VirtualSize
    o.extend_from_slice(&0u32.to_le_bytes()); // VirtualAddress
    o.extend_from_slice(&(s.len() as u32).to_le_bytes()); // SizeOfRawData
    o.extend_from_slice(&(raw_ptr as u32).to_le_bytes()); // PointerToRawData
    o.extend_from_slice(&(reloc_ptr as u32).to_le_bytes()); // PointerToRelocations
    o.extend_from_slice(&0u32.to_le_bytes()); // PointerToLinenumbers
    o.extend_from_slice(&(relocs.len() as u16).to_le_bytes()); // NumberOfRelocations
    o.extend_from_slice(&0u16.to_le_bytes()); // NumberOfLinenumbers
    o.extend_from_slice(&0xC000_0040u32.to_le_bytes()); // INITIALIZED_DATA | MEM_READ | MEM_WRITE(同 windres)
    debug_assert_eq!(o.len(), raw_ptr);
    o.extend_from_slice(&s);
    // 重定位: VirtualAddress(节内偏移) + SymbolTableIndex(0 = .rsrc 节符号) + Type(3 = IMAGE_REL_AMD64_ADDR32NB)
    for r in &relocs {
        o.extend_from_slice(&r.to_le_bytes());
        o.extend_from_slice(&0u32.to_le_bytes());
        o.extend_from_slice(&3u16.to_le_bytes());
    }
    // 符号表: ".rsrc" 静态节符号
    o.extend_from_slice(b".rsrc\0\0\0");
    o.extend_from_slice(&0u32.to_le_bytes()); // Value
    o.extend_from_slice(&1i16.to_le_bytes()); // SectionNumber
    o.extend_from_slice(&0u16.to_le_bytes()); // Type
    o.push(3); // StorageClass: STATIC
    o.push(0); // NumberOfAuxSymbols
    // 字符串表(空, 只有 4 字节长度)
    o.extend_from_slice(&4u32.to_le_bytes());
    o
}
