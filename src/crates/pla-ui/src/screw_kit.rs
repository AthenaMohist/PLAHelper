//! N5 打孔面板「放螺丝」: 选型兼容性 + 螺丝长度 + 摆放计划 + 认 asset pack 里的现成件(纯函数, 单测)。
//! 用户 2026-11-18: 第一行选标准(国标各种紧固件 / 头型), 第二行选规格(长度、垫圈 / 弹垫 / 螺母), 再往下是孔参数;
//! 只有通孔(过孔)且长度够才能配螺母, 盲孔不能配螺母; 长度按所选面算, 加了垫圈 / 弹垫要扣掉它们的厚度。
//! 尺寸全部来自 [`crate::fasteners`] 的目录(pack 里不再放负体)。
//!
//! 深度坐标: 沿孔轴(= 入口面外法向的反向)从孔口量, 孔里为正、面外为负。

use crate::fasteners::{self, pitch, Head};
use pla_bridge::t;

/// 界面上的选择。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Choice {
    /// 目录里的螺钉标准 id(fasteners::SCREWS)
    pub std: String,
    pub d: f64,
    /// 手选的公称长度(None = 自动)
    pub length: Option<f64>,
    /// 头下平垫圈标准(None = 不加)
    pub washer: Option<String>,
    /// 头下弹垫
    pub spring: bool,
    /// 螺母标准(None = 不配)
    pub nut: Option<String>,
    /// 螺母下平垫圈(标准同头下的; 头下没选就用 GB/T 97.1)
    pub nut_washer: bool,
    pub nut_spring: bool,
}

/// 孔的情况(来自打孔表单 + 读到的厚度)。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HoleCtx {
    /// 单件孔的用途 = 螺纹底孔(螺丝拧进这一件)
    pub tap: bool,
    pub through: bool,
    /// 组合孔: 前几件过孔, 最后一件螺纹
    pub combo: bool,
    /// 入口实体沿孔轴的厚度
    pub t_entry: Option<f64>,
    /// 组合孔: 最后一件的入口深度(= 被夹紧的总厚)
    pub t0_last: Option<f64>,
    /// 组合孔: 最后一件的厚度(通孔时用)
    pub t_last: Option<f64>,
    /// 盲孔深度(单件 = 从孔口; 组合 = 最后一件里)
    pub depth: Option<f64>,
    /// 沉孔深度(圆柱头沉下去; 没有 = 0)
    pub cb_depth: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Screw,
    Washer,
    Nut,
}

/// 一件要放的东西: 标准 + 规格(+ 螺钉长度), 位置 = 深度 z(沿孔轴), up = 朝面外(+n)摆。
/// 资产 / 宏的约定: 资产 pivot = 贴合面中心、+Z = 朝外(螺钉 = 杆 → 头); 放资产时 normal = up ? n : −n;
/// 宏的 axis: 垫圈 / 螺母 = 同 normal; 螺钉 = 反过来(从头指向杆端)。
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub std: &'static str,
    pub d: f64,
    pub length: Option<f64>,
    pub z: f64,
    pub up: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// 这个规格可选的公称长度
    pub lengths: Vec<f64>,
    /// 自动算出的长度
    pub auto: Option<f64>,
    /// 实际用的长度(手选优先)
    pub length: Option<f64>,
    /// 至少要多长 / 最多多长(给界面显示)
    pub need: Option<f64>,
    pub max: Option<f64>,
    /// 能不能配螺母(不能时 nut_why 说为什么)
    pub nut_ok: bool,
    pub nut_why: Option<String>,
    /// 头下垫圈 / 弹垫允许吗(沉头 / 紧定不行)
    pub washers_ok: bool,
    /// 说明(灰字)与警告(红字)
    pub info: Vec<String>,
    pub warn: Vec<String>,
    /// 摆放
    pub items: Vec<Item>,
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn smallest_at_least(ls: &[f64], need: f64, max: Option<f64>) -> Option<f64> {
    ls.iter().copied().find(|l| *l >= need - 1e-9 && max.is_none_or(|m| *l <= m + 1e-9))
}
fn largest_at_most(ls: &[f64], max: f64) -> Option<f64> {
    ls.iter().copied().rev().find(|l| *l <= max + 1e-9)
}

/// 选型 + 长度 + 摆放。
pub fn plan(c: &Choice, h: &HoleCtx) -> Plan {
    let mut p = Plan::default();
    let Some(st) = fasteners::screw_std(&c.std) else {
        p.warn.push(t!("未知标准 {}", "Unknown standard {}", c.std));
        return p;
    };
    if st.row(c.d).is_none() {
        p.warn.push(t!("{} 没有 M{}", "{} has no M{}", st.code, c.d));
        return p;
    }
    let d = c.d;
    let pp = pitch(d);
    p.lengths = st.lengths(d);
    let csk = st.head == Head::Csk;
    let set = st.head == Head::Set;
    // ---- 兼容性 ----
    p.washers_ok = !csk && !set;
    let washer = c.washer.as_deref().filter(|_| p.washers_ok).and_then(fasteners::washer_std).filter(|w| !w.spring);
    let spring = c.spring && p.washers_ok;
    if !p.washers_ok && (c.washer.is_some() || c.spring) {
        p.info.push(if csk { t!("沉头螺钉不加头下垫圈", "No washer under a countersunk head") } else { t!("紧定螺钉没有头, 不加垫圈", "Set screws take no washer") });
    }
    if csk && h.cb_depth > 0.0 {
        p.warn.push(t!("沉头螺钉应配锥形沉孔(现在是圆柱沉孔)", "A countersunk screw wants a countersink (the hole has a counterbore)"));
    }
    p.nut_why = if set {
        Some(t!("紧定螺钉不配螺母", "Set screws take no nut"))
    } else if h.combo {
        Some(t!("组合孔的最后一件是螺纹孔, 不配螺母", "A stacked hole ends in a tapped part, no nut"))
    } else if h.tap {
        Some(t!("螺纹孔不配螺母", "Tapped hole, no nut"))
    } else if !h.through {
        Some(t!("盲孔不能配螺母", "Blind hole, no nut"))
    } else {
        None
    };
    let nut_std = c.nut.as_deref().and_then(fasteners::nut_std).filter(|_| p.nut_why.is_none());
    let nut_row = nut_std.and_then(|n| n.row(d));
    if nut_std.is_some() && nut_row.is_none() {
        p.warn.push(t!("{} 没有 M{d}", "{} has no M{d}", nut_std.map_or("", |n| n.code)));
    }
    let nut_on = nut_row.is_some();
    let wash_id = washer.map_or("gb97.1", |w| w.id);
    let nut_washer = fasteners::washer_std(wash_id).filter(|_| nut_on && c.nut_washer);
    let nut_spring = nut_on && c.nut_spring && !nut_std.is_some_and(|n| n.nylon);
    if nut_on && c.nut_spring && !nut_spring {
        p.info.push(t!("尼龙锁紧螺母不加弹垫", "No spring washer with a nylon lock nut"));
    }
    // ---- 厚度 ----
    let th = |w: Option<&fasteners::WasherStd>| w.and_then(|w| w.row(d)).map_or(0.0, |r| r.3);
    let spring_std = fasteners::washer_std("gb93");
    let hf = th(washer);
    let hs = if spring { th(spring_std) } else { 0.0 };
    let nf = th(nut_washer);
    let ns = if nut_spring { th(spring_std) } else { 0.0 };
    if (washer.is_some() && hf == 0.0) || (spring && hs == 0.0) {
        p.warn.push(t!("垫圈表里没有 M{d}", "No M{d} in the washer table"));
    }
    let sink = if csk { 0.0 } else { h.cb_depth };
    let head_stack = hf + hs;
    // 夹紧厚度(从支承面起, 不含垫圈)
    let clamp = if h.combo {
        h.t0_last.map(|t| t - sink)
    } else if h.tap || set {
        Some(0.0)
    } else {
        h.t_entry.map(|t| t - sink)
    };
    let Some(clamp) = clamp else {
        p.warn.push(t!("读不到厚度, 请手选长度", "Thickness unknown — pick the length by hand"));
        p.length = c.length.filter(|l| p.lengths.contains(l));
        place(&mut p, c, st.id, csk, set, sink, hf, hs, washer, spring, None, 0.0, 0.0, None);
        return p;
    };
    let base = clamp + head_stack;
    let ls = p.lengths.clone();
    // ---- 长度 ----
    if set {
        let avail = if h.tap || h.combo {
            if h.through { h.t_entry } else { h.depth }
        } else {
            h.t_entry.filter(|_| h.through).or(h.depth)
        };
        if !h.tap {
            p.info.push(t!("紧定螺钉应拧进螺纹孔", "A set screw belongs in a tapped hole"));
        }
        if let Some(a) = avail {
            p.max = Some(round2(a));
            p.auto = largest_at_most(&ls, a);
        }
    } else if nut_on {
        let (_, _, m) = nut_row.unwrap();
        let need = base + nf + ns + m + 2.0 * pp;
        p.need = Some(round2(need));
        p.auto = smallest_at_least(&ls, need, None);
        if p.auto.is_none() {
            p.warn.push(t!("最长 {} 不够配螺母(要 ≥ {:.1})", "Longest {} is too short for the nut (needs ≥ {:.1})", ls.last().copied().unwrap_or(0.0), need));
        }
        p.info.push(t!("夹紧 {:.1} + 垫圈 {:.1} + 螺母 {m} + 露出 2P", "Grip {:.1} + washers {:.1} + nut {m} + 2P out", clamp, head_stack + nf + ns));
    } else if h.combo || h.tap {
        // 拧进螺纹: 旋合 1.5d(不超过能用的螺纹长)
        let avail = if h.through {
            if h.combo { h.t_last } else { h.t_entry }
        } else {
            h.depth.map(|x| x - 2.0 * pp)
        };
        match avail.filter(|a| *a > 0.0) {
            None => p.warn.push(t!("读不到螺纹孔深度", "Tapped depth unknown")),
            Some(a) => {
                let want = (1.5 * d).min(a);
                let max = base + a + if h.through { 2.0 * pp } else { 0.0 };
                p.need = Some(round2(base + want));
                p.max = Some(round2(max));
                p.auto = smallest_at_least(&ls, base + want, Some(max)).or_else(|| largest_at_most(&ls, max));
                if let Some(l) = p.auto {
                    if l - base < d - 1e-9 {
                        p.warn.push(t!("旋合只有 {:.1}(< d)", "Only {:.1} of thread engaged (< d)", l - base));
                    }
                }
                p.info.push(t!("夹紧 {:.1} + 垫圈 {:.1} + 旋合 ≈ {:.1}", "Grip {:.1} + washers {:.1} + engage ≈ {:.1}", clamp, head_stack, want));
            }
        }
    } else if h.through {
        // 过孔通孔没配螺母: 当作穿过去拧进没建模的螺纹件
        let need = base + 1.5 * d;
        p.need = Some(round2(need));
        p.auto = smallest_at_least(&ls, need, None);
        p.info.push(t!("没配螺母: 按穿过后旋入 1.5d 算(对面件未建模)", "No nut: sized to pass through and engage 1.5d in an unmodelled part"));
    } else {
        // 盲过孔: 只要不顶底
        let max = h.depth.map(|x| x - sink + head_stack);
        if let Some(m) = max {
            p.max = Some(round2(m));
            p.auto = largest_at_most(&ls, m);
        }
        p.warn.push(t!("盲过孔: 螺丝不夹紧任何东西", "Blind clearance hole: the screw clamps nothing"));
    }
    if p.auto.is_none() && p.warn.is_empty() {
        p.warn.push(t!("{} M{d} 没有合适的标准长度", "No standard length of {} M{d} fits", st.code));
    }
    p.nut_ok = p.nut_why.is_none() && (!nut_on || p.auto.is_some());
    p.length = c.length.filter(|l| ls.contains(l)).or(p.auto);
    if let (Some(l), Some(_)) = (c.length, p.auto) {
        if let Some(n) = p.need.filter(|n| l < *n - 1e-9) {
            p.warn.push(t!("手选 {l} 比需要的 {n} 短", "Picked {l} is shorter than the {n} needed"));
        }
        if let Some(m) = p.max.filter(|m| l > *m + 1e-9) {
            p.warn.push(t!("手选 {l} 超过最长 {m}(会顶底 / 穿出)", "Picked {l} exceeds the {m} limit (bottoms out / sticks out)"));
        }
    }
    let exit = if nut_on && h.through && !h.combo { h.t_entry } else { None };
    place(&mut p, c, st.id, csk, set, sink, hf, hs, washer, spring, nut_std.filter(|_| nut_on).map(|n| n.id), nf, ns, exit.map(|e| (e, nut_washer.map(|w| w.id))));
    p
}


#[allow(clippy::too_many_arguments)]
fn place(
    p: &mut Plan,
    c: &Choice,
    screw: &'static str,
    csk: bool,
    set: bool,
    sink: f64,
    hf: f64,
    hs: f64,
    washer: Option<&'static fasteners::WasherStd>,
    spring: bool,
    nut: Option<&'static str>,
    nf: f64,
    ns: f64,
    exit: Option<(f64, Option<&'static str>)>,
) {
    let d = c.d;
    let mut items = Vec::new();
    // 头侧: 零件面(z = sink)往外: 平垫 → 弹垫 → 螺钉支承面
    let mut z = sink;
    if let Some(w) = washer {
        items.push(Item { kind: Kind::Washer, std: w.id, d, length: None, z, up: true });
        z -= hf;
    }
    if spring {
        items.push(Item { kind: Kind::Washer, std: "gb93", d, length: None, z, up: true });
        z -= hs;
    }
    let zs = if csk || set { 0.0 } else { z };
    items.push(Item { kind: Kind::Screw, std: screw, d, length: p.length, z: zs, up: true });
    // 螺母侧: 出口面(z = 厚度)往里: 平垫 → 弹垫 → 螺母(都朝 −n)
    if let (Some(n), Some((e, w))) = (nut, exit) {
        let mut z = e;
        if let Some(w) = w {
            items.push(Item { kind: Kind::Washer, std: w, d, length: None, z, up: false });
            z += nf;
        }
        if ns > 0.0 {
            items.push(Item { kind: Kind::Washer, std: "gb93", d, length: None, z, up: false });
            z += ns;
        }
        items.push(Item { kind: Kind::Nut, std: n, d, length: None, z, up: false });
    }
    for it in &mut items {
        it.z = round2(it.z);
    }
    p.items = items;
}

// ---------------------------------------------------------------------------
// asset pack 里的现成件

/// asset.list 里的一个资产。
#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub pack: String,
    pub id: String,
    pub name: String,
    pub folder: String,
    /// 包围盒尺寸(文档单位)
    pub size: Vec<f64>,
}

fn norm(s: &str) -> String {
    s.to_lowercase().replace(['×', '*', 'Ｘ'], "x").replace("gb/t", "gb").chars().filter(|c| !c.is_whitespace() && *c != '_' && *c != '-').collect()
}

/// 名字里的 "M<d>" 或 "M<d>x<l>"。
fn sizes_in(s: &str) -> Vec<(f64, Option<f64>)> {
    let b: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let num = |i: &mut usize| -> Option<f64> {
        let st = *i;
        while *i < b.len() && (b[*i].is_ascii_digit() || b[*i] == '.') {
            *i += 1;
        }
        b[st..*i].iter().collect::<String>().trim_end_matches('.').parse().ok()
    };
    let mut i = 0;
    while i < b.len() {
        if b[i] == 'm' && i + 1 < b.len() && b[i + 1].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_alphabetic()) {
            let mut j = i + 1;
            if let Some(d) = num(&mut j) {
                let mut l = None;
                if j + 1 < b.len() && b[j] == 'x' && b[j + 1].is_ascii_digit() {
                    let mut k = j + 1;
                    l = num(&mut k);
                    j = k;
                }
                out.push((d, l));
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn aliases(kind: Kind, std: &str) -> &'static [&'static str] {
    match kind {
        Kind::Screw => fasteners::screw_std(std).map_or(&[], |s| s.aliases),
        Kind::Washer => fasteners::washer_std(std).map_or(&[], |s| s.aliases),
        Kind::Nut => fasteners::nut_std(std).map_or(&[], |s| s.aliases),
    }
}

/// 资产的螺钉长度: 名字里有 ×l 就用它, 否则用包围盒最长边 − 头露出的高度(六角头 = k)。
fn asset_length(a: &Asset, std: &str, d: f64, named: Option<f64>) -> Option<f64> {
    if named.is_some() {
        return named;
    }
    let st = fasteners::screw_std(std)?;
    let ext = a.size.iter().copied().fold(0.0, f64::max);
    let head = st.head_above(d);
    let l = ext - head;
    st.lengths(d).into_iter().find(|x| (x - l).abs() < 0.3)
}

/// 在 pack 里找这件东西(标准俗称在名字 / 文件夹里 + 规格对 + 螺钉长度对)。
pub fn find<'a>(assets: &'a [Asset], it: &Item) -> Option<&'a Asset> {
    let al: Vec<String> = aliases(it.kind, it.std).iter().map(|a| norm(a)).collect();
    assets.iter().find(|a| {
        let hay = norm(&format!("{}/{}", a.folder, a.name));
        if !al.iter().any(|x| hay.contains(x.as_str())) {
            return false;
        }
        sizes_in(&hay).iter().any(|(d, l)| {
            (d - it.d).abs() < 1e-9 && (it.kind != Kind::Screw || it.length.is_some_and(|want| asset_length(a, it.std, it.d, *l).is_some_and(|x| (x - want).abs() < 1e-6)))
        })
    })
}

/// pack 里这个标准 + 规格有哪些长度(界面在长度下拉里标出来)。
pub fn pack_lengths(assets: &[Asset], std: &str, d: f64) -> Vec<f64> {
    let mut out: Vec<f64> = fasteners::screw_std(std)
        .map(|st| st.lengths(d))
        .unwrap_or_default()
        .into_iter()
        .filter(|l| find(assets, &Item { kind: Kind::Screw, std: fasteners::screw_std(std).map_or("", |s| s.id), d, length: Some(*l), z: 0.0, up: true }).is_some())
        .collect();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(std: &str, d: f64) -> Choice {
        Choice { std: std.into(), d, ..Default::default() }
    }

    #[test]
    fn through_with_nut_and_washers() {
        // M6 圆柱头穿 10 厚板, 头下平垫 + 弹垫, 螺母 6170 + 平垫 + 弹垫
        let mut c = ch("gb70.1", 6.0);
        c.washer = Some("gb97.1".into());
        c.spring = true;
        c.nut = Some("gb6170".into());
        c.nut_washer = true;
        c.nut_spring = true;
        let h = HoleCtx { through: true, t_entry: Some(10.0), ..Default::default() };
        let p = plan(&c, &h);
        // 需要 10 + (1.6 + 1.6) + (1.6 + 1.6) + 5.2 + 2 × 1 = 23.6 → 25
        assert_eq!(p.need, Some(23.6));
        assert_eq!(p.length, Some(25.0));
        assert!(p.nut_ok && p.warn.is_empty(), "{:?}", p.warn);
        let z: Vec<(Kind, &str, f64, bool)> = p.items.iter().map(|i| (i.kind, i.std, i.z, i.up)).collect();
        assert_eq!(
            z,
            vec![
                (Kind::Washer, "gb97.1", 0.0, true),
                (Kind::Washer, "gb93", -1.6, true),
                (Kind::Screw, "gb70.1", -3.2, true),
                (Kind::Washer, "gb97.1", 10.0, false),
                (Kind::Washer, "gb93", 11.6, false),
                (Kind::Nut, "gb6170", 13.2, false),
            ]
        );
    }

    #[test]
    fn nut_rules() {
        let mut c = ch("gb70.1", 4.0);
        c.nut = Some("gb6170".into());
        // 盲孔不能配螺母
        let p = plan(&c, &HoleCtx { through: false, depth: Some(8.0), t_entry: Some(20.0), ..Default::default() });
        assert!(p.nut_why.is_some() && !p.items.iter().any(|i| i.kind == Kind::Nut));
        // 螺纹孔不配螺母
        assert!(plan(&c, &HoleCtx { tap: true, through: true, t_entry: Some(10.0), ..Default::default() }).nut_why.is_some());
        // 太厚: M4 最长 40 不够
        let p = plan(&c, &HoleCtx { through: true, t_entry: Some(36.0), ..Default::default() });
        assert!(!p.nut_ok && p.warn.iter().any(|w| w.contains("40")), "{:?}", p.warn);
        // 尼龙螺母不加弹垫
        c.nut = Some("gb889.1".into());
        c.nut_spring = true;
        let p = plan(&c, &HoleCtx { through: true, t_entry: Some(5.0), ..Default::default() });
        assert!(!p.items.iter().any(|i| i.std == "gb93"));
    }

    #[test]
    fn tapped_and_combo_lengths() {
        // 组合孔: 上板 5(过孔)+ 下件盲螺纹孔 12, M4 → 夹紧 5, 旋合 min(6, 12 − 1.4) = 6 → 需要 11 → 12
        let c = ch("gb70.1", 4.0);
        let p = plan(&c, &HoleCtx { combo: true, t0_last: Some(5.0), depth: Some(12.0), ..Default::default() });
        assert_eq!((p.need, p.max, p.length), (Some(11.0), Some(15.6), Some(12.0)));
        // 加平垫: 需要 + 0.8
        let mut c2 = c.clone();
        c2.washer = Some("gb97.1".into());
        assert_eq!(plan(&c2, &HoleCtx { combo: true, t0_last: Some(5.0), depth: Some(12.0), ..Default::default() }).length, Some(12.0));
        c2.spring = true;
        // 5 + 0.8 + 1.1 + 6 = 12.9 → 16(≤ 5 + 1.9 + 10.6 = 17.5)
        assert_eq!(plan(&c2, &HoleCtx { combo: true, t0_last: Some(5.0), depth: Some(12.0), ..Default::default() }).length, Some(16.0));
        // 浅盲孔: 只能取 ≤ 上限的, 旋合 < d 警告
        let p = plan(&c, &HoleCtx { combo: true, t0_last: Some(5.0), depth: Some(4.0), ..Default::default() });
        assert_eq!(p.length, Some(6.0));
        assert!(p.warn.iter().any(|w| w.contains("1.0")), "{:?}", p.warn);
        // 沉孔: 夹紧扣掉沉孔深
        let p = plan(&c, &HoleCtx { combo: true, t0_last: Some(10.0), depth: Some(12.0), cb_depth: 4.4, ..Default::default() });
        assert_eq!(p.need, Some(11.6));
        assert_eq!(p.items[0].z, 4.4);
        // 沉头: 长度从面算, 头顶与面平
        let p = plan(&ch("gb70.3", 4.0), &HoleCtx { combo: true, t0_last: Some(4.0), depth: Some(10.0), ..Default::default() });
        assert_eq!((p.length, p.items[0].z), (Some(10.0), 0.0));
        // 手选太长 → 警告
        let mut c3 = c.clone();
        c3.length = Some(30.0);
        let p = plan(&c3, &HoleCtx { combo: true, t0_last: Some(5.0), depth: Some(12.0), ..Default::default() });
        assert_eq!(p.length, Some(30.0));
        assert!(p.warn.iter().any(|w| w.contains("30")));
    }

    #[test]
    fn csk_and_set_rules() {
        let mut c = ch("gb819.1", 4.0);
        c.washer = Some("gb97.1".into());
        let p = plan(&c, &HoleCtx { through: true, t_entry: Some(6.0), ..Default::default() });
        assert!(!p.washers_ok && p.items.len() == 1);
        let mut s = ch("gb77", 5.0);
        s.nut = Some("gb6170".into());
        let p = plan(&s, &HoleCtx { tap: true, through: false, depth: Some(9.0), ..Default::default() });
        assert_eq!((p.length, p.nut_why.is_some()), (Some(8.0), true));
    }

    #[test]
    fn finds_pack_assets() {
        let a = |name: &str, folder: &str, size: [f64; 3]| Asset { pack: "p".into(), id: format!("node:{name}"), name: name.into(), folder: folder.into(), size: size.to_vec() };
        // 用户的 pack: 文件夹「杯头内六角」, 资产名 M4, 长度从包围盒推(10 + 头 4 = 14)
        let assets = vec![
            a("M4", "杯头内六角", [7.0, 7.0, 14.0]),
            a("GB/T 818 M3×8", "十字盘头", [5.6, 5.6, 10.4]),
            a("平垫 M4", "垫圈", [9.0, 9.0, 0.8]),
            a("M4", "六角螺母", [8.1, 7.0, 3.2]),
        ];
        let sc = |std: &'static str, d: f64, l: f64| Item { kind: Kind::Screw, std, d, length: Some(l), z: 0.0, up: true };
        assert_eq!(find(&assets, &sc("gb70.1", 4.0, 10.0)).map(|x| x.id.as_str()), Some("node:M4"));
        assert!(find(&assets, &sc("gb70.1", 4.0, 12.0)).is_none());
        assert!(find(&assets, &sc("gb70.1", 3.0, 10.0)).is_none());
        assert_eq!(find(&assets, &sc("gb818", 3.0, 8.0)).map(|x| x.name.as_str()), Some("GB/T 818 M3×8"));
        let w = Item { kind: Kind::Washer, std: "gb97.1", d: 4.0, length: None, z: 0.0, up: true };
        assert_eq!(find(&assets, &w).map(|x| x.name.as_str()), Some("平垫 M4"));
        let n = Item { kind: Kind::Nut, std: "gb6170", d: 4.0, length: None, z: 0.0, up: false };
        assert_eq!(find(&assets, &n).map(|x| x.folder.as_str()), Some("六角螺母"));
        assert_eq!(pack_lengths(&assets, "gb70.1", 4.0), vec![10.0]);
        assert_eq!(sizes_in("m2.5x10/m3"), vec![(2.5, Some(10.0)), (3.0, None)]);
    }
}
