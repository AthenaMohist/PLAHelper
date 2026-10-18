# PLA 助手 — 项目详情总结（2026-10-20 打包）

## 1. 项目是什么

为 Plasticity（Studio 版 CAD）做外部辅助工具：通过 WebSocket bridge 二进制协议读取/操控场景，
提供 **快照管理（P0-1）、选择集（P0-2）、尺寸驱动（P1-1）** 三大功能 + GUI。
当前完成度：**bridge 协议逆向 ✅ / CLI(entities 全树) ✅ / GUI 骨架+快捷键 ✅ / bridge 数据进 UI ✅(刚接通) / 选择集与尺寸驱动 ⏳ / 曲线组截断 ⏳**

## 2. 目录结构

- `PRD/PRD-v1.md` — 定稿 PRD（功能范围唯一权威）
- `src/` — Rust workspace 全源码（crates: pla-core/bridge/snapshot/selection/dim/app/ui）
- `know-how/wire22_layout_notes.md` — 码22 二进制布局逆向笔记（核心资产）
- `know-how/protocol-facts.md` — 协议实锤汇总（码支持/帧上限/握手）
- `pitfalls.md` — 踩过的坑（全部带实锤）
- `TODO.md` — 待办路线
- `git-log.txt` — main 分支提交史（含每步 FIX 说明）

## 3. 架构一句话

pla-app(CLI) / pla-ui(GUI, egui) → pla-bridge(二进制协议) → ws://127.0.0.1:8980 → Plasticity。
逆向布局见 know-how；交叉编译: WSL mingw-w64 → x86_64-pc-windows-gnu 单 exe。

## 4. 关键实锤（详情见 pitfalls.md）

1. 码20 只回顶层实体；**码22 才是全场景树**（探针逐码扫描实锤）
2. 码22 全部实体打包在**单个大容器 payload**，需 tag=6+slen+name 嵌套扫描展开
3. **码22 单帧上限 ≈368040B**，超出静默截断（声明 len vs 实收 len 差 96B 实锤）→ 已 fail-loud
4. egui 默认字体无 CJK → 方块；**msyh.ttc 是 TrueType Collection 会让 ab_glyph panic 崩溃**
   （双击无反应的根因），只能用单文件 TTF
5. mock 测试里嵌套扫描对平铺小记录会误报 → 4KB 阈值隔离

## 5. 构建方法

WSL: `apt install mingw-w64; rustup target add x86_64-pc-windows-gnu`
`cargo build --release -p pla-ui --target x86_64-pc-windows-gnu`
Windows 侧零依赖双击运行（全部系统 DLL）。
