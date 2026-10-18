//! pla-core: 核心领域逻辑占位 crate（M1-a 脚手架）。
//!
//! 后续承载模块：
//!
//! - `snapshot`   — 文档快照状态管理
//! - `selection`  — 选区（selection）跟踪
//! - `dim`        — dim 维度计算

/// 占位：core crate 已就位。
pub fn core_ready() -> bool {
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_core_scaffold() {
        assert!(super::core_ready());
    }
}
