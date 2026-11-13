//! F2: pla-ui 功能 → 垫片能力的映射（唯一来源）。
//!
//! 每个依赖 CDP 的功能在这里登记一次：显示名 + 所需能力。界面上的拦截提示、设置面板的
//! 「CDP 能力」卡片、工具栏状态环都从这张表推导，不在各处另写判断。
//! 新功能（材质方案、打孔……）上线时只在这里加一行。

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
            Group::Selection => ("SELECTION", "选择与选择集"),
            Group::View => ("VIEW", "视角与视口状态"),
            Group::Input => ("INPUT", "按键与命令"),
            Group::Document => ("DOCUMENT", "文档、历史与诊断"),
        }
    }

    pub fn features(self) -> impl Iterator<Item = Feature> {
        Feature::ALL.iter().copied().filter(move |f| f.group() == self)
    }
}

/// 能力的中文说明（面板显示用；能力名本身来自 `shim::cap`）。
pub fn cap_label(c: &str) -> &'static str {
    match c {
        cap::EDITOR => "定位 Plasticity 编辑器",
        cap::SEL_READ => "读取当前选择",
        cap::SEL_WRITE => "按 id 选中 / 清空选择",
        cap::HIST_READ => "读取撤销栈与几何版本",
        cap::CAM_READ => "读取相机与轨道控制器",
        cap::CAM_WRITE => "写入相机与轨道控制器",
        cap::CAM_NAV => "转到选择正视",
        cap::CAM_STEP => "改步进角(运行时)",
        cap::CAM_ROLL => "注册滚转命令",
        cap::SCAN => "悬停扫描定位",
        cap::VP_STATE => "视口显示状态(着色/剖面/构造面…)",
        cap::KEYMAP => "改按键绑定(运行时)",
        cap::ERR_HOOK => "捕获命令报错",
        cap::TREE => "读取实体树(Outliner)",
        cap::MAT_SET => "材质方案",
        cap::CMD_EXEC => "带参数执行命令",
        cap::BOOL_RUN => "布尔运算",
        cap::ASSET_PLACE => "放置常用件",
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
            Feature::SelMirror => "实时选择",
            Feature::SelSave => "保存选择集",
            Feature::SelApply => "应用选择集",
            Feature::VpSave => "保存视口状态",
            Feature::VpApply => "恢复视口状态",
            Feature::OrbitStepLive => "步进角即时生效",
            Feature::ArrowKeysLive => "方向键绑定即时生效",
            Feature::Roll => "Alt+←/→ 滚转",
            Feature::AutoSnapSteps => "自动快照(定步数)",
            Feature::ErrorDisplay => "Plasticity 报错显示",
            Feature::EntityTree => "实体树",
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
            Feature::SelApply => &[cap::SEL_WRITE, cap::SCAN, cap::CAM_WRITE, cap::CAM_NAV],
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
        return Availability::Blocked("CDP 未连接".into());
    };
    if !r.editor {
        return Availability::Blocked("未找到 Plasticity 编辑器".into());
    }
    let blocked = r.blocked(f.caps());
    if !blocked.is_empty() {
        let detail: Vec<String> = blocked
            .iter()
            .map(|(c, miss)| format!("{c}(缺 {})", miss.join(", ")))
            .collect();
        return Availability::Blocked(format!("当前 Plasticity 版本不支持: {}", detail.join("; ")));
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

        let r = report(&[(cap::SEL_WRITE, &["ed.click.addById"])], &[(cap::VP_STATE, &["vp.settings.grid"])]);
        match availability(Some(&r), Feature::SelApply) {
            Availability::Blocked(m) => assert!(m.contains("ed.click.addById"), "{m}"),
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
