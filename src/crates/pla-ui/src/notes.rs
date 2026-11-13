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
    Table(Vec<Vec<String>>),
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
    while i < lines.len() {
        let raw = lines[i];
        let (indent, rest) = split_indent(raw);
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
            let w = rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
            for r in rows.iter_mut() {
                r.resize(w, String::new());
            }
            let b = note.block(Kind::Table(rows), "", 0);
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
                cells.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    cells.push(cur.trim().to_string());
    cells
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
            Kind::Table(rows) => {
                let w = rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
                for (ri, r) in rows.iter().enumerate() {
                    out.push('|');
                    for c in 0..w {
                        let cell = r.get(c).map(|s| s.as_str()).unwrap_or("");
                        out.push_str(&format!(" {} |", cell.replace('|', "\\|").replace('\n', " ")));
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
}

/// `**粗**` `*斜*` `` `代码` `` `~~删除~~` → 带样式片段(不支持嵌套交叉; 未闭合的标记按原文显示)。
pub fn inline_spans(text: &str) -> Vec<(String, Style)> {
    let mut out: Vec<(String, Style)> = Vec::new();
    let mut st = Style::default();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    let has_closing = |from: usize, pat: &[char]| -> bool {
        let mut k = from;
        while k + pat.len() <= chars.len() {
            if chars[k..k + pat.len()] == *pat {
                return true;
            }
            k += 1;
        }
        false
    };
    let mut i = 0;
    let flush = |cur: &mut String, out: &mut Vec<(String, Style)>, st: Style| {
        if !cur.is_empty() {
            out.push((std::mem::take(cur), st));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if st.code {
            if c == '`' {
                flush(&mut cur, &mut out, st);
                st.code = false;
            } else {
                cur.push(c);
            }
            i += 1;
            continue;
        }
        let two = |a: char| i + 1 < chars.len() && chars[i] == a && chars[i + 1] == a;
        if c == '`' && has_closing(i + 1, &['`']) {
            flush(&mut cur, &mut out, st);
            st.code = true;
            i += 1;
        } else if two('*') && (st.bold || has_closing(i + 2, &['*', '*'])) {
            flush(&mut cur, &mut out, st);
            st.bold = !st.bold;
            i += 2;
        } else if two('~') && (st.strike || has_closing(i + 2, &['~', '~'])) {
            flush(&mut cur, &mut out, st);
            st.strike = !st.strike;
            i += 2;
        } else if c == '*' && (st.italic || has_closing(i + 1, &['*'])) {
            flush(&mut cur, &mut out, st);
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
            Kind::Table(rows) => {
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

    #[test]
    fn empty_and_trailing_blank_lines() {
        assert_eq!(parse("").blocks.len(), 1);
        assert_eq!(parse("a\n\n\n").blocks.len(), 1);
        assert_eq!(parse("a\n\nb\n").blocks.len(), 3, "中间空行保留");
    }
}
