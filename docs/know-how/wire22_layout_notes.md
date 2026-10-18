# wire_dump.bin 码22 布局逆向 (2026-10-20 OC)

## 头 (32B)
[code=22][mid][status=200][dlen=17]["model1.plasticity"+pad3]
@36: [0] @40: [顶层 count=2]

## 实体块统一布局(实锤 5 块)
[flags u32][payload_len u32][kind_raw u32][type_code u32][idx][?][0][parent=0xffffffff][tag=6][slen][name+pad][payload_len 字节 mesh]
- tag=6 + slen + name 锚定可靠(re 扫描 [6][slen 3..40][printable] 命中全部 5 块, 0 误报)

## dump 内容 (model1.plasticity)
Scene, Extruded.001(389v/350f 实锤), Extruded.002, Pipe.001, Box.001(8v 全解出)
⚠️ 无任何 Curve/Spiral — dump 是 model1 文档, Frank 列的 13 对象是另一文档(mydoc)。

## mesh payload (Box 8v 实锤)
[nverts][verts n×12B][normals n×12B][idx_count][tri idx×idx_count][face (start,count) 表]
Extruded.001: [389][verts][350 faces]... 索引段已定位

## 码支持
仅 20(LIST_ALL 顶层) + 22(全树 dump)。10/11/16/24-30 全 ERR。

## 下一步 (entities 全树)
1. pla-bridge 加 send(22) + parse: tag6-slen-name 扫描 + kr/tc→kind 映射(kr: 1=scene, 4=solid...)
2. entities 命令改走 22, 输出全部命名块


## 10-20 二次逆向补丁 (model1 实锤)
- Extruded.001 块头@88: [flags=0x65][payload_len=367944][kr=1][tc=4][0][idx=3][3][0][parent=ff][tag=6][slen=12][name]
- payload_len 声明 367944B, 实收 367848B → **响应截断 ~96B**: 码22 单帧上限≈368040B(360KB),
  model1 完整树 > 上限 → Curves 组(Circle/Polygon/Curve6-10/Spiral)整段被切
- Box.001 块尾后仍有浮点数据 = 截断残段
- 曲线不在 dump ≠ 不支持, 是截断丢的
- 方案: bridge 加码22 + payload_len vs 实收校验 fail-loud; 大文档需分块策略(发22时带起始组id? 待探针 v3 验证参数)
