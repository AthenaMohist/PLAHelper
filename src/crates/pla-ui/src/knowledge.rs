//! K1 专业知识库: 腾讯 ima OpenAPI(只读)。
//!
//! 接口(官方 ima-skill 1.1.10 的 knowledge-base/references/api.md; 实测 2026-11-19):
//! `POST https://ima.qq.com/openapi/wiki/v1/<op>`, 头 `ima-openapi-clientid` / `ima-openapi-apikey`, JSON body,
//! 回包 `{code, msg, data}`(code 0 = 成功)。
//! - search_knowledge_base {query, cursor, limit≤20} → info_list[{kb_id, kb_name, content_count, base_type, role_type}]
//!   (实测字段名是 kb_id / kb_name, 文档写的 id / name 不对)
//! - search_knowledge {query, cursor, knowledge_base_id} → info_list[{media_id, title, media_type, highlight_content}]
//!   实测: **按标题关键词匹配**("联轴器" 有 44 条, "联轴器 孔径" 0 条), highlight_content 为空 → 多词查询拆开逐词搜再按命中词数排序。
//! - get_media_info {media_id} → url_info{url, headers} / notebook_ext_info{notebook_id}。
//!   **订阅知识库的文件拿不到**(code 220030 "没有权限通过skill获取订阅知识库的文件"), 只有自己的知识库可以。
//!
//! 凭证: 环境变量 IMA_OPENAPI_CLIENTID / IMA_OPENAPI_APIKEY(或 IMA_CLIENT_ID / IMA_API_KEY), 否则 ~/.config/ima/{client_id, api_key}
//! (与官方 ima-skill 同一位置)。凭证只发往 ima.qq.com, 不写日志、不进回包。

use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

const BASE: &str = "https://ima.qq.com/openapi/wiki/v1/";
/// 没指定知识库时搜这些(名字含任一片段)。实测 ima 对每个知识库有**每日搜索次数上限**(220021 "资料获取次数已达上限")
/// 和频控(200001), 默认只搜几个核心机械库; 其他的用 kb 点名。可用 ~/.config/ima/plahelper_kbs.txt(一行一个名字片段)覆盖。
pub const DEFAULT_KB_KEYWORDS: &[&str] = &["机械设计手册", "非标机械设计", "机械国家标准", "机械设计库", "标准大全"];
/// 每日上限(不重试)
const DAILY_QUOTA: i64 = 220021;
/// 搜索结果本地缓存时长
const CACHE_SECS: u64 = 24 * 3600;

fn default_keywords() -> Vec<String> {
    home()
        .and_then(|h| std::fs::read_to_string(h.join(".config").join("ima").join("plahelper_kbs.txt")).ok())
        .map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect::<Vec<_>>())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_KB_KEYWORDS.iter().map(|s| s.to_string()).collect())
}

/// 单个 (知识库, 词) 搜索结果的本地缓存(省每日额度)
fn cache_file(kb_id: &str, term: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (kb_id, term).hash(&mut h);
    cache_dir().join("search").join(format!("{:016x}.json", h.finish()))
}

fn cached_search(kb_id: &str, term: &str) -> Option<Vec<Value>> {
    let p = cache_file(kb_id, term);
    let age = std::fs::metadata(&p).ok()?.modified().ok()?.elapsed().ok()?.as_secs();
    if age > CACHE_SECS {
        return None;
    }
    let v: Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    (v["kb"].as_str() == Some(kb_id) && v["term"].as_str() == Some(term)).then(|| v["hits"].as_array().cloned().unwrap_or_default())
}

fn store_search(kb_id: &str, term: &str, hits: &[Value]) {
    let p = cache_file(kb_id, term);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, json!({"kb": kb_id, "term": term, "hits": hits}).to_string());
}
/// 订阅库拿原文的错误码(实测)
const NO_SUBSCRIBED_FILE: i64 = 220030;
/// 频控(HTTP 403 + code 200001)
const RATE_LIMITED: i64 = 200001;
/// 搜索并发数
const WORKERS: usize = 4;

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

fn read_trim(p: PathBuf) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// (client_id, api_key); 缺了给出怎么配的说明。
pub fn credentials() -> Result<(String, String), String> {
    let env = |a: &str, b: &str| std::env::var(a).ok().or_else(|| std::env::var(b).ok()).filter(|s| !s.trim().is_empty());
    let dir = home().map(|h| h.join(".config").join("ima"));
    let id = env("IMA_OPENAPI_CLIENTID", "IMA_CLIENT_ID").or_else(|| dir.as_ref().and_then(|d| read_trim(d.join("client_id"))));
    let key = env("IMA_OPENAPI_APIKEY", "IMA_API_KEY").or_else(|| dir.as_ref().and_then(|d| read_trim(d.join("api_key"))));
    match (id, key) {
        (Some(i), Some(k)) => Ok((i, k)),
        _ => Err("ima credentials missing: create a Client ID + API Key at https://ima.qq.com/agent-interface and put them in ~/.config/ima/client_id and ~/.config/ima/api_key (or env IMA_OPENAPI_CLIENTID / IMA_OPENAPI_APIKEY)".into()),
    }
}

fn agent() -> ureq::Agent {
    // ureq 用 native-tls 时要显式给 TLS 连接器(同 llm.rs), 否则 "no TLS backend is configured"
    let ab = ureq::AgentBuilder::new().timeout(Duration::from_secs(20));
    match native_tls::TlsConnector::new() {
        Ok(tls) => ab.tls_connector(std::sync::Arc::new(tls)).build(),
        Err(_) => ab.build(),
    }
}

/// 调一个接口 → data(code ≠ 0 → Err((code, msg)))。
pub fn post(op: &str, body: &Value) -> Result<Value, (i64, String)> {
    let (id, key) = credentials().map_err(|e| (-1, e))?;
    let r = agent()
        .post(&format!("{BASE}{op}"))
        .set("ima-openapi-clientid", &id)
        .set("ima-openapi-apikey", &key)
        .set("ima-openapi-ctx", "skill_version=plahelper")
        .set("Content-Type", "application/json")
        .send_string(&body.to_string());
    let text = match r {
        Ok(resp) => resp.into_string().map_err(|e| (-1, format!("ima: read response: {e}")))?,
        Err(ureq::Error::Status(s, resp)) => {
            // 出错时回包也是 {code, msg}(如 403 + 200001 频控)
            let body = resp.into_string().unwrap_or_default();
            let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            return Err(match v["code"].as_i64() {
                Some(c) => (c, v["msg"].as_str().unwrap_or("").to_string()),
                None => (-1, format!("ima HTTP {s}: {}", body.chars().take(200).collect::<String>())),
            });
        }
        Err(e) => return Err((-1, format!("ima: {e}"))),
    };
    let v: Value = serde_json::from_str(&text).map_err(|e| (-1, format!("ima: bad JSON ({e})")))?;
    match v["code"].as_i64() {
        Some(0) => Ok(v["data"].clone()),
        c => Err((c.unwrap_or(-1), v["msg"].as_str().unwrap_or("ima error").to_string())),
    }
}

fn msg((c, m): (i64, String)) -> String {
    if c > 0 { format!("ima error {c}: {m}") } else { m }
}

/// 全部知识库(自建 + 订阅): [{id, name, items, kind, role}]。query 过滤名字(空 = 全部)。
pub fn bases(query: &str) -> Result<Vec<Value>, String> {
    let mut out = vec![];
    let mut cursor = String::new();
    for _ in 0..50 {
        let d = post("search_knowledge_base", &json!({"query": query, "cursor": cursor, "limit": 20})).map_err(msg)?;
        for k in d["info_list"].as_array().into_iter().flatten() {
            out.push(json!({
                "id": k["kb_id"].as_str().or(k["id"].as_str()).unwrap_or(""),
                "name": k["kb_name"].as_str().or(k["name"].as_str()).unwrap_or(""),
                "items": k["content_count"].as_str().and_then(|s| s.parse::<i64>().ok()).or(k["content_count"].as_i64()),
                "kind": k["base_type"], "role": k["role_type"],
            }));
        }
        if d["is_end"].as_bool() != Some(false) {
            break;
        }
        cursor = d["next_cursor"].as_str().unwrap_or("").to_string();
        if cursor.is_empty() {
            break;
        }
    }
    Ok(out)
}

/// 知识库清单(24 小时缓存; 搜索时用, 免得每次翻 6 页)
pub fn bases_cached() -> Result<Vec<Value>, String> {
    let p = cache_dir().join("bases.json");
    let fresh = std::fs::metadata(&p).ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).map(|a| a.as_secs() < CACHE_SECS).unwrap_or(false);
    if fresh {
        if let Some(v) = std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok()).filter(|v| !v.is_empty()) {
            return Ok(v);
        }
    }
    let all = bases("")?;
    let _ = std::fs::create_dir_all(cache_dir());
    let _ = std::fs::write(&p, Value::Array(all.clone()).to_string());
    Ok(all)
}

/// 选知识库: kb = 名字片段 / id 列表; 空 = 名字含默认片段的(DEFAULT_KB_KEYWORDS 或 plahelper_kbs.txt)。
pub fn pick_bases(all: &[Value], kb: &[String]) -> Vec<Value> {
    pick_with(all, kb, &default_keywords())
}

fn pick_with(all: &[Value], kb: &[String], defaults: &[String]) -> Vec<Value> {
    let name = |b: &Value| b["name"].as_str().unwrap_or("").to_string();
    all.iter()
        .filter(|b| {
            let n = name(b);
            if kb.is_empty() {
                defaults.iter().any(|k| n.contains(k.as_str()))
            } else {
                kb.iter().any(|k| b["id"].as_str() == Some(k.as_str()) || n.contains(k.as_str()))
            }
        })
        .cloned()
        .collect()
}

/// 查询拆词(空白 / 中英文逗号); 标题按命中词数排序。
pub fn terms(q: &str) -> Vec<String> {
    q.split(|c: char| c.is_whitespace() || c == ',' || c == '，' || c == '、' || c == ';' || c == '；')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 搜索: 每个知识库 × 每个词并发查, 合并去重; score = 标题里出现的词数。→ {hits:[{kb, kb_id, media_id, title, type, score, own}], searched, errors}
pub fn search(query: &str, kb: &[String], limit: usize) -> Result<Value, String> {
    let ts = terms(query);
    if ts.is_empty() {
        return Err("query is empty".into());
    }
    let all = bases_cached()?;
    let picked = pick_bases(&all, kb);
    if picked.is_empty() {
        return Err(format!("no knowledge base matches {kb:?} (see knowledge_bases)"));
    }
    let jobs: Vec<(Value, String)> = picked.iter().flat_map(|b| ts.iter().map(move |t| (b.clone(), t.clone()))).collect();
    // 并发上限 + 频控重试(实测 40 路并发 → 一部分 200001 "请求频率超限")
    let next = std::sync::atomic::AtomicUsize::new(0);
    let out = std::sync::Mutex::new(Vec::<Result<(Value, Vec<Value>), String>>::new());
    std::thread::scope(|s| {
        for _ in 0..WORKERS.min(jobs.len()) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((b, t)) = jobs.get(i) else { break };
                let id = b["id"].as_str().unwrap_or("");
                if let Some(h) = cached_search(id, t) {
                    out.lock().unwrap().push(Ok((b.clone(), h)));
                    continue;
                }
                let mut r = post("search_knowledge", &json!({"query": t, "cursor": "", "knowledge_base_id": id}));
                for wait in [1500u64, 3000, 6000] {
                    match &r {
                        Err((c, _)) if *c == RATE_LIMITED => {
                            std::thread::sleep(Duration::from_millis(wait));
                            r = post("search_knowledge", &json!({"query": t, "cursor": "", "knowledge_base_id": id}));
                        }
                        _ => break,
                    }
                }
                let item = r
                    .map(|d| {
                        let h = d["info_list"].as_array().cloned().unwrap_or_default();
                        store_search(id, t, &h);
                        (b.clone(), h)
                    })
                    .map_err(|e| {
                        let quota = e.0 == DAILY_QUOTA;
                        format!("{}: {}{}", b["name"].as_str().unwrap_or(""), msg(e), if quota { " (daily quota of this knowledge base used up)" } else { "" })
                    });
                out.lock().unwrap().push(item);
            });
        }
    });
    let results = out.into_inner().unwrap();
    let mut seen = std::collections::HashMap::<String, usize>::new();
    let mut hits: Vec<Value> = vec![];
    let mut errors = vec![];
    for r in results {
        match r {
            Err(e) => errors.push(e),
            Ok((b, list)) => {
                for x in list {
                    let mid = x["media_id"].as_str().unwrap_or("").to_string();
                    if mid.is_empty() || seen.contains_key(&mid) {
                        continue;
                    }
                    let title = x["title"].as_str().unwrap_or("").to_string();
                    let score = ts.iter().filter(|t| title.to_lowercase().contains(&t.to_lowercase())).count();
                    seen.insert(mid.clone(), hits.len());
                    hits.push(json!({
                        "kb": b["name"], "kb_id": b["id"], "media_id": mid, "title": title, "type": media_type_name(x["media_type"].as_i64().unwrap_or(0)),
                        "score": score, "own": b["role"].as_str() == Some("创建者"),
                        "snippet": x["highlight_content"].as_str().filter(|s| !s.is_empty()),
                    }));
                }
            }
        }
    }
    hits.sort_by(|a, b| b["score"].as_u64().cmp(&a["score"].as_u64()));
    let total = hits.len();
    hits.truncate(limit.max(1));
    Ok(json!({
        "terms": ts, "searched": picked.iter().map(|b| b["name"].clone()).collect::<Vec<_>>(), "total": total, "hits": hits, "errors": errors,
        "note": "ima matches titles by keyword (one word per search; multi-word queries are split and ranked by how many words the title contains). Files of subscribed knowledge bases cannot be fetched through the API (only titles); files in the user's own knowledge base can (knowledge_get)."
    }))
}

pub fn media_type_name(t: i64) -> &'static str {
    match t {
        1 => "pdf", 2 => "web", 3 => "word", 4 => "ppt", 5 => "excel", 6 => "wechat-article", 7 => "markdown", 9 => "image",
        11 => "note", 12 => "ai-session", 13 => "txt", 14 => "xmind", 15 => "audio", 16 => "video", 20 => "html", 21 => "epub",
        _ => "other",
    }
}

fn ext_for(t: i64, url: &str) -> String {
    let from_url = url.split(['?', '#']).next().and_then(|p| p.rsplit('/').next()).and_then(|f| f.rsplit_once('.').map(|(_, e)| e.to_lowercase())).filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()));
    from_url.unwrap_or_else(|| match t {
        1 => "pdf", 3 => "docx", 4 => "pptx", 5 => "xlsx", 7 => "md", 9 => "png", 13 => "txt", 2 | 6 | 20 => "html", 21 => "epub",
        _ => "bin",
    }.into())
}

/// 下载缓存目录(%LOCALAPPDATA%\PLAhelper\ima-cache)
pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).or_else(home).unwrap_or_else(std::env::temp_dir);
    base.join("PLAhelper").join("ima-cache")
}

/// 取原文: 下载到本地缓存, 返回路径(PDF / 图片等交给调用方的阅读工具)。订阅库 → 说明拿不到。
pub fn get(media_id: &str) -> Result<Value, String> {
    let mid = media_id.trim();
    if mid.is_empty() || !mid.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("media_id looks wrong (copy it from knowledge_search)".into());
    }
    let d = match post("get_media_info", &json!({"media_id": mid})) {
        Ok(d) => d,
        Err((NO_SUBSCRIBED_FILE, m)) => {
            return Ok(json!({"media_id": mid, "available": false, "reason": m,
                "hint": "This file is in a subscribed knowledge base: ima only lets the API read files of the user's own knowledge base. Ask the user to save it into their own knowledge base in the ima app (or open it there), then retry."}))
        }
        Err(e) => return Err(msg(e)),
    };
    let t = d["media_type"].as_i64().unwrap_or(0);
    if let Some(nb) = d["notebook_ext_info"]["notebook_id"].as_str() {
        let c = post_note(nb)?;
        return Ok(json!({"media_id": mid, "type": "note", "available": true, "text": c}));
    }
    let url = d["url_info"]["url"].as_str().unwrap_or("");
    if url.is_empty() {
        return Ok(json!({"media_id": mid, "type": media_type_name(t), "available": false, "reason": "ima gives no URL for this item (open it in the ima app)"}));
    }
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cache dir: {e}"))?;
    let path = dir.join(format!("{mid}.{}", ext_for(t, url)));
    if !path.exists() {
        let mut req = agent().get(url);
        if let Some(h) = d["url_info"]["headers"].as_object() {
            for (k, v) in h {
                if let Some(v) = v.as_str() {
                    req = req.set(k, v);
                }
            }
        }
        let resp = req.call().map_err(|e| format!("download: {e}"))?;
        let mut buf = vec![];
        resp.into_reader().take(300 * 1024 * 1024).read_to_end(&mut buf).map_err(|e| format!("download: {e}"))?;
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, &buf).map_err(|e| format!("save: {e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("save: {e}"))?;
    }
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(json!({"media_id": mid, "type": media_type_name(t), "available": true, "path": path.to_string_lossy(), "bytes": size, "source_url": url,
              "hint": "Read the local file (PDF pages / image / text) and cite the title + page."}))
}

fn post_note(note_id: &str) -> Result<String, String> {
    // 笔记正文在 notes 模块: POST /openapi/note/v1/get_doc_content {note_id}
    let (id, key) = credentials()?;
    let r = agent()
        .post("https://ima.qq.com/openapi/note/v1/get_doc_content")
        .set("ima-openapi-clientid", &id)
        .set("ima-openapi-apikey", &key)
        .set("Content-Type", "application/json")
        .send_string(&json!({"note_id": note_id}).to_string())
        .map_err(|e| format!("ima note: {e}"))?;
    let v: Value = serde_json::from_str(&r.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if v["code"].as_i64() != Some(0) {
        return Err(format!("ima note error {}: {}", v["code"], v["msg"]));
    }
    Ok(v["data"]["content"].as_str().unwrap_or("").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_split_and_bases_pick() {
        assert_eq!(terms("联轴器 孔径，扭矩、 键"), vec!["联轴器", "孔径", "扭矩", "键"]);
        let all = vec![
            json!({"id": "a", "name": "机械设计手册"}), json!({"id": "b", "name": "期货交易"}), json!({"id": "c", "name": "机械国家标准"}),
        ];
        let defaults: Vec<String> = DEFAULT_KB_KEYWORDS.iter().map(|s| s.to_string()).collect();
        let d: Vec<_> = pick_with(&all, &[], &defaults).iter().map(|b| b["id"].as_str().unwrap().to_string()).collect();
        assert_eq!(d, vec!["a", "c"], "默认 = 核心机械库");
        let x: Vec<_> = pick_bases(&all, &["b".into()]).iter().map(|b| b["id"].as_str().unwrap().to_string()).collect();
        assert_eq!(x, vec!["b"], "按 id");
        assert_eq!(pick_bases(&all, &["手册".into()]).len(), 1, "按名字片段");
        assert_eq!(ext_for(1, "https://x/y/file.PDF?sig=1"), "pdf");
        assert_eq!(ext_for(5, "https://x/y/noext"), "xlsx");
    }
}
