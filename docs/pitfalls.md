# 踩过的坑（全部实锤）

| # | 坑 | 根因 | 解法 |
|---|---|---|---|
| 1 | entities 只出 1 个实体，且两次跑实体不同 | 码20 只回顶层; 码22 把全部实体打包在单容器 payload, 顶层只解析出 1 条 | 码22 + 嵌套扫描(tag=6+slen+name+parent=ff)平铺展开 |
| 2 | 曲线组(Circle/Polygon/Curve6-10/Spiral)消失 | 码22 单帧上限≈368040B, 完整树超限被**静默截断**(声明367944 实收367848) | 解析层 fail-loud 报 truncated; 根治需分块查询(待办) |
| 3 | GUI 全部文字方块 | egui 默认字体无 CJK 字形 | 启动注入系统 TTF + 内嵌 simhei 兜底 |
| 4 | 双击 exe 无反应 | **msyh.ttc = TrueType Collection, ab_glyph 不支持 ttc → 渲染 panic**; 无控制台看不到 | 候选字体只用单文件 TTF(simhei/Deng); ttc 禁入清单已写注释 |
| 5 | 顶栏按钮重复两次 | clicked 与 secondary_clicked 各构建了一次 button | 一次构建, resp.clicked()/secondary_clicked() 分流 |
| 6 | mock 测试误报嵌套实体 | 嵌套扫描对平铺小记录的 pad 字节误判名字 | 4KB 阈值: 只对容器级大记录扫描 |
| 7 | eframe 0.29 API | run_native 回调要返回 Result; ui.columns 闭包参数是 &mut [Ui] | 用 SidePanel::show_inside 替代 columns |
| 8 | BridgeClient::connect 是同步 fn | 包 tokio::time::timeout 报 not-a-future | connect 直接调用, timeout 只包 async 的 list_visible |
| 9 | bin 里 crate:: 找不到 lib 模块 | bin 的 crate root 是 main.rs 自己 | 用 pla_ui::(lib 路径)引用 |

## 假按钮教训
v1 GUI 多个按钮点了没反应（骨架期只接了 2 个 action）。R2 改 run_action 集中分发:
未接线的功能弹诚实提示(说明缺什么), 不允许静默无反应。
