# N5 资产(asset pack)放置: 实测(26.2.3, 2026-11-18, model1 + 用户的 screws.plasticitypack)

用户建议(2026-11-18): 螺丝这类常用标准件**优先用 Plasticity 的 asset 功能**, 不要每次现建; 放螺丝放在打孔面板里。

## Plasticity 里的结构

- `editor.assets` = `AssetPackManager`: `getAssetPacks()` → `[{filePath, path, lastOpened}]`(只是登记信息);
  `load(filePath)`(参数是**路径字符串**; 传对象报 path must be string)→ 读 pack 文件:
  `{schemaVersion, id, revision, exportedAt, unit, bufferViews[], assets[]}`, 每个资产
  `{id: 'node:…', type, subtype: 'group', name, path: [文件夹], thumbnail(dataURL), bbox(米), pivot: {position, orientation}, contentHash}`。只读, 不进撤销栈。
- 用户的螺丝 pack(旧版): 12 个(M1.4 … M20, 文件夹「杯头内六角」), 每个资产 = 螺钉(如 `杯内六M4x10`)+ 负体(`杯内六M4Neg`)。
  负体是没有 helper 时记孔径的折中; 用户 2026-11-18 决定**去掉负体**, pack 只放螺丝本体(另出新 pack), 孔径以国标目录为准, helper 里负体相关操作已删。

## 手工拖进视口发生了什么(透明记录器实测)

1. Assets 面板 `dragstart` → 画布 `drop`: `DataTransfer` 类型 **`plasticity/asset`** = JSON
   `{id, pivot, packId, packRevision, packAbsolutePath, contentHash}`。
2. 起 `ImportWithPlacementCommand`(字段 `assetFile / id / pivot / assetReference / origin`): 导入资产内容(不走几何工厂),
   `nodes.setAssetReference(节点, {packId, packRevision, packPath(相对), packAbsolutePath, assetId, contentHash})`。
3. 等放置: 命令的 `promises`(4 个 `CancellablePromise`: `finish / cancel / interrupt`)。
4. `PlaceFactory` commit: `origin`(= pivot)/ `originOrientation` → `destination`(落点)/ `destinationOrientation`(资产 +Z 对着落点面的外法向)。
   整个拖放 = 一个撤销步。

## 自动放置(垫片 `asset.place`)

合成同样的 drop(`new DataTransfer()` + `setData('plasticity/asset', …)` + `DragEvent` dragenter/dragover/drop 打在画布中心)→
等**新的** `ImportWithPlacementCommand`(看 `executor.active.command`; `lastCommand` 在上一条被取消后还指着旧的)→
合成一次 `pointermove`(拾取器没有当前点时 `finish()` 什么也不做)→ 临时给 `PlaceFactory.prototype` 挂一个 commit, 把
destination / destinationOrientation 换成要的(`setFromUnitVectors(+Z, normal)` + 绕 normal 转 rotate°)→ 对命令的 promises 逐个 `finish()` →
撤销栈 +1 = 放好 → 删掉临时 commit。失败: 取消命令(导入的东西随之消失)+ 撤回到放之前。
(曾有可选 `cut` 用 `BooleanFactory`(setter `targets` / `tools`, `operationType` 默认 subtract)把负体减掉, 2026-11-18 随负体一起删了。)

坑:
- **命令对象是 thenable**(有 `then`): 拿它去 resolve 自己的 Promise, 链会等命令结束 —— 命令在等我们放置 = 死锁。包一层 `{cmd}`。
- 窗口不可见时拒绝(拖放 / 拾取器要可见窗口)。
- pla-ui 与 pla-app / pla-mcp 的垫片版本要一致(pla-ui 会把页面里的垫片换回它自己的版本, 正在跑的调用会被打断)。

实测: M4 放到 (300, 50, 20), 0.98 s, 螺钉头在面上方(pivot = 支承面, +Z = 杆 → 头)。
`hole.make {dry: true}` 不打孔, 只返回孔位 + 第一个孔位上各实体沿孔轴的入口 / 出口深度 `layers: [{solid, t0, t1}]`(包围盒投影), 打孔面板算螺丝长度用。

## 用在哪

- 打孔面板(「操作」→ 打孔), 用户 2026-11-18 定的顺序: 第一行选标准(国标螺钉 14 种, `fasteners::SCREWS`), 第二行规格 × 长度 +
  头下垫圈 / 弹垫 + 螺母, 再往下是原来的孔参数。规则在 `screw_kit.rs`(有单测):
  - 螺母只有**过孔 + 通孔 + 单件**才能配(盲孔 / 螺纹底孔 / 组合孔 / 紧定螺钉都不配); 尼龙锁紧螺母不加弹垫; 沉头 / 紧定不加头下垫圈。
  - 长度: 夹紧厚(单件 = 「操作」读到的面厚, 组合孔 = 最后一件的入口深度; 圆柱沉孔扣掉沉孔深)+ 头下垫圈 / 弹垫厚, 然后
    配螺母: + 螺母侧垫圈 + 螺母厚 + 2P, 取 ≥ 它的最短标准长度; 拧进螺纹: + 旋合 min(1.5d, 可用螺纹)(盲孔可用 = 孔深 − 2P, 不超过), 旋合 < d 报警;
    过孔通孔不配螺母 = 当作穿过去拧进没建模的件(+1.5d); 盲过孔 = 不顶底。手选长度超出范围报警。
  - 选沉头 → 孔类型自动切锥形沉孔、沉头径 = 头径; 选紧定 → 用途切螺纹底孔; 规格 → 孔规格跟着选同名 M 规格。
  - 每件(垫圈 / 螺钉 / 螺母)先在 pack 里认现成件(标准俗称在名字或文件夹里 + M 规格 + 螺钉长度: 名字里的 ×l, 没有就用包围盒最长边 − 头高推),
    认到就 `asset.place`, 没有就按国标宏建(`cmd.exec`); 长度下拉里 ★ = pack 里有。
- pla-mcp: `model_assets`(列表)、`model_asset_place`(放)、`model_part`(`kind: screw` + `std` = 目录 id)。

## 改长度(用户 2026-11-18)

pack 里的螺钉统一 10mm; 放进来后**只移动杆端的面**, 不整体缩放。垫片 `part.stretch {solid, axis(朝杆端), delta}`: 沿 axis 投影后贴着最远端、
且整个落在端部区(默认 min(2.5, 0.3 × 总长))里的面 = 端面 + 倒角锥, 一起 `moveFaces` 平移 delta, 杆的圆柱面跟着伸缩(一个撤销步)。
实测: M4×10 +15 → 杆端 z 10 → −5, 动了 2 个面, 头不变。打孔面板: pack 认到螺钉(不看长度)→ asset.place → 按 (要的长度 − 资产长度) 接着 stretch;
资产长度 = 名字里的 ×l, 否则包围盒最长边 − 头高, 都没有当 10。

## 建库 / 分发(N5b, 2026-11-18 实测)

实测建库: GB/T 70.1 全部 14 个规格 → `assets/packs/PLAhelper GB-T 70.1.plasticitypack`(679 KB, 15 s, 文档撤销深度不变, 自动登记);
从库里放 M4 到 (2000, 0, 20): 支承面正好在 z = 20(头 20..24), 再 part.stretch +15 → 杆端 −5 = M4×25。

- 资产 = 标了资产标记的组**或单个实体**(用户)。`ToggleAssetCommand` 作用于当前选择(一个撤销步); 标完 Plasticity 自动起一个
  `MoveItemCommand`(agent automatic, 不是选 pivot), 取消即可。`nodes.isAsset / findAllAssets` 可查。
- **setPivot 要真的 Vector3 / Quaternion**(类从视图的 position / quaternion 取): 给数组时 getPivot 照样回, 但之后导出**静默不写文件**。
- 选上实体时 Plasticity 会自动起 MoveItemCommand, 排在后面的 ToggleAsset 就一直等(第二件起必卡)→ 标记期间不是标记命令的活动命令都取消。
- **名字和 pivot 在标记那一刻定下来**: 之后 `setName` / `setPivot` 不进 pack。`nodes.setPivot(key, {position: [米], orientation: [x,y,z,w]})`
  (格式同 pack 里的 pivot; 不进撤销栈); 不设 = 包围盒中心(用户旧 pack 的 pivot 就是这样: 螺钉 + 负体的中心)。
- `ExportAssetPackCommand`: 主进程 `dialog.showSaveDialog`(title "Export asset pack", 过滤 .plasticitypack), 用 export.rs 的一次性应答即可,
  没有站内对话框; **文档里所有资产**都进 pack; **目标文件已存在时什么也不写** → 导到同目录临时文件再换上; 导出会**自动登记**这个 pack。
- 登记清单: `assets.add([路径…])` 收**数组**(新的排最前、去重; 给字符串会被拆成单个字符登记, 还会把别的项挤掉 —— 实测踩过, 已修复),
  `assets.remove(路径)`。`getAssetPacks()` → `[{filePath, path, lastOpened}]`。
- 实现: 垫片 `asset.mark`(改名 + pivot + 标, 取消自动移动, 还原选择)/ `asset.exportPack {expect}`(文档里资产数不对就拒绝, 免得用户自己的资产混进库)/
  `asset.register` · `asset.unregister {paths}`; pla-bridge `build_pack`: 开事务 → 逐件 cmd.exec + asset.mark → 武装另存为到 `~build-<ms>.plasticitypack` →
  导出 → 等写完 → 撤掉临时文件登记 → 回滚事务 → 换上目标。pla-ui `asset_lib`: 建库件(螺钉 10mm、沿 X 排开、头朝 +Z、pivot = 支承面,
  名字 `GB/T 70.1 M4x10`)、一个标准一个 pack(`PLAhelper GB-T 70.1.plasticitypack`)、分发目录 = 程序往上找 `assets/packs`(开发时 = 仓库),
  「加入 Plasticity」= 拷到 `数据目录/asset-packs` 再登记。界面: 打孔面板紧固件下面「资产库」一行; pla-mcp `model_asset_library`(series / build / install)、`model_part_stretch`。