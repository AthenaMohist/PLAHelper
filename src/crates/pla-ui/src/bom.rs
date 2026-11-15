//! B1/B2: BOM 表(垫片 `bom.scan` 的结果)→ 显示用的行 / CSV 导出(PRD-bom-drawings §2)。
//!
//! 导出到 `<文档目录>/.pla/bom/<doc_key>/<范围>.csv`(带 UTF-8 BOM, Excel 直接打开中文不乱码)。

use pla_bridge::i18n::tr;
use pla_bridge::t;
use serde_json::Value;

use crate::state_store::StateStore;

/// 只借用它的"按文档的目录"规则(不存状态卡片)。
pub const STORE: StateStore = StateStore { subdir: "bom", legacy_subdir: None, prefix: "BOM" };

/// 一行的备注: 镜像件 / 同一种零件但名字不同。
pub fn remark(line: &Value) -> String {
    let mut out = Vec::new();
    if let Some(m) = line["mirrorOf"].as_str() {
        out.push(t!("{m} 的镜像件", "mirror of {m}"));
    }
    let names: Vec<&str> = line["names"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
    if names.len() > 1 {
        out.push(t!("几何相同、名字不同: {}", "same geometry, different names: {}", names.join(", ")));
    }
    out.join("; ")
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// `bom.scan` 的回包 → CSV 文本(序号, 名称, 数量, 材料, 所在组, 体积, 备注)。
pub fn to_csv(scan: &Value) -> String {
    let units = scan["units"].as_str().unwrap_or("");
    let u = match units { "millimeter" => "mm", "centimeter" => "cm", "meter" => "m", "inch" => "in", "foot" => "ft", x => x };
    let mut s = String::from("\u{feff}");
    s.push_str(&[tr("序号", "No."), tr("名称", "Name"), tr("数量", "Qty"), tr("材料", "Material"), tr("所在组", "Group"), &t!("体积({u}³)", "Volume ({u}³)"), tr("备注", "Remarks")].map(csv_cell).join(","));
    s.push_str("\r\n");
    for (i, l) in scan["lines"].as_array().into_iter().flatten().enumerate() {
        let row = [
            (i + 1).to_string(),
            l["name"].as_str().unwrap_or("").to_string(),
            l["qty"].as_u64().unwrap_or(0).to_string(),
            l["material"].as_str().unwrap_or("").to_string(),
            l["path"].as_str().unwrap_or("").to_string(),
            l["volume"].as_f64().map(|v| format!("{v}")).unwrap_or_default(),
            remark(l),
        ];
        s.push_str(&row.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        s.push_str("\r\n");
    }
    s
}

/// 写 CSV 到 `<bom 目录>/<范围名>.csv`, 返回路径。
pub fn export(doc_key: &str, scope: &str, scan: &Value) -> Result<std::path::PathBuf, String> {
    let dir = STORE.dir(doc_key);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let safe: String = scope.chars().map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let p = dir.join(format!("{safe}.csv"));
    std::fs::write(&p, to_csv(scan)).map_err(|e| e.to_string())?;
    Ok(p)
}

/// 合计: (行数 = 零件种类, 总件数)。
pub fn totals(scan: &Value) -> (usize, u64) {
    let lines = scan["lines"].as_array().cloned().unwrap_or_default();
    (lines.len(), lines.iter().map(|l| l["qty"].as_u64().unwrap_or(0)).sum())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn csv_and_remarks() {
        let scan = json!({ "units": "millimeter", "lines": [
            { "name": "螺钉 M4", "names": ["螺钉 M4"], "qty": 4, "material": "Steel", "path": "紧固件", "volume": 125.5 },
            { "name": "支架, 左", "names": ["支架, 左", "Bracket"], "qty": 1, "material": null, "path": "", "volume": 9000, "mirrorOf": "支架 右" },
        ]});
        let c = to_csv(&scan);
        assert!(c.starts_with('\u{feff}'), "UTF-8 BOM");
        let rows: Vec<&str> = c.trim_end().split("\r\n").collect();
        assert_eq!(rows.len(), 3);
        assert!(rows[1].starts_with("1,螺钉 M4,4,Steel,紧固件,125.5,"));
        assert!(rows[2].contains("\"支架, 左\""), "带逗号的名字加引号: {}", rows[2]);
        assert!(rows[2].contains("支架 右"), "镜像备注");
        assert_eq!(totals(&scan), (2, 5));
    }
}
