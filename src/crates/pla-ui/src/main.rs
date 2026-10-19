//! [PLA-000] PLA Toolbar — Plasticity 风格置顶工具栏。
//!
//! 形态: 无边框横向工具栏, 自动吸附到 Plasticity 主窗口标题栏空白区
//! (以文件标题区右侧为左锚点), AlwaysOnTop。多实例一对一绑定多个
//! Plasticity 窗口(见 window_finder 模块注释)。
//!
//! 子视口: 悬停提示/功能面板/提示浮窗均为独立 OS 子窗口(egui 多视口),
//! 弹出在工具栏正下方, 跟随工具栏移动 — 30px 工具栏窗口自身装不下它们。
//!
//! Bug 修复: bridge 常驻后台线程 + mpsc 通道(不再 UI 线程短连接阻塞);
//! 工具栏按钮每功能只构建一次(clicked / secondary_clicked 分流)。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::{Duration, Instant};

use pla_ui::bridge_thread::{BridgeCmd, BridgeEvent, BridgeHandle};
use pla_ui::hotkey::{Action, Hotkey, HotkeyMap};
use pla_ui::round_corners;
use pla_ui::window_finder;

// ---------------- Plasticity 风格色板 ----------------
const COL_PANEL: egui::Color32 = egui::Color32::from_rgb(0x26, 0x26, 0x26);
const COL_BTN: egui::Color32 = egui::Color32::from_rgb(0x33, 0x33, 0x33);
const COL_BTN_HOVER: egui::Color32 = egui::Color32::from_rgb(0x3d, 0x3d, 0x3d);
/// Plasticity 图标方块绿(强调色)。
const COL_ACCENT: egui::Color32 = egui::Color32::from_rgb(0x3e, 0xa3, 0x4d);
const COL_TEXT: egui::Color32 = egui::Color32::from_rgb(0xcc, 0xcc, 0xcc);
const COL_MUTED: egui::Color32 = egui::Color32::from_rgb(0x8a, 0x8a, 0x8a);
const COL_YELLOW: egui::Color32 = egui::Color32::from_rgb(0xd9, 0xa8, 0x2c);
const COL_RED: egui::Color32 = egui::Color32::from_rgb(0xd0, 0x50, 0x45);
const COL_GRAY_DOT: egui::Color32 = egui::Color32::from_rgb(0x77, 0x77, 0x77);

// ---------------- 工具栏形态/吸附参数(可调) ----------------
/// 工具栏逻辑尺寸(横向一行按钮): 圆点+分隔线+6 按钮+分隔线+关闭键。
const TOOLBAR_SIZE: [f32; 2] = [264.0, 30.0];
/// 方形图标按钮边长。
const BTN_SIZE: f32 = 24.0;
/// 吸附定位(物理像素): 优先像素探测 Plasticity 顶栏「文件名文字块」右缘
/// (titlebar_probe, PrintWindow 离屏渲染不受本工具栏遮挡), 工具栏停在其右侧;
/// 探测失败(渲染失败/找不到文字块)降级为本固定偏移 — 预留足够距离越过文件名。
const SNAP_LEFT_OFFSET: f32 = 500.0;
/// 探测成功时, 文件名块右缘与工具栏左缘的间隙(物理像素)。
const SNAP_FILENAME_GAP: f32 = pla_ui::titlebar_probe::FILENAME_GAP_PX;
/// 吸附: 相对目标窗口顶边的下移(物理像素)。
const SNAP_TOP_GAP: i32 = 2;
/// 吸附/绑定轮询间隔。
const SNAP_POLL_MS: u64 = 300;
/// 文件名探测间隔(PrintWindow 有渲染开销, 比吸附轮询慢一档)。
const PROBE_POLL_MS: u64 = 1000;
/// 位置漂移小于该值(物理像素)不重发定位命令, 避免抖动。
const SNAP_EPS: f32 = 2.0;
/// 提示浮窗自动关闭时长。
const NOTICE_TTL: Duration = Duration::from_secs(6);
/// Win32 SetWindowRgn 圆角轮询间隔(子视口懒创建, 需定时补套)。
const ROUND_POLL_MS: u64 = 500;

/// 全局圆角(工具栏/面板/按钮一致)。
const ROUNDING: f32 = 6.0;

/// 工具栏按钮列(横向排列, 顺序即产品顺序)。
const TOOLBAR_ACTIONS: [Action; 6] = [
    Action::EntitiesRefresh,
    Action::SnapshotManual,
    Action::ToggleSnapshots,
    Action::ToggleEntities,
    Action::ToggleDim,
    Action::ToggleSettings,
];

/// 面板逻辑尺寸 (宽, 高) — 子视口 OS 窗口大小。
const PANEL_SNAPSHOTS: [f32; 2] = [280.0, 380.0];
const PANEL_ENTITIES: [f32; 2] = [280.0, 410.0];
const PANEL_DIM: [f32; 2] = [280.0, 170.0];
const PANEL_SETTINGS: [f32; 2] = [300.0, 470.0];

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(TOOLBAR_SIZE)
            .with_title("PLA Toolbar")
            .with_resizable(false)
            .with_decorations(false)
            .with_transparent(true)
            .with_window_level(egui::WindowLevel::AlwaysOnTop),
        ..Default::default()
    };
    eframe::run_native(
        "pla-ui",
        opts,
        Box::new(|cc| Ok::<Box<dyn eframe::App>, _>(Box::new(PlaApp::new(cc)))),
    )
}

struct PlaApp {
    /// 常驻 bridge 线程句柄(mpsc 收发)。
    bridge: BridgeHandle,
    bridge_status: String,
    /// 吸附目标(未绑定 = None)。
    snap_target: Option<window_finder::TargetWindow>,
    /// 上次吸附定位使用的目标锚点 (left, top, x_phys), 用于漂移检测。
    /// x_phys 含探测结果 — 文件名长度变化(切换文档)也会触发重定位。
    last_snap_anchor: Option<(i32, i32, i32)>,
    last_snap_poll: Instant,
    /// 文件名探测缓存: (目标 hwnd, 文件名块右缘物理 x)。None = 未探测到(降级偏移)。
    probed_edge: Option<(isize, i32)>,
    last_probe_poll: Instant,
    entities: Vec<serde_json::Value>,
    snapshots: Vec<String>,
    show_snapshots: bool,
    show_entities: bool,
    show_dim: bool,
    show_settings: bool,
    sel_name: String,
    sel_members: Vec<String>,
    dim_value: f64,
    hotkeys: HotkeyMap,
    /// 正在录制快捷键的动作名 (None = 未在录制)。
    recording: Option<String>,
    /// 面板内提示: (是否错误, 文本)。诚实提示原则: 未接线必须弹明确文字。
    notice: Option<(bool, String)>,
    /// 提示浮窗出现时刻(自动关闭用)。
    notice_at: Instant,
    /// 本帧悬停按钮的提示 (第一行, 第二行); None = 无悬停。
    hovered_hint: Option<(String, String)>,
    doc_key: String,
    /// Win32 窗口圆角裁剪状态(已套 Rgn 的 HWND + 尺寸)。
    round_state: round_corners::RoundState,
    /// 上次圆角轮询时刻。
    last_round_poll: Instant,
}

impl Drop for PlaApp {
    fn drop(&mut self) {
        self.bridge.send(BridgeCmd::Shutdown);
    }
}

/// 中文矢量字体: 运行时优先 C:\Windows\Fonts\simhei.ttf → Deng.ttf → msyh.ttf,
/// 失败回退嵌入的 simhei.ttf 兜底。注意: 禁用任何 .ttc —
/// TTC(TrueType Collection) ab_glyph 不支持, 会 panic 崩溃(实锤 10-20)。
fn install_cjk_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions};
    let mut fonts = FontDefinitions::default();
    let candidates = [
        ("C:\\Windows\\Fonts\\simhei.ttf", "simhei"),
        ("C:\\Windows\\Fonts\\Deng.ttf", "deng"),
        ("C:\\Windows\\Fonts\\msyh.ttf", "msyh"),
    ];
    let mut installed = false;
    for (path, name) in candidates {
        if let Ok(data) = std::fs::read(path) {
            fonts
                .font_data
                .insert(name.to_string(), FontData::from_owned(data).into());
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
        const SIMHEI: &[u8] = include_bytes!("../assets/simhei.ttf");
        fonts
            .font_data
            .insert("simhei".into(), FontData::from_owned(SIMHEI.to_vec()).into());
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "simhei".to_string());
    }
    // 等宽族也补 CJK(快捷键/日志区)。
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push("simhei".to_string());
    ctx.set_fonts(fonts);
}

/// Plasticity 风格样式: 深灰面板/按钮、绿色强调、统一圆角、弱阴影、小字号。
fn apply_plasticity_style(ctx: &egui::Context) {
    const SW: f32 = 1.0; // stroke 宽度
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    v.dark_mode = true;
    v.panel_fill = COL_PANEL;
    v.window_fill = COL_PANEL;
    v.extreme_bg_color = egui::Color32::from_rgb(0x1e, 0x1e, 0x1e);
    v.faint_bg_color = egui::Color32::from_rgb(0x2e, 0x2e, 0x2e);
    v.widgets.noninteractive.bg_fill = COL_PANEL;
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(SW, COL_TEXT);
    v.widgets.inactive.bg_fill = COL_BTN;
    v.widgets.inactive.weak_bg_fill = COL_BTN;
    v.widgets.inactive.fg_stroke = egui::Stroke::new(SW, egui::Color32::WHITE);
    v.widgets.hovered.bg_fill = COL_BTN_HOVER;
    v.widgets.hovered.weak_bg_fill = COL_BTN_HOVER;
    v.widgets.hovered.fg_stroke = egui::Stroke::new(SW, egui::Color32::WHITE);
    v.widgets.active.bg_fill = COL_ACCENT;
    v.widgets.active.weak_bg_fill = COL_ACCENT;
    v.widgets.active.fg_stroke = egui::Stroke::new(SW, egui::Color32::WHITE);
    v.widgets.open.bg_fill = COL_BTN_HOVER;
    v.widgets.open.weak_bg_fill = COL_BTN_HOVER;
    v.selection.bg_fill = COL_ACCENT;
    v.selection.stroke = egui::Stroke::new(SW, egui::Color32::WHITE);
    // 统一圆角: 逐 widget 类设置 + 窗口圆角。
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = egui::Rounding::same(ROUNDING);
    }
    v.window_rounding = egui::Rounding::same(ROUNDING);
    v.window_stroke = egui::Stroke::new(SW, egui::Color32::from_rgb(0x44, 0x44, 0x44));
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;
    v.button_frame = true;
    // 小字号(12-13)。
    style.text_styles = [
        (egui::TextStyle::Heading, egui::FontId::proportional(14.0)),
        (egui::TextStyle::Body, egui::FontId::proportional(12.5)),
        (egui::TextStyle::Button, egui::FontId::proportional(12.0)),
        (egui::TextStyle::Small, egui::FontId::proportional(10.0)),
        (egui::TextStyle::Monospace, egui::FontId::monospace(12.0)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    ctx.set_style(style);
}

/// bridge 状态文本 → 圆点颜色。
fn status_color(status: &str) -> egui::Color32 {
    if status.starts_with("已连接") {
        COL_ACCENT
    } else if status.starts_with("连接中") {
        COL_YELLOW
    } else {
        COL_RED
    }
}

/// 节标题: 全大写浅灰小字(文本由调用方保证大写)。
fn section_header(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .small()
            .strong()
            .color(COL_MUTED),
    );
}

/// 工具栏/子视口共用的圆角深色面板 Frame。
fn panel_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(COL_PANEL)
        .rounding(egui::Rounding::same(ROUNDING))
        .inner_margin(egui::Margin::same(6.0))
}

/// 子视口面板头: 标题 + 右侧 ✕ 关闭钮。
fn panel_header(ui: &mut egui::Ui, title: &str, open: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).size(13.0).strong().color(COL_TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = egui::Button::new(egui::RichText::new("✕").color(egui::Color32::WHITE))
                .frame(false)
                .rounding(egui::Rounding::same(4.0));
            if ui.add(close).clicked() {
                *open = false;
            }
        });
    });
    ui.separator();
}

impl PlaApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_cjk_fonts(&cc.egui_ctx);
        apply_plasticity_style(&cc.egui_ctx);
        Self {
            bridge: BridgeHandle::spawn(),
            bridge_status: "连接中…".into(),
            snap_target: None,
            last_snap_anchor: None,
            last_snap_poll: Instant::now() - Duration::from_millis(SNAP_POLL_MS),
            probed_edge: None,
            last_probe_poll: Instant::now() - Duration::from_millis(PROBE_POLL_MS),
            entities: Vec::new(),
            snapshots: pla_ui::snapshot::list_snapshots(),
            show_snapshots: false,
            show_entities: false,
            show_dim: false,
            show_settings: false,
            sel_name: String::new(),
            sel_members: Vec::new(),
            dim_value: 0.0,
            hotkeys: HotkeyMap::default(),
            recording: None,
            notice: None,
            notice_at: Instant::now(),
            hovered_hint: None,
            doc_key: "model1".into(),
            round_state: round_corners::RoundState::new(),
            last_round_poll: Instant::now() - Duration::from_millis(ROUND_POLL_MS),
        }
    }

    /// 非阻塞收取 bridge 线程事件(每帧调用)。
    fn poll_bridge(&mut self) {
        while let Some(ev) = self.bridge.poll() {
            match ev {
                BridgeEvent::Status(s) => self.bridge_status = s,
                BridgeEvent::Entities(list) => self.entities = list,
                BridgeEvent::Error(e) => {
                    self.notice = Some((true, e));
                    self.notice_at = Instant::now();
                }
            }
        }
        // 提示浮窗自动关闭。
        if self.notice.is_some() && self.notice_at.elapsed() > NOTICE_TTL {
            self.notice = None;
        }
    }

    /// 集中动作分发: 每个按钮都有真实行为; 未实现的功能弹明确提示, 不做假按钮。
    fn run_action(&mut self, a: Action) {
        match a {
            Action::EntitiesRefresh => self.bridge.send(BridgeCmd::RefreshEntities),
            Action::SnapshotManual => match pla_ui::snapshot::take_manual_snapshot(&self.doc_key) {
                Ok(name) => {
                    self.snapshots.insert(0, name);
                    self.notice = Some((false, "快照已保存到 snapshots/ 目录".into()));
                }
                Err(e) => self.notice = Some((true, e)),
            },
            Action::SnapshotRestore => {
                self.notice = Some((true, "切换快照未接线: 恢复写入需要 PLA-004 G4 闭环".into()));
            }
            Action::SelSave => {
                let doc = self.doc_key.clone();
                let name = self.sel_name.clone();
                let members = self.sel_members.clone();
                match pla_ui::selection::save_selection(&doc, &name, &members) {
                    Ok(path) => self.notice = Some((false, format!("选择集已保存: {path}"))),
                    Err(e) => self.notice = Some((true, e)),
                }
            }
            Action::SelApply => {
                self.notice = Some((true, "应用选择集需要 PUT_SOME 组创建语义(PLA-004 G4), 未接线".into()));
            }
            Action::DimApply => {
                self.notice = Some((true, "尺寸驱动需要 facet 读写闭环(PLA-004 G4 后), 未接线".into()));
            }
            Action::ToggleSnapshots => self.show_snapshots = !self.show_snapshots,
            Action::ToggleEntities => self.show_entities = !self.show_entities,
            Action::ToggleDim => self.show_dim = !self.show_dim,
            Action::ToggleSettings => self.show_settings = !self.show_settings,
        }
    }

    fn set_notice(&mut self, n: (bool, String)) {
        self.notice = Some(n);
        self.notice_at = Instant::now();
    }

    /// 吸附定位: 轮询目标窗口, 需要时发 OuterPosition(egui points)。
    /// 横向定位优先像素探测文件名块右缘(R6), 探测失败降级 SNAP_LEFT_OFFSET。
    /// 锚点 = (目标 left, top, 本轮 x_phys) — 三者任一漂移超 SNAP_EPS 才重发。
    fn update_snap(&mut self, ctx: &egui::Context) {
        if self.last_snap_poll.elapsed() < Duration::from_millis(SNAP_POLL_MS) {
            return;
        }
        self.last_snap_poll = Instant::now();
        let new_target = window_finder::acquire_target();
        self.snap_target = new_target;
        // 用 owned 克隆避免在 self 上叠加可变借用。
        let Some(target) = self.snap_target.clone() else {
            // 未绑定: 清缓存, 等 Plasticity 出现。
            self.last_snap_anchor = None;
            self.probed_edge = None;
            return;
        };
        // 文件名探测: 换绑定窗口立即探, 同窗口按 PROBE_POLL_MS 节流
        // (文件名长度随文档切换变化, 需周期性刷新)。
        if self.last_probe_poll.elapsed() >= Duration::from_millis(PROBE_POLL_MS)
            || self.probed_edge.as_ref().map_or(true, |(h, _)| *h != target.hwnd)
        {
            let edge = pla_ui::titlebar_probe::probe_filename_right_edge(&target, ctx.pixels_per_point());
            self.probed_edge = edge.map(|e| (target.hwnd, e));
            self.last_probe_poll = Instant::now();
        }
        // 物理 x: 探测命中 → 文件名右缘+间隙; 否则降级固定偏移。
        // 两者都按 ppp 缩放: probe/GetWindowRect 在 DPI-aware 进程里是物理像素,
        // 固定常量是逻辑 px, 不缩放在高 DPI 下会缩水遮文件名(R6 实测 4K/3x)。
        let ppp = ctx.pixels_per_point();
        let x_phys = match self.probed_edge {
            Some((h, edge)) if h == target.hwnd => edge as f32 + SNAP_FILENAME_GAP * ppp,
            _ => target.left as f32 + SNAP_LEFT_OFFSET * ppp,
        };
        let new_anchor = Some((target.left, target.top, x_phys as i32));
        let anchor_changed = match (new_anchor, self.last_snap_anchor) {
            (Some(n), Some(o)) => {
                (n.0 - o.0).abs() as f32 > SNAP_EPS
                    || (n.1 - o.1).abs() as f32 > SNAP_EPS
                    || (n.2 - o.2).abs() as f32 > SNAP_EPS
            }
            _ => true,
        };
        if !anchor_changed {
            return;
        }
        self.last_snap_anchor = new_anchor;
        // 物理 → egui points: OuterPosition 用 points(见 egui 0.29 ViewportInfo)。
        let y_phys = target.top as f32 + SNAP_TOP_GAP as f32;
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            x_phys / ppp,
            y_phys / ppp,
        )));
    }
}

/// 子视口渲染的公共入口: Immediate 模式 = 独立 OS 窗口(圆角深色面板);
/// Embedded 回退(后端不支持多视口) = 原地 egui::Window, 行为不劣化。
/// 面板位置每帧跟随工具栏(漂移 >2px 重发 OuterPosition), 关闭走 ✕ 或 OS 关闭。
fn show_child_window(
    parent: &egui::Context,
    id: &'static str,
    title: &'static str,
    desired: egui::Pos2,
    size: egui::Vec2,
    open: &mut bool,
    active: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let mut flag = *open;
    let mut contents = Some(add_contents);
    let builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_decorations(false)
        .with_resizable(false)
        .with_transparent(true)
        .with_window_level(egui::WindowLevel::AlwaysOnTop)
        .with_active(active)
        .with_position(desired)
        .with_inner_size(size);
    parent.show_viewport_immediate(
        egui::ViewportId::from_hash_of(id),
        builder,
        // 非 move: flag 借用进闭包, ✕/OS 关闭的结果能在调用后传回。
        |ctx, class| {
            if ctx.input(|i| i.viewport().close_requested()) {
                flag = false;
            }
            let Some(contents) = contents.take() else { return };
            match class {
                // 回退: 后端不支持多视口时嵌进当前视口。
                egui::ViewportClass::Embedded | egui::ViewportClass::Root | egui::ViewportClass::Deferred => {
                    egui::Window::new(title)
                        .open(&mut flag)
                        .resizable(false)
                        .collapsible(false)
                        .default_size(size)
                        .show(ctx, |ui| contents(ui));
                }
                egui::ViewportClass::Immediate => {
                    egui::CentralPanel::default()
                        .frame(panel_frame())
                        .show(ctx, |ui| {
                            panel_header(ui, title, &mut flag);
                            contents(ui);
                        });
                    // 跟随工具栏: 外框漂移超阈值即重定位。
                    let cur = ctx.input(|i| i.viewport().outer_rect);
                    if let Some(r) = cur {
                        if (r.left() - desired.x).abs() > SNAP_EPS
                            || (r.top() - desired.y).abs() > SNAP_EPS
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(desired));
                        }
                    }
                }
            }
        },
    );
    *open = flag;
}

impl eframe::App for PlaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_bridge();
        self.update_snap(ctx);
        // 圆角裁剪: egui 画的圆角在 Windows glow 后端透明不可靠(R5 实测只有
        // 左上角圆), 由 SetWindowRgn 兜底; 子视口懒创建, 定时枚举补套。
        if self.last_round_poll.elapsed() >= Duration::from_millis(ROUND_POLL_MS) {
            self.last_round_poll = Instant::now();
            round_corners::poll_round_windows(&mut self.round_state);
        }

        // ---- 快捷键触发(全局): 解本帧按键事件, 纯逻辑匹配 ----
        if self.recording.is_none() {
            let keys: Vec<(bool, bool, bool, String)> = ctx.input(|i| {
                let m = i.modifiers;
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Key { key, pressed: true, repeat: false, .. } => {
                            Some((m.ctrl, m.alt, m.shift, format!("{key:?}")))
                        }
                        _ => None,
                    })
                    .collect()
            });
            for (ctrl, alt, shift, kname) in keys {
                for a in Action::ALL {
                    if self.hotkeys.matches_raw(&format!("{a:?}"), ctrl, alt, shift, &kname) {
                        self.run_action(a);
                    }
                }
            }
        }

        // ---- 横向工具栏本体(单按钮单构建 — Bug 2 修复) ----
        let mut hovered: Option<(String, String)> = None;
        egui::CentralPanel::default()
            .frame(panel_frame())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // 状态圆点区: 未绑定=灰; 已绑定显示 bridge 状态色。
                    let (dot_rect, dot_resp) =
                        ui.allocate_exact_size(egui::vec2(12.0, BTN_SIZE), egui::Sense::hover());
                    let dot_color = if self.snap_target.is_none() {
                        COL_GRAY_DOT
                    } else {
                        status_color(&self.bridge_status)
                    };
                    ui.painter().circle_filled(dot_rect.center(), 4.0, dot_color);
                    let bind_tip = match &self.snap_target {
                        Some(t) => format!("已绑定: {}\nbridge: {}/{}", t.title, self.bridge_status, self.bridge.url),
                        None => "未绑定 — 等待 Plasticity 窗口…\n(找到后自动吸附到其标题栏)".to_string(),
                    };
                    if dot_resp.hovered() {
                        hovered = Some((
                            "PLA 助手".into(),
                            match &self.snap_target {
                                Some(t) => format!("已绑定: {} / {}", t.title, self.bridge_status),
                                None => "未绑定 — 等待 Plasticity 窗口…".into(),
                            },
                        ));
                    }
                    let _ = bind_tip;
                    ui.separator();

                    for a in TOOLBAR_ACTIONS {
                        let active = match a {
                            Action::ToggleSnapshots => self.show_snapshots,
                            Action::ToggleEntities => self.show_entities,
                            Action::ToggleDim => self.show_dim,
                            Action::ToggleSettings => self.show_settings,
                            _ => false,
                        };
                        let hk = self
                            .hotkeys
                            .binds
                            .get(&format!("{a:?}"))
                            .map(|h| h.display())
                            .unwrap_or_else(|| "未绑定".into());
                        // 关键: 每个功能只构建一次按钮。
                        // (旧代码 clicked / secondary_clicked 各 add 一个 → 屏幕上渲染两份)
                        let btn = egui::Button::new(
                            // 图标纯白 — 深灰底上足够醒目(反差修复)。
                            egui::RichText::new(a.icon()).size(13.0).color(egui::Color32::WHITE),
                        )
                        .min_size(egui::vec2(BTN_SIZE, BTN_SIZE))
                        .rounding(egui::Rounding::same(ROUNDING))
                        // 选中(面板开着)=绿底; 常态=深灰底。
                        .fill(if active { COL_ACCENT } else { COL_BTN });
                        let resp = ui.add(btn);
                        if resp.hovered() {
                            hovered = Some((
                                format!("{} {}", a.icon(), a.label()),
                                format!("快捷键: {hk} / 右键设置"),
                            ));
                        }
                        if resp.clicked() {
                            self.run_action(a);
                        }
                        // 右键 = 进入该按钮的快捷键录制。
                        if resp.secondary_clicked() {
                            self.recording = Some(format!("{a:?}"));
                            self.notice = None;
                        }
                    }

                    ui.separator();
                    // 关闭按钮: 退出助手工具栏。
                    let close_btn = egui::Button::new(
                        egui::RichText::new("✕").size(13.0).color(egui::Color32::WHITE),
                    )
                    .min_size(egui::vec2(BTN_SIZE, BTN_SIZE))
                    .rounding(egui::Rounding::same(ROUNDING))
                    .fill(COL_BTN);
                    let close_resp = ui.add(close_btn);
                    if close_resp.hovered() {
                        hovered = Some(("关闭".into(), "退出 PLA 助手工具栏".into()));
                    }
                    if close_resp.clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        self.hovered_hint = hovered;

        // ---- 工具栏外框基准位置(子视口都锚在它正下方) ----
        let toolbar_bottom_left = ctx.input(|i| {
            i.viewport()
                .outer_rect
                .map(|r| egui::pos2(r.left() + 2.0, r.bottom() + 6.0))
        });

        // ---- 悬停提示: 子视口弹出在工具栏正下方(不遮按钮) ----
        if let Some((line1, line2)) = self.hovered_hint.clone() {
            if let Some(base) = toolbar_bottom_left {
                let size = egui::vec2(260.0, 54.0);
                let desired = egui::pos2(base.x, base.y);
                let mut contents = Some(());
                let builder = egui::ViewportBuilder::default()
                    .with_decorations(false)
                    .with_resizable(false)
                    .with_transparent(true)
                    .with_window_level(egui::WindowLevel::AlwaysOnTop)
                    .with_active(false) // 不抢焦点
                    .with_position(desired)
                    .with_inner_size(size);
                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("hover-tooltip"),
                    builder,
                    move |tctx, class| {
                        let Some(()) = contents.take() else { return };
                        let show_ui = |ui: &mut egui::Ui| {
                            ui.label(egui::RichText::new(&line1).size(12.5).strong().color(egui::Color32::WHITE));
                            ui.label(
                                egui::RichText::new(&line2)
                                    .size(11.5)
                                    .color(egui::Color32::from_rgb(0x9a, 0x9a, 0x9a)),
                            );
                        };
                        match class {
                            egui::ViewportClass::Embedded | egui::ViewportClass::Root | egui::ViewportClass::Deferred => {
                                egui::Window::new("提示")
                                    .resizable(false)
                                    .collapsible(false)
                                    .show(tctx, |ui| show_ui(ui));
                            }
                            egui::ViewportClass::Immediate => {
                                egui::CentralPanel::default()
                                    .frame(panel_frame())
                                    .show(tctx, |ui| show_ui(ui));
                                let cur = tctx.input(|i| i.viewport().outer_rect);
                                if let Some(r) = cur {
                                    if (r.left() - desired.x).abs() > SNAP_EPS
                                        || (r.top() - desired.y).abs() > SNAP_EPS
                                    {
                                        tctx.send_viewport_cmd(
                                            egui::ViewportCommand::OuterPosition(desired),
                                        );
                                    }
                                }
                            }
                        }
                    },
                );
            }
        }

        // ---- 功能面板: 子视口弹出在工具栏正下方(纵向堆叠) ----
        let mut stack_y: Option<f32> = toolbar_bottom_left.map(|p| p.y);
        let stack_x = toolbar_bottom_left.map(|p| p.x);
        let next_pos = |stack_y: &mut Option<f32>, h: f32| -> Option<egui::Pos2> {
            let y = (*stack_y)?;
            let p = egui::pos2(stack_x.unwrap_or(y), y);
            *stack_y = Some(y + h + 8.0);
            Some(p)
        };

        if self.show_snapshots {
            let desired = next_pos(&mut stack_y, PANEL_SNAPSHOTS[1]);
            if let Some(desired) = desired {
                let mut flag = self.show_snapshots;
                show_child_window(
                    ctx,
                    "panel-snapshots",
                    "快照墙",
                    desired,
                    egui::vec2(PANEL_SNAPSHOTS[0], PANEL_SNAPSHOTS[1]),
                    &mut flag,
                    true,
                    |ui| {
                        section_header(ui, "SNAPSHOTS");
                        ui.horizontal(|ui| {
                            if ui.button("📂 打开文件夹").clicked() {
                                if let Err(e) = pla_ui::snapshot::open_snapshot_dir() {
                                    self.set_notice((true, e));
                                }
                            }
                            if ui.button("📷 手动快照").clicked() {
                                self.run_action(Action::SnapshotManual);
                            }
                        });
                        ui.separator();
                        egui::ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
                            if self.snapshots.is_empty() {
                                ui.label(egui::RichText::new("(暂无快照)").color(COL_MUTED));
                            }
                            for s in &self.snapshots {
                                ui.horizontal(|ui| {
                                    ui.label("📷");
                                    ui.label(s);
                                });
                            }
                        });
                    },
                );
                self.show_snapshots = flag;
            }
        }

        if self.show_entities {
            let desired = next_pos(&mut stack_y, PANEL_ENTITIES[1]);
            if let Some(desired) = desired {
                let mut flag = self.show_entities;
                show_child_window(
                    ctx,
                    "panel-entities",
                    "实体树 / 选择集",
                    desired,
                    egui::vec2(PANEL_ENTITIES[0], PANEL_ENTITIES[1]),
                    &mut flag,
                    true,
                    |ui| {
                        section_header(ui, "ENTITIES");
                        ui.label(
                            egui::RichText::new(format!("doc: {} — 勾选进选择集", self.doc_key))
                                .small()
                                .color(COL_MUTED),
                        );
                        ui.separator();
                        egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                            if self.entities.is_empty() {
                                ui.label(
                                    egui::RichText::new("(空 — 点工具栏 🔄 刷新实体)").color(COL_MUTED),
                                );
                            }
                            for e in &self.entities {
                                let name =
                                    e.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                let kind =
                                    e.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                                let id = match e.get("id") {
                                    Some(serde_json::Value::String(s)) => s.clone(),
                                    Some(serde_json::Value::Number(n)) => n.to_string(),
                                    _ => "?".into(),
                                };
                                let mut checked = self.sel_members.iter().any(|m| m == &id);
                                if ui.checkbox(&mut checked, format!("{name} ({kind})")).changed() {
                                    if let Some(pos) =
                                        self.sel_members.iter().position(|m| m == &id)
                                    {
                                        self.sel_members.remove(pos);
                                    } else {
                                        self.sel_members.push(id);
                                    }
                                }
                            }
                        });
                        ui.separator();
                        section_header(ui, "SELECTION");
                        ui.horizontal(|ui| {
                            ui.label("名称:");
                            ui.text_edit_singleline(&mut self.sel_name);
                        });
                        ui.horizontal(|ui| {
                            if ui.button("保存(本地 JSON)").clicked() {
                                self.run_action(Action::SelSave);
                            }
                            if ui.button("应用").clicked() {
                                self.run_action(Action::SelApply);
                            }
                        });
                        ui.label(
                            egui::RichText::new(format!(
                                "已勾选 {} 个成员",
                                self.sel_members.len()
                            ))
                            .small()
                            .color(COL_MUTED),
                        );
                    },
                );
                self.show_entities = flag;
            }
        }

        if self.show_dim {
            let desired = next_pos(&mut stack_y, PANEL_DIM[1]);
            if let Some(desired) = desired {
                let mut flag = self.show_dim;
                show_child_window(
                    ctx,
                    "panel-dim",
                    "尺寸驱动",
                    desired,
                    egui::vec2(PANEL_DIM[0], PANEL_DIM[1]),
                    &mut flag,
                    true,
                    |ui| {
                        section_header(ui, "DIMENSION");
                        ui.horizontal(|ui| {
                            ui.label("数值:");
                            ui.add(egui::DragValue::new(&mut self.dim_value).speed(0.1));
                            if ui.button("应用平移").clicked() {
                                self.run_action(Action::DimApply);
                            }
                        });
                        ui.label(
                            egui::RichText::new("⚠ 需 facet 读写闭环(PLA-004 G4 后), 当前未接线")
                                .small()
                                .color(COL_MUTED),
                        );
                    },
                );
                self.show_dim = flag;
            }
        }

        if self.show_settings {
            let desired = next_pos(&mut stack_y, PANEL_SETTINGS[1]);
            if let Some(desired) = desired {
                let mut flag = self.show_settings;
                show_child_window(
                    ctx,
                    "panel-settings",
                    "设置",
                    desired,
                    egui::vec2(PANEL_SETTINGS[0], PANEL_SETTINGS[1]),
                    &mut flag,
                    true,
                    |ui| {
                        section_header(ui, "BRIDGE");
                        ui.monospace(&self.bridge.url);
                        let bind_text = match &self.snap_target {
                            Some(t) => format!("绑定窗口: {}", t.title),
                            None => "绑定窗口: 未绑定(等待 Plasticity…)".into(),
                        };
                        ui.label(egui::RichText::new(bind_text).small().color(COL_MUTED));
                        ui.label(
                            egui::RichText::new(format!("状态: {}", self.bridge_status)).small(),
                        );
                        ui.separator();
                        section_header(ui, "HOTKEYS");
                        ui.label(
                            egui::RichText::new("提示: 右键工具栏按钮可直接开始录制")
                                .small()
                                .color(COL_MUTED),
                        );
                        egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                            for a in Action::ALL {
                                let key = format!("{a:?}");
                                ui.horizontal(|ui| {
                                    ui.label(a.icon());
                                    ui.label(a.label());
                                    let cur = self
                                        .hotkeys
                                        .binds
                                        .get(&key)
                                        .map(|h| h.display())
                                        .unwrap_or_else(|| "-".into());
                                    ui.monospace(cur);
                                    if ui.small_button("绑定").clicked() {
                                        self.recording = Some(key.clone());
                                        self.notice = None;
                                    }
                                    if self.hotkeys.binds.contains_key(&key)
                                        && ui.small_button("清除").clicked()
                                    {
                                        self.hotkeys.binds.remove(&key);
                                    }
                                });
                            }
                            let conflicts = self.hotkeys.conflicts();
                            if !conflicts.is_empty() {
                                for (k, acts) in &conflicts {
                                    ui.colored_label(COL_RED, format!("⚠ 冲突: {k} → {acts:?}"));
                                }
                            }
                        });
                    },
                );
                self.show_settings = flag;
            }
        }

        // ---- 提示/错误浮窗(诚实提示): 子视口, 自动 6s 关闭 ----
        if let Some((is_err, msg)) = self.notice.clone() {
            if let Some(base) = toolbar_bottom_left {
                let desired = egui::pos2(base.x, base.y);
                let size = egui::vec2(300.0, 80.0);
                let mut contents = Some((is_err, msg));
                let builder = egui::ViewportBuilder::default()
                    .with_title("提示")
                    .with_decorations(false)
                    .with_resizable(false)
                    .with_transparent(true)
                    .with_window_level(egui::WindowLevel::AlwaysOnTop)
                    .with_active(false)
                    .with_position(desired)
                    .with_inner_size(size);
                let mut open_flag = true;
                // 非 move: open_flag 借用进闭包, ✕ 关闭能传回。
                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("notice-popup"),
                    builder,
                    |nctx, class| {
                        let Some((is_err, msg)) = contents.take() else { return };
                        if nctx.input(|i| i.viewport().close_requested()) {
                            open_flag = false;
                        }
                        let show_ui = |ui: &mut egui::Ui, open: &mut bool| {
                            panel_header(ui, if is_err { "⚠ 提示" } else { "✔ 完成" }, open);
                            ui.label(
                                egui::RichText::new(&msg)
                                    .color(if is_err { COL_RED } else { COL_ACCENT }),
                            );
                        };
                        match class {
                            egui::ViewportClass::Embedded | egui::ViewportClass::Root | egui::ViewportClass::Deferred => {
                                egui::Window::new("提示")
                                    .resizable(false)
                                    .collapsible(false)
                                    .show(nctx, |ui| show_ui(ui, &mut open_flag));
                            }
                            egui::ViewportClass::Immediate => {
                                egui::CentralPanel::default()
                                    .frame(panel_frame())
                                    .show(nctx, |ui| show_ui(ui, &mut open_flag));
                                let cur = nctx.input(|i| i.viewport().outer_rect);
                                if let Some(r) = cur {
                                    if (r.left() - desired.x).abs() > SNAP_EPS
                                        || (r.top() - desired.y).abs() > SNAP_EPS
                                    {
                                        nctx.send_viewport_cmd(
                                            egui::ViewportCommand::OuterPosition(desired),
                                        );
                                    }
                                }
                            }
                        }
                    },
                );
                if !open_flag {
                    self.notice = None;
                }
            }
        }

        // ---- 录制捕获(沿用 hotkey.rs 逻辑) ----
        if self.recording.is_some() {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.recording = None;
            } else {
                let (mods, key) = ctx.input(|i| {
                    (
                        i.modifiers,
                        i.events.iter().find_map(|e| match e {
                            egui::Event::Key { key, pressed: true, repeat: false, .. } => Some(*key),
                            _ => None,
                        }),
                    )
                });
                if let Some(key) = key {
                    let hk = Hotkey {
                        ctrl: mods.ctrl,
                        alt: mods.alt,
                        shift: mods.shift,
                        key: format!("{key:?}"),
                    };
                    if let Some(action) = self.recording.clone() {
                        match self.hotkeys.try_bind(&action, hk.clone()) {
                            Ok(()) => {
                                self.recording = None;
                                self.notice = None;
                            }
                            Err(others) => {
                                self.set_notice((
                                    true,
                                    format!("{} 已被 {:?} 占用", hk.display(), others),
                                ));
                            }
                        }
                    }
                }
            }
            ctx.request_repaint();
        }

        // bridge 事件到达时保持重绘(轮询线程 200ms 一跳)。
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}
