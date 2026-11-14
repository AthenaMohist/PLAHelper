//! [PLA-000 P0] 快捷键管理 — 绑定 + 冲突检测。
//! 按钮/动作可绑定全局快捷键; 绑定时即时检测冲突(同键多动作 = 冲突, 高亮告警)。
use pla_bridge::i18n::tr;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String, // "1".."9", "A".."Z", "F1".."F12", "Space"...
}

impl Hotkey {
    pub fn display(&self) -> String {
        let mut s = String::new();
        if self.ctrl { s.push_str("Ctrl+"); }
        if self.alt { s.push_str("Alt+"); }
        if self.shift { s.push_str("Shift+"); }
        s.push_str(&self.key);
        s
    }
}

/// 可绑定动作 — 与 UI 按钮一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    SnapshotManual,   // P0-1 手动快照
    SnapshotRestore,  // P0-1 切换快照
    SelSave,          // P0-2 保存选择集
    SelApply,         // P0-2 应用选择集
    DimApply,         // P1-1 应用尺寸
    EntitiesRefresh,  // 刷新实体树
    ToggleSnapshots,  // 工具栏: 开/关快照墙面板
    ToggleEntities,   // 工具栏: 开/关实体树面板
    ToggleDim,        // 工具栏: 开/关尺寸驱动面板
    ToggleSettings,   // 工具栏: 开/关设置面板
    ToggleVpState,    // R8-1: 开/关视口状态面板
    ToggleMatState,   // R34: 开/关材质状态面板
    ToggleNotes,      // R30: 开/关笔记面板(记录 + 计算器, 不与其他面板互斥)
}

impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Action::SnapshotManual => tr("手动快照", "Manual snapshot"),
            Action::SnapshotRestore => tr("切换快照", "Switch snapshot"),
            Action::SelSave => tr("保存选择集", "Save selection set"),
            Action::SelApply => tr("应用选择集", "Apply selection set"),
            Action::DimApply => tr("应用尺寸", "Apply dimensions"),
            Action::EntitiesRefresh => tr("刷新实体", "Refresh entities"),
            Action::ToggleSnapshots => tr("快照墙面板", "Snapshot wall panel"),
            Action::ToggleEntities => tr("实体树面板", "Entity tree panel"),
            Action::ToggleDim => tr("尺寸驱动面板", "Dimension panel"),
            Action::ToggleSettings => tr("设置面板", "Settings panel"),
            Action::ToggleVpState => tr("视口状态面板", "Viewport state panel"),
            Action::ToggleMatState => tr("材质状态面板", "Material state panel"),
            Action::ToggleNotes => tr("笔记(记录 + 计算)", "Notes (notes + calculator)"),
        }
    }
    /// 图标 (emoji, 随 CJK 字体渲染)。
    pub fn icon(&self) -> &'static str {
        match self {
            Action::SnapshotManual => "📷",
            Action::SnapshotRestore => "⏪",
            Action::SelSave => "💾",
            Action::SelApply => "🎯",
            Action::DimApply => "📐",
            Action::EntitiesRefresh => "🔄",
            Action::ToggleSnapshots => "🖼",
            Action::ToggleEntities => "🌳",
            Action::ToggleDim => "📐",
            Action::ToggleSettings => "⚙",
            Action::ToggleVpState => "🎥",
            Action::ToggleMatState => "🎨",
            Action::ToggleNotes => "📝",
        }
    }
    pub const ALL: [Action; 13] = [
        Action::SnapshotManual, Action::SnapshotRestore, Action::SelSave,
        Action::SelApply, Action::DimApply, Action::EntitiesRefresh,
        Action::ToggleSnapshots, Action::ToggleEntities, Action::ToggleDim,
        Action::ToggleSettings, Action::ToggleVpState, Action::ToggleMatState, Action::ToggleNotes,
    ];
}

/// 绑定表 + 冲突检测。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HotkeyMap {
    /// action 序列化名 -> hotkey
    pub binds: std::collections::BTreeMap<String, Hotkey>,
}

impl HotkeyMap {
    /// 冲突检测: 返回 (hotkey_display, [动作名…]) — 多于 1 个动作共用同一键即冲突。
    pub fn conflicts(&self) -> Vec<(String, Vec<String>)> {
        use std::collections::BTreeMap;
        let mut by_key: BTreeMap<Hotkey, Vec<String>> = BTreeMap::new();
        for (a, hk) in &self.binds {
            by_key.entry(hk.clone()).or_default().push(a.clone());
        }
        by_key
            .into_iter()
            .filter(|(_, v)| v.len() > 1)
            .map(|(k, v)| (k.display(), v))
            .collect()
    }

    /// 设置绑定前检查: 返回 Err(冲突动作) 若该键已被别的动作占用。
    pub fn try_bind(&mut self, action: &str, hk: Hotkey) -> Result<(), Vec<String>> {
        let others: Vec<String> = self
            .binds
            .iter()
            .filter(|(a, h)| *a != action && **h == hk)
            .map(|(a, _)| a.clone())
            .collect();
        if others.is_empty() {
            self.binds.insert(action.to_string(), hk);
            Ok(())
        } else {
            Err(others)
        }
    }

    /// 纯逻辑匹配: 修饰键 + 键名(大写比较)是否命中某动作。egui 层在 main.rs 解事件后调用。
    pub fn matches_raw(&self, action: &str, ctrl: bool, alt: bool, shift: bool, key_name: &str) -> bool {
        let Some(hk) = self.binds.get(action) else { return false };
        hk.ctrl == ctrl && hk.alt == alt && hk.shift == shift
            && hk.key.eq_ignore_ascii_case(key_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hk(ctrl: bool, key: &str) -> Hotkey {
        Hotkey { ctrl, alt: false, shift: false, key: key.into() }
    }

    #[test]
    fn bind_and_conflict() {
        let mut m = HotkeyMap::default();
        assert!(m.try_bind("SnapshotManual", hk(true, "S")).is_ok());
        // 同键绑别的动作 = 冲突拒绝
        let err = m.try_bind("SelSave", hk(true, "S")).unwrap_err();
        assert_eq!(err, vec!["SnapshotManual".to_string()]);
        // 冲突总览报告
        let _ = m.try_bind("SelApply", hk(true, "D"));
        assert!(m.conflicts().is_empty());
    }

    #[test]
    fn raw_match_case_insensitive() {
        let mut m = HotkeyMap::default();
        let _ = m.try_bind("SnapshotManual", hk(true, "S"));
        assert!(m.matches_raw("SnapshotManual", true, false, false, "S"));
        assert!(m.matches_raw("SnapshotManual", true, false, false, "s"));
        assert!(!m.matches_raw("SnapshotManual", false, false, false, "S"));
        assert!(!m.matches_raw("SnapshotManual", true, false, false, "D"));
        assert!(!m.matches_raw("SelSave", true, false, false, "S"));
    }

    #[test]
    fn display_format() {
        assert_eq!(hk(true, "S").display(), "Ctrl+S");
        let mut h = hk(true, "F5");
        h.alt = true; h.shift = true;
        assert_eq!(h.display(), "Ctrl+Alt+Shift+F5");
    }
}

/// 持久化路径: exe 同级 hotkeys.json(R7-15, 工程债 #13)。
fn config_path() -> PathBuf {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    exe.join("hotkeys.json")
}

use std::path::PathBuf;

/// 保存快捷键绑定到 hotkeys.json(尽力而为, 失败静默)。
pub fn save_binds(binds: &HotkeyMap) {
    let path = config_path();
    if let Ok(json) = serde_json::to_string_pretty(&binds.binds) {
        let _ = std::fs::write(&path, json);
    }
}

/// 启动时加载 hotkeys.json; 文件不存在/损坏时返回 None(用默认绑定)。
pub fn load_binds() -> Option<HotkeyMap> {
    let path = config_path();
    let data = std::fs::read_to_string(&path).ok()?;
    let binds: std::collections::BTreeMap<String, Hotkey> = serde_json::from_str(&data).ok()?;
    Some(HotkeyMap { binds })
}
