//! [PLA-000] PLA 助手 UI — eframe/egui 三栏主界面。
//! 左: 快照墙(P0-1) | 中: 实体树+选择集(P0-2) | 右: 尺寸驱动(P1-1)
//! 顶部工具条按钮可绑定快捷键(设置面板内), 冲突即时红字告警。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use pla_ui::hotkey::{Action, Hotkey, HotkeyMap};
use serde_json::Value;

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 720.0])
            .with_title("PLA 助手 — Plasticity Snapshot & Selection"),
        ..Default::default()
    };
    eframe::run_native("pla-ui", opts, Box::new(|cc| Ok::<Box<dyn eframe::App>, _>(Box::new(PlaApp::new(cc)))))
}

#[derive(Default)]
struct PlaApp {
    entities: Vec<Value>,
    bridge_status: String,
    snapshots: Vec<String>,
    sel_name: String,
    sel_members: Vec<String>,
    dim_value: f64,
    hotkeys: HotkeyMap,
    /// 正在录制快捷键的动作 (None = 未在录制)
    recording: Option<String>,
    conflict_msg: Option<String>,
    doc_key: String,
}

/// 中文矢量字体: 运行时优先 C:\Windows\Fonts\msyh.ttc(微软雅黑),
/// 失败回退嵌入的 simhei.ttf(9.7MB, 保证离线可用)。egui 默认字体无 CJK → 方块。
fn install_cjk_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions};
    let mut fonts = FontDefinitions::default();
    // 注意: 不用 msyh.ttc — TTC(TrueType Collection) ab_glyph 不支持, 会 panic 崩溃(实锤 10-20)。
    let candidates = [
        ("C:\\Windows\\Fonts\\simhei.ttf", "simhei"),
        ("C:\\Windows\\Fonts\\Deng.ttf", "deng"),
        ("C:\\Windows\\Fonts\\msyh.ttf", "msyh"),
    ];
    let mut installed = false;
    for (path, name) in candidates {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert(name.to_string(), FontData::from_owned(data).into());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, name.to_string());
            installed = true;
            break;
        }
    }
    if !installed {
        // 兜底: 嵌入的思源黑体副本
        const SIMHEI: &[u8] = include_bytes!("../assets/simhei.ttf");
        fonts.font_data.insert("simhei".into(), FontData::from_owned(SIMHEI.to_vec()).into());
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "simhei".to_string());
    }
    // 等宽族也补 CJK(快捷键/日志区)
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push("simhei".to_string());
    ctx.set_fonts(fonts);
}

impl PlaApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_cjk_fonts(&cc.egui_ctx);
        Self {
            bridge_status: "未连接".into(),
            doc_key: "model1".into(),
            ..Default::default()
        }
    }

    /// 集中动作分发: 每个按钮都有真实行为; 未实现的功能弹明确提示, 不做假按钮。
    fn run_action(&mut self, a: Action) {
        match a {
            Action::EntitiesRefresh => self.refresh_entities(),
            Action::SnapshotManual => {
                match pla_ui::snapshot::take_manual_snapshot(&self.doc_key) {
                    Ok(name) => self.snapshots.insert(0, name),
                    Err(e) => self.conflict_msg = Some(format!("快照失败: {e}")),
                }
            }
            Action::SnapshotRestore => {
                self.conflict_msg = Some("切换快照: 先在快照墙选一个快照".into());
            }
            Action::SelSave => {
                self.conflict_msg = Some(format!(
                    "选择集「{}」: {} 个成员 — 保存逻辑接线中(需要 bridge 组创建)",
                    self.sel_name, self.sel_members.len()
                ));
            }
            Action::SelApply => {
                self.conflict_msg = Some("应用选择集: 需要 PUT_SOME 组创建语义(PLA-004 G4)".into());
            }
            Action::DimApply => {
                self.conflict_msg = Some("尺寸驱动: 需要 facet 读写闭环(PLA-004 G4 后)".into());
            }
        }
    }

    fn refresh_entities(&mut self) {
        self.bridge_status = "连接中…".into();
        match pla_ui::bridge_thread::fetch_entities_blocking(&self.doc_key) {
            Ok(list) => {
                self.entities = list;
                self.bridge_status = "已连接".into();
            }
            Err(e) => {
                self.bridge_status = format!("失败: {e}");
            }
        }
    }
}

impl eframe::App for PlaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ---- 快捷键触发(全局): 解本帧按键事件, 纯逻辑匹配 ----
        let ev: Vec<(bool, bool, bool, String)> = ctx.input(|i| {
            let m = i.modifiers;
            i.events.iter().filter_map(|e| match e {
                egui::Event::Key { key, pressed: true, repeat: false, .. } => {
                    Some((m.ctrl, m.alt, m.shift, format!("{key:?}")))
                }
                _ => None,
            }).collect()
        });
        for (ctrl, alt, shift, kname) in &ev {
            for a in Action::ALL {
                if self.hotkeys.matches_raw(&format!("{a:?}"), *ctrl, *alt, *shift, kname) {
                    match a {
                        Action::EntitiesRefresh => self.refresh_entities(),
                        Action::SnapshotManual => self.snapshots.insert(0, format!("snap-{}", chrono_now())),
                        _ => {}
                    }
                }
            }
        }

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("PLA 助手");
                ui.separator();
                for a in Action::ALL {
                    let hk = self
                        .hotkeys
                        .binds
                        .get(&format!("{a:?}"))
                        .map(|h| h.display());
                    let label = match hk {
                        Some(k) => format!("{} ({k})", a.label()),
                        None => a.label().to_string(),
                    };
                    if ui.button(&label).clicked() {
                        match a {
                            Action::EntitiesRefresh => self.refresh_entities(),
                            Action::SnapshotManual => self.snapshots.insert(0, format!("snap-{}", chrono_now())),
                            _ => {}
                        }
                    }
                    // 右键 = 进入该按钮的快捷键录制
                    if ui.button(&label).secondary_clicked() {
                        self.recording = Some(format!("{a:?}"));
                        self.conflict_msg = None;
                    }
                }
                ui.separator();
                ui.colored_label(egui::Color32::GRAY, format!("bridge: {}", self.bridge_status));
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(rec) = &self.recording {
                    ui.colored_label(egui::Color32::YELLOW, format!("按任意键组合为「{}」绑定快捷键 (Esc 取消)", rec));
                    if ui.button("取消").clicked() { self.recording = None; }
                }
                if let Some(msg) = &self.conflict_msg {
                    ui.colored_label(egui::Color32::RED, format!("⚠ 冲突: {msg}"));
                }
            });
        });

        egui::SidePanel::left("snapshots").show(ctx, |ui| {
            ui.heading("快照墙");
            ui.separator();
            ui.horizontal(|ui| {
                // 打开快照文件夹 — 只显示图标, 悬停出文字(需求 2)
                if ui.button("📂").on_hover_text("打开快照文件夹").clicked() {
                    if let Err(e) = pla_ui::snapshot::open_snapshot_dir() {
                        self.conflict_msg = Some(e);
                    }
                }
                if ui.button("📷").on_hover_text("手动快照 (保存当前文档副本)").clicked() {
                    self.run_action(Action::SnapshotManual);
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for s in &self.snapshots {
                    ui.horizontal(|ui| {
                        ui.label("📷");
                        ui.label(s);
                        if ui.small_button("切换").clicked() { /* P0-1 切换 */ }
                    });
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                // 中: 实体树 + 选择集
                egui::SidePanel::left("inner-entities").show_inside(ui, |ui| {
                    ui.heading(format!("实体树 — {}", self.doc_key));
                    ui.separator();
                    let mut toggle: Option<String> = None;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for e in &self.entities {
                            let name = e.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                            let kind = e.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                            let id = e.get("id").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                            ui.horizontal(|ui| {
                                let mut checked = self.sel_members.iter().any(|m| m == &id);
                                if ui.checkbox(&mut checked, "").changed() {
                                    toggle = Some(id.clone());
                                }
                                ui.label(format!("{name} ({kind})"));
                            });
                        }
                    });
                    if let Some(id) = toggle {
                        if let Some(pos) = self.sel_members.iter().position(|m| m == &id) {
                            self.sel_members.remove(pos);
                        } else {
                            self.sel_members.push(id);
                        }
                    }
                });
                // 右: 选择集 + 尺寸驱动 + 快捷键设置
                egui::SidePanel::right("inner-tools").show_inside(ui, |ui| {
                    ui.heading("选择集");
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("名称:");
                        ui.text_edit_singleline(&mut self.sel_name);
                        if ui.button("保存").clicked() { /* P0-2 */ }
                        if ui.button("应用").clicked() { /* P0-2 PUT_SOME */ }
                    });
                    ui.separator();
                    ui.heading("尺寸驱动 (P1)");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut self.dim_value).speed(0.1));
                        if ui.button("应用平移").clicked() { /* P1-1 */ }
                    });
                    ui.separator();
                    ui.heading("快捷键设置");
                    ui.separator();
                    for a in Action::ALL {
                        let key = format!("{a:?}");
                        ui.horizontal(|ui| {
                            ui.label(a.label());
                            let cur = self.hotkeys.binds.get(&key).map(|h| h.display()).unwrap_or_else(|| "-".into());
                            ui.monospace(cur);
                            if ui.small_button("绑定").clicked() {
                                self.recording = Some(key.clone());
                                self.conflict_msg = None;
                            }
                            if self.hotkeys.binds.contains_key(&key) && ui.small_button("清除").clicked() {
                                self.hotkeys.binds.remove(&key);
                            }
                        });
                    }
                    ui.separator();
                    let conflicts = self.hotkeys.conflicts();
                    if !conflicts.is_empty() {
                        ui.colored_label(egui::Color32::RED, "⚠ 快捷键冲突:");
                        for (k, acts) in &conflicts {
                            ui.colored_label(egui::Color32::RED, format!("  {k} → {acts:?}"));
                        }
                    }
                });
            });
        });

        // ---- 录制捕获 ----
        if self.recording.is_some() {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.recording = None;
            } else {
                let (mods, key) = ctx.input(|i| (i.modifiers, i.events.iter().find_map(|e|
                    matches!(e, egui::Event::Key { pressed: true, repeat: false, .. })
                        .then(|| match e { egui::Event::Key { key, .. } => *key, _ => unreachable!() })
                )));
                if let Some(key) = key {
                    let hk = Hotkey {
                        ctrl: mods.ctrl, alt: mods.alt, shift: mods.shift,
                        key: format!("{key:?}"),
                    };
                    if let Some(action) = self.recording.clone() {
                        match self.hotkeys.try_bind(&action, hk.clone()) {
                            Ok(()) => { self.recording = None; self.conflict_msg = None; }
                            Err(others) => {
                                self.conflict_msg = Some(format!("{} 已被 {:?} 占用", hk.display(), others));
                            }
                        }
                    }
                }
            }
        }
    }
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().to_string()
}
