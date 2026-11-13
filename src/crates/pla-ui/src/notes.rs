//! R30: 笔记面板的数据模型 —— 跟着文档走的 Markdown 记录 + 计算。
//!
//! 一行 = 一个块(Block): 正文 / 标题 / 无序 / 有序 / 待办 / 分隔线 / 表格。界面只显示渲染效果
//! (类似 Google Keep), 存盘是普通 Markdown, 任何编辑器都能打开:
//!   * 无序 `- 文本`, 有序 `1. 文本`(存盘时自动重排序号), 待办 `- [ ] 文本` / `- [x] 文本`
//!   * 标题 `# ` / `## ` / `### `, 分隔线 `---`, 缩进 = 每级 2 个空格
//!   * 表格 = 管道表(首行作表头); 单元格里存原始内容(公式 `=SUM(A1:A3)` 原样存, 显示时计算)
//! 存放位置: `<文档目录>/.pla/notes/<doc_key>/notes.md`(定位不到文档时退回 exe 同级 `notes/<doc_key>/`)。

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Text,
    Heading(u8),
    Bullet,
    Number,
    Todo(bool),
    Divider,
    Table(Table),
}

/// R31: 表格 = 单元格原文 + 每列宽 + 行高(逻辑 pt)。
/// 尺寸非默认时存成表格前一行的 HTML 注释 `<!-- pla:table cols=72,90,60 row=22 -->`
/// (其他 Markdown 编辑器看不见, 不影响表格本身)。
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub cells: Vec<Vec<String>>,
    pub col_w: Vec<f32>,
    pub row_h: f32,
}

impl Table {
    pub const DEFAULT_COL_W: f32 = 72.0;
    pub const DEFAULT_ROW_H: f32 = 22.0;
    pub const MIN_COL_W: f32 = 28.0;
    pub const MIN_ROW_H: f32 = 18.0;

    pub fn new(rows: usize, cols: usize) -> Self {
        let mut t = Table { cells: vec![vec![String::new(); cols.max(1)]; rows.max(1)], col_w: Vec::new(), row_h: Self::DEFAULT_ROW_H };
        t.normalize();
        t
    }

    pub fn rows(&self) -> usize {
        self.cells.len()
    }

    pub fn cols(&self) -> usize {
        self.cells.iter().map(|r| r.len()).max().unwrap_or(1).max(1)
    }

    /// 补齐行长 / 列宽表到统一列数。
    pub fn normalize(&mut self) {
        if self.cells.is_empty() {
            self.cells.push(vec![String::new()]);
        }
        let w = self.cols();
        for r in self.cells.iter_mut() {
            r.resize(w, String::new());
        }
        self.col_w.resize(w, Self::DEFAULT_COL_W);
        for x in self.col_w.iter_mut() {
            *x = x.max(Self::MIN_COL_W);
        }
        self.row_h = self.row_h.max(Self::MIN_ROW_H);
    }

    fn is_default_size(&self) -> bool {
        self.row_h == Self::DEFAULT_ROW_H && self.col_w.iter().all(|w| *w == Self::DEFAULT_COL_W)
    }

    fn meta_comment(&self) -> String {
        let cols: Vec<String> = self.col_w.iter().map(|w| format!("{}", w.round() as i32)).collect();
        format!("<!-- pla:table cols={} row={} -->", cols.join(","), self.row_h.round() as i32)
    }

    /// 解析 `<!-- pla:table cols=.. row=.. -->`; 不是这种注释 → None。
    fn parse_meta(line: &str) -> Option<(Vec<f32>, f32)> {
        let inner = line.trim().strip_prefix("<!--")?.strip_suffix("-->")?.trim();
        let rest = inner.strip_prefix("pla:table")?;
        let mut cols = Vec::new();
        let mut row = Self::DEFAULT_ROW_H;
        for kv in rest.split_whitespace() {
            if let Some(v) = kv.strip_prefix("cols=") {
                cols = v.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect();
            } else if let Some(v) = kv.strip_prefix("row=") {
                row = v.parse().unwrap_or(row);
            }
        }
        Some((cols, row))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// 运行期唯一 id(egui 控件 id / 拖动排序用), 不存盘。
    pub id: u64,
    pub kind: Kind,
    pub text: String,
    /// 缩进级数 0..=MAX_INDENT。
    pub indent: u8,
}

pub const MAX_INDENT: u8 = 4;

#[derive(Debug, Clone, Default)]
pub struct Note {
    pub blocks: Vec<Block>,
    next_id: u64,
}

impl Note {
    pub fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn block(&mut self, kind: Kind, text: impl Into<String>, indent: u8) -> Block {
        Block { id: self.new_id(), kind, text: text.into(), indent }
    }

    /// 空笔记至少有一行可输入。
    pub fn ensure_nonempty(&mut self) {
        if self.blocks.is_empty() {
            let b = self.block(Kind::Text, "", 0);
            self.blocks.push(b);
        }
    }

    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.blocks.iter().position(|b| b.id == id)
    }

    /// 有序列表每项的显示序号(同缩进、连续的 Number 块计数; 被别的块打断则重新从 1 开始)。
    pub fn number_of(&self, i: usize) -> usize {
        let b = &self.blocks[i];
        let mut n = 1;
        let mut j = i;
        while j > 0 {
            j -= 1;
            let p = &self.blocks[j];
            if p.indent > b.indent {
                continue; // 更深一级的子项不打断
            }
            if p.indent == b.indent && p.kind == Kind::Number {
                n += 1;
                continue;
            }
            break;
        }
        n
    }
}

// ---------------- 解析 ----------------

pub fn parse(md: &str) -> Note {
    let mut note = Note::default();
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;
    let mut pending_meta: Option<(Vec<f32>, f32)> = None;
    while i < lines.len() {
        let raw = lines[i];
        let (indent, rest) = split_indent(raw);
        // R31: 表格尺寸注释 — 只在紧跟着表格时生效, 否则当普通文本
        if let Some(meta) = Table::parse_meta(rest) {
            if lines.get(i + 1).map_or(false, |l| split_indent(l).1.starts_with('|')) {
                pending_meta = Some(meta);
                i += 1;
                continue;
            }
        }
        if rest.starts_with('|') {
            // 管道表: 连续以 | 开头的行; 第 2 行若是分隔行(|---|)则跳过
            let mut rows = Vec::new();
            while i < lines.len() && split_indent(lines[i]).1.starts_with('|') {
                let r = split_indent(lines[i]).1;
                if !is_separator_row(r) {
                    rows.push(split_row(r));
                }
                i += 1;
            }
            let (col_w, row_h) = pending_meta.take().unwrap_or((Vec::new(), Table::DEFAULT_ROW_H));
            let mut t = Table { cells: rows, col_w, row_h };
            t.normalize();
            let b = note.block(Kind::Table(t), "", 0);
            note.blocks.push(b);
            continue;
        }
        let (kind, text) = classify(rest);
        let b = note.block(kind, text, indent);
        note.blocks.push(b);
        i += 1;
    }
    // 文件末尾的空行不当内容(编辑器常自动补换行)
    while note.blocks.len() > 1
        && note.blocks.last().map_or(false, |b| b.kind == Kind::Text && b.text.is_empty())
    {
        note.blocks.pop();
    }
    note.ensure_nonempty();
    note
}

fn split_indent(line: &str) -> (u8, &str) {
    let mut width = 0usize;
    let mut cut = 0usize;
    for (i, ch) in line.char_indices() {
        match ch {
            ' ' => width += 1,
            '\t' => width += 2,
            _ => {
                cut = i;
                break;
            }
        }
        cut = i + ch.len_utf8();
    }
    (((width / 2) as u8).min(MAX_INDENT), &line[cut..])
}

/// 行首标记 → (类型, 正文)。
fn classify(s: &str) -> (Kind, String) {
    for (pre, done) in [("- [ ] ", false), ("- [x] ", true), ("- [X] ", true), ("* [ ] ", false), ("* [x] ", true)] {
        if let Some(t) = s.strip_prefix(pre) {
            return (Kind::Todo(done), t.to_string());
        }
    }
    for (pre, done) in [("- [ ]", false), ("- [x]", true), ("- [X]", true)] {
        if s == pre {
            return (Kind::Todo(done), String::new());
        }
    }
    for pre in ["- ", "* ", "+ "] {
        if let Some(t) = s.strip_prefix(pre) {
            return (Kind::Bullet, t.to_string());
        }
    }
    if s == "-" || s == "*" {
        return (Kind::Bullet, String::new());
    }
    if let Some(t) = strip_number_prefix(s) {
        return (Kind::Number, t.to_string());
    }
    for (pre, lv) in [("### ", 3u8), ("## ", 2), ("# ", 1)] {
        if let Some(t) = s.strip_prefix(pre) {
            return (Kind::Heading(lv), t.to_string());
        }
    }
    let t = s.trim();
    if t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*') || t.chars().all(|c| c == '_')) {
        return (Kind::Divider, String::new());
    }
    (Kind::Text, s.to_string())
}

/// "12. 文本" / "3) 文本" → "文本"。
fn strip_number_prefix(s: &str) -> Option<&str> {
    let digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 4 {
        return None;
    }
    let rest = &s[digits..];
    rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")).or_else(|| {
        if rest == "." || rest == ")" { Some("") } else { None }
    })
}

fn is_separator_row(r: &str) -> bool {
    let inner = r.trim().trim_matches('|');
    !inner.is_empty()
        && inner.split('|').all(|c| {
            let c = c.trim();
            !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':')
        })
}

/// `| a | b \| c |` → ["a", "b | c"]。
fn split_row(r: &str) -> Vec<String> {
    let s = r.trim();
    let s = s.strip_prefix('|').unwrap_or(s);
    let s = s.strip_suffix('|').filter(|x| !x.ends_with('\\')).unwrap_or(s);
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cur.push('|');
                chars.next();
            }
            '|' => {
                cells.push(cell_from_md(&cur));
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    cells.push(cell_from_md(&cur));
    cells
}

/// R31: 单元格内换行(Alt+Enter)在 Markdown 表格里存成 `<br>`。
fn cell_from_md(s: &str) -> String {
    s.trim().replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n")
}

fn cell_to_md(s: &str) -> String {
    s.replace('|', "\\|").replace("\r\n", "\n").replace('\n', "<br>")
}

// ---------------- 序列化 ----------------

pub fn to_markdown(note: &Note) -> String {
    let mut out = String::new();
    for (i, b) in note.blocks.iter().enumerate() {
        let pad = "  ".repeat(b.indent as usize);
        match &b.kind {
            Kind::Text => out.push_str(&format!("{pad}{}", b.text)),
            Kind::Heading(lv) => out.push_str(&format!("{pad}{} {}", "#".repeat(*lv as usize), b.text)),
            Kind::Bullet => out.push_str(&format!("{pad}- {}", b.text)),
            Kind::Number => out.push_str(&format!("{pad}{}. {}", note.number_of(i), b.text)),
            Kind::Todo(done) => out.push_str(&format!("{pad}- [{}] {}", if *done { "x" } else { " " }, b.text)),
            Kind::Divider => out.push_str("---"),
            Kind::Table(t) => {
                if !t.is_default_size() {
                    out.push_str(&t.meta_comment());
                    out.push('\n');
                }
                let rows = &t.cells;
                let w = t.cols();
                for (ri, r) in rows.iter().enumerate() {
                    out.push('|');
                    for c in 0..w {
                        let cell = r.get(c).map(|s| s.as_str()).unwrap_or("");
                        out.push_str(&format!(" {} |", cell_to_md(cell)));
                    }
                    if ri == 0 {
                        out.push('\n');
                        out.push('|');
                        for _ in 0..w {
                            out.push_str(" --- |");
                        }
                    }
                    if ri + 1 < rows.len() {
                        out.push('\n');
                    }
                }
            }
        }
        out.push('\n');
    }
    out
}

// ---------------- 输入快捷方式(Markdown 前缀即时转换) ----------------

/// 正文块里敲出 Markdown 前缀时转换类型并去掉前缀(类似 Keep / Notion)。返回是否转换。
/// `- ` `* ` → 无序; `1. ` → 有序; `[] ` `[ ] ` `- [ ] ` → 待办; `# ` → 标题; `---` → 分隔线。
pub fn apply_shortcut(b: &mut Block) -> bool {
    if b.kind != Kind::Text {
        // 列表项里再敲 "[] " 也转待办(常见于从无序列表改待办)
        if b.kind == Kind::Bullet {
            for pre in ["[] ", "[ ] "] {
                if let Some(t) = b.text.strip_prefix(pre) {
                    b.text = t.to_string();
                    b.kind = Kind::Todo(false);
                    return true;
                }
            }
        }
        return false;
    }
    for pre in ["[] ", "[ ] ", "- [ ] ", "- [] "] {
        if let Some(t) = b.text.strip_prefix(pre) {
            b.text = t.to_string();
            b.kind = Kind::Todo(false);
            return true;
        }
    }
    if b.text == "---" {
        b.text.clear();
        b.kind = Kind::Divider;
        return true;
    }
    let (kind, text) = classify(&b.text);
    match kind {
        Kind::Text | Kind::Divider | Kind::Table(_) => false,
        k => {
            b.kind = k;
            b.text = text;
            true
        }
    }
}

// ---------------- 行内样式 ----------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    /// `<span style="color:#RRGGBB">` 文字颜色(RGB)。
    pub color: Option<[u8; 3]>,
    /// `<span style="background:#RRGGBB">` 背景(高亮)色(RGB)。
    pub bg: Option<[u8; 3]>,
}

/// 原文的一段: `markup = true` 是标记本身(`**` / `<span …>` 等), 否则是正文。
/// 所有片段的 `text` 依次拼接 == 原文(编辑态 layouter 靠这个保证 galley 与原文一致)。
#[derive(Debug, Clone, PartialEq)]
pub struct InlineTok {
    pub text: String,
    pub style: Style,
    pub markup: bool,
}

/// 一对配上了的 `<span …>…</span>`(字符下标, 半开区间)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpanTag {
    pub open: (usize, usize),
    pub close: (usize, usize),
    pub color: Option<[u8; 3]>,
    pub bg: Option<[u8; 3]>,
}

const SPAN_CLOSE: &str = "</span>";

/// `#RGB` / `#RRGGBB` → RGB。
pub fn parse_hex_color(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().strip_prefix('#')?;
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match h.len() {
        6 => Some([
            u8::from_str_radix(&h[0..2], 16).ok()?,
            u8::from_str_radix(&h[2..4], 16).ok()?,
            u8::from_str_radix(&h[4..6], 16).ok()?,
        ]),
        3 => {
            let d = |k: usize| u8::from_str_radix(&h[k..k + 1], 16).ok().map(|v| v * 17);
            Some([d(0)?, d(1)?, d(2)?])
        }
        _ => None,
    }
}

pub fn hex_color(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// `<span style="color:#..; background:#..">` 开标签; `start` 起是 `<`。
/// 是 → (标签字符数, 颜色, 背景)。不认识的标签 / 没有 `>` → None(按原文显示)。
fn parse_span_open(chars: &[char], start: usize) -> Option<(usize, Option<[u8; 3]>, Option<[u8; 3]>)> {
    let head: String = chars.get(start..start + 5)?.iter().collect();
    if !head.eq_ignore_ascii_case("<span") {
        return None;
    }
    let after = *chars.get(start + 5)?;
    if !(after == '>' || after.is_whitespace()) {
        return None;
    }
    let mut end = start + 5;
    while end < chars.len() && chars[end] != '>' {
        if chars[end] == '<' || chars[end] == '\n' {
            return None;
        }
        end += 1;
    }
    if end >= chars.len() {
        return None;
    }
    let attrs: String = chars[start + 5..end].iter().collect();
    let (mut color, mut bg) = (None, None);
    let lower = attrs.to_ascii_lowercase();
    if let Some(p) = lower.find("style") {
        let rest = attrs[p + 5..].trim_start();
        if let Some(rest) = rest.strip_prefix('=') {
            let rest = rest.trim_start();
            let q = rest.chars().next().filter(|c| *c == '"' || *c == '\'');
            let body = match q {
                Some(q) => rest[1..].split(q).next().unwrap_or(""),
                None => rest.split_whitespace().next().unwrap_or(""),
            };
            for decl in body.split(';') {
                let Some((k, v)) = decl.split_once(':') else { continue };
                match k.trim().to_ascii_lowercase().as_str() {
                    "color" => color = parse_hex_color(v).or(color),
                    "background" | "background-color" => bg = parse_hex_color(v).or(bg),
                    _ => {}
                }
            }
        }
    }
    Some((end + 1 - start, color, bg))
}

fn is_span_close(chars: &[char], i: usize) -> bool {
    let n = SPAN_CLOSE.len();
    i + n <= chars.len() && chars[i..i + n].iter().collect::<String>().eq_ignore_ascii_case(SPAN_CLOSE)
}

fn has_closing(chars: &[char], from: usize, pat: &[char]) -> bool {
    let mut k = from;
    while k + pat.len() <= chars.len() {
        if chars[k..k + pat.len()] == *pat {
            return true;
        }
        k += 1;
    }
    false
}

/// 原文里所有配上对的 span 标签(代码段里的不算; 未闭合的开标签 / 多余的闭标签不算)。
pub fn span_tags(text: &str) -> Vec<SpanTag> {
    let chars: Vec<char> = text.chars().collect();
    let mut stack: Vec<(usize, usize, Option<[u8; 3]>, Option<[u8; 3]>)> = Vec::new();
    let mut out = Vec::new();
    let mut code = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if code {
            code = c != '`';
            i += 1;
        } else if c == '`' && has_closing(&chars, i + 1, &['`']) {
            code = true;
            i += 1;
        } else if let Some((len, color, bg)) = (c == '<').then(|| parse_span_open(&chars, i)).flatten() {
            stack.push((i, i + len, color, bg));
            i += len;
        } else if c == '<' && is_span_close(&chars, i) {
            let n = SPAN_CLOSE.len();
            if let Some((a, b, color, bg)) = stack.pop() {
                out.push(SpanTag { open: (a, b), close: (i, i + n), color, bg });
            }
            i += n;
        } else {
            i += 1;
        }
    }
    out.sort_by_key(|t| t.open.0);
    out
}

/// 原文 → 标记 / 正文片段(带样式)。见 `InlineTok`。
pub fn inline_tokens(text: &str) -> Vec<InlineTok> {
    let chars: Vec<char> = text.chars().collect();
    // 配上对的 span 标签: 下标 → (长度, Some(开标签效果) / None = 闭标签)
    let mut tags: std::collections::HashMap<usize, (usize, Option<(Option<[u8; 3]>, Option<[u8; 3]>)>)> =
        std::collections::HashMap::new();
    for t in span_tags(text) {
        tags.insert(t.open.0, (t.open.1 - t.open.0, Some((t.color, t.bg))));
        tags.insert(t.close.0, (t.close.1 - t.close.0, None));
    }
    let mut out: Vec<InlineTok> = Vec::new();
    let mut st = Style::default();
    let mut cur = String::new();
    let mut span_stack: Vec<(Option<[u8; 3]>, Option<[u8; 3]>)> = Vec::new();
    let flush = |cur: &mut String, out: &mut Vec<InlineTok>, st: Style| {
        if !cur.is_empty() {
            out.push(InlineTok { text: std::mem::take(cur), style: st, markup: false });
        }
    };
    let mark = |out: &mut Vec<InlineTok>, s: &[char], st: Style| {
        out.push(InlineTok { text: s.iter().collect(), style: st, markup: true });
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if st.code {
            if c == '`' {
                flush(&mut cur, &mut out, st);
                mark(&mut out, &chars[i..i + 1], st);
                st.code = false;
            } else {
                cur.push(c);
            }
            i += 1;
            continue;
        }
        let two = |a: char| i + 1 < chars.len() && chars[i] == a && chars[i + 1] == a;
        if let Some(&(len, eff)) = tags.get(&i) {
            flush(&mut cur, &mut out, st);
            mark(&mut out, &chars[i..i + len], st);
            match eff {
                Some((color, bg)) => {
                    span_stack.push((st.color, st.bg));
                    st.color = color.or(st.color);
                    st.bg = bg.or(st.bg);
                }
                None => {
                    if let Some((color, bg)) = span_stack.pop() {
                        st.color = color;
                        st.bg = bg;
                    }
                }
            }
            i += len;
        } else if c == '`' && has_closing(&chars, i + 1, &['`']) {
            flush(&mut cur, &mut out, st);
            mark(&mut out, &chars[i..i + 1], st);
            st.code = true;
            i += 1;
        } else if two('*') && (st.bold || has_closing(&chars, i + 2, &['*', '*'])) {
            flush(&mut cur, &mut out, st);
            mark(&mut out, &chars[i..i + 2], st);
            st.bold = !st.bold;
            i += 2;
        } else if two('~') && (st.strike || has_closing(&chars, i + 2, &['~', '~'])) {
            flush(&mut cur, &mut out, st);
            mark(&mut out, &chars[i..i + 2], st);
            st.strike = !st.strike;
            i += 2;
        } else if c == '*' && (st.italic || has_closing(&chars, i + 1, &['*'])) {
            flush(&mut cur, &mut out, st);
            mark(&mut out, &chars[i..i + 1], st);
            st.italic = !st.italic;
            i += 1;
        } else {
            cur.push(c);
            i += 1;
        }
    }
    flush(&mut cur, &mut out, st);
    out
}

/// `**粗**` `*斜*` `` `代码` `` `~~删除~~` `<span style="color:#..">` → 带样式片段
/// (未闭合的标记 / 不认识的标签按原文显示)。
pub fn inline_spans(text: &str) -> Vec<(String, Style)> {
    let mut out: Vec<(String, Style)> = Vec::new();
    for t in inline_tokens(text).into_iter().filter(|t| !t.markup) {
        match out.last_mut() {
            Some((s, st)) if *st == t.style => s.push_str(&t.text),
            _ => out.push((t.text, t.style)),
        }
    }
    out
}

// ---------------- 存取(跟着文档走) ----------------

pub fn notes_path(doc_key: &str) -> PathBuf {
    let dir = crate::snapshot::doc_side_dir(doc_key, "notes").unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("notes")
            .join(crate::snapshot::sanitize_filename(doc_key))
    });
    dir.join("notes.md")
}

pub fn load(doc_key: &str) -> Note {
    match std::fs::read_to_string(notes_path(doc_key)) {
        Ok(s) => parse(&s),
        Err(_) => {
            let mut n = Note::default();
            n.ensure_nonempty();
            n
        }
    }
}

/// 写盘(先写临时文件再改名, 写到一半崩溃不会留下半截笔记)。
pub fn save(doc_key: &str, note: &Note) -> Result<PathBuf, String> {
    let path = notes_path(doc_key);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, to_markdown(note)).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# 标题\n正文 **粗**\n- 项目\n  - 子项目\n1. 第一\n1. 第二\n- [ ] 待办\n- [x] 已办\n---\n| 名称 | 数量 | 小计 |\n| --- | --- | --- |\n| 螺丝 | 4 | =B2*0.5 |\n| 合计 | =SUM(B2:B2) | a\\|b |\n";

    #[test]
    fn parses_all_kinds() {
        let n = parse(SAMPLE);
        let kinds: Vec<_> = n.blocks.iter().map(|b| (b.kind.clone(), b.indent)).collect();
        assert_eq!(kinds[0], (Kind::Heading(1), 0));
        assert_eq!(kinds[1], (Kind::Text, 0));
        assert_eq!(kinds[2], (Kind::Bullet, 0));
        assert_eq!(kinds[3], (Kind::Bullet, 1));
        assert_eq!(kinds[4].0, Kind::Number);
        assert_eq!(kinds[6].0, Kind::Todo(false));
        assert_eq!(kinds[7].0, Kind::Todo(true));
        assert_eq!(kinds[8].0, Kind::Divider);
        match &n.blocks[9].kind {
            Kind::Table(t) => {
                let rows = &t.cells;
                assert_eq!(rows.len(), 3, "分隔行不算数据行");
                assert_eq!(rows[1][2], "=B2*0.5");
                assert_eq!(rows[2][2], "a|b", "转义的竖线");
            }
            k => panic!("应为表格: {k:?}"),
        }
        assert_eq!(n.blocks.len(), 10);
    }

    #[test]
    fn round_trips_and_renumbers() {
        let n = parse(SAMPLE);
        let md = to_markdown(&n);
        assert!(md.contains("1. 第一\n2. 第二\n"), "有序列表重排序号: {md}");
        assert!(md.contains("  - 子项目\n"));
        assert!(md.contains("| a\\|b |"));
        let again = parse(&md);
        assert_eq!(
            again.blocks.iter().map(|b| (&b.kind, &b.text, b.indent)).collect::<Vec<_>>(),
            n.blocks.iter().map(|b| (&b.kind, &b.text, b.indent)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn numbering_restarts_after_break_and_skips_children() {
        let n = parse("1. a\n  - x\n1. b\n正文\n1. c\n");
        assert_eq!(n.number_of(0), 1);
        assert_eq!(n.number_of(2), 2, "子项不打断");
        assert_eq!(n.number_of(4), 1, "被正文打断后重新计数");
    }

    #[test]
    fn shortcuts_convert_text_blocks() {
        let mut n = Note::default();
        let cases = [
            ("- 买螺丝", Kind::Bullet, "买螺丝"),
            ("1. 第一步", Kind::Number, "第一步"),
            ("[] 检查", Kind::Todo(false), "检查"),
            ("- [ ] 检查", Kind::Todo(false), "检查"),
            ("## 小节", Kind::Heading(2), "小节"),
            ("---", Kind::Divider, ""),
        ];
        for (input, kind, text) in cases {
            let mut b = n.block(Kind::Text, input, 0);
            assert!(apply_shortcut(&mut b), "{input}");
            assert_eq!((b.kind, b.text.as_str()), (kind, text), "{input}");
        }
        let mut plain = n.block(Kind::Text, "普通-文字", 0);
        assert!(!apply_shortcut(&mut plain));
        let mut bullet = n.block(Kind::Bullet, "[] 改待办", 0);
        assert!(apply_shortcut(&mut bullet));
        assert_eq!(bullet.kind, Kind::Todo(false));
    }

    #[test]
    fn inline_styles() {
        let s = inline_spans("a **b** *c* `d*` ~~e~~ f*");
        let flat: Vec<(&str, Style)> = s.iter().map(|(t, st)| (t.as_str(), *st)).collect();
        assert_eq!(flat[0], ("a ", Style::default()));
        assert_eq!(flat[1], ("b", Style { bold: true, ..Default::default() }));
        assert_eq!(flat[3], ("c", Style { italic: true, ..Default::default() }));
        assert_eq!(flat[5], ("d*", Style { code: true, ..Default::default() }), "代码里不解析星号");
        assert_eq!(flat[7], ("e", Style { strike: true, ..Default::default() }));
        assert_eq!(flat.last().unwrap().0, " f*", "未闭合的星号按原文");
    }

    const RED: [u8; 3] = [0xFF, 0x6B, 0x6B];
    const DARK: [u8; 3] = [0x1E, 0x3A, 0x5F];

    #[test]
    fn inline_span_colors() {
        let s = inline_spans(r#"a <span style="color:#FF6B6B">红</span> b"#);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0], ("a ".into(), Style::default()));
        assert_eq!(s[1], ("红".into(), Style { color: Some(RED), ..Default::default() }));
        assert_eq!(s[2], (" b".into(), Style::default()));

        let s = inline_spans(r#"<span style="background:#1e3a5f">hl</span>"#);
        assert_eq!(s, vec![("hl".into(), Style { bg: Some(DARK), ..Default::default() })]);

        let s = inline_spans(r#"<SPAN style='color: #f66; background-color:#1E3A5F'>x</SPAN>"#);
        assert_eq!(s, vec![("x".into(), Style { color: Some([0xFF, 0x66, 0x66]), bg: Some(DARK), ..Default::default() })]);
    }

    #[test]
    fn inline_span_nesting() {
        let s = inline_spans(r#"<span style="color:#FF6B6B">a **b** <span style="background:#1E3A5F">c</span></span>d"#);
        let flat: Vec<(&str, Style)> = s.iter().map(|(t, st)| (t.as_str(), *st)).collect();
        let red = Style { color: Some(RED), ..Default::default() };
        assert_eq!(flat[0], ("a ", red));
        assert_eq!(flat[1], ("b", Style { bold: true, ..red }));
        assert_eq!(flat[2], (" ", red));
        assert_eq!(flat[3], ("c", Style { bg: Some(DARK), ..red }));
        assert_eq!(flat[4], ("d", Style::default()));
        // 粗体包着 span
        let s = inline_spans(r#"**<span style="color:#FF6B6B">x</span>**"#);
        assert_eq!(s, vec![("x".into(), Style { bold: true, color: Some(RED), ..Default::default() })]);
    }

    #[test]
    fn inline_span_literal_fallbacks() {
        let raw = r#"<span style="color:#FF6B6B">未闭合"#;
        assert_eq!(inline_spans(raw), vec![(raw.to_string(), Style::default())]);
        assert_eq!(inline_spans("a </span> b"), vec![("a </span> b".to_string(), Style::default())]);
        assert_eq!(inline_spans("<b>x</b> 1<2"), vec![("<b>x</b> 1<2".to_string(), Style::default())]);
        // 代码里的标签按原文
        let s = inline_spans(r#"`<span style="color:#FF6B6B">`x</span>"#);
        assert_eq!(s[0].0, r#"<span style="color:#FF6B6B">"#);
        assert!(s[0].1.code);
        assert_eq!(s[1].0, "x</span>");
    }

    #[test]
    fn inline_tokens_cover_source() {
        for src in [
            r#"a **b** <span style="color:#FF6B6B">*c*</span> `d` ~~e~~ f* <span>"#,
            "中文 **粗体** 和 *斜体*",
            "",
        ] {
            let toks = inline_tokens(src);
            assert_eq!(toks.iter().map(|t| t.text.as_str()).collect::<String>(), src);
        }
        let toks = inline_tokens(r#"<span style="color:#FF6B6B">x</span>"#);
        assert!(toks[0].markup && !toks[1].markup && toks[2].markup);
    }

    #[test]
    fn table_size_meta_round_trips() {
        let md = "<!-- pla:table cols=60,120 row=30 -->\n| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let n = parse(md);
        assert_eq!(n.blocks.len(), 1, "注释不单独成行");
        let Kind::Table(t) = &n.blocks[0].kind else { panic!() };
        assert_eq!((t.col_w.clone(), t.row_h), (vec![60.0, 120.0], 30.0));
        assert_eq!(to_markdown(&n), md);
        // 默认尺寸不写注释; 不跟表格的注释当正文
        let plain = parse("| a |\n| --- |\n");
        assert!(!to_markdown(&plain).contains("pla:table"));
        let lone = parse("<!-- pla:table cols=1 row=2 -->\n正文\n");
        assert_eq!(lone.blocks[0].kind, Kind::Text);
        // 单元格内换行 ↔ <br>
        let br = parse("| a<br>b |\n| --- |\n");
        let Kind::Table(t) = &br.blocks[0].kind else { panic!() };
        assert_eq!(t.cells[0][0], "a\nb");
        assert!(to_markdown(&br).contains("| a<br>b |"));
    }

    #[test]
    fn empty_and_trailing_blank_lines() {
        assert_eq!(parse("").blocks.len(), 1);
        assert_eq!(parse("a\n\n\n").blocks.len(), 1);
        assert_eq!(parse("a\n\nb\n").blocks.len(), 3, "中间空行保留");
    }
}
