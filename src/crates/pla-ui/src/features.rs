//! F2: pla-ui 功能 → 垫片能力的映射（唯一来源）。
//!
//! 每个依赖 CDP 的功能在这里登记一次：显示名 + 所需能力。界面上的拦截提示、设置面板的
//! 「CDP 能力」卡片、工具栏状态环都从这张表推导，不在各处另写判断。
//! 新功能（材质方案、打孔……）上线时只在这里加一行。

use pla_bridge::i18n::tr;
use pla_bridge::shim::{cap, CapsReport};

/// 功能分组（CDP 能力面板按组显示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Selection,
    View,
    Input,
    Document,
}

impl Group {
    pub const ALL: &'static [Group] = &[Group::Selection, Group::View, Group::Input, Group::Document];

    /// 卡片标题（大写英文，与设置面板卡片风格一致）+ 中文副标题。
    pub fn title(self) -> (&'static str, &'static str) {
        match self {
            Group::Selection => ("SELECTION", tr("选择与选择集", "Selection & selection sets")),
            Group::View => ("VIEW", tr("视角与视口状态", "View & viewport state")),
            Group::Input => ("INPUT", tr("按键与命令", "Keys & commands")),
            Group::Document => ("DOCUMENT", tr("文档、历史与诊断", "Document, history & diagnostics")),
        }
    }

    pub fn features(self) -> impl Iterator<Item = Feature> {
        Feature::ALL.iter().copied().filter(move |f| f.group() == self)
    }
}

/// 能力的中文说明（面板显示用；能力名本身来自 `shim::cap`）。
pub fn cap_label(c: &str) -> &'static str {
    match c {
        cap::EDITOR => tr("定位 Plasticity 编辑器", "Locate Plasticity editor"),
        cap::SEL_READ => tr("读取当前选择", "Read current selection"),
        cap::SEL_WRITE => tr("取消活动命令", "Cancel active command"),
        cap::HIST_READ => tr("读取撤销栈与几何版本", "Read undo stack & geometry version"),
        cap::CAM_READ => tr("读取相机与轨道控制器", "Read camera & orbit controls"),
        cap::CAM_WRITE => tr("写入相机与轨道控制器", "Write camera & orbit controls"),
        cap::CAM_NAV => tr("转到选择正视", "Face-on view of selection"),
        cap::CAM_STEP => tr("改步进角(运行时)", "Change step angle (runtime)"),
        cap::CAM_ROLL => tr("注册滚转命令", "Register roll commands"),
        cap::SEL_APPLY => tr("按 id 恢复选择(经选择命令)", "Restore selection by id (via select command)"),
        cap::VP_STATE => tr("视口显示状态(着色/剖面/构造面…)", "Viewport display state (shading/section/cplane…)"),
        cap::KEYMAP => tr("改按键绑定(运行时)", "Change keybindings (runtime)"),
        cap::CMD_DISPATCH => tr("注册 / 派发命令", "Register / dispatch commands"),
        cap::ERR_HOOK => tr("捕获命令报错", "Capture command errors"),
        cap::TREE => tr("读取实体树(Outliner)", "Read entity tree (Outliner)"),
        cap::MAT_SET => tr("材质方案", "Material schemes"),
        cap::CMD_EXEC => tr("带参数执行命令", "Execute commands with parameters"),
        cap::BOOL_RUN => tr("布尔运算", "Boolean operations"),
        cap::ASSET_PLACE => tr("放置常用件", "Place common parts"),
        _ => "",
    }
}

/// 路线图预留的能力（面板「ROADMAP」组显示其状态）。
pub const ROADMAP_CAPS: &[&str] = &[cap::MAT_SET, cap::CMD_EXEC, cap::BOOL_RUN, cap::ASSET_PLACE];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    SelMirror,
    SelSave,
    SelApply,
    VpSave,
    VpApply,
    OrbitStepLive,
    ArrowKeysLive,
    Roll,
    AutoSnapSteps,
    ErrorDisplay,
    EntityTree,
}

impl Feature {
    pub const ALL: &'static [Feature] = &[
        Feature::SelMirror,
        Feature::SelSave,
        Feature::SelApply,
        Feature::VpSave,
        Feature::VpApply,
        Feature::OrbitStepLive,
        Feature::ArrowKeysLive,
        Feature::Roll,
        Feature::AutoSnapSteps,
        Feature::ErrorDisplay,
        Feature::EntityTree,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Feature::SelMirror => tr("实时选择", "Live selection"),
            Feature::SelSave => tr("保存选择集", "Save selection set"),
            Feature::SelApply => tr("应用选择集", "Apply selection set"),
            Feature::VpSave => tr("保存视口状态", "Save viewport state"),
            Feature::VpApply => tr("恢复视口状态", "Restore viewport state"),
            Feature::OrbitStepLive => tr("步进角即时生效", "Live step angle"),
            Feature::ArrowKeysLive => tr("方向键绑定即时生效", "Live arrow-key bindings"),
            Feature::Roll => tr("Alt+←/→ 滚转", "Alt+←/→ roll"),
            Feature::AutoSnapSteps => tr("自动快照(定步数)", "Auto snapshot (every N steps)"),
            Feature::ErrorDisplay => tr("Plasticity 报错显示", "Plasticity error display"),
            Feature::EntityTree => tr("实体树", "Entity tree"),
        }
    }

    pub fn group(self) -> Group {
        match self {
            Feature::SelMirror | Feature::SelSave | Feature::SelApply => Group::Selection,
            Feature::VpSave | Feature::VpApply | Feature::OrbitStepLive | Feature::Roll => Group::View,
            Feature::ArrowKeysLive => Group::Input,
            Feature::AutoSnapSteps | Feature::ErrorDisplay | Feature::EntityTree => Group::Document,
        }
    }

    /// 所需能力（全部可用才算可用）。
    pub fn caps(self) -> &'static [&'static str] {
        match self {
            Feature::SelMirror => &[cap::SEL_READ],
            Feature::SelSave => &[cap::SEL_READ, cap::CAM_READ],
            Feature::SelApply => &[cap::SEL_APPLY],
            Feature::VpSave => &[cap::VP_STATE, cap::CAM_READ],
            Feature::VpApply => &[cap::VP_STATE, cap::CAM_WRITE],
            Feature::OrbitStepLive => &[cap::CAM_STEP],
            Feature::ArrowKeysLive => &[cap::KEYMAP],
            Feature::Roll => &[cap::CAM_ROLL],
            Feature::AutoSnapSteps => &[cap::HIST_READ],
            Feature::ErrorDisplay => &[cap::ERR_HOOK],
            Feature::EntityTree => &[cap::TREE],
        }
    }
}

/// 一个功能在当前探测结果下的状态。
#[derive(Debug, Clone, PartialEq)]
pub enum Availability {
    Ok,
    /// 可用，但部分可选结构缺失（列出缺的表键）。
    Degraded(Vec<String>),
    /// 不可用：原因（给用户看的一句话）。
    Blocked(String),
}

/// 判定功能可用性。caps = None 表示 CDP 未连接 / 尚未探测。
pub fn availability(caps: Option<&CapsReport>, f: Feature) -> Availability {
    let Some(r) = caps else {
        return Availability::Blocked(tr("CDP 未连接", "CDP not connected").into());
    };
    if !r.editor {
        return Availability::Blocked(tr("未找到 Plasticity 编辑器", "Plasticity editor not found").into());
    }
    let blocked = r.blocked(f.caps());
    if !blocked.is_empty() {
        let detail: Vec<String> = blocked
            .iter()
            .map(|(c, miss)| pla_bridge::t!("{c}(缺 {})", "{c} (missing {})", miss.join(", ")))
            .collect();
        return Availability::Blocked(pla_bridge::t!("当前 Plasticity 版本不支持: {}", "Not supported by this Plasticity version: {}", detail.join("; ")));
    }
    let mut opt: Vec<String> = f
        .caps()
        .iter()
        .filter_map(|c| r.caps.get(*c))
        .flat_map(|s| s.optional_missing.iter().cloned())
        .collect();
    opt.sort();
    opt.dedup();
    if opt.is_empty() {
        Availability::Ok
    } else {
        Availability::Degraded(opt)
    }
}

/// 整体健康度（工具栏状态环颜色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// 未连接 / 未探测。
    Unknown,
    /// 连上了但没有编辑器。
    NoEditor,
    /// 有功能不可用或降级。
    Partial,
    /// 全部功能可用。
    Full,
}

pub fn health(caps: Option<&CapsReport>) -> Health {
    let Some(r) = caps else { return Health::Unknown };
    if !r.editor {
        return Health::NoEditor;
    }
    if Feature::ALL.iter().all(|f| availability(Some(r), *f) == Availability::Ok) {
        Health::Full
    } else {
        Health::Partial
    }
}

/// 不可用 / 降级的功能数（工具栏提示用）。
pub fn summary(caps: Option<&CapsReport>) -> (usize, usize) {
    let mut blocked = 0;
    let mut degraded = 0;
    for f in Feature::ALL {
        match availability(caps, *f) {
            Availability::Blocked(_) => blocked += 1,
            Availability::Degraded(_) => degraded += 1,
            Availability::Ok => {}
        }
    }
    (blocked, degraded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pla_bridge::shim::CapState;

    fn report(bad: &[(&str, &[&str])], opt: &[(&str, &[&str])]) -> CapsReport {
        let mut r = CapsReport { editor: true, ..Default::default() };
        for c in cap::ALL {
            r.caps.insert(c.to_string(), CapState { ok: true, ..Default::default() });
        }
        for (c, miss) in bad {
            let s = r.caps.get_mut(*c).unwrap();
            s.ok = false;
            s.missing = miss.iter().map(|x| x.to_string()).collect();
        }
        for (c, miss) in opt {
            r.caps.get_mut(*c).unwrap().optional_missing = miss.iter().map(|x| x.to_string()).collect();
        }
        r
    }

    #[test]
    fn every_feature_uses_known_caps() {
        // 功能表引用的能力名必须都在 shim::cap::ALL 里(防止拼错 / 垫片改名后漏改)。
        for f in Feature::ALL {
            assert!(!f.caps().is_empty(), "{f:?} 没登记能力");
            for c in f.caps() {
                assert!(cap::ALL.contains(c), "{f:?} 引用了未知能力 {c}");
            }
        }
    }

    #[test]
    fn groups_cover_features_and_caps_have_labels() {
        let grouped: usize = Group::ALL.iter().map(|g| g.features().count()).sum();
        assert_eq!(grouped, Feature::ALL.len(), "每个功能必须恰好属于一个分组");
        for c in cap::ALL {
            assert!(!cap_label(c).is_empty(), "能力 {c} 缺中文说明");
        }
        for c in ROADMAP_CAPS {
            assert!(cap::ALL.contains(c));
        }
    }

    #[test]
    fn availability_and_health() {
        assert_eq!(health(None), Health::Unknown);
        assert!(matches!(availability(None, Feature::SelApply), Availability::Blocked(m) if m.contains("未连接")));
        let mut noed = report(&[], &[]);
        noed.editor = false;
        assert_eq!(health(Some(&noed)), Health::NoEditor);

        let full = report(&[], &[]);
        assert_eq!(health(Some(&full)), Health::Full);
        assert_eq!(summary(Some(&full)), (0, 0));

        let r = report(&[(cap::SEL_APPLY, &["ed.enqueue"])], &[(cap::VP_STATE, &["vp.settings.grid"])]);
        match availability(Some(&r), Feature::SelApply) {
            Availability::Blocked(m) => assert!(m.contains("ed.enqueue"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(availability(Some(&r), Feature::SelMirror), Availability::Ok);
        assert_eq!(
            availability(Some(&r), Feature::VpSave),
            Availability::Degraded(vec!["vp.settings.grid".into()])
        );
        assert_eq!(health(Some(&r)), Health::Partial);
        assert_eq!(summary(Some(&r)), (1, 2), "应用选择集不可用; 视口保存/恢复降级");
    }
}
