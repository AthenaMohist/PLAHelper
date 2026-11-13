//! R24: 选择项的可读标签(ENTITIES 列等界面用)。
//!
//! 面/边在 Plasticity 里没有名字, 只能靠「所属物体名 + 曲面类型 + 编号」区分;
//! 物体(实体/片体/曲线)用自身名字。数据来自垫片 `sel.snapshot` 的 info(所属物体名 / 名字 / 曲面类型原名)。
//! 中文名称表只在这里一份。

use pla_bridge::cdp::SelItem;
use pla_bridge::i18n::tr;

/// 选择项类型 → 中文。
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "face" => tr("面", "face"),
        "edge" => tr("边", "edge"),
        "solid" => tr("实体", "solid"),
        "sheet" => tr("片体", "sheet"),
        "curve" => tr("曲线", "curve"),
        "segment" => tr("线段", "segment"),
        "curveVertex" => tr("曲线顶点", "curve vertex"),
        "group" => tr("组", "group"),
        "plane" => tr("基准面", "plane"),
        _ => tr("其他", "other"),
    }
}

/// 曲面类型原名(垫片给的构造函数名)→ 中文; 未知的原样显示。
pub fn surface_label(raw: &str) -> String {
    match raw {
        "Plane" => tr("平面", "Plane").into(),
        "Cylinder" => tr("圆柱面", "Cylinder").into(),
        "Cone" => tr("圆锥面", "Cone").into(),
        "Sphere" => tr("球面", "Sphere").into(),
        "Torus" => tr("圆环面", "Torus").into(),
        "BlendSurf" => tr("过渡面", "Blend").into(),
        "Spun" | "SpunSurf" => tr("旋转面", "Revolved").into(),
        "Swept" | "SweptSurf" => tr("扫掠面", "Swept").into(),
        "Offset" | "OffsetSurf" => tr("偏置面", "Offset").into(),
        "BSurf" | "BSplineSurface" | "Bsurf" => tr("自由曲面", "Freeform").into(),
        other => other.to_string(),
    }
}

/// 拓扑项编号的短形式: 版本 id `<实体 id><类型字母><序号>` 取序号部分(`74f594624` → `#594624`);
/// 不是这种格式就原样。
pub fn short_id(id: &str) -> String {
    match id.rfind(|c: char| c.is_ascii_alphabetic()) {
        Some(i) if i + 1 < id.len() && id[..i].chars().all(|c| c.is_ascii_digit()) => format!("#{}", &id[i + 1..]),
        _ => format!("#{id}"),
    }
}

/// 行标签: 面 = 曲面类型 + 短编号; 边 = "边" + 短编号; 物体 = 类型 + 名字(没名字用编号)。
pub fn row_label(it: &SelItem) -> String {
    match it.kind.as_str() {
        "face" => {
            let s = if it.surface.is_empty() { tr("面", "Face").to_string() } else { surface_label(&it.surface) };
            format!("{s} {}", short_id(&it.id))
        }
        "edge" | "segment" | "curveVertex" => format!("{} {}", kind_label(&it.kind), short_id(&it.id)),
        _ => {
            if it.name.is_empty() {
                format!("{} {}", kind_label(&it.kind), short_id(&it.id))
            } else {
                format!("{} {}", kind_label(&it.kind), it.name)
            }
        }
    }
}

/// 分组键: 面/边归到所属物体名下; 物体本身归到「物体」组。
fn group_key(it: &SelItem) -> String {
    match it.kind.as_str() {
        "face" | "edge" | "segment" | "curveVertex" => {
            if it.owner.is_empty() { tr("(未知物体)", "(unknown object)").into() } else { it.owner.clone() }
        }
        _ => tr("物体", "Objects").into(),
    }
}

/// 按分组键分组, 组与组内顺序都保持首次出现顺序。
pub fn group_live_selection(items: &[SelItem]) -> Vec<(String, Vec<&SelItem>)> {
    let mut out: Vec<(String, Vec<&SelItem>)> = Vec::new();
    for it in items {
        let k = group_key(it);
        match out.iter_mut().find(|(g, _)| *g == k) {
            Some((_, v)) => v.push(it),
            None => out.push((k, vec![it])),
        }
    }
    out
}

/// 组内各类数量, 如 "16 面" / "3 面 2 边"。
pub fn kind_counts(items: &[&SelItem]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for it in items {
        let k = kind_label(&it.kind);
        match counts.iter_mut().find(|(n, _)| *n == k) {
            Some((_, c)) => *c += 1,
            None => counts.push((k, 1)),
        }
    }
    counts.iter().map(|(k, c)| format!("{c} {k}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, id: &str, owner: &str, surface: &str, name: &str) -> SelItem {
        SelItem {
            kind: kind.into(),
            id: id.into(),
            name: name.into(),
            ctor: kind.into(),
            key: None,
            owner: owner.into(),
            surface: surface.into(),
        }
    }

    #[test]
    fn labels_distinguish_same_kind_items() {
        assert_eq!(short_id("74f594624"), "#594624");
        assert_eq!(short_id("7"), "#7");
        assert_eq!(row_label(&item("face", "74f594624", "plate", "Plane", "")), "平面 #594624");
        assert_eq!(row_label(&item("face", "74f1", "plate", "Cylinder", "")), "圆柱面 #1");
        assert_eq!(row_label(&item("face", "74f2", "plate", "", "")), "面 #2", "读不到曲面类型时退回'面'");
        assert_eq!(row_label(&item("solid", "74", "", "", "silicone_upper")), "实体 silicone_upper");
        assert_eq!(row_label(&item("edge", "74e9", "plate", "", "")), "边 #9");
        assert_eq!(surface_label("Weird"), "Weird", "未知类型原样显示");
    }

    #[test]
    fn groups_by_owner_in_first_seen_order() {
        let v = vec![
            item("face", "1f1", "A", "Plane", ""),
            item("face", "2f1", "B", "Plane", ""),
            item("edge", "1e3", "A", "", ""),
            item("solid", "9", "", "", "Body9"),
            item("face", "3f1", "", "Plane", ""),
        ];
        let g = group_live_selection(&v);
        let names: Vec<&str> = g.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["A", "B", "物体", "(未知物体)"]);
        assert_eq!(kind_counts(&g[0].1), "1 面 1 边");
        assert_eq!(kind_counts(&g[2].1), "1 实体");
    }
}
