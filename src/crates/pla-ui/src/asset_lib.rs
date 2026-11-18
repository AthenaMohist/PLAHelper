//! N5b 紧固件资产库: 按国标目录把一个系列建成 Plasticity asset pack, pack 随 PLAhelper 分发, 一键加入用户的 Plasticity。
//! 用户 2026-11-18: 螺钉统一留 10mm(放进来后只移动杆端改长度); 资产可以是单个实体; 按需 / 有空时逐系列建库。
//!
//! 约定(打孔面板 / screw_kit 靠它认件、摆放): 名字 = `GB/T 70.1 M4x10`(标准号 + 规格 [+ 长度]); 一件一个实体;
//! pivot = 贴合面中心(螺钉 = 头的支承面, 沉头 = 头顶, 紧定 = 内六角端; 垫圈 / 螺母 = 底面), 资产 +Z = 朝外(螺钉 = 杆 → 头)。
//! 一个标准一个 pack 文件(导出时文档里所有资产都会进 pack, 建完就回滚, 所以按系列分文件)。

use crate::fasteners::{self, Head};
use crate::screw_kit::PACK_SCREW_LEN;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// 一个系列(标准 id)对应的 pack 文件名。
pub fn pack_file(std_id: &str) -> Option<String> {
    let code = fasteners::screw_std(std_id)
        .map(|s| s.code)
        .or_else(|| fasteners::washer_std(std_id).map(|s| s.code))
        .or_else(|| fasteners::nut_std(std_id).map(|s| s.code))?;
    Some(format!("PLAhelper {}.plasticitypack", code.replace('/', "-")))
}

/// 所有能建库的系列(螺钉 + 垫圈 + 螺母)。
pub fn series() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    let mut v: Vec<_> = fasteners::SCREWS.iter().map(|s| (s.id, s.code, s.zh, s.en)).collect();
    v.extend(fasteners::WASHERS.iter().map(|s| (s.id, s.code, s.zh, s.en)));
    v.extend(fasteners::NUTS.iter().map(|s| (s.id, s.code, s.zh, s.en)));
    v
}

fn fmt(x: f64) -> String {
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// 建库的件(给 pla-bridge build_pack): [{ ops, name, pivot, units }]。sizes 为空 = 该标准全部规格。
/// 件沿 X 排开(间距按最大外径), 都在 z = 0 贴合: 螺钉头朝 +Z。
pub fn build_items(std_id: &str, sizes: &[f64]) -> Result<Vec<Value>, String> {
    let want = |d: f64| sizes.is_empty() || sizes.iter().any(|s| (s - d).abs() < 1e-9);
    let mut out = Vec::new();
    let mut x = 0.0;
    let mut push = |req: Value, name: String, width: f64, out: &mut Vec<Value>| -> Result<(), String> {
        let at = [x, 0.0, 0.0];
        let mut req = req;
        req["at"] = json!(at);
        let q: fasteners::PartReq = serde_json::from_value(req).map_err(|e| e.to_string())?;
        let e = fasteners::expand(&q).map_err(|e| format!("{name}: {e}"))?;
        out.push(json!({ "ops": e.ops, "name": name, "pivot": at, "units": "millimeter" }));
        x += (width * 2.0).max(20.0).ceil();
        Ok(())
    };
    if let Some(st) = fasteners::screw_std(std_id) {
        for r in st.rows.iter().filter(|r| want(r.d)) {
            // 统一 10mm; 沉头的公称长度含头, 头太高时取 k + 2 起
            let l = if st.head == Head::Csk { PACK_SCREW_LEN.max((r.k + 2.0).ceil()) } else { PACK_SCREW_LEN };
            let name = format!("{} M{}x{}", st.code, fmt(r.d), fmt(l));
            push(json!({ "kind": "screw", "std": st.id, "size": r.d, "length": l, "axis": [0, 0, -1] }), name, r.dk, &mut out)?;
        }
    } else if let Some(ws) = fasteners::washer_std(std_id) {
        for r in ws.rows.iter().filter(|r| want(r.0)) {
            push(json!({ "kind": "washer", "std": ws.id, "size": r.0, "axis": [0, 0, 1] }), format!("{} M{}", ws.code, fmt(r.0)), r.2, &mut out)?;
        }
    } else if let Some(ns) = fasteners::nut_std(std_id) {
        for r in ns.rows.iter().filter(|r| want(r.0)) {
            push(json!({ "kind": "nut", "std": ns.id, "size": r.0, "axis": [0, 0, 1] }), format!("{} M{}", ns.code, fmt(r.0)), r.1 * 1.16, &mut out)?;
        }
    } else {
        return Err(format!("unknown series {std_id}"));
    }
    if out.is_empty() {
        return Err(format!("{std_id}: none of the sizes {sizes:?} are in the table"));
    }
    Ok(out)
}

/// 随软件分发的 pack 目录: 从程序所在目录往上找 `assets/packs`(开发时 = 仓库里的, 建库直接写进去, 跟着提交 / 打包)。
pub fn bundled_dir() -> Option<PathBuf> {
    let mut d = std::env::current_exe().ok()?.parent()?.to_path_buf();
    for _ in 0..7 {
        let c = d.join("assets").join("packs");
        if c.is_dir() {
            return Some(c);
        }
        d = d.parent()?.to_path_buf();
    }
    None
}

/// 建库写到哪: 有分发目录就写分发目录, 否则写用户数据目录。
pub fn build_dir() -> PathBuf {
    bundled_dir().unwrap_or_else(installed_dir)
}

/// 加入 Plasticity 时 pack 拷到这里再登记(路径稳定, 不随安装目录变)。
pub fn installed_dir() -> PathBuf {
    crate::prefs::data_dir().join("asset-packs")
}

/// 目录里的 pack 文件(不含建库时的临时文件)。
pub fn packs_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("plasticitypack")) && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('~')))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// 把分发目录里的 pack 拷到安装目录(新的 / 大小或时间不同的才拷), 返回安装目录里全部 pack 的路径(给 asset.register)。
pub fn install_from(src: &Path, dst: &Path) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("mkdir {}: {e}", dst.display()))?;
    for p in packs_in(src) {
        let to = dst.join(p.file_name().unwrap_or_default());
        let same = match (std::fs::metadata(&p), std::fs::metadata(&to)) {
            (Ok(a), Ok(b)) => a.len() == b.len() && a.modified().ok() <= b.modified().ok(),
            _ => false,
        };
        if !same {
            std::fs::copy(&p, &to).map_err(|e| format!("copy {} → {}: {e}", p.display(), to.display()))?;
        }
    }
    Ok(packs_in(dst))
}

/// 一键加入: 分发目录 → 安装目录; 没有分发目录(或里面是空的)就只登记安装目录里已有的。
pub fn install() -> Result<Vec<PathBuf>, String> {
    let dst = installed_dir();
    match bundled_dir() {
        Some(src) if src != dst => install_from(&src, &dst),
        _ => Ok(packs_in(&dst)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_follow_the_conventions() {
        let it = build_items("gb70.1", &[3.0, 4.0]).unwrap();
        assert_eq!(it.len(), 2);
        assert_eq!(it[1]["name"], "GB/T 70.1 M4x10");
        // pivot = 支承面 = at; 头朝 +Z(螺钉 axis 朝 −Z)
        assert_eq!(it[0]["pivot"], json!([0.0, 0.0, 0.0]));
        assert_eq!(it[0]["ops"][0]["axis"], json!([-0.0, -0.0, 1.0]), "旋转轴朝头");
        assert!(it[1]["pivot"][0].as_f64().unwrap() >= 20.0, "沿 X 排开");
        // 沉头: M20 头高 10.16 → 13
        let csk = build_items("gb70.3", &[20.0]).unwrap();
        assert_eq!(csk[0]["name"], "GB/T 70.3 M20x13");
        assert_eq!(build_items("gb70.3", &[]).unwrap().len(), fasteners::screw_std("gb70.3").unwrap().rows.len());
        let w = build_items("gb93", &[6.0]).unwrap();
        assert_eq!(w[0]["name"], "GB/T 93 M6");
        assert!(build_items("gb6170", &[7.0]).is_err());
        assert!(build_items("rivets", &[]).is_err());
        // 名字认得回来(screw_kit::find 用的俗称 + 规格 + 长度)
        let a = crate::screw_kit::Asset { pack: "p".into(), id: "x".into(), name: "GB/T 70.1 M4x10".into(), folder: String::new(), size: vec![7.0, 7.0, 14.0] };
        let item = crate::screw_kit::Item { kind: crate::screw_kit::Kind::Screw, std: "gb70.1", d: 4.0, length: Some(25.0), z: 0.0, up: true };
        assert!(crate::screw_kit::find(std::slice::from_ref(&a), &item).is_some());
        assert_eq!(crate::screw_kit::asset_len(&a, "gb70.1", 4.0), 10.0);
    }

    #[test]
    fn pack_names_and_install() {
        assert_eq!(pack_file("gb70.1").as_deref(), Some("PLAhelper GB-T 70.1.plasticitypack"));
        assert_eq!(pack_file("gb889.1").as_deref(), Some("PLAhelper GB-T 889.1.plasticitypack"));
        assert!(pack_file("nope").is_none());
        assert!(series().len() >= 20);
        let base = std::env::temp_dir().join(format!("pla-asset-lib-{}", std::process::id()));
        let (src, dst) = (base.join("src"), base.join("dst"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("PLAhelper GB-T 70.1.plasticitypack"), b"x").unwrap();
        std::fs::write(src.join("~build-1.plasticitypack"), b"tmp").unwrap();
        std::fs::write(src.join("readme.txt"), b"no").unwrap();
        let got = install_from(&src, &dst).unwrap();
        assert_eq!(got.len(), 1);
        assert!(got[0].ends_with("PLAhelper GB-T 70.1.plasticitypack"));
        assert_eq!(install_from(&src, &dst).unwrap().len(), 1, "再装一次不重复");
        let _ = std::fs::remove_dir_all(&base);
    }
}
