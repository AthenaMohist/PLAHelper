//! N3d: 尺寸记录(参数化的"记住尺寸, 以后反复改")。
//!
//! 存 `<文档目录>/.pla/models/<doc_key>/dims.json`(A9 映射表的位置; 设计见 docs/PRD/PRD-ai-modeling-v2.md §4)。
//! 面 id 每次修改都会变 → 记录里存面的**几何签名**(垫片 `dim.signature`), 用时经 `geo.resolve` 重新定位;
//! 本工具改完尺寸后按已知平移量预测新签名再定位, 刷新记录, 记录始终跟得上模型。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state_store::StateStore;

/// 目录与其它按文档存的状态一致(`.pla/<subdir>/<doc_key>/`)。
pub const STORE: StateStore = StateStore { subdir: "models", legacy_subdir: None, prefix: "D" };
const FILE: &str = "dims.json";

/// 一条尺寸记录。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DimRecord {
    /// D001, D002…(可改名)。
    pub name: String,
    /// "face" = 单面(厚度 / 半径 / 圆角半径); "distance" = 参考面 → 动的面组。
    pub mode: String,
    /// 尺寸类型(thickness / radius / filletRadius / distance), 界面显示用。
    pub kind: String,
    /// 面签名: face 模式 = [选中面]; distance 模式 = [参考面, 动的面…](顺序即选择顺序)。
    pub faces: Vec<Value>,
    /// face 模式的厚度: 所选对面的签名。
    #[serde(default)]
    pub opposite: Option<Value>,
    /// N3b 改哪一侧(厚度: 0 选中面 / 1 对面 / 2 两侧各半; 距离: 0 动的面 / 1 参考面 / 2 两边各半)。
    #[serde(default)]
    pub side: u8,
    /// N3c 距离: 1 = 以参考面为对称面向两侧移动(动的面分在两侧时)。
    #[serde(default)]
    pub sym: u8,
    /// 对面是圆柱时的测量点: None = 自动(孔 = 最近, 凸 = 最远) / 0 最近 / 1 轴心 / 2 最远。
    #[serde(default)]
    pub cyl: Option<u8>,
    /// 最近一次读到 / 改成的值(文档单位)。
    pub value: f64,
    #[serde(default)]
    pub units: String,
    #[serde(default)]
    pub created: u64,
    /// 收藏(列表里置顶)。
    #[serde(default)]
    pub fav: bool,
    /// P2: 稳定 id(改名不变; 旧数据载入时补上)。
    #[serde(default)]
    pub id: String,
    /// P2: 父级(组 / 以后的特征)的 id; None = 顶层。
    #[serde(default)]
    pub parent: Option<String>,
    /// P2: "manual" = 用户记录 / 挂入; "auto" = 特征工具生成(删特征时一起删)。
    #[serde(default = "origin_manual")]
    pub origin: String,
    /// 旧 P4"跟随组的驱动值"(已取消, 载入时迁移成表达式 = 组名变量, 然后清掉; 只为读旧文件保留)。
    #[serde(default, skip_serializing)]
    pub follow: bool,
    /// P5: 驱动这条尺寸的表达式(引用参数 / 其它尺寸名, 见 `params.rs`); None = 不绑定(值就是读到的值)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
}

fn origin_manual() -> String {
    "manual".into()
}

/// P2: 用户建的组(文件夹)。组里的尺寸可折叠查看、整组操作(组只做归类; 联动改用 P5 参数 / 表达式)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Group {
    pub id: String,
    /// G001…(可改名)。
    pub name: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub fav: bool,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub created: u64,
    /// P3: "group" = 用户建的文件夹; "hole" = 打孔工具生成的特征(删它时自动子项一起删)。
    #[serde(default = "kind_group")]
    pub kind: String,
    /// P3: 特征参数(孔: type / d / depth / cbD / cbDepth / cskD / std / usage …), 悬停详情与以后"按规格重做"用。
    #[serde(default)]
    pub spec: Option<Value>,
    /// 旧 P4 组驱动值(已取消, 载入时迁移成同名变量; 只为读旧文件保留)。
    #[serde(default, skip_serializing)]
    pub value: Option<f64>,
}

fn kind_group() -> String {
    "group".into()
}

/// P2: 文件格式 v2 = { version, dims, groups, next_id }。v1 = 只有尺寸的数组(载入时迁移)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DimsFile {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub dims: Vec<DimRecord>,
    #[serde(default)]
    pub groups: Vec<Group>,
    /// 下一个 id 序号(id = d<n> / g<n> / p<n>)。
    #[serde(default)]
    pub next_id: u64,
    /// P5: 参数(常量 / 变量 / 函数)。
    #[serde(default)]
    pub params: Vec<crate::params::Param>,
}

/// v3 = 加了 params 与尺寸的 expr; 旧的组驱动值 / 跟随迁移成参数 + 表达式。
const FILE_VERSION: u32 = 3;
const GROUP_PREFIX: &str = "G";

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl DimsFile {
    fn new_id(&mut self, tag: char) -> String {
        self.next_id += 1;
        format!("{tag}{}", self.next_id)
    }

    /// 补齐缺的 id、清掉指向不存在父级的引用; 返回是否改过(改过要写回)。
    fn normalize(&mut self) -> bool {
        let mut changed = self.version != FILE_VERSION;
        self.version = FILE_VERSION;
        for i in 0..self.dims.len() {
            if self.dims[i].id.is_empty() {
                let id = self.new_id('d');
                self.dims[i].id = id;
                changed = true;
            }
        }
        for i in 0..self.groups.len() {
            if self.groups[i].id.is_empty() {
                let id = self.new_id('g');
                self.groups[i].id = id;
                changed = true;
            }
        }
        let gids: Vec<String> = self.groups.iter().map(|g| g.id.clone()).collect();
        for d in &mut self.dims {
            if d.parent.as_ref().is_some_and(|p| !gids.contains(p)) {
                d.parent = None;
                changed = true;
            }
        }
        changed |= self.migrate_group_values();
        for i in 0..self.params.len() {
            if self.params[i].id.is_empty() {
                let id = self.new_id('p');
                self.params[i].id = id;
                changed = true;
            }
        }
        changed
    }

    /// 旧 P4 → P5: 有驱动值的用户组 → 同名变量(名字不合法 / 重名时改写成合法唯一名); 组里跟随的尺寸 → 表达式 = 该变量。
    fn migrate_group_values(&mut self) -> bool {
        let mut changed = false;
        for gi in 0..self.groups.len() {
            let Some(v) = self.groups[gi].value.take() else { continue };
            changed = true;
            if self.groups[gi].kind != "group" {
                continue;
            }
            let gid = self.groups[gi].id.clone();
            let mut base: String = self.groups[gi].name.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
            if !crate::params::valid_name(&base) {
                base = format!("G_{base}");
            }
            let mut name = base.clone();
            let mut k = 2;
            while self.name_taken(&name) {
                name = format!("{base}_{k}");
                k += 1;
            }
            let id = self.new_id('p');
            self.params.push(crate::params::Param { id, name: name.clone(), kind: "var".into(), expr: fmt_num(v), note: String::new() });
            for d in &mut self.dims {
                if d.follow && d.parent.as_deref() == Some(gid.as_str()) {
                    d.expr = Some(name.clone());
                }
            }
        }
        for d in &mut self.dims {
            if d.follow {
                d.follow = false;
                changed = true;
            }
        }
        changed
    }

    /// 名字是否已被参数或尺寸占用。
    pub fn name_taken(&self, name: &str) -> bool {
        self.params.iter().any(|p| p.name == name) || self.dims.iter().any(|d| d.name == name)
    }
}

fn fmt_num(v: f64) -> String {
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// 读整个文件(v1 数组自动迁移成 v2 并写回; 没有文件 → 空)。
pub fn load_file(doc_key: &str) -> DimsFile {
    let raw = std::fs::read_to_string(path(doc_key)).ok();
    let txt = raw.as_deref().map(|s| s.trim_start_matches('\u{feff}')).unwrap_or("");
    let mut f = if txt.trim_start().starts_with('[') {
        DimsFile { dims: serde_json::from_str(txt).unwrap_or_default(), ..Default::default() }
    } else {
        serde_json::from_str::<DimsFile>(txt).unwrap_or_default()
    };
    if f.normalize() && raw.is_some() {
        let _ = save_file(doc_key, &f);
    }
    f
}

/// 写整个文件(先写临时文件再改名, 不留半截文件)。
pub fn save_file(doc_key: &str, f: &DimsFile) -> Result<(), String> {
    let p = path(doc_key);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(f).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

/// 全部组。
pub fn load_groups(doc_key: &str) -> Vec<Group> {
    load_file(doc_key).groups
}

/// 新建组(自动命名 G001…), 返回 id。
pub fn add_group(doc_key: &str) -> Result<String, String> {
    let mut f = load_file(doc_key);
    let max = f.groups.iter().filter_map(|g| g.name.strip_prefix(GROUP_PREFIX).and_then(|n| n.parse::<u32>().ok())).max().unwrap_or(0);
    let id = f.new_id('g');
    f.groups.push(Group { id: id.clone(), name: format!("{GROUP_PREFIX}{:03}", max + 1), parent: None, fav: false, collapsed: false, created: now_secs(), kind: kind_group(), spec: None, value: None });
    save_file(doc_key, &f)?;
    Ok(id)
}

/// 特征名前缀(孔 = H)。
pub fn feature_prefix(kind: &str) -> &'static str {
    match kind {
        "hole" => "H",
        _ => GROUP_PREFIX,
    }
}

/// P3: 新建一个特征(工具生成, 如孔 H001)及其自动子尺寸(origin = auto, 名字 = "<特征名>.<后缀>"), 返回 (id, 名字)。
/// children: (后缀, 尺寸记录模板); 子尺寸重名时加序号。
pub fn add_feature(doc_key: &str, kind: &str, spec: Value, children: Vec<(String, DimRecord)>) -> Result<(String, String), String> {
    let mut f = load_file(doc_key);
    let pre = feature_prefix(kind);
    let max = f.groups.iter().filter(|g| g.kind == kind).filter_map(|g| g.name.strip_prefix(pre).and_then(|n| n.parse::<u32>().ok())).max().unwrap_or(0);
    let name = format!("{pre}{:03}", max + 1);
    let id = f.new_id('g');
    f.groups.push(Group { id: id.clone(), name: name.clone(), parent: None, fav: false, collapsed: false, created: now_secs(), kind: kind.into(), spec: Some(spec), value: None });
    for (suffix, mut rec) in children {
        let mut nm = format!("{name}.{suffix}");
        let mut k = 2;
        while f.dims.iter().any(|d| d.name == nm) {
            nm = format!("{name}.{suffix}{k}");
            k += 1;
        }
        rec.name = nm;
        rec.id = f.new_id('d');
        rec.parent = Some(id.clone());
        rec.origin = "auto".into();
        if rec.created == 0 {
            rec.created = now_secs();
        }
        f.dims.push(rec);
    }
    save_file(doc_key, &f)?;
    Ok((id, name))
}

/// 组改名(组之间不重名)。
pub fn rename_group(doc_key: &str, id: &str, new: &str) -> Result<(), String> {
    let new = new.trim();
    if new.is_empty() {
        return Err("empty name".into());
    }
    let mut f = load_file(doc_key);
    if f.groups.iter().any(|g| g.name == new && g.id != id) {
        return Err(format!("group {new} already exists"));
    }
    f.groups.iter_mut().find(|g| g.id == id).ok_or(format!("no group {id}"))?.name = new.to_string();
    save_file(doc_key, &f)
}

/// 改组的收藏 / 折叠。
pub fn set_group_flags(doc_key: &str, id: &str, fav: Option<bool>, collapsed: Option<bool>) -> Result<(), String> {
    let mut f = load_file(doc_key);
    let g = f.groups.iter_mut().find(|g| g.id == id).ok_or(format!("no group {id}"))?;
    if let Some(x) = fav {
        g.fav = x;
    }
    if let Some(x) = collapsed {
        g.collapsed = x;
    }
    save_file(doc_key, &f)
}

/// 删组 / 特征: 手动挂入的尺寸回到父级(顶层); 特征(kind ≠ group)自动生成的子尺寸一起删(连同缩略图)。
/// 只删记录, 不动模型。
pub fn delete_group(doc_key: &str, id: &str) -> Result<(), String> {
    let mut f = load_file(doc_key);
    let Some(pos) = f.groups.iter().position(|g| g.id == id) else { return Err(format!("no group {id}")) };
    let up = f.groups[pos].parent.clone();
    let feature = f.groups[pos].kind != "group";
    f.groups.remove(pos);
    let mut gone = Vec::new();
    f.dims.retain(|d| {
        let drop = feature && d.parent.as_deref() == Some(id) && d.origin == "auto";
        if drop {
            gone.push(d.name.clone());
        }
        !drop
    });
    for d in &mut f.dims {
        if d.parent.as_deref() == Some(id) {
            d.parent = up.clone();
        }
    }
    save_file(doc_key, &f)?;
    for nm in gone {
        let _ = std::fs::remove_file(png_path(doc_key, &nm));
    }
    Ok(())
}

/// 把若干尺寸(按名字)挂到组下(None = 移到顶层)。
pub fn set_parent(doc_key: &str, names: &[String], parent: Option<&str>) -> Result<usize, String> {
    let mut f = load_file(doc_key);
    if let Some(p) = parent {
        if !f.groups.iter().any(|g| g.id == p) {
            return Err(format!("no group {p}"));
        }
    }
    let mut n = 0;
    for d in &mut f.dims {
        if names.contains(&d.name) {
            d.parent = parent.map(String::from);
            n += 1;
        }
    }
    save_file(doc_key, &f)?;
    Ok(n)
}

// ---------------------------------------------------------------- P5 参数 / 表达式

/// 新建参数(kind = const / var / func), 默认名 C1 / V1 / F1…, 默认表达式: 数 = "0"; 函数 = "0"。返回名字。
pub fn add_param(doc_key: &str, kind: &str) -> Result<String, String> {
    if !crate::params::KINDS.contains(&kind) {
        return Err(format!("unknown parameter kind {kind}"));
    }
    let mut f = load_file(doc_key);
    let taken: std::collections::BTreeSet<String> = f.params.iter().map(|p| p.name.clone()).chain(f.dims.iter().map(|d| d.name.clone())).collect();
    let name = crate::params::default_name(kind, &taken);
    let id = f.new_id('p');
    f.params.push(crate::params::Param { id, name: name.clone(), kind: kind.into(), expr: "0".into(), note: String::new() });
    save_file(doc_key, &f)?;
    Ok(name)
}

/// 改参数: 名字(合法、不与参数 / 尺寸重名; 引用它的表达式跟着改)/ 类型 / 表达式。
pub fn update_param(doc_key: &str, id: &str, name: Option<&str>, kind: Option<&str>, expr: Option<&str>) -> Result<(), String> {
    let mut f = load_file(doc_key);
    let i = f.params.iter().position(|p| p.id == id).ok_or(format!("no parameter {id}"))?;
    if let Some(k) = kind {
        if !crate::params::KINDS.contains(&k) {
            return Err(format!("unknown parameter kind {k}"));
        }
        f.params[i].kind = k.into();
    }
    if let Some(e) = expr {
        f.params[i].expr = e.trim().to_string();
    }
    if let Some(n) = name.map(str::trim) {
        let old = f.params[i].name.clone();
        if n != old {
            if !crate::params::valid_name(n) {
                return Err(format!("invalid name {n}"));
            }
            if f.name_taken(n) {
                return Err(format!("name {n} already used"));
            }
            f.params[i].name = n.to_string();
            rename_refs_in(&mut f, &old, n);
        }
    }
    save_file(doc_key, &f)
}

/// 删参数。还有表达式引用它 → 拒绝并列出引用者(先改掉那些表达式)。
pub fn delete_param(doc_key: &str, id: &str) -> Result<(), String> {
    let mut f = load_file(doc_key);
    let i = f.params.iter().position(|p| p.id == id).ok_or(format!("no parameter {id}"))?;
    let name = f.params[i].name.clone();
    let users = users_of(&f, &name);
    if !users.is_empty() {
        return Err(format!("{name} is used by {}", users.join(", ")));
    }
    f.params.remove(i);
    save_file(doc_key, &f)
}

/// 设 / 清尺寸的表达式(None 或空 = 不绑定)。
pub fn set_dim_expr(doc_key: &str, name: &str, expr: Option<&str>) -> Result<(), String> {
    let mut f = load_file(doc_key);
    let d = f.dims.iter_mut().find(|d| d.name == name).ok_or(format!("no dimension named {name}"))?;
    d.expr = expr.map(str::trim).filter(|e| !e.is_empty()).map(String::from);
    save_file(doc_key, &f)
}

/// 引用某名字的参数 / 尺寸。
pub fn users_of(f: &DimsFile, name: &str) -> Vec<String> {
    let mut out: Vec<String> = f.params.iter().filter(|p| crate::params::refs(&p.expr).iter().any(|r| r == name)).map(|p| p.name.clone()).collect();
    out.extend(f.dims.iter().filter(|d| d.expr.as_deref().is_some_and(|e| crate::params::refs(e).iter().any(|r| r == name))).map(|d| d.name.clone()));
    out
}

fn rename_refs_in(f: &mut DimsFile, old: &str, new: &str) {
    for p in &mut f.params {
        p.expr = crate::params::rename_ref(&p.expr, old, new);
    }
    for d in &mut f.dims {
        if let Some(e) = d.expr.as_mut() {
            *e = crate::params::rename_ref(e, old, new);
        }
    }
}

/// 当前文件求值(参数 + 尺寸)。
pub fn evaluate(f: &DimsFile) -> crate::params::Eval {
    crate::params::evaluate(&crate::params::Inputs {
        params: &f.params,
        dims: f.dims.iter().map(|d| (d.name.as_str(), d.value, d.expr.as_deref())).collect(),
    })
}

/// 记录的缩略图(记录 / 更新时截的视口, 悬停详情里显示): `<目录>/<名字>.png`。
pub fn png_path(doc_key: &str, name: &str) -> PathBuf {
    STORE.dir(doc_key).join(format!("{name}.png"))
}

/// 列表显示顺序: 收藏在前, 再按名字。
pub fn sorted(mut list: Vec<DimRecord>) -> Vec<DimRecord> {
    list.sort_by(|a, b| b.fav.cmp(&a.fav).then_with(|| a.name.cmp(&b.name)));
    list
}

/// 改名(连同缩略图); 新名不能为空或与其它记录重名。
pub fn rename(doc_key: &str, old: &str, new: &str) -> Result<(), String> {
    let new = new.trim();
    if new.is_empty() {
        return Err("empty name".into());
    }
    let mut f = load_file(doc_key);
    if new != old && f.name_taken(new) {
        return Err(format!("name {new} already exists"));
    }
    match f.dims.iter_mut().find(|r| r.name == old) {
        Some(r) => r.name = new.to_string(),
        None => return Err(format!("no dimension named {old}")),
    }
    // P5: 引用旧名字的表达式跟着改
    if new != old {
        rename_refs_in(&mut f, old, new);
    }
    save_file(doc_key, &f)?;
    let _ = std::fs::rename(png_path(doc_key, old), png_path(doc_key, new));
    Ok(())
}

pub fn set_fav(doc_key: &str, name: &str, fav: bool) -> Result<(), String> {
    let mut list = load(doc_key);
    match list.iter_mut().find(|r| r.name == name) {
        Some(r) => r.fav = fav,
        None => return Err(format!("no dimension named {name}")),
    }
    save(doc_key, &list)
}

/// 批量删除(连同缩略图), 返回删了几条。
pub fn delete_many(doc_key: &str, names: &[String]) -> Result<usize, String> {
    let mut list = load(doc_key);
    let n = list.len();
    list.retain(|r| !names.contains(&r.name));
    save(doc_key, &list)?;
    for nm in names {
        let _ = std::fs::remove_file(png_path(doc_key, nm));
    }
    Ok(n - list.len())
}

fn path(doc_key: &str) -> PathBuf {
    STORE.dir(doc_key).join(FILE)
}

/// 读全部尺寸记录(没有文件 / 读不了 → 空)。
pub fn load(doc_key: &str) -> Vec<DimRecord> {
    load_file(doc_key).dims
}

/// 写全部尺寸记录(组原样保留; 新记录补 id)。
pub fn save(doc_key: &str, list: &[DimRecord]) -> Result<(), String> {
    let mut f = load_file(doc_key);
    f.dims = list.to_vec();
    f.normalize();
    save_file(doc_key, &f)
}

/// 下一个自动名: D + 三位序号(取现有最大号 + 1)。
pub fn next_name(list: &[DimRecord]) -> String {
    let max = list
        .iter()
        .filter_map(|r| r.name.strip_prefix(STORE.prefix).and_then(|n| n.parse::<u32>().ok()))
        .max()
        .unwrap_or(0);
    format!("{}{:03}", STORE.prefix, max + 1)
}

/// 新增一条(自动命名), 返回名字。
pub fn add(doc_key: &str, mut rec: DimRecord) -> Result<String, String> {
    let mut f = load_file(doc_key);
    rec.name = next_name(&f.dims);
    if rec.created == 0 {
        rec.created = now_secs();
    }
    rec.id = f.new_id('d');
    let name = rec.name.clone();
    f.dims.push(rec);
    save_file(doc_key, &f)?;
    Ok(name)
}

/// 按名字替换一条(刷新签名 / 值后写回)。
pub fn update(doc_key: &str, rec: &DimRecord) -> Result<(), String> {
    let mut list = load(doc_key);
    match list.iter_mut().find(|r| r.name == rec.name) {
        Some(r) => *r = rec.clone(),
        None => return Err(format!("no dimension named {}", rec.name)),
    }
    save(doc_key, &list)
}

pub fn delete(doc_key: &str, name: &str) -> Result<(), String> {
    match delete_many(doc_key, &[name.to_string()])? {
        0 => Err(format!("no dimension named {name}")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(kind: &str) -> DimRecord {
        DimRecord {
            name: String::new(),
            mode: "face".into(),
            kind: kind.into(),
            faces: vec![json!({ "surface": "Plane", "center": [1, 2, 3], "normal": [0, 0, 1] })],
            opposite: None,
            side: 0,
            sym: 0,
            cyl: None,
            value: 5.0,
            units: "millimeter".into(),
            created: 0,
            fav: false,
            id: String::new(),
            parent: None,
            origin: "manual".into(),
            follow: false,
            expr: None,
        }
    }

    #[test]
    fn v1_migration_and_groups() {
        let doc = format!("__test_dims_grp_{}", std::process::id());
        let _ = std::fs::remove_dir_all(STORE.dir(&doc));
        // v1 = 只有尺寸的数组, 没有 id / parent / origin
        std::fs::create_dir_all(STORE.dir(&doc)).unwrap();
        let mut a = rec("thickness");
        a.name = "D001".into();
        let mut v = serde_json::to_value(vec![a]).unwrap();
        for k in ["id", "parent", "origin"] {
            v[0].as_object_mut().unwrap().remove(k);
        }
        std::fs::write(path(&doc), serde_json::to_vec(&v).unwrap()).unwrap();
        let f = load_file(&doc);
        assert_eq!((f.version, f.dims.len(), f.dims[0].id.as_str(), f.dims[0].origin.as_str()), (FILE_VERSION, 1, "d1", "manual"));
        assert!(std::fs::read_to_string(path(&doc)).unwrap().contains(&format!("\"version\": {FILE_VERSION}")), "迁移后写回");
        // 组: 新建 / 挂入 / 改名 / 折叠 / 删组(子项回顶层); 尺寸增删不影响组
        let g = add_group(&doc).unwrap();
        assert_eq!(load_groups(&doc)[0].name, "G001");
        add(&doc, rec("radius")).unwrap();
        assert_eq!(set_parent(&doc, &["D001".into(), "D002".into()], Some(&g)).unwrap(), 2);
        assert!(set_parent(&doc, &["D001".into()], Some("g999")).is_err());
        rename_group(&doc, &g, "外壳壁厚").unwrap();
        set_group_flags(&doc, &g, Some(true), Some(true)).unwrap();
        let gg = &load_groups(&doc)[0];
        assert_eq!((gg.name.as_str(), gg.fav, gg.collapsed), ("外壳壁厚", true, true));
        let mut l = load(&doc);
        l[0].value = 9.0;
        save(&doc, &l).unwrap();
        assert_eq!(load_groups(&doc).len(), 1, "存尺寸不丢组");
        assert!(load(&doc).iter().all(|d| d.parent.as_deref() == Some(g.as_str())));
        delete_group(&doc, &g).unwrap();
        assert!(load_groups(&doc).is_empty());
        assert!(load(&doc).iter().all(|d| d.parent.is_none()), "删组: 子项回顶层");
        let ids: Vec<String> = load(&doc).iter().map(|d| d.id.clone()).collect();
        assert!(ids[0] != ids[1] && !ids[1].is_empty(), "id 唯一");
        // P3 特征: 自动子尺寸(名字 = 特征名.后缀, origin auto); 手动挂入的在删特征时只解除挂靠
        let (hid, hname) = add_feature(&doc, "hole", json!({ "type": "simple", "d": 4.5 }), vec![("R".into(), rec("radius")), ("R".into(), rec("radius"))]).unwrap();
        assert_eq!(hname, "H001");
        let l = load(&doc);
        let kids: Vec<&DimRecord> = l.iter().filter(|d| d.parent.as_deref() == Some(hid.as_str())).collect();
        assert_eq!(kids.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), vec!["H001.R", "H001.R2"], "重名加序号");
        assert!(kids.iter().all(|d| d.origin == "auto"));
        set_parent(&doc, &["D001".into()], Some(&hid)).unwrap();
        assert_eq!(add_feature(&doc, "hole", json!({}), vec![]).unwrap().1, "H002");
        delete_group(&doc, &hid).unwrap();
        let l = load(&doc);
        assert!(!l.iter().any(|d| d.name.starts_with("H001.")), "自动子尺寸随特征删除");
        assert!(l.iter().any(|d| d.name == "D001" && d.parent.is_none()), "手动挂入的回到顶层");
        assert_eq!(load_groups(&doc).iter().map(|g| (g.name.as_str(), g.kind.as_str())).collect::<Vec<_>>(), vec![("H002", "hole")]);
        let _ = std::fs::remove_dir_all(STORE.dir(&doc));
    }

    #[test]
    fn params_exprs_and_p4_migration() {
        let doc = format!("__test_dims_params_{}", std::process::id());
        let _ = std::fs::remove_dir_all(STORE.dir(&doc));
        // 旧 P4 文件: 组 "外壳 壁厚" 驱动值 2.5, D001 跟随、D002 不跟随
        std::fs::create_dir_all(STORE.dir(&doc)).unwrap();
        let old = serde_json::json!({ "version": 2, "next_id": 3,
            "groups": [{ "id": "g1", "name": "外壳 壁厚", "kind": "group", "value": 2.5 }],
            "dims": [
                { "name": "D001", "mode": "face", "kind": "thickness", "faces": [], "value": 2.0, "id": "d1", "parent": "g1", "follow": true },
                { "name": "D002", "mode": "face", "kind": "thickness", "faces": [], "value": 4.0, "id": "d2", "parent": "g1", "follow": false }
            ] });
        std::fs::write(path(&doc), serde_json::to_vec(&old).unwrap()).unwrap();
        let f = load_file(&doc);
        assert_eq!(f.params.len(), 1);
        assert_eq!((f.params[0].name.as_str(), f.params[0].kind.as_str(), f.params[0].expr.as_str()), ("外壳_壁厚", "var", "2.5"), "组值 → 同名变量(空格换成 _)");
        assert_eq!(f.dims[0].expr.as_deref(), Some("外壳_壁厚"), "跟随的尺寸 → 表达式 = 该变量");
        assert_eq!(f.dims[1].expr, None);
        assert_eq!(f.groups[0].value, None);
        let raw = std::fs::read_to_string(path(&doc)).unwrap();
        assert!(!raw.contains("\"follow\"") && !raw.contains("\"value\": 2.5"), "旧字段不再写出: {raw}");
        assert_eq!(evaluate(&f).changes(&[("D001".to_string(), 2.0)].into_iter().collect(), 5e-4), vec![("D001".to_string(), 2.5)]);
        // 参数增改删 + 重名 / 非法名 / 改名跟着改表达式 / 被引用不能删
        let v = add_param(&doc, "var").unwrap();
        assert_eq!(v, "V1");
        let vid = load_file(&doc).params.iter().find(|p| p.name == "V1").unwrap().id.clone();
        update_param(&doc, &vid, None, None, Some("100")).unwrap();
        set_dim_expr(&doc, "D002", Some(" V1 / 25 ")).unwrap();
        assert_eq!(load(&doc)[1].expr.as_deref(), Some("V1 / 25"));
        assert!(update_param(&doc, &vid, Some("D001"), None, None).is_err(), "与尺寸重名");
        assert!(update_param(&doc, &vid, Some("1x"), None, None).is_err(), "非法名");
        update_param(&doc, &vid, Some("W"), None, None).unwrap();
        assert_eq!(load(&doc)[1].expr.as_deref(), Some("W / 25"), "改名: 引用跟着改");
        assert!(delete_param(&doc, &vid).unwrap_err().contains("D002"), "被引用不能删");
        // 尺寸改名: 表达式里的引用跟着改; 与参数重名拒绝
        let fid = { add_param(&doc, "func").unwrap(); load_file(&doc).params.iter().find(|p| p.name == "F1").unwrap().id.clone() };
        update_param(&doc, &fid, None, None, Some("D001 * 2")).unwrap();
        rename(&doc, "D001", "壁厚").unwrap();
        assert_eq!(load_file(&doc).params.iter().find(|p| p.name == "F1").unwrap().expr, "壁厚 * 2");
        assert!(rename(&doc, "壁厚", "W").is_err(), "与参数重名");
        let e = evaluate(&load_file(&doc));
        assert_eq!(e.targets["D002"], Ok(4.0));
        assert_eq!(e.values["F1"], Ok(5.0), "壁厚 绑定 外壳_壁厚 = 2.5 → ×2");
        set_dim_expr(&doc, "D002", None).unwrap();
        delete_param(&doc, &vid).unwrap();
        let _ = std::fs::remove_dir_all(STORE.dir(&doc));
    }

    #[test]
    fn add_update_delete_roundtrip() {
        let doc = format!("__test_dims_{}", std::process::id());
        let _ = std::fs::remove_dir_all(STORE.dir(&doc));
        assert!(load(&doc).is_empty());
        assert_eq!(add(&doc, rec("thickness")).unwrap(), "D001");
        assert_eq!(add(&doc, rec("radius")).unwrap(), "D002");
        let mut l = load(&doc);
        assert_eq!(l.len(), 2);
        assert!(l[0].created > 0);
        l[0].value = 8.0;
        update(&doc, &l[0]).unwrap();
        assert_eq!(load(&doc)[0].value, 8.0);
        delete(&doc, "D001").unwrap();
        assert_eq!(load(&doc).iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["D002"]);
        assert_eq!(add(&doc, rec("thickness")).unwrap(), "D003", "取最大号 + 1, 不复用删掉的号");
        assert!(delete(&doc, "nope").is_err());
        // 改名(连同缩略图)/ 重名拒绝 / 收藏置顶 / 批量删
        std::fs::write(png_path(&doc, "D002"), b"png").unwrap();
        rename(&doc, "D002", "壁厚").unwrap();
        assert!(png_path(&doc, "壁厚").exists() && !png_path(&doc, "D002").exists());
        assert!(rename(&doc, "壁厚", "D003").is_err(), "重名");
        assert!(rename(&doc, "壁厚", "  ").is_err(), "空名");
        set_fav(&doc, "D003", true).unwrap();
        assert_eq!(sorted(load(&doc))[0].name, "D003", "收藏置顶");
        assert_eq!(delete_many(&doc, &["壁厚".into(), "D003".into()]).unwrap(), 2);
        assert!(load(&doc).is_empty() && !png_path(&doc, "壁厚").exists());
        let _ = std::fs::remove_dir_all(STORE.dir(&doc));
    }
}
