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

mod state_wall;

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
/// R30: +30 笔记按钮 → 276。R34: +30 材质状态按钮 → 306。
/// 宽度 = 状态点 + 6 个按钮(24 + 间距 6); 2026-11-17 九个按钮减到六个, 306 → 216。
const TOOLBAR_SIZE: [f32; 2] = [216.0, 30.0];
/// 配置窗口(配置 / 视口 / 材质 / 选择集 四个标签共用一个 OS 窗口)。
const CFG_PANEL_ID: &str = "panel-config";
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
/// 用户 2026-11-17: 视口 / 材质 / 选择集并入配置窗口成为二级标签 → 工具栏只留一个配置按钮(快捷键仍可直达各标签)。
const TOOLBAR_ACTIONS: [Action; 6] = [
    Action::ToggleSnapshots,
    Action::ToggleConfig,
    Action::ToggleBom,
    Action::ToggleDim,
    Action::ToggleNotes,
    Action::ToggleSettings,
];

/// R8-1: 视口状态面板尺寸(同快照墙)。
const PANEL_VPSTATE: [f32; 2] = [PANEL_SNAPSHOTS[0], PANEL_SNAPSHOTS[1]];
/// R34: 材质状态面板尺寸(同视口状态)。
const PANEL_MATSTATE: [f32; 2] = PANEL_VPSTATE;
/// C0: 配置面板尺寸(同视口状态)。
const PANEL_CONFIG: [f32; 2] = PANEL_VPSTATE;

/// 面板逻辑尺寸 (宽, 高) — 子视口 OS 窗口大小。
/// R7-6: 快照墙改窄加高(232x480) — 一行一个快照, 缩略图撑满内容宽。
/// R29: 宽 232 → 280, 管理条(已选 N · 全选 · 反选 · 删除 · 完成)中英文都能一行放下。
const PANEL_SNAPSHOTS: [f32; 2] = [280.0, 480.0];
/// R7-20: 加高放选择集墙(96 滚动区)。
/// R23: 选择集面板 = 左列选择集墙(约 280) + 右列 ENTITIES(SEL_ENTITIES_COL_W)。
const PANEL_ENTITIES: [f32; 2] = [520.0, 560.0];
/// R23: 选择集面板右侧 ENTITIES 列宽。
const SEL_ENTITIES_COL_W: f32 = 220.0;
/// N3 尺寸浮窗的默认外框(之后用户拖 / 拉的大小会记住)。
const PANEL_DIM: [f32; 2] = [360.0, 340.0];
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
    /// B1/B2 BOM 浮窗: 位置记忆; 范围(None = 当前可见, Some(配置名)); 最近结果及其范围; 扫描中; 等回包的范围;
    /// "每个配置各出一份"的待扫队列与已导出的文件。
    show_bom: bool,
    bom_win: FloatWin,
    bom_scope: Option<String>,
    bom_result: Option<Result<serde_json::Value, String>>,
    bom_result_scope: Option<String>,
    bom_busy: bool,
    bom_pending_scope: Option<Option<String>>,
    bom_batch: Option<(std::collections::VecDeque<String>, Vec<String>)>,
    /// B3 模板(bom_table): 模板名列表 / 当前名 / 内容 / 修改时间 + 上次检查(热生效)/ 读错;
    /// 本文档的编辑(覆盖 / 排除 / 手动行 / 默认值)及其所属文档; 单元格输入缓冲("行key|列id")。
    bom_tpl_list: Vec<String>,
    bom_tpl_name: String,
    bom_tpl: pla_ui::bom_table::Template,
    bom_tpl_mtime: Option<std::time::SystemTime>,
    bom_tpl_checked: Option<Instant>,
    bom_tpl_err: Option<String>,
    bom_edits: pla_ui::bom_table::Edits,
    bom_edits_doc: String,
    bom_cell_buf: std::collections::HashMap<String, String>,
    /// B4 零件库: BOM 窗口标签(0 清单 / 1 零件库); 库内容 + 修改时间 / 上次检查(外部改了热生效);
    /// 库里搜索词; 正在编辑的条目(副本)及其原料号(None = 新建); 行"库"菜单里的搜索词。
    bom_tab: u8,
    /// D1 出图(BOM 窗口「出图」标签): 本文档的图设置(及所属文档)、选中的零件 key、进行中的任务、
    /// 选中图的编译报告 + 预览 PNG(uri, 字节; uri 带修改时间, 换图即换)、输入缓冲。
    draw_sheets: pla_ui::drawing::Sheets,
    draw_doc: String,
    draw_sel: Option<String>,
    draw_job: Option<DrawJob>,
    draw_report: Option<(String, pla_ui::drawing::Report)>,
    draw_png: Option<(String, egui::load::Bytes)>,
    /// D8: 预览显示的是上传的图纸(第几页, 共几页, 纸张 mm); None = 生成的图。
    draw_up_view: Option<(usize, usize, [f64; 2])>,
    draw_up_page: usize,
    /// D8: 上传线程(选文件 + 复制 + 转预览): 零件 key → 结果(None = 取消了选择)。
    draw_up_job: Option<(String, std::sync::mpsc::Receiver<Result<Option<pla_ui::drawing::Uploaded>, String>>)>,
    /// D8: 解锁要点两次(第一次点了的零件 key)。
    draw_unlock_confirm: Option<String>,
    /// D5: 大模型配置(设置 → LLM)、key 输入框(从不回显已存的 key)、连接测试。
    llm_cfg: pla_ui::llm::Config,
    llm_key_buf: String,
    llm_test: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    llm_test_msg: Option<(bool, String)>,
    /// 外发包: 供应商配置、当前选的供应商、加列输入框、删除确认、后台生成、上次生成的目录。
    suppliers: pla_ui::release::Suppliers,
    rel_sel: usize,
    rel_new_col: String,
    rel_del_confirm: bool,
    rel_job: Option<std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    rel_last: Option<std::path::PathBuf>,
    /// MCP 桥: 上次处理 / 上次写的状态(没变就不写, 5 s 心跳)。
    mcp_checked: Option<Instant>,
    mcp_last: String,
    mcp_written: Option<Instant>,
    /// D5: 出图助手对话(当前选中的图)、输入框、后台一轮。
    agent_chat: pla_ui::draw_agent::Chat,
    agent_input: String,
    agent_job: Option<(String, std::sync::mpsc::Receiver<Result<pla_ui::draw_agent::TurnOut, String>>)>,
    draw_buf: std::collections::HashMap<String, String>,
    /// 补尺寸: 等 dim.signature 回包的图 key(在 Plasticity 里选的面 → 这张图的 dims_add)。
    draw_sig_pending: Option<String>,
    /// BOM 表: 列宽(按列 id)/ 冻结前几列; 改了等松开鼠标再存(prefs "bom_cols")。
    bom_col_w: std::collections::HashMap<String, f32>,
    bom_freeze: usize,
    bom_cols_dirty: bool,
    /// 图纸预览窗口(贴生产窗口右边): 窗口状态 / 上次对齐时生产窗口的外框 / 当前图 uri(换图时缩放复位)/ 缩放 / 平移。
    draw_prev_win: FloatWin,
    draw_prev_anchor: Option<[f32; 4]>,
    draw_prev_uri: Option<String>,
    draw_zoom: f32,
    draw_pan: egui::Vec2,
    /// 批量出图: 待出的 (零件 key, 重新导出 STEP?, 标题栏); 本批 (总数, 失败的图名)。
    draw_queue: std::collections::VecDeque<(String, bool, std::collections::BTreeMap<String, String>)>,
    draw_batch: Option<(usize, Vec<String>)>,
    /// 本批的零件顺序(出完合订用); 合订 PDF 的后台线程。
    draw_batch_keys: Vec<String>,
    draw_book: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    /// D3 装配图: 当前 BOM 范围的明细(每帧随出图面板刷新; 导出 / 编译装配图时用)。
    draw_asm: std::collections::BTreeMap<String, Vec<pla_ui::drawing::AsmLine>>,
    /// D6 出图偏好(全局; 第一次用时读)。
    draw_prefs: Option<pla_ui::drawing::Prefs>,
    lib: pla_ui::parts_lib::Library,
    lib_mtime: Option<std::time::SystemTime>,
    lib_checked: Option<Instant>,
    lib_search: String,
    lib_import_path: String,
    lib_import_report: Option<String>,
    /// 按类型整理的待确认计划: (料号, 新类型, 勾选)
    lib_reclass: Option<Vec<(String, String, bool)>>,
    lib_edit: Option<pla_ui::parts_lib::Entry>,
    lib_edit_orig: Option<String>,
    lib_edit_aliases: String,
    lib_link_search: String,
    lib_del_confirm: Option<String>,
    show_settings: bool,
    /// F2: CDP 能力面板(点工具栏状态点开关)。
    show_caps: bool,
    /// R30: 笔记面板(独立浮窗: 不互斥、不自动关、位置/大小记忆、可调大小)。
    show_notes: bool,
    notes: pla_ui::notes_ui::NotesPanel,
    /// 笔记浮窗的位置记忆与拖动/调大小状态(见 `floating_window`)。
    notes_win: FloatWin,
    /// N3: 尺寸窗口同样是独立浮窗(用户 2026-11-15: 与笔记一样, 不受其他二级面板影响)。
    dim_win: FloatWin,
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
    /// N3 改尺寸: 上次查询的面(拓扑 id); 查询结果(`dim.face` 的 v 或错误); 输入的目标值; 应用中。
    dim_face: Option<String>,
    dim_info: Option<Result<serde_json::Value, String>>,
    dim_target: f64,
    /// 目标值输入框的文本(每帧解析; DragValue 要回车 / 失焦才提交, 右键应用时拿不到新值 —— 用户实测)。
    dim_text: String,
    /// 当前用的对面(dim.face 候选列表里的序号)。
    dim_cand: usize,
    /// N3b 改哪一侧: 0 = 移动选中面, 1 = 移动对面, 2 = 两侧各一半。
    dim_side: u8,
    /// N3c 对称: 动的面分在参考面两侧时 0 = 整体平移, 1 = 以参考面为对称面向两侧移动(距离记录的 side 存它)。
    dim_sym: u8,
    /// 对面是圆柱时的测量点: None = 自动 / 0 最近 / 1 轴心 / 2 最远(CYL_POINTS)。
    dim_cyl: Option<u8>,
    /// 参数化面板标签: 0 操作 / 1 记录; 记住标签所属的文档; 在"记录"时选择变了 → "操作"上的提示点。
    dim_tab: u8,
    dim_tab_doc: String,
    dim_tab_dot: bool,
    /// P2 记录列表: 组; 搜索词; 筛选(0 全部 / 1 尺寸 / 2 收藏); 按实体分组视图 + 其中收起的实体名;
    /// 组改名(id + 输入); 待确认删除的组 id。
    dims_groups: Vec<pla_ui::dims::Group>,
    /// P5: 参数(常量 / 变量 / 函数)与最近一次求值(参数 + 尺寸)。
    dims_params: Vec<pla_ui::params::Param>,
    dims_eval: pla_ui::params::Eval,
    /// P5: 参数表折叠; 参数改名中(id)+ 输入。
    params_open: bool,
    param_rename: Option<String>,
    param_rename_buf: String,
    rec_search: String,
    rec_filter: u8,
    rec_by_solid: bool,
    rec_solid_closed: std::collections::HashSet<String>,
    grp_rename: Option<String>,
    grp_rename_buf: String,
    grp_del_confirm: Option<String>,
    /// P3「操作」模式: 0 改尺寸 / 1 打孔。打孔表单: 规格(当前方案 hole_sizes 的下标, None = 自定义)、用途(0 过孔 / 1 螺纹底孔)、
    /// 类型(HOLE_TYPES 下标)、通孔; 数值输入(std_hole_vals 的 6 项); 等回包的孔参数。
    dim_mode: u8,
    hole_std: Option<usize>,
    hole_usage: u8,
    hole_type: u8,
    hole_through: bool,
    hole_vals: [String; 6],
    hole_pending: Option<serde_json::Value>,
    /// N4b 孔规格方案(hole_presets): 方案名列表、当前方案、其规格、文件修改时间 / 上次检查(热生效)、读错、改名输入。
    hole_presets: Vec<String>,
    hole_preset: String,
    hole_sizes: Vec<pla_ui::hole_presets::HoleSize>,
    hole_preset_mtime: Option<std::time::SystemTime>,
    hole_preset_checked: Option<Instant>,
    hole_preset_err: Option<String>,
    hole_preset_rename: Option<String>,
    /// P4: 进行中的组联动。
    link_run: Option<LinkRun>,
    /// P3: 手动记录的尺寸落在某个孔的面上 → 询问"挂到孔下?"(尺寸名, 孔 id, 孔名)。
    attach_prompt: Option<(String, String, String)>,
    /// 上次发出的高亮(面 + 对面 + 候选数), 没变就不重发。
    dim_hl_key: String,
    /// N3d: 本文档的尺寸记录(懒加载, 换文档重读)。
    dims_list: Vec<pla_ui::dims::DimRecord>,
    dims_loaded: bool,
    /// 当前对应哪条记录(载入的或刚记录的)及其面 id; 选择换成别的面即清除。
    dim_active: Option<String>,
    dim_active_ids: Vec<String>,
    /// 等回包: 记录签名 / 载入定位 / 改后刷新。
    dim_pending: Option<DimPending>,
    /// 载入后, 等 dim.face 回包时要切到的对面(拓扑 id)。
    dim_pending_opp: Option<String>,
    /// 最近一次应用(成功后刷新记录用)。
    dim_last_apply: Option<DimApplied>,
    /// 删除确认中的记录名。
    dim_del_confirm: Option<String>,
    /// 已记录列表的交互(同快照墙): 管理模式 + 勾选、改名、更新确认、批量删除确认。
    dims_manage: bool,
    dims_selected: std::collections::HashSet<String>,
    dims_rename: Option<String>,
    dims_rename_buf: String,
    dims_update_confirm: Option<String>,
    dims_batch_confirm: bool,
    /// 各条目数值框的文本(直接改 + Enter = 载入并应用)。
    dims_vals: std::collections::HashMap<String, String>,
    /// 上一帧有条目数值框获得焦点(此时右键归条目, 不触发上面的应用)。
    dims_row_focus: bool,
    /// 正在编辑数值的条目(右键确认用: 右键按下时文本框已失焦, 见 dim_saved_ui)。
    dims_edit_row: Option<String>,
    /// 右键确认扩展到 Plasticity 窗口: 已让垫片 arm / 上次看到的页面右键计数(−1 = 还没读到)。
    rc_armed: bool,
    rc_seen: i64,
    /// 缩略图缓存(名字 → PNG 字节)。
    dims_pngs: std::collections::HashMap<String, Vec<u8>>,
    /// 记录 / 更新后等视口截图回包的记录名。
    dims_thumb_pending: Option<String>,
    dim_busy: bool,
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
    /// 收藏的快照名(含 .plasticity), 与 .pla/snapshots/meta.json 同步。
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
    vp_update_target: Option<String>,
    /// R8-1: 视口状态面板(R34: 墙的界面状态并入 state_wall, 与材质状态共用)。
    vp_wall: state_wall::StateWall,
    vp_pending_state: Option<String>,
    vp_shot_path: Option<String>,
    /// R34: 材质状态面板 + 采集/更新流程(采集回包 → 缩略图 → 落盘)。
    show_matstate: bool,
    ms_wall: state_wall::StateWall,
    ms_pending_state: Option<String>,
    ms_shot_path: Option<String>,
    ms_update_target: Option<String>,
    /// C0 配置面板: 同材质状态(采集回包 → 截图 → 落盘)。
    show_config: bool,
    /// 配置窗口上次停留的标签(工具栏按钮重开时回到这里)。
    cfg_last: PanelId,
    /// 标签点击: 下一帧切换(本帧同一个视口已经画过, 不能再画一次)。
    cfg_pending_tab: Option<PanelId>,
    cfg_wall: state_wall::StateWall,
    cfg_pending_state: Option<String>,
    cfg_shot_path: Option<String>,
    cfg_update_target: Option<String>,
    /// R8-1: 选择集截图缓存 + 管理模式 + pending 保存流程。
    sel_pngs: std::collections::HashMap<String, std::sync::Arc<[u8]>>,
    sel_manage: bool,
    sel_selected: std::collections::HashSet<String>,
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
    /// 图片(纹理 URI + PNG 字节; 如尺寸记录的缩略图)。
    Image(String, Vec<u8>),
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
}

thread_local! {
    static INFO_HINT: std::cell::RefCell<Option<InfoHint>> = std::cell::RefCell::new(None);
    /// 三级面板实测内容高度(按标题), 隐藏渲染阶段测出, 显示前即调整好窗口高度。
    static INFO_HEIGHT: std::cell::RefCell<std::collections::HashMap<&'static str, f32>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// 悬停时登记三级面板(由根视口在面板旁渲染)。面板顶部对齐窗口顶部(不跟随悬停行), 行的位置 `_row` 不再参与定位。
fn hover_info(ui: &egui::Ui, _row: egui::Rect, hovered: bool, title: &'static str, lines: Vec<InfoLine>) {
    if !hovered {
        return;
    }
    let Some(inner) = ui.ctx().input(|i| i.viewport().inner_rect) else {
        return;
    };
    INFO_HINT.with(|h| {
        *h.borrow_mut() = Some(InfoHint { title, lines, panel: inner });
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
        ui.painter().circle_stroke(c, 5.5, egui::Stroke::new(1.0_f32, col));
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
                ui.painter().circle_stroke(c, 5.5, egui::Stroke::new(1.0_f32, col));
                ui.painter().text(c, egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(8.5), col);
                hover_info(ui, rect, resp.hovered(), t, lines);
            }
            add(ui)
        })
        .inner
}

// ---- N3: 尺寸窗口的原生风格控件(参照 Plasticity 命令面板, 如 PUSH FACE: 信息卡片 / 分段选择 / 上标签下数值框 / Esc Cancel · OK ↵) ----

/// 尺寸显示精度: 最多 3 位小数, 去掉末尾 0(用户 2026-11-15)。
fn fmt3(x: f64) -> String {
    let s = format!("{x:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// 信息卡片: 圆角浅一级底, 主文字白色粗体 + 副文字灰色(同 PUSH FACE 的 "Select target shell or face / to match")。
fn rz_info_card(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    egui::Frame::none()
        .fill(COL_CARD)
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin { left: 12.0, right: 10.0, top: 7.0, bottom: 7.0 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
            paint_text(ui.painter(), egui::pos2(rect.left(), rect.center().y), egui::Align2::LEFT_CENTER, title, 13.0, egui::Color32::WHITE, true);
            if !subtitle.is_empty() {
                ui.add(egui::Label::new(egui::RichText::new(subtitle).size(11.5).color(COL_MUTED)).wrap());
            }
        });
}

/// 小节标签(分段选择 / 数值框上方的灰色小字)。
fn rz_label(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(text).size(12.0).color(COL_TEXT));
}

/// 分段选择(同 Moving | Fixed | None): 整宽圆角底, 等分; 选中段 = 浅一级底 + 白字。返回是否改了。
fn rz_segmented(ui: &mut egui::Ui, id: &str, options: &[String], selected: &mut usize) -> bool {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, egui::Rounding::same(6.0), COL_FIELD);
    let n = options.len().max(1);
    let w = rect.width() / n as f32;
    let mut changed = false;
    for (i, label) in options.iter().enumerate() {
        let r = egui::Rect::from_min_size(egui::pos2(rect.left() + w * i as f32, rect.top()), egui::vec2(w, rect.height()));
        let resp = ui.interact(r, ui.id().with((id, i)), egui::Sense::click());
        let on = i == *selected;
        if on || resp.hovered() {
            let fill = if on { COL_STEPPER } else { lerp_color(COL_FIELD, egui::Color32::WHITE, 0.06) };
            ui.painter().rect_filled(r.shrink(2.0), egui::Rounding::same(5.0), fill);
        }
        let (shown, size, _) = fit_text(ui, label, w - 8.0, 12.5, 10.0);
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, shown, egui::FontId::proportional(size), if on { egui::Color32::WHITE } else { COL_MUTED });
        if resp.clicked() && !on {
            *selected = i;
            changed = true;
        }
    }
    changed
}

/// 尺寸窗口 "?" 的说明(三级面板)。
fn dim_help_lines() -> Vec<InfoLine> {
    vec![
        info_text(tr(
            "选一个面: 平面 = 厚度(到背对的平行面; 有多个时在\"对面候选\"里切换, 橙 = 当前, 蓝 = 其它), 圆柱 = 半径(孔 / 凸台, 轴线不动), 倒圆 = 圆角半径(保持相切)。",
            "One face: plane = thickness (to the opposite parallel face; switch among candidates, orange = current, blue = others), cylinder = radius (hole / boss, axis stays), fillet = fillet radius (stays tangent).",
        )),
        info_text(tr(
            "约定: 选多个面时,【第一个选的面 = 参考面】(绿, 固定不动), 之后 Shift 加选的都是要动的面(可多个: 孔的圆柱 + 倒角、凸台侧面…)→ 距离 / 孔位, 改值时整组平移。",
            "Convention: with several faces, the FIRST selected face is the REFERENCE (green, fixed); faces Shift-added after it move (several: hole + chamfer, boss sides…) → distance / hole position; the group is translated.",
        )),
        info_text(tr("移动: 选中面 / 对面 / 两侧各半(厚度)。", "Move: selected / opposite / both halves (thickness).")),
        info_text(tr(
            "对称: 要动的面分在参考面两侧时, 可选\"整体平移\"或\"对称(以参考面)\"—— 对称 = 与锚面同侧的往外 / 往里 Δ, 另一侧反向 Δ。",
            "Symmetric: when the moving faces lie on both sides of the reference, choose \"Translate all\" or \"Symmetric (about reference)\" — the anchor side moves by Δ, the other side by −Δ.",
        )),
        info_text(tr(
            "打孔(「操作」→ 打孔): 选一个平面(孔的入口), 再 Shift 加选草图点(曲线端点 / 圆 = 圆心)定孔位, 一点一孔, 不选点 = 面中点; 再加选别的实体 = 组合孔: 沉头在入口实体, 中间实体是过孔, 最后一个实体是底孔(盲孔深度从它的入口算)。规格来自「规格方案」(JSON, 可切换 / 改名 / 新建 / 编辑, 保存即生效)。打完每个孔在「记录」里生成 H00n, 下面挂半径 / 深度等子尺寸, 可直接改; 手动记录的尺寸若量的是孔的面, 会问要不要挂到孔下。",
            "Hole (Operate → Hole): select a plane (the entry), Shift-add sketch points (curve ends / circles = centres) for positions, one hole each, none = face midpoint; Shift-add more solids for a stacked hole: counterbore in the entry solid, clearance through the middle ones, tap hole in the last (blind depth from its entry). Sizes come from the size table (JSON: switch / rename / new / edit, applies on save). Each hole appears in Records as H00n with radius / depth dimensions you can edit; a manually saved dimension on a hole face offers to attach to it.",
        )),
        info_text(tr(
            "组联动: 「记录」里建组(＋组)、把尺寸移进去(⋯ → 移到组), 组行右侧填驱动值 + Enter / 右键 = 组里跟随(●)的尺寸逐条改成这个值; 点 ●/○ 切换跟随 / 独立。任何一条失败, 本次已改的全部撤回(每条是一个撤销步)。",
            "Group linking: in Records create a group (+Group), move dimensions in (⋯ → Move to group), type a driving value on the group row + Enter / right-click = every following (●) dimension is changed to it; click ●/○ to follow / go independent. If any one fails, the whole run is rolled back (each change is one undo step).",
        )),
        info_key(tr("右键 / Enter", "Right-click / Enter"), tr("应用(一步可撤销, 失败自动撤回)", "Apply (one undo step; rolled back on failure)")),
        info_key("Esc", tr("目标值复原", "Reset the target value")),
        info_text(tr(
            "记录: 在「操作」读出尺寸 → 切到「记录」点顶部 ＋ 保存当前尺寸(在「记录」里选择变了不跳标签, 「操作」上出提示点)。点条目 = 载入并选中这些面(面换了 id 也能找回), 并在 Plasticity 里建一个同名测量(厚度 / 距离; 条目改名测量跟着改); 直接改条目里的数值再 Enter = 应用。★ 收藏、笔 = 改名、↻ 用当前尺寸更新、管理 = 批量删除。",
            "Saved: read a dimension in Operate → in the Records tab click + Save current dimension at the top (selection changes do not switch tabs; a dot appears on Operate). Click an entry to load and select its faces (found even after face ids change) and create a Plasticity measurement with the same name (thickness / distance; renaming the entry renames it); edit its value and press Enter to apply. ★ favourite, pencil = rename, ↻ update from the current dimension, Manage = batch delete.",
        )),
    ]
}

/// 0xRRGGBB → Color32(高亮色块用)。
fn rgb_hex(c: u32) -> egui::Color32 {
    egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// 标签栏(与分段单选区分开, 用户 2026-11-15): 左对齐的文字标签 + 贯通的底线; 当前标签白色粗体 + 橙色下划线,
/// 其它灰色(悬停变亮, 手形光标)。dot = 在第几个标签文字右上画提示点。返回是否切换了。
fn rz_tabs(ui: &mut egui::Ui, labels: &[String], selected: &mut usize, dot: Option<usize>) -> bool {
    let h = 30.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::hover());
    let font = egui::FontId::proportional(13.0);
    let pad = 12.0;
    let mut x = rect.left();
    let mut changed = false;
    for (i, label) in labels.iter().enumerate() {
        let tw = ui.fonts(|f| f.layout_no_wrap(label.clone(), font.clone(), COL_TEXT).size().x);
        let r = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(tw + pad * 2.0, h));
        let resp = ui.interact(r, ui.id().with(("rz-tab", i)), egui::Sense::click());
        let on = i == *selected;
        if resp.hovered() && !on {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            ui.painter().rect_filled(r.shrink2(egui::vec2(2.0, 4.0)), egui::Rounding::same(5.0), lerp_color(COL_CARD, egui::Color32::WHITE, 0.05));
        }
        let col = if on { egui::Color32::WHITE } else if resp.hovered() { COL_TEXT } else { COL_MUTED };
        paint_text(ui.painter(), egui::pos2(r.left() + pad, r.center().y - 1.0), egui::Align2::LEFT_CENTER, label, 13.0, col, on);
        if on {
            ui.painter().rect_filled(
                egui::Rect::from_min_max(egui::pos2(r.left() + 6.0, rect.bottom() - 2.5), egui::pos2(r.right() - 6.0, rect.bottom())),
                egui::Rounding::same(1.0),
                COL_ACCENT,
            );
        }
        if dot == Some(i) {
            ui.painter().circle_filled(egui::pos2(r.left() + pad + tw + 4.0, r.top() + 8.0), 3.5, COL_ACCENT);
        }
        if resp.clicked() && !on {
            *selected = i;
            changed = true;
        }
        x = r.right();
    }
    // 贯通底线(当前标签的下划线压在上面)
    ui.painter().line_segment([egui::pos2(rect.left(), rect.bottom() - 0.5), egui::pos2(rect.right(), rect.bottom() - 0.5)], egui::Stroke::new(1.0_f32, lerp_color(COL_CARD, egui::Color32::WHITE, 0.15)));
    changed
}

/// 对面候选: ◀ [色块 + 距离 | …] ▶。色块 = Plasticity 里的高亮色(当前 = 橙, 其它 = 蓝);
/// round[i] = 该候选是圆柱面 → 色块画成同心圆(用户 2026-11-15)。返回是否换了。
fn rz_candidates(ui: &mut egui::Ui, values: &[String], round: &[bool], selected: &mut usize) -> bool {
    let n = values.len().max(1);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::hover());
    let aw = 24.0;
    let mut changed = false;
    // 两侧箭头
    for (r, step, glyph) in [
        (egui::Rect::from_min_size(rect.min, egui::vec2(aw, rect.height())), n - 1, "◀"),
        (egui::Rect::from_min_size(egui::pos2(rect.right() - aw, rect.top()), egui::vec2(aw, rect.height())), 1, "▶"),
    ] {
        let resp = ui.interact(r, ui.id().with(("dim-cand-arrow", glyph)), egui::Sense::click());
        let fill = if resp.hovered() { lerp_color(COL_STEPPER, egui::Color32::WHITE, 0.15) } else { COL_STEPPER };
        ui.painter().rect_filled(r, egui::Rounding::same(6.0), fill);
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(11.0), egui::Color32::WHITE);
        if resp.clicked() && values.len() > 1 {
            *selected = (*selected + step) % n;
            changed = true;
        }
    }
    // 中间分段
    let mid = egui::Rect::from_min_max(egui::pos2(rect.left() + aw + 4.0, rect.top()), egui::pos2(rect.right() - aw - 4.0, rect.bottom()));
    ui.painter().rect_filled(mid, egui::Rounding::same(6.0), COL_FIELD);
    let w = mid.width() / n as f32;
    for (i, label) in values.iter().enumerate() {
        let r = egui::Rect::from_min_size(egui::pos2(mid.left() + w * i as f32, mid.top()), egui::vec2(w, mid.height()));
        let resp = ui.interact(r, ui.id().with(("dim-cand", i)), egui::Sense::click());
        let on = i == *selected;
        if on || resp.hovered() {
            let fill = if on { COL_STEPPER } else { lerp_color(COL_FIELD, egui::Color32::WHITE, 0.06) };
            ui.painter().rect_filled(r.shrink(2.0), egui::Rounding::same(5.0), fill);
        }
        let swatch = rgb_hex(if on { pla_bridge::shim::HIGHLIGHT_PRIMARY } else { pla_bridge::shim::HIGHLIGHT_CANDIDATE });
        let font = egui::FontId::proportional(12.5);
        let tw = ui.fonts(|f| f.layout_no_wrap(label.clone(), font.clone(), COL_TEXT).size().x);
        let total = 10.0 + 5.0 + tw;
        let x0 = r.center().x - total / 2.0;
        if round.get(i).copied().unwrap_or(false) {
            // 圆柱: 外圈描边 + 实心小圆心(同心圆)
            let c = egui::pos2(x0 + 5.0, r.center().y);
            ui.painter().circle_stroke(c, 5.0, egui::Stroke::new(1.5_f32, swatch));
            ui.painter().circle_filled(c, 2.2, swatch);
        } else {
            ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(x0, r.center().y - 5.0), egui::vec2(10.0, 10.0)), egui::Rounding::same(2.0), swatch);
        }
        ui.painter().text(egui::pos2(x0 + 15.0, r.center().y), egui::Align2::LEFT_CENTER, label, font, if on { egui::Color32::WHITE } else { COL_MUTED });
        if resp.clicked() && !on {
            *selected = i;
            changed = true;
        }
    }
    changed
}

/// 圆角深色数值框(白字居中 + 单位); editable 时是输入框。返回 (Response 或 None, 回车提交)。
fn rz_field(ui: &mut egui::Ui, buf: &mut String, units: &str, editable: bool) -> bool {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::hover());
    let fill = if editable { COL_FIELD } else { lerp_color(COL_FIELD, COL_CARD, 0.6) };
    ui.painter().rect_filled(rect, egui::Rounding::same(6.0), fill);
    let mut enter = false;
    if editable {
        let r = ui.put(
            rect.shrink2(egui::vec2(8.0, 4.0)),
            egui::TextEdit::singleline(buf)
                .frame(false)
                .horizontal_align(egui::Align::Center)
                .font(egui::FontId::proportional(13.0))
                .text_color(egui::Color32::WHITE),
        );
        enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    } else {
        paint_text(ui.painter(), rect.center(), egui::Align2::CENTER_CENTER, buf, 13.0, COL_TEXT, false);
    }
    if !units.is_empty() {
        ui.painter().text(egui::pos2(rect.right() - 8.0, rect.center().y), egui::Align2::RIGHT_CENTER, units, egui::FontId::proportional(11.5), COL_MUTED);
    }
    enter
}

/// 键帽(底部 Esc / ↵ 提示)。
fn rz_keycap(painter: &egui::Painter, pos: egui::Pos2, anchor: egui::Align2, key: &str) -> egui::Rect {
    let font = egui::FontId::proportional(11.0);
    let w = painter.layout_no_wrap(key.to_string(), font.clone(), COL_TEXT).size().x + 10.0;
    let r = anchor.anchor_size(pos, egui::vec2(w, 18.0));
    painter.rect_filled(r, egui::Rounding::same(4.0), COL_FIELD);
    painter.text(r.center(), egui::Align2::CENTER_CENTER, key, font, COL_TEXT);
    r
}

/// 底部操作行: 左 "Esc Cancel", 右 "OK ↵"(同 Plasticity 命令面板)。返回 (取消, 确定)。
fn rz_footer(ui: &mut egui::Ui, ok_enabled: bool, busy: bool) -> (bool, bool) {
    ui.add_space(6.0);
    let (line, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(line, 0.0, COL_FIELD);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::hover());
    let p = ui.painter().clone();
    // 左: Esc Cancel
    let kc = rz_keycap(&p, egui::pos2(rect.left() + 2.0, rect.center().y), egui::Align2::LEFT_CENTER, "Esc");
    let ct = p.text(egui::pos2(kc.right() + 8.0, rect.center().y), egui::Align2::LEFT_CENTER, tr("取消", "Cancel"), egui::FontId::proportional(12.5), COL_TEXT);
    let cancel_r = kc.union(ct).expand(3.0);
    let cancel = ui.interact(cancel_r, ui.id().with("rz-cancel"), egui::Sense::click()).clicked();
    // 右: OK ↵
    // 内置字体没有 ↵ 字形(实测显示成方框)→ 键帽写 Enter
    let kr = rz_keycap(&p, egui::pos2(rect.right() - 2.0, rect.center().y), egui::Align2::RIGHT_CENTER, "Enter");
    let ok_col = if ok_enabled { egui::Color32::WHITE } else { COL_GRAY_DOT };
    let ok_txt = if busy { tr("修改中…", "Applying…") } else { "OK" };
    let ot = paint_text(&p, egui::pos2(kr.left() - 8.0, rect.center().y), egui::Align2::RIGHT_CENTER, ok_txt, 12.5, ok_col, ok_enabled);
    let ok_r = ot.union(kr).expand(3.0);
    let ok_resp = ui.interact(ok_r, ui.id().with("rz-ok"), egui::Sense::click());
    if ok_enabled && ok_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    (cancel, ok_enabled && ok_resp.clicked())
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
                w.fg_stroke = egui::Stroke::new(1.0_f32, egui::Color32::WHITE);
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

/// P2 记录"移到组"菜单的选择。
enum MoveChoice {
    /// 移到某组(None = 顶层)。
    To(Option<String>),
    /// 新建组并移入。
    NewGroup,
}

/// P2: 点 resp 弹出"移到组"菜单(各组 / 新建组并移入 / 移出组)。current = 条目当前所在组(不列它; 有组时给"移出组")。
fn move_menu(ui: &mut egui::Ui, resp: &egui::Response, key: &str, groups: &[pla_ui::dims::Group], current: Option<&str>) -> Option<MoveChoice> {
    let pid = ui.id().with(("move-menu", key));
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(pid));
    }
    let mut out = None;
    egui::popup::popup_below_widget(ui, pid, resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        ui.set_min_width(150.0);
        for g in groups.iter().filter(|g| Some(g.id.as_str()) != current) {
            if ui.button(&g.name).clicked() {
                out = Some(MoveChoice::To(Some(g.id.clone())));
            }
        }
        if ui.button(tr("＋ 新建组并移入", "+ New group and move in")).clicked() {
            out = Some(MoveChoice::NewGroup);
        }
        if current.is_some() && ui.button(tr("移出组(回到顶层)", "Remove from group (top level)")).clicked() {
            out = Some(MoveChoice::To(None));
        }
    });
    out
}

/// 参数化面板: 文档上次停留的标签(0 操作 / 1 记录), prefs "dim_tabs" = {doc_key: n}。
fn prefs_dim_tab(doc: &str) -> u8 {
    pla_ui::prefs::load_value("dim_tabs").and_then(|v| v[doc].as_u64()).map_or(0, |x| x.min(1) as u8)
}

fn prefs_save_dim_tab(doc: &str, tab: u8) {
    let mut v = pla_ui::prefs::load_value("dim_tabs").filter(|v| v.is_object()).unwrap_or_else(|| serde_json::json!({}));
    v[doc] = serde_json::json!(tab);
    if let Err(e) = pla_ui::prefs::save_value("dim_tabs", v) {
        pla_ui::diag::log(&format!("save dim_tabs failed: {e}"));
    }
}

/// 孔特征一行摘要: "M4 柱形沉头 · 通孔" / "Ø3 简单孔 · 深 5" / 组合孔加 "· 组合 3 件"。
fn hole_summary(spec: Option<&serde_json::Value>) -> String {
    let Some(s) = spec else { return String::new() };
    let size = s["std"].as_str().map(String::from).unwrap_or_else(|| format!("Ø{}", fmt3(s["d"].as_f64().unwrap_or(0.0))));
    let tap = if s["usage"] == "tap" { tr(" 底孔", " tap") } else { "" };
    let combo = s["stack"].as_array().filter(|a| !a.is_empty()).map(|a| t!(" · 组合 {} 件", " · stack of {}", a.len() + 1)).unwrap_or_default();
    let depth = match s["depth"].as_f64() {
        Some(d) => t!("深 {}", "depth {}", fmt3(d)),
        None => tr("通孔", "through").to_string(),
    };
    format!("{size}{tap} {} · {depth}{combo}", hole_type_label(s["type"].as_str().unwrap_or("")))
}

/// 规格 + 用途 → 表单数值 [d, 盲孔深(默认 2 × 孔径), 柱形沉头 D, 柱形沉头深, 锥形沉头 D, 组合孔底孔径(= 螺纹底孔)]。
/// 单实体: 用途决定 d(过孔 / 螺纹底孔); 组合孔固定 d = 过孔、底孔 = 螺纹底孔。
fn std_hole_vals(s: &pla_ui::hole_presets::HoleSize, usage: u8) -> [String; 6] {
    let d = if usage == 1 { s.tap } else { s.clearance };
    [fmt3(d), fmt3(d * 2.0), fmt3(s.cb_d), fmt3(s.cb_depth), fmt3(s.csk_d), fmt3(s.tap)]
}

/// N4 孔类型(与垫片 hole.make 的 type 同名; 下标 = 界面分段)。
const HOLE_TYPES: [&str; 3] = ["simple", "counterbore", "countersink"];

/// 孔类型的显示名。
fn hole_type_label(t: &str) -> &'static str {
    match t {
        "counterbore" => tr("柱形沉头", "Counterbore"),
        "countersink" => tr("锥形沉头", "Countersink"),
        _ => tr("简单孔", "Simple"),
    }
}

/// 圆柱对面的测量点(与垫片 dim.face 候选的 values 键同名; 下标 = 界面分段 / 记录 cyl)。
const CYL_POINTS: [&str; 3] = ["near", "axis", "far"];

/// 厚度候选的量值: 圆柱候选且选了测量点 → values[点]; 否则候选默认值(圆柱 = 自动点)。
fn cand_value(c: &serde_json::Value, cyl: Option<u8>) -> Option<f64> {
    match cyl {
        Some(i) if c["values"].is_object() => c["values"][CYL_POINTS[(i as usize).min(2)]].as_f64(),
        _ => c["value"].as_f64(),
    }
}

/// 改名图标: 斜放的笔(笔杆 + 笔尖), 约 12 pt 见方, 以 c 为中心。
fn paint_pencil(p: &egui::Painter, c: egui::Pos2, col: egui::Color32) {
    let d = egui::vec2(1.0, -1.0).normalized();
    let n = egui::vec2(d.y, -d.x);
    let tail = c + d * 5.0;
    let base = c - d * 2.5;
    p.line_segment([base, tail], egui::Stroke::new(2.6_f32, col));
    let tip = base - d * 3.0;
    p.add(egui::Shape::convex_polygon(vec![base + n * 1.3, base - n * 1.3, tip], col, egui::Stroke::NONE));
}

/// 渲染三级面板(由根视口在所有面板之后调用, 与 HUD 同理: 面板本帧写入的悬停才能本帧显示)。
fn show_info_popup(ctx: &egui::Context, h: InfoHint) {
    const W: f32 = 320.0;
    let key = h.title;
    let height = INFO_HEIGHT.with(|m| m.borrow().get(key).copied()).unwrap_or(120.0);
    let monitor = ctx.input(|i| i.viewport().monitor_size);
    let monitor_w = monitor.map_or(f32::MAX, |s| s.x);
    let x = if h.panel.right() + 8.0 + W <= monitor_w {
        h.panel.right() + 8.0
    } else {
        h.panel.left() - 8.0 - W
    };
    // 顶部与面板标题栏对齐(用户 2026-11-15: 跟着行走会被挤出屏幕底部看不见); 仍放不下再往上收
    let monitor_h = monitor.map_or(f32::MAX, |s| s.y);
    let y = h.panel.top().min(monitor_h - height).max(0.0);
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
                        InfoLine::Image(uri, bytes) => {
                            ui.add(egui::Image::from_bytes(uri.clone(), bytes.clone()).max_width(W - 24.0).rounding(4.0));
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

/// R34: 用同一套存档墙的两个面板。
#[derive(Clone, Copy, PartialEq, Eq)]
enum StatePanel {
    Vp,
    Mat,
    /// C0 配置: 可见性 + 材质指派。
    Cfg,
}

/// R8b: 二级面板标识(互斥用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelId {
    Snapshots,
    Entities,
    VpState,
    MatState,
    Config,
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

/// D1 出图任务: 导出 STEP(等 CDP 回包 FILE_EXPORT_STEP)→ 编译(后台线程跑 tools/drawing/draw.ps1)。
struct DrawJob {
    key: String,
    stage: DrawStage,
    /// 编译线程的结果(stage = Compile 时)。
    rx: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    t0: Instant,
    /// 本次用的标题栏(BOM 值 + 本图覆盖), 导出完编译时用。
    title: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DrawStage {
    Export,
    Compile,
}

/// D1: 出图面板里可编辑的标题栏字段(id 与 tools/drawing/standards/GB.json 的 field 一致)。
const DRAW_TITLE_FIELDS: &[(&str, &str, &str)] = &[
    ("pn", "料号", "Part no."),
    ("name", "名称", "Name"),
    ("material", "材质", "Material"),
    ("finish", "表处", "Finish"),
    ("hardness", "硬度", "Hardness"),
    ("project", "项目名称", "Project"),
    ("designer", "设计", "Designer"),
    ("reviewer", "审核", "Reviewer"),
    ("version", "版本", "Version"),
];

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
                    if enter || right_click_confirm(ui) {
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
    let st = egui::Stroke::new(1.1_f32, dcol);
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
    let st = egui::Stroke::new(1.3_f32, col);
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
            ui.painter().line_segment([p(7.0, 6.0), p(7.0, 10.0)], egui::Stroke::new(1.0_f32, col));
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

/// 右键 = 确认(与 Plasticity 本体一致: 命令里右键 = 确定)。所有面板的"确认类"动作都接它:
/// 确认弹层(删除 / 更新)、改名输入、尺寸面板的应用。面板是各自的视口, 只看本视口内的右键。
/// 有自己右键菜单的地方(笔记、工具栏按钮)不接。
fn right_click_confirm(ui: &egui::Ui) -> bool {
    // 调用本身 = "这里正等着确认"(各处都只在等待确认时才调用) → 下一帧让 Plasticity 窗口里的右键也算数
    RC_WANT.set(true);
    ui.input(|i| i.pointer.secondary_clicked()) || RC_EXT.get()
}

thread_local! {
    /// 本帧有地方在等右键确认(right_click_confirm 被调用过)。
    static RC_WANT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// 本帧收到了 Plasticity 窗口里的右键(垫片计数增加)。
    static RC_EXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// R8b: 通用删除确认弹层 — 覆盖在面板视口顶部居中(egui::Area + 新 layer)。
/// 不占内容流空间(行内确认条会被 auto_shrink 滚动区推出可视区, 实测不可见)。
/// 返回 Some(true)=确认, Some(false)=取消, None=仍在展示。
fn confirm_popup(
    ctx: &egui::Context,
    id: &str,
    text: &str,
) -> Option<bool> {
    confirm_popup_with(ctx, id, text, tr("确认删除", "Delete"), COL_RED)
}

/// R35: 带确认按钮文字与强调色的确认弹层(删除 = 红色 "Delete"; 更新 = 强调色 "Update")。
fn confirm_popup_with(
    ctx: &egui::Context,
    id: &str,
    text: &str,
    confirm_label: &str,
    accent: egui::Color32,
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
                .stroke(egui::Stroke::new(1.0_f32, accent))
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    ui.set_min_width(200.0);
                    ui.label(
                        egui::RichText::new(text).small().color(accent),
                    );
                    ui.horizontal(|ui| {
                        if ui.button(confirm_label).clicked() {
                            result = Some(true);
                        }
                        if ui.button(tr("取消", "Cancel")).clicked() {
                            result = Some(false);
                        }
                    });
                    // 右键(面板内任意处)= 确认; Esc = 取消
                    if result.is_none() && right_click_confirm(ui) {
                        result = Some(true);
                    }
                    if result.is_none() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        result = Some(false);
                    }
                    ui.label(egui::RichText::new(tr("右键 = 确认 · Esc = 取消", "Right-click = confirm · Esc = cancel")).small().color(COL_MUTED));
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
            show_bom: false,
            bom_win: FloatWin::load(BOM_RECT_KEY),
            bom_scope: None,
            bom_result: None,
            bom_result_scope: None,
            bom_busy: false,
            bom_pending_scope: None,
            bom_batch: None,
            bom_tpl_list: Vec::new(),
            bom_tpl_name: String::new(),
            bom_tpl: pla_ui::bom_table::builtin(),
            bom_tpl_mtime: None,
            bom_tpl_checked: None,
            bom_tpl_err: None,
            bom_edits: Default::default(),
            bom_edits_doc: String::new(),
            bom_cell_buf: Default::default(),
            bom_tab: 0,
            draw_sheets: Default::default(),
            draw_doc: String::new(),
            draw_sel: None,
            draw_job: None,
            draw_report: None,
            draw_png: None,
            draw_up_view: None,
            draw_up_page: 0,
            draw_up_job: None,
            draw_unlock_confirm: None,
            llm_cfg: pla_ui::llm::Config::load(),
            llm_key_buf: String::new(),
            llm_test: None,
            llm_test_msg: None,
            suppliers: pla_ui::release::Suppliers::load(),
            rel_sel: 0,
            rel_new_col: String::new(),
            rel_del_confirm: false,
            rel_job: None,
            rel_last: None,
            mcp_checked: None,
            mcp_last: String::new(),
            mcp_written: None,
            agent_chat: Default::default(),
            agent_input: String::new(),
            agent_job: None,
            draw_buf: Default::default(),
            draw_sig_pending: None,
            bom_col_w: pla_ui::prefs::load_value(BOM_COLS_KEY)
                .and_then(|v| serde_json::from_value(v["w"].clone()).ok())
                .unwrap_or_default(),
            bom_freeze: pla_ui::prefs::load_value(BOM_COLS_KEY).and_then(|v| v["freeze"].as_u64()).unwrap_or(0) as usize,
            bom_cols_dirty: false,
            draw_prev_win: FloatWin::load(DRAW_PREV_RECT_KEY),
            draw_prev_anchor: None,
            draw_prev_uri: None,
            draw_zoom: 1.0,
            draw_pan: egui::Vec2::ZERO,
            draw_queue: Default::default(),
            draw_batch: None,
            draw_batch_keys: Vec::new(),
            draw_book: None,
            draw_asm: Default::default(),
            draw_prefs: None,
            lib: Default::default(),
            lib_mtime: None,
            lib_checked: None,
            lib_search: String::new(),
            lib_import_path: String::new(),
            lib_import_report: None,
            lib_reclass: None,
            lib_edit: None,
            lib_edit_orig: None,
            lib_edit_aliases: String::new(),
            lib_link_search: String::new(),
            lib_del_confirm: None,
            show_settings: false,
            show_caps: false,
            show_notes: false,
            notes: pla_ui::notes_ui::NotesPanel::new(),
            notes_win: FloatWin::load(NOTES_RECT_KEY),
            dim_win: FloatWin::load(DIM_RECT_KEY),
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
            dim_face: None,
            dim_info: None,
            dim_target: 0.0,
            dim_text: String::new(),
            dim_cand: 0,
            dim_side: 0,
            dim_sym: 0,
            dim_cyl: None,
            dim_tab: 0,
            dim_tab_doc: String::new(),
            dim_tab_dot: false,
            dims_groups: Vec::new(),
            dims_params: Vec::new(),
            dims_eval: Default::default(),
            params_open: true,
            param_rename: None,
            param_rename_buf: String::new(),
            rec_search: String::new(),
            rec_filter: 0,
            rec_by_solid: false,
            rec_solid_closed: Default::default(),
            grp_rename: None,
            grp_rename_buf: String::new(),
            grp_del_confirm: None,
            dim_mode: 0,
            hole_std: None,
            hole_usage: 0,
            hole_type: 0,
            hole_through: true,
            hole_vals: Default::default(),
            hole_pending: None,
            hole_presets: Vec::new(),
            hole_preset: pla_ui::prefs::load_value("hole_preset").and_then(|v| v.as_str().map(String::from)).unwrap_or_default(),
            hole_sizes: Vec::new(),
            hole_preset_mtime: None,
            hole_preset_checked: None,
            hole_preset_err: None,
            hole_preset_rename: None,
            attach_prompt: None,
            link_run: None,
            dim_hl_key: String::new(),
            dims_list: Vec::new(),
            dims_loaded: false,
            dim_active: None,
            dim_active_ids: Vec::new(),
            dim_pending: None,
            dim_pending_opp: None,
            dim_last_apply: None,
            dim_del_confirm: None,
            dims_manage: false,
            dims_selected: Default::default(),
            dims_rename: None,
            dims_rename_buf: String::new(),
            dims_update_confirm: None,
            dims_batch_confirm: false,
            dims_vals: Default::default(),
            dims_row_focus: false,
            dims_edit_row: None,
            rc_armed: false,
            rc_seen: -1,
            dims_pngs: Default::default(),
            dims_thumb_pending: None,
            dim_busy: false,
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
            vp_update_target: None,
            vp_wall: state_wall::StateWall::default(),
            vp_pending_state: None,
            vp_shot_path: None,
            show_matstate: false,
            ms_wall: state_wall::StateWall::default(),
            ms_pending_state: None,
            ms_shot_path: None,
            ms_update_target: None,
            show_config: false,
            cfg_last: PanelId::Config,
            cfg_pending_tab: None,
            cfg_wall: state_wall::StateWall::default(),
            cfg_pending_state: None,
            cfg_shot_path: None,
            cfg_update_target: None,
            sel_pngs: std::collections::HashMap::new(),
            sel_manage: false,
            sel_selected: std::collections::HashSet::new(),
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
                    // Plasticity 窗口里的右键确认(只在 arm 时计数): 增加 = 本帧算一次右键
                    if snap.rclick >= 0 {
                        if self.rc_seen >= 0 && snap.rclick > self.rc_seen && self.rc_armed {
                            RC_EXT.set(true);
                        } else if snap.rclick < self.rc_seen {
                            // 计数变小 = 垫片重新注入(arm 状态丢了)→ 下一帧按需重新 arm
                            self.rc_armed = false;
                        }
                        self.rc_seen = snap.rclick;
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
                // N3: 面的可改尺寸 → 面板显示, 目标值初始化为当前值
                pla_bridge::cdp::CdpEvent::CallResult { name, result }
                    if name == pla_bridge::shim::api::DIM_FACE || name == pla_bridge::shim::api::DIM_DISTANCE || name == pla_bridge::shim::api::DIM_FACES =>
                {
                    if let Ok(v) = &result {
                        if let Some(x) = v["value"].as_f64() {
                            self.dim_target = x;
                            self.dim_text = fmt3(x);
                        }
                        // 当前对面 = 回包里选定的那个(默认第一个候选); N3d 载入记录时 = 记录里的对面
                        let want = self.dim_pending_opp.take();
                        if want.is_none() {
                            // 新选择(不是载入记录): 圆柱测量点回到自动
                            self.dim_cyl = None;
                        }
                        let opp = want.as_deref().unwrap_or(v["opposite"].as_str().unwrap_or(""));
                        let cands = v["candidates"].as_array().cloned().unwrap_or_default();
                        self.dim_cand = cands.iter().position(|c| c["id"].as_str() == Some(opp)).unwrap_or(0);
                        if let Some(x) = cands.get(self.dim_cand).and_then(|c| cand_value(c, self.dim_cyl)) {
                            self.dim_text = fmt3(x);
                        }
                    }
                    self.dim_info = Some(result);
                }
                // 出图补尺寸: 选中面的签名 → 这张图的 dims_add
                pla_bridge::cdp::CdpEvent::CallResult { name, result } if name == pla_bridge::shim::api::DIM_SIGNATURE && self.draw_sig_pending.is_some() => {
                    let key = self.draw_sig_pending.take().unwrap_or_default();
                    match result {
                        Ok(v) => {
                            let sigs = v["faces"].as_array().cloned().unwrap_or_default();
                            if let Some(e) = sigs.iter().find_map(|s| s["err"].as_str().map(String::from)) {
                                self.set_notice((true, t!("补尺寸失败: {e}", "Add dimension failed: {e}")));
                            } else if let Some(mut s) = self.draw_sheets.sheets.iter().find(|s| s.key == key).cloned() {
                                let name = pla_ui::drawing::next_added_name(&s);
                                s.dims_add.push(pla_ui::drawing::AddedDim { name: name.clone(), faces: sigs });
                                s.edits += 1;
                                self.draw_sheets.upsert(s);
                                self.draw_save();
                                self.set_notice((false, t!("已补尺寸 {name}, 点「出图」生效", "Added dimension {name}; click Draw to apply")));
                            }
                        }
                        Err(e) => self.set_notice((true, t!("补尺寸失败: {e}", "Add dimension failed: {e}"))),
                    }
                }
                // N3d: 面签名回包(记录 / 改后刷新之外的签名请求不会发)
                pla_bridge::cdp::CdpEvent::CallResult { name, result } if name == pla_bridge::shim::api::DIM_SIGNATURE => {
                    match (self.dim_pending.take(), result) {
                        (Some(DimPending::Save(mut rec, n, update)), Ok(v)) => {
                            let sigs = v["faces"].as_array().cloned().unwrap_or_default();
                            if let Some(e) = sigs.iter().find_map(|s| s["err"].as_str().map(String::from)) {
                                self.set_notice((true, t!("记录失败: {e}", "Save failed: {e}")));
                            } else if sigs.len() < n {
                                self.set_notice((true, tr("记录失败: 签名不全", "Save failed: incomplete signatures").into()));
                            } else {
                                rec.faces = sigs[..n].to_vec();
                                rec.opposite = sigs.get(n).cloned();
                                let ids: Vec<String> = rec.faces.iter().filter_map(|s| s["id"].as_str().map(String::from)).collect();
                                let saved = if update {
                                    // 更新: 保留名字 / 收藏 / 创建时间, 其余换成当前尺寸
                                    if let Some(old) = self.dims_list.iter().find(|r| r.name == rec.name) {
                                        rec.fav = old.fav;
                                        rec.created = old.created;
                                        // 层级身份也保留(id / 所在组 / 来源)
                                        rec.id = old.id.clone();
                                        rec.parent = old.parent.clone();
                                        rec.origin = old.origin.clone();
                                        rec.expr = old.expr.clone();
                                    }
                                    pla_ui::dims::update(&self.doc_key, &rec).map(|_| rec.name.clone())
                                } else {
                                    pla_ui::dims::add(&self.doc_key, rec)
                                };
                                match saved {
                                    Ok(name) => {
                                        self.dims_reload();
                                        // P3 自动归属: 新记录的面属于某个孔 → 询问挂到孔下
                                        if !update {
                                            if let Some((hid, hn)) = self.hole_for(&name) {
                                                self.attach_prompt = Some((name.clone(), hid, hn));
                                            }
                                        }
                                        self.dims_vals.remove(&name);
                                        self.set_notice((false, if update { t!("已更新尺寸 {name}", "Dimension {name} updated") } else { t!("已记录尺寸 {name}", "Dimension saved as {name}") }));
                                        // 缩略图: 当前视口(带高亮)截一张, 悬停详情里显示
                                        self.cdp.send(pla_bridge::cdp::CdpCmd::Call {
                                            name: pla_bridge::shim::api::CAM_THUMBNAIL,
                                            args: serde_json::json!({ "maxWidth": 480 }),
                                        });
                                        self.dims_thumb_pending = Some(name.clone());
                                        self.dim_active = Some(name);
                                        self.dim_active_ids = ids;
                                    }
                                    Err(e) => self.set_notice((true, t!("记录失败: {e}", "Save failed: {e}"))),
                                }
                            }
                        }
                        (Some(_), Err(e)) => self.set_notice((true, t!("记录失败: {e}", "Save failed: {e}"))),
                        (other, _) => self.dim_pending = other,
                    }
                }
                // N3d: 记录 / 更新后截的视口缩略图 → `<models>/<名字>.png`
                pla_bridge::cdp::CdpEvent::CallResult { name, result }
                    if name == pla_bridge::shim::api::CAM_THUMBNAIL && self.dims_thumb_pending.is_some() =>
                {
                    let nm = self.dims_thumb_pending.take().unwrap_or_default();
                    if let Ok(v) = result {
                        if let Some(bytes) = v["data"].as_str().and_then(|b| pla_bridge::cdp::base64_decode(b).ok()) {
                            if std::fs::write(pla_ui::dims::png_path(&self.doc_key, &nm), &bytes).is_ok() {
                                self.dims_pngs.insert(nm, bytes);
                            }
                        }
                    }
                }
                // N3d: 一条记录的定位 + 尺寸回包 → 载入(在 Plasticity 里选中 + 建同名本体测量)/ 条目里改值(直接应用)
                pla_bridge::cdp::CdpEvent::CallResult { name, result } if name == pla_bridge::shim::api::DIM_RECORD => {
                    let pending = self.dim_pending.take();
                    let (rec, target) = match pending {
                        Some(DimPending::Load(r)) => (r, None),
                        Some(DimPending::Apply(r, t)) => (r, Some(t)),
                        other => {
                            self.dim_pending = other;
                            continue;
                        }
                    };
                    match result {
                        Err(e) => {
                            let what = if target.is_some() { tr("修改", "Changing") } else { tr("载入", "Loading") };
                            self.set_notice((true, t!("{what}尺寸 {} 失败: {e}", "{what} dimension {} failed: {e}", rec.name)));
                            self.link_fail(&format!("{}: {e}", rec.name));
                        }
                        Ok(v) => {
                            let ids: Vec<String> = v["ids"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
                            let info = v["info"].clone();
                            let worst = v["worst"].as_f64().unwrap_or(0.0);
                            self.dim_side = rec.side;
                            self.dim_sym = rec.sym;
                            self.dim_cyl = rec.cyl;
                            self.dim_active = Some(rec.name.clone());
                            self.dim_active_ids = ids.clone();
                            // 当前对面 = 记录里的对面(找不到就用默认对面, 如实提示)
                            let opp = v["opposite"].as_str().map(String::from);
                            let cands = info["candidates"].as_array().cloned().unwrap_or_default();
                            self.dim_cand = opp.as_deref().and_then(|o| cands.iter().position(|c| c["id"].as_str() == Some(o))).unwrap_or(0);
                            self.dim_measure_for(&rec.name, &info);
                            let warn = if v["oppositeLost"] == true {
                                Some(tr("记录的对面找不到了, 用的是默认对面", "the saved opposite face is gone; using the default one").to_string())
                            } else if worst > 1.0 {
                                // 偏差 > 1 个文档单位: 模型在别处被改过, 定位可能不准 —— 如实提示
                                Some(t!("定位偏差 {} (模型改动较大, 请核对高亮的面)", "match offset {} (model changed; check the highlighted faces)", fmt3(worst)))
                            } else {
                                None
                            };
                            match target {
                                Some(t) if !self.dim_busy => {
                                    self.dim_apply(&info, &ids, t);
                                    if let Some(w) = warn {
                                        self.set_notice((true, t!("正在修改 {}: {w}", "Changing {}: {w}", rec.name)));
                                    }
                                    // 读到的尺寸没法应用(dim_apply 没发命令)→ 联动不能停在半路
                                    if !self.dim_busy {
                                        self.link_fail(&t!("{} 读到的尺寸无法应用", "{}: the dimension read cannot be applied", rec.name));
                                    }
                                }
                                Some(_) => {
                                    self.set_notice((true, tr("上一次修改还没完成, 稍后再改", "The previous change is still running; try again shortly").into()));
                                    self.link_fail(tr("上一次修改还没完成", "the previous change is still running"));
                                }
                                None => {
                                    let entries: Vec<pla_bridge::cdp::ApplyEntry> = ids
                                        .iter()
                                        .map(|id| pla_bridge::cdp::ApplyEntry { kind: "face".into(), id: id.clone(), key: None, camera: None })
                                        .collect();
                                    self.send_apply(entries);
                                    self.dim_pending_opp = opp;
                                    match warn {
                                        Some(w) => self.set_notice((true, t!("已载入 {}, 但{w}", "Loaded {}, but {w}", rec.name))),
                                        None => self.set_notice((false, t!("已载入尺寸 {}", "Loaded dimension {}", rec.name))),
                                    }
                                }
                            }
                        }
                    }
                }
                // B1/B2 BOM 扫描回包: 批量模式 → 导出这一份再扫下一个配置; 否则显示
                pla_bridge::cdp::CdpEvent::CallResult { name, result } if name == pla_bridge::shim::api::BOM_SCAN => {
                    self.bom_busy = false;
                    // 增量更新: 改过几何的零件(key 变了、实体还是那个)→ BOM 填写和出图设置跟过去
                    if let Ok(v) = &result {
                        self.identity_migrate(v);
                    }
                    let scope = self.bom_pending_scope.take().flatten();
                    if let Some((mut queue, mut done)) = self.bom_batch.take() {
                        let label = Self::bom_scope_label(&scope);
                        match &result {
                            Ok(v) => match self.bom_export(&label, &v.clone()) {
                                Ok(_) => done.push(label),
                                Err(e) => self.set_notice((true, t!("{label} 导出失败: {e}", "{label} export failed: {e}"))),
                            },
                            Err(e) => self.set_notice((true, t!("{label} 扫描失败: {e}", "{label} scan failed: {e}"))),
                        }
                        self.bom_result = Some(result);
                        self.bom_result_scope = scope;
                        match queue.pop_front() {
                            Some(next) => {
                                self.bom_batch = Some((queue, done));
                                self.bom_scan(Some(next));
                            }
                            None => self.set_notice((false, t!("已导出 {} 份 BOM(打开文件夹查看)", "{} BOM file(s) exported (open the folder)", done.len()))),
                        }
                    } else {
                        self.bom_result = Some(result);
                        self.bom_result_scope = scope;
                    }
                }
                // D1 出图: STEP 导出回包 → 记下体积 / 面积 → 后台编译
                pla_bridge::cdp::CdpEvent::CallResult { name, result } if name == pla_bridge::shim::api::FILE_EXPORT_STEP => self.draw_exported(result),
                // C0 配置采集回包 → 截图 → 落盘(同材质状态的串行流程)
                pla_bridge::cdp::CdpEvent::CallResult { name, result } if name == pla_bridge::shim::api::CFG_SAVE => match result {
                    Ok(v) => {
                        self.cfg_pending_state = Some(v.to_string());
                        let path = self.cfg_shot_path.clone().unwrap_or_else(|| pla_ui::configs::STORE.dir(&self.doc_key).join("__shot_tmp.png").display().to_string());
                        self.cdp.send(pla_bridge::cdp::CdpCmd::CaptureViewport(path));
                    }
                    Err(e) => {
                        self.cfg_update_target = None;
                        self.set_notice((true, t!("配置采集失败: {e}", "Capturing the configuration failed: {e}")));
                    }
                },
                // C0 配置恢复回包: 如实报缺失 / 失败
                pla_bridge::cdp::CdpEvent::CallResult { name, result } if name == pla_bridge::shim::api::CFG_RESTORE => match result {
                    Ok(v) => {
                        let n = |k: &str| v[k].as_array().map_or(0, |a| a.len());
                        let (missing, failed) = (n("missing"), n("failed"));
                        if failed > 0 {
                            let first = v["failed"][0].as_str().unwrap_or("").to_string();
                            self.set_notice((true, t!("配置部分恢复失败({failed} 项): {first}", "Configuration partly failed ({failed}): {first}")));
                        } else if missing > 0 {
                            self.set_notice((true, t!("配置已应用, {missing} 项在当前模型里找不到(已删除的物体 / 材质)", "Configuration applied; {missing} item(s) not found in the current model (deleted objects / materials)")));
                        } else {
                            self.set_notice((false, tr("配置已应用", "Configuration applied").into()));
                        }
                    }
                    Err(e) => self.set_notice((true, t!("应用配置失败: {e}", "Applying the configuration failed: {e}"))),
                },
                // P3 打孔回包 → 「记录」里生成孔特征 H00n + 自动子尺寸(半径 / 深度 / 沉头半径 / 沉头深度)
                pla_bridge::cdp::CdpEvent::CallResult { name, result } if name == pla_bridge::shim::api::HOLE_MAKE => {
                    self.dim_busy = false;
                    self.dim_face = None;
                    self.dim_info = None;
                    let spec = self.hole_pending.take().unwrap_or_default();
                    match result {
                        Err(e) => self.set_notice((true, t!("打孔失败(已撤回): {e}", "Hole failed (rolled back): {e}"))),
                        Ok(v) => {
                            let units = v["units"].as_str().unwrap_or("").to_string();
                            let rec = |mode: &str, kind: &str, faces: Vec<serde_json::Value>, value: f64| pla_ui::dims::DimRecord {
                                name: String::new(), mode: mode.into(), kind: kind.into(), faces, opposite: None, side: 0, sym: 0, cyl: None,
                                value, units: units.clone(), created: 0, fav: false, id: String::new(), parent: None, origin: "auto".into(), follow: false, expr: None,
                            };
                            let num = |k: &str| spec[k].as_f64().unwrap_or(0.0);
                            let depth = v["depth"].as_f64().unwrap_or(0.0);
                            let mut made: Vec<String> = Vec::new();
                            let mut errs: Vec<String> = Vec::new();
                            // 每个孔位一个孔特征 H00n
                            for h in v["holes"].as_array().cloned().unwrap_or_default() {
                                let f = &h["faces"];
                                let mut kids: Vec<(String, pla_ui::dims::DimRecord)> = Vec::new();
                                if f["wall"].is_object() {
                                    kids.push((tr("半径", "R").into(), rec("face", "radius", vec![f["wall"].clone()], num("d") / 2.0)));
                                }
                                if f["tapWall"].is_object() {
                                    kids.push((tr("底孔半径", "TapR").into(), rec("face", "radius", vec![f["tapWall"].clone()], num("tapD") / 2.0)));
                                }
                                // 深度: 单实体 = 入口 → 孔底; 组合孔 = 最后一件的入口 → 孔底
                                let top = if f["tapEntry"].is_object() { &f["tapEntry"] } else { &f["entry"] };
                                if top.is_object() && f["bottom"].is_object() {
                                    kids.push((tr("深度", "Depth").into(), rec("distance", "distance", vec![top.clone(), f["bottom"].clone()], depth)));
                                }
                                if f["cbWall"].is_object() {
                                    kids.push((tr("沉头半径", "BoreR").into(), rec("face", "radius", vec![f["cbWall"].clone()], num("cbD") / 2.0)));
                                }
                                if f["entry"].is_object() && f["cbFloor"].is_object() {
                                    kids.push((tr("沉头深度", "BoreDepth").into(), rec("distance", "distance", vec![f["entry"].clone(), f["cbFloor"].clone()], num("cbDepth"))));
                                }
                                let mut s = spec.clone();
                                s["at"] = h["at"].clone();
                                s["normal"] = v["normal"].clone();
                                s["solidName"] = f["wall"]["solidName"].clone();
                                match pla_ui::dims::add_feature(&self.doc_key, "hole", s, kids) {
                                    Ok((_, hn)) => made.push(hn),
                                    Err(e) => errs.push(e),
                                }
                            }
                            self.dims_reload();
                            match (made.len(), errs.first()) {
                                (_, Some(e)) => self.set_notice((true, t!("孔已打, 但记录失败: {e}", "Holes made, but saving records failed: {e}"))),
                                (0, None) => self.set_notice((true, tr("打孔回包里没有孔(模型可能已改, 请核对)", "The reply listed no holes (check the model)").into())),
                                (1, None) => self.set_notice((false, t!("已打孔 {}(在「记录」标签, 子尺寸可直接改)", "Hole {} made (see Records; its dimensions can be edited)", made[0]))),
                                (n, None) => self.set_notice((false, t!("已打 {n} 个孔 {}…{}(在「记录」标签)", "{n} holes made {}…{} (see Records)", made[0], made[n - 1]))),
                            }
                        }
                    }
                }
                // N3e: 本体测量回包: 成功只记日志(大纲里能看到); 失败如实提示(尺寸记录本身不受影响)
                pla_bridge::cdp::CdpEvent::CallResult { name, result } if name == pla_bridge::shim::api::DIM_MEASURE => match result {
                    Ok(v) => pla_ui::diag::log(&format!("dim.measure ok: {v}")),
                    Err(e) => self.set_notice((true, t!("本体测量没建成(尺寸记录不受影响): {e}", "Plasticity measurement not created (the record is fine): {e}"))),
                },
                // N3d: 改后刷新: 按预测签名定位, 写回当前签名与新值
                pla_bridge::cdp::CdpEvent::CallResult { name, result } if name == pla_bridge::shim::api::GEO_RESOLVE => {
                    let pending = self.dim_pending.take();
                    let res = match &result {
                        Ok(v) => v["faces"].as_array().cloned().unwrap_or_default(),
                        Err(_) => Vec::new(),
                    };
                    let ids: Vec<Option<String>> = res.iter().map(|r| r["id"].as_str().map(String::from)).collect();
                    match (pending, &result) {
                        (Some(DimPending::Refresh(mut rec)), Ok(_)) => {
                            let n = rec.faces.len();
                            let sigs: Vec<serde_json::Value> = res.iter().map(|r| r["sig"].clone()).collect();
                            if sigs.len() >= n && sigs.iter().take(n).all(|s| s.is_object()) {
                                rec.faces = sigs[..n].to_vec();
                                if let Some(o) = sigs.get(n).filter(|s| s.is_object()) {
                                    rec.opposite = Some(o.clone());
                                }
                                self.dim_active_ids = ids.iter().take(n).flatten().cloned().collect();
                                if let Err(e) = pla_ui::dims::update(&self.doc_key, &rec) {
                                    pla_ui::diag::log(&format!("dims refresh failed: {e}"));
                                }
                                self.dims_reload();
                            } else {
                                // 面没找回来: 至少把新值写进记录(列表显示与模型一致), 签名留旧的
                                pla_ui::diag::log("dims refresh: faces not found after apply");
                                let _ = pla_ui::dims::update(&self.doc_key, &rec);
                                self.dims_reload();
                            }
                            // P4: 联动中 → 下一条
                            self.link_next();
                        }
                        (Some(DimPending::Refresh(rec)), Err(e)) => {
                            pla_ui::diag::log(&format!("dims refresh resolve failed: {e}"));
                            let _ = pla_ui::dims::update(&self.doc_key, &rec);
                            self.dims_reload();
                            self.link_next();
                        }
                        (other, _) => self.dim_pending = other,
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
                // N2: 命令派发回包(界面尚无发起入口; 结果文字随后的 Report 显示, 这里只记日志)。
                pla_bridge::cdp::CdpEvent::ExecResult(r) => {
                    if let Err(e) = &r {
                        eprintln!("[pla-ui] 命令派发失败: {e}");
                    }
                    // N3: 应用完成 → 面 id 已变(几何改了), 下一帧按当前选择重新查询
                    if self.dim_busy {
                        self.dim_busy = false;
                        self.dim_face = None;
                        self.dim_info = None;
                        // N3d: 改的是某条记录 → 按"旧签名 + 已知平移量"预测新位置, 重新定位后刷新记录
                        let ok = matches!(&r, Ok(v) if v["failed"].is_null());
                        // P4 联动: 这一条失败 → 撤回整次; 成功但不刷新记录(没对应记录)→ 直接下一条
                        if !ok {
                            let why = match &r { Ok(v) => v["failed"]["err"].as_str().unwrap_or("?").to_string(), Err(e) => e.clone() };
                            self.link_fail(&why);
                        } else if self.dim_last_apply.as_ref().map_or(true, |ap| !self.dims_list.iter().any(|x| x.name == ap.name)) {
                            self.dim_last_apply = None;
                            self.link_next();
                        }
                        if let (true, Some(ap)) = (ok, self.dim_last_apply.take()) {
                            if let Some(mut rec) = self.dims_list.iter().find(|x| x.name == ap.name).cloned() {
                                let mut sigs: Vec<serde_json::Value> = rec.faces.clone();
                                sigs.extend(rec.opposite.clone());
                                for (s, m) in sigs.iter_mut().zip(ap.moves.iter()) {
                                    if let Some(c) = s["center"].as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect::<Vec<_>>()) {
                                        if c.len() == 3 {
                                            s["center"] = serde_json::json!([c[0] + m[0], c[1] + m[1], c[2] + m[2]]);
                                        }
                                    }
                                }
                                rec.value = ap.value;
                                self.cdp.send(pla_bridge::cdp::CdpCmd::Call {
                                    name: pla_bridge::shim::api::GEO_RESOLVE,
                                    args: serde_json::json!({ "faces": sigs, "units": rec.units }),
                                });
                                self.dim_pending = Some(DimPending::Refresh(rec));
                            }
                        }
                    }
                }
                // R34: 材质状态采集回包 → 失败如实提示; 成功则暂存并补发缩略图(同视口状态的串行流程)。
                pla_bridge::cdp::CdpEvent::MatStateResult(state) => {
                    let err = serde_json::from_str::<serde_json::Value>(&state)
                        .ok()
                        .and_then(|v| v["err"].as_str().map(String::from));
                    if let Some(e) = err {
                        self.ms_update_target = None;
                        self.set_notice((true, t!("材质状态采集失败: {e}", "Capturing material state failed: {e}")));
                    } else {
                        // 发 MatStateSave 前总会先设好 ms_shot_path; 截图失败也会回 ViewportPng(0) → 无图落盘
                        self.ms_pending_state = Some(state);
                        let path = self.ms_shot_path.clone().unwrap_or_else(|| {
                            pla_ui::matstate::STORE.dir(&self.doc_key).join("__shot_tmp.png").display().to_string()
                        });
                        self.cdp.send(pla_bridge::cdp::CdpCmd::CaptureViewport(path));
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
                                self.vp_wall.invalidate();
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
                                self.vp_wall.invalidate();
                                self.set_notice((false, t!("视口状态已保存: {stem}", "View state saved: {stem}")));
                            }
                            Err(e) => self.set_notice((true, e)),
                        }
                    } else if let Some(state) = self.vp_pending_state.take() {
                        // 截图失败: 仍然保存(无图)
                        match pla_ui::vpstate::save_vpstate(&self.doc_key, &state, None) {
                            Ok(stem) => {
                                self.vp_wall.invalidate();
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
                    // R34: 材质状态存档 / 更新(截图失败照样落盘, 只是没有缩略图)
                    if let Some(state) = self.ms_pending_state.take() {
                        let png = if png_len > 0 {
                            self.ms_shot_path.clone().and_then(|p| std::fs::read(p).ok())
                        } else {
                            None
                        };
                        let store = pla_ui::matstate::STORE;
                        let res = match self.ms_update_target.take() {
                            Some(target) => store
                                .update(&self.doc_key, &target, &state, png.as_deref())
                                .map(|_| t!("材质状态已更新: {target}", "Material state updated: {target}")),
                            None => store
                                .save(&self.doc_key, &state, png.as_deref())
                                .map(|stem| t!("材质状态已保存: {stem}", "Material state saved: {stem}")),
                        };
                        match res {
                            Ok(mut msg) => {
                                self.ms_wall.invalidate();
                                if png.is_none() {
                                    msg.push_str(tr("(截图失败, 无缩略图)", " (capture failed, no thumbnail)"));
                                }
                                self.set_notice((false, msg));
                            }
                            Err(e) => self.set_notice((true, e)),
                        }
                    }
                    // C0: 配置存档 / 更新(截图失败照样落盘)
                    if let Some(state) = self.cfg_pending_state.take() {
                        let png = if png_len > 0 { self.cfg_shot_path.clone().and_then(|p| std::fs::read(p).ok()) } else { None };
                        let store = pla_ui::configs::STORE;
                        let res = match self.cfg_update_target.take() {
                            Some(target) => store.update(&self.doc_key, &target, &state, png.as_deref()).map(|_| t!("配置已更新: {target}", "Configuration updated: {target}")),
                            None => store.save(&self.doc_key, &state, png.as_deref()).map(|stem| t!("配置已保存: {stem}", "Configuration saved: {stem}")),
                        };
                        match res {
                            Ok(mut msg) => {
                                self.cfg_wall.invalidate();
                                if png.is_none() {
                                    msg.push_str(tr("(截图失败, 无缩略图)", " (capture failed, no thumbnail)"));
                                }
                                self.set_notice((false, msg));
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
    fn try_auto_snapshot(&mut self, _reason: &str) {
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
            Ok(_) => {
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
            // 视口 / 材质 / 选择集 = 配置窗口的标签: 快捷键直达该标签; 已经停在该标签 = 关窗口
            Action::ToggleEntities => self.toggle_cfg_tab(PanelId::Entities),
            Action::ToggleVpState => self.toggle_cfg_tab(PanelId::VpState),
            Action::ToggleMatState => self.toggle_cfg_tab(PanelId::MatState),
            // 工具栏配置按钮: 窗口开着(任意标签)= 关; 关着 = 回到上次的标签
            Action::ToggleConfig => {
                if self.cfg_group_open() {
                    self.close_cfg_group();
                } else {
                    self.open_cfg_tab(self.cfg_last);
                }
            }
            Action::ToggleDim => {
                // N3: 与笔记一样是独立浮窗 —— 不关别的面板, 也不被别的面板关掉(用户 2026-11-15)。
                self.show_dim = !self.show_dim;
                if !self.show_dim {
                    self.dim_win.closed();
                    self.dim_set_highlight(String::new(), Vec::new());
                }
            }
            Action::ToggleBom => {
                // B1/B2: 独立浮窗(同参数化), 打开时按当前范围扫描一次
                self.show_bom = !self.show_bom;
                if self.show_bom {
                    // 用户 2026-11-17: 生产窗口占地方, 与快照墙 / 配置窗口不共存
                    self.show_snapshots = false;
                    self.close_cfg_group();
                    let scope = self.bom_scope.clone();
                    self.bom_scan(scope);
                } else {
                    self.bom_win.closed();
                }
            }
            Action::ToggleNotes => {
                // R30: 不调用 close_secondary_panels_except — 笔记与其他面板并存。
                self.show_notes = !self.show_notes;
                if self.show_notes {
                    self.notes.set_doc(&self.doc_key);
                } else {
                    self.notes.flush();
                    self.notes_win.closed();
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

    /// 独立浮窗的默认外框: 工具栏下方、二级面板右侧。
    fn default_float_rect(ctx: &egui::Context, w: f32, h: f32) -> [f32; 4] {
        let root = ctx.input(|i| i.viewport().outer_rect).unwrap_or(egui::Rect::from_min_size(
            egui::pos2(200.0, 120.0),
            egui::vec2(TOOLBAR_SIZE[0], TOOLBAR_SIZE[1]),
        ));
        [root.left() + PANEL_SNAPSHOTS[0] + 12.0, root.bottom() + 8.0, w, h]
    }

    /// R30: 笔记浮窗(独立浮窗的外壳见 `floating_window`)。
    fn show_notes_window(&mut self, ctx: &egui::Context) {
        let default = Self::default_float_rect(ctx, 320.0, 440.0);
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
        let mut reload_notes = false;
        let notes = &mut self.notes;
        let notes_crash = &mut self.notes_crash;
        let open = floating_window(
            ctx,
            FloatSpec {
                id: "panel-notes",
                prefs_key: NOTES_RECT_KEY,
                title: tr("笔记", "Notes"),
                subtitle: truncate_chars(&doc, 24),
                close_tip: tr("关闭(内容已自动保存)", "Close (content is saved automatically)"),
                default_rect: default,
                min_size: egui::vec2(NotesDrag::MIN_W, NotesDrag::MIN_H),
                resizable: true,
                fit_height: false,
            },
            &mut self.notes_win,
            |ui| {
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
            },
        );
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
            self.notes_win.closed();
        }
    }

    /// N3: 尺寸浮窗(独立浮窗, 同笔记: 不跟随工具栏、不互斥、位置记忆)。
    fn show_dim_window(&mut self, ctx: &egui::Context) {
        let default = Self::default_float_rect(ctx, PANEL_DIM[0], PANEL_DIM[1]);
        let mut win = std::mem::take(&mut self.dim_win);
        let open = floating_window(
            ctx,
            FloatSpec {
                id: "panel-dim",
                prefs_key: DIM_RECT_KEY,
                title: tr("参数化", "Parametric"),
                subtitle: String::new(),
                close_tip: tr("关闭", "Close"),
                default_rect: default,
                min_size: egui::vec2(NotesDrag::MIN_W, NotesDrag::MIN_H),
                resizable: true,
                // 用户 2026-11-15: 默认贴合内容高度; 手动调过大小后保持, 内容超出出滚动条
                fit_height: true,
            },
            &mut win,
            |ui| {
                // N3 改尺寸(原生风格: 节标题 / 信息卡片 / 分段选择 / 当前·目标数值框 / Esc Cancel · OK ↵)。
                // 单选一个面 → dim.face(厚度 + 候选对面 / 半径 / 圆角半径); 多选 → dim.distance(约定: 第一个选的面 = 参考面)。
                if let Some(why) = self.feature_block(Feature::DimFace) {
                    ui.label(egui::RichText::new(why).small().color(COL_RED));
                }
                let faces: Vec<String> =
                    self.live_sel.iter().filter(|it| it.kind == "face").map(|it| it.id.clone()).collect();
                let only_faces = faces.len() == self.live_sel.len();
                // 查询键: 单选 = "面"; 多选 = "D:参考|动的…"(第一个选的面 = 参考面 —— 用户 2026-11-15 确认)
                // 打孔模式: 只要恰好一个面就读它(同时选的草图点 / 实体是孔位与组合孔的其余实体)
                let hole_mode = self.dim_mode == 1 && self.dim_tab == 0;
                let cur = match faces.len() {
                    1 if only_faces || hole_mode => Some(faces[0].clone()),
                    n if n >= 2 && only_faces => Some(format!("D:{}", faces.join("|"))),
                    _ => None,
                };
                if cur != self.dim_face && !self.dim_busy {
                    // 在"记录"标签时选择变了: 不跳标签, 只在"操作"标签上点个提示点(用户 2026-11-15)
                    if self.dim_tab != 0 && cur.is_some() {
                        self.dim_tab_dot = true;
                    }
                    self.dim_face = cur.clone();
                    self.dim_info = None;
                    if let Some(k) = &cur {
                        match k.strip_prefix("D:") {
                            Some(list) => {
                                // 第一个是平面 → 距离; 全是等半径倒圆 / 圆柱 → 一起改半径(垫片 dim.faces 判)
                                let ids: Vec<&str> = list.split('|').collect();
                                self.cdp.send(pla_bridge::cdp::CdpCmd::Call {
                                    name: pla_bridge::shim::api::DIM_FACES,
                                    args: serde_json::json!({ "faces": ids }),
                                });
                            }
                            None => self.cdp.send(pla_bridge::cdp::CdpCmd::Call {
                                name: pla_bridge::shim::api::DIM_FACE,
                                args: serde_json::json!({ "face": k }),
                            }),
                        }
                    }
                }
                let info = match (&self.dim_face, &self.dim_info) {
                    (Some(_), Some(Ok(v))) => Some(v.clone()),
                    _ => None,
                };
                // 高亮: 厚度 → 当前对面橙、其余候选蓝; 距离 → 参考面绿、锚面橙; 其他清除
                let (hl_key, hl_faces) = match &info {
                    Some(v) if v["kind"] == "thickness" => {
                        let cands = v["candidates"].as_array().cloned().unwrap_or_default();
                        let idx = self.dim_cand.min(cands.len().saturating_sub(1));
                        let list: Vec<serde_json::Value> = cands
                            .iter()
                            .enumerate()
                            .map(|(i, c)| serde_json::json!({ "id": c["id"], "role": if i == idx { "primary" } else { "candidate" } }))
                            .collect();
                        (format!("{}#{idx}#{}", v["face"], cands.len()), list)
                    }
                    Some(v) if v["kind"] == "distance" => (
                        format!("D{}#{}", v["reference"], v["anchorFace"]),
                        vec![
                            serde_json::json!({ "id": v["reference"], "role": "source" }),
                            serde_json::json!({ "id": v["anchorFace"], "role": "primary" }),
                        ],
                    ),
                    _ => (String::new(), Vec::new()),
                };
                // 打孔模式不画改尺寸的候选高亮(选中的面本身就是孔要打的面)
                if self.dim_mode == 1 {
                    self.dim_set_highlight(String::new(), Vec::new());
                } else {
                    self.dim_set_highlight(hl_key, hl_faces);
                }
                // ---- 标签: 操作(随选择变) / 记录(存下来的东西; 以后孔等类型也在这里按类型筛选) ----
                {
                    // 每个文档记住上次停在哪个标签(prefs "dim_tabs": {doc_key: 0|1})
                    if self.dim_tab_doc != self.doc_key {
                        self.dim_tab_doc = self.doc_key.clone();
                        self.dim_tab = crate::prefs_dim_tab(&self.doc_key);
                        self.dim_tab_dot = false;
                    }
                    if !self.dims_loaded {
                        self.dims_reload();
                        self.dims_loaded = true;
                    }
                    let n = self.dims_list.len();
                    let labels = vec![tr("操作", "Operate").to_string(), if n > 0 { t!("记录 {n}", "Records {n}") } else { tr("记录", "Records").to_string() }];
                    let mut s = self.dim_tab as usize;
                    let dot = (self.dim_tab_dot && self.dim_tab != 0).then_some(0);
                    if rz_tabs(ui, &labels, &mut s, dot) {
                        self.dim_tab = s as u8;
                        if self.dim_tab == 0 {
                            self.dim_tab_dot = false;
                        }
                        crate::prefs_save_dim_tab(&self.doc_key, self.dim_tab);
                    }
                    ui.add_space(4.0);
                }
                if self.dim_tab == 0 {
                    // 列表不在本标签: 条目编辑态不能残留(否则会挡住这里的右键应用)
                    self.dims_row_focus = false;
                    self.dims_edit_row = None;
                }
                if self.dim_tab != 0 {
                    // 记录标签: 顶部"保存当前尺寸" + 列表(保存 / 更新 ↻ 都用「操作」里当前读到的尺寸)
                    self.dim_save_row(ui, info.as_ref(), &faces);
                    self.dim_saved_ui(ui, info.clone());
                    return;
                }
                // ---- 「操作」的模式: 改尺寸 / 打孔(以后的新操作加在这一行) ----
                {
                    let labels = vec![tr("改尺寸", "Resize").to_string(), tr("打孔", "Hole").to_string()];
                    let mut s = self.dim_mode as usize;
                    if rz_segmented(ui, "dim-mode", &labels, &mut s) {
                        self.dim_mode = s as u8;
                    }
                }
                if self.dim_mode == 1 {
                    self.hole_ui(ui, info.as_ref(), &faces);
                    return;
                }
                let units = |v: &serde_json::Value| -> String {
                    match v["units"].as_str().unwrap_or("") {
                        "millimeter" => "mm", "centimeter" => "cm", "meter" => "m", "inch" => "in", "foot" => "ft", u => u,
                    }
                    .to_string()
                };
                // ---- 节标题 ----
                let title = match &info {
                    Some(v) => match v["kind"].as_str().unwrap_or("") {
                        "thickness" => tr("厚度", "THICKNESS"),
                        "radius" => tr("半径", "RADIUS"),
                        "filletRadius" => tr("圆角半径", "FILLET RADIUS"),
                        "distance" => tr("距离", "DISTANCE"),
                        _ => tr("尺寸", "DIMENSION"),
                    },
                    None => tr("尺寸", "DIMENSION"),
                };
                let (trow, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::hover());
                let tr_ = paint_text(ui.painter(), egui::pos2(trow.left(), trow.center().y), egui::Align2::LEFT_CENTER, title, 12.0, COL_SECTION, true);
                // "?" 说明(悬停弹三级面板; 含"第一个选的面 = 参考面"约定)
                {
                    let c = egui::pos2(tr_.right() + 10.0, trow.center().y);
                    let hot = ui.rect_contains_pointer(egui::Rect::from_center_size(c, egui::vec2(16.0, 16.0)));
                    let col = if hot { egui::Color32::WHITE } else { COL_MUTED };
                    ui.painter().circle_stroke(c, 5.5, egui::Stroke::new(1.0_f32, col));
                    ui.painter().text(c, egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(8.5), col);
                    hover_info(ui, trow, hot, tr("参数化 · 说明", "Parametric · help"), dim_help_lines());
                }
                if info.is_some() {
                    self.dim_record_control(ui, trow, &faces);
                }
                match (self.dim_face.clone(), self.dim_info.clone()) {
                    (None, _) => rz_info_card(
                        ui,
                        tr("选择要改的面", "Select a face to resize"),
                        tr(
                            "一个面: 平面 = 厚度, 圆柱 = 半径, 倒圆 = 圆角半径。\n先选参考平面, 再 Shift 加选要动的面 = 距离 / 孔位。",
                            "One face: plane = thickness, cylinder = radius, fillet = fillet radius.\nReference plane first, then Shift-add faces to move = distance / hole position.",
                        ),
                    ),
                    (Some(_), None) => rz_info_card(ui, tr("读取中…", "Reading…"), ""),
                    (Some(_), Some(Err(e))) => {
                        rz_info_card(ui, tr("读不到这个面的尺寸", "Cannot read this face"), &e);
                    }
                    (Some(_), Some(Ok(v))) if v["kind"] == "distance" => {
                        let u = units(&v);
                        self.dim_distance_ui(ui, &v, &u, &faces);
                    }
                    (Some(_), Some(Ok(v))) => {
                        let kind = v["kind"].as_str().unwrap_or("none");
                        let u = units(&v);
                        let cands = v["candidates"].as_array().cloned().unwrap_or_default();
                        // 厚度: 值随当前候选; 其它: 回包里的值(应用时 dim_apply 自己再取对面)
                        let value = self.dim_value_of(&v);
                        match (kind, value, v["op"].as_str(), v["offsetSign"].as_f64()) {
                            ("none", ..) | (_, None, ..) | (_, _, None, _) | (.., None) => {
                                let surf = v["surface"].as_str().unwrap_or("?").to_string();
                                rz_info_card(
                                    ui,
                                    tr("这个面没有可改的尺寸", "No editable dimension on this face"),
                                    &t!("曲面 {surf}; 平面需要同一实体里有背对的平行面", "Surface {surf}; a plane needs an opposite parallel face in the same solid"),
                                );
                            }
                            (_, Some(value), Some(_), Some(_)) => {
                                let (card_title, card_sub) = match kind {
                                    "thickness" => (tr("到背对的平行面", "To the opposite parallel face"), t!("对面 {}/{}(橙)", "Opposite {}/{} (orange)", self.dim_cand.min(cands.len().saturating_sub(1)) + 1, cands.len())),
                                    "radius" if v["convex"] == true => (tr("凸台", "Boss"), tr("轴线不动", "Axis stays").to_string()),
                                    "radius" => (tr("孔", "Hole"), tr("轴线不动", "Axis stays").to_string()),
                                    _ => (tr("倒圆", "Fillet"), tr("保持与两侧相切", "Stays tangent to both sides").to_string()),
                                };
                                let nf = v["faces"].as_array().map_or(1, |a| a.len());
                                let card_sub = if nf > 1 { t!("{card_sub} · {nf} 个面一起改", "{card_sub} · {nf} faces together") } else { card_sub };
                                rz_info_card(ui, card_title, &card_sub);
                                // ---- 对面候选(分段选择, 橙 = 当前) ----
                                if kind == "thickness" && cands.len() > 1 {
                                    rz_label(ui, tr("对面候选(量得的距离)", "Opposite face candidates (measured distance)"));
                                    let labels: Vec<String> = cands.iter().map(|c| fmt3(cand_value(c, self.dim_cyl).unwrap_or(0.0))).collect();
                                    let mut i = self.dim_cand.min(cands.len() - 1);
                                    let round: Vec<bool> = cands.iter().map(|c| c["values"].is_object()).collect();
                                    if rz_candidates(ui, &labels, &round, &mut i) {
                                        self.dim_cand = i;
                                        self.dim_text = fmt3(cand_value(&cands[i], self.dim_cyl).unwrap_or(0.0));
                                    }
                                }
                                // ---- 对面是圆柱: 测量点 最近 / 轴心 / 最远(用户 2026-11-15) ----
                                if let Some(c) = cands.get(self.dim_cand.min(cands.len().saturating_sub(1))).filter(|c| c["values"].is_object()) {
                                    rz_label(ui, tr("圆柱测量点", "Cylinder measure point"));
                                    let labels = vec![tr("最近点", "Nearest").to_string(), tr("轴心", "Axis").to_string(), tr("最远点", "Farthest").to_string()];
                                    let auto = CYL_POINTS.iter().position(|p| c["point"].as_str() == Some(*p)).unwrap_or(0);
                                    let mut s = self.dim_cyl.map_or(auto, |x| x as usize);
                                    if rz_segmented(ui, "dim-cyl", &labels, &mut s) {
                                        self.dim_cyl = Some(s as u8);
                                        self.dim_text = fmt3(cand_value(c, self.dim_cyl).unwrap_or(0.0));
                                    }
                                }
                                // ---- N3b 改哪一侧 ----
                                if kind == "thickness" {
                                    rz_label(ui, tr("移动", "Move"));
                                    let labels = vec![tr("选中面", "Selected").to_string(), tr("对面", "Opposite").to_string(), tr("两侧各半", "Both halves").to_string()];
                                    let mut s = self.dim_side as usize;
                                    if rz_segmented(ui, "dim-side", &labels, &mut s) {
                                        self.dim_side = s as u8;
                                    }
                                }
                                if let Some(t) = self.dim_value_block(ui, value, &u, false) {
                                    self.dim_apply(&v, &faces, t);
                                }
                            }
                        }
                    }
                }
                // 操作标签底部不再放列表(在"记录"标签); 当前选择正是某条记录时标题右侧显示记录名
            },
        );
        self.dim_win = win;
        if !open {
            self.show_dim = false;
            self.dim_win.closed();
            self.dim_set_highlight(String::new(), Vec::new());
        }
    }

    /// N3d: 当前尺寸对应的面 id(单选 = [面]; 多选 = [参考, 动的…])与要记录的对面。
    fn dim_current_faces(v: &serde_json::Value, cand: usize) -> (Vec<String>, Option<String>) {
        if v["kind"] == "distance" {
            let mut ids = vec![v["reference"].as_str().unwrap_or("").to_string()];
            ids.extend(v["moving"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)));
            return (ids, None);
        }
        // 多选等半径倒圆 / 圆柱: 全部面
        if let Some(all) = v["faces"].as_array().filter(|a| a.len() > 1) {
            return (all.iter().filter_map(|x| x.as_str().map(String::from)).collect(), None);
        }
        let face = v["face"].as_str().unwrap_or("").to_string();
        let opp = (v["kind"] == "thickness")
            .then(|| v["candidates"].as_array().and_then(|a| a.get(cand.min(a.len().saturating_sub(1)))).and_then(|c| c["id"].as_str().map(String::from)))
            .flatten();
        (vec![face], opp)
    }

    /// P3: 尺寸 name 的某个面是否就是某个孔(的自动子尺寸)的面 → (孔 id, 孔名)。按签名比: 同曲面类型 + 中点重合。
    fn hole_for(&self, name: &str) -> Option<(String, String)> {
        let rec = self.dims_list.iter().find(|d| d.name == name)?;
        if rec.parent.is_some() {
            return None;
        }
        let mine: Vec<&serde_json::Value> = rec.faces.iter().chain(rec.opposite.iter()).collect();
        let same = |a: &serde_json::Value, b: &serde_json::Value| -> bool {
            let c = |v: &serde_json::Value| -> Vec<f64> { v["center"].as_array().map(|x| x.iter().filter_map(|y| y.as_f64()).collect()).unwrap_or_default() };
            let (ca, cb) = (c(a), c(b));
            a["surface"] == b["surface"] && ca.len() == 3 && cb.len() == 3 && ((ca[0] - cb[0]).powi(2) + (ca[1] - cb[1]).powi(2) + (ca[2] - cb[2]).powi(2)).sqrt() < 1e-3
        };
        self.dims_groups.iter().filter(|g| g.kind == "hole").find_map(|g| {
            let hit = self.dims_list.iter().filter(|d| d.parent.as_deref() == Some(g.id.as_str()) && d.origin == "auto").any(|d| d.faces.iter().any(|f| mine.iter().any(|m| same(m, f))));
            hit.then(|| (g.id.clone(), g.name.clone()))
        })
    }

    /// P5: 参数 / 表达式变了 → 重新求值, 绑定了表达式且目标值与当前值不同的尺寸逐条改(每条走"按签名定位 + 读 + 应用 +
    /// 刷新记录"同一条路; 后改的尺寸若引用先改的尺寸, 用的是求值时的值 —— 求值一次算全)。整个过程包在事务里
    /// (hist.begin … commit), 任何一条失败 → 撤回本次已改的全部(hist.rollback)。每条是一个撤销步。
    fn formula_run(&mut self, label: &str) {
        self.dims_reload();
        let bad: Vec<String> = self.dims_eval.targets.iter().filter_map(|(n, r)| r.as_ref().err().map(|e| format!("{n}: {e}"))).collect();
        if !bad.is_empty() {
            self.set_notice((true, t!("表达式有错, 没有改模型: {}", "Expression errors, model unchanged: {}", bad.join("; "))));
            return;
        }
        let cur: std::collections::BTreeMap<String, f64> = self.dims_list.iter().map(|d| (d.name.clone(), d.value)).collect();
        let changes = self.dims_eval.changes(&cur, 5e-4);
        if changes.is_empty() {
            self.set_notice((false, t!("{label}: 绑定的尺寸都已是计算值", "{label}: bound dimensions already match")));
            return;
        }
        if self.link_run.is_some() || self.dim_busy || self.dim_pending.is_some() {
            self.set_notice((true, tr("上一次修改还没完成, 稍后再改", "The previous change is still running; try again shortly").into()));
            return;
        }
        if !self.require_feature(Feature::DimFace) {
            return;
        }
        self.cdp.send(pla_bridge::cdp::CdpCmd::Call { name: pla_bridge::shim::api::HIST_BEGIN, args: serde_json::Value::Null });
        let total = changes.len();
        self.link_run = Some(LinkRun { label: label.to_string(), queue: changes.into_iter().collect(), total });
        self.link_next();
    }

    /// P5: 下一条(没有了 → 提交事务)。
    fn link_next(&mut self) {
        let Some(run) = self.link_run.as_mut() else { return };
        let next = run.queue.pop_front();
        let (label, total) = (run.label.clone(), run.total);
        match next.and_then(|(n, t)| self.dims_list.iter().find(|d| d.name == n).cloned().map(|d| (d, t))) {
            Some((rec, target)) => {
                self.set_notice((false, t!("{label}: {} → {}…", "{label}: {} → {}…", rec.name, fmt3(target))));
                self.dim_record_call(&rec, DimPending::Apply(rec.clone(), target));
            }
            None => {
                self.link_run = None;
                self.cdp.send(pla_bridge::cdp::CdpCmd::Call { name: pla_bridge::shim::api::HIST_COMMIT, args: serde_json::Value::Null });
                self.set_notice((false, t!("{label}: 已改 {total} 处尺寸", "{label}: {total} dimension(s) changed")));
            }
        }
    }

    /// P5: 某条失败 → 撤回本次已改的全部。
    fn link_fail(&mut self, why: &str) {
        let Some(run) = self.link_run.take() else { return };
        self.cdp.send(pla_bridge::cdp::CdpCmd::Call { name: pla_bridge::shim::api::HIST_ROLLBACK, args: serde_json::Value::Null });
        self.set_notice((true, t!("{} 失败, 已撤回本次全部修改: {why}", "{} failed; all changes of this run rolled back: {why}", run.label)));
    }

    /// B1/B2: 按范围扫一次 BOM(None = 当前可见; Some(配置名) = 该配置抓取时的可见实体, 不用切换显示)。
    fn bom_scan(&mut self, scope: Option<String>) {
        let args = match &scope {
            None => serde_json::json!({}),
            Some(cf) => match pla_ui::configs::STORE.load(&self.doc_key, cf) {
                Some(st) => serde_json::json!({ "members": pla_ui::configs::members(&st).into_iter().map(|m| m.0).collect::<Vec<_>>() }),
                None => {
                    self.set_notice((true, t!("配置 {cf} 的数据缺失", "Configuration {cf} has no data")));
                    return;
                }
            },
        };
        self.cdp.send(pla_bridge::cdp::CdpCmd::Call { name: pla_bridge::shim::api::BOM_SCAN, args });
        self.bom_busy = true;
        self.bom_pending_scope = Some(scope);
    }

    /// BOM 表列宽 / 冻结列数 → prefs(全局, 跨文档)。
    fn bom_cols_save(&mut self) {
        let v = serde_json::json!({ "w": self.bom_col_w, "freeze": self.bom_freeze });
        if let Err(e) = pla_ui::prefs::save_value(BOM_COLS_KEY, v) {
            self.set_notice((true, t!("列宽保存失败: {e}", "Saving column widths failed: {e}")));
        }
    }

    /// B3: 本文档的编辑按需载入; 模板列表 / 当前模板每秒查一次修改时间, 改了重读(读错保留上一次能用的)。
    fn bom_template_tick(&mut self) {
        if self.bom_edits_doc != self.doc_key {
            self.bom_edits_doc = self.doc_key.clone();
            self.bom_edits = pla_ui::bom_table::load_edits(&self.doc_key);
            self.bom_cell_buf.clear();
            if !self.bom_edits.template.is_empty() {
                self.bom_tpl_name = self.bom_edits.template.clone();
                self.bom_tpl_mtime = None;
            }
        }
        if self.bom_tpl_checked.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return;
        }
        self.bom_tpl_checked = Some(Instant::now());
        let dir = pla_ui::bom_table::dir();
        self.bom_tpl_list = pla_ui::bom_table::list_in(&dir);
        if !self.bom_tpl_list.contains(&self.bom_tpl_name) {
            self.bom_tpl_name = self.bom_tpl_list.first().cloned().unwrap_or_default();
            self.bom_tpl_mtime = None;
        }
        let m = pla_ui::bom_table::mtime_in(&dir, &self.bom_tpl_name);
        if m.is_some() && m == self.bom_tpl_mtime {
            return;
        }
        self.bom_tpl_mtime = m;
        match pla_ui::bom_table::load_in(&dir, &self.bom_tpl_name) {
            Ok(t) => {
                self.bom_tpl = t;
                self.bom_tpl_err = None;
                self.bom_cell_buf.clear();
            }
            Err(e) => self.bom_tpl_err = Some(e),
        }
    }

    fn bom_tpl_select(&mut self, name: String) {
        self.bom_tpl_name = name;
        self.bom_tpl_mtime = None;
        self.bom_tpl_checked = None;
        self.bom_edits.template = self.bom_tpl_name.clone();
        let _ = pla_ui::bom_table::save_edits(&self.doc_key, &self.bom_edits);
    }

    /// B4: 零件库每秒查一次修改时间(外部编辑 library.json 也能热生效)。
    fn lib_tick(&mut self) {
        if self.lib_checked.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return;
        }
        self.lib_checked = Some(Instant::now());
        let dir = pla_ui::parts_lib::dir();
        let m = pla_ui::parts_lib::mtime_in(&dir);
        if m != self.lib_mtime || (m.is_none() && self.lib.entries.is_empty()) {
            self.lib_mtime = m;
            self.lib = pla_ui::parts_lib::load_in(&dir);
        }
    }

    fn lib_save(&mut self) {
        let dir = pla_ui::parts_lib::dir();
        match pla_ui::parts_lib::save_in(&dir, &self.lib) {
            Ok(()) => self.lib_mtime = pla_ui::parts_lib::mtime_in(&dir),
            Err(e) => self.set_notice((true, t!("零件库保存失败: {e}", "Saving the parts library failed: {e}"))),
        }
    }

    fn bom_edits_save(&mut self) {
        self.bom_edits.template = self.bom_tpl_name.clone();
        if let Err(e) = pla_ui::bom_table::save_edits(&self.doc_key, &self.bom_edits) {
            self.set_notice((true, t!("BOM 编辑保存失败: {e}", "Saving BOM edits failed: {e}")));
        }
    }

    /// B4: 把 BOM 里的一行存成零件库新条目(料号按规则自动给, 字段 / 单价取这一行的当前值, 记下零件 key 与模型名作匹配), 并链接。
    fn lib_add_from_row(&mut self, row: &pla_ui::bom_table::Row, model_name: &str) {
        let cols = self.bom_tpl.columns.clone();
        let get = |id: &str| cols.iter().position(|c| c.id == id).map(|i| row.cells[i].clone()).unwrap_or_default();
        let origin = get("origin");
        // 分类: 按名称 / 型号关键词猜(工具 / 耗材 / 橡胶 / 外购组件 / 传动 / 紧固件 / 电气), 否则采购 → 007, 再按材料(塑胶 001 / 金属 002); 认不出 → 001
        let category = pla_ui::parts_lib::guess_category(&get("name"), &get("model"), &get("material"), &origin).unwrap_or("001");
        let pn = if get("pn").trim().is_empty() || self.lib.find(get("pn").trim()).is_some() { self.lib.next_pn_for(&self.bom_edits.project, category) } else { get("pn").trim().to_string() };
        let mut en = pla_ui::parts_lib::Entry { pn: pn.clone(), name: get("name"), category: category.into(), ..Default::default() };
        for c in cols.iter().filter(|c| c.source.as_deref() == Some("field") && c.formula.is_none() && c.id != "pn" && c.id != "unit_price") {
            let v = get(&c.id);
            if !v.trim().is_empty() {
                en.fields.insert(c.id.clone(), v);
            }
        }
        let price = get("unit_price");
        if price.trim().parse::<f64>().is_ok_and(|p| p != 0.0) {
            en.suppliers.push(pla_ui::parts_lib::Supplier { price, date: pla_ui::parts_lib::today(), ..Default::default() });
        }
        if !model_name.is_empty() && model_name != en.name {
            en.aliases.push(model_name.to_string());
        }
        if !row.manual {
            en.keys.push(row.key.clone());
        }
        if let Some(sp) = &row.spec {
            en.spec = sp.key.clone();
            // 型号空着就用认出的规格
            if en.fields.get("model").is_none_or(|m| m.trim().is_empty()) {
                en.fields.insert("model".into(), sp.text());
            }
        }
        self.lib.upsert(en);
        self.lib_save();
        self.bom_edits.links.insert(row.key.clone(), pn.clone());
        // 已存进库的字段不再作为本文档覆盖重复保存
        if let Some(m) = self.bom_edits.parts.get_mut(&row.key) {
            m.retain(|k, _| k == "usage" || k == "sets");
        }
        self.bom_edits_save();
        self.bom_cell_buf.clear();
        self.set_notice((false, t!("已存入零件库: {pn}", "Saved to the parts library: {pn}")));
    }

    /// B4: 从 CSV 导入零件库, 报告写进 lib_import_report。
    fn lib_import(&mut self) {
        let path = self.lib_import_path.trim().trim_matches('"').to_string();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                self.set_notice((true, t!("读取失败: {e}", "Read failed: {e}")));
                return;
            }
        };
        let headers: Vec<(String, String)> = self.bom_tpl.columns.iter().map(|c| (c.id.clone(), c.header.clone())).collect();
        let rep = pla_ui::parts_lib::import_csv(&mut self.lib, &text, &headers);
        if !rep.added.is_empty() || !rep.updated.is_empty() {
            self.lib_save();
        }
        let mut s = t!("新增 {} · 更新 {} · 跳过 {} · 类型码与材料不符 {}", "Added {} · updated {} · skipped {} · type code vs material mismatch {}", rep.added.len(), rep.updated.len(), rep.skipped.len(), rep.mismatched.len());
        for (row, why) in &rep.skipped {
            s.push_str(&t!("\n  跳过 第 {row} 行: {why}", "\n  skipped row {row}: {why}"));
        }
        for (row, old, new) in &rep.renumbered {
            s.push_str(&t!("\n  第 {row} 行: 料号 {old} 在表里重复, 给了新料号 {new}", "\n  row {row}: part number {old} is used twice, given new number {new}"));
        }
        for (pn, mat, got, want) in &rep.mismatched {
            s.push_str(&t!("\n  {pn}: 材料 {mat} 应为 {want} 类, 料号是 {got}", "\n  {pn}: material {mat} suggests type {want}, number has {got}"));
        }
        self.lib_import_report = Some(s);
    }

    /// B4 按类型整理: 建议改类型的条目(可取消勾选 / 改成别的类型), 应用 = 按料号顺序逐条重新编号, 旧号进 former, 本文档链接跟着改。
    fn lib_reclass_ui(&mut self, ui: &mut egui::Ui) {
        let Some(mut plan) = self.lib_reclass.take() else { return };
        let cats: Vec<(String, String)> = self.lib.pn_rule.categories.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        ui.label(egui::RichText::new(t!("{} 条建议改类型(旧料号保留, 搜索 / 旧表 / 链接仍认得)", "{} entries suggested for a new type (old numbers are kept and still resolve)", plan.len())).strong());
        let (mut apply, mut cancel) = (false, false);
        egui::ScrollArea::vertical().id_salt("lib-reclass").max_height(420.0).show(ui, |ui| {
            egui::Grid::new("lib-reclass-g").striped(true).num_columns(5).spacing([8.0, 3.0]).show(ui, |ui| {
                for h in ["", tr("料号", "Part no."), tr("名称", "Name"), tr("现类型", "Now"), tr("改为", "New type")] {
                    ui.label(egui::RichText::new(h).color(COL_MUTED).size(11.5));
                }
                ui.end_row();
                for (i, (pn, cat, on)) in plan.iter_mut().enumerate() {
                    let Some(e) = self.lib.find(pn) else { continue };
                    ui.checkbox(on, "");
                    ui.label(pn.as_str());
                    ui.label(&e.name).on_hover_text(e.fields.get("model").cloned().unwrap_or_default());
                    ui.label(&e.category);
                    egui::ComboBox::from_id_salt(("lib-reclass-c", i)).width(200.0).selected_text(cats.iter().find(|c| c.0 == *cat).map(|c| format!("{} {}", c.0, c.1)).unwrap_or(cat.clone())).show_ui(ui, |ui| {
                        for (k, v) in &cats {
                            if ui.selectable_label(cat == k, format!("{k} {v}")).clicked() {
                                *cat = k.clone();
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        });
        ui.horizontal(|ui| {
            apply = ui.button(egui::RichText::new(tr("应用", "Apply")).strong()).clicked();
            cancel = ui.button(tr("取消", "Cancel")).clicked();
        });
        if cancel {
            return;
        }
        if !apply {
            self.lib_reclass = Some(plan);
            return;
        }
        let mut n = 0;
        for (pn, cat, _) in plan.iter().filter(|p| p.2) {
            if let Some(new_pn) = self.lib.reclassify(pn, cat) {
                for v in self.bom_edits.links.values_mut() {
                    if v == pn {
                        *v = new_pn.clone();
                    }
                }
                n += 1;
            }
        }
        self.lib_save();
        self.bom_edits_save();
        self.bom_cell_buf.clear();
        self.set_notice((false, t!("已重新编号 {n} 条", "Renumbered {n} entries")));
    }

    /// B4 零件库标签: 搜索 + 列表(料号 / 名称 / 型号 / 首选单价 / 供应商数 / 本文档在用); 点"编辑"或"＋ 新建"进条目表单。
    fn lib_ui(&mut self, ui: &mut egui::Ui) {
        let field_cols: Vec<pla_ui::bom_table::Col> = self
            .bom_tpl
            .columns
            .iter()
            .filter(|c| c.source.as_deref() == Some("field") && c.formula.is_none() && !matches!(c.id.as_str(), "pn" | "unit_price" | "sets" | "redundancy" | "usage"))
            .cloned()
            .collect();
        if self.lib_edit.is_some() {
            self.lib_form(ui, &field_cols);
            return;
        }
        if self.lib_reclass.is_some() {
            self.lib_reclass_ui(ui);
            return;
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.lib_search).hint_text(tr("搜索料号 / 名称 / 型号…", "Search part number / name / model…")).desired_width(220.0));
            if ui.button(tr("＋ 新建", "+ New")).clicked() {
                let pn = self.lib.next_pn_for(&self.bom_edits.project, "009");
                self.lib_edit = Some(pla_ui::parts_lib::Entry { pn, category: "009".into(), ..Default::default() });
                self.lib_edit_orig = None;
                self.lib_edit_aliases.clear();
            }
            if ui.button(tr("按类型整理…", "Sort by type…")).on_hover_text(tr("按名称 / 型号猜类型, 列出与当前类型码不同的条目, 确认后重新编号(旧号保留)", "Guess each entry's type from name / model, list the ones whose type code differs, renumber after you confirm (old numbers are kept)")).clicked() {
                let plan: Vec<(String, String, bool)> = self
                    .lib
                    .entries
                    .iter()
                    .filter_map(|e| {
                        let f = |k: &str| e.fields.get(k).cloned().unwrap_or_default();
                        let g = pla_ui::parts_lib::guess_category(&e.name, &f("model"), &f("material"), &f("origin"))?;
                        (g != e.category).then(|| (e.pn.clone(), g.to_string(), true))
                    })
                    .collect();
                self.lib_reclass = Some(plan);
            }
            if ui.button(tr("打开文件夹", "Open folder")).clicked() {
                let d = pla_ui::parts_lib::dir();
                let _ = std::fs::create_dir_all(&d);
                let _ = std::process::Command::new("explorer").arg(&d).spawn();
            }
        });
        // 本文档项目编号(料号前三位) + 分类表说明
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr("本文档项目编号", "Project code (this doc)")).color(COL_MUTED));
            let mut p = self.bom_edits.project.clone();
            let hint = self.lib.pn_rule.prefix.clone();
            if ui.add(egui::TextEdit::singleline(&mut p).hint_text(hint).desired_width(50.0)).on_hover_text(tr("料号前三位, 如 904 = OYM; 空 = 默认前缀", "First three digits of part numbers, e.g. 904 = OYM; empty = default prefix")).changed() {
                self.bom_edits.project = p.trim().to_string();
                self.bom_edits_save();
            }
            let cats: String = self.lib.pn_rule.categories.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join("\n");
            ui.label(egui::RichText::new(tr("料号 = 项目 + 类型 + 序号 ⓘ", "Part no. = project + type + serial ⓘ")).size(11.0).color(COL_MUTED)).on_hover_text(cats);
        });
        // 从现有 BOM 表(CSV)导入
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.lib_import_path).hint_text(tr("BOM CSV 路径", "BOM CSV path")).desired_width(300.0));
            if ui.button(tr("从 CSV 导入", "Import CSV")).on_hover_text(tr("按当前 BOM 模板的表头对列; 有料号的行进库, 已有料号则更新", "Columns are matched by the current BOM template's headers; rows with a part number go in, existing ones are updated")).clicked() {
                self.lib_import();
            }
        });
        if let Some(r) = &self.lib_import_report {
            ui.label(egui::RichText::new(r).size(11.0).color(COL_MUTED));
        }
        ui.label(egui::RichText::new(tr("零件库跨文档共用: BOM 行的「库」菜单可存入 / 链接; 链接后名称、料号、型号、单价等从这里带出(蓝色)。", "The library is shared across documents: use the row's 库 menu in the list to save / link; linked rows take name, part number, model, price from here (blue).")).size(11.0).color(COL_MUTED));
        let used: std::collections::HashSet<String> = self.bom_edits.links.values().cloned().collect();
        let q = self.lib_search.trim().to_lowercase();
        let entries: Vec<pla_ui::parts_lib::Entry> = self
            .lib
            .entries
            .iter()
            .filter(|e| {
                q.is_empty()
                    || e.pn.to_lowercase().contains(&q)
                    || e.former.iter().any(|f| f.contains(&q))
                    || e.name.to_lowercase().contains(&q)
                    || e.fields.get("model").is_some_and(|m| m.to_lowercase().contains(&q))
            })
            .cloned()
            .collect();
        if entries.is_empty() {
            ui.label(egui::RichText::new(tr("(空)", "(empty)")).color(COL_MUTED));
            return;
        }
        let mut edit: Option<pla_ui::parts_lib::Entry> = None;
        egui::ScrollArea::horizontal().id_salt("lib-h").show(ui, |ui| {
            egui::Grid::new("lib-grid").striped(true).num_columns(7).spacing([10.0, 3.0]).show(ui, |ui| {
                for h in [tr("料号", "Part no."), tr("名称", "Name"), tr("型号", "Model"), tr("单价", "Price"), tr("供应商", "Suppliers"), tr("本文档", "This doc"), ""] {
                    ui.label(egui::RichText::new(h).color(COL_MUTED).size(11.5));
                }
                ui.end_row();
                for e in &entries {
                    ui.label(&e.pn);
                    ui.label(&e.name);
                    ui.label(e.fields.get("model").cloned().unwrap_or_default());
                    let s = e.suppliers.get(e.preferred).or_else(|| e.suppliers.first());
                    ui.label(s.map(|s| format!("{} {}", s.price, s.currency)).unwrap_or_default()).on_hover_text(s.map(|s| t!("{} · 记录于 {}", "{} · recorded {}", s.name, s.date)).unwrap_or_default());
                    ui.label(format!("{}", e.suppliers.len()));
                    ui.label(if used.contains(&e.pn) { "●" } else { "" });
                    if ui.small_button(tr("编辑", "Edit")).clicked() {
                        edit = Some(e.clone());
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(e) = edit {
            self.lib_edit_aliases = e.aliases.join(", ");
            self.lib_edit_orig = Some(e.pn.clone());
            self.lib_edit = Some(e);
        }
    }

    /// B4 零件库条目表单: 料号(可按分类自动给)/ 名称 / 分类 / 模板字段 / 别名 / 备注 / 供应商(首选、链接可打开、改价自动记日期)。
    fn lib_form(&mut self, ui: &mut egui::Ui, field_cols: &[pla_ui::bom_table::Col]) {
        let Some(mut e) = self.lib_edit.take() else { return };
        let cats: Vec<(String, String)> = self.lib.pn_rule.categories.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let mut save = false;
        let mut cancel = false;
        let mut delete = false;
        egui::Grid::new("lib-form").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
            ui.label(egui::RichText::new(tr("料号", "Part no.")).color(COL_MUTED));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut e.pn).desired_width(130.0));
                if ui.small_button(tr("按分类自动", "Auto by category")).clicked() {
                    e.pn = self.lib.next_pn_for(&self.bom_edits.project, &e.category);
                }
            });
            ui.end_row();
            ui.label(egui::RichText::new(tr("分类", "Category")).color(COL_MUTED));
            egui::ComboBox::from_id_salt("lib-cat").width(160.0).selected_text(cats.iter().find(|c| c.0 == e.category).map(|c| format!("{} {}", c.0, c.1)).unwrap_or(e.category.clone())).show_ui(ui, |ui| {
                for (k, v) in &cats {
                    if ui.selectable_label(e.category == *k, format!("{k} {v}")).clicked() {
                        e.category = k.clone();
                    }
                }
            });
            ui.end_row();
            ui.label(egui::RichText::new(tr("名称(采购名)", "Name (purchasing)")).color(COL_MUTED));
            ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(300.0));
            ui.end_row();
            for c in field_cols {
                ui.label(egui::RichText::new(&c.header).color(COL_MUTED));
                let mut v = e.fields.get(&c.id).cloned().unwrap_or_default();
                if ui.add(egui::TextEdit::singleline(&mut v).desired_width(300.0)).changed() {
                    if v.trim().is_empty() { e.fields.remove(&c.id); } else { e.fields.insert(c.id.clone(), v); }
                }
                ui.end_row();
            }
            ui.label(egui::RichText::new(tr("别名(逗号分隔)", "Aliases (comma-separated)")).color(COL_MUTED)).on_hover_text(tr("模型里的名字等, 用来自动建议链接", "Names used in models etc., for link suggestions"));
            ui.add(egui::TextEdit::singleline(&mut self.lib_edit_aliases).desired_width(300.0));
            ui.end_row();
            ui.label(egui::RichText::new(tr("备注", "Notes")).color(COL_MUTED));
            ui.add(egui::TextEdit::singleline(&mut e.notes).desired_width(300.0));
            ui.end_row();
        });
        // ---- 供应商 ----
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tr("供应商(选中的 = 首选, BOM 单价取它)", "Suppliers (the selected one is preferred; BOM price uses it)")).strong());
        let mut remove: Option<usize> = None;
        egui::ScrollArea::horizontal().id_salt("lib-sup-h").show(ui, |ui| {
            egui::Grid::new("lib-sup").num_columns(9).spacing([6.0, 3.0]).show(ui, |ui| {
                for h in ["", tr("名称", "Name"), tr("链接", "Link"), "", tr("单价", "Price"), tr("币种", "Currency"), tr("起订量", "MOQ"), tr("交期", "Lead time"), tr("记录日期", "Recorded")] {
                    ui.label(egui::RichText::new(h).color(COL_MUTED).size(11.0));
                }
                ui.label("");
                ui.end_row();
                for (i, s) in e.suppliers.iter_mut().enumerate() {
                    ui.radio_value(&mut e.preferred, i, "");
                    ui.add(egui::TextEdit::singleline(&mut s.name).desired_width(90.0));
                    ui.add(egui::TextEdit::singleline(&mut s.url).desired_width(140.0));
                    if ui.add_enabled(s.url.starts_with("http"), egui::Button::new("↗").small()).on_hover_text(tr("在浏览器里打开", "Open in the browser")).clicked() {
                        let _ = std::process::Command::new("cmd").args(["/C", "start", "", &s.url]).spawn();
                    }
                    if ui.add(egui::TextEdit::singleline(&mut s.price).desired_width(60.0)).changed() {
                        s.date = pla_ui::parts_lib::today();
                    }
                    ui.add(egui::TextEdit::singleline(&mut s.currency).desired_width(40.0).hint_text("CNY"));
                    ui.add(egui::TextEdit::singleline(&mut s.moq).desired_width(50.0));
                    ui.add(egui::TextEdit::singleline(&mut s.lead).desired_width(60.0));
                    ui.label(egui::RichText::new(&s.date).size(11.0).color(COL_MUTED));
                    if ui.small_button("×").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(i) = remove {
            e.suppliers.remove(i);
            if e.preferred >= e.suppliers.len() {
                e.preferred = 0;
            }
        }
        if ui.small_button(tr("＋ 供应商", "+ Supplier")).clicked() {
            e.suppliers.push(pla_ui::parts_lib::Supplier { currency: "CNY".into(), date: pla_ui::parts_lib::today(), ..Default::default() });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button(egui::RichText::new(tr("保存", "Save")).strong()).clicked() {
                save = true;
            }
            if ui.button(tr("取消", "Cancel")).clicked() {
                cancel = true;
            }
            if self.lib_edit_orig.is_some() && ui.button(egui::RichText::new(tr("删除条目", "Delete entry")).color(COL_RED)).clicked() {
                delete = true;
            }
        });
        if save {
            e.pn = e.pn.trim().to_string();
            if e.pn.is_empty() {
                e.pn = self.lib.next_pn_for(&self.bom_edits.project, &e.category);
            }
            let orig = self.lib_edit_orig.clone();
            if self.lib.find(&e.pn).is_some() && orig.as_deref() != Some(e.pn.as_str()) {
                self.set_notice((true, t!("料号 {} 已存在", "Part number {} already exists", e.pn)));
                self.lib_edit = Some(e);
                return;
            }
            e.aliases = self.lib_edit_aliases.split([',', '，']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let renamed = orig.as_ref().is_some_and(|o| *o != e.pn);
            if let Some(o) = orig.filter(|o| *o != e.pn) {
                // 改了料号: 库里换掉旧条目, 旧号记进 former(其它文档 / 旧表写着旧号也认得), 本文档的链接跟着改
                self.lib.remove(&o);
                if !e.former.contains(&o) {
                    e.former.push(o.clone());
                }
                for v in self.bom_edits.links.values_mut() {
                    if *v == o {
                        *v = e.pn.clone();
                    }
                }
                self.bom_edits_save();
                self.set_notice((false, t!("料号 {o} → {}: 旧号已记下, 链接旧号的行仍认得这个条目", "Part number {o} → {}: the old number is kept, rows linked to it still resolve", e.pn)));
            }
            let pn = e.pn.clone();
            self.lib.upsert(e);
            self.lib_save();
            self.lib_edit_orig = None;
            self.bom_cell_buf.clear();
            if !renamed {
                self.set_notice((false, t!("零件库已保存: {pn}", "Library saved: {pn}")));
            }
            return;
        }
        if cancel {
            self.lib_edit_orig = None;
            return;
        }
        if delete {
            self.lib_del_confirm = self.lib_edit_orig.clone();
        }
        self.lib_edit = Some(e);
        if let Some(pn) = self.lib_del_confirm.clone() {
            match confirm_popup(ui.ctx(), "lib-del", &t!("从零件库删除 {pn}?(链接它的 BOM 行回到原值)", "Delete {pn} from the library? (BOM rows linked to it fall back to their own values)")) {
                Some(true) => {
                    self.lib_del_confirm = None;
                    self.lib.remove(&pn);
                    self.lib_save();
                    self.lib_edit = None;
                    self.lib_edit_orig = None;
                    self.bom_cell_buf.clear();
                }
                Some(false) => self.lib_del_confirm = None,
                None => {}
            }
        }
    }

    /// 按当前模板 + 本文档编辑 + 零件库导出一份 CSV(含手动行与合计)。
    fn bom_export(&mut self, scope_label: &str, scan: &serde_json::Value) -> Result<std::path::PathBuf, String> {
        self.bom_template_tick();
        self.lib_tick();
        let (rows, totals) = pla_ui::bom_table::build(scan, &self.bom_tpl, &self.bom_edits, &self.lib);
        pla_ui::bom::export_text(&self.doc_key, scope_label, &pla_ui::bom_table::to_csv(&self.bom_tpl, &rows, &totals))
    }

    fn bom_scope_label(scope: &Option<String>) -> String {
        match scope {
            None => tr("当前可见", "Currently visible").to_string(),
            Some(s) => t!("配置 {s}", "Configuration {s}"),
        }
    }

    // ---------------------------------------------------------------- D1 出图
    /// 换文档 → 重读本文档的图设置; 收编译线程的结果。
    fn draw_tick(&mut self) {
        if self.draw_doc != self.doc_key {
            self.draw_doc = self.doc_key.clone();
            self.draw_sheets = pla_ui::drawing::load_in(&pla_ui::drawing::dir(&self.doc_key));
            self.draw_sel = None;
            self.draw_report = None;
            self.draw_png = None;
            self.draw_buf.clear();
            self.draw_queue.clear();
            self.draw_batch = None;
        }
        let got = match self.draw_job.as_ref().and_then(|j| j.rx.as_ref()) {
            Some(rx) => match rx.try_recv() {
                Ok(r) => Some(r),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(tr("编译线程意外退出", "The compile thread exited unexpectedly").to_string())),
            },
            None => None,
        };
        if let Some(r) = got {
            let key = self.draw_job.take().map(|j| j.key).unwrap_or_default();
            self.draw_compiled(&key, r);
        }
        // 批量: 上一张出完(或起不来)就接下一张; 排空了报一次结果
        while self.draw_job.is_none() {
            let Some((key, export, title)) = self.draw_queue.pop_front() else { break };
            self.draw_start(&key, export, title);
            if self.draw_job.is_none() {
                self.draw_failed(&key);
            }
        }
        if self.draw_job.is_none() && self.draw_queue.is_empty() {
            if let Some((n, failed)) = self.draw_batch.take() {
                let ok = n.saturating_sub(failed.len());
                let msg = if failed.is_empty() {
                    t!("批量出图完成: {ok} 张", "Batch drawing done: {ok} sheet(s)")
                } else {
                    t!("批量出图: {ok} 张完成, 失败 {}: {}", "Batch drawing: {ok} done, {} failed: {}", failed.len(), failed.join(", "))
                };
                self.set_notice((!failed.is_empty(), msg));
                // 出成了两张以上 → 顺手合订一份
                if ok >= 2 {
                    let keys = std::mem::take(&mut self.draw_batch_keys);
                    self.draw_book_start(&keys);
                }
            }
        }
        // D8: 上传线程的结果
        let got = self.draw_up_job.as_ref().and_then(|(_, rx)| match rx.try_recv() {
            Ok(r) => Some(r),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(tr("上传线程意外退出", "The upload thread exited unexpectedly").to_string())),
        });
        if let Some(r) = got {
            let key = self.draw_up_job.take().map(|j| j.0).unwrap_or_default();
            match r {
                Ok(None) => {}
                Ok(Some(up)) => {
                    if let Some(mut s) = self.draw_sheets.get(&key).cloned() {
                        let name = up.orig.clone();
                        let preview = !up.pages.is_empty();
                        s.uploaded = Some(up);
                        self.draw_sheets.upsert(s);
                        self.draw_save();
                        let msg = if preview {
                            t!("已上传 {name}: 这张图以上传的为准(已锁定, 不再生成)", "Uploaded {name}: this sheet now follows the upload (locked, no regeneration)")
                        } else {
                            t!("已上传 {name}(DWG / DXF 没有预览): 已锁定, 不再生成", "Uploaded {name} (no preview for DWG / DXF): locked, no regeneration")
                        };
                        self.set_notice((false, msg));
                        if self.draw_sel.as_deref() == Some(key.as_str()) {
                            self.draw_up_page = 0;
                            self.draw_load(&key);
                        }
                    }
                }
                Err(e) => self.set_notice((true, t!("上传图纸失败: {e}", "Uploading the drawing failed: {e}"))),
            }
        }
        let got = self.draw_book.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(r) => Some(r),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(tr("合订线程意外退出", "The combine thread exited unexpectedly").to_string())),
        });
        if let Some(r) = got {
            self.draw_book = None;
            match r {
                Ok(_) => {
                    let p = pla_ui::drawing::book_file(&pla_ui::drawing::dir(&self.doc_key), &self.doc_key);
                    self.set_notice((false, t!("已合订: {}", "Combined: {}", p.display())));
                }
                Err(e) => self.set_notice((true, t!("合订 PDF 失败: {e}", "Combining the PDF failed: {e}"))),
            }
        }
    }

    /// 按 keys 的顺序把出过的图(有 SVG 的)合成一份 PDF(后台线程)。
    fn draw_book_start(&mut self, keys: &[String]) {
        if self.draw_book.is_some() {
            return;
        }
        let Some(tools) = pla_ui::drawing::tools_dir() else { return };
        let dir = pla_ui::drawing::dir(&self.doc_key);
        // D8: 锁定的图用上传图纸的页, 其余用生成的 SVG
        let svgs: Vec<String> = keys.iter().filter_map(|k| self.draw_sheets.get(k)).flat_map(|s| pla_ui::drawing::book_pages(&dir, s)).collect();
        if svgs.is_empty() {
            self.set_notice((true, tr("还没有出过的图可合订", "No drawings to combine yet").to_string()));
            return;
        }
        let out = pla_ui::drawing::book_file(&dir, &self.doc_key);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(pla_ui::drawing::book(&tools, &out, &svgs));
        });
        self.draw_book = Some(rx);
    }

    /// 批量里有一张失败: 记下图名(单张出图时不记)。
    fn draw_failed(&mut self, key: &str) {
        let base = self.draw_sheets.get(key).map(|s| s.base.clone()).unwrap_or_else(|| key.to_string());
        if let Some((_, f)) = self.draw_batch.as_mut() {
            f.push(base);
        }
    }

    /// 零件第一次出图: 建一张图(图名 = 料号或名字, 重名加 -2…; 密度按材料猜); 已有的跟着 BOM 换实例。
    fn draw_ensure(&mut self, key: &str, title: &std::collections::BTreeMap<String, String>, member: &(String, String)) {
        if self.draw_sheets.get(key).is_none() {
            let want = pla_ui::drawing::base_name(title.get("pn").map(|s| s.as_str()).unwrap_or(""), title.get("name").map(|s| s.as_str()).unwrap_or(&member.1));
            let base = self.draw_sheets.unique_base(key, &want);
            let mut s = pla_ui::drawing::Sheet::new(key, &base, &member.0, &member.1);
            s.density = title.get("material").and_then(|m| pla_ui::drawing::guess_density(m));
            self.draw_sheets.upsert(s);
            self.draw_save();
        } else if let Some(mut s) = self.draw_sheets.get(key).cloned() {
            // 实例可能换了(删了重建): 跟着 BOM 走
            if !member.0.is_empty() && (s.member != member.0 || s.model_name != member.1) {
                s.member = member.0.clone();
                s.model_name = member.1.clone();
                self.draw_sheets.upsert(s);
                self.draw_save();
            }
        }
    }

    fn prefs_get(&mut self) -> &pla_ui::drawing::Prefs {
        self.draw_prefs.get_or_insert_with(pla_ui::drawing::Prefs::load)
    }

    fn prefs_set(&mut self, p: pla_ui::drawing::Prefs) {
        match p.save_to(&pla_ui::drawing::Prefs::path()) {
            Ok(()) => self.set_notice((false, tr("已存为默认(以后每张图都用)", "Saved as default (used for every drawing)").to_string())),
            Err(e) => self.set_notice((true, t!("保存偏好失败: {e}", "Saving preferences failed: {e}"))),
        }
        self.draw_prefs = Some(p);
    }

    fn draw_save(&mut self) {
        if let Err(e) = pla_ui::drawing::save_in(&pla_ui::drawing::dir(&self.doc_key), &self.draw_sheets) {
            self.set_notice((true, t!("出图设置保存失败: {e}", "Saving drawing settings failed: {e}")));
        }
    }

    /// 出一张图: 没有 STEP(或要求重新导出)→ 先让 Plasticity 导出, 回包后编译; 否则直接编译。
    fn draw_start(&mut self, key: &str, export: bool, title: std::collections::BTreeMap<String, String>) {
        if self.draw_job.is_some() {
            return;
        }
        let Some(s) = self.draw_sheets.get(key).cloned() else { return };
        // D8: 以上传图纸为准的图不生成(用户解锁才行)
        if s.locked() {
            self.set_notice((true, t!("{} 以上传的图纸为准(已锁定), 要重新生成先解锁", "{} follows the uploaded drawing (locked); unlock it to regenerate", s.base)));
            return;
        }
        if pla_ui::drawing::tools_dir().is_none() || pla_ui::drawing::freecad_bin().is_none() {
            self.set_notice((true, tr("出图需要 tools/drawing 编译器和 FreeCAD 1.1", "Drawing needs the tools/drawing compiler and FreeCAD 1.1").to_string()));
            return;
        }
        let dir = pla_ui::drawing::dir(&self.doc_key);
        let step = pla_ui::drawing::file(&dir, &s, "step");
        if pla_ui::drawing::is_asm(key) {
            // 装配图: 每次都重新导出整个范围(哪个零件动了都会影响装配)
            let ids: Vec<String> = self.draw_asm.get(key).into_iter().flatten().flat_map(|l| l.ids.iter().cloned()).collect();
            if ids.is_empty() {
                self.set_notice((true, tr("BOM 里没有零件(先扫描 BOM)", "No parts in the BOM (scan the BOM first)").to_string()));
                return;
            }
            let _ = export;
            self.cdp.send(pla_bridge::cdp::CdpCmd::ExportStep { path: step.display().to_string(), ids, format: Some("AP214".into()) });
            self.draw_job = Some(DrawJob { key: key.to_string(), stage: DrawStage::Export, rx: None, t0: Instant::now(), title });
            return;
        }
        if export || !step.exists() {
            if s.member.is_empty() {
                self.set_notice((true, tr("这张图没有对应的模型实体(重新扫描 BOM)", "No model solid for this drawing (rescan the BOM)").to_string()));
                return;
            }
            self.cdp.send(pla_bridge::cdp::CdpCmd::ExportStep { path: step.display().to_string(), ids: vec![s.member.clone()], format: Some("AP214".into()) });
            self.draw_job = Some(DrawJob { key: key.to_string(), stage: DrawStage::Export, rx: None, t0: Instant::now(), title });
        } else {
            self.draw_compile(key, title);
        }
    }

    /// STEP 导出回包(只认出图面板发起的)。
    fn draw_exported(&mut self, result: Result<serde_json::Value, String>) {
        let Some(job) = self.draw_job.as_ref().filter(|j| j.stage == DrawStage::Export) else { return };
        let (key, title) = (job.key.clone(), job.title.clone());
        match result {
            Ok(v) => {
                if let Some(mut s) = self.draw_sheets.get(&key).cloned() {
                    // 零件在组里时 STEP 会含全部可见实体 → 记下这个零件的体积 / 面积, 编译器按它挑
                    if let Some(p) = v["parts"].as_array().filter(|a| a.len() == 1 && !pla_ui::drawing::is_asm(&key)).and_then(|a| a.first()) {
                        s.volume = p["volume"].as_f64();
                        s.area = p["area"].as_f64();
                    }
                    s.exported = pla_ui::parts_lib::today();
                    self.draw_sheets.upsert(s);
                    self.draw_save();
                }
                self.draw_job = None;
                self.draw_compile(&key, title);
                if self.draw_job.is_none() {
                    self.draw_failed(&key);
                }
            }
            Err(e) => {
                self.draw_job = None;
                self.draw_failed(&key);
                self.set_notice((true, t!("导出 STEP 失败: {e}", "STEP export failed: {e}")));
            }
        }
    }

    fn draw_compile(&mut self, key: &str, title: std::collections::BTreeMap<String, String>) {
        let (Some(s), Some(tools)) = (self.draw_sheets.get(key).cloned(), pla_ui::drawing::tools_dir()) else { return };
        let dir = pla_ui::drawing::dir(&self.doc_key);
        // D6: 偏好(标题栏默认字段、技术要求)垫在底下, BOM / 本图覆盖
        let (title, mut run) = self.prefs_get().apply(&s, &title);
        run.title = title;
        let spec = if pla_ui::drawing::is_asm(key) {
            pla_ui::drawing::asm_spec(&run, &dir, self.draw_asm.get(key).map(|v| v.as_slice()).unwrap_or(&[]))
        } else {
            let mut v = pla_ui::drawing::spec(&run, &dir, Some(&pla_ui::dims::path(&self.doc_key)));
            // D4: 装配图上记下的配合 → 本件孔 / 轴的公差
            let fits: Vec<serde_json::Value> = self.draw_sheets.sheets.iter().filter(|a| pla_ui::drawing::is_asm(&a.key)).flat_map(|a| pla_ui::drawing::part_fits(&a.fits, key)).collect();
            if !fits.is_empty() {
                v["fits"] = serde_json::json!(fits);
            }
            v
        };
        let spec_file = pla_ui::drawing::file(&dir, &s, "spec.json");
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&spec_file, serde_json::to_vec_pretty(&spec).unwrap_or_default())) {
            self.set_notice((true, t!("写出图方案失败: {e}", "Writing the drawing spec failed: {e}")));
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(pla_ui::drawing::compile(&tools, &spec_file));
        });
        self.draw_job = Some(DrawJob { key: key.to_string(), stage: DrawStage::Compile, rx: Some(rx), t0: Instant::now(), title: Default::default() });
    }

    fn draw_compiled(&mut self, key: &str, r: Result<String, String>) {
        match r {
            Ok(_) => {
                if let Some(mut s) = self.draw_sheets.get(key).cloned() {
                    s.compiled = pla_ui::parts_lib::today();
                    s.model_changed = false;
                    let base = s.base.clone();
                    self.draw_sheets.upsert(s);
                    self.draw_save();
                    self.set_notice((false, t!("已出图 {base}", "Drawing {base} done")));
                }
                // 批量时别把预览换成后台出的那张
                if self.draw_sel.as_deref() == Some(key) {
                    self.draw_load(key);
                }
            }
            Err(e) => {
                self.draw_failed(key);
                self.set_notice((true, t!("出图失败: {e}", "Drawing failed: {e}")));
            }
        }
    }

    /// 读选中图的报告 + 预览。
    fn draw_load(&mut self, key: &str) {
        self.draw_report = None;
        self.draw_png = None;
        self.draw_up_view = None;
        let Some(s) = self.draw_sheets.get(key).cloned() else { return };
        let dir = pla_ui::drawing::dir(&self.doc_key);
        if let Some(r) = pla_ui::drawing::load_report(&dir, &s) {
            self.draw_report = Some((key.to_string(), r));
        }
        let gen = pla_ui::drawing::file(&dir, &s, "png");
        // D8: 锁定(或还没生成过)→ 预览上传的图纸
        if let Some(u) = s.uploaded.as_ref().filter(|u| !u.pages.is_empty() && (u.locked || !gen.exists())) {
            let i = self.draw_up_page.min(u.pages.len() - 1);
            if let Ok(b) = std::fs::read(dir.join(&u.pages[i].png)) {
                self.draw_png = Some((png_uri("drawup", &format!("{}-{i}", s.base), &b), b.into()));
                self.draw_up_view = Some((i, u.pages.len(), u.pages[i].mm));
                return;
            }
        }
        if let Ok(b) = std::fs::read(gen) {
            self.draw_png = Some((png_uri("draw", &s.base, &b), b.into()));
        }
    }

    /// D8: 上传图纸(后台线程: 没给文件就弹选择框 → 复制进出图目录 → 转预览 PNG)。
    fn draw_upload_start(&mut self, key: &str, src: Option<std::path::PathBuf>) {
        if self.draw_up_job.is_some() {
            return;
        }
        let Some(s) = self.draw_sheets.get(key).cloned() else { return };
        if let Some(p) = &src {
            if pla_ui::drawing::upload_kind(p).is_none() {
                self.set_notice((true, tr("只支持 PDF / PNG / JPG / DWG / DXF", "Only PDF / PNG / JPG / DWG / DXF are supported").into()));
                return;
            }
        }
        let dir = pla_ui::drawing::dir(&self.doc_key);
        let tools = pla_ui::drawing::tools_dir();
        let today = pla_ui::parts_lib::today();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let r = match src.or_else(pla_ui::drawing::pick_upload_file) {
                None => Ok(None),
                Some(p) => pla_ui::drawing::import_upload(&dir, &s, &p, tools.as_deref(), &today).map(Some),
            };
            let _ = tx.send(r);
        });
        self.draw_up_job = Some((key.to_string(), rx));
    }

    /// 增量更新(每次 BOM 扫描回包): 记 key → 实例 id, 找出"同一实体换了 key"的零件, 把 BOM 填写 / 链接 / 排除和出图设置迁过去。
    /// 以前没有 ids 记录的旧图: 用图上记的实体 id(member)补上, 这样早先因改模型变成"不在当前范围"的图也能接回来。
    fn identity_migrate(&mut self, scan: &serde_json::Value) {
        let dir = pla_ui::drawing::dir(&self.doc_key);
        let mut sheets = if self.draw_doc == self.doc_key { self.draw_sheets.clone() } else { pla_ui::drawing::load_in(&dir) };
        let mut ids = pla_ui::identity::load(&self.doc_key);
        for s in sheets.sheets.iter().filter(|s| !pla_ui::drawing::is_asm(&s.key) && !s.member.is_empty()) {
            ids.entry(s.key.clone()).or_insert_with(|| vec![s.member.clone()]);
        }
        let renames = pla_ui::identity::track(&mut ids, scan);
        if let Err(e) = pla_ui::identity::save(&self.doc_key, &ids) {
            self.set_notice((true, t!("零件身份记录保存失败: {e}", "Saving part identities failed: {e}")));
        }
        if renames.is_empty() {
            return;
        }
        let mut edits = if self.bom_edits_doc == self.doc_key { self.bom_edits.clone() } else { pla_ui::bom_table::load_edits(&self.doc_key) };
        let (mut nb, mut nd) = (0, 0);
        for (old, new) in &renames {
            nb += usize::from(edits.rename_key(old, new));
            nd += usize::from(sheets.rename_key(&dir, old, new));
            if self.draw_sel.as_deref() == Some(old.as_str()) {
                self.draw_sel = Some(new.clone());
            }
        }
        if nb > 0 {
            if let Err(e) = pla_ui::bom_table::save_edits(&self.doc_key, &edits) {
                self.set_notice((true, t!("BOM 填写保存失败: {e}", "Saving BOM entries failed: {e}")));
            }
            if self.bom_edits_doc == self.doc_key {
                self.bom_edits = edits;
                self.bom_cell_buf.clear();
            }
        }
        if nd > 0 {
            if let Err(e) = pla_ui::drawing::save_in(&dir, &sheets) {
                self.set_notice((true, t!("出图设置保存失败: {e}", "Saving drawing settings failed: {e}")));
            }
            if self.draw_doc == self.doc_key {
                self.draw_sheets = sheets;
                if let Some(k) = self.draw_sel.clone() {
                    self.draw_load(&k);
                }
            }
        }
        if nb + nd > 0 {
            self.set_notice((
                false,
                t!(
                    "{} 个零件改过模型: 沿用了原来的 BOM 填写({nb})和出图设置({nd}), 图标成「待重出」",
                    "{} part(s) changed shape: kept their BOM entries ({nb}) and drawing settings ({nd}); drawings marked for redraw",
                    renames.len()
                ),
            ));
        }
    }

    // ---------------------------------------------------------------- MCP 桥(pla-mcp)
    /// BOM 行 → 每个零件: 标题栏字段(pn / name / material / finish / qty)、第一个实例、给 MCP 的摘要。
    fn bom_parts(&self) -> Vec<(std::collections::BTreeMap<String, String>, (String, String), pla_ui::mcp_bridge::PartInfo)> {
        let Some(Ok(v)) = &self.bom_result else { return Vec::new() };
        let (rows, _) = pla_ui::bom_table::build(v, &self.bom_tpl, &self.bom_edits, &self.lib);
        let cols = &self.bom_tpl.columns;
        let mut out = Vec::new();
        for r in rows.iter().filter(|r| !r.manual) {
            let get = |id: &str| cols.iter().position(|c| c.id == id).map(|i| r.cells[i].trim().to_string()).unwrap_or_default();
            let line = v["lines"].as_array().into_iter().flatten().find(|l| l["key"].as_str() == Some(r.key.as_str()));
            let m = line.and_then(|l| l["members"].as_array()).and_then(|a| a.first());
            let member = m.map(|m| (m["id"].as_str().unwrap_or("").to_string(), m["name"].as_str().unwrap_or("").to_string())).unwrap_or_default();
            let qty = line.and_then(|l| l["qty"].as_u64()).unwrap_or(1);
            let mut title = std::collections::BTreeMap::new();
            for id in ["pn", "name", "material", "finish"] {
                let x = get(id);
                if !x.is_empty() {
                    title.insert(id.to_string(), x);
                }
            }
            title.insert("qty".into(), qty.to_string());
            let name = if get("name").is_empty() { member.1.clone() } else { get("name") };
            let bought = line.is_some_and(|l| pla_ui::drawing::member_bought(&l["members"], &self.draw_sheets.purchased)) || pla_ui::drawing::looks_standard(&name, &get("pn"));
            let info = pla_ui::mcp_bridge::PartInfo {
                key: r.key.clone(),
                pn: get("pn"),
                name,
                qty,
                material: get("material"),
                supplier: pla_ui::release::process_of(&self.bom_edits, &r.key).to_string(),
                bought,
            };
            out.push((title, member, info));
        }
        out
    }

    fn mcp_tick(&mut self) {
        if self.mcp_checked.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
            return;
        }
        self.mcp_checked = Some(Instant::now());
        self.bom_template_tick();
        let parts = self.bom_parts();
        let mut st = pla_ui::mcp_bridge::State {
            doc_key: self.doc_key.clone(),
            drawings_dir: pla_ui::drawing::dir(&self.doc_key).display().to_string(),
            parts: parts.iter().map(|p| p.2.clone()).collect(),
            selected: self.draw_sel.clone(),
            drawing: self.draw_job.as_ref().map(|j| j.key.clone()),
            queued: self.draw_queue.len(),
            ts: 0,
        };
        let ser = serde_json::to_string(&st).unwrap_or_default();
        if ser != self.mcp_last || self.mcp_written.map_or(true, |t| t.elapsed() > Duration::from_secs(5)) {
            st.ts = pla_ui::mcp_bridge::now();
            if pla_ui::mcp_bridge::write_state(&st).is_ok() {
                self.mcp_last = ser;
                self.mcp_written = Some(Instant::now());
            }
        }
        for (id, req) in pla_ui::mcp_bridge::take_requests() {
            let resp = self.mcp_handle(&req, &parts);
            pla_ui::mcp_bridge::respond(&id, &resp);
        }
    }

    /// 处理一个 pla-mcp 请求: apply(改设置, 可多张)/ draw(排队出图)/ rule / show。
    fn mcp_handle(&mut self, req: &serde_json::Value, parts: &[(std::collections::BTreeMap<String, String>, (String, String), pla_ui::mcp_bridge::PartInfo)]) -> serde_json::Value {
        use serde_json::json;
        let find = |k: &str| parts.iter().find(|p| p.2.key == k);
        let keys: Vec<String> = req["keys"].as_array().into_iter().flatten().filter_map(|k| k.as_str().map(String::from)).collect();
        match req["op"].as_str().unwrap_or("") {
            "apply" => {
                let patch: pla_ui::draw_agent::Patch = match serde_json::from_value(req["patch"].clone()) {
                    Ok(p) => p,
                    Err(e) => return json!({"ok": false, "error": e.to_string()}),
                };
                let dir = pla_ui::drawing::dir(&self.doc_key);
                let mut results = Vec::new();
                let mut redraw = Vec::new();
                for k in &keys {
                    if let Some(p) = find(k) {
                        self.draw_ensure(k, &p.0, &p.1);
                    }
                    let Some(mut s) = self.draw_sheets.get(k).cloned() else {
                        results.push(json!({"part": k, "error": "no such part"}));
                        continue;
                    };
                    if s.locked() {
                        results.push(json!({"part": s.base, "skipped": "locked to an uploaded drawing"}));
                        continue;
                    }
                    let rep = pla_ui::drawing::load_report(&dir, &s);
                    match pla_ui::draw_agent::apply(&mut s, &patch, rep.as_ref()) {
                        Ok(changes) => {
                            s.edits += 1;
                            pla_ui::draw_agent::log_applied(&dir, &s, &format!("MCP: {}", patch.summary), &patch, &changes);
                            results.push(json!({"part": s.base, "changes": changes}));
                            self.draw_sheets.upsert(s);
                            redraw.push(k.clone());
                        }
                        Err(e) => results.push(json!({"part": s.base, "error": e})),
                    }
                }
                self.draw_save();
                if let Some(k) = self.draw_sel.clone() {
                    self.draw_buf.clear();
                    self.draw_load(&k);
                }
                if !redraw.is_empty() {
                    self.set_notice((false, t!("外部助手改了 {} 张图的设置", "An external assistant changed {} drawing(s)", redraw.len())));
                }
                if req["redraw"].as_bool().unwrap_or(false) {
                    self.mcp_queue_draw(&redraw, false, parts);
                }
                json!({"ok": true, "results": results})
            }
            "draw" => {
                let skipped = self.mcp_queue_draw(&keys, req["export"].as_bool().unwrap_or(false), parts);
                json!({"ok": true, "skipped": skipped})
            }
            // 还没扫过 BOM(零件清单空): 按当前范围扫一次(只读)
            "scan" => {
                if !self.bom_busy {
                    let s = self.bom_scope.clone();
                    self.bom_scan(s);
                }
                json!({"ok": true})
            }
            "rule" => {
                let r = req["rule"].as_str().unwrap_or("").trim().to_string();
                if r.is_empty() {
                    return json!({"ok": false, "error": "empty rule"});
                }
                let mut p = self.prefs_get().clone();
                if !p.agent_rules.contains(&r) {
                    p.agent_rules.push(r);
                }
                self.prefs_set(p);
                json!({"ok": true})
            }
            "show" => {
                let k = req["key"].as_str().unwrap_or("").to_string();
                let Some(p) = find(&k) else { return json!({"ok": false, "error": "no such part"}) };
                self.draw_ensure(&k, &p.0, &p.1);
                self.show_snapshots = false;
                self.close_cfg_group();
                self.show_bom = true;
                self.bom_tab = 2;
                self.draw_sel = Some(k.clone());
                self.draw_buf.clear();
                self.draw_up_page = 0;
                self.draw_load(&k);
                json!({"ok": true})
            }
            op => json!({"ok": false, "error": format!("unknown op {op}")}),
        }
    }

    /// 排队出图(同批量出图的队列); 锁定的跳过。返回跳过的图名。
    fn mcp_queue_draw(&mut self, keys: &[String], export: bool, parts: &[(std::collections::BTreeMap<String, String>, (String, String), pla_ui::mcp_bridge::PartInfo)]) -> Vec<String> {
        let mut skipped = Vec::new();
        let mut n = 0;
        for k in keys {
            let Some(p) = parts.iter().find(|p| p.2.key == *k) else {
                skipped.push(format!("{k} (not in the current BOM scope)"));
                continue;
            };
            self.draw_ensure(k, &p.0, &p.1);
            let Some(s) = self.draw_sheets.get(k) else { continue };
            if s.locked() {
                skipped.push(format!("{} (locked)", s.base));
                continue;
            }
            if self.draw_queue.iter().any(|q| &q.0 == k) || self.draw_job.as_ref().is_some_and(|j| &j.key == k) {
                continue;
            }
            let mut title = p.0.clone();
            title.extend(s.title.clone());
            self.draw_queue.push_back((k.clone(), export, title));
            n += 1;
        }
        if n > 0 && self.draw_batch.is_none() {
            self.draw_batch = Some((n, Vec::new()));
            self.draw_batch_keys = Vec::new();
        }
        skipped
    }

    // ---------------------------------------------------------------- 外发包
    /// 生产窗口「外发」标签: 供应商(要哪些文件 / BOM 列)→ 零件归哪家 → 每家的加工 BOM(自有列可填)→ 生成外发包。
    fn rel_ui(&mut self, ui: &mut egui::Ui) {
        use pla_ui::release::{self as rel, Supplier, BUY, FILE_KINDS, PROCESS_COL};
        if let Some(r) = self.rel_job.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.rel_job = None;
            match r {
                Ok(p) => {
                    self.set_notice((false, t!("外发包已生成: {}", "Release package written: {}", p.display())));
                    self.rel_last = Some(p);
                }
                Err(e) => self.set_notice((true, t!("生成外发包失败: {e}", "Writing the release package failed: {e}"))),
            }
        }
        let Some(Ok(scan)) = self.bom_result.clone() else {
            ui.label(egui::RichText::new(tr("先扫描 BOM(外发按当前 BOM 范围出)", "Scan the BOM first (the release follows the current BOM scope)")).color(COL_MUTED));
            if ui.add_enabled(!self.bom_busy, egui::Button::new(tr("扫描", "Scan"))).clicked() {
                let s = self.bom_scope.clone();
                self.bom_scan(s);
            }
            return;
        };
        let (rows, totals) = pla_ui::bom_table::build(&scan, &self.bom_tpl, &self.bom_edits, &self.lib);
        let ddir = pla_ui::drawing::dir(&self.doc_key);
        let sheets = if self.draw_doc == self.doc_key { self.draw_sheets.clone() } else { pla_ui::drawing::load_in(&ddir) };
        let col_of = |id: &str| self.bom_tpl.columns.iter().position(|c| c.id == id);
        let (ci_name, ci_pn, ci_qty) = (col_of("name"), col_of("pn"), col_of("total_qty").or(col_of("per_set")));
        let cell = |r: &pla_ui::bom_table::Row, i: Option<usize>| i.map(|i| r.cells[i].clone()).unwrap_or_default();
        let parts: Vec<&pla_ui::bom_table::Row> = rows.iter().filter(|r| !r.manual).collect();
        let mut sups_dirty = false;
        let mut edits_dirty = false;
        if self.rel_sel >= self.suppliers.list.len() {
            self.rel_sel = 0;
        }
        // ---- 供应商 ----
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(tr("供应商", "Suppliers")).color(COL_MUTED));
            for (i, s) in self.suppliers.list.iter().enumerate() {
                let n = parts.iter().filter(|r| rel::process_of(&self.bom_edits, &r.key) == s.id).count();
                if ui.selectable_label(self.rel_sel == i, format!("{} ({n})", s.name)).clicked() {
                    self.rel_sel = i;
                    self.rel_del_confirm = false;
                }
            }
            if ui.small_button(tr("＋ 供应商", "＋ Supplier")).clicked() {
                let id = self.suppliers.next_id();
                self.suppliers.list.push(Supplier { id, name: tr("新供应商", "New supplier").into(), files: vec!["pdf".into(), "step".into()], columns: vec!["pn".into(), "name".into(), "total_qty".into()], extra: vec![] });
                self.rel_sel = self.suppliers.list.len() - 1;
                sups_dirty = true;
            }
        });
        let master: Vec<(String, String)> = self.bom_tpl.columns.iter().map(|c| (c.id.clone(), c.header.clone())).collect();
        let mut delete_sup = false;
        if let Some(sp) = self.suppliers.list.get_mut(self.rel_sel) {
            egui::CollapsingHeader::new(t!("「{}」的设置: 要哪些文件 / BOM 列", "\"{}\" settings: files / BOM columns", sp.name)).id_salt("rel-sup-set").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tr("名称", "Name")).color(COL_MUTED));
                    sups_dirty |= ui.add(egui::TextEdit::singleline(&mut sp.name).desired_width(160.0)).lost_focus();
                    ui.label(egui::RichText::new(tr("文件", "Files")).color(COL_MUTED));
                    for k in FILE_KINDS {
                        let mut on = sp.files.iter().any(|f| f == k);
                        if ui.checkbox(&mut on, k.to_uppercase()).changed() {
                            sp.files.retain(|f| f != k);
                            if on {
                                sp.files.push(k.to_string());
                            }
                            sups_dirty = true;
                        }
                    }
                });
                ui.label(egui::RichText::new(tr("主 BOM 的列(勾上的进这家的 BOM):", "Columns from the main BOM (ticked ones go into this supplier's BOM):")).size(11.0).color(COL_MUTED));
                ui.horizontal_wrapped(|ui| {
                    for (id, h) in &master {
                        let mut on = sp.columns.contains(id);
                        if ui.checkbox(&mut on, h).changed() {
                            // 保持主表顺序
                            let mut set: Vec<String> = sp.columns.iter().filter(|c| *c != id).cloned().collect();
                            if on {
                                set.push(id.clone());
                            }
                            sp.columns = master.iter().filter(|(m, _)| set.contains(m)).map(|(m, _)| m.clone()).collect();
                            sups_dirty = true;
                        }
                    }
                });
                ui.label(egui::RichText::new(tr("这家自己的列(值按零件填, 在下面的表里改):", "This supplier's own columns (filled per part in the table below):")).size(11.0).color(COL_MUTED));
                let mut remove: Option<usize> = None;
                ui.horizontal_wrapped(|ui| {
                    for (i, c) in sp.extra.iter_mut().enumerate() {
                        sups_dirty |= ui.add(egui::TextEdit::singleline(&mut c.header).desired_width(90.0)).lost_focus();
                        if ui.small_button("✕").clicked() {
                            remove = Some(i);
                        }
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.rel_new_col).hint_text(tr("新列名", "New column")).desired_width(90.0));
                    if ui.add_enabled(!self.rel_new_col.trim().is_empty(), egui::Button::new(tr("加列", "Add")).small()).clicked() {
                        let header = std::mem::take(&mut self.rel_new_col).trim().to_string();
                        let id = (1..).map(|n| format!("{}_x{n}", sp.id)).find(|id| !sp.extra.iter().any(|c| &c.id == id)).unwrap_or_default();
                        sp.extra.push(pla_ui::bom_table::Col { id, header, source: Some("field".into()), formula: None, default: None, sum: false });
                        sups_dirty = true;
                    }
                });
                if let Some(i) = remove {
                    sp.extra.remove(i);
                    sups_dirty = true;
                }
                let b = egui::Button::new(if self.rel_del_confirm { tr("确认删除这家?", "Confirm delete?") } else { tr("删除这家供应商", "Delete supplier") }).small();
                if ui.add(b).clicked() {
                    if self.rel_del_confirm {
                        delete_sup = true;
                    }
                    self.rel_del_confirm = !self.rel_del_confirm;
                }
            });
        }
        if delete_sup {
            self.suppliers.list.remove(self.rel_sel);
            self.rel_sel = 0;
            self.rel_del_confirm = false;
            sups_dirty = true;
        }
        // ---- 零件归哪家 ----
        let cur = self.suppliers.list.get(self.rel_sel).cloned();
        let names: Vec<(String, String)> = std::iter::once((String::new(), tr("(未分配)", "(unassigned)").to_string()))
            .chain(std::iter::once((BUY.to_string(), tr("外购 / 不发", "Bought / not sent").to_string())))
            .chain(self.suppliers.list.iter().map(|s| (s.id.clone(), s.name.clone())))
            .collect();
        let unassigned = parts.iter().filter(|r| rel::process_of(&self.bom_edits, &r.key).is_empty()).count();
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(t!("零件归哪家({unassigned} 个未分配)", "Who makes each part ({unassigned} unassigned)")).strong());
            if let Some(c) = &cur {
                if ui.add_enabled(unassigned > 0, egui::Button::new(t!("未分配的 → {}", "Unassigned → {}", c.name)).small()).clicked() {
                    for r in &parts {
                        if rel::process_of(&self.bom_edits, &r.key).is_empty() {
                            self.bom_edits.set_cell(&r.key, PROCESS_COL, &c.id, "");
                        }
                    }
                    edits_dirty = true;
                }
            }
        });
        egui::ScrollArea::vertical().id_salt("rel-assign").max_height(200.0).show(ui, |ui| {
            egui::Grid::new("rel-assign-grid").striped(true).num_columns(5).show(ui, |ui| {
                for h in [tr("名称", "Name"), tr("料号", "P/N"), tr("数量", "Qty"), tr("归哪家", "Supplier"), tr("图纸", "Drawing")] {
                    ui.label(egui::RichText::new(h).color(COL_MUTED));
                }
                ui.end_row();
                for r in &parts {
                    ui.label(cell(r, ci_name));
                    ui.label(cell(r, ci_pn));
                    ui.label(cell(r, ci_qty));
                    let p = rel::process_of(&self.bom_edits, &r.key).to_string();
                    let shown = names.iter().find(|(id, _)| *id == p).map(|(_, n)| n.clone()).unwrap_or_else(|| p.clone());
                    egui::ComboBox::from_id_salt(("rel-proc", &r.key)).width(110.0).selected_text(shown).show_ui(ui, |ui| {
                        for (id, n) in &names {
                            if ui.selectable_label(*id == p, n).clicked() && *id != p {
                                self.bom_edits.set_cell(&r.key, PROCESS_COL, id, "");
                                edits_dirty = true;
                            }
                        }
                    });
                    let st = match sheets.get(&r.key) {
                        Some(s) if s.locked() => tr("上传图纸", "uploaded").to_string(),
                        Some(s) if s.model_changed => tr("模型改过, 待重出", "model changed").to_string(),
                        Some(s) if !s.compiled.is_empty() => t!("已出 {}", "drawn {}", s.compiled),
                        _ => tr("没出图", "not drawn").to_string(),
                    };
                    ui.label(egui::RichText::new(st).size(11.0).color(COL_MUTED));
                    ui.end_row();
                }
            });
        });
        // ---- 当前这家的加工 BOM(自有列可填) ----
        let mut packs_preview: Vec<(String, usize, usize)> = Vec::new();
        if let Some(c) = &cur {
            let (h, t, keys) = rel::supplier_table(c, &self.bom_tpl, &rows, &self.bom_edits);
            ui.separator();
            ui.label(egui::RichText::new(t!("「{}」的加工 BOM({} 行)", "\"{}\" BOM ({} rows)", c.name, t.len())).strong());
            let n_master = 1 + c.columns.iter().filter(|id| self.bom_tpl.columns.iter().any(|m| &m.id == *id)).count();
            egui::ScrollArea::both().id_salt("rel-sup-bom").max_height(200.0).show(ui, |ui| {
                egui::Grid::new("rel-sup-grid").striped(true).num_columns(h.len()).show(ui, |ui| {
                    for x in &h {
                        ui.label(egui::RichText::new(x).color(COL_MUTED));
                    }
                    ui.end_row();
                    for (ri, row) in t.iter().enumerate() {
                        for (ci, v) in row.iter().enumerate() {
                            if ci < n_master {
                                ui.label(v);
                            } else {
                                let col = &c.extra[ci - n_master];
                                let mut buf = v.clone();
                                if ui.add(egui::TextEdit::singleline(&mut buf).desired_width(100.0)).changed() {
                                    self.bom_edits.set_cell(&keys[ri], &col.id, &buf, col.default.as_deref().unwrap_or(""));
                                    edits_dirty = true;
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
            });
        }
        // ---- 生成 ----
        let root = pla_ui::release::STORE.dir(&self.doc_key);
        let ver = rel::next_version(&root);
        let mut packs: Vec<rel::Pack> = Vec::new();
        for s in &self.suppliers.list {
            let (h, t, keys) = rel::supplier_table(s, &self.bom_tpl, &rows, &self.bom_edits);
            if t.is_empty() {
                continue;
            }
            let mut files = Vec::new();
            let mut missing = Vec::new();
            for (k, row) in keys.iter().zip(&t) {
                let (f, m) = rel::part_files(&ddir, sheets.get(k), &s.files);
                files.extend(f);
                if !m.is_empty() {
                    missing.push((row.get(2).cloned().unwrap_or_default(), m));
                }
            }
            packs_preview.push((s.name.clone(), t.len(), missing.len()));
            packs.push(rel::Pack { supplier: s.clone(), csv: rel::table_csv(&h, &t), files, missing });
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            for (n, rows_n, miss) in &packs_preview {
                let txt = if *miss > 0 { t!("{n}: {rows_n} 件, {miss} 件缺文件", "{n}: {rows_n} parts, {miss} missing files") } else { t!("{n}: {rows_n} 件", "{n}: {rows_n} parts") };
                ui.label(egui::RichText::new(txt).color(if *miss > 0 { COL_YELLOW } else { COL_MUTED }));
            }
        });
        let missing_all: Vec<String> = packs.iter().flat_map(|p| p.missing.iter().map(move |(n, m)| format!("{} · {n}: {}", p.supplier.name, m.join("/")))).collect();
        if !missing_all.is_empty() {
            egui::CollapsingHeader::new(t!("缺的文件({})", "Missing files ({})", missing_all.len())).id_salt("rel-missing").show(ui, |ui| {
                ui.label(egui::RichText::new(tr("没出图 / 模型改过没重出的零件, 先到「出图」标签出图(「出未出的」会一起出)", "Parts not drawn or changed since: draw them in the Drawings tab first (\"Draw undrawn\" covers both)")).size(11.0).color(COL_MUTED));
                for m in &missing_all {
                    ui.label(egui::RichText::new(m).size(11.0));
                }
            });
        }
        ui.horizontal(|ui| {
            let can = self.rel_job.is_none() && !packs.is_empty();
            let tip = if missing_all.is_empty() { tr("总BOM + 每家的 BOM 和文件 + 每家一个 zip + 版本记录", "Main BOM + each supplier's BOM and files + one zip per supplier + a version record") } else { tr("有缺的文件也能生成, 缺的记在 manifest.json 里", "Can be written with files missing; they are listed in manifest.json") };
            if ui.add_enabled(can, egui::Button::new(t!("生成外发包 {ver}", "Write release {ver}"))).on_hover_text(tip).clicked() {
                let total = pla_ui::bom_table::to_csv(&self.bom_tpl, &rows, &totals);
                let date = pla_ui::parts_lib::today();
                let (tx, rx) = std::sync::mpsc::channel();
                let (root2, ver2) = (root.clone(), ver.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(rel::write(&root2, &ver2, &date, &total, &packs));
                });
                self.rel_job = Some(rx);
            }
            if self.rel_job.is_some() {
                ui.label(egui::RichText::new(tr("生成中…", "Writing…")).color(COL_ACCENT));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
            }
            if ui.button(tr("打开外发文件夹", "Open release folder")).clicked() {
                let p = self.rel_last.clone().unwrap_or(root.clone());
                let _ = std::fs::create_dir_all(&p);
                self.open_path(&p);
            }
        });
        if sups_dirty {
            if let Err(e) = self.suppliers.save() {
                self.set_notice((true, t!("供应商设置保存失败: {e}", "Saving suppliers failed: {e}")));
            }
        }
        if edits_dirty {
            self.bom_edits_save();
        }
    }

    // ---------------------------------------------------------------- D5 出图助手
    /// 收后台一轮的结果(换了零件就丢掉)。
    fn agent_tick(&mut self) {
        let got = self.agent_job.as_ref().and_then(|(_, rx)| match rx.try_recv() {
            Ok(r) => Some(r),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(tr("助手线程意外退出", "The assistant thread exited unexpectedly").to_string())),
        });
        let Some(r) = got else { return };
        let key = self.agent_job.take().map(|j| j.0).unwrap_or_default();
        if key != self.agent_chat.key {
            return;
        }
        match r {
            Ok(out) => {
                self.agent_chat.history.extend(out.history_add);
                self.agent_chat.lines.extend(out.lines);
                self.agent_chat.open_tools.extend(out.open_tools);
            }
            Err(e) => {
                // 这一轮失败: 撤掉刚加的用户消息(否则下一条会连着两条 user), 话还给输入框
                if let Some(pla_ui::draw_agent::Line::User(t)) = self.agent_chat.lines.last().cloned() {
                    self.agent_chat.lines.pop();
                    if let Some(m) = self.agent_chat.history.pop() {
                        // 撤回的消息里带着 tool_result → 放回待回列表
                        for b in m["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "tool_result") {
                            let id = b["tool_use_id"].as_str().unwrap_or("").to_string();
                            let c = b["content"].as_str().unwrap_or("").to_string();
                            self.agent_chat.open_tools.push((id, c));
                        }
                    }
                    self.agent_chat.last_image.clear();
                    if self.agent_input.trim().is_empty() {
                        self.agent_input = t;
                    }
                }
                self.agent_chat.lines.push(pla_ui::draw_agent::Line::Error(e));
            }
        }
    }

    /// 发一句话: 带上当前方案 / 报告 / 预览图, 后台跑一轮。
    fn agent_send(&mut self, key: &str, text: &str, bom_title: &std::collections::BTreeMap<String, String>) {
        let Some(s) = self.draw_sheets.get(key).cloned() else { return };
        let rep = self.draw_report.as_ref().filter(|(k, _)| k == key && self.draw_up_view.is_none()).map(|(_, r)| r.clone());
        let all: Vec<pla_ui::mcp_bridge::PartInfo> = self.bom_parts().into_iter().map(|p| p.2).collect();
        let ctx = pla_ui::draw_agent::context_with(&s, rep.as_ref(), bom_title, s.locked(), &all);
        let img = self.draw_png.clone();
        self.agent_chat.push_user(text, &ctx, img.as_ref().map(|(u, b)| (u.as_str(), &b[..])));
        let history = self.agent_chat.history.clone();
        let rules = self.prefs_get().agent_rules.clone();
        let cfg = self.llm_cfg.clone();
        let others: std::collections::HashMap<String, pla_ui::drawing::Sheet> = self.draw_sheets.sheets.iter().map(|x| (x.key.clone(), x.clone())).collect();
        let dir = pla_ui::drawing::dir(&self.doc_key);
        let cur = key.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // 预览: 当前这张用内存里的报告; 改多张时逐张打到副本上(各自的报告从磁盘读), 有一张不认 → 整个补丁退回给模型改
            let preview = |p: &pla_ui::draw_agent::Patch| -> Result<Vec<String>, String> {
                let ts = pla_ui::draw_agent::targets(p, &cur, &all)?;
                if ts.len() == 1 && ts[0] == cur {
                    let mut c = s.clone();
                    return pla_ui::draw_agent::apply(&mut c, p, rep.as_ref());
                }
                let mut lines = Vec::new();
                let mut errors = Vec::new();
                for t in &ts {
                    let label = all.iter().find(|x| &x.key == t).map(|x| format!("{} {}", x.pn, x.name).trim().to_string()).unwrap_or_else(|| t.clone());
                    let mut sh = others.get(t).cloned().unwrap_or_else(|| pla_ui::drawing::Sheet::new(t, &label, "", ""));
                    if sh.locked() {
                        lines.push(t!("[{label}] 跳过(以上传图纸为准)", "[{label}] skipped (follows an uploaded drawing)"));
                        continue;
                    }
                    let r = if *t == cur { rep.clone() } else { pla_ui::drawing::load_report(&dir, &sh) };
                    match pla_ui::draw_agent::apply(&mut sh, p, r.as_ref()) {
                        Ok(ch) => lines.push(format!("[{label}] {}", ch.join("; "))),
                        Err(e) => errors.push(format!("{label}: {e}")),
                    }
                }
                if errors.is_empty() {
                    Ok(lines)
                } else {
                    Err(errors.join(" | "))
                }
            };
            let _ = tx.send(pla_ui::draw_agent::run_turn(&cfg, &rules, &history, &preview));
        });
        self.agent_job = Some((key.to_string(), rx));
    }

    /// 用户点了「应用」: 补丁打到图上(+1 人修正次数, 记案例)/ 规则存进偏好; 可选接着出图。
    fn agent_apply(&mut self, key: &str, id: &str, draw: bool, bom_title: &std::collections::BTreeMap<String, String>) {
        use pla_ui::draw_agent::{Line, Proposal};
        let Some(prop) = self.agent_chat.lines.iter().find_map(|l| match l {
            Line::Proposal { id: i, prop, decided: None, .. } if i == id => Some(prop.clone()),
            _ => None,
        }) else {
            return;
        };
        let request = self.agent_chat.lines.iter().rev().find_map(|l| if let Line::User(t) = l { Some(t.clone()) } else { None }).unwrap_or_default();
        match prop {
            Proposal::Patch(p) => {
                let parts = self.bom_parts();
                let all: Vec<pla_ui::mcp_bridge::PartInfo> = parts.iter().map(|x| x.2.clone()).collect();
                let ts = match pla_ui::draw_agent::targets(&p, key, &all) {
                    Ok(t) => t,
                    Err(e) => {
                        self.agent_chat.decide(id, false, &format!("Applying failed: {e}"));
                        self.set_notice((true, t!("应用失败: {e}", "Applying failed: {e}")));
                        return;
                    }
                };
                let dir = pla_ui::drawing::dir(&self.doc_key);
                let mut done: Vec<String> = Vec::new();
                let mut failed: Vec<String> = Vec::new();
                for t in &ts {
                    if let Some(x) = parts.iter().find(|x| &x.2.key == t) {
                        self.draw_ensure(t, &x.0, &x.1);
                    }
                    let Some(mut s) = self.draw_sheets.get(t).cloned() else { continue };
                    if s.locked() {
                        continue;
                    }
                    let rep = if t == key { self.draw_report.as_ref().filter(|(k, _)| k == key && self.draw_up_view.is_none()).map(|(_, r)| r.clone()) } else { pla_ui::drawing::load_report(&dir, &s) };
                    match pla_ui::draw_agent::apply(&mut s, &p, rep.as_ref()) {
                        Ok(changes) => {
                            s.edits += 1;
                            pla_ui::draw_agent::log_applied(&dir, &s, &request, &p, &changes);
                            self.draw_sheets.upsert(s);
                            done.push(t.clone());
                        }
                        Err(e) => failed.push(format!("{}: {e}", s.base)),
                    }
                }
                self.draw_save();
                self.draw_buf.clear();
                self.draw_load(key);
                let note = if !failed.is_empty() { format!("Failed on: {}", failed.join("; ")) } else if draw { "The drawings are being recompiled; the next message will carry the new image.".into() } else { String::new() };
                self.agent_chat.decide(id, !done.is_empty(), &note);
                if !failed.is_empty() {
                    self.set_notice((true, t!("{} 张没改成: {}", "{} sheet(s) not changed: {}", failed.len(), failed.join("; "))));
                } else if draw {
                    if done.len() == 1 && done[0] == key {
                        let mut title = bom_title.clone();
                        title.extend(self.draw_sheets.get(key).map(|s| s.title.clone()).unwrap_or_default());
                        self.draw_start(key, false, title);
                    } else {
                        self.mcp_queue_draw(&done, false, &parts);
                    }
                } else {
                    self.set_notice((false, t!("已应用到 {} 张图, 点「出图」生效", "Applied to {} drawing(s); click Draw to see it", done.len())));
                }
            }
            Proposal::Rule(r) => {
                let mut p = self.prefs_get().clone();
                if !p.agent_rules.contains(&r) {
                    p.agent_rules.push(r);
                }
                self.prefs_set(p);
                self.agent_chat.decide(id, true, "Saved as a standing rule.");
            }
        }
    }

    fn open_path(&mut self, p: &std::path::Path) {
        if let Err(e) = std::process::Command::new("cmd").args(["/C", "start", "", &p.display().to_string()]).spawn() {
            self.set_notice((true, t!("打不开 {}: {e}", "Cannot open {}: {e}", p.display())));
        }
    }

    /// BOM 窗口「出图」标签: 左 = 零件列表(BOM 扫描结果 + 已有的图), 右 = 选中零件的出图设置 / 修正 / 预览。
    fn draw_ui(&mut self, ui: &mut egui::Ui) {
        self.draw_tick();
        if pla_ui::drawing::tools_dir().is_none() {
            ui.label(egui::RichText::new(tr("找不到出图编译器(tools/drawing; 或设环境变量 PLA_DRAWING_DIR)", "Drawing compiler not found (tools/drawing; or set PLA_DRAWING_DIR)")).color(COL_RED));
        }
        if pla_ui::drawing::freecad_bin().is_none() {
            ui.label(egui::RichText::new(tr("找不到 FreeCAD 1.1(出图后端, 装在默认位置)", "FreeCAD 1.1 not found (drawing backend, default install location)")).color(COL_RED));
        }
        // 零件列表: BOM 行(料号 / 名称 / 材料 + 第一个实例)
        struct Item {
            key: String,
            label: String,
            title: std::collections::BTreeMap<String, String>,
            member: (String, String),
            /// 外购: 全部实例都在导入的 STEP / IGES 组里(电机 / 电路板等的零件) —— 不出零件图
            bought: bool,
        }
        let mut items: Vec<Item> = Vec::new();
        // 装配图: key → 明细(整个范围 + 每个顶层组); 表格里的料号 / 名称 / 材料 / 表处按 BOM 行 key 取
        let mut asms: Vec<(String, String, Vec<pla_ui::drawing::AsmLine>)> = Vec::new();
        let mut infos: std::collections::HashMap<String, pla_ui::drawing::RowInfo> = std::collections::HashMap::new();
        if let Some(Ok(v)) = &self.bom_result {
            let (rows, _) = pla_ui::bom_table::build(v, &self.bom_tpl, &self.bom_edits, &self.lib);
            let cols = &self.bom_tpl.columns;
            for r in rows.iter().filter(|r| !r.manual) {
                let get = |id: &str| cols.iter().position(|c| c.id == id).map(|i| r.cells[i].trim().to_string()).unwrap_or_default();
                let line = v["lines"].as_array().into_iter().flatten().find(|l| l["key"].as_str() == Some(r.key.as_str()));
                let m = line.and_then(|l| l["members"].as_array()).and_then(|a| a.first());
                let member = m.map(|m| (m["id"].as_str().unwrap_or("").to_string(), m["name"].as_str().unwrap_or("").to_string())).unwrap_or_default();
                infos.insert(r.key.clone(), pla_ui::drawing::RowInfo { pn: get("pn"), name: get("name"), material: get("material"), finish: get("finish") });
                let mut title = std::collections::BTreeMap::new();
                for id in ["pn", "name", "material", "finish"] {
                    let x = get(id);
                    if !x.is_empty() {
                        title.insert(id.to_string(), x);
                    }
                }
                let qty = line.and_then(|l| l["qty"].as_u64()).unwrap_or(1);
                title.insert("qty".into(), qty.to_string());
                let bought = line.is_some_and(|l| pla_ui::drawing::member_bought(&l["members"], &self.draw_sheets.purchased));
                // 标准件(螺钉 / 销 / 轴承 …, 看名字猜): 同外购一样不进批量, 单张还能出
                let nm = if get("name").is_empty() { member.1.clone() } else { get("name") };
                let standard = !bought && pla_ui::drawing::looks_standard(&nm, &get("pn"));
                let tag = if bought {
                    tr("  (外购)", "  (purchased)")
                } else if standard {
                    tr("  (标准件)", "  (standard part)")
                } else {
                    ""
                };
                let label = format!("{}  {}  ×{qty}{tag}", get("pn"), get("name")).trim().to_string();
                items.push(Item { key: r.key.clone(), label, title, member, bought: bought || standard });
            }
            let info = |k: &str| infos.get(k).cloned().unwrap_or_default();
            let stem = self.doc_key.rsplit(['/', '\\']).next().unwrap_or("").to_string();
            let pur = &self.draw_sheets.purchased;
            let all = pla_ui::drawing::asm_lines(&v["lines"], &info, None, pur);
            if !all.is_empty() {
                asms.push((pla_ui::drawing::asm_key(None), stem.clone(), all));
            }
            for g in pla_ui::drawing::asm_groups(&v["lines"], pur) {
                let ls = pla_ui::drawing::asm_lines(&v["lines"], &info, Some(&g), pur);
                asms.push((pla_ui::drawing::asm_key(Some(&g)), g, ls));
            }
        }
        // 装配图放最前(合订本里也在最前): 整个范围 + 每个顶层组(方案 / 子装配); 外购组件(导入的 STEP 组)合成一行
        for (i, (k, name, ls)) in asms.iter().enumerate() {
            let n: u64 = ls.iter().map(|l| l.qty).sum();
            let mut title = std::collections::BTreeMap::new();
            title.insert("name".to_string(), t!("{name} 装配", "{name} assembly"));
            title.insert("qty".to_string(), "1".to_string());
            let label = if i == 0 && k == pla_ui::drawing::ASM_KEY {
                t!("装配图 · 全部  ({} 种 / {n} 件)", "Assembly · all  ({} kinds / {n} pcs)", ls.len())
            } else {
                t!("装配图 · {name}  ({} 种 / {n} 件)", "Assembly · {name}  ({} kinds / {n} pcs)", ls.len())
            };
            items.insert(i, Item { key: k.clone(), label, title, member: (String::new(), String::new()), bought: false });
        }
        self.draw_asm = asms.into_iter().map(|(k, _, ls)| (k, ls)).collect();
        // (模型改过的零件: BOM 扫描回包时 identity_migrate 已经把整张图迁到新 key, 这里不用再找旧图)
        let locked_keys: std::collections::HashSet<String> = self.draw_sheets.sheets.iter().filter(|s| s.locked()).map(|s| s.key.clone()).collect();
        for s in &self.draw_sheets.sheets {
            if !items.iter().any(|i| i.key == s.key) {
                items.push(Item { key: s.key.clone(), label: format!("{}  ({})", s.base, tr("不在当前范围", "not in the current scope")), title: Default::default(), member: (s.member.clone(), s.model_name.clone()), bought: false });
            }
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(t!("{} 个零件", "{} part(s)", items.len())).color(COL_MUTED));
            if ui.add_enabled(!self.bom_busy, egui::Button::new(tr("重新扫描", "Rescan"))).clicked() {
                let s = self.bom_scope.clone();
                self.bom_scan(s);
            }
            if ui.button(tr("打开文件夹", "Open folder")).clicked() {
                if let Err(e) = pla_ui::drawing::STORE.open_dir(&self.doc_key) {
                    self.set_notice((true, e));
                }
            }
        });
        // ---- 外购组件: 名字不像导入文件的组(如 "heidhan encoder")人来勾 → 装配图合成一行, 组里零件不出零件图 ----
        let groups: Vec<String> = match &self.bom_result {
            Some(Ok(v)) => pla_ui::drawing::all_groups(&v["lines"]),
            _ => Vec::new(),
        };
        if !groups.is_empty() {
            let mut pur = self.draw_sheets.purchased.clone();
            egui::CollapsingHeader::new(t!("外购组件(已勾 {})", "Purchased groups ({} ticked)", pur.len())).id_salt("draw-purchased").show(ui, |ui| {
                ui.label(
                    egui::RichText::new(tr(
                        "名字是 .step / .igs 等文件名的组自动算外购; 别的组(电机、编码器…)在这里勾: 装配图明细合成一行, 组里零件不出零件图",
                        "Groups named like .step / .igs files count as purchased automatically; tick other groups (motors, encoders…) here: one parts-list line, no part drawings for their parts",
                    ))
                    .size(11.0)
                    .color(COL_MUTED),
                );
                egui::ScrollArea::vertical().id_salt("draw-purchased-list").max_height(120.0).show(ui, |ui| {
                    for g in &groups {
                        let mut on = pur.contains(g);
                        if ui.checkbox(&mut on, g).changed() {
                            pur.retain(|x| x != g);
                            if on {
                                pur.push(g.clone());
                            }
                        }
                    }
                });
            });
            if pur != self.draw_sheets.purchased {
                self.draw_sheets.purchased = pur;
                self.draw_save();
            }
        }
        let mut pick: Option<usize> = None;
        egui::ScrollArea::vertical().id_salt("draw-list").max_height(150.0).show(ui, |ui| {
            for (i, it) in items.iter().enumerate() {
                let sheet = self.draw_sheets.get(&it.key);
                let mut mark = match sheet.and_then(|s| s.uploaded.as_ref()) {
                    Some(u) if u.stale => tr("  · 上传图纸(模型改过, 待确认)", "  · uploaded (model changed, to confirm)").to_string(),
                    Some(u) if u.locked => tr("  · 上传图纸(锁定)", "  · uploaded (locked)").to_string(),
                    _ => match sheet {
                        Some(s) if !s.compiled.is_empty() => t!("  · 已出 {}", "  · drawn {}", s.compiled),
                        _ => String::new(),
                    },
                };
                if sheet.is_some_and(|s| s.model_changed && !s.locked()) {
                    mark.push_str(tr("  · 模型改过, 待重出", "  · model changed, redraw"));
                }
                let busy = self.draw_job.as_ref().is_some_and(|j| j.key == it.key);
                let text = egui::RichText::new(format!("{}{}{}", it.label, mark, if busy { "  …" } else { "" }));
                if ui.selectable_label(self.draw_sel.as_deref() == Some(it.key.as_str()), text).clicked() {
                    pick = Some(i);
                }
            }
        });
        // ---- 批量: 当前 BOM 范围里的零件(不在范围里的旧图不管) ----
        let mut batch: Option<bool> = None; // Some(包括已出过的?)
        // 批量出哪些: 装配图 + 自制件(外购组件里的零件不出零件图)
        // D8: 以上传图纸为准(锁定)的不出
        let drawable = |it: &Item| (pla_ui::drawing::is_asm(&it.key) || (!it.member.0.is_empty() && !it.bought)) && !locked_keys.contains(&it.key);
        let n_locked = items.iter().filter(|it| locked_keys.contains(&it.key)).count();
        ui.horizontal(|ui| {
            if self.draw_batch.is_some() {
                let (n, f) = self.draw_batch.as_ref().map(|(n, f)| (*n, f.len())).unwrap_or_default();
                let left = self.draw_queue.len() + usize::from(self.draw_job.is_some());
                ui.label(egui::RichText::new(t!("批量出图 {}/{n}(失败 {f})", "Batch {}/{n} ({f} failed)", (n + 1).saturating_sub(left).min(n))).color(COL_ACCENT));
                if ui.small_button(tr("停止", "Stop")).on_hover_text(tr("正在出的这张出完就停", "Stops after the current sheet")).clicked() {
                    // 当前这张出完为止; 没排上的不算失败
                    let dropped = self.draw_queue.len();
                    self.draw_queue.clear();
                    if let Some((n, _)) = self.draw_batch.as_mut() {
                        *n -= dropped;
                    }
                }
            } else {
                let free = self.draw_job.is_none() && !self.bom_busy;
                // 没出过的 + 模型改过待重出的
                let todo = items.iter().filter(|it| drawable(it) && self.draw_sheets.get(&it.key).map_or(true, |s| s.compiled.is_empty() || s.model_changed)).count();
                let all = items.iter().filter(|it| drawable(it)).count();
                if ui.add_enabled(free && todo > 0, egui::Button::new(t!("出未出的 {todo} 张", "Draw {todo} undrawn"))).on_hover_text(tr("按 BOM 顺序一张张出(每张约 20–90 s); 可以接着在 Plasticity 里干活, 但别关文档", "Draws one by one in BOM order (~20–90 s each); you can keep working in Plasticity, but keep the document open")).clicked() {
                    batch = Some(false);
                }
                if ui.add_enabled(free && all > 0, egui::Button::new(t!("全部重出 {all} 张", "Redraw all {all}"))).on_hover_text(tr("每张都重新导出 STEP 再出图(模型改过之后用)", "Re-exports every STEP, then draws (use after the model changed)")).clicked() {
                    batch = Some(true);
                }
                if n_locked > 0 {
                    ui.label(egui::RichText::new(t!("{n_locked} 张以上传图纸为准, 不出", "{n_locked} follow uploads, skipped")).size(11.0).color(COL_MUTED))
                        .on_hover_text(tr("合订本里用上传的图纸; 要重新生成先在该零件里解锁", "The combined PDF uses the uploads; unlock a part to regenerate it"));
                }
            }
            let done = items.iter().filter(|it| self.draw_sheets.get(&it.key).is_some_and(|s| s.locked() || !s.compiled.is_empty())).count();
            if self.draw_book.is_some() {
                ui.label(egui::RichText::new(tr("合订中…", "Combining…")).color(COL_ACCENT));
            } else if ui
                .add_enabled(done > 0 && self.draw_batch.is_none(), egui::Button::new(t!("合订 PDF({done} 张)", "Combine PDF ({done})")))
                .on_hover_text(tr("按 BOM 顺序把出过的图合成一份 PDF(批量出完会自动合订)", "Combines the drawn sheets into one PDF in BOM order (done automatically after a batch)"))
                .clicked()
            {
                let keys: Vec<String> = items.iter().map(|it| it.key.clone()).collect();
                self.draw_book_start(&keys);
            }
            let bookp = pla_ui::drawing::book_file(&pla_ui::drawing::dir(&self.doc_key), &self.doc_key);
            if bookp.exists() && self.draw_book.is_none() && ui.button(tr("打开合订本", "Open combined")).clicked() {
                self.open_path(&bookp);
            }
        });
        if let Some(redo) = batch {
            for it in items.iter().filter(|it| drawable(it)) {
                if !redo && self.draw_sheets.get(&it.key).is_some_and(|s| !s.compiled.is_empty() && !s.model_changed) {
                    continue;
                }
                self.draw_ensure(&it.key, &it.title, &it.member);
                let mut title = it.title.clone();
                title.extend(self.draw_sheets.get(&it.key).map(|s| s.title.clone()).unwrap_or_default());
                self.draw_queue.push_back((it.key.clone(), redo, title));
            }
            self.draw_batch = Some((self.draw_queue.len(), Vec::new()));
            // 合订按 BOM 顺序收全部图(含之前出过、这批没重出的)
            self.draw_batch_keys = items.iter().map(|it| it.key.clone()).collect();
        }
        if let Some(i) = pick {
            let it = &items[i];
            self.draw_ensure(&it.key, &it.title, &it.member);
            self.draw_sel = Some(it.key.clone());
            self.draw_buf.clear();
            self.draw_up_page = 0;
            self.draw_unlock_confirm = None;
            let k = it.key.clone();
            self.draw_load(&k);
        }
        ui.separator();
        let Some(key) = self.draw_sel.clone() else {
            ui.label(egui::RichText::new(tr("选一个零件出图(零件图, 国标第一角; 后台用 FreeCAD 编译, 约 20–90 s)", "Pick a part to draw (GB first-angle part drawing; compiled in the background with FreeCAD, ~20–90 s)")).color(COL_MUTED));
            return;
        };
        let Some(mut s) = self.draw_sheets.get(&key).cloned() else { return };
        let bom_title = items.iter().find(|i| i.key == key).map(|i| i.title.clone()).unwrap_or_default();
        let orig = s.clone();
        let mut go: Option<bool> = None; // Some(重新导出?)
        let job = self.draw_job.as_ref().map(|j| (j.key == key, j.stage, j.t0.elapsed().as_secs()));
        // ---- 操作 + 状态 ----
        let locked = s.locked();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&s.base).strong());
            let free = job.is_none() && !locked;
            let why_locked = tr("以上传的图纸为准(已锁定); 要重新生成先解锁", "Follows the uploaded drawing (locked); unlock to regenerate");
            let r = ui.add_enabled(free, egui::Button::new(tr("出图", "Draw")));
            if r.on_hover_text(if locked { why_locked } else { tr("没有 STEP 时先从 Plasticity 导出; 有了直接按当前设置编译", "Exports a STEP from Plasticity if there is none, then compiles with the current settings") }).clicked() {
                go = Some(false);
            }
            let r = ui.add_enabled(free, egui::Button::new(tr("重新导出 STEP 并出图", "Re-export STEP and draw")));
            if r.on_hover_text(if locked { why_locked } else { tr("模型改过之后用", "Use after the model changed") }).clicked() {
                go = Some(true);
            }
            let dir = pla_ui::drawing::dir(&self.doc_key);
            let pdf = pla_ui::drawing::file(&dir, &s, "pdf");
            if ui.add_enabled(pdf.exists(), egui::Button::new(tr("打开 PDF", "Open PDF"))).clicked() {
                self.open_path(&pdf);
            }
            match job {
                Some((true, DrawStage::Export, t)) => {
                    ui.label(egui::RichText::new(t!("导出 STEP… {t}s", "Exporting STEP… {t}s")).color(COL_ACCENT));
                }
                Some((true, DrawStage::Compile, t)) => {
                    ui.label(egui::RichText::new(t!("编译中… {t}s", "Compiling… {t}s")).color(COL_ACCENT));
                }
                Some((false, _, _)) => {
                    ui.label(egui::RichText::new(tr("另一张图在出", "Another drawing is running")).color(COL_MUTED));
                }
                None => {}
            }
        });
        // ---- D8: 上传的图纸(以它为准 = 锁定; 解锁要点两次) ----
        enum UpAct {
            Pick,
            Drop(std::path::PathBuf),
            Unlock,
            Relock,
            Delete,
            DropRefs(Vec<String>),
        }
        let mut up_act: Option<UpAct> = None;
        let up_busy = self.draw_up_job.as_ref().is_some_and(|(k, _)| *k == key);
        let confirm = self.draw_unlock_confirm.clone();
        let updir = pla_ui::drawing::dir(&self.doc_key);
        let mut open_up: Option<std::path::PathBuf> = None;
        // 拖文件到生产窗口 = 给选中的零件上传
        if let Some(p) = ui.ctx().input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone())) {
            up_act = Some(UpAct::Drop(p));
        }
        // 增量更新: 模型改过 → 设置都沿用了; 提示要重出, 并列出新报告里已经对不上的修正
        if s.model_changed && !locked {
            ui.label(egui::RichText::new(tr("模型改过: 原来的出图设置都沿用了, 点「出图」按新模型重出", "The model changed: all drawing settings were kept; click Draw to redraw from the new model")).color(COL_YELLOW));
        }
        let stale: Vec<String> = self.draw_report.as_ref().filter(|(k, _)| *k == key && !s.model_changed).map(|(_, r)| pla_ui::drawing::stale_refs(&s, r)).unwrap_or_default();
        if !stale.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t!("这些修正在新图里找不到了(模型改了, 尺寸重新命名): {}", "These corrections no longer match the new drawing (model changed, dimensions renamed): {}", stale.join(", "))).color(COL_YELLOW));
                if ui.small_button(tr("清理", "Clean up")).on_hover_text(tr("去掉这些名字的隐藏 / 关键 / 文字 / 公差 / 孔位设置", "Removes hide / key / text / tolerance / hole-position settings for these names")).clicked() {
                    up_act = Some(UpAct::DropRefs(stale.clone()));
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            match &s.uploaded {
                None => {
                    if ui
                        .add_enabled(!up_busy && self.draw_up_job.is_none(), egui::Button::new(tr("上传图纸…", "Upload drawing…")))
                        .on_hover_text(tr(
                            "已有图纸(PDF / 图片 / DWG / DXF)就上传: 这个零件以上传的为准, 不再生成。也可以把文件拖到这个窗口",
                            "Have a drawing already (PDF / image / DWG / DXF)? Upload it: this part then follows the upload and is not regenerated. You can also drop the file on this window",
                        ))
                        .clicked()
                    {
                        up_act = Some(UpAct::Pick);
                    }
                    ui.label(egui::RichText::new(tr("(或把文件拖到这个窗口)", "(or drop a file on this window)")).size(11.0).color(COL_MUTED));
                }
                Some(u) => {
                    let state = if u.locked {
                        egui::RichText::new(tr("以上传的图纸为准(锁定)", "Follows the upload (locked)")).color(COL_ACCENT)
                    } else {
                        egui::RichText::new(tr("已解锁: 用生成的图, 上传的留作参考", "Unlocked: the generated drawing is used, the upload is kept for reference")).color(COL_MUTED)
                    };
                    ui.label(state);
                    ui.label(egui::RichText::new(t!("{} · {}", "{} · {}", u.orig, u.date)).size(11.0).color(COL_MUTED));
                    if ui.small_button(tr("打开", "Open")).clicked() {
                        open_up = Some(updir.join(&u.file));
                    }
                    if ui.add_enabled(!up_busy && self.draw_up_job.is_none(), egui::Button::new(tr("替换…", "Replace…")).small()).clicked() {
                        up_act = Some(UpAct::Pick);
                    }
                    if u.locked {
                        let armed = confirm.as_deref() == Some(&format!("unlock:{key}"));
                        let b = egui::Button::new(if armed { tr("确认解锁?", "Confirm unlock?") } else { tr("解锁(允许重新生成)", "Unlock (allow regeneration)") }).small();
                        if ui.add(b).on_hover_text(tr("解锁后出图 / 批量会生成新图, 合订本也用生成的图; 上传的文件保留, 可以再锁回去", "After unlocking, Draw / batch generate a new drawing and the combined PDF uses it; the upload is kept and can be locked again")).clicked() {
                            if armed {
                                up_act = Some(UpAct::Unlock);
                            } else {
                                self.draw_unlock_confirm = Some(format!("unlock:{key}"));
                            }
                        }
                    } else if ui.small_button(tr("重新锁定", "Lock again")).on_hover_text(tr("回到以上传的图纸为准", "Back to following the upload")).clicked() {
                        up_act = Some(UpAct::Relock);
                    }
                    let armed = confirm.as_deref() == Some(&format!("delete:{key}"));
                    if ui.small_button(if armed { tr("确认删除?", "Confirm delete?") } else { tr("删除上传", "Remove upload") }).on_hover_text(tr("删掉出图目录里的这份上传(你原来的文件不动)", "Deletes this upload from the drawings folder (your original file is untouched)")).clicked() {
                        if armed {
                            up_act = Some(UpAct::Delete);
                        } else {
                            self.draw_unlock_confirm = Some(format!("delete:{key}"));
                        }
                    }
                    if u.stale {
                        ui.label(egui::RichText::new(tr("模型改过, 这份图纸可能过期", "The model changed; this drawing may be out of date")).color(COL_RED));
                        if ui.small_button(tr("仍然有效", "Still valid")).clicked() {
                            up_act = Some(UpAct::Relock);
                        }
                    }
                }
            }
            if up_busy {
                ui.label(egui::RichText::new(tr("上传中…", "Uploading…")).color(COL_ACCENT));
            }
        });
        if locked {
            ui.label(egui::RichText::new(tr("已锁定: 下面的出图设置只在解锁后重新生成时才用", "Locked: the settings below only apply after unlocking and regenerating")).size(11.0).color(COL_MUTED));
        }
        if let Some(p) = open_up {
            self.open_path(&p);
        }
        // ---- D5: 出图助手(自然语言 → 方案补丁, 用户确认后应用) ----
        enum AgentAct {
            Send(String),
            Apply(String, bool),
            Reject(String),
            Clear,
        }
        let mut agent_act: Option<AgentAct> = None;
        if self.agent_chat.key != key {
            self.agent_chat = pla_ui::draw_agent::Chat::new(&key);
        }
        self.agent_tick();
        let agent_busy = self.agent_job.as_ref().is_some_and(|(k, _)| *k == key);
        if self.agent_job.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
        }
        egui::CollapsingHeader::new(tr("出图助手(大模型)", "Drawing assistant (LLM)")).id_salt("draw-agent").default_open(true).show(ui, |ui| {
            if !self.llm_cfg.has_key() {
                ui.label(
                    egui::RichText::new(tr(
                        "用一句话改图: 「外径标 h7」「孔位只标 4 个安装孔」「主视翻过来」。先在 设置 → LLM 里填你自己的 Anthropic API key。",
                        "Change the drawing in plain words: \"give the outer diameter h7\", \"only dimension the 4 mounting holes\", \"flip the front view\". First enter your own Anthropic API key in Settings → LLM.",
                    ))
                    .size(11.0)
                    .color(COL_MUTED),
                );
                return;
            }
            use pla_ui::draw_agent::{Line, Proposal};
            egui::ScrollArea::vertical().id_salt("draw-agent-log").max_height(220.0).stick_to_bottom(true).show(ui, |ui| {
                for l in &self.agent_chat.lines {
                    match l {
                        Line::User(t) => {
                            ui.label(egui::RichText::new(format!("» {t}")).color(COL_TEXT));
                        }
                        Line::Assistant(t) => {
                            ui.label(egui::RichText::new(t).color(COL_MUTED));
                        }
                        Line::Error(t) => {
                            ui.label(egui::RichText::new(t).color(COL_RED));
                        }
                        Line::Proposal { id, prop, preview, decided } => {
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                let head = match prop {
                                    Proposal::Patch(p) => t!("提议修改: {}", "Proposed change: {}", p.summary),
                                    Proposal::Rule(_) => tr("提议记一条长期规则(以后每张图都用):", "Proposed standing rule (used for every future drawing):").to_string(),
                                };
                                ui.label(egui::RichText::new(head).strong());
                                for c in preview {
                                    ui.label(egui::RichText::new(format!("· {c}")).size(11.0));
                                }
                                match decided {
                                    Some(true) => {
                                        ui.label(egui::RichText::new(tr("已应用", "Applied")).color(COL_ACCENT));
                                    }
                                    Some(false) => {
                                        ui.label(egui::RichText::new(tr("没采用", "Rejected")).color(COL_MUTED));
                                    }
                                    None => {
                                        ui.horizontal(|ui| {
                                            if ui.button(tr("应用", "Apply")).clicked() {
                                                agent_act = Some(AgentAct::Apply(id.clone(), false));
                                            }
                                            if matches!(prop, Proposal::Patch(_)) && ui.add_enabled(!locked && job.is_none(), egui::Button::new(tr("应用并出图", "Apply and draw"))).clicked() {
                                                agent_act = Some(AgentAct::Apply(id.clone(), true));
                                            }
                                            if ui.button(tr("不要", "Reject")).clicked() {
                                                agent_act = Some(AgentAct::Reject(id.clone()));
                                            }
                                        });
                                    }
                                }
                            });
                        }
                    }
                }
                if agent_busy {
                    ui.label(egui::RichText::new(tr("思考中…", "Thinking…")).color(COL_ACCENT));
                }
            });
            ui.horizontal(|ui| {
                let te = ui.add(
                    egui::TextEdit::multiline(&mut self.agent_input)
                        .desired_rows(2)
                        .desired_width(ui.available_width() - 90.0)
                        .hint_text(tr("想怎么改? (Ctrl+Enter 发送)", "What should change? (Ctrl+Enter to send)")),
                );
                let ctrl_enter = te.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.ctrl);
                ui.vertical(|ui| {
                    let can = !agent_busy && self.agent_job.is_none() && !self.agent_input.trim().is_empty();
                    if (ui.add_enabled(can, egui::Button::new(tr("发送", "Send"))).clicked() || (ctrl_enter && can)) && can {
                        agent_act = Some(AgentAct::Send(std::mem::take(&mut self.agent_input).trim().to_string()));
                    }
                    if ui.add_enabled(!self.agent_chat.lines.is_empty() && !agent_busy, egui::Button::new(tr("清空", "Clear")).small()).clicked() {
                        agent_act = Some(AgentAct::Clear);
                    }
                });
            });
            let rules = self.prefs_get().agent_rules.len();
            if rules > 0 {
                ui.label(egui::RichText::new(t!("已记住 {rules} 条长期规则(设置在 drawing-prefs.json)", "{rules} standing rule(s) remembered (in drawing-prefs.json)")).size(11.0).color(COL_MUTED));
            }
        });
        // ---- 视图朝向(要先出过一次, 用报告里的实际朝向) ----
        let axes = self.draw_report.as_ref().filter(|(k, _)| *k == key).and_then(|(_, r)| r.axes);
        let mut reorient: Option<(Option<[f64; 3]>, Option<[f64; 3]>)> = None;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr("主视", "Front view")).color(COL_MUTED));
            ui.add_enabled_ui(axes.is_some() && job.is_none(), |ui| {
                if let Some(a) = axes {
                    if ui.small_button(tr("上下翻转", "Flip")).clicked() {
                        let (f, u) = a.flipped();
                        reorient = Some((Some(f), Some(u)));
                    }
                    if ui.small_button(tr("转 90°", "Rotate 90°")).clicked() {
                        let (f, u) = a.rotated();
                        reorient = Some((Some(f), Some(u)));
                    }
                    if ui.small_button(tr("看背面", "View the back")).clicked() {
                        let (f, u) = a.back();
                        reorient = Some((Some(f), Some(u)));
                    }
                } else {
                    let _ = ui.small_button(tr("上下翻转", "Flip"));
                }
            });
            if ui.add_enabled((s.front.is_some() || s.up.is_some()) && job.is_none(), egui::Button::new(tr("自动", "Auto")).small()).on_hover_text(tr("恢复自动选的朝向", "Back to the automatic orientation")).clicked() {
                reorient = Some((None, None));
            }
            if !pla_ui::drawing::is_asm(&key) {
                ui.checkbox(&mut s.section, tr("剖视 A-A", "Section A-A"));
            }
        });
        // ---- 装配剖视: 剖哪个视图 + 剖切面过哪里 ----
        if pla_ui::drawing::is_asm(&key) {
            let cands: Vec<pla_ui::drawing::FitCand> = self.draw_report.as_ref().filter(|(k, _)| *k == key).map(|(_, r)| r.fit_candidates.clone()).unwrap_or_default();
            let sec_info = self.draw_report.as_ref().filter(|(k, _)| *k == key).and_then(|(_, r)| r.section.clone());
            ui.horizontal(|ui| {
                ui.checkbox(&mut s.section, tr("剖视 A-A(全剖)", "Section A-A (full)")).on_hover_text(tr(
                    "剖切面垂直视线、留后半; 标准件 / 实心轴 / 外购件被剖到按不剖画; 相邻零件剖面线方向或间距不同",
                    "Cutting plane perpendicular to the view, back half kept; standard parts / solid shafts / purchased parts are drawn uncut; adjacent parts get different hatching",
                ));
                ui.add_enabled_ui(s.section, |ui| {
                    let cur_v = s.section_view.clone().unwrap_or_else(|| "Front".into());
                    let vname = |v: &str| if v == "Left" { tr("左视", "Left view") } else { tr("主视", "Front view") };
                    let mut pick_v = cur_v.clone();
                    egui::ComboBox::from_id_salt("draw-sec-view").selected_text(vname(&cur_v)).width(70.0).show_ui(ui, |ui| {
                        for v in ["Front", "Left"] {
                            ui.selectable_value(&mut pick_v, v.to_string(), vname(v));
                        }
                    });
                    if pick_v != cur_v {
                        s.section_view = if pick_v == "Front" { None } else { Some(pick_v) };
                    }
                    ui.label(egui::RichText::new(tr("过", "through")).color(COL_MUTED));
                    let label_of = |id: &Option<String>| -> String {
                        match id.as_deref() {
                            None => tr("自动(最大配合的轴线)", "auto (axis of the largest fit)").to_string(),
                            Some("@center") => tr("中心", "center").to_string(),
                            Some(f) => cands.iter().find(|c| c.id == f).map(|c| format!("Ø{} {} ↔ {}", pla_ui::drawing::fmt_d(c.d), c.hole, c.shaft)).unwrap_or_else(|| f.to_string()),
                        }
                    };
                    let mut pick_f = s.section_fit.clone();
                    egui::ComboBox::from_id_salt("draw-sec-fit").selected_text(label_of(&s.section_fit)).width(200.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut pick_f, None, label_of(&None));
                        ui.selectable_value(&mut pick_f, Some("@center".to_string()), label_of(&Some("@center".into())));
                        for c in &cands {
                            ui.selectable_value(&mut pick_f, Some(c.id.clone()), label_of(&Some(c.id.clone())));
                        }
                    });
                    s.section_fit = pick_f;
                });
            });
            if let Some(si) = sec_info {
                ui.label(
                    egui::RichText::new(t!(
                        "上次: 剖 {} 个零件, {} 个按不剖画",
                        "Last run: {} part(s) cut, {} drawn uncut",
                        si["cut"].as_u64().unwrap_or(0),
                        si["uncut"].as_u64().unwrap_or(0)
                    ))
                    .size(11.0)
                    .color(COL_MUTED),
                );
            }
        }
        if pla_ui::drawing::is_asm(&key) {
            ui.label(
                egui::RichText::new(tr(
                    "装配图每次都重新导出当前 BOM 范围的全部零件; 序号按视图外一圈顺时针编, 明细表跟着序号排",
                    "The assembly drawing re-exports every part in the current BOM scope each time; balloons are numbered clockwise around the views and the parts list follows them",
                ))
                .size(11.0)
                .color(COL_MUTED),
            );
        }
        if let Some((f, u)) = reorient {
            s.front = f;
            s.up = u;
            go = Some(false);
        }
        // ---- 孔组: 勾 = 标孔位(坐标尺寸) ----
        if let Some((_, r)) = self.draw_report.as_ref().filter(|(k, _)| *k == key) {
            if !r.hole_groups.is_empty() {
                let front: Vec<&pla_ui::drawing::HoleGroup> = r.hole_groups.iter().filter(|g| g.view == "Front").collect();
                egui::CollapsingHeader::new(t!("孔组 {}(勾 = 标孔位)", "Hole groups {} (ticked = position dims)", r.hole_groups.len())).id_salt("draw-holes").default_open(true).show(ui, |ui| {
                    // 默认(没点名)= 编译器报告里标了坐标的那些(均布在节圆上的孔组默认不标)
                    let default_on: Vec<String> = front.iter().filter(|g| g.ordinate).map(|g| g.id.clone()).collect();
                    let mut on: Vec<String> = match &s.ordinate {
                        None => default_on.clone(),
                        Some(v) => v.clone(),
                    };
                    let mut changed = false;
                    for g in &r.hole_groups {
                        if g.view == "Front" {
                            let mut c = on.contains(&g.id);
                            if ui.checkbox(&mut c, format!("{}  {}", g.id, g.text)).changed() {
                                changed = true;
                                if c {
                                    on.push(g.id.clone());
                                } else {
                                    on.retain(|x| x != &g.id);
                                }
                            }
                        } else {
                            ui.label(egui::RichText::new(format!("{}  {}  ({})", g.id, g.text, g.view)).color(COL_MUTED));
                        }
                    }
                    if changed {
                        let same = default_on.iter().all(|g| on.contains(g)) && on.len() == default_on.len();
                        s.ordinate = if same { None } else { Some(on) };
                    }
                });
            }
            // ---- D2 配合(装配图): 候选 = 不同零件之间同轴同径的孔 / 轴; 选了代号 → 装配图标 "⌀8 H7/g6", 两边零件图标公差 ----
            if !r.fit_candidates.is_empty() {
                egui::CollapsingHeader::new(t!("配合 {}(选代号 → 装配图标注 + 零件图公差)", "Fits {} (pick a code → assembly label + part tolerances)", r.fit_candidates.len()))
                    .id_salt("draw-fits")
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::Grid::new("draw-fits-grid").num_columns(3).spacing([8.0, 3.0]).show(ui, |ui| {
                            for c in &r.fit_candidates {
                                // 代号放前面(零件名可能很长, 放后面被挤出窗口的是说明而不是下拉框); Ø / ↔ 用面板字体里有的字
                                let desc = format!("Ø{}  {} ↔ {}  ×{}", pla_ui::drawing::fmt_d(c.d), c.hole, c.shaft, c.count);
                                let cur = s.fits.iter().find(|f| f.id == c.id).map(|f| f.code.clone()).unwrap_or_default();
                                let mut pick = cur.clone();
                                egui::ComboBox::from_id_salt(("fit", c.id.as_str())).selected_text(if cur.is_empty() { "—".to_string() } else { cur.clone() }).width(80.0).show_ui(ui, |ui| {
                                    ui.selectable_value(&mut pick, String::new(), "—");
                                    for (code, zh, en) in pla_ui::drawing::FIT_CODES {
                                        ui.selectable_value(&mut pick, code.to_string(), format!("{code}  {}", tr(zh, en)));
                                    }
                                });
                                let buf = self.draw_buf.entry(format!("fit|{}", c.id)).or_insert_with(|| cur.clone());
                                let te = ui.add(egui::TextEdit::singleline(buf).hint_text("H7/g6").desired_width(70.0));
                                if te.lost_focus() && buf.trim() != cur {
                                    pick = buf.trim().to_string();
                                } else if !te.has_focus() {
                                    *buf = cur.clone();
                                }
                                ui.label(egui::RichText::new(desc).size(12.0)).on_hover_text(tr("孔 ↔ 轴, ×N = 这样的配合处数", "hole ↔ shaft, ×N = places with this fit"));
                                if pick != cur {
                                    s.fits.retain(|f| f.id != c.id);
                                    if !pick.is_empty() {
                                        s.fits.push(pla_ui::drawing::Fit { id: c.id.clone(), d: c.d, hole_key: c.hole_key.clone(), shaft_key: c.shaft_key.clone(), code: pick });
                                    }
                                }
                                ui.end_row();
                            }
                        });
                        ui.label(egui::RichText::new(tr("改了配合: 重出装配图 + 相关零件图(零件图公差按 ISO 286 查表)", "After changing fits: redraw the assembly and the affected parts (part tolerances from ISO 286)")).size(11.0).color(COL_MUTED));
                    });
            }
            if !r.warnings.is_empty() {
                egui::CollapsingHeader::new(t!("编译提示 {}", "Compiler notes {}", r.warnings.len())).id_salt("draw-warn").show(ui, |ui| {
                    for w in &r.warnings {
                        ui.label(egui::RichText::new(w).size(11.0).color(COL_MUTED));
                    }
                });
            }
        }
        let (pl_rows, bal_rep, tech_used, tech_def) = self
            .draw_report
            .as_ref()
            .filter(|(k, _)| *k == key)
            .map(|(_, r)| (r.parts_list.clone(), r.balloons.clone(), r.tech.clone(), r.tech_default.clone()))
            .unwrap_or_default();
        // ---- 装配图: 明细 / 序号人改(点预览上的序号圈也能选中) ----
        let bal_sel = self.draw_buf.get("balsel").cloned().unwrap_or_default();
        if pla_ui::drawing::is_asm(&key) && !pl_rows.is_empty() {
            let bom_lines: Vec<pla_ui::drawing::AsmLine> = self.draw_asm.get(&key).cloned().unwrap_or_default();
            egui::CollapsingHeader::new(t!("明细 / 序号 {}(点预览上的序号也能选中)", "Parts list / balloons {} (or click a balloon on the preview)", pl_rows.len()))
                .id_salt("draw-pl")
                .default_open(!bal_sel.is_empty())
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("draw-pl-scroll").max_height(260.0).show(ui, |ui| {
                        egui::Grid::new("draw-pl-grid").num_columns(7).spacing([6.0, 2.0]).show(ui, |ui| {
                            for h in [tr("序号", "No."), tr("名称", "Name"), tr("数量", "Qty"), tr("备注", "Note"), tr("序号", "Balloon"), tr("位置", "Side"), tr("视图", "View")] {
                                ui.label(egui::RichText::new(h).size(11.0).color(COL_MUTED));
                            }
                            ui.end_row();
                            for row in &pl_rows {
                                let Some(k) = row.key.clone() else { continue };
                                let bl = bom_lines.iter().find(|l| l.key == k);
                                let picked = bal_sel == k;
                                if ui.selectable_label(picked, egui::RichText::new(row.no.to_string()).strong()).clicked() {
                                    self.draw_buf.insert("balsel".into(), if picked { String::new() } else { k.clone() });
                                }
                                for (col, w, bom_v) in [
                                    ("name", 150.0, bl.map(|l| l.name.clone()).unwrap_or_default()),
                                    ("qty", 36.0, bl.map(|l| l.qty.to_string()).unwrap_or_default()),
                                    ("note", 80.0, bl.map(|l| l.note.clone()).unwrap_or_default()),
                                ] {
                                    let cur = s.parts_list.get(&k).and_then(|m| m.get(col)).cloned().unwrap_or_default();
                                    let buf = self.draw_buf.entry(format!("pl|{k}|{col}")).or_insert_with(|| cur.clone());
                                    let te = ui.add(egui::TextEdit::singleline(buf).hint_text(bom_v.clone()).desired_width(w).text_color(COL_ACCENT));
                                    if te.lost_focus() {
                                        let v = buf.trim().to_string();
                                        let m = s.parts_list.entry(k.clone()).or_default();
                                        if v.is_empty() || v == bom_v {
                                            m.remove(col);
                                        } else {
                                            m.insert(col.to_string(), v);
                                        }
                                        if m.is_empty() {
                                            s.parts_list.remove(&k);
                                        }
                                    } else if !te.has_focus() {
                                        *buf = cur;
                                    }
                                }
                                let mut e = s.balloons.get(&k).cloned().unwrap_or_default();
                                let mut show = !e.hide;
                                ui.checkbox(&mut show, "").on_hover_text(tr("不勾 = 不标序号(明细表里仍有)", "Unticked = no balloon (still in the parts list)"));
                                e.hide = !show;
                                let side_name = |x: &Option<String>| match x.as_deref() {
                                    Some("left") => tr("左列", "left"),
                                    Some("top") => tr("上排", "top"),
                                    _ => tr("自动", "auto"),
                                };
                                egui::ComboBox::from_id_salt(("bal-side", k.as_str())).selected_text(side_name(&e.side)).width(56.0).show_ui(ui, |ui| {
                                    for o in [None, Some("left".to_string()), Some("top".to_string())] {
                                        let lab = side_name(&o);
                                        ui.selectable_value(&mut e.side, o, lab);
                                    }
                                });
                                let view_name = |x: &Option<String>| match x.as_deref() {
                                    Some("Front") => tr("主视", "front"),
                                    Some("Left") => tr("左视", "left"),
                                    Some("Top") => tr("俯视", "top"),
                                    _ => tr("自动", "auto"),
                                };
                                let shown_in = bal_rep.iter().find(|b| b.key.as_deref() == Some(k.as_str())).map(|b| b.view.clone()).unwrap_or_default();
                                egui::ComboBox::from_id_salt(("bal-view", k.as_str())).selected_text(view_name(&e.view)).width(56.0).show_ui(ui, |ui| {
                                    for o in [None, Some("Front".to_string()), Some("Left".to_string()), Some("Top".to_string())] {
                                        let lab = view_name(&o);
                                        ui.selectable_value(&mut e.view, o, lab);
                                    }
                                })
                                .response
                                .on_hover_text(t!("上次标在: {}", "Last placed in: {}", view_name(&Some(shown_in))));
                                if e.is_default() {
                                    s.balloons.remove(&k);
                                } else {
                                    s.balloons.insert(k.clone(), e);
                                }
                                ui.end_row();
                            }
                        });
                    });
                    ui.label(
                        egui::RichText::new(tr(
                            "序号按位置顺时针重新编号, 明细表跟着排; 改名称 / 数量只改这张图(BOM 不变); 改完点「出图」",
                            "Balloons are renumbered clockwise by position and the parts list follows; name / qty edits apply to this drawing only (not the BOM); click Draw to apply",
                        ))
                        .size(11.0)
                        .color(COL_MUTED),
                    );
                });
        }
        // ---- 技术要求(零件图 / 装配图): 默认 = 规范 + 按配合自动加的条目; 改了存本图, 可设为自己的默认 ----
        let mut tech_pref: Option<Option<Vec<String>>> = None;
        if !tech_used.is_empty() || s.tech.is_some() {
            egui::CollapsingHeader::new(if s.tech.is_some() { tr("技术要求(已改)", "Technical requirements (edited)") } else { tr("技术要求", "Technical requirements") })
                .id_salt("draw-tech")
                .show(ui, |ui| {
                    let cur = s.tech.clone().unwrap_or_else(|| tech_used.clone());
                    let cur_text = pla_ui::drawing::tech_text(&cur);
                    let buf = self.draw_buf.entry("tech".into()).or_insert_with(|| cur_text.clone());
                    let te = ui.add(egui::TextEdit::multiline(buf).desired_rows(6).desired_width(f32::INFINITY));
                    if te.lost_focus() {
                        let l = pla_ui::drawing::tech_lines(buf);
                        if l.is_empty() || l == tech_def {
                            s.tech = None;
                        } else if l != cur {
                            s.tech = Some(l);
                        }
                    } else if !te.has_focus() {
                        *buf = cur_text;
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(s.tech.is_some(), egui::Button::new(tr("恢复默认", "Reset")).small()).clicked() {
                            s.tech = None;
                            self.draw_buf.remove("tech");
                        }
                        if ui
                            .small_button(tr("设为我的默认", "Make my default"))
                            .on_hover_text(if pla_ui::drawing::is_asm(&key) { tr("以后所有装配图都用这段(配合条目不再自动加)", "Used for every assembly drawing from now on (fit lines are no longer added automatically)") } else { tr("以后所有零件图都用这段(配合面粗糙度不再自动加)", "Used for every part drawing from now on (fit roughness is no longer added automatically)") })
                            .clicked()
                        {
                            tech_pref = Some(Some(pla_ui::drawing::tech_lines(&pla_ui::drawing::tech_text(&cur))));
                        }
                        if ui.small_button(tr("清除我的默认", "Clear my default")).clicked() {
                            tech_pref = Some(None);
                        }
                    });
                    ui.label(egui::RichText::new(tr("一行一条; 默认会按配合加条目(装配图: 压装 / 压入 / 灵活; 零件图: 配合面粗糙度)", "One line per item; by default lines are added from the fits (assembly: press / push fit / free running; part: fit-surface roughness)")).size(11.0).color(COL_MUTED));
                });
        }
        if let Some(t) = tech_pref {
            let t_is_set = t.is_some();
            let mut p = self.prefs_get().clone();
            if pla_ui::drawing::is_asm(&key) {
                p.tech_asm = t;
            } else {
                p.tech_part = t;
            }
            if t_is_set {
                s.tech = None; // 本图跟着默认走
                self.draw_buf.remove("tech");
            }
            self.prefs_set(p);
        }
        let pref_title = self.prefs_get().title.clone();
        let mut title_pref = false;
        // ---- 标题栏 + 螺纹规则 + 密度 ----
        egui::CollapsingHeader::new(tr("标题栏 / 螺纹 / 重量", "Title block / threads / weight")).id_salt("draw-title").show(ui, |ui| {
            if ui
                .small_button(tr("设计 / 审核 / 项目 / 表处 / 硬度 / 版本 设为默认", "Make designer / reviewer / project / finish / hardness / version my default"))
                .on_hover_text(tr("以后每张新图的这些栏自动填(本图填了的优先)", "These fields are filled in on every drawing (this drawing's own values win)"))
                .clicked()
            {
                title_pref = true;
            }
            egui::Grid::new("draw-title-grid").num_columns(2).spacing([8.0, 3.0]).show(ui, |ui| {
                for (id, zh, en) in DRAW_TITLE_FIELDS {
                    ui.label(egui::RichText::new(tr(zh, en)).color(COL_MUTED));
                    let from_bom = bom_title.get(*id).or_else(|| pref_title.get(*id)).cloned().unwrap_or_default();
                    let cur = s.title.get(*id).cloned().unwrap_or_default();
                    let buf = self.draw_buf.entry(format!("t|{id}")).or_insert_with(|| cur.clone());
                    let te = ui.add(egui::TextEdit::singleline(buf).hint_text(from_bom.clone()).desired_width(200.0).text_color(COL_ACCENT));
                    if te.lost_focus() {
                        let v = buf.trim().to_string();
                        if v.is_empty() || v == from_bom {
                            s.title.remove(*id);
                        } else {
                            s.title.insert(id.to_string(), v);
                        }
                    } else if !te.has_focus() {
                        *buf = cur;
                    }
                    ui.end_row();
                }
                if pla_ui::drawing::is_asm(&key) {
                    return; // 装配图: 螺纹 / 密度是零件的事(重量按各零件材料汇总)
                }
                ui.label(egui::RichText::new(tr("孔径→螺纹", "Hole → thread")).color(COL_MUTED)).on_hover_text(tr(
                    "没有打孔记录的螺纹底孔按孔径认, 如 3.3=M4, 2.5=M3(⌀3.2 也可能是过孔, 所以按项目填)",
                    "Tapped holes without a hole record are recognised by diameter, e.g. 3.3=M4, 2.5=M3 (⌀3.2 may be a clearance hole, so set per project)",
                ));
                let cur: String = s.threads.iter().map(|(d, m)| format!("{d}={m}")).collect::<Vec<_>>().join(", ");
                let buf = self.draw_buf.entry("threads".into()).or_insert_with(|| cur.clone());
                let te = ui.add(egui::TextEdit::singleline(buf).hint_text("3.3=M4, 2.5=M3").desired_width(200.0));
                if te.lost_focus() {
                    s.threads = buf
                        .split([',', '，', ';'])
                        .filter_map(|p| p.split_once('=').map(|(d, m)| (d.trim().to_string(), m.trim().to_string())))
                        .filter(|(d, m)| d.parse::<f64>().is_ok() && !m.is_empty())
                        .collect();
                } else if !te.has_focus() {
                    *buf = cur;
                }
                ui.end_row();
                ui.label(egui::RichText::new(tr("密度 g/cm³", "Density g/cm³")).color(COL_MUTED)).on_hover_text(tr("重量 = 体积 × 密度; 空 = 不填重量", "Weight = volume × density; empty = no weight"));
                let cur = s.density.map(|d| d.to_string()).unwrap_or_default();
                let buf = self.draw_buf.entry("density".into()).or_insert_with(|| cur.clone());
                let te = ui.add(egui::TextEdit::singleline(buf).desired_width(80.0));
                if te.lost_focus() {
                    s.density = buf.trim().parse::<f64>().ok().filter(|d| *d > 0.0);
                } else if !te.has_focus() {
                    *buf = cur;
                }
                ui.end_row();
            });
        });
        if title_pref {
            let mut p = self.prefs_get().clone();
            for f in pla_ui::drawing::PREF_TITLE_FIELDS {
                match s.title.get(*f).map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
                    Some(v) => {
                        p.title.insert(f.to_string(), v);
                    }
                    None => {
                        p.title.remove(*f);
                    }
                }
            }
            self.prefs_set(p);
        }
        // ---- 尺寸: 列表 / 点预览选中 → 不标 / 关键尺寸 * / 改文字(重出后生效) ----
        let placed: Vec<pla_ui::drawing::Placed> = self.draw_report.as_ref().filter(|(k, _)| *k == key).map(|(_, r)| r.placed.clone()).unwrap_or_default();
        let sel_dim = self.draw_buf.get("dimsel").cloned().unwrap_or_default();
        if !placed.is_empty() {
            egui::CollapsingHeader::new(t!("尺寸 {}(点预览上的尺寸也能选中)", "Dimensions {} (or click one on the preview)", placed.len()))
                .id_salt("draw-dims")
                .default_open(!sel_dim.is_empty())
                .show(ui, |ui| {
                    egui::Grid::new("draw-dims-grid").num_columns(4).spacing([8.0, 2.0]).show(ui, |ui| {
                        for p in &placed {
                            let picked = sel_dim == p.name;
                            if ui.selectable_label(picked, egui::RichText::new(format!("{}  {}", p.name, p.label())).size(12.0)).clicked() {
                                self.draw_buf.insert("dimsel".into(), if picked { String::new() } else { p.name.clone() });
                            }
                            let mut hide = s.dims_hide.contains(&p.name);
                            if ui.checkbox(&mut hide, tr("不标", "Hide")).changed() {
                                s.dims_hide.retain(|n| n != &p.name);
                                if hide {
                                    s.dims_hide.push(p.name.clone());
                                }
                            }
                            let mut keyd = s.dims_key.contains(&p.name);
                            if ui.checkbox(&mut keyd, tr("关键 *", "Key *")).on_hover_text(tr("关键尺寸: 数字前加 *(技术要求第 2 条)", "Key dimension: * in front of the value")).changed() {
                                s.dims_key.retain(|n| n != &p.name);
                                if keyd {
                                    s.dims_key.push(p.name.clone());
                                }
                            }
                            if picked {
                                ui.horizontal(|ui| {
                                    for (map_tol, hint, w) in [
                                        (false, tr("改文字(空 = 自动)", "Text (empty = automatic)"), 150.0),
                                        (true, tr("公差: h7 / ±0.05 / +0.1/-0.05", "Tolerance: h7 / ±0.05 / +0.1/-0.05"), 150.0),
                                    ] {
                                        let map = if map_tol { &mut s.dims_tol } else { &mut s.dims_text };
                                        let cur = map.get(&p.name).cloned().unwrap_or_default();
                                        let bk = format!("dim{}|{}", if map_tol { "tol" } else { "text" }, p.name);
                                        let buf = self.draw_buf.entry(bk).or_insert_with(|| cur.clone());
                                        let te = ui.add(egui::TextEdit::singleline(buf).hint_text(hint).desired_width(w));
                                        if te.lost_focus() {
                                            let v = buf.trim().to_string();
                                            if v.is_empty() {
                                                map.remove(&p.name);
                                            } else {
                                                map.insert(p.name.clone(), v);
                                            }
                                        } else if !te.has_focus() {
                                            *buf = cur;
                                        }
                                    }
                                    if p.name.starts_with('U') && s.dims_add.iter().any(|a| a.name == p.name) && ui.small_button(tr("删除", "Delete")).clicked() {
                                        s.dims_add.retain(|a| a.name != p.name);
                                    }
                                });
                            } else {
                                ui.label("");
                            }
                            ui.end_row();
                        }
                    });
                    ui.label(egui::RichText::new(tr("改完点「出图」生效; 孔位坐标值由上面的孔组勾选控制", "Click Draw to apply; hole position values are controlled by the hole groups above")).size(11.0).color(COL_MUTED));
                });
        }
        // ---- 补尺寸: 在 Plasticity 里选 1 ~ 2 个面 → 签名存进这张图 ----
        {
            let faces: Vec<String> = self.live_sel.iter().filter(|it| it.kind == "face").map(|it| it.id.clone()).collect();
            let failed: Vec<(String, String)> = self
                .draw_report
                .as_ref()
                .filter(|(k, _)| *k == key)
                .map(|(_, r)| r.added_failed.iter().map(|f| (f.name.clone(), f.error.clone().unwrap_or_default())).collect())
                .unwrap_or_default();
            ui.horizontal_wrapped(|ui| {
                let ok = (1..=2).contains(&faces.len()) && self.draw_sig_pending.is_none();
                let b = ui
                    .add_enabled(ok, egui::Button::new(t!("＋ 补尺寸(已选 {} 个面)", "+ Add dimension ({} faces selected)", faces.len())))
                    .on_hover_text(tr(
                        "在 Plasticity 里选: 1 个圆柱面 = 直径; 1 个平面 = 厚度; 2 个平行的面(平面 / 圆柱轴线)= 距离",
                        "Select in Plasticity: 1 cylinder = diameter; 1 plane = thickness; 2 parallel faces (planes / cylinder axes) = distance",
                    ))
                    .on_disabled_hover_text(tr("先在 Plasticity 里选 1 ~ 2 个面", "Select 1–2 faces in Plasticity first"));
                if b.clicked() {
                    self.cdp.send(pla_bridge::cdp::CdpCmd::Call {
                        name: pla_bridge::shim::api::DIM_SIGNATURE,
                        args: serde_json::json!({ "faces": faces, "units": "millimeter" }),
                    });
                    self.draw_sig_pending = Some(key.clone());
                }
                if !s.dims_add.is_empty() {
                    ui.label(egui::RichText::new(t!("已补 {}: {}", "Added {}: {}", s.dims_add.len(), s.dims_add.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(" "))).size(11.0).color(COL_MUTED));
                }
            });
            for (n, e) in failed {
                ui.label(egui::RichText::new(t!("{n} 没标出: {e}", "{n} not drawn: {e}")).size(11.0).color(COL_RED));
            }
        }
        if s != orig {
            s.edits += 1; // D6 指标: 人修正的次数
            self.draw_sheets.upsert(s.clone());
            self.draw_save();
        }
        if let Some(export) = go {
            let mut title = bom_title.clone();
            title.extend(s.title.clone());
            self.draw_start(&key, export, title);
        }
        // D8: 上传动作放在设置保存之后(上面会用本帧开头的副本覆盖整张图); 不算"人修正"次数
        if let Some(act) = up_act {
            match act {
                UpAct::Pick => self.draw_upload_start(&key, None),
                UpAct::Drop(p) => self.draw_upload_start(&key, Some(p)),
                UpAct::DropRefs(names) => {
                    if let Some(mut cur) = self.draw_sheets.get(&key).cloned() {
                        pla_ui::drawing::drop_refs(&mut cur, &names);
                        self.draw_sheets.upsert(cur);
                        self.draw_save();
                        self.draw_buf.clear();
                    }
                }
                other => {
                    if let Some(mut cur) = self.draw_sheets.get(&key).cloned() {
                        match other {
                            UpAct::Unlock => {
                                if let Some(u) = cur.uploaded.as_mut() {
                                    u.locked = false;
                                }
                            }
                            UpAct::Relock => {
                                if let Some(u) = cur.uploaded.as_mut() {
                                    u.locked = true;
                                    u.stale = false;
                                }
                            }
                            UpAct::Delete => {
                                pla_ui::drawing::remove_upload_files(&updir, &cur);
                                cur.uploaded = None;
                            }
                            _ => {}
                        }
                        self.draw_sheets.upsert(cur);
                        self.draw_save();
                    }
                    self.draw_unlock_confirm = None;
                }
            }
            self.draw_up_page = 0;
            self.draw_load(&key);
        }
        // D5: 助手动作同样放在设置保存之后
        if let Some(act) = agent_act {
            match act {
                AgentAct::Send(text) => self.agent_send(&key, &text, &bom_title),
                AgentAct::Clear => self.agent_chat = pla_ui::draw_agent::Chat::new(&key),
                AgentAct::Reject(id) => self.agent_chat.decide(&id, false, ""),
                AgentAct::Apply(id, draw) => self.agent_apply(&key, &id, draw, &bom_title),
            }
        }
        // 预览在旁边的图纸窗口(show_draw_preview): 选了零件就显示, 切换就切换
        let _ = (&bal_rep, &bal_sel, &placed, &sel_dim);
        if self.draw_png.is_none() && job.is_none() {
            ui.label(egui::RichText::new(tr("还没出过图", "Not drawn yet")).color(COL_MUTED));
        }
    }

    /// 图纸预览窗口: 生产窗口「出图」标签选了零件时贴在它右边常显示(切换零件跟着换, 不选就消失; × = 取消选择)。
    /// 窗口可拖动 / 调大小; 图: 滚轮缩放(以光标为中心)、拖动平移、「适应」复位; 单击选尺寸 / 序号, 双击打开 PDF。
    fn show_draw_preview(&mut self, ctx: &egui::Context) {
        let Some(key) = self.draw_sel.clone().filter(|_| self.show_bom && self.bom_tab == 2) else {
            if self.draw_prev_win.open_rect.is_some() {
                self.draw_prev_win.closed();
            }
            return;
        };
        let Some(s) = self.draw_sheets.get(&key).cloned() else { return };
        // 贴在生产窗口右边(屏幕放不下就放左边); 生产窗口挪过(松手后)就跟过去
        let anchor = self.bom_win.saved_rect.or(self.bom_win.open_rect);
        let size = self.draw_prev_win.saved_rect.map(|r| [r[2], r[3]]).unwrap_or([720.0, anchor.map(|a| a[3].max(480.0)).unwrap_or(560.0)]);
        let place = |a: [f32; 4]| -> [f32; 2] {
            let screen_w = ctx.input(|i| i.viewport().monitor_size).map(|m| m.x).unwrap_or(1920.0);
            let x = if a[0] + a[2] + 6.0 + size[0] <= screen_w { a[0] + a[2] + 6.0 } else { (a[0] - 6.0 - size[0]).max(0.0) };
            [x, a[1]]
        };
        if self.draw_prev_win.open_rect.is_none() {
            if let Some(a) = anchor {
                let p = place(a);
                self.draw_prev_win.open_rect = Some([p[0], p[1], size[0], size[1]]);
            }
            self.draw_prev_anchor = anchor;
        } else if anchor != self.draw_prev_anchor {
            if let Some(a) = anchor {
                let p = place(a);
                ctx.send_viewport_cmd_to(egui::ViewportId::from_hash_of("panel-draw-prev"), egui::ViewportCommand::OuterPosition(egui::pos2(p[0], p[1])));
            }
            self.draw_prev_anchor = anchor;
        }
        // D8: 显示的是上传的图纸 → 纸张按上传页; 不能选尺寸 / 序号(那是生成图的报告)
        let up_view = self.draw_up_view;
        let rep = self.draw_report.as_ref().filter(|(k, _)| *k == key && up_view.is_none()).map(|(_, r)| r.clone());
        let paper = up_view.map(|u| u.2).or(rep.as_ref().and_then(|r| r.paper_mm)).unwrap_or([420.0, 297.0]);
        let placed = rep.as_ref().map(|r| r.placed.clone()).unwrap_or_default();
        let bal_rep = rep.as_ref().map(|r| r.balloons.clone()).unwrap_or_default();
        let mut page_to: Option<usize> = None;
        let sel_dim = self.draw_buf.get("dimsel").cloned().unwrap_or_default();
        let bal_sel = self.draw_buf.get("balsel").cloned().unwrap_or_default();
        let png = self.draw_png.clone();
        if png.as_ref().map(|p| &p.0) != self.draw_prev_uri.as_ref() {
            self.draw_prev_uri = png.as_ref().map(|p| p.0.clone());
            self.draw_zoom = 1.0;
            self.draw_pan = egui::Vec2::ZERO;
        }
        let busy = self.draw_job.as_ref().is_some_and(|j| j.key == key);
        let mut zoom = self.draw_zoom;
        let mut pan = self.draw_pan;
        let mut pick: Option<(f64, f64)> = None;
        let mut open_pdf = false;
        let mut win = std::mem::take(&mut self.draw_prev_win);
        let open = floating_window(
            ctx,
            FloatSpec {
                id: "panel-draw-prev",
                prefs_key: DRAW_PREV_RECT_KEY,
                title: tr("图纸", "Drawing"),
                subtitle: if up_view.is_some() { t!("{} · 上传的图纸", "{} · uploaded drawing", s.base) } else { s.base.clone() },
                close_tip: tr("关闭(取消选择零件)", "Close (deselects the part)"),
                default_rect: [200.0, 120.0, size[0], size[1]],
                min_size: egui::vec2(320.0, 240.0),
                resizable: true,
                fit_height: false,
            },
            &mut win,
            |ui| {
                ui.horizontal(|ui| {
                    if ui.small_button(tr("适应", "Fit")).on_hover_text(tr("缩放复位", "Reset zoom")).clicked() {
                        zoom = 1.0;
                        pan = egui::Vec2::ZERO;
                    }
                    ui.label(egui::RichText::new(format!("{:.0}%", zoom * 100.0)).color(COL_MUTED));
                    if let Some((i, n, _)) = up_view {
                        if n > 1 {
                            if ui.add_enabled(i > 0, egui::Button::new("◀").small()).clicked() {
                                page_to = Some(i - 1);
                            }
                            ui.label(format!("{}/{n}", i + 1));
                            if ui.add_enabled(i + 1 < n, egui::Button::new("▶").small()).clicked() {
                                page_to = Some(i + 1);
                            }
                        }
                        ui.label(egui::RichText::new(tr("上传的图纸 · 滚轮缩放 · 拖动平移 · 双击打开原文件", "Uploaded drawing · wheel = zoom · drag = pan · double-click = open the file")).size(11.0).color(COL_MUTED));
                    } else {
                        ui.label(
                            egui::RichText::new(tr("滚轮缩放 · 拖动平移 · 单击选尺寸 / 序号 · 双击打开 PDF", "Wheel = zoom · drag = pan · click = pick · double-click = open PDF"))
                                .size(11.0)
                                .color(COL_MUTED),
                        );
                    }
                    if busy {
                        ui.label(egui::RichText::new(tr("出图中…", "Drawing…")).color(COL_ACCENT));
                    }
                });
                let (area, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
                ui.painter().rect_filled(area, 2.0, egui::Color32::from_gray(40));
                let Some((uri, bytes)) = png.clone() else {
                    let msg = if s.locked() {
                        tr("上传的是 DWG / DXF, 没有预览(在出图标签里点「打开」)", "The upload is DWG / DXF: no preview (click Open in the drawings tab)")
                    } else {
                        tr("还没出过图", "Not drawn yet")
                    };
                    ui.painter().text(area.center(), egui::Align2::CENTER_CENTER, msg, egui::FontId::proportional(13.0), COL_MUTED);
                    return;
                };
                let aspect = (paper[0] / paper[1]) as f32;
                let fit_w = area.width().min(area.height() * aspect);
                // 滚轮: 以光标为中心缩放(0.2 ~ 12 倍)
                if resp.hovered() {
                    let dy = ui.input(|i| i.smooth_scroll_delta.y);
                    if dy != 0.0 {
                        let nz = (zoom * (dy * 0.003).exp()).clamp(0.2, 12.0);
                        if let Some(m) = resp.hover_pos() {
                            let c = area.center() + pan;
                            pan += (m - c) * (1.0 - nz / zoom);
                        }
                        zoom = nz;
                    }
                }
                if resp.dragged() {
                    pan += resp.drag_delta();
                }
                let sz = egui::vec2(fit_w * zoom, fit_w * zoom / aspect);
                let img = egui::Rect::from_center_size(area.center() + pan, sz);
                let painter = ui.painter_at(area);
                ui.set_clip_rect(area);
                egui::Image::from_bytes(uri, bytes).paint_at(ui, img);
                let to_scr = |at: [f64; 2]| egui::pos2(img.left() + (at[0] / paper[0]) as f32 * img.width(), img.top() + (1.0 - at[1] / paper[1]) as f32 * img.height());
                if let Some(b) = bal_rep.iter().find(|b| !bal_sel.is_empty() && b.key.as_deref() == Some(bal_sel.as_str())) {
                    painter.circle_stroke(to_scr(b.at), 12.0 * zoom.sqrt(), egui::Stroke::new(2.0_f32, COL_ACCENT));
                }
                if let Some(p) = placed.iter().find(|p| !sel_dim.is_empty() && p.name == sel_dim) {
                    painter.circle_stroke(to_scr(p.at), 14.0 * zoom.sqrt(), egui::Stroke::new(2.0_f32, COL_ACCENT));
                }
                if resp.double_clicked() {
                    open_pdf = true;
                } else if resp.clicked() {
                    if let Some(pos) = resp.interact_pointer_pos().filter(|p| img.contains(*p)) {
                        pick = Some((((pos.x - img.left()) / img.width()) as f64, ((pos.y - img.top()) / img.height()) as f64));
                    }
                }
            },
        );
        self.draw_prev_win = win;
        self.draw_zoom = zoom;
        self.draw_pan = pan;
        if let Some((fx, fy)) = pick {
            if let Some(b) = pla_ui::drawing::nearest_balloon(&bal_rep, paper, fx, fy) {
                self.draw_buf.insert("balsel".into(), b.key.clone().unwrap_or_default());
            } else {
                let hit = pla_ui::drawing::nearest_dim(&placed, paper, fx, fy).map(|p| p.name.clone()).unwrap_or_default();
                self.draw_buf.insert("dimsel".into(), hit);
                self.draw_buf.insert("balsel".into(), String::new());
            }
        }
        if let Some(i) = page_to {
            self.draw_up_page = i;
            self.draw_load(&key);
        }
        if open_pdf {
            let dir = pla_ui::drawing::dir(&self.doc_key);
            let p = match (&up_view, &s.uploaded) {
                (Some(_), Some(u)) => dir.join(&u.file),
                _ => pla_ui::drawing::file(&dir, &s, "pdf"),
            };
            self.open_path(&p);
        }
        if !open {
            self.draw_sel = None;
            self.draw_prev_win.closed();
        }
    }

    /// B1/B2 BOM 浮窗: 范围(当前可见 / 某个配置)→ 相同零件合并计数的清单; 导出 CSV; 每个配置各出一份。
    fn show_bom_window(&mut self, ctx: &egui::Context) {
        // 模板表列多(OYM 16 列): 默认宽一些, 表格在窗口内横向滚动
        let default = Self::default_float_rect(ctx, 760.0, 460.0);
        let mut win = std::mem::take(&mut self.bom_win);
        let configs: Vec<String> = pla_ui::configs::STORE.list(&self.doc_key).into_iter().map(|m| m.name).collect();
        let open = floating_window(
            ctx,
            FloatSpec {
                id: "panel-bom",
                prefs_key: BOM_RECT_KEY,
                title: tr("生产", "Manufacture"),
                subtitle: String::new(),
                close_tip: tr("关闭", "Close"),
                default_rect: default,
                min_size: egui::vec2(NotesDrag::MIN_W, NotesDrag::MIN_H),
                resizable: true,
                fit_height: true,
            },
            &mut win,
            |ui| {
                if let Some(why) = self.feature_block(Feature::Bom) {
                    ui.label(egui::RichText::new(why).small().color(COL_RED));
                }
                // ---- 标签: 清单 / 零件库(B4) ----
                self.lib_tick();
                {
                    let labels = vec![
                        tr("BOM", "BOM").to_string(),
                        t!("零件库 {}", "Parts library {}", self.lib.entries.len()),
                        tr("出图", "Drawings").to_string(),
                        tr("外发", "Release").to_string(),
                    ];
                    let mut s = self.bom_tab as usize;
                    if rz_tabs(ui, &labels, &mut s, None) {
                        self.bom_tab = s as u8;
                    }
                    ui.add_space(4.0);
                }
                if self.bom_tab == 1 {
                    self.lib_ui(ui);
                    return;
                }
                if self.bom_tab == 2 {
                    self.bom_template_tick();
                    self.draw_ui(ui);
                    return;
                }
                if self.bom_tab == 3 {
                    self.bom_template_tick();
                    self.rel_ui(ui);
                    return;
                }
                // ---- 范围 + 操作 ----
                let mut rescan: Option<Option<String>> = None;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tr("范围", "Scope")).color(COL_MUTED));
                    egui::ComboBox::from_id_salt("bom-scope").width(160.0).selected_text(Self::bom_scope_label(&self.bom_scope)).show_ui(ui, |ui| {
                        if ui.selectable_label(self.bom_scope.is_none(), Self::bom_scope_label(&None)).clicked() {
                            rescan = Some(None);
                        }
                        for c in &configs {
                            let s = Some(c.clone());
                            if ui.selectable_label(self.bom_scope == s, Self::bom_scope_label(&s)).clicked() {
                                rescan = Some(s);
                            }
                        }
                    });
                    if ui.add_enabled(!self.bom_busy, egui::Button::new(tr("重新扫描", "Rescan"))).clicked() {
                        rescan = Some(self.bom_scope.clone());
                    }
                });
                // ---- 模板(JSON 文件: 切换 / 编辑, 保存即生效)+ 导出 ----
                self.bom_template_tick();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tr("模板", "Template")).color(COL_MUTED));
                    let mut pick: Option<String> = None;
                    egui::ComboBox::from_id_salt("bom-tpl").width(120.0).selected_text(self.bom_tpl_name.clone()).show_ui(ui, |ui| {
                        for n in &self.bom_tpl_list {
                            if ui.selectable_label(*n == self.bom_tpl_name, n).clicked() {
                                pick = Some(n.clone());
                            }
                        }
                    });
                    if let Some(n) = pick {
                        self.bom_tpl_select(n);
                    }
                    if ui.small_button(tr("编辑模板", "Edit template")).on_hover_text(tr("用系统默认程序打开模板 JSON(列 / 默认值 / 公式); 保存后自动生效", "Open the template JSON (columns / defaults / formulas) in the default app; applies on save")).clicked() {
                        let p = pla_ui::bom_table::path_in(&pla_ui::bom_table::dir(), &self.bom_tpl_name);
                        if let Err(e) = std::process::Command::new("cmd").args(["/C", "start", "", &p.display().to_string()]).spawn() {
                            self.set_notice((true, t!("打不开 {}: {e}", "Cannot open {}: {e}", p.display())));
                        }
                    }
                });
                if let Some(e) = &self.bom_tpl_err {
                    ui.label(egui::RichText::new(t!("模板读取出错(仍用上一次的): {e}", "Template read error (still using the last good one): {e}")).size(11.0).color(COL_RED));
                }
                ui.horizontal(|ui| {
                    let have = matches!(self.bom_result, Some(Ok(_)));
                    if ui.add_enabled(have && !self.bom_busy, egui::Button::new(tr("导出 CSV", "Export CSV"))).clicked() {
                        if let Some(Ok(v)) = self.bom_result.clone() {
                            let label = Self::bom_scope_label(&self.bom_result_scope);
                            match self.bom_export(&label, &v) {
                                Ok(p) => self.set_notice((false, t!("已导出 {}", "Exported {}", p.display()))),
                                Err(e) => self.set_notice((true, t!("导出失败: {e}", "Export failed: {e}"))),
                            }
                        }
                    }
                    let tip = tr("对每个已存的配置各扫一次并各导出一份 CSV(不切换显示)", "Scan every saved configuration and export one CSV each (no display switching)");
                    if ui.add_enabled(!configs.is_empty() && !self.bom_busy, egui::Button::new(tr("每个配置各出一份", "One per configuration"))).on_hover_text(tip).clicked() {
                        let mut q: std::collections::VecDeque<String> = configs.iter().cloned().collect();
                        if let Some(first) = q.pop_front() {
                            self.bom_batch = Some((q, Vec::new()));
                            self.bom_scan(Some(first));
                        }
                    }
                    if ui.button(tr("打开文件夹", "Open folder")).clicked() {
                        if let Err(e) = pla_ui::bom::STORE.open_dir(&self.doc_key) {
                            self.set_notice((true, e));
                        }
                    }
                });
                if let Some(s) = rescan {
                    self.bom_scope = s.clone();
                    self.bom_scan(s);
                }
                // ---- 本文档的字段默认值(如本项目套数) ----
                let cols = self.bom_tpl.columns.clone();
                let mut edits_changed = false;
                egui::CollapsingHeader::new(tr("本文档默认值(如套数)", "Document defaults (e.g. sets)")).id_salt("bom-defaults").show(ui, |ui| {
                    egui::Grid::new("bom-defaults-grid").num_columns(2).spacing([8.0, 3.0]).show(ui, |ui| {
                        for c in cols.iter().filter(|c| c.source.as_deref() == Some("field") && c.formula.is_none()) {
                            ui.label(egui::RichText::new(&c.header).color(COL_MUTED));
                            let cur = self.bom_edits.defaults.get(&c.id).cloned().or_else(|| c.default.clone()).unwrap_or_default();
                            let key = format!("def|{}", c.id);
                            let buf = self.bom_cell_buf.entry(key).or_insert_with(|| cur.clone());
                            let te = ui.add(egui::TextEdit::singleline(buf).desired_width(90.0));
                            if te.lost_focus() {
                                let v = buf.trim().to_string();
                                if v == c.default.clone().unwrap_or_default() || v.is_empty() {
                                    self.bom_edits.defaults.remove(&c.id);
                                } else {
                                    self.bom_edits.defaults.insert(c.id.clone(), v);
                                }
                                edits_changed = true;
                            } else if !te.has_focus() {
                                *buf = cur;
                            }
                            ui.end_row();
                        }
                    });
                });
                ui.add_space(4.0);
                // ---- 结果(按模板; 可编辑格: 改了即覆盖并保存, 清空 = 恢复原值; 公式列只读) ----
                match &self.bom_result {
                    _ if self.bom_busy => {
                        ui.label(egui::RichText::new(tr("扫描中…(大文档需要几秒)", "Scanning… (a few seconds for large documents)")).color(COL_MUTED));
                    }
                    None => {
                        ui.label(egui::RichText::new(tr("还没有结果", "No result yet")).color(COL_MUTED));
                    }
                    Some(Err(e)) => {
                        ui.label(egui::RichText::new(t!("扫描失败: {e}", "Scan failed: {e}")).color(COL_RED));
                    }
                    Some(Ok(v)) => {
                        let v = v.clone();
                        let (rows, totals) = pla_ui::bom_table::build(&v, &self.bom_tpl, &self.bom_edits, &self.lib);
                        let lib_list: Vec<(String, String)> = self.lib.entries.iter().map(|e| (e.pn.clone(), e.name.clone())).collect();
                        let model_names: std::collections::HashMap<String, String> = v["lines"].as_array().into_iter().flatten().map(|l| (l["key"].as_str().unwrap_or("").to_string(), l["name"].as_str().unwrap_or("").to_string())).collect();
                        enum LibAct { Link(String, String), Unlink(String), Add(usize), Edit(String) }
                        let mut lib_acts: Vec<LibAct> = Vec::new();
                        let link_search = &mut self.lib_link_search;
                        let (kinds, total) = pla_ui::bom::totals(&v);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(t!("{}: 模型里 {kinds} 种 / 共 {total} 件", "{}: {kinds} distinct in the model / {total} in total", Self::bom_scope_label(&self.bom_result_scope))).strong());
                            let n_ex = self.bom_edits.excluded.len();
                            if n_ex > 0 && ui.small_button(t!("恢复已排除({n_ex})", "Restore excluded ({n_ex})")).clicked() {
                                self.bom_edits.excluded.clear();
                                edits_changed = true;
                            }
                            ui.add_space(12.0);
                            ui.label(egui::RichText::new(tr("冻结前", "Freeze first")).color(COL_MUTED));
                            let mut nf = self.bom_freeze.min(cols.len());
                            if ui.add(egui::DragValue::new(&mut nf).range(0..=cols.len())).on_hover_text(tr("左边这几列不随横向滚动", "These columns stay put when scrolling sideways")).changed() {
                                self.bom_freeze = nf;
                                self.bom_cols_dirty = true;
                            }
                            ui.label(egui::RichText::new(tr("列", "columns")).color(COL_MUTED));
                            if !self.bom_col_w.is_empty() && ui.small_button(tr("重置列宽", "Reset widths")).on_hover_text(tr("拖表头右边缘可改列宽", "Drag a header's right edge to resize")).clicked() {
                                self.bom_col_w.clear();
                                self.bom_cols_dirty = true;
                            }
                        });
                        if let Some(sk) = v["skipped"].as_array().filter(|a| !a.is_empty()) {
                            let names: Vec<&str> = sk.iter().filter_map(|x| x["name"].as_str()).collect();
                            ui.label(egui::RichText::new(t!("{} 个实体没算进来(读不到网格): {}", "{} solid(s) skipped (no mesh): {}", sk.len(), names.join(", "))).size(11.0).color(COL_RED));
                        }
                        // 名称格悬停 = 各实例与所在组
                        let mut members_of: std::collections::HashMap<String, String> = std::collections::HashMap::new();
                        for l in v["lines"].as_array().into_iter().flatten() {
                            let ms: Vec<String> = l["members"].as_array().into_iter().flatten().map(|m| {
                                let path = m["path"].as_array().map(|p| p.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" / ")).unwrap_or_default();
                                if path.is_empty() { m["name"].as_str().unwrap_or("").to_string() } else { format!("{} · {path}", m["name"].as_str().unwrap_or("")) }
                            }).collect();
                            let rm = pla_ui::bom::remark(l);
                            members_of.insert(l["key"].as_str().unwrap_or("").to_string(), if rm.is_empty() { ms.join("\n") } else { format!("{}\n{rm}", ms.join("\n")) });
                        }
                        let mut cell_edits: Vec<(String, String, String)> = Vec::new();
                        let mut remove_row: Option<String> = None;
                        let buf_map = &mut self.bom_cell_buf;
                        // 列宽(表头右缘拖动; 按列 id 记住)+ 冻结前 N 列(左边一块不随横向滚动)
                        let nf = self.bom_freeze.min(cols.len());
                        let mut ws: Vec<f32> = cols
                            .iter()
                            .map(|c| {
                                let d = if c.source.as_deref() == Some("name") || c.id == "model" || c.id == "usage" { 140.0 } else { 64.0 };
                                *self.bom_col_w.get(&c.id).unwrap_or(&d)
                            })
                            .collect();
                        let ws0 = ws.clone();
                        const RH: f32 = 20.0;
                        let mut cell = |ui: &mut egui::Ui, r: &pla_ui::bom_table::Row, j: usize, w: f32| {
                            let c = &cols[j];
                            let val = &r.cells[j];
                            if c.formula.is_some() {
                                fixed_label(ui, w, egui::RichText::new(val).color(egui::Color32::WHITE));
                                return;
                            }
                            let key = format!("{}|{}", r.key, c.id);
                            let buf = buf_map.entry(key).or_insert_with(|| val.clone());
                            // 橙 = 本文档手工覆盖; 蓝 = 来自零件库
                            let col = if r.overridden[j] { COL_ACCENT } else if r.from_lib[j] { rgb_hex(pla_bridge::shim::HIGHLIGHT_CANDIDATE) } else { egui::Color32::WHITE };
                            let te = ui.add_sized([w, RH], egui::TextEdit::singleline(buf).text_color(col));
                            if c.source.as_deref() == Some("name") {
                                if let Some(m) = members_of.get(&r.key) {
                                    te.clone().on_hover_text(m.as_str());
                                }
                            }
                            if te.lost_focus() {
                                if *buf != *val {
                                    cell_edits.push((r.key.clone(), c.id.clone(), buf.trim().to_string()));
                                }
                            } else if !te.has_focus() {
                                *buf = val.clone();
                            }
                        };
                        let head = |ui: &mut egui::Ui, j: usize, w: &mut f32| {
                            let c = &cols[j];
                            let h = egui::RichText::new(&c.header).size(11.5).color(if c.formula.is_some() { COL_ACCENT } else { COL_MUTED });
                            col_head(ui, ("bom-col", &c.id), h, c.formula.as_deref(), w);
                        };
                        let totals_cell = |ui: &mut egui::Ui, j: usize, w: f32| {
                            if cols[j].sum {
                                fixed_label(ui, w, egui::RichText::new(&totals[j]).strong().color(COL_ACCENT));
                            } else if cols.get(j + 1).is_some_and(|n| n.sum) {
                                fixed_label(ui, w, egui::RichText::new(tr("合计", "total")).color(COL_MUTED));
                            } else {
                                fixed_label(ui, w, egui::RichText::new(""));
                            }
                        };
                        let has_sum = cols.iter().any(|c| c.sum);
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                        // 冻结块: 序号 + 前 nf 列
                        egui::Grid::new("bom-grid-frozen").striped(true).min_row_height(RH + 2.0).spacing([8.0, 3.0]).show(ui, |ui| {
                            fixed_label(ui, 22.0, egui::RichText::new("#").color(COL_MUTED).size(11.5));
                            for j in 0..nf {
                                head(ui, j, &mut ws[j]);
                            }
                            ui.end_row();
                            for (i, r) in rows.iter().enumerate() {
                                fixed_label(ui, 22.0, if r.manual { egui::RichText::new(format!("{}*", i + 1)).color(COL_MUTED) } else { egui::RichText::new(format!("{}", i + 1)) })
                                    .on_hover_text(if r.manual { tr("手动行(模型里没有)", "Manual row (not in the model)") } else { "" });
                                for j in 0..nf {
                                    cell(ui, r, j, ws[j]);
                                }
                                ui.end_row();
                            }
                            if has_sum {
                                fixed_label(ui, 22.0, egui::RichText::new(""));
                                for j in 0..nf {
                                    totals_cell(ui, j, ws[j]);
                                }
                                ui.end_row();
                            }
                        });
                        if nf > 0 {
                            ui.separator();
                        }
                        egui::ScrollArea::horizontal().id_salt("bom-h").show(ui, |ui| {
                            egui::Grid::new("bom-grid").striped(true).min_row_height(RH + 2.0).spacing([8.0, 3.0]).show(ui, |ui| {
                                for j in nf..cols.len() {
                                    head(ui, j, &mut ws[j]);
                                }
                                ui.label("");
                                ui.end_row();
                                for (i, r) in rows.iter().enumerate() {
                                    for j in nf..cols.len() {
                                        cell(ui, r, j, ws[j]);
                                    }
                                    ui.horizontal(|ui| {
                                        // B4 "库"菜单: 链接 / 建议 / 存入零件库 / 取消链接 / 编辑条目
                                        let label = match (&r.link, &r.suggest) {
                                            (Some(_), _) => egui::RichText::new("库").color(rgb_hex(pla_bridge::shim::HIGHLIGHT_CANDIDATE)),
                                            (None, Some(_)) => egui::RichText::new("库?").color(COL_ACCENT),
                                            _ => egui::RichText::new("库").color(COL_MUTED),
                                        };
                                        let mut tip = match (&r.link, &r.suggest) {
                                            (Some(pn), _) => t!("已链接零件库 {pn}", "Linked to library {pn}"),
                                            (None, Some(pn)) => t!("零件库里可能是 {pn}", "Library suggests {pn}"),
                                            _ => tr("零件库: 存入 / 链接", "Parts library: save / link").to_string(),
                                        };
                                        if let Some(sp) = &r.spec {
                                            tip = t!("{tip}\n识别为标准件: {}", "{tip}\nRecognized standard part: {}", sp.text());
                                        }
                                        let b = ui.small_button(label).on_hover_text(tip);
                                        let pid = ui.id().with(("bom-lib-menu", &r.key));
                                        if b.clicked() {
                                            ui.memory_mut(|m| m.toggle_popup(pid));
                                        }
                                        egui::popup::popup_below_widget(ui, pid, &b, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                                            ui.set_min_width(240.0);
                                            if let Some(sp) = &r.spec {
                                                ui.label(egui::RichText::new(t!("识别: {}", "Recognized: {}", sp.text())).color(COL_MUTED))
                                                    .on_hover_text(tr("按几何识别的标准件规格; 同规格的库条目会被建议链接(名字不同也认, 长度不同则算新物料)", "Standard part recognized from geometry; library entries with the same spec are suggested (any name; a different length is a new item)"));
                                            }
                                            match &r.link {
                                                Some(pn) => {
                                                    ui.label(egui::RichText::new(t!("已链接 {pn}", "Linked {pn}")).strong());
                                                    if ui.button(tr("在零件库里编辑", "Edit in the library")).clicked() {
                                                        lib_acts.push(LibAct::Edit(pn.clone()));
                                                        ui.memory_mut(|m| m.close_popup());
                                                    }
                                                    if ui.button(tr("取消链接", "Unlink")).clicked() {
                                                        lib_acts.push(LibAct::Unlink(r.key.clone()));
                                                        ui.memory_mut(|m| m.close_popup());
                                                    }
                                                }
                                                None => {
                                                    if let Some(pn) = &r.suggest {
                                                        let nm = lib_list.iter().find(|x| &x.0 == pn).map(|x| x.1.clone()).unwrap_or_default();
                                                        if ui.button(t!("链接到建议: {pn} {nm}", "Link to suggestion: {pn} {nm}")).clicked() {
                                                            lib_acts.push(LibAct::Link(r.key.clone(), pn.clone()));
                                                            ui.memory_mut(|m| m.close_popup());
                                                        }
                                                    }
                                                    if ui.button(tr("＋ 存入零件库(新料号)", "+ Save to the library (new part number)")).clicked() {
                                                        lib_acts.push(LibAct::Add(i));
                                                        ui.memory_mut(|m| m.close_popup());
                                                    }
                                                    ui.separator();
                                                    ui.add(egui::TextEdit::singleline(link_search).hint_text(tr("搜索零件库链接…", "Search the library to link…")).desired_width(220.0));
                                                    let q = link_search.trim().to_lowercase();
                                                    for (pn, nm) in lib_list.iter().filter(|(pn, nm)| q.is_empty() || pn.to_lowercase().contains(&q) || nm.to_lowercase().contains(&q)).take(8) {
                                                        if ui.selectable_label(false, format!("{pn}  {nm}")).clicked() {
                                                            lib_acts.push(LibAct::Link(r.key.clone(), pn.clone()));
                                                            ui.memory_mut(|m| m.close_popup());
                                                        }
                                                    }
                                                }
                                            }
                                        });
                                        let tip = if r.manual { tr("删除这一手动行", "Delete this manual row") } else { tr("从 BOM 里排除(不影响模型; 可恢复)", "Exclude from the BOM (model unaffected; restorable)") };
                                        if ui.small_button("×").on_hover_text(tip).clicked() {
                                            remove_row = Some(r.key.clone());
                                        }
                                    });
                                    ui.end_row();
                                }
                                // 合计行
                                if has_sum {
                                    for j in nf..cols.len() {
                                        totals_cell(ui, j, ws[j]);
                                    }
                                    ui.label("");
                                    ui.end_row();
                                }
                            });
                        });
                        });
                        if ws != ws0 {
                            for (c, w) in cols.iter().zip(&ws) {
                                self.bom_col_w.insert(c.id.clone(), *w);
                            }
                            self.bom_cols_dirty = true;
                        }
                        if self.bom_cols_dirty && !ui.input(|i| i.pointer.any_down()) {
                            self.bom_cols_dirty = false;
                            self.bom_cols_save();
                        }
                        for a in lib_acts {
                            match a {
                                LibAct::Link(rk, pn) => {
                                    self.bom_edits.links.insert(rk, pn);
                                    self.bom_cell_buf.clear();
                                    edits_changed = true;
                                }
                                LibAct::Unlink(rk) => {
                                    self.bom_edits.links.remove(&rk);
                                    self.bom_cell_buf.clear();
                                    edits_changed = true;
                                }
                                LibAct::Add(i) => {
                                    if let Some(r) = rows.get(i).cloned() {
                                        let mn = model_names.get(&r.key).cloned().unwrap_or_default();
                                        self.lib_add_from_row(&r, &mn);
                                    }
                                }
                                LibAct::Edit(pn) => {
                                    if let Some(e) = self.lib.find(&pn).cloned() {
                                        self.lib_edit_aliases = e.aliases.join(", ");
                                        self.lib_edit_orig = Some(pn);
                                        self.lib_edit = Some(e);
                                        self.bom_tab = 1;
                                    }
                                }
                            }
                        }
                        if ui.button(tr("＋ 手动行(模型里没有的采购件 / 工具)", "+ Manual row (purchased parts / tools not in the model)")).clicked() {
                            self.bom_edits.add_manual();
                            edits_changed = true;
                        }
                        for (rk, cid, val) in cell_edits {
                            // 清空 = 去掉覆盖(恢复模型值 / 默认值); 手动行直接存
                            let base = if rk.starts_with("manual:") { "\u{0}".to_string() } else if val.is_empty() { String::new() } else { "\u{0}".to_string() };
                            self.bom_edits.set_cell(&rk, &cid, &val, &base);
                            self.bom_cell_buf.remove(&format!("{rk}|{cid}"));
                            edits_changed = true;
                        }
                        if let Some(rk) = remove_row {
                            if let Some(mid) = rk.strip_prefix("manual:") {
                                self.bom_edits.manual.retain(|m| m.id != mid);
                            } else if !self.bom_edits.excluded.contains(&rk) {
                                self.bom_edits.excluded.push(rk);
                            }
                            edits_changed = true;
                        }
                    }
                }
                if edits_changed {
                    self.bom_edits.template = self.bom_tpl_name.clone();
                    if let Err(e) = pla_ui::bom_table::save_edits(&self.doc_key, &self.bom_edits) {
                        self.set_notice((true, t!("BOM 编辑保存失败: {e}", "Saving BOM edits failed: {e}")));
                    }
                }
            },
        );
        self.bom_win = win;
        if !open {
            self.show_bom = false;
            self.bom_win.closed();
        }
    }

    /// P2: 从文件重读尺寸记录与组(任何增删改之后)。
    fn dims_reload(&mut self) {
        let f = pla_ui::dims::load_file(&self.doc_key);
        self.dims_eval = pla_ui::dims::evaluate(&f);
        self.dims_list = f.dims;
        self.dims_groups = f.groups;
        self.dims_params = f.params;
    }

    /// N3d: 「操作」标题行右侧 ——当前选择正是某条记录 → 显示记录名(保存按钮在「记录」标签, 用户 2026-11-15)。
    fn dim_record_control(&mut self, ui: &mut egui::Ui, row: egui::Rect, sel: &[String]) {
        if self.dim_active.is_some() && sel == self.dim_active_ids.as_slice() {
            let name = self.dim_active.clone().unwrap_or_default();
            paint_text(ui.painter(), egui::pos2(row.right(), row.center().y), egui::Align2::RIGHT_CENTER, &name, 12.0, COL_ACCENT, true);
        }
    }

    /// N4b: 孔规格方案 —— 列表 / 当前方案的规格; 每秒查一次文件修改时间, 改了就重读(热生效; 读错如实报, 保留上一次能用的)。
    fn hole_presets_tick(&mut self) {
        let due = self.hole_preset_checked.map_or(true, |t| t.elapsed() >= Duration::from_secs(1));
        if !due {
            return;
        }
        self.hole_preset_checked = Some(Instant::now());
        let dir = pla_ui::hole_presets::dir();
        self.hole_presets = pla_ui::hole_presets::list_in(&dir);
        if !self.hole_presets.contains(&self.hole_preset) {
            self.hole_preset = self.hole_presets.first().cloned().unwrap_or_default();
            self.hole_preset_mtime = None;
        }
        let m = pla_ui::hole_presets::mtime_in(&dir, &self.hole_preset);
        if m.is_some() && m == self.hole_preset_mtime && !self.hole_sizes.is_empty() {
            return;
        }
        self.hole_preset_mtime = m;
        match pla_ui::hole_presets::load_in(&dir, &self.hole_preset) {
            Ok(s) => {
                let keep = self.hole_std.and_then(|i| self.hole_sizes.get(i)).map(|x| x.size.clone());
                self.hole_sizes = s;
                self.hole_preset_err = None;
                // 规格按名字保留(没有就 M4 / 第一个), 数值跟着新方案刷新
                let want = keep.or_else(|| Some("M4".to_string()));
                let i = want.and_then(|w| self.hole_sizes.iter().position(|x| x.size == w)).unwrap_or(0);
                if self.hole_std.is_some() || self.hole_vals[0].is_empty() {
                    self.hole_std = Some(i);
                    self.hole_vals = std_hole_vals(&self.hole_sizes[i], self.hole_usage);
                }
            }
            Err(e) => self.hole_preset_err = Some(e),
        }
    }

    fn hole_preset_select(&mut self, name: String) {
        self.hole_preset = name;
        self.hole_preset_mtime = None;
        self.hole_preset_checked = None;
        let _ = pla_ui::prefs::save_value("hole_preset", serde_json::json!(self.hole_preset));
    }

    /// P3/N4b 打孔表单(「操作」→ 打孔)。选择 = 一个平面(入口)+ 可选的草图点(曲线端点 / 圆; 一点一孔, 没有 = 面中点)
    /// + 可选的其它实体(组合孔, 按选择顺序, 最后一个 = 底孔)。规格方案 / 规格 / 用途 / 类型 / 深度 / 数值 → OK(回车 / 右键)= 垫片 hole.make,
    /// 一次原子执行(失败整体撤回); 成功后每个孔在「记录」里生成孔特征 + 自动子尺寸。
    fn hole_ui(&mut self, ui: &mut egui::Ui, info: Option<&serde_json::Value>, faces: &[String]) {
        self.hole_presets_tick();
        let plane = info.filter(|v| faces.len() == 1 && v["surface"] == "Plane" && v["axes"].is_object()).cloned();
        let u = match plane.as_ref().and_then(|v| v["units"].as_str()).unwrap_or("millimeter") {
            "millimeter" => "mm", "centimeter" => "cm", "meter" => "m", "inch" => "in", "foot" => "ft", x => x,
        }
        .to_string();
        // 孔位: 选择里的草图点 / 曲线; 组合孔: 选择里的实体(去掉入口面所在实体, 垫片也会再去一次)
        let points: Vec<serde_json::Value> = self
            .live_sel
            .iter()
            .filter(|it| it.kind == "curveVertex" || it.kind == "curve")
            .map(|it| serde_json::json!({ "kind": it.kind, "id": it.id }))
            .collect();
        let entry_solid = plane.as_ref().and_then(|v| v["solid"].as_i64()).map(|x| x.to_string());
        let stack: Vec<String> = self.live_sel.iter().filter(|it| it.kind == "solid" && Some(&it.id) != entry_solid.as_ref()).map(|it| it.id.clone()).collect();
        let combo = !stack.is_empty();
        match &plane {
            None => rz_info_card(
                ui,
                tr("选一个平面(孔的入口)", "Select a plane face (the hole entry)"),
                tr(
                    "再 Shift 加选草图点(曲线端点 / 圆)= 孔位, 一点一孔; 不选点 = 面中点。\n再加选别的实体 = 组合孔(最后一个实体 = 底孔)。",
                    "Shift-add sketch points (curve ends / circles) = hole positions, one hole each; none = face midpoint.\nShift-add more solids = stacked hole (the last solid gets the tap hole).",
                ),
            ),
            Some(_) => {
                let n = points.len();
                let where_ = if n == 0 { tr("面中点 1 个孔", "1 hole at the face midpoint").to_string() } else { t!("{n} 处草图点位", "{n} sketch point position(s)") };
                let sub = if combo {
                    t!("{where_} · 组合孔: 入口实体 + {} 件(最后一件 = 底孔)", "{where_} · stacked: entry solid + {} (last = tap hole)", stack.len())
                } else {
                    where_
                };
                rz_info_card(ui, tr("在选中的平面上打孔", "Hole in the selected plane"), &sub);
            }
        }
        // ---- 规格方案(JSON 文件; 下拉切换 · 改名 · 新建 · 编辑) ----
        rz_label(ui, tr("规格方案", "Size table"));
        ui.horizontal(|ui| {
            if let Some(buf) = self.hole_preset_rename.as_mut() {
                let e = ui.add(egui::TextEdit::singleline(buf).desired_width(150.0));
                e.request_focus();
                let (en, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                if en || right_click_confirm(ui) {
                    let new = buf.trim().to_string();
                    self.hole_preset_rename = None;
                    match pla_ui::hole_presets::rename_in(&pla_ui::hole_presets::dir(), &self.hole_preset, &new) {
                        Ok(()) => self.hole_preset_select(new),
                        Err(e) => self.set_notice((true, t!("方案改名失败: {e}", "Rename failed: {e}"))),
                    }
                } else if esc {
                    self.hole_preset_rename = None;
                }
                return;
            }
            let mut pick: Option<String> = None;
            egui::ComboBox::from_id_salt("hole-preset").width(150.0).selected_text(self.hole_preset.clone()).show_ui(ui, |ui| {
                for p in &self.hole_presets {
                    if ui.selectable_label(*p == self.hole_preset, p).clicked() {
                        pick = Some(p.clone());
                    }
                }
            });
            if let Some(p) = pick {
                self.hole_preset_select(p);
            }
            if ui.small_button(tr("改名", "Rename")).on_hover_text(tr("改方案名(= 改文件名)", "Rename the table (= its file name)")).clicked() {
                self.hole_preset_rename = Some(self.hole_preset.clone());
            }
            if ui.small_button(tr("＋ 新建", "+ New")).on_hover_text(tr("复制当前方案成一个新方案", "Copy the current table into a new one")).clicked() {
                match pla_ui::hole_presets::create_in(&pla_ui::hole_presets::dir(), &self.hole_preset) {
                    Ok(n) => self.hole_preset_select(n),
                    Err(e) => self.set_notice((true, t!("新建方案失败: {e}", "New table failed: {e}"))),
                }
            }
            if ui.small_button(tr("编辑", "Edit")).on_hover_text(tr("用系统默认程序打开 JSON; 保存后自动生效", "Open the JSON in the default app; changes apply on save")).clicked() {
                let p = pla_ui::hole_presets::path_in(&pla_ui::hole_presets::dir(), &self.hole_preset);
                if let Err(e) = std::process::Command::new("cmd").args(["/C", "start", "", &p.display().to_string()]).spawn() {
                    self.set_notice((true, t!("打不开 {}: {e}", "Cannot open {}: {e}", p.display())));
                }
            }
        });
        if let Some(e) = &self.hole_preset_err {
            ui.label(egui::RichText::new(t!("方案读取出错(仍用上一次的): {e}", "Table read error (still using the last good one): {e}")).size(11.0).color(COL_RED));
        }
        // ---- 规格 + 用途 ----
        rz_label(ui, tr("规格", "Size"));
        ui.horizontal(|ui| {
            let cur = self.hole_std.and_then(|i| self.hole_sizes.get(i)).map_or(tr("自定义", "Custom").to_string(), |s| s.size.clone());
            let mut pick: Option<Option<usize>> = None;
            egui::ComboBox::from_id_salt("hole-std").width(90.0).selected_text(cur).show_ui(ui, |ui| {
                if ui.selectable_label(self.hole_std.is_none(), tr("自定义", "Custom")).clicked() {
                    pick = Some(None);
                }
                for (i, s) in self.hole_sizes.iter().enumerate() {
                    if ui.selectable_label(self.hole_std == Some(i), &s.size).clicked() {
                        pick = Some(Some(i));
                    }
                }
            });
            if let Some(p) = pick {
                self.hole_std = p;
                if let Some(s) = p.and_then(|i| self.hole_sizes.get(i)) {
                    self.hole_vals = std_hole_vals(s, self.hole_usage);
                }
            }
        });
        // 用途只对单实体孔有意义(组合孔固定: 过孔 + 底孔)
        if self.hole_std.is_some() && !combo {
            let labels = vec![tr("过孔", "Clearance").to_string(), tr("螺纹底孔", "Tap drill").to_string()];
            let mut s = self.hole_usage as usize;
            if rz_segmented(ui, "hole-usage", &labels, &mut s) {
                self.hole_usage = s as u8;
                if let Some(sz) = self.hole_std.and_then(|i| self.hole_sizes.get(i)) {
                    self.hole_vals = std_hole_vals(sz, self.hole_usage);
                }
            }
        }
        // 组合孔: d 一律按过孔
        if combo && self.hole_usage == 1 {
            self.hole_usage = 0;
            if let Some(sz) = self.hole_std.and_then(|i| self.hole_sizes.get(i)) {
                self.hole_vals = std_hole_vals(sz, 0);
            }
        }
        // ---- 类型 / 深度 ----
        rz_label(ui, tr("类型", "Type"));
        let labels: Vec<String> = HOLE_TYPES.iter().map(|t| hole_type_label(t).to_string()).collect();
        let mut s = self.hole_type as usize;
        if rz_segmented(ui, "hole-type", &labels, &mut s) {
            self.hole_type = s as u8;
        }
        rz_label(ui, if combo { tr("底孔深度(最后一件)", "Tap hole depth (last solid)") } else { tr("深度", "Depth") });
        let labels = vec![tr("通孔", "Through").to_string(), tr("盲孔", "Blind").to_string()];
        let mut s = if self.hole_through { 0 } else { 1 };
        if rz_segmented(ui, "hole-through", &labels, &mut s) {
            self.hole_through = s == 0;
        }
        // ---- 数值(改了任何一个 → 规格变"自定义") ----
        let mut enter = false;
        let before = self.hole_vals.clone();
        let ty = HOLE_TYPES[self.hole_type as usize];
        ui.columns(2, |cols| {
            rz_label(&mut cols[0], if combo { tr("过孔径", "Clearance dia.") } else { tr("孔径", "Diameter") });
            enter |= rz_field(&mut cols[0], &mut self.hole_vals[0], &u, true);
            if combo {
                rz_label(&mut cols[1], tr("底孔径", "Tap dia."));
                enter |= rz_field(&mut cols[1], &mut self.hole_vals[5], &u, true);
            } else if !self.hole_through {
                rz_label(&mut cols[1], tr("孔深", "Depth"));
                enter |= rz_field(&mut cols[1], &mut self.hole_vals[1], &u, true);
            }
        });
        if combo && !self.hole_through {
            ui.columns(2, |cols| {
                rz_label(&mut cols[0], tr("底孔深(从最后一件的入口)", "Tap depth (from the last solid)"));
                enter |= rz_field(&mut cols[0], &mut self.hole_vals[1], &u, true);
            });
        }
        if ty == "counterbore" {
            ui.columns(2, |cols| {
                rz_label(&mut cols[0], tr("沉头直径", "Bore dia."));
                enter |= rz_field(&mut cols[0], &mut self.hole_vals[2], &u, true);
                rz_label(&mut cols[1], tr("沉头深度", "Bore depth"));
                enter |= rz_field(&mut cols[1], &mut self.hole_vals[3], &u, true);
            });
        } else if ty == "countersink" {
            ui.columns(2, |cols| {
                rz_label(&mut cols[0], tr("沉头直径(90°)", "Sink dia. (90°)"));
                enter |= rz_field(&mut cols[0], &mut self.hole_vals[4], &u, true);
            });
        }
        if self.hole_vals != before && self.hole_std.and_then(|i| self.hole_sizes.get(i)).is_some_and(|s| std_hole_vals(s, self.hole_usage) != self.hole_vals) {
            self.hole_std = None;
        }
        // ---- 校验 + 底部 ----
        let num = |s: &str| s.trim().parse::<f64>().ok();
        let d = num(&self.hole_vals[0]).filter(|x| *x > 0.0);
        let depth = num(&self.hole_vals[1]).filter(|x| *x > 0.0);
        let cbd = num(&self.hole_vals[2]).filter(|x| d.is_some_and(|d| *x > d));
        let cbh = num(&self.hole_vals[3]).filter(|x| *x > 0.0);
        let cskd = num(&self.hole_vals[4]).filter(|x| d.is_some_and(|d| *x > d));
        let tapd = num(&self.hole_vals[5]).filter(|x| *x > 0.0);
        let valid = d.is_some()
            && (self.hole_through || depth.is_some())
            && (!combo || tapd.is_some())
            && match ty { "counterbore" => cbd.is_some() && cbh.is_some(), "countersink" => cskd.is_some(), _ => true };
        let can = plane.is_some() && valid && !self.dim_busy;
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let (cancel, ok) = rz_footer(ui, can, self.dim_busy);
        if cancel || esc {
            if let Some(s) = self.hole_std.and_then(|i| self.hole_sizes.get(i)) {
                self.hole_vals = std_hole_vals(s, self.hole_usage);
            }
        }
        let go = ok || (can && (right_click_confirm(ui) || enter));
        if !(go && can && self.require_feature(Feature::DimFace)) {
            return;
        }
        let (Some(v), Some(d)) = (plane, d) else { return };
        let mut args = serde_json::json!({ "face": faces[0], "type": ty, "d": d, "units": v["units"], "points": points });
        if !self.hole_through {
            args["depth"] = serde_json::json!(depth);
        }
        if combo {
            args["stack"] = serde_json::json!(stack);
            args["tapD"] = serde_json::json!(tapd);
        }
        match ty {
            "counterbore" => {
                args["cbD"] = serde_json::json!(cbd);
                args["cbDepth"] = serde_json::json!(cbh);
            }
            "countersink" => args["cskD"] = serde_json::json!(cskd),
            _ => {}
        }
        let mut spec = args.clone();
        spec.as_object_mut().map(|o| o.remove("points"));
        spec["std"] = serde_json::json!(self.hole_std.and_then(|i| self.hole_sizes.get(i)).map(|s| s.size.clone()));
        spec["preset"] = serde_json::json!(self.hole_preset);
        spec["usage"] = serde_json::json!(if self.hole_usage == 1 { "tap" } else { "clearance" });
        self.cdp.send(pla_bridge::cdp::CdpCmd::Call { name: pla_bridge::shim::api::HOLE_MAKE, args });
        self.hole_pending = Some(spec);
        self.dim_busy = true;
        self.set_notice((false, tr("正在打孔…", "Making the hole(s)…").into()));
    }

    /// 「记录」标签顶部: 把「操作」里当前读到的尺寸存成一条记录。没读到 / 已经是某条记录 → 灰色说明, 不可点。
    fn dim_save_row(&mut self, ui: &mut egui::Ui, cur: Option<&serde_json::Value>, sel: &[String]) {
        let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
        let savable = cur.filter(|v| matches!(v["kind"].as_str(), Some("thickness" | "radius" | "filletRadius" | "distance")));
        let already = self.dim_active.clone().filter(|_| sel == self.dim_active_ids.as_slice());
        let can = savable.is_some() && already.is_none() && self.dim_pending.is_none();
        let hot = can && resp.hovered();
        let fill = if hot { lerp_color(COL_STEPPER, egui::Color32::WHITE, 0.12) } else if can { COL_STEPPER } else { COL_CARD };
        ui.painter().rect_filled(r, egui::Rounding::same(6.0), fill);
        let text = match (&already, savable) {
            (Some(n), _) => t!("当前选择已记录为 {n}", "Current selection is saved as {n}"),
            (None, Some(v)) => {
                let kind = match v["kind"].as_str().unwrap_or("") { "thickness" => tr("厚度", "thickness"), "radius" => tr("半径", "radius"), "filletRadius" => tr("圆角", "fillet"), _ => tr("距离", "distance") };
                let u = match v["units"].as_str().unwrap_or("") { "millimeter" => "mm", "centimeter" => "cm", "meter" => "m", "inch" => "in", "foot" => "ft", x => x };
                let val = self.dim_value_of(v).map(fmt3).unwrap_or_default();
                t!("＋ 保存当前尺寸({kind} {val} {u})", "+ Save current dimension ({kind} {val} {u})")
            }
            (None, None) => tr("在「操作」标签选面读出尺寸后可保存", "Select faces in the Operate tab to read a dimension, then save it here").to_string(),
        };
        let col = if can { if hot { egui::Color32::WHITE } else { COL_ACCENT } } else { COL_MUTED };
        paint_text(ui.painter(), r.center(), egui::Align2::CENTER_CENTER, &text, 12.0, col, can);
        if can && resp.on_hover_text(tr("存下来以后点条目就能再改(面换了 id 也能找回)", "Click the entry later to edit again (found even after face ids change)")).clicked() {
            if let Some(v) = savable.cloned() {
                self.dim_request_save(&v, None);
            }
        }
        ui.add_space(4.0);
    }

    /// N3d: 取当前尺寸各面的签名 → 新增记录(update = None)或覆盖已有记录(update = Some(名字))。
    fn dim_request_save(&mut self, v: &serde_json::Value, update: Option<String>) {
        let (ids, opp) = Self::dim_current_faces(v, self.dim_cand);
        let n = ids.len();
        let value = self.dim_value_of(v);
        let rec = pla_ui::dims::DimRecord {
            name: update.clone().unwrap_or_default(),
            mode: if v["kind"] == "distance" { "distance".into() } else { "face".into() },
            kind: v["kind"].as_str().unwrap_or("").into(),
            faces: Vec::new(),
            opposite: None,
            side: self.dim_side,
            sym: self.dim_sym,
            cyl: self.dim_cyl,
            value: value.unwrap_or(0.0),
            units: v["units"].as_str().unwrap_or("").into(),
            created: 0,
            fav: false,
            id: String::new(),
            parent: None,
            origin: "manual".into(),
            follow: false,
            expr: None,
        };
        let mut all = ids;
        all.extend(opp);
        self.cdp.send(pla_bridge::cdp::CdpCmd::Call {
            name: pla_bridge::shim::api::DIM_SIGNATURE,
            args: serde_json::json!({ "faces": all, "units": rec.units }),
        });
        self.dim_pending = Some(DimPending::Save(rec, n, update.is_some()));
    }

    /// 把当前尺寸改成 target(编辑区的应用 / 条目里直接改数值共用)。单面: offsetFace / refillet(按移动侧);
    /// 距离: moveFaces 整组平移。成功后若对应某条记录, 由 ExecResult 刷新记录。
    fn dim_apply(&mut self, v: &serde_json::Value, sel: &[String], target: f64) {
        let kind = v["kind"].as_str().unwrap_or("");
        let units = match v["units"].as_str().unwrap_or("") {
            "millimeter" => "mm", "centimeter" => "cm", "meter" => "m", "inch" => "in", "foot" => "ft", u => u,
        }
        .to_string();
        let script = if kind == "distance" {
            let Some(value) = v["value"].as_f64() else { return };
            let dir: Vec<f64> = v["direction"].as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
            if dir.len() != 3 {
                return;
            }
            self.dim_note_apply(v, sel, value, target);
            let d = target - value;
            let mv = |s: f64| serde_json::json!([dir[0] * d * s, dir[1] * d * s, dir[2] * d * s]);
            if Self::dim_symmetric(v, self.dim_sym) {
                // 对称: 与锚同侧的面 +d, 另一侧 −d(跨在参考面上的不动)。第二步按位置挑面(第一步之后面 id 已变)
                let moving = v["moving"].as_array().cloned().unwrap_or_default();
                let sides = v["sides"].as_array().cloned().unwrap_or_default();
                let centers = v["boxCenters"].as_array().cloned().unwrap_or_default();
                let near: Vec<serde_json::Value> = (0..moving.len())
                    .filter(|&i| sides.get(i).and_then(|s| s.as_i64()) == Some(-1))
                    .filter_map(|i| centers.get(i).filter(|c| c.is_array()).map(|c| serde_json::json!({ "near": c, "tol": 0.01 })))
                    .collect();
                let same: Vec<serde_json::Value> =
                    (0..moving.len()).filter(|&i| sides.get(i).and_then(|s| s.as_i64()) == Some(1)).map(|i| moving[i].clone()).collect();
                (value, serde_json::json!({ "atomic": true, "ops": [
                    { "op": "moveFaces", "faces": same, "move": mv(1.0) },
                    { "op": "moveFaces", "faces": near, "move": mv(-1.0) },
                ] }))
            } else {
                // 移动侧(用户 2026-11-15, 跨实体也要): 0 动的面 +d / 1 参考面 −d / 2 两边各半(第二步按包围盒中心挑参考面)
                let ops = match self.dim_side {
                    1 => serde_json::json!([{ "op": "moveFaces", "faces": [v["reference"]], "move": mv(-1.0) }]),
                    2 => serde_json::json!([
                        { "op": "moveFaces", "faces": v["moving"], "move": mv(0.5) },
                        { "op": "moveFaces", "faces": [{ "near": v["referenceBoxCenter"], "tol": 0.01 }], "move": mv(-0.5) },
                    ]),
                    _ => serde_json::json!([{ "op": "moveFaces", "faces": v["moving"], "move": mv(1.0) }]),
                };
                (value, serde_json::json!({ "atomic": true, "ops": ops }))
            }
        } else {
            let cands = v["candidates"].as_array().cloned().unwrap_or_default();
            let (value, opposite) = if kind == "thickness" && !cands.is_empty() {
                let c = &cands[self.dim_cand.min(cands.len() - 1)];
                (cand_value(c, self.dim_cyl), c["id"].as_str().map(String::from))
            } else {
                (v["value"].as_f64(), None)
            };
            let (Some(value), Some(op), Some(sign)) = (value, v["op"].as_str(), v["offsetSign"].as_f64()) else { return };
            self.dim_note_apply(v, sel, value, target);
            let face = v["face"].as_str().unwrap_or("").to_string();
            let delta = target - value;
            // 对面是圆柱(孔 / 凸台): 改它 = 沿 −n 整体平移(偏移会改半径); 两侧各半 = 先偏移选中面, 再按包围盒中心挑圆柱平移
            if kind == "thickness" && v["oppositeOp"] == "moveFaces" && self.dim_side != 0 {
                let n: Vec<f64> = v["normal"].as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
                let c = &cands[self.dim_cand.min(cands.len() - 1)];
                if n.len() == 3 {
                    let half = if self.dim_side == 2 { 0.5 } else { 1.0 };
                    let mv = serde_json::json!([-n[0] * delta * half, -n[1] * delta * half, -n[2] * delta * half]);
                    let ops = if self.dim_side == 2 {
                        serde_json::json!([
                            { "op": "offsetFace", "faces": [face], "distance": delta / 2.0 },
                            { "op": "moveFaces", "faces": [{ "near": c["boxCenter"], "tol": 0.01 }], "move": mv },
                        ])
                    } else {
                        serde_json::json!([{ "op": "moveFaces", "faces": [c["id"]], "move": mv }])
                    };
                    self.cdp.send(pla_bridge::cdp::CdpCmd::Exec(serde_json::json!({ "atomic": true, "ops": ops })));
                    self.dim_busy = true;
                    self.set_notice((false, t!("正在修改: {} → {} {units}…", "Changing: {} → {} {units}…", fmt3(value), fmt3(target))));
                    return;
                }
            }
            let (faces_arg, dist) = match (kind, self.dim_side, &opposite) {
                ("thickness", 1, Some(o)) => (serde_json::json!([o]), delta),
                ("thickness", 2, Some(o)) => (serde_json::json!([face, o]), delta / 2.0),
                // 多选等半径倒圆 / 圆柱: 一起改
                _ => (v["faces"].as_array().filter(|a| a.len() > 1).map(|a| serde_json::json!(a)).unwrap_or_else(|| serde_json::json!([face])), delta * sign),
            };
            (value, serde_json::json!({ "atomic": true, "ops": [{ "op": op, "faces": faces_arg, "distance": dist }] }))
        };
        let (value, script) = script;
        self.cdp.send(pla_bridge::cdp::CdpCmd::Exec(script));
        self.dim_busy = true;
        self.set_notice((false, t!("正在修改: {} → {} {units}…", "Changing: {} → {} {units}…", fmt3(value), fmt3(target))));
    }

    /// 当前尺寸值: 厚度 = 当前对面候选(圆柱按选的测量点); 其它 = 回包里的值。
    fn dim_value_of(&self, v: &serde_json::Value) -> Option<f64> {
        let cands = v["candidates"].as_array();
        match cands {
            Some(c) if v["kind"] == "thickness" && !c.is_empty() => cand_value(&c[self.dim_cand.min(c.len() - 1)], self.dim_cyl),
            _ => v["value"].as_f64(),
        }
    }

    /// N3c: 距离尺寸按"对称"改(动的面分在参考面两侧且选了对称)。
    fn dim_symmetric(v: &serde_json::Value, sym: u8) -> bool {
        v["kind"] == "distance" && v["mixed"] == true && sym == 1
    }

    /// N3d: 应用前登记 ——当前选择正是某条记录时, 记下各面(faces 之后 opposite)的平移量, 成功后据此刷新记录。
    fn dim_note_apply(&mut self, v: &serde_json::Value, sel: &[String], value: f64, target: f64) {
        self.dim_last_apply = None;
        let Some(name) = self.dim_active.clone() else { return };
        if sel != self.dim_active_ids.as_slice() {
            // 选择已换成别的面: 不再关联这条记录
            self.dim_active = None;
            return;
        }
        let Some(rec) = self.dims_list.iter().find(|r| r.name == name).cloned() else { return };
        let delta = target - value;
        let vec3 = |x: &serde_json::Value| -> [f64; 3] {
            let a: Vec<f64> = x.as_array().map(|a| a.iter().filter_map(|y| y.as_f64()).collect()).unwrap_or_default();
            if a.len() == 3 { [a[0], a[1], a[2]] } else { [0.0; 3] }
        };
        let scale = |n: [f64; 3], s: f64| [n[0] * s, n[1] * s, n[2] * s];
        let moves: Vec<[f64; 3]> = match v["kind"].as_str() {
            Some("distance") => {
                let d = vec3(&v["direction"]);
                let sym = Self::dim_symmetric(v, self.dim_sym);
                let sides = v["sides"].as_array().cloned().unwrap_or_default();
                // 参考面 / 动的面各自的平移系数(对称时不动参考面)
                let (kr, km) = if sym { (0.0, 1.0) } else { match self.dim_side { 1 => (-1.0, 0.0), 2 => (-0.5, 0.5), _ => (0.0, 1.0) } };
                std::iter::once(scale(d, delta * kr))
                    .chain((1..rec.faces.len()).map(|i| {
                        let s = if sym { sides.get(i - 1).and_then(|x| x.as_f64()).unwrap_or(0.0) } else { km };
                        scale(d, delta * s)
                    }))
                    .collect()
            }
            Some("thickness") => {
                let n_sel = rec.faces.first().map(|s| vec3(&s["normal"])).unwrap_or([0.0; 3]);
                // 对面移动方向 = −选中面法向(平面对面的外法向本来就是它; 圆柱对面的"中点法向"不是移动方向)
                let n_opp = scale(n_sel, -1.0);
                match self.dim_side {
                    1 => vec![[0.0; 3], scale(n_opp, delta)],
                    2 => vec![scale(n_sel, delta / 2.0), scale(n_opp, delta / 2.0)],
                    _ => vec![scale(n_sel, delta), [0.0; 3]],
                }
            }
            // 半径 / 圆角: 面中心的移动不好预测, 交给"最近匹配"
            _ => vec![[0.0; 3]; rec.faces.len() + rec.opposite.iter().count()],
        };
        // 应用时的选项写回记录(刷新时一起存)
        if let Some(r) = self.dims_list.iter_mut().find(|r| r.name == rec.name) {
            r.side = self.dim_side;
            r.sym = self.dim_sym;
            r.cyl = self.dim_cyl;
        }
        self.dim_last_apply = Some(DimApplied { name, moves, value: target });
    }

    /// N3d: 载入一条记录 —— 按签名在当前模型里重新定位, 回包后在 Plasticity 里按原顺序选中(参考面在前)。
    fn dim_load(&mut self, rec: &pla_ui::dims::DimRecord) {
        self.dim_record_call(rec, DimPending::Load(rec.clone()));
    }

    /// N3d: 一条记录 → dim.record(按签名定位 + 读当前尺寸, 一次往返), 回包按 pending 载入或直接应用。
    fn dim_record_call(&mut self, rec: &pla_ui::dims::DimRecord, pending: DimPending) {
        self.cdp.send(pla_bridge::cdp::CdpCmd::Call {
            name: pla_bridge::shim::api::DIM_RECORD,
            args: serde_json::json!({ "mode": rec.mode, "faces": rec.faces, "opposite": rec.opposite, "units": rec.units }),
        });
        self.dim_pending = Some(pending);
    }

    /// N3e: 记录对应的本体测量(两面面心之间, 名字 = 记录名; 同名已有则不重复建)。半径 / 圆角没有配对面 → 不建。
    fn dim_measure_for(&mut self, name: &str, info: &serde_json::Value) {
        let pair = match info["kind"].as_str() {
            // 对面是圆柱时面心吸附点不在"最近 / 最远处", 量出来不是厚度 → 不建
            Some("thickness") if info["oppositeOp"] != "moveFaces" => (info["face"].as_str(), info["opposite"].as_str()),
            Some("distance") => (info["reference"].as_str(), info["anchorFace"].as_str()),
            _ => (None, None),
        };
        if let (Some(a), Some(b)) = pair {
            self.cdp.send(pla_bridge::cdp::CdpCmd::Call {
                name: pla_bridge::shim::api::DIM_MEASURE,
                args: serde_json::json!({ "faces": [a, b], "name": name }),
            });
        }
    }

    /// N3d: 已记录尺寸列表。交互同快照墙: ★ 收藏(置顶)、✎ / 双击名字改名、↻ 用当前尺寸更新(确认)、管理 = 勾选批量删除;
    /// 点条目空白处 = 载入并在 Plasticity 里选中; 条目里的数值框直接改 + Enter / 右键 = 载入并应用;
    /// 悬停 = 三级面板显示详细信息与缩略图(条目本身不放截图)。cur = 当前编辑区读到的尺寸(更新用)。
    /// P5 参数表: 常量 / 变量 / 函数。行 = 类型徽标(点一下换类型)· 名字(双击改名, 引用它的表达式跟着改)· 值或表达式框
    /// (Enter / 右键 = 保存并按新值改模型)· 计算值(出错红色 ⚠, 悬停看原因)· ×。返回是否有输入框在编辑。
    fn params_ui(&mut self, ui: &mut egui::Ui) -> bool {
        enum PAct {
            Add(&'static str),
            Kind(String, &'static str),
            Expr(String, String, String),
            RenameStart(String),
            RenameCommit(String),
            Delete(String),
        }
        let mut acts: Vec<PAct> = Vec::new();
        let mut focus = false;
        let busy = self.dim_pending.is_some() || self.dim_busy || self.link_run.is_some();
        // 标题行: ▸/▾ 参数 (n) ······ ＋常量 ＋变量 ＋函数
        let (row, hdr) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::click());
        let cy = row.center().y;
        let tri = egui::pos2(row.left() + 5.0, cy);
        let pts = if self.params_open {
            vec![tri + egui::vec2(-4.0, -2.5), tri + egui::vec2(4.0, -2.5), tri + egui::vec2(0.0, 3.5)]
        } else {
            vec![tri + egui::vec2(-2.5, -4.0), tri + egui::vec2(-2.5, 4.0), tri + egui::vec2(3.5, 0.0)]
        };
        ui.painter().add(egui::Shape::convex_polygon(pts, COL_MUTED, egui::Stroke::NONE));
        let tr_ = paint_text(ui.painter(), egui::pos2(row.left() + 14.0, cy), egui::Align2::LEFT_CENTER, tr("参数", "PARAMETERS"), 12.0, COL_SECTION, true);
        ui.painter().text(egui::pos2(tr_.right() + 6.0, cy), egui::Align2::LEFT_CENTER, format!("{}", self.dims_params.len()), egui::FontId::proportional(11.0), COL_MUTED);
        let mut bx = row.right();
        for (kind, label, tip) in [
            ("func", tr("＋函数", "+Func"), tr("由其它名字算出来的表达式, 如 W/2 - T", "An expression of other names, e.g. W/2 - T")),
            ("var", tr("＋变量", "+Var"), tr("平时拨的数, 如宽度 W", "A number you turn, e.g. width W")),
            ("const", tr("＋常量", "+Const"), tr("固定的数, 如板厚 T(显示锁定)", "A fixed number, e.g. sheet thickness T (shown locked)")),
        ] {
            let font = egui::FontId::proportional(11.5);
            let w = ui.fonts(|f| f.layout_no_wrap(label.into(), font.clone(), COL_ACCENT).size().x);
            let r = egui::Rect::from_min_max(egui::pos2(bx - w, row.top()), egui::pos2(bx, row.bottom()));
            let b = ui.interact(r, ui.id().with(("param-add", kind)), egui::Sense::click());
            ui.painter().text(egui::pos2(bx, cy), egui::Align2::RIGHT_CENTER, label, font, if b.hovered() { egui::Color32::WHITE } else { COL_ACCENT });
            if b.on_hover_text(tip).clicked() {
                acts.push(PAct::Add(kind));
            }
            bx -= w + 10.0;
        }
        if hdr.clicked() {
            self.params_open = !self.params_open;
        }
        if self.params_open {
            if self.dims_params.is_empty() {
                ui.label(egui::RichText::new(tr("还没有参数。常量 / 变量是数, 函数是表达式; 尺寸记录点 ƒ 可绑定表达式, 改参数时一起改模型。", "No parameters yet. Constants / variables are numbers, functions are expressions; click ƒ on a saved dimension to bind it, and changing a parameter updates the model.")).size(11.0).color(COL_MUTED));
            }
            for p in self.dims_params.clone() {
                let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
                ui.painter().rect_filled(r, egui::Rounding::same(6.0), COL_CARD);
                let cy = r.center().y;
                // 类型徽标: 常 / 变 / ƒ(点一下 常 → 变 → ƒ → 常)
                let (badge, bcol) = match p.kind.as_str() {
                    "const" => (tr("常", "C"), egui::Color32::from_gray(200)),
                    "func" => ("ƒ", rgb_hex(pla_bridge::shim::HIGHLIGHT_CANDIDATE)),
                    _ => (tr("变", "V"), COL_ACCENT),
                };
                let br = egui::Rect::from_min_size(egui::pos2(r.left() + 4.0, cy - 9.0), egui::vec2(20.0, 18.0));
                let bresp = ui.interact(br, ui.id().with(("param-kind", &p.id)), egui::Sense::click());
                ui.painter().rect_stroke(br, egui::Rounding::same(4.0), egui::Stroke::new(1.0_f32, if bresp.hovered() { egui::Color32::WHITE } else { bcol }));
                ui.painter().text(br.center(), egui::Align2::CENTER_CENTER, badge, egui::FontId::proportional(11.5), bcol);
                let kind_tip = match p.kind.as_str() { "const" => tr("常量(点一下换类型)", "Constant (click to change type)"), "func" => tr("函数(点一下换类型)", "Function (click to change type)"), _ => tr("变量(点一下换类型)", "Variable (click to change type)") };
                if bresp.on_hover_text(kind_tip).clicked() {
                    let next = match p.kind.as_str() { "const" => "var", "var" => "func", _ => "const" };
                    acts.push(PAct::Kind(p.id.clone(), next));
                }
                // ×
                let xr = egui::Rect::from_center_size(egui::pos2(r.right() - 12.0, cy), egui::vec2(18.0, 18.0));
                let xb = ui.interact(xr, ui.id().with(("param-del", &p.id)), egui::Sense::click());
                ui.painter().text(xr.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(13.0), if xb.hovered() { egui::Color32::WHITE } else { COL_MUTED });
                if xb.on_hover_text(tr("删除(还有表达式引用它时不让删)", "Delete (refused while expressions use it)")).clicked() {
                    acts.push(PAct::Delete(p.id.clone()));
                }
                // 计算值 / 错误
                let val_r = egui::Rect::from_min_max(egui::pos2(r.right() - 92.0, cy - 10.0), egui::pos2(r.right() - 26.0, cy + 10.0));
                match self.dims_eval.values.get(&p.name) {
                    Some(Ok(v)) => {
                        ui.painter().text(egui::pos2(val_r.right(), cy), egui::Align2::RIGHT_CENTER, if p.kind == "func" { format!("= {}", fmt3(*v)) } else { String::new() }, egui::FontId::proportional(11.5), COL_MUTED);
                    }
                    Some(Err(e)) => {
                        ui.painter().text(egui::pos2(val_r.right(), cy), egui::Align2::RIGHT_CENTER, "⚠", egui::FontId::proportional(13.0), COL_RED);
                        ui.interact(val_r, ui.id().with(("param-err", &p.id)), egui::Sense::hover()).on_hover_text(e.clone());
                    }
                    None => {}
                }
                // 值 / 表达式框
                let fr = egui::Rect::from_min_max(egui::pos2(r.left() + 100.0, cy - 10.0), egui::pos2(val_r.left() - 6.0, cy + 10.0));
                ui.painter().rect_filled(fr, egui::Rounding::same(5.0), COL_FIELD);
                let key = format!("p:{}", p.id);
                let buf = self.dims_vals.entry(key.clone()).or_insert_with(|| p.expr.clone());
                let te = ui.put(
                    fr.shrink2(egui::vec2(4.0, 2.0)),
                    egui::TextEdit::singleline(buf).frame(false).font(egui::FontId::proportional(12.5))
                        .horizontal_align(if p.kind == "func" { egui::Align::LEFT } else { egui::Align::Center })
                        .text_color(if p.kind == "const" { egui::Color32::from_gray(200) } else { egui::Color32::WHITE }),
                );
                let focused = te.has_focus();
                if focused {
                    self.dims_edit_row = Some(key.clone());
                }
                let editing = focused || self.dims_edit_row.as_deref() == Some(key.as_str());
                focus |= editing;
                let enter = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let rclick = editing && right_click_confirm(ui);
                if (enter || rclick) && !busy {
                    self.dims_edit_row = None;
                    ui.memory_mut(|m| m.surrender_focus(te.id));
                    if buf.trim() != p.expr {
                        acts.push(PAct::Expr(p.id.clone(), p.name.clone(), buf.trim().to_string()));
                    }
                } else if !focused && editing && ui.input(|i| i.pointer.primary_pressed() || i.key_pressed(egui::Key::Escape)) {
                    self.dims_edit_row = None;
                    *buf = p.expr.clone();
                } else if !editing {
                    *buf = p.expr.clone();
                }
                // 名字(双击改名)
                let name_r = egui::Rect::from_min_max(egui::pos2(br.right() + 6.0, cy - 10.0), egui::pos2(fr.left() - 6.0, cy + 10.0));
                if self.param_rename.as_deref() == Some(p.id.as_str()) {
                    ui.painter().rect_filled(name_r, egui::Rounding::same(5.0), COL_FIELD);
                    let e = ui.put(name_r.shrink2(egui::vec2(4.0, 2.0)), egui::TextEdit::singleline(&mut self.param_rename_buf).frame(false).font(egui::FontId::proportional(12.5)));
                    e.request_focus();
                    focus = true;
                    let (en, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                    if en || right_click_confirm(ui) {
                        acts.push(PAct::RenameCommit(p.id.clone()));
                    } else if esc {
                        self.param_rename = None;
                    }
                } else {
                    let (shown, size, _) = fit_text(ui, &p.name, name_r.width().max(20.0), 12.5, 10.0);
                    paint_text(ui.painter(), egui::pos2(name_r.left(), cy), egui::Align2::LEFT_CENTER, &shown, size, egui::Color32::WHITE, true);
                    let used = pla_ui::dims::users_of(&pla_ui::dims::DimsFile { params: self.dims_params.clone(), dims: self.dims_list.clone(), ..Default::default() }, &p.name);
                    let tip = if used.is_empty() { tr("双击改名 · 还没有表达式用到它", "Double-click to rename · not used by any expression yet").to_string() } else { t!("双击改名 · 被 {} 引用", "Double-click to rename · used by {}", used.join(", ")) };
                    if ui.interact(name_r, ui.id().with(("param-name", &p.id)), egui::Sense::click()).on_hover_text(tip).double_clicked() {
                        acts.push(PAct::RenameStart(p.id.clone()));
                    }
                }
                ui.add_space(2.0);
            }
        }
        for a in acts {
            match a {
                PAct::Add(kind) => match pla_ui::dims::add_param(&self.doc_key, kind) {
                    Ok(name) => {
                        self.dims_reload();
                        self.params_open = true;
                        if let Some(p) = self.dims_params.iter().find(|p| p.name == name) {
                            self.param_rename_buf = name.clone();
                            self.param_rename = Some(p.id.clone());
                        }
                    }
                    Err(e) => self.set_notice((true, t!("新建参数失败: {e}", "New parameter failed: {e}"))),
                },
                PAct::Kind(id, k) => {
                    if let Err(e) = pla_ui::dims::update_param(&self.doc_key, &id, None, Some(k), None) {
                        self.set_notice((true, t!("改类型失败: {e}", "Changing the type failed: {e}")));
                    }
                    self.dims_reload();
                }
                PAct::Expr(id, name, e) => {
                    self.dims_vals.remove(&format!("p:{id}"));
                    match pla_ui::dims::update_param(&self.doc_key, &id, None, None, Some(&e)) {
                        Ok(()) => self.formula_run(&format!("{name} = {e}")),
                        Err(err) => self.set_notice((true, t!("保存参数失败: {err}", "Saving the parameter failed: {err}"))),
                    }
                }
                PAct::RenameStart(id) => {
                    self.param_rename_buf = self.dims_params.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
                    self.param_rename = Some(id);
                }
                PAct::RenameCommit(id) => {
                    self.param_rename = None;
                    let new = self.param_rename_buf.trim().to_string();
                    if let Err(e) = pla_ui::dims::update_param(&self.doc_key, &id, Some(&new), None, None) {
                        self.set_notice((true, t!("改名失败: {e}", "Rename failed: {e}")));
                    }
                    // 引用它的表达式已跟着改; 尺寸记录框里的表达式缓存清掉
                    self.dims_vals.clear();
                    self.dims_reload();
                }
                PAct::Delete(id) => {
                    if let Err(e) = pla_ui::dims::delete_param(&self.doc_key, &id) {
                        self.set_notice((true, t!("不能删: {e}", "Cannot delete: {e}")));
                    }
                    self.dims_reload();
                }
            }
        }
        ui.add_space(4.0);
        focus
    }

    fn dim_saved_ui(&mut self, ui: &mut egui::Ui, cur: Option<serde_json::Value>) {
        if !self.dims_loaded {
            self.dims_reload();
            self.dims_loaded = true;
        }
        ui.add_space(8.0);
        let pfocus = self.params_ui(ui);
        let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::hover());
        paint_text(ui.painter(), egui::pos2(row.left(), row.center().y), egui::Align2::LEFT_CENTER, tr("已记录", "SAVED"), 12.0, COL_SECTION, true);
        if !self.dims_list.is_empty() {
            let label = if self.dims_manage { tr("完成", "Done") } else { tr("管理", "Manage") };
            let font = egui::FontId::proportional(12.0);
            let w = ui.fonts(|f| f.layout_no_wrap(label.into(), font.clone(), COL_ACCENT).size().x);
            let r = egui::Rect::from_min_max(egui::pos2(row.right() - w - 4.0, row.top()), row.max);
            let resp = ui.interact(r, ui.id().with("dims-manage"), egui::Sense::click());
            ui.painter().text(egui::pos2(row.right(), row.center().y), egui::Align2::RIGHT_CENTER, label, font, if resp.hovered() { egui::Color32::WHITE } else { COL_ACCENT });
            if resp.clicked() {
                self.dims_manage = !self.dims_manage;
                self.dims_selected.clear();
            }
        }
        if self.dims_list.is_empty() {
            ui.label(egui::RichText::new(tr("还没有记录。在「操作」标签读出尺寸后, 点上方的 ＋ 保存当前尺寸。", "Nothing saved yet. Read a dimension in the Operate tab, then click + Save current dimension above.")).size(11.5).color(COL_MUTED));
            self.dims_row_focus = pfocus;
            return;
        }
        enum Act {
            Load(pla_ui::dims::DimRecord),
            Value(pla_ui::dims::DimRecord, f64),
            Fav(String, bool),
            RenameStart(String),
            RenameCommit(String),
            RenameCancel,
            Update(String),
            Delete(String),
            // P2 层级
            Move(Vec<String>, Option<String>),
            MoveNewGroup(Vec<String>),
            NewGroup,
            GroupCollapse(String, bool),
            GroupFav(String, bool),
            GroupRenameStart(String),
            GroupRenameCommit(String),
            GroupDelete(String),
            SolidToggle(String),
            // P5 绑定 / 解除 / 改表达式(None = 解除)
            ExprSet(String, Option<String>),
        }
        let mut acts: Vec<Act> = Vec::new();
        // ---- 工具行: 搜索 · 筛选(全部 / 尺寸 / ★) · 按实体 · ＋组 ----
        {
            let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
            let sw = (row.width() * 0.42).max(110.0);
            let sr = egui::Rect::from_min_size(row.min, egui::vec2(sw, row.height()));
            ui.painter().rect_filled(sr, egui::Rounding::same(5.0), COL_FIELD);
            ui.put(
                sr.shrink2(egui::vec2(6.0, 3.0)),
                egui::TextEdit::singleline(&mut self.rec_search).frame(false).font(egui::FontId::proportional(12.0)).hint_text(tr("搜索名字…", "Search names…")),
            );
            let mut x = sr.right() + 6.0;
            let chip = |ui: &mut egui::Ui, x: &mut f32, label: &str, on: bool, key: &str| -> bool {
                let font = egui::FontId::proportional(11.5);
                let w = ui.fonts(|f| f.layout_no_wrap(label.into(), font.clone(), COL_TEXT).size().x) + 12.0;
                let r = egui::Rect::from_min_size(egui::pos2(*x, row.top() + 2.0), egui::vec2(w, row.height() - 4.0));
                let b = ui.interact(r, ui.id().with(("rec-chip", key)), egui::Sense::click());
                let fill = if on { COL_STEPPER } else if b.hovered() { lerp_color(COL_CARD, egui::Color32::WHITE, 0.08) } else { COL_CARD };
                ui.painter().rect_filled(r, egui::Rounding::same(9.0), fill);
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, font, if on { egui::Color32::WHITE } else { COL_MUTED });
                *x = r.right() + 4.0;
                b.clicked()
            };
            for (i, label) in [tr("全部", "All"), tr("尺寸", "Dims"), tr("孔", "Holes"), "★"].iter().enumerate() {
                if chip(ui, &mut x, label, self.rec_filter as usize == i, label) {
                    self.rec_filter = i as u8;
                }
            }
            if chip(ui, &mut x, tr("按实体", "By solid"), self.rec_by_solid, "by-solid") {
                self.rec_by_solid = !self.rec_by_solid;
            }
            // ＋组: 靠右
            let label = tr("＋组", "+Group");
            let font = egui::FontId::proportional(12.0);
            let w = ui.fonts(|f| f.layout_no_wrap(label.into(), font.clone(), COL_ACCENT).size().x);
            let gr = egui::Rect::from_min_max(egui::pos2(row.right() - w - 4.0, row.top()), row.max);
            let g = ui.interact(gr, ui.id().with("rec-new-group"), egui::Sense::click());
            ui.painter().text(egui::pos2(row.right(), row.center().y), egui::Align2::RIGHT_CENTER, label, font, if g.hovered() { egui::Color32::WHITE } else { COL_ACCENT });
            if g.on_hover_text(tr("新建一个组(文件夹); 条目右侧 ⋯ 可移入", "New group (folder); move entries in with ⋯ on the right")).clicked() {
                acts.push(Act::NewGroup);
            }
            ui.add_space(4.0);
        }
        // ---- 行: 组 / 尺寸(带缩进层级) ----
        enum Row {
            Group { id: String, name: String, fav: bool, collapsed: bool, count: usize, virt: bool, hole: bool, summary: String },
            Dim(pla_ui::dims::DimRecord, u8),
        }
        let q = self.rec_search.trim().to_lowercase();
        let keep = |d: &pla_ui::dims::DimRecord| -> bool { (self.rec_filter != 3 || d.fav) && (q.is_empty() || d.name.to_lowercase().contains(&q)) };
        let dims_sorted = pla_ui::dims::sorted(self.dims_list.clone());
        let mut rows: Vec<Row> = Vec::new();
        if !q.is_empty() || self.rec_filter == 3 {
            // 搜索 / 只看收藏: 平铺结果
            rows.extend(dims_sorted.iter().filter(|d| keep(d)).map(|d| Row::Dim(d.clone(), 0)));
        } else if self.rec_by_solid {
            // 按实体分组(纯视图, 不改存储)
            let mut solids: Vec<String> = Vec::new();
            for d in &dims_sorted {
                let s = d.faces.first().and_then(|f| f["solidName"].as_str()).unwrap_or("?").to_string();
                if !solids.contains(&s) {
                    solids.push(s);
                }
            }
            solids.sort();
            for s in solids {
                let kids: Vec<&pla_ui::dims::DimRecord> = dims_sorted.iter().filter(|d| d.faces.first().and_then(|f| f["solidName"].as_str()).unwrap_or("?") == s).collect();
                let collapsed = self.rec_solid_closed.contains(&s);
                rows.push(Row::Group { id: s.clone(), name: s.clone(), fav: false, collapsed, count: kids.len(), virt: true, hole: false, summary: String::new() });
                if !collapsed {
                    rows.extend(kids.into_iter().map(|d| Row::Dim(d.clone(), 1)));
                }
            }
        } else {
            // 筛选: 尺寸 = 不看孔特征; 孔 = 只看孔特征
            let mut groups: Vec<pla_ui::dims::Group> = self.dims_groups.iter().filter(|g| match self.rec_filter { 1 => g.kind != "hole", 2 => g.kind == "hole", _ => true }).cloned().collect();
            groups.sort_by(|a, b| b.fav.cmp(&a.fav).then_with(|| a.name.cmp(&b.name)));
            for g in &groups {
                let kids: Vec<&pla_ui::dims::DimRecord> = dims_sorted.iter().filter(|d| d.parent.as_deref() == Some(g.id.as_str())).collect();
                rows.push(Row::Group { id: g.id.clone(), name: g.name.clone(), fav: g.fav, collapsed: g.collapsed, count: kids.len(), virt: false, hole: g.kind == "hole", summary: hole_summary(g.spec.as_ref()) });
                if !g.collapsed {
                    rows.extend(kids.into_iter().map(|d| Row::Dim(d.clone(), 1)));
                }
            }
            if self.rec_filter != 2 {
                rows.extend(dims_sorted.iter().filter(|d| d.parent.is_none()).map(|d| Row::Dim(d.clone(), 0)));
            }
        }
        if self.dims_manage {
            let names: Vec<String> = dims_sorted.iter().map(|r| r.name.clone()).collect();
            let (del, done) = manage_bar(ui, &names, &mut self.dims_selected, tr("删除勾选的尺寸记录(不影响模型)", "Delete the checked records (the model is not affected)"));
            if del {
                self.dims_batch_confirm = true;
            }
            if done {
                self.dims_manage = false;
                self.dims_selected.clear();
            }
            // 勾选的一起移到组
            if !self.dims_selected.is_empty() {
                let b = ui.add(egui::Button::new(egui::RichText::new(t!("移到组… ({})", "Move to group… ({})", self.dims_selected.len())).size(12.0)));
                let sel: Vec<String> = self.dims_selected.iter().cloned().collect();
                // current = Some("") → 列出全部组, 也给"移出组"
                match move_menu(ui, &b, "dims-batch-move", &self.dims_groups, Some("")) {
                    Some(MoveChoice::To(p)) => acts.push(Act::Move(sel, p)),
                    Some(MoveChoice::NewGroup) => acts.push(Act::MoveNewGroup(sel)),
                    None => {}
                }
            }
        }
        let mut focus_any = false;
        let busy = self.dim_pending.is_some() || self.dim_busy || self.link_run.is_some();
        for row in &rows {
            let (rec, depth) = match row {
                Row::Dim(d, depth) => (d, *depth),
                Row::Group { id, name, fav, collapsed, count, virt, hole, summary } => {
                    // 组行: ▸/▾ 折叠 · 名字(双击改名) · 条数 · ★ · ×
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
                    let hot = ui.rect_contains_pointer(r);
                    ui.painter().rect_filled(r, egui::Rounding::same(6.0), if hot { lerp_color(COL_FIELD, egui::Color32::WHITE, 0.05) } else { COL_FIELD });
                    let cy = r.center().y;
                    let tri = egui::pos2(r.left() + 12.0, cy);
                    let pts = if *collapsed {
                        vec![tri + egui::vec2(-2.5, -4.0), tri + egui::vec2(-2.5, 4.0), tri + egui::vec2(3.5, 0.0)]
                    } else {
                        vec![tri + egui::vec2(-4.0, -2.5), tri + egui::vec2(4.0, -2.5), tri + egui::vec2(0.0, 3.5)]
                    };
                    ui.painter().add(egui::Shape::convex_polygon(pts, COL_MUTED, egui::Stroke::NONE));
                    // 孔特征: 名字前画同心圆标记(与对面候选里的圆柱标记一致)
                    let name_left = if *hole {
                        let c = egui::pos2(r.left() + 30.0, cy);
                        ui.painter().circle_stroke(c, 5.0, egui::Stroke::new(1.5_f32, COL_ACCENT));
                        ui.painter().circle_filled(c, 2.0, COL_ACCENT);
                        r.left() + 40.0
                    } else {
                        r.left() + 24.0
                    };
                    let name_right = r.right() - 64.0;
                    let name_r = egui::Rect::from_min_max(egui::pos2(name_left, cy - 10.0), egui::pos2(name_right, cy + 10.0));
                    if !virt && self.grp_rename.as_deref() == Some(id.as_str()) {
                        ui.painter().rect_filled(name_r, egui::Rounding::same(5.0), COL_STEPPER);
                        let e = ui.put(name_r.shrink2(egui::vec2(4.0, 2.0)), egui::TextEdit::singleline(&mut self.grp_rename_buf).frame(false).font(egui::FontId::proportional(12.5)));
                        e.request_focus();
                        focus_any = true;
                        let (en, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                        if en || right_click_confirm(ui) {
                            acts.push(Act::GroupRenameCommit(id.clone()));
                        } else if esc {
                            self.grp_rename = None;
                        }
                    } else {
                        let label = if *virt { t!("{name} · 实体", "{name} · solid") } else if *hole { format!("{name} · {summary}") } else { name.clone() };
                        let (shown, size, _) = fit_text(ui, &label, name_r.width() - 40.0, 12.5, 10.5);
                        let tr_ = paint_text(ui.painter(), egui::pos2(name_r.left(), cy), egui::Align2::LEFT_CENTER, &shown, size, egui::Color32::WHITE, true);
                        ui.painter().text(egui::pos2(tr_.right() + 6.0, cy), egui::Align2::LEFT_CENTER, format!("{count}"), egui::FontId::proportional(11.0), COL_MUTED);
                        if !virt && ui.interact(name_r, ui.id().with(("grp-name", id)), egui::Sense::click()).double_clicked() {
                            acts.push(Act::GroupRenameStart(id.clone()));
                        }
                    }
                    if !virt {
                        let small = |ui: &mut egui::Ui, cx: f32, glyph: &str, col: egui::Color32, tip: &str| -> bool {
                            let br = egui::Rect::from_center_size(egui::pos2(cx, cy), egui::vec2(18.0, 18.0));
                            let b = ui.interact(br, ui.id().with(("grp-btn", id, glyph)), egui::Sense::click());
                            ui.painter().text(br.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(13.0), if b.hovered() { egui::Color32::WHITE } else { col });
                            b.on_hover_text(tip).clicked()
                        };
                        if small(ui, r.right() - 12.0, "×", COL_MUTED, tr("删除组(组内尺寸回到顶层, 不影响模型; 需确认)", "Delete group (its entries go to the top level; model unaffected; confirm)")) {
                            acts.push(Act::GroupDelete(id.clone()));
                        }
                        let star_col = if *fav { egui::Color32::from_rgb(255, 200, 60) } else { COL_MUTED };
                        if small(ui, r.right() - 32.0, if *fav { "★" } else { "☆" }, star_col, tr("收藏(置顶)", "Favourite (pinned)")) {
                            acts.push(Act::GroupFav(id.clone(), !fav));
                        }
                    }
                    if resp.clicked() {
                        acts.push(if *virt { Act::SolidToggle(id.clone()) } else { Act::GroupCollapse(id.clone(), !collapsed) });
                    }
                    ui.add_space(2.0);
                    continue;
                }
            };
            let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::click());
            let active = self.dim_active.as_deref() == Some(rec.name.as_str());
            let hot = ui.rect_contains_pointer(r);
            let fill = if active { COL_STEPPER } else if hot { lerp_color(COL_CARD, egui::Color32::WHITE, 0.06) } else { COL_CARD };
            // 子项缩进: 卡片左边留出层级缩进
            let r = egui::Rect::from_min_max(egui::pos2(r.left() + depth as f32 * 14.0, r.top()), r.max);
            ui.painter().rect_filled(r, egui::Rounding::same(6.0), fill);
            let cy = r.center().y;
            let mut x = r.left() + 6.0;
            // 管理勾选
            if self.dims_manage {
                let mut on = self.dims_selected.contains(&rec.name);
                let cb = egui::Rect::from_min_size(egui::pos2(x, cy - 9.0), egui::vec2(18.0, 18.0));
                if ui.put(cb, egui::Checkbox::without_text(&mut on)).changed() {
                    if on { self.dims_selected.insert(rec.name.clone()); } else { self.dims_selected.remove(&rec.name); }
                }
                x += 20.0;
            }
            // ★ 收藏
            let sr = egui::Rect::from_min_size(egui::pos2(x, cy - 9.0), egui::vec2(18.0, 18.0));
            let s = ui.interact(sr, ui.id().with(("dims-fav", &rec.name)), egui::Sense::click());
            ui.painter().text(sr.center(), egui::Align2::CENTER_CENTER, if rec.fav { "★" } else { "☆" }, egui::FontId::proportional(13.0), if rec.fav { egui::Color32::from_rgb(255, 200, 60) } else { COL_MUTED });
            if s.on_hover_text(tr("收藏(置顶)", "Favourite (pinned)")).clicked() {
                acts.push(Act::Fav(rec.name.clone(), !rec.fav));
            }
            x += 20.0;
            // P5 ƒ: 绑定表达式(橙 = 由表达式驱动; 灰 = 没绑定, 点一下绑定, 先填当前值)
            let bound = rec.expr.is_some();
            {
                let tr_ = egui::Rect::from_min_size(egui::pos2(x, cy - 9.0), egui::vec2(16.0, 18.0));
                let t = ui.interact(tr_, ui.id().with(("dims-expr", &rec.name)), egui::Sense::click());
                let col = if t.hovered() { egui::Color32::WHITE } else if bound { COL_ACCENT } else { COL_MUTED };
                ui.painter().text(tr_.center(), egui::Align2::CENTER_CENTER, "ƒ", egui::FontId::proportional(13.5), col);
                let tip = if bound {
                    t!("由表达式驱动: = {}(点一下解除绑定)", "Driven by an expression: = {} (click to unbind)", rec.expr.as_deref().unwrap_or(""))
                } else {
                    tr("绑定表达式: 数值框改成填表达式(可引用参数和其它尺寸名, 如 W/2 - 3)", "Bind an expression: the field then takes an expression (parameters / other dimension names, e.g. W/2 - 3)").to_string()
                };
                if t.on_hover_text(tip).clicked() {
                    acts.push(Act::ExprSet(rec.name.clone(), if bound { None } else { Some(fmt3(rec.value)) }));
                }
                x += 16.0;
            }
            // 右侧从右往左: × ↻ ✎ 数值框
            let btn = |ui: &mut egui::Ui, cx: f32, glyph: &str, tip: &str, key: &str| -> bool {
                let br = egui::Rect::from_center_size(egui::pos2(cx, cy), egui::vec2(18.0, 18.0));
                let b = ui.interact(br, ui.id().with((key, glyph)), egui::Sense::click());
                let col = if b.hovered() { egui::Color32::WHITE } else { COL_MUTED };
                if glyph == "✎" {
                    // 界面字体里没有 ✎(显示成方块, 用户 2026-11-15) → 画一支笔
                    paint_pencil(ui.painter(), br.center(), col);
                } else {
                    ui.painter().text(br.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(13.0), col);
                }
                b.on_hover_text(tip).clicked()
            };
            let key = rec.name.as_str();
            if btn(ui, r.right() - 12.0, "×", tr("删除(需确认)", "Delete (confirm)"), key) {
                acts.push(Act::Delete(rec.name.clone()));
            }
            if btn(ui, r.right() - 32.0, "↻", tr("用当前读到的尺寸更新这条(保留名字与收藏; 需确认)", "Update from the current dimension (keeps name and favourite; confirm)"), key) {
                acts.push(Act::Update(rec.name.clone()));
            }
            if btn(ui, r.right() - 52.0, "✎", tr("改名(也可双击名字)", "Rename (or double-click the name)"), key) {
                acts.push(Act::RenameStart(rec.name.clone()));
            }
            // ⋯ 移到组(弹出菜单: 各组 / 新建组并移入 / 移出组)
            {
                let br = egui::Rect::from_center_size(egui::pos2(r.right() - 72.0, cy), egui::vec2(18.0, 18.0));
                let b = ui.interact(br, ui.id().with(("dims-move-btn", key)), egui::Sense::click());
                let col = if b.hovered() { egui::Color32::WHITE } else { COL_MUTED };
                for dx in [-4.0f32, 0.0, 4.0] {
                    ui.painter().circle_filled(br.center() + egui::vec2(dx, 0.0), 1.4, col);
                }
                let b = b.on_hover_text(tr("移到组…", "Move to group…"));
                match move_menu(ui, &b, &format!("dims-move-{key}"), &self.dims_groups, rec.parent.as_deref()) {
                    Some(MoveChoice::To(p)) => acts.push(Act::Move(vec![rec.name.clone()], p)),
                    Some(MoveChoice::NewGroup) => acts.push(Act::MoveNewGroup(vec![rec.name.clone()])),
                    None => {}
                }
            }
            let u = match rec.units.as_str() { "millimeter" => "mm", "centimeter" => "cm", "meter" => "m", "inch" => "in", "foot" => "ft", x => x };
            let unit_w = 22.0;
            let fr = egui::Rect::from_min_max(egui::pos2(r.right() - 84.0 - 66.0 - unit_w, cy - 11.0), egui::pos2(r.right() - 84.0 - unit_w, cy + 11.0));
            ui.painter().rect_filled(fr, egui::Rounding::same(5.0), COL_FIELD);
            // P5: 绑定了表达式 → 框里是表达式(橙色), 改它 = 改表达式并按新值改模型; 没绑定 → 框里是数值
            let shown_txt = rec.expr.clone().unwrap_or_else(|| fmt3(rec.value));
            let buf = self.dims_vals.entry(rec.name.clone()).or_insert_with(|| shown_txt.clone());
            let te = ui.put(
                fr.shrink2(egui::vec2(4.0, 2.0)),
                egui::TextEdit::singleline(buf).frame(false).horizontal_align(egui::Align::Center).font(egui::FontId::proportional(12.5)).text_color(if bound { COL_ACCENT } else { egui::Color32::WHITE }),
            );
            ui.painter().text(egui::pos2(fr.right() + 4.0, cy), egui::Align2::LEFT_CENTER, u, egui::FontId::proportional(11.0), COL_MUTED);
            let focused = te.has_focus();
            // 右键确认: 按下右键那一帧 egui 就把文本框焦点收走了, 到"右键单击"(松开)那一帧已不在编辑 →
            // 记住"正在编辑的条目"(dims_edit_row), 直到左键点别处 / Esc / 应用(用户 2026-11-15: 右键确认没生效)
            if focused {
                self.dims_edit_row = Some(rec.name.clone());
            }
            let editing = focused || self.dims_edit_row.as_deref() == Some(rec.name.as_str());
            focus_any |= editing;
            let enter = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let rclick = editing && right_click_confirm(ui);
            if (enter || rclick) && !busy {
                // 确认 = 应用并强制退出编辑(右键点在框上时焦点不会自己丢, 不退出的话框里不会刷新成新值 —— 用户 2026-11-15)
                self.dims_edit_row = None;
                ui.memory_mut(|m| m.surrender_focus(te.id));
                if bound {
                    if buf.trim() != shown_txt {
                        acts.push(Act::ExprSet(rec.name.clone(), Some(buf.trim().to_string())));
                    }
                } else {
                    match buf.trim().parse::<f64>() {
                        Ok(t) if t >= 0.0 && (t - rec.value).abs() > 5e-4 => acts.push(Act::Value(rec.clone(), t)),
                        _ => *buf = shown_txt.clone(),
                    }
                }
            } else if !focused && editing && ui.input(|i| i.pointer.primary_pressed() || i.key_pressed(egui::Key::Escape)) {
                // 左键点了别处 / Esc = 放弃这次输入
                self.dims_edit_row = None;
                *buf = shown_txt.clone();
            } else if !editing {
                // 没在编辑: 永远显示记录里的值 / 表达式(不缓存旧输入 —— 否则改完后记录已更新, 框里还停在旧数, 用户 2026-11-15 图1)
                *buf = shown_txt.clone();
            }
            // 名字 / 改名
            let name_r = egui::Rect::from_min_max(egui::pos2(x, cy - 11.0), egui::pos2(fr.left() - 50.0, cy + 11.0));
            if self.dims_rename.as_deref() == Some(rec.name.as_str()) {
                ui.painter().rect_filled(name_r, egui::Rounding::same(5.0), COL_FIELD);
                let e = ui.put(name_r.shrink2(egui::vec2(4.0, 2.0)), egui::TextEdit::singleline(&mut self.dims_rename_buf).frame(false).font(egui::FontId::proportional(12.5)));
                e.request_focus();
                focus_any = true;
                let (en, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                if en || right_click_confirm(ui) {
                    acts.push(Act::RenameCommit(rec.name.clone()));
                } else if esc {
                    acts.push(Act::RenameCancel);
                }
            } else {
                let (shown, size, _) = fit_text(ui, &rec.name, name_r.width().max(20.0), 12.5, 10.5);
                paint_text(ui.painter(), egui::pos2(name_r.left(), cy), egui::Align2::LEFT_CENTER, &shown, size, if active { COL_ACCENT } else { egui::Color32::WHITE }, true);
                let nr = ui.interact(name_r, ui.id().with(("dims-name", &rec.name)), egui::Sense::click());
                if nr.double_clicked() {
                    acts.push(Act::RenameStart(rec.name.clone()));
                } else if nr.clicked() {
                    if self.dims_manage {
                        if !self.dims_selected.remove(&rec.name) { self.dims_selected.insert(rec.name.clone()); }
                    } else if !busy {
                        acts.push(Act::Load(rec.clone()));
                    }
                }
            }
            let kind = match rec.kind.as_str() { "thickness" => tr("厚度", "Thick."), "radius" => tr("半径", "Radius"), "filletRadius" => tr("圆角", "Fillet"), "distance" => tr("距离", "Dist."), _ => "" };
            if bound {
                // 绑定: 框左边显示当前值(与计算值不同 = 待应用, 橙色; 表达式出错 = 红色 ⚠, 悬停看原因)
                let (txt, col, tip) = match self.dims_eval.targets.get(&rec.name) {
                    Some(Ok(t)) if (t - rec.value).abs() > 5e-4 => (format!("{} → {}", fmt3(rec.value), fmt3(*t)), COL_ACCENT, tr("与计算值不同: 改任一参数 / 表达式时一起应用", "Differs from the computed value: applied with the next parameter / expression change").to_string()),
                    Some(Ok(_)) => (fmt3(rec.value), COL_MUTED, String::new()),
                    Some(Err(e)) => ("⚠".to_string(), COL_RED, e.clone()),
                    None => (fmt3(rec.value), COL_MUTED, String::new()),
                };
                let lr = paint_text(ui.painter(), egui::pos2(fr.left() - 8.0, cy), egui::Align2::RIGHT_CENTER, &txt, 11.5, col, false);
                if !tip.is_empty() {
                    ui.interact(lr, ui.id().with(("dims-expr-val", &rec.name)), egui::Sense::hover()).on_hover_text(tip);
                }
            } else {
                ui.painter().text(egui::pos2(fr.left() - 8.0, cy), egui::Align2::RIGHT_CENTER, kind, egui::FontId::proportional(11.5), COL_MUTED);
            }
            // 条目空白处单击 = 载入(管理模式 = 勾选)
            if resp.clicked() {
                if self.dims_manage {
                    if !self.dims_selected.remove(&rec.name) { self.dims_selected.insert(rec.name.clone()); }
                } else if !busy {
                    acts.push(Act::Load(rec.clone()));
                }
            }
            // 悬停详情(三级面板, 含缩略图)
            if hot {
                let mut lines = vec![info_text(format!("{} · {} {} {u}", rec.name, kind, fmt3(rec.value)))];
                let face0 = rec.faces.first();
                let solid = face0.and_then(|f| f["solidName"].as_str()).unwrap_or("?");
                let surf = face0.and_then(|f| f["surface"].as_str()).unwrap_or("?");
                lines.push(info_text(if rec.mode == "distance" {
                    t!("参考面: {solid} 的 {surf} 面 → 动 {} 个面", "Reference: {surf} face of {solid} → {} faces move", rec.faces.len().saturating_sub(1))
                } else {
                    let side = match rec.side { 1 => tr("移动对面", "move opposite"), 2 => tr("两侧各半", "both halves"), _ => tr("移动选中面", "move selected") };
                    t!("{solid} 的 {surf} 面 · {side}", "{surf} face of {solid} · {side}")
                }));
                if !self.dims_pngs.contains_key(&rec.name) {
                    if let Ok(b) = std::fs::read(pla_ui::dims::png_path(&self.doc_key, &rec.name)) {
                        self.dims_pngs.insert(rec.name.clone(), b);
                    }
                }
                if let Some(b) = self.dims_pngs.get(&rec.name) {
                    lines.push(InfoLine::Image(png_uri("dims", &rec.name, b), b.clone()));
                } else {
                    lines.push(info_text(tr("(没有截图: 记录 / 更新时会截一张)", "(no thumbnail: taken on save / update)")));
                }
                lines.push(info_text(tr("单击 = 载入并选中 · 改数值 + Enter / 右键 = 应用 · 双击名字 = 改名", "Click = load & select · edit value + Enter / right-click = apply · double-click name = rename")));
                hover_info(ui, r, true, tr("尺寸记录", "Saved dimension"), lines);
            }
            ui.add_space(2.0);
        }
        self.dims_row_focus = focus_any || pfocus;
        for a in acts {
            match a {
                Act::Load(rec) => self.dim_load(&rec),
                Act::Value(rec, t) => {
                    self.dims_vals.remove(&rec.name);
                    self.dim_record_call(&rec, DimPending::Apply(rec.clone(), t));
                }
                Act::Fav(name, fav) => {
                    if let Err(e) = pla_ui::dims::set_fav(&self.doc_key, &name, fav) {
                        self.set_notice((true, t!("收藏失败: {e}", "Favourite failed: {e}")));
                    }
                    self.dims_reload();
                }
                Act::RenameStart(name) => {
                    self.dims_rename_buf = name.clone();
                    self.dims_rename = Some(name);
                }
                Act::RenameCancel => self.dims_rename = None,
                Act::RenameCommit(old) => {
                    let new = self.dims_rename_buf.trim().to_string();
                    self.dims_rename = None;
                    if new != old {
                        match pla_ui::dims::rename(&self.doc_key, &old, &new) {
                            Ok(()) => {
                                if let Some(b) = self.dims_pngs.remove(&old) {
                                    self.dims_pngs.insert(new.clone(), b);
                                }
                                self.dims_vals.remove(&old);
                                if self.dim_active.as_deref() == Some(old.as_str()) {
                                    self.dim_active = Some(new.clone());
                                }
                                // N3e: 同名本体测量跟着改名(没有就什么也不做)
                                self.cdp.send(pla_bridge::cdp::CdpCmd::Call {
                                    name: pla_bridge::shim::api::DIM_MEASURE,
                                    args: serde_json::json!({ "rename": { "from": old, "to": new } }),
                                });
                                self.dims_reload();
                            }
                            Err(e) => self.set_notice((true, t!("改名失败: {e}", "Rename failed: {e}"))),
                        }
                    }
                }
                Act::Update(name) => self.dims_update_confirm = Some(name),
                Act::Delete(name) => self.dim_del_confirm = Some(name),
                // ---- P2 层级 ----
                Act::Move(names, parent) => {
                    if let Err(e) = pla_ui::dims::set_parent(&self.doc_key, &names, parent.as_deref()) {
                        self.set_notice((true, t!("移动失败: {e}", "Move failed: {e}")));
                    }
                    self.dims_reload();
                }
                Act::MoveNewGroup(names) => {
                    match pla_ui::dims::add_group(&self.doc_key).and_then(|g| pla_ui::dims::set_parent(&self.doc_key, &names, Some(&g)).map(|_| g)) {
                        Ok(g) => {
                            self.dims_reload();
                            // 新组直接进入改名
                            self.grp_rename_buf = self.dims_groups.iter().find(|x| x.id == g).map(|x| x.name.clone()).unwrap_or_default();
                            self.grp_rename = Some(g);
                        }
                        Err(e) => self.set_notice((true, t!("新建组失败: {e}", "New group failed: {e}"))),
                    }
                }
                Act::NewGroup => match pla_ui::dims::add_group(&self.doc_key) {
                    Ok(g) => {
                        self.dims_reload();
                        self.grp_rename_buf = self.dims_groups.iter().find(|x| x.id == g).map(|x| x.name.clone()).unwrap_or_default();
                        self.grp_rename = Some(g);
                    }
                    Err(e) => self.set_notice((true, t!("新建组失败: {e}", "New group failed: {e}"))),
                },
                Act::GroupCollapse(id, c) => {
                    let _ = pla_ui::dims::set_group_flags(&self.doc_key, &id, None, Some(c));
                    self.dims_reload();
                }
                Act::GroupFav(id, f) => {
                    let _ = pla_ui::dims::set_group_flags(&self.doc_key, &id, Some(f), None);
                    self.dims_reload();
                }
                Act::GroupRenameStart(id) => {
                    self.grp_rename_buf = self.dims_groups.iter().find(|x| x.id == id).map(|x| x.name.clone()).unwrap_or_default();
                    self.grp_rename = Some(id);
                }
                Act::GroupRenameCommit(id) => {
                    self.grp_rename = None;
                    if let Err(e) = pla_ui::dims::rename_group(&self.doc_key, &id, &self.grp_rename_buf) {
                        self.set_notice((true, t!("组改名失败: {e}", "Group rename failed: {e}")));
                    }
                    self.dims_reload();
                }
                Act::GroupDelete(id) => self.grp_del_confirm = Some(id),
                Act::ExprSet(name, expr) => {
                    self.dims_vals.remove(&name);
                    match pla_ui::dims::set_dim_expr(&self.doc_key, &name, expr.as_deref()) {
                        Ok(()) => match &expr {
                            Some(e) => self.formula_run(&format!("{name} = {e}")),
                            None => self.dims_reload(),
                        },
                        Err(e) => self.set_notice((true, t!("设置表达式失败: {e}", "Setting the expression failed: {e}"))),
                    }
                }
                Act::SolidToggle(s) => {
                    if !self.rec_solid_closed.remove(&s) {
                        self.rec_solid_closed.insert(s);
                    }
                }
            }
        }
        // P3: 刚记录的手动尺寸落在某个孔的面上 → 问要不要挂到孔下
        if let Some((dn, hid, hn)) = self.attach_prompt.clone() {
            match confirm_popup_with(ui.ctx(), "dims-attach", &t!("{dn} 量的是孔 {hn} 的面, 挂到 {hn} 下?", "{dn} measures a face of hole {hn}. Put it under {hn}?"), tr("挂上", "Attach"), COL_ACCENT) {
                Some(true) => {
                    self.attach_prompt = None;
                    if let Err(e) = pla_ui::dims::set_parent(&self.doc_key, &[dn], Some(&hid)) {
                        self.set_notice((true, t!("挂入失败: {e}", "Attach failed: {e}")));
                    }
                    self.dims_reload();
                }
                Some(false) => self.attach_prompt = None,
                None => {}
            }
        }
        if let Some(id) = self.grp_del_confirm.clone() {
            let g = self.dims_groups.iter().find(|g| g.id == id).cloned();
            let name = g.as_ref().map(|g| g.name.clone()).unwrap_or_default();
            let text = if g.as_ref().is_some_and(|g| g.kind == "hole") {
                t!("删除孔记录 {name}?(它自动生成的子尺寸一起删, 手动挂入的回到顶层; 模型里的孔不受影响)", "Delete hole record {name}? (its generated dimensions go too, attached ones return to the top level; the hole in the model stays)")
            } else {
                t!("删除组 {name}?(组内尺寸回到顶层, 不影响模型)", "Delete group {name}? (its entries go to the top level; the model is not affected)")
            };
            match confirm_popup(ui.ctx(), "grp-del", &text) {
                Some(true) => {
                    self.grp_del_confirm = None;
                    if let Err(e) = pla_ui::dims::delete_group(&self.doc_key, &id) {
                        self.set_notice((true, t!("删除组失败: {e}", "Delete group failed: {e}")));
                    }
                    self.dims_reload();
                }
                Some(false) => self.grp_del_confirm = None,
                None => {}
            }
        }
        // ---- 确认弹层(右键 = 确认, Esc = 取消) ----
        if let Some(name) = self.dims_update_confirm.clone() {
            let ok_cur = cur.as_ref().filter(|v| matches!(v["kind"].as_str(), Some("thickness" | "radius" | "filletRadius" | "distance")));
            if ok_cur.is_none() {
                self.dims_update_confirm = None;
                self.set_notice((true, tr("先在 Plasticity 里选面读出一个尺寸, 再用它更新记录", "Select faces in Plasticity to read a dimension first, then update the record with it").into()));
            } else {
                match confirm_popup_with(ui.ctx(), "dims-update", &t!("用当前读到的尺寸覆盖 {name}?(名字与收藏保留)", "Overwrite {name} with the current dimension? (name and favourite kept)"), tr("更新", "Update"), COL_ACCENT) {
                    Some(true) => {
                        self.dims_update_confirm = None;
                        let v = ok_cur.cloned().unwrap_or_default();
                        self.dim_request_save(&v, Some(name));
                    }
                    Some(false) => self.dims_update_confirm = None,
                    None => {}
                }
            }
        }
        if self.dims_batch_confirm {
            let names: Vec<String> = self.dims_selected.iter().cloned().collect();
            match confirm_popup(ui.ctx(), "dims-batch", &t!("删除勾选的 {} 条尺寸记录?(不影响模型)", "Delete {} checked records? (the model is not affected)", names.len())) {
                Some(true) => {
                    self.dims_batch_confirm = false;
                    match pla_ui::dims::delete_many(&self.doc_key, &names) {
                        Ok(_) => {
                            if self.dim_active.as_ref().is_some_and(|a| names.contains(a)) {
                                self.dim_active = None;
                            }
                            for n in &names {
                                self.dims_pngs.remove(n);
                                self.dims_vals.remove(n);
                            }
                            self.dims_selected.clear();
                            self.dims_manage = false;
                            self.dims_reload();
                        }
                        Err(e) => self.set_notice((true, t!("删除失败: {e}", "Delete failed: {e}"))),
                    }
                }
                Some(false) => self.dims_batch_confirm = false,
                None => {}
            }
        }
        if let Some(name) = self.dim_del_confirm.clone() {
            match confirm_popup(ui.ctx(), "dim-del", &t!("删除尺寸记录 {name}?(不影响模型)", "Delete dimension {name}? (the model is not affected)")) {
                Some(true) => {
                    self.dim_del_confirm = None;
                    match pla_ui::dims::delete(&self.doc_key, &name) {
                        Ok(()) => {
                            if self.dim_active.as_deref() == Some(name.as_str()) {
                                self.dim_active = None;
                            }
                            self.dims_pngs.remove(&name);
                            self.dims_vals.remove(&name);
                            self.dims_reload();
                        }
                        Err(e) => self.set_notice((true, t!("删除失败: {e}", "Delete failed: {e}"))),
                    }
                }
                Some(false) => self.dim_del_confirm = None,
                None => {}
            }
        }
    }

    /// 当前 / 目标两栏数值框 + 底部 Esc Cancel · OK ↵。右键(窗口内任意处)/ 回车 / OK = 应用; Esc / Cancel = 目标值复原。
    /// 返回 Some(目标值) = 本帧要应用。"改过" 以显示精度判断(差 > 0.0005), 偏移量仍按真实值算 → 结果正好是输入值。
    fn dim_value_block(&mut self, ui: &mut egui::Ui, value: f64, units: &str, allow_zero: bool) -> Option<f64> {
        let mut enter = false;
        let mut cur = fmt3(value);
        ui.add_space(2.0);
        ui.columns(2, |cols| {
            rz_label(&mut cols[0], tr("当前", "Current"));
            rz_field(&mut cols[0], &mut cur, units, false);
            rz_label(&mut cols[1], tr("目标", "Target"));
            enter = rz_field(&mut cols[1], &mut self.dim_text, units, true);
        });
        let target = self.dim_text.trim().parse::<f64>().ok();
        let changed = target.is_some_and(|t| (t - value).abs() > 5e-4);
        let valid = target.is_some_and(|t| t > 0.0 || (allow_zero && t >= 0.0));
        let can = changed && valid && !self.dim_busy;
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let (cancel, ok) = rz_footer(ui, can, self.dim_busy);
        if cancel || esc {
            self.dim_text = fmt3(value);
        }
        // 条目数值框有焦点时右键归条目(见 dim_saved_ui), 不在这里应用
        let go = ok || (can && ((right_click_confirm(ui) && !self.dims_row_focus) || enter));
        if go && self.require_feature(Feature::DimFace) { target } else { None }
    }

    /// N3c: 多选 = 参考面(第一个, 绿) → 动的面组(锚 = 孔轴线 / 最近的平行面 / 面中点, 橙); 改值 = 整组沿方向平移。
    fn dim_distance_ui(&mut self, ui: &mut egui::Ui, v: &serde_json::Value, units: &str, sel: &[String]) {
        let Some(value) = v["value"].as_f64() else { return };
        let moving: Vec<serde_json::Value> = v["moving"].as_array().cloned().unwrap_or_default();
        let dir: Vec<f64> = v["direction"].as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
        let anchor = match v["anchor"].as_str().unwrap_or("") {
            "axis" => tr("量到孔 / 凸台轴线", "to the hole / boss axis"),
            "plane" => tr("量到最近的平行面", "to the nearest parallel face"),
            _ => tr("量到面中点(近似)", "to the face midpoint (approx.)"),
        };
        let n = moving.len();
        rz_info_card(
            ui,
            tr("距离", "Distance"),
            &t!("参考面(绿, 第一个选的) → {n} 个面 · {anchor}(橙)", "Reference (green, first selected) → {n} faces · {anchor} (orange)"),
        );
        if dir.len() != 3 {
            return;
        }
        // 对称: 动的面分在参考面两侧时可选 —— 整体平移 / 以参考面为对称面向两侧移动(用户 2026-11-15)
        if v["mixed"] == true {
            rz_label(ui, tr("动的面在参考面两侧", "Moving faces on both sides of the reference"));
            let labels = vec![tr("整体平移", "Translate all").to_string(), tr("对称(以参考面)", "Symmetric (about reference)").to_string()];
            let mut s = self.dim_sym as usize;
            if rz_segmented(ui, "dim-sym", &labels, &mut s) {
                self.dim_sym = s as u8;
            }
        }
        // 移动哪一边(对称时两侧都动, 不显示): 动的面 / 参考面 / 两边各半(跨实体同样适用, 用户 2026-11-15)
        if !Self::dim_symmetric(v, self.dim_sym) {
            rz_label(ui, tr("移动", "Move"));
            let labels = vec![tr("动的面", "Moving faces").to_string(), tr("参考面", "Reference").to_string(), tr("两边各半", "Both halves").to_string()];
            let mut s = self.dim_side as usize;
            if rz_segmented(ui, "dim-dside", &labels, &mut s) {
                self.dim_side = s as u8;
            }
        }
        if let Some(t) = self.dim_value_block(ui, value, units, true) {
            self.dim_apply(v, sel, t);
        }
    }

    /// N3: 在 Plasticity 里高亮面(对面 = primary, 其余候选 = candidate); key 没变不重发; 空 = 清除。
    fn dim_set_highlight(&mut self, key: String, faces: Vec<serde_json::Value>) {
        if key == self.dim_hl_key {
            return;
        }
        self.dim_hl_key = key;
        self.cdp.send(pla_bridge::cdp::CdpCmd::Call {
            name: pla_bridge::shim::api::VIEW_HIGHLIGHT,
            args: serde_json::json!({ "faces": faces }),
        });
    }

    /// R34: 视口状态 / 材质状态面板(同一套墙, 按 kind 取存储/文字/功能/CDP 命令)。返回面板是否仍开着。
    fn state_panel(&mut self, ctx: &egui::Context, desired: egui::Pos2, kind: StatePanel) -> bool {
        let (store, text, size, f_save, f_apply) = match kind {
            StatePanel::Vp => (
                pla_ui::vpstate::STORE,
                state_wall::WallText {
                    id: "vp",
                    kind: tr("视口状态", "view state"),
                    empty: tr("(空 — 点右上软盘图标保存当前视口)", "(Empty — click the disk icon at top right to save the view)"),
                    batch_delete_tip: tr("删除勾选的视口状态(含截图), 不可恢复", "Delete checked view states (with images), cannot be undone"),
                    update_confirm: |n| t!("用当前视口状态覆盖「{n}」? 原内容不可恢复", "Overwrite \"{n}\" with the current view state? The old content cannot be recovered"),
                },
                PANEL_VPSTATE,
                Feature::VpSave,
                Feature::VpApply,
            ),
            StatePanel::Mat => (
                pla_ui::matstate::STORE,
                state_wall::WallText {
                    id: "ms",
                    kind: tr("材质状态", "material state"),
                    empty: tr("(空 — 点右上软盘图标保存当前材质与指派)", "(Empty — click the disk icon at top right to save materials and assignments)"),
                    batch_delete_tip: tr("删除勾选的材质状态(含截图), 不可恢复", "Delete checked material states (with images), cannot be undone"),
                    update_confirm: |n| t!("用当前材质与指派覆盖「{n}」? 原内容不可恢复", "Overwrite \"{n}\" with the current materials and assignments? The old content cannot be recovered"),
                },
                PANEL_MATSTATE,
                Feature::MatSave,
                Feature::MatApply,
            ),
            StatePanel::Cfg => (
                pla_ui::configs::STORE,
                state_wall::WallText {
                    id: "cfg",
                    kind: tr("配置", "configuration"),
                    empty: tr(
                        "(空 — 先在 Plasticity 里显示 / 隐藏出一套方案、指好材质, 再点右上软盘图标保存为一个配置)",
                        "(Empty — show / hide one design variant in Plasticity and assign materials, then click the disk icon to save it as a configuration)",
                    ),
                    batch_delete_tip: tr("删除勾选的配置(含截图), 不可恢复; 不影响模型", "Delete checked configurations (with images), cannot be undone; the model is not affected"),
                    update_confirm: |n| t!("用当前的显示 / 隐藏与材质覆盖配置「{n}」? 原内容不可恢复", "Overwrite configuration \"{n}\" with the current visibility and materials? The old content cannot be recovered"),
                },
                PANEL_CONFIG,
                Feature::CfgSave,
                Feature::CfgApply,
            ),
        };
        let mut flag = true;
        let do_save = std::rc::Rc::new(std::cell::Cell::new(false));
        let do_open_dir = std::rc::Rc::new(std::cell::Cell::new(false));
        let do_manage = std::rc::Rc::new(std::cell::Cell::new(false));
        let tip_slot = std::rc::Rc::new(std::cell::RefCell::new(None::<(String, String)>));
        let (dsv, dodv, dmv, tsv) = (do_save.clone(), do_open_dir.clone(), do_manage.clone(), tip_slot.clone());
        let (folder_tip, save_tip) = match kind {
            StatePanel::Vp => (
                tr("打开视口状态文件夹", "Open view state folder"),
                tr("保存当前视口状态(VS 自动命名 + 截图)", "Save current view state (auto VS name + image)"),
            ),
            StatePanel::Mat => (
                tr("打开材质状态文件夹", "Open material state folder"),
                tr("保存当前材质与指派(MS 自动命名 + 截图)", "Save current materials and assignments (auto MS name + image)"),
            ),
            StatePanel::Cfg => (
                tr("打开配置文件夹", "Open configuration folder"),
                tr("把当前的显示 / 隐藏 + 材质存成一个配置(CF 自动命名 + 截图)", "Save the current visibility + materials as a configuration (auto CF name + image)"),
            ),
        };
        let icons: Box<dyn FnOnce(&mut egui::Ui)> = Box::new(move |ui| {
            let vbtn = |ui: &mut egui::Ui, kind: IconKind, tip: &str, hit: &std::cell::Cell<bool>| {
                if vector_icon_button(ui, kind, tip, &tsv).clicked() {
                    hit.set(true);
                }
            };
            vbtn(ui, IconKind::Trash, tr("批量删除", "Batch delete"), &dmv);
            vbtn(ui, IconKind::Folder, folder_tip, &dodv);
            vbtn(ui, IconKind::Save, save_tip, &dsv);
        });
        // 能力不可用: 面板顶部如实说明原因(点保存/恢复时 require_feature 同样拦截并提示)
        let blocked: Vec<String> = [f_save, f_apply]
            .iter()
            .filter_map(|f| self.feature_block(*f).map(|why| t!("「{}」不可用: {why}", "\"{}\" unavailable: {why}", f.label())))
            .collect();
        let doc_key = self.doc_key.clone();
        let mut wall = std::mem::take(match kind {
            StatePanel::Vp => &mut self.vp_wall,
            StatePanel::Mat => &mut self.ms_wall,
            StatePanel::Cfg => &mut self.cfg_wall,
        });
        let brief = |name: &str| -> String {
            match store.load(&doc_key, name) {
                Some(s) => match kind {
                    StatePanel::Vp => pla_ui::vpstate::brief(&s),
                    StatePanel::Mat => pla_ui::matstate::brief(&s),
                    StatePanel::Cfg => pla_ui::configs::brief(&s),
                },
                None => tr("状态数据缺失", "State data missing").into(),
            }
        };
        let mut outcome = state_wall::WallOutcome::default();
        let cur_tab = match kind {
            StatePanel::Vp => PanelId::VpState,
            StatePanel::Mat => PanelId::MatState,
            StatePanel::Cfg => PanelId::Config,
        };
        let mut tab_click = None;
        header_extra_and_show(ctx, CFG_PANEL_ID, cfg_title(), desired, egui::vec2(size[0], size[1]), &mut flag, true, |ui| {
            tab_click = cfg_tabs(ui, cur_tab);
            for b in &blocked {
                ui.label(egui::RichText::new(b).small().color(COL_RED));
            }
            outcome = state_wall::show(ui, &mut wall, store, &doc_key, &text, &brief);
        }, Some(icons));
        match kind {
            StatePanel::Vp => self.vp_wall = wall,
            StatePanel::Mat => self.ms_wall = wall,
            StatePanel::Cfg => self.cfg_wall = wall,
        }
        if tab_click.is_some() {
            self.cfg_pending_tab = tab_click;
        }
        for n in outcome.notices {
            self.set_notice(n);
        }
        if let Some(t) = tip_slot.borrow_mut().take().or(outcome.hint) {
            self.hovered_hint = Some(t);
        }
        // 点击恢复
        if let Some(name) = outcome.apply.filter(|_| self.require_feature(f_apply)) {
            match store.load(&self.doc_key, &name) {
                Some(state) => {
                    let (cmd, msg) = match kind {
                        StatePanel::Vp => (
                            pla_bridge::cdp::CdpCmd::VpStateApply(state),
                            t!("正在恢复视口状态「{name}」…", "Restoring view state \"{name}\"…"),
                        ),
                        StatePanel::Mat => (
                            pla_bridge::cdp::CdpCmd::MatStateApply(state),
                            t!("正在恢复材质状态「{name}」…", "Restoring material state \"{name}\"…"),
                        ),
                        StatePanel::Cfg => (
                            pla_bridge::cdp::CdpCmd::Call {
                                name: pla_bridge::shim::api::CFG_RESTORE,
                                args: serde_json::from_str(&state).unwrap_or_default(),
                            },
                            t!("正在应用配置「{name}」…", "Applying configuration \"{name}\"…"),
                        ),
                    };
                    self.cdp.send(cmd);
                    self.pending_focus = true;
                    self.set_notice((false, msg));
                }
                None => self.set_notice((true, tr("状态数据缺失", "State data missing").into())),
            }
        }
        // 保存 / 更新: 先采集状态 → 回包后再截图(VpStateResult / MatStateResult 处理器触发) → 落盘
        let update = outcome.update;
        let want_save = do_save.get();
        if (update.is_some() || want_save) && self.require_feature(f_save) {
            let tmp = store.dir(&self.doc_key).join("__shot_tmp.png");
            let _ = std::fs::create_dir_all(tmp.parent().unwrap_or(std::path::Path::new(".")));
            let tmp = tmp.display().to_string();
            let target = if want_save { None } else { update };
            let msg = match &target {
                Some(n) => t!("正在更新 {n}…", "Updating {n}…"),
                None => tr("正在采集当前状态…", "Capturing current state…").into(),
            };
            match kind {
                StatePanel::Vp => {
                    self.vp_shot_path = Some(tmp);
                    self.vp_pending_state = None;
                    self.vp_update_target = target;
                    self.cdp.send(pla_bridge::cdp::CdpCmd::VpStateSave);
                }
                StatePanel::Mat => {
                    self.ms_shot_path = Some(tmp);
                    self.ms_pending_state = None;
                    self.ms_update_target = target;
                    self.cdp.send(pla_bridge::cdp::CdpCmd::MatStateSave);
                }
                StatePanel::Cfg => {
                    self.cfg_shot_path = Some(tmp);
                    self.cfg_pending_state = None;
                    self.cfg_update_target = target;
                    self.cdp.send(pla_bridge::cdp::CdpCmd::Call { name: pla_bridge::shim::api::CFG_SAVE, args: serde_json::Value::Null });
                }
            }
            self.set_notice((false, msg));
        }
        if do_open_dir.get() {
            if let Err(e) = store.open_dir(&self.doc_key) {
                self.set_notice((true, e));
            }
        }
        if do_manage.get() {
            let w = match kind {
                StatePanel::Vp => &mut self.vp_wall,
                StatePanel::Mat => &mut self.ms_wall,
                StatePanel::Cfg => &mut self.cfg_wall,
            };
            w.manage = !w.manage;
            w.selected.clear();
            w.rename = None;
        }
        flag
    }

    /// 配置窗口开着(任意一个标签)。
    fn cfg_group_open(&self) -> bool {
        self.show_config || self.show_vpstate || self.show_matstate || self.show_entities
    }

    fn close_cfg_group(&mut self) {
        self.show_config = false;
        self.show_vpstate = false;
        self.show_matstate = false;
        self.show_entities = false;
    }

    /// 打开配置窗口并停在 tab(其余二级面板照旧互斥关掉); 每次切到某标签都重新读存档墙。
    fn open_cfg_tab(&mut self, tab: PanelId) {
        self.close_secondary_panels_except(tab);
        match tab {
            PanelId::Entities => {
                self.show_entities = true;
                self.sel_saved_loaded = false;
            }
            PanelId::VpState => {
                self.show_vpstate = true;
                self.vp_wall.loaded = false;
            }
            PanelId::MatState => {
                self.show_matstate = true;
                self.ms_wall.loaded = false;
            }
            _ => {
                self.show_config = true;
                self.cfg_wall.loaded = false;
            }
        }
        self.cfg_last = tab;
    }

    /// 快捷键: 已经停在这个标签 = 关窗口; 否则打开 / 切到它。
    fn toggle_cfg_tab(&mut self, tab: PanelId) {
        let on = match tab {
            PanelId::Entities => self.show_entities,
            PanelId::VpState => self.show_vpstate,
            PanelId::MatState => self.show_matstate,
            _ => self.show_config,
        };
        if on {
            self.close_cfg_group();
        } else {
            self.open_cfg_tab(tab);
        }
    }

    /// R8b: 二级面板统一互斥 — 打开一个, 关掉其余全部。
    fn close_secondary_panels_except(&mut self, keep: PanelId) {
        // 打开快照墙 / 配置窗口 → 关生产窗口(用户 2026-11-17: 不共存; 设置 / 能力面板小, 不受影响)
        if !matches!(keep, PanelId::Settings | PanelId::Caps) && self.show_bom {
            self.show_bom = false;
            self.bom_win.closed();
        }
        if keep != PanelId::Snapshots {
            self.show_snapshots = false;
        }
        if keep != PanelId::Entities {
            self.show_entities = false;
        }
        if keep != PanelId::VpState {
            self.show_vpstate = false;
        }
        if keep != PanelId::MatState {
            self.show_matstate = false;
        }
        if keep != PanelId::Config {
            self.show_config = false;
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
        self.vp_wall.reset();
        self.vp_pending_state = None;
        self.snap_update_confirm = None;
        self.sel_update_confirm = None;
        self.vp_update_target = None;
        // 材质状态
        self.ms_wall.reset();
        self.ms_pending_state = None;
        self.ms_update_target = None;
        self.cfg_wall.reset();
        self.cfg_pending_state = None;
        self.cfg_update_target = None;
        self.bom_result = None;
        self.bom_result_scope = None;
        self.bom_scope = None;
        self.bom_busy = false;
        self.bom_pending_scope = None;
        self.bom_batch = None;
        // N3 改尺寸 / N3d 尺寸记录
        self.dim_face = None;
        self.dim_info = None;
        self.dim_busy = false;
        self.dims_list.clear();
        self.dims_loaded = false;
        self.dim_active = None;
        self.dim_active_ids.clear();
        self.dim_pending = None;
        self.dim_pending_opp = None;
        self.dim_last_apply = None;
        self.dim_del_confirm = None;
        self.dims_manage = false;
        self.dims_selected.clear();
        self.dims_rename = None;
        self.dims_update_confirm = None;
        self.dims_batch_confirm = false;
        self.dims_vals.clear();
        self.dims_pngs.clear();
        self.dims_thumb_pending = None;
        self.dim_sym = 0;
        self.dim_cyl = None;
        self.dims_edit_row = None;
        self.dims_groups.clear();
        self.rec_search.clear();
        self.rec_solid_closed.clear();
        self.grp_rename = None;
        self.grp_del_confirm = None;
        self.hole_pending = None;
        self.attach_prompt = None;
        self.link_run = None;
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
            (self.show_matstate, PANEL_MATSTATE),
            (self.show_config, PANEL_CONFIG),
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

fn cfg_title() -> &'static str {
    tr("配置", "Configurations")
}

/// 配置窗口顶部的二级标签(配置 / 视口 / 材质 / 选择集); 点了别的标签 → 返回它。
fn cfg_tabs(ui: &mut egui::Ui, cur: PanelId) -> Option<PanelId> {
    let mut hit = None;
    ui.horizontal(|ui| {
        for (tab, label, tip) in [
            (PanelId::Config, tr("配置", "Configs"), tr("显示 / 隐藏 + 材质 = 一个方案", "Visibility + materials = one variant")),
            (PanelId::VpState, tr("视口", "Views"), tr("视角 / 着色 / 隐藏 / 网格", "Camera / shading / hidden / grid")),
            (PanelId::MatState, tr("材质", "Materials"), tr("材质定义 + 指派", "Material definitions + assignments")),
            (PanelId::Entities, tr("选择集", "Selections"), tr("保存 / 恢复一组选择", "Save / restore a selection")),
        ] {
            if ui.selectable_label(cur == tab, label).on_hover_text(tip).clicked() && cur != tab {
                hit = Some(tab);
            }
        }
    });
    ui.separator();
    hit
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

/// P4: 一次组联动(组名、目标值、待改的尺寸名队列、总条数)。
/// P5: 一次"按表达式改模型": 要改的 (尺寸名, 目标值) 逐条走同一条应用链路, 包在一个事务里。
struct LinkRun {
    /// 触发原因(状态栏显示, 如 "W = 120")。
    label: String,
    queue: std::collections::VecDeque<(String, f64)>,
    total: usize,
}

/// N3d: 等垫片回包的尺寸记录操作。
enum DimPending {
    /// 记录: 模板 + 主面个数(签名列表里前 n 个是 faces, 之后一个是 opposite)+ 是否"更新"已有记录(保留名字 / 收藏 / 创建时间)。
    Save(pla_ui::dims::DimRecord, usize, bool),
    /// 载入: dim.record 定位 + 读尺寸后在 Plasticity 里选中, 并建同名本体测量。
    Load(pla_ui::dims::DimRecord),
    /// 条目里直接改数值: dim.record 回包后直接按读到的尺寸应用(不等 Plasticity 选择回传)。
    Apply(pla_ui::dims::DimRecord, f64),
    /// 改后刷新: 按预测签名定位, 用定位到的当前签名与新值写回。
    Refresh(pla_ui::dims::DimRecord),
}

/// N3d: 最近一次应用 —— 记录里每个面(faces 之后 opposite)的平移量(文档单位)与新值。
struct DimApplied {
    name: String,
    moves: Vec<[f64; 3]>,
    value: f64,
}

/// 独立浮窗外框在 ui-prefs.json 里的键。
const NOTES_RECT_KEY: &str = "notes_rect";
const DIM_RECT_KEY: &str = "dim_rect";
const BOM_RECT_KEY: &str = "bom_rect";
const BOM_COLS_KEY: &str = "bom_cols";
const DRAW_PREV_RECT_KEY: &str = "draw_prev_rect";

/// 固定宽度、左对齐、超长截断的标签(表格里各行对齐用)。
fn fixed_label(ui: &mut egui::Ui, w: f32, text: egui::RichText) -> egui::Response {
    ui.allocate_ui_with_layout(egui::vec2(w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(w);
        ui.add(egui::Label::new(text).truncate())
    })
    .inner
}

/// 可拖宽的表头: 固定宽度 w 的标题, 右缘 ±4 px 可拖(30 ~ 600)。
fn col_head(ui: &mut egui::Ui, id: impl std::hash::Hash, text: egui::RichText, hover: Option<&str>, w: &mut f32) {
    let r = fixed_label(ui, *w, text);
    let cell = r.rect;
    if let Some(h) = hover {
        r.on_hover_text(h);
    }
    let x = cell.right() + 4.0; // 列间距(8)的中线
    let handle = egui::Rect::from_min_max(egui::pos2(x - 4.0, cell.top()), egui::pos2(x + 4.0, cell.bottom()));
    let resp = ui.interact(handle, ui.id().with(id), egui::Sense::drag());
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        ui.painter().vline(x, handle.y_range(), egui::Stroke::new(1.0_f32, COL_MUTED));
    }
    if resp.dragged() {
        *w = (*w + resp.drag_delta().x).clamp(30.0, 600.0);
    }
}

/// 独立浮窗(笔记 / 尺寸)的状态: 位置记忆 + 自管拖动/调大小会话。
#[derive(Default)]
struct FloatWin {
    /// 本次打开时交给 ViewportBuilder 的外框(打开期间不变 → egui 不会每帧重发定位, 不和拖动打架)。
    open_rect: Option<[f32; 4]>,
    /// 上次写进偏好文件的外框(变化且松手后再写)。
    saved_rect: Option<[f32; 4]>,
    /// R30b: 自管拖动/调大小会话(SetCapture + 屏幕坐标 delta; 不用 OS StartDrag/BeginResize, pitfalls #49)。
    drag: Option<NotesDrag>,
    /// 用户本次打开后手动调过大小 → 不再自动贴合内容高度(内容超出时用滚动条)。
    user_resized: bool,
}

impl FloatWin {
    fn load(key: &str) -> Self {
        FloatWin { saved_rect: pla_ui::prefs::load_rect(key), ..Default::default() }
    }
    /// 关掉了: 下次打开重新按记住的外框定位。
    fn closed(&mut self) {
        self.open_rect = None;
        self.drag = None;
        self.user_resized = false;
    }
}

/// 独立浮窗的外观与行为参数。
struct FloatSpec<'a> {
    id: &'static str,
    prefs_key: &'static str,
    title: &'a str,
    subtitle: String,
    close_tip: &'a str,
    default_rect: [f32; 4],
    min_size: egui::Vec2,
    resizable: bool,
    /// 高度自动贴合内容(用户手动调过大小后停止); 内容一律包在纵向滚动区里, 超出时出滚动条。
    fit_height: bool,
}

/// R30 / N3: 独立浮窗(笔记、尺寸)。与二级面板不同: 不跟随工具栏、不互斥、不被其他面板关掉;
/// 首次在默认位置, 之后用户拖到哪(/拉多大)就记住(ui-prefs.json `<prefs_key>`), 关掉重开仍在原处。
/// 标题行 = 拖动柄 + 标题 + 副标题 + ×; resizable 时四边四角可调大小。返回是否仍打开。
fn floating_window(ctx: &egui::Context, spec: FloatSpec, st: &mut FloatWin, body: impl FnOnce(&mut egui::Ui)) -> bool {
    let id = spec.id;
    let saved = st.saved_rect;
    let open_rect = *st.open_rect.get_or_insert_with(|| saved.unwrap_or(spec.default_rect));
    let (hide, reveal) = viewport_reveal(ctx, id);
    let mut builder = egui::ViewportBuilder::default()
        .with_title(id)
        .with_decorations(false)
        .with_resizable(spec.resizable)
        .with_transparent(true)
        .with_window_level(egui::WindowLevel::AlwaysOnTop)
        .with_position(egui::pos2(open_rect[0], open_rect[1]))
        .with_inner_size(egui::vec2(open_rect[2], open_rect[3]))
        .with_min_inner_size(spec.min_size);
    if hide {
        builder = builder.with_visible(false);
    }
    let mut open = true;
    let mut live_rect: Option<[f32; 4]> = None;
    let mut pointer_down = false;
    let mut start: Option<NotesDragMode> = None;
    let mut want_h: Option<f32> = None;
    let st_user_resized = st.user_resized;
    let drag_state = &mut st.drag;
    let mut body = Some(body);
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(id), builder, |ctx, _class| {
        if reveal {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            open = false;
        }
        egui::CentralPanel::default().frame(panel_frame()).show(ctx, |ui| {
            // ---- 标题行: 拖动柄 + 标题 + 副标题 + × ----
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
            let tr_ = paint_text(p, egui::pos2(row.left() + 18.0, row.center().y), egui::Align2::LEFT_CENTER, spec.title, 13.0, egui::Color32::WHITE, true);
            p.text(
                egui::pos2(tr_.right() + 8.0, row.center().y),
                egui::Align2::LEFT_CENTER,
                &spec.subtitle,
                egui::FontId::proportional(11.0),
                COL_MUTED,
            );
            let xr = egui::Rect::from_center_size(egui::pos2(row.right() - 9.0, row.center().y), egui::vec2(18.0, 18.0));
            let xresp = ui.interact(xr, egui::Id::new((id, "close")), egui::Sense::click());
            ui.painter().text(xr.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(15.0), if xresp.hovered() { egui::Color32::WHITE } else { COL_MUTED });
            if xresp.on_hover_text(spec.close_tip).clicked() {
                open = false;
            }
            if let Some(b) = body.take() {
                if spec.fit_height {
                    // 内容包在纵向滚动区: 窗口比内容矮时出滚动条; 记下内容高度供自动贴合
                    let scroll_top = ui.cursor().min.y;
                    let out = egui::ScrollArea::vertical()
                        .id_salt((id, "scroll"))
                        .auto_shrink([false, true])
                        .show(ui, |ui| b(ui));
                    // 窗口内高 = 滚动区顶部(含标题行与外边距)+ 内容高 + 底部外边距
                    want_h = Some(scroll_top + out.content_size.y + 14.0);
                } else {
                    b(ui);
                }
            }
        });
        paint_panel_border(ctx);
        let sr = ctx.screen_rect();
        if spec.resizable {
            // ---- 调大小热区(四边 + 四角) ----
            use egui::viewport::ResizeDirection as D;
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
            egui::Area::new(egui::Id::new((id, "resize")))
                .order(egui::Order::Foreground)
                .fixed_pos(sr.min)
                .show(ctx, |ui| {
                    for (i, (r, dir, icon)) in zones.into_iter().enumerate() {
                        let resp = ui.interact(r, egui::Id::new((id, "rz", i)), egui::Sense::drag());
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
                            egui::Stroke::new(1.0_f32, COL_MUTED),
                        );
                    }
                });
        }
        let vp = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
        if let (Some(o), Some(inner)) = vp {
            live_rect = Some([o.left(), o.top(), inner.width(), inner.height()]);
            // 自动贴合内容高度(没手动调过大小、没在拖动时; 封顶 900, 不低于最小高)
            if let (Some(h), false, true) = (want_h, st_user_resized, drag_state.is_none()) {
                let h = h.clamp(spec.min_size.y, 900.0);
                if (h - inner.height()).abs() > 1.5 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(inner.width(), h)));
                }
            }
        }
        // ---- R30b: 自管拖动/调大小 —— 开始与结束都以鼠标左键**物理**状态为准 ----
        // (旧版用 OS StartDrag/BeginResize: 模态循环吞掉松手事件 → egui 以为一直按着,
        //  鼠标移到别的窗口再回来就自己又开始拖/调大小, 窗口越拉越大。pitfalls #49)
        let phys_down = pla_ui::window_drag::primary_button_down();
        let ppp = ctx.pixels_per_point().max(0.01);
        if let (Some(mode), None, true, Some(r)) = (start, drag_state.as_ref(), phys_down, live_rect) {
            if let (Some(c), Some(h)) = (
                pla_ui::window_drag::cursor_pos_phys(),
                pla_ui::window_drag::find_own_window(id, r[2], ppp),
            ) {
                pla_ui::window_drag::set_capture(h);
                *drag_state = Some(NotesDrag { mode, cursor: c, rect: r, last: r });
            }
        }
        if let Some(d) = drag_state.as_mut() {
            if !phys_down {
                pla_ui::window_drag::release_capture();
                *drag_state = None;
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
        pointer_down = phys_down || drag_state.is_some();
    });
    if matches!(start, Some(NotesDragMode::Resize(_))) {
        st.user_resized = true;
    }
    // 位置/大小变了且已松手 → 写偏好(拖动/调整中不写, 免得每帧写盘)
    if let Some(r) = live_rect {
        let moved = st.saved_rect.map_or(true, |s| s.iter().zip(r.iter()).any(|(a, b)| (a - b).abs() > 1.0));
        if moved && !pointer_down {
            if let Err(e) = pla_ui::prefs::save_rect(spec.prefs_key, r) {
                pla_ui::diag::log(&format!("save {} rect failed: {e}", spec.prefs_key));
            }
            st.saved_rect = Some(r);
        }
    }
    open
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
        // 右键确认扩展到 Plasticity 窗口: 上一帧有等待确认的地方(调用过 right_click_confirm)→ 让垫片吃掉页面右键并计数;
        // 没有 → 解除, 完全不碰 Plasticity 的右键。本帧的"外部右键"由下面 poll_bridge 收到计数增加时置位。
        let want = RC_WANT.replace(false);
        RC_EXT.set(false);
        if want != self.rc_armed && self.cdp_caps.is_some() {
            self.rc_armed = want;
            self.cdp.send(pla_bridge::cdp::CdpCmd::Call { name: pla_bridge::shim::api::UI_RCLICK, args: serde_json::json!({ "arm": want }) });
        }
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
                        ui.painter().circle_stroke(dot_rect.center(), 5.4, egui::Stroke::new(1.2_f32, c));
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
                            Action::ToggleConfig => self.cfg_group_open(),
                            Action::ToggleDim => self.show_dim,
                            Action::ToggleBom => self.show_bom,
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
            // 配置 / 视口 / 材质 / 选择集 = 同一个窗口的四个标签(共用 panel-config)
            if self.cfg_group_open() {
                open.insert(CFG_PANEL_ID);
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
                            match confirm_popup_with(ui.ctx(), "snap-upd", &text, tr("确认更新", "Update"), COL_ACCENT) {
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
                let mut tab_click = None;
                header_extra_and_show(
                    ctx,
                    CFG_PANEL_ID,
                    cfg_title(),
                    desired,
                    egui::vec2(PANEL_ENTITIES[0], PANEL_ENTITIES[1]),
                    &mut flag,
                    true,
                    |ui| {
                        tab_click = cfg_tabs(ui, PanelId::Entities);
                        let open_err: Option<String> = None;
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
                                                    if enter || right_click_confirm(ui) { rename_commit = true; }
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
                                                if pencil_button(ui).on_hover_text(tr("改名", "Rename")).clicked() { rn = true; } if refresh_button(ui, 14.0).on_hover_text(tr("更新: 用当前状态覆盖这一项(需确认)", "Update: overwrite this item with the current state (confirm)")).clicked() { upd = true; }
                                                let (drect, dresp) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
                                                let dcol = if dresp.hovered() { COL_RED } else { COL_MUTED };
                                                let p = |x: f32, y: f32| egui::pos2(drect.left() + x, drect.top() + y);
                                                ui.painter().line_segment([p(3.0,4.0), p(11.0,4.0)], egui::Stroke::new(1.2_f32, dcol));
                                                ui.painter().line_segment([p(5.5,4.0), p(5.5,2.5)], egui::Stroke::new(1.2_f32, dcol));
                                                ui.painter().line_segment([p(8.5,4.0), p(8.5,2.5)], egui::Stroke::new(1.2_f32, dcol));
                                                ui.painter().line_segment([p(6.2,2.5), p(7.8,2.5)], egui::Stroke::new(1.2_f32, dcol));
                                                ui.painter().line_segment([p(4.0,4.0), p(5.0,12.0)], egui::Stroke::new(1.2_f32, dcol));
                                                ui.painter().line_segment([p(10.0,4.0), p(9.0,12.0)], egui::Stroke::new(1.2_f32, dcol));
                                                ui.painter().line_segment([p(5.0,12.0), p(9.0,12.0)], egui::Stroke::new(1.2_f32, dcol));
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
                                            if rn { rename_start = Some(meta.name.clone()); } if upd { update_ask = Some(meta.name.clone()); }
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
                                        if rn { rename_start = Some(meta.name.clone()); } if upd { update_ask = Some(meta.name.clone()); }
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
                            match confirm_popup_with(ui.ctx(), "sel-upd", &text, tr("确认更新", "Update"), COL_ACCENT) {
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
                if tab_click.is_some() {
                    self.cfg_pending_tab = tab_click;
                }
                self.show_entities = flag;
            }
        }

        // ============ R8-1 视口状态 / R34 材质状态面板(同一套墙: state_wall) ============
        if self.show_vpstate {
            if let Some(desired) = next_pos(&mut stack_y, PANEL_VPSTATE[1]) {
                self.show_vpstate = self.state_panel(ctx, desired, StatePanel::Vp);
            }
        }
        if self.show_matstate {
            if let Some(desired) = next_pos(&mut stack_y, PANEL_MATSTATE[1]) {
                self.show_matstate = self.state_panel(ctx, desired, StatePanel::Mat);
            }
        }
        if self.show_config {
            if let Some(desired) = next_pos(&mut stack_y, PANEL_CONFIG[1]) {
                self.show_config = self.state_panel(ctx, desired, StatePanel::Cfg);
            }
        }
        // 标签切换放到四个标签都画完之后(同一个视口一帧只画一次), 下一帧生效
        if let Some(tab) = self.cfg_pending_tab.take() {
            if self.cfg_group_open() {
                self.open_cfg_tab(tab);
                ctx.request_repaint();
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
                        // ---- D5: 大模型(出图助手): 用户自己的 Anthropic API key ----
                        settings_card(
                            ui,
                            "LLM",
                            Some((
                                tr("大模型(出图助手)", "LLM (drawing assistant)"),
                                vec![
                                    info_text(tr(
                                        "用你自己的 Anthropic API key 调 Claude, 按调用量计费, 费用走你的账号。key 用 Windows DPAPI 加密存在本机, 只有当前 Windows 用户能解开。",
                                        "Calls Claude with your own Anthropic API key, billed per use to your account. The key is stored encrypted with Windows DPAPI; only the current Windows user can decrypt it.",
                                    )),
                                    info_text(tr(
                                        "每次对话会把当前零件的出图设置、尺寸清单和预览图发给 Anthropic。",
                                        "Each message sends the current part's drawing settings, dimension list and preview image to Anthropic.",
                                    )),
                                ],
                            )),
                            |ui| {
                                let has = self.llm_cfg.has_key();
                                ui.label(
                                    egui::RichText::new(if has { tr("API key: 已保存(加密)", "API key: saved (encrypted)") } else { tr("API key: 未设置", "API key: not set") })
                                        .color(if has { COL_ACCENT } else { COL_MUTED }),
                                );
                                ui.add(egui::TextEdit::singleline(&mut self.llm_key_buf).password(true).hint_text("sk-ant-…").desired_width(f32::INFINITY));
                                let mut save = false;
                                ui.horizontal(|ui| {
                                    if ui.add_enabled(!self.llm_key_buf.trim().is_empty(), egui::Button::new(tr("保存 key", "Save key"))).clicked() {
                                        let k = std::mem::take(&mut self.llm_key_buf);
                                        match self.llm_cfg.set_key(&k) {
                                            Ok(()) => save = true,
                                            Err(e) => self.set_notice((true, e)),
                                        }
                                    }
                                    if ui.add_enabled(has, egui::Button::new(tr("清除", "Clear"))).clicked() {
                                        let _ = self.llm_cfg.set_key("");
                                        save = true;
                                    }
                                });
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(tr("模型", "Model")).color(COL_MUTED));
                                    save |= ui.add(egui::TextEdit::singleline(&mut self.llm_cfg.model).hint_text(pla_ui::llm::DEFAULT_MODEL).desired_width(f32::INFINITY)).lost_focus();
                                });
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(tr("接口", "Endpoint")).color(COL_MUTED))
                                        .on_hover_text(tr("走代理 / 网关时改; 空 = 官方接口", "Change for a proxy / gateway; empty = official API"));
                                    save |= ui.add(egui::TextEdit::singleline(&mut self.llm_cfg.base_url).hint_text(pla_ui::llm::DEFAULT_BASE).desired_width(f32::INFINITY)).lost_focus();
                                });
                                ui.horizontal(|ui| {
                                    let auto = self.llm_cfg.proxy_url().unwrap_or_else(|| tr("直连", "direct").to_string());
                                    ui.label(egui::RichText::new(tr("代理", "Proxy")).color(COL_MUTED)).on_hover_text(tr(
                                        "空 = 用环境变量 HTTPS_PROXY 或 Windows 系统代理; 填 none = 直连。官方接口在部分地区直连会被拒(403)",
                                        "Empty = HTTPS_PROXY env var or the Windows system proxy; none = direct. The official API rejects direct connections from some regions (403)",
                                    ));
                                    save |= ui.add(egui::TextEdit::singleline(&mut self.llm_cfg.proxy).hint_text(auto).desired_width(f32::INFINITY)).lost_focus();
                                });
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(tr("思考深度", "Effort")).color(COL_MUTED))
                                        .on_hover_text(tr("越深越准、越慢越贵; 一般 medium 就够", "Deeper = more accurate, slower and pricier; medium is usually enough"));
                                    let cur = self.llm_cfg.effort().to_string();
                                    for e in ["low", "medium", "high"] {
                                        if ui.selectable_label(cur == e, e).clicked() && cur != e {
                                            self.llm_cfg.effort = e.into();
                                            save = true;
                                        }
                                    }
                                });
                                if save {
                                    if let Err(e) = self.llm_cfg.save() {
                                        self.set_notice((true, e));
                                    }
                                }
                                ui.horizontal(|ui| {
                                    if ui
                                        .add_enabled(has && self.llm_test.is_none(), egui::Button::new(tr("测试连接", "Test connection")))
                                        .on_hover_text(tr("发一条很短的消息(花费不到一分钱)", "Sends one very short message (costs a fraction of a cent)"))
                                        .clicked()
                                    {
                                        let cfg = self.llm_cfg.clone();
                                        let (tx, rx) = std::sync::mpsc::channel();
                                        std::thread::spawn(move || {
                                            let r = pla_ui::llm::messages(&cfg, serde_json::json!({"max_tokens": 512, "messages": [{"role": "user", "content": "Reply with just: OK"}]}))
                                                .map(|v| format!("{} · {}", v["model"].as_str().unwrap_or(""), pla_ui::llm::text_of(&v).trim()));
                                            let _ = tx.send(r);
                                        });
                                        self.llm_test = Some(rx);
                                        self.llm_test_msg = None;
                                    }
                                    if let Some(r) = self.llm_test.as_ref().and_then(|rx| rx.try_recv().ok()) {
                                        self.llm_test = None;
                                        self.llm_test_msg = Some(match r {
                                            Ok(t) => (false, t!("连接正常: {t}", "Connected: {t}")),
                                            Err(e) => (true, e),
                                        });
                                    }
                                    if self.llm_test.is_some() {
                                        ui.label(egui::RichText::new(tr("测试中…", "Testing…")).color(COL_ACCENT));
                                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                                    }
                                });
                                if let Some((err, m)) = &self.llm_test_msg {
                                    ui.label(egui::RichText::new(m).size(11.0).color(if *err { COL_RED } else { COL_ACCENT }));
                                }
                            },
                        );
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
                                            "快照 = 文档同级 .pla/snapshots/ 下的副本; 文档未保存新内容时自动跳过。",
                                            "A snapshot is a copy in .pla/snapshots/ next to the document; skipped when nothing new was saved.",
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
        if self.show_dim {
            self.show_dim_window(ctx);
        }
        // D1 出图任务在后台跑, 窗口关了也要收结果
        self.draw_tick();
        // MCP 桥: 心跳 / 状态 + 处理 pla-mcp 的请求(窗口不动也每 2 s 醒一次)
        self.mcp_tick();
        ctx.request_repaint_after(Duration::from_secs(2));
        if self.draw_job.is_some() || self.draw_book.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        if self.show_bom {
            self.show_bom_window(ctx);
        }
        self.show_draw_preview(ctx);

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
