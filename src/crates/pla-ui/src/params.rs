//! P5 参数: 常量 / 变量 / 函数关系(用户 2026-11-16: 参数化里保留分组, 取消"组值联动", 升级为常量、变量和函数关系)。
//!
//! - **常量**: 一个数(表里改, 显示锁定), 如板厚 T = 3;
//! - **变量**: 一个数, 平时拨的"旋钮", 如 W = 100;
//! - **函数**: 由其它名字算出来的表达式, 如 Half = W / 2 - T。
//! - **尺寸绑定**: 尺寸记录可带表达式(`DimRecord.expr`), 带了就由它驱动, 如 D002 = W/2 - 3;
//!   没绑定的尺寸可在表达式里按名字当输入用(值 = 最近读到的值), 如 D001 + 5。
//!
//! 表达式 = `formula` 引擎的语法(+ - * / ^、括号、SQRT / ROUND / IF / MIN / MAX…); 名字先换成数再交给引擎。
//! 名字: 字母 / 汉字 / 下划线开头, 后面可跟字母 / 数字 / 下划线 / 汉字 / 点(尺寸名如 H001.R 带点)。
//! 后面紧跟 "(" 的是函数名, 不当变量。
//! 求值按依赖顺序, 环 → 报错(环上每个名字都报), 不会死循环。参数和尺寸记录一起存在 dims.json。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::formula;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Param {
    pub id: String,
    pub name: String,
    /// "const" | "var" | "func"。
    pub kind: String,
    /// 常量 / 变量 = 数字文本(也允许表达式); 函数 = 表达式。
    pub expr: String,
    #[serde(default)]
    pub note: String,
}

pub const KINDS: [&str; 3] = ["const", "var", "func"];

/// 表达式里的一个片段。
#[derive(Debug, Clone, PartialEq)]
enum Tok<'a> {
    Name(&'a str),
    Func(&'a str),
    Other(&'a str),
}

fn name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

/// 切词: 名字 / 函数名 / 其它(数字、运算符、空白、字符串字面量原样)。数字里的字母(1e3)不当名字。
fn tokens(src: &str) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    let b: Vec<(usize, char)> = src.char_indices().collect();
    let mut i = 0;
    let mut other_start = 0;
    fn flush<'a>(out: &mut Vec<Tok<'a>>, src: &'a str, a: usize, z: usize) {
        if z > a {
            out.push(Tok::Other(&src[a..z]));
        }
    }
    while i < b.len() {
        let (pos, c) = b[i];
        if c == '"' {
            // 字符串字面量: 跳到配对的引号
            i += 1;
            while i < b.len() && b[i].1 != '"' {
                i += 1;
            }
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && b.get(i + 1).is_some_and(|x| x.1.is_ascii_digit())) {
            // 数字(含小数点 / 科学计数法)
            i += 1;
            while i < b.len() && (b[i].1.is_ascii_digit() || b[i].1 == '.') {
                i += 1;
            }
            if i < b.len() && (b[i].1 == 'e' || b[i].1 == 'E') {
                let j = i + 1 + usize::from(b.get(i + 1).is_some_and(|x| x.1 == '+' || x.1 == '-'));
                if b.get(j).is_some_and(|x| x.1.is_ascii_digit()) {
                    i = j;
                    while i < b.len() && b[i].1.is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            continue;
        }
        if name_start(c) {
            flush(&mut out, src, other_start, pos);
            let mut j = i + 1;
            while j < b.len() && name_char(b[j].1) {
                j += 1;
            }
            // 名字末尾的点不算(如 "W." 少见, 保守处理)
            while j > i + 1 && b[j - 1].1 == '.' {
                j -= 1;
            }
            let end = b.get(j).map(|x| x.0).unwrap_or(src.len());
            let mut k = j;
            while k < b.len() && b[k].1 == ' ' {
                k += 1;
            }
            let word = &src[pos..end];
            out.push(if b.get(k).is_some_and(|x| x.1 == '(') { Tok::Func(word) } else { Tok::Name(word) });
            i = j;
            other_start = end;
            continue;
        }
        i += 1;
    }
    flush(&mut out, src, other_start, src.len());
    out
}

/// 表达式引用的名字(去重、按出现顺序)。
pub fn refs(expr: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for t in tokens(expr) {
        if let Tok::Name(n) = t {
            if seen.insert(n.to_string()) {
                out.push(n.to_string());
            }
        }
    }
    out
}

/// 把表达式里的名字 old 改成 new(改名时用; 函数名、字符串、数字不动)。
pub fn rename_ref(expr: &str, old: &str, new: &str) -> String {
    let mut out = String::new();
    let mut last = 0;
    for t in tokens(expr) {
        if let Tok::Name(n) = t {
            let start = n.as_ptr() as usize - expr.as_ptr() as usize;
            out.push_str(&expr[last..start]);
            out.push_str(if n == old { new } else { n });
            last = start + n.len();
        }
    }
    out.push_str(&expr[last..]);
    out
}

/// 名字是否合法(参数名 / 能被表达式引用)。
pub fn valid_name(name: &str) -> bool {
    let mut cs = name.chars();
    match cs.next() {
        Some(c) if name_start(c) => {}
        _ => return false,
    }
    name.chars().all(name_char) && !name.ends_with('.')
}

/// 把名字换成值后交给公式引擎。
fn eval_with(expr: &str, get: &mut dyn FnMut(&str) -> Result<f64, String>) -> Result<f64, String> {
    let src = expr.trim();
    if src.is_empty() {
        return Err("empty expression".into());
    }
    let mut s = String::new();
    let mut last = 0;
    for t in tokens(src) {
        if let Tok::Name(n) = t {
            let start = n.as_ptr() as usize - src.as_ptr() as usize;
            s.push_str(&src[last..start]);
            let v = get(n)?;
            s.push_str(&format!("({v:?})"));
            last = start + n.len();
        }
    }
    s.push_str(&src[last..]);
    match formula::eval_expr(&format!("={s}")) {
        formula::Value::Num(x) if x.is_finite() => Ok(x),
        formula::Value::Num(_) => Err("not a finite number".into()),
        formula::Value::Bool(b) => Ok(if b { 1.0 } else { 0.0 }),
        formula::Value::Error(e) => Err(format!("{e}")),
        other => Err(format!("not a number: {other:?}")),
    }
}

/// 一次求值的输入: 参数 + 尺寸(名字, 当前值, 绑定的表达式)。
pub struct Inputs<'a> {
    pub params: &'a [Param],
    pub dims: Vec<(&'a str, f64, Option<&'a str>)>,
}

/// 求值结果: 每个名字(参数 + 尺寸)→ 值或错误; 绑定了表达式的尺寸 → 目标值。
#[derive(Debug, Default, Clone)]
pub struct Eval {
    pub values: BTreeMap<String, Result<f64, String>>,
    /// 绑定了表达式的尺寸: 名字 → 目标值或错误。
    pub targets: BTreeMap<String, Result<f64, String>>,
}

impl Eval {
    /// 要改的尺寸: 目标值与当前值差 > tol 的 (名字, 目标)。
    pub fn changes(&self, current: &BTreeMap<String, f64>, tol: f64) -> Vec<(String, f64)> {
        self.targets
            .iter()
            .filter_map(|(n, r)| match (r, current.get(n)) {
                (Ok(t), Some(c)) if (t - c).abs() > tol => Some((n.clone(), *t)),
                _ => None,
            })
            .collect()
    }

    pub fn errors(&self) -> Vec<(String, String)> {
        self.values.iter().filter_map(|(n, r)| r.as_ref().err().map(|e| (n.clone(), e.clone()))).collect()
    }
}

/// 按依赖顺序求全部名字的值。重名: 参数优先于尺寸(界面不允许建重名参数, 这里只防御)。
pub fn evaluate(inp: &Inputs) -> Eval {
    #[derive(Clone)]
    enum Src<'a> {
        Expr(&'a str),
        Fixed(f64),
    }
    let mut src: BTreeMap<&str, Src> = BTreeMap::new();
    for (n, v, e) in &inp.dims {
        src.insert(n, match e { Some(x) if !x.trim().is_empty() => Src::Expr(x), _ => Src::Fixed(*v) });
    }
    for p in inp.params {
        src.insert(&p.name, Src::Expr(&p.expr));
    }
    let mut done: BTreeMap<String, Result<f64, String>> = BTreeMap::new();
    let mut visiting: Vec<String> = Vec::new();

    fn go<'a>(
        n: &str,
        src: &BTreeMap<&'a str, Src<'a>>,
        done: &mut BTreeMap<String, Result<f64, String>>,
        visiting: &mut Vec<String>,
    ) -> Result<f64, String> {
        if let Some(r) = done.get(n) {
            return r.clone();
        }
        let Some(s) = src.get(n) else { return Err(format!("unknown name {n}")) };
        if let Some(pos) = visiting.iter().position(|x| x == n) {
            let cyc: Vec<String> = visiting[pos..].iter().cloned().chain(std::iter::once(n.to_string())).collect();
            return Err(format!("cycle: {}", cyc.join(" → ")));
        }
        let r = match s {
            Src::Fixed(v) => Ok(*v),
            Src::Expr(e) => {
                visiting.push(n.to_string());
                let e = *e;
                let r = eval_with(e, &mut |m: &str| go(m, src, done, visiting).map_err(|err| if err.starts_with("cycle") || err.starts_with("unknown") { err } else { format!("{m}: {err}") }));
                visiting.pop();
                r
            }
        };
        done.insert(n.to_string(), r.clone());
        r
    }

    let names: Vec<&str> = src.keys().copied().collect();
    for n in names {
        let _ = go(n, &src, &mut done, &mut visiting);
    }
    let mut out = Eval { values: done, targets: BTreeMap::new() };
    for (n, _, e) in &inp.dims {
        if e.is_some_and(|x| !x.trim().is_empty()) {
            out.targets.insert(n.to_string(), out.values.get(*n).cloned().unwrap_or_else(|| Err("not evaluated".into())));
        }
    }
    out
}

/// 新参数的默认名: 常量 C1… / 变量 V1… / 函数 F1…(取不冲突的最小号)。
pub fn default_name(kind: &str, taken: &BTreeSet<String>) -> String {
    let pre = match kind {
        "const" => "C",
        "func" => "F",
        _ => "V",
    };
    (1..).map(|i| format!("{pre}{i}")).find(|n| !taken.contains(n)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, kind: &str, expr: &str) -> Param {
        Param { id: name.to_lowercase(), name: name.into(), kind: kind.into(), expr: expr.into(), note: String::new() }
    }

    #[test]
    fn tokenizer_names_funcs_numbers() {
        assert_eq!(refs("W/2 - T + ROUND(H001.R*2, 1) + 1e3 + 壁厚"), vec!["W", "T", "H001.R", "壁厚"]);
        assert_eq!(refs("\"W\" + X"), vec!["X"], "字符串里的不算");
        assert_eq!(refs("SQRT (A)"), vec!["A"], "函数名后有空格也认");
        assert_eq!(rename_ref("W/2 + Wall + W", "W", "Width"), "Width/2 + Wall + Width");
        assert_eq!(rename_ref("D001+D0010", "D001", "壁厚"), "壁厚+D0010");
        assert!(valid_name("W") && valid_name("壁厚") && valid_name("H001.R") && valid_name("_x1"));
        assert!(!valid_name("1W") && !valid_name("") && !valid_name("a b") && !valid_name("x."));
    }

    #[test]
    fn evaluate_order_dims_and_cycles() {
        let params = vec![p("Half", "func", "W/2 - T"), p("W", "var", "100"), p("T", "const", "3")];
        let inp = Inputs { params: &params, dims: vec![("D001", 12.0, None), ("D002", 40.0, Some("Half + D001")), ("D003", 5.0, Some("T*2"))] };
        let e = evaluate(&inp);
        assert_eq!(e.values["Half"], Ok(47.0), "函数按依赖顺序(写在前面也行)");
        assert_eq!(e.targets["D002"], Ok(59.0), "绑定尺寸可引用参数 + 未绑定尺寸(D001 = 当前值 12)");
        assert_eq!(e.targets["D003"], Ok(6.0));
        let cur: BTreeMap<String, f64> = [("D002".to_string(), 40.0), ("D003".to_string(), 6.0)].into_iter().collect();
        assert_eq!(e.changes(&cur, 5e-4), vec![("D002".to_string(), 59.0)], "只改有差的");
        // 环 + 未知名字 + 语法错 → 报错, 不死循环
        let bad = vec![p("A", "func", "B+1"), p("B", "func", "A*2"), p("U", "func", "nope*2"), p("S", "func", "1+*")];
        let e = evaluate(&Inputs { params: &bad, dims: vec![] });
        assert!(e.values["A"].as_ref().unwrap_err().contains("cycle"), "{:?}", e.values["A"]);
        assert!(e.values["B"].is_err());
        assert!(e.values["U"].as_ref().unwrap_err().contains("unknown name nope"));
        assert!(e.values["S"].is_err());
        assert_eq!(e.errors().len(), 4);
        // 函数可用公式引擎的函数
        let f = vec![p("R", "func", "ROUND(SQRT(2)*10, 1)"), p("M", "func", "MAX(1, R, 3)")];
        let e = evaluate(&Inputs { params: &f, dims: vec![] });
        assert_eq!(e.values["R"], Ok(14.1));
        assert_eq!(e.values["M"], Ok(14.1));
    }

    #[test]
    fn default_names() {
        let taken: BTreeSet<String> = ["V1".to_string(), "C1".to_string()].into_iter().collect();
        assert_eq!(default_name("var", &taken), "V2");
        assert_eq!(default_name("const", &taken), "C2");
        assert_eq!(default_name("func", &taken), "F1");
    }
}
