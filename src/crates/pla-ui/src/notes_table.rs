//! R31: 笔记表格 —— Excel 手感。
//!
//! * 选中 / 编辑分开: 单击 = 选中(活动格蓝框), 再单击 / 双击 / F2 = 编辑(保留原值), 直接打字 = 覆盖编辑;
//!   方向键 / Tab / Enter 移动; Shift+点击 / Shift+方向键 / 拖动 = 扩选; 点列号/行号 = 整列/整行;
//!   Delete 清空; Esc 取消编辑。
//! * 填充柄: 选区右下角小方块, 拖动 → 公式按相对引用平移填充(`$` 锁定的行/列不动)。
//! * Ctrl+C / Ctrl+X / Ctrl+V 复制剪切粘贴(公式按位移改写); Ctrl+Shift+V 只粘贴值;
//!   系统剪贴板同时放 TSV(值), 可直接粘到 Excel; 从 Excel 复制来的也能粘。
//! * 编辑中 Ctrl+Enter = 把内容填进选区每一格; Alt+Enter = 单元格内换行。
//! * 编辑公式时(光标前是 `=` `(` `,` 运算符…)点 / 拖别的单元格 = 插入引用 A1 / 范围 A1:B3。
//! * 拖列号右边界调列宽, 拖行号下边界调行高; 宽表格出现横向滚动条。
//! * 右键列号/行号: 插入/删除行列(所有公式里的引用同步改写)。

use egui::{Color32, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke};
use pla_bridge::i18n::tr;

use crate::formula::{self, Value};
use crate::notes::Table;
use crate::notes_ui::Theme;

const HDR_W: f32 = 18.0;
const HDR_H: f32 = 16.0;
const EDGE: f32 = 4.0;
const FILL_HANDLE: f32 = 6.0;

#[derive(Clone)]
struct EditCell {
    r: usize,
    c: usize,
    buf: String,
    original: String,
    focus_pending: bool,
    /// 下一帧光标放到这个字符位置。
    caret: Option<usize>,
}

#[derive(Clone)]
struct Clip {
    /// 复制时选区左上角。
    origin: (usize, usize),
    raw: Vec<Vec<String>>,
    values: Vec<Vec<String>>,
    /// 放进系统剪贴板的文本(粘贴时比对: 一致 = 来自本表格, 用公式; 否则当外部 TSV)。
    tsv: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Corner,
    ColHeader(usize),
    ColEdge(usize),
    RowHeader(usize),
    RowEdge,
    Cell(usize, usize),
    FillHandle,
}

#[derive(Clone, Copy, PartialEq)]
enum DragMode {
    Select,
    Fill,
    ColResize { c: usize, start_w: f32, start_x: f32 },
    RowResize { start_h: f32, start_y: f32 },
    /// 编辑公式时拖选引用: 插入位置(字符下标) + 当前插入文本长度 + 起点格。
    RefPick { at: usize, len: usize, start: (usize, usize) },
    /// R31b: 在行号/列号上拖动 = 多选整行/整列。
    HeaderSelect { rows: bool },
    /// R31b: 拖已选中整行/整列的行号/列号 = 调换顺序。`to` = 当前落点(原坐标下的插入位置)。
    MoveBand { rows: bool, from: usize, n: usize, to: Option<usize> },
}

#[derive(Default)]
pub struct TableState {
    /// 当前活动的表格(块 id)。
    active: Option<u64>,
    anchor: (usize, usize),
    cursor: (usize, usize),
    editing: Option<EditCell>,
    clip: Option<Clip>,
    drag: Option<DragMode>,
    /// 填充柄拖动的当前目标格。
    fill_to: Option<(usize, usize)>,
    /// 本次按下时活动格已是这一格(松手且没拖动 → 进入编辑)。
    second_click: bool,
    /// 右键菜单对应的位置。
    menu_hit: Option<Hit>,
    /// R31b: 右键菜单"添加 [x] 行/列"的 x(默认 1)。
    menu_count: usize,
}

#[derive(Default)]
pub struct TableOut {
    pub changed: bool,
    pub activated: bool,
}

impl TableState {
    pub fn clear(&mut self) {
        self.active = None;
        self.editing = None;
        self.drag = None;
        self.fill_to = None;
    }

    pub fn select(&mut self, id: u64, r: usize, c: usize) {
        self.active = Some(id);
        self.anchor = (r, c);
        self.cursor = (r, c);
        self.editing = None;
    }

    fn sel_rect(&self) -> (usize, usize, usize, usize) {
        let (r0, r1) = (self.anchor.0.min(self.cursor.0), self.anchor.0.max(self.cursor.0));
        let (c0, c1) = (self.anchor.1.min(self.cursor.1), self.anchor.1.max(self.cursor.1));
        (r0, c0, r1, c1)
    }

    /// R31c: 把选区/编辑格夹回表内。**每次结构变化(删/移行列、粘贴、撤销)之后都要调**
    /// —— 以前只在帧首调, 右键删除 3 行后同一帧里状态栏按旧选区下标读格子 → 越界崩溃。
    fn clamp_to(&mut self, t: &Table) {
        let (mr, mc) = (t.rows().saturating_sub(1), t.cols().saturating_sub(1));
        self.anchor = (self.anchor.0.min(mr), self.anchor.1.min(mc));
        self.cursor = (self.cursor.0.min(mr), self.cursor.1.min(mc));
        if self.editing.as_ref().map_or(false, |e| e.r > mr || e.c > mc) {
            self.editing = None;
        }
        if let Some((r, c)) = self.fill_to {
            self.fill_to = Some((r.min(mr), c.min(mc)));
        }
    }

    /// 选区是整行(覆盖所有列) → (起行, 行数)。
    fn full_rows(&self, t: &Table) -> Option<(usize, usize)> {
        let (r0, c0, r1, c1) = self.sel_rect();
        (c0 == 0 && c1 + 1 == t.cols()).then_some((r0, r1 - r0 + 1))
    }

    /// 选区是整列(覆盖所有行) → (起列, 列数)。
    fn full_cols(&self, t: &Table) -> Option<(usize, usize)> {
        let (r0, c0, r1, c1) = self.sel_rect();
        (r0 == 0 && r1 + 1 == t.rows()).then_some((c0, c1 - c0 + 1))
    }

    fn in_sel(&self, r: usize, c: usize) -> bool {
        let (r0, c0, r1, c1) = self.sel_rect();
        r >= r0 && r <= r1 && c >= c0 && c <= c1
    }

    fn begin_edit(&mut self, t: &Table, keep: bool, initial: Option<String>) {
        let (r, c) = self.cursor;
        let original = t.cells[r][c].clone();
        let buf = match initial {
            Some(s) => s,
            None if keep => original.clone(),
            None => String::new(),
        };
        let caret = Some(buf.chars().count());
        self.editing = Some(EditCell { r, c, buf, original, focus_pending: true, caret });
    }
}

/// 布局: 列左边界 x(相对表格原点)。
struct Layout {
    origin: Pos2,
    col_x: Vec<f32>,
    total_w: f32,
    row_h: f32,
    rows: usize,
}

impl Layout {
    fn new(origin: Pos2, t: &Table) -> Self {
        let mut col_x = Vec::with_capacity(t.cols() + 1);
        let mut x = HDR_W;
        for w in &t.col_w {
            col_x.push(x);
            x += w;
        }
        col_x.push(x);
        Layout { origin, col_x, total_w: x, row_h: t.row_h, rows: t.rows() }
    }

    fn total_h(&self) -> f32 {
        HDR_H + self.rows as f32 * self.row_h
    }

    fn cell(&self, r: usize, c: usize) -> Rect {
        Rect::from_min_max(
            self.origin + egui::vec2(self.col_x[c], HDR_H + r as f32 * self.row_h),
            self.origin + egui::vec2(self.col_x[c + 1], HDR_H + (r + 1) as f32 * self.row_h),
        )
    }

    fn col_header(&self, c: usize) -> Rect {
        Rect::from_min_max(self.origin + egui::vec2(self.col_x[c], 0.0), self.origin + egui::vec2(self.col_x[c + 1], HDR_H))
    }

    fn row_header(&self, r: usize) -> Rect {
        Rect::from_min_max(
            self.origin + egui::vec2(0.0, HDR_H + r as f32 * self.row_h),
            self.origin + egui::vec2(HDR_W, HDR_H + (r + 1) as f32 * self.row_h),
        )
    }

    fn sel_rect(&self, (r0, c0, r1, c1): (usize, usize, usize, usize)) -> Rect {
        self.cell(r0, c0).union(self.cell(r1, c1))
    }

    /// 坐标 → 最近的格(越界夹到边上) — 拖动扩选/填充用。
    fn nearest_cell(&self, p: Pos2) -> (usize, usize) {
        let x = p.x - self.origin.x;
        let y = p.y - self.origin.y - HDR_H;
        let ncols = self.col_x.len() - 1;
        let mut c = ncols - 1;
        for i in 0..ncols {
            if x < self.col_x[i + 1] {
                c = i;
                break;
            }
        }
        let r = ((y / self.row_h).floor().max(0.0) as usize).min(self.rows - 1);
        (r, c)
    }

    fn hit(&self, p: Pos2, fill_handle: Option<Rect>) -> Option<Hit> {
        if let Some(fh) = fill_handle {
            if fh.expand(2.0).contains(p) {
                return Some(Hit::FillHandle);
            }
        }
        let x = p.x - self.origin.x;
        let y = p.y - self.origin.y;
        if x < 0.0 || y < 0.0 || x > self.total_w + EDGE || y > self.total_h() + EDGE {
            return None;
        }
        let ncols = self.col_x.len() - 1;
        if y < HDR_H {
            if x < HDR_W {
                return Some(Hit::Corner);
            }
            for c in 0..ncols {
                if (x - self.col_x[c + 1]).abs() <= EDGE {
                    return Some(Hit::ColEdge(c));
                }
            }
            return (x <= self.total_w).then(|| Hit::ColHeader(self.nearest_cell(p).1));
        }
        if x < HDR_W {
            let rel = y - HDR_H;
            let k = (rel / self.row_h).round();
            if k >= 1.0 && (rel - k * self.row_h).abs() <= EDGE {
                return Some(Hit::RowEdge);
            }
            return (y <= self.total_h()).then(|| Hit::RowHeader(self.nearest_cell(p).0));
        }
        if x <= self.total_w && y <= self.total_h() {
            let (r, c) = self.nearest_cell(p);
            return Some(Hit::Cell(r, c));
        }
        None
    }
}

/// 编辑公式时, 光标前的字符允许插入引用(Excel 的"点选引用"模式)。
fn ref_pick_ok(buf: &str, caret: usize) -> bool {
    if !buf.starts_with('=') {
        return false;
    }
    let before: Vec<char> = buf.chars().take(caret).collect();
    match before.iter().rev().find(|c| !c.is_whitespace()) {
        Some(ch) => "=+-*/^(,;:<>&%".contains(*ch),
        None => false,
    }
}

fn char_to_byte(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len())
}

/// 系统剪贴板文本 → 单元格网格(Excel 复制出来的是 TSV)。
fn parse_tsv(s: &str) -> Vec<Vec<String>> {
    let s = s.strip_suffix("\r\n").or_else(|| s.strip_suffix('\n')).unwrap_or(s);
    s.split('\n').map(|line| line.trim_end_matches('\r').split('\t').map(|c| c.to_string()).collect()).collect()
}

/// 保证表格至少有 rows×cols。
fn grow(t: &mut Table, rows: usize, cols: usize) {
    while t.cells.len() < rows {
        t.cells.push(vec![String::new(); t.cols()]);
    }
    if cols > t.cols() {
        for r in t.cells.iter_mut() {
            r.resize(cols, String::new());
        }
    }
    t.normalize();
}

/// 对所有单元格公式做结构性改写(插入/删除行列)。
fn rewrite_all(t: &mut Table, f: impl Fn(&str) -> String) {
    for row in t.cells.iter_mut() {
        for cell in row.iter_mut() {
            if cell.starts_with('=') {
                *cell = f(cell);
            }
        }
    }
}

pub fn table_ui(ui: &mut egui::Ui, id: u64, t: &mut Table, st: &mut TableState, th: &Theme) -> TableOut {
    t.normalize();
    let mut out = TableOut::default();
    let is_active = st.active == Some(id);
    if is_active {
        st.clamp_to(t);
    }
    let values = formula::eval_table(&t.cells);
    let avail = ui.available_width().max(60.0);

    // ---------------- 绘制 + 指针 ----------------
    let mut table_resp: Option<egui::Response> = None;
    let mut layout_out: Option<Layout> = None;
    egui::ScrollArea::horizontal()
        .id_salt(("tbl-scroll", id))
        .max_width(avail)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let size0 = Layout::new(Pos2::ZERO, t);
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(size0.total_w + 2.0, size0.total_h() + 2.0), Sense::click_and_drag());
            let lay = Layout::new(rect.min, t);
            let p = ui.painter_at(rect.expand(1.0));
            let font = FontId::proportional(12.5);
            let hdr_font = FontId::proportional(10.0);

            // 表头
            for c in 0..t.cols() {
                let r = lay.col_header(c);
                let sel = is_active && { let (_, c0, _, c1) = st.sel_rect(); c >= c0 && c <= c1 };
                p.rect_filled(r.shrink(0.5), egui::Rounding::same(2.0), if sel { th.accent.gamma_multiply(0.35) } else { th.card });
                p.text(r.center(), egui::Align2::CENTER_CENTER, formula::col_name(c), hdr_font.clone(), th.muted);
            }
            for rr in 0..t.rows() {
                let r = lay.row_header(rr);
                let sel = is_active && { let (r0, _, r1, _) = st.sel_rect(); rr >= r0 && rr <= r1 };
                p.rect_filled(r.shrink(0.5), egui::Rounding::same(2.0), if sel { th.accent.gamma_multiply(0.35) } else { th.card });
                p.text(r.center(), egui::Align2::CENTER_CENTER, format!("{}", rr + 1), hdr_font.clone(), th.muted);
            }
            // 单元格
            for r in 0..t.rows() {
                for c in 0..t.cols() {
                    let cr = lay.cell(r, c);
                    let editing_here = is_active && st.editing.as_ref().map_or(false, |e| e.r == r && e.c == c);
                    let bg = if r == 0 { th.card.gamma_multiply(1.4) } else { th.field };
                    p.rect_filled(cr.shrink(0.5), egui::Rounding::same(1.5), bg);
                    if is_active && st.in_sel(r, c) && st.sel_rect() != (r, c, r, c) {
                        p.rect_filled(cr.shrink(0.5), egui::Rounding::same(1.5), th.accent.gamma_multiply(0.18));
                    }
                    if editing_here {
                        continue;
                    }
                    let raw = &t.cells[r][c];
                    let v = values.get(r).and_then(|row| row.get(c)).cloned().unwrap_or(Value::Empty);
                    let (txt, col, right) = match &v {
                        Value::Num(_) => (v.display(), if raw.starts_with('=') { th.accent } else { th.text }, true),
                        Value::Error(_) => (v.display(), th.red, false),
                        Value::Bool(_) => (v.display(), th.text, true),
                        _ => (v.display(), if r == 0 { Color32::WHITE } else { th.text }, false),
                    };
                    let first_line = txt.lines().next().unwrap_or("").to_string();
                    let shown = clip_text(ui, &first_line, &font, cr.width() - 8.0);
                    let (pos, align) = if right {
                        (egui::pos2(cr.right() - 4.0, cr.center().y), egui::Align2::RIGHT_CENTER)
                    } else {
                        (egui::pos2(cr.left() + 4.0, cr.center().y), egui::Align2::LEFT_CENTER)
                    };
                    p.with_clip_rect(cr).text(pos, align, shown, font.clone(), col);
                }
            }
            // 编辑中公式的引用范围(拖选时)高亮
            if let (true, Some(DragMode::RefPick { start, .. })) = (is_active, st.drag) {
                if let Some(pp) = ui.ctx().pointer_latest_pos() {
                    let end = lay.nearest_cell(pp);
                    let rr = lay.sel_rect((start.0.min(end.0), start.1.min(end.1), start.0.max(end.0), start.1.max(end.1)));
                    p.rect_stroke(rr, egui::Rounding::same(1.0), Stroke::new(1.5f32, th.green));
                }
            }
            // 选区边框 + 填充柄 / 填充预览
            if is_active {
                let sr = lay.sel_rect(st.sel_rect());
                p.rect_stroke(sr, egui::Rounding::same(1.0), Stroke::new(1.5f32, th.accent));
                let ac = lay.cell(st.cursor.0, st.cursor.1);
                p.rect_stroke(ac.shrink(0.5), egui::Rounding::same(1.0), Stroke::new(2.0f32, th.accent));
                if st.editing.is_none() {
                    let fh = Rect::from_center_size(sr.right_bottom(), egui::vec2(FILL_HANDLE, FILL_HANDLE));
                    p.rect_filled(fh, egui::Rounding::same(1.0), th.accent);
                }
                if let (Some(DragMode::Fill), Some(to)) = (st.drag, st.fill_to) {
                    let fr = fill_target_rect(st.sel_rect(), to);
                    p.rect_stroke(lay.sel_rect(fr), egui::Rounding::same(1.0), Stroke::new(1.0f32, th.text));
                }
            }
            table_resp = Some(resp);
            layout_out = Some(lay);
        });
    let (Some(resp), Some(lay)) = (table_resp, layout_out) else { return out };
    let fill_handle = (is_active && st.editing.is_none())
        .then(|| Rect::from_center_size(lay.sel_rect(st.sel_rect()).right_bottom(), egui::vec2(FILL_HANDLE, FILL_HANDLE)));

    // 光标形状
    if let Some(hp) = resp.hover_pos() {
        match lay.hit(hp, fill_handle) {
            Some(Hit::ColEdge(_)) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
            Some(Hit::RowEdge) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical),
            Some(Hit::FillHandle) => ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair),
            _ => {}
        }
    }

    // ---------------- 编辑框 ----------------
    let edit_id = ui.make_persistent_id(("tbl-edit", id));
    let mut commit: Option<(bool /*to all selected*/, Option<(i64, i64)> /*then move*/)> = None;
    let mut cancel = false;
    let mut lost_focus = false;
    if is_active {
        if let Some(e) = st.editing.as_mut() {
            let cr = lay.cell(e.r, e.c);
            let has_focus = ui.memory(|m| m.has_focus(edit_id));
            if has_focus {
                let (enter, shift_enter, tab, shift_tab, ctrl_enter, alt_enter, esc) = ui.input_mut(|i| {
                    (
                        i.consume_key(Modifiers::NONE, Key::Enter),
                        i.consume_key(Modifiers::SHIFT, Key::Enter),
                        i.consume_key(Modifiers::NONE, Key::Tab),
                        i.consume_key(Modifiers::SHIFT, Key::Tab),
                        i.consume_key(Modifiers::COMMAND, Key::Enter),
                        i.consume_key(Modifiers::ALT, Key::Enter),
                        i.consume_key(Modifiers::NONE, Key::Escape),
                    )
                });
                if alt_enter {
                    let caret = egui::TextEdit::load_state(ui.ctx(), edit_id)
                        .and_then(|s| s.cursor.char_range())
                        .map(|r| r.primary.index)
                        .unwrap_or(e.buf.chars().count());
                    let b = char_to_byte(&e.buf, caret);
                    e.buf.insert(b, '\n');
                    e.caret = Some(caret + 1);
                }
                if enter {
                    commit = Some((false, Some((1, 0))));
                } else if shift_enter {
                    commit = Some((false, Some((-1, 0))));
                } else if tab {
                    commit = Some((false, Some((0, 1))));
                } else if shift_tab {
                    commit = Some((false, Some((0, -1))));
                } else if ctrl_enter {
                    commit = Some((true, None));
                } else if esc {
                    cancel = true;
                }
            }
            let multiline = e.buf.contains('\n');
            let h = if multiline { (cr.height() * 2.0).max(cr.height() + 18.0) } else { cr.height() };
            let er = Rect::from_min_size(cr.min, egui::vec2(cr.width().max(80.0), h));
            let te = egui::TextEdit::multiline(&mut e.buf)
                .id(edit_id)
                .desired_rows(1)
                .font(FontId::proportional(12.5))
                .text_color(th.text)
                .margin(egui::Margin::symmetric(3.0, 2.0));
            let r = ui.put(er, te);
            if e.focus_pending {
                r.request_focus();
                e.focus_pending = false;
            }
            if let Some(n) = e.caret.take() {
                if let Some(mut s) = egui::TextEdit::load_state(ui.ctx(), edit_id) {
                    s.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(n))));
                    s.store(ui.ctx(), edit_id);
                }
                ui.memory_mut(|m| m.request_focus(edit_id));
            }
            if r.lost_focus() {
                lost_focus = true;
            }
        }
    }

    // ---------------- 指针交互 ----------------
    let (pressed, released, mods, double) = ui.input(|i| {
        (i.pointer.primary_pressed(), i.pointer.primary_released(), i.modifiers, i.pointer.button_double_clicked(egui::PointerButton::Primary))
    });
    let press_pos = ui.input(|i| i.pointer.press_origin());
    let latest = ui.ctx().pointer_latest_pos();
    let mut ref_pick_started = false;

    if pressed && resp.is_pointer_button_down_on() {
        if let Some(pp) = press_pos {
            out.activated = true;
            if !is_active {
                st.select(id, 0, 0);
            }
            match lay.hit(pp, fill_handle) {
                Some(Hit::ColEdge(c)) => {
                    st.drag = Some(DragMode::ColResize { c, start_w: t.col_w[c], start_x: pp.x });
                }
                Some(Hit::RowEdge) => {
                    st.drag = Some(DragMode::RowResize { start_h: t.row_h, start_y: pp.y });
                }
                Some(Hit::FillHandle) => {
                    st.drag = Some(DragMode::Fill);
                    st.fill_to = Some(st.cursor);
                }
                Some(Hit::Cell(r, c)) => {
                    let editing_other = st.editing.as_ref().map_or(false, |e| (e.r, e.c) != (r, c));
                    let caret = egui::TextEdit::load_state(ui.ctx(), edit_id)
                        .and_then(|s| s.cursor.char_range())
                        .map(|cr| cr.primary.index);
                    let pick = editing_other
                        && st.editing.as_ref().map_or(false, |e| ref_pick_ok(&e.buf, caret.unwrap_or(e.buf.chars().count())));
                    if pick {
                        // 点选引用: 在光标处插入 A1, 编辑框保持焦点
                        let e = st.editing.as_mut().unwrap();
                        let at = caret.unwrap_or(e.buf.chars().count());
                        let name = formula::ref_name(r, c);
                        let b = char_to_byte(&e.buf, at);
                        e.buf.insert_str(b, &name);
                        e.caret = Some(at + name.chars().count());
                        st.drag = Some(DragMode::RefPick { at, len: name.chars().count(), start: (r, c) });
                        ref_pick_started = true;
                    } else {
                        if st.editing.is_some() && editing_other {
                            commit_edit(t, st, false);
                            out.changed = true;
                        }
                        if st.editing.is_none() {
                            if mods.shift {
                                st.cursor = (r, c);
                                st.second_click = false;
                            } else {
                                st.second_click = st.anchor == (r, c) && st.cursor == (r, c);
                                st.anchor = (r, c);
                                st.cursor = (r, c);
                            }
                            st.drag = Some(DragMode::Select);
                        }
                    }
                }
                Some(Hit::ColHeader(c)) => {
                    if st.editing.is_some() {
                        commit_edit(t, st, false);
                        out.changed = true;
                    }
                    let band = st.full_cols(t).filter(|(c0, n)| c >= *c0 && c < c0 + n);
                    if mods.shift {
                        // Shift+点列号 = 从锚点列扩到这一列(整列)
                        st.anchor = (0, st.anchor.1);
                        st.cursor = (t.rows() - 1, c);
                    } else if let (Some((c0, n)), true) = (band, is_active) {
                        // 按在已选中的整列上 → 拖动调换列顺序(没拖动则松手时变成单选这一列)
                        st.drag = Some(DragMode::MoveBand { rows: false, from: c0, n, to: None });
                        st.fill_to = Some((0, c));
                    } else {
                        st.anchor = (0, c);
                        st.cursor = (t.rows() - 1, c);
                        st.drag = Some(DragMode::HeaderSelect { rows: false });
                    }
                }
                Some(Hit::RowHeader(r)) => {
                    if st.editing.is_some() {
                        commit_edit(t, st, false);
                        out.changed = true;
                    }
                    let band = st.full_rows(t).filter(|(r0, n)| r >= *r0 && r < r0 + n);
                    if mods.shift {
                        st.anchor = (st.anchor.0, 0);
                        st.cursor = (r, t.cols() - 1);
                    } else if let (Some((r0, n)), true) = (band, is_active) {
                        st.drag = Some(DragMode::MoveBand { rows: true, from: r0, n, to: None });
                        st.fill_to = Some((r, 0));
                    } else {
                        st.anchor = (r, 0);
                        st.cursor = (r, t.cols() - 1);
                        st.drag = Some(DragMode::HeaderSelect { rows: true });
                    }
                }
                Some(Hit::Corner) => {
                    st.anchor = (0, 0);
                    st.cursor = (t.rows() - 1, t.cols() - 1);
                }
                None => {}
            }
        }
    }
    // 拖动进行中
    if let (Some(mode), Some(lp)) = (st.drag, latest) {
        match mode {
            DragMode::Select => {
                if resp.dragged() {
                    st.cursor = lay.nearest_cell(lp);
                    if st.cursor != st.anchor {
                        st.second_click = false;
                    }
                }
            }
            DragMode::Fill => st.fill_to = Some(lay.nearest_cell(lp)),
            DragMode::HeaderSelect { rows } => {
                let (r, c) = lay.nearest_cell(lp);
                st.cursor = if rows { (r, t.cols() - 1) } else { (t.rows() - 1, c) };
            }
            DragMode::MoveBand { rows, from, n, .. } => {
                // 落点 = 离指针最近的行/列边界(原坐标); 落在被拖的块内部 = 不动
                let to = if rows {
                    let y = lp.y - lay.origin.y - HDR_H;
                    ((y / lay.row_h).round().max(0.0) as usize).min(t.rows())
                } else {
                    let x = lp.x - lay.origin.x;
                    let mut best = 0;
                    for (i, cx) in lay.col_x.iter().enumerate() {
                        if (x - cx).abs() < (x - lay.col_x[best]).abs() {
                            best = i;
                        }
                    }
                    best.min(t.cols())
                };
                let to = (to <= from || to >= from + n).then_some(to);
                st.drag = Some(DragMode::MoveBand { rows, from, n, to });
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                if let Some(to) = to {
                    let p = ui.painter();
                    if rows {
                        let y = lay.origin.y + HDR_H + to as f32 * lay.row_h;
                        p.line_segment([egui::pos2(lay.origin.x, y), egui::pos2(lay.origin.x + lay.total_w, y)], Stroke::new(2.5f32, th.accent));
                    } else {
                        let x = lay.origin.x + lay.col_x[to];
                        p.line_segment([egui::pos2(x, lay.origin.y), egui::pos2(x, lay.origin.y + lay.total_h())], Stroke::new(2.5f32, th.accent));
                    }
                }
            }
            DragMode::ColResize { c, start_w, start_x } => {
                let w = (start_w + lp.x - start_x).max(Table::MIN_COL_W);
                if (t.col_w[c] - w).abs() > 0.5 {
                    t.col_w[c] = w;
                    out.changed = true;
                }
            }
            DragMode::RowResize { start_h, start_y } => {
                // 拖第 k 条横线, 行高按整体比例变(所有行同高)
                let k = ((start_y - lay.origin.y - HDR_H) / start_h).round().max(1.0);
                let h = (start_h + (lp.y - start_y) / k).max(Table::MIN_ROW_H);
                if (t.row_h - h).abs() > 0.5 {
                    t.row_h = h;
                    out.changed = true;
                }
            }
            DragMode::RefPick { at, len, start } => {
                let end = lay.nearest_cell(lp);
                let name = if end == start {
                    formula::ref_name(start.0, start.1)
                } else {
                    let (r0, c0) = (start.0.min(end.0), start.1.min(end.1));
                    let (r1, c1) = (start.0.max(end.0), start.1.max(end.1));
                    format!("{}:{}", formula::ref_name(r0, c0), formula::ref_name(r1, c1))
                };
                if let Some(e) = st.editing.as_mut() {
                    let n = name.chars().count();
                    if n != len || !e.buf.chars().skip(at).take(len).eq(name.chars()) {
                        let b0 = char_to_byte(&e.buf, at);
                        let b1 = char_to_byte(&e.buf, at + len);
                        e.buf.replace_range(b0..b1, &name);
                        e.caret = Some(at + n);
                        st.drag = Some(DragMode::RefPick { at, len: n, start });
                    }
                }
            }
        }
        if released || !ui.input(|i| i.pointer.primary_down()) {
            let mode = st.drag.unwrap_or(mode); // 用本帧更新过的状态(拖动落点等)
            match mode {
                DragMode::Fill => {
                    if let Some(to) = st.fill_to.take() {
                        if fill(t, st, to) {
                            out.changed = true;
                        }
                    }
                }
                DragMode::Select => {
                    if st.second_click && resp.hovered() {
                        st.begin_edit(t, true, None);
                    }
                }
                DragMode::RefPick { .. } => {
                    if let Some(e) = st.editing.as_mut() {
                        e.focus_pending = true;
                    }
                }
                DragMode::MoveBand { rows, from, n, to } => match to {
                    Some(to) => {
                        let dst = if rows { move_rows(t, from, n, to) } else { move_cols(t, from, n, to) };
                        // 选区跟着移动后的块
                        if rows {
                            st.anchor = (dst, 0);
                            st.cursor = (dst + n - 1, t.cols() - 1);
                        } else {
                            st.anchor = (0, dst);
                            st.cursor = (t.rows() - 1, dst + n - 1);
                        }
                        out.changed = true;
                    }
                    None => {
                        // 没拖动: 单击已选中的行号/列号 = 只选这一行/列
                        if let Some((r, c)) = st.fill_to {
                            if rows {
                                st.anchor = (r, 0);
                                st.cursor = (r, t.cols() - 1);
                            } else {
                                st.anchor = (0, c);
                                st.cursor = (t.rows() - 1, c);
                            }
                        }
                    }
                },
                _ => {}
            }
            st.fill_to = None;
            st.second_click = false;
            st.drag = None;
        }
    }
    if double && is_active && st.editing.is_none() && resp.hovered() {
        if let Some(Hit::Cell(r, c)) = latest.and_then(|lp| lay.hit(lp, None)) {
            st.anchor = (r, c);
            st.cursor = (r, c);
            st.begin_edit(t, true, None);
        }
    }

    // ---------------- 右键菜单: 插入/删除行列 ----------------
    if resp.secondary_clicked() {
        st.menu_hit = latest.and_then(|lp| lay.hit(lp, None));
        match st.menu_hit {
            Some(Hit::Cell(r, c)) if !st.in_sel(r, c) || st.active != Some(id) => st.select(id, r, c),
            // 右键没选中的行号/列号 → 先选中那一整行/列(同 Excel)
            Some(Hit::RowHeader(r)) if st.active != Some(id) || !(st.full_rows(t).map_or(false, |(r0, n)| r >= r0 && r < r0 + n)) => {
                st.select(id, r, 0);
                st.cursor = (r, t.cols() - 1);
            }
            Some(Hit::ColHeader(c)) if st.active != Some(id) || !(st.full_cols(t).map_or(false, |(c0, n)| c >= c0 && c < c0 + n)) => {
                st.select(id, 0, c);
                st.cursor = (t.rows() - 1, c);
            }
            _ => {}
        }
        out.activated = true;
    }
    resp.context_menu(|ui| {
        let (r, c) = match st.menu_hit {
            Some(Hit::Cell(r, c)) => (Some(r), Some(c)),
            Some(Hit::ColHeader(c)) => (None, Some(c)),
            Some(Hit::RowHeader(r)) => (Some(r), None),
            _ => (Some(st.cursor.0), Some(st.cursor.1)),
        };
        // R31d: 菜单项不换行, 宽度按最长项撑开(英文"Add 5 column(s) right"以前被折成两行)
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        ui.set_min_width(150.0);
        let mut done = false;
        // R31b: "添加 [x] 行/列" — x 默认 1, 可改
        if st.menu_count == 0 {
            st.menu_count = 1;
        }
        ui.horizontal(|ui| {
            ui.label(tr("数量", "Count"));
            ui.add(egui::DragValue::new(&mut st.menu_count).range(1..=500).speed(0.2));
        });
        let k = st.menu_count.max(1);
        // 右键点在选区内 → 作用于整个选区的行/列范围; 否则只作用于点到的那一行/列
        let (sr0, sc0, sr1, sc1) = st.sel_rect();
        let row_band = r.map(|r| if r >= sr0 && r <= sr1 { (sr0, sr1) } else { (r, r) });
        let col_band = c.map(|c| if c >= sc0 && c <= sc1 { (sc0, sc1) } else { (c, c) });
        ui.separator();
        if let Some((r0, r1)) = row_band {
            if ui.button(pla_bridge::t!("上方添加 {k} 行", "Add {k} row(s) above")).clicked() {
                insert_rows(t, r0, k);
                done = true;
            }
            if ui.button(pla_bridge::t!("下方添加 {k} 行", "Add {k} row(s) below")).clicked() {
                insert_rows(t, r1 + 1, k);
                done = true;
            }
            let n = r1 - r0 + 1;
            if n < t.rows() && ui.button(pla_bridge::t!("删除 {n} 行", "Delete {n} row(s)")).clicked() {
                delete_rows(t, r0, n);
                done = true;
            }
        }
        if row_band.is_some() && col_band.is_some() {
            ui.separator();
        }
        if let Some((c0, c1)) = col_band {
            if ui.button(pla_bridge::t!("左侧添加 {k} 列", "Add {k} column(s) left")).clicked() {
                insert_cols(t, c0, k);
                done = true;
            }
            if ui.button(pla_bridge::t!("右侧添加 {k} 列", "Add {k} column(s) right")).clicked() {
                insert_cols(t, c1 + 1, k);
                done = true;
            }
            let n = c1 - c0 + 1;
            if n < t.cols() && ui.button(pla_bridge::t!("删除 {n} 列", "Delete {n} column(s)")).clicked() {
                delete_cols(t, c0, n);
                done = true;
            }
        }
        if done {
            out.changed = true;
            st.editing = None;
            st.clamp_to(t);
            ui.close_menu();
        }
    });
    // 结构可能刚变(右键插删、拖动调换、粘贴) → 后面的代码按新尺寸读格子
    if is_active {
        st.clamp_to(t);
    }

    // ---------------- 结束编辑 ----------------
    if is_active {
        if cancel {
            st.editing = None;
        } else if let Some((all, mv)) = commit {
            commit_edit(t, st, all);
            out.changed = true;
            if let Some((dr, dc)) = mv {
                move_cursor(t, st, dr, dc, false);
            }
        } else if lost_focus && !ref_pick_started && !matches!(st.drag, Some(DragMode::RefPick { .. })) {
            // 点到表格外 / 别处 → 提交
            if st.editing.as_ref().map_or(false, |e| !e.focus_pending && e.caret.is_none()) {
                commit_edit(t, st, false);
                out.changed = true;
            }
        }
    }

    // ---------------- 键盘(选中、未编辑) ----------------
    if is_active && st.editing.is_none() && ui.memory(|m| m.focused().is_none()) {
        if keyboard(ui, t, st, &values) {
            out.changed = true;
        }
    }

    // ---------------- 底部按钮 ----------------
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if ui.small_button(tr("+ 行", "+ Row")).clicked() {
            let n = t.rows();
            insert_rows(t, n, 1);
            out.changed = true;
        }
        if ui.small_button(tr("+ 列", "+ Col")).clicked() {
            let n = t.cols();
            insert_cols(t, n, 1);
            out.changed = true;
        }
        if is_active {
            let (r0, c0, r1, c1) = st.sel_rect();
            let label = if (r0, c0) == (r1, c1) {
                formula::ref_name(r0, c0)
            } else {
                format!("{}:{}", formula::ref_name(r0, c0), formula::ref_name(r1, c1))
            };
            ui.label(egui::RichText::new(label).small().color(th.muted));
            if st.editing.is_none() {
                if let Some(raw) = t.cells.get(st.cursor.0).and_then(|r| r.get(st.cursor.1)) {
                    if raw.starts_with('=') {
                        ui.label(egui::RichText::new(raw).small().monospace().color(th.muted));
                    }
                }
            }
        }
    });
    out
}

/// 把编辑框内容写回(all = 写进选区每一格, 公式按相对活动格的位移改写)。
fn commit_edit(t: &mut Table, st: &mut TableState, all: bool) {
    let Some(e) = st.editing.take() else { return };
    if all {
        let (r0, c0, r1, c1) = st.sel_rect();
        for r in r0..=r1 {
            for c in c0..=c1 {
                t.cells[r][c] = formula::shift_formula(&e.buf, r as i64 - e.r as i64, c as i64 - e.c as i64);
            }
        }
    } else {
        t.cells[e.r][e.c] = e.buf;
    }
    let _ = e.original;
}

fn move_cursor(t: &Table, st: &mut TableState, dr: i64, dc: i64, extend: bool) {
    let r = (st.cursor.0 as i64 + dr).clamp(0, t.rows() as i64 - 1) as usize;
    let c = (st.cursor.1 as i64 + dc).clamp(0, t.cols() as i64 - 1) as usize;
    st.cursor = (r, c);
    if !extend {
        st.anchor = (r, c);
    }
}

/// 填充目标区域: 源选区沿拖动方向延伸到 `to`(只沿一个方向, 取超出更多的那个轴)。
fn fill_target_rect((r0, c0, r1, c1): (usize, usize, usize, usize), to: (usize, usize)) -> (usize, usize, usize, usize) {
    let down = to.0.saturating_sub(r1);
    let up = r0.saturating_sub(to.0);
    let right = to.1.saturating_sub(c1);
    let left = c0.saturating_sub(to.1);
    if down.max(up) >= right.max(left) {
        if down >= up { (r0, c0, to.0.max(r1), c1) } else { (to.0.min(r0), c0, r1, c1) }
    } else if right >= left {
        (r0, c0, r1, to.1.max(c1))
    } else {
        (r0, to.1.min(c0), r1, c1)
    }
}

/// 填充柄松手: 源选区按位移平铺到目标区域。返回是否有改动。
fn fill(t: &mut Table, st: &mut TableState, to: (usize, usize)) -> bool {
    let src = st.sel_rect();
    let dst = fill_target_rect(src, to);
    if dst == src {
        return false;
    }
    let (r0, c0, r1, c1) = src;
    let (h, w) = (r1 - r0 + 1, c1 - c0 + 1);
    let snapshot = t.cells.clone();
    for r in dst.0..=dst.2 {
        for c in dst.1..=dst.3 {
            if r >= r0 && r <= r1 && c >= c0 && c <= c1 {
                continue;
            }
            let sr = r0 + (r as i64 - r0 as i64).rem_euclid(h as i64) as usize;
            let sc = c0 + (c as i64 - c0 as i64).rem_euclid(w as i64) as usize;
            t.cells[r][c] = formula::shift_formula(&snapshot[sr][sc], r as i64 - sr as i64, c as i64 - sc as i64);
        }
    }
    st.anchor = (dst.0, dst.1);
    st.cursor = (dst.2, dst.3);
    true
}

fn insert_rows(t: &mut Table, at: usize, n: usize) {
    rewrite_all(t, |s| formula::insert_rows(s, at, n));
    let w = t.cols();
    for _ in 0..n {
        t.cells.insert(at.min(t.cells.len()), vec![String::new(); w]);
    }
    t.normalize();
}

fn delete_rows(t: &mut Table, at: usize, n: usize) {
    if t.rows() <= n {
        return;
    }
    for _ in 0..n {
        if at < t.cells.len() {
            t.cells.remove(at);
        }
    }
    rewrite_all(t, |s| formula::delete_rows(s, at, n));
    t.normalize();
}

/// R31b: 把第 from..from+n 行移到原坐标第 `to` 行之前(公式引用跟着走)。返回移动后块的起始行。
fn move_rows(t: &mut Table, from: usize, n: usize, to: usize) -> usize {
    if n == 0 || (to > from && to < from + n) || from + n > t.rows() {
        return from;
    }
    rewrite_all(t, |s| formula::move_rows(s, from, n, to));
    let block: Vec<Vec<String>> = t.cells.drain(from..from + n).collect();
    let dst = if to > from { to - n } else { to };
    for (i, row) in block.into_iter().enumerate() {
        t.cells.insert(dst + i, row);
    }
    dst
}

/// 列版 [`move_rows`](列宽一起移动)。
fn move_cols(t: &mut Table, from: usize, n: usize, to: usize) -> usize {
    if n == 0 || (to > from && to < from + n) || from + n > t.cols() {
        return from;
    }
    rewrite_all(t, |s| formula::move_cols(s, from, n, to));
    let dst = if to > from { to - n } else { to };
    for row in t.cells.iter_mut() {
        let block: Vec<String> = row.drain(from..from + n).collect();
        for (i, v) in block.into_iter().enumerate() {
            row.insert(dst + i, v);
        }
    }
    let wb: Vec<f32> = t.col_w.drain(from..from + n).collect();
    for (i, w) in wb.into_iter().enumerate() {
        t.col_w.insert(dst + i, w);
    }
    dst
}

fn insert_cols(t: &mut Table, at: usize, n: usize) {
    rewrite_all(t, |s| formula::insert_cols(s, at, n));
    for row in t.cells.iter_mut() {
        for _ in 0..n {
            row.insert(at.min(row.len()), String::new());
        }
    }
    for _ in 0..n {
        t.col_w.insert(at.min(t.col_w.len()), Table::DEFAULT_COL_W);
    }
    t.normalize();
}

fn delete_cols(t: &mut Table, at: usize, n: usize) {
    if t.cols() <= n {
        return;
    }
    for row in t.cells.iter_mut() {
        for _ in 0..n {
            if at < row.len() {
                row.remove(at);
            }
        }
    }
    for _ in 0..n {
        if at < t.col_w.len() {
            t.col_w.remove(at);
        }
    }
    rewrite_all(t, |s| formula::delete_cols(s, at, n));
    t.normalize();
}

/// 选中状态下的键盘: 移动/扩选、F2/打字进编辑、Delete、复制剪切粘贴。返回内容是否改动。
fn keyboard(ui: &mut egui::Ui, t: &mut Table, st: &mut TableState, values: &[Vec<Value>]) -> bool {
    let mut changed = false;
    let events: Vec<egui::Event> = ui.input(|i| i.events.clone());
    // Tab 不让 egui 拿去做焦点切换
    ui.input_mut(|i| {
        i.consume_key(Modifiers::NONE, Key::Tab);
        i.consume_key(Modifiers::SHIFT, Key::Tab);
    });
    for ev in events {
        match ev {
            egui::Event::Key { key, pressed: true, modifiers, .. } => {
                let ext = modifiers.shift;
                match key {
                    Key::ArrowUp => move_cursor(t, st, -1, 0, ext),
                    Key::ArrowDown => move_cursor(t, st, 1, 0, ext),
                    Key::ArrowLeft => move_cursor(t, st, 0, -1, ext),
                    Key::ArrowRight => move_cursor(t, st, 0, 1, ext),
                    Key::Tab => move_cursor(t, st, 0, if ext { -1 } else { 1 }, false),
                    Key::Enter if !modifiers.command => move_cursor(t, st, if ext { -1 } else { 1 }, 0, false),
                    Key::F2 => st.begin_edit(t, true, None),
                    Key::Delete | Key::Backspace => {
                        let (r0, c0, r1, c1) = st.sel_rect();
                        for r in r0..=r1 {
                            for c in c0..=c1 {
                                if !t.cells[r][c].is_empty() {
                                    t.cells[r][c].clear();
                                    changed = true;
                                }
                            }
                        }
                        if key == Key::Backspace {
                            st.begin_edit(t, false, None);
                        }
                    }
                    Key::Escape => {
                        st.anchor = st.cursor;
                    }
                    _ => {}
                }
            }
            egui::Event::Text(s) => {
                // 直接打字 = 覆盖编辑(Excel)
                if !s.chars().any(|c| c.is_control()) && !ui.input(|i| i.modifiers.command) {
                    st.begin_edit(t, false, Some(s));
                    break;
                }
            }
            egui::Event::Copy => copy(ui, t, st, values, false),
            egui::Event::Cut => {
                copy(ui, t, st, values, true);
                changed = true;
            }
            egui::Event::Paste(text) => {
                let values_only = ui.input(|i| i.modifiers.shift);
                if paste(t, st, &text, values_only) {
                    changed = true;
                }
            }
            _ => {}
        }
    }
    changed
}

fn copy(ui: &egui::Ui, t: &mut Table, st: &mut TableState, values: &[Vec<Value>], cut: bool) {
    let (r0, c0, r1, c1) = st.sel_rect();
    let raw: Vec<Vec<String>> = (r0..=r1).map(|r| t.cells[r][c0..=c1].to_vec()).collect();
    let vals: Vec<Vec<String>> = (r0..=r1)
        .map(|r| (c0..=c1).map(|c| values.get(r).and_then(|row| row.get(c)).map(|v| v.display()).unwrap_or_default()).collect())
        .collect();
    let tsv = vals.iter().map(|row| row.join("\t")).collect::<Vec<_>>().join("\n");
    ui.ctx().output_mut(|o| o.copied_text = tsv.clone());
    st.clip = Some(Clip { origin: (r0, c0), raw, values: vals, tsv });
    if cut {
        for r in r0..=r1 {
            for c in c0..=c1 {
                t.cells[r][c].clear();
            }
        }
    }
}

/// 粘贴: 剪贴板文本与上次复制一致 → 用内部剪贴板(公式按位移改写; values_only 只贴值);
/// 否则当外部 TSV(Excel 等)。选区比剪贴板大且是整数倍 → 平铺(同 Excel)。
fn paste(t: &mut Table, st: &mut TableState, text: &str, values_only: bool) -> bool {
    let internal = st.clip.as_ref().filter(|c| c.tsv.replace("\r\n", "\n").trim_end() == text.replace("\r\n", "\n").trim_end()).cloned();
    let (grid, origin): (Vec<Vec<String>>, Option<(usize, usize)>) = match &internal {
        Some(c) if values_only => (c.values.clone(), None),
        Some(c) => (c.raw.clone(), Some(c.origin)),
        None => (parse_tsv(text), None),
    };
    if grid.is_empty() {
        return false;
    }
    let (gh, gw) = (grid.len(), grid.iter().map(|r| r.len()).max().unwrap_or(1));
    let (r0, c0, r1, c1) = st.sel_rect();
    let (sh, sw) = (r1 - r0 + 1, c1 - c0 + 1);
    let (th_, tw) = if sh % gh == 0 && sw % gw == 0 && (sh > gh || sw > gw) { (sh, sw) } else { (gh, gw) };
    grow(t, r0 + th_, c0 + tw);
    for dr in 0..th_ {
        for dc in 0..tw {
            let src = grid[dr % gh].get(dc % gw).cloned().unwrap_or_default();
            let (r, c) = (r0 + dr, c0 + dc);
            t.cells[r][c] = match origin {
                Some((or, oc)) => {
                    let (sr, sc) = (or + dr % gh, oc + dc % gw);
                    formula::shift_formula(&src, r as i64 - sr as i64, c as i64 - sc as i64)
                }
                None => src,
            };
        }
    }
    st.anchor = (r0, c0);
    st.cursor = (r0 + th_ - 1, c0 + tw - 1);
    true
}

/// 单元格文字超宽 → 省略号(单元格不换行)。
fn clip_text(ui: &egui::Ui, s: &str, font: &FontId, max_w: f32) -> String {
    let width = |t: &str| ui.fonts(|f| f.layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE).size().x);
    if width(s) <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut n = chars.len();
    while n > 1 && width(&format!("{}…", chars[..n].iter().collect::<String>())) > max_w {
        n -= 1;
    }
    format!("{}…", chars[..n].iter().collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tbl(rows: &[&[&str]]) -> Table {
        let mut t = Table { cells: rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect(), col_w: vec![], row_h: 22.0 };
        t.normalize();
        t
    }

    #[test]
    fn fill_down_shifts_relative_refs_only() {
        let mut t = tbl(&[&["1", "=A1*2", "=$A$1+A1"], &["2", "", ""], &["3", "", ""]]);
        let mut st = TableState::default();
        st.select(1, 0, 1);
        st.cursor = (0, 2);
        assert!(fill(&mut t, &mut st, (2, 2)));
        assert_eq!(t.cells[1][1], "=A2*2");
        assert_eq!(t.cells[2][1], "=A3*2");
        assert_eq!(t.cells[2][2], "=$A$1+A3");
        assert_eq!(st.sel_rect(), (0, 1, 2, 2));
    }

    #[test]
    fn fill_direction_picks_dominant_axis() {
        assert_eq!(fill_target_rect((0, 0, 0, 0), (3, 1)), (0, 0, 3, 0));
        assert_eq!(fill_target_rect((2, 2, 2, 2), (2, 0)), (2, 0, 2, 2));
    }

    #[test]
    fn copy_paste_internal_shifts_and_values_only() {
        let mut t = tbl(&[&["5", "=A1+1"], &["", ""], &["", ""]]);
        let mut st = TableState::default();
        st.select(1, 0, 1);
        let vals = formula::eval_table(&t.cells);
        let (r0, c0, r1, c1) = st.sel_rect();
        let raw: Vec<Vec<String>> = (r0..=r1).map(|r| t.cells[r][c0..=c1].to_vec()).collect();
        let v: Vec<Vec<String>> = vec![vec![vals[0][1].display()]];
        st.clip = Some(Clip { origin: (0, 1), raw, values: v, tsv: "6".into() });
        st.select(1, 2, 1);
        assert!(paste(&mut t, &mut st, "6", false));
        assert_eq!(t.cells[2][1], "=A3+1", "公式按位移改写");
        st.select(1, 1, 1);
        assert!(paste(&mut t, &mut st, "6", true));
        assert_eq!(t.cells[1][1], "6", "只贴值");
    }

    #[test]
    fn paste_external_tsv_grows_table_and_tiles() {
        let mut t = tbl(&[&["a"]]);
        let mut st = TableState::default();
        st.select(1, 0, 0);
        assert!(paste(&mut t, &mut st, "1\t2\r\n3\t4\r\n", false));
        assert_eq!((t.rows(), t.cols()), (2, 2));
        assert_eq!(t.cells[1][1], "4");
        // 选区是剪贴板整数倍 → 平铺
        let mut t2 = tbl(&[&["", ""], &["", ""]]);
        st.select(1, 0, 0);
        st.cursor = (1, 1);
        assert!(paste(&mut t2, &mut st, "x", false));
        assert!(t2.cells.iter().flatten().all(|c| c == "x"));
    }

    #[test]
    fn ctrl_enter_applies_to_selection_with_shift() {
        let mut t = tbl(&[&["1", ""], &["2", ""]]);
        let mut st = TableState::default();
        st.select(1, 0, 1);
        st.cursor = (1, 1);
        st.anchor = (0, 1);
        st.cursor = (1, 1);
        st.editing = Some(EditCell { r: 1, c: 1, buf: "=A2*10".into(), original: String::new(), focus_pending: false, caret: None });
        commit_edit(&mut t, &mut st, true);
        assert_eq!(t.cells[0][1], "=A1*10");
        assert_eq!(t.cells[1][1], "=A2*10");
    }

    #[test]
    fn structural_edits_rewrite_formulas() {
        let mut t = tbl(&[&["1", "=A2"], &["2", ""]]);
        insert_rows(&mut t, 0, 1);
        assert_eq!(t.cells[2][1], "", "原 B1 下移到第 2 行");
        assert_eq!(t.cells[1][1], "=A3", "引用跟着下移");
        insert_cols(&mut t, 0, 1);
        assert_eq!(t.cells[1][2], "=B3");
        assert_eq!(t.col_w.len(), 3);
        delete_cols(&mut t, 0, 1);
        assert_eq!(t.cells[1][1], "=A3");
    }

    #[test]
    fn move_rows_and_cols_reorder_and_rewrite() {
        let mut t = tbl(&[&["a", "=A3"], &["b", ""], &["c", ""]]);
        // 第 3 行(c) 拖到最上面
        let dst = move_rows(&mut t, 2, 1, 0);
        assert_eq!(dst, 0);
        assert_eq!(t.cells.iter().map(|r| r[0].as_str()).collect::<Vec<_>>(), ["c", "a", "b"]);
        assert_eq!(t.cells[1][1], "=A1", "引用跟着 c 走");
        // 前两行一起下移到末尾
        let dst = move_rows(&mut t, 0, 2, 3);
        assert_eq!(dst, 1);
        assert_eq!(t.cells.iter().map(|r| r[0].as_str()).collect::<Vec<_>>(), ["b", "c", "a"]);
        // 列: B 列移到 A 前, 列宽一起走
        let mut t2 = tbl(&[&["1", "2"]]);
        t2.col_w = vec![50.0, 90.0];
        move_cols(&mut t2, 1, 1, 0);
        assert_eq!(t2.cells[0], ["2", "1"]);
        assert_eq!(t2.col_w, [90.0, 50.0]);
        // 落在块内部 = 不动
        assert_eq!(move_rows(&mut t, 0, 2, 1), 0);
    }

    #[test]
    fn delete_selected_rows_then_selection_stays_in_bounds() {
        // 用户场景(R31c 崩溃): 4 行表选中后 3 行, 删除 → 同一帧继续按选区读格子不能越界
        let mut t = tbl(&[&["a"], &["b"], &["c"], &["d"]]);
        let mut st = TableState::default();
        st.select(1, 1, 0);
        st.cursor = (3, 0);
        let (r0, _, r1, _) = st.sel_rect();
        delete_rows(&mut t, r0, r1 - r0 + 1);
        assert_eq!(t.rows(), 1);
        st.clamp_to(&t);
        let (r0, c0, r1, c1) = st.sel_rect();
        assert!(r1 < t.rows() && c1 < t.cols());
        assert_eq!(t.cells[st.cursor.0][st.cursor.1], "a");
        assert_eq!((r0, c0), (0, 0));
        // 编辑格也被清掉/夹回
        st.editing = Some(EditCell { r: 5, c: 0, buf: String::new(), original: String::new(), focus_pending: false, caret: None });
        st.clamp_to(&t);
        assert!(st.editing.is_none());
    }

    #[test]
    fn full_row_and_col_bands() {
        let t = tbl(&[&["", ""], &["", ""], &["", ""]]);
        let mut st = TableState::default();
        st.select(1, 1, 0);
        st.cursor = (2, 1);
        assert_eq!(st.full_rows(&t), Some((1, 2)));
        assert_eq!(st.full_cols(&t), None);
        st.anchor = (0, 1);
        st.cursor = (2, 1);
        assert_eq!(st.full_cols(&t), Some((1, 1)));
    }

    #[test]
    fn ref_pick_only_after_operator() {
        assert!(ref_pick_ok("=", 1));
        assert!(ref_pick_ok("=SUM(", 5));
        assert!(ref_pick_ok("=A1+ ", 5));
        assert!(!ref_pick_ok("=A1", 3));
        assert!(!ref_pick_ok("abc", 3));
    }
}
