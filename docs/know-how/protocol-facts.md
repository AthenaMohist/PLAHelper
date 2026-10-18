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
