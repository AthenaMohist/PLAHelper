//! D5 / R3(a): 内置大模型 —— 用户填自己的 Anthropic API key, Helper 直接调 Claude Messages API(Rust 没有官方 SDK → 原始 HTTPS)。
//!
//! - 配置存 `<数据目录>/llm.json`: 模型 / 接口地址 / API key(Windows DPAPI 加密, 只有本机本用户能解开; 非 Windows 不存 key)。
//! - `messages()` 阻塞调用, 放后台线程; 返回响应 JSON(调用方自己看 stop_reason / content)。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use pla_bridge::i18n::tr;

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
pub const DEFAULT_BASE: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2026-10-21";
/// 被安全分类器拒绝时服务端自动换模型重跑(Opus 5.5 默认带上)。
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// 空 = DEFAULT_MODEL。
    #[serde(default)]
    pub model: String,
    /// 空 = DEFAULT_BASE(代理 / 网关时改)。
    #[serde(default)]
    pub base_url: String,
    /// effort: low / medium / high; 空 = medium。
    #[serde(default)]
    pub effort: String,
    /// HTTP 代理(如 http://127.0.0.1:7897); 空 = 环境变量 HTTPS_PROXY / HTTP_PROXY / ALL_PROXY, 再没有就用 Windows 系统代理; "none" = 直连。
    #[serde(default)]
    pub proxy: String,
    /// DPAPI 加密后的 key(十六进制)。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key_enc: String,
}

impl Config {
    pub fn path() -> PathBuf {
        crate::prefs::data_dir().join("llm.json")
    }
    pub fn load() -> Config {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
    }
    pub fn save(&self) -> Result<(), String> {
        let p = Self::path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(p, serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
    pub fn model(&self) -> &str {
        if self.model.trim().is_empty() { DEFAULT_MODEL } else { self.model.trim() }
    }
    pub fn base(&self) -> &str {
        let b = self.base_url.trim().trim_end_matches('/');
        if b.is_empty() { DEFAULT_BASE } else { b }
    }
    pub fn effort(&self) -> &str {
        match self.effort.trim() {
            "low" | "high" => self.effort.trim(),
            _ => "medium",
        }
    }
    /// 实际用的代理(见 `proxy` 字段)。实测 2026-11-17: 本机直连官方接口回 403, 走 Clash(HTTPS_PROXY)正常。
    pub fn proxy_url(&self) -> Option<String> {
        let p = self.proxy.trim();
        if p.eq_ignore_ascii_case("none") {
            return None;
        }
        let pick = if !p.is_empty() {
            Some(p.to_string())
        } else {
            ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy", "HTTP_PROXY", "http_proxy"]
                .iter()
                .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
                .or_else(system_proxy)
        }?;
        Some(if pick.contains("://") { pick } else { format!("http://{pick}") })
    }
    pub fn has_key(&self) -> bool {
        !self.key_enc.is_empty()
    }
    pub fn set_key(&mut self, key: &str) -> Result<(), String> {
        let key = key.trim();
        if key.is_empty() {
            self.key_enc.clear();
            return Ok(());
        }
        self.key_enc = hex(&protect(key.as_bytes())?);
        Ok(())
    }
    pub fn key(&self) -> Result<String, String> {
        if self.key_enc.is_empty() {
            return Err(tr("还没填 API key(设置 → 大模型)", "No API key yet (Settings → LLM)").into());
        }
        let raw = unhex(&self.key_enc).ok_or("bad key data")?;
        String::from_utf8(unprotect(&raw)?).map_err(|e| e.to_string())
    }
}

/// Windows 系统代理(Internet 设置里开着的那个; "http=a:1;https=b:2" 取 https 那段)。
#[cfg(windows)]
fn system_proxy() -> Option<String> {
    use std::os::windows::process::CommandExt;
    let q = |name: &str| -> Option<String> {
        let out = std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings", "/v", name])
            .creation_flags(0x0800_0000)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        text.lines().find(|l| l.trim_start().starts_with(name)).and_then(|l| l.split_whitespace().nth(2).map(String::from))
    };
    if q("ProxyEnable").as_deref() != Some("0x1") {
        return None;
    }
    let s = q("ProxyServer")?;
    if s.contains('=') {
        s.split(';').find_map(|p| p.strip_prefix("https=").or_else(|| p.strip_prefix("http="))).map(String::from)
    } else {
        Some(s)
    }
}

#[cfg(not(windows))]
fn system_proxy() -> Option<String> {
    None
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect()
}

#[cfg(windows)]
fn dpapi(data: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB};
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        if encrypt {
            CryptProtectData(&input, windows::core::w!("PLAhelper LLM key"), None, None, None, 0, &mut out)
        } else {
            CryptUnprotectData(&input, None, None, None, None, 0, &mut out)
        }
        .map_err(|e| e.to_string())?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(out.pbData as _));
        Ok(v)
    }
}

#[cfg(windows)]
fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
    dpapi(data, true)
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    dpapi(data, false)
}

#[cfg(not(windows))]
fn protect(_: &[u8]) -> Result<Vec<u8>, String> {
    Err("API key storage needs Windows DPAPI".into())
}

#[cfg(not(windows))]
fn unprotect(_: &[u8]) -> Result<Vec<u8>, String> {
    Err("API key storage needs Windows DPAPI".into())
}

/// 一次 Messages API 调用(非流式, 阻塞)。`body` 里不用填 model / max_tokens / output_config, 这里按配置补上。
pub fn messages(cfg: &Config, mut body: Value) -> Result<Value, String> {
    let key = cfg.key()?;
    body["model"] = json!(cfg.model());
    if body.get("max_tokens").is_none() {
        body["max_tokens"] = json!(16000);
    }
    body["output_config"]["effort"] = json!(cfg.effort());
    // 被安全分类器拒绝 → 服务端按类别换模型重跑(只对官方接口; 代理 / 网关可能不认这个参数)
    let official = cfg.base() == DEFAULT_BASE;
    if official {
        body["fallbacks"] = json!("default");
    }
    let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
    let mut ab = ureq::AgentBuilder::new().tls_connector(std::sync::Arc::new(tls)).timeout(std::time::Duration::from_secs(600));
    if let Some(p) = cfg.proxy_url() {
        ab = ab.proxy(ureq::Proxy::new(&p).map_err(|e| pla_bridge::t!("代理地址不对 {p}: {e}", "Bad proxy address {p}: {e}"))?);
    }
    let agent = ab.build();
    let mut req = agent
        .post(&format!("{}/v1/messages", cfg.base()))
        .set("content-type", "application/json")
        .set("x-api-key", &key)
        .set("anthropic-version", API_VERSION);
    if official {
        req = req.set("anthropic-beta", FALLBACK_BETA);
    }
    match req.send_json(body) {
        Ok(r) => r.into_json::<Value>().map_err(|e| e.to_string()),
        Err(ureq::Error::Status(code, r)) => {
            let v: Value = r.into_json().unwrap_or_default();
            let msg = v["error"]["message"].as_str().unwrap_or("").to_string();
            Err(match code {
                401 => tr("API key 无效(401)", "Invalid API key (401)").to_string(),
                // 实测(2026-11-17): 本机直连官方接口回 403 "Request not allowed"(按地区拒绝), 走 Clash 代理正常
                403 => pla_bridge::t!(
                    "接口拒绝了请求(403: {msg})。多半是当前网络 / 地区不能直连: 在 设置 → LLM 填「代理」(如 http://127.0.0.1:7897)或换「接口」为网关",
                    "The endpoint refused the request (403: {msg}). This network / region probably can't connect directly: set a Proxy under Settings → LLM (e.g. http://127.0.0.1:7897) or a gateway Endpoint"
                ),
                429 => pla_bridge::t!("请求太频繁 / 额度不够(429): {msg}", "Rate limited / out of credit (429): {msg}"),
                529 | 500..=599 => pla_bridge::t!("服务暂时不可用({code}), 稍后再试", "Service unavailable ({code}), try again later"),
                _ => format!("HTTP {code}: {msg}"),
            })
        }
        Err(e) => Err(pla_bridge::t!("连不上 {}: {e}", "Cannot reach {}: {e}", cfg.base())),
    }
}

/// 响应里全部 text 块拼起来。
pub fn text_of(resp: &Value) -> String {
    resp["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手动(要联网): 假 key 打真接口 → 走通代理 / TLS / 请求头, 服务器认出 key 无效 = 401(不花钱)。
    #[test]
    #[ignore]
    fn live_bad_key_is_401() {
        let mut c = Config::default();
        c.set_key("sk-ant-invalid-test").unwrap();
        println!("proxy: {:?}", c.proxy_url());
        let e = messages(&c, json!({"max_tokens": 16, "messages": [{"role": "user", "content": "hi"}]})).unwrap_err();
        println!("{e}");
        assert!(e.contains("401"), "{e}");
    }

    #[test]
    fn config_defaults_and_key_roundtrip() {
        let mut c = Config::default();
        assert_eq!((c.model(), c.base(), c.effort()), (DEFAULT_MODEL, DEFAULT_BASE, "medium"));
        c.base_url = "https://proxy.example/ ".into();
        assert_eq!(c.base(), "https://proxy.example");
        assert!(c.key().is_err());
        #[cfg(windows)]
        {
            c.set_key("sk-test-123").unwrap();
            assert!(c.has_key() && !c.key_enc.contains("sk-test"));
            assert_eq!(c.key().unwrap(), "sk-test-123");
            c.set_key("").unwrap();
            assert!(!c.has_key());
        }
        assert_eq!(unhex(&hex(&[0, 15, 255])), Some(vec![0, 15, 255]));
        let r = json!({"content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": "a"}, {"type": "text", "text": "b"}]});
        assert_eq!(text_of(&r), "a\nb");
    }
}
