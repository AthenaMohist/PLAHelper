//! [PLA-000 P0] 快捷键管理 — 绑定 + 冲突检测。
//! 按钮/动作可绑定全局快捷键; 绑定时即时检测冲突(同键多动作 = 冲突, 高亮告警)。
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
}

impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Action::SnapshotManual => "手动快照",
            Action::SnapshotRestore => "切换快照",
            Action::SelSave => "保存选择集",
            Action::SelApply => "应用选择集",
            Action::DimApply => "应用尺寸",
            Action::EntitiesRefresh => "刷新实体",
            Action::ToggleSnapshots => "快照墙面板",
            Action::ToggleEntities => "实体树面板",
            Action::ToggleDim => "尺寸驱动面板",
            Action::ToggleSettings => "设置面板",
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
        }
    }
    pub const ALL: [Action; 10] = [
        Action::SnapshotManual, Action::SnapshotRestore, Action::SelSave,
        Action::SelApply, Action::DimApply, Action::EntitiesRefresh,
        Action::ToggleSnapshots, Action::ToggleEntities, Action::ToggleDim,
        Action::ToggleSettings,
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
