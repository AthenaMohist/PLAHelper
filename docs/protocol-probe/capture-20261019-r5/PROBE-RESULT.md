# R5 探针报告 — 码 23 SUBSCRIBE_ALL + 码 22 帧长实测（2026-10-19）

执行者: R5 BugFix 会话（软件工程师）。目标: 为 P0「刷实体失败」取证。
环境: bridge 在线 ws://127.0.0.1:8980，Plasticity 聚焦文档 model1.plasticity。
探针源码: `crates/pla-snapshot/examples/subscribe_probe.rs`（本目录产物即其输出）。

## 结论速览

1. **码 23 (SUBSCRIBE_ALL_1) 两种顺序均零推帧** — direct 模式收集 60s、
   after22 模式首跑收集 60s 均无任何服务端推送帧（落盘取证复跑
   `--secs 8` 的 8s 窗口同样 0 帧）。它不是分片/分页机制，PLA-004
   TODO#9 不能指望它。注: 23 语义应为「文档变更时推事件」（NEW_VERSION 类），
   空闲期间无编辑自然无事件；但即使有也只是变更通知，不是树数据。
2. **码 22 当前实测帧长 1,091,464B ≈ 1.04MB**（两次 after22 运行均测得，
   同一文档未变；direct 模式未发码 22，n=2 均为 after22）——远超此前
   记录的「≈368040B 单帧上限」。**服务端没有在 368KB 截断**，
   该数字是当时文档的实际大小，不是上限。
3. **1MB 帧逐字节解析完全干净**: code=22 mid=50 resp=200 file=model1.plasticity
   ver=0，2 条 record（容器 1,091,300B + 112B），consumed == frame_len，
   tail_clean=True。**截断理论被证伪** — Frank「刷实体失败」的真实根因是
   **UI 侧 5s 超时**（1MB 序列化+传输 > 5s），不是数据丢失。
4. 已落地修复: UI 外层超时 5s→30s（错误文案「刷新超时(30s) — 文档过大?」），
   请求级超时 10s→40s（必须大于外层，保证友好文案先触发）；
   截断 fail-loud 保留为安全网（带「声明/实收」双数字的中文错误，
   直达 UI 错误浮窗）。
5. connection.rs `MAX_FRAME_BYTES=4MB` 对 1.09MB 帧足够；未来文档更大
   （>4MB）时 WS 层会断链，届时再评估。

## 产物清单（本目录）

| 文件 | 说明 |
|:--|:--|
| subscribe_probe_direct.txt | 直接发码23，收集 60s: 0 帧 |
| subscribe_probe_after22.txt | 先码22(1,091,464B) 再码23，收集 8s（落盘取证复跑; 首跑 60s 同样 0 帧）: 0 帧 |
| subscribe_pre22_dump.bin | 码22 完整响应原始帧 1,091,464B（分片设计取证素材） |
| subscribe_dump_*.bin | 码23 收集期落盘（均 0 字节，即无推送的证明） |

## 码表佐证

握手响应 68B 解析支持码 = [0,1,2,3,10,11,20,22,23,24,25,26,31,100] —
与 wire_probe 结论一致；21(ListSome) 不在表内（反馈文档建议的 ListSome
路线不可行），23 在表内但实测为「事件订阅」语义而非数据分片。
