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
pub fn write(root: &Path, version: &str, date: &str, total_csv: &str, packs: &[Pack], ecn: Option<&Ecn>) -> Result<PathBuf, String> {
    let out = root.join(format!("{date}-{version}"));
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    std::fs::write(out.join("总BOM.csv"), total_csv).map_err(|e| e.to_string())?;
    if let Some(e) = ecn {
        let rows: Vec<Vec<String>> = e.items.iter().enumerate().map(|(i, it)| vec![(i + 1).to_string(), it.label.clone(), it.desc.clone()]).collect();
        let mut csv = table_csv(&["#".into(), "零件".into(), "变更内容".into()], &rows);
        csv.push_str(&format!("\r\n{},{},{}\r\n", e.no, if e.note.is_empty() { "-" } else { &e.note }, e.date));
        std::fs::write(out.join(format!("{}.csv", e.no)), csv).map_err(|e| e.to_string())?;
    }
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
    let m = json!({"version": version, "date": date, "ecn": ecn, "suppliers": manifest});
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

// ---------------------------------------------------------------- 版本 / ECN(用户 2026-11-17)
//
// 外发一次 = 锁定这次之前的全部改动(存一份零件快照)。之后的改动不单独记: 当前状态 vs 上次外发的快照 = 一条"进行中的变更",
// 期间改多少次都在这一条里。下次外发时选:
// - 新的外发(R<n+1>): 这条变更冻结成 ECN-<m>, 写进受影响零件图的变更表;
// - 同一批补发(仍是 R<n>): 不新增 ECN, R<n> 的快照(和它带的那条 ECN)按现状重算。
// 本地改动(没外发过)不产生 ECN。零件按 key 对应, key 变了(模型改了)按实体 id 对应(同 identity)。

/// 一个零件在某次外发时的状态。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PartSnap {
    #[serde(default)]
    pub pn: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub qty: u64,
    #[serde(default)]
    pub material: String,
    #[serde(default)]
    pub supplier: String,
    /// 实例 id(模型改了 key 会变, 靠它认出是同一个零件)。
    #[serde(default)]
    pub ids: Vec<String>,
    /// 出图设置的指纹(人改过朝向 / 公差 / 技术要求 … 就变)。
    #[serde(default)]
    pub sheet: String,
    /// 上传图纸(文件名 + 日期)。
    #[serde(default)]
    pub upload: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Release {
    pub ver: String,
    pub date: String,
    /// 零件 key → 快照。
    #[serde(default)]
    pub parts: std::collections::BTreeMap<String, PartSnap>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct EcnItem {
    /// 零件 key(模型改了跟着改名, 见 History::rename_key)。
    pub key: String,
    #[serde(default)]
    pub label: String,
    /// added / removed / model / qty / material / pn / name / supplier / drawing / upload
    pub kinds: Vec<String>,
    pub desc: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Ecn {
    pub no: String,
    /// 随哪次外发发出。
    pub ver: String,
    pub date: String,
    #[serde(default)]
    pub by: String,
    /// 变更原因(人填)。
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub items: Vec<EcnItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct History {
    #[serde(default)]
    pub releases: Vec<Release>,
    #[serde(default)]
    pub ecns: Vec<Ecn>,
    /// 进行中那条变更的原因 / 责任人(外发时进 ECN)。
    #[serde(default)]
    pub open_note: String,
    #[serde(default)]
    pub open_by: String,
}

pub fn history_path(root: &Path) -> PathBuf {
    root.join("history.json")
}

impl History {
    pub fn load(root: &Path) -> History {
        std::fs::read_to_string(history_path(root)).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        std::fs::write(history_path(root), serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
    pub fn last(&self) -> Option<&Release> {
        self.releases.last()
    }
    /// 下一个新外发的版本号。
    pub fn next_ver(&self) -> String {
        format!("R{}", self.releases.len() + 1)
    }
    fn next_ecn_no(&self) -> String {
        format!("ECN-{:03}", self.ecns.len() + 1)
    }
    /// 模型改了(key 变了): 快照 / ECN 里的 key 跟着改。
    pub fn rename_key(&mut self, old: &str, new: &str) -> bool {
        let mut changed = false;
        for r in &mut self.releases {
            if let Some(p) = r.parts.remove(old) {
                r.parts.insert(new.into(), p);
                changed = true;
            }
        }
        for e in &mut self.ecns {
            for it in e.items.iter_mut().filter(|i| i.key == old) {
                it.key = new.into();
                changed = true;
            }
        }
        changed
    }

    /// 外发: amend = 同一批补发(改最后一次), 否则新外发(有上次 → 冻结一条 ECN)。返回版本号和新 / 重算的 ECN 号。
    pub fn record(&mut self, now: std::collections::BTreeMap<String, PartSnap>, date: &str, amend: bool) -> (String, Option<String>) {
        if amend && !self.releases.is_empty() {
            let i = self.releases.len() - 1;
            let ver = self.releases[i].ver.clone();
            // 这次外发带的那条 ECN 按现状重算(对比的是再上一次)
            if i > 0 {
                let items = diff(&self.releases[i - 1].parts, &now);
                if let Some(e) = self.ecns.iter_mut().find(|e| e.ver == ver) {
                    e.items = items;
                    e.date = date.into();
                    if !self.open_note.trim().is_empty() {
                        e.note = self.open_note.trim().to_string();
                    }
                }
            }
            self.releases[i].parts = now;
            self.releases[i].date = date.into();
            let no = self.ecns.iter().find(|e| e.ver == ver).map(|e| e.no.clone());
            self.open_note.clear();
            return (ver, no);
        }
        let ver = self.next_ver();
        let mut no = None;
        if let Some(last) = self.releases.last() {
            let items = diff(&last.parts, &now);
            if !items.is_empty() {
                let n = self.next_ecn_no();
                self.ecns.push(Ecn { no: n.clone(), ver: ver.clone(), date: date.into(), by: self.open_by.clone(), note: self.open_note.trim().to_string(), items });
                no = Some(n);
            }
        }
        self.releases.push(Release { ver: ver.clone(), date: date.into(), parts: now });
        self.open_note.clear();
        (ver, no)
    }

    /// 某个零件图上的变更表(最近两条, 旧的在第 1 行): (ECN 号, 内容, 责任人, 日期)。
    pub fn rev_rows(&self, key: &str) -> Vec<(String, String, String, String)> {
        let mut rows: Vec<(String, String, String, String)> = self
            .ecns
            .iter()
            .filter_map(|e| {
                let it = e.items.iter().find(|i| i.key == key && !i.kinds.iter().any(|k| k == "removed"))?;
                let desc = if e.note.is_empty() { it.desc.clone() } else { format!("{}; {}", it.desc, e.note) };
                Some((e.no.clone(), desc, e.by.clone(), e.date.clone()))
            })
            .collect();
        if rows.len() > 2 {
            rows.drain(..rows.len() - 2);
        }
        rows
    }
}

/// 两次状态的差: 按 key 对应; key 不同但实例 id 重合 = 同一零件改了模型。
pub fn diff(old: &std::collections::BTreeMap<String, PartSnap>, now: &std::collections::BTreeMap<String, PartSnap>) -> Vec<EcnItem> {
    let mut items = Vec::new();
    let mut matched_old: std::collections::BTreeSet<&String> = std::collections::BTreeSet::new();
    let label = |p: &PartSnap| format!("{} {}", p.pn, p.name).trim().to_string();
    for (k, n) in now {
        let o = old.get_key_value(k).or_else(|| old.iter().find(|(ok, op)| !now.contains_key(*ok) && op.ids.iter().any(|i| n.ids.contains(i))));
        let Some((ok, o)) = o else {
            items.push(EcnItem { key: k.clone(), label: label(n), kinds: vec!["added".into()], desc: format!("新增 ×{}", n.qty) });
            continue;
        };
        matched_old.insert(ok);
        let mut kinds = Vec::new();
        let mut desc = Vec::new();
        if ok != k {
            kinds.push("model".to_string());
            desc.push("模型修改".to_string());
        }
        let mut cmp = |kind: &str, what: &str, a: &str, b: &str| {
            if a != b {
                kinds.push(kind.to_string());
                desc.push(format!("{what} {} → {}", if a.is_empty() { "-" } else { a }, if b.is_empty() { "-" } else { b }));
            }
        };
        cmp("qty", "数量", &o.qty.to_string(), &n.qty.to_string());
        cmp("material", "材料", &o.material, &n.material);
        cmp("pn", "料号", &o.pn, &n.pn);
        cmp("name", "名称", &o.name, &n.name);
        cmp("supplier", "加工方", &o.supplier, &n.supplier);
        if o.upload != n.upload {
            kinds.push("upload".into());
            desc.push("图纸更换".into());
        } else if o.sheet != n.sheet && !kinds.iter().any(|k| k == "model") {
            kinds.push("drawing".into());
            desc.push("图纸标注修改".into());
        }
        if !kinds.is_empty() {
            items.push(EcnItem { key: k.clone(), label: label(n), kinds, desc: desc.join("; ") });
        }
    }
    for (k, o) in old {
        if !matched_old.contains(k) {
            items.push(EcnItem { key: k.clone(), label: label(o), kinds: vec!["removed".into()], desc: "删除".into() });
        }
    }
    items
}

/// 出图设置的指纹: 去掉时间 / 计数 / 实例这些不算"图改了"的字段。
pub fn sheet_hash(s: &Sheet) -> String {
    let mut c = s.clone();
    c.compiled.clear();
    c.exported.clear();
    c.edits = 0;
    c.model_changed = false;
    c.member.clear();
    c.model_name.clear();
    c.volume = None;
    c.area = None;
    c.uploaded = None;
    let txt = serde_json::to_string(&c).unwrap_or_default();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in txt.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::Uploaded;

    fn snap(pn: &str, qty: u64, ids: &[&str]) -> PartSnap {
        PartSnap { pn: pn.into(), name: pn.into(), qty, ids: ids.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn ecn_new_release_vs_same_batch() {
        use std::collections::BTreeMap;
        let mut h = History::default();
        let r1: BTreeMap<String, PartSnap> = [("a".to_string(), snap("A", 2, &["e1"])), ("b".to_string(), snap("B", 1, &["e2"]))].into();
        // 第一次外发: 没有 ECN
        assert_eq!(h.record(r1.clone(), "2026-11-17", false), ("R1".into(), None));
        // 之后改了几次: a 改了模型(key 变、实体同)+ 数量, b 删了, c 新增 → 都在一条里
        let mut now: BTreeMap<String, PartSnap> = [("a2".to_string(), snap("A", 4, &["e1"])), ("c".to_string(), snap("C", 1, &["e3"]))].into();
        let items = diff(&h.last().unwrap().parts, &now);
        assert_eq!(items.len(), 3);
        let a = items.iter().find(|i| i.key == "a2").unwrap();
        assert_eq!(a.kinds, vec!["model", "qty"]);
        assert_eq!(a.desc, "模型修改; 数量 2 → 4");
        assert!(items.iter().any(|i| i.key == "b" && i.kinds == vec!["removed"]));
        assert!(items.iter().any(|i| i.key == "c" && i.kinds == vec!["added"]));
        // 同一批补发(还是 R1): 不新增 ECN, R1 的快照更新
        h.open_note = "客户改需求".into();
        assert_eq!(h.record(now.clone(), "2026-11-18", true), ("R1".into(), None));
        assert!(h.ecns.is_empty() && h.releases.len() == 1 && h.last().unwrap().parts.contains_key("a2"));
        // 再改材料 → 新外发 R2 → ECN-001(只含 R1 之后的改动)
        now.get_mut("a2").unwrap().material = "6061".into();
        h.open_note = "改材料".into();
        h.open_by = "frg".into();
        let (v, no) = h.record(now.clone(), "2026-11-19", false);
        assert_eq!((v.as_str(), no.as_deref()), ("R2", Some("ECN-001")));
        assert_eq!(h.ecns[0].items.len(), 1);
        assert_eq!(h.ecns[0].items[0].desc, "材料 - → 6061");
        assert_eq!(h.rev_rows("a2"), vec![("ECN-001".to_string(), "材料 - → 6061; 改材料".to_string(), "frg".to_string(), "2026-11-19".to_string())]);
        // R2 发出去之前又改了数量, 还是同一批 → ECN-001 重算(对比 R1)
        now.get_mut("a2").unwrap().qty = 6;
        assert_eq!(h.record(now.clone(), "2026-11-20", true), ("R2".into(), Some("ECN-001".into())));
        assert_eq!(h.ecns.len(), 1);
        assert_eq!(h.ecns[0].items[0].desc, "数量 4 → 6; 材料 - → 6061");
        // 模型改名跟着走; 变更表最多两行
        assert!(h.rename_key("a2", "a3"));
        assert_eq!(h.rev_rows("a3").len(), 1);
        now = [("a3".to_string(), snap("A", 7, &["e1"])), ("c".to_string(), snap("C", 1, &["e3"]))].into();
        h.record(now.clone(), "d", false);
        now.get_mut("a3").unwrap().qty = 8;
        h.record(now, "d", false);
        let rows = h.rev_rows("a3");
        assert_eq!(rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), vec!["ECN-002", "ECN-003"]);
        // 图纸指纹: 时间 / 计数不算改动, 公差算
        let mut s = Sheet::new("k", "P", "m", "n");
        let h0 = sheet_hash(&s);
        s.compiled = "x".into();
        s.edits = 5;
        assert_eq!(sheet_hash(&s), h0);
        s.dims_tol.insert("D".into(), "h7".into());
        assert_ne!(sheet_hash(&s), h0);
    }

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
        let e = Ecn { no: "ECN-001".into(), ver: "R1".into(), date: "2026-11-17".into(), items: vec![EcnItem { key: "a".into(), label: "Base".into(), kinds: vec!["qty".into()], desc: "数量 1 → 2".into() }], ..Default::default() };
        let out = write(&root, "R1", "2026-11-17", "total", &[pack], Some(&e)).unwrap();
        assert!(std::fs::read_to_string(out.join("ECN-001.csv")).unwrap().contains("数量 1 → 2"));
        assert!(out.join("总BOM.csv").exists() && out.join("CNC").join("CNC-BOM.csv").exists() && out.join("CNC").join("904002001.pdf").exists());
        assert!(out.join("CNC.zip").exists() && out.join("manifest.json").exists());
        assert_eq!(next_version(&root), "R2");
        let _ = std::fs::remove_dir_all(&d);
    }
}
