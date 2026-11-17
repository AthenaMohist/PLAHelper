//! 外发包(用户 2026-11-17): 给加工方的一整包 —— 总 BOM + 按供应商分开的加工 BOM + 每个零件的图纸 / DXF / STEP。
//!
//! - **供应商**(全局 `<数据目录>/suppliers.json`): CNC / 3D 打印 / 硅胶开模 / 塑料开模(预置, 可增删改)。
//!   每家: 要哪些文件(PDF / DXF / STEP)、BOM 用主表的哪些列(按主模板列 id)+ 自己的列(表面处理 / 颜色 / 打印材料 …, 值按零件存在 BOM 填写里)。
//! - **零件归哪家**: BOM 填写里的 `process` 字段(= 供应商 id, "buy" = 外购不发)。跟着零件身份走(模型改了也不丢, 见 identity)。
//! - **外发包**: `<文档目录>/.pla/release/<doc>/<日期>-R<n>/`: 总BOM.csv、每家一个文件夹(<家>-BOM.csv + 文件)+ 同名 zip、manifest.json(版本记录)。
//!   锁定的零件发上传的图纸; 模型改过还没重出的图不发, 列进"缺"。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::bom_table::{Col, Edits, Row, Template};
use crate::drawing::Sheet;
use crate::state_store::StateStore;

pub const STORE: StateStore = StateStore { subdir: "release", legacy_subdir: None, prefix: "REL" };
/// BOM 填写里"归哪家"的字段 id。
pub const PROCESS_COL: &str = "process";
/// 外购(不进任何加工包)。
pub const BUY: &str = "buy";
pub const FILE_KINDS: &[&str] = &["pdf", "dxf", "step"];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Supplier {
    /// 稳定 id(存在零件的 process 字段里)。
    pub id: String,
    pub name: String,
    /// 要哪些文件: pdf / dxf / step。
    #[serde(default)]
    pub files: Vec<String>,
    /// 用主表的哪些列(主模板列 id, 按这个顺序)。
    #[serde(default)]
    pub columns: Vec<String>,
    /// 这家自己的列(可编辑字段, 值按零件存在 BOM 填写里)。
    #[serde(default)]
    pub extra: Vec<Col>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Suppliers {
    #[serde(default)]
    pub list: Vec<Supplier>,
}

fn field(id: &str, header: &str, default: Option<&str>) -> Col {
    Col { id: id.into(), header: header.into(), source: Some("field".into()), formula: None, default: default.map(String::from), sum: false }
}

fn sup(id: &str, name: &str, files: &[&str], columns: &[&str], extra: Vec<Col>) -> Supplier {
    Supplier { id: id.into(), name: name.into(), files: files.iter().map(|s| s.to_string()).collect(), columns: columns.iter().map(|s| s.to_string()).collect(), extra }
}

impl Suppliers {
    /// 预置四家(列 id 对应内置 OYM 模板; "finish" 同时是出图标题栏的表处)。
    pub fn defaults() -> Suppliers {
        Suppliers {
            list: vec![
                sup("cnc", "CNC", &["pdf", "dxf", "step"], &["pn", "name", "material", "total_qty"], vec![field("finish", "表面处理", None), field("cnc_note", "加工备注", None)]),
                sup(
                    "print3d",
                    "3D打印",
                    &["step"],
                    &["pn", "name", "total_qty"],
                    vec![field("print_mat", "打印材料", None), field("print_tech", "工艺(SLA/MJF/FDM)", None), field("color", "颜色", None), field("print_note", "备注", None)],
                ),
                sup("silicone", "硅胶开模", &["pdf", "step"], &["pn", "name", "total_qty"], vec![field("shore", "硬度(邵氏A)", None), field("color", "颜色", None), field("mold_note", "备注", None)]),
                sup(
                    "plastic",
                    "塑料开模",
                    &["pdf", "step"],
                    &["pn", "name", "total_qty"],
                    vec![field("resin", "材料牌号", None), field("color", "颜色", None), field("cavities", "模穴数", Some("1")), field("mold_note", "备注", None)],
                ),
            ],
        }
    }
    pub fn path() -> PathBuf {
        crate::prefs::data_dir().join("suppliers.json")
    }
    /// 没有文件 = 预置四家。
    pub fn load() -> Suppliers {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_else(Self::defaults)
    }
    pub fn save(&self) -> Result<(), String> {
        let p = Self::path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(p, serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
    pub fn get(&self, id: &str) -> Option<&Supplier> {
        self.list.iter().find(|s| s.id == id)
    }
    /// 新供应商的 id(sup1, sup2 … 不和已有的重)。
    pub fn next_id(&self) -> String {
        (1..).map(|i| format!("sup{i}")).find(|id| self.get(id).is_none()).unwrap_or_default()
    }
}

/// 零件归哪家("" = 没分配)。
pub fn process_of<'a>(e: &'a Edits, key: &str) -> &'a str {
    e.parts.get(key).and_then(|m| m.get(PROCESS_COL)).map(|s| s.as_str()).unwrap_or("")
}

/// 一家的加工 BOM: (表头, 行, 行的零件 key)。主表列取主表算好的值(公式也算完了), 自有列取填写 / 默认。
pub fn supplier_table(s: &Supplier, master: &Template, rows: &[Row], e: &Edits) -> (Vec<String>, Vec<Vec<String>>, Vec<String>) {
    let idx: Vec<(usize, String)> = s.columns.iter().filter_map(|id| master.columns.iter().position(|c| &c.id == id).map(|i| (i, master.columns[i].header.clone()))).collect();
    let mut headers: Vec<String> = vec!["#".into()];
    headers.extend(idx.iter().map(|(_, h)| h.clone()));
    headers.extend(s.extra.iter().map(|c| c.header.clone()));
    let mut out = Vec::new();
    let mut keys = Vec::new();
    for r in rows.iter().filter(|r| !r.manual && process_of(e, &r.key) == s.id) {
        let mut cells = vec![(out.len() + 1).to_string()];
        cells.extend(idx.iter().map(|(i, _)| r.cells[*i].clone()));
        let over = e.parts.get(&r.key);
        cells.extend(s.extra.iter().map(|c| over.and_then(|m| m.get(&c.id)).cloned().or_else(|| c.default.clone()).unwrap_or_default()));
        out.push(cells);
        keys.push(r.key.clone());
    }
    (headers, out, keys)
}

pub fn table_csv(headers: &[String], rows: &[Vec<String>]) -> String {
    let cell = |s: &str| if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
    let mut out = String::from("\u{feff}");
    for r in std::iter::once(headers).chain(rows.iter().map(|r| r.as_slice())) {
        out.push_str(&r.iter().map(|c| cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    out
}

/// 一个零件要发的文件: (源文件, 包里的文件名) + 缺了什么。锁定 = 发上传的图纸; 模型改过没重出 = 生成的图 / STEP 都不发。
pub fn part_files(dir: &Path, sh: Option<&Sheet>, want: &[String]) -> (Vec<(PathBuf, String)>, Vec<String>) {
    let mut files = Vec::new();
    let mut missing = Vec::new();
    let Some(s) = sh else {
        return (files, want.iter().map(|w| w.to_uppercase()).collect());
    };
    let up = s.uploaded.as_ref().filter(|u| u.locked);
    let up_ext = up.and_then(|u| Path::new(&u.file).extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()));
    for w in want {
        let (src, name) = match (w.as_str(), up, up_ext.as_deref()) {
            // 上传的图纸: PDF / 图片顶 PDF, DWG / DXF 顶 DXF
            ("pdf", Some(u), Some(x)) if !matches!(x, "dwg" | "dxf") => (dir.join(&u.file), format!("{}.{x}", s.base)),
            ("dxf", Some(u), Some(x)) if matches!(x, "dwg" | "dxf") => (dir.join(&u.file), format!("{}.{x}", s.base)),
            ("pdf", Some(_), _) | ("dxf", Some(_), _) => {
                // 锁定了但上传的不是这种格式: 不拿生成的顶(生成的不是准的)
                missing.push(w.to_uppercase());
                continue;
            }
            (x, _, _) => (dir.join(format!("{}.{x}", s.base)), format!("{}.{x}", s.base)),
        };
        let generated = up.is_none() || w == "step";
        if src.exists() && !(generated && s.model_changed) {
            files.push((src, name));
        } else {
            missing.push(w.to_uppercase());
        }
    }
    (files, missing)
}

/// 下一个版本号: 发布目录里已有 …-R<n> → R<n+1>。
pub fn next_version(root: &Path) -> String {
    let n = std::fs::read_dir(root)
        .map(|rd| rd.filter_map(|e| e.ok()).filter_map(|e| e.file_name().to_str().and_then(|n| n.rsplit_once("-R").and_then(|(_, v)| v.parse::<u32>().ok()))).max().unwrap_or(0))
        .unwrap_or(0);
    format!("R{}", n + 1)
}

/// 一家的包内容。
pub struct Pack {
    pub supplier: Supplier,
    pub csv: String,
    pub files: Vec<(PathBuf, String)>,
    /// 零件名 → 缺的文件。
    pub missing: Vec<(String, Vec<String>)>,
}

fn safe(s: &str) -> String {
    s.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect::<String>().trim().to_string()
}

/// 写外发包(阻塞, 放后台线程): 文件夹 + 每家 zip + manifest。返回包目录。
pub fn write(root: &Path, version: &str, date: &str, total_csv: &str, packs: &[Pack]) -> Result<PathBuf, String> {
    let out = root.join(format!("{date}-{version}"));
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    std::fs::write(out.join("总BOM.csv"), total_csv).map_err(|e| e.to_string())?;
    let mut manifest = Vec::new();
    for p in packs {
        let name = safe(&p.supplier.name);
        let d = out.join(&name);
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        std::fs::write(d.join(format!("{name}-BOM.csv")), &p.csv).map_err(|e| e.to_string())?;
        for (src, dst) in &p.files {
            std::fs::copy(src, d.join(safe(dst))).map_err(|e| format!("{}: {e}", src.display()))?;
        }
        zip_dir(&d, &out.join(format!("{name}.zip")))?;
        manifest.push(json!({"supplier": p.supplier.name, "files": p.files.iter().map(|f| &f.1).collect::<Vec<_>>(), "missing": p.missing}));
    }
    let m = json!({"version": version, "date": date, "suppliers": manifest});
    std::fs::write(out.join("manifest.json"), serde_json::to_vec_pretty(&m).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(out)
}

/// 文件夹 → zip(.NET ZipFile, 文件名 UTF-8, 中文不乱码)。
fn zip_dir(dir: &Path, zip: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(zip);
    let ps = format!(
        "Add-Type -AssemblyName System.IO.Compression.FileSystem; [IO.Compression.ZipFile]::CreateFromDirectory('{}', '{}')",
        dir.display().to_string().replace('\'', "''"),
        zip.display().to_string().replace('\'', "''")
    );
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-Command", &ps]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let o = cmd.output().map_err(|e| format!("powershell: {e}"))?;
    if o.status.success() && zip.exists() {
        Ok(())
    } else {
        Err(format!("zip: {}", String::from_utf8_lossy(&o.stderr).lines().find(|l| !l.trim().is_empty()).unwrap_or("")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::Uploaded;

    #[test]
    fn supplier_tables_files_and_package() {
        let master = crate::bom_table::builtin();
        let scan = serde_json::json!({"lines": [
            {"key": "a", "name": "Base", "qty": 2, "material": "6061", "path": ""},
            {"key": "b", "name": "Cover", "qty": 1, "material": "", "path": ""},
            {"key": "c", "name": "M4 screw", "qty": 8, "material": "", "path": ""},
        ]});
        let mut e = Edits::default();
        e.set_cell("a", PROCESS_COL, "cnc", "");
        e.set_cell("a", "finish", "阳极氧化", "");
        e.set_cell("a", "pn", "904002001", "");
        e.set_cell("b", PROCESS_COL, "print3d", "");
        e.set_cell("c", PROCESS_COL, BUY, "");
        let (rows, _) = crate::bom_table::build(&scan, &master, &e, &crate::parts_lib::Library::default());
        let sups = Suppliers::defaults();
        let (h, t, keys) = supplier_table(sups.get("cnc").unwrap(), &master, &rows, &e);
        assert_eq!(h, vec!["#", "料号", "名称", "材料", "总数量", "表面处理", "加工备注"]);
        assert_eq!(t, vec![vec!["1", "904002001", "Base", "6061", "2", "阳极氧化", ""]]);
        assert_eq!(keys, vec!["a"]);
        assert_eq!(supplier_table(sups.get("print3d").unwrap(), &master, &rows, &e).1.len(), 1);
        assert!(table_csv(&h, &t).starts_with("\u{feff}#,料号"));

        let d = std::env::temp_dir().join(format!("pla_rel_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut s = Sheet::new("a", "904002001", "m", "Base");
        for x in ["pdf", "dxf", "step"] {
            std::fs::write(d.join(format!("904002001.{x}")), x).unwrap();
        }
        let want: Vec<String> = FILE_KINDS.iter().map(|s| s.to_string()).collect();
        let (f, miss) = part_files(&d, Some(&s), &want);
        assert_eq!((f.len(), miss.len()), (3, 0));
        // 模型改过没重出: 都不发
        s.model_changed = true;
        assert_eq!(part_files(&d, Some(&s), &want).1, vec!["PDF", "DXF", "STEP"]);
        s.model_changed = false;
        // 锁定上传 PDF: PDF 用上传的, DXF 不拿生成的顶
        std::fs::write(d.join("904002001.uploaded.pdf"), "up").unwrap();
        s.uploaded = Some(Uploaded { file: "904002001.uploaded.pdf".into(), orig: "x.pdf".into(), date: String::new(), locked: true, pages: vec![], stale: false });
        let (f, miss) = part_files(&d, Some(&s), &want);
        assert_eq!(f[0].0, d.join("904002001.uploaded.pdf"));
        assert_eq!(miss, vec!["DXF"]);
        assert_eq!(part_files(&d, None, &want).1.len(), 3);

        let root = d.join("rel");
        assert_eq!(next_version(&root), "R1");
        let pack = Pack { supplier: sups.get("cnc").unwrap().clone(), csv: table_csv(&h, &t), files: f, missing: vec![("Base".into(), miss)] };
        let out = write(&root, "R1", "2026-11-17", "total", &[pack]).unwrap();
        assert!(out.join("总BOM.csv").exists() && out.join("CNC").join("CNC-BOM.csv").exists() && out.join("CNC").join("904002001.pdf").exists());
        assert!(out.join("CNC.zip").exists() && out.join("manifest.json").exists());
        assert_eq!(next_version(&root), "R2");
        let _ = std::fs::remove_dir_all(&d);
    }
}
