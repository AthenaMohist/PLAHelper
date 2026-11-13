//! R30: 笔记面板界面(记录 + 计算器)。数据模型见 `notes.rs`, 公式见 `formula.rs`。
//!
//! 交互(类似 Google Keep):
//!   * 每行一个块; 没在编辑的行只显示渲染效果, 点一下进入编辑(显示原文)。
//!   * 敲 `- ` `1. ` `[] ` `# ` `---` 即时转成列表/待办/标题/分隔线; Enter 新起一行(同类型),
//!     空列表项再按 Enter 退出列表; 行首 Backspace 先退类型再并入上一行; Tab/Shift+Tab 缩进。
//!   * 待办: 点方框打勾, 勾上加删除线; Ctrl+Enter 也能切换。
//!   * 行左侧拖动柄(悬停出现)拖动重排; 右键拖动柄: 删除/复制/转换类型。
//!   * 计算: 以 `=` 开头的行显示 `表达式 = 结果`; 表格单元格支持 Excel 公式(A1 引用、SUM 等)。
//! 存盘: 改动后 0.6s 防抖写回 `.pla/notes/<doc>/notes.md`; 切文档、关面板、退出时立即写。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::{Color32, FontId, Key, Modifiers, Rect, Sense, Stroke, TextFormat};
use pla_bridge::i18n::tr;
use pla_bridge::t;

use crate::formula::{self, Value};
use crate::notes::{self, Block, Kind, Note, MAX_INDENT};

/// 配色由调用方(main.rs 的调色板)传入 —— 不在这里另立一套颜色。
#[derive(Clone, Copy)]
pub struct Theme {
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub red: Color32,
    pub green: Color32,
    pub field: Color32,
    pub card: Color32,
}

const SAVE_DEBOUNCE: Duration = Duration::from_millis(600);
const INDENT_W: f32 = 18.0;
const HANDLE_W: f32 = 14.0;
const MARK_W: f32 = 22.0;
const BODY_SIZE: f32 = 13.5;

pub struct NotesPanel {
    doc_key: Option<String>,
    note: Note,
    dirty_at: Option<Instant>,
    /// 正在编辑(显示原文 TextEdit)的块。
    editing: Option<u64>,
    /// 下一帧要聚焦的块 + 光标字符位置(None = 末尾)。
    pending_focus: Option<(u64, Option<usize>)>,
    /// 最近编辑过的块(面板工具条按钮作用对象 — 点按钮时编辑框已失焦)。
    last_focus: Option<u64>,
    /// 正在编辑的单元格 (块 id, 行, 列) + 是否需要聚焦。
    editing_cell: Option<(u64, usize, usize)>,
    cell_focus_pending: bool,
    /// 拖动重排: 被拖的块 id。
    dragging: Option<u64>,
    /// 上一帧各行矩形(悬停显示拖动柄、计算落点)。
    row_rects: HashMap<u64, Rect>,
    pub last_error: Option<String>,
}

impl Default for NotesPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// 循环里收集、循环后统一执行的结构性改动(避免边遍历边改 Vec)。
enum Op {
    Split { i: usize, parts: Vec<String>, single: bool },
    ExitList(usize),
    BackspaceAtStart(usize),
    Indent(usize, i8),
    FocusPrev(usize),
    FocusNext(usize),
    Move { from: usize, to: usize },
    Delete(usize),
    Duplicate(usize),
    Convert(usize, Kind),
    ToggleTodo(usize),
    InsertTextAfter(usize),
}

impl NotesPanel {
    pub fn new() -> Self {
        let mut note = Note::default();
        note.ensure_nonempty();
        Self {
            doc_key: None,
            note,
            dirty_at: None,
            editing: None,
            pending_focus: None,
            last_focus: None,
            editing_cell: None,
            cell_focus_pending: false,
            dragging: None,
            row_rects: HashMap::new(),
            last_error: None,
        }
    }

    /// 跟随当前文档: 文档变了 → 先把旧文档的笔记写盘, 再读新文档的。每帧调用, 未变化时零开销。
    pub fn set_doc(&mut self, doc_key: &str) {
        if self.doc_key.as_deref() == Some(doc_key) {
            return;
        }
        self.flush();
        self.note = notes::load(doc_key);
        self.doc_key = Some(doc_key.to_string());
        self.editing = None;
        self.pending_focus = None;
        self.editing_cell = None;
        self.dragging = None;
        self.row_rects.clear();
    }

    pub fn doc_key(&self) -> Option<&str> {
        self.doc_key.as_deref()
    }

    fn mark_dirty(&mut self) {
        self.dirty_at = Some(Instant::now());
    }

    /// 防抖写盘(每帧调用)。
    pub fn tick(&mut self) {
        if self.dirty_at.map_or(false, |t| t.elapsed() >= SAVE_DEBOUNCE) {
            self.flush();
        }
    }

    /// 立即写盘(有改动才写)。
    pub fn flush(&mut self) {
        if self.dirty_at.take().is_none() {
            return;
        }
        if let Some(k) = self.doc_key.clone() {
            match notes::save(&k, &self.note) {
                Ok(_) => self.last_error = None,
                Err(e) => self.last_error = Some(t!("笔记保存失败: {e}", "Failed to save notes: {e}")),
            }
        }
    }

    fn focus(&mut self, id: u64, cursor: Option<usize>) {
        self.editing = Some(id);
        self.last_focus = Some(id);
        self.pending_focus = Some((id, cursor));
        self.editing_cell = None;
    }

    fn target_index(&self) -> Option<usize> {
        self.editing.or(self.last_focus).and_then(|id| self.note.index_of(id))
    }

    // ---------------- 面板工具条 ----------------

    fn toolbar(&mut self, ui: &mut egui::Ui, th: &Theme) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let btn = |ui: &mut egui::Ui, label: &str, tip: &str| {
                ui.add(
                    egui::Button::new(egui::RichText::new(label).size(12.5).color(th.text))
                        .fill(th.card)
                        .rounding(egui::Rounding::same(5.0))
                        .min_size(egui::vec2(26.0, 22.0)),
                )
                .on_hover_text(tip)
                .clicked()
            };
            let mut convert: Option<Kind> = None;
            if btn(ui, "☐", tr("待办(也可输入 [] 空格)", "To-do (or type [] + space)")) {
                convert = Some(Kind::Todo(false));
            }
            if btn(ui, "•", tr("无序列表(也可输入 - 空格)", "Bulleted list (or type - + space)")) {
                convert = Some(Kind::Bullet);
            }
            if btn(ui, "1.", tr("有序列表(也可输入 1. 空格)", "Numbered list (or type 1. + space)")) {
                convert = Some(Kind::Number);
            }
            if btn(ui, "H", tr("标题(也可输入 # 空格)", "Heading (or type # + space)")) {
                convert = Some(Kind::Heading(2));
            }
            if btn(ui, "⊞", tr("插入表格(单元格支持 Excel 公式, 如 =SUM(A1:A3))", "Insert table (cells take Excel formulas, e.g. =SUM(A1:A3))")) {
                self.insert_table();
            }
            if let Some(k) = convert {
                match self.target_index() {
                    Some(i) if !matches!(self.note.blocks[i].kind, Kind::Table(_) | Kind::Divider) => {
                        let b = &mut self.note.blocks[i];
                        b.kind = if b.kind == k { Kind::Text } else { k };
                        let id = b.id;
                        let len = b.text.chars().count();
                        self.focus(id, Some(len));
                    }
                    _ => {
                        let b = self.note.block(k, "", 0);
                        let id = b.id;
                        self.note.blocks.push(b);
                        self.focus(id, None);
                    }
                }
                self.mark_dirty();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new("?").size(12.0).color(th.muted)).on_hover_text(tr(
                    "Markdown: - 无序  1. 有序  [] 待办  # 标题  --- 分隔线\n\
                     **粗体**  *斜体*  `代码`  ~~删除线~~\n\
                     以 = 开头的行直接计算, 如 =12*3+sqrt(2)\n\
                     Enter 新行 · Tab 缩进 · Ctrl+Enter 勾选待办 · 拖左侧柄排序 · 右键柄更多操作\n\
                     笔记跟着当前文档保存在 .pla/notes/ 下",
                    "Markdown: - bullets  1. numbers  [] to-do  # heading  --- divider\n\
                     **bold**  *italic*  `code`  ~~strike~~\n\
                     Lines starting with = are calculated, e.g. =12*3+sqrt(2)\n\
                     Enter new line · Tab indent · Ctrl+Enter check to-do · drag the left handle to reorder · right-click it for more\n\
                     Notes are saved with the current document under .pla/notes/",
                ));
            });
        });
    }

    fn insert_table(&mut self) {
        let rows = vec![
            vec![String::new(), String::new(), String::new()],
            vec![String::new(), String::new(), String::new()],
            vec![String::new(), String::new(), String::new()],
        ];
        let at = self.target_index().map_or(self.note.blocks.len(), |i| i + 1);
        let tb = self.note.block(Kind::Table(rows), "", 0);
        let tid = tb.id;
        self.note.blocks.insert(at, tb);
        // 表格后面总留一行正文, 方便继续往下写
        if at + 1 >= self.note.blocks.len() {
            let b = self.note.block(Kind::Text, "", 0);
            self.note.blocks.push(b);
        }
        self.editing = None;
        self.editing_cell = Some((tid, 0, 0));
        self.cell_focus_pending = true;
        self.mark_dirty();
    }

    // ---------------- 主体 ----------------

    pub fn ui(&mut self, ui: &mut egui::Ui, th: &Theme) {
        self.toolbar(ui, th);
        ui.add_space(2.0);
        ui.separator();
        if let Some(e) = &self.last_error {
            ui.label(egui::RichText::new(e).small().color(th.red));
        }
        let mut ops: Vec<Op> = Vec::new();
        let numbers: Vec<usize> = (0..self.note.blocks.len()).map(|i| self.note.number_of(i)).collect();
        let pointer = ui.ctx().pointer_hover_pos();
        let mut new_rects: HashMap<u64, Rect> = HashMap::new();
        let mut changed = false;

        egui::ScrollArea::vertical()
            .id_salt("notes-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                let n = self.note.blocks.len();
                for i in 0..n {
                    let id = self.note.blocks[i].id;
                    let hovered_row = pointer.map_or(false, |p| self.row_rects.get(&id).map_or(false, |r| r.contains(p)));
                    let row = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        // ---- 拖动柄 ----
                        let (hr, hresp) = ui.allocate_exact_size(egui::vec2(HANDLE_W, 20.0), Sense::click_and_drag());
                        let show_handle = hovered_row || self.dragging == Some(id) || hresp.hovered();
                        if show_handle {
                            let col = if hresp.hovered() || self.dragging == Some(id) { th.text } else { th.muted };
                            for dy in [-4.0f32, 0.0, 4.0] {
                                for dx in [-2.5f32, 2.5] {
                                    ui.painter().circle_filled(hr.center() + egui::vec2(dx, dy), 1.2, col);
                                }
                            }
                        }
                        if hresp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                        }
                        if hresp.drag_started() {
                            self.dragging = Some(id);
                            self.editing = None;
                        }
                        let is_table = matches!(self.note.blocks[i].kind, Kind::Table(_));
                        hresp.context_menu(|ui| {
                            if ui.button(tr("删除此行", "Delete line")).clicked() {
                                ops.push(Op::Delete(i));
                                ui.close_menu();
                            }
                            if ui.button(tr("复制此行", "Duplicate line")).clicked() {
                                ops.push(Op::Duplicate(i));
                                ui.close_menu();
                            }
                            if !is_table {
                                ui.separator();
                                for (label, k) in [
                                    (tr("正文", "Text"), Kind::Text),
                                    (tr("标题", "Heading"), Kind::Heading(2)),
                                    (tr("无序列表", "Bulleted list"), Kind::Bullet),
                                    (tr("有序列表", "Numbered list"), Kind::Number),
                                    (tr("待办", "To-do"), Kind::Todo(false)),
                                ] {
                                    if ui.button(label).clicked() {
                                        ops.push(Op::Convert(i, k));
                                        ui.close_menu();
                                    }
                                }
                            }
                            ui.separator();
                            if ui.button(tr("在下方插入一行", "Insert line below")).clicked() {
                                ops.push(Op::InsertTextAfter(i));
                                ui.close_menu();
                            }
                        });
                        ui.add_space(self.note.blocks[i].indent as f32 * INDENT_W);
                        if self.block_ui(ui, i, numbers[i], th, &mut ops) {
                            changed = true;
                        }
                    });
                    new_rects.insert(id, row.response.rect);
                }
                // 末尾空白区: 点击 = 在最后新起一行(或聚焦最后一个空行)
                let rest = ui.available_size_before_wrap();
                let (_, tail) = ui.allocate_exact_size(egui::vec2(rest.x, rest.y.max(40.0)), Sense::click());
                if tail.clicked() {
                    match self.note.blocks.last() {
                        Some(b) if b.kind == Kind::Text && b.text.is_empty() => {
                            let id = b.id;
                            self.focus(id, None);
                        }
                        _ => {
                            let b = self.note.block(Kind::Text, "", 0);
                            let id = b.id;
                            self.note.blocks.push(b);
                            self.focus(id, None);
                            changed = true;
                        }
                    }
                }
            });

        // ---- 拖动重排: 落点指示线 + 松手移动 ----
        if let Some(did) = self.dragging {
            let to = self.drop_index(&new_rects, pointer);
            if let (Some(to), Some(from)) = (to, self.note.index_of(did)) {
                let y = if to < self.note.blocks.len() {
                    new_rects.get(&self.note.blocks[to].id).map(|r| r.top() - 1.5)
                } else {
                    self.note.blocks.last().and_then(|b| new_rects.get(&b.id)).map(|r| r.bottom() + 1.5)
                };
                if let Some(y) = y {
                    let x0 = ui.min_rect().left();
                    let x1 = ui.min_rect().right();
                    ui.painter().line_segment([egui::pos2(x0, y), egui::pos2(x1, y)], Stroke::new(2.0, th.accent));
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                if !ui.input(|i| i.pointer.primary_down()) {
                    self.dragging = None;
                    if to != from && to != from + 1 {
                        ops.push(Op::Move { from, to });
                    }
                }
            } else if !ui.input(|i| i.pointer.primary_down()) {
                self.dragging = None;
            }
        }
        self.row_rects = new_rects;

        if !ops.is_empty() {
            changed = true;
            self.apply_ops(ops);
        }
        if changed {
            self.mark_dirty();
        }
    }

    fn drop_index(&self, rects: &HashMap<u64, Rect>, pointer: Option<egui::Pos2>) -> Option<usize> {
        let p = pointer?;
        for (i, b) in self.note.blocks.iter().enumerate() {
            if let Some(r) = rects.get(&b.id) {
                if p.y < r.center().y {
                    return Some(i);
                }
            }
        }
        Some(self.note.blocks.len())
    }

    /// 一行的标记 + 内容。返回内容是否被修改。
    fn block_ui(&mut self, ui: &mut egui::Ui, i: usize, number: usize, th: &Theme, ops: &mut Vec<Op>) -> bool {
        let kind = self.note.blocks[i].kind.clone();
        match kind {
            Kind::Divider => {
                let w = ui.available_width();
                let (r, resp) = ui.allocate_exact_size(egui::vec2(w, 14.0), Sense::click());
                ui.painter().line_segment(
                    [egui::pos2(r.left(), r.center().y), egui::pos2(r.right(), r.center().y)],
                    Stroke::new(1.0, th.muted),
                );
                if resp.clicked() {
                    ops.push(Op::InsertTextAfter(i));
                }
                false
            }
            Kind::Table(_) => self.table_ui(ui, i, th),
            _ => {
                // 标记列
                match kind {
                    Kind::Bullet => {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(MARK_W, 18.0), Sense::hover());
                        let dot = if self.note.blocks[i].indent % 2 == 0 { "•" } else { "◦" };
                        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, dot, FontId::proportional(15.0), th.text);
                    }
                    Kind::Number => {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(MARK_W, 18.0), Sense::hover());
                        ui.painter().text(
                            egui::pos2(r.right() - 3.0, r.center().y),
                            egui::Align2::RIGHT_CENTER,
                            format!("{number}."),
                            FontId::proportional(BODY_SIZE),
                            th.muted,
                        );
                    }
                    Kind::Todo(done) => {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(MARK_W, 18.0), Sense::click());
                        let bx = Rect::from_center_size(r.center(), egui::vec2(13.0, 13.0));
                        if done {
                            ui.painter().rect_filled(bx, egui::Rounding::same(3.0), th.accent);
                            let a = bx.left_center() + egui::vec2(2.5, 0.0);
                            let b = bx.center_bottom() + egui::vec2(-1.0, -3.0);
                            let c = bx.right_top() + egui::vec2(-2.5, 3.0);
                            ui.painter().line_segment([a, b], Stroke::new(1.8, Color32::BLACK));
                            ui.painter().line_segment([b, c], Stroke::new(1.8, Color32::BLACK));
                        } else {
                            let col = if resp.hovered() { th.text } else { th.muted };
                            ui.painter().rect_stroke(bx, egui::Rounding::same(3.0), Stroke::new(1.3, col));
                        }
                        if resp.clicked() {
                            ops.push(Op::ToggleTodo(i));
                        }
                    }
                    _ => {}
                }
                self.text_ui(ui, i, th, ops)
            }
        }
    }

    fn font_for(kind: &Kind) -> FontId {
        match kind {
            Kind::Heading(1) => FontId::proportional(19.0),
            Kind::Heading(2) => FontId::proportional(16.5),
            Kind::Heading(_) => FontId::proportional(14.5),
            _ => FontId::proportional(BODY_SIZE),
        }
    }

    /// 文本类块: 编辑中 → 原文 TextEdit; 否则 → 渲染后的富文本(点击进入编辑)。
    fn text_ui(&mut self, ui: &mut egui::Ui, i: usize, th: &Theme, ops: &mut Vec<Op>) -> bool {
        let id = self.note.blocks[i].id;
        let kind = self.note.blocks[i].kind.clone();
        let font = Self::font_for(&kind);
        let w = ui.available_width().max(40.0);
        let edit_id = ui.make_persistent_id(("note-line", id));

        if self.editing != Some(id) {
            let job = self.render_job(&self.note.blocks[i], &font, th, w);
            let resp = ui.add(egui::Label::new(job).wrap().sense(Sense::click()));
            // 整行宽度都可点(短文字右侧空白也算)
            let rest = ui.available_width();
            let filler = if rest > 1.0 {
                ui.allocate_exact_size(egui::vec2(rest, resp.rect.height().max(18.0)), Sense::click()).1.clicked()
            } else {
                false
            };
            if resp.clicked() || filler {
                let len = self.note.blocks[i].text.chars().count();
                self.focus(id, Some(len));
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
            return false;
        }

        // ---- 编辑模式 ----
        let before_cursor = egui::TextEdit::load_state(ui.ctx(), edit_id)
            .and_then(|s| s.cursor.char_range())
            .filter(|r| r.primary == r.secondary)
            .map(|r| r.primary.index);
        let has_focus_now = ui.memory(|m| m.has_focus(edit_id));
        let text_len = self.note.blocks[i].text.chars().count();
        if has_focus_now {
            // 先吃掉 Tab / Ctrl+Enter, 不让 TextEdit 处理
            let (tab, shift_tab, ctrl_enter) = ui.input_mut(|inp| {
                (
                    inp.consume_key(Modifiers::NONE, Key::Tab),
                    inp.consume_key(Modifiers::SHIFT, Key::Tab),
                    inp.consume_key(Modifiers::COMMAND, Key::Enter),
                )
            });
            if tab {
                ops.push(Op::Indent(i, 1));
            }
            if shift_tab {
                ops.push(Op::Indent(i, -1));
            }
            if ctrl_enter {
                ops.push(Op::ToggleTodo(i));
            }
            let (up, down, bksp) = ui.input(|inp| {
                (inp.key_pressed(Key::ArrowUp), inp.key_pressed(Key::ArrowDown), inp.key_pressed(Key::Backspace))
            });
            if up && before_cursor == Some(0) {
                ops.push(Op::FocusPrev(i));
            }
            if down && before_cursor == Some(text_len) {
                ops.push(Op::FocusNext(i));
            }
            if bksp && before_cursor == Some(0) {
                ops.push(Op::BackspaceAtStart(i));
            }
        }

        let done = matches!(kind, Kind::Todo(true));
        let block = &mut self.note.blocks[i];
        let out = egui::TextEdit::multiline(&mut block.text)
            .id(edit_id)
            .frame(false)
            .desired_rows(1)
            .desired_width(w)
            .font(font)
            .text_color(if done { th.muted } else { th.text })
            .hint_text(egui::RichText::new(tr("输入… (Markdown)", "Type… (Markdown)")).color(th.muted.gamma_multiply(0.6)))
            .show(ui);

        if let Some((pid, cur)) = self.pending_focus {
            if pid == id {
                out.response.request_focus();
                let n = cur.unwrap_or(text_len).min(text_len);
                if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), edit_id) {
                    st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(n))));
                    st.store(ui.ctx(), edit_id);
                }
                self.pending_focus = None;
            }
        } else if out.response.lost_focus() {
            if self.editing == Some(id) {
                self.editing = None;
            }
        }

        let mut changed = false;
        if out.response.changed() {
            changed = true;
            let block = &mut self.note.blocks[i];
            if block.text.contains('\n') {
                let parts: Vec<String> = block.text.split('\n').map(|s| s.replace('\r', "")).collect();
                let single = parts.len() == 2;
                if single && parts[0].is_empty() && parts[1].is_empty() && block.kind != Kind::Text {
                    // 空列表项里按 Enter → 退出列表
                    block.text.clear();
                    ops.push(Op::ExitList(i));
                } else {
                    ops.push(Op::Split { i, parts, single });
                }
            } else {
                let before = block.text.chars().count();
                if notes::apply_shortcut(block) {
                    let after = block.text.chars().count();
                    let cur = out.cursor_range.map(|r| r.primary.ccursor.index).unwrap_or(before);
                    let n = cur.saturating_sub(before - after);
                    self.pending_focus = Some((id, Some(n)));
                    if block.kind == Kind::Divider {
                        ops.push(Op::InsertTextAfter(i));
                    }
                }
            }
        }
        changed
    }

    /// 渲染后的富文本(行内样式 + 待办删除线 + 计算行结果)。
    fn render_job(&self, b: &Block, font: &FontId, th: &Theme, w: f32) -> LayoutJob {
        let mut job = LayoutJob::default();
        job.wrap.max_width = w;
        let done = matches!(b.kind, Kind::Todo(true));
        let heading = matches!(b.kind, Kind::Heading(_));
        let base = if done { th.muted } else if heading { Color32::WHITE } else { th.text };
        if b.text.is_empty() {
            job.append(" ", 0.0, TextFormat { font_id: font.clone(), color: base, ..Default::default() });
            return job;
        }
        // 计算行: "=表达式" → "表达式 = 结果"
        if b.kind == Kind::Text {
            if let Some(expr) = b.text.strip_prefix('=') {
                let v = formula::eval_expr(expr);
                let (res, col) = match &v {
                    Value::Error(_) => (v.display(), th.red),
                    _ => (v.display(), th.accent),
                };
                job.append(expr.trim(), 0.0, TextFormat { font_id: font.clone(), color: th.text, ..Default::default() });
                job.append(" = ", 0.0, TextFormat { font_id: font.clone(), color: th.muted, ..Default::default() });
                job.append(&res, 0.0, TextFormat { font_id: font.clone(), color: col, ..Default::default() });
                return job;
            }
        }
        for (s, st) in notes::inline_spans(&b.text) {
            let mut f = TextFormat { font_id: font.clone(), color: base, ..Default::default() };
            if st.bold {
                f.color = if done { th.muted } else { Color32::WHITE };
            }
            if st.italic {
                f.italics = true;
            }
            if st.code {
                f.font_id = FontId::monospace(font.size - 1.0);
                f.background = th.field;
            }
            if st.strike || done {
                f.strikethrough = Stroke::new(1.0, if done { th.muted } else { base });
            }
            job.append(&s, 0.0, f);
        }
        job
    }

    // ---------------- 表格 ----------------

    fn table_ui(&mut self, ui: &mut egui::Ui, i: usize, th: &Theme) -> bool {
        let id = self.note.blocks[i].id;
        // 渲染用快照(编辑中的单元格直接改 self.note, 不能同时借用)
        let Kind::Table(rows) = self.note.blocks[i].kind.clone() else { return false };
        let values = formula::eval_table(&rows);
        let h = rows.len();
        let w = rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
        let avail = ui.available_width();
        let hdr_w = 22.0;
        let cell_w = ((avail - hdr_w - 4.0) / w as f32 - 1.0).clamp(56.0, 160.0);
        let cell_h = 22.0;

        enum TOp {
            InsRow(usize),
            DelRow(usize),
            InsCol(usize),
            DelCol(usize),
        }
        let mut top: Option<TOp> = None;
        let mut changed = false;
        let mut next_cell: Option<(usize, usize)> = None;
        let mut stop_edit = false;

        ui.vertical(|ui| {
            egui::ScrollArea::horizontal().id_salt(("tbl-scroll", id)).show(ui, |ui| {
                egui::Grid::new(("tbl", id)).spacing([1.0, 1.0]).show(ui, |ui| {
                    ui.label("");
                    for c in 0..w {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(cell_w, 16.0), Sense::click());
                        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, formula::col_name(c), FontId::proportional(11.0), th.muted);
                        resp.on_hover_text(tr("右键: 插入/删除列", "Right-click: insert/delete column")).context_menu(|ui| {
                            if ui.button(tr("左侧插入列", "Insert column left")).clicked() {
                                top = Some(TOp::InsCol(c));
                                ui.close_menu();
                            }
                            if ui.button(tr("右侧插入列", "Insert column right")).clicked() {
                                top = Some(TOp::InsCol(c + 1));
                                ui.close_menu();
                            }
                            if w > 1 && ui.button(tr("删除此列", "Delete column")).clicked() {
                                top = Some(TOp::DelCol(c));
                                ui.close_menu();
                            }
                        });
                    }
                    ui.end_row();
                    for r in 0..h {
                        let (hr, hresp) = ui.allocate_exact_size(egui::vec2(hdr_w, cell_h), Sense::click());
                        ui.painter().text(hr.center(), egui::Align2::CENTER_CENTER, format!("{}", r + 1), FontId::proportional(11.0), th.muted);
                        hresp.on_hover_text(tr("右键: 插入/删除行", "Right-click: insert/delete row")).context_menu(|ui| {
                            if ui.button(tr("上方插入行", "Insert row above")).clicked() {
                                top = Some(TOp::InsRow(r));
                                ui.close_menu();
                            }
                            if ui.button(tr("下方插入行", "Insert row below")).clicked() {
                                top = Some(TOp::InsRow(r + 1));
                                ui.close_menu();
                            }
                            if h > 1 && ui.button(tr("删除此行", "Delete row")).clicked() {
                                top = Some(TOp::DelRow(r));
                                ui.close_menu();
                            }
                        });
                        for c in 0..w {
                            let editing = self.editing_cell == Some((id, r, c));
                            if editing {
                                let cid = ui.make_persistent_id(("cell", id, r, c));
                                let has_focus = ui.memory(|m| m.has_focus(cid));
                                let (tab, shift_tab, enter) = if has_focus {
                                    ui.input_mut(|inp| {
                                        (
                                            inp.consume_key(Modifiers::NONE, Key::Tab),
                                            inp.consume_key(Modifiers::SHIFT, Key::Tab),
                                            inp.consume_key(Modifiers::NONE, Key::Enter),
                                        )
                                    })
                                } else {
                                    (false, false, false)
                                };
                                let Kind::Table(rows) = &mut self.note.blocks[i].kind else { return };
                                let cell = &mut rows[r][c];
                                let resp = ui.add_sized(
                                    [cell_w, cell_h],
                                    egui::TextEdit::singleline(cell)
                                        .id(cid)
                                        .font(FontId::proportional(12.5))
                                        .text_color(th.text),
                                );
                                if self.cell_focus_pending {
                                    resp.request_focus();
                                    self.cell_focus_pending = false;
                                }
                                if resp.changed() {
                                    changed = true;
                                }
                                if tab {
                                    next_cell = Some(if c + 1 < w { (r, c + 1) } else if r + 1 < h { (r + 1, 0) } else { (r, c) });
                                } else if shift_tab {
                                    next_cell = Some(if c > 0 { (r, c - 1) } else if r > 0 { (r - 1, w - 1) } else { (r, c) });
                                } else if enter {
                                    if r + 1 < h { next_cell = Some((r + 1, c)) } else { stop_edit = true }
                                } else if resp.lost_focus() && !self.cell_focus_pending {
                                    stop_edit = true;
                                }
                            } else {
                                let raw = rows[r].get(c).cloned().unwrap_or_default();
                                let v = values.get(r).and_then(|row| row.get(c)).cloned().unwrap_or(Value::Empty);
                                let (rect, resp) = ui.allocate_exact_size(egui::vec2(cell_w, cell_h), Sense::click());
                                let bg = if r == 0 { th.card.gamma_multiply(1.4) } else { th.field };
                                ui.painter().rect_filled(rect, egui::Rounding::same(2.0), if resp.hovered() { bg.gamma_multiply(1.3) } else { bg });
                                let (txt, col, right) = match &v {
                                    Value::Num(_) => (v.display(), if raw.starts_with('=') { th.accent } else { th.text }, true),
                                    Value::Error(_) => (v.display(), th.red, false),
                                    Value::Bool(_) => (v.display(), th.text, true),
                                    _ => (v.display(), if r == 0 { Color32::WHITE } else { th.text }, false),
                                };
                                let font = FontId::proportional(12.5);
                                let shown = clip_text(ui, &txt, &font, cell_w - 8.0);
                                let pos = if right { egui::pos2(rect.right() - 4.0, rect.center().y) } else { egui::pos2(rect.left() + 4.0, rect.center().y) };
                                let align = if right { egui::Align2::RIGHT_CENTER } else { egui::Align2::LEFT_CENTER };
                                ui.painter().with_clip_rect(rect).text(pos, align, shown, font, col);
                                let resp = if raw.starts_with('=') { resp.on_hover_text(&raw) } else { resp };
                                if resp.clicked() {
                                    next_cell = Some((r, c));
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if ui.small_button(tr("+ 行", "+ Row")).clicked() {
                    top = Some(TOp::InsRow(h));
                }
                if ui.small_button(tr("+ 列", "+ Col")).clicked() {
                    top = Some(TOp::InsCol(w));
                }
            });
        });

        if let Some((r, c)) = next_cell {
            self.editing = None;
            self.editing_cell = Some((id, r, c));
            self.cell_focus_pending = true;
        } else if stop_edit {
            if matches!(self.editing_cell, Some((tid, _, _)) if tid == id) {
                self.editing_cell = None;
            }
        }
        if let Some(op) = top {
            let Kind::Table(rows) = &mut self.note.blocks[i].kind else { return changed };
            match op {
                TOp::InsRow(at) => rows.insert(at.min(rows.len()), vec![String::new(); w]),
                TOp::DelRow(at) => {
                    if rows.len() > 1 {
                        rows.remove(at);
                    }
                }
                TOp::InsCol(at) => {
                    for row in rows.iter_mut() {
                        row.insert(at.min(row.len()), String::new());
                    }
                }
                TOp::DelCol(at) => {
                    for row in rows.iter_mut() {
                        if row.len() > 1 && at < row.len() {
                            row.remove(at);
                        }
                    }
                }
            }
            self.editing_cell = None;
            changed = true;
        }
        changed
    }

    // ---------------- 结构性改动 ----------------

    fn apply_ops(&mut self, ops: Vec<Op>) {
        // 只执行第一个结构性改动(同一帧多个会让下标错位); 其余丢弃 — 一帧内用户只做一个动作。
        let Some(op) = ops.into_iter().next() else { return };
        let len = self.note.blocks.len();
        match op {
            Op::Split { i, parts, single } => {
                let (kind, indent) = {
                    let b = &self.note.blocks[i];
                    (b.kind.clone(), b.indent)
                };
                let next_kind = match kind {
                    Kind::Todo(_) => Kind::Todo(false),
                    Kind::Bullet | Kind::Number => kind,
                    _ => Kind::Text,
                };
                self.note.blocks[i].text = parts[0].clone();
                let mut last = self.note.blocks[i].id;
                let mut at = i + 1;
                for p in parts.iter().skip(1) {
                    let mut b = self.note.block(next_kind.clone(), p.clone(), indent);
                    if b.kind == Kind::Text {
                        notes::apply_shortcut(&mut b);
                    }
                    last = b.id;
                    self.note.blocks.insert(at, b);
                    at += 1;
                }
                self.focus(last, if single { Some(0) } else { None });
            }
            Op::ExitList(i) => {
                let b = &mut self.note.blocks[i];
                b.kind = Kind::Text;
                let id = b.id;
                self.focus(id, Some(0));
            }
            Op::BackspaceAtStart(i) => {
                let b = &mut self.note.blocks[i];
                if b.kind != Kind::Text {
                    b.kind = Kind::Text;
                    let id = b.id;
                    self.focus(id, Some(0));
                } else if b.indent > 0 {
                    b.indent -= 1;
                    let id = b.id;
                    self.focus(id, Some(0));
                } else if i > 0 && !matches!(self.note.blocks[i - 1].kind, Kind::Table(_) | Kind::Divider) {
                    let text = self.note.blocks.remove(i).text;
                    let prev = &mut self.note.blocks[i - 1];
                    let at = prev.text.chars().count();
                    prev.text.push_str(&text);
                    let id = prev.id;
                    self.focus(id, Some(at));
                } else if i > 0 && self.note.blocks[i].text.is_empty() {
                    // 上一行是表格/分隔线: 空行直接删掉
                    self.note.blocks.remove(i);
                    self.editing = None;
                }
            }
            Op::Indent(i, d) => {
                let b = &mut self.note.blocks[i];
                b.indent = (b.indent as i8 + d).clamp(0, MAX_INDENT as i8) as u8;
                let id = b.id;
                self.pending_focus = Some((id, None));
            }
            Op::FocusPrev(i) => {
                if let Some(j) = (0..i).rev().find(|&j| is_texty(&self.note.blocks[j].kind)) {
                    let id = self.note.blocks[j].id;
                    self.focus(id, None);
                }
            }
            Op::FocusNext(i) => {
                if let Some(j) = (i + 1..len).find(|&j| is_texty(&self.note.blocks[j].kind)) {
                    let id = self.note.blocks[j].id;
                    self.focus(id, Some(0));
                }
            }
            Op::Move { from, to } => {
                let b = self.note.blocks.remove(from);
                let to = if to > from { to - 1 } else { to };
                self.note.blocks.insert(to.min(self.note.blocks.len()), b);
            }
            Op::Delete(i) => {
                self.note.blocks.remove(i);
                self.editing = None;
                self.editing_cell = None;
                self.note.ensure_nonempty();
            }
            Op::Duplicate(i) => {
                let mut b = self.note.blocks[i].clone();
                b.id = self.note.new_id();
                self.note.blocks.insert(i + 1, b);
            }
            Op::Convert(i, k) => {
                let b = &mut self.note.blocks[i];
                b.kind = k;
                let id = b.id;
                let n = b.text.chars().count();
                self.focus(id, Some(n));
            }
            Op::ToggleTodo(i) => {
                let b = &mut self.note.blocks[i];
                b.kind = match b.kind {
                    Kind::Todo(d) => Kind::Todo(!d),
                    _ => Kind::Todo(false),
                };
            }
            Op::InsertTextAfter(i) => {
                let indent = self.note.blocks[i].indent;
                let b = self.note.block(Kind::Text, "", indent);
                let id = b.id;
                self.note.blocks.insert(i + 1, b);
                self.focus(id, None);
            }
        }
    }
}

fn is_texty(k: &Kind) -> bool {
    !matches!(k, Kind::Table(_) | Kind::Divider)
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
