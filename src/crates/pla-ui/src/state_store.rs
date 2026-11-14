//! R34: "状态存档墙"的公共存储 —— 视口状态(VS…)与材质状态(MS…)共用。
//!
//! 布局: `<文档目录>/.pla/<subdir>/<doc_key>/<PREFIX>001.json` + 同名 `.png`(缩略图)。
//! JSON = `{doc_key, name, fav, created, state}`, state 是垫片回包的原样 JSON 字符串,
//! 恢复时原样交回垫片。定位不到文档时退回 exe 同级旧位置(有旧位置的会迁移)。

use std::path::PathBuf;

use pla_bridge::i18n::tr;
use pla_bridge::t;

/// 单条存档元数据(列表用)。
#[derive(Debug, Clone)]
pub struct StateMeta {
    pub name: String,
    pub fav: bool,
    pub created: String,
}

/// 一类存档(目录名 + 序号前缀)。
#[derive(Debug, Clone, Copy)]
pub struct StateStore {
    /// `.pla/` 下的子目录名, 如 "viewstates" / "materials"。
    pub subdir: &'static str,
    /// exe 同级的旧目录名(迁移来源); None = 没有旧位置。
    pub legacy_subdir: Option<&'static str>,
    /// 自动命名前缀, 如 "VS" / "MS"。
    pub prefix: &'static str,
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn write_json(path: &std::path::Path, v: &serde_json::Value) -> Result<(), String> {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(v).map_err(|e| t!("序列化失败: {e}", "Serialization failed: {e}"))?,
    )
    .map_err(|e| t!("写文件失败: {e}", "Failed to write file: {e}"))
}

impl StateStore {
    /// 某文档的存档目录 —— 跟随文档。
    pub fn dir(&self, doc_key: &str) -> PathBuf {
        let fallback = exe_dir()
            .join(self.legacy_subdir.unwrap_or(self.subdir))
            .join(super::snapshot::sanitize_filename(doc_key));
        let dir = super::snapshot::doc_side_dir(doc_key, self.subdir).unwrap_or_else(|| fallback.clone());
        if dir != fallback && self.legacy_subdir.is_some() {
            super::snapshot::migrate_dir(&fallback, &dir);
        }
        dir
    }

    /// 下一个序号名 PREFIX001/PREFIX002…
    fn next_stem(&self, doc_key: &str) -> String {
        let mut max = 0u32;
        if let Ok(rd) = std::fs::read_dir(self.dir(doc_key)) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                let stem = n.strip_suffix(".json").or_else(|| n.strip_suffix(".png")).unwrap_or("");
                if let Some(num) = stem.strip_prefix(self.prefix) {
                    if let Ok(v) = num.parse::<u32>() {
                        max = max.max(v);
                    }
                }
            }
        }
        format!("{}{:03}", self.prefix, max + 1)
    }

    fn entry(doc_key: &str, name: &str, fav: bool, state: &str) -> serde_json::Value {
        serde_json::json!({
            "doc_key": doc_key,
            "name": name,
            "fav": fav,
            "created": super::snapshot::now_local_display(),
            "state": state,
        })
    }

    /// 保存一条(state = 垫片回包 JSON)。返回自动生成的名字。
    pub fn save(&self, doc_key: &str, state: &str, png: Option<&[u8]>) -> Result<String, String> {
        if state.trim().is_empty() {
            return Err(tr("状态为空", "State is empty").into());
        }
        let dir = self.dir(doc_key);
        std::fs::create_dir_all(&dir).map_err(|e| t!("创建目录失败: {e}", "Failed to create directory: {e}"))?;
        let stem = self.next_stem(doc_key);
        write_json(&dir.join(format!("{stem}.json")), &Self::entry(doc_key, &stem, false, state))?;
        if let Some(bytes) = png {
            let _ = std::fs::write(dir.join(format!("{stem}.png")), bytes);
        }
        Ok(stem)
    }

    /// R35: 用当前状态覆盖已有的一条(名字、收藏保留; 时间改为现在; 有新截图则替换)。
    pub fn update(&self, doc_key: &str, name: &str, state: &str, png: Option<&[u8]>) -> Result<(), String> {
        if state.trim().is_empty() {
            return Err(tr("状态为空", "State is empty").into());
        }
        let dir = self.dir(doc_key);
        let path = dir.join(format!("{name}.json"));
        let fav = std::fs::read_to_string(&path)
            .ok()
            .and_then(|d| serde_json::from_str::<serde_json::Value>(&d).ok())
            .and_then(|v| v["fav"].as_bool())
            .unwrap_or(false);
        write_json(&path, &Self::entry(doc_key, name, fav, state))?;
        if let Some(bytes) = png {
            let _ = std::fs::write(dir.join(format!("{name}.png")), bytes);
        }
        Ok(())
    }

    /// 读取一条的本体 JSON。
    pub fn load(&self, doc_key: &str, name: &str) -> Option<String> {
        let data = std::fs::read_to_string(self.dir(doc_key).join(format!("{name}.json"))).ok()?;
        let v: serde_json::Value = serde_json::from_str(&data).ok()?;
        v["state"].as_str().map(String::from)
    }

    /// 列出全部(收藏优先, 再按名字)。
    pub fn list(&self, doc_key: &str) -> Vec<StateMeta> {
        let dir = self.dir(doc_key);
        let mut out: Vec<StateMeta> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                let Some(stem) = n.strip_suffix(".json") else { continue };
                let meta = std::fs::read_to_string(dir.join(&n))
                    .ok()
                    .and_then(|d| serde_json::from_str::<serde_json::Value>(&d).ok());
                if let Some(m) = meta {
                    out.push(StateMeta {
                        name: stem.to_string(),
                        fav: m["fav"].as_bool().unwrap_or(false),
                        created: m["created"].as_str().unwrap_or("").to_string(),
                    });
                }
            }
        }
        out.sort_by(|a, b| b.fav.cmp(&a.fav).then_with(|| a.name.cmp(&b.name)));
        out
    }

    /// 收藏/取消收藏。
    pub fn set_fav(&self, doc_key: &str, name: &str, fav: bool) -> Result<(), String> {
        let path = self.dir(doc_key).join(format!("{name}.json"));
        let data = std::fs::read_to_string(&path).map_err(|e| t!("读取失败: {e}", "Read failed: {e}"))?;
        let mut v: serde_json::Value =
            serde_json::from_str(&data).map_err(|e| t!("解析失败: {e}", "Parse failed: {e}"))?;
        v["fav"] = serde_json::Value::Bool(fav);
        write_json(&path, &v)
    }

    /// 改名(连同截图)。
    pub fn rename(&self, doc_key: &str, old: &str, new: &str) -> Result<(), String> {
        let new = new.trim();
        if new.is_empty() {
            return Err(tr("新名称为空", "New name is empty").into());
        }
        if new == old {
            return Ok(());
        }
        let dir = self.dir(doc_key);
        let pj = dir.join(format!("{old}.json"));
        let nj = dir.join(format!("{new}.json"));
        if nj.exists() {
            return Err(t!("「{new}」已存在", "\"{new}\" already exists"));
        }
        let data = std::fs::read_to_string(&pj).map_err(|e| t!("读取失败: {e}", "Read failed: {e}"))?;
        let mut v: serde_json::Value =
            serde_json::from_str(&data).map_err(|e| t!("解析失败: {e}", "Parse failed: {e}"))?;
        v["name"] = serde_json::Value::String(new.to_string());
        write_json(&nj, &v)?;
        std::fs::remove_file(&pj).map_err(|e| t!("移除旧文件失败: {e}", "Failed to remove old file: {e}"))?;
        let pp = dir.join(format!("{old}.png"));
        if pp.exists() {
            let np = dir.join(format!("{new}.png"));
            std::fs::rename(&pp, &np).map_err(|e| t!("改名截图失败: {e}", "Failed to rename screenshot: {e}"))?;
        }
        Ok(())
    }

    /// 删除(含截图)。支持批量, 返回删除个数。
    pub fn delete(&self, doc_key: &str, names: &[String]) -> Result<usize, String> {
        let dir = self.dir(doc_key);
        let mut n = 0;
        for name in names {
            let mut ok = true;
            for f in [format!("{name}.json"), format!("{name}.png")] {
                if let Err(e) = std::fs::remove_file(dir.join(&f)) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        ok = false;
                    }
                }
            }
            if ok {
                n += 1;
            }
        }
        Ok(n)
    }

    /// 打开目录(explorer)。
    pub fn open_dir(&self, doc_key: &str) -> Result<(), String> {
        let dir = self.dir(doc_key);
        std::fs::create_dir_all(&dir).map_err(|e| t!("创建目录失败: {e}", "Failed to create directory: {e}"))?;
        std::process::Command::new("explorer")
            .arg(dir.as_os_str())
            .spawn()
            .map(|_| ())
            .map_err(|e| t!("打开失败: {e}", "Failed to open: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: StateStore = StateStore { subdir: "__test_store", legacy_subdir: None, prefix: "TS" };

    #[test]
    fn update_overwrites_state_and_keeps_name_and_fav() {
        let doc = format!("__test_stupd_{}", std::process::id());
        let stem = T.save(&doc, r#"{"v":1}"#, None).unwrap();
        T.set_fav(&doc, &stem, true).unwrap();
        T.update(&doc, &stem, r#"{"v":2}"#, Some(b"png")).unwrap();
        assert_eq!(T.load(&doc, &stem).as_deref(), Some(r#"{"v":2}"#), "内容被覆盖");
        let list = T.list(&doc);
        assert_eq!(list.len(), 1, "不新建条目");
        assert!(list[0].fav, "收藏保留");
        assert_eq!(std::fs::read(T.dir(&doc).join(format!("{stem}.png"))).unwrap(), b"png");
        let _ = T.delete(&doc, &[stem]);
        let _ = std::fs::remove_dir_all(T.dir(&doc));
    }

    #[test]
    fn save_list_fav_rename_delete_flow() {
        let doc = format!("__test_st_{}", std::process::id());
        let a = T.save(&doc, r#"{"a":1}"#, None).unwrap();
        let b = T.save(&doc, r#"{"b":1}"#, None).unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("TS001", "TS002"), "按前缀递增命名");
        T.set_fav(&doc, &b, true).unwrap();
        let list = T.list(&doc);
        assert_eq!(list[0].name, b, "收藏置顶");
        T.rename(&doc, &a, "renamed").unwrap();
        assert_eq!(T.load(&doc, "renamed").as_deref(), Some(r#"{"a":1}"#));
        assert!(T.rename(&doc, "renamed", &b).is_err(), "重名拒绝");
        assert_eq!(T.delete(&doc, &["renamed".into(), b.clone()]).unwrap(), 2);
        assert!(T.list(&doc).is_empty());
        let _ = std::fs::remove_dir_all(T.dir(&doc));
    }
}
