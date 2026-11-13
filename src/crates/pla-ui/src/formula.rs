//! Self-contained Excel-style formula evaluator for the notes/calculator panel.
//!
//! * [`eval_table`] evaluates a whole grid of raw cell strings (formulas start with `=`),
//!   resolving references between cells with memoization and cycle detection (`#CIRC!`).
//! * [`eval_expr`] evaluates a standalone calculator expression (no cell context).
//!
//! Precedence (lowest → highest): comparisons, `&`, `+ -`, `* /`, `^` (left-assoc, as in Excel),
//! postfix `%`, unary `+ -`, primary (numbers, strings, refs, ranges, calls, parens).
//! Like Excel, unary minus binds tighter than `^`, so `-2^2 = 4`.
//!
//! Error codes: `#DIV/0!`, `#REF!`, `#NAME?`, `#VALUE!`, `#CIRC!`, `#ERROR!` (parse error or
//! wrong argument count) and `#NUM!` (math domain errors / non-finite results).

use std::cmp::Ordering;

/// Evaluated cell value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Text(String),
    Bool(bool),
    Empty,
    /// Excel-like error code, e.g. "#DIV/0!".
    Error(String),
}

impl Value {
    /// Human-readable rendering of the value (what a cell shows).
    pub fn display(&self) -> String {
        match self {
            Value::Num(x) => fmt_num(*x),
            Value::Text(s) => s.clone(),
            Value::Bool(true) => "TRUE".to_string(),
            Value::Bool(false) => "FALSE".to_string(),
            Value::Empty => String::new(),
            Value::Error(code) => code.clone(),
        }
    }
}

const E_DIV0: &str = "#DIV/0!";
const E_REF: &str = "#REF!";
const E_NAME: &str = "#NAME?";
const E_VALUE: &str = "#VALUE!";
const E_CIRC: &str = "#CIRC!";
const E_PARSE: &str = "#ERROR!";
const E_NUM: &str = "#NUM!";
/// Internal placeholder while a deep evaluation is suspended (never escapes `eval_table`).
const E_DEFER: &str = "#DEFER";

/// Max nesting of cell→cell evaluation per pass (keeps native stack use small; deeper
/// chains are resolved incrementally by `eval_table`).
const CELL_DEPTH_LIMIT: usize = 64;
/// Max nesting of parentheses / unary operators in one expression.
const PARSE_DEPTH_LIMIT: usize = 200;
const MAX_ROWS: usize = 1_048_576;

fn err(code: &str) -> Value {
    Value::Error(code.to_string())
}

fn num(x: f64) -> Value {
    if x.is_finite() {
        Value::Num(x)
    } else {
        err(E_NUM)
    }
}

/// Column letters helper: 0 → "A", 25 → "Z", 26 → "AA".
pub fn col_name(c: usize) -> String {
    let mut n = c + 1;
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Number formatting / parsing
// ---------------------------------------------------------------------------

fn trim_frac_zeros(s: &str) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

fn fmt_num(x: f64) -> String {
    if !x.is_finite() {
        return E_NUM.to_string();
    }
    if x == 0.0 {
        return "0".to_string();
    }
    let a = x.abs();
    if !(1e-6..1e15).contains(&a) {
        let s = format!("{:.9e}", x);
        return match s.split_once('e') {
            Some((mant, exp)) => format!("{}e{}", trim_frac_zeros(mant), exp),
            None => s,
        };
    }
    let int_digits = if a >= 1.0 { a.log10().floor() as i32 + 1 } else { 0 };
    let decimals = (15 - int_digits).clamp(0, 10) as usize;
    let s = trim_frac_zeros(&format!("{:.*}", decimals, x));
    if s == "-0" {
        "0".to_string()
    } else {
        s
    }
}

/// Round to 15 significant digits to remove binary floating-point noise.
fn snap(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    format!("{:.14e}", x).parse().unwrap_or(x)
}

/// Parse a literal number: "1,234.5", "50%", " 42 ", "1e3", "-.5". `None` if not numeric.
fn parse_number(s: &str) -> Option<f64> {
    let mut t = s.trim();
    let mut pct = false;
    if let Some(rest) = t.strip_suffix('%') {
        t = rest.trim_end();
        pct = true;
    }
    let (neg, body) = if let Some(r) = t.strip_prefix('-') {
        (true, r)
    } else if let Some(r) = t.strip_prefix('+') {
        (false, r)
    } else {
        (false, t)
    };
    if body.is_empty() {
        return None;
    }
    // Thousands separators: only in the integer part, with valid 3-digit grouping.
    let split = body.find(['.', 'e', 'E']).unwrap_or(body.len());
    let (int_part, rest) = body.split_at(split);
    let cleaned = if int_part.contains(',') {
        if rest.contains(',') {
            return None;
        }
        let groups: Vec<&str> = int_part.split(',').collect();
        let first_ok = (1..=3).contains(&groups[0].len());
        let rest_ok = groups[1..].iter().all(|g| g.len() == 3);
        if !first_ok || !rest_ok {
            return None;
        }
        format!("{}{}", groups.concat(), rest)
    } else {
        body.to_string()
    };
    let first = cleaned.chars().next()?;
    if !(first.is_ascii_digit() || first == '.') {
        return None;
    }
    if !cleaned
        .chars()
        .all(|ch| ch.is_ascii_digit() || matches!(ch, '.' | 'e' | 'E' | '+' | '-'))
    {
        return None;
    }
    let mut v: f64 = cleaned.parse().ok()?;
    if neg {
        v = -v;
    }
    if pct {
        v /= 100.0;
    }
    if v.is_finite() {
        Some(v)
    } else {
        None
    }
}

/// Raw (non-formula) cell literal → value.
fn literal(raw: &str) -> Value {
    let t = raw.trim();
    if t.is_empty() {
        return Value::Empty;
    }
    if let Some(v) = parse_number(t) {
        return Value::Num(v);
    }
    if t.eq_ignore_ascii_case("TRUE") {
        return Value::Bool(true);
    }
    if t.eq_ignore_ascii_case("FALSE") {
        return Value::Bool(false);
    }
    Value::Text(raw.to_string())
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    LParen,
    RParen,
    Comma,
    Colon,
    Op(&'static str),
}

fn lex(src: &str) -> Result<Vec<Tok>, ()> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        let next = chars.get(i + 1).copied();
        if ch.is_ascii_digit() || (ch == '.' && next.is_some_and(|n| n.is_ascii_digit())) {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i < chars.len() && chars[i] == '.' {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            let text: String = chars[start..i].iter().collect();
            toks.push(Tok::Num(text.parse().map_err(|_| ())?));
            continue;
        }
        if ch == '"' {
            i += 1;
            let mut s = String::new();
            loop {
                match chars.get(i) {
                    None => return Err(()),
                    Some('"') => {
                        if chars.get(i + 1) == Some(&'"') {
                            s.push('"');
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    }
                    Some(&c) => {
                        s.push(c);
                        i += 1;
                    }
                }
            }
            toks.push(Tok::Str(s));
            continue;
        }
        if ch.is_alphabetic() || ch == '$' || ch == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '$' || chars[i] == '_')
            {
                i += 1;
            }
            toks.push(Tok::Ident(chars[start..i].iter().collect()));
            continue;
        }
        let tok = match (ch, next) {
            ('(', _) => Tok::LParen,
            (')', _) => Tok::RParen,
            (',', _) | (';', _) => Tok::Comma,
            (':', _) => Tok::Colon,
            ('<', Some('=')) => Tok::Op("<="),
            ('<', Some('>')) => Tok::Op("<>"),
            ('>', Some('=')) => Tok::Op(">="),
            ('+', _) => Tok::Op("+"),
            ('-', _) => Tok::Op("-"),
            ('*', _) => Tok::Op("*"),
            ('/', _) => Tok::Op("/"),
            ('^', _) => Tok::Op("^"),
            ('&', _) => Tok::Op("&"),
            ('%', _) => Tok::Op("%"),
            ('=', _) => Tok::Op("="),
            ('<', _) => Tok::Op("<"),
            ('>', _) => Tok::Op(">"),
            _ => return Err(()),
        };
        i += match &tok {
            Tok::Op(s) => s.len(),
            _ => 1,
        };
        toks.push(tok);
    }
    Ok(toks)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Cat,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Num(f64),
    Str(String),
    Bool(bool),
    Ref(usize, usize),
    /// (r1, c1, r2, c2), normalized so r1 <= r2 and c1 <= c2.
    Range(usize, usize, usize, usize),
    Neg(Box<Expr>),
    Pct(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    /// Identifier that is neither a function call, boolean nor cell ref → `#NAME?`.
    BadName,
}

/// One parsed cell reference including its `$` lock markers.
#[derive(Debug, Clone, Copy, PartialEq)]
struct RefParts {
    row: usize,
    col: usize,
    row_abs: bool,
    col_abs: bool,
}

impl RefParts {
    fn render(self) -> String {
        format!(
            "{}{}{}{}",
            if self.col_abs { "$" } else { "" },
            col_name(self.col),
            if self.row_abs { "$" } else { "" },
            self.row + 1
        )
    }
}

/// Parse "A1" / "$A$1" / "aa10" keeping the `$` flags.
fn parse_ref_parts(s: &str) -> Option<RefParts> {
    let b = s.as_bytes();
    let mut i = 0;
    let col_abs = b.get(i) == Some(&b'$');
    if col_abs {
        i += 1;
    }
    let ls = i;
    while i < b.len() && b[i].is_ascii_alphabetic() {
        i += 1;
    }
    let letters = &s[ls..i];
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let row_abs = b.get(i) == Some(&b'$');
    if row_abs {
        i += 1;
    }
    let digits = &s[i..];
    if digits.is_empty() || digits.len() > 7 || !digits.bytes().all(|d| d.is_ascii_digit()) {
        return None;
    }
    let row: usize = digits.parse().ok()?;
    if row == 0 || row > MAX_ROWS {
        return None;
    }
    let col = letters
        .bytes()
        .fold(0usize, |n, ch| n * 26 + (ch.to_ascii_uppercase() - b'A' + 1) as usize);
    Some(RefParts { row: row - 1, col: col - 1, row_abs, col_abs })
}

/// Parse a single cell ref like "B3" / "$B$3" (case-insensitive) → 0-based (row, col).
pub fn parse_ref(s: &str) -> Option<(usize, usize)> {
    parse_ref_parts(s).map(|p| (p.row, p.col))
}

/// "A1"-style name for 0-based (row, col): ref_name(0,0) == "A1", ref_name(9, 27) == "AB10".
pub fn ref_name(row: usize, col: usize) -> String {
    format!("{}{}", col_name(col), row + 1)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

type PResult = Result<Expr, ()>;

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn peek_op(&self) -> Option<&'static str> {
        match self.peek() {
            Some(Tok::Op(op)) => Some(op),
            _ => None,
        }
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, t: Tok) -> Result<(), ()> {
        if self.next() == Some(t) {
            Ok(())
        } else {
            Err(())
        }
    }

    fn bin(op: BinOp, l: Expr, r: Expr) -> Expr {
        Expr::Bin(op, Box::new(l), Box::new(r))
    }

    fn parse_cmp(&mut self) -> PResult {
        let mut l = self.parse_cat()?;
        loop {
            let op = match self.peek_op() {
                Some("=") => BinOp::Eq,
                Some("<>") => BinOp::Ne,
                Some("<") => BinOp::Lt,
                Some(">") => BinOp::Gt,
                Some("<=") => BinOp::Le,
                Some(">=") => BinOp::Ge,
                _ => return Ok(l),
            };
            self.pos += 1;
            let r = self.parse_cat()?;
            l = Self::bin(op, l, r);
        }
    }

    fn parse_cat(&mut self) -> PResult {
        let mut l = self.parse_add()?;
        while self.peek_op() == Some("&") {
            self.pos += 1;
            let r = self.parse_add()?;
            l = Self::bin(BinOp::Cat, l, r);
        }
        Ok(l)
    }

    fn parse_add(&mut self) -> PResult {
        let mut l = self.parse_mul()?;
        loop {
            let op = match self.peek_op() {
                Some("+") => BinOp::Add,
                Some("-") => BinOp::Sub,
                _ => return Ok(l),
            };
            self.pos += 1;
            let r = self.parse_mul()?;
            l = Self::bin(op, l, r);
        }
    }

    fn parse_mul(&mut self) -> PResult {
        let mut l = self.parse_pow()?;
        loop {
            let op = match self.peek_op() {
                Some("*") => BinOp::Mul,
                Some("/") => BinOp::Div,
                _ => return Ok(l),
            };
            self.pos += 1;
            let r = self.parse_pow()?;
            l = Self::bin(op, l, r);
        }
    }

    /// `^` is left-associative like Excel: 2^3^2 = (2^3)^2 = 64.
    fn parse_pow(&mut self) -> PResult {
        let mut base = self.parse_pct()?;
        while self.peek_op() == Some("^") {
            self.pos += 1;
            let exp = self.parse_pct()?;
            base = Self::bin(BinOp::Pow, base, exp);
        }
        Ok(base)
    }

    fn parse_pct(&mut self) -> PResult {
        let mut e = self.parse_unary()?;
        while self.peek_op() == Some("%") {
            self.pos += 1;
            e = Expr::Pct(Box::new(e));
        }
        Ok(e)
    }

    fn enter(&mut self) -> Result<(), ()> {
        self.depth += 1;
        if self.depth > PARSE_DEPTH_LIMIT {
            Err(())
        } else {
            Ok(())
        }
    }

    fn parse_unary(&mut self) -> PResult {
        match self.peek_op() {
            Some(op @ ("-" | "+")) => {
                self.pos += 1;
                self.enter()?;
                let inner = self.parse_unary();
                self.depth -= 1;
                let inner = inner?;
                Ok(if op == "-" { Expr::Neg(Box::new(inner)) } else { inner })
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> PResult {
        match self.next().ok_or(())? {
            Tok::Num(v) => Ok(Expr::Num(v)),
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::LParen => {
                self.enter()?;
                let e = self.parse_cmp();
                self.depth -= 1;
                let e = e?;
                self.expect(Tok::RParen)?;
                Ok(e)
            }
            Tok::Ident(name) => {
                if self.peek() == Some(&Tok::LParen) {
                    self.pos += 1;
                    self.enter()?;
                    let args = self.parse_args();
                    self.depth -= 1;
                    return Ok(Expr::Call(name.to_ascii_uppercase(), args?));
                }
                if name.eq_ignore_ascii_case("TRUE") {
                    return Ok(Expr::Bool(true));
                }
                if name.eq_ignore_ascii_case("FALSE") {
                    return Ok(Expr::Bool(false));
                }
                match parse_ref(&name) {
                    Some((r1, c1)) => {
                        if self.peek() == Some(&Tok::Colon) {
                            self.pos += 1;
                            let (r2, c2) = match self.next() {
                                Some(Tok::Ident(n2)) => parse_ref(&n2).ok_or(())?,
                                _ => return Err(()),
                            };
                            Ok(Expr::Range(r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2)))
                        } else {
                            Ok(Expr::Ref(r1, c1))
                        }
                    }
                    None => Ok(Expr::BadName),
                }
            }
            _ => Err(()),
        }
    }

    /// After '(' of a call: comma-separated args until ')'.
    fn parse_args(&mut self) -> Result<Vec<Expr>, ()> {
        let mut args = Vec::new();
        if self.peek() == Some(&Tok::RParen) {
            self.pos += 1;
            return Ok(args);
        }
        loop {
            args.push(self.parse_cmp()?);
            match self.next() {
                Some(Tok::Comma) => continue,
                Some(Tok::RParen) => return Ok(args),
                _ => return Err(()),
            }
        }
    }
}

fn parse(src: &str) -> PResult {
    let toks = lex(src)?;
    if toks.is_empty() {
        return Err(());
    }
    let mut p = Parser { toks, pos: 0, depth: 0 };
    let e = p.parse_cmp()?;
    if p.pos != p.toks.len() {
        return Err(());
    }
    Ok(e)
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// Cell access for the evaluator. `depth` is the cell-nesting depth of the caller.
trait Ctx {
    fn cell(&mut self, r: usize, c: usize, depth: usize) -> Value;
    /// All values of a range (row-major), or an error (e.g. `#REF!`).
    fn range(&mut self, r1: usize, c1: usize, r2: usize, c2: usize, depth: usize)
        -> Result<Vec<Value>, Value>;
}

/// Context for standalone expressions: every reference is `#REF!`.
struct NoCells;

impl Ctx for NoCells {
    fn cell(&mut self, _: usize, _: usize, _: usize) -> Value {
        err(E_REF)
    }
    fn range(&mut self, _: usize, _: usize, _: usize, _: usize, _: usize) -> Result<Vec<Value>, Value> {
        Err(err(E_REF))
    }
}

fn to_num(v: &Value) -> Result<f64, Value> {
    match v {
        Value::Num(x) => Ok(*x),
        Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        Value::Empty => Ok(0.0),
        Value::Text(s) => parse_number(s).ok_or_else(|| err(E_VALUE)),
        Value::Error(_) => Err(v.clone()),
    }
}

fn to_text(v: &Value) -> Result<String, Value> {
    match v {
        Value::Error(_) => Err(v.clone()),
        other => Ok(other.display()),
    }
}

fn to_bool(v: &Value) -> Result<bool, Value> {
    match v {
        Value::Num(x) => Ok(*x != 0.0),
        Value::Bool(b) => Ok(*b),
        Value::Empty => Ok(false),
        Value::Text(s) if s.trim().eq_ignore_ascii_case("TRUE") => Ok(true),
        Value::Text(s) if s.trim().eq_ignore_ascii_case("FALSE") => Ok(false),
        Value::Text(_) => Err(err(E_VALUE)),
        Value::Error(_) => Err(v.clone()),
    }
}

/// Excel ordering: numbers < text < booleans; text compares case-insensitively;
/// an empty cell takes the type of the other side (0, "", FALSE).
fn compare(a: &Value, b: &Value) -> Result<Ordering, Value> {
    if let Value::Error(_) = a {
        return Err(a.clone());
    }
    if let Value::Error(_) = b {
        return Err(b.clone());
    }
    fn blank_like(other: &Value) -> Value {
        match other {
            Value::Text(_) => Value::Text(String::new()),
            Value::Bool(_) => Value::Bool(false),
            _ => Value::Num(0.0),
        }
    }
    let a2 = if *a == Value::Empty { blank_like(b) } else { a.clone() };
    let b2 = if *b == Value::Empty { blank_like(a) } else { b.clone() };
    let rank = |v: &Value| match v {
        Value::Num(_) => 0,
        Value::Text(_) => 1,
        _ => 2,
    };
    Ok(match (&a2, &b2) {
        (Value::Num(x), Value::Num(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Text(x), Value::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => rank(&a2).cmp(&rank(&b2)),
    })
}

fn eval(e: &Expr, ctx: &mut dyn Ctx, depth: usize) -> Value {
    match e {
        Expr::Num(x) => num(*x),
        Expr::Str(s) => Value::Text(s.clone()),
        Expr::Bool(b) => Value::Bool(*b),
        Expr::Ref(r, c) => ctx.cell(*r, *c, depth),
        // A bare range is only meaningful as a function argument.
        Expr::Range(..) => err(E_VALUE),
        Expr::BadName => err(E_NAME),
        Expr::Neg(inner) => match to_num(&eval(inner, ctx, depth)) {
            Ok(x) => num(-x),
            Err(e) => e,
        },
        Expr::Pct(inner) => match to_num(&eval(inner, ctx, depth)) {
            Ok(x) => num(x / 100.0),
            Err(e) => e,
        },
        Expr::Bin(op, l, r) => {
            let lv = eval(l, ctx, depth);
            if let Value::Error(_) = lv {
                return lv;
            }
            let rv = eval(r, ctx, depth);
            if let Value::Error(_) = rv {
                return rv;
            }
            binop(*op, &lv, &rv).unwrap_or_else(|e| e)
        }
        Expr::Call(name, args) => call(name, args, ctx, depth).unwrap_or_else(|e| e),
    }
}

fn binop(op: BinOp, a: &Value, b: &Value) -> Result<Value, Value> {
    let cmp = |want: &dyn Fn(Ordering) -> bool| -> Result<Value, Value> {
        Ok(Value::Bool(want(compare(a, b)?)))
    };
    match op {
        BinOp::Cat => Ok(Value::Text(to_text(a)? + &to_text(b)?)),
        BinOp::Eq => cmp(&|o| o == Ordering::Equal),
        BinOp::Ne => cmp(&|o| o != Ordering::Equal),
        BinOp::Lt => cmp(&|o| o == Ordering::Less),
        BinOp::Gt => cmp(&|o| o == Ordering::Greater),
        BinOp::Le => cmp(&|o| o != Ordering::Greater),
        BinOp::Ge => cmp(&|o| o != Ordering::Less),
        _ => {
            let x = to_num(a)?;
            let y = to_num(b)?;
            Ok(match op {
                BinOp::Add => num(x + y),
                BinOp::Sub => num(x - y),
                BinOp::Mul => num(x * y),
                BinOp::Div => {
                    if y == 0.0 {
                        err(E_DIV0)
                    } else {
                        num(x / y)
                    }
                }
                _ => power(x, y),
            })
        }
    }
}

fn power(x: f64, y: f64) -> Value {
    if x == 0.0 && y == 0.0 {
        err(E_NUM)
    } else if x == 0.0 && y < 0.0 {
        err(E_DIV0)
    } else {
        num(x.powf(y))
    }
}

/// A function argument as seen by aggregate functions.
enum Arg {
    /// Values from a range or a direct reference (text/empty are ignored by aggregates).
    Cells(Vec<Value>),
    /// A directly computed value (non-numeric text → `#VALUE!`).
    Scalar(Value),
}

fn eval_arg(e: &Expr, ctx: &mut dyn Ctx, depth: usize) -> Arg {
    match e {
        Expr::Range(r1, c1, r2, c2) => match ctx.range(*r1, *c1, *r2, *c2, depth) {
            Ok(vals) => Arg::Cells(vals),
            Err(e) => Arg::Scalar(e),
        },
        Expr::Ref(r, c) => Arg::Cells(vec![ctx.cell(*r, *c, depth)]),
        other => Arg::Scalar(eval(other, ctx, depth)),
    }
}

fn eval_args(args: &[Expr], ctx: &mut dyn Ctx, depth: usize) -> Vec<Arg> {
    args.iter().map(|a| eval_arg(a, ctx, depth)).collect()
}

fn collect_nums(args: &[Arg]) -> Result<Vec<f64>, Value> {
    let mut out = Vec::new();
    for a in args {
        match a {
            Arg::Cells(vals) => {
                for v in vals {
                    match v {
                        Value::Num(x) => out.push(*x),
                        Value::Error(_) => return Err(v.clone()),
                        _ => {}
                    }
                }
            }
            Arg::Scalar(Value::Empty) => {}
            Arg::Scalar(v) => out.push(to_num(v)?),
        }
    }
    Ok(out)
}

fn collect_bools(args: &[Arg]) -> Result<Vec<bool>, Value> {
    let mut out = Vec::new();
    for a in args {
        match a {
            Arg::Cells(vals) => {
                for v in vals {
                    match v {
                        Value::Num(x) => out.push(*x != 0.0),
                        Value::Bool(b) => out.push(*b),
                        Value::Error(_) => return Err(v.clone()),
                        _ => {}
                    }
                }
            }
            Arg::Scalar(Value::Empty) => {}
            Arg::Scalar(v) => out.push(to_bool(v)?),
        }
    }
    if out.is_empty() {
        Err(err(E_VALUE))
    } else {
        Ok(out)
    }
}

#[derive(Clone, Copy)]
enum RoundMode {
    Nearest,
    Up,
    Down,
}

fn round_to(x: f64, digits: f64, mode: RoundMode) -> f64 {
    let n = digits.trunc().clamp(-300.0, 300.0) as i32;
    let p = 10f64.powi(n.abs());
    let y = snap(if n >= 0 { x * p } else { x / p });
    let r = match mode {
        RoundMode::Nearest => y.round(),
        RoundMode::Up => {
            if y >= 0.0 {
                y.ceil()
            } else {
                y.floor()
            }
        }
        RoundMode::Down => y.trunc(),
    };
    if n >= 0 {
        r / p
    } else {
        r * p
    }
}

fn arity(args: &[Expr], min: usize, max: usize) -> Result<(), Value> {
    if args.len() < min || args.len() > max {
        Err(err(E_PARSE))
    } else {
        Ok(())
    }
}

/// Evaluate an argument in scalar context (ranges → `#VALUE!`).
fn scalar(e: &Expr, ctx: &mut dyn Ctx, depth: usize) -> Value {
    eval(e, ctx, depth)
}

fn num_arg(e: &Expr, ctx: &mut dyn Ctx, depth: usize) -> Result<f64, Value> {
    to_num(&scalar(e, ctx, depth))
}

fn text_arg(e: &Expr, ctx: &mut dyn Ctx, depth: usize) -> Result<String, Value> {
    to_text(&scalar(e, ctx, depth))
}

/// One-argument numeric function with an optional domain check.
fn unary_math(
    args: &[Expr],
    ctx: &mut dyn Ctx,
    depth: usize,
    f: fn(f64) -> f64,
    domain: fn(f64) -> bool,
) -> Result<Value, Value> {
    arity(args, 1, 1)?;
    let x = num_arg(&args[0], ctx, depth)?;
    if !domain(x) {
        return Err(err(E_NUM));
    }
    Ok(num(f(x)))
}

fn any(_: f64) -> bool {
    true
}

fn call(name: &str, args: &[Expr], ctx: &mut dyn Ctx, depth: usize) -> Result<Value, Value> {
    match name {
        "SUM" => Ok(num(collect_nums(&eval_args(args, ctx, depth))?.iter().sum())),
        "PRODUCT" => {
            let v = collect_nums(&eval_args(args, ctx, depth))?;
            Ok(num(if v.is_empty() { 0.0 } else { v.iter().product() }))
        }
        "AVERAGE" => {
            let v = collect_nums(&eval_args(args, ctx, depth))?;
            if v.is_empty() {
                return Err(err(E_DIV0));
            }
            Ok(num(v.iter().sum::<f64>() / v.len() as f64))
        }
        "MIN" | "MAX" => {
            let v = collect_nums(&eval_args(args, ctx, depth))?;
            if v.is_empty() {
                return Ok(Value::Num(0.0));
            }
            let pick = if name == "MIN" { f64::min } else { f64::max };
            Ok(num(v.iter().copied().fold(v[0], pick)))
        }
        "MEDIAN" => {
            let mut v = collect_nums(&eval_args(args, ctx, depth))?;
            if v.is_empty() {
                return Err(err(E_NUM));
            }
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let m = v.len() / 2;
            Ok(num(if v.len() % 2 == 1 { v[m] } else { (v[m - 1] + v[m]) / 2.0 }))
        }
        "COUNT" => {
            let mut n = 0usize;
            for a in eval_args(args, ctx, depth) {
                match a {
                    Arg::Cells(vals) => n += vals.iter().filter(|v| matches!(v, Value::Num(_))).count(),
                    Arg::Scalar(Value::Num(_)) | Arg::Scalar(Value::Bool(_)) => n += 1,
                    Arg::Scalar(Value::Text(s)) if parse_number(&s).is_some() => n += 1,
                    Arg::Scalar(_) => {}
                }
            }
            Ok(Value::Num(n as f64))
        }
        "COUNTA" => {
            let mut n = 0usize;
            for a in eval_args(args, ctx, depth) {
                match a {
                    Arg::Cells(vals) => n += vals.iter().filter(|v| **v != Value::Empty).count(),
                    Arg::Scalar(Value::Empty) => {}
                    Arg::Scalar(_) => n += 1,
                }
            }
            Ok(Value::Num(n as f64))
        }
        "AND" => Ok(Value::Bool(collect_bools(&eval_args(args, ctx, depth))?.iter().all(|b| *b))),
        "OR" => Ok(Value::Bool(collect_bools(&eval_args(args, ctx, depth))?.iter().any(|b| *b))),
        "NOT" => {
            arity(args, 1, 1)?;
            Ok(Value::Bool(!to_bool(&scalar(&args[0], ctx, depth))?))
        }
        "IF" => {
            arity(args, 2, 3)?;
            let cond = to_bool(&scalar(&args[0], ctx, depth))?;
            if cond {
                Ok(scalar(&args[1], ctx, depth))
            } else if let Some(b) = args.get(2) {
                Ok(scalar(b, ctx, depth))
            } else {
                Ok(Value::Bool(false))
            }
        }
        "ROUND" | "ROUNDUP" | "ROUNDDOWN" => {
            arity(args, 1, 2)?;
            let x = num_arg(&args[0], ctx, depth)?;
            let n = match args.get(1) {
                Some(a) => num_arg(a, ctx, depth)?,
                None => 0.0,
            };
            let mode = match name {
                "ROUND" => RoundMode::Nearest,
                "ROUNDUP" => RoundMode::Up,
                _ => RoundMode::Down,
            };
            Ok(num(round_to(x, n, mode)))
        }
        "INT" => unary_math(args, ctx, depth, f64::floor, any),
        "ABS" => unary_math(args, ctx, depth, f64::abs, any),
        "SQRT" => unary_math(args, ctx, depth, f64::sqrt, |x| x >= 0.0),
        "SIN" => unary_math(args, ctx, depth, f64::sin, any),
        "COS" => unary_math(args, ctx, depth, f64::cos, any),
        "TAN" => unary_math(args, ctx, depth, f64::tan, any),
        "ASIN" => unary_math(args, ctx, depth, f64::asin, |x| (-1.0..=1.0).contains(&x)),
        "ACOS" => unary_math(args, ctx, depth, f64::acos, |x| (-1.0..=1.0).contains(&x)),
        "ATAN" => unary_math(args, ctx, depth, f64::atan, any),
        "DEGREES" => unary_math(args, ctx, depth, f64::to_degrees, any),
        "RADIANS" => unary_math(args, ctx, depth, f64::to_radians, any),
        "EXP" => unary_math(args, ctx, depth, f64::exp, any),
        "LN" => unary_math(args, ctx, depth, f64::ln, |x| x > 0.0),
        "LOG10" => unary_math(args, ctx, depth, f64::log10, |x| x > 0.0),
        "LOG" => {
            arity(args, 1, 2)?;
            let x = num_arg(&args[0], ctx, depth)?;
            let base = match args.get(1) {
                Some(a) => num_arg(a, ctx, depth)?,
                None => 10.0,
            };
            if x <= 0.0 || base <= 0.0 {
                return Err(err(E_NUM));
            }
            if base == 1.0 {
                return Err(err(E_DIV0));
            }
            Ok(num(snap(x.ln() / base.ln())))
        }
        "ATAN2" => {
            arity(args, 2, 2)?;
            let x = num_arg(&args[0], ctx, depth)?;
            let y = num_arg(&args[1], ctx, depth)?;
            if x == 0.0 && y == 0.0 {
                return Err(err(E_DIV0));
            }
            Ok(num(y.atan2(x)))
        }
        "POWER" => {
            arity(args, 2, 2)?;
            let x = num_arg(&args[0], ctx, depth)?;
            let y = num_arg(&args[1], ctx, depth)?;
            Ok(power(x, y))
        }
        "MOD" => {
            arity(args, 2, 2)?;
            let a = num_arg(&args[0], ctx, depth)?;
            let b = num_arg(&args[1], ctx, depth)?;
            if b == 0.0 {
                return Err(err(E_DIV0));
            }
            Ok(num(snap(a - b * (a / b).floor())))
        }
        "PI" => {
            arity(args, 0, 0)?;
            Ok(Value::Num(std::f64::consts::PI))
        }
        "CEILING" | "FLOOR" => {
            arity(args, 1, 2)?;
            let x = num_arg(&args[0], ctx, depth)?;
            let sig = match args.get(1) {
                Some(a) => num_arg(a, ctx, depth)?,
                None => 1.0,
            };
            if sig == 0.0 {
                return Ok(Value::Num(0.0));
            }
            if x > 0.0 && sig < 0.0 {
                return Err(err(E_NUM));
            }
            let q = snap(x / sig);
            let q = if name == "CEILING" { q.ceil() } else { q.floor() };
            Ok(num(snap(q * sig)))
        }
        "CONCAT" | "CONCATENATE" => {
            let mut s = String::new();
            for a in eval_args(args, ctx, depth) {
                match a {
                    Arg::Cells(vals) => {
                        for v in &vals {
                            s.push_str(&to_text(v)?);
                        }
                    }
                    Arg::Scalar(v) => s.push_str(&to_text(&v)?),
                }
            }
            Ok(Value::Text(s))
        }
        "LEN" => {
            arity(args, 1, 1)?;
            Ok(Value::Num(text_arg(&args[0], ctx, depth)?.chars().count() as f64))
        }
        "UPPER" => {
            arity(args, 1, 1)?;
            Ok(Value::Text(text_arg(&args[0], ctx, depth)?.to_uppercase()))
        }
        "LOWER" => {
            arity(args, 1, 1)?;
            Ok(Value::Text(text_arg(&args[0], ctx, depth)?.to_lowercase()))
        }
        _ => Err(err(E_NAME)),
    }
}

// ---------------------------------------------------------------------------
// Table evaluation
// ---------------------------------------------------------------------------

enum State {
    Todo,
    Busy,
    Done(Value),
}

struct Table<'a> {
    cells: &'a [Vec<String>],
    rows: usize,
    cols: usize,
    state: Vec<Vec<State>>,
    /// Set when the depth limit suspended evaluation at this cell; everything on the
    /// current evaluation stack unwinds without memoizing and is retried later.
    deferred: Option<(usize, usize)>,
}

impl Table<'_> {
    fn compute(&mut self, r: usize, c: usize, depth: usize) -> Value {
        let cells = self.cells;
        let raw = &cells[r][c];
        match raw.trim_start().strip_prefix('=') {
            Some(src) => match parse(src) {
                Ok(expr) => match eval(&expr, self, depth + 1) {
                    Value::Empty => Value::Num(0.0),
                    v => v,
                },
                Err(()) => err(E_PARSE),
            },
            None => literal(raw),
        }
    }
}

impl Ctx for Table<'_> {
    fn cell(&mut self, r: usize, c: usize, depth: usize) -> Value {
        if r >= self.rows || c >= self.cols {
            return err(E_REF);
        }
        if c >= self.cells[r].len() {
            return Value::Empty;
        }
        match &self.state[r][c] {
            State::Done(v) => return v.clone(),
            State::Busy => return err(E_CIRC),
            State::Todo => {}
        }
        if self.deferred.is_some() {
            return err(E_DEFER);
        }
        if depth > CELL_DEPTH_LIMIT {
            self.deferred = Some((r, c));
            return err(E_DEFER);
        }
        self.state[r][c] = State::Busy;
        let v = self.compute(r, c, depth);
        if self.deferred.is_some() {
            self.state[r][c] = State::Todo;
            return err(E_DEFER);
        }
        self.state[r][c] = State::Done(v.clone());
        v
    }

    fn range(&mut self, r1: usize, c1: usize, r2: usize, c2: usize, depth: usize)
        -> Result<Vec<Value>, Value> {
        if r2 >= self.rows || c2 >= self.cols {
            return Err(err(E_REF));
        }
        let mut out = Vec::with_capacity((r2 - r1 + 1) * (c2 - c1 + 1));
        for r in r1..=r2 {
            for c in c1..=c2 {
                out.push(self.cell(r, c, depth));
            }
        }
        Ok(out)
    }
}

/// Evaluate a whole table. `cells[row][col]` are raw cell strings (rows may have different lengths).
/// A cell starting with '=' is a formula; otherwise a number if it parses (allow "1,234.5",
/// "50%" → 0.5, leading/trailing spaces), else text (empty string → Empty).
/// Returns the same shape of evaluated values. Handles references between cells, detects
/// cycles (→ "#CIRC!"), memoized.
pub fn eval_table(cells: &[Vec<String>]) -> Vec<Vec<Value>> {
    let rows = cells.len();
    let cols = cells.iter().map(Vec::len).max().unwrap_or(0);
    let mut t = Table {
        cells,
        rows,
        cols,
        state: cells.iter().map(|row| row.iter().map(|_| State::Todo).collect()).collect(),
        deferred: None,
    };
    for r in 0..rows {
        for c in 0..cells[r].len() {
            // Work stack: when a deep chain is suspended at cell `d`, evaluate `d` first
            // (from depth 0), then retry the cells waiting on it.
            let mut stack = vec![(r, c)];
            while let Some(&(tr, tc)) = stack.last() {
                t.deferred = None;
                t.cell(tr, tc, 0);
                match t.deferred.take() {
                    None => {
                        stack.pop();
                    }
                    Some(d) if stack.contains(&d) => {
                        // `d` (transitively) depends on itself: a long cycle.
                        t.state[d.0][d.1] = State::Done(err(E_CIRC));
                    }
                    Some(d) => stack.push(d),
                }
            }
        }
    }
    t.state
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|s| match s {
                    State::Done(v) => v,
                    _ => err(E_CIRC),
                })
                .collect()
        })
        .collect()
}

/// Evaluate a standalone expression with no cell context (calculator lines like
/// "=12*3+sqrt(2)"; the leading '=' is optional). Cell refs → "#REF!".
pub fn eval_expr(src: &str) -> Value {
    let s = src.trim();
    let s = s.strip_prefix('=').unwrap_or(s);
    match parse(s) {
        Ok(expr) => match eval(&expr, &mut NoCells, 0) {
            Value::Empty => Value::Num(0.0),
            v => v,
        },
        Err(()) => err(E_PARSE),
    }
}

// ---------------------------------------------------------------------------
// Reference rewriting (fill / copy / insert / delete)
// ---------------------------------------------------------------------------

/// Columns representable by `parse_ref` (up to "ZZZ").
const MAX_COLS: usize = 26 + 26 * 26 + 26 * 26 * 26;

/// A reference token: byte span within the formula body plus its parsed parts.
#[derive(Debug, Clone, Copy)]
struct RefTok {
    start: usize,
    end: usize,
    parts: RefParts,
}

enum RefItem {
    Single(RefTok),
    Range(RefTok, RefTok),
}

enum LexItem {
    Ref(RefTok),
    Colon,
    Other,
}

/// Find every cell reference / range in a formula body (text after '='), mirroring `lex`:
/// string literals are skipped, numbers are consumed whole, and an identifier followed by
/// `(` (e.g. `LOG10(`) is a function call, not a reference.
fn scan_refs(body: &str) -> Vec<RefItem> {
    let cs: Vec<(usize, char)> = body.char_indices().collect();
    let at = |k: usize| cs.get(k).map(|&(_, c)| c);
    let off = |k: usize| cs.get(k).map_or(body.len(), |&(b, _)| b);
    let mut items = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let ch = cs[i].1;
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '"' {
            i += 1;
            while i < cs.len() {
                if cs[i].1 == '"' {
                    if at(i + 1) == Some('"') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            items.push(LexItem::Other);
            continue;
        }
        let digit = |k: usize| at(k).is_some_and(|c| c.is_ascii_digit());
        if ch.is_ascii_digit() || (ch == '.' && digit(i + 1)) {
            while digit(i) {
                i += 1;
            }
            if at(i) == Some('.') {
                i += 1;
                while digit(i) {
                    i += 1;
                }
            }
            if matches!(at(i), Some('e' | 'E')) {
                let mut j = i + 1;
                if matches!(at(j), Some('+' | '-')) {
                    j += 1;
                }
                if digit(j) {
                    while digit(j) {
                        j += 1;
                    }
                    i = j;
                }
            }
            items.push(LexItem::Other);
            continue;
        }
        if ch.is_alphabetic() || ch == '$' || ch == '_' {
            let start = i;
            while at(i).is_some_and(|c| c.is_alphanumeric() || c == '$' || c == '_') {
                i += 1;
            }
            let mut j = i;
            while at(j).is_some_and(char::is_whitespace) {
                j += 1;
            }
            let is_call = at(j) == Some('(');
            let (s, e) = (off(start), off(i));
            let parts = if is_call { None } else { parse_ref_parts(&body[s..e]) };
            items.push(match parts {
                Some(parts) => LexItem::Ref(RefTok { start: s, end: e, parts }),
                None => LexItem::Other,
            });
            continue;
        }
        items.push(if ch == ':' { LexItem::Colon } else { LexItem::Other });
        i += 1;
    }
    let mut out = Vec::new();
    let mut k = 0;
    while k < items.len() {
        if let LexItem::Ref(a) = items[k] {
            if let (Some(LexItem::Colon), Some(LexItem::Ref(b))) = (items.get(k + 1), items.get(k + 2)) {
                out.push(RefItem::Range(a, *b));
                k += 3;
                continue;
            }
            out.push(RefItem::Single(a));
        }
        k += 1;
    }
    out
}

fn push_rep(reps: &mut Vec<(usize, usize, String)>, t: &RefTok, new: RefParts) {
    if new != t.parts {
        reps.push((t.start, t.end, new.render()));
    }
}

/// Rewrite the reference tokens of a formula in place. `single` maps a lone ref and `range`
/// maps both endpoints of a range; `None` turns the whole token (or whole range) into `#REF!`.
/// Unchanged refs keep their original text (case included); non-formulas are returned as is.
fn rewrite(
    src: &str,
    single: impl Fn(RefParts) -> Option<RefParts>,
    range: impl Fn(RefParts, RefParts) -> Option<(RefParts, RefParts)>,
) -> String {
    let lead = src.len() - src.trim_start().len();
    if !src[lead..].starts_with('=') {
        return src.to_string();
    }
    let base = lead + 1;
    let body = &src[base..];
    let mut reps: Vec<(usize, usize, String)> = Vec::new();
    for item in scan_refs(body) {
        match item {
            RefItem::Single(t) => match single(t.parts) {
                Some(p) => push_rep(&mut reps, &t, p),
                None => reps.push((t.start, t.end, E_REF.to_string())),
            },
            RefItem::Range(a, b) => match range(a.parts, b.parts) {
                Some((pa, pb)) => {
                    push_rep(&mut reps, &a, pa);
                    push_rep(&mut reps, &b, pb);
                }
                None => reps.push((a.start, b.end, E_REF.to_string())),
            },
        }
    }
    let mut out = String::with_capacity(src.len() + 8);
    out.push_str(&src[..base]);
    let mut last = 0;
    for (s, e, text) in reps {
        out.push_str(&body[last..s]);
        out.push_str(&text);
        last = e;
    }
    out.push_str(&body[last..]);
    out
}

fn offset(v: usize, d: i64, limit: usize) -> Option<usize> {
    let n = i64::try_from(v).ok()?.checked_add(d)?;
    let n = usize::try_from(n).ok()?;
    (n < limit).then_some(n)
}

fn shift_ref(p: RefParts, d_row: i64, d_col: i64) -> Option<RefParts> {
    Some(RefParts {
        row: if p.row_abs { p.row } else { offset(p.row, d_row, MAX_ROWS)? },
        col: if p.col_abs { p.col } else { offset(p.col, d_col, MAX_COLS)? },
        ..p
    })
}

/// Copy/fill semantics: move a formula by (d_row, d_col). Relative row/col parts shift, `$`-locked parts don't.
/// Non-formula input (not starting with '=') is returned unchanged. A ref that would move to row<0 or col<0
/// becomes `#REF!` in the text (a range with such an endpoint becomes a single `#REF!`). Everything else is
/// preserved verbatim; only reference tokens are rewritten in place (string literals are never touched).
pub fn shift_formula(src: &str, d_row: i64, d_col: i64) -> String {
    let one = |p: RefParts| shift_ref(p, d_row, d_col);
    rewrite(src, one, |a, b| Some((one(a)?, one(b)?)))
}

#[derive(Clone, Copy)]
enum Axis {
    Row,
    Col,
}

impl Axis {
    fn get(self, p: RefParts) -> usize {
        match self {
            Axis::Row => p.row,
            Axis::Col => p.col,
        }
    }
    fn with(self, p: RefParts, v: usize) -> RefParts {
        match self {
            Axis::Row => RefParts { row: v, ..p },
            Axis::Col => RefParts { col: v, ..p },
        }
    }
    fn limit(self) -> usize {
        match self {
            Axis::Row => MAX_ROWS,
            Axis::Col => MAX_COLS,
        }
    }
}

fn insert_axis(src: &str, axis: Axis, at: usize, n: usize) -> String {
    let mv = |p: RefParts| {
        let v = axis.get(p);
        if v < at {
            return Some(p);
        }
        v.checked_add(n).filter(|&v| v < axis.limit()).map(|v| axis.with(p, v))
    };
    rewrite(src, mv, |a, b| Some((mv(a)?, mv(b)?)))
}

fn delete_axis(src: &str, axis: Axis, at: usize, n: usize) -> String {
    let end = at.saturating_add(n);
    let single = |p: RefParts| {
        let v = axis.get(p);
        if v < at {
            Some(p)
        } else if v < end {
            None
        } else {
            Some(axis.with(p, v - n))
        }
    };
    let range = |a: RefParts, b: RefParts| {
        let (va, vb) = (axis.get(a), axis.get(b));
        let (lo, hi) = (va.min(vb), va.max(vb));
        if hi < at {
            return Some((a, b));
        }
        if lo >= end {
            return Some((axis.with(a, va - n), axis.with(b, vb - n)));
        }
        if lo >= at && hi < end {
            return None;
        }
        // Partial overlap: clip to the surviving cells.
        let nlo = lo.min(at);
        let nhi = if hi >= end { hi - n } else { at - 1 };
        let (na, nb) = if va <= vb { (nlo, nhi) } else { (nhi, nlo) };
        Some((axis.with(a, na), axis.with(b, nb)))
    };
    rewrite(src, single, range)
}

/// Structural edits (Excel semantics: BOTH relative and absolute refs are adjusted, `$` markers preserved):
/// inserting `n` rows before 0-based row `at` moves refs with row >= at down by n; ranges spanning the insertion point grow.
pub fn insert_rows(src: &str, at: usize, n: usize) -> String {
    insert_axis(src, Axis::Row, at, n)
}

/// Column counterpart of [`insert_rows`].
pub fn insert_cols(src: &str, at: usize, n: usize) -> String {
    insert_axis(src, Axis::Col, at, n)
}

/// Deleting rows at..at+n: refs below move up; a single ref inside the deleted band → `#REF!`;
/// a range partially covering the band shrinks; a range entirely inside → `#REF!`.
pub fn delete_rows(src: &str, at: usize, n: usize) -> String {
    delete_axis(src, Axis::Row, at, n)
}

/// Column counterpart of [`delete_rows`].
pub fn delete_cols(src: &str, at: usize, n: usize) -> String {
    delete_axis(src, Axis::Col, at, n)
}

/// R31b: index after moving the block `from..from+n` so it sits before original index `to`
/// (`to <= from` = move up/left, `to >= from + n` = move down/right; anything else = no-op).
pub fn move_index(v: usize, from: usize, n: usize, to: usize) -> usize {
    let end = from + n;
    if to <= from {
        if v >= from && v < end {
            to + (v - from)
        } else if v >= to && v < from {
            v + n
        } else {
            v
        }
    } else if to >= end {
        if v >= from && v < end {
            to - n + (v - from)
        } else if v >= end && v < to {
            v - n
        } else {
            v
        }
    } else {
        v
    }
}

fn move_axis(src: &str, axis: Axis, from: usize, n: usize, to: usize) -> String {
    let m = |p: RefParts| Some(axis.with(p, move_index(axis.get(p), from, n, to)));
    rewrite(src, m, |a, b| {
        let (ma, mb) = (m(a)?, m(b)?);
        let (va, vb) = (axis.get(ma), axis.get(mb));
        // keep the range in lo..hi order (each endpoint keeps its own `$` flags)
        Some(if va <= vb { (ma, mb) } else { (axis.with(ma, vb), axis.with(mb, va)) })
    })
}

/// Moving rows (drag to reorder): refs follow the moved cells (both relative and absolute, as in Excel).
pub fn move_rows(src: &str, from: usize, n: usize, to: usize) -> String {
    move_axis(src, Axis::Row, from, n, to)
}

/// Column counterpart of [`move_rows`].
pub fn move_cols(src: &str, from: usize, n: usize, to: usize) -> String {
    move_axis(src, Axis::Col, from, n, to)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn e(s: &str) -> String {
        eval_expr(s).display()
    }

    #[test]
    fn move_rows_and_cols_follow_cells() {
        // 第 2 行移到最上面
        assert_eq!(move_rows("=A2+A1", 1, 1, 0), "=A1+A2");
        // 第 1..2 行移到第 4 行之前(下移)
        assert_eq!(move_rows("=A1*A3", 0, 2, 3), "=A2*A1");
        assert_eq!(move_rows("=SUM(A1:A2)", 0, 2, 3), "=SUM(A2:A3)");
        assert_eq!(move_rows("=$A$2", 1, 1, 0), "=$A$1", "绝对引用也跟着走");
        // B 列移到 A 列前面
        assert_eq!(move_cols("=A1-B1", 1, 1, 0), "=B1-A1");
        assert_eq!(move_index(5, 1, 2, 3), 5, "块内插入点 = 不动");
    }

    fn table(rows: &[&[&str]]) -> Vec<Vec<Value>> {
        let cells: Vec<Vec<String>> =
            rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect();
        eval_table(&cells)
    }

    fn tshow(rows: &[&[&str]]) -> Vec<Vec<String>> {
        table(rows).iter().map(|r| r.iter().map(Value::display).collect()).collect()
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(e("1+2*3"), "7");
        assert_eq!(e("(1+2)*3"), "9");
        assert_eq!(e("10/4"), "2.5");
        assert_eq!(e("10-2-3"), "5");
        assert_eq!(e("12/2/3"), "2");
        assert_eq!(e("2^3^2"), "64", "Excel: ^ 左结合");
        assert_eq!(e("-2^2"), "4");
        assert_eq!(e("0-2^2"), "-4");
        assert_eq!(e("2*-3"), "-6");
        assert_eq!(e("2^-1"), "0.5");
        assert_eq!(e("--3"), "3");
        assert_eq!(e("+3"), "3");
        assert_eq!(e("1e3+.5"), "1000.5");
        assert_eq!(e("2E-2"), "0.02");
        assert_eq!(e("=12*3+sqrt(4)"), "38");
        assert_eq!(e("  7 - 10  "), "-3");
        assert_eq!(e("2*3^2"), "18");
    }

    #[test]
    fn percent() {
        assert_eq!(e("50%"), "0.5");
        assert_eq!(e("200*10%"), "20");
        assert_eq!(e("-50%"), "-0.5");
        assert_eq!(e("10%%"), "0.001");
        assert_eq!(e("4^50%"), "2");
    }

    #[test]
    fn text_concat_and_strings() {
        assert_eq!(e("\"a\"&\"b\"&1"), "ab1");
        assert_eq!(e("1+2&3"), "33");
        assert_eq!(e("\"say \"\"hi\"\"\""), "say \"hi\"");
        assert_eq!(e("\"x\"&TRUE"), "xTRUE");
        assert_eq!(e("\"5\"+1"), "6");
        assert_eq!(e("0.1+0.2&\"\""), "0.3");
    }

    #[test]
    fn comparisons() {
        assert_eq!(e("1<2"), "TRUE");
        assert_eq!(e("1<>1"), "FALSE");
        assert_eq!(e("3>=3"), "TRUE");
        assert_eq!(e("3<=2"), "FALSE");
        assert_eq!(e("2>1"), "TRUE");
        assert_eq!(e("1+1=2"), "TRUE");
        assert_eq!(e("\"a\"=\"A\""), "TRUE");
        assert_eq!(e("\"a\"<\"b\""), "TRUE");
        assert_eq!(e("\"1\"=1"), "FALSE");
        assert_eq!(e("1<\"a\""), "TRUE");
        assert_eq!(e("\"z\"<TRUE"), "TRUE");
        assert_eq!(e("1&2=\"12\""), "TRUE");
        assert_eq!(tshow(&[&["", "=A1=0", "=A1=\"\""]])[0][1..], ["TRUE", "TRUE"]);
    }

    #[test]
    fn references_and_ranges() {
        let v = tshow(&[
            &["1", "2", "=A1+B1"],
            &["3", "4", "=SUM(A1:B2)"],
            &["=$A$1*10", "=a2+b2", "=SUM(B2:A1)"],
        ]);
        assert_eq!(v[0][2], "3");
        assert_eq!(v[1][2], "10");
        assert_eq!(v[2][0], "10");
        assert_eq!(v[2][1], "7");
        assert_eq!(v[2][2], "10");
        // Bare range in scalar context.
        assert_eq!(tshow(&[&["1", "=A1:A1"]])[0][1], "#VALUE!");
    }

    #[test]
    fn chains_forward_and_backward() {
        assert_eq!(tshow(&[&["1", "=A1+1", "=B1*10"]])[0], ["1", "2", "20"]);
        assert_eq!(tshow(&[&["=B1+1", "=C1+1", "5"]])[0], ["7", "6", "5"]);
    }

    #[test]
    fn deep_chain_beyond_depth_limit() {
        let n = 1000;
        let mut cells: Vec<Vec<String>> = (0..n).map(|i| vec![format!("=A{}+1", i + 2)]).collect();
        cells[n - 1][0] = "0".to_string();
        let v = eval_table(&cells);
        assert_eq!(v[0][0], Value::Num((n - 1) as f64));
        assert_eq!(v[n - 2][0], Value::Num(1.0));
    }

    #[test]
    fn cycles() {
        assert_eq!(tshow(&[&["=B1", "=A1"]])[0], ["#CIRC!", "#CIRC!"]);
        assert_eq!(tshow(&[&["=A1"]])[0], ["#CIRC!"]);
        assert_eq!(tshow(&[&["=B1", "=A1", "=A1+1", "5"]])[0], ["#CIRC!", "#CIRC!", "#CIRC!", "5"]);
        assert_eq!(tshow(&[&["=SUM(A1:B1)", "1"]])[0][0], "#CIRC!");
        // Long cycle (longer than the depth limit).
        let n = 300;
        let cells: Vec<Vec<String>> =
            (0..n).map(|i| vec![format!("=A{}+1", (i + 1) % n + 1)]).collect();
        let v = eval_table(&cells);
        assert!(v.iter().all(|r| r[0] == err(E_CIRC)));
    }

    #[test]
    fn error_codes() {
        assert_eq!(e("1/0"), "#DIV/0!");
        assert_eq!(e("MOD(1,0)"), "#DIV/0!");
        assert_eq!(e("A1"), "#REF!");
        assert_eq!(e("SUM(A1:B2)"), "#REF!");
        assert_eq!(e("FOO(1)"), "#NAME?");
        assert_eq!(e("foo"), "#NAME?");
        assert_eq!(e("\"a\"+1"), "#VALUE!");
        assert_eq!(e("1+"), "#ERROR!");
        assert_eq!(e("(1"), "#ERROR!");
        assert_eq!(e("1)"), "#ERROR!");
        assert_eq!(e("\"abc"), "#ERROR!");
        assert_eq!(e(""), "#ERROR!");
        assert_eq!(e("="), "#ERROR!");
        assert_eq!(e("1 2"), "#ERROR!");
        assert_eq!(e("1#2"), "#ERROR!");
        assert_eq!(e("ROUND()"), "#ERROR!");
        assert_eq!(e("SQRT(-1)"), "#NUM!");
        assert_eq!(e("0^0"), "#NUM!");
        assert_eq!(e("0^-1"), "#DIV/0!");
        assert_eq!(e(&"(".repeat(500)), "#ERROR!");
        let v = tshow(&[&["=1/0", "=A1+1", "=Z99", "=IF(TRUE,A1,1)", "=SUM(A1,1)", "=1+"]]);
        assert_eq!(v[0], ["#DIV/0!", "#DIV/0!", "#REF!", "#DIV/0!", "#DIV/0!", "#ERROR!"]);
        // Range partly outside the table.
        assert_eq!(tshow(&[&["1", "=SUM(A1:A5)"]])[0][1], "#REF!");
    }

    #[test]
    fn aggregates_ignore_text_in_ranges() {
        let v = tshow(&[
            &["1"],
            &["x"],
            &[""],
            &["3"],
            &["=SUM(A1:A4)", "=COUNT(A1:A4)", "=COUNTA(A1:A4)", "=AVERAGE(A1:A4)"],
            &["=MIN(A1:A4)", "=MAX(A1:A4)", "=PRODUCT(A1:A4)", "=MEDIAN(A1:A4,10)"],
            &["=SUM(A2)", "=SUM(\"x\")", "=SUM(\"2\",3,TRUE)", "=AVERAGE(A2:A3)"],
            &["=SUM(A1:A4,10)", "=COUNT(1,\"a\",\"2\")", "=COUNTA(1,\"a\")", "=MAX(A2:A3)"],
        ]);
        assert_eq!(v[4][..4], ["4", "2", "3", "2"]);
        assert_eq!(v[5][..4], ["1", "3", "3", "3"]);
        assert_eq!(v[6][..4], ["0", "#VALUE!", "6", "#DIV/0!"]);
        assert_eq!(v[7][..4], ["14", "2", "2", "0"]);
        assert_eq!(e("MEDIAN(4,1,3,2)"), "2.5");
        assert_eq!(e("PRODUCT(2,3,4)"), "24");
        assert_eq!(e("MIN(5,-1,3)"), "-1");
    }

    #[test]
    fn logic_functions() {
        assert_eq!(e("IF(1>2,\"yes\",\"no\")"), "no");
        assert_eq!(e("IF(1<2,\"yes\",\"no\")"), "yes");
        assert_eq!(e("IF(FALSE,1)"), "FALSE");
        assert_eq!(e("IF(FALSE,1/0,2)"), "2");
        assert_eq!(e("IF(1/0,1,2)"), "#DIV/0!");
        assert_eq!(e("IF(\"x\",1,2)"), "#VALUE!");
        assert_eq!(e("AND(TRUE,1,2>1)"), "TRUE");
        assert_eq!(e("AND(TRUE,0)"), "FALSE");
        assert_eq!(e("OR(FALSE,0)"), "FALSE");
        assert_eq!(e("OR(FALSE,1)"), "TRUE");
        assert_eq!(e("NOT(TRUE)"), "FALSE");
        assert_eq!(e("NOT(0)"), "TRUE");
        assert_eq!(e("AND(\"x\")"), "#VALUE!");
        let v = tshow(&[&["TRUE", "x", "0", "=AND(A1:B1)", "=OR(B1:C1)", "=AND(B1)"]]);
        assert_eq!(v[0][3..], ["TRUE", "FALSE", "#VALUE!"]);
    }

    #[test]
    fn math_functions() {
        assert_eq!(e("ROUND(2.675,2)"), "2.68");
        assert_eq!(e("ROUND(1.005,2)"), "1.01");
        assert_eq!(e("ROUND(-2.5,0)"), "-3");
        assert_eq!(e("ROUND(1234,-2)"), "1200");
        assert_eq!(e("ROUND(2.5)"), "3");
        assert_eq!(e("ROUNDUP(1.21,1)"), "1.3");
        assert_eq!(e("ROUNDUP(-1.21,1)"), "-1.3");
        assert_eq!(e("ROUNDDOWN(-1.29,1)"), "-1.2");
        assert_eq!(e("ROUNDDOWN(1.29,1)"), "1.2");
        assert_eq!(e("INT(-1.5)"), "-2");
        assert_eq!(e("INT(1.9)"), "1");
        assert_eq!(e("ABS(-4)"), "4");
        assert_eq!(e("SQRT(16)"), "4");
        assert_eq!(e("POWER(2,10)"), "1024");
        assert_eq!(e("MOD(-3,2)"), "1");
        assert_eq!(e("MOD(3,-2)"), "-1");
        assert_eq!(e("MOD(5.5,2)"), "1.5");
        assert_eq!(e("PI()"), "3.1415926536");
        assert_eq!(e("ATAN2(1,1)*4"), "3.1415926536");
        assert_eq!(e("ATAN2(-1,0)"), "3.1415926536");
        assert_eq!(e("ATAN2(0,0)"), "#DIV/0!");
        assert_eq!(e("DEGREES(PI())"), "180");
        assert_eq!(e("RADIANS(180)"), "3.1415926536");
        assert_eq!(e("SIN(0)"), "0");
        assert_eq!(e("COS(0)"), "1");
        assert_eq!(e("TAN(PI()/4)"), "1");
        assert_eq!(e("ASIN(1)*2"), "3.1415926536");
        assert_eq!(e("ACOS(1)"), "0");
        assert_eq!(e("ATAN(1)*4"), "3.1415926536");
        assert_eq!(e("ASIN(2)"), "#NUM!");
        assert_eq!(e("EXP(0)"), "1");
        assert_eq!(e("LN(EXP(2))"), "2");
        assert_eq!(e("LN(0)"), "#NUM!");
        assert_eq!(e("LOG(100)"), "2");
        assert_eq!(e("LOG(8,2)"), "3");
        assert_eq!(e("LOG(8,1)"), "#DIV/0!");
        assert_eq!(e("LOG10(1000)"), "3");
        assert_eq!(e("CEILING(2.1)"), "3");
        assert_eq!(e("CEILING(2.5,2)"), "4");
        assert_eq!(e("CEILING(-2.5,2)"), "-2");
        assert_eq!(e("CEILING(-2.5,-2)"), "-4");
        assert_eq!(e("CEILING(0.25,0.1)"), "0.3");
        assert_eq!(e("CEILING(2.5,-1)"), "#NUM!");
        assert_eq!(e("FLOOR(2.9)"), "2");
        assert_eq!(e("FLOOR(-2.5,2)"), "-4");
        assert_eq!(e("FLOOR(7,0)"), "0");
        assert_eq!(e("sum(1,2)"), "3");
        assert_eq!(e("Sqrt(9)"), "3");
    }

    #[test]
    fn text_functions() {
        assert_eq!(e("LEN(\"hello\")"), "5");
        assert_eq!(e("LEN(123.5)"), "5");
        assert_eq!(e("UPPER(\"abc\")"), "ABC");
        assert_eq!(e("LOWER(\"AbC\")"), "abc");
        assert_eq!(e("CONCAT(\"a\",1,TRUE)"), "a1TRUE");
        assert_eq!(e("CONCATENATE(\"x\",\"-\",2.50)"), "x-2.5");
        assert_eq!(tshow(&[&["a", "", "3", "=CONCAT(A1:C1)"]])[0][3], "a3");
    }

    #[test]
    fn cell_literal_parsing() {
        let v = table(&[&[
            "1,234.5", "50%", " 42 ", "", "abc", "1,23", "1e3", "TRUE", "-5", "  ", "-.5", "inf", "12,345,678",
        ]]);
        assert_eq!(v[0][0], Value::Num(1234.5));
        assert_eq!(v[0][1], Value::Num(0.5));
        assert_eq!(v[0][2], Value::Num(42.0));
        assert_eq!(v[0][3], Value::Empty);
        assert_eq!(v[0][4], Value::Text("abc".into()));
        assert_eq!(v[0][5], Value::Text("1,23".into()));
        assert_eq!(v[0][6], Value::Num(1000.0));
        assert_eq!(v[0][7], Value::Bool(true));
        assert_eq!(v[0][8], Value::Num(-5.0));
        assert_eq!(v[0][9], Value::Empty);
        assert_eq!(v[0][10], Value::Num(-0.5));
        assert_eq!(v[0][11], Value::Text("inf".into()));
        assert_eq!(v[0][12], Value::Num(12_345_678.0));
    }

    #[test]
    fn ragged_rows_and_empty_refs() {
        let v = table(&[&["1", "2", "3"], &["=C2+1"], &["", "=A3", "=B2*5"]]);
        assert_eq!(v.len(), 3);
        assert_eq!(v[1].len(), 1);
        assert_eq!(v[1][0], Value::Num(1.0)); // C2 is inside the table but beyond row 2's length.
        assert_eq!(v[2][0], Value::Empty);
        assert_eq!(v[2][1], Value::Num(0.0)); // formula referencing an empty cell shows 0
        assert_eq!(v[2][2], Value::Num(0.0));
        assert_eq!(eval_table(&[]), Vec::<Vec<Value>>::new());
    }

    #[test]
    fn display_formatting() {
        assert_eq!(e("0.1+0.2"), "0.3");
        assert_eq!(e("1/3"), "0.3333333333");
        assert_eq!(e("2/3"), "0.6666666667");
        assert_eq!(e("1e20"), "1e20");
        assert_eq!(e("-1.5e20"), "-1.5e20");
        assert_eq!(e("1e-7"), "1e-7");
        assert_eq!(e("10/3*1000000"), "3333333.33333333");
        assert_eq!(e("123456789.123"), "123456789.123");
        assert_eq!(Value::Num(-0.0).display(), "0");
        assert_eq!(Value::Num(-1e-12).display(), "-1e-12");
        assert_eq!(Value::Num(5.0).display(), "5");
        assert_eq!(Value::Num(-1.5).display(), "-1.5");
        assert_eq!(Value::Num(0.99999999999).display(), "1");
        assert_eq!(Value::Num(f64::NAN).display(), "#NUM!");
        assert_eq!(Value::Bool(true).display(), "TRUE");
        assert_eq!(Value::Bool(false).display(), "FALSE");
        assert_eq!(Value::Empty.display(), "");
        assert_eq!(Value::Text("hi".into()).display(), "hi");
        assert_eq!(Value::Error("#REF!".into()).display(), "#REF!");
    }

    #[test]
    fn col_names() {
        assert_eq!(col_name(0), "A");
        assert_eq!(col_name(25), "Z");
        assert_eq!(col_name(26), "AA");
        assert_eq!(col_name(27), "AB");
        assert_eq!(col_name(701), "ZZ");
        assert_eq!(col_name(702), "AAA");
        for c in [0, 5, 25, 26, 100, 701, 702, 16383] {
            assert_eq!(parse_ref(&format!("{}1", col_name(c))), Some((0, c)));
        }
    }

    #[test]
    fn ref_name_and_parse_ref() {
        assert_eq!(ref_name(0, 0), "A1");
        assert_eq!(ref_name(9, 27), "AB10");
        assert_eq!(ref_name(0, 701), "ZZ1");
        assert_eq!(ref_name(99, 702), "AAA100");
        assert_eq!(parse_ref("B3"), Some((2, 1)));
        assert_eq!(parse_ref("b3"), Some((2, 1)));
        assert_eq!(parse_ref("$B$3"), Some((2, 1)));
        assert_eq!(parse_ref("$B3"), Some((2, 1)));
        assert_eq!(parse_ref("B$3"), Some((2, 1)));
        assert_eq!(parse_ref("ab10"), Some((9, 27)));
        assert_eq!(parse_ref("$AAA$100"), Some((99, 702)));
        for bad in ["", "A", "1", "A0", "AAAA1", "A1B", "$$A1", "A$$1", "1A", "A-1", " A1", "LOG10x"] {
            assert_eq!(parse_ref(bad), None, "{bad}");
        }
        for (r, c) in [(0, 0), (9, 27), (12345, 701), (MAX_ROWS - 1, MAX_COLS - 1)] {
            assert_eq!(parse_ref(&ref_name(r, c)), Some((r, c)));
        }
    }

    #[test]
    fn shift_dollar_combos() {
        assert_eq!(shift_formula("=B2", 1, 1), "=C3");
        assert_eq!(shift_formula("=$B$2", 1, 1), "=$B$2");
        assert_eq!(shift_formula("=$B2", 1, 1), "=$B3");
        assert_eq!(shift_formula("=B$2", 1, 1), "=C$2");
        assert_eq!(shift_formula("=B2+$B$2*$B2-B$2", 2, 3), "=E4+$B$2*$B4-E$2");
        assert_eq!(shift_formula("=Z1", 0, 1), "=AA1");
        assert_eq!(shift_formula("=AA1", 0, -1), "=Z1");
        assert_eq!(shift_formula("=b2", 1, 0), "=B3", "rewritten refs are upper-cased");
        assert_eq!(shift_formula("=b2+c3", 0, 0), "=b2+c3", "unchanged refs keep their text");
    }

    #[test]
    fn shift_ranges_and_negative() {
        assert_eq!(shift_formula("=SUM(A1:B3)", 1, 0), "=SUM(A2:B4)");
        assert_eq!(shift_formula("=SUM($A$1:B3)", 1, 1), "=SUM($A$1:C4)");
        assert_eq!(shift_formula("=SUM(A1 : B3)", 0, 2), "=SUM(C1 : D3)");
        assert_eq!(shift_formula("=A1", -1, 0), "=#REF!");
        assert_eq!(shift_formula("=A1", 0, -1), "=#REF!");
        assert_eq!(shift_formula("=$A$1+A2", -1, -1), "=$A$1+#REF!");
        assert_eq!(shift_formula("=A$1", -5, 0), "=A$1");
        assert_eq!(shift_formula("=SUM(A1:B3)+1", -1, 0), "=SUM(#REF!)+1");
        assert_eq!(shift_formula("=SUM(A2:B3)", -1, 0), "=SUM(A1:B2)");
        assert_eq!(shift_formula("=A1", MAX_ROWS as i64, 0), "=#REF!");
    }

    #[test]
    fn shift_preserves_everything_else() {
        assert_eq!(
            shift_formula("=IF(A1>0, \"A1 is \"\"B2\"\"\" & A1, sum( a2 ,3.5e2))", 1, 0),
            "=IF(A2>0, \"A1 is \"\"B2\"\"\" & A2, sum( A3 ,3.5e2))"
        );
        assert_eq!(shift_formula("  = A1 ", 1, 0), "  = A2 ");
        // Functions with digits in the name are calls, not refs (even with a space before '(').
        assert_eq!(shift_formula("=LOG10(A1)+ATAN2(B1,C1)", 1, 0), "=LOG10(A2)+ATAN2(B2,C2)");
        assert_eq!(shift_formula("=LOG10 (A1)", 1, 0), "=LOG10 (A2)");
        assert_eq!(shift_formula("=AB10+LOG10", 1, 0), "=AB11+LOG11");
        assert_eq!(shift_formula("=TRUE+foo+Sheet1", 1, 1), "=TRUE+foo+Sheet1");
        assert_eq!(shift_formula("=1E3+2", 1, 1), "=1E3+2");
        assert_eq!(shift_formula("=\"unterminated A1", 1, 1), "=\"unterminated A1");
        assert_eq!(shift_formula("=#REF!+A1", 1, 0), "=#REF!+A2");
        assert_eq!(shift_formula("=\"中文\"&A1", 1, 0), "=\"中文\"&A2");
        // Non-formulas are untouched.
        for s in ["A1", "hello A1", "", "  ", "12", "'=A1"] {
            assert_eq!(shift_formula(s, 3, 3), s);
        }
    }

    #[test]
    fn shift_round_trip() {
        for f in [
            "=A2+$B$3*C$4-$D5",
            "=SUM(B2:C9, $A$1:A2) & \"A1\"",
            "=IF(LOG10(Z9)>1, AA10, ATAN2(B2, 3))",
        ] {
            let there = shift_formula(f, 1, 1);
            assert_ne!(there, f);
            assert_eq!(shift_formula(&there, -1, -1), f);
            assert_eq!(shift_formula(&shift_formula(f, 0, 5), 0, -5), f);
        }
    }

    #[test]
    fn insert_rows_cols() {
        assert_eq!(insert_rows("=A1+A2+A3", 1, 2), "=A1+A4+A5");
        assert_eq!(insert_rows("=$A$2+A$3+$A1", 1, 1), "=$A$3+A$4+$A1", "absolute refs move too");
        assert_eq!(insert_rows("=SUM(A1:A3)", 1, 1), "=SUM(A1:A4)", "spanning range grows");
        assert_eq!(insert_rows("=SUM(A1:A3)", 0, 1), "=SUM(A2:A4)");
        assert_eq!(insert_rows("=SUM(A1:A3)", 3, 1), "=SUM(A1:A3)", "insert just below: unchanged");
        assert_eq!(insert_rows("=SUM(A3:A1)", 1, 1), "=SUM(A4:A1)");
        assert_eq!(insert_rows("=B5", 4, 0), "=B5");
        assert_eq!(insert_rows("=\"A5\"&A5", 0, 1), "=\"A5\"&A6");
        assert_eq!(insert_rows("A5", 0, 1), "A5");
        assert_eq!(insert_cols("=A1+B1+C1", 1, 1), "=A1+C1+D1");
        assert_eq!(insert_cols("=$B$1+SUM($A1:$C1)", 1, 2), "=$D$1+SUM($A1:$E1)");
        assert_eq!(insert_cols("=Z1", 25, 1), "=AA1");
        assert_eq!(insert_cols("=LOG10(B1)", 0, 1), "=LOG10(C1)");
    }

    #[test]
    fn delete_rows_cols() {
        // Single refs.
        assert_eq!(delete_rows("=A1+A2+A3+A5", 1, 2), "=A1+#REF!+#REF!+A3");
        assert_eq!(delete_rows("=$A$5+A$1", 1, 2), "=$A$3+A$1");
        assert_eq!(delete_rows("=A$2", 1, 1), "=#REF!");
        // Ranges: entirely below / above / inside / partially overlapping / spanning.
        assert_eq!(delete_rows("=SUM(A5:B6)", 1, 2), "=SUM(A3:B4)");
        assert_eq!(delete_rows("=SUM(A1:B1)", 1, 2), "=SUM(A1:B1)");
        assert_eq!(delete_rows("=SUM(A2:B3)+1", 1, 2), "=SUM(#REF!)+1");
        assert_eq!(delete_rows("=SUM(A1:A3)", 1, 5), "=SUM(A1:A1)", "tail overlap shrinks");
        assert_eq!(delete_rows("=SUM(A2:A6)", 0, 3), "=SUM(A1:A3)", "head overlap shrinks");
        assert_eq!(delete_rows("=SUM(A1:A10)", 2, 3), "=SUM(A1:A7)", "spanning range shrinks");
        assert_eq!(delete_rows("=SUM($A$1:$A$10)", 2, 3), "=SUM($A$1:$A$7)");
        assert_eq!(delete_rows("=SUM(A6:A2)", 0, 3), "=SUM(A3:A1)", "reversed endpoints");
        assert_eq!(delete_rows("=SUM(A1:A3)", 1, 0), "=SUM(A1:A3)");
        // Columns.
        assert_eq!(delete_cols("=A1+B1+C1+D1", 1, 2), "=A1+#REF!+#REF!+B1");
        assert_eq!(delete_cols("=SUM(A1:E1)", 1, 2), "=SUM(A1:C1)");
        assert_eq!(delete_cols("=SUM($B$1:C9)*2", 1, 2), "=SUM(#REF!)*2");
        assert_eq!(delete_cols("=SUM(C1:F1)", 0, 3), "=SUM(A1:C1)");
        assert_eq!(delete_cols("=\"B1\"&ATAN2(B1,C1)", 0, 1), "=\"B1\"&ATAN2(A1,B1)");
        assert_eq!(delete_cols("B1", 0, 1), "B1");
    }

    #[test]
    fn rewritten_formulas_still_evaluate() {
        let src = "=SUM(A1:B2)+$A$1";
        let moved = shift_formula(src, 1, 0);
        assert_eq!(moved, "=SUM(A2:B3)+$A$1");
        let v = tshow(&[&["1", "2"], &["3", "4"], &["5", "6"], &["", "", src, moved.as_str()]]);
        assert_eq!(v[3][2], "11");
        assert_eq!(v[3][3], "19");
    }
}
