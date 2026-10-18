# TODO 路线

## G2 — bridge 数据全接通（进行中）
1. ✅ 刷新实体 → 码22 全树（已接, 待 Windows 实测确认「已连接」+ 4 实体）
2. 快照墙读 snapshots/ 目录真实渲染（现在只有手动快照后插入的条目）
3. 切换快照 = 把快照副本写回 Plasticity 文档路径（需确认文档实际保存位置+文件锁）

## G3 — 选择集（P0-2）
4. 保存选择集 → bridge 组创建语义（PUT_SOME 协议逆向）
5. 应用选择集 → 组高亮/选中
6. 选择集持久化（本地 JSON）

## G4 — 尺寸驱动（P1-1）
7. facet 读写闭环（mesh payload 布局已逆向到顶点/面表, 待写回协议）
8. 应用平移 → PUT 写回 + Plasticity 刷新

## 曲线组补全
9. 码22 分块查询/翻页机制逆向（或探针试带参请求）→ 拉回 Circle/Polygon/Curve6-10/Spiral

## id 对齐
10. 嵌套实体 n0-n3 占位 → 服务器真实 id（影响 select/dim 引用）

## 工程债
11. bridge 调用换后台线程+通道（现在 UI 线程同步阻塞 ≤5s, 刷新时界面冻结）
12. pla-ui lib 1 warning 清理
13. 快捷键绑定持久化(现在重启丢失, 待加 config json)
