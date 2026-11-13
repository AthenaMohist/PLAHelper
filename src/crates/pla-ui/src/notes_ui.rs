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

/// 文字色预设(深色 #181818 底上清晰可读)。
const TEXT_COLORS: [([u8; 3], &str, &str); 8] = [
    ([0xFF, 0x6B, 0x6B], "红", "Red"),
    ([0xFF, 0xA9, 0x4D], "橙", "Orange"),
    ([0xFF, 0xD4, 0x3B], "黄", "Yellow"),
    ([0x69, 0xDB, 0x7C], "绿", "Green"),
    ([0x3B, 0xC9, 0xDB], "青", "Cyan"),
    ([0x4D, 0xAB, 0xF7], "蓝", "Blue"),
    ([0xB1, 0x97, 0xFC], "紫", "Purple"),
    ([0xAD, 0xB5, 0xBD], "灰", "Gray"),
];

/// 高亮(背景)色预设: 偏暗, 浅色字压在上面仍然清楚。
const BG_COLORS: [([u8; 3], &str, &str); 8] = [
    ([0x5C, 0x2B, 0x2B], "红", "Red"),
    ([0x5C, 0x3D, 0x1E], "橙", "Orange"),
    ([0x5C, 0x52, 0x17], "黄", "Yellow"),
    ([0x23, 0x4D, 0x2C], "绿", "Green"),
    ([0x1C, 0x4A, 0x50], "青", "Cyan"),
    ([0x1E, 0x3A, 0x5F], "蓝", "Blue"),
    ([0x3F, 0x2D, 0x63], "紫", "Purple"),
    ([0x3A, 0x3D, 0x42], "灰", "Gray"),
];

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

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
    /// R31: 表格交互状态(选区/编辑格/剪贴板…), 见 `notes_table.rs`。
    table: crate::notes_table::TableState,
    /// 拖动重排: 被拖的块 id。
    dragging: Option<u64>,
    /// 上一帧各行矩形(悬停显示拖动柄、计算落点)。
    row_rects: HashMap<u64, Rect>,
    pub last_error: Option<String>,
    /// 编辑中行的最近选区(块 id, 起, 止; 字符下标) — 点工具条颜色按钮时编辑框已失焦, 靠它定位。
    last_sel: Option<(u64, usize, usize)>,
    /// 下一帧聚焦时恢复的选区(配合 `pending_focus`)。
    pending_sel: Option<(u64, usize, usize)>,
    /// 最近用过的文字色 / 高亮色(工具条按钮上显示)。
    last_color: [u8; 3],
    last_bg: [u8; 3],
    /// R31b: 撤销/重做历史(整篇笔记快照)。
    history: History,
}

/// R31b: 撤销/重做 —— 存整篇块列表快照(笔记很小, 克隆便宜)。
/// 连续打字合并: 同一行、上次改动 1.2s 内 → 不新增撤销点(一次 Ctrl+Z 撤掉一段输入, 而不是一个字)。
#[derive(Default)]
struct History {
    undo: Vec<Vec<Block>>,
    redo: Vec<Vec<Block>>,
    last_change: Option<(u64, Instant)>,
}

impl History {
    const LIMIT: usize = 200;
    const GROUP: Duration = Duration::from_millis(1200);

    /// 一帧结束发现内容变了: before = 变化前快照。
    fn record(&mut self, before: Vec<Block>, after: &[Block]) {
        let single = single_changed_block(&before, after);
        let now = Instant::now();
        let merge = matches!((single, self.last_change), (Some(id), Some((lid, t))) if id == lid && now.duration_since(t) < Self::GROUP)
            && !self.undo.is_empty();
        if !merge {
            self.undo.push(before);
            if self.undo.len() > Self::LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_change = single.map(|id| (id, now));
    }

    fn undo(&mut self, cur: Vec<Block>) -> Option<Vec<Block>> {
        let prev = self.undo.pop()?;
        self.redo.push(cur);
        self.last_change = None;
        Some(prev)
    }

    fn redo(&mut self, cur: Vec<Block>) -> Option<Vec<Block>> {
        let next = self.redo.pop()?;
        self.undo.push(cur);
        self.last_change = None;
        Some(next)
    }

    fn clear(&mut self) {
        *self = History::default();
    }
}

/// 只有一个块的文字变了(长度、顺序、类型都没变) → 该块 id; 否则(结构性改动) None。
fn single_changed_block(a: &[Block], b: &[Block]) -> Option<u64> {
    if a.len() != b.len() {
        return None;
    }
    let mut found = None;
    for (x, y) in a.iter().zip(b) {
        if x != y {
            if x.id != y.id || x.kind != y.kind || x.indent != y.indent || found.is_some() {
                return None;
            }
            found = Some(x.id);
        }
    }
    found
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
            table: Default::default(),
            dragging: None,
            row_rects: HashMap::new(),
            last_error: None,
            last_sel: None,
            pending_sel: None,
            last_color: TEXT_COLORS[0].0,
            last_bg: BG_COLORS[2].0,
            history: History::default(),
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
        self.table.clear();
        self.dragging = None;
        self.row_rects.clear();
        self.history.clear(); // 撤销历史只属于当前文档
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
        self.table.clear();
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
            ui.add_space(4.0);
            if let Some((kind, c)) = self.color_buttons(ui, th) {
                self.apply_color(kind, c);
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
                     Ctrl+B 粗体 · Ctrl+I 斜体(作用于选中文字, 再按一次取消)\n\
                     A / ab 按钮: 文字颜色 / 高亮(作用于选中文字, 未选中则整行; \"清除\" 去掉颜色)\n\
                     以 = 开头的行直接计算, 如 =12*3+sqrt(2)\n\
                     Enter 新行 · Tab 缩进 · Ctrl+Enter 勾选待办 · 拖左侧柄排序 · 右键柄更多操作\n\
                     笔记跟着当前文档保存在 .pla/notes/ 下",
                    "Markdown: - bullets  1. numbers  [] to-do  # heading  --- divider\n\
                     **bold**  *italic*  `code`  ~~strike~~\n\
                     Ctrl+B bold · Ctrl+I italic (on the selection; press again to remove)\n\
                     A / ab buttons: text color / highlight (on the selection, or the whole line; \"Clear\" removes it)\n\
                     Lines starting with = are calculated, e.g. =12*3+sqrt(2)\n\
                     Enter new line · Tab indent · Ctrl+Enter check to-do · drag the left handle to reorder · right-click it for more\n\
                     Notes are saved with the current document under .pla/notes/",
                ));
            });
        });
    }

    /// 文字色 "A" + 高亮 "ab" 两个按钮及其色板弹窗。选了颜色 → Some((类型, Some(色))); 选 "清除" → Some((类型, None))。
    fn color_buttons(&self, ui: &mut egui::Ui, th: &Theme) -> Option<(fmt::SpanKind, Option<[u8; 3]>)> {
        let mut picked = None;
        let style_btn = |text: egui::RichText| {
            egui::Button::new(text.size(12.5)).fill(th.card).rounding(egui::Rounding::same(5.0)).min_size(egui::vec2(26.0, 22.0))
        };
        // 文字色: "A" 下面一条当前色的下划线
        let a = ui
            .add(style_btn(egui::RichText::new("A").strong().color(th.text)))
            .on_hover_text(tr("文字颜色(作用于选中文字; 未选中则整行)", "Text color (applies to the selection, or the whole line)"));
        let y = a.rect.bottom() - 4.5;
        ui.painter().line_segment(
            [egui::pos2(a.rect.left() + 7.0, y), egui::pos2(a.rect.right() - 7.0, y)],
            Stroke::new(2.5f32, rgb(self.last_color)),
        );
        // 高亮: 当前高亮色做底的 "ab"
        let h = ui
            .add(style_btn(egui::RichText::new("ab").color(th.text).background_color(rgb(self.last_bg))))
            .on_hover_text(tr("高亮背景色(作用于选中文字; 未选中则整行)", "Highlight color (applies to the selection, or the whole line)"));
        for (resp, kind, palette) in [(&a, fmt::SpanKind::Color, &TEXT_COLORS), (&h, fmt::SpanKind::Bg, &BG_COLORS)] {
            let pid = ui.make_persistent_id(("notes-color-popup", kind == fmt::SpanKind::Color));
            if resp.clicked() {
                ui.memory_mut(|m| m.toggle_popup(pid));
            }
            egui::popup_below_widget(ui, pid, resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                ui.set_min_width(8.0 * 22.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (c, zh, en) in palette.iter() {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::click());
                        let stroke = if resp.hovered() { Stroke::new(1.5f32, th.text) } else { Stroke::new(1.0f32, th.muted.gamma_multiply(0.5)) };
                        if kind == fmt::SpanKind::Color {
                            ui.painter().rect_filled(r, egui::Rounding::same(4.0), th.field);
                            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "A", FontId::proportional(13.0), rgb(*c));
                        } else {
                            ui.painter().rect_filled(r, egui::Rounding::same(4.0), rgb(*c));
                            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "a", FontId::proportional(12.0), th.text);
                        }
                        ui.painter().rect_stroke(r, egui::Rounding::same(4.0), stroke);
                        if resp.on_hover_text(tr(zh, en)).clicked() {
                            picked = Some((kind, Some(*c)));
                        }
                    }
                });
                if ui.button(tr("清除", "Clear")).clicked() {
                    picked = Some((kind, None));
                }
            });
        }
        picked
    }

    /// 给最近编辑行的选区(无选区 → 整行)加 / 清除颜色 span, 然后带着选区重新聚焦该行。
    fn apply_color(&mut self, kind: fmt::SpanKind, color: Option<[u8; 3]>) {
        let sel = self.last_sel.filter(|(id, ..)| self.note.index_of(*id).is_some());
        let (id, s, e) = match sel {
            Some(x) => x,
            None => match self.target_index() {
                Some(i) => (self.note.blocks[i].id, 0, 0),
                None => return,
            },
        };
        let Some(i) = self.note.index_of(id) else { return };
        if !is_texty(&self.note.blocks[i].kind) {
            return;
        }
        let b = &mut self.note.blocks[i];
        let n = b.text.chars().count();
        let (s, e) = (s.min(n), e.min(n));
        let (s, e) = if s == e { (0, n) } else { (s, e) };
        let (text, s2, e2) = match color {
            Some(c) => fmt::apply_span(&b.text, s, e, kind, c),
            None => fmt::clear_spans(&b.text, s, e, kind),
        };
        let changed = text != b.text;
        b.text = text;
        if let Some(c) = color {
            match kind {
                fmt::SpanKind::Color => self.last_color = c,
                fmt::SpanKind::Bg => self.last_bg = c,
            }
        }
        self.focus(id, Some(e2));
        self.pending_sel = Some((id, s2, e2));
        self.last_sel = Some((id, s2, e2));
        if changed {
            self.mark_dirty();
        }
    }

    fn insert_table(&mut self) {
        let at = self.target_index().map_or(self.note.blocks.len(), |i| i + 1);
        let tb = self.note.block(Kind::Table(notes::Table::new(3, 3)), "", 0);
        let tid = tb.id;
        self.note.blocks.insert(at, tb);
        // 表格后面总留一行正文, 方便继续往下写
        if at + 1 >= self.note.blocks.len() {
            let b = self.note.block(Kind::Text, "", 0);
            self.note.blocks.push(b);
        }
        self.editing = None;
        self.table.select(tid, 0, 0);
        self.mark_dirty();
    }

    // ---------------- 主体 ----------------

    /// R31b: 面板入口 = 撤销/重做包装 + 主体。
    /// Ctrl+Z 撤销, Ctrl+Y / Ctrl+Shift+Z 重做; 先于任何控件吃掉按键(行内 TextEdit 自己的撤销不再生效,
    /// 统一走整篇笔记的历史, 文字、表格、排序、格式都能撤)。
    pub fn ui(&mut self, ui: &mut egui::Ui, th: &Theme) {
        let (redo, undo) = ui.input_mut(|i| {
            let redo = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y);
            let undo = i.consume_key(Modifiers::COMMAND, Key::Z);
            (redo, undo)
        });
        if undo {
            self.undo_redo(true);
        } else if redo {
            self.undo_redo(false);
        }
        let before = self.note.blocks.clone();
        self.ui_body(ui, th);
        if self.note.blocks != before {
            self.history.record(before, &self.note.blocks);
        }
    }

    fn undo_redo(&mut self, undo: bool) {
        let cur = self.note.blocks.clone();
        let restored = if undo { self.history.undo(cur) } else { self.history.redo(cur) };
        if let Some(blocks) = restored {
            self.note.blocks = blocks;
            self.note.ensure_nonempty();
            self.editing = None;
            self.pending_focus = None;
            self.pending_sel = None;
            self.table.clear();
            self.mark_dirty();
        }
    }

    fn ui_body(&mut self, ui: &mut egui::Ui, th: &Theme) {
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
                    ui.painter().line_segment([egui::pos2(x0, y), egui::pos2(x1, y)], Stroke::new(2.0f32, th.accent));
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
                    Stroke::new(1.0f32, th.muted),
                );
                if resp.clicked() {
                    ops.push(Op::InsertTextAfter(i));
                }
                false
            }
            Kind::Table(_) => {
                let id = self.note.blocks[i].id;
                let Kind::Table(t) = &mut self.note.blocks[i].kind else { return false };
                let out = crate::notes_table::table_ui(ui, id, t, &mut self.table, th);
                if out.activated {
                    // 表格拿到焦点 → 退出文本行编辑
                    self.editing = None;
                }
                out.changed
            }
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
                            ui.painter().line_segment([a, b], Stroke::new(1.8f32, Color32::BLACK));
                            ui.painter().line_segment([b, c], Stroke::new(1.8f32, Color32::BLACK));
                        } else {
                            let col = if resp.hovered() { th.text } else { th.muted };
                            ui.painter().rect_stroke(bx, egui::Rounding::same(3.0), Stroke::new(1.3f32, col));
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
        let mut text_len = self.note.blocks[i].text.chars().count();
        let mut fmt_changed = false;
        if has_focus_now {
            // 先吃掉 Tab / Ctrl+Enter / Ctrl+B / Ctrl+I, 不让 TextEdit 处理
            let (tab, shift_tab, ctrl_enter, ctrl_b, ctrl_i) = ui.input_mut(|inp| {
                (
                    inp.consume_key(Modifiers::NONE, Key::Tab),
                    inp.consume_key(Modifiers::SHIFT, Key::Tab),
                    inp.consume_key(Modifiers::COMMAND, Key::Enter),
                    inp.consume_key(Modifiers::COMMAND, Key::B),
                    inp.consume_key(Modifiers::COMMAND, Key::I),
                )
            });
            if ctrl_b || ctrl_i {
                // 选区包上 / 去掉 ** 或 *, 然后把新选区写回 TextEdit 状态
                if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), edit_id) {
                    let (s, e) = st
                        .cursor
                        .char_range()
                        .map(|r| {
                            let (a, b) = (r.primary.index.min(text_len), r.secondary.index.min(text_len));
                            (a.min(b), a.max(b))
                        })
                        .unwrap_or((text_len, text_len));
                    let block = &mut self.note.blocks[i];
                    let (t, s2, e2) = fmt::toggle_wrap(&block.text, s, e, if ctrl_b { "**" } else { "*" });
                    block.text = t;
                    text_len = block.text.chars().count();
                    st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(s2), CCursor::new(e2))));
                    st.store(ui.ctx(), edit_id);
                    fmt_changed = true;
                }
            }
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
        let thc = *th;
        let layout_font = font.clone();
        let mut layouter = |ui: &egui::Ui, s: &str, wrap_w: f32| {
            let job = Self::edit_job(s, &layout_font, &thc, &kind, wrap_w);
            ui.fonts(|f| f.layout_job(job))
        };
        let block = &mut self.note.blocks[i];
        let out = egui::TextEdit::multiline(&mut block.text)
            .id(edit_id)
            .frame(false)
            .desired_rows(1)
            .desired_width(w)
            .font(font)
            .text_color(if done { th.muted } else { th.text })
            .layouter(&mut layouter)
            .hint_text(egui::RichText::new(tr("输入… (Markdown)", "Type… (Markdown)")).color(th.muted.gamma_multiply(0.6)))
            .show(ui);

        if let Some((pid, cur)) = self.pending_focus {
            if pid == id {
                out.response.request_focus();
                let len = self.note.blocks[i].text.chars().count();
                let range = match self.pending_sel.take() {
                    Some((sid, a, b)) if sid == id => CCursorRange::two(CCursor::new(a.min(len)), CCursor::new(b.min(len))),
                    _ => CCursorRange::one(CCursor::new(cur.unwrap_or(len).min(len))),
                };
                if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), edit_id) {
                    st.cursor.set_char_range(Some(range));
                    st.store(ui.ctx(), edit_id);
                }
                self.pending_focus = None;
            }
        } else if out.response.lost_focus() {
            if self.editing == Some(id) {
                self.editing = None;
            }
        }
        // 记住选区: 点工具条颜色按钮时编辑框已失焦, 用这个定位要上色的文字
        if out.response.has_focus() {
            if let Some(r) = out.cursor_range {
                let (a, b) = (r.primary.ccursor.index, r.secondary.ccursor.index);
                self.last_sel = Some((id, a.min(b), a.max(b)));
            }
        }

        let mut changed = fmt_changed;
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
            job.append(&s, 0.0, Self::span_format(&st, font, th, base, done));
        }
        job
    }

    /// 一段正文的格式(渲染态 / 编辑态共用)。完成的待办: 灰色 + 删除线优先于文字色。
    fn span_format(st: &notes::Style, font: &FontId, th: &Theme, base: Color32, done: bool) -> TextFormat {
        let mut f = TextFormat { font_id: font.clone(), color: base, ..Default::default() };
        if st.bold {
            f.color = if done { th.muted } else { Color32::WHITE };
        }
        if let (Some(c), false) = (st.color, done) {
            f.color = rgb(c);
        }
        if st.italic {
            f.italics = true;
        }
        if st.code {
            f.font_id = FontId::monospace(font.size - 1.0);
            f.background = th.field;
        }
        if let Some(c) = st.bg {
            f.background = rgb(c);
        }
        if st.strike || done {
            f.strikethrough = Stroke::new(1.0f32, if done { th.muted } else { f.color });
        }
        f
    }

    /// 编辑态 layouter 用: 原文一字不差, 标记(`**` `<span …>` 等)变暗, 正文按样式显示。
    fn edit_job(text: &str, font: &FontId, th: &Theme, kind: &Kind, wrap_w: f32) -> LayoutJob {
        let done = matches!(kind, Kind::Todo(true));
        let base = if done { th.muted } else if matches!(kind, Kind::Heading(_)) { Color32::WHITE } else { th.text };
        if text.is_empty() {
            let mut job = LayoutJob::single_section(String::new(), TextFormat { font_id: font.clone(), color: base, ..Default::default() });
            job.wrap.max_width = wrap_w;
            return job;
        }
        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap_w;
        let markup = TextFormat { font_id: font.clone(), color: th.muted.gamma_multiply(0.75), ..Default::default() };
        for t in notes::inline_tokens(text) {
            let f = if t.markup { markup.clone() } else { Self::span_format(&t.style, font, th, base, done) };
            job.append(&t.text, 0.0, f);
        }
        job
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
                self.table.clear();
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

/// 行内格式的纯字符串编辑(按字符下标; 返回 (新文本, 新选区起, 新选区止))。
mod fmt {
    use crate::notes::{self, SpanTag};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum SpanKind {
        Color,
        Bg,
    }

    impl SpanKind {
        fn get(self, t: &SpanTag) -> Option<[u8; 3]> {
            match self {
                SpanKind::Color => t.color,
                SpanKind::Bg => t.bg,
            }
        }
    }

    fn run_back(c: &[char], from: usize, ch: char, limit: usize) -> usize {
        (0..from.saturating_sub(limit)).take_while(|k| c[from - 1 - k] == ch).count()
    }

    fn run_fwd(c: &[char], from: usize, ch: char, limit: usize) -> usize {
        (from..limit).take_while(|&k| c[k] == ch).count()
    }

    /// Ctrl+B / Ctrl+I: 选区包上 `marker`(`**` / `*`); 已包着(标记紧贴选区外侧或在选区内侧边缘)则去掉。
    /// 无选区: 插入一对标记, 光标放中间(光标正夹在一对标记中间时则去掉这对)。
    /// 星号连写按 `***` = 粗+斜 判断: 斜体看星号个数是否为奇数, 粗体看是否 ≥ 2。
    pub fn toggle_wrap(text: &str, s: usize, e: usize, marker: &str) -> (String, usize, usize) {
        let c: Vec<char> = text.chars().collect();
        let m: Vec<char> = marker.chars().collect();
        let k = m.len();
        let n = c.len();
        let (mut s, mut e) = (s.min(n), e.min(n));
        if s > e {
            std::mem::swap(&mut s, &mut e);
        }
        let mc = m[0];
        let present = |run: usize| if k == 1 { run % 2 == 1 } else { run >= k };
        let join = |parts: &[&[char]]| parts.iter().flat_map(|p| p.iter()).collect::<String>();
        // 标记在选区外侧
        let (l, r) = (run_back(&c, s, mc, 0), run_fwd(&c, e, mc, n));
        if present(l) && present(r) {
            return (join(&[&c[..s - k], &c[s..e], &c[e + k..]]), s - k, e - k);
        }
        // 标记在选区内侧边缘
        if e - s >= 2 * k {
            let li = run_fwd(&c, s, mc, e);
            let ri = run_back(&c, e, mc, s);
            if li < e - s && present(li) && present(ri) {
                return (join(&[&c[..s], &c[s + k..e - k], &c[e..]]), s, e - 2 * k);
            }
        }
        // 包上(选区首尾的空格留在标记外, `** x**` 在 Markdown 里不算粗体)
        if s < e && c[s..e].iter().any(|ch| !ch.is_whitespace()) {
            while c[s].is_whitespace() {
                s += 1;
            }
            while c[e - 1].is_whitespace() {
                e -= 1;
            }
        }
        (join(&[&c[..s], &m, &c[s..e], &m, &c[e..]]), s + k, e + k)
    }

    fn open_tag(color: Option<[u8; 3]>, bg: Option<[u8; 3]>) -> Option<String> {
        let mut decl = Vec::new();
        if let Some(c) = color {
            decl.push(format!("color:{}", notes::hex_color(c)));
        }
        if let Some(c) = bg {
            decl.push(format!("background:{}", notes::hex_color(c)));
        }
        (!decl.is_empty()).then(|| format!("<span style=\"{}\">", decl.join("; ")))
    }

    fn with_kind(t: &SpanTag, kind: SpanKind, v: Option<[u8; 3]>) -> (Option<[u8; 3]>, Option<[u8; 3]>) {
        match kind {
            SpanKind::Color => (v, t.bg),
            SpanKind::Bg => (t.color, v),
        }
    }

    /// 按非重叠的 (起, 止, 替换文本) 改写, 选区跟着平移(落在被删区间里的下标收到区间起点)。
    fn apply_edits(c: &[char], mut edits: Vec<(usize, usize, String)>, s: usize, e: usize) -> (String, usize, usize) {
        edits.sort_by_key(|x| x.0);
        let map = |idx: usize| -> usize {
            let mut d: isize = 0;
            for (a, b, r) in &edits {
                let rl = r.chars().count();
                if idx >= *b {
                    d += rl as isize - (b - a) as isize;
                } else if idx > *a {
                    return (*a as isize + d) as usize + (idx - a).min(rl);
                }
            }
            (idx as isize + d) as usize
        };
        let mut out = String::new();
        let mut pos = 0;
        for (a, b, r) in &edits {
            out.extend(&c[pos..*a]);
            out.push_str(r);
            pos = *b;
        }
        out.extend(&c[pos..]);
        (out, map(s), map(e))
    }

    /// 选区加颜色 span。选区正好被一个 span 包着(标签紧贴外侧, 或标签在选区两端)→ 只改那个标签的颜色;
    /// 否则新包一层 `<span style="…">…</span>`(里层颜色优先)。
    pub fn apply_span(text: &str, s: usize, e: usize, kind: SpanKind, color: [u8; 3]) -> (String, usize, usize) {
        let c: Vec<char> = text.chars().collect();
        let n = c.len();
        let (s, e) = (s.min(e).min(n), e.max(s).min(n));
        for t in notes::span_tags(text) {
            if (t.open.1 == s && t.close.0 == e) || (t.open.0 == s && t.close.1 == e) {
                let (fc, fb) = with_kind(&t, kind, Some(color));
                let tag = open_tag(fc, fb).unwrap_or_default();
                return apply_edits(&c, vec![(t.open.0, t.open.1, tag)], s, e);
            }
        }
        let (fc, fb) = match kind {
            SpanKind::Color => (Some(color), None),
            SpanKind::Bg => (None, Some(color)),
        };
        let open: Vec<char> = open_tag(fc, fb).unwrap_or_default().chars().collect();
        let mut out: String = c[..s].iter().collect();
        out.extend(&open);
        out.extend(&c[s..e]);
        out.push_str("</span>");
        out.extend(&c[e..]);
        (out, s + open.len(), e + open.len())
    }

    /// 去掉与选区相交的该类颜色(选区为空 → 整行)。标签另有一种颜色时只删这一项, 保留标签。
    pub fn clear_spans(text: &str, s: usize, e: usize, kind: SpanKind) -> (String, usize, usize) {
        let c: Vec<char> = text.chars().collect();
        let n = c.len();
        let (s, e) = (s.min(e).min(n), e.max(s).min(n));
        let whole = s == e;
        let mut edits = Vec::new();
        for t in notes::span_tags(text) {
            if kind.get(&t).is_none() || !(whole || (t.open.0 < e && s < t.close.1)) {
                continue;
            }
            let (fc, fb) = with_kind(&t, kind, None);
            match open_tag(fc, fb) {
                Some(tag) => edits.push((t.open.0, t.open.1, tag)),
                None => {
                    edits.push((t.open.0, t.open.1, String::new()));
                    edits.push((t.close.0, t.close.1, String::new()));
                }
            }
        }
        apply_edits(&c, edits, s, e)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const RED: [u8; 3] = [0xFF, 0x6B, 0x6B];
        const BLUE: [u8; 3] = [0x1E, 0x3A, 0x5F];

        fn sel(t: &(String, usize, usize)) -> String {
            t.0.chars().skip(t.1).take(t.2 - t.1).collect()
        }

        #[test]
        fn wrap_selection_and_toggle_off() {
            let r = toggle_wrap("a bc d", 2, 4, "**");
            assert_eq!(r, ("a **bc** d".into(), 4, 6));
            assert_eq!(sel(&r), "bc");
            // 再按一次(选区还是 bc): 去掉外侧标记
            assert_eq!(toggle_wrap(&r.0, r.1, r.2, "**"), ("a bc d".into(), 2, 4));
            // 选区连标记一起选中: 去掉内侧标记
            assert_eq!(toggle_wrap("a **bc** d", 2, 8, "**"), ("a bc d".into(), 2, 4));
            // 斜体
            assert_eq!(toggle_wrap("xy", 0, 2, "*"), ("*xy*".into(), 1, 3));
            assert_eq!(toggle_wrap("*xy*", 1, 3, "*"), ("xy".into(), 0, 2));
        }

        #[test]
        fn bold_and_italic_markers_do_not_confuse() {
            // 粗体里按 Ctrl+I → 粗斜, 不会吃掉粗体的一个星号
            assert_eq!(toggle_wrap("**b**", 2, 3, "*"), ("***b***".into(), 3, 4));
            assert_eq!(toggle_wrap("***b***", 3, 4, "*"), ("**b**".into(), 2, 3));
            assert_eq!(toggle_wrap("***b***", 3, 4, "**"), ("*b*".into(), 1, 2));
            // 斜体里按 Ctrl+B
            assert_eq!(toggle_wrap("*b*", 1, 2, "**"), ("***b***".into(), 3, 4));
        }

        #[test]
        fn empty_selection_inserts_pair() {
            assert_eq!(toggle_wrap("ab", 1, 1, "**"), ("a****b".into(), 3, 3));
            assert_eq!(toggle_wrap("a****b", 3, 3, "**"), ("ab".into(), 1, 1), "再按一次撤掉空的一对");
            assert_eq!(toggle_wrap("", 0, 0, "*"), ("**".into(), 1, 1));
        }

        #[test]
        fn cjk_char_indices_and_space_trim() {
            let r = toggle_wrap("中文粗体字", 2, 4, "**");
            assert_eq!(r, ("中文**粗体**字".into(), 4, 6));
            assert_eq!(sel(&r), "粗体");
            assert_eq!(toggle_wrap("你 好 吗", 1, 4, "*"), ("你 *好* 吗".into(), 3, 4));
        }

        #[test]
        fn apply_and_recolor_span() {
            let r = apply_span("中文颜色", 2, 4, SpanKind::Color, RED);
            assert_eq!(r.0, "中文<span style=\"color:#FF6B6B\">颜色</span>");
            assert_eq!(sel(&r), "颜色");
            // 同一选区换色: 改标签而不是再套一层
            let r2 = apply_span(&r.0, r.1, r.2, SpanKind::Color, BLUE);
            assert_eq!(r2.0, "中文<span style=\"color:#1E3A5F\">颜色</span>");
            assert_eq!(sel(&r2), "颜色");
            // 同一选区加高亮: 合到一个标签
            let r3 = apply_span(&r2.0, r2.1, r2.2, SpanKind::Bg, BLUE);
            assert_eq!(r3.0, "中文<span style=\"color:#1E3A5F; background:#1E3A5F\">颜色</span>");
            let st = notes::inline_spans(&r3.0);
            assert_eq!(st[1].1.color, Some(BLUE));
            assert_eq!(st[1].1.bg, Some(BLUE));
        }

        #[test]
        fn clear_spans_by_kind() {
            let src = "a<span style=\"color:#FF6B6B; background:#1E3A5F\">b</span>c<span style=\"color:#FF6B6B\">d</span>";
            // 整行清文字色: 第一个保留背景, 第二个整个去掉
            let (t, ..) = clear_spans(src, 0, 0, SpanKind::Color);
            assert_eq!(t, "a<span style=\"background:#1E3A5F\">b</span>cd");
            // 只清与选区相交的(选中 "d")
            let n = src.chars().count();
            let r = clear_spans(src, n - 8, n - 7, SpanKind::Color);
            assert_eq!(r.0, "a<span style=\"color:#FF6B6B; background:#1E3A5F\">b</span>cd");
            assert_eq!(sel(&r), "d");
            // 没有该类颜色 → 不变
            assert_eq!(clear_spans("x<span style=\"color:#FF6B6B\">y</span>", 0, 0, SpanKind::Bg).0, "x<span style=\"color:#FF6B6B\">y</span>");
        }
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    fn blk(id: u64, text: &str) -> Block {
        Block { id, kind: Kind::Text, text: text.into(), indent: 0 }
    }

    #[test]
    fn typing_groups_structure_splits_and_redo_works() {
        let mut h = History::default();
        let v0 = vec![blk(1, "")];
        let v1 = vec![blk(1, "a")];
        let v2 = vec![blk(1, "ab")];
        h.record(v0.clone(), &v1);
        h.record(v1.clone(), &v2); // 同一行连续输入 → 合并
        assert_eq!(h.undo.len(), 1);
        let v3 = vec![blk(1, "ab"), blk(2, "")];
        h.record(v2.clone(), &v3); // 新起一行 = 结构性改动 → 新撤销点
        assert_eq!(h.undo.len(), 2);
        let back = h.undo(v3.clone()).unwrap();
        assert_eq!(back, v2);
        let back2 = h.undo(back).unwrap();
        assert_eq!(back2, v0, "一次撤销撤掉整段输入");
        let fwd = h.redo(back2).unwrap();
        assert_eq!(fwd, v2);
        assert!(single_changed_block(&v2, &v3).is_none());
        assert_eq!(single_changed_block(&v1, &v2), Some(1));
    }
}
