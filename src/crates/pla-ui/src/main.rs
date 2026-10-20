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
const COL_PANEL: egui::Color32 = egui::Color32::from_rgb(0x18, 0x18, 0x18);
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
/// 工具栏固定逻辑尺寸(横向一行按钮): 圆点+分隔线+5 按钮+分隔线+关闭键。
/// 宽 246 = 276(6 按钮旧值) - 30(手动快照按钮 24 + 间距 6 移入快照墙, R7-5);
/// 高 30: 垂直边距 3+3 + 按钮 24 = 30, 按钮行恰在窗口中心(吸附按中心对齐)。
/// 窗口 = 面板本体, 极简细描边(R7-19: 光晕被用户否决移除)。
const TOOLBAR_SIZE: [f32; 2] = [216.0, 30.0];
/// 面板整体右移微调(逻辑 px, 按 ppp 缩放; R6-2e 用户要求再偏右 10)。
const SNAP_RIGHT_SHIFT: f32 = 10.0;
/// 吸附垂直微调(逻辑 pt, 按 ppp 缩放; 仅作用于探测命中分支)。
/// R6-2l 曾设 -2.0 校准「窗口实际高=请求+4pt」造成的内容偏上, R6-2m 改为
/// 按 egui 上报的实际内窗高取中心后已无需补偿, 归零留作微调旋钮。
const SNAP_Y_NUDGE_PT: f32 = 0.0;
/// 注: 勿对 panel ui 用 min_rect 测尺寸做自适应 — CentralPanel 的 min_rect
/// 含整个可用区, need=窗口+余量 每帧自增(R6-2b 已回退, 教训留档)。
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
const ROUNDING: f32 = 10.0;

/// 工具栏按钮列(横向排列, 顺序即产品顺序)。
/// R7-5: 手动快照移入快照墙头部(图标钮); R7-11: 刷新实体移入选择集窗口头部 —
/// 一级工具栏只留面板开关类按钮。
/// 宽度 216 = 246 - (按钮24 + 间距6)。
const TOOLBAR_ACTIONS: [Action; 4] = [
    Action::ToggleSnapshots,
    Action::ToggleEntities,
    Action::ToggleDim,
    Action::ToggleSettings,
];

/// 面板逻辑尺寸 (宽, 高) — 子视口 OS 窗口大小。
/// R7-6: 快照墙改窄加高(232x480) — 一行一个快照, 缩略图撑满内容宽。
const PANEL_SNAPSHOTS: [f32; 2] = [232.0, 480.0];
const PANEL_ENTITIES: [f32; 2] = [280.0, 410.0];
const PANEL_DIM: [f32; 2] = [280.0, 170.0];
const PANEL_SETTINGS: [f32; 2] = [300.0, 560.0];

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
    last_snap_anchor: Option<(i32, i32, i32, i32)>,
    last_snap_poll: Instant,
    /// 文件名探测缓存: (目标 hwnd, 文件名块右缘物理 x)。None = 未探测到(降级偏移)。
    probed_edge: Option<(isize, pla_ui::titlebar_probe::ProbeHit)>,
    last_probe_poll: Instant,
    entities: Vec<serde_json::Value>,
    snapshots: Vec<String>,
    show_snapshots: bool,
    show_entities: bool,
    show_dim: bool,
    show_settings: bool,
    sel_name: String,
    sel_members: Vec<String>,
    /// CDP 会话句柄(R7-14): 实时选择/实体树通道(Plasticity 需 PLA-CDP 补丁版启动)。
    cdp: pla_bridge::cdp::CdpHandle,
    cdp_status: String,
    /// 实时选择快照 (kind, id, name, ctor)。
    live_sel: Vec<(String, String, String, String)>,
    /// CDP 实体树平铺 (name, ctor, indent, selected)。
    cdp_tree: Vec<(String, String, i64, bool)>,
    /// CDP 选择集勾选成员 (kind, id, name)。
    sel_cdp_members: Vec<(String, String, String)>,
    /// 自动快照(R7-15, PRD P0-1): 定时(分钟, 0=关) + 定步数(undo 增量, 0=关)。
    auto_snap_enabled: bool,
    auto_interval_min: i32,
    auto_steps: i32,
    /// 清理策略: 数量上限(0=不限) / 保留天数(0=不限)。
    snap_max_count: i32,
    snap_max_age_days: i32,
    last_auto_time: Instant,
    last_auto_step: i64,
    last_auto_mtime: Option<std::time::SystemTime>,
    /// CDP 侧 undo 栈深度(步数计数源)。
    cdp_undo: i64,
    dim_value: f64,
    /// 尺寸驱动(R7-16): 关联选择集 + 方向 + 数值(PRD P1-1 定义)。
    dim_sets: Vec<String>,
    dim_set_sel: usize,
    dim_axis: usize,
    dim_loaded: bool,
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
    /// 快照 PNG 字节缓存(name → bytes): 快照文件不可变, 读盘一次。
    snap_pngs: std::collections::HashMap<String, std::sync::Arc<[u8]>>,
    /// 收藏的快照名(含 .plasticity), 与 snapshot/meta.json 同步。
    snap_favs: std::collections::HashSet<String>,
    /// 快照元数据(名 → (源文档, 拍摄时间)): 开墙/拍/删/改名后整表重读, 悬浮显示用。
    snap_infos: std::collections::HashMap<String, (String, String)>,
    /// 管理模式: 多选删除。
    snap_manage: bool,
    /// 管理模式勾选集合。
    snap_selected: std::collections::HashSet<String>,
    /// 正在改名的快照名 + 编辑缓冲。
    rename_edit: Option<String>,
    rename_buf: String,
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
/// R7-19: 面板边框 = 按钮悬停同款(egui 默认 hovered bg_stroke: 1px 灰150) —
/// 用户实测否决内侧羽化光晕(奶白渐变太亮), 改极简细描边。
fn panel_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(COL_PANEL)
        .rounding(egui::Rounding::same(ROUNDING))
        .inner_margin(egui::Margin { left: 6.0, right: 6.0, top: 3.0, bottom: 3.0 })
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(150)))
}

/// 快照项交互结果(由 snap_item 采集, 调用方统一执行)。
#[derive(Default)]
struct SnapItemActions {
    open: bool,
    fav_toggle: bool,
    /// 管理模式勾选翻转后的新状态。
    select_toggle: Option<bool>,
    rename_start: bool,
    rename_commit: bool,
    rename_cancel: bool,
}

/// PNG 尺寸解析(只读 IHDR 头, 不解码): 签名 8B + len 4B + "IHDR" 4B + W 4B + H 4B。
/// 我们的截图 PNG 由 wincap.rs 生成, 结构确定; 解析失败返回 None 走占位。
fn png_dims(bytes: &[u8]) -> Option<(f32, f32)> {
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?) as f32;
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?) as f32;
    (w > 0.0 && h > 0.0).then_some((w, h))
}

/// 快照缩略图。uniform=false(全部区): 按真实宽高比把格宽撑满(fit_to_exact_size,
/// 不变形), 高封顶 max_h; uniform=true(收藏区): 高度统一 = max_h, 宽按比例且
/// 不超格宽, 格内水平居中 — 各快照截图时窗口大小不同, 宽度驱动会导致行内
/// 高低差(R7-9 问题1), 统一高后名称行齐平。
/// 点击行为随模式: 浏览=打开, 管理=勾选。
fn snap_image(
    ui: &mut egui::Ui,
    png: Option<&std::sync::Arc<[u8]>>,
    name: &str,
    cell_w: f32,
    max_h: f32,
    uniform: bool,
) -> egui::Response {
    // R7-7: max_size 只限制不撑满(宽屏截图缩到限高后宽度远小于格宽) —
    // 由 IHDR 真实比例反推精确显示尺寸。
    let iw = match png.and_then(|b| png_dims(b)) {
        Some((w, h)) => {
            if uniform {
                (max_h * w / h).min(cell_w)
            } else {
                let fh = cell_w * h / w;
                if fh <= max_h {
                    cell_w
                } else {
                    let s = max_h / fh; // 竖长图封顶: 等比缩, 宽度让位
                    cell_w * s
                }
            }
        }
        None => {
            if uniform {
                (max_h * 1.2).min(cell_w)
            } else {
                cell_w
            }
        }
    };
    let ih = if uniform { max_h } else {
        png.and_then(|b| png_dims(b)).map_or(max_h.min(cell_w * 0.45), |(w, h)| {
            let fh = cell_w * h / w;
            if fh <= max_h { fh } else { max_h }
        })
    };
    let draw = |ui: &mut egui::Ui| -> egui::Response {
        if let Some(bytes) = png {
            ui.add(
                egui::Image::from_bytes(format!("bytes://{name}"), bytes.clone())
                    .fit_to_exact_size(egui::vec2(iw, ih))
                    .sense(egui::Sense::click()),
            )
        } else {
            ui.add_sized(
                egui::vec2(iw, ih),
                egui::Label::new(
                    egui::RichText::new("📷").size(if cell_w > 100.0 { 30.0 } else { 16.0 }),
                )
                .sense(egui::Sense::click()),
            )
        }
    };
    if uniform {
        // 统一高模式: 图占满整格高, 水平居中(宽可能小于格宽)
        ui.allocate_ui_with_layout(
            egui::vec2(cell_w, ih),
            egui::Layout::top_down(egui::Align::Center),
            draw,
        )
        .inner
    } else {
        draw(ui)
    }
}

/// 14×14 矢量铅笔钮(feather edit-2 缩绘): ✎(U+270E) 字形在 SimHei(GB2312)与
/// NotoEmoji 中都没有 → 只能画出来(R7-8)。
fn pencil_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
    let col = if resp.hovered() { COL_TEXT } else { COL_MUTED };
    let s = 14.0 / 24.0;
    let pt = |x: f32, y: f32| egui::pos2(rect.left() + x * s, rect.top() + y * s);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            pt(2.0, 22.0),
            pt(7.5, 20.5),
            pt(21.0, 7.0),
            pt(17.0, 3.0),
            pt(3.5, 16.5),
        ],
        col,
        egui::Stroke::NONE,
    ));
    resp
}

/// 单个快照项: 缩略图 + 名称行(勾选框[管理模式]/名称/收藏星/改名钮)。
/// 改名中(renaming)时名称行变为编辑框 + √/×。只采集动作, 不执行 — 借用隔离。
/// cell_w 是项的固定宽度(收藏 3 列均分 / 全部区撑满): 名称 truncate 依赖
/// 固定宽度子区才生效 — Grid 列宽随内容走, truncate 形同虚设(R7-7)。
fn snap_item(
    ui: &mut egui::Ui,
    name: &str,
    png: Option<&std::sync::Arc<[u8]>>,
    cell_w: f32,
    max_img_h: f32,
    uniform_img: bool,
    img_tip: Option<(String, String)>,
    hover_out: &std::cell::RefCell<Option<(String, String)>>,
    is_fav: bool,
    manage: bool,
    selected: bool,
    renaming: bool,
    rename_buf: &mut String,
) -> SnapItemActions {
    let mut a = SnapItemActions::default();
    let stem = name.trim_end_matches(".plasticity").to_string();
    ui.allocate_ui_with_layout(
        egui::vec2(cell_w, 36.0),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            let img_resp =
                snap_image(ui, png, name, cell_w, max_img_h, uniform_img);
            // 悬浮缩略图 → HUD(工具栏右侧文本框)显示元数据(源文档/拍摄时间)
            if img_resp.hovered() {
                if let Some(t) = &img_tip {
                    *hover_out.borrow_mut() = Some(t.clone());
                }
            }
            if img_resp.clicked() {
                if manage {
                    a.select_toggle = Some(!a.select_toggle.unwrap_or(false));
                } else {
                    a.open = true;
                }
            }
            ui.horizontal(|ui| {
                if renaming {
                    ui.add_sized(
                        egui::vec2((cell_w - 46.0).max(30.0), 18.0),
                        egui::TextEdit::singleline(rename_buf),
                    )
                    .request_focus();
                    // R7-9: 回车确认 / Esc 取消 — 不必去点 √/× 按钮
                    let (enter, esc) = ui.input(|i| {
                        (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape))
                    });
                    if enter {
                        a.rename_commit = true;
                    }
                    if esc {
                        a.rename_cancel = true;
                    }
                    return;
                }
                if manage {
                    let mut on = selected;
                    if ui.checkbox(&mut on, "").changed() {
                        a.select_toggle = Some(on);
                    }
                }
                let name_resp = {
                    // R7-8: 名称行各元素全部定宽 — 之前 Button 自带 padding 使
                    // 单元格内容最小宽 > cell_w, 收藏第 3 列被挤出面板右缘。
                    let sp = ui.spacing().item_spacing.x;
                    let reserve = if manage { 16.0 + sp } else { 0.0 };
                    let rem = (cell_w - (14.0 + sp) * 2.0 - reserve).max(24.0);
                    let nr = ui
                        .allocate_ui_with_layout(
                            egui::vec2(rem, 14.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&stem).small().color(COL_TEXT),
                                    )
                                    .sense(egui::Sense::click())
                                    .truncate(),
                                )
                            },
                        )
                        .inner;
                    nr.on_hover_text(if manage { &stem } else { "点击在新窗口打开快照" })
                };
                if name_resp.clicked() && !manage {
                    a.open = true;
                }
                // 收藏星: 定宽 14, painter 画字(Button 自带 padding 撑宽单元格)
                let star_col = if is_fav {
                    egui::Color32::from_rgb(255, 200, 60)
                } else {
                    COL_MUTED
                };
                let (srect, sresp) =
                    ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
                ui.painter().text(
                    srect.center(),
                    egui::Align2::CENTER_CENTER,
                    if is_fav { "★" } else { "☆" },
                    egui::FontId::proportional(13.0),
                    star_col,
                );
                if sresp
                    .on_hover_text(if is_fav { "取消收藏" } else { "收藏(置顶小图)" })
                    .clicked()
                {
                    a.fav_toggle = true;
                }
                if pencil_button(ui).on_hover_text("改名").clicked() {
                    a.rename_start = true;
                }
            });
        },
    );
    a
}


fn panel_header(ui: &mut egui::Ui, title: &str, open: &mut bool, icons: Option<Box<dyn FnOnce(&mut egui::Ui)>>) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).size(13.0).strong().color(COL_TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = egui::Button::new(
                egui::RichText::new("×").size(13.0).color(egui::Color32::WHITE),
            )
            .frame(false)
            .rounding(egui::Rounding::same(4.0));
            if ui.add(close).clicked() {
                *open = false;
            }
            // 图标按钮区: right_to_left 布局下加在 × 之后 → 出现在 × 左侧
            if let Some(extra) = icons {
                extra(ui);
            }
        });
    });
    ui.separator();
}

impl PlaApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_cjk_fonts(&cc.egui_ctx);
        apply_plasticity_style(&cc.egui_ctx);
        // 快照墙缩略图: Image::from_bytes 需要 extras 加载器解码 PNG(子视口共享同一 ctx)
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let doc_key = "model1".to_string();
        Self {
            bridge: BridgeHandle::spawn(),
            bridge_status: "连接中…".into(),
            snap_target: None,
            last_snap_anchor: None,
            last_snap_poll: Instant::now() - Duration::from_millis(SNAP_POLL_MS),
            probed_edge: None,
            last_probe_poll: Instant::now() - Duration::from_millis(PROBE_POLL_MS),
            entities: Vec::new(),
            snapshots: pla_ui::snapshot::list_snapshots(&doc_key),
            show_snapshots: false,
            show_entities: false,
            show_dim: false,
            show_settings: false,
            sel_name: String::new(),
            sel_members: Vec::new(),
            cdp: pla_bridge::cdp::CdpHandle::spawn(9333),
            cdp_status: "CDP 连接中…".into(),
            live_sel: Vec::new(),
            cdp_tree: Vec::new(),
            sel_cdp_members: Vec::new(),
            auto_snap_enabled: true,
            auto_interval_min: 10,
            auto_steps: 20,
            snap_max_count: 50,
            snap_max_age_days: 7,
            last_auto_time: Instant::now(),
            last_auto_step: -1,
            last_auto_mtime: None,
            cdp_undo: -1,
            dim_value: 0.0,
            dim_sets: Vec::new(),
            dim_set_sel: 0,
            dim_axis: 0,
            dim_loaded: false,
            hotkeys: pla_ui::hotkey::load_binds().unwrap_or_default(),
            recording: None,
            notice: None,
            notice_at: Instant::now(),
            hovered_hint: None,
            doc_key,
            snap_pngs: std::collections::HashMap::new(),
            snap_favs: std::collections::HashSet::new(),
            snap_infos: std::collections::HashMap::new(),
            snap_manage: false,
            snap_selected: std::collections::HashSet::new(),
            rename_edit: None,
            rename_buf: String::new(),
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
        // CDP 事件: 实时选择/实体树/状态(R7-14)。
        while let Some(ev) = self.cdp.poll() {
            match ev {
                pla_bridge::cdp::CdpEvent::Status(s) => self.cdp_status = s,
                pla_bridge::cdp::CdpEvent::Selection(snap) => {
                    if snap.undo_len >= 0 {
                        self.cdp_undo = snap.undo_len;
                    }
                    if snap.ok {
                        self.live_sel = snap
                            .items
                            .into_iter()
                            .map(|i| (i.kind.to_string(), i.id, i.name, i.ctor))
                            .collect();
                    } else if self.live_sel.is_empty() && snap.state == "no-editor" {
                        // 编辑器尚未就绪: 保持状态行即可
                    } else {
                        self.live_sel.clear();
                    }
                }
                pla_bridge::cdp::CdpEvent::EvalResult(s) => {
                    self.set_notice((false, format!("CDP: {}", s)));
                }
                pla_bridge::cdp::CdpEvent::Tree(nodes) => {
                    self.cdp_tree = nodes
                        .into_iter()
                        .map(|n| (n.name, n.ctor, n.indent, n.selected))
                        .collect();
                }
            }
        }
        // 提示浮窗自动关闭。
        if self.notice.is_some() && self.notice_at.elapsed() > NOTICE_TTL {
            self.notice = None;
        }
        self.auto_snapshot_tick();
    }

    /// 自动快照(R7-15, PRD P0-1): 定时 + 定步数 + mtime 防重 + 清理策略。
    fn auto_snapshot_tick(&mut self) {
        if !self.auto_snap_enabled || !self.cdp_status.contains("已连接") {
            return;
        }
        // 定时
        if self.auto_interval_min > 0
            && self.last_auto_time.elapsed()
                >= Duration::from_secs(self.auto_interval_min as u64 * 60)
        {
            self.try_auto_snapshot("定时");
            return;
        }
        // 定步数: undo 栈深度增量(首次见到计数器只记录基线)
        if self.auto_steps > 0 && self.cdp_undo >= 0 {
            if self.last_auto_step < 0 {
                self.last_auto_step = self.cdp_undo;
            } else if self.cdp_undo - self.last_auto_step >= self.auto_steps as i64 {
                self.try_auto_snapshot("定步数");
            }
        }
    }

    /// 尝试一次自动快照: 文档 mtime 未变(无新保存)则跳过; 成功后跑清理策略。
    fn try_auto_snapshot(&mut self, reason: &str) {
        self.last_auto_time = Instant::now();
        if self.cdp_undo >= 0 {
            self.last_auto_step = self.cdp_undo;
        }
        let mtime = pla_ui::snapshot::latest_doc_path(&self.doc_key)
            .and_then(|p| p.metadata().ok())
            .and_then(|m| m.modified().ok());
        // mtime 防重: 上次自动快照后没有新的 Ctrl+S → 无新内容可拍
        if let (Some(a), Some(b)) = (mtime, self.last_auto_mtime) {
            if a == b {
                return;
            }
        }
        let hwnd = self.snap_target.as_ref().map(|t| t.hwnd);
        match pla_ui::snapshot::take_manual_snapshot(&self.doc_key, hwnd) {
            Ok(name) => {
                self.last_auto_mtime = mtime;
                self.snapshots = pla_ui::snapshot::list_snapshots(&self.doc_key);
                self.snap_pngs.clear();
                self.snap_infos = pla_ui::snapshot::load_meta_infos(&self.doc_key);
                self.run_snapshot_cleanup();
            }
            Err(_) => {} // 自动快照失败不打扰(如未保存过), 下次再试
        }
    }

    /// 集中动作分发: 每个按钮都有真实行为; 未实现的功能弹明确提示, 不做假按钮。
    fn run_action(&mut self, a: Action) {
        match a {
            Action::EntitiesRefresh => self.bridge.send(BridgeCmd::RefreshEntities),
            Action::SnapshotManual => {
                let hwnd = self.snap_target.as_ref().map(|t| t.hwnd);
                match pla_ui::snapshot::take_manual_snapshot(&self.doc_key, hwnd) {
                    Ok(name) => {
                        self.snapshots = pla_ui::snapshot::list_snapshots(&self.doc_key);
                        self.snap_pngs.clear(); // 新快照可能有新 PNG, 下次进面板重读
                        self.snap_infos = pla_ui::snapshot::load_meta_infos(&self.doc_key);
                        self.run_snapshot_cleanup();
                        self.notice = Some((false, format!("快照已保存: {name}")));
                    }
                    Err(e) => self.notice = Some((true, e)),
                }
            }
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
            Action::ToggleSnapshots => {
                self.show_snapshots = !self.show_snapshots;
                if self.show_snapshots {
                    // 每次开墙刷新: 快照可能被外部增删(含 Plasticity 侧)
                    self.snapshots = pla_ui::snapshot::list_snapshots(&self.doc_key);
                    self.snap_favs = pla_ui::snapshot::load_favs(&self.doc_key);
                    self.snap_infos = pla_ui::snapshot::load_meta_infos(&self.doc_key);
                }
            }
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
    /// 从窗口标题提取 doc_key: "model1.plasticity - Plasticity" → "model1";
    /// 无标题/未命名 → "untitled"。
    fn doc_key_from_title(title: &str) -> String {
        let base = title.split(" - ").next().unwrap_or(title).trim();
        let base = base.trim_start_matches('*').trim();
        if base.is_empty() || base.eq_ignore_ascii_case("Plasticity") {
            return "untitled".into();
        }
        base.strip_suffix(".plasticity")
            .unwrap_or(base)
            .to_string()
    }

    /// 手动/自动快照共用的清理策略(R7-17)。
    fn run_snapshot_cleanup(&mut self) {
        let removed = pla_ui::snapshot::cleanup_snapshots(
            &self.doc_key,
            self.snap_max_count.max(0) as usize,
            self.snap_max_age_days.max(0) as u64,
        )
        .unwrap_or(0);
        if removed > 0 {
            self.snapshots = pla_ui::snapshot::list_snapshots(&self.doc_key);
        }
    }

    fn update_snap(&mut self, ctx: &egui::Context) {
        if self.last_snap_poll.elapsed() < Duration::from_millis(SNAP_POLL_MS) {
            return;
        }
        self.last_snap_poll = Instant::now();
        let new_target = window_finder::acquire_target(&self.doc_key);
        self.snap_target = new_target;
        // R7-17: doc_key 跟随绑定窗口标题("model1.plasticity - Plasticity" →
        // "model1") — 多文档/多窗口时快照/选择集/实体缓存按文档隔离(PRD 验收)。
        if let Some(target) = self.snap_target.clone() {
            let derived = Self::doc_key_from_title(&target.title);
            if derived != self.doc_key {
                self.doc_key = derived;
                // 文档切换: 全部按 doc_key 归档的缓存失效重载
                self.entities.clear();
                self.snapshots = pla_ui::snapshot::list_snapshots(&self.doc_key);
                self.snap_pngs.clear();
                self.snap_favs = pla_ui::snapshot::load_favs(&self.doc_key);
                self.snap_infos = pla_ui::snapshot::load_meta_infos(&self.doc_key);
                self.sel_cdp_members.clear();
                self.dim_loaded = false;
            }
        }
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
        // 物理 x: 探测命中 → 文件名右缘+间隙+右移微调; 否则降级固定偏移。
        // 物理 y: 探测命中 → 按文件名块垂直中心居中(顶栏内容居中, 正常/最大化
        // 顶栏高度不同也能吸附到正中, R6-2i); 否则降级顶部固定间隙。
        // 两者都按 ppp 缩放: probe/GetWindowRect 在 DPI-aware 进程里是物理像素,
        // 固定常量是逻辑 px, 不缩放在高 DPI 下会缩水遮文件名(R6 实测 4K/3x)。
        let ppp = ctx.pixels_per_point();
        let (x_phys, y_phys) = match &self.probed_edge {
            Some((h, hit)) if *h == target.hwnd => {
                let x = hit.right as f32 + (SNAP_FILENAME_GAP + SNAP_RIGHT_SHIFT) * ppp;
                // 实际内窗高(egui 上报): 请求 30pt 但 winit 隐形边框撑到 34pt,
                // 按实际值取中心, 否则恒偏 2pt(R6-2m)。
                let h_pt = ctx.input(|i| {
                    i.viewport()
                        .inner_rect
                        .map(|r| r.height())
                        .unwrap_or(TOOLBAR_SIZE[1])
                });
                let y = target.top as f32
                    + (hit.cy as f32 + SNAP_Y_NUDGE_PT * ppp)
                    - h_pt * 0.5 * ppp;
                (x, y)
            }
            _ => (
                target.left as f32 + (SNAP_LEFT_OFFSET + SNAP_RIGHT_SHIFT) * ppp,
                target.top as f32 + SNAP_TOP_GAP as f32,
            ),
        };
        let new_anchor = Some((target.left, target.top, x_phys as i32, y_phys as i32));
        let anchor_changed = match (new_anchor, self.last_snap_anchor) {
            (Some(n), Some(o)) => {
                (n.0 - o.0).abs() as f32 > SNAP_EPS
                    || (n.1 - o.1).abs() as f32 > SNAP_EPS
                    || (n.2 - o.2).abs() as f32 > SNAP_EPS
                    || (n.3 - o.3).abs() as f32 > SNAP_EPS
            }
            _ => true,
        };
        if !anchor_changed {
            return;
        }
        self.last_snap_anchor = new_anchor;
        // 物理 → egui points: OuterPosition 用 points(见 egui 0.29 ViewportInfo)。
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            x_phys / ppp,
            y_phys / ppp,
        )));
    }
}

/// 子视口渲染的公共入口: Immediate 模式 = 独立 OS 窗口(圆角深色面板);
/// Embedded 回退(后端不支持多视口) = 原地 egui::Window, 行为不劣化。
/// 面板位置每帧跟随工具栏(漂移 >2px 重发 OuterPosition), 关闭走 × 或 OS 关闭。
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
    header_extra_and_show(parent, id, title, desired, size, open, active, add_contents, None)
}

/// 同 show_child_window, 但允许在标题右侧、× 左侧插入图标按钮区(R7-5 快照墙)。
#[allow(clippy::too_many_arguments)]
fn header_extra_and_show(
    parent: &egui::Context,
    id: &'static str,
    title: &'static str,
    desired: egui::Pos2,
    size: egui::Vec2,
    open: &mut bool,
    active: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
    header_icons: Option<Box<dyn FnOnce(&mut egui::Ui)>>,
) {
    let mut flag = *open;
    let mut contents = Some(add_contents);
    let mut icons = header_icons;
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
        // 非 move: flag 借用进闭包, ×/OS 关闭的结果能在调用后传回。
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
                            panel_header(ui, title, &mut flag, icons.take());
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
            round_corners::poll_round_windows(
                &mut self.round_state,
                (ROUNDING * 2.0 * ctx.pixels_per_point()).round() as i32,
            );
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
                // R6-2m: 行显式垂直居中。窗口实际高 = 请求值+4pt(winit 对
                // undecorated+shadow 窗口保留 WS_THICKFRAME 隐形可调边框,
                // egui 0.29 无 with_shadow 可关), egui horizontal 对超高行
                // 顶对齐 → 行内偏上 2pt(用户实测)。改为按可用区中心显式
                // 放置行矩形, 与窗口实际高度解耦。
                let avail = ui.available_rect_before_wrap();
                // 行高 = 按钮边长(24), 不能用 TOOLBAR_SIZE[1](30) — min(30, 可用28)
                // 会把行矩形撑满内容区, 居中形同虚设(R6-2m 首版实测按钮仍偏上)。
                let row_h = BTN_SIZE.min(avail.height());
                let row_rect = egui::Rect::from_min_max(
                    egui::pos2(avail.left(), avail.center().y - row_h * 0.5),
                    egui::pos2(avail.right(), avail.center().y + row_h * 0.5),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(row_rect), |ui| {
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
                        egui::RichText::new("×").size(13.0).color(egui::Color32::WHITE),
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
                }); // allocate_new_ui(居中行矩形)
            });
        self.hovered_hint = hovered;

        // ---- 工具栏外框基准位置(功能面板锚正下方, HUD 锚右侧) ----
        let toolbar_rect = ctx
            .input(|i| i.viewport().outer_rect);
        let toolbar_bottom_left =
            toolbar_rect.map(|r| egui::pos2(r.left() + 2.0, r.bottom() + 6.0));

        // ---- HUD 视口实际渲染点已移到所有功能面板之后(R7-9) —
        // 面板内按钮(如快照墙头部图标)的 hover 在面板视口渲染时才写入
        // hovered_hint, HUD 若在面板之前渲染则永远晚一帧且被工具栏的
        // 帧首覆盖清掉 → 表现为「二级按钮没有悬浮文本框」。

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
                // PNG 缓存填充(快照文件不可变, 每个只读盘一次)
                let snap_dir = pla_ui::snapshot::snapshot_dir_of(&self.doc_key);
                for s in &self.snapshots {
                    if !self.snap_pngs.contains_key(s) {
                        let stem = s.trim_end_matches(".plasticity");
                        if let Ok(b) = std::fs::read(snap_dir.join(format!("{stem}.png"))) {
                            self.snap_pngs.insert(s.clone(), std::sync::Arc::from(b));
                        }
                    }
                }
                // 头部图标钮的点击结果经 Cell 传出 — icons 闭包要求 'static,
                // 不能借用局部变量; Rc<Cell<bool>> 兼顾两者, 面板关闭后统一执行。
                let do_snapshot = std::rc::Rc::new(std::cell::Cell::new(false));
                let do_open_dir = std::rc::Rc::new(std::cell::Cell::new(false));
                let do_manage = std::rc::Rc::new(std::cell::Cell::new(false));
                // hover 提示槽: egui 原生 tooltip 在窄视口右上角被裁(R7-8 问题3),
                // 改写入槽位 → 回到主视口并入 hovered_hint → HUD 弹工具栏右侧。
                let tip_slot =
                    std::rc::Rc::new(std::cell::RefCell::new(None::<(String, String)>));
                let (ds, dod, dm, ts) = (
                    do_snapshot.clone(),
                    do_open_dir.clone(),
                    do_manage.clone(),
                    tip_slot.clone(),
                );
                let icons: Box<dyn FnOnce(&mut egui::Ui)> = Box::new(move |ui| {
                    let icon_btn = |ui: &mut egui::Ui,
                                    glyph: &str,
                                    tip: &str,
                                    hit: &std::cell::Cell<bool>,
                                    ts: &std::cell::RefCell<Option<(String, String)>>| {
                        let resp = ui.add(
                            egui::Button::new(
                                egui::RichText::new(glyph)
                                    .size(13.0)
                                    .color(egui::Color32::WHITE),
                            )
                            .frame(false)
                            .rounding(egui::Rounding::same(4.0)),
                        );
                        if resp.hovered() {
                            *ts.borrow_mut() = Some((format!("{glyph} {tip}"), String::new()));
                        }
                        if resp.clicked() {
                            hit.set(true);
                        }
                    };
                    // right_to_left 布局: 先加的靠右 → 视觉顺序 🗑 📂 📷 ×(右→左)
                    icon_btn(ui, "🗑", "管理 — 多选删除", &dm, &ts);
                    icon_btn(ui, "📂", "打开快照文件夹", &dod, &ts);
                    icon_btn(ui, "📷", "手动快照 — 保存文档副本 + 截图当前画面", &ds, &ts);
                });
                header_extra_and_show(
                    ctx,
                    "panel-snapshots",
                    "快照墙",
                    desired,
                    egui::vec2(PANEL_SNAPSHOTS[0], PANEL_SNAPSHOTS[1]),
                    &mut flag,
                    true,
                    |ui| {
                        let mut open_err: Option<String> = None;
                        // 缩略图悬浮元数据槽: 面板视口渲染时写入, 面板关闭后并入 HUD
                        let tip_slot = std::cell::RefCell::new(None::<(String, String)>);

                        // ---- 管理模式操作条(删除立即执行, 列表随后按新数据渲染) ----
                        if self.snap_manage {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("已选 {}", self.snap_selected.len()))
                                        .small()
                                        .color(COL_MUTED),
                                );
                                let del = ui.add(
                                    egui::Button::new(
                                        egui::RichText::new("🗑 删除所选")
                                            .small()
                                            .color(egui::Color32::WHITE),
                                    )
                                    .fill(egui::Color32::from_rgb(150, 45, 45)),
                                );
                                let del = del.on_hover_text("删除勾选的快照(含截图), 不可恢复");
                                if del.clicked() && !self.snap_selected.is_empty() {
                                    let names: Vec<String> =
                                        self.snap_selected.iter().cloned().collect();
                                    match pla_ui::snapshot::delete_snapshots(&self.doc_key, &names)
                                    {
                                        Ok(n) => {
                                            self.snapshots =
                                                pla_ui::snapshot::list_snapshots(&self.doc_key);
                                            self.snap_infos = pla_ui::snapshot::load_meta_infos(
                                                &self.doc_key,
                                            );
                                            for n in &names {
                                                self.snap_pngs.remove(n);
                                                self.snap_selected.remove(n);
                                                self.snap_favs.remove(n);
                                            }
                                            self.set_notice((false, format!("已删除 {n} 个快照")));
                                        }
                                        Err(e) => self.set_notice((true, e)),
                                    }
                                }
                                if ui.small_button("完成").clicked() {
                                    self.snap_manage = false;
                                    self.snap_selected.clear();
                                }
                            });
                            ui.separator();
                        }

                        // 收藏置顶(小图 3 列), 其余一行一个(大图撑满)
                        let (fav_list, other_list): (Vec<String>, Vec<String>) = self
                            .snapshots
                            .iter()
                            .cloned()
                            .partition(|s| self.snap_favs.contains(s));
                        let mut fav_ops: Vec<(String, bool)> = Vec::new();
                        let mut sel_toggles: Vec<(String, bool)> = Vec::new();
                        let mut opens: Vec<String> = Vec::new();
                        let mut rename_start: Option<String> = None;
                        let mut rename_commit = false;
                        let mut rename_cancel = false;
                        let mut refresh = false;

                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            // R7-10: 宽度必须在 ScrollArea 内部测 — 竖向滚动条出现后
                            // 内部可用宽比外部少一个滚动条宽, 在外面测会让每行总宽
                            // 超出可视区, 第 3 列被裁掉(用户实测"宽度还超了")。
                            let avail_w = ui.available_width();
                            let sp = ui.spacing().item_spacing.x;
                            let fav_cell_w = ((avail_w - sp * 2.0) / 3.0).max(40.0);
                            let fav_img_h = 56.0_f32;
                            let fav_row_h = fav_img_h + 20.0; // 图 + 名称行
                            let big_cell_w = avail_w;
                            // 悬浮提示文本: SS 短名 + meta 里的源文档/拍摄时间
                            let tip_of = |s: &str| -> Option<(String, String)> {
                                let stem = s.trim_end_matches(".plasticity");
                                match self.snap_infos.get(s) {
                                    Some((doc, created)) => Some((
                                        format!("{stem} — {doc}"),
                                        format!("拍摄于 {created} · 点击在新窗口打开"),
                                    )),
                                    None => Some((
                                        stem.to_string(),
                                        "点击在新窗口打开".to_string(),
                                    )),
                                }
                            };
                            if self.snapshots.is_empty() {
                                ui.label(
                                    egui::RichText::new("(暂无快照 — 点右上 📷 拍摄)").color(COL_MUTED),
                                );
                            }
                            if !fav_list.is_empty() {
                                ui.label(
                                    egui::RichText::new("★ 收藏").small().strong().color(COL_MUTED),
                                );
                                // R7-10: 收藏行显式矩形定位(allocate_exact_size + UiBuilder
                                // .max_rect), 列宽/行高完全确定 — 不依赖 horizontal 累进,
                                // 无高低差、无宽度外溢。
                                for trio in fav_list.chunks(3) {
                                    let row_rect = ui
                                        .allocate_exact_size(
                                            egui::vec2(avail_w, fav_row_h),
                                            egui::Sense::hover(),
                                        )
                                        .0;
                                    for (ci, s) in trio.iter().enumerate() {
                                        let cell = egui::Rect::from_min_size(
                                            egui::pos2(
                                                row_rect.left()
                                                    + ci as f32 * (fav_cell_w + sp),
                                                row_rect.top(),
                                            ),
                                            egui::vec2(fav_cell_w, fav_row_h),
                                        );
                                        let tip = tip_of(s);
                                        let act = ui
                                            .allocate_new_ui(
                                                egui::UiBuilder::new().max_rect(cell),
                                                |ui| {
                                                    snap_item(
                                                        ui,
                                                        s,
                                                        self.snap_pngs.get(s),
                                                        fav_cell_w,
                                                        fav_img_h,
                                                        true, // 统一图高
                                                        tip,
                                                        &tip_slot,
                                                        true,
                                                        self.snap_manage,
                                                        self.snap_selected.contains(s),
                                                        self.rename_edit.as_deref()
                                                            == Some(s.as_str()),
                                                        &mut self.rename_buf,
                                                    )
                                                },
                                            )
                                            .inner;
                                        if act.open {
                                            opens.push(s.clone());
                                        }
                                        if act.fav_toggle {
                                            fav_ops.push((s.clone(), false));
                                        }
                                        if let Some(on) = act.select_toggle {
                                            sel_toggles.push((s.clone(), on));
                                        }
                                        if act.rename_start && !self.snap_manage {
                                            rename_start = Some(s.clone());
                                        }
                                        rename_commit |= act.rename_commit;
                                        rename_cancel |= act.rename_cancel;
                                    }
                                }
                                ui.separator();
                                ui.label(
                                    egui::RichText::new("全部").small().strong().color(COL_MUTED),
                                );
                            }
                            for s in &other_list {
                                let tip = tip_of(s);
                                let act = snap_item(
                                    ui,
                                    s,
                                    self.snap_pngs.get(s),
                                    big_cell_w,
                                    300.0,
                                    false, // 全部区: 撑满格宽
                                    tip,
                                    &tip_slot,
                                    self.snap_favs.contains(s),
                                    self.snap_manage,
                                    self.snap_selected.contains(s),
                                    self.rename_edit.as_deref() == Some(s.as_str()),
                                    &mut self.rename_buf,
                                );
                                ui.add_space(4.0);
                                if act.open {
                                    opens.push(s.clone());
                                }
                                if act.fav_toggle {
                                    fav_ops.push((s.clone(), true));
                                }
                                if let Some(on) = act.select_toggle {
                                    sel_toggles.push((s.clone(), on));
                                }
                                if act.rename_start && !self.snap_manage {
                                    rename_start = Some(s.clone());
                                }
                                rename_commit |= act.rename_commit;
                                rename_cancel |= act.rename_cancel;
                            }
                        });
                        // 悬浮缩略图时的元数据 → HUD(工具栏右侧文本框)
                        if let Some(t) = tip_slot.into_inner() {
                            self.hovered_hint = Some(t);
                        }

                        // ---- 统一执行采集到的动作(此时列表借用已结束) ----
                        for (s, v) in fav_ops {
                            if let Err(e) = pla_ui::snapshot::set_fav(&self.doc_key, &s, v) {
                                open_err = Some(e);
                            }
                            if v {
                                self.snap_favs.insert(s);
                            } else {
                                self.snap_favs.remove(&s);
                            }
                        }
                        for (s, on) in sel_toggles {
                            if on {
                                self.snap_selected.insert(s);
                            } else {
                                self.snap_selected.remove(&s);
                            }
                        }
                        if rename_cancel {
                            self.rename_edit = None;
                        }
                        if let Some(s) = rename_start {
                            self.rename_buf = s.trim_end_matches(".plasticity").to_string();
                            self.rename_edit = Some(s);
                        }
                        if rename_commit {
                            if let Some(old) = self.rename_edit.clone() {
                                match pla_ui::snapshot::rename_snapshot(
                                    &self.doc_key,
                                    &old,
                                    &self.rename_buf,
                                ) {
                                    Ok(new_name) => {
                                        self.snapshots =
                                            pla_ui::snapshot::list_snapshots(&self.doc_key);
                                        self.snap_pngs.remove(&old);
                                        // 收藏键跟随改名
                                        if self.snap_favs.remove(&old) {
                                            self.snap_favs.insert(new_name.clone());
                                        }
                                        if self.rename_edit.as_deref() == Some(old.as_str()) {
                                            self.rename_edit = None;
                                        }
                                        self.set_notice((false, format!("已改名: {new_name}")));
                                    }
                                    Err(e) => self.set_notice((true, e)),
                                }
                                refresh = true;
                            }
                            self.rename_edit = None;
                        }
                        if refresh {
                            self.snapshots = pla_ui::snapshot::list_snapshots(&self.doc_key);
                            self.snap_infos = pla_ui::snapshot::load_meta_infos(&self.doc_key);
                        }
                        for o in opens {
                            if let Err(e) = pla_ui::snapshot::open_in_plasticity(&o, &self.doc_key) {
                                open_err = Some(e);
                            }
                        }
                        if let Some(e) = open_err {
                            self.set_notice((true, e));
                        }
                    },
                    Some(icons),
                );
                // 图标 hover → HUD(工具栏右侧), 覆盖工具栏自身的 hint
                if let Some(t) = tip_slot.borrow_mut().take() {
                    self.hovered_hint = Some(t);
                }
                if do_snapshot.get() {
                    self.run_action(Action::SnapshotManual);
                }
                if do_open_dir.get() {
                    if let Err(e) = pla_ui::snapshot::open_snapshot_dir(&self.doc_key) {
                        self.set_notice((true, e));
                    }
                }
                if do_manage.get() {
                    self.snap_manage = !self.snap_manage;
                    self.snap_selected.clear();
                    self.rename_edit = None;
                }
                self.show_snapshots = flag;
            }
        }

        if self.show_entities {
            let desired = next_pos(&mut stack_y, PANEL_ENTITIES[1]);
            if let Some(desired) = desired {
                let mut flag = self.show_entities;
                // R7-11: 刷新实体钮移到本窗口头部(与快照墙同款: 图标 + HUD 悬浮提示)
                let do_refresh = std::rc::Rc::new(std::cell::Cell::new(false));
                // Rc<RefCell>: icons Box<dyn FnOnce> 要求 'static, 借用不得
                let tip_slot = std::rc::Rc::new(std::cell::RefCell::new(None::<(String, String)>));
                let (dr, ts) = (do_refresh.clone(), tip_slot.clone());
                let icons: Box<dyn FnOnce(&mut egui::Ui)> = Box::new(move |ui| {
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("🔄").size(13.0).color(egui::Color32::WHITE),
                        )
                        .frame(false)
                        .rounding(egui::Rounding::same(4.0)),
                    );
                    if resp.hovered() {
                        *ts.borrow_mut() = Some((
                            "🔄 刷新实体树".into(),
                            "经官方桥协议拉取 Plasticity 全树(LIST_VISIBLE)".into(),
                        ));
                    }
                    if resp.clicked() {
                        dr.set(true);
                    }
                });
                header_extra_and_show(
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
                        // ---- R7-14: 实时选择(CDP) — 视口里选什么这里实时显示什么 ----
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("实时选择: {}", self.cdp_status))
                                    .small()
                                    .color(COL_MUTED),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("清空勾选").clicked() {
                                    self.sel_cdp_members.clear();
                                }
                            });
                        });
                        if !self.live_sel.is_empty() {
                            egui::ScrollArea::vertical()
                                .id_salt("live-sel")
                                .max_height(90.0)
                                .show(ui, |ui| {
                                    for (kind, id, name, ctor) in self.live_sel.clone() {
                                        let key = format!("{kind}:{id}:{name}");
                                        let mut on = self
                                            .sel_cdp_members
                                            .iter()
                                            .any(|(k, i, n)| format!("{k}:{i}:{n}") == key);
                                        let label = if name.is_empty() {
                                            format!("{ctor} ({kind})")
                                        } else {
                                            format!("{name} ({kind})")
                                        };
                                        if ui.checkbox(&mut on, label).changed() {
                                            if on {
                                                self.sel_cdp_members
                                                    .push((kind.clone(), id.clone(), name.clone()));
                                            } else {
                                                self.sel_cdp_members
                                                    .retain(|(k, i, n)| format!("{k}:{i}:{n}") != key);
                                            }
                                        }
                                    }
                                });
                        } else {
                            ui.label(
                                egui::RichText::new("(空 — 在 Plasticity 视口/树里选择实体)")
                                    .small()
                                    .color(COL_MUTED),
                            );
                        }
                        ui.separator();
                        // ---- CDP 实体树: 含 bridge 不下发的 curves/instances ----
                        if !self.cdp_tree.is_empty() {
                            section_header(ui, "TREE (CDP)");
                            egui::ScrollArea::vertical()
                                .id_salt("cdp-tree")
                                .max_height(120.0)
                                .show(ui, |ui| {
                                    for (name, ctor, indent, sel) in self.cdp_tree.clone() {
                                        ui.horizontal(|ui| {
                                            ui.add_space(indent as f32 * 12.0);
                                            let color = if sel {
                                                COL_ACCENT
                                            } else {
                                                COL_TEXT
                                            };
                                            ui.label(
                                                egui::RichText::new(name)
                                                    .small()
                                                    .color(color),
                                            );
                                            ui.label(
                                                egui::RichText::new(ctor)
                                                    .small()
                                                    .color(COL_MUTED),
                                            );
                                        });
                                    }
                                });
                            ui.separator();
                        }
                        ui.separator();
                        egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                            if self.entities.is_empty() {
                                ui.label(
                                    egui::RichText::new("(空 — 点右上 🔄 刷新实体)").color(COL_MUTED),
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
                        // ---- R7-14: CDP 选择集 —— 保存勾选的实时选择 + 一键应用回 Plasticity ----
                        ui.horizontal(|ui| {
                            if ui.button("保存实时选择").clicked() {
                                let name = self.sel_name.trim().to_string();
                                if name.is_empty() {
                                    self.set_notice((true, "请先在名称框输入选择集名称".into()));
                                } else if self.sel_cdp_members.is_empty() {
                                    self.set_notice((true, "实时选择勾选为空".into()));
                                } else {
                                    match pla_ui::selection::save_cdp_selection(
                                        &self.doc_key,
                                        &name,
                                        &self.sel_cdp_members.clone(),
                                    ) {
                                        Ok(path) => self.set_notice((
                                            false,
                                            format!(
                                                "已保存 {} ({} 项) → {}",
                                                name,
                                                self.sel_cdp_members.len(),
                                                path
                                            ),
                                        )),
                                        Err(e) => self.set_notice((true, e)),
                                    }
                                }
                            }
                            if ui.button("应用到 Plasticity").clicked() {
                                let ids: Vec<u32> = self
                                    .sel_cdp_members
                                    .iter()
                                    .filter_map(|(_, id, _)| id.parse::<u32>().ok())
                                    .collect();
                                if ids.is_empty() {
                                    self.set_notice((true, "勾选项无可用的数值 id".into()));
                                } else {
                                    let n = ids.len();
                                    self.cdp.send(pla_bridge::cdp::CdpCmd::ApplyIds(ids));
                                    self.set_notice((
                                        false,
                                        format!("已应用 {n} 项选择到 Plasticity"),
                                    ));
                                }
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
                    Some(icons),
                );
                // 悬浮提示 → HUD(工具栏右侧), 与快照墙头部图标同款
                if let Some(t) = tip_slot.borrow_mut().take() {
                    self.hovered_hint = Some(t);
                }
                if do_refresh.get() {
                    self.run_action(Action::EntitiesRefresh);
                }
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
                        // R7-16: 尺寸关联 = 选择集 + 方向 + 数值(PRD P1-1)。
                        if !self.dim_loaded {
                            self.dim_sets =
                                pla_ui::selection::list_cdp_selections(&self.doc_key);
                            self.dim_loaded = true;
                        }
                        ui.horizontal(|ui| {
                            ui.label("关联选择集:");
                            let cur = self
                                .dim_sets
                                .get(self.dim_set_sel)
                                .map(|s| s.as_str())
                                .unwrap_or("(无)");
                            egui::ComboBox::from_id_salt("dim-set")
                                .selected_text(cur)
                                .show_ui(ui, |ui| {
                                    for (i, s) in self.dim_sets.iter().enumerate() {
                                        ui.selectable_value(&mut self.dim_set_sel, i, s);
                                    }
                                });
                            if ui.small_button("刷新").clicked() {
                                self.dim_sets =
                                    pla_ui::selection::list_cdp_selections(&self.doc_key);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("方向:");
                            for (i, ax) in ["X", "Y", "Z"].iter().enumerate() {
                                ui.selectable_value(&mut self.dim_axis, i, *ax);
                            }
                            ui.label("数值(毫米):");
                            ui.add(egui::DragValue::new(&mut self.dim_value).speed(0.1));
                        });
                        ui.horizontal(|ui| {
                            if ui.button("① 选中关联实体").clicked() {
                                let Some(name) = self.dim_sets.get(self.dim_set_sel).cloned() else {
                                    self.set_notice((true, "没有已保存的选择集 — 先在选择集窗口保存".into()));
                                    return;
                                };
                                match pla_ui::selection::load_cdp_selection(&self.doc_key, &name) {
                                    Some(members) => {
                                        let ids: Vec<u32> = members
                                            .iter()
                                            .filter_map(|m| m.id.parse::<u32>().ok())
                                            .collect();
                                        if ids.is_empty() {
                                            self.set_notice((true, "选择集无可用的数值 id".into()));
                                        } else {
                                            let n = ids.len();
                                            self.cdp
                                                .send(pla_bridge::cdp::CdpCmd::ApplyIds(ids));
                                            self.set_notice((
                                                false,
                                                format!("已选中 {n} 项 ({name})"),
                                            ));
                                        }
                                    }
                                    None => {
                                        self.set_notice((true, format!("选择集 {name} 读取失败")))
                                    }
                                }
                            }
                            if ui.button("💾 保存关联").clicked() {
                                let Some(name) = self.dim_sets.get(self.dim_set_sel).cloned() else {
                                    self.set_notice((true, "没有关联的选择集".into()));
                                    return;
                                };
                                let axis = ["X", "Y", "Z"][self.dim_axis.min(2)];
                                let exe = std::env::current_exe()
                                    .ok()
                                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                                let dir = exe
                                    .join("selections-cdp")
                                    .join(&self.doc_key);
                                let _ = std::fs::create_dir_all(&dir);
                                let json = serde_json::json!({
                                    "set": name, "axis": axis, "value": self.dim_value,
                                });
                                match std::fs::write(
                                    dir.join(format!("dim-{}.json", name)),
                                    serde_json::to_vec_pretty(&json).unwrap_or_default(),
                                ) {
                                    Ok(_) => self.set_notice((
                                        false,
                                        format!("尺寸关联已保存: {name} {axis}={}", self.dim_value),
                                    )),
                                    Err(e) => self.set_notice((true, format!("保存失败: {e}"))),
                                }
                            }
                        });
                        ui.label(
                            egui::RichText::new(
                                "② 应用平移: 命令参数化(move-item 交互状态机)待活体逆向 — ① 已可用",
                            )
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
                        section_header(ui, "AUTO SNAPSHOT");
                        ui.checkbox(&mut self.auto_snap_enabled, "启用自动快照(需 CDP 已连接)");
                        ui.horizontal(|ui| {
                            ui.label("定时(分钟):");
                            ui.add(
                                egui::DragValue::new(&mut self.auto_interval_min)
                                    .range(0..=1440),
                            );
                            ui.label("定步数:");
                            ui.add(egui::DragValue::new(&mut self.auto_steps).range(0..=1000));
                        });
                        ui.horizontal(|ui| {
                            ui.label("上限(个):");
                            ui.add(egui::DragValue::new(&mut self.snap_max_count).range(0..=500));
                            ui.label("保留(天):");
                            ui.add(egui::DragValue::new(&mut self.snap_max_age_days).range(0..=365));
                        });
                        ui.label(
                            egui::RichText::new(format!(
                                "当前步数: {} — 快照按文档保存副本, 未保存新内容时自动跳过",
                                if self.cdp_undo >= 0 { self.cdp_undo.to_string() } else { "-".into() }
                            ))
                            .small()
                            .color(COL_MUTED),
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
                                        pla_ui::hotkey::save_binds(&self.hotkeys);
                                    }
                                });
                            }
                            let conflicts = self.hotkeys.conflicts();
                            if !conflicts.is_empty() {
                                for (k, acts) in &conflicts {
                                    ui.colored_label(COL_RED, format!("! 冲突: {k} → {acts:?}"));
                                }
                            }
                        });
                    },
                );
                self.show_settings = flag;
            }
        }

        // ---- HUD 视口: 悬停提示 + 操作通知, 弹在工具栏右侧 ----
        // 渲染点在所有功能面板之后(R7-9): 面板视口本帧写入的 hover 才能被
        // 本帧渲染, 且不会被下一帧工具栏的帧首覆盖抢先清掉。
        if self.hovered_hint.is_some() || self.notice.is_some() {
            if let Some(tr) = toolbar_rect {
                let hint = self.hovered_hint.clone();
                let notice = self.notice.clone();
                let size = egui::vec2(
                    260.0,
                    if hint.is_some() && notice.is_some() { 104.0 } else { 56.0 },
                );
                let desired = egui::pos2(tr.right() + 8.0, tr.top() + 2.0);
                let mut hud = Some((hint, notice));
                let mut open_flag = true; // notice 的 × 关闭经它传出
                let builder = egui::ViewportBuilder::default()
                    .with_decorations(false)
                    .with_resizable(false)
                    .with_transparent(true)
                    .with_window_level(egui::WindowLevel::AlwaysOnTop)
                    .with_active(false) // 不抢焦点
                    .with_position(desired)
                    .with_inner_size(size);
                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("hud-popup"),
                    builder,
                    |hctx, class| {
                        let Some((hint, notice)) = hud.take() else { return };
                        if hctx.input(|i| i.viewport().close_requested()) {
                            open_flag = false;
                        }
                        let show_ui = |ui: &mut egui::Ui, open: &mut bool| {
                            if let Some((l1, l2)) = &hint {
                                ui.label(
                                    egui::RichText::new(l1)
                                        .size(12.5)
                                        .strong()
                                        .color(egui::Color32::WHITE),
                                );
                                ui.label(
                                    egui::RichText::new(l2)
                                        .size(11.5)
                                        .color(egui::Color32::from_rgb(0x9a, 0x9a, 0x9a)),
                                );
                            }
                            if let Some((is_err, msg)) = &notice {
                                if hint.is_some() {
                                    ui.separator();
                                }
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(if *is_err { "! 提示" } else { "√ 完成" })
                                            .size(12.5)
                                            .strong()
                                            .color(if *is_err { COL_RED } else { COL_ACCENT }),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        egui::RichText::new("×")
                                                            .size(12.0)
                                                            .color(egui::Color32::WHITE),
                                                    )
                                                    .frame(false),
                                                )
                                                .clicked()
                                            {
                                                *open = false;
                                            }
                                        },
                                    );
                                });
                                ui.label(
                                    egui::RichText::new(msg)
                                        .color(if *is_err { COL_RED } else { COL_ACCENT }),
                                );
                            }
                        };
                        match class {
                            egui::ViewportClass::Embedded
                            | egui::ViewportClass::Root
                            | egui::ViewportClass::Deferred => {
                                egui::Window::new("提示")
                                    .resizable(false)
                                    .collapsible(false)
                                    .show(hctx, |ui| show_ui(ui, &mut open_flag));
                            }
                            egui::ViewportClass::Immediate => {
                                egui::CentralPanel::default()
                                    .frame(panel_frame())
                                    .show(hctx, |ui| show_ui(ui, &mut open_flag));
                                let cur = hctx.input(|i| i.viewport().outer_rect);
                                if let Some(r) = cur {
                                    if (r.left() - desired.x).abs() > SNAP_EPS
                                        || (r.top() - desired.y).abs() > SNAP_EPS
                                    {
                                        hctx.send_viewport_cmd(
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
                                pla_ui::hotkey::save_binds(&self.hotkeys);
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

#[cfg(test)]
mod r717_tests {
    use super::*;

    #[test]
    fn doc_key_from_title_parses_plasticity_titles() {
        assert_eq!(PlaApp::doc_key_from_title("model1.plasticity - Plasticity"), "model1");
        assert_eq!(PlaApp::doc_key_from_title("*model1.plasticity - Plasticity"), "model1");
        assert_eq!(PlaApp::doc_key_from_title("Plasticity"), "untitled");
        assert_eq!(PlaApp::doc_key_from_title(""), "untitled");
        assert_eq!(PlaApp::doc_key_from_title("untitled - Plasticity"), "untitled");
        assert_eq!(PlaApp::doc_key_from_title("my-doc_v2.plasticity - Plasticity"), "my-doc_v2");
    }
}
