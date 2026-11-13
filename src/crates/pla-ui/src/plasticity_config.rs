//! Q1: Plasticity 用户配置读写 — 方向键步进旋转。
//!
//! 文件: `~/.plasticity/config/v2/{keymap,settings}.json`(JSON5, Plasticity 自己写的格式:
//! 4 空格缩进、单引号字符串、不加引号的标识符键、尾逗号、LF 换行)。
//!
//! 只做**最小文本改动**, 不整体重新序列化 — 用户文件里的其他内容、顺序、格式原样保留:
//!   - settings: 改/插 `OrbitControls.rotateDiscreteIncrement`(**单位弧度**, 实测
//!     用户文件 0.2617993877991494 = 15°);
//!   - keymap: 在 `ORBIT_SELECTOR` 块里加/删 4 条方向键绑定, 只删我们自己加的那几行。
//! 每个文件首次改写前备份一次为 `<file>.pla-bak`。纯文本变换函数全部单测覆盖。

use std::path::{Path, PathBuf};

use pla_bridge::i18n::tr;
use pla_bridge::t;

/// 方向键绑定所在的选择器 — 必须是**真实存在的 DOM 元素**。
///
/// R14 实测(CDP): `orbit:rotate:*` 的监听器挂在 `<plasticity-viewport>` 元素上
/// (从该元素派发 → 转 rotateDiscreteIncrement=15°; 从 body 派发 → 不动)。
/// 用户 keymap 里鼠标手势所在的 `orbit-controls` **不是 DOM 元素**(页面里 0 个),
/// 是 Plasticity 轨道控制器自己按名字读的虚拟选择器 → 键盘绑定写在那里永远不会匹配
/// (R14 首版的失败原因)。
/// 单一来源: `pla_bridge::shim::VIEWPORT_SELECTOR`(垫片注入配置用的也是它)。
pub const ORBIT_SELECTOR: &str = pla_bridge::shim::VIEWPORT_SELECTOR;

/// 方向键 → 步进旋转命令。
///
/// R14 用户要求: 左右、上下与 Plasticity 原生命令方向对调(按键方向 = 模型转动方向);
/// Alt+←/→ = 绕屏幕法线(视线)轴旋转。Plasticity 自带的 `viewport:roll:*` 固定 90°,
/// 不读步进角 → 用垫片在视口元素上注册的滚转命令(命令名单一来源 = `pla_bridge::shim`),
/// 未连 CDP 时 Alt+←/→ 无效果。
pub const ARROW_BINDINGS: [(&str, &str); 6] = [
    ("left", "orbit:rotate:right"),
    ("right", "orbit:rotate:left"),
    ("up", "orbit:rotate:down"),
    ("down", "orbit:rotate:up"),
    ("alt-left", pla_bridge::shim::ROLL_LEFT_COMMAND),
    ("alt-right", pla_bridge::shim::ROLL_RIGHT_COMMAND),
];

/// 旧版(R14 首版)写过的绑定 — 停用/迁移时一并清掉。
const LEGACY_BINDINGS: [(&str, &str); 4] = [
    ("left", "orbit:rotate:left"),
    ("right", "orbit:rotate:right"),
    ("up", "orbit:rotate:up"),
    ("down", "orbit:rotate:down"),
];

/// JSON5 键: 合法标识符原样写, 否则加单引号(`'alt-left'`)。
fn json5_key(k: &str) -> String {
    let ident = k
        .chars()
        .enumerate()
        .all(|(i, c)| c == '_' || c == '$' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()));
    if ident {
        k.to_string()
    } else {
        format!("'{k}'")
    }
}

/// 是否是我们(现行或旧版)写的绑定行。
fn is_our_binding(k: &str, v: &str) -> bool {
    ARROW_BINDINGS
        .iter()
        .chain(LEGACY_BINDINGS.iter())
        .any(|(ak, ac)| *ak == k && *ac == v)
}

const STEP_KEY: &str = "rotateDiscreteIncrement";
const ORBIT_CATEGORY: &str = "OrbitControls";

/// 步进角允许范围(度)。
pub const STEP_DEG_MIN: f64 = 0.1;
pub const STEP_DEG_MAX: f64 = 180.0;

// ---------------------------------------------------------------- 文件层

/// `~/.plasticity/config/v2`。
pub fn config_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".plasticity").join("config").join("v2"))
}

fn settings_path() -> Result<PathBuf, String> {
    config_dir()
        .map(|d| d.join("settings.json"))
        .ok_or_else(|| tr("找不到用户目录(USERPROFILE)", "User profile directory (USERPROFILE) not found").into())
}

fn keymap_path() -> Result<PathBuf, String> {
    config_dir()
        .map(|d| d.join("keymap.json"))
        .ok_or_else(|| tr("找不到用户目录(USERPROFILE)", "User profile directory (USERPROFILE) not found").into())
}

/// 读文件; 不存在视为空对象(Plasticity 未写过该文件时)。
fn read_text(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok("{\n}\n".into()),
        Err(e) => Err(t!("读取 {} 失败: {e}", "Failed to read {}: {e}", path.display())),
    }
}

/// 首次改写前备份(`.pla-bak` 已存在则不覆盖, 保留最早的原始版本), 再整体写入。
fn write_text(path: &Path, text: &str) -> Result<(), String> {
    if path.exists() {
        let bak = path.with_extension("json.pla-bak");
        if !bak.exists() {
            std::fs::copy(path, &bak)
                .map_err(|e| t!("备份 {} 失败: {e}", "Failed to back up {}: {e}", path.display()))?;
        }
    } else if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| t!("创建 {} 失败: {e}", "Failed to create {}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| t!("写入 {} 失败: {e}", "Failed to write {}: {e}", path.display()))
}

/// 当前步进角(度)。文件里没有该项时返回 None(Plasticity 用其内置默认值)。
pub fn read_step_deg() -> Result<Option<f64>, String> {
    let text = read_text(&settings_path()?)?;
    Ok(get_step_rad(&text).map(f64::to_degrees))
}

/// 写入步进角(度), 范围 [STEP_DEG_MIN, STEP_DEG_MAX]。
pub fn write_step_deg(deg: f64) -> Result<(), String> {
    if !deg.is_finite() || !(STEP_DEG_MIN..=STEP_DEG_MAX).contains(&deg) {
        return Err(t!("角度需在 {STEP_DEG_MIN}°–{STEP_DEG_MAX}° 之间", "Angle must be between {STEP_DEG_MIN}° and {STEP_DEG_MAX}°"));
    }
    let path = settings_path()?;
    let text = read_text(&path)?;
    let new = set_step_rad(&text, deg.to_radians())?;
    if new != text {
        write_text(&path, &new)?;
    }
    Ok(())
}

/// 方向键绑定是否已全部就位。
pub fn arrow_bindings_installed() -> Result<bool, String> {
    Ok(bindings_present(&read_text(&keymap_path()?)?))
}

/// 启用/停用方向键步进旋转绑定(停用只删除我们加的那几行)。
pub fn set_arrow_bindings(enable: bool) -> Result<(), String> {
    let path = keymap_path()?;
    let text = read_text(&path)?;
    let new = if enable { add_bindings(&text)? } else { remove_bindings(&text) };
    if new != text {
        write_text(&path, &new)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- 实时生效(CDP)
//
// R14 实测(26.2.3):
//   - keymap.json 被 Plasticity 监听, 改文件后会自动重载(有延迟);
//   - settings.json **不会**重载; 且步进角被各视口的轨道控制器缓存在
//     `viewport.orbitControls.rotateDiscreteIncrement`(改 `ed.settings` 或调
//     `onSettingsChanged` 都无效, 只有改这个字段立即生效: 30/45/15° 实测)。
//   - Plasticity 改任何偏好设置时会用内存值整体重写 settings.json →
//     有 CDP 时把内存里的 `ed.settings` 一并改掉, 防止被旧值覆盖。

/// 实时生效(步进角): 垫片接口 `cam.setOrbitStep` + 参数(弧度)。
/// 垫片同时改内存设置与各视口轨道控制器的缓存值; UI 经 `CdpCmd::Call` 发送。
pub fn live_step_call(deg: f64) -> (&'static str, serde_json::Value) {
    (pla_bridge::shim::api::CAM_SET_ORBIT_STEP, serde_json::json!({ "rad": deg.to_radians() }))
}

/// 实时生效(方向键绑定): 垫片接口 `keymap.setBindings` + 参数。
/// 先删视口选择器下我们的(现行 / 旧版)绑定, enable 时再加现行的; 绑定表与写 keymap.json 共用同一份常量。
pub fn live_bindings_call(enable: bool) -> (&'static str, serde_json::Value) {
    let obj = |pairs: &[(&str, &str)]| -> serde_json::Map<String, serde_json::Value> {
        pairs.iter().map(|(k, c)| (k.to_string(), serde_json::Value::String(c.to_string()))).collect()
    };
    (
        pla_bridge::shim::api::KEYMAP_SET_BINDINGS,
        serde_json::json!({
            "enable": enable,
            "bindings": obj(&ARROW_BINDINGS),
            "legacy": obj(&LEGACY_BINDINGS),
            "source": "pla-helper",
        }),
    )
}

// ---------------------------------------------------------------- 纯文本层

fn newline_of(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// 第一个 `{` 的位置(文件顶层对象起点)。
fn top_open_brace(text: &str) -> Option<usize> {
    text.find('{')
}

/// 从 `open`(指向 `{`)起找匹配的 `}`, 跳过字符串与注释。
fn matching_brace(text: &str, open: usize) -> Option<usize> {
    let b = text.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' => {
                let q = b[i];
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'/' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// 找对象键 `key`(裸/单引号/双引号)后紧跟 `:` 的位置, 返回冒号之后的下标。
fn find_key_value(text: &str, key: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = text[from..].find(key) {
        let start = from + rel;
        let end = start + key.len();
        from = end;
        let before = text[..start].chars().next_back();
        let is_ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$' || c == '-';
        if before.map_or(false, is_ident) {
            continue;
        }
        let mut rest = &text[end..];
        let mut consumed = 0;
        if let Some(c) = rest.chars().next() {
            if c == '\'' || c == '"' {
                if before != Some(c) {
                    continue;
                }
                rest = &rest[1..];
                consumed += 1;
            } else if is_ident(c) {
                continue;
            }
        }
        let trimmed = rest.trim_start();
        consumed += rest.len() - trimmed.len();
        if trimmed.starts_with(':') {
            return Some(end + consumed + 1);
        }
    }
    None
}

/// 行首缩进。
fn line_indent(text: &str, pos: usize) -> &str {
    let line_start = text[..pos].rfind('\n').map_or(0, |i| i + 1);
    let line = &text[line_start..];
    let n = line.len() - line.trim_start_matches([' ', '\t']).len();
    &line[..n]
}

/// 读 `rotateDiscreteIncrement`(弧度)。
pub fn get_step_rad(text: &str) -> Option<f64> {
    let v = find_key_value(text, STEP_KEY)?;
    let rest = text[v..].trim_start();
    let num: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
        .collect();
    num.parse().ok()
}

/// 写 `rotateDiscreteIncrement`(弧度): 有则改数值, 无则插到 `OrbitControls` 块首,
/// 连块都没有则在顶层对象首新建块。
pub fn set_step_rad(text: &str, rad: f64) -> Result<String, String> {
    let nl = newline_of(text);
    if let Some(v) = find_key_value(text, STEP_KEY) {
        let ws = text[v..].len() - text[v..].trim_start().len();
        let num_start = v + ws;
        let num_len = text[num_start..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
            .count();
        let mut out = String::with_capacity(text.len() + 8);
        out.push_str(&text[..num_start]);
        out.push_str(&rad.to_string());
        out.push_str(&text[num_start + num_len..]);
        return Ok(out);
    }
    if let Some(v) = find_key_value(text, ORBIT_CATEGORY) {
        let open = v + text[v..].find('{').ok_or(tr("OrbitControls 不是对象", "OrbitControls is not an object"))?;
        let indent = format!("{}    ", line_indent(text, open));
        let mut out = String::new();
        out.push_str(&text[..=open]);
        out.push_str(&format!("{nl}{indent}{STEP_KEY}: {rad},"));
        out.push_str(&text[open + 1..]);
        return Ok(out);
    }
    let Some(open) = top_open_brace(text) else {
        return Ok(format!(
            "{{{nl}    {ORBIT_CATEGORY}: {{{nl}        {STEP_KEY}: {rad},{nl}    }},{nl}}}{nl}"
        ));
    };
    let mut out = String::new();
    out.push_str(&text[..=open]);
    out.push_str(&format!(
        "{nl}    {ORBIT_CATEGORY}: {{{nl}        {STEP_KEY}: {rad},{nl}    }},"
    ));
    out.push_str(&text[open + 1..]);
    Ok(out)
}

/// 解析一行 `key: 'value',` → (key, value), 去引号。
fn parse_binding_line(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    let (k, v) = t.split_once(':')?;
    let unq = |s: &str| s.trim().trim_matches(|c| c == '\'' || c == '"').to_string();
    let v = v.trim().trim_end_matches(',');
    Some((unq(k), unq(v)))
}

/// `ORBIT_SELECTOR` 块的 (`{` 位置, `}` 位置)。
fn selector_block(text: &str) -> Option<(usize, usize)> {
    let v = find_key_value(text, ORBIT_SELECTOR)?;
    let open = v + text[v..].find('{')?;
    let close = matching_brace(text, open)?;
    Some((open, close))
}

/// 方向键绑定是否已全部在 `ORBIT_SELECTOR` 块里。
pub fn bindings_present(text: &str) -> bool {
    let Some((open, close)) = selector_block(text) else {
        return false;
    };
    let lines: Vec<(String, String)> = text[open + 1..close]
        .lines()
        .filter_map(parse_binding_line)
        .collect();
    ARROW_BINDINGS
        .iter()
        .all(|(k, c)| lines.iter().any(|(lk, lv)| lk == k && lv == c))
}

/// 加绑定: 先删掉选择器块里同名方向键的旧绑定(避免重复键), 再插到块首。
pub fn add_bindings(text: &str) -> Result<String, String> {
    let nl = newline_of(text);
    let base = remove_bindings(text);
    if let Some((open, close)) = selector_block(&base) {
        let indent = format!("{}    ", line_indent(&base, open));
        // 块内其他方向键绑定(用户自己的)也会和我们冲突 → 去掉。
        let inner: Vec<&str> = base[open + 1..close]
            .split_inclusive('\n')
            .filter(|l| {
                parse_binding_line(l)
                    .map_or(true, |(k, _)| !ARROW_BINDINGS.iter().any(|(ak, _)| *ak == k))
            })
            .collect();
        let mut ins = String::new();
        for (k, c) in ARROW_BINDINGS {
            ins.push_str(&format!("{nl}{indent}{}: '{c}',", json5_key(k)));
        }
        let mut out = String::new();
        out.push_str(&base[..=open]);
        out.push_str(&ins);
        out.push_str(&inner.concat());
        out.push_str(&base[close..]);
        return Ok(out);
    }
    let mut block = format!("{nl}    '{ORBIT_SELECTOR}': {{");
    for (k, c) in ARROW_BINDINGS {
        block.push_str(&format!("{nl}        {}: '{c}',", json5_key(k)));
    }
    block.push_str(&format!("{nl}    }},"));
    match top_open_brace(&base) {
        Some(open) => {
            let mut out = String::new();
            out.push_str(&base[..=open]);
            out.push_str(&block);
            out.push_str(&base[open + 1..]);
            Ok(out)
        }
        None => Ok(format!("{{{block}{nl}}}{nl}")),
    }
}

/// 删绑定: 只删「方向键 → 我们的步进命令」这几行, 其他一律不动;
/// 删完后 `ORBIT_SELECTOR` 块若已空(多半是我们当初新建的), 连块一起删掉。
pub fn remove_bindings(text: &str) -> String {
    let out: String = text
        .split_inclusive('\n')
        .filter(|l| !parse_binding_line(l).map_or(false, |(k, v)| is_our_binding(&k, &v)))
        .collect();
    let Some((open, close)) = selector_block(&out) else {
        return out;
    };
    if !out[open + 1..close].trim().is_empty() {
        return out;
    }
    // 块首行(选择器键所在行)起, 到 `}` 后的逗号与换行止。
    let start = out[..open].rfind('\n').map_or(0, |i| i + 1);
    let mut end = close + 1;
    let tail = &out[end..];
    let skip = tail.len() - tail.trim_start_matches([' ', '\t']).len();
    end += skip;
    if out[end..].starts_with(',') {
        end += 1;
    }
    if out[end..].starts_with("\r\n") {
        end += 2;
    } else if out[end..].starts_with('\n') {
        end += 1;
    }
    format!("{}{}", &out[..start], &out[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用户真实 settings.json(2026-11-13 实拍, 节选关键段)。
    const SETTINGS: &str = "{\n    Unit: {\n        length: 'millimeter',\n    },\n    OrbitControls: {\n        wheelSpeed: 0.1,\n        rotateSpeed: 1,\n        rotateDiscreteIncrement: 0.2617993877991494,\n        panDiscreteIncrement: 10,\n    },\n    Server: {\n        enabled: true,\n        host: 'localhost',\n        port: 8980,\n    },\n}";

    /// 用户真实 keymap.json(2026-11-13 实拍)。
    const KEYMAP: &str = "{\n    'orbit-controls': {\n        mouse1: 'orbit:rotate',\n        mouse2: 'orbit:pan',\n        'ctrl-mouse1': 'orbit:dolly',\n        'alt-mouse1': 'orbit:align',\n        alt: 'orbit:snap',\n    },\n    'body:not([gizmo])': {\n        'ctrl-alt-x': 'command:freestyle-mirror',\n        'alt-c': 'command:curve',\n    },\n}";

    #[test]
    fn reads_step_in_radians() {
        let r = get_step_rad(SETTINGS).unwrap();
        assert!((r.to_degrees() - 15.0).abs() < 1e-9);
    }

    #[test]
    fn rewrites_only_the_number() {
        let out = set_step_rad(SETTINGS, 30f64.to_radians()).unwrap();
        assert!((get_step_rad(&out).unwrap().to_degrees() - 30.0).abs() < 1e-9);
        assert_eq!(out.lines().count(), SETTINGS.lines().count());
        // panDiscreteIncrement 等邻居不受影响
        assert!(out.contains("panDiscreteIncrement: 10,"));
        assert!(out.contains("rotateSpeed: 1,"));
    }

    #[test]
    fn inserts_into_existing_category_or_creates_it() {
        let no_key = SETTINGS.replace("        rotateDiscreteIncrement: 0.2617993877991494,\n", "");
        let out = set_step_rad(&no_key, 0.5).unwrap();
        assert_eq!(get_step_rad(&out), Some(0.5));
        assert!(out.contains("    OrbitControls: {\n        rotateDiscreteIncrement: 0.5,"));

        let out = set_step_rad("{\n    Grid: {\n        size: 120,\n    },\n}\n", 0.5).unwrap();
        assert_eq!(get_step_rad(&out), Some(0.5));
        assert!(out.contains("Grid: {"));

        let out = set_step_rad("", 0.5).unwrap();
        assert_eq!(get_step_rad(&out), Some(0.5));
    }

    #[test]
    fn key_match_is_exact() {
        // 键名前后有标识符字符的不算命中
        let t = "{ xrotateDiscreteIncrement: 1, rotateDiscreteIncrementX: 2 }";
        assert_eq!(get_step_rad(t), None);
        let t = "{ 'rotateDiscreteIncrement': 0.25 }";
        assert_eq!(get_step_rad(t), Some(0.25));
    }

    #[test]
    fn add_then_remove_is_identity_and_idempotent() {
        assert!(!bindings_present(KEYMAP));
        let added = add_bindings(KEYMAP).unwrap();
        assert!(bindings_present(&added));
        assert!(added.contains("    'plasticity-viewport': {\n        left: 'orbit:rotate:right',"));
        assert!(added.contains("        down: 'orbit:rotate:up',"));
        // 带 '-' 的键必须加引号, 否则 JSON5 非法
        assert!(added.contains("        'alt-left': 'pla:roll:left',"));
        assert!(added.contains("        'alt-right': 'pla:roll:right',"));
        // 原有绑定全部保留
        assert!(added.contains("mouse1: 'orbit:rotate',"));
        assert!(added.contains("'alt-c': 'command:curve',"));
        // 幂等
        assert_eq!(add_bindings(&added).unwrap(), added);
        // 删除恢复原文
        assert_eq!(remove_bindings(&added), KEYMAP);
    }

    #[test]
    fn replaces_conflicting_arrow_binding_in_block() {
        let t = "{\n    'plasticity-viewport': {\n        left: 'something:else',\n        q: 'keep:me',\n    },\n}\n";
        let added = add_bindings(t).unwrap();
        assert!(bindings_present(&added));
        assert!(!added.contains("something:else"));
        assert!(added.contains("q: 'keep:me',"));
    }

    /// R14 首版把绑定写进了 `'orbit-controls'`(不是 DOM 元素, 永远不生效) → 迁移时删掉旧行。
    #[test]
    fn migrates_bindings_out_of_orbit_controls() {
        let old = KEYMAP.replace(
            "    'orbit-controls': {\n",
            "    'orbit-controls': {\n        left: 'orbit:rotate:left',\n        right: 'orbit:rotate:right',\n        up: 'orbit:rotate:up',\n        down: 'orbit:rotate:down',\n",
        );
        assert!(!bindings_present(&old));
        let added = add_bindings(&old).unwrap();
        assert!(bindings_present(&added));
        assert_eq!(added.matches("orbit:rotate:left").count(), 1);
        // 鼠标手势原样保留在 orbit-controls 块
        assert!(added.contains("    'orbit-controls': {\n        mouse1: 'orbit:rotate',"));
        assert_eq!(remove_bindings(&added), KEYMAP);
    }

    /// R14 第二版(plasticity-viewport 下的原生方向) → 第三版(对调 + Alt 滚转)。
    #[test]
    fn migrates_from_first_plasticity_viewport_mapping() {
        let v2 = "{\n    'plasticity-viewport': {\n        left: 'orbit:rotate:left',\n        right: 'orbit:rotate:right',\n        up: 'orbit:rotate:up',\n        down: 'orbit:rotate:down',\n    },\n    'body:not([gizmo])': {\n        b: 'command:corner-box',\n    },\n}\n";
        let added = add_bindings(v2).unwrap();
        assert!(bindings_present(&added));
        assert!(!added.contains("left: 'orbit:rotate:left'"));
        assert_eq!(added.matches("'plasticity-viewport'").count(), 1);
        // 全部停用 → 连空块一起删掉
        let removed = remove_bindings(&added);
        assert!(!removed.contains("plasticity-viewport"));
        assert!(removed.contains("b: 'command:corner-box',"));
    }

    #[test]
    fn creates_selector_block_when_missing() {
        let t = "{\n    'body:not([gizmo])': {\n        b: 'command:corner-box',\n    },\n}\n";
        let added = add_bindings(t).unwrap();
        assert!(bindings_present(&added));
        assert!(added.contains("b: 'command:corner-box',"));
        let added = add_bindings("").unwrap();
        assert!(bindings_present(&added));
    }

    #[test]
    fn live_calls_carry_the_right_values() {
        let (name, args) = live_step_call(30.0);
        assert_eq!(name, "cam.setOrbitStep");
        assert!((args["rad"].as_f64().unwrap() - 30f64.to_radians()).abs() < 1e-12);
        let (name, on) = live_bindings_call(true);
        assert_eq!(name, "keymap.setBindings");
        assert_eq!(on["enable"], true);
        assert_eq!(on["bindings"]["left"], "orbit:rotate:right");
        assert_eq!(on["bindings"]["alt-left"], "pla:roll:left");
        // 旧映射也会被实时清掉
        assert_eq!(on["legacy"]["left"], "orbit:rotate:left");
        assert_eq!(live_bindings_call(false).1["enable"], false);
    }

    #[test]
    fn keeps_crlf_line_endings() {
        let t = KEYMAP.replace('\n', "\r\n");
        let added = add_bindings(&t).unwrap();
        assert!(!added.replace("\r\n", "").contains('\n'));
        assert_eq!(remove_bindings(&added), t);
    }
}
