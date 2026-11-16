# D0 出图编译器验证 · FreeCAD TechDraw 路线(2026-11-16)

脚本: `tools/drawing/d0_freecad.py`(无界面, `freecadcmd.exe`)+ `tools/drawing/d0_export_gui.py`(整页 SVG / PDF, `freecad.exe` + `QT_QPA_PLATFORM=offscreen`, 不出窗口)。
本机: FreeCAD 1.1.1, `%LOCALAPPDATA%\Programs\FreeCAD 1.1\bin`, 自带 Python 3.11(有 numpy / matplotlib / PIL, 没有 ezdxf)。
测试件: 80×50×12 板, 沉头孔 Φ6.6 / Φ11×6.5、通孔 Φ5.5、30×16×4 槽。

## 结果

| 项 | 结果 |
|---|---|
| 第一角三视图 | ✓ `DrawProjGroup.ProjectionType = "First angle"`, 俯视在主视下、左视在主视右 |
| 轴测 | ✓ 单独 `DrawViewPart`, Direction (1,-1,1) |
| 剖视 + 剖面线 | ✓ `DrawViewSection`(BaseView = 俯视, 过孔轴线), 俯视上自动画 A—A 剖切符号, PDF / SVG 里有剖面线 |
| 虚线(不可见轮廓) | ✓ `HardHidden = True`(数据侧属性) |
| 圆心标记 | 界面侧属性 `ViewObject.ArcCenterMarks`, 只能在界面脚本里开 |
| 尺寸 | ✓ 值正确(Φ11 / Φ6.6 / Φ5.5 / 80 / 50 / 12); 支持公差(Over/UnderTolerance)、格式(FormatSpec)、理论正确尺寸 |
| 尺寸放置 | ✗ 不给位置 = 全堆在视图中心; 要我们算 X/Y(相对视图中心, 页面毫米, Y 向上)。第二版按"外框外 + 间距"放, 长宽已好, 孔径引线仍会穿进相邻视图 → 需要避让 |
| 孔标注 | ✗ 没有自动的沉孔 / 螺纹孔标注(⌴Φ11↧6.5、M5-6H↧10), 要我们按 `hole.make` 记录拼文字(FormatSpec / Arbitrary) |
| 图框 / 标题栏 | ✓ 自带 ISO 5457 A0–A4 模板, 可编辑字段 16 个(title / drawing_number / part_material / scale / creator / date …), `EditableTexts` 直接写; 国标(GB/T 10609)标题栏需自己做一个 SVG 模板 |
| 导出 | 无界面: `TechDraw.writeDXFPage`(有线 / 圆 / 样条 / 6 个 DIMENSION / 文字, **没有剖面线、没有图框**); 整页 SVG / PDF 要界面模块(`TechDrawGui.exportPageAsSvg/Pdf`), 先 `page.ViewObject.doubleClicked()` 打开页面再导, 否则只有图框 |
| 速度 | 启动 + 导入 ≈ 3.8 s; 建视图 + 剖视 + 尺寸 + DXF ≈ 0.35 s; 界面导出 SVG + PDF ≈ 4–5 s(含界面启动) |
| 产物 | A3 PDF 377 KB / SVG 211 KB / DXF 45 KB / FCStd 13 KB(FCStd 用户可在 FreeCAD 里打开继续改) |

## 结论

- FreeCAD 底座够用: 投影、剖视剖面线、虚线、尺寸对象(带公差)、图框模板、PDF / SVG / DXF 都现成, 本机已装, 零下载。
- 我们要写的(两条路线都要写, 与底座无关): ① 尺寸 / 引线放置与避让; ② 孔标注文字(从 `hole.make` 记录来); ③ 国标模板与制图规范 JSON(层 / 线宽 / 字体 / 剖面); ④ 视图选择(主视方向、剖切位置)= 出图方案 JSON。
- 坑: 圆心标记、整页导出是界面侧 → 出图走 "freecadcmd 建页 + freecad 离屏导出" 两步, 或一步全在离屏 freecad 里做。
- 还没验证: Plasticity 零件 → STEP(垫片里还没有导出能力, 需先查 Plasticity 的导出命令); 装配图(多实体、序号气泡 `DrawViewBalloon`、明细表); 中文字体(仿宋)在 PDF 里的嵌入。
- build123d + ezdxf 路线需要另装 Python 与 pip 包(要下载, 待用户同意); 对比项同上表。

## 螺纹画法（D1b，2026-11-16）

- TechDraw 没有螺纹画法；大径 3/4 圈细实线自己画进页面级 SVG（与坐标尺寸同一个 DrawViewSymbol，快且导出可见）。
- 位置用视图里实际的圆边圆心。视图边坐标 = 模型单位、以包围盒中心为原点，但正负不保证：用主视里全部孔对圆边做四种正负匹配（主视 模型 → 页面已由坐标尺寸验证），得到 边 → 页面 的正负，其他视图通用。
- 可见性：贯穿，或孔轴（指向入口外侧）· 视图 Direction（零件 → 观察者）> 0。


## 视图边坐标 → 页面（2026-11-17 实测）

- 视图边坐标 e = ((p − C)·X, (p − C)·U)：X = 视图 XDirection，U = Direction × XDirection，C = 视图所画形体的包围盒中心；
  未乘比例，页面位置 = 视图中心 + e × 比例（主视校准的正负为 (+, +)）。
- 剖视的 C 是"留下的那一半"的包围盒中心：两半各算一次，取投影外形与视图边包围盒对得上的那个。
- `DrawViewPart.projectPoint` 只是丢掉一个坐标（不居中、不旋转），不能用来求页面位置。
- 装配图（compile_assembly.py）也用这个关系放序号 / 配合标注；FreeCAD `Face.normalAt` 已按 Reversed 翻过，不要再翻。
- 多张 SVG 内嵌进同一个 HTML 打印合订本时，id（剖面线图案）要加前缀，否则后面的页用前面页的图案。
