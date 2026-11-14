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

use pla_bridge::i18n::tr;
use pla_bridge::t;

use pla_ui::bridge_thread::{BridgeCmd, BridgeEvent, BridgeHandle};
use pla_ui::features::{Availability, Feature, Health};
use pla_ui::hotkey::{Action, Hotkey, HotkeyMap};
use pla_ui::round_corners;
use pla_ui::window_finder;

// ---------------- Plasticity 风格色板 ----------------
const COL_PANEL: egui::Color32 = egui::Color32::from_rgb(0x18, 0x18, 0x18);
const COL_BTN: egui::Color32 = egui::Color32::from_rgb(0x33, 0x33, 0x33);
const COL_BTN_HOVER: egui::Color32 = egui::Color32::from_rgb(0x3d, 0x3d, 0x3d);
/// Plasticity 主题强调黄(取色自用户 Plasticity 26.2.3 激活态按钮实测像素 #ca8a1e)。
/// 只用于"主题"类着色(激活按钮/选中/钉住手柄); **状态语义色不跟主题**, 见下方。
const COL_ACCENT: egui::Color32 = egui::Color32::from_rgb(0xca, 0x8a, 0x1e);
const COL_TEXT: egui::Color32 = egui::Color32::from_rgb(0xd4, 0xd4, 0xd4);
/// R15: 0x8a 在 #181818 底上对比度不足(用户反馈看不清) → 0xa0。
const COL_MUTED: egui::Color32 = egui::Color32::from_rgb(0xa0, 0xa0, 0xa0);
// ---- R15: 原生风格设置控件(取色参照 Plasticity 26.2.3 右侧属性面板) ----
/// 分组卡片底色(比面板底亮一级)。
const COL_CARD: egui::Color32 = egui::Color32::from_rgb(0x27, 0x27, 0x27);
/// 分组标题(大写)。
const COL_SECTION: egui::Color32 = egui::Color32::from_rgb(0xbc, 0xbc, 0xbc);
/// 数值框底色 / 悬停。
const COL_FIELD: egui::Color32 = egui::Color32::from_rgb(0x3a, 0x3a, 0x3a);
const COL_FIELD_HOVER: egui::Color32 = egui::Color32::from_rgb(0x46, 0x46, 0x46);
/// 步进按钮(−/+)底色。
const COL_STEPPER: egui::Color32 = egui::Color32::from_rgb(0x4a, 0x4a, 0x4a);
/// 开关关闭时的轨道色。
const COL_TOGGLE_OFF: egui::Color32 = egui::Color32::from_rgb(0x4e, 0x4e, 0x4e);
// ---- 状态语义色(固定, 与主题无关): 灰=未绑定 / 黄=连接中 / 绿=已连接·成功 / 红=失败 ----
const COL_YELLOW: egui::Color32 = egui::Color32::from_rgb(0xd9, 0xa8, 0x2c);
const COL_GREEN: egui::Color32 = egui::Color32::from_rgb(0x3e, 0xa3, 0x4d);
const COL_RED: egui::Color32 = egui::Color32::from_rgb(0xd0, 0x50, 0x45);
const COL_GRAY_DOT: egui::Color32 = egui::Color32::from_rgb(0x77, 0x77, 0x77);

// ---------------- 工具栏形态/吸附参数(可调) ----------------
/// 工具栏固定逻辑尺寸: 圆点+分隔线+6 按钮+分隔线+关闭键。
/// 宽 246(R8-1: +30 视口状态按钮); 高 30。
/// R30: +30 笔记按钮 → 276。
const TOOLBAR_SIZE: [f32; 2] = [276.0, 30.0];
/// 面板整体右移微调(逻辑 px, 按 ppp 缩放; R6-2e 用户要求再偏右 10)。
const SNAP_RIGHT_SHIFT: f32 = 10.0;
/// 吸附垂直微调(逻辑 pt, 按 ppp 缩放; 仅作用于探测命中分支)。
/// 文件名 ".plasticity" 含下伸字母 p/y, 像素包围盒中心偏下 ~1.5pt, 微调使工具栏在 43px 顶栏内居中。
const SNAP_Y_NUDGE_PT: f32 = -1.5;
/// 注: 勿对 panel ui 用 min_rect 测尺寸做自适应 — CentralPanel 的 min_rect
/// 含整个可用区, need=窗口+余量 每帧自增(R6-2b 已回退, 教训留档)。
/// 方形图标按钮边长。
const BTN_SIZE: f32 = 24.0;
/// 吸附定位(物理像素): 优先像素探测 Plasticity 顶栏「文件名文字块」右缘
/// (titlebar_probe, PrintWindow 离屏渲染不受本工具栏遮挡), 工具栏停在其右侧;
/// 探测失败(渲染失败/找不到文字块)降级为本固定偏移 — 预留足够距离越过文件名。
const SNAP_LEFT_OFFSET: f32 = 310.0;
/// 探测成功时, 文件名块右缘与工具栏左缘的间隙(物理像素)。
const SNAP_FILENAME_GAP: f32 = pla_ui::titlebar_probe::FILENAME_GAP_PX;
/// 吸附: 相对目标窗口顶边的下移(物理像素)。
const SNAP_TOP_GAP: i32 = 3;
/// 吸附/绑定轮询间隔。
const SNAP_POLL_MS: u64 = 300;
/// 文件名探测间隔(PrintWindow 有渲染开销, 比吸附轮询慢一档)。
const PROBE_POLL_MS: u64 = 1000;
/// 位置漂移小于该值(物理像素)不重发定位命令, 避免抖动。
const SNAP_EPS: f32 = 2.0;
/// R8-3: 拖回吸附范围 — 拖动位置进入顶栏吸附区(水平 ±200pt, 垂直 ±48pt)时恢复自动吸附。
const SNAP_REENTER_X_PT: f32 = 200.0;
const SNAP_REENTER_Y_PT: f32 = 48.0;
/// 提示浮窗自动关闭时长。
const NOTICE_TTL: Duration = Duration::from_secs(6);
/// DWM 系统圆角兜底轮询间隔(新面板在隐藏阶段即已设置, 见 viewport_reveal)。
const ROUND_POLL_MS: u64 = 500;

/// 全局圆角(工具栏/面板/按钮一致)。
const ROUNDING: f32 = 10.0;

/// 工具栏按钮列(横向排列, 顺序即产品顺序)。
/// R7-5: 手动快照移入快照墙头部(图标钮); R7-11: 刷新实体移入选择集窗口头部 —
/// 一级工具栏只留面板开关类按钮。R8-1: 加视口状态。
const TOOLBAR_ACTIONS: [Action; 6] = [
    Action::ToggleSnapshots,
    Action::ToggleEntities,
    Action::ToggleVpState,
    Action::ToggleDim,
    Action::ToggleNotes,
    Action::ToggleSettings,
];

/// R8-1: 视口状态面板尺寸(同快照墙)。
const PANEL_VPSTATE: [f32; 2] = [PANEL_SNAPSHOTS[0], PANEL_SNAPSHOTS[1]];

/// 面板逻辑尺寸 (宽, 高) — 子视口 OS 窗口大小。
/// R7-6: 快照墙改窄加高(232x480) — 一行一个快照, 缩略图撑满内容宽。
/// R29: 宽 232 → 280, 管理条(已选 N · 全选 · 反选 · 删除 · 完成)中英文都能一行放下。
const PANEL_SNAPSHOTS: [f32; 2] = [280.0, 480.0];
/// R7-20: 加高放选择集墙(96 滚动区)。
/// R23: 选择集面板 = 左列选择集墙(约 280) + 右列 ENTITIES(SEL_ENTITIES_COL_W)。
const PANEL_ENTITIES: [f32; 2] = [520.0, 560.0];
/// R23: 选择集面板右侧 ENTITIES 列宽。
const SEL_ENTITIES_COL_W: f32 = 220.0;
const PANEL_DIM: [f32; 2] = [280.0, 170.0];
const PANEL_SETTINGS: [f32; 2] = [300.0, 560.0];
/// F2: CDP 能力面板(点工具栏状态点打开)。
const PANEL_CAPS: [f32; 2] = [320.0, 600.0];

fn main() -> eframe::Result<()> {
    pla_ui::diag::install_panic_hook();
    pla_ui::diag::log("start");
    // R25: 界面语言(偏好文件; 默认简体中文)。
    pla_bridge::i18n::set(pla_ui::prefs::load_lang());
    // R28: 窗口图标(任务栏显示的是窗口图标; exe 资源里的同一张图由 build.rs 嵌入, 供资源管理器/开始菜单用)。
    let mut root = egui::ViewportBuilder::default();
    match eframe::icon_data::from_png_bytes(include_bytes!("../assets/pla-helper.png")) {
        Ok(icon) => root = root.with_icon(std::sync::Arc::new(icon)),
        Err(e) => pla_ui::diag::log(&format!("icon decode failed: {e}")),
    }
    let opts = eframe::NativeOptions {
        viewport: root
            .with_inner_size(TOOLBAR_SIZE)
            .with_title(ROOT_TITLE)
            .with_resizable(false)
            .with_decorations(false)
            .with_transparent(true)
            .with_window_level(egui::WindowLevel::AlwaysOnTop),
        ..Default::default()
    };
    let r = eframe::run_native(
        "pla-ui",
        opts,
        Box::new(|cc| Ok::<Box<dyn eframe::App>, _>(Box::new(PlaApp::new(cc)))),
    );
    pla_ui::diag::log(&format!("run_native returned: {:?}", r.as_ref().err()));
    r
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
    /// F2: CDP 能力面板(点工具栏状态点开关)。
    show_caps: bool,
    /// R30: 笔记面板(独立浮窗: 不互斥、不自动关、位置/大小记忆、可调大小)。
    show_notes: bool,
    notes: pla_ui::notes_ui::NotesPanel,
    /// 本次打开时交给 ViewportBuilder 的外框(打开期间不变 → egui 不会每帧重发定位, 不和 OS 拖动打架)。
    notes_open_rect: Option<[f32; 4]>,
    /// 上次写进偏好文件的外框(变化且松手后再写)。
    notes_saved_rect: Option<[f32; 4]>,
    /// R30b: 笔记窗口自管拖动/调大小会话(SetCapture + 屏幕坐标 delta; 不用 OS StartDrag/BeginResize)。
    notes_drag: Option<NotesDrag>,
    /// R32: 笔记面板 panic 被拦下后的错误信息(面板改显示错误 + 重新载入按钮)。
    notes_crash: Option<String>,
    notes_crashed_saved: bool,
    /// R8-1: 视口状态面板。
    show_vpstate: bool,
    /// R8b: 工具栏拖动位置(None=自动吸附; Some=用户拖到的位置, 逻辑 pt)。
    toolbar_drag_pos: Option<egui::Pos2>,
    toolbar_dragging: bool,
    /// R9: 自管拖动的起始鼠标位置(**屏幕物理像素**, 不能用窗口内相对坐标)。
    drag_origin_cursor: Option<(f32, f32)>,
    /// R9: 自管拖动的起始窗口外框左上角(逻辑 pt, 屏幕坐标系)。
    drag_origin_win: Option<egui::Pos2>,
    sel_name: String,
    sel_members: Vec<String>,
    /// CDP 会话句柄(R7-14): 实时选择/实体树通道(Plasticity 需 PLA-CDP 补丁版启动)。
    cdp: pla_bridge::cdp::CdpHandle,
    /// R25: CDP 连接状态(结构化; 文本按当前语言生成, 颜色按变体, 不再字符串匹配)。
    cdp_conn: pla_bridge::cdp::ConnState,
    /// F2: 最近一次能力探测(None = CDP 未连接/未探测)。功能可用性一律经 `features` 表判定。
    cdp_caps: Option<pla_bridge::shim::CapsReport>,
    /// 最近一次收到能力探测的时刻。
    cdp_caps_at: Option<Instant>,
    /// R19: 本实例工具栏根窗口 HWND(缓存)。
    root_hwnd: Option<isize>,
    /// R19: 上次置顶同步时刻。
    last_topmost_sync: Instant,
    /// 实时选择快照 (kind, id, name, ctor)。
    live_sel: Vec<pla_bridge::cdp::SelItem>,
    /// R7-26: 上一次实时选择的 key 集合("kind:id"), 用于侦测选择变化。
    live_sel_keys: Vec<String>,
    /// R7-26: 已发出「应用」命令, 等 CDP 回包后把焦点交还 Plasticity。
    pending_focus: bool,
    /// CDP 实体树平铺 (name, ctor, indent, selected)。
    cdp_tree: Vec<(String, String, i64, bool)>,
    /// 当前选择镜像 (kind, id, name, key=原始 id 元素 JSON, camera=选中时刻相机)。
    /// v7(R7-32): 纯镜像 — 与实时选择严格同步(选中即入/取消即删/空选择清空),
    /// 面板只读展示, 无勾选交互(用户决策: 避免误操作 + 相机绑定选中时刻)。
    sel_cdp_members: Vec<(String, String, String, Option<String>, Option<String>)>,
    /// v6: 最新实时相机(来自选择快照流), 保存选择集时随集存储。
    sel_camera: Option<String>,
    /// R7-20: 已保存 CDP 选择集(收藏优先); 打开面板/变更后置 sel_saved_loaded=false 重载。
    sel_saved: Vec<pla_ui::selection::CdpSelMeta>,
    sel_saved_loaded: bool,
    /// R7-20: 选择集改名中(正在改的原名 + 编辑缓冲)。
    sel_rename: Option<String>,
    sel_rename_buf: String,
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
    /// R7-27: Plasticity 侧最近一次命令报错(原文 + 堆栈)。
    cdp_err_msg: Option<String>,
    cdp_err_stack: String,
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
    /// R8-2: 快照单项删除确认中。
    snap_delete_confirm: Option<String>,
    /// R8-1: 选择集单项删除确认中。
    sel_delete_confirm: Option<String>,
    /// R35: 三个面板的"更新"确认中(项名) + 视口状态更新的目标(采集回包后覆盖它, 而不是新建)。
    snap_update_confirm: Option<String>,
    sel_update_confirm: Option<String>,
    vp_update_confirm: Option<String>,
    vp_update_target: Option<String>,
    /// R8-1: 视口状态面板数据。
    vp_states: Vec<pla_ui::vpstate::VpStateMeta>,
    vp_states_loaded: bool,
    vp_pending_state: Option<String>,
    vp_rename: Option<String>,
    vp_rename_buf: String,
    vp_delete_confirm: Option<String>,
    vp_pngs: std::collections::HashMap<String, std::sync::Arc<[u8]>>,
    vp_shot_path: Option<String>,
    /// R8-1: 选择集截图缓存 + 管理模式 + pending 保存流程。
    sel_pngs: std::collections::HashMap<String, std::sync::Arc<[u8]>>,
    sel_manage: bool,
    sel_selected: std::collections::HashSet<String>,
    /// R28: 视口状态面板自己的管理模式/勾选集(以前借用 sel_manage, 两个面板会一起进管理模式)。
    vp_manage: bool,
    vp_selected: std::collections::HashSet<String>,
    sel_save_pending: Option<(String, Vec<(String, String, String, Option<String>, Option<String>)>, Option<String>)>,
    sel_shot_path: Option<String>,
    /// 正在改名的快照名 + 编辑缓冲。
    rename_edit: Option<String>,
    rename_buf: String,
    /// 上次圆角轮询时刻(DWM 圆角状态在 thread_local ROUND_STATE)。
    last_round_poll: Instant,
    /// Q1: 方向键步进旋转 — 设置面板打开时从 Plasticity 配置重读。
    step_loaded: bool,
    /// 方向键绑定是否已在 keymap.json 里(None = 读取失败)。
    step_bound: Option<bool>,
    /// 步进角输入框缓冲(度)。
    step_deg_buf: String,
    /// 最近一次读写结果 (是否出错, 文本)。
    step_msg: Option<(bool, String)>,
    /// F7: CDP 代理守护(Plasticity 带调试端口时自动拉起 9333 代理)。
    proxy: pla_ui::cdp_mode::ProxyManager,
    /// F7: 是否已配置为 CDP 模式启动(None = 未读取)。
    cdp_startup: Option<bool>,
    /// F7: 最近一次配置结果 (是否出错, 文本)。
    cdp_startup_msg: Option<(bool, String)>,
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
    v.window_stroke = egui::Stroke::new(PANEL_BORDER_STROKE_W, COL_PANEL_BORDER);
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
    // R15: egui 0.29 的 set_style 只设"当前主题", 而主题默认跟随系统 —— 系统为浅色时
    // 二级面板拿到的是浅色主题控件(白底输入框、深色文字、悬停变黑), 贴在我们的深色面板上
    // 几乎看不见(用户截图)。固定深色主题, 并把同一套样式写进深/浅两个主题。
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
}

/// 角度显示: 最多两位小数, 去掉多余的 0(15 / 22.5 / 7.25)。
fn fmt_deg(d: f64) -> String {
    let s = format!("{d:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// 按字符截断(超长加 …)。
fn truncate_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

/// HUD 宽度(逻辑 pt)。
const HUD_W: f32 = 260.0;

/// HUD 尺寸: 宽固定, 高按内容实测(与 HUD 内的换行宽度一致)。
fn hud_size(
    ctx: &egui::Context,
    hint: &Option<(String, String)>,
    notice: &Option<(bool, String)>,
) -> egui::Vec2 {
    // 内容可用宽 = HUD 宽 - 面板外边距/内边距/边框(约 6pt×2) - 余量。
    let wrap_w = HUD_W - 18.0;
    let h = |text: &str, font: egui::FontId| {
        ctx.fonts(|f| f.layout(text.to_owned(), font, egui::Color32::WHITE, wrap_w).size().y)
    };
    let sp = ctx.style().spacing.item_spacing.y;
    let body = egui::TextStyle::Body.resolve(&ctx.style());
    let mut total = 10.0; // 上下内边距 + 边框
    if let Some((l1, l2)) = hint {
        total += h(l1, egui::FontId::proportional(12.5)) + sp + h(l2, egui::FontId::proportional(11.5));
    }
    if let Some((_, msg)) = notice {
        if hint.is_some() {
            total += sp * 2.0 + 6.0; // 分隔线
        }
        total += 20.0 + sp + h(msg, body);
    }
    egui::vec2(HUD_W, (total + 6.0).ceil())
}

// ================= F2: CDP 能力面板 =================

/// 能力面板一帧所需的数据快照(避免面板闭包借用 self)。
struct CapsView {
    caps: Option<pla_bridge::shim::CapsReport>,
    age_s: Option<u64>,
    cdp_conn: pla_bridge::cdp::ConnState,
    proxy_status: String,
    /// R27: 原设置面板 BRIDGE 卡的内容搬到这里(连接/通道信息集中在一个面板)。
    bridge_status: String,
    bridge_color: egui::Color32,
    bridge_url: String,
    /// 本助手绑定的 Plasticity 窗口标题(CDP 与 Bridge 都跟随它)。
    bound_title: Option<String>,
}

fn avail_style(a: &Availability) -> (&'static str, egui::Color32) {
    match a {
        Availability::Ok => (tr("可用", "OK"), COL_GREEN),
        Availability::Degraded(_) => (tr("部分可用", "Partial"), COL_YELLOW),
        Availability::Blocked(_) => (tr("不可用", "Unavailable"), COL_RED),
    }
}

/// 能力明细行: 色点 + 能力名 + 中文说明; 有缺失时下一行列出缺的内部结构(垫片 LOC 表键)。
fn cap_line(ui: &mut egui::Ui, name: &str, st: Option<&pla_bridge::shim::CapState>) {
    let (col, extra) = match st {
        None => (COL_MUTED, Some(tr("垫片未提供该能力", "Shim does not provide this capability").to_string())),
        Some(s) if s.todo.is_some() => (
            COL_MUTED,
            Some(t!("路线图预留({}), 尚未实现", "Roadmap ({}), not implemented yet", s.todo.as_deref().unwrap_or(""))),
        ),
        Some(s) if !s.ok => (COL_RED, Some(t!("缺: {}", "Missing: {}", s.missing.join(", ")))),
        Some(s) if !s.optional_missing.is_empty() => {
            (COL_YELLOW, Some(t!("缺可选: {}", "Missing optional: {}", s.optional_missing.join(", "))))
        }
        Some(_) => (COL_GREEN, None),
    };
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 14.0), egui::Sense::hover());
        ui.painter().circle_filled(r.center(), 3.0, col);
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("{name}  {}", pla_ui::features::cap_label(name)))
                    .size(11.0)
                    .color(COL_MUTED),
            )
            .wrap(),
        );
    });
    if let Some(t) = extra {
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            ui.add(egui::Label::new(egui::RichText::new(t).size(10.5).color(col)).wrap());
        });
    }
}

fn caps_panel_ui(ui: &mut egui::Ui, v: &CapsView, reprobe: &mut bool) {
    use pla_ui::features::{availability, Group, ROADMAP_CAPS};
    let caps = v.caps.as_ref();
    // ---- 概览 ----
    settings_card(
        ui,
        "OVERVIEW",
        Some((
            tr("CDP 能力", "CDP capabilities"),
            vec![
                info_text(tr(
                    "连接 Plasticity 时逐项探测 pla-ui 依赖的内部结构。某项失效说明当前 Plasticity 版本改了内部实现: \
                     相关功能会被拦截并说明原因, 不会静默失败。",
                    "On connecting to Plasticity, each internal structure pla-ui relies on is probed. A failure means this \
                     Plasticity version changed its internals: affected features are blocked with a reason, never failing silently.",
                )),
                info_text(tr(
                    "绿 = 可用; 黄 = 可用但缺可选子项; 红 = 不可用; 灰 = 预留 / 未知。",
                    "Green = OK; yellow = OK but optional parts missing; red = unavailable; gray = reserved / unknown.",
                )),
                info_text(tr(
                    "缺失项写的是垫片内部结构表的键; Plasticity 升级后在垫片表里补备选路径即可恢复。",
                    "Missing items are keys of the shim's structure table; after a Plasticity upgrade, add fallback paths there to restore them.",
                )),
                info_text(tr(
                    "工具栏状态点外环与本面板同源; 命令行 pla-app cdp-probe 输出原始探测结果。",
                    "The toolbar status dot's outer ring uses the same data; `pla-app cdp-probe` prints the raw probe result.",
                )),
            ],
        )),
        |ui| {
            let bound = v
                .bound_title
                .clone()
                .unwrap_or_else(|| tr("未绑定(等待 Plasticity…)", "Unbound (waiting for Plasticity…)").into());
            value_row(
                ui,
                tr("绑定窗口", "Bound window"),
                &bound,
                if v.bound_title.is_some() { COL_TEXT } else { COL_YELLOW },
            );
            value_row(ui, tr("连接", "Connection"), &v.cdp_conn.text(), cdp_conn_color(&v.cdp_conn));
            value_row(ui, tr("代理", "Proxy"), &v.proxy_status, COL_TEXT);
            match caps {
                None => value_row(
                    ui,
                    tr("能力", "Capabilities"),
                    tr("CDP 未连接, 能力未知", "CDP not connected, unknown"),
                    COL_MUTED,
                ),
                Some(r) => {
                    let ed = if r.editor {
                        t!("已找到(经 {})", "Found (via {})", r.finder.as_deref().unwrap_or("?"))
                    } else {
                        tr("未找到", "Not found").into()
                    };
                    value_row(ui, tr("编辑器", "Editor"), &ed, if r.editor { COL_GREEN } else { COL_RED });
                    let (b, d) = pla_ui::features::summary(caps);
                    let ok = Feature::ALL.len() - b - d;
                    let col = if b > 0 { COL_RED } else if d > 0 { COL_YELLOW } else { COL_GREEN };
                    value_row(
                        ui,
                        tr("功能", "Features"),
                        &t!("{ok} 可用 · {d} 部分 · {b} 不可用", "{ok} OK · {d} partial · {b} unavailable"),
                        col,
                    );
                    value_row(
                        ui,
                        tr("垫片", "Shim"),
                        &format!("api {} · rev {}", pla_bridge::shim::API_VERSION, r.rev.get(..8).unwrap_or(&r.rev)),
                        COL_MUTED,
                    );
                }
            }
            if let Some(s) = v.age_s {
                value_row(ui, tr("上次探测", "Last probe"), &t!("{s} 秒前", "{s}s ago"), COL_MUTED);
            }
            ui.add_space(2.0);
            let btn = egui::Button::new(egui::RichText::new(tr("重新探测", "Re-probe")).size(12.0).color(egui::Color32::WHITE))
                .rounding(egui::Rounding::same(6.0))
                .fill(COL_BTN)
                .min_size(egui::vec2(ui.available_width(), 24.0));
            if ui.add_enabled(caps.is_some(), btn).clicked() {
                *reprobe = true;
            }
        },
    );
    // ---- R27: 官方 Bridge(只读冗余通道, 原在设置面板) — 与 CDP 能力无关, 断开不影响上面任何功能 ----
    settings_card(
        ui,
        "BRIDGE",
        Some((
            tr("官方 Bridge", "Official Bridge"),
            vec![
                info_text(tr(
                    "Plasticity 自带的 WebSocket 服务(偏好设置 → Server, 端口 8980)。\
                     pla-ui 用它读取实体网格; 只读, 不能修改模型。",
                    "Plasticity's built-in WebSocket server (Preferences → Server, port 8980). \
                     pla-ui reads entity meshes through it; read-only, cannot modify the model.",
                )),
                info_text(tr(
                    "它是 CDP 之外的冗余通道: 断开只影响实体网格读取, 选择集/视口/快照等走 CDP 的功能照常。",
                    "A backup channel besides CDP: if it's down only entity-mesh reading is affected; \
                     selection sets, viewports, snapshots etc. go through CDP and keep working.",
                )),
            ],
        )),
        |ui| {
            value_row(ui, tr("状态", "Status"), &v.bridge_status, v.bridge_color);
            value_row(ui, tr("地址", "Address"), &v.bridge_url, COL_MUTED);
        },
    );
    // ---- 按组: 功能状态 + 所需能力明细 ----
    for g in Group::ALL {
        let (title, sub) = g.title();
        settings_card(ui, title, None, |ui| {
            ui.label(egui::RichText::new(sub).size(10.5).color(COL_MUTED));
            for f in g.features() {
                let a = availability(caps, f);
                let (txt, col) = avail_style(&a);
                value_row(ui, f.label(), txt, col);
                if let Some(r) = caps {
                    for c in f.caps() {
                        cap_line(ui, c, r.caps.get(*c));
                    }
                } else if let Availability::Blocked(why) = &a {
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.label(egui::RichText::new(why.as_str()).size(10.5).color(COL_MUTED));
                    });
                }
            }
        });
    }
    let Some(r) = caps else { return };
    // ---- 路线图预留 ----
    settings_card(ui, "ROADMAP", None, |ui| {
        ui.label(egui::RichText::new(tr(
            "后续功能所需能力(垫片已登记, 尚未实现)",
            "Capabilities for upcoming features (registered in shim, not implemented yet)",
        )).size(10.5).color(COL_MUTED));
        for c in ROADMAP_CAPS {
            cap_line(ui, c, r.caps.get(*c));
        }
    });
    // ---- 其他能力: 探测结果里有、但没有任何功能引用的(自动收录新能力, 保证完整) ----
    let used: std::collections::HashSet<&str> = Feature::ALL
        .iter()
        .flat_map(|f| f.caps().iter().copied())
        .chain(ROADMAP_CAPS.iter().copied())
        .collect();
    let others: Vec<&String> = r.caps.keys().filter(|k| !used.contains(k.as_str())).collect();
    if !others.is_empty() {
        settings_card(ui, "OTHER", None, |ui| {
            for c in others {
                cap_line(ui, c, r.caps.get(c));
            }
        });
    }
    // ---- 备选路径: Plasticity 改过名、垫片靠备选路径兜住的结构 ----
    if !r.alt_paths.is_empty() {
        settings_card(ui, "ALT PATHS", None, |ui| {
            ui.label(egui::RichText::new(tr(
                "这些结构走了垫片表里的备选路径(说明 Plasticity 改过名)",
                "These structures resolved via fallback paths in the shim table (Plasticity renamed them)",
            )).size(10.5).color(COL_MUTED));
            for (k, p) in &r.alt_paths {
                ui.add(egui::Label::new(egui::RichText::new(format!("{k} → {p}")).size(11.0).color(COL_YELLOW)).wrap());
            }
        });
    }
}

/// R23: 管理模式的「全选 / 反选」(快照墙、选择集、视口状态共用, 行为一致)。
/// 全选: 已全部选中时再按 = 全部取消; 否则选中全部。反选: 逐项取反。
fn select_all_invert_buttons(
    ui: &mut egui::Ui,
    all: &[String],
    selected: &mut std::collections::HashSet<String>,
) {
    let everything = !all.is_empty() && all.iter().all(|n| selected.contains(n));
    if ui
        .small_button(if everything { tr("取消全选", "None") } else { tr("全选", "All") })
        .on_hover_text(tr("选中全部; 已全部选中时再按 = 全部取消", "Select all; press again when all selected = clear"))
        .clicked()
    {
        if everything {
            selected.clear();
        } else {
            selected.extend(all.iter().cloned());
        }
    }
    if ui
        .small_button(tr("反选", "Invert"))
        .on_hover_text(tr("选中的取消、没选中的选中", "Selected become unselected and vice versa"))
        .clicked()
    {
        let next: std::collections::HashSet<String> =
            all.iter().filter(|n| !selected.contains(*n)).cloned().collect();
        *selected = next;
    }
}

/// R28: 管理模式操作条 —— 快照墙 / 选择集 / 视口状态三处共用(位置都在列表顶部, 样式一致):
/// 「已选 N」+ 全选/反选 + 红色删除 + 完成。返回 (点了删除且有勾选, 点了完成), 删除动作由调用方执行。
fn manage_bar(
    ui: &mut egui::Ui,
    all: &[String],
    selected: &mut std::collections::HashSet<String>,
    delete_tip: &str,
) -> (bool, bool) {
    let mut out = (false, false);
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(t!("已选 {}", "{} selected", selected.len())).small().color(COL_MUTED));
        select_all_invert_buttons(ui, all, selected);
        let del = ui
            .add(
                egui::Button::new(egui::RichText::new(tr("删除所选", "Delete")).small().color(egui::Color32::WHITE))
                    .fill(egui::Color32::from_rgb(150, 45, 45)),
            )
            .on_hover_text(delete_tip);
        out.0 = del.clicked() && !selected.is_empty();
        out.1 = ui.small_button(tr("完成", "Done")).clicked();
    });
    ui.separator();
    out
}

#[cfg(test)]
mod select_all_tests {
    // 纯逻辑部分(全选切换 / 反选)在 UI 外单测。
    fn toggle_all(all: &[&str], sel: &mut std::collections::HashSet<String>) {
        let everything = !all.is_empty() && all.iter().all(|n| sel.contains(*n));
        if everything { sel.clear() } else { sel.extend(all.iter().map(|s| s.to_string())) }
    }
    #[test]
    fn select_all_toggles_and_invert_flips() {
        let all = ["a", "b", "c"];
        let mut sel: std::collections::HashSet<String> = ["b".to_string()].into();
        toggle_all(&all, &mut sel);
        assert_eq!(sel.len(), 3, "部分选中 → 全选");
        toggle_all(&all, &mut sel);
        assert!(sel.is_empty(), "全部选中时再按 = 全部取消");
        let mut sel: std::collections::HashSet<String> = ["a".to_string()].into();
        let inv: std::collections::HashSet<String> = all.iter().filter(|n| !sel.contains(**n)).map(|s| s.to_string()).collect();
        sel = inv;
        let mut v: Vec<_> = sel.into_iter().collect();
        v.sort();
        assert_eq!(v, ["b", "c"]);
    }
}

/// CDP 连接状态 → 圆点颜色(U7 语义: 黄 连接中 / 绿 已连接 / 红 未连上)。按变体判定, 与界面语言无关。
fn cdp_conn_color(c: &pla_bridge::cdp::ConnState) -> egui::Color32 {
    use pla_bridge::cdp::ConnState;
    match c {
        ConnState::Connected => COL_GREEN,
        ConnState::Connecting | ConnState::Timeout => COL_YELLOW,
        _ => COL_RED,
    }
}

/// bridge 状态文本是否为「已连接」(兼容中英文状态文本 — 文本由 bridge 线程按当时语言生成)。
fn bridge_is_connected(status: &str) -> bool {
    status.starts_with("已连接") || status.starts_with("Connected")
}

/// bridge 状态文本 → 圆点颜色。
fn status_color(status: &str) -> egui::Color32 {
    if bridge_is_connected(status) {
        COL_GREEN
    } else if status.starts_with("连接中") || status.starts_with("Connecting") {
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

// ================= R15: 原生风格设置控件 + 悬停三级面板 =================
//
// 视觉参照 Plasticity 26.2.3 右侧属性面板: 分组 = 圆角卡片 + 大写粗体标题; 每行
// 左标签右控件; 开关 = 胶囊轨道(开 = 强调黄, 白色圆钮); 数值 = 圆角深灰框, 白色居中;
// 步进数值两侧 −/+。说明文字不占面板空间, 悬停时在面板旁弹出三级面板。

/// 三级面板的一行。
#[derive(Clone)]
enum InfoLine {
    /// 普通说明(自动换行)。
    Text(String),
    /// 快捷键: 键帽 + 说明。
    Key(String, String),
}

fn info_text(s: impl Into<String>) -> InfoLine {
    InfoLine::Text(s.into())
}

fn info_key(k: &str, d: &str) -> InfoLine {
    InfoLine::Key(k.into(), d.into())
}

/// 本帧要显示的三级面板。
#[derive(Clone)]
struct InfoHint {
    title: &'static str,
    lines: Vec<InfoLine>,
    /// 发起悬停的面板屏幕矩形(逻辑 pt)— 三级面板贴其右侧, 放不下则左侧。
    panel: egui::Rect,
    /// 悬停行的屏幕 y(逻辑 pt)。
    row_y: f32,
}

thread_local! {
    static INFO_HINT: std::cell::RefCell<Option<InfoHint>> = std::cell::RefCell::new(None);
    /// 三级面板实测内容高度(按标题), 隐藏渲染阶段测出, 显示前即调整好窗口高度。
    static INFO_HEIGHT: std::cell::RefCell<std::collections::HashMap<&'static str, f32>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// 悬停时登记三级面板(由根视口在面板旁渲染)。
fn hover_info(ui: &egui::Ui, rect: egui::Rect, hovered: bool, title: &'static str, lines: Vec<InfoLine>) {
    if !hovered {
        return;
    }
    let Some(inner) = ui.ctx().input(|i| i.viewport().inner_rect) else {
        return;
    };
    INFO_HINT.with(|h| {
        *h.borrow_mut() = Some(InfoHint { title, lines, panel: inner, row_y: inner.min.y + rect.min.y });
    });
}

/// 画文字; bold = 横向错开 0.6pt 叠画(内置 SimHei 无粗体字形, 用伪粗体)。
fn paint_text(
    painter: &egui::Painter,
    pos: egui::Pos2,
    anchor: egui::Align2,
    text: &str,
    size: f32,
    color: egui::Color32,
    bold: bool,
) -> egui::Rect {
    let font = egui::FontId::proportional(size);
    let r = painter.text(pos, anchor, text, font.clone(), color);
    if bold {
        painter.text(pos + egui::vec2(0.6, 0.0), anchor, text, font, color);
    }
    r
}

fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// 设置行高度。
const ROW_H: f32 = 28.0;
/// 右侧控件(数值框)占行宽比例。
const FIELD_FRAC: f32 = 0.5;

/// 行标签(常态浅灰, 悬停变白加粗); 有说明时在标签后画一个小 "?" 圈。
/// R27: 文本适配宽度 —— 先把字号从 `size` 缩到 `min_size`(英文通常比中文长), 仍放不下再省略号截断。
/// 返回 (显示文本, 字号, 是否截断)。
fn fit_text(ui: &egui::Ui, text: &str, max_w: f32, size: f32, min_size: f32) -> (String, f32, bool) {
    let w = |s: &str, sz: f32| {
        ui.fonts(|f| f.layout_no_wrap(s.into(), egui::FontId::proportional(sz), egui::Color32::WHITE).size().x)
    };
    let mut sz = size;
    while sz > min_size && w(text, sz) > max_w {
        sz -= 0.5;
    }
    if w(text, sz) <= max_w {
        return (text.to_string(), sz, false);
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len();
    while n > 1 && w(&format!("{}…", chars[..n].iter().collect::<String>()), sz) > max_w {
        n -= 1;
    }
    (format!("{}…", chars[..n].iter().collect::<String>()), sz, true)
}

/// 行标签: 限定在 `max_w` 内(缩字号/省略号, 截断时悬停显示全文)。
fn paint_row_label(ui: &egui::Ui, rect: egui::Rect, text: &str, hovered: bool, has_info: bool, max_w: f32) {
    let max_w = (max_w - if has_info { 18.0 } else { 0.0 }).max(20.0);
    let (shown, size, cut) = fit_text(ui, text, max_w, 12.5, 10.5);
    let r = paint_text(
        ui.painter(),
        egui::pos2(rect.left(), rect.center().y),
        egui::Align2::LEFT_CENTER,
        &shown,
        size,
        if hovered { egui::Color32::WHITE } else { COL_TEXT },
        hovered,
    );
    if cut && hovered {
        egui::show_tooltip_text(ui.ctx(), ui.layer_id(), ui.id().with(("row-label", text)), text);
    }
    if has_info {
        let c = egui::pos2(r.right() + 9.0, rect.center().y);
        let col = if hovered { egui::Color32::WHITE } else { COL_MUTED };
        ui.painter().circle_stroke(c, 5.5, egui::Stroke::new(1.0, col));
        ui.painter().text(c, egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(8.5), col);
    }
}

/// 原生风格分组卡片: 圆角浅一级底 + 大写粗体标题(标题悬停可弹说明)。
fn settings_card<R>(
    ui: &mut egui::Ui,
    title: &str,
    info: Option<(&'static str, Vec<InfoLine>)>,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::none()
        .fill(COL_CARD)
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin { left: 10.0, right: 10.0, top: 7.0, bottom: 5.0 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
            let hovered = resp.hovered() && info.is_some();
            let r = paint_text(
                ui.painter(),
                egui::pos2(rect.left(), rect.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                11.5,
                if hovered { egui::Color32::WHITE } else { COL_SECTION },
                true,
            );
            if let Some((t, lines)) = info {
                let c = egui::pos2(r.right() + 9.0, rect.center().y);
                let col = if hovered { egui::Color32::WHITE } else { COL_MUTED };
                ui.painter().circle_stroke(c, 5.5, egui::Stroke::new(1.0, col));
                ui.painter().text(c, egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(8.5), col);
                hover_info(ui, rect, resp.hovered(), t, lines);
            }
            add(ui)
        })
        .inner
}

/// 开关行: 整行可点; 返回 Response(changed = 本帧切换)。
fn toggle_row(
    ui: &mut egui::Ui,
    label: &str,
    on: &mut bool,
    info: Option<(&'static str, Vec<InfoLine>)>,
) -> egui::Response {
    let (rect, mut resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let hovered = resp.hovered();
    paint_row_label(ui, rect, label, hovered, info.is_some(), rect.width() - 34.0 - 10.0);
    let t = ui.ctx().animate_bool(resp.id, *on);
    let track = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 17.0, rect.center().y),
        egui::vec2(34.0, 18.0),
    );
    let off = if hovered { lerp_color(COL_TOGGLE_OFF, egui::Color32::WHITE, 0.12) } else { COL_TOGGLE_OFF };
    ui.painter().rect_filled(track, egui::Rounding::same(9.0), lerp_color(off, COL_ACCENT, t));
    let kx = egui::lerp((track.left() + 9.0)..=(track.right() - 9.0), t);
    ui.painter().circle_filled(egui::pos2(kx, track.center().y), 6.5, egui::Color32::WHITE);
    if let Some((title, lines)) = info {
        hover_info(ui, rect, hovered, title, lines);
    }
    resp
}

/// 只读信息行: 左标签, 右侧值(过长省略)。
fn value_row(ui: &mut egui::Ui, label: &str, value: &str, color: egui::Color32) {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
    // R27: 标签与值分宽 — 值最多占 68%, 标签用剩下的(英文标签/值都可能变长, 不再互相压字)。
    let (shown, vsize, _) = fit_text(ui, value, rect.width() * 0.68, 12.0, 10.5);
    let font = egui::FontId::proportional(vsize);
    let vw = ui.fonts(|f| f.layout_no_wrap(shown.clone(), font.clone(), color).size().x);
    paint_row_label(ui, rect, label, resp.hovered(), false, rect.width() - vw - 10.0);
    ui.painter().text(
        egui::pos2(rect.right(), rect.center().y),
        egui::Align2::RIGHT_CENTER,
        &shown,
        font,
        color,
    );
    if shown != value {
        resp.on_hover_text(value);
    }
}

/// 右侧数值框矩形。
fn field_rect(row: egui::Rect) -> egui::Rect {
    let w = row.width() * FIELD_FRAC;
    egui::Rect::from_min_size(
        egui::pos2(row.right() - w, row.center().y - 12.0),
        egui::vec2(w, 24.0),
    )
}

/// 可直接输入 + 两侧 −/+ 的数值行。返回 (提交: 回车或失焦, 步进: -1/0/+1)。
fn stepper_row(
    ui: &mut egui::Ui,
    label: &str,
    buf: &mut String,
    suffix: &str,
    info: Option<(&'static str, Vec<InfoLine>)>,
) -> (bool, i32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), egui::Sense::hover());
    let hovered = ui.rect_contains_pointer(rect);
    let field = field_rect(rect);
    paint_row_label(ui, rect, label, hovered, info.is_some(), field.left() - rect.left() - 6.0);
    ui.painter().rect_filled(field, egui::Rounding::same(6.0), COL_FIELD);
    let bw = 24.0;
    let minus = egui::Rect::from_min_size(field.min, egui::vec2(bw, field.height()));
    let plus = egui::Rect::from_min_size(egui::pos2(field.right() - bw, field.top()), egui::vec2(bw, field.height()));
    let mut step = 0;
    for (r, sign, glyph, round) in [
        (minus, -1, "−", egui::Rounding { nw: 6.0, sw: 6.0, ne: 0.0, se: 0.0 }),
        (plus, 1, "+", egui::Rounding { nw: 0.0, sw: 0.0, ne: 6.0, se: 6.0 }),
    ] {
        let b = ui.interact(r, ui.id().with((label, sign)), egui::Sense::click());
        let fill = if b.hovered() { lerp_color(COL_STEPPER, egui::Color32::WHITE, 0.15) } else { COL_STEPPER };
        ui.painter().rect_filled(r, round, fill);
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(14.0), egui::Color32::WHITE);
        if b.clicked() {
            step = sign;
        }
    }
    let mid = egui::Rect::from_min_max(egui::pos2(minus.right(), field.top()), egui::pos2(plus.left(), field.bottom()));
    let edit = ui.put(
        mid.shrink2(egui::vec2(4.0, 3.0)),
        egui::TextEdit::singleline(buf)
            .frame(false)
            .horizontal_align(egui::Align::Center)
            .font(egui::FontId::proportional(13.0))
            .text_color(egui::Color32::WHITE)
            .hint_text(tr("默认", "Default")),
    );
    if !suffix.is_empty() {
        ui.painter().text(
            egui::pos2(mid.right() - 6.0, mid.center().y),
            egui::Align2::RIGHT_CENTER,
            suffix,
            egui::FontId::proportional(12.0),
            COL_MUTED,
        );
    }
    if let Some((title, lines)) = info {
        hover_info(ui, rect, hovered, title, lines);
    }
    (edit.lost_focus(), step)
}

/// 拖拽/点击编辑的整数行(原生风格数值框)。返回 changed。
fn drag_row<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    info: Option<(&'static str, Vec<InfoLine>)>,
) -> bool {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), egui::Sense::hover());
    let hovered = ui.rect_contains_pointer(rect);
    let field = field_rect(rect);
    paint_row_label(ui, rect, label, hovered, info.is_some(), field.left() - rect.left() - 6.0);
    let changed = ui
        .allocate_new_ui(egui::UiBuilder::new().max_rect(field), |ui| {
            let v = ui.visuals_mut();
            for (w, fill) in [
                (&mut v.widgets.inactive, COL_FIELD),
                (&mut v.widgets.hovered, COL_FIELD_HOVER),
                (&mut v.widgets.active, COL_FIELD_HOVER),
            ] {
                w.weak_bg_fill = fill;
                w.bg_fill = fill;
                w.bg_stroke = egui::Stroke::NONE;
                w.rounding = egui::Rounding::same(6.0);
                w.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
                w.expansion = 0.0;
            }
            ui.style_mut().override_font_id = Some(egui::FontId::proportional(13.0));
            ui.add_sized(field.size(), egui::DragValue::new(value).range(range)).changed()
        })
        .inner;
    if let Some((title, lines)) = info {
        hover_info(ui, rect, hovered, title, lines);
    }
    changed
}

/// 渲染三级面板(由根视口在所有面板之后调用, 与 HUD 同理: 面板本帧写入的悬停才能本帧显示)。
fn show_info_popup(ctx: &egui::Context, h: InfoHint) {
    const W: f32 = 320.0;
    let key = h.title;
    let height = INFO_HEIGHT.with(|m| m.borrow().get(key).copied()).unwrap_or(120.0);
    let monitor_w = ctx.input(|i| i.viewport().monitor_size).map_or(f32::MAX, |s| s.x);
    let x = if h.panel.right() + 8.0 + W <= monitor_w {
        h.panel.right() + 8.0
    } else {
        h.panel.left() - 8.0 - W
    };
    let y = (h.row_y - 10.0).max(h.panel.top());
    let (hide, reveal) = viewport_reveal(ctx, key);
    let mut builder = egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_resizable(false)
        .with_transparent(true)
        .with_window_level(egui::WindowLevel::AlwaysOnTop)
        .with_active(false)
        .with_position(egui::pos2(x, y))
        .with_inner_size(egui::vec2(W, height));
    if hide {
        builder = builder.with_visible(false);
    }
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(("info-popup", key)), builder, |ictx, _class| {
        if reveal {
            ictx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
        egui::CentralPanel::default()
            .frame(panel_frame().inner_margin(egui::Margin::symmetric(12.0, 10.0)))
            .show(ictx, |ui| {
                let top = ui.cursor().min.y;
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.label(egui::RichText::new(h.title).size(13.0).strong().color(egui::Color32::WHITE));
                for line in &h.lines {
                    match line {
                        InfoLine::Text(t) => {
                            ui.add(egui::Label::new(egui::RichText::new(t).size(12.0).color(COL_TEXT)).wrap());
                        }
                        InfoLine::Key(k, d) => {
                            ui.horizontal(|ui| {
                                egui::Frame::none()
                                    .fill(COL_FIELD)
                                    .rounding(egui::Rounding::same(4.0))
                                    .inner_margin(egui::Margin::symmetric(6.0, 2.0))
                                    .show(ui, |ui| {
                                        ui.label(egui::RichText::new(k).size(12.0).color(egui::Color32::WHITE));
                                    });
                                // 显式换行: 横排里 Label 默认不换行, 超宽会撑大内容区,
                                // 连带后面的说明文字都越过右边框(R15 实测)。
                                ui.add(egui::Label::new(egui::RichText::new(d).size(12.0).color(COL_TEXT)).wrap());
                            });
                        }
                    }
                }
                // 实测内容高度(用光标位置, 不用 min_rect — CentralPanel 的 min_rect 含整个可用区)
                let content = ui.cursor().min.y - top;
                let want = (content + 10.0 * 2.0 + 2.0 * PANEL_BORDER_INSET + 4.0).ceil();
                INFO_HEIGHT.with(|m| {
                    let mut m = m.borrow_mut();
                    if m.get(key).map_or(true, |old| (old - want).abs() > 0.5) {
                        m.insert(key, want);
                        ictx.request_repaint();
                    }
                });
            });
        paint_panel_border(ictx);
    });
}

/// 面板浅灰轮廓宽度(逻辑 pt)与颜色。
/// R13: 1px(R7-19)看不清、3pt(R12)太粗 → 1.5pt(150% 缩放下约 2 物理像素)。
/// epaint 0.29 的矩形描边以边线为中心向两侧各扩 w/2, 外侧再加约 0.5px 抗锯齿羽化;
/// 内缩 = w/2 + 0.5 让整条描边(含羽化)完整落在窗口内, 不被窗口边缘切掉。
const PANEL_BORDER_STROKE_W: f32 = 1.5;
const PANEL_BORDER_INSET: f32 = PANEL_BORDER_STROKE_W / 2.0 + 0.5;
const COL_PANEL_BORDER: egui::Color32 = egui::Color32::from_gray(180);
/// 面板窗口外圆角(逻辑 pt) = Win11 DWM 系统圆角半径(8 逻辑 px, 随 DPI 缩放)。
/// egui 轮廓内缩 PANEL_BORDER_INSET 后与系统裁剪弧同心, 圆角处不被切、不露黑角。
const PANEL_OUTER_ROUNDING: f32 = 8.0;

/// 工具栏/子视口共用的圆角深色面板 Frame。
fn panel_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(COL_PANEL)
        .rounding(egui::Rounding::same((PANEL_OUTER_ROUNDING - PANEL_BORDER_INSET).max(2.0)))
        .outer_margin(egui::Margin::same(PANEL_BORDER_INSET))
        .inner_margin(egui::Margin {
            left: 6.0 - PANEL_BORDER_INSET,
            right: 6.0 - PANEL_BORDER_INSET,
            top: 3.0 - PANEL_BORDER_INSET,
            bottom: 3.0 - PANEL_BORDER_INSET,
        })
        .stroke(egui::Stroke::new(PANEL_BORDER_STROKE_W, COL_PANEL_BORDER))
}

thread_local! {
    /// R13: DWM 圆角已处理窗口表(见 round_corners.rs)。
    static ROUND_STATE: std::cell::RefCell<round_corners::RoundState> =
        std::cell::RefCell::new(round_corners::RoundState::new());
}

/// 根视口(工具栏)窗口标题 — 二级面板以它为 owner(不占任务栏)。
const ROOT_TITLE: &str = "PLA Toolbar";

/// R13: 给本进程新出现的顶层窗口设置 DWM 系统圆角, 并把面板挂到工具栏名下(不占任务栏)。
fn apply_round_corners_now() {
    ROUND_STATE.with(|s| round_corners::apply_round_corners(&mut s.borrow_mut(), ROOT_TITLE));
}

thread_local! {
    /// R13: 子视口"年龄" — key → (上次渲染所在的根帧号, 连续渲染帧数)。
    /// 中间断档(面板关过)即归零, 无需显式清理。
    static VIEWPORT_AGE: std::cell::RefCell<std::collections::HashMap<&'static str, (u64, u32)>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// R13: 新建子视口先隐藏渲染这么多帧(定位 + 首帧画好)再显示。
/// 实测(屏幕连拍): 不隐藏时新窗口先以白底未绘制状态、在错误位置出现, 要等下一次
/// 重绘才画出内容 — 无输入时可持续 1s 以上。
const VIEWPORT_SHOW_AFTER_FRAMES: u32 = 2;

/// 返回 key 对应子视口已连续渲染的帧数(本帧首次渲染 = 0)。
fn viewport_age(ctx: &egui::Context, key: &'static str) -> u32 {
    let nr = ctx.cumulative_pass_nr();
    VIEWPORT_AGE.with(|m| {
        let mut m = m.borrow_mut();
        let e = m.entry(key).or_insert((nr, 0));
        if e.0 + 1 == nr {
            *e = (nr, e.1.saturating_add(1));
        } else if e.0 != nr {
            *e = (nr, 0);
        }
        e.1
    })
}

/// 子视口显隐阶段: 返回 (builder 需强制隐藏, 本帧需在子视口内补发 Visible(true))。
/// 未到显示帧时请求重绘, 保证无输入也能走完隐藏阶段。
///
/// 注意 egui 0.29 的坑: `ViewportBuilder::patch` 判断 visible 变化时误与
/// `self.active` 比较(viewport.rs:731), with_active(true) 的窗口从隐藏改为可见时
/// 永远不发 Visible(true)。所以显示一律在子视口内显式 send_viewport_cmd,
/// 显示后 builder 不再带 with_visible(否则 active 窗口会被每帧重新隐藏)。
fn viewport_reveal(ctx: &egui::Context, key: &'static str) -> (bool, bool) {
    let age = viewport_age(ctx, key);
    let hide = age < VIEWPORT_SHOW_AFTER_FRAMES;
    if hide {
        ctx.request_repaint();
        // 窗口在 age 0 那帧创建(隐藏); 之后的隐藏帧里设好 DWM 圆角, 显示时即为圆角。
        if age >= 1 {
            apply_round_corners_now();
        }
    }
    // 连发几帧(winit 对已可见窗口的 Visible(true) 是空操作), 防单帧命令丢失。
    let reveal = (VIEWPORT_SHOW_AFTER_FRAMES..VIEWPORT_SHOW_AFTER_FRAMES + 3).contains(&age);
    (hide, reveal)
}

/// 在视口前景层补绘浅灰圆角边框, 防止内部控件(如标题栏分割线/滚动区)压盖轮廓。
fn paint_panel_border(ctx: &egui::Context) {
    let rect = ctx.screen_rect().shrink(PANEL_BORDER_INSET);
    if rect.width() > 4.0 && rect.height() > 4.0 {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("pla_panel_border"),
        ));
        painter.add(egui::epaint::RectShape::stroke(
            rect,
            egui::Rounding::same((PANEL_OUTER_ROUNDING - PANEL_BORDER_INSET).max(2.0)),
            egui::Stroke::new(PANEL_BORDER_STROKE_W, COL_PANEL_BORDER),
        ));
    }
}

thread_local! {
    static PANEL_DRAG_HINT: std::cell::RefCell<bool> =
        const { std::cell::RefCell::new(false) };
    /// R8d: 标题行正在被拖动(本帧) — 面板视口据此启动拖动会话。
    static PANEL_DRAG_ACTIVE: std::cell::RefCell<bool> =
        const { std::cell::RefCell::new(false) };
    /// R9: 面板 id → 进行中的拖动会话(起始指针/起始窗口位/是否接管)。
    static PANEL_DRAG: std::cell::RefCell<std::collections::HashMap<&'static str, PanelDragState>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    /// R9: 面板 id → 最近一次下发的定位目标。目标没变就不重发 OuterPosition
    /// (DPI 取整会让「实际-目标」永远差零点几 pt, 不记目标就会每帧 SetWindowPos
    /// = 面板抖动/瞬移的元凶之一)。
    static PANEL_LAST_CMD: std::cell::RefCell<std::collections::HashMap<&'static str, egui::Pos2>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    /// 面板id → 用户拖到的绝对位置(逻辑 pt)。主循环渲染时优先于默认跟随位;
    /// 面板关闭后由主循环清除(重开回默认)。
    static PANEL_USER_POS: std::cell::RefCell<std::collections::HashMap<&'static str, egui::Pos2>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    /// R9-2: 面板id → 剩余等待帧数(降级 OS 拖动时, 等窗口真的被挪走后把落点
    /// 补记为"用户位置", 否则它还会跟着工具栏跑)。
    static PANEL_PENDING_USER: std::cell::RefCell<std::collections::HashMap<&'static str, u8>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    /// R9-2: 主工具栏正在拖动(一级面板拖动中不要判定"用户挪了二级面板")。
    static PRIMARY_DRAGGING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// R9: 面板拖动会话状态(自管拖动: SetCapture + 指针 delta)。
#[derive(Clone, Copy)]
struct PanelDragState {
    /// true = 已接管(拿到 hwnd 并 SetCapture); false = 降级给 OS StartDrag。
    active: bool,
    /// 按下瞬间的鼠标位置(**屏幕物理像素**)。
    origin_cursor: (f32, f32),
    /// 按下瞬间的窗口外框左上角(逻辑 pt, 屏幕坐标系)。
    origin_win: egui::Pos2,
}

impl Default for PanelDragState {
    fn default() -> Self {
        Self {
            active: false,
            origin_cursor: (0.0, 0.0),
            origin_win: egui::Pos2::ZERO,
        }
    }
}

/// R8b: 二级面板标识(互斥用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelId {
    Snapshots,
    Entities,
    VpState,
    Dim,
    Settings,
    Caps,
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
    /// R8-2: 删除按钮被点(单删, 带确认)。
    delete: bool,
    /// R35: 更新按钮被点(用当前状态覆盖, 带确认)。
    update: bool,
}

/// R35: 更新按钮(旋转箭头圆圈, 矢量绘制): 3/4 圆弧 + 箭头。
fn refresh_button(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    let col = if resp.hovered() { COL_ACCENT } else { COL_MUTED };
    let c = rect.center();
    let r = size * 0.36;
    let st = egui::Stroke::new((size / 11.0).max(1.0), col);
    // 从 -60° 顺时针画到 230°(留一个口给箭头)
    let (a0, a1) = (-60.0f32.to_radians(), 230.0f32.to_radians());
    let n = 18;
    let pts: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            egui::pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    ui.painter().add(egui::Shape::line(pts, st));
    // 箭头: 在起点(-60°)处, 沿切线方向
    let tip = egui::pos2(c.x + r * a0.cos(), c.y + r * a0.sin());
    let t = egui::vec2(-a0.sin(), a0.cos()); // 顺时针切线
    let nrm = egui::vec2(a0.cos(), a0.sin());
    let h = size * 0.28;
    ui.painter().add(egui::Shape::convex_polygon(
        vec![tip - t * h * 0.15, tip - t * h + nrm * h * 0.6, tip - t * h - nrm * h * 0.6],
        col,
        egui::Stroke::NONE,
    ));
    resp
}

/// R26: 截图纹理 URI —— egui 按 URI 缓存解码后的纹理, 只用名字(SS001/SG001…)做 URI 时,
/// 换文件或删掉重拍同名项会一直显示第一张图。URI 里带上内容指纹(长度 + 稀疏采样哈希),
/// 内容不同 → URI 不同 → 必然重新解码。采样而非全量哈希: 每帧每张图都要算, PNG 可达数 MB。
fn png_uri(kind: &str, name: &str, bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a
    let step = (bytes.len() / 2048).max(1);
    for b in bytes.iter().step_by(step).chain(bytes.iter().rev().take(64)) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("bytes://{kind}/{name}#{}-{h:016x}", bytes.len())
}

/// R29: 截图宽高比(Plasticity 窗口截图约 16:10)。缩略图高度都从格子宽度推出 —— 面板改宽/改窄时,
/// 收藏区(3 列)和全部区(整宽)一起按比例缩放, 不再有写死的高度(以前收藏固定 56、全部区固定 96)。
const THUMB_ASPECT: f32 = 0.62;

/// 收藏格(3 列小图)的图片高度。
fn fav_img_h_for(cell_w: f32) -> f32 {
    (cell_w * THUMB_ASPECT).clamp(36.0, 120.0)
}

/// 全部区(整宽大图)的图片高度。
fn big_img_h_for(w: f32) -> f32 {
    (w * THUMB_ASPECT).clamp(60.0, 200.0)
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
                egui::Image::from_bytes(png_uri("snap", name, bytes), bytes.clone())
                    .fit_to_exact_size(egui::vec2(iw, ih))
                    .sense(egui::Sense::click()),
            )
        } else {
            ui.add_sized(
                egui::vec2(iw, ih),
                egui::Label::new(
                    egui::RichText::new(tr("无截图", "No image")).small().color(COL_MUTED),
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
    // R8e: 宽卡单行 36; 窄卡两行 48。
    let card_h = if cell_w >= 140.0 { 36.0 } else { 48.0 };
    ui.allocate_ui_with_layout(
        egui::vec2(cell_w, card_h),
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
            // R8e(用户反馈): 宽卡(全部区, cell_w>=140)恢复单行(名称+按钮同排
            // 空间足够); 窄卡(收藏三列 ~66px)保持两行。图标缩小到 11px 且
            // 两行模式按钮行上移贴近名称(负 spacing 补偿)。
            let wide = cell_w >= 140.0;
            if !wide {
                // R9: 名称行整体上移贴近缩略图(用户反馈: 与按钮之间空隙太大)
                ui.add_space(-2.0);
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
                    let sp = ui.spacing().item_spacing.x;
                    // 宽卡: 名称与按钮同排(扣 3 钮+管理勾选); 窄卡: 名称独占
                    let icons_w = if wide { (11.0 + sp) * 3.0 } else { 0.0 };
                    let reserve = if manage { 16.0 + sp } else { 0.0 } + icons_w;
                    let rem = (cell_w - reserve).max(24.0);
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
                    nr.on_hover_text(if manage { &stem } else { tr("点击在新窗口打开快照", "Click to open snapshot in a new window") })
                };
                if name_resp.clicked() && !manage {
                    a.open = true;
                }
                // R8e: 宽卡 — 按钮并入名称行(同排)
                if wide {
                    draw_item_buttons(ui, is_fav, &mut a);
                }
            });
            // 窄卡第二行: 按钮组(★ ✎ 🗑), 11px 紧凑。
            // R9: 负 spacing 把按钮行压到名称行紧贴(R8e 的 -2 用户仍嫌大)。
            if !wide {
                ui.add_space(-5.0);
                ui.horizontal(|ui| {
                    draw_item_buttons(ui, is_fav, &mut a);
                });
            }
        },
    );
    a
}

/// R8e: 卡片按钮组(★ 收藏星 + ✎ 改名 + 🗑 删除) — 11px 紧凑版。
/// 宽卡与名称同排, 窄卡独立一行。
fn draw_item_buttons(ui: &mut egui::Ui, is_fav: bool, a: &mut SnapItemActions) {
    // 收藏星
    let star_col = if is_fav {
        egui::Color32::from_rgb(255, 200, 60)
    } else {
        COL_MUTED
    };
    let (srect, sresp) = ui.allocate_exact_size(egui::vec2(11.0, 11.0), egui::Sense::click());
    ui.painter().text(
        srect.center(),
        egui::Align2::CENTER_CENTER,
        if is_fav { "★" } else { "☆" },
        egui::FontId::proportional(10.0),
        star_col,
    );
    if sresp
        .on_hover_text(if is_fav { tr("取消收藏", "Unfavorite") } else { tr("收藏(置顶小图)", "Favorite (pin to top)") })
        .clicked()
    {
        a.fav_toggle = true;
    }
    // 改名铅笔(11px)
    let (prect, presp) = ui.allocate_exact_size(egui::vec2(11.0, 11.0), egui::Sense::click());
    let pcol = if presp.hovered() { COL_TEXT } else { COL_MUTED };
    let ps = 11.0 / 24.0;
    let pp = |x: f32, y: f32| egui::pos2(prect.left() + x * ps, prect.top() + y * ps);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            pp(2.0, 22.0),
            pp(7.5, 20.5),
            pp(21.0, 7.0),
            pp(17.0, 3.0),
            pp(3.5, 16.5),
        ],
        pcol,
        egui::Stroke::NONE,
    ));
    if presp.on_hover_text(tr("改名", "Rename")).clicked() {
        a.rename_start = true;
    }
    // R35: 更新(用当前状态覆盖)
    if refresh_button(ui, 11.0)
        .on_hover_text(tr("更新: 用当前文档覆盖这个快照(需确认)", "Update: overwrite this snapshot with the current document (confirm)"))
        .clicked()
    {
        a.update = true;
    }
    // 删除垃圾桶(11px, 线稿缩放 11/14)
    let (drect, dresp) = ui.allocate_exact_size(egui::vec2(11.0, 11.0), egui::Sense::click());
    let dcol = if dresp.hovered() { COL_RED } else { COL_MUTED };
    let k = 11.0 / 14.0;
    let p = |x: f32, y: f32| egui::pos2(drect.left() + x * k, drect.top() + y * k);
    let st = egui::Stroke::new(1.1, dcol);
    ui.painter().line_segment([p(3.0, 4.0), p(11.0, 4.0)], st);
    ui.painter().line_segment([p(5.5, 4.0), p(5.5, 2.5)], st);
    ui.painter().line_segment([p(8.5, 4.0), p(8.5, 2.5)], st);
    ui.painter().line_segment([p(6.2, 2.5), p(7.8, 2.5)], st);
    ui.painter().line_segment([p(4.0, 4.0), p(5.0, 12.0)], st);
    ui.painter().line_segment([p(10.0, 4.0), p(9.0, 12.0)], st);
    ui.painter().line_segment([p(5.0, 12.0), p(9.0, 12.0)], st);
    if dresp.on_hover_text(tr("删除(需确认)", "Delete (confirm)")).clicked() {
        a.delete = true;
    }
}


/// R8b: 头部图标按钮种类(SimHei 无 emoji 字形, 全部矢量绘制)。
#[derive(Clone, Copy, PartialEq, Eq)]
enum IconKind {
    Trash,
    Folder,
    Camera,
    Save,
}

/// 矢量图标钮: 14x14 画布上按 kind 画线稿。返回点击响应。
fn vector_icon_button(
    ui: &mut egui::Ui,
    kind: IconKind,
    tip: &str,
    tip_slot: &std::cell::RefCell<Option<(String, String)>>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 14.0), egui::Sense::click());
    let col = if resp.hovered() { COL_TEXT } else { COL_MUTED };
    let p = |x: f32, y: f32| egui::pos2(rect.left() + x, rect.top() + y);
    let st = egui::Stroke::new(1.3, col);
    match kind {
        IconKind::Trash => {
            // 桶身+盖+提手
            ui.painter().line_segment([p(3.0, 4.0), p(11.0, 4.0)], st);
            ui.painter().line_segment([p(5.5, 4.0), p(5.5, 2.5)], st);
            ui.painter().line_segment([p(8.5, 4.0), p(8.5, 2.5)], st);
            ui.painter().line_segment([p(6.0, 2.5), p(8.0, 2.5)], st);
            ui.painter().line_segment([p(4.2, 4.0), p(5.2, 12.0)], st);
            ui.painter().line_segment([p(9.8, 4.0), p(8.8, 12.0)], st);
            ui.painter().line_segment([p(5.2, 12.0), p(8.8, 12.0)], st);
            // 桶内竖线
            ui.painter().line_segment([p(7.0, 6.0), p(7.0, 10.0)], egui::Stroke::new(1.0, col));
        }
        IconKind::Folder => {
            // 文件夹: 顶舌 + 盒体
            ui.painter().line_segment([p(2.0, 4.0), p(5.0, 4.0)], st);
            ui.painter().line_segment([p(5.0, 4.0), p(6.0, 2.8)], st);
            ui.painter().line_segment([p(6.0, 2.8), p(9.0, 2.8)], st);
            ui.painter().line_segment([p(9.0, 2.8), p(14.0, 2.8)], st);
            ui.painter().line_segment([p(14.0, 2.8), p(14.0, 11.5)], st);
            ui.painter().line_segment([p(14.0, 11.5), p(2.0, 11.5)], st);
            ui.painter().line_segment([p(2.0, 11.5), p(2.0, 4.0)], st);
        }
        IconKind::Camera => {
            // 相机: 机身 + 镜头圆
            ui.painter().line_segment([p(2.0, 4.5), p(4.0, 4.5)], st);
            ui.painter().line_segment([p(4.0, 4.5), p(5.2, 3.0)], st);
            ui.painter().line_segment([p(5.2, 3.0), p(9.8, 3.0)], st);
            ui.painter().line_segment([p(9.8, 3.0), p(11.0, 4.5)], st);
            ui.painter().line_segment([p(11.0, 4.5), p(14.0, 4.5)], st);
            ui.painter().line_segment([p(14.0, 4.5), p(14.0, 11.5)], st);
            ui.painter().line_segment([p(14.0, 11.5), p(2.0, 11.5)], st);
            ui.painter().line_segment([p(2.0, 11.5), p(2.0, 4.5)], st);
            ui.painter()
                .circle_stroke(egui::pos2(rect.left() + 8.0, rect.top() + 8.0), 2.2, st);
        }
        IconKind::Save => {
            // 软盘: 外框 + 上缺口标签 + 下标签线
            ui.painter().line_segment([p(2.0, 2.5), p(11.0, 2.5)], st);
            ui.painter().line_segment([p(11.0, 2.5), p(14.0, 5.5)], st);
            ui.painter().line_segment([p(14.0, 5.5), p(14.0, 11.5)], st);
            ui.painter().line_segment([p(14.0, 11.5), p(2.0, 11.5)], st);
            ui.painter().line_segment([p(2.0, 11.5), p(2.0, 2.5)], st);
            ui.painter().line_segment([p(5.0, 2.5), p(5.0, 6.0)], st);
            ui.painter().line_segment([p(5.0, 6.0), p(10.5, 6.0)], st);
            ui.painter().line_segment([p(10.5, 6.0), p(10.5, 2.5)], st);
            ui.painter().line_segment([p(5.0, 9.5), p(11.0, 9.5)], st);
        }
    }
    if resp.hovered() {
        *tip_slot.borrow_mut() = Some((tip.to_string(), String::new()));
    }
    resp
}

/// R8b: 通用删除确认弹层 — 覆盖在面板视口顶部居中(egui::Area + 新 layer)。
/// 不占内容流空间(行内确认条会被 auto_shrink 滚动区推出可视区, 实测不可见)。
/// 返回 Some(true)=确认, Some(false)=取消, None=仍在展示。
fn confirm_popup(
    ctx: &egui::Context,
    id: &str,
    text: &str,
) -> Option<bool> {
    let mut result = None;
    egui::Area::new(egui::Id::new(format!("confirm-{id}")))
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 24.0))
        .order(egui::Order::Foreground)
        .interactable(true)
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(egui::Color32::from_rgb(0x24, 0x24, 0x24))
                .rounding(egui::Rounding::same(8.0))
                .stroke(egui::Stroke::new(1.0, COL_RED))
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    ui.set_min_width(200.0);
                    ui.label(
                        egui::RichText::new(text).small().color(COL_RED),
                    );
                    ui.horizontal(|ui| {
                        if ui.button(tr("确认删除", "Delete")).clicked() {
                            result = Some(true);
                        }
                        if ui.button(tr("取消", "Cancel")).clicked() {
                            result = Some(false);
                        }
                    });
                });
        });
    result
}

fn panel_header(
    ui: &mut egui::Ui,
    title: &str,
    id: &'static str,
    open: &mut bool,
    icons: Option<Box<dyn FnOnce(&mut egui::Ui)>>,
) {
    // R8e: 标题整行 = OS 级拖动区。先画行内容拿到行矩形, 再铺 interact drag 层
    // (egui 后注册者 z 序在上; 按钮先画在下层 — drag 层会挡住按钮点击?
    // 不会: egui 命中测试按"后注册优先", drag 层后注册会拦截按钮…
    // 反过来: 按钮需要的 click 会被 drag 层吃掉。因此顺序 = 先注册 drag 层,
    // 再画按钮(按钮在上层, 可点; 行空白 = drag 层, 可拖)。
    let row_rect = ui.available_rect_before_wrap();
    let row_h = 20.0;
    let drag_rect = egui::Rect::from_min_size(row_rect.min, egui::vec2(row_rect.width(), row_h));
    let _drag_layer = ui.interact(
        drag_rect,
        egui::Id::new("panel-title-drag"),
        egui::Sense::click_and_drag(),
    );
    ui.horizontal(|ui| {
        ui.set_min_height(row_h);
        // 拖动视觉提示(2列x3点, 矢量)
        // R9-2: 双击手柄或标题空白处 = 归位(清掉用户位置, 重新跟随工具栏下方默认位)。
        let (hr, hresp) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
        let pinned = PANEL_USER_POS.with(|u| u.borrow().contains_key(id));
        let col = if hresp.hovered() {
            COL_TEXT
        } else if pinned {
            COL_ACCENT
        } else {
            COL_MUTED
        };
        for cx in [hr.left() + 4.0, hr.left() + 9.0] {
            for cy in [hr.top() + 3.5, hr.center().y, hr.bottom() - 3.5] {
                ui.painter().circle_filled(egui::pos2(cx, cy), 1.3, col);
            }
        }
        if hresp.hovered() {
            PANEL_DRAG_HINT.with(|d| *d.borrow_mut() = true);
        }
        if hresp.double_clicked() || _drag_layer.double_clicked() {
            PANEL_USER_POS.with(|u| {
                u.borrow_mut().remove(id);
            });
            PANEL_LAST_CMD.with(|u| {
                u.borrow_mut().remove(id);
            });
        }
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
    // 拖动检测在 horizontal 之后(响应聚合): drag 层响应直接查
    if _drag_layer.dragged_by(egui::PointerButton::Primary) {
        PANEL_DRAG_ACTIVE.with(|d| *d.borrow_mut() = true);
    }
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
            bridge_status: tr("连接中…", "Connecting…").into(),
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
            show_caps: false,
            show_notes: false,
            notes: pla_ui::notes_ui::NotesPanel::new(),
            notes_open_rect: None,
            notes_saved_rect: pla_ui::prefs::load_notes_rect(),
            notes_drag: None,
            notes_crash: None,
            notes_crashed_saved: false,
            step_loaded: false,
            step_bound: None,
            step_deg_buf: String::new(),
            step_msg: None,
            proxy: pla_ui::cdp_mode::ProxyManager::default(),
            cdp_startup: None,
            cdp_startup_msg: None,
            show_vpstate: false,
    toolbar_drag_pos: None,
    toolbar_dragging: false,
    // R9: 自管拖动的起始基准(鼠标屏幕位置/窗口位), 松手即清。
    drag_origin_cursor: None,
    drag_origin_win: None,
            sel_name: String::new(),
            sel_members: Vec::new(),
            cdp: pla_bridge::cdp::CdpHandle::spawn(9333),
            cdp_conn: pla_bridge::cdp::ConnState::Connecting,
            cdp_caps: None,
            cdp_caps_at: None,
            root_hwnd: None,
            last_topmost_sync: Instant::now(),
            live_sel: Vec::new(),
            live_sel_keys: Vec::new(),
            pending_focus: false,
            cdp_tree: Vec::new(),
            sel_cdp_members: Vec::new(),
            sel_camera: None,
            sel_saved: Vec::new(),
            sel_saved_loaded: false,
            sel_rename: None,
            sel_rename_buf: String::new(),
            auto_snap_enabled: true,
            auto_interval_min: 10,
            auto_steps: 20,
            snap_max_count: 50,
            snap_max_age_days: 7,
            last_auto_time: Instant::now(),
            last_auto_step: -1,
            last_auto_mtime: None,
            cdp_undo: -1,
            cdp_err_msg: None,
            cdp_err_stack: String::new(),
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
            snap_delete_confirm: None,
            sel_delete_confirm: None,
            snap_update_confirm: None,
            sel_update_confirm: None,
            vp_update_confirm: None,
            vp_update_target: None,
            vp_states: Vec::new(),
            vp_states_loaded: false,
            vp_pending_state: None,
            vp_rename: None,
            vp_rename_buf: String::new(),
            vp_delete_confirm: None,
            vp_pngs: std::collections::HashMap::new(),
            vp_shot_path: None,
            sel_pngs: std::collections::HashMap::new(),
            sel_manage: false,
            sel_selected: std::collections::HashSet::new(),
            vp_manage: false,
            vp_selected: std::collections::HashSet::new(),
            sel_save_pending: None,
            sel_shot_path: None,
            rename_edit: None,
            rename_buf: String::new(),
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
                pla_bridge::cdp::CdpEvent::Conn(c) => {
                    self.cdp_conn = c;
                }
                // R25: 操作结果走通知(成功绿 / 失败红), 不再覆盖连接状态。
                pla_bridge::cdp::CdpEvent::Report { ok, msg } => {
                    self.set_notice((!ok, msg));
                    // R7-26: 「应用」回包后把焦点交还 Plasticity ——
                    // 工具栏是独立窗口, 焦点留在它上面时 Plasticity 收不到键盘
                    // 事件, 主程序"任何操作都不生效"(用户实测)。
                    if self.pending_focus {
                        self.pending_focus = false;
                        self.focus_plasticity();
                    }
                }
                pla_bridge::cdp::CdpEvent::Caps(c) => {
                    self.cdp_caps_at = c.as_ref().map(|_| Instant::now());
                    self.cdp_caps = c;
                }
                pla_bridge::cdp::CdpEvent::Selection(snap) => {
                    if snap.undo_len >= 0 {
                        self.cdp_undo = snap.undo_len;
                    }
                    // v6(R7-31l): 持续跟踪最新相机(保存选择集时随集存储)。
                    if snap.camera.is_some() {
                        self.sel_camera = snap.camera.clone();
                    }
                    // R7-27: Plasticity 侧命令报错(例如 G 移动的 "reading 'active'")
                    // 直接显示在助手面板, 不用只盯着 Plasticity 的弹窗。
                    if let Some((msg, stack)) = snap.error.as_ref() {
                        if self.cdp_err_msg.as_deref() != Some(msg.as_str()) {
                            self.cdp_err_msg = Some(msg.clone());
                            self.cdp_err_stack = stack.clone();
                            self.set_notice((true, t!("Plasticity 报错: {msg}", "Plasticity error: {msg}")));
                        }
                    }
                    if snap.ok {
                        // v7(R7-32): 勾选区 = 实时选择的纯镜像。选择一变就整表重建:
                        // 选中即入(相机绑定"选中时刻"的同拍快照), 取消即删,
                        // 全不选 → 清空(旧 R7-26 语义"空选择不动勾选区"是为
                        // 手动勾选/载入编辑而设, 交互移除后不再成立)。
                        let mut new_keys: Vec<String> = snap
                            .items
                            .iter()
                            .map(|it| pla_ui::selection::member_key(&it.kind, &it.id))
                            .collect();
                        new_keys.sort();
                        if new_keys != self.live_sel_keys {
                            // v7 + v6.1 多视角语义: 新成员绑定"选中时刻"相机
                            // (本快照同拍); 已在镜像中的成员**保留其原相机**
                            // (A 面视角1选中 → 转视角选 B → A 仍是视角1,
                            //  应用时按各自视角分组点击)。
                            let old = std::mem::take(&mut self.sel_cdp_members);
                            self.sel_cdp_members = snap
                                .items
                                .iter()
                                .map(|it| {
                                    let key = pla_ui::selection::member_key(&it.kind, &it.id);
                                    let kept_cam = old.iter().find_map(|(k, i, _, _, c)| {
                                        (pla_ui::selection::member_key(k, i) == key)
                                            .then(|| c.clone())
                                            .flatten()
                                    });
                                    (
                                        it.kind.to_string(),
                                        it.id.clone(),
                                        it.name.clone(),
                                        it.key.clone(),
                                        kept_cam.or_else(|| snap.camera.clone()),
                                    )
                                })
                                .collect();
                            self.live_sel_keys = new_keys;
                        }
                        self.live_sel = snap.items;
                    } else if self.live_sel.is_empty() && snap.state == "no-editor" {
                        // 编辑器尚未就绪: 保持状态行即可
                    } else {
                        self.live_sel.clear();
                    }
                }
                pla_bridge::cdp::CdpEvent::CallResult { name, result } => match result {
                    // 即时生效类调用成功时不打扰(面板上已显示"立即生效"); 失败如实提示。
                    Ok(v) => pla_ui::diag::log(&format!("CDP {name} ok: {v}")),
                    Err(e) => self.set_notice((true, t!("即时生效失败({name}): {e}", "Live apply failed ({name}): {e}"))),
                },
                // R8-1: 视口状态采集回包 → 暂存并立即补发截图命令(串行化,
                // 避免 VpStateSave/CaptureViewport 回包交错导致 pending 空)。
                pla_bridge::cdp::CdpEvent::VpStateResult(state) => {
                    self.vp_pending_state = Some(state);
                    if let Some(path) = self.vp_shot_path.clone() {
                        self.cdp
                            .send(pla_bridge::cdp::CdpCmd::CaptureViewport(path));
                    }
                }
                // R8-1: 视口截图回包 → 读盘, 完成 pending 存档。
                // (len=0 = 截图失败 — 跳过读盘, pending 照常落盘为无图版本)
                pla_bridge::cdp::CdpEvent::ViewportPng(png_len) => {
                    let png_bytes = if png_len > 0 {
                        self.vp_shot_path.clone().and_then(|p| std::fs::read(p).ok())
                    } else {
                        None
                    };
                    // R35: 更新已有项(覆盖, 名字/收藏保留)
                    if let (Some(target), Some(state)) = (self.vp_update_target.clone(), self.vp_pending_state.clone()) {
                        self.vp_update_target = None;
                        self.vp_pending_state = None;
                        match pla_ui::vpstate::update_vpstate(&self.doc_key, &target, &state, png_bytes.as_deref()) {
                            Ok(()) => {
                                self.vp_states_loaded = false;
                                self.vp_pngs.clear();
                                self.set_notice((false, t!("视口状态已更新: {target}", "View state updated: {target}")));
                            }
                            Err(e) => self.set_notice((true, e)),
                        }
                    }
                    // 视口状态存档
                    if let (Some(state), Some(png)) =
                        (self.vp_pending_state.take(), png_bytes.as_deref())
                    {
                        match pla_ui::vpstate::save_vpstate(
                            &self.doc_key,
                            &state,
                            Some(png),
                        ) {
                            Ok(stem) => {
                                self.vp_states_loaded = false;
                                self.vp_pngs.clear();
                                self.set_notice((false, t!("视口状态已保存: {stem}", "View state saved: {stem}")));
                            }
                            Err(e) => self.set_notice((true, e)),
                        }
                    } else if let Some(state) = self.vp_pending_state.take() {
                        // 截图失败: 仍然保存(无图)
                        match pla_ui::vpstate::save_vpstate(&self.doc_key, &state, None) {
                            Ok(stem) => {
                                self.vp_states_loaded = false;
                                self.set_notice((
                                    false,
                                    t!(
                                        "视口状态已保存: {stem}(截图失败, 无缩略图)",
                                        "View state saved: {stem} (capture failed, no thumbnail)"
                                    ),
                                ));
                            }
                            Err(e) => self.set_notice((true, e)),
                        }
                    }
                    // 选择集存档(pending = (SG名, members, camera))
                    if let Some((name, members, cam)) = self.sel_save_pending.take() {
                        let png2 = if png_len > 0 {
                            self.sel_shot_path.clone().and_then(|p| std::fs::read(p).ok())
                        } else {
                            None
                        };
                        match pla_ui::selection::save_cdp_selection_with_camera(
                            &self.doc_key,
                            &name,
                            &members,
                            cam.as_deref(),
                            png2.as_deref(),
                        ) {
                            Ok(_) => {
                                self.sel_saved_loaded = false;
                                self.sel_pngs.clear();
                                self.set_notice((
                                    false,
                                    t!("已保存 {} ({} 项, 含截图)", "Saved {} ({} items, with image)", name, members.len()),
                                ));
                            }
                            Err(e) => self.set_notice((true, e)),
                        }
                    }
                }
                pla_bridge::cdp::CdpEvent::RequestFocus => self.focus_plasticity(),
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
        if !self.auto_snap_enabled || !self.cdp_conn.is_connected() {
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
                        self.notice = Some((false, t!("快照已保存: {name}", "Snapshot saved: {name}")));
                    }
                    Err(e) => self.notice = Some((true, e)),
                }
            }
            Action::SnapshotRestore => {
                self.notice = Some((
                    true,
                    tr(
                        "切换快照未接线: 恢复写入需要 PLA-004 G4 闭环",
                        "Snapshot switching not wired: restore needs the PLA-004 G4 loop",
                    )
                    .into(),
                ));
            }
            Action::SelSave => {
                let doc = self.doc_key.clone();
                let name = self.sel_name.clone();
                let members = self.sel_members.clone();
                match pla_ui::selection::save_selection(&doc, &name, &members) {
                    Ok(path) => self.notice = Some((false, t!("选择集已保存: {path}", "Selection set saved: {path}"))),
                    Err(e) => self.notice = Some((true, e)),
                }
            }
            Action::SelApply => {
                self.notice = Some((
                    true,
                    tr(
                        "应用选择集需要 PUT_SOME 组创建语义(PLA-004 G4), 未接线",
                        "Applying selection sets needs PUT_SOME group semantics (PLA-004 G4), not wired",
                    )
                    .into(),
                ));
            }
            Action::DimApply => {
                self.notice = Some((
                    true,
                    tr(
                        "尺寸驱动需要 facet 读写闭环(PLA-004 G4 后), 未接线",
                        "Dimension drive needs facet read/write loop (after PLA-004 G4), not wired",
                    )
                    .into(),
                ));
            }
            Action::ToggleSnapshots => {
                self.show_snapshots = !self.show_snapshots;
                if self.show_snapshots {
                    // R8b: 互斥 = 全部二级面板(快照墙/选择集/视口状态/尺寸驱动/设置)。
                    self.close_secondary_panels_except(PanelId::Snapshots);
                    // 每次开墙刷新: 快照可能被外部增删(含 Plasticity 侧)
                    self.snapshots = pla_ui::snapshot::list_snapshots(&self.doc_key);
                    self.snap_favs = pla_ui::snapshot::load_favs(&self.doc_key);
                    self.snap_infos = pla_ui::snapshot::load_meta_infos(&self.doc_key);
                }
            }
            Action::ToggleEntities => {
                self.show_entities = !self.show_entities;
                if self.show_entities {
                    self.close_secondary_panels_except(PanelId::Entities);
                    self.sel_saved_loaded = false;
                }
            }
            Action::ToggleVpState => {
                self.show_vpstate = !self.show_vpstate;
                if self.show_vpstate {
                    self.close_secondary_panels_except(PanelId::VpState);
                    self.vp_states_loaded = false;
                }
            }
            Action::ToggleDim => {
                self.show_dim = !self.show_dim;
                if self.show_dim {
                    self.close_secondary_panels_except(PanelId::Dim);
                }
            }
            Action::ToggleNotes => {
                // R30: 不调用 close_secondary_panels_except — 笔记与其他面板并存。
                self.show_notes = !self.show_notes;
                if self.show_notes {
                    self.notes.set_doc(&self.doc_key);
                } else {
                    self.notes.flush();
                    self.notes_open_rect = None;
                }
            }
            Action::ToggleSettings => {
                self.show_settings = !self.show_settings;
                if self.show_settings {
                    self.close_secondary_panels_except(PanelId::Settings);
                    // Q1: 每次打开都重读(用户可能在 Plasticity 里改过)
                    self.step_loaded = false;
                }
            }
        }
    }

    /// R30: 笔记浮窗。与二级面板不同: 不跟随工具栏、不互斥; 首次在默认位置(工具栏下方、二级面板右侧),
    /// 之后用户拖到哪/拉多大就记住(ui-prefs.json `notes_rect`), 关掉重开仍在原处。
    /// 拖动 = 标题行 OS StartDrag; 调大小 = 四边/四角 BeginResize(无边框窗口自己画热区)。
    fn show_notes_window(&mut self, ctx: &egui::Context) {
        const ID: &str = "panel-notes";
        let open_rect = *self.notes_open_rect.get_or_insert_with(|| {
            self.notes_saved_rect.unwrap_or_else(|| {
                let root = ctx.input(|i| i.viewport().outer_rect).unwrap_or(egui::Rect::from_min_size(
                    egui::pos2(200.0, 120.0),
                    egui::vec2(TOOLBAR_SIZE[0], TOOLBAR_SIZE[1]),
                ));
                [root.left() + PANEL_SNAPSHOTS[0] + 12.0, root.bottom() + 8.0, 320.0, 440.0]
            })
        });
        let (hide, reveal) = viewport_reveal(ctx, ID);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(ID)
            .with_decorations(false)
            .with_resizable(true)
            .with_transparent(true)
            .with_window_level(egui::WindowLevel::AlwaysOnTop)
            .with_position(egui::pos2(open_rect[0], open_rect[1]))
            .with_inner_size(egui::vec2(open_rect[2], open_rect[3]))
            .with_min_inner_size(egui::vec2(220.0, 160.0));
        if hide {
            builder = builder.with_visible(false);
        }
        let theme = pla_ui::notes_ui::Theme {
            text: COL_TEXT,
            muted: COL_MUTED,
            accent: COL_ACCENT,
            red: COL_RED,
            green: COL_GREEN,
            field: COL_FIELD,
            card: COL_CARD,
        };
        let doc = self.doc_key.clone();
        let mut open = true;
        let mut live_rect: Option<[f32; 4]> = None;
        let mut pointer_down = false;
        let mut start: Option<NotesDragMode> = None;
        let mut reload_notes = false;
        let notes = &mut self.notes;
        let notes_drag = &mut self.notes_drag;
        let notes_crash = &mut self.notes_crash;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(ID), builder, |ctx, _class| {
            if reveal {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                open = false;
            }
            egui::CentralPanel::default().frame(panel_frame()).show(ctx, |ui| {
                // ---- 标题行: 拖动柄 + 标题 + 文档名 + × ----
                let (row, drag) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click_and_drag());
                if drag.drag_started() {
                    start = Some(NotesDragMode::Move);
                }
                let p = ui.painter();
                for dy in [-4.0f32, 0.0, 4.0] {
                    for dx in [-2.5f32, 2.5] {
                        p.circle_filled(egui::pos2(row.left() + 7.0 + dx, row.center().y + dy), 1.2, COL_MUTED);
                    }
                }
                let tr_ = paint_text(p, egui::pos2(row.left() + 18.0, row.center().y), egui::Align2::LEFT_CENTER, tr("笔记", "Notes"), 13.0, egui::Color32::WHITE, true);
                p.text(
                    egui::pos2(tr_.right() + 8.0, row.center().y),
                    egui::Align2::LEFT_CENTER,
                    truncate_chars(&doc, 24),
                    egui::FontId::proportional(11.0),
                    COL_MUTED,
                );
                let xr = egui::Rect::from_center_size(egui::pos2(row.right() - 9.0, row.center().y), egui::vec2(18.0, 18.0));
                let xresp = ui.interact(xr, egui::Id::new("notes-close"), egui::Sense::click());
                ui.painter().text(xr.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(15.0), if xresp.hovered() { egui::Color32::WHITE } else { COL_MUTED });
                if xresp.on_hover_text(tr("关闭(内容已自动保存)", "Close (content is saved automatically)")).clicked() {
                    open = false;
                }
                // R32: 崩溃隔离 —— 笔记面板里的 bug 只让面板显示错误, 不再带走整个助手
                // (panic 冒到 winit 的窗口过程 = FFI 边界, 那里无法展开 → 进程直接终止, 见 pitfalls #50)。
                match notes_crash.as_ref() {
                    Some(msg) => {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(tr("笔记面板出错了(助手本身没受影响)", "The notes panel hit an error (the helper itself is fine)")).color(COL_RED));
                        ui.add(egui::Label::new(egui::RichText::new(msg).small().color(COL_MUTED)).wrap());
                        ui.label(egui::RichText::new(tr("详情已写入 %TEMP%\\pla-ui.log", "Details were written to %TEMP%\\pla-ui.log")).small().color(COL_MUTED));
                        if ui.button(tr("重新载入笔记", "Reload notes")).clicked() {
                            reload_notes = true;
                        }
                    }
                    None => {
                        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| notes.ui(ui, &theme)));
                        if let Err(p) = r {
                            let msg = p
                                .downcast_ref::<String>()
                                .cloned()
                                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                                .unwrap_or_else(|| "panic".into());
                            *notes_crash = Some(msg);
                        }
                    }
                }
            });
            paint_panel_border(ctx);
            // ---- 调大小热区(四边 + 四角) ----
            use egui::viewport::ResizeDirection as D;
            let sr = ctx.screen_rect();
            let e = 5.0;
            let c = 12.0;
            let zones = [
                (egui::Rect::from_min_max(egui::pos2(sr.right() - c, sr.bottom() - c), sr.max), D::SouthEast, egui::CursorIcon::ResizeSouthEast),
                (egui::Rect::from_min_max(egui::pos2(sr.left(), sr.bottom() - c), egui::pos2(sr.left() + c, sr.bottom())), D::SouthWest, egui::CursorIcon::ResizeSouthWest),
                (egui::Rect::from_min_max(egui::pos2(sr.right() - c, sr.top()), egui::pos2(sr.right(), sr.top() + c)), D::NorthEast, egui::CursorIcon::ResizeNorthEast),
                (egui::Rect::from_min_max(sr.min, egui::pos2(sr.left() + c, sr.top() + c)), D::NorthWest, egui::CursorIcon::ResizeNorthWest),
                (egui::Rect::from_min_max(egui::pos2(sr.right() - e, sr.top() + c), egui::pos2(sr.right(), sr.bottom() - c)), D::East, egui::CursorIcon::ResizeEast),
                (egui::Rect::from_min_max(egui::pos2(sr.left(), sr.top() + c), egui::pos2(sr.left() + e, sr.bottom() - c)), D::West, egui::CursorIcon::ResizeWest),
                (egui::Rect::from_min_max(egui::pos2(sr.left() + c, sr.bottom() - e), egui::pos2(sr.right() - c, sr.bottom())), D::South, egui::CursorIcon::ResizeSouth),
                (egui::Rect::from_min_max(egui::pos2(sr.left() + c, sr.top()), egui::pos2(sr.right() - c, sr.top() + e)), D::North, egui::CursorIcon::ResizeNorth),
            ];
            egui::Area::new(egui::Id::new("notes-resize"))
                .order(egui::Order::Foreground)
                .fixed_pos(sr.min)
                .show(ctx, |ui| {
                    for (i, (r, dir, icon)) in zones.into_iter().enumerate() {
                        let resp = ui.interact(r, egui::Id::new(("notes-rz", i)), egui::Sense::drag());
                        if resp.hovered() || resp.dragged() {
                            ctx.set_cursor_icon(icon);
                        }
                        if resp.drag_started() {
                            start = Some(NotesDragMode::Resize(dir));
                        }
                    }
                    // 右下角斜纹提示
                    let p = ui.painter();
                    for k in 1..=3 {
                        let o = k as f32 * 3.5;
                        p.line_segment(
                            [egui::pos2(sr.right() - 3.0 - o, sr.bottom() - 3.0), egui::pos2(sr.right() - 3.0, sr.bottom() - 3.0 - o)],
                            egui::Stroke::new(1.0, COL_MUTED),
                        );
                    }
                });
            let vp = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
            if let (Some(o), Some(inner)) = vp {
                live_rect = Some([o.left(), o.top(), inner.width(), inner.height()]);
            }
            // ---- R30b: 自管拖动/调大小 —— 开始与结束都以鼠标左键**物理**状态为准 ----
            // (旧版用 OS StartDrag/BeginResize: 模态循环吞掉松手事件 → egui 以为一直按着,
            //  鼠标移到别的窗口再回来就自己又开始拖/调大小, 窗口越拉越大。pitfalls #49)
            let phys_down = pla_ui::window_drag::primary_button_down();
            let ppp = ctx.pixels_per_point().max(0.01);
            if let (Some(mode), None, true, Some(r)) = (start, notes_drag.as_ref(), phys_down, live_rect) {
                if let (Some(c), Some(h)) = (
                    pla_ui::window_drag::cursor_pos_phys(),
                    pla_ui::window_drag::find_own_window(ID, r[2], ppp),
                ) {
                    pla_ui::window_drag::set_capture(h);
                    *notes_drag = Some(NotesDrag { mode, cursor: c, rect: r, last: r });
                }
            }
            if let Some(d) = notes_drag.as_mut() {
                if !phys_down {
                    pla_ui::window_drag::release_capture();
                    *notes_drag = None;
                } else if let Some(c) = pla_ui::window_drag::cursor_pos_phys() {
                    let next = d.apply((c.0 - d.cursor.0) / ppp, (c.1 - d.cursor.1) / ppp);
                    if (next[0] - d.last[0]).abs() > 0.4 || (next[1] - d.last[1]).abs() > 0.4 {
                        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(next[0], next[1])));
                    }
                    if (next[2] - d.last[2]).abs() > 0.4 || (next[3] - d.last[3]).abs() > 0.4 {
                        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(next[2], next[3])));
                    }
                    d.last = next;
                    ctx.request_repaint();
                }
            }
            pointer_down = phys_down || notes_drag.is_some();
        });
        // 位置/大小变了且已松手 → 写偏好(拖动/调整中不写, 免得每帧写盘)
        if let Some(r) = live_rect {
            let moved = self.notes_saved_rect.map_or(true, |s| s.iter().zip(r.iter()).any(|(a, b)| (a - b).abs() > 1.0));
            if moved && !pointer_down {
                if let Err(e) = pla_ui::prefs::save_notes_rect(r) {
                    pla_ui::diag::log(&format!("save notes rect failed: {e}"));
                }
                self.notes_saved_rect = Some(r);
            }
        }
        if self.notes_crash.is_some() && !self.notes_crashed_saved {
            // 出错前已输入的内容先落盘(数据结构本身是完整的, 只是某个操作逻辑出错)
            self.notes.flush();
            self.notes_crashed_saved = true;
        }
        if reload_notes {
            let doc = self.doc_key.clone();
            self.notes = pla_ui::notes_ui::NotesPanel::new();
            self.notes.set_doc(&doc);
            self.notes_crash = None;
            self.notes_crashed_saved = false;
        }
        if !open {
            self.show_notes = false;
            self.notes.flush();
            self.notes_open_rect = None;
        }
    }

    /// R8b: 二级面板统一互斥 — 打开一个, 关掉其余全部。
    fn close_secondary_panels_except(&mut self, keep: PanelId) {
        if keep != PanelId::Snapshots {
            self.show_snapshots = false;
        }
        if keep != PanelId::Entities {
            self.show_entities = false;
        }
        if keep != PanelId::VpState {
            self.show_vpstate = false;
        }
        if keep != PanelId::Dim {
            self.show_dim = false;
        }
        if keep != PanelId::Settings {
            self.show_settings = false;
        }
        if keep != PanelId::Caps {
            self.show_caps = false;
        }
    }

    /// F2: 开关 CDP 能力面板(与其他二级面板互斥); 打开时顺带重新探测一次。
    fn toggle_caps_panel(&mut self) {
        self.show_caps = !self.show_caps;
        if self.show_caps {
            self.close_secondary_panels_except(PanelId::Caps);
            self.cdp.send(pla_bridge::cdp::CdpCmd::ProbeCaps);
        }
    }

    fn set_notice(&mut self, n: (bool, String)) {
        self.notice = Some(n);
        self.notice_at = Instant::now();
    }

    /// 发送「应用」命令。
    /// (旧"重新武装"分支 R7-32 移除 — 该方案已被用户否决, v7 镜像模式不再需要。)
    fn send_apply(&mut self, entries: Vec<pla_bridge::cdp::ApplyEntry>) {
        self.cdp.send(pla_bridge::cdp::CdpCmd::ApplySel(entries));
    }

    /// R7-26: 把前台焦点交还 Plasticity 窗口。
    /// 辅助工具栏是独立无边框浮窗, 用户点完「应用」后焦点停在工具栏上,
    /// Plasticity 收不到键盘事件 → 主程序任何操作都不生效(用户实测)。
    /// 应用完成后主动把焦点还回去, 用户的下一步(shift+D / E / G)可直接按。
    fn focus_plasticity(&mut self) {
        let Some(target) = self.snap_target.clone() else { return };
        if window_finder::focus_window(target.hwnd) {
            self.hovered_hint = Some((
                tr("焦点已交还 Plasticity", "Focus returned to Plasticity").into(),
                t!("已聚焦: {}", "Focused: {}", target.title),
            ));
        }
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

    /// R7-17 / R26: doc_key 跟随绑定窗口标题("model1.plasticity - Plasticity" → "model1")。
    /// 同一窗口内换文件(标题变)也会触发; 吸附/拖动两条路径都调用。
    /// 返回 true = 本次发生了文档切换。
    fn sync_doc_key(&mut self) -> bool {
        let Some(title) = self.snap_target.as_ref().map(|t| t.title.clone()) else {
            return false;
        };
        let derived = Self::doc_key_from_title(&title);
        if derived == self.doc_key {
            return false;
        }
        pla_ui::diag::log(&format!("doc switch: {} -> {}", self.doc_key, derived));
        self.doc_key = derived;
        self.reset_doc_scoped();
        true
    }

    /// R26: 文档切换时作废**所有**按 doc_key 归档的状态 —— 唯一清单, 新增按文档存的字段必须加到这里。
    /// (以前只重置了一部分: 视口状态列表、选择集/视口截图缓存按名字缓存, 新文档同名 SG001 会显示旧图。)
    fn reset_doc_scoped(&mut self) {
        // 实体 / 当前选择镜像
        self.entities.clear();
        self.sel_cdp_members.clear();
        self.sel_save_pending = None;
        // 快照墙
        self.snapshots = pla_ui::snapshot::list_snapshots(&self.doc_key);
        self.snap_pngs.clear();
        self.snap_favs = pla_ui::snapshot::load_favs(&self.doc_key);
        self.snap_infos = pla_ui::snapshot::load_meta_infos(&self.doc_key);
        self.snap_selected.clear();
        self.snap_delete_confirm = None;
        self.rename_edit = None;
        self.last_auto_mtime = None;
        // 选择集
        self.sel_saved.clear();
        self.sel_saved_loaded = false;
        self.sel_pngs.clear();
        self.sel_selected.clear();
        self.sel_delete_confirm = None;
        self.sel_rename = None;
        // 视口状态
        self.vp_states.clear();
        self.vp_states_loaded = false;
        self.vp_pngs.clear();
        self.vp_selected.clear();
        self.vp_pending_state = None;
        self.vp_delete_confirm = None;
        self.vp_rename = None;
        self.snap_update_confirm = None;
        self.sel_update_confirm = None;
        self.vp_update_confirm = None;
        self.vp_update_target = None;
        // 尺寸驱动(关联选择集)
        self.dim_sets.clear();
        self.dim_set_sel = 0;
        self.dim_loaded = false;
    }

    fn update_snap(&mut self, ctx: &egui::Context) {
        if self.last_snap_poll.elapsed() < Duration::from_millis(SNAP_POLL_MS) {
            return;
        }
        self.last_snap_poll = Instant::now();
        // R8e: OS 拖动会话进行中 — 不做任何重定位(否则与 OS 移动打架=跳动)。
        if self.toolbar_dragging {
            return;
        }
        // R8-3: 用户拖动后停在拖动位置; 拖回默认锚点附近(SNAP_REENTER_X_PT/Y_PT)才恢复吸附。
        // R19: 认领式绑定 — 跟着 HWND 走, 换文档/改标题/最小化都不换绑。
        let root = self.own_root();
        let bound = window_finder::bind_target(self.snap_target.as_ref().map(|t| t.hwnd), root);
        if self.toolbar_drag_pos.is_some() {
            if let Some(target) = bound.clone().filter(|t| !t.minimized) {
                let ppp = ctx.pixels_per_point();
                let anchor = self.default_anchor(&target, ctx, ppp);
                if let (Some(dp), Some(da)) = (self.toolbar_drag_pos, anchor) {
                    if Self::is_in_toolbar_snap_zone(dp, da) {
                        // 拖回吸附区: 退出拖动模式, 走正常吸附流程
                        self.toolbar_drag_pos = None;
                        self.last_snap_anchor = None;
                    } else {
                        // 保持拖动位置: 只更新绑定/探测, 不发 OuterPosition
                        self.snap_target = Some(target.clone());
                        self.cdp.set_target(Some(target.hwnd));
                        // R26: 拖动模式也要跟随同窗口内的文档切换(以前这里提前 return, doc_key 不更新)。
                        if self.sync_doc_key() {
                            ctx.forget_all_images();
                        }
                        if self.last_probe_poll.elapsed()
                            >= Duration::from_millis(PROBE_POLL_MS)
                            || self
                                .probed_edge
                                .as_ref()
                                .map_or(true, |(h, _)| *h != target.hwnd)
                        {
                            let edge = pla_ui::titlebar_probe::probe_filename_right_edge(
                                &target, ppp,
                            );
                            self.probed_edge = edge.map(|e| (target.hwnd, e));
                            self.last_probe_poll = Instant::now();
                        }
                        return;
                    }
                }
            }
        }
        self.snap_target = bound;
        // R19: CDP 跟随绑定窗口(多窗口时每个助手只读写自己那扇窗口的 renderer)。
        self.cdp.set_target(self.snap_target.as_ref().map(|t| t.hwnd));
        if self.sync_doc_key() {
            ctx.forget_all_images(); // 释放旧文档的截图纹理(URI 已带内容指纹, 这里只为回收内存)
        }
        // 用 owned 克隆避免在 self 上叠加可变借用。
        let Some(target) = self.snap_target.clone() else {
            // 未绑定: 清缓存, 等 Plasticity 出现。
            self.last_snap_anchor = None;
            self.probed_edge = None;
            return;
        };
        // 最小化: 保持绑定, 不定位(矩形是 -32000); 置顶管理会把助手沉到底部。
        if target.minimized {
            self.last_snap_anchor = None;
            return;
        }
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
        let Some((x_phys, y_phys)) = self.default_anchor_phys(&target, ctx, ppp) else {
            return;
        };
        let desired_pt = egui::pos2(x_phys / ppp, y_phys / ppp);
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
        let rect_drifted = ctx
            .input(|i| i.viewport().outer_rect)
            .map_or(false, |r| {
                (r.min.x - desired_pt.x).abs() > SNAP_EPS
                    || (r.min.y - desired_pt.y).abs() > SNAP_EPS
            });
        if !anchor_changed && !rect_drifted {
            return;
        }
        self.last_snap_anchor = new_anchor;
        // 物理 → egui points: OuterPosition 用 points(见 egui 0.29 ViewportInfo)。
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(desired_pt));
    }

    /// Q1: 从 Plasticity 配置重读方向键绑定状态与步进角。
    fn reload_step_config(&mut self) {
        use pla_ui::plasticity_config as pc;
        self.step_msg = None;
        match pc::arrow_bindings_installed() {
            Ok(b) => self.step_bound = Some(b),
            Err(e) => {
                self.step_bound = None;
                self.step_msg = Some((true, e));
            }
        }
        match pc::read_step_deg() {
            Ok(Some(d)) => self.step_deg_buf = fmt_deg(d),
            Ok(None) => self.step_deg_buf.clear(), // 未设置 = Plasticity 内置默认
            Err(e) => self.step_msg = Some((true, e)),
        }
    }

    /// Q1: 输入框提交 → 写 settings.json(度 → 弧度)。
    fn apply_step_deg(&mut self) {
        let raw = self.step_deg_buf.trim().trim_end_matches('°').trim();
        if raw.is_empty() {
            return;
        }
        self.step_msg = Some(match raw.parse::<f64>() {
            Ok(d) => match pla_ui::plasticity_config::write_step_deg(d) {
                Ok(()) => {
                    self.step_deg_buf = fmt_deg(d);
                    // settings.json 不会被 Plasticity 重载 → 能力可用时实时改运行时值。
                    match self.feature_block(Feature::OrbitStepLive) {
                        None => {
                            let (name, args) = pla_ui::plasticity_config::live_step_call(d);
                            self.cdp.send(pla_bridge::cdp::CdpCmd::Call { name, args });
                            (false, t!("已设为每次 {}°(立即生效)", "Set to {}° per step (applied now)", fmt_deg(d)))
                        }
                        Some(why) => (
                            false,
                            t!(
                                "已设为每次 {}°(重启 Plasticity 后生效; 即时生效不可用: {why})",
                                "Set to {}° per step (takes effect after restarting Plasticity; live apply unavailable: {why})",
                                fmt_deg(d)
                            ),
                        ),
                    }
                }
                Err(e) => (true, e),
            },
            Err(_) => (true, t!("「{raw}」不是数字", "\"{raw}\" is not a number")),
        });
    }

    /// R19: Bridge 状态文本 — 连不上且 CDP 看到 Plasticity 的 Bridge 服务端已关闭时, 直接说清原因和恢复办法。
    fn bridge_status_text(&self) -> String {
        let closed = self
            .cdp_caps
            .as_ref()
            .and_then(|c| c.bridge_server.as_ref())
            .filter(|b| b.closed_while_enabled());
        match closed {
            Some(b) if !bridge_is_connected(&self.bridge_status) => t!(
                "这个窗口的 Bridge 服务已停止: 端口 {} 被占用 — 多窗口时只有一个窗口能占用它; \
                 单窗口时多半是上次没退干净, 到偏好设置 → Network 把 Server 关掉再打开即可",
                "This window's Bridge server stopped: port {} is in use — only one window can hold it; \
                 with a single window it's likely a stale instance: in Preferences → Network turn Server off and on again",
                b.port.unwrap_or(8980)
            ),
            _ => self.bridge_status.clone(),
        }
    }

    /// R23: HUD(悬停提示 / 通知)的左缘 = 工具栏与当前打开的二级面板中较宽者的右侧 + 间隙。
    /// 二级面板贴在工具栏下方、与工具栏左对齐, 比工具栏宽时 HUD 不能再按工具栏宽度放(会压在面板上)。
    fn hud_left(&self, toolbar_x: f32) -> f32 {
        let mut w = TOOLBAR_SIZE[0];
        for (open, size) in [
            (self.show_snapshots, PANEL_SNAPSHOTS),
            (self.show_entities, PANEL_ENTITIES),
            (self.show_vpstate, PANEL_VPSTATE),
            (self.show_dim, PANEL_DIM),
            (self.show_settings, PANEL_SETTINGS),
            (self.show_caps, PANEL_CAPS),
        ] {
            if open {
                w = w.max(size[0]);
            }
        }
        toolbar_x + w + 8.0
    }

    /// R19: 本实例工具栏根窗口(认领属性写在它上面); 找到后缓存。
    fn own_root(&mut self) -> Option<isize> {
        if self.root_hwnd.is_none() {
            self.root_hwnd = window_finder::own_root_window(ROOT_TITLE);
        }
        self.root_hwnd
    }

    /// R19: 置顶只在绑定窗口(或其弹窗、或助手自己)处于前台时生效; 100ms 节流。
    fn update_topmost(&mut self) {
        if self.toolbar_dragging || self.last_topmost_sync.elapsed() < Duration::from_millis(100) {
            return;
        }
        self.last_topmost_sync = Instant::now();
        let target = self.snap_target.as_ref();
        let active = target.map_or(false, |t| !t.minimized && window_finder::target_is_active(t.hwnd));
        window_finder::sync_topmost(target, active);
    }

    /// CDP 通道是否已连上 Plasticity。
    fn cdp_connected(&self) -> bool {
        self.cdp_conn.is_connected()
    }

    /// F2: 功能不可用的原因(None = 可用, 降级也算可用)。判定一律走 `features` 表。
    fn feature_block(&self, f: Feature) -> Option<String> {
        match pla_ui::features::availability(self.cdp_caps.as_ref(), f) {
            Availability::Blocked(why) => Some(why),
            _ => None,
        }
    }

    /// F2: 工具栏提示里的一行 CDP 能力摘要。
    fn cdp_summary(&self) -> String {
        let caps = self.cdp_caps.as_ref();
        match pla_ui::features::health(caps) {
            Health::Unknown => tr("CDP: 未连接", "CDP: not connected").into(),
            Health::NoEditor => tr("CDP: 已连接, 但未找到 Plasticity 编辑器", "CDP: connected, but no Plasticity editor found").into(),
            Health::Full => tr("CDP: 全部功能可用", "CDP: all features OK").into(),
            Health::Partial => {
                let (b, d) = pla_ui::features::summary(caps);
                match (b, d) {
                    (0, d) => t!("CDP: {d} 项功能部分可用", "CDP: {d} features partial"),
                    (b, 0) => t!("CDP: {b} 项功能不可用", "CDP: {b} features unavailable"),
                    (b, d) => t!("CDP: {b} 项不可用, {d} 项部分可用", "CDP: {b} unavailable, {d} partial"),
                }
            }
        }
    }

    /// F2: 动作入口的拦截: 不可用时如实提示并返回 false(绝不静默无反应)。
    fn require_feature(&mut self, f: Feature) -> bool {
        match self.feature_block(f) {
            None => true,
            Some(why) => {
                self.set_notice((true, t!("「{}」不可用: {why}", "\"{}\" unavailable: {why}", f.label())));
                false
            }
        }
    }

    fn is_in_toolbar_snap_zone(dp: egui::Pos2, da: egui::Pos2) -> bool {
        (dp.x - da.x).abs() <= SNAP_REENTER_X_PT && (dp.y - da.y).abs() <= SNAP_REENTER_Y_PT
    }

    /// R8-3: 默认吸附锚点(逻辑 pt) — 拖回判定用。
    fn default_anchor(
        &self,
        target: &window_finder::TargetWindow,
        ctx: &egui::Context,
        ppp: f32,
    ) -> Option<egui::Pos2> {
        let (x, y) = self.default_anchor_phys(target, ctx, ppp)?;
        Some(egui::pos2(x / ppp, y / ppp))
    }

    /// R8-3: 默认吸附锚点(物理 px)。探测命中 → 文件名右缘; 否则固定偏移。
    fn default_anchor_phys(
        &self,
        target: &window_finder::TargetWindow,
        ctx: &egui::Context,
        ppp: f32,
    ) -> Option<(f32, f32)> {
        Some(match &self.probed_edge {
            Some((h, hit)) if *h == target.hwnd => {
                let x = hit.right as f32 + (SNAP_FILENAME_GAP + SNAP_RIGHT_SHIFT) * ppp;
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
        })
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
    let (hide, reveal) = viewport_reveal(parent, id);
    let mut builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_decorations(false)
        .with_resizable(false)
        .with_transparent(true)
        .with_window_level(egui::WindowLevel::AlwaysOnTop)
        .with_active(active)
        .with_position(desired)
        .with_inner_size(size);
    if hide {
        builder = builder.with_visible(false);
    }
    parent.show_viewport_immediate(
        egui::ViewportId::from_hash_of(id),
        builder,
        // 非 move: flag 借用进闭包, ×/OS 关闭的结果能在调用后传回。
        |ctx, class| {
            if reveal {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                flag = false;
            }
            let Some(contents) = contents.take() else { return };
            match class {
                // 回退: 后端不支持多视口时嵌进当前视口。
                egui::ViewportClass::Embedded | egui::ViewportClass::Root | egui::ViewportClass::Deferred => {
                    egui::Window::new(title)
                        .id(egui::Id::new(id)) // 标题随语言变, 窗口 id 固定
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
                            panel_header(ui, title, id, &mut flag, icons.take());
                            contents(ui);
                        });
                    paint_panel_border(ctx);
                    // ---- R9: 面板拖动(自管 SetCapture + delta) + 跟随 ----
                    // 读一帧清一帧的拖动信号(标题行的 drag 层写入)。
                    let drag_signal = PANEL_DRAG_ACTIVE.with(|d| {
                        let v = *d.borrow();
                        *d.borrow_mut() = false;
                        v
                    });
                    // 同工具栏: delta 用屏幕绝对坐标(窗口内相对坐标会自我抵消)。
                    let cur = pla_ui::window_drag::cursor_pos_phys();
                    let down = ctx.input(|i| i.pointer.primary_down());
                    let mut st = PANEL_DRAG.with(|m| m.borrow().get(id).copied());
                    let dragging = st.map_or(false, |s| s.active);
                    if !dragging && drag_signal {
                        // 按标题+宽度定位本窗口 hwnd; 拿不到(非 Win/未创建)
                        // 或取不到光标才降级 OS StartDrag。
                        match (
                            cur,
                            ctx.input(|i| i.viewport().outer_rect),
                            pla_ui::window_drag::find_own_window(
                                title,
                                size.x,
                                ctx.pixels_per_point(),
                            ),
                        ) {
                            (Some(c), Some(r), Some(h)) => {
                                pla_ui::window_drag::set_capture(h);
                                st = Some(PanelDragState {
                                    active: true,
                                    origin_cursor: c,
                                    origin_win: r.min,
                                });
                            }
                            _ => {
                                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                                st = None;
                                // 降级: OS 拖完会把窗口挪走, 之后若干帧内把它补
                                // 记为"用户位置"(否则还会跟着工具栏跑)。
                                PANEL_PENDING_USER.with(|u| {
                                    u.borrow_mut().insert(id, 90);
                                });
                            }
                        }
                    }
                    if let Some(mut s) = st {
                        if s.active {
                            if !down {
                                // 松手: 会话收口; 若拖回默认跟随位附近则自动归位吸附
                                s.active = false;
                                pla_ui::window_drag::release_capture();
                                let snap_back = PANEL_USER_POS.with(|u| {
                                    u.borrow().get(id).copied().map_or(false, |up| {
                                        (up.x - desired.x).abs() <= 80.0
                                            && (up.y - desired.y).abs() <= SNAP_REENTER_Y_PT
                                    })
                                });
                                if snap_back {
                                    PANEL_USER_POS.with(|u| {
                                        u.borrow_mut().remove(id);
                                    });
                                    PANEL_LAST_CMD.with(|u| {
                                        u.borrow_mut().remove(id);
                                    });
                                    ctx.send_viewport_cmd(
                                        egui::ViewportCommand::OuterPosition(desired),
                                    );
                                }
                            } else if let Some(c) = cur {
                                let ppp = ctx.pixels_per_point().max(0.01);
                                let target = s.origin_win
                                    + egui::vec2((c.0 - s.origin_cursor.0) / ppp, (c.1 - s.origin_cursor.1) / ppp);
                                PANEL_USER_POS.with(|u| {
                                    u.borrow_mut().insert(id, target);
                                });
                                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(target));
                            }
                            PANEL_DRAG.with(|m| m.borrow_mut().insert(id, s));
                        }
                    }
                    // 跟随: **用户拖过 → 彻底不跟随**(R9-2)。
                    // 之前"拖过之后仍每帧按落点钉一次"与"默认位跟随"是两个写入者,
                    // 拖动一级面板时表现为每帧闪回默认位再刷回拖动位(用户实拍)。
                    // 规则改为: 有用户位置 = 完全冻结(只由用户拖动决定位置),
                    // 无用户位置 = 跟随工具栏下方默认位。双击标题手柄可归位。
                    let user_pos = PANEL_USER_POS.with(|u| u.borrow().get(id).copied());
                    if user_pos.is_some() {
                        // 冻结 — 本帧不发任何定位命令。
                    } else if let Some(r) = ctx.input(|i| i.viewport().outer_rect) {
                        // 降级路径补记: OS 拖动把窗口挪离默认位 → 记为用户位置。
                        let pending = PANEL_PENDING_USER.with(|u| u.borrow().get(id).copied());
                        if let Some(p) = pending {
                            if p == 0 {
                                PANEL_PENDING_USER.with(|u| u.borrow_mut().remove(id));
                            } else if !PRIMARY_DRAGGING.get()
                                && (r.left() - desired.x).abs().max((r.top() - desired.y).abs())
                                    > 8.0
                            {
                                PANEL_USER_POS.with(|u| {
                                    u.borrow_mut().insert(id, r.min);
                                });
                                PANEL_PENDING_USER.with(|u| u.borrow_mut().remove(id));
                            } else {
                                PANEL_PENDING_USER.with(|u| u.borrow_mut().insert(id, p - 1));
                            }
                        }
                        let off =
                            (r.left() - desired.x).abs().max((r.top() - desired.y).abs());
                        // 目标变了才发; 目标没变但偏得离谱(>8pt, 别人挪的)也纠正。
                        // 这样既跟得紧(0.5pt 阈值 ≈ 亚像素), 又不会因 DPI 取整
                        // 每帧重复 SetWindowPos(= 面板抖动/瞬移的根因)。
                        if off > 0.5 {
                            let changed = PANEL_LAST_CMD.with(|m| {
                                let mut m = m.borrow_mut();
                                match m.get(id) {
                                    Some(l)
                                        if (l.x - desired.x).abs() <= 0.25
                                            && (l.y - desired.y).abs() <= 0.25 =>
                                    {
                                        false
                                    }
                                    _ => {
                                        m.insert(id, desired);
                                        true
                                    }
                                }
                            });
                            if changed || off > 8.0 {
                                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(desired));
                            }
                        }
                    }
                }
            }
        },
    );
    *open = flag;
}

/// R30b: 笔记窗口拖动/调大小会话。
#[derive(Clone, Copy)]
enum NotesDragMode {
    Move,
    Resize(egui::viewport::ResizeDirection),
}

#[derive(Clone, Copy)]
struct NotesDrag {
    mode: NotesDragMode,
    /// 开始时的鼠标屏幕物理坐标。
    cursor: (f32, f32),
    /// 开始时的窗口 [x, y, 宽, 高](逻辑 pt)。
    rect: [f32; 4],
    /// 上次发出的目标(没变就不重发命令)。
    last: [f32; 4],
}

impl NotesDrag {
    const MIN_W: f32 = 220.0;
    const MIN_H: f32 = 160.0;

    /// 鼠标位移(逻辑 pt) → 新外框。调左/上边时右/下边保持不动(最小尺寸处截住)。
    fn apply(&self, dx: f32, dy: f32) -> [f32; 4] {
        use egui::viewport::ResizeDirection as D;
        let [x, y, w, h] = self.rect;
        match self.mode {
            NotesDragMode::Move => [x + dx, y + dy, w, h],
            NotesDragMode::Resize(dir) => {
                let (mut nx, mut ny, mut nw, mut nh) = (x, y, w, h);
                let east = matches!(dir, D::East | D::NorthEast | D::SouthEast);
                let west = matches!(dir, D::West | D::NorthWest | D::SouthWest);
                let south = matches!(dir, D::South | D::SouthEast | D::SouthWest);
                let north = matches!(dir, D::North | D::NorthEast | D::NorthWest);
                if east {
                    nw = (w + dx).max(Self::MIN_W);
                }
                if west {
                    nw = (w - dx).max(Self::MIN_W);
                    nx = x + w - nw;
                }
                if south {
                    nh = (h + dy).max(Self::MIN_H);
                }
                if north {
                    nh = (h - dy).max(Self::MIN_H);
                    ny = y + h - nh;
                }
                [nx, ny, nw, nh]
            }
        }
    }
}

#[cfg(test)]
mod crash_isolation_tests {
    /// R32: 帧中途在嵌套布局里 panic 并被 catch_unwind 拦下后, egui 上下文仍可继续跑后续帧。
    #[test]
    fn egui_survives_caught_panic_mid_frame() {
        let ctx = egui::Context::default();
        let mut caught = false;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    ui.horizontal(|ui| {
                        ui.label("before");
                        egui::ScrollArea::vertical().show(ui, |_ui| panic!("boom in notes"));
                    });
                }));
                caught = r.is_err();
                ui.label("panel still draws after the panic");
            });
        });
        assert!(caught);
        // 下一帧正常
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("next frame ok");
            });
        });
        assert!(!out.shapes.is_empty());
    }
}

#[cfg(test)]
mod notes_drag_tests {
    use super::*;
    use egui::viewport::ResizeDirection as D;

    #[test]
    fn move_and_resize_math() {
        let d = |mode| NotesDrag { mode, cursor: (0.0, 0.0), rect: [100.0, 100.0, 300.0, 400.0], last: [0.0; 4] };
        assert_eq!(d(NotesDragMode::Move).apply(10.0, -5.0), [110.0, 95.0, 300.0, 400.0]);
        assert_eq!(d(NotesDragMode::Resize(D::SouthEast)).apply(20.0, 30.0), [100.0, 100.0, 320.0, 430.0]);
        // 拖左边向右: 宽变小, 右边不动
        assert_eq!(d(NotesDragMode::Resize(D::West)).apply(50.0, 0.0), [150.0, 100.0, 250.0, 400.0]);
        // 缩到最小尺寸后截住, 右/下边仍不动
        assert_eq!(d(NotesDragMode::Resize(D::NorthWest)).apply(500.0, 500.0), [180.0, 340.0, 220.0, 160.0]);
    }
}

impl eframe::App for PlaApp {
    /// R13: 全透明清屏 — 圆角外的四角由窗口透明露出背后的 Plasticity。
    /// (eframe 默认是 alpha≈0.7 的深灰, 去掉 SetWindowRgn 后会在四角露出暗角。)
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    /// R14 诊断: 正常退出留痕(区分"用户关闭"与"无声崩溃/被外部结束")。
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.notes.flush(); // R30: 退出前把未落盘的笔记写掉
        pla_ui::diag::log("on_exit (正常关闭)");
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // F7: Plasticity 以 CDP 模式运行而代理没起时自动拉起(内部 3s 节流)。
        self.proxy.tick();
        self.poll_bridge();
        self.update_snap(ctx);
        self.update_topmost();
        // R9-2: 一级面板拖动中(二级面板别据此误判"用户挪了我")。
        PRIMARY_DRAGGING.set(self.toolbar_dragging);
        // R13: 圆角 = Win11 DWM 系统圆角(抗锯齿, 不依赖窗口透明), 取代 SetWindowRgn
        // (锯齿 + 激活切换时闪经典标题栏)。新面板在隐藏阶段即设置(viewport_reveal),
        // 这里低频轮询兜底(其他新建窗口, 如根视口)。拖动会话期间不枚举窗口。
        if !self.toolbar_dragging
            && self.last_round_poll.elapsed() >= Duration::from_millis(ROUND_POLL_MS)
        {
            self.last_round_poll = Instant::now();
            apply_round_corners_now();
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
                // R8e: 拖动层 = 整行矩形(Sense::click_and_drag; 面板背景 response 只有
                // click 没有 drag, 之前 dragged_by 永false = 拖不动的根因)。
                // 先注册 → 按钮后画在上层 → 按钮点击不受影响, 空白处拖动, 双击归位。
                let tb_drag = ui.interact(
                    row_rect,
                    egui::Id::new("tb-drag"),
                    egui::Sense::click_and_drag(),
                );
                let mut reset_to_snap = tb_drag.double_clicked();
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(row_rect), |ui| {
                ui.horizontal(|ui| {
                    // 状态圆点区: 未绑定=灰; 已绑定显示 bridge 状态色; 双击亦可归位。
                    let (dot_rect, dot_resp) =
                        ui.allocate_exact_size(egui::vec2(12.0, BTN_SIZE), egui::Sense::click());
                    if dot_resp.double_clicked() {
                        reset_to_snap = true;
                    }
                    // R18.1: 内点 = CDP(主通道)连接状态; bridge(只读冗余)只进悬停提示与能力面板。
                    let dot_color = if self.snap_target.is_none() {
                        COL_GRAY_DOT
                    } else {
                        cdp_conn_color(&self.cdp_conn)
                    };
                    ui.painter().circle_filled(dot_rect.center(), 4.0, dot_color);
                    // F2: 外环只在能力受损时出现(黄 部分可用 / 红 未找到编辑器); 全部可用或未连接不画。
                    let ring = match pla_ui::features::health(self.cdp_caps.as_ref()) {
                        Health::Unknown | Health::Full => None,
                        Health::NoEditor => Some(COL_RED),
                        Health::Partial => Some(COL_YELLOW),
                    };
                    if let Some(c) = ring {
                        ui.painter().circle_stroke(dot_rect.center(), 5.4, egui::Stroke::new(1.2, c));
                    }
                    // F2: 单击 = 开关 CDP 能力面板; 双击 = 归位(见上)。
                    if dot_resp.clicked() {
                        self.toggle_caps_panel();
                    }
                    if dot_resp.hovered() {
                        // 一行一项, 长标题截断 — HUD 按内容高度自适应(见 hud_size)。
                        let win = match &self.snap_target {
                            Some(t) => t!("窗口: {}", "Window: {}", truncate_chars(&t.title, 20)),
                            None => tr("窗口: 未绑定, 等待 Plasticity…", "Window: unbound, waiting for Plasticity…").into(),
                        };
                        hovered = Some((
                            tr("PLA 助手", "PLA Helper").into(),
                            t!(
                                "{win}\n{}\nBridge(只读): {}\n单击: CDP 能力面板 · 双击: 归位",
                                "{win}\n{}\nBridge (read-only): {}\nClick: CDP capabilities · Double-click: re-dock",
                                self.cdp_summary(),
                                self.bridge_status_text()
                            ),
                        ));
                    }
                    ui.separator();

                    for a in TOOLBAR_ACTIONS {
                        let active = match a {
                            Action::ToggleSnapshots => self.show_snapshots,
                            Action::ToggleEntities => self.show_entities,
                            Action::ToggleVpState => self.show_vpstate,
                            Action::ToggleDim => self.show_dim,
                            Action::ToggleSettings => self.show_settings,
                            Action::ToggleNotes => self.show_notes,
                            _ => false,
                        };
                        let hk = self
                            .hotkeys
                            .binds
                            .get(&format!("{a:?}"))
                            .map(|h| h.display())
                            .unwrap_or_else(|| tr("未绑定", "Unbound").into());
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
                                t!("快捷键: {hk} / 右键设置", "Hotkey: {hk} / right-click to set"),
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
                        hovered = Some((tr("关闭", "Close").into(), tr("退出 PLA 助手工具栏", "Quit the PLA Helper toolbar").into()));
                    }
                    if close_resp.clicked() {
                        pla_ui::diag::log("工具栏 × 被点击");
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                }); // allocate_new_ui(居中行矩形)
                // R9: 自管拖动(SetCapture + 屏幕坐标 delta) — 取代 OS 模态循环。
                // 帧循环照常运行 → 二级面板在同一帧拿到同一个权威坐标, 与工具栏
                // 同步平移(不再"疯狂瞬移"); 鼠标移出小窗口也不断流。
                // delta 一律用屏幕绝对坐标(GetCursorPos): egui 的 pointer 是窗口
                // 内相对坐标, 窗口一移动它就变, 用它算 delta 会自我抵消。
                let cur_now = pla_ui::window_drag::cursor_pos_phys();
                let down_now = ctx.input(|i| i.pointer.primary_down());
                if !self.toolbar_dragging
                    && tb_drag.dragged_by(egui::PointerButton::Primary)
                {
                    self.toolbar_dragging = match (
                        cur_now,
                        ctx.input(|i| i.viewport().outer_rect),
                        pla_ui::window_drag::find_own_window(
                            "PLA Toolbar",
                            TOOLBAR_SIZE[0],
                            ctx.pixels_per_point(),
                        ),
                    ) {
                        (Some(c), Some(r), Some(h)) => {
                            pla_ui::window_drag::set_capture(h);
                            self.drag_origin_cursor = Some(c);
                            self.drag_origin_win = Some(r.min);
                            true
                        }
                        // 拿不到 hwnd / 取不到光标(非 Win 或窗口未就绪): 降级 OS 拖动
                        _ => false,
                    };
                    if !self.toolbar_dragging {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                }
                if self.toolbar_dragging {
                    if !down_now {
                        self.toolbar_dragging = false;
                        pla_ui::window_drag::release_capture();
                        self.drag_origin_cursor = None;
                        self.drag_origin_win = None;
                        if let (Some(dp), Some(target)) = (
                            self.toolbar_drag_pos,
                            self.snap_target
                                .clone()
                                .filter(|t| !t.minimized),
                        ) {
                            let ppp = ctx.pixels_per_point();
                            if let Some(da) = self.default_anchor(&target, ctx, ppp) {
                                if Self::is_in_toolbar_snap_zone(dp, da) {
                                    reset_to_snap = true;
                                }
                            }
                        }
                    } else if let (Some(oc), Some(ow), Some(c)) =
                        (self.drag_origin_cursor, self.drag_origin_win, cur_now)
                    {
                        let ppp = ctx.pixels_per_point().max(0.01);
                        let target = ow
                            + egui::vec2((c.0 - oc.0) / ppp, (c.1 - oc.1) / ppp);
                        self.toolbar_drag_pos = Some(target);
                        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(target));
                    }
                }
                if reset_to_snap {
                    self.toolbar_drag_pos = None;
                    self.last_snap_anchor = None;
                    if let Some(target) = self
                        .snap_target
                        .clone()
                        .filter(|t| !t.minimized)
                    {
                        let ppp = ctx.pixels_per_point();
                        if let Some(da) = self.default_anchor(&target, ctx, ppp) {
                            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(da));
                        }
                    }
                }
            });
        paint_panel_border(ctx);
        self.hovered_hint = hovered;

        // R8b: 面板用户位置清理 — 关闭的面板移除记录(重开回默认跟随位)。
        {
            let mut open: std::collections::HashSet<&'static str> =
                std::collections::HashSet::new();
            if self.show_snapshots {
                open.insert("panel-snapshots");
            }
            if self.show_entities {
                open.insert("panel-entities");
            }
            if self.show_vpstate {
                open.insert("panel-vpstate");
            }
            if self.show_dim {
                open.insert("panel-dim");
            }
            if self.show_settings {
                open.insert("panel-settings");
            }
            if self.show_caps {
                open.insert("panel-caps");
            }
            PANEL_USER_POS.with(|u| {
                u.borrow_mut().retain(|k, _| open.contains(k));
            });
            PANEL_LAST_CMD.with(|u| {
                u.borrow_mut().retain(|k, _| open.contains(k));
            });
            PANEL_DRAG.with(|u| {
                u.borrow_mut().retain(|k, _| open.contains(k));
            });
            PANEL_PENDING_USER.with(|u| {
                u.borrow_mut().retain(|k, _| open.contains(k));
            });
        }

        // R8b: 拖动落位后每帧钉住位置(吸附轮询已短路, 防 egui/OS 抖动回弹)。
        if let Some(dp) = self.toolbar_drag_pos {
            if !self.toolbar_dragging {
                if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
                    if (rect.min.x - dp.x).abs() > SNAP_EPS
                        || (rect.min.y - dp.y).abs() > SNAP_EPS
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(dp));
                    }
                }
            }
        }

        // ---- 工具栏外框基准位置(功能面板锚正下方, HUD 锚右侧) ----
        // R9: 面板锚点必须取「权威坐标」而不是每帧回读的实际外框 — 回读值比
        // 拖动目标滞后一帧, 面板按滞后值追 → 拖动时二级面板乱跳。拖动中/拖过
        // 之后用自算的 toolbar_drag_pos(与二级面板同帧同源 → 同步平移)。
        let toolbar_rect = ctx
            .input(|i| i.viewport().outer_rect);
        let tb_h = toolbar_rect
            .map(|r| r.height())
            .unwrap_or(TOOLBAR_SIZE[1] + 4.0);
        let tb_origin = self
            .toolbar_drag_pos
            .or_else(|| toolbar_rect.map(|r| r.min));
        // R13: 工具栏与面板外框内缩相同(PANEL_BORDER_INSET), 左缘不再额外 +2pt
        // (旧值让面板比工具栏右偏约 3 物理像素, 用户截图实锤)。
        let toolbar_bottom_left = tb_origin.map(|p| egui::pos2(p.x, p.y + tb_h + 6.0));

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
                    let vbtn = |ui: &mut egui::Ui,
                                kind: IconKind,
                                tip: &str,
                                hit: &std::cell::Cell<bool>,
                                ts: &std::cell::RefCell<Option<(String, String)>>| {
                        let resp = vector_icon_button(ui, kind, tip, ts);
                        if resp.clicked() {
                            hit.set(true);
                        }
                    };
                    // right_to_left: 视觉顺序(右→左) = 🗑 📂 📷 ×
                    vbtn(ui, IconKind::Trash, tr("管理 — 多选删除", "Manage — multi-delete"), &dm, &ts);
                    vbtn(ui, IconKind::Folder, tr("打开快照文件夹", "Open snapshot folder"), &dod, &ts);
                    vbtn(
                        ui,
                        IconKind::Camera,
                        tr("手动快照 — 保存文档副本 + 截图当前画面", "Snapshot now — save a doc copy + screenshot"),
                        &ds,
                        &ts,
                    );
                });
                header_extra_and_show(
                    ctx,
                    "panel-snapshots",
                    tr("快照墙", "Snapshot wall"),
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
                            let all = self.snapshots.clone();
                            let (del, done) = manage_bar(
                                ui,
                                &all,
                                &mut self.snap_selected,
                                tr("删除勾选的快照(含截图), 不可恢复", "Delete checked snapshots (with images), cannot be undone"),
                            );
                            if del {
                                let names: Vec<String> = self.snap_selected.iter().cloned().collect();
                                match pla_ui::snapshot::delete_snapshots(&self.doc_key, &names) {
                                    Ok(n) => {
                                        self.snapshots = pla_ui::snapshot::list_snapshots(&self.doc_key);
                                        self.snap_infos = pla_ui::snapshot::load_meta_infos(&self.doc_key);
                                        for n in &names {
                                            self.snap_pngs.remove(n);
                                            self.snap_selected.remove(n);
                                            self.snap_favs.remove(n);
                                        }
                                        self.set_notice((false, t!("已删除 {n} 个快照", "Deleted {n} snapshots")));
                                    }
                                    Err(e) => self.set_notice((true, e)),
                                }
                            }
                            if done {
                                self.snap_manage = false;
                                self.snap_selected.clear();
                            }
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
                        let mut delete_ask: Option<String> = None; let mut update_ask: Option<String> = None;

                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            // R7-10: 宽度必须在 ScrollArea 内部测 — 竖向滚动条出现后
                            // 内部可用宽比外部少一个滚动条宽, 在外面测会让每行总宽
                            // 超出可视区, 第 3 列被裁掉(用户实测"宽度还超了")。
                            let avail_w = ui.available_width();
                            let sp = ui.spacing().item_spacing.x;
                            let fav_cell_w = ((avail_w - sp * 2.0) / 3.0).max(40.0);
                            let fav_img_h = fav_img_h_for(fav_cell_w);
                            let fav_row_h = fav_img_h + 26.0; // 图 + 名称 + 紧凑按钮行(R8e)
                            let big_cell_w = avail_w;
                            // 悬浮提示文本: SS 短名 + meta 里的源文档/拍摄时间
                            let tip_of = |s: &str| -> Option<(String, String)> {
                                let stem = s.trim_end_matches(".plasticity");
                                match self.snap_infos.get(s) {
                                    Some((doc, created)) => Some((
                                        format!("{stem} — {doc}"),
                                        t!("拍摄于 {created} · 点击在新窗口打开", "Taken {created} · click to open in new window"),
                                    )),
                                    None => Some((
                                        stem.to_string(),
                                        tr("点击在新窗口打开", "Click to open in new window").to_string(),
                                    )),
                                }
                            };
                            if self.snapshots.is_empty() {
                                ui.label(
                                    egui::RichText::new(tr(
                                        "(暂无快照 — 点右上相机图标拍摄)",
                                        "(No snapshots — click the camera icon at top right)",
                                    ))
                                    .color(COL_MUTED),
                                );
                            }
                            if !fav_list.is_empty() {
                                ui.label(
                                    egui::RichText::new(tr("★ 收藏", "★ Favorites")).small().strong().color(COL_MUTED),
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
                                        if act.update {
                                            update_ask = Some(s.clone());
                                        }
                                        if act.delete {
                                            delete_ask = Some(s.clone());
                                        }
                                    }
                                }
                                ui.separator();
                                ui.label(
                                    egui::RichText::new(tr("全部", "All")).small().strong().color(COL_MUTED),
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
                                if act.update {
                                    update_ask = Some(s.clone());
                                }
                                if act.delete {
                                    delete_ask = Some(s.clone());
                                }
                            }
                        });
                        // 悬浮缩略图时的元数据 → HUD(工具栏右侧文本框)
                        if let Some(t) = tip_slot.into_inner() {
                            self.hovered_hint = Some(t);
                        }
                        // R8b: 单删确认 — 弹层(行内条会被滚动区推出可视区, 实测不可见)
                        if let Some(name) = self.snap_delete_confirm.clone() {
                            let text = t!(
                                "删除「{}」? 含截图, 不可恢复",
                                "Delete \"{}\"? Includes image, cannot be undone",
                                name.trim_end_matches(".plasticity")
                            );
                            match confirm_popup(ui.ctx(), "snap-del", &text) {
                                Some(true) => {
                                    match pla_ui::snapshot::delete_snapshots(
                                        &self.doc_key,
                                        &[name.clone()],
                                    ) {
                                        Ok(_) => {
                                            self.snapshots = pla_ui::snapshot::list_snapshots(
                                                &self.doc_key,
                                            );
                                            self.snap_infos = pla_ui::snapshot::load_meta_infos(
                                                &self.doc_key,
                                            );
                                            self.snap_pngs.remove(&name);
                                            self.snap_favs.remove(&name);
                                            self.snap_selected.remove(&name);
                                            self.set_notice((false, t!("已删除「{name}」", "Deleted \"{name}\"")));
                                        }
                                        Err(e) => self.set_notice((true, e)),
                                    }
                                    self.snap_delete_confirm = None;
                                }
                                Some(false) => self.snap_delete_confirm = None,
                                None => {}
                            }
                        }
                        if let Some(n) = delete_ask {
                            self.snap_delete_confirm = Some(n);
                        }
                        // R35: 更新确认 — 用当前(已保存的)文档覆盖这个快照
                        if let Some(name) = self.snap_update_confirm.clone() {
                            let stem = name.trim_end_matches(".plasticity").to_string();
                            let text = t!(
                                "用当前文档覆盖「{stem}」? 原内容不可恢复(取的是最近一次 Ctrl+S 保存的文件)",
                                "Overwrite \"{stem}\" with the current document? The old content cannot be recovered (uses the last Ctrl+S save)"
                            );
                            match confirm_popup(ui.ctx(), "snap-upd", &text) {
                                Some(true) => {
                                    let hwnd = self.snap_target.as_ref().map(|t| t.hwnd);
                                    match pla_ui::snapshot::update_snapshot(&self.doc_key, &name, hwnd) {
                                        Ok(s) => {
                                            self.snapshots = pla_ui::snapshot::list_snapshots(&self.doc_key);
                                            self.snap_infos = pla_ui::snapshot::load_meta_infos(&self.doc_key);
                                            self.snap_pngs.remove(&name);
                                            self.set_notice((false, t!("已更新「{s}」", "Updated \"{s}\"")));
                                        }
                                        Err(e) => self.set_notice((true, e)),
                                    }
                                    self.snap_update_confirm = None;
                                }
                                Some(false) => self.snap_update_confirm = None,
                                None => {}
                            }
                        }
                        if let Some(n) = update_ask {
                            self.snap_update_confirm = Some(n);
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
                                        self.set_notice((false, t!("已改名: {new_name}", "Renamed: {new_name}")));
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
                // R8-1: 头部三图标按钮: 保存(SG 自动命名)/打开文件夹/批量删除
                let do_save = std::rc::Rc::new(std::cell::Cell::new(false));
                let do_open_dir = std::rc::Rc::new(std::cell::Cell::new(false));
                let do_manage = std::rc::Rc::new(std::cell::Cell::new(false));
                let tip_slot = std::rc::Rc::new(std::cell::RefCell::new(None::<(String, String)>));
                let (dsv, dodv, dmv, tsv) = (do_save.clone(), do_open_dir.clone(), do_manage.clone(), tip_slot.clone());
                let icons: Box<dyn FnOnce(&mut egui::Ui)> = Box::new(move |ui| {
                    let vbtn = |ui: &mut egui::Ui, kind: IconKind, tip: &str, hit: &std::cell::Cell<bool>, ts: &std::cell::RefCell<Option<(String, String)>>| {
                        let resp = vector_icon_button(ui, kind, tip, ts);
                        if resp.clicked() {
                            hit.set(true);
                        }
                    };
                    vbtn(ui, IconKind::Trash, tr("批量删除", "Batch delete"), &dmv, &tsv);
                    vbtn(ui, IconKind::Folder, tr("打开选择集文件夹", "Open selection set folder"), &dodv, &tsv);
                    vbtn(
                        ui,
                        IconKind::Save,
                        tr("保存当前选择(SG 自动命名 + 截图)", "Save current selection (auto SG name + image)"),
                        &dsv,
                        &tsv,
                    );
                });
                header_extra_and_show(
                    ctx,
                    "panel-entities",
                    tr("选择集", "Selection sets"),
                    desired,
                    egui::vec2(PANEL_ENTITIES[0], PANEL_ENTITIES[1]),
                    &mut flag,
                    true,
                    |ui| {
                        let mut open_err: Option<String> = None;
                        let inner_tip = std::cell::RefCell::new(None::<(String, String)>);

                        // ============ ENTITIES: 右侧单独一列(R23 用户要求, 不再挤占选择集墙) ============
                        egui::SidePanel::right("sel-entities-col")
                            .resizable(false)
                            .exact_width(SEL_ENTITIES_COL_W)
                            .frame(egui::Frame::none().inner_margin(egui::Margin { left: 8.0, right: 0.0, top: 0.0, bottom: 0.0 }))
                            .show_inside(ui, |ui| {
                        section_header(ui, "ENTITIES");
                        ui.label(
                            egui::RichText::new(t!(
                                "实时选择({} 项): {}",
                                "Live selection ({} items): {}",
                                self.sel_cdp_members.len(),
                                self.cdp_conn.text()
                            ))
                                .small()
                                .color(COL_MUTED),
                        );
                        if !self.live_sel.is_empty() {
                            egui::ScrollArea::vertical()
                                .id_salt("live-sel")
                                // 单独一列: 占满列高, 底部给报错信息留位置。
                                .max_height((ui.available_height() - if self.cdp_err_msg.is_some() { 80.0 } else { 0.0 }).max(60.0))
                                .show(ui, |ui| {
                                    // R24: 按所属物体分组; 组头 = 物体名 + 各类数量, 行 = 曲面类型 + 短编号(面/边没有名字)。
                                    for (group, items) in pla_ui::sel_labels::group_live_selection(&self.live_sel) {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(format!("{}  · {}", group, pla_ui::sel_labels::kind_counts(&items)))
                                                    .small()
                                                    .strong()
                                                    .color(COL_MUTED),
                                            )
                                            .truncate(),
                                        );
                                        for it in items {
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(format!("  ● {}", pla_ui::sel_labels::row_label(it)))
                                                        .color(if it.kind == "face" { COL_YELLOW } else { COL_TEXT }),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(format!("{} {}", it.kind, it.id));
                                        }
                                        ui.add_space(4.0);
                                    }
                                });
                        } else {
                            ui.label(
                                egui::RichText::new(tr(
                                    "(空 — 在 Plasticity 视口/树里选择实体)",
                                    "(Empty — select entities in the Plasticity viewport/tree)",
                                ))
                                .small()
                                .color(COL_MUTED),
                            );
                        }
                        // R8-1(需求7): 报错信息溢出修复 — 包 ScrollArea + 换行
                        if let Some(err) = self.cdp_err_msg.clone() {
                            ui.separator();
                            egui::ScrollArea::vertical()
                                .id_salt("sel-err")
                                .max_height(60.0)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(format!("⚠ {err}"))
                                                .small()
                                                .color(COL_RED),
                                        )
                                        .wrap(),
                                    );
                                });
                        }

                            });
                        // ============ SAVED SELECTIONS(置顶, 参照快照墙) ============
                        section_header(ui, "SAVED SELECTIONS");
                        if !self.sel_saved_loaded {
                            self.sel_saved = pla_ui::selection::list_cdp_selections_meta(&self.doc_key);
                            self.sel_saved_loaded = true;
                        }
                        // R28: 管理模式操作条放列表顶部(与快照墙一致)
                        if self.sel_manage {
                            let all: Vec<String> = self.sel_saved.iter().map(|m| m.name.clone()).collect();
                            let (del, done) = manage_bar(
                                ui,
                                &all,
                                &mut self.sel_selected,
                                tr("删除勾选的选择集(含截图), 不可恢复", "Delete checked selection sets (with images), cannot be undone"),
                            );
                            if del {
                                let names: Vec<String> = self.sel_selected.iter().cloned().collect();
                                match pla_ui::selection::delete_cdp_selections_batch(&self.doc_key, &names) {
                                    Ok(n) => {
                                        self.sel_saved_loaded = false;
                                        self.sel_saved.retain(|m| !names.contains(&m.name));
                                        for nm in &names { self.sel_pngs.remove(nm); }
                                        self.sel_selected.clear();
                                        self.set_notice((false, t!("已删除 {n} 个选择集", "Deleted {n} selection sets")));
                                    }
                                    Err(e) => self.set_notice((true, e)),
                                }
                            }
                            if done {
                                self.sel_manage = false;
                                self.sel_selected.clear();
                            }
                        }
                        // 截图缓存填充
                        let sel_dir = pla_ui::selection::cdp_selections_dir(&self.doc_key);
                        for meta in self.sel_saved.clone() {
                            if !self.sel_pngs.contains_key(&meta.name) {
                                if let Ok(b) = std::fs::read(sel_dir.join(format!("{}.png", meta.name))) {
                                    self.sel_pngs.insert(meta.name.clone(), std::sync::Arc::from(b));
                                }
                            }
                        }
                        // 收藏区(一行3个) + 全部区
                        let (fav_list, other_list): (Vec<_>, Vec<_>) =
                            self.sel_saved.iter().cloned().partition(|m| m.fav);
                        if self.sel_saved.is_empty() {
                            ui.label(
                                egui::RichText::new(tr(
                                    "(空 — 视口选中后点右上软盘图标保存)",
                                    "(Empty — select in viewport, then click the disk icon at top right)",
                                ))
                                .small()
                                .color(COL_MUTED),
                            );
                        }
                        let mut delete_ask: Option<String> = None; let mut update_ask: Option<String> = None;
                        let mut apply_now: Option<String> = None;
                        let mut fav_toggle: Option<(String, bool)> = None;
                        let mut rename_start: Option<String> = None;
                        let mut rename_commit = false;
                        let mut rename_cancel = false;
                        // 动态高度: 内容少 = 内容高; 多 = 屏高 1/3 上限 + 滚动。
                        // 实现手段: ScrollArea max_height(预估值) — egui 无法先量后限,
                        // 用"行数×行高"估算内容高, 上限 = 屏高/3。
                        // R23: 存档墙占满剩余高度(以前封顶屏高 1/3 → 收藏区挤得放不下一个); 管理模式给下方操作条留高度。
                        let content_h = ui.available_height().max(72.0); // R28: 操作条已移到列表上方
                        egui::ScrollArea::vertical()
                            .id_salt("sel-saved-wall")
                            .max_height(content_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let avail_w = ui.available_width();
                                let sp = ui.spacing().item_spacing.x;
                                let cell_w = ((avail_w - sp * 2.0) / 3.0).max(40.0);
                                // 单个已存选择项(小卡: 截图 + 名称行[★/改/删])
                                let mut item = |ui: &mut egui::Ui, meta: &pla_ui::selection::CdpSelMeta, w: f32, img_h: f32| {
                                    let mut del = false;
                                    let mut fav = false;
                                    let mut rn = false;
                                    let mut upd = false;
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(w, img_h + 38.0),
                                        egui::Layout::top_down(egui::Align::LEFT),
                                        |ui| {
                                            // 截图(点击 = 应用; 悬浮 = 信息入 tip 槽)
                                            let png = self.sel_pngs.get(&meta.name);
                                            let dims = png.and_then(|b| png_dims(b));
                                            let img_resp = match (png, dims) {
                                                (Some(bytes), Some((pw, ph))) => {
                                                    let iw = (img_h * pw / ph).min(w);
                                                    ui.allocate_ui_with_layout(
                                                        egui::vec2(w, img_h),
                                                        egui::Layout::top_down(egui::Align::Center),
                                                        |ui| ui.add(
                                                            egui::Image::from_bytes(
                                                                png_uri("sel", &meta.name, &bytes),
                                                                bytes.clone(),
                                                            )
                                                            .fit_to_exact_size(egui::vec2(iw, img_h))
                                                            .sense(egui::Sense::click()),
                                                        ),
                                                    ).inner
                                                }
                                                _ => ui.add_sized(
                                                    egui::vec2(w, img_h),
                                                    egui::Label::new(egui::RichText::new(tr("无截图", "No image")).small().color(COL_MUTED)).sense(egui::Sense::click()),
                                                ),
                                            };
                                            if img_resp.hovered() {
                                                *inner_tip.borrow_mut() = Some((
                                                    t!("{} — {} 项", "{} — {} items", meta.name, meta.n_members),
                                                    t!(
                                                        "保存于 {} · 点击应用回 Plasticity",
                                                        "Saved {} · click to apply in Plasticity",
                                                        if meta.created.is_empty() { "-" } else { &meta.created }
                                                    ),
                                                ));
                                            }
                                            if img_resp.clicked() {
                                                apply_now = Some(meta.name.clone());
                                            }
                                            // 名称行: [管理勾选] 名称 ★ ✎ 🗑
                                            ui.horizontal(|ui| {
                                                if self.sel_manage {
                                                    let mut on = self.sel_selected.contains(&meta.name);
                                                    if ui.checkbox(&mut on, "").changed() {
                                                        if on { self.sel_selected.insert(meta.name.clone()); } else { self.sel_selected.remove(&meta.name); }
                                                    }
                                                }
                                                if self.sel_rename.as_deref() == Some(meta.name.as_str()) {
                                                    ui.add_sized(egui::vec2((w - 60.0).max(30.0), 16.0), egui::TextEdit::singleline(&mut self.sel_rename_buf)).request_focus();
                                                    let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                                                    if enter { rename_commit = true; }
                                                    if esc { rename_cancel = true; }
                                                } else {
                                                    // R8d: 名称独占一行
                                                    let name_w = (w - if self.sel_manage { 16.0 + sp } else { 0.0 }).max(30.0);
                                                    let nr = ui.allocate_ui_with_layout(
                                                        egui::vec2(name_w, 14.0),
                                                        egui::Layout::left_to_right(egui::Align::Center),
                                                        |ui| ui.add(egui::Label::new(egui::RichText::new(&meta.name).small().color(COL_TEXT)).sense(egui::Sense::click()).truncate()),
                                                    ).inner;
                                                    if nr.clicked() {
                                                        apply_now = Some(meta.name.clone());
                                                    }
                                                }
                                            });
                                            // R8d: 第二行按钮组(★ ✎ 🗑)
                                            ui.horizontal(|ui| {
                                                let (srect, sresp) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
                                                ui.painter().text(srect.center(), egui::Align2::CENTER_CENTER, if meta.fav { "★" } else { "☆" }, egui::FontId::proportional(13.0), if meta.fav { egui::Color32::from_rgb(255,200,60) } else { COL_MUTED });
                                                if sresp.on_hover_text(if meta.fav { tr("取消收藏", "Unfavorite") } else { tr("收藏(置顶)", "Favorite (pin to top)") }).clicked() { fav = true; }
                                                if pencil_button(ui).on_hover_text(tr("改名", "Rename")).clicked() { rn = true; }
                                                let (drect, dresp) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
                                                let dcol = if dresp.hovered() { COL_RED } else { COL_MUTED };
                                                let p = |x: f32, y: f32| egui::pos2(drect.left() + x, drect.top() + y);
                                                ui.painter().line_segment([p(3.0,4.0), p(11.0,4.0)], egui::Stroke::new(1.2, dcol));
                                                ui.painter().line_segment([p(5.5,4.0), p(5.5,2.5)], egui::Stroke::new(1.2, dcol));
                                                ui.painter().line_segment([p(8.5,4.0), p(8.5,2.5)], egui::Stroke::new(1.2, dcol));
                                                ui.painter().line_segment([p(6.2,2.5), p(7.8,2.5)], egui::Stroke::new(1.2, dcol));
                                                ui.painter().line_segment([p(4.0,4.0), p(5.0,12.0)], egui::Stroke::new(1.2, dcol));
                                                ui.painter().line_segment([p(10.0,4.0), p(9.0,12.0)], egui::Stroke::new(1.2, dcol));
                                                ui.painter().line_segment([p(5.0,12.0), p(9.0,12.0)], egui::Stroke::new(1.2, dcol));
                                                if dresp.on_hover_text(tr("删除(需确认)", "Delete (confirm)")).clicked() { del = true; }
                                            });
                                        },
                                    );
                                    (del, fav, rn, upd)
                                };
                                // 收藏行(3列, 定位同快照墙)
                                if !fav_list.is_empty() {
                                    ui.label(egui::RichText::new(tr("★ 收藏", "★ Favorites")).small().strong().color(COL_MUTED));
                                    let fav_img_h = fav_img_h_for(cell_w);
                                    let fav_row_h = fav_img_h + 26.0; // R8e: 图+名称+紧凑按钮行
                                    for trio in fav_list.chunks(3) {
                                        let row_rect = ui.allocate_exact_size(egui::vec2(avail_w, fav_row_h), egui::Sense::hover()).0;
                                        for (ci, meta) in trio.iter().enumerate() {
                                            let cell = egui::Rect::from_min_size(
                                                egui::pos2(row_rect.left() + ci as f32 * (cell_w + sp), row_rect.top()),
                                                egui::vec2(cell_w, fav_row_h),
                                            );
                                            let (del, fav, rn, upd) = ui.allocate_new_ui(egui::UiBuilder::new().max_rect(cell), |ui| item(ui, meta, cell_w, fav_img_h)).inner;
                                            if del { delete_ask = Some(meta.name.clone()); }
                                            if fav { fav_toggle = Some((meta.name.clone(), !meta.fav)); }
                                            if rn { rename_start = Some(meta.name.clone()); }
                                        }
                                    }
                                    ui.separator();
                                }
                                // 全部区(一行一个, 大图)
                                if !other_list.is_empty() || fav_list.is_empty() {
                                    if !other_list.is_empty() {
                                        ui.label(egui::RichText::new(tr("全部", "All")).small().strong().color(COL_MUTED));
                                    }
                                    for meta in &other_list {
                                        let (del, fav, rn, upd) = item(ui, meta, avail_w, big_img_h_for(avail_w));
                                        ui.add_space(4.0);
                                        if del { delete_ask = Some(meta.name.clone()); }
                                        if fav { fav_toggle = Some((meta.name.clone(), !meta.fav)); }
                                        if rn { rename_start = Some(meta.name.clone()); }
                                    }
                                }
                            });
                        // 删除确认(R8b: 弹层)
                        if let Some(name) = self.sel_delete_confirm.clone() {
                            let text = t!("删除「{name}」? 含截图, 不可恢复", "Delete \"{name}\"? Includes image, cannot be undone");
                            match confirm_popup(ui.ctx(), "sel-del", &text) {
                                Some(true) => {
                                    match pla_ui::selection::delete_cdp_selections_batch(
                                        &self.doc_key,
                                        &[name.clone()],
                                    ) {
                                        Ok(_) => {
                                            self.sel_saved_loaded = false;
                                            self.sel_pngs.remove(&name);
                                            self.sel_selected.remove(&name);
                                            self.set_notice((false, t!("已删除「{name}」", "Deleted \"{name}\"")));
                                        }
                                        Err(e) => self.set_notice((true, e)),
                                    }
                                    self.sel_delete_confirm = None;
                                }
                                Some(false) => self.sel_delete_confirm = None,
                                None => {}
                            }
                        }
                        if let Some(n) = delete_ask {
                            self.sel_delete_confirm = Some(n);
                        }
                        // R35: 更新确认 — 用当前选择(+ 相机 + 截图)覆盖这个选择集, 名字与收藏保留
                        if let Some(name) = self.sel_update_confirm.clone() {
                            let text = t!("用当前选择覆盖「{name}」? 原内容不可恢复", "Overwrite \"{name}\" with the current selection? The old content cannot be recovered");
                            match confirm_popup(ui.ctx(), "sel-upd", &text) {
                                Some(true) => {
                                    if self.sel_cdp_members.is_empty() {
                                        self.set_notice((true, tr("实时选择为空 — 请先在视口里选中目标", "Live selection is empty — select something in the viewport first").into()));
                                    } else {
                                        let members = self.sel_cdp_members.clone();
                                        let cam = self.sel_camera.clone();
                                        let tmp = pla_ui::selection::cdp_selections_dir(&self.doc_key).join("__shot_tmp.png");
                                        let tmp_str = tmp.display().to_string();
                                        self.sel_shot_path = Some(tmp_str.clone());
                                        self.sel_save_pending = Some((name.clone(), members, cam));
                                        self.cdp.send(pla_bridge::cdp::CdpCmd::CaptureViewport(tmp_str));
                                        self.sel_pngs.remove(&name);
                                        self.set_notice((false, t!("正在更新选择集 {name}(截图中)…", "Updating selection set {name} (capturing)…")));
                                    }
                                    self.sel_update_confirm = None;
                                }
                                Some(false) => self.sel_update_confirm = None,
                                None => {}
                            }
                        }
                        if let Some(n) = update_ask {
                            self.sel_update_confirm = Some(n);
                        }

                        // ---- 统一执行动作(借用结束) ----
                        if let Some((name, fav)) = fav_toggle {
                            let _ = pla_ui::selection::set_cdp_selection_fav(&self.doc_key, &name, fav);
                            self.sel_saved_loaded = false;
                        }
                        if rename_cancel {
                            self.sel_rename = None;
                        }
                        if let Some(old) = rename_start {
                            self.sel_rename_buf = old.clone();
                            self.sel_rename = Some(old);
                        }
                        if rename_commit {
                            if let Some(old) = self.sel_rename.clone() {
                                match pla_ui::selection::rename_cdp_selection(&self.doc_key, &old, &self.sel_rename_buf) {
                                    Ok(_) => {
                                        self.sel_saved_loaded = false;
                                        self.sel_pngs.remove(&old);
                                        self.set_notice((false, t!("已改名: {old} → {}", "Renamed: {old} → {}", self.sel_rename_buf.trim())));
                                    }
                                    Err(e) => self.set_notice((true, e)),
                                }
                            }
                            self.sel_rename = None;
                        }
                        if let Some(name) = apply_now.filter(|_| self.require_feature(Feature::SelApply)) {
                            match pla_ui::selection::load_cdp_selection(&self.doc_key, &name) {
                                Some(members) => {
                                    let entries: Vec<pla_bridge::cdp::ApplyEntry> = members
                                        .iter()
                                        .map(|m| pla_bridge::cdp::ApplyEntry {
                                            kind: m.kind.clone(),
                                            id: m.id.clone(),
                                            key: m.key.clone(),
                                            camera: m.camera.clone(),
                                        })
                                        .collect();
                                    if entries.is_empty() {
                                        self.set_notice((true, tr("该选择集没有成员", "This selection set has no members").into()));
                                    } else {
                                        let n = entries.len();
                                        self.send_apply(entries);
                                        self.pending_focus = true;
                                        self.set_notice((false, t!("正在应用「{name}」{n} 项…", "Applying \"{name}\" ({n} items)…")));
                                    }
                                }
                                None => self.set_notice((true, tr("读取失败", "Failed to read").into())),
                            }
                        }
                        if let Some(e) = open_err {
                            self.set_notice((true, e));
                        }
                        if let Some(t) = inner_tip.into_inner() {
                            self.hovered_hint = Some(t);
                        }
                    },
                    Some(icons),
                );
                // 图标 hover → HUD
                if let Some(t) = tip_slot.borrow_mut().take() {
                    self.hovered_hint = Some(t);
                }
                // 💾 保存: SG 自动命名 + 截图 → (pending 流程: 截图回包后落盘)
                if do_save.get() && self.require_feature(Feature::SelSave) {
                    if self.sel_cdp_members.is_empty() {
                        self.set_notice((
                            true,
                            tr("实时选择为空 — 请先在视口里选中目标", "Live selection is empty — select something in the viewport first").into(),
                        ));
                    } else {
                        let name = pla_ui::selection::next_sg_name(&self.doc_key);
                        let members = self.sel_cdp_members.clone();
                        let cam = self.sel_camera.clone();
                        // 截图临时路径(exe 同级 temp)
                        let tmp = pla_ui::selection::cdp_selections_dir(&self.doc_key)
                            .join(format!("__shot_tmp.png"));
                        let tmp_str = tmp.display().to_string();
                        self.sel_shot_path = Some(tmp_str.clone());
                        self.sel_save_pending = Some((name.clone(), members, cam));
                        self.cdp.send(pla_bridge::cdp::CdpCmd::CaptureViewport(tmp_str));
                        self.set_notice((false, t!("正在保存选择集 {name}(截图中)…", "Saving selection set {name} (capturing)…")));
                    }
                }
                if do_open_dir.get() {
                    if let Err(e) = pla_ui::selection::open_cdp_selections_dir(&self.doc_key) {
                        self.set_notice((true, e));
                    }
                }
                if do_manage.get() {
                    self.sel_manage = !self.sel_manage;
                    self.sel_selected.clear();
                    self.sel_rename = None;
                }
                self.show_entities = flag;
            }
        }

        // ============ R8-1: 视口状态面板(布局与快照墙一致) ============
        if self.show_vpstate {
            let desired = next_pos(&mut stack_y, PANEL_VPSTATE[1]);
            if let Some(desired) = desired {
                let mut flag = self.show_vpstate;
                let do_save = std::rc::Rc::new(std::cell::Cell::new(false));
                let do_open_dir = std::rc::Rc::new(std::cell::Cell::new(false));
                let do_manage = std::rc::Rc::new(std::cell::Cell::new(false));
                let tip_slot = std::rc::Rc::new(std::cell::RefCell::new(None::<(String, String)>));
                let (dsv, dodv, dmv, tsv) = (do_save.clone(), do_open_dir.clone(), do_manage.clone(), tip_slot.clone());
                let icons: Box<dyn FnOnce(&mut egui::Ui)> = Box::new(move |ui| {
                    let vbtn = |ui: &mut egui::Ui, kind: IconKind, tip: &str, hit: &std::cell::Cell<bool>, ts: &std::cell::RefCell<Option<(String, String)>>| {
                        let resp = vector_icon_button(ui, kind, tip, ts);
                        if resp.clicked() {
                            hit.set(true);
                        }
                    };
                    vbtn(ui, IconKind::Trash, tr("批量删除", "Batch delete"), &dmv, &tsv);
                    vbtn(ui, IconKind::Folder, tr("打开视口状态文件夹", "Open view state folder"), &dodv, &tsv);
                    vbtn(
                        ui,
                        IconKind::Save,
                        tr("保存当前视口状态(VS 自动命名 + 截图)", "Save current view state (auto VS name + image)"),
                        &dsv,
                        &tsv,
                    );
                });
                header_extra_and_show(
                    ctx,
                    "panel-vpstate",
                    tr("视口状态", "View states"),
                    desired,
                    egui::vec2(PANEL_VPSTATE[0], PANEL_VPSTATE[1]),
                    &mut flag,
                    true,
                    |ui| {
                        let inner_tip = std::cell::RefCell::new(None::<(String, String)>);
                        if !self.vp_states_loaded {
                            self.vp_states = pla_ui::vpstate::list_vpstates(&self.doc_key);
                            self.vp_states_loaded = true;
                        }
                        // R28: 管理模式操作条放列表顶部(与快照墙一致; 独立的 vp_manage/vp_selected, 不再借用选择集的)
                        if self.vp_manage {
                            let all: Vec<String> = self.vp_states.iter().map(|m| m.name.clone()).collect();
                            let (del, done) = manage_bar(
                                ui,
                                &all,
                                &mut self.vp_selected,
                                tr("删除勾选的视口状态(含截图), 不可恢复", "Delete checked view states (with images), cannot be undone"),
                            );
                            if del {
                                let names: Vec<String> = self.vp_selected.iter().cloned().collect();
                                match pla_ui::vpstate::delete_vpstates(&self.doc_key, &names) {
                                    Ok(n) => {
                                        self.vp_states_loaded = false;
                                        self.vp_states.retain(|m| !names.contains(&m.name));
                                        for nm in &names { self.vp_pngs.remove(nm); }
                                        self.vp_selected.clear();
                                        self.set_notice((false, t!("已删除 {n} 个视口状态", "Deleted {n} view states")));
                                    }
                                    Err(e) => self.set_notice((true, e)),
                                }
                            }
                            if done {
                                self.vp_manage = false;
                                self.vp_selected.clear();
                            }
                        }
                        // PNG 缓存
                        let vdir = pla_ui::vpstate::vpstate_dir(&self.doc_key);
                        for m in self.vp_states.clone() {
                            if !self.vp_pngs.contains_key(&m.name) {
                                if let Ok(b) = std::fs::read(vdir.join(format!("{}.png", m.name))) {
                                    self.vp_pngs.insert(m.name.clone(), std::sync::Arc::from(b));
                                }
                            }
                        }
                        let (fav_list, other_list): (Vec<_>, Vec<_>) =
                            self.vp_states.iter().cloned().partition(|m| m.fav);
                        if self.vp_states.is_empty() {
                            ui.label(
                                egui::RichText::new(tr(
                                    "(空 — 点右上软盘图标保存当前视口)",
                                    "(Empty — click the disk icon at top right to save the view)",
                                ))
                                .small()
                                .color(COL_MUTED),
                            );
                        }
                        let mut delete_ask: Option<String> = None; let mut update_ask: Option<String> = None;
                        let mut apply_now: Option<String> = None;
                        let mut fav_toggle: Option<(String, bool)> = None;
                        let mut rename_start: Option<String> = None;
                        let mut rename_commit = false;
                        let mut rename_cancel = false;
                        // 悬浮信息: 从 JSON 里提取相机/显示模式摘要
                        let state_brief = |name: &str| -> String {
                            match pla_ui::vpstate::load_vpstate(&self.doc_key, name) {
                                Some(s) => {
                                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                                        let mode = v["shading"]["mode"].as_str().unwrap_or("-");
                                        let sec = v["shading"]["section"].as_str().unwrap_or("none");
                                        let nsec = v["sections"].as_array().map(|a| a.len()).unwrap_or(0);
                                        let zoom = v["orbit"]["zoom"].as_f64();
                                        let fov = v["cam"]["fov"].as_f64();
                                        t!(
                                            "显示: {mode} / 剖面模式: {sec}({nsec}) / {} / zoom {}",
                                            "Shading: {mode} / Section: {sec} ({nsec}) / {} / zoom {}",
                                            if fov == Some(0.0) { tr("正交", "Ortho") } else { tr("透视", "Persp") },
                                            zoom.map(|z| format!("{z:.1}")).unwrap_or("-".into()),
                                        )
                                    } else {
                                        tr("状态数据无法解析", "State data unparsable").into()
                                    }
                                }
                                None => tr("状态数据缺失", "State data missing").into(),
                            }
                        };
                        // R23: 存档墙占满剩余高度(以前封顶屏高 1/3 → 收藏区挤得放不下一个); 管理模式给下方操作条留高度。
                        let content_h = ui.available_height().max(72.0);
                        egui::ScrollArea::vertical()
                            .id_salt("vp-wall")
                            .max_height(content_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let avail_w = ui.available_width();
                                let sp = ui.spacing().item_spacing.x;
                                let cell_w = ((avail_w - sp * 2.0) / 3.0).max(40.0);
                                let mut item = |ui: &mut egui::Ui, meta: &pla_ui::vpstate::VpStateMeta, w: f32, img_h: f32| {
                                    let mut del = false; let mut fav = false; let mut rn = false; let mut upd = false;
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(w, img_h + 38.0),
                                        egui::Layout::top_down(egui::Align::LEFT),
                                        |ui| {
                                            let png = self.vp_pngs.get(&meta.name);
                                            let brief = state_brief(&meta.name);
                                            let dims = png.and_then(|b| png_dims(b));
                                            let img_resp = match (png, dims) {
                                                (Some(bytes), Some((pw, ph))) => {
                                                    let iw = (img_h * pw / ph).min(w);
                                                    ui.allocate_ui_with_layout(
                                                        egui::vec2(w, img_h),
                                                        egui::Layout::top_down(egui::Align::Center),
                                                        |ui| ui.add(
                                                            egui::Image::from_bytes(png_uri("vp", &meta.name, &bytes), bytes.clone())
                                                                .fit_to_exact_size(egui::vec2(iw, img_h))
                                                                .sense(egui::Sense::click()),
                                                        ),
                                                    ).inner
                                                }
                                                _ => ui.add_sized(
                                                    egui::vec2(w, img_h),
                                                    egui::Label::new(egui::RichText::new(tr("无截图", "No image")).small().color(COL_MUTED)).sense(egui::Sense::click()),
                                                ),
                                            };
                                            if img_resp.hovered() {
                                                *inner_tip.borrow_mut() = Some((
                                                    t!("{} — 视口状态", "{} — view state", meta.name),
                                                    t!(
                                                        "{brief} · 保存于 {} · 点击恢复",
                                                        "{brief} · saved {} · click to restore",
                                                        if meta.created.is_empty() { "-" } else { &meta.created }
                                                    ),
                                                ));
                                            }
                                            if img_resp.clicked() {
                                                apply_now = Some(meta.name.clone());
                                            }
                                            ui.horizontal(|ui| {
                                                if self.vp_manage {
                                                    let mut on = self.vp_selected.contains(&meta.name);
                                                    if ui.checkbox(&mut on, "").changed() {
                                                        if on { self.vp_selected.insert(meta.name.clone()); } else { self.vp_selected.remove(&meta.name); }
                                                    }
                                                }
                                                if self.vp_rename.as_deref() == Some(meta.name.as_str()) {
                                                    ui.add_sized(egui::vec2((w - 60.0).max(30.0), 16.0), egui::TextEdit::singleline(&mut self.vp_rename_buf)).request_focus();
                                                    let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                                                    if enter { rename_commit = true; }
                                                    if esc { rename_cancel = true; }
                                                } else {
                                                    // R8d: 名称独占一行
                                                    let name_w = (w - if self.vp_manage { 16.0 + sp } else { 0.0 }).max(30.0);
                                                    let nr = ui.allocate_ui_with_layout(
                                                        egui::vec2(name_w, 14.0),
                                                        egui::Layout::left_to_right(egui::Align::Center),
                                                        |ui| ui.add(egui::Label::new(egui::RichText::new(&meta.name).small().color(COL_TEXT)).sense(egui::Sense::click()).truncate()),
                                                    ).inner;
                                                    if nr.clicked() {
                                                        apply_now = Some(meta.name.clone());
                                                    }
                                                }
                                            });
                                            // R8d: 第二行按钮组(★ ✎ 🗑)
                                            ui.horizontal(|ui| {
                                                let (srect, sresp) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
                                                ui.painter().text(srect.center(), egui::Align2::CENTER_CENTER, if meta.fav { "★" } else { "☆" }, egui::FontId::proportional(13.0), if meta.fav { egui::Color32::from_rgb(255,200,60) } else { COL_MUTED });
                                                if sresp.on_hover_text(if meta.fav { tr("取消收藏", "Unfavorite") } else { tr("收藏(置顶)", "Favorite (pin to top)") }).clicked() { fav = true; }
                                                if pencil_button(ui).on_hover_text(tr("改名", "Rename")).clicked() { rn = true; }
                                                let (drect, dresp) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
                                                let dcol = if dresp.hovered() { COL_RED } else { COL_MUTED };
                                                let p = |x: f32, y: f32| egui::pos2(drect.left() + x, drect.top() + y);
                                                ui.painter().line_segment([p(3.0,4.0), p(11.0,4.0)], egui::Stroke::new(1.2, dcol));
                                                ui.painter().line_segment([p(5.5,4.0), p(5.5,2.5)], egui::Stroke::new(1.2, dcol));
                                                ui.painter().line_segment([p(8.5,4.0), p(8.5,2.5)], egui::Stroke::new(1.2, dcol));
                                                ui.painter().line_segment([p(6.2,2.5), p(7.8,2.5)], egui::Stroke::new(1.2, dcol));
                                                ui.painter().line_segment([p(4.0,4.0), p(5.0,12.0)], egui::Stroke::new(1.2, dcol));
                                                ui.painter().line_segment([p(10.0,4.0), p(9.0,12.0)], egui::Stroke::new(1.2, dcol));
                                                ui.painter().line_segment([p(5.0,12.0), p(9.0,12.0)], egui::Stroke::new(1.2, dcol));
                                                if dresp.on_hover_text(tr("删除(需确认)", "Delete (confirm)")).clicked() { del = true; }
                                            });
                                        },
                                    );
                                    (del, fav, rn, upd)
                                };
                                if !fav_list.is_empty() {
                                    ui.label(egui::RichText::new(tr("★ 收藏", "★ Favorites")).small().strong().color(COL_MUTED));
                                    let fav_img_h = fav_img_h_for(cell_w);
                                    let fav_row_h = fav_img_h + 26.0; // R8e: 图+名称+紧凑按钮行
                                    for trio in fav_list.chunks(3) {
                                        let row_rect = ui.allocate_exact_size(egui::vec2(avail_w, fav_row_h), egui::Sense::hover()).0;
                                        for (ci, meta) in trio.iter().enumerate() {
                                            let cell = egui::Rect::from_min_size(
                                                egui::pos2(row_rect.left() + ci as f32 * (cell_w + sp), row_rect.top()),
                                                egui::vec2(cell_w, fav_row_h),
                                            );
                                            let (del, fav, rn, upd) = ui.allocate_new_ui(egui::UiBuilder::new().max_rect(cell), |ui| item(ui, meta, cell_w, fav_img_h)).inner;
                                            if del { delete_ask = Some(meta.name.clone()); }
                                            if fav { fav_toggle = Some((meta.name.clone(), !meta.fav)); }
                                            if rn { rename_start = Some(meta.name.clone()); }
                                        }
                                    }
                                    ui.separator();
                                }
                                if !other_list.is_empty() {
                                    ui.label(egui::RichText::new(tr("全部", "All")).small().strong().color(COL_MUTED));
                                    for meta in &other_list {
                                        let (del, fav, rn, upd) = item(ui, meta, avail_w, big_img_h_for(avail_w));
                                        ui.add_space(4.0);
                                        if del { delete_ask = Some(meta.name.clone()); }
                                        if fav { fav_toggle = Some((meta.name.clone(), !meta.fav)); }
                                        if rn { rename_start = Some(meta.name.clone()); }
                                    }
                                }
                            });
                        // 删除确认(R8b: 弹层)
                        if let Some(name) = self.vp_delete_confirm.clone() {
                            let text = t!("删除「{name}」? 不可恢复", "Delete \"{name}\"? Cannot be undone");
                            match confirm_popup(ui.ctx(), "vp-del", &text) {
                                Some(true) => {
                                    match pla_ui::vpstate::delete_vpstates(
                                        &self.doc_key,
                                        &[name.clone()],
                                    ) {
                                        Ok(_) => {
                                            self.vp_states_loaded = false;
                                            self.vp_pngs.remove(&name);
                                            self.set_notice((false, t!("已删除「{name}」", "Deleted \"{name}\"")));
                                        }
                                        Err(e) => self.set_notice((true, e)),
                                    }
                                    self.vp_delete_confirm = None;
                                }
                                Some(false) => self.vp_delete_confirm = None,
                                None => {}
                            }
                        }
                        if let Some(n) = delete_ask {
                            self.vp_delete_confirm = Some(n);
                        }
                        // R35: 更新确认 — 采集当前视口状态 + 缩略图, 覆盖这一项(名字与收藏保留)
                        if let Some(name) = self.vp_update_confirm.clone() {
                            let text = t!("用当前视口状态覆盖「{name}」? 原内容不可恢复", "Overwrite \"{name}\" with the current view state? The old content cannot be recovered");
                            match confirm_popup(ui.ctx(), "vp-upd", &text) {
                                Some(true) => {
                                    let tmp = pla_ui::vpstate::vpstate_dir(&self.doc_key).join("__shot_tmp.png");
                                    let _ = std::fs::create_dir_all(tmp.parent().unwrap_or(std::path::Path::new(".")));
                                    self.vp_shot_path = Some(tmp.display().to_string());
                                    self.vp_pending_state = None;
                                    self.vp_update_target = Some(name.clone());
                                    self.vp_pngs.remove(&name);
                                    self.cdp.send(pla_bridge::cdp::CdpCmd::VpStateSave);
                                    self.set_notice((false, t!("正在更新视口状态 {name}…", "Updating view state {name}…")));
                                    self.vp_update_confirm = None;
                                }
                                Some(false) => self.vp_update_confirm = None,
                                None => {}
                            }
                        }
                        if let Some(n) = update_ask {
                            self.vp_update_confirm = Some(n);
                        }
                        // 动作执行
                        if let Some((name, fav)) = fav_toggle {
                            let _ = pla_ui::vpstate::set_fav(&self.doc_key, &name, fav);
                            self.vp_states_loaded = false;
                        }
                        if rename_cancel {
                            self.vp_rename = None;
                        }
                        if let Some(old) = rename_start {
                            self.vp_rename_buf = old.clone();
                            self.vp_rename = Some(old);
                        }
                        if rename_commit {
                            if let Some(old) = self.vp_rename.clone() {
                                match pla_ui::vpstate::rename_vpstate(&self.doc_key, &old, &self.vp_rename_buf) {
                                    Ok(_) => {
                                        self.vp_states_loaded = false;
                                        self.vp_pngs.remove(&old);
                                        self.set_notice((false, t!("已改名: {old} → {}", "Renamed: {old} → {}", self.vp_rename_buf.trim())));
                                    }
                                    Err(e) => self.set_notice((true, e)),
                                }
                            }
                            self.vp_rename = None;
                        }
                        if let Some(name) = apply_now.filter(|_| self.require_feature(Feature::VpApply)) {
                            match pla_ui::vpstate::load_vpstate(&self.doc_key, &name) {
                                Some(state) => {
                                    self.cdp.send(pla_bridge::cdp::CdpCmd::VpStateApply(state));
                                    self.pending_focus = true;
                                    self.set_notice((false, t!("正在恢复视口状态「{name}」…", "Restoring view state \"{name}\"…")));
                                }
                                None => self.set_notice((true, tr("状态数据缺失", "State data missing").into())),
                            }
                        }
                        if let Some(t) = inner_tip.into_inner() {
                            self.hovered_hint = Some(t);
                        }
                    },
                    Some(icons),
                );
                if let Some(t) = tip_slot.borrow_mut().take() {
                    self.hovered_hint = Some(t);
                }
                // 💾: 采集 + 截图 → pending 落盘
                if do_save.get() && self.require_feature(Feature::VpSave) {
                    // 串行流程: 先采集状态 → 回包后再截图(VpStateResult 处理器触发)
                    let tmp = pla_ui::vpstate::vpstate_dir(&self.doc_key)
                        .join("__shot_tmp.png");
                    let _ = std::fs::create_dir_all(
                        &tmp.parent().unwrap_or(std::path::Path::new(".")),
                    );
                    self.vp_shot_path = Some(tmp.display().to_string());
                    self.vp_pending_state = None;
                    self.cdp.send(pla_bridge::cdp::CdpCmd::VpStateSave);
                    self.set_notice((false, tr("正在采集视口状态…", "Capturing view state…").into()));
                }
                if do_open_dir.get() {
                    if let Err(e) = pla_ui::vpstate::open_dir(&self.doc_key) {
                        self.set_notice((true, e));
                    }
                }
                if do_manage.get() {
                    self.vp_manage = !self.vp_manage;
                    self.vp_selected.clear();
                    self.vp_rename = None;
                }
                self.show_vpstate = flag;
            }
        }

        if self.show_dim {
            let desired = next_pos(&mut stack_y, PANEL_DIM[1]);
            if let Some(desired) = desired {
                let mut flag = self.show_dim;
                show_child_window(
                    ctx,
                    "panel-dim",
                    tr("尺寸驱动", "Dimension drive"),
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
                        ui.horizontal_wrapped(|ui| {
                            ui.label(tr("关联选择集:", "Linked set:"));
                            let cur = self
                                .dim_sets
                                .get(self.dim_set_sel)
                                .map(|s| s.as_str())
                                .unwrap_or(tr("(无)", "(none)"));
                            egui::ComboBox::from_id_salt("dim-set")
                                .selected_text(cur)
                                .show_ui(ui, |ui| {
                                    for (i, s) in self.dim_sets.iter().enumerate() {
                                        ui.selectable_value(&mut self.dim_set_sel, i, s);
                                    }
                                });
                            if ui.small_button(tr("刷新", "Refresh")).clicked() {
                                self.dim_sets =
                                    pla_ui::selection::list_cdp_selections(&self.doc_key);
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.label(tr("方向:", "Axis:"));
                            for (i, ax) in ["X", "Y", "Z"].iter().enumerate() {
                                ui.selectable_value(&mut self.dim_axis, i, *ax);
                            }
                            ui.label(tr("数值(毫米):", "Value (mm):"));
                            ui.add(egui::DragValue::new(&mut self.dim_value).speed(0.1));
                        });
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(tr("① 选中关联实体", "① Select linked")).clicked() {
                                let Some(name) = self.dim_sets.get(self.dim_set_sel).cloned() else {
                                    self.set_notice((
                                        true,
                                        tr(
                                            "没有已保存的选择集 — 先在选择集窗口保存",
                                            "No saved selection sets — save one in the Selection sets panel first",
                                        )
                                        .into(),
                                    ));
                                    return;
                                };
                                match pla_ui::selection::load_cdp_selection(&self.doc_key, &name) {
                                    Some(members) => {
                                        let entries: Vec<pla_bridge::cdp::ApplyEntry> = members
                                            .iter()
                                            .map(|m| pla_bridge::cdp::ApplyEntry {
                                                kind: m.kind.clone(),
                                                id: m.id.clone(),
                                                key: m.key.clone(),
                                                camera: m.camera.clone(),
                                            })
                                            .collect();
                                        if entries.is_empty() {
                                            self.set_notice((true, tr("选择集没有成员", "Selection set has no members").into()));
                                        } else {
                                            let n = entries.len();
                                            self.send_apply(entries);
                                            // R7-26: 应用完成后把焦点还给 Plasticity。
                                            self.pending_focus = true;
                                            self.set_notice((
                                                false,
                                                t!("正在选中 {n} 项 ({name})…", "Selecting {n} items ({name})…"),
                                            ));
                                        }
                                    }
                                    None => {
                                        self.set_notice((true, t!("选择集 {name} 读取失败", "Failed to read selection set {name}")))
                                    }
                                }
                            }
                            if ui.button(tr("保存关联", "Save link")).clicked() {
                                let Some(name) = self.dim_sets.get(self.dim_set_sel).cloned() else {
                                    self.set_notice((true, tr("没有关联的选择集", "No linked selection set").into()));
                                    return;
                                };
                                let axis = ["X", "Y", "Z"][self.dim_axis.min(2)];
                                // 与选择集同目录(唯一入口 cdp_selections_dir)。
                                let dir = pla_ui::selection::cdp_selections_dir(&self.doc_key);
                                let json = serde_json::json!({
                                    "set": name, "axis": axis, "value": self.dim_value,
                                });
                                match std::fs::write(
                                    dir.join(format!("dim-{}.json", name)),
                                    serde_json::to_vec_pretty(&json).unwrap_or_default(),
                                ) {
                                    Ok(_) => self.set_notice((
                                        false,
                                        t!("尺寸关联已保存: {name} {axis}={}", "Dimension link saved: {name} {axis}={}", self.dim_value),
                                    )),
                                    Err(e) => self.set_notice((true, t!("保存失败: {e}", "Save failed: {e}"))),
                                }
                            }
                        });
                        ui.label(
                            egui::RichText::new(tr(
                                "② 应用平移: 命令参数化(move-item 交互状态机)待活体逆向 — ① 已可用",
                                "② Apply move: command parameterization (move-item state machine) pending live RE — ① works",
                            ))
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
                    tr("设置", "Settings"),
                    desired,
                    egui::vec2(PANEL_SETTINGS[0], PANEL_SETTINGS[1]),
                    &mut flag,
                    true,
                    |ui| {
                        if !self.step_loaded {
                            self.step_loaded = true;
                            self.reload_step_config();
                            self.cdp_startup = Some(pla_ui::cdp_mode::cdp_startup_enabled());
                        }
                        // R15: 原生风格分组卡片; 说明文字全部移到悬停三级面板。
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        // ---- R25: 界面语言(一键切换, 立即生效, 记入 ui-prefs.json) ----
                        // 选项用各自语言的名字显示(简体中文 / English), 无论当前是哪种语言都认得出来。
                        settings_card(ui, "LANGUAGE", None, |ui| {
                            let cur = pla_bridge::i18n::get();
                            let w = ((ui.available_width() - 8.0) / 2.0).max(60.0);
                            ui.horizontal(|ui| {
                                for lang in pla_bridge::i18n::Lang::ALL {
                                    let on = cur == lang;
                                    let btn = egui::Button::new(
                                        egui::RichText::new(lang.native_name())
                                            .color(if on { egui::Color32::BLACK } else { COL_TEXT }),
                                    )
                                    .fill(if on { COL_ACCENT } else { COL_BTN })
                                    .rounding(egui::Rounding::same(6.0))
                                    .min_size(egui::vec2(w, 24.0));
                                    if ui.add(btn).clicked() && !on {
                                        pla_bridge::i18n::set(lang);
                                        if let Err(e) = pla_ui::prefs::save_lang(lang) {
                                            self.set_notice((true, e));
                                        }
                                    }
                                }
                            });
                        });
                        // R27: BRIDGE 卡已移到 CDP 能力面板(点工具栏状态点打开)。
                        // ---- F7: Plasticity 默认以 CDP 模式运行(pla-ui 托管代理) ----
                        settings_card(
                            ui,
                            "CDP MODE",
                            Some((
                                tr("CDP 通道", "CDP channel"),
                                vec![
                                    info_text(tr(
                                        "pla-ui 经 CDP 读写 Plasticity 的运行时状态: 选择集、视口状态、\
                                         角度即时生效、Alt+←/→ 绕屏幕轴旋转都依赖它。",
                                        "pla-ui reads/writes Plasticity's runtime state via CDP: selection sets, view states, \
                                         live angle changes and Alt+←/→ screen-axis orbit all rely on it.",
                                    )),
                                    info_text(tr(
                                        "代理(9333)由 pla-ui 在 Plasticity 带调试端口启动时自动拉起(需要 node)。",
                                        "The proxy (9333) is started by pla-ui when Plasticity runs with a debug port (requires node).",
                                    )),
                                ],
                            )),
                            |ui| {
                                let cdp_col = if self.cdp_connected() { COL_GREEN } else { COL_MUTED };
                                value_row(ui, "CDP", &self.cdp_conn.text(), cdp_col);
                                value_row(ui, tr("代理", "Proxy"), &self.proxy.status, COL_TEXT);
                        let mut cdp_on = self.cdp_startup.unwrap_or(false);
                        if toggle_row(
                            ui,
                            tr("默认以 CDP 模式启动", "Launch in CDP mode by default"),
                            &mut cdp_on,
                            Some((
                                tr("Plasticity 默认以 CDP 模式启动", "Launch Plasticity in CDP mode by default"),
                                vec![
                                    info_text(tr(
                                        "开启: 任务栏 / 开始菜单快捷方式与 .plasticity 双击打开, 改为带调试端口 9334 \
                                         启动已打补丁的 Plasticity。下次启动 Plasticity 生效; 关闭即恢复原样。",
                                        "On: taskbar / Start menu shortcuts and double-clicking .plasticity files launch the \
                                         patched Plasticity with debug port 9334. Applies on next launch; turn off to restore.",
                                    )),
                                    info_text(tr(
                                        "调试端口只监听本机(局域网不可达), 但本机程序可借此控制 Plasticity。",
                                        "The debug port listens on localhost only (not reachable from LAN), but local programs could use it to control Plasticity.",
                                    )),
                                    info_text(tr(
                                        "公用桌面快捷方式需管理员权限, 不会修改。",
                                        "Public desktop shortcuts need admin rights and are left unchanged.",
                                    )),
                                ],
                            )),
                        )
                        .changed()
                        {
                            self.cdp_startup_msg = Some(
                                match pla_ui::cdp_mode::set_cdp_startup(cdp_on) {
                                    Ok(lines) => {
                                        self.cdp_startup = Some(cdp_on);
                                        // 结果行由 cdp_mode 按当时语言生成 → 中英文失败标记都认。
                                        let failed = lines.iter().any(|l| l.contains("失败") || l.contains("failed"));
                                        (failed, lines.join("\n"))
                                    }
                                    Err(e) => (true, e),
                                },
                            );
                        }
                        if let Some((is_err, msg)) = &self.cdp_startup_msg {
                            ui.label(
                                egui::RichText::new(msg)
                                    .small()
                                    .color(if *is_err { COL_RED } else { COL_GREEN }),
                            );
                        }
                            },
                        );
                        // ---- Q1: 方向键步进旋转(写 Plasticity 自己的 keymap/settings) ----
                        settings_card(ui, "ORBIT STEP", None, |ui| {
                        let mut bound = self.step_bound.unwrap_or(false);
                        if toggle_row(
                            ui,
                            tr("方向键步进旋转视角", "Arrow keys orbit the view"),
                            &mut bound,
                            Some((
                                tr("方向键步进旋转视角", "Arrow keys orbit the view"),
                                vec![
                                    info_key("←  /  →", tr("左右旋转", "Orbit left/right")),
                                    info_key("↑  /  ↓", tr("上下旋转", "Orbit up/down")),
                                    info_key("Alt+←  /  Alt+→", tr("绕屏幕轴旋转(需 CDP)", "Roll around screen axis (needs CDP)")),
                                    info_text(tr("每按一下转动的角度见下一项。", "The angle per press is set below.")),
                                    info_text(tr(
                                        "按键写入 ~/.plasticity/config/v2/keymap.json, Plasticity 自动加载。\
                                         视口立方体右键的旋转按钮固定 90°, 不受此设置影响。",
                                        "Keys are written to ~/.plasticity/config/v2/keymap.json and auto-loaded by Plasticity. \
                                         The view cube's right-click rotate buttons stay at 90° regardless.",
                                    )),
                                ],
                            )),
                        )
                        .changed()
                        {
                            self.step_msg = Some(
                                match pla_ui::plasticity_config::set_arrow_bindings(bound) {
                                    Ok(()) => {
                                        self.step_bound = Some(bound);
                                        // keymap.json 会被 Plasticity 自动重载(有延迟);
                                        // 能力可用时顺带立即生效。
                                        if self.feature_block(Feature::ArrowKeysLive).is_none() {
                                            let (name, args) =
                                                pla_ui::plasticity_config::live_bindings_call(bound);
                                            self.cdp.send(pla_bridge::cdp::CdpCmd::Call { name, args });
                                        }
                                        (
                                            false,
                                            if bound {
                                                tr("已绑定 ←→↑↓ 与 Alt+←/→", "Bound ←→↑↓ and Alt+←/→").into()
                                            } else {
                                                tr("已解除方向键绑定", "Arrow key bindings removed").into()
                                            },
                                        )
                                    }
                                    Err(e) => (true, e),
                                },
                            );
                        }
                        let (mut commit_step, step) = stepper_row(
                            ui,
                            tr("每按一下转动", "Angle per press"),
                            &mut self.step_deg_buf,
                            "°",
                            Some((
                                tr("每按一下转动的角度", "Rotation angle per key press"),
                                vec![
                                    info_text(tr(
                                        "单位 °, 范围 0.1–180; 直接输入后回车, 或点两侧 − / + 按 5° 调整。",
                                        "In degrees, 0.1–180; type and press Enter, or use − / + to step by 5°.",
                                    )),
                                    info_text(tr(
                                        "写入 Plasticity 的 settings.json。CDP 已连接时立即生效, \
                                         否则重启 Plasticity 后生效。Alt+←/→ 的旋转也用这个角度。",
                                        "Written to Plasticity's settings.json. Applies immediately when CDP is connected, \
                                         otherwise after restarting Plasticity. Alt+←/→ uses the same angle.",
                                    )),
                                ],
                            )),
                        );
                        if step != 0 {
                            let cur: f64 = self
                                .step_deg_buf
                                .trim()
                                .trim_end_matches('°')
                                .parse()
                                .unwrap_or(15.0);
                            let next = (cur + 5.0 * step as f64).clamp(
                                pla_ui::plasticity_config::STEP_DEG_MIN,
                                pla_ui::plasticity_config::STEP_DEG_MAX,
                            );
                            self.step_deg_buf = fmt_deg(next);
                            commit_step = true;
                        }
                        if commit_step {
                            self.apply_step_deg();
                        }
                        if let Some((is_err, msg)) = &self.step_msg {
                            ui.label(
                                egui::RichText::new(msg)
                                    .small()
                                    .color(if *is_err { COL_RED } else { COL_GREEN }),
                            );
                        }
                        });
                        let steps_now = if self.cdp_undo >= 0 { self.cdp_undo.to_string() } else { "-".into() };
                        settings_card(ui, "AUTO SNAPSHOT", None, |ui| {
                            toggle_row(
                                ui,
                                tr("启用自动快照", "Enable auto snapshots"),
                                &mut self.auto_snap_enabled,
                                Some((
                                    tr("自动快照", "Auto snapshots"),
                                    vec![
                                        info_text(tr(
                                            "需 CDP 已连接。按「定时」或「定步数」(撤销栈步数增加)触发。",
                                            "Requires CDP. Triggered by \"Interval\" or \"Every N steps\" (undo stack growth).",
                                        )),
                                        info_text(tr(
                                            "快照 = 文档同级 snapshot/ 下的副本; 文档未保存新内容时自动跳过。",
                                            "A snapshot is a copy in snapshot/ next to the document; skipped when nothing new was saved.",
                                        )),
                                        info_text(t!("当前步数: {steps_now}", "Current steps: {steps_now}")),
                                    ],
                                )),
                            );
                            drag_row(ui, tr("定时(分钟)", "Interval (min)"), &mut self.auto_interval_min, 0..=1440,
                                Some((tr("定时(分钟)", "Interval (min)"), vec![info_text(tr(
                                    "每隔多少分钟拍一张; 0 = 不按时间触发。",
                                    "Take one every N minutes; 0 = no time trigger.",
                                ))])));
                            drag_row(ui, tr("定步数", "Every N steps"), &mut self.auto_steps, 0..=1000,
                                Some((tr("定步数", "Every N steps"), vec![info_text(tr(
                                    "撤销栈每增加多少步拍一张; 0 = 不按步数触发。",
                                    "Take one every N undo steps; 0 = no step trigger.",
                                ))])));
                            drag_row(ui, tr("数量上限", "Max count"), &mut self.snap_max_count, 0..=500,
                                Some((tr("数量上限", "Max count"), vec![info_text(tr(
                                    "每个文档最多保留多少张快照(收藏的除外), 超出删最旧的; 0 = 不限。",
                                    "Max snapshots kept per document (favorites excluded); oldest deleted first; 0 = unlimited.",
                                ))])));
                            drag_row(ui, tr("保留天数", "Keep days"), &mut self.snap_max_age_days, 0..=365,
                                Some((tr("保留天数", "Keep days"), vec![info_text(tr(
                                    "超过多少天的快照会被清理(收藏的除外); 0 = 不限。",
                                    "Snapshots older than N days are removed (favorites excluded); 0 = unlimited.",
                                ))])));
                        });
                        settings_card(
                            ui,
                            "HOTKEYS",
                            Some((
                                tr("快捷键", "Hotkeys"),
                                vec![
                                    info_text(tr(
                                        "点「绑定」后按下组合键完成录制; 也可以右键工具栏按钮直接开始录制。",
                                        "Click \"Bind\" then press a key combo; or right-click a toolbar button to start recording.",
                                    )),
                                    info_text(tr(
                                        "快捷键是全局的(Plasticity 在前台时也有效), 保存在 pla-ui 同级的 hotkeys.json。",
                                        "Hotkeys are global (work while Plasticity is focused), saved in hotkeys.json next to pla-ui.",
                                    )),
                                ],
                            )),
                            |ui| {
                            for a in Action::ALL {
                                let key = format!("{a:?}");
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(a.icon());
                                    ui.label(a.label());
                                    let cur = self
                                        .hotkeys
                                        .binds
                                        .get(&key)
                                        .map(|h| h.display())
                                        .unwrap_or_else(|| "-".into());
                                    ui.monospace(cur);
                                    if ui.small_button(tr("绑定", "Bind")).clicked() {
                                        self.recording = Some(key.clone());
                                        self.notice = None;
                                    }
                                    if self.hotkeys.binds.contains_key(&key)
                                        && ui.small_button(tr("清除", "Clear")).clicked()
                                    {
                                        self.hotkeys.binds.remove(&key);
                                        pla_ui::hotkey::save_binds(&self.hotkeys);
                                    }
                                });
                            }
                            let conflicts = self.hotkeys.conflicts();
                            if !conflicts.is_empty() {
                                for (k, acts) in &conflicts {
                                    ui.colored_label(COL_RED, t!("! 冲突: {k} → {acts:?}", "! Conflict: {k} → {acts:?}"));
                                }
                            }
                            },
                        );
                            });
                    },
                );
                self.show_settings = flag;
            }
        }

        // ---- F2: CDP 能力面板(点工具栏状态点打开; 内容全部由 features 表 + 探测结果推导) ----
        if self.show_caps {
            let desired = next_pos(&mut stack_y, PANEL_CAPS[1]);
            if let Some(desired) = desired {
                let mut flag = self.show_caps;
                let view = CapsView {
                    caps: self.cdp_caps.clone(),
                    age_s: self.cdp_caps_at.map(|t| t.elapsed().as_secs()),
                    cdp_conn: self.cdp_conn.clone(),
                    proxy_status: self.proxy.status.clone(),
                    bridge_status: self.bridge_status_text(),
                    bridge_color: status_color(&self.bridge_status),
                    bridge_url: self.bridge.url.clone(),
                    bound_title: self.snap_target.as_ref().map(|t| t.title.clone()),
                };
                let mut reprobe = false;
                show_child_window(
                    ctx,
                    "panel-caps",
                    tr("CDP 能力", "CDP capabilities"),
                    desired,
                    egui::vec2(PANEL_CAPS[0], PANEL_CAPS[1]),
                    &mut flag,
                    true,
                    |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 6.0;
                                caps_panel_ui(ui, &view, &mut reprobe);
                            });
                    },
                );
                if reprobe {
                    self.cdp.send(pla_bridge::cdp::CdpCmd::ProbeCaps);
                }
                self.show_caps = flag;
            }
        }

        // ---- R30: 笔记面板(独立浮窗, 与上面的二级面板并存) ----
        self.notes.tick();
        if self.show_notes {
            self.notes.set_doc(&self.doc_key);
            self.show_notes_window(ctx);
        }

        // ---- R15: 设置项悬停三级面板(面板本帧登记, 本帧渲染; 不悬停即关闭) ----
        if let Some(h) = INFO_HINT.with(|x| x.borrow_mut().take()) {
            show_info_popup(ctx, h);
        }

        // ---- HUD 视口: 悬停提示 + 操作通知, 弹在工具栏右侧 ----
        // 渲染点在所有功能面板之后(R7-9): 面板视口本帧写入的 hover 才能被
        // 本帧渲染, 且不会被下一帧工具栏的帧首覆盖抢先清掉。
        if self.hovered_hint.is_some() || self.notice.is_some() {
            // R9: 同面板 — 用权威锚点(拖动中不滞后)
            if let Some(tb) = tb_origin {
                let hint = self.hovered_hint.clone();
                let notice = self.notice.clone();
                // F2: 高度按内容实测(多行提示 / 长通知不再被裁掉)。
                let size = hud_size(ctx, &hint, &notice);
                let desired = egui::pos2(self.hud_left(tb.x), tb.y + 2.0);
                let mut hud = Some((hint, notice));
                let mut open_flag = true; // notice 的 × 关闭经它传出
                let (hud_hide, hud_reveal) = viewport_reveal(ctx, "hud-popup");
                let mut builder = egui::ViewportBuilder::default()
                    .with_decorations(false)
                    .with_resizable(false)
                    .with_transparent(true)
                    .with_window_level(egui::WindowLevel::AlwaysOnTop)
                    .with_active(false) // 不抢焦点
                    .with_position(desired)
                    .with_inner_size(size);
                if hud_hide {
                    builder = builder.with_visible(false);
                }
                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("hud-popup"),
                    builder,
                    |hctx, class| {
                        if hud_reveal {
                            hctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        }
                        let Some((hint, notice)) = hud.take() else { return };
                        if hctx.input(|i| i.viewport().close_requested()) {
                            open_flag = false;
                        }
                        let show_ui = |ui: &mut egui::Ui, open: &mut bool| {
                            if let Some((l1, l2)) = &hint {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(l1)
                                            .size(12.5)
                                            .strong()
                                            .color(egui::Color32::WHITE),
                                    )
                                    .wrap(),
                                );
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(l2)
                                            .size(11.5)
                                            .color(egui::Color32::from_rgb(0x9a, 0x9a, 0x9a)),
                                    )
                                    .wrap(),
                                );
                            }
                            if let Some((is_err, msg)) = &notice {
                                if hint.is_some() {
                                    ui.separator();
                                }
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(if *is_err { tr("! 提示", "! Notice") } else { tr("√ 完成", "√ Done") })
                                            .size(12.5)
                                            .strong()
                                            .color(if *is_err { COL_RED } else { COL_GREEN }),
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
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(msg)
                                            .color(if *is_err { COL_RED } else { COL_GREEN }),
                                    )
                                    .wrap(),
                                );
                            }
                        };
                        match class {
                            egui::ViewportClass::Embedded
                            | egui::ViewportClass::Root
                            | egui::ViewportClass::Deferred => {
                                // 标题随语言变, 窗口 id 固定(不随标题文本变化)。
                                egui::Window::new(tr("提示", "Notice"))
                                    .id(egui::Id::new("hud-notice-window"))
                                    .resizable(false)
                                    .collapsible(false)
                                    .show(hctx, |ui| show_ui(ui, &mut open_flag));
                            }
                            egui::ViewportClass::Immediate => {
                                egui::CentralPanel::default()
                                    .frame(panel_frame())
                                    .show(hctx, |ui| show_ui(ui, &mut open_flag));
                                paint_panel_border(hctx);
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
                                    t!("{} 已被 {:?} 占用", "{} is already used by {:?}", hk.display(), others),
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
