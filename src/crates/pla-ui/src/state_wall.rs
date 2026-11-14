//! R34: 状态存档墙(视口状态 / 材质状态共用的面板主体)。
//!
//! 交互: 列表 + 缩略图、收藏置顶(3 列)、改名、删除(确认)、更新(旋转箭头, 确认后用当前状态覆盖,
//! 名字与收藏保留)、管理条(全选/反选/红色删除/完成, 在列表顶部)、点击恢复。
//! 文件操作(收藏/改名/删除)在这里直接做; 需要 CDP 的动作(恢复、更新采集)交回调用方。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pla_bridge::i18n::tr;
use pla_bridge::t;
use pla_ui::state_store::{StateMeta, StateStore};

use super::{
    big_img_h_for, confirm_popup, confirm_popup_with, fav_img_h_for, manage_bar, pencil_button, png_dims, png_uri,
    refresh_button, COL_ACCENT, COL_MUTED, COL_RED, COL_TEXT,
};

/// 一面墙的界面状态。
#[derive(Default)]
pub struct StateWall {
    pub states: Vec<StateMeta>,
    pub loaded: bool,
    pub pngs: HashMap<String, Arc<[u8]>>,
    pub rename: Option<String>,
    pub rename_buf: String,
    pub delete_confirm: Option<String>,
    pub update_confirm: Option<String>,
    pub manage: bool,
    pub selected: HashSet<String>,
}

impl StateWall {
    /// 切换文档 / 外部改动后清空。
    pub fn reset(&mut self) {
        *self = StateWall::default();
    }
    /// 存档已变: 下一帧重新列出并重读缩略图。
    pub fn invalidate(&mut self) {
        self.loaded = false;
        self.pngs.clear();
    }
}

/// 每面墙的文字与标识。
pub struct WallText {
    /// egui id 前缀 + 缩略图 uri 前缀, 如 "vp" / "ms"。
    pub id: &'static str,
    /// 单项的类别名(悬浮标题), 如 "视口状态"。
    pub kind: &'static str,
    pub empty: &'static str,
    pub batch_delete_tip: &'static str,
    /// 更新确认文字(参数 = 名字)。
    pub update_confirm: fn(&str) -> String,
}

/// 本帧需要调用方处理的结果。
#[derive(Default)]
pub struct WallOutcome {
    pub notices: Vec<(bool, String)>,
    /// 点击恢复的项。
    pub apply: Option<String>,
    /// 确认更新的项(调用方去采集当前状态再 `store.update`)。
    pub update: Option<String>,
    pub hint: Option<(String, String)>,
}

/// 画墙。`brief(name)` = 悬浮摘要。
pub fn show(
    ui: &mut egui::Ui,
    wall: &mut StateWall,
    store: StateStore,
    doc_key: &str,
    text: &WallText,
    brief: &dyn Fn(&str) -> String,
) -> WallOutcome {
    let mut out = WallOutcome::default();
    let inner_tip = std::cell::RefCell::new(None::<(String, String)>);
    if !wall.loaded {
        wall.states = store.list(doc_key);
        wall.loaded = true;
    }
    // 管理模式操作条放列表顶部
    if wall.manage {
        let all: Vec<String> = wall.states.iter().map(|m| m.name.clone()).collect();
        let (del, done) = manage_bar(ui, &all, &mut wall.selected, text.batch_delete_tip);
        if del {
            let names: Vec<String> = wall.selected.iter().cloned().collect();
            match store.delete(doc_key, &names) {
                Ok(n) => {
                    wall.loaded = false;
                    wall.states.retain(|m| !names.contains(&m.name));
                    for nm in &names {
                        wall.pngs.remove(nm);
                    }
                    wall.selected.clear();
                    out.notices.push((false, t!("已删除 {n} 项", "Deleted {n} items")));
                }
                Err(e) => out.notices.push((true, e)),
            }
        }
        if done {
            wall.manage = false;
            wall.selected.clear();
        }
    }
    // PNG 缓存
    let dir = store.dir(doc_key);
    for m in wall.states.clone() {
        if !wall.pngs.contains_key(&m.name) {
            if let Ok(b) = std::fs::read(dir.join(format!("{}.png", m.name))) {
                wall.pngs.insert(m.name.clone(), Arc::from(b));
            }
        }
    }
    let (fav_list, other_list): (Vec<_>, Vec<_>) = wall.states.iter().cloned().partition(|m| m.fav);
    if wall.states.is_empty() {
        ui.label(egui::RichText::new(text.empty).small().color(COL_MUTED));
    }
    let mut delete_ask: Option<String> = None;
    let mut update_ask: Option<String> = None;
    let mut fav_toggle: Option<(String, bool)> = None;
    let mut rename_start: Option<String> = None;
    let mut rename_commit = false;
    let mut rename_cancel = false;
    let content_h = ui.available_height().max(72.0);
    egui::ScrollArea::vertical()
        .id_salt(format!("{}-wall", text.id))
        .max_height(content_h)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let avail_w = ui.available_width();
            let sp = ui.spacing().item_spacing.x;
            let cell_w = ((avail_w - sp * 2.0) / 3.0).max(40.0);
            let mut item = |ui: &mut egui::Ui, meta: &StateMeta, w: f32, img_h: f32| {
                let mut del = false;
                let mut fav = false;
                let mut rn = false;
                let mut upd = false;
                ui.allocate_ui_with_layout(egui::vec2(w, img_h + 38.0), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    let png = wall.pngs.get(&meta.name);
                    let dims = png.and_then(|b| png_dims(b));
                    let img_resp = match (png, dims) {
                        (Some(bytes), Some((pw, ph))) => {
                            let iw = (img_h * pw / ph).min(w);
                            ui.allocate_ui_with_layout(egui::vec2(w, img_h), egui::Layout::top_down(egui::Align::Center), |ui| {
                                ui.add(
                                    egui::Image::from_bytes(png_uri(text.id, &meta.name, bytes), bytes.clone())
                                        .fit_to_exact_size(egui::vec2(iw, img_h))
                                        .sense(egui::Sense::click()),
                                )
                            })
                            .inner
                        }
                        _ => ui.add_sized(
                            egui::vec2(w, img_h),
                            egui::Label::new(egui::RichText::new(tr("无截图", "No image")).small().color(COL_MUTED))
                                .sense(egui::Sense::click()),
                        ),
                    };
                    if img_resp.hovered() {
                        let b = brief(&meta.name);
                        *inner_tip.borrow_mut() = Some((
                            format!("{} — {}", meta.name, text.kind),
                            t!(
                                "{b} · 保存于 {} · 点击恢复",
                                "{b} · saved {} · click to restore",
                                if meta.created.is_empty() { "-" } else { &meta.created }
                            ),
                        ));
                    }
                    if img_resp.clicked() {
                        out.apply = Some(meta.name.clone());
                    }
                    ui.horizontal(|ui| {
                        if wall.manage {
                            let mut on = wall.selected.contains(&meta.name);
                            if ui.checkbox(&mut on, "").changed() {
                                if on {
                                    wall.selected.insert(meta.name.clone());
                                } else {
                                    wall.selected.remove(&meta.name);
                                }
                            }
                        }
                        if wall.rename.as_deref() == Some(meta.name.as_str()) {
                            ui.add_sized(egui::vec2((w - 60.0).max(30.0), 16.0), egui::TextEdit::singleline(&mut wall.rename_buf))
                                .request_focus();
                            let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                            if enter {
                                rename_commit = true;
                            }
                            if esc {
                                rename_cancel = true;
                            }
                        } else {
                            // 名称独占一行
                            let name_w = (w - if wall.manage { 16.0 + sp } else { 0.0 }).max(30.0);
                            let nr = ui
                                .allocate_ui_with_layout(egui::vec2(name_w, 14.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(&meta.name).small().color(COL_TEXT))
                                            .sense(egui::Sense::click())
                                            .truncate(),
                                    )
                                })
                                .inner;
                            if nr.clicked() {
                                out.apply = Some(meta.name.clone());
                            }
                        }
                    });
                    // 第二行按钮组(★ ✎ ⟳ 🗑)
                    ui.horizontal(|ui| {
                        let (srect, sresp) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
                        ui.painter().text(
                            srect.center(),
                            egui::Align2::CENTER_CENTER,
                            if meta.fav { "★" } else { "☆" },
                            egui::FontId::proportional(13.0),
                            if meta.fav { egui::Color32::from_rgb(255, 200, 60) } else { COL_MUTED },
                        );
                        if sresp
                            .on_hover_text(if meta.fav { tr("取消收藏", "Unfavorite") } else { tr("收藏(置顶)", "Favorite (pin to top)") })
                            .clicked()
                        {
                            fav = true;
                        }
                        if pencil_button(ui).on_hover_text(tr("改名", "Rename")).clicked() {
                            rn = true;
                        }
                        if refresh_button(ui, 14.0)
                            .on_hover_text(tr("更新: 用当前状态覆盖这一项(需确认)", "Update: overwrite this item with the current state (confirm)"))
                            .clicked()
                        {
                            upd = true;
                        }
                        let (drect, dresp) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
                        let dcol = if dresp.hovered() { COL_RED } else { COL_MUTED };
                        let p = |x: f32, y: f32| egui::pos2(drect.left() + x, drect.top() + y);
                        let st = egui::Stroke::new(1.2f32, dcol);
                        let segs: [((f32, f32), (f32, f32)); 7] = [
                            ((3.0, 4.0), (11.0, 4.0)),
                            ((5.5, 4.0), (5.5, 2.5)),
                            ((8.5, 4.0), (8.5, 2.5)),
                            ((6.2, 2.5), (7.8, 2.5)),
                            ((4.0, 4.0), (5.0, 12.0)),
                            ((10.0, 4.0), (9.0, 12.0)),
                            ((5.0, 12.0), (9.0, 12.0)),
                        ];
                        for (a, b) in segs {
                            ui.painter().line_segment([p(a.0, a.1), p(b.0, b.1)], st);
                        }
                        if dresp.on_hover_text(tr("删除(需确认)", "Delete (confirm)")).clicked() {
                            del = true;
                        }
                    });
                });
                (del, fav, rn, upd)
            };
            let mut route = |meta: &StateMeta, (del, fav, rn, upd): (bool, bool, bool, bool)| {
                if del {
                    delete_ask = Some(meta.name.clone());
                }
                if fav {
                    fav_toggle = Some((meta.name.clone(), !meta.fav));
                }
                if rn {
                    rename_start = Some(meta.name.clone());
                }
                if upd {
                    update_ask = Some(meta.name.clone());
                }
            };
            if !fav_list.is_empty() {
                ui.label(egui::RichText::new(tr("★ 收藏", "★ Favorites")).small().strong().color(COL_MUTED));
                let fav_img_h = fav_img_h_for(cell_w);
                let fav_row_h = fav_img_h + 26.0;
                for trio in fav_list.chunks(3) {
                    let row_rect = ui.allocate_exact_size(egui::vec2(avail_w, fav_row_h), egui::Sense::hover()).0;
                    for (ci, meta) in trio.iter().enumerate() {
                        let cell = egui::Rect::from_min_size(
                            egui::pos2(row_rect.left() + ci as f32 * (cell_w + sp), row_rect.top()),
                            egui::vec2(cell_w, fav_row_h),
                        );
                        let r = ui.allocate_new_ui(egui::UiBuilder::new().max_rect(cell), |ui| item(ui, meta, cell_w, fav_img_h)).inner;
                        route(meta, r);
                    }
                }
                ui.separator();
            }
            if !other_list.is_empty() {
                ui.label(egui::RichText::new(tr("全部", "All")).small().strong().color(COL_MUTED));
                for meta in &other_list {
                    let r = item(ui, meta, avail_w, big_img_h_for(avail_w));
                    ui.add_space(4.0);
                    route(meta, r);
                }
            }
        });
    // 删除确认(弹层)
    if let Some(name) = wall.delete_confirm.clone() {
        let q = t!("删除「{name}」? 不可恢复", "Delete \"{name}\"? Cannot be undone");
        match confirm_popup(ui.ctx(), &format!("{}-del", text.id), &q) {
            Some(true) => {
                match store.delete(doc_key, &[name.clone()]) {
                    Ok(_) => {
                        wall.loaded = false;
                        wall.pngs.remove(&name);
                        out.notices.push((false, t!("已删除「{name}」", "Deleted \"{name}\"")));
                    }
                    Err(e) => out.notices.push((true, e)),
                }
                wall.delete_confirm = None;
            }
            Some(false) => wall.delete_confirm = None,
            None => {}
        }
    }
    if let Some(n) = delete_ask {
        wall.delete_confirm = Some(n);
    }
    // 更新确认 — 确认后交调用方采集当前状态 + 缩略图, 覆盖这一项
    if let Some(name) = wall.update_confirm.clone() {
        let q = (text.update_confirm)(&name);
        match confirm_popup_with(ui.ctx(), &format!("{}-upd", text.id), &q, tr("确认更新", "Update"), COL_ACCENT) {
            Some(true) => {
                wall.pngs.remove(&name);
                out.update = Some(name);
                wall.update_confirm = None;
            }
            Some(false) => wall.update_confirm = None,
            None => {}
        }
    }
    if let Some(n) = update_ask {
        wall.update_confirm = Some(n);
    }
    if let Some((name, fav)) = fav_toggle {
        if let Err(e) = store.set_fav(doc_key, &name, fav) {
            out.notices.push((true, e));
        }
        wall.loaded = false;
    }
    if rename_cancel {
        wall.rename = None;
    }
    if let Some(old) = rename_start {
        wall.rename_buf = old.clone();
        wall.rename = Some(old);
    }
    if rename_commit {
        if let Some(old) = wall.rename.clone() {
            match store.rename(doc_key, &old, &wall.rename_buf) {
                Ok(_) => {
                    wall.loaded = false;
                    wall.pngs.remove(&old);
                    out.notices.push((false, t!("已改名: {old} → {}", "Renamed: {old} → {}", wall.rename_buf.trim())));
                }
                Err(e) => out.notices.push((true, e)),
            }
        }
        wall.rename = None;
    }
    out.hint = inner_tip.into_inner();
    out
}
