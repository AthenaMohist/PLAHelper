# 从 Plasticity 导出 STEP(实测 26.2.3, 2026-11-16)

代码: 垫片 `file.exportStep` / `file.exportRestore`(`pla-shim.js`, 名字表 `NAMES.exportStep`)+ `pla-bridge/src/export.rs`(主进程武装)
+ `cdp.rs::export_step`(全流程)+ `CdpCmd::ExportStep` + CLI `pla-app cdp-export-step` + BOM 窗口每行的 **STEP** 按钮。

## Plasticity 的导出链路

1. `Editor.export()`(无参; 命令 `file:export` 也走它)→ 渲染进程经 IPC `show-save-dialog` → 主进程 `dialog.showSaveDialog(win, opts)`。
   opts.filters 列出全部格式: OBJ / STL / PNG / **STEP(stp, step)** / Parasolid x_b, x_t / 3MF / SVG / IGES / SAT;
   defaultPath = 文档目录 + 文档名。
2. 按扩展名弹**站内**对话框(DOM 元素 `<export-step-58-dialog>`): Format = AP203 / AP214 / AP242 按钮、复选框
   `saveAnalyticsToNurbs`(解析面转 NURBS); **OK / Cancel 不在这个元素里**, 在外层容器(往上找同时含两者的祖先)。
3. 点 OK → 自带写出器写文件(约 1 s; 选中一个零件 8 KB, 全模型 2 MB)。
4. 导出范围 = 当前选择(`editor.selection.selected`), **没选 = 全部可见**。
   **零件在组里时, 选了它也会写出全部可见实体**(Box.001 在根下 → 只有它; Extruded.001 在两层组里 → 26 个实体全写)。

## 我们的做法(不碰写出器)

- 主进程: 经 inspector 9334 给 `dialog.showSaveDialog` 装一次性包装 —— 只应答绑定窗口(HWND)的下一次调用,
  回 `{canceled:false, filePath}`, 用一次或 20 s 超时就还原; 别的窗口 / 之后的调用照常弹系统对话框。
  代理进程(9333)不能改(它拉起 Plasticity, 不能重启), 所以 Rust 直接连 9334 的 node 目标做一次性 evaluate。
- 垫片: 记下用户选择 → `selection.selected.removeAll()` + `add(view)`(直接改选择, 不走选择命令)→ `Editor.export()` →
  等站内对话框 → 点格式按钮 / 复选框 → 点外层的 OK → 等对话框消失 → 回 `{selected, format, prev, names, parts:[{name, volume, area}]}`。
- Rust: 等文件出现且大小稳定 → `file.exportRestore` 还原选择。**写文件期间不能动选择**: 导出是命令, 动选择会取消它
  (日志 "invalid commit state: cancelled", 文件不写)。
- 武装失败就不调导出(否则用户面前会弹出真的"另存为"); 导出失败撤销武装、还原选择。

## 出图端怎么拿到那个零件

FreeCAD 读 STEP 不保留实体名(`MANIFOLD_SOLID_BREP('Box.001', …)` 里有名字, 但 Import 把对象叫 COMPOUND…, 顺序也和文件不同),
所以出图方案给 `part.volume`(+ `part.area`), `compile_freecad.py` 挑体积 / 面积最接近的实体(相同件挑哪个都一样; 差 > 0.5% 告警)。
BOM 的体积是网格估算, 和 B-rep 差约 0.2%(实测 679050 vs 680689)。

## 未解决

- `sel.apply`(走 Plasticity 选择命令)这次在窗口隐藏时没选上任何东西(missing solid:4), 原因待查; 导出改用直接 `add`, 不受影响。
- 镜像件体积 / 面积相同, 按体积挑可能挑到镜像的那个(要加手性判别)。
