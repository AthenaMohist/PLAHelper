# 协议实锤汇总

## 通道
ws://127.0.0.1:8980；握手后可用的码: 20(LIST_ALL 顶层) + 22(LIST_VISIBLE 全树)。10/11/16/24-30 全 ERR。

## 码22 响应布局（model1 实锤）
- 头 32B: [code=22][mid][status=200][dlen=17]["model1.plasticity"+pad3]
- @36 [0], @40 [顶层 count]
- 实体块: [flags u32][payload_len u32][kind_raw u32][type_code u32][idx][?][0][parent=0xffffffff]
  [tag=6][slen][name+pad][payload_len 字节 mesh]
- kind_raw 映射: 1=scene, 4=solid(待补全)
- mesh payload: [nverts][verts n×12B][normals n×12B][idx_count][tri idx][face(start,count) 表]
  Box=8v; Extruded.001=389v/350f; Pipe.001=7353v/14510f

## 截断
单帧上限≈368040B; 超限静默截断 → 必须校验声明 len vs 实收 len, 不等即报错。

## 嵌套扫描算法（0 误报）
扫描 [tag=6][slen 3..40][printable name] + 块头含 parent=0xffffffff;
仅对 ≥4KB 记录启用(防平铺小记录 pad 字节误报); 同名去重。

## R7-12 记录布局实锤（2026-10-20 双帧 17/17 验证 + 活桥复测）

- 码 20/22 记录体 = 背靠背名块流: `[T][idx][a][b][parent=ffffffff][tag=6][slen][name+pad]`
- **T(type_code): 0=实体(solid) 1=片体(sheet) 5=组/容器(Scene/Group)**。
  旧 DPL-218 "kind_raw=4=Extruded" 系把运行计数器误当类型, 已证伪
  (该槽值 4/14/232 随文档变化)。
- idx 树内编号全帧唯一 → 对外 id；a 是分类型计数器（跨类型撞号，不可作 id）。
- **b = 父节点 idx**（真层级）：实测 Scene(0)→'111'(1)→'111222'(2) 三层嵌套，
  体/片体 b∈{0,1,2} 与之一一吻合。parent 槽恒 0xffffffff（根标记，无用）。
- 体(T=0/1)名后 mesh: [nverts][verts nv×24B][ntri][tris ntri×12B][nn(=nv)]
  [normals nn×12B][尾部块 ~96B 结构未解]。前五段可精确跳过（扫描器已实现）。
- **curve/instance 不在码 22 下发** — 服务端 `_list/toEncodableItem` 只编码
  Blender 集合模型（组+体+片体），无过滤开关（jsc 字符串上下文实锤）。
  官方桥通道拿不到曲线/实例，后续需其它路线。
- 解析器: pla-bridge protocol.rs `scan_entity_blocks`（替换 parse_object_record
  + scan_nested_names）；mock 测试同步更新。
