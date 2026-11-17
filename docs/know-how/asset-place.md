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

## 建库 / 分发(N5b, 待实测)

只读探查(2026-11-18): `editor.assets` 只有 `getAssetPacks / load / add / remove`(源码被隐藏, 只能看行为); 命令里有 `ToggleAssetCommand`(把组标成资产)、
`ExportAssetPackCommand`(导出 pack)、`ReloadAssetCommand`; `nodes` 上有 `setAsset / isAsset / findAllAssets / setAssetReference / getAssetReference`。
打算: 在单独的库文档里按规范姿态建件 → 成组命名 → 标成资产 → 导出(存盘对话框用 export.rs 的主进程一次性应答)→ `assets.add` 登记。
