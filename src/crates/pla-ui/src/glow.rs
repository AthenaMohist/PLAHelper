//! 面板内侧羽化光晕。
//!
//! 为什么不用多圈描边: 离散 N 圈描边有肉眼可见的色带(「框中框」, R6-2e
//! 用户实测否决); 为什么不用 Frame::shadow: glow 后端透明窗口不可靠,
//! 外侧光晕以黑为底合成 = 半透明黑框(R6-2d 教训)。
//!
//! 做法: 沿圆角矩形路径取内外多环顶点, 顶点色带 alpha, 交给 GPU 在环带
//! 三角网格上线性插值 — 连续渐变, 无色带。环序列 (内缩距离, alpha) 首环
//! 贴边最亮、末环 alpha=0, 两环之间再线性插值一次, 曲面近似二次衰减。

use std::f32::consts::FRAC_PI_2;

use egui::{Color32, Mesh, Pos2, Rect};

/// 每个圆角的采样段数。4 角 × (SEG+1) 点 = 每环 44 点, 足够圆滑。
const SEG: usize = 10;

/// 圆角矩形路径: 顺时针, 从左上角圆弧起点开始, 每个圆角 SEG 段。
/// 直边不插中间点(两环点数按构造一致, 直边段三角形自然连接)。
fn rounded_rect_path(rect: Rect, r: f32, seg: usize) -> Vec<Pos2> {
    let r = r.clamp(0.0, rect.width().min(rect.height()) / 2.0);
    let (l, t, ri, b) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    let mut pts = Vec::with_capacity(4 * (seg + 1));
    let arc = |cx: f32, cy: f32, a0: f32, out: &mut Vec<Pos2>| {
        for i in 0..=seg {
            let a = a0 + FRAC_PI_2 * (i as f32) / (seg as f32);
            out.push(Pos2::new(cx + r * a.cos(), cy + r * a.sin()));
        }
    };
    // 四角圆心角: TL 180°..270°, TR 270°..360°, BR 0°..90°, BL 90°..180°。
    // TL: 角 180°→270°
    arc(l + r, t + r, std::f32::consts::PI, &mut pts);
    // TR: 角 270°(=3π/2)→360°(=2π)
    arc(ri - r, t + r, 3.0 * FRAC_PI_2, &mut pts);
    // BR: 角 0°→90°
    arc(ri - r, b - r, 0.0, &mut pts);
    // BL: 角 90°→180°
    arc(l + r, b - r, FRAC_PI_2, &mut pts);
    pts
}

/// 在 mesh 末尾追加内侧羽化光晕。
///
/// * `rect`      — 面板可见矩形(光晕画在其内侧)。
/// * `rounding`  — 面板圆角半径。
/// * `rings`     — (内缩距离 pt, alpha 0..255) 递增序列, 末环 alpha 必须 0。
/// * `rgb`       — 光晕颜色(奶白系)。
pub fn append_inner_glow(mesh: &mut Mesh, rect: Rect, rounding: f32, rings: &[(f32, u8)], rgb: [u8; 3]) {
    let color = |a: u8| Color32::from_rgba_premultiplied(
        ((rgb[0] as u16 * a as u16) / 255) as u8,
        ((rgb[1] as u16 * a as u16) / 255) as u8,
        ((rgb[2] as u16 * a as u16) / 255) as u8,
        a,
    );
    for w in rings.windows(2) {
        let (da, aa) = (w[0].0, w[0].1);
        let (db, ab) = (w[1].0, w[1].1);
        if (db - da).abs() < 0.01 {
            continue;
        }
        let pa = rounded_rect_path(rect.shrink(da), (rounding - da).max(0.0), SEG);
        let pb = rounded_rect_path(rect.shrink(db), (rounding - db).max(0.0), SEG);
        let n = pa.len();
        debug_assert_eq!(n, pb.len());
        let base = mesh.vertices.len() as u32;
        for p in &pa {
            mesh.colored_vertex(*p, color(aa));
        }
        for p in &pb {
            mesh.colored_vertex(*p, color(ab));
        }
        // 闭环三角带: 外环 i→j 与内环 i→j 连两个三角形。
        for i in 0..n as u32 {
            let j = (i + 1) % n as u32;
            let (ai, aj, bi, bj) = (base + i, base + j, base + n as u32 + i, base + n as u32 + j);
            mesh.add_triangle(ai, aj, bj);
            mesh.add_triangle(ai, bj, bi);
        }
    }
}
